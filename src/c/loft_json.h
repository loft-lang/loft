#ifdef LOFT_JSON
#include <errno.h>
/* ── JSON and loft-literal text (src/json.rs's `JParser`, src/lexer.rs's JSON mode) ──────────
 * A PORT: `JParser` reads its tokens from loft's own lexer in `LexConfig::json()` — the tokens
 * `{ } [ ] : , - .`, no keywords, no comment, no interpolation, JSON string escapes — and
 * `loft_jl_*` below reproduce exactly what that lexer answers there: the token at a cursor, its
 * start (`token_at`; a string's is its first content byte) and the decoded text of a string,
 * escapes and their quirks included (`\b` and `\f` are not loft escapes and read as `?`; an
 * unpaired high surrogate takes the backslash after it).  Numbers, string errors and every
 * delimiter are read from the raw bytes as `JParser` reads them, so a value's parse, its
 * errors, their offsets and the JSON pointer of the failing value are the Rust's.  The
 * emitter defines `LOFT_JSON` only for a unit that parses text. */

#define LOFT_JT_END 0
#define LOFT_JT_NONE 1
#define LOFT_JT_TOK 2
#define LOFT_JT_STR 3
#define LOFT_JT_NUM 4
#define LOFT_JT_IDENT 5
#define LOFT_JT_OTHER 6

/* `Parsed`: the value tree a JSON read builds. */
#define LOFT_JV_NULL 0
#define LOFT_JV_BOOL 1
#define LOFT_JV_NUMBER 2
#define LOFT_JV_INT 3
#define LOFT_JV_STR 4
#define LOFT_JV_IDENT 5
#define LOFT_JV_ARRAY 6
#define LOFT_JV_OBJECT 7
#define LOFT_JV_CTOR 8

typedef struct loft_jv {
    int kind;
    int b;
    double n;
    int64_t i;
    loft_text s;          /* a string, an identifier, a constructor's tag */
    size_t off;           /* a constructor's tag offset */
    struct loft_jv *kids; /* an array's items, an object's member values, a constructor's body */
    loft_text *keys;      /* an object's member names */
    size_t *key_offs;     /* an object's member key offsets */
    size_t nkids, cap;
} loft_jv;

/* A code point onto `t` as UTF-8. */
static void loft_jv_push_cp(loft_text *t, uint32_t cp) {
    char buf[4];
    size_t n = loft_utf8(cp, buf);
    if (n) loft_push(t, buf, n);
}

static void loft_jv_free(loft_jv *v) {
    loft_text_free(&v->s);
    for (size_t i = 0; i < v->nkids; i++) {
        loft_jv_free(&v->kids[i]);
        if (v->keys) loft_text_free(&v->keys[i]);
    }
    free(v->kids);
    free(v->keys);
    free(v->key_offs);
    memset(v, 0, sizeof *v);
}

static loft_jv *loft_jv_push(loft_jv *v, int keyed) {
    if (v->nkids == v->cap) {
        size_t n = v->cap ? v->cap * 2 : 4;
        v->kids = (loft_jv *) realloc(v->kids, n * sizeof *v->kids);
        if (keyed) {
            v->keys = (loft_text *) realloc(v->keys, n * sizeof *v->keys);
            v->key_offs = (size_t *) realloc(v->key_offs, n * sizeof *v->key_offs);
        }
        if (!v->kids || (keyed && (!v->keys || !v->key_offs))) { fputs("loft: out of memory\n", stderr); abort(); }
        v->cap = n;
    }
    loft_jv *k = &v->kids[v->nkids++];
    memset(k, 0, sizeof *k);
    if (keyed) {
        v->keys[v->nkids - 1].p = 0;
        v->keys[v->nkids - 1].len = v->keys[v->nkids - 1].cap = 0;
        v->key_offs[v->nkids - 1] = 0;
    }
    return k;
}

/* The token the lexer answers: its kind, `token_at`, the cursor after it, the token character,
 * and a string's decoded text. */
typedef struct {
    int kind;
    size_t at, end;
    char tok;
    loft_text text;
} loft_jtok;

/* A failure: the message and the byte offset it applies to (`Fault`). */
typedef struct {
    char msg[96];
    size_t at;
} loft_jfault;

/* A line's extent, cached across the tokens on it (`loft_jl_eol_at`). */
typedef struct {
    size_t from, end;
    int valid;
} loft_jline;

typedef struct {
    const uint8_t *b;
    size_t len;
    int lenient;
    size_t value_end;
    size_t cur;   /* the lexer's cursor */
    loft_jtok pk; /* the token at the cursor */
    /* `err_path`: the JSON pointer segments of a failing value, innermost first. */
    loft_text *path;
    size_t npath, path_cap;
    /* The line the cursor is on, [line_from, line_end]: no newline lies between them, so a
     * token anywhere in it ends its line at line_end.  A one-line document of N tokens then
     * finds its line end once instead of once per token. */
    loft_jline line;
} loft_jp;

static size_t loft_jl_ws(const uint8_t *b, size_t len, size_t i) {
    while (i < len && (b[i] == ' ' || b[i] == '\t' || b[i] == '\n' || b[i] == '\r')) i++;
    return i;
}

static size_t loft_jl_utf8(const uint8_t *b, size_t len, size_t i) {
    uint8_t c = b[i];
    size_t n = c < 0x80 ? 1 : c >= 0xF0 && c <= 0xF7 ? 4 : c >= 0xE0 ? 3 : c >= 0xC0 ? 2 : 1;
    return i + n > len ? len : i + n;
}

static int loft_jl_hex(uint8_t c) {
    return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
}

/* The end of the line `i` is on: the lexer reads the text a line at a time (`split('\n')`). */
static size_t loft_jl_eol(const uint8_t *b, size_t len, size_t i) {
    while (i < len && b[i] != '\n') i++;
    return i;
}

/* `read_hex4`: four hex digits at `*i`, leaving `*i` AT the fourth (the caller steps past it);
 * at a non-hex character `*i` stays on it and the answer is -1. */
static int32_t loft_jl_hex4(const uint8_t *b, size_t eol, size_t *i) {
    int32_t v = 0;
    for (int k = 0; k < 4; k++) {
        int d = *i < eol ? loft_jl_hex(b[*i]) : -1;
        if (d < 0) return -1;
        v = (v << 4) | d;
        if (k < 3) (*i)++;
    }
    return v;
}

/* `loft_jl_eol` for a position on the line `line` last answered: the same end, found once per
 * line instead of once per token. */
static size_t loft_jl_eol_at(const uint8_t *b, size_t len, size_t i, loft_jline *line) {
    if (line->valid && i >= line->from && i <= line->end) return line->end;
    line->from = i;
    line->end = loft_jl_eol(b, len, i);
    line->valid = 1;
    return line->end;
}

/* `Lexer::string` in JSON mode (`close` '"') or `Lexer::char` (`close` '\'') from the opening
 * quote `q`, whose line ends at `eol`: the decoded text and the cursor past the closing quote,
 * or 0 when the lexer cannot close the literal on its line.  A character literal steps past an
 * escape's last character even where a string's over-read does not. */
static int loft_jl_quoted(const uint8_t *b, size_t eol, size_t q, uint8_t close, loft_text *out, size_t *end) {
    size_t i = q + 1;
    out->p = 0;
    out->len = out->cap = 0;
    for (;;) {
        size_t run = i;
        while (i < eol && b[i] != close && b[i] != '\\') i++;
        if (i > run) loft_push(out, (const char *) b + run, i - run);
        if (i >= eol) break;
        if (b[i] == close) {
            *end = i + 1;
            return 1;
        }
        i++; /* the backslash */
        if (i >= eol) break;
        uint8_t c = b[i];
        if (c == '"' || c == '\'' || c == '\\' || c == '`' || c == '/') {
            loft_push(out, (const char *) &c, 1);
        } else if (c == 't') {
            loft_push(out, "\t", 1);
        } else if (c == 'r') {
            loft_push(out, "\r", 1);
        } else if (c == 'n') {
            loft_push(out, "\n", 1);
        } else if (c == '0') {
            loft_push(out, "", 1);
        } else if (c == 'x') {
            /* \xNN, ASCII only; the caller steps past the second digit. */
            i++;
            int ok = 0;
            if (i < eol) {
                int d1 = loft_jl_hex(b[i]);
                size_t j = loft_jl_utf8(b, eol, i);
                if (j < eol) {
                    i = j;
                    int d2 = loft_jl_hex(b[i]);
                    if (d1 >= 0 && d2 >= 0 && ((d1 << 4) | d2) < 0x80) {
                        char ch = (char) ((d1 << 4) | d2);
                        loft_push(out, &ch, 1);
                        ok = 1;
                    }
                } else {
                    i = j;
                }
            }
            if (!ok) loft_push(out, "?", 1);
            if (i >= eol) break;
        } else if (c == 'u') {
            i++;
            if (i < eol && b[i] == '{') {
                /* loft's own \u{N…}: 1-6 hex digits and a closing brace. */
                i++;
                size_t h = i;
                uint32_t cp = 0;
                while (i < eol && loft_jl_hex(b[i]) >= 0) {
                    cp = (cp << 4) | (uint32_t) loft_jl_hex(b[i]);
                    i++;
                }
                if (i >= eol || b[i] != '}' || i == h || i - h > 6 || cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF)) {
                    loft_push(out, "?", 1);
                } else {
                    loft_jv_push_cp(out, cp);
                }
                if (i >= eol) break;
            } else {
                int32_t cp = loft_jl_hex4(b, eol, &i);
                if (cp < 0) {
                    loft_jv_push_cp(out, 0xFFFD);
                } else if (cp < 0xD800 || cp > 0xDFFF) {
                    loft_jv_push_cp(out, (uint32_t) cp);
                } else if (cp >= 0xDC00) {
                    loft_jv_push_cp(out, 0xFFFD);
                } else {
                    i++; /* past the high surrogate's fourth digit */
                    int paired = 0;
                    if (i < eol && b[i] == '\\') {
                        i++;
                        paired = i < eol && b[i] == 'u';
                    }
                    if (!paired) {
                        /* Over-read: the cursor is past the escape already — for a string;
                         * a character literal steps on regardless. */
                        loft_jv_push_cp(out, 0xFFFD);
                        if (close == '"') continue;
                        if (i >= eol) break;
                        i = loft_jl_utf8(b, eol, i);
                        continue;
                    }
                    i++;
                    int32_t low = loft_jl_hex4(b, eol, &i);
                    if (low >= 0xDC00 && low <= 0xDFFF) {
                        loft_jv_push_cp(out, (uint32_t) (0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00)));
                    } else if (low >= 0) {
                        loft_jv_push_cp(out, 0xFFFD);
                        loft_jv_push_cp(out, low >= 0xD800 && low <= 0xDFFF ? 0xFFFD : (uint32_t) low);
                    } else {
                        loft_jv_push_cp(out, 0xFFFD);
                    }
                }
                if (i >= eol) break;
            }
        } else {
            loft_push(out, "?", 1);
        }
        i = loft_jl_utf8(b, eol, i); /* the caller's step past the escape's last character */
    }
    loft_text_free(out);
    *end = eol;
    return 0;
}

/* The token at cursor `i` (`Lexer::scan_once` in JSON mode). */
static loft_jtok loft_jl_token(const uint8_t *b, size_t len, size_t i, loft_jline *line) {
    loft_jtok t;
    memset(&t, 0, sizeof t);
    i = loft_jl_ws(b, len, i);
    t.at = i;
    if (i >= len) {
        t.kind = LOFT_JT_END;
        t.at = len;
        t.end = len;
        return t;
    }
    uint8_t c = b[i];
    if (c >= '0' && c <= '9') {
        /* A number as `Lexer::number` reads it: digits, a base marker after a leading 0,
         * separators; a `.` that a digit follows is a fraction, one that a `.`, a letter or
         * `_` follows ends the integer; a fraction or an exponent with no digits is no token;
         * a float takes a letter suffix.  What the parser takes is read again from the raw
         * text. */
        size_t j = i;
        int base = 0;
        while (j < len) {
            uint8_t d = b[j];
            if (d >= '0' && d <= '9') {
                j++;
            } else if ((d == 'x' || d == 'b' || d == 'o') && !base && j == i + 1 && b[i] == '0') {
                base = d == 'x' ? 16 : 2;
                j++;
            } else if (base == 16 && ((d >= 'a' && d <= 'f') || (d >= 'A' && d <= 'F'))) {
                j++;
            } else if (d == '_') {
                j++;
                if (!(j < len && b[j] >= '0' && b[j] <= '9')) break;
            } else {
                break;
            }
        }
        int fl = 0;
        if (j < len && b[j] == '.') {
            uint8_t n = j + 1 < len ? b[j + 1] : 0;
            if (n == '.' || (n >= 'a' && n <= 'z') || (n >= 'A' && n <= 'Z') || n == '_' || n >= 0x80) {
                t.kind = LOFT_JT_NUM;
                t.end = j;
                return t;
            }
            j++;
            size_t f0 = j;
            while (j < len && b[j] >= '0' && b[j] <= '9') j++;
            if (j == f0) {
                t.kind = LOFT_JT_NONE;
                t.end = j;
                return t;
            }
            fl = 1;
        }
        if (j < len && (b[j] == 'e' || b[j] == 'E')) {
            j++;
            if (j < len && (b[j] == '+' || b[j] == '-')) j++;
            size_t e0 = j;
            while (j < len && b[j] >= '0' && b[j] <= '9') j++;
            if (j == e0) {
                t.kind = LOFT_JT_NONE;
                t.end = j;
                return t;
            }
            fl = 1;
        }
        if (fl && j < len && ((b[j] >= 'a' && b[j] <= 'z') || (b[j] >= 'A' && b[j] <= 'Z')) && b[j] != 'f') {
            while (j < len && ((b[j] >= 'a' && b[j] <= 'z') || (b[j] >= 'A' && b[j] <= 'Z') || (b[j] >= '0' && b[j] <= '9') || b[j] == '_'))
                j++;
        }
        if (fl && j < len && b[j] == 'f') j++;
        t.kind = LOFT_JT_NUM;
        t.end = j;
        return t;
    }
    if (c == '"') {
        if (loft_jl_quoted(b, loft_jl_eol_at(b, len, i, line), i, '"', &t.text, &t.end)) {
            t.kind = LOFT_JT_STR;
            t.at = i + 1;
        } else {
            t.kind = LOFT_JT_NONE;
        }
        return t;
    }
    if (c == '\'') {
        /* A character literal (`Lexer::char`): everything to the next `'` on the line, escapes
         * read as a string's, its `token_at` after the quote; none when the line ends first. */
        loft_text lit = { 0, 0, 0 };
        t.at = i + 1;
        t.kind = loft_jl_quoted(b, loft_jl_eol_at(b, len, i, line), i, '\'', &lit, &t.end)
            ? LOFT_JT_OTHER : LOFT_JT_NONE;
        loft_text_free(&lit);
        return t;
    }
    if (c == '{' || c == '}' || c == '[' || c == ']' || c == ':' || c == ',' || c == '-' || c == '.') {
        t.kind = LOFT_JT_TOK;
        t.tok = (char) c;
        t.end = i + 1;
        return t;
    }
    if ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_') {
        size_t j = i;
        while (j < len && ((b[j] >= 'a' && b[j] <= 'z') || (b[j] >= 'A' && b[j] <= 'Z') || (b[j] >= '0' && b[j] <= '9') || b[j] == '_'))
            j++;
        t.kind = LOFT_JT_IDENT;
        t.end = j;
        return t;
    }
    if (c == '`') {
        t.kind = LOFT_JT_OTHER;
        t.end = loft_jl_eol_at(b, len, i, line);
        return t;
    }
    /* An unrecognized character: consumed, and no token. */
    t.kind = LOFT_JT_NONE;
    t.end = loft_jl_utf8(b, len, i);
    return t;
}

static void loft_jp_peek(loft_jp *p) {
    loft_text_free(&p->pk.text);
    p->pk = loft_jl_token(p->b, p->len, p->cur, &p->line);
}

/* `Lexer::cont`: past the token at the cursor. */
static void loft_jp_cont(loft_jp *p) {
    p->cur = p->pk.end;
    loft_jp_peek(p);
}

static size_t loft_jp_at_token(const loft_jp *p) { return p->pk.at < p->len ? p->pk.at : p->len; }

static int loft_jp_is(const loft_jp *p, char tok) { return p->pk.kind == LOFT_JT_TOK && p->pk.tok == tok; }

static void loft_jp_advance_past(loft_jp *p, size_t end) {
    while (p->pk.kind != LOFT_JT_END && p->pk.kind != LOFT_JT_NONE && loft_jp_at_token(p) < end) loft_jp_cont(p);
}

static size_t loft_jp_after_value(const loft_jp *p) { return loft_jl_ws(p->b, p->len, p->value_end); }

static int loft_jfail(loft_jfault *f, size_t at, const char *msg) {
    snprintf(f->msg, sizeof f->msg, "%s", msg);
    f->at = at;
    return 0;
}

static void loft_jp_path(loft_jp *p, const char *seg, size_t n) {
    if (p->npath == p->path_cap) {
        p->path_cap = p->path_cap ? p->path_cap * 2 : 8;
        p->path = (loft_text *) realloc(p->path, p->path_cap * sizeof *p->path);
        if (!p->path) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_text *t = &p->path[p->npath++];
    t->p = 0;
    t->len = t->cap = 0;
    loft_push(t, seg, n);
}

/* `JParser::open`: a reader at the start of `text` (a leading BOM stripped, a `#!/` first line
 * skipped as the lexer skips it), and the offset its value is expected at. */
static size_t loft_jp_open(loft_jp *p, const uint8_t *text, size_t len, int lenient) {
    memset(p, 0, sizeof *p);
    if (len >= 3 && text[0] == 0xEF && text[1] == 0xBB && text[2] == 0xBF) {
        text += 3;
        len -= 3;
    }
    p->b = text;
    p->len = len;
    p->lenient = lenient;
    if (len >= 3 && text[0] == '#' && text[1] == '!' && text[2] == '/') {
        size_t e = loft_jl_eol(text, len, 0);
        p->cur = e < len ? e + 1 : len;
    }
    loft_jp_peek(p);
    return loft_jl_ws(text, len, 0);
}

static void loft_jp_close(loft_jp *p) {
    loft_text_free(&p->pk.text);
    for (size_t i = 0; i < p->npath; i++) loft_text_free(&p->path[i]);
    free(p->path);
}

/* `json_string_validate`: the offset past the closing quote, or the first error. */
static int loft_jp_validate(const uint8_t *b, size_t len, size_t start, size_t *end, loft_jfault *f) {
    size_t i = start + 1;
    while (i < len) {
        uint8_t c = b[i];
        if (c == '"') {
            *end = i + 1;
            return 1;
        }
        if (c == '\\') {
            if (i + 1 >= len) return loft_jfail(f, i, "unterminated escape");
            i++;
            c = b[i];
            if (c == '"' || c == '\\' || c == '/' || c == 'b' || c == 'f' || c == 'n' || c == 'r' || c == 't') {
                i++;
            } else if (c == 'u') {
                if (i + 4 >= len) return loft_jfail(f, i + 1, "truncated \\uXXXX escape");
                for (int k = 1; k <= 4; k++)
                    if (loft_jl_hex(b[i + k]) < 0) return loft_jfail(f, i + 1, "invalid hex in \\uXXXX escape");
                i += 5;
            } else {
                /* `other as char`: the byte as a code point, so a UTF-8 lead byte reads as
                 * its Latin-1 character. */
                char m[32], ch[4];
                size_t n = loft_utf8(c, ch);
                snprintf(m, sizeof m, "invalid escape \\%.*s", (int) n, ch);
                return loft_jfail(f, i, m);
            }
        } else if (c < 0x20) {
            char m[48];
            snprintf(m, sizeof m, "raw control byte 0x%x in string", c);
            return loft_jfail(f, i, m);
        } else {
            i = loft_jl_utf8(b, len, i);
        }
    }
    return loft_jfail(f, start, "unterminated string");
}

/* `JParser::scan_number`: a JSON number from the raw text at `start`. */
static int loft_jp_scan_number(const loft_jp *p, size_t start, loft_jv *v, size_t *end, loft_jfault *f) {
    const uint8_t *b = p->b;
    size_t len = p->len, i = start;
    if (i < len && b[i] == '-') i++;
    if (i >= len || b[i] < '0' || b[i] > '9') return loft_jfail(f, i, "expected digit in number");
    if (b[i] == '0') {
        i++;
    } else {
        while (i < len && b[i] >= '0' && b[i] <= '9') i++;
    }
    int fractional = 0;
    if (i < len && b[i] == '.') {
        fractional = 1;
        i++;
        if (i >= len || b[i] < '0' || b[i] > '9') return loft_jfail(f, i, "expected digit after `.`");
        while (i < len && b[i] >= '0' && b[i] <= '9') i++;
    }
    if (i < len && (b[i] == 'e' || b[i] == 'E')) {
        fractional = 1;
        i++;
        if (i < len && (b[i] == '+' || b[i] == '-')) i++;
        if (i >= len || b[i] < '0' || b[i] > '9') return loft_jfail(f, i, "expected digit in exponent");
        while (i < len && b[i] >= '0' && b[i] <= '9') i++;
    }
    char buf[400];
    size_t n = i - start;
    if (n >= sizeof buf) n = sizeof buf - 1;
    memcpy(buf, b + start, n);
    buf[n] = 0;
    if (!fractional) {
        errno = 0;
        char *e = 0;
        long long x = strtoll(buf, &e, 10);
        if (errno == 0 && e && *e == 0) {
            v->kind = LOFT_JV_INT;
            v->i = (int64_t) x;
            *end = i;
            return 1;
        }
    }
    v->kind = LOFT_JV_NUMBER;
    v->n = strtod(buf, 0);
    *end = i;
    return 1;
}

static int loft_jp_value(loft_jp *p, size_t at, loft_jv *v, loft_jfault *f);

static int loft_jp_number(loft_jp *p, loft_jv *v, loft_jfault *f) {
    size_t start = loft_jp_at_token(p), end = 0;
    if (!loft_jp_scan_number(p, start, v, &end, f)) return 0;
    p->value_end = end;
    loft_jp_advance_past(p, end);
    return 1;
}

/* `JParser::take_string`: the string at the cursor, validated against JSON's grammar. */
static int loft_jp_take_string(loft_jp *p, loft_text *out, loft_jfault *f) {
    size_t quote = loft_jp_at_token(p) - 1, end = 0;
    if (!loft_jp_validate(p->b, p->len, quote, &end, f)) return 0;
    p->value_end = end;
    *out = p->pk.text;
    p->pk.text.p = 0;
    p->pk.text.len = p->pk.text.cap = 0;
    loft_jp_cont(p);
    return 1;
}

/* `scan_bare_ident`: the bare identifier from `start`, `.`-joined segments included. */
static size_t loft_jp_bare_ident(const uint8_t *b, size_t len, size_t start) {
    size_t j = start + 1;
    for (;;) {
        while (j < len && ((b[j] >= 'a' && b[j] <= 'z') || (b[j] >= 'A' && b[j] <= 'Z') || (b[j] >= '0' && b[j] <= '9') || b[j] == '_'))
            j++;
        if (j + 1 < len && b[j] == '.' && ((b[j + 1] >= 'a' && b[j + 1] <= 'z') || (b[j + 1] >= 'A' && b[j + 1] <= 'Z') || b[j + 1] == '_'))
            j++;
        else
            break;
    }
    return j;
}

/* `JParser::word`: the bare word at the cursor in full — its value, offset, and whether a `{`
 * follows an identifier (a `Tag{…}` constructor's tag). */
static int loft_jp_word(loft_jp *p, loft_jv *v, size_t *off) {
    *off = loft_jp_at_token(p);
    size_t end = loft_jp_bare_ident(p->b, p->len, *off);
    p->value_end = end;
    loft_jp_advance_past(p, end);
    size_t n = end - *off;
    const char *w = (const char *) p->b + *off;
    if (n == 4 && memcmp(w, "null", 4) == 0) {
        v->kind = LOFT_JV_NULL;
    } else if (n == 4 && memcmp(w, "true", 4) == 0) {
        v->kind = LOFT_JV_BOOL;
        v->b = 1;
    } else if (n == 5 && memcmp(w, "false", 5) == 0) {
        v->kind = LOFT_JV_BOOL;
        v->b = 0;
    } else {
        v->kind = LOFT_JV_IDENT;
        loft_push(&v->s, w, n);
    }
    return v->kind == LOFT_JV_IDENT && loft_jp_is(p, '{');
}

static int loft_jp_object(loft_jp *p, loft_jv *v, loft_jfault *f);

static int loft_jp_ident_value(loft_jp *p, loft_jv *v, loft_jfault *f) {
    if (p->lenient) {
        size_t off = 0;
        if (loft_jp_word(p, v, &off)) {
            /* `Tag{…}`: the tag, its offset, and the object as the one child. */
            v->kind = LOFT_JV_CTOR;
            v->off = off;
            loft_jv *body = loft_jv_push(v, 0);
            return loft_jp_object(p, body, f);
        }
        return 1;
    }
    size_t off = loft_jp_at_token(p);
    uint8_t c = off < p->len ? p->b[off] : 0;
    if (c == 'n' || c == 't' || c == 'f') {
        const char *word = c == 'n' ? "null" : c == 't' ? "true" : "false";
        size_t n = p->pk.end - off;
        if (n == strlen(word) && memcmp(p->b + off, word, n) == 0) {
            v->kind = c == 'n' ? LOFT_JV_NULL : LOFT_JV_BOOL;
            v->b = c == 't';
            loft_jp_cont(p);
            p->value_end = off + n;
            return 1;
        }
        char m[64];
        snprintf(m, sizeof m, "expected `%s` at offset %zu", word, off);
        return loft_jfail(f, off, m);
    }
    char m[64];
    snprintf(m, sizeof m, "unexpected byte 0x%x at offset %zu", c, off);
    return loft_jfail(f, off, m);
}

/* `JParser::open_array`: the array's offset, and where its first element is expected, or
 * none for `[]`, read in full. */
static int loft_jp_open_array(loft_jp *p, size_t *start, size_t *at) {
    *start = loft_jp_at_token(p);
    loft_jp_cont(p);
    if (loft_jp_is(p, ']')) {
        size_t close = loft_jp_at_token(p);
        loft_jp_cont(p);
        p->value_end = close + 1;
        return 0;
    }
    *at = loft_jl_ws(p->b, p->len, *start + 1);
    return 1;
}

/* `JParser::next_item`: 1 with `*at` set when another element follows, 0 at `]`, -1 on error. */
static int loft_jp_next_item(loft_jp *p, size_t start, size_t *at, loft_jfault *f) {
    size_t d = loft_jp_after_value(p);
    if (d >= p->len) return loft_jfail(f, start, "unterminated array") - 1;
    if (p->b[d] == ',') {
        loft_jp_cont(p);
        *at = loft_jl_ws(p->b, p->len, d + 1);
        return 1;
    }
    if (p->b[d] == ']') {
        loft_jp_cont(p);
        p->value_end = d + 1;
        return 0;
    }
    char m[64];
    snprintf(m, sizeof m, "expected `,` or `]` in array, got 0x%x", p->b[d]);
    return loft_jfail(f, d, m) - 1;
}

static int loft_jp_array(loft_jp *p, loft_jv *v, loft_jfault *f) {
    v->kind = LOFT_JV_ARRAY;
    size_t start = 0, at = 0;
    int more = loft_jp_open_array(p, &start, &at);
    while (more == 1) {
        loft_jv *item = loft_jv_push(v, 0);
        if (!loft_jp_value(p, at, item, f)) {
            char seg[24];
            snprintf(seg, sizeof seg, "%zu", v->nkids - 1);
            loft_jp_path(p, seg, strlen(seg));
            return 0;
        }
        more = loft_jp_next_item(p, start, &at, f);
        if (more < 0) return 0;
    }
    return 1;
}

/* `JParser::parse_object_key`: a quoted key, or (lenient) a bare identifier key. */
static int loft_jp_key(loft_jp *p, loft_text *name, size_t *key_at, loft_jfault *f) {
    if (p->pk.kind == LOFT_JT_END || p->pk.kind == LOFT_JT_NONE) return loft_jfail(f, p->len, "expected object key");
    if (p->pk.kind == LOFT_JT_STR) {
        *key_at = loft_jp_at_token(p) - 1;
        return loft_jp_take_string(p, name, f);
    }
    if (p->pk.kind == LOFT_JT_IDENT && p->lenient) {
        *key_at = loft_jp_at_token(p);
        loft_push(name, (const char *) p->b + p->pk.at, p->pk.end - p->pk.at);
        loft_jp_cont(p);
        return 1;
    }
    return loft_jfail(f, loft_jp_at_token(p), "expected string key in object");
}

/* `JParser::member`: a member's key and its `:`, and where its value is expected. */
static int loft_jp_member(loft_jp *p, loft_text *name, size_t *key_at, size_t *at, loft_jfault *f) {
    if (!loft_jp_key(p, name, key_at, f)) return 0;
    if (!loft_jp_is(p, ':')) {
        size_t off = p->pk.kind == LOFT_JT_END || p->pk.kind == LOFT_JT_NONE ? p->len : loft_jp_at_token(p);
        return loft_jfail(f, off, "expected `:` after object key");
    }
    size_t colon = loft_jp_at_token(p);
    loft_jp_cont(p);
    *at = loft_jl_ws(p->b, p->len, colon + 1);
    return 1;
}

/* `JParser::next_member`: 1 when another member follows, 0 at `}`, -1 on error. */
static int loft_jp_next_member(loft_jp *p, size_t start, loft_jfault *f) {
    size_t d = loft_jp_after_value(p);
    if (d >= p->len) return loft_jfail(f, start, "unterminated object") - 1;
    if (p->b[d] == ',') {
        loft_jp_cont(p);
        return 1;
    }
    if (p->b[d] == '}') {
        loft_jp_cont(p);
        p->value_end = d + 1;
        return 0;
    }
    char m[64];
    snprintf(m, sizeof m, "expected `,` or `}` in object, got 0x%x", p->b[d]);
    return loft_jfail(f, d, m) - 1;
}

/* `JParser::open_object`: the object's offset, and whether it has a member. */
static int loft_jp_open_object(loft_jp *p, size_t *start) {
    *start = loft_jp_at_token(p);
    loft_jp_cont(p);
    if (loft_jp_is(p, '}')) {
        size_t close = loft_jp_at_token(p);
        loft_jp_cont(p);
        p->value_end = close + 1;
        return 0;
    }
    return 1;
}

static int loft_jp_object(loft_jp *p, loft_jv *v, loft_jfault *f) {
    v->kind = LOFT_JV_OBJECT;
    size_t start = 0;
    int more = loft_jp_open_object(p, &start);
    while (more == 1) {
        loft_jv *val = loft_jv_push(v, 1);
        size_t i = v->nkids - 1, at = 0;
        if (!loft_jp_member(p, &v->keys[i], &v->key_offs[i], &at, f)) return 0;
        if (!loft_jp_value(p, at, val, f)) {
            loft_jp_path(p, v->keys[i].p ? v->keys[i].p : "", v->keys[i].len);
            return 0;
        }
        more = loft_jp_next_member(p, start, f);
        if (more < 0) return 0;
    }
    return 1;
}

/* `JParser::parse_value`: the value expected at `at`. */
static int loft_jp_value(loft_jp *p, size_t at, loft_jv *v, loft_jfault *f) {
    switch (p->pk.kind) {
    case LOFT_JT_END:
    case LOFT_JT_NONE:
        /* No token where a value is expected: the raw byte says whether a string or a number
         * the lexer could not read is there. */
        if (at < p->len) {
            if (p->b[at] == '"') {
                size_t end = 0;
                if (loft_jp_validate(p->b, p->len, at, &end, f)) return loft_jfail(f, p->len, "unexpected end of input");
                return 0;
            }
            if (p->b[at] == '-' || (p->b[at] >= '0' && p->b[at] <= '9')) {
                size_t end = 0;
                if (!loft_jp_scan_number(p, at, v, &end, f)) return 0;
                p->value_end = end;
                loft_jp_advance_past(p, end);
                return 1;
            }
        }
        return loft_jfail(f, p->len, "unexpected end of input");
    case LOFT_JT_NUM:
        return loft_jp_number(p, v, f);
    case LOFT_JT_STR:
        v->kind = LOFT_JV_STR;
        return loft_jp_take_string(p, &v->s, f);
    case LOFT_JT_IDENT:
        return loft_jp_ident_value(p, v, f);
    case LOFT_JT_TOK:
        if (p->pk.tok == '-') return loft_jp_number(p, v, f);
        if (p->pk.tok == '[') return loft_jp_array(p, v, f);
        if (p->pk.tok == '{') return loft_jp_object(p, v, f);
        /* fall through: a misplaced structural token */
    default: {
        size_t off = loft_jp_at_token(p);
        char m[64];
        snprintf(m, sizeof m, "unexpected byte 0x%x at offset %zu", off < p->len ? p->b[off] : 0, off);
        return loft_jfail(f, off, m);
    }
    }
}

/* `JParser::finish`: nothing but whitespace follows the document's one value. */
static int loft_jp_finish(const loft_jp *p, loft_jfault *f) {
    size_t end = loft_jp_after_value(p);
    if (end == p->len) return 1;
    char m[64];
    snprintf(m, sizeof m, "unexpected trailing byte at offset %zu", end);
    return loft_jfail(f, end, m);
}

/* `render_path`: the JSON pointer of the failing value, outermost first, `~` and `/` escaped. */
static void loft_jp_render_path(const loft_jp *p, loft_text *out) {
    for (size_t k = p->npath; k > 0; k--) {
        const loft_text *s = &p->path[k - 1];
        loft_push(out, "/", 1);
        for (size_t i = 0; i < s->len; i++) {
            if (s->p[i] == '~') loft_push(out, "~0", 2);
            else if (s->p[i] == '/') loft_push(out, "~1", 2);
            else loft_push(out, &s->p[i], 1);
        }
    }
}

/* `stores.last_json_errors`: the errors of the last parse, in order; `json_errors()` reads them
 * joined by `|`. */
static LOFT_TLS loft_text *loft_jerr = 0;
static LOFT_TLS size_t loft_jerr_n = 0, loft_jerr_cap = 0;

static void loft_jerr_clear(void) {
    for (size_t i = 0; i < loft_jerr_n; i++) loft_text_free(&loft_jerr[i]);
    loft_jerr_n = 0;
}

/* One more error, its text taken over. */
static void loft_jerr_take(loft_text t) {
    if (loft_jerr_n == loft_jerr_cap) {
        loft_jerr_cap = loft_jerr_cap ? loft_jerr_cap * 2 : 4;
        loft_jerr = (loft_text *) realloc(loft_jerr, loft_jerr_cap * sizeof *loft_jerr);
        if (!loft_jerr) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_jerr[loft_jerr_n++] = t;
}

static void loft_jerr_push(loft_str msg) {
    loft_jerr_take(loft_text_own(msg));
}

/* `json::parse_with`: the whole of `text` as one value; 0 with the fault and the failing
 * value's JSON pointer on an error. */
static int loft_json_parse(const uint8_t *text, size_t len, int lenient, loft_jv *v, loft_jfault *f, loft_text *path) {
    loft_jp p;
    size_t at = loft_jp_open(&p, text, len, lenient);
    memset(v, 0, sizeof *v);
    int ok = loft_jp_value(&p, at, v, f) && loft_jp_finish(&p, f);
    if (!ok) loft_jp_render_path(&p, path);
    loft_jp_close(&p);
    return ok;
}
#endif

#ifdef LOFT_JV_SIZE
/* ── The JsonValue tree in a store (native.rs `json_parse_into_stores` and the readers) ──
 * A JsonValue is an enum record: its variant's discriminant at byte 0, the payload at the
 * offsets the emitter defines (LOFT_JV_* below) from the program's own type table.  An array's
 * items and an object's JsonField members are vectors in the root's store, so the tree goes
 * with it. */
#define LOFT_JD_NULL 1
#define LOFT_JD_BOOL 2
#define LOFT_JD_NUMBER 3
#define LOFT_JD_STRING 4
#define LOFT_JD_ARRAY 5
#define LOFT_JD_OBJECT 6
#define LOFT_JD_INT 7


static uint8_t loft_jd(loft_ref v) {
    return *loft_at(loft_store_of(v), v.rec, v.pos);
}

/* `materialise_primitive_into`: tree `v` written at `slot`. */
static void loft_jv_into(loft_ref slot, const loft_jv *v) {
    switch (v->kind) {
    case LOFT_JV_NULL: loft_set_byte(slot, 0, LOFT_JD_NULL); break;
    case LOFT_JV_BOOL:
        loft_set_byte(slot, 0, LOFT_JD_BOOL);
        loft_set_byte(slot, LOFT_JV_BOOL_VALUE, (uint8_t) (v->b != 0));
        break;
    case LOFT_JV_NUMBER:
        loft_set_byte(slot, 0, LOFT_JD_NUMBER);
        loft_set_float(slot, LOFT_JV_NUMBER_VALUE, v->n);
        break;
    case LOFT_JV_INT:
        loft_set_byte(slot, 0, LOFT_JD_INT);
        loft_set_int(slot, LOFT_JV_INT_VALUE, v->i);
        break;
    case LOFT_JV_STR:
    case LOFT_JV_IDENT:
        loft_set_text(slot, LOFT_JV_STRING_VALUE, loft_view(&v->s));
        loft_set_byte(slot, 0, LOFT_JD_STRING);
        break;
    case LOFT_JV_ARRAY: {
        loft_set_byte(slot, 0, LOFT_JD_ARRAY);
        loft_ref items = slot;
        items.pos += LOFT_JV_ITEMS;
        loft_set_u32(loft_store_of(slot), slot.rec, items.pos, 0);
        for (size_t i = 0; i < v->nkids; i++) {
            loft_ref e = loft_vec_append(items, LOFT_JV_SIZE);
            loft_jv_into(e, &v->kids[i]);
            loft_vec_finish(items);
        }
        break;
    }
    case LOFT_JV_OBJECT: {
        loft_set_byte(slot, 0, LOFT_JD_OBJECT);
        loft_ref fields = slot;
        fields.pos += LOFT_JV_FIELDS;
        loft_set_u32(loft_store_of(slot), slot.rec, fields.pos, 0);
        for (size_t i = 0; i < v->nkids; i++) {
            loft_ref e = loft_vec_append(fields, LOFT_JF_SIZE);
            loft_set_text(e, LOFT_JF_NAME, loft_view(&v->keys[i]));
            loft_ref val = e;
            val.pos += LOFT_JF_VALUE;
            loft_jv_into(val, &v->kids[i]);
            loft_vec_finish(fields);
        }
        break;
    }
    case LOFT_JV_CTOR: {
        /* Lenient only: the single-member object `{tag: body}`. */
        loft_jv obj;
        memset(&obj, 0, sizeof obj);
        obj.kind = LOFT_JV_OBJECT;
        obj.nkids = 1;
        obj.kids = v->kids;
        obj.keys = (loft_text *) &v->s;
        loft_jv_into(slot, &obj);
        break;
    }
    }
}

/* `jv_alloc`: a JsonValue root in a store of its own, zeroed. */
static loft_ref loft_jv_alloc(void) {
    loft_ref r = loft_ref_null();
    uint32_t words = (LOFT_JV_SIZE + 7) / 8 + 1;
    if (words < 2) words = 2;
    loft_database(&r, (words - 1) * 8);
    return r;
}

/* `jv_null_sentinel`: the one JNull the readers answer for a miss, in a store nothing frees. */
static loft_ref loft_jv_null(void) {
    static LOFT_TLS loft_ref sentinel;
    static LOFT_TLS int made = 0;
    if (!made) {
        sentinel = loft_jv_alloc();
        loft_set_byte(sentinel, 0, LOFT_JD_NULL);
        loft_stores[sentinel.store_nr].pinned = 1;
        made = 1;
    }
    return sentinel;
}

/* `json::format_error(input, err, 2, 1)`: where, the path, the message, and the lines around
 * the fault with a caret under its column. */
static void loft_json_format_error(loft_text *out, const uint8_t *in, size_t len, const loft_jfault *f,
                                   loft_str path) {
    size_t cap = f->at < len ? f->at : len;
    size_t line = 1, col_start = 0;
    for (size_t i = 0; i < cap; i++)
        if (in[i] == '\n') { line++; col_start = i + 1; }
    size_t col = cap - col_start + 1;
    char head[160];
    int n = snprintf(head, sizeof head, "parse error at line %zu col %zu (byte %zu):\n  path: ", line, col, f->at);
    loft_push(out, head, (size_t) n);
    if (path.len) loft_text_append(out, path); else loft_push_cstr(out, "(root)");
    loft_push_cstr(out, "\n  ");
    loft_push_cstr(out, f->msg);
    loft_push_char(out, '\n');
    /* `str::lines`: split at '\n', a "\r\n" ending's '\r' dropped, no line after a final '\n'. */
    size_t nlines = 0;
    for (size_t i = 0; i < len; i++) if (in[i] == '\n') nlines++;
    if (len && in[len - 1] != '\n') nlines++;
    size_t lo = line > 3 ? line - 3 : 0;
    size_t hi = line + 1 < nlines ? line + 1 : nlines;
    int width = snprintf(0, 0, "%zu", hi);
    size_t at = 0;
    for (size_t idx = 0; idx < hi; idx++) {
        size_t e = at;
        while (e < len && in[e] != '\n') e++;
        if (idx >= lo) {
            size_t ce = e;
            if (e < len && ce > at && in[ce - 1] == '\r') ce--;
            n = snprintf(head, sizeof head, "    %*zu \xe2\x94\x82 ", width, idx + 1);
            loft_push(out, head, (size_t) n);
            loft_push(out, (const char *) in + at, ce - at);
            loft_push_char(out, '\n');
            if (idx + 1 == line) {
                n = snprintf(head, sizeof head, "    %*s \xe2\x94\x82 ", width, "");
                loft_push(out, head, (size_t) n);
                for (size_t c = 1; c < col; c++) loft_push_char(out, ' ');
                loft_push_cstr(out, "^\n");
            }
        }
        at = e + 1;
    }
}

/* `json_parse`: the document as a fresh JsonValue tree, or JNull with the error kept. */
static loft_ref loft_json_parse_value(loft_str raw) {
    loft_jv v;
    loft_jfault f;
    loft_text path = { 0, 0, 0 };
    int ok = loft_json_parse((const uint8_t *) raw.p, raw.len, 0, &v, &f, &path);
    loft_ref r = loft_jv_alloc();
    loft_jerr_clear();
    if (ok) {
        loft_jv_into(r, &v);
    } else {
        loft_set_byte(r, 0, LOFT_JD_NULL);
        loft_text e = { 0, 0, 0 };
        loft_json_format_error(&e, (const uint8_t *) raw.p, raw.len, &f, loft_view(&path));
        loft_jerr_take(e);
    }
    loft_jv_free(&v);
    loft_text_free(&path);
    return r;
}

static loft_text loft_json_errors(void) {
    loft_text t = { 0, 0, 0 };
    for (size_t i = 0; i < loft_jerr_n; i++) {
        if (i) loft_push_char(&t, '|');
        loft_text_append(&t, loft_view(&loft_jerr[i]));
    }
    return t;
}

/* The collection record of a container's items or members, 0 for none. */
static uint32_t loft_jv_kids(loft_ref v, uint8_t want) {
    if (loft_jd(v) != want) return 0;
    return loft_u32(loft_store_of(v), v.rec, v.pos + (want == LOFT_JD_ARRAY ? LOFT_JV_ITEMS : LOFT_JV_FIELDS));
}

static int64_t loft_jv_count(loft_ref v, uint32_t rec) {
    return rec == 0 ? 0 : (int64_t) loft_u32(loft_store_of(v), rec, 4);
}

/* `field`: the member named `name` of an object, in place; the JNull sentinel otherwise. */
static loft_ref loft_jv_field(loft_ref v, loft_str name) {
    uint32_t fr = loft_jv_kids(v, LOFT_JD_OBJECT);
    int64_t n = loft_jv_count(v, fr);
    loft_store_t *s = loft_store_of(v);
    for (int64_t i = 0; i < n; i++) {
        uint32_t at = 8 + (uint32_t) i * LOFT_JF_SIZE;
        if (loft_str_eq(loft_store_str(s, loft_u32(s, fr, at + LOFT_JF_NAME)), name)) {
            loft_ref r = v;
            r.rec = fr;
            r.pos = at + LOFT_JF_VALUE;
            return r;
        }
    }
    return loft_jv_null();
}

static uint8_t loft_jv_has_field(loft_ref v, loft_str name) {
    uint32_t fr = loft_jv_kids(v, LOFT_JD_OBJECT);
    int64_t n = loft_jv_count(v, fr);
    loft_store_t *s = loft_store_of(v);
    for (int64_t i = 0; i < n; i++)
        if (loft_str_eq(loft_store_str(s, loft_u32(s, fr, 8 + (uint32_t) i * LOFT_JF_SIZE + LOFT_JF_NAME)), name))
            return 1;
    return 0;
}

/* `item`: element `index` of an array, in place; the JNull sentinel otherwise. */
static loft_ref loft_jv_item(loft_ref v, int64_t index) {
    int32_t i = (int32_t) index;
    uint32_t ir = loft_jv_kids(v, LOFT_JD_ARRAY);
    if (i < 0 || i >= loft_jv_count(v, ir)) return loft_jv_null();
    loft_ref r = v;
    r.rec = ir;
    r.pos = 8 + (uint32_t) i * LOFT_JV_SIZE;
    return r;
}

/* `len`: an array's items or an object's members; null for anything else. */
static int64_t loft_jv_len(loft_ref v) {
    uint8_t d = loft_jd(v);
    if (d != LOFT_JD_ARRAY && d != LOFT_JD_OBJECT) return LOFT_NULL;
    return loft_jv_count(v, loft_jv_kids(v, d));
}

static loft_text loft_jv_as_text(loft_ref v) {
    if (loft_jd(v) != LOFT_JD_STRING) return loft_text_own(loft_null_text());
    return loft_text_own(loft_get_text(v, LOFT_JV_STRING_VALUE));
}

static double loft_jv_as_number(loft_ref v) {
    uint8_t d = loft_jd(v);
    if (d == LOFT_JD_NUMBER) return loft_get_float(v, LOFT_JV_NUMBER_VALUE);
    if (d == LOFT_JD_INT) return (double) loft_get_int(v, LOFT_JV_INT_VALUE);
    return NAN;
}

static int64_t loft_jv_as_long(loft_ref v) {
    uint8_t d = loft_jd(v);
    if (d == LOFT_JD_INT) return loft_get_int(v, LOFT_JV_INT_VALUE);
    if (d == LOFT_JD_NUMBER) {
        /* Rust's `as i64`: saturating, NaN to 0. */
        double x = trunc(loft_get_float(v, LOFT_JV_NUMBER_VALUE));
        if (x != x) return 0;
        if (x >= 9223372036854775807.0) return INT64_MAX;
        if (x <= -9223372036854775808.0) return INT64_MIN;
        return (int64_t) x;
    }
    return LOFT_NULL;
}

static uint8_t loft_jv_as_bool(loft_ref v) {
    if (loft_jd(v) != LOFT_JD_BOOL) return 255;
    return *loft_at(loft_store_of(v), v.rec, v.pos + LOFT_JV_BOOL_VALUE) != 0;
}

static loft_text loft_jv_kind(loft_ref v) {
    static const char *names[] = { "JUnknown", "JNull", "JBool", "JNumber", "JString", "JArray", "JObject", "JInteger" };
    uint8_t d = loft_jd(v);
    return loft_text_own(loft_lit(names[d <= 7 ? d : 0], strlen(names[d <= 7 ? d : 0])));
}

/* `keys`: an object's member names, in order, in a fresh vector; empty for anything else. */
static loft_ref loft_jv_keys(loft_ref v) {
    loft_ref vec = loft_ref_null();
    loft_database(&vec, LOFT_TEXT_SIZE * 8 - 8);
    loft_set_u32(loft_store_of(vec), vec.rec, vec.pos, 0);
    uint32_t fr = loft_jv_kids(v, LOFT_JD_OBJECT);
    int64_t n = loft_jv_count(v, fr);
    for (int64_t i = 0; i < n; i++) {
        loft_store_t *s = loft_store_of(v);
        loft_text name = loft_text_own(loft_store_str(s, loft_u32(s, fr, 8 + (uint32_t) i * LOFT_JF_SIZE + LOFT_JF_NAME)));
        loft_ref e = loft_vec_append(vec, LOFT_TEXT_SIZE);
        loft_set_text(e, 0, loft_view(&name));
        loft_vec_finish(vec);
        loft_text_free(&name);
    }
    return vec;
}

static void loft_jv_render(loft_text *out, loft_ref v, int pretty, int depth) {
    switch (loft_jd(v)) {
    case LOFT_JD_BOOL:
        loft_push_cstr(out, *loft_at(loft_store_of(v), v.rec, v.pos + LOFT_JV_BOOL_VALUE) ? "true" : "false");
        return;
    case LOFT_JD_NUMBER: {
        double n = loft_get_float(v, LOFT_JV_NUMBER_VALUE);
        if (isfinite(n)) loft_push_shortest(out, n, 0); else loft_push_cstr(out, "null");
        return;
    }
    case LOFT_JD_INT: loft_push_i64(out, loft_get_int(v, LOFT_JV_INT_VALUE)); return;
    case LOFT_JD_STRING: loft_push_json(out, loft_get_text(v, LOFT_JV_STRING_VALUE)); return;
    case LOFT_JD_ARRAY:
    case LOFT_JD_OBJECT: {
        int obj = loft_jd(v) == LOFT_JD_OBJECT;
        uint32_t kr = loft_jv_kids(v, loft_jd(v));
        int64_t n = loft_jv_count(v, kr);
        if (n <= 0) { loft_push_cstr(out, obj ? "{}" : "[]"); return; }
        loft_push_char(out, obj ? '{' : '[');
        for (int64_t i = 0; i < n; i++) {
            if (i > 0) loft_push_char(out, ',');
            if (pretty) loft_nl(out, depth + 1);
            loft_ref e = v;
            e.rec = kr;
            if (obj) {
                uint32_t at = 8 + (uint32_t) i * LOFT_JF_SIZE;
                loft_store_t *s = loft_store_of(v);
                loft_text name = loft_text_own(loft_store_str(s, loft_u32(s, kr, at + LOFT_JF_NAME)));
                loft_push_json(out, loft_view(&name));
                loft_text_free(&name);
                loft_push_char(out, ':');
                if (pretty) loft_push_char(out, ' ');
                e.pos = at + LOFT_JF_VALUE;
            } else {
                e.pos = 8 + (uint32_t) i * LOFT_JV_SIZE;
            }
            loft_jv_render(out, e, pretty, depth + 1);
        }
        if (pretty) loft_nl(out, depth);
        loft_push_char(out, obj ? '}' : ']');
        return;
    }
    default: loft_push_cstr(out, "null"); return;
    }
}

/* `dbref_to_parsed`: the tree at `src` read back into `v`, so a builder copies it whole into
 * its own store. */
static void loft_jv_read(loft_ref src, loft_jv *v) {
    memset(v, 0, sizeof *v);
    switch (loft_jd(src)) {
    case LOFT_JD_BOOL:
        v->kind = LOFT_JV_BOOL;
        v->b = *loft_at(loft_store_of(src), src.rec, src.pos + LOFT_JV_BOOL_VALUE) != 0;
        return;
    case LOFT_JD_NUMBER: v->kind = LOFT_JV_NUMBER; v->n = loft_get_float(src, LOFT_JV_NUMBER_VALUE); return;
    case LOFT_JD_INT: v->kind = LOFT_JV_INT; v->i = loft_get_int(src, LOFT_JV_INT_VALUE); return;
    case LOFT_JD_STRING:
        v->kind = LOFT_JV_STR;
        v->s = loft_text_own(loft_get_text(src, LOFT_JV_STRING_VALUE));
        return;
    case LOFT_JD_ARRAY:
    case LOFT_JD_OBJECT: {
        int obj = loft_jd(src) == LOFT_JD_OBJECT;
        v->kind = obj ? LOFT_JV_OBJECT : LOFT_JV_ARRAY;
        uint32_t kr = loft_jv_kids(src, loft_jd(src));
        int64_t n = loft_jv_count(src, kr);
        for (int64_t i = 0; i < n; i++) {
            loft_jv *k = loft_jv_push(v, obj);
            loft_ref e = src;
            e.rec = kr;
            if (obj) {
                uint32_t at = 8 + (uint32_t) i * LOFT_JF_SIZE;
                loft_store_t *s = loft_store_of(src);
                v->keys[v->nkids - 1] = loft_text_own(loft_store_str(s, loft_u32(s, kr, at + LOFT_JF_NAME)));
                e.pos = at + LOFT_JF_VALUE;
            } else {
                e.pos = 8 + (uint32_t) i * LOFT_JV_SIZE;
            }
            loft_jv_read(e, k);
        }
        return;
    }
    default: v->kind = LOFT_JV_NULL; return;
    }
}

/* The builders: a fresh JsonValue of their variant, the last error cleared. */
static loft_ref loft_jv_build(const loft_jv *v) {
    loft_ref r = loft_jv_alloc();
    loft_jv_into(r, v);
    loft_jerr_clear();
    return r;
}

static loft_ref loft_json_null(void) {
    loft_jv v;
    memset(&v, 0, sizeof v);
    return loft_jv_build(&v);
}

static loft_ref loft_json_bool(uint8_t b) {
    loft_jv v;
    memset(&v, 0, sizeof v);
    v.kind = LOFT_JV_BOOL;
    v.b = b;
    return loft_jv_build(&v);
}

/* `json_number`: a non-finite value is stored as JNull, and says so in `json_errors()`. */
static loft_ref loft_json_number(double n) {
    loft_jv v;
    memset(&v, 0, sizeof v);
    if (isfinite(n)) {
        v.kind = LOFT_JV_NUMBER;
        v.n = n;
        return loft_jv_build(&v);
    }
    loft_ref r = loft_jv_build(&v);
    loft_text e = { 0, 0, 0 };
    loft_push_cstr(&e, "json_number: non-finite value ");
    loft_push_cstr(&e, isnan(n) ? "NaN" : n < 0 ? "-inf" : "inf");
    loft_push_cstr(&e, " stored as JNull");
    loft_jerr_take(e);
    return r;
}

static loft_ref loft_json_string(loft_str t) {
    loft_jv v;
    memset(&v, 0, sizeof v);
    v.kind = LOFT_JV_STR;
    v.s = loft_text_own(t);
    loft_ref r = loft_jv_build(&v);
    loft_jv_free(&v);
    return r;
}

/* `json_array` / `json_object`: the elements of a vector of JsonValue or JsonField, copied
 * whole into the new value's store. */
static loft_ref loft_json_container(loft_ref vec, int obj) {
    loft_jv v;
    memset(&v, 0, sizeof v);
    v.kind = obj ? LOFT_JV_OBJECT : LOFT_JV_ARRAY;
    int64_t n = loft_vec_len(vec);
    if (n > 0) {
        loft_store_t *s = loft_store_of(vec);
        uint32_t kr = loft_collection_rec(s, vec.rec, vec.pos);
        for (int64_t i = 0; i < n; i++) {
            loft_jv *k = loft_jv_push(&v, obj);
            loft_ref e = vec;
            e.rec = kr;
            if (obj) {
                uint32_t at = 8 + (uint32_t) i * LOFT_JF_SIZE;
                v.keys[v.nkids - 1] = loft_text_own(loft_store_str(s, loft_u32(s, kr, at + LOFT_JF_NAME)));
                e.pos = at + LOFT_JF_VALUE;
            } else {
                e.pos = 8 + (uint32_t) i * LOFT_JV_SIZE;
            }
            loft_jv_read(e, k);
        }
    }
    loft_ref r = loft_jv_build(&v);
    loft_jv_free(&v);
    return r;
}

static loft_ref loft_json_array(loft_ref items) {
    return loft_json_container(items, 0);
}

static loft_ref loft_json_object(loft_ref fields) {
    return loft_json_container(fields, 1);
}

/* `fields`: an object's members, each value copied whole, in a fresh vector; empty for
 * anything else. */
static loft_ref loft_jv_fields(loft_ref v) {
    loft_ref vec = loft_ref_null();
    loft_database(&vec, LOFT_JF_SIZE * 8 - 8);
    loft_set_u32(loft_store_of(vec), vec.rec, vec.pos, 0);
    if (loft_jd(v) != LOFT_JD_OBJECT) return vec;
    loft_jv tree;
    loft_jv_read(v, &tree);
    for (size_t i = 0; i < tree.nkids; i++) {
        loft_ref e = loft_vec_append(vec, LOFT_JF_SIZE);
        loft_set_text(e, LOFT_JF_NAME, loft_view(&tree.keys[i]));
        loft_ref val = e;
        val.pos += LOFT_JF_VALUE;
        loft_jv_into(val, &tree.kids[i]);
        loft_vec_finish(vec);
    }
    loft_jv_free(&tree);
    return vec;
}

static loft_text loft_jv_to_json(loft_ref v) {
    loft_text t = { 0, 0, 0 };
    loft_jv_render(&t, v, 0, 0);
    return t;
}

static loft_text loft_jv_to_json_pretty(loft_ref v) {
    loft_text t = { 0, 0, 0 };
    loft_jv_render(&t, v, 1, 0);
    return t;
}
#endif
