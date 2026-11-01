/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
#ifdef LOFT_LEX
#ifndef LOFT_LEX_H
#define LOFT_LEX_H
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
/* ── loft source text as tokens (src/lexer.rs, `LexConfig::default()`) ────────────────────────
 * A PORT of the loft mode of loft's own lexer, for the front end written in loft (@PLN91 stage
 * 1): `loft_lex` answers every token of a whole source text with the position Rust gives it
 * (`line` from 1, `col` the character on the line from 1, a `"…"` literal's at its first content
 * character).  Ported one routine at a time, each named after the Rust it follows: `scan_once`,
 * `number` with `decimal`/`get_number`/`wide_suffix`/`ret_number`, `char`, `escape_seq`, `string`,
 * `string_nested`, `backtick_string` with its resume and dedent, `resume_string`,
 * `hole_closes_on_this_line`.  The lines are the ones `str::lines` gives a file (a `\r` before
 * the `\n` dropped, no line after a final `\n`), and a `#!` first line is skipped and counted.
 *
 * The one part that is not the lexer's: Rust's lexer reads a string's `{…}` holes in the modes
 * the PARSER sets (`Lexer::set_mode`, `Lexer::whitespace`; src/parser/objects.rs `parse_string`,
 * src/parser/vectors.rs).  Whitespace is a token in `Formatting` mode, a line end is one, and a
 * `}` read there resumes the string without being a token.  `loft_lx_after` replays the
 * parser's switches from the tokens alone: a hole's first token in the mode the scan left, the
 * expression as code up to its `:` or `}` at depth 0, a spec's fill and flags as text and its
 * width as code (pass 1's reading: every name that is not a radix letter is a width).
 *
 * A number followed by `..` or by a member `.` answers both tokens, as the Rust queues them: the
 * `..` at the number's place, the `.` at the place after it.  A scan Rust ends in no token (an
 * error after its diagnostic) is a `LOFT_LX_BAD` token at the place the scan began.  The
 * JSON mode of the same lexer is ported in loft_json.h (`loft_jl_*`), whose string and number
 * readers follow `JParser`'s stricter grammar, so nothing of it is shared here.  A unit that
 * lexes loft source defines `LOFT_LEX`. */

/* Token kinds: one per entry of src/lexer.rs's TOKENS (but `//`, the comment) in its order, the
 * space token of `Formatting` mode, one per KEYWORDS entry, then the literals. */
static const char *const loft_lx_spell[] = {
    ":", "::", ".", "..", ",", "{", "}", "(", ")", "[", "]", ";", "!", "!=", "+", "+=", "-", "-=",
    "*", "**", "*=", "/", "/=", "%", "%=", "=", "==", "<", "<=", ">", ">=", "&", "&&", "|", "||",
    "->", "=>", "^", "<<", ">>", "$", "#", "?", "??", "@", "~", " ",
    "as", "if", "in", "else", "for", "while", "continue", "break", "return", "yield", "true",
    "false", "null", "struct", "fn", "type", "enum", "interface", "pub", "and", "or", "use",
    "match", "sizeof", "debug_assert", "assert", "panic", "is", "operator"};
#define LOFT_LX_SPACE 46
#define LOFT_LX_KW0 47
#define LOFT_LX_IDENT 76
#define LOFT_LX_INT 77
#define LOFT_LX_LONG 78
#define LOFT_LX_FLOAT 79
#define LOFT_LX_SINGLE 80
#define LOFT_LX_WIDE 81
#define LOFT_LX_DOUBLE 82
#define LOFT_LX_STR 83
#define LOFT_LX_STRHOLE 84
#define LOFT_LX_CHAR 85
#define LOFT_LX_LINEEND 86
#define LOFT_LX_BAD 87
#define LOFT_LX_END 88
/* A few punctuation kinds the hole emulation asks about. */
#define LOFT_LX_COLON 0
#define LOFT_LX_DOT 2
#define LOFT_LX_LBRACE 5
#define LOFT_LX_RBRACE 6
#define LOFT_LX_LPAREN 7
#define LOFT_LX_RPAREN 8
#define LOFT_LX_LBRACKET 9
#define LOFT_LX_RBRACKET 10

typedef struct {
    uint8_t kind;
    uint8_t zero;   /* an Int's or Float's spelling began with `0` */
    uint8_t fmt;    /* a string put while the lexer was in `Formatting` mode (the trace's `str+`) */
    uint32_t line, col;
    int64_t i;      /* Int, Long, Wide, Char */
    double f;       /* Float, Double, Single (exactly the `f32`) */
    size_t s_at, s_len; /* Ident, Str, StrHole: the text in `loft_lx_toks.txt` */
} loft_lx_tok;

typedef struct {
    uint8_t *p;
    size_t len, cap;
} loft_lx_buf;

typedef struct {
    loft_lx_tok *t;
    size_t n, cap;
    loft_lx_buf txt;
} loft_lx_toks;

static void loft_lx_buf_put(loft_lx_buf *b, const void *p, size_t n) {
    if (b->len + n > b->cap) {
        size_t c = b->cap ? b->cap * 2 : 64;
        while (c < b->len + n) c *= 2;
        b->p = (uint8_t *) realloc(b->p, c);
        if (!b->p) { fputs("loft: out of memory\n", stderr); abort(); }
        b->cap = c;
    }
    if (n) memcpy(b->p + b->len, p, n);
    b->len += n;
}

static void loft_lx_buf_byte(loft_lx_buf *b, uint8_t c) { loft_lx_buf_put(b, &c, 1); }

static void loft_lx_buf_cp(loft_lx_buf *b, uint32_t cp) {
    uint8_t o[4];
    size_t n;
    if (cp < 0x80) { o[0] = (uint8_t) cp; n = 1; }
    else if (cp < 0x800) { o[0] = (uint8_t) (0xC0 | (cp >> 6)); o[1] = (uint8_t) (0x80 | (cp & 0x3F)); n = 2; }
    else if (cp < 0x10000) {
        o[0] = (uint8_t) (0xE0 | (cp >> 12)); o[1] = (uint8_t) (0x80 | ((cp >> 6) & 0x3F));
        o[2] = (uint8_t) (0x80 | (cp & 0x3F)); n = 3;
    } else {
        o[0] = (uint8_t) (0xF0 | (cp >> 18)); o[1] = (uint8_t) (0x80 | ((cp >> 12) & 0x3F));
        o[2] = (uint8_t) (0x80 | ((cp >> 6) & 0x3F)); o[3] = (uint8_t) (0x80 | (cp & 0x3F)); n = 4;
    }
    loft_lx_buf_put(b, o, n);
}

static void loft_lx_toks_free(loft_lx_toks *o) {
    free(o->t);
    free(o->txt.p);
    memset(o, 0, sizeof *o);
}

/* `Mode` and `StrKind`. */
#define LOFT_LX_CODE 0
#define LOFT_LX_FMT 1
#define LOFT_LX_SK_PLAIN 0
#define LOFT_LX_SK_BACKTICK 1
#define LOFT_LX_SK_NESTED 2

/* The parser's reading of one open hole (`loft_lx_after`). */
#define LOFT_LX_H_FIRST 0
#define LOFT_LX_H_EXPR 1
#define LOFT_LX_H_SPEC 2
#define LOFT_LX_H_FLAGS 3
#define LOFT_LX_H_WIDTH 4
#define LOFT_LX_H_AFTER 5

typedef struct {
    uint8_t kind, flag; /* StrKind and its bool */
    int phase, depth, operand;
} loft_lx_hole;

typedef struct {
    const uint8_t *b;
    size_t len;
    /* The source lines, as `str::lines` splits them. */
    size_t *ls, *le;
    size_t nlines, next_line;
    /* The cursor: the line being read [cur_from, cur_end) and the byte offset in it. */
    size_t at, cur_end;
    uint32_t line, pos;
    int mode, in_format_expr;
    /* `open_strings` and the parser's reading of each, innermost last. */
    loft_lx_hole *holes;
    size_t nholes, hcap;
    /* `backtick_strip`: -1 for `None`. */
    long *strip;
    size_t nstrip, scap;
    int popped; /* the scan resumed a string, closing the innermost hole */
    loft_lx_toks *out;
} loft_lx;

/* ── The cursor (`LineCursor`, `next_char`) ─────────────────────────────────────────────── */

/* The character at the cursor, -1 at the end of the line. */
static int32_t loft_lx_cp_at(const loft_lx *x, size_t at, size_t *n) {
    if (at >= x->cur_end) { *n = 0; return -1; }
    const uint8_t *s = x->b + at;
    uint8_t c = s[0];
    size_t k = c < 0x80 ? 1 : c >= 0xF0 ? 4 : c >= 0xE0 ? 3 : c >= 0xC0 ? 2 : 1;
    if (at + k > x->cur_end) k = x->cur_end - at;
    uint32_t cp = k == 1 ? c : k == 2 ? (uint32_t) (c & 0x1F) : k == 3 ? (uint32_t) (c & 0x0F) : (uint32_t) (c & 0x07);
    for (size_t i = 1; i < k; i++) cp = (cp << 6) | (s[i] & 0x3F);
    *n = k;
    return (int32_t) cp;
}

static int32_t loft_lx_peek(const loft_lx *x) {
    size_t n;
    return loft_lx_cp_at(x, x->at, &n);
}

/* The character after the one at the cursor (`rest().nth(1)`). */
static int32_t loft_lx_peek2(const loft_lx *x) {
    size_t n;
    if (loft_lx_cp_at(x, x->at, &n) < 0) return -1;
    return loft_lx_cp_at(x, x->at + n, &n);
}

/* `next_char`: a step past the character, and the position counts one even at the line end. */
static void loft_lx_next(loft_lx *x) {
    size_t n;
    if (loft_lx_cp_at(x, x->at, &n) >= 0) x->at += n;
    x->pos++;
}

/* The cursor's character copied onto `b` as written. */
static void loft_lx_take(loft_lx *x, loft_lx_buf *b) {
    size_t n;
    if (loft_lx_cp_at(x, x->at, &n) >= 0) loft_lx_buf_put(b, x->b + x->at, n);
}

static int loft_lx_more_lines(const loft_lx *x) { return x->next_line < x->nlines; }

/* `enter_line` of the next source line. */
static void loft_lx_enter(loft_lx *x) {
    x->at = x->ls[x->next_line];
    x->cur_end = x->le[x->next_line];
    x->next_line++;
}

/* `advance_line`: the next line inside a backtick literal; 0 at the end of the text. */
static int loft_lx_advance_line(loft_lx *x) {
    if (!loft_lx_more_lines(x)) return 0;
    loft_lx_enter(x);
    x->line++;
    x->pos = 1;
    return 1;
}

static int loft_lx_ascii_alnum(int32_t c) {
    return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
}

static int loft_lx_ascii_alpha(int32_t c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'); }

static int loft_lx_hexval(int32_t c) {
    return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
}

/* `char::is_whitespace`. */
static int loft_lx_space(uint32_t c) {
    return (c >= 9 && c <= 13) || c == ' ' || c == 0x85 || c == 0xA0 || c == 0x1680 ||
           (c >= 0x2000 && c <= 0x200A) || c == 0x2028 || c == 0x2029 || c == 0x202F || c == 0x205F ||
           c == 0x3000;
}

/* `char::is_alphabetic`, for the character after a number's `.`: an ASCII letter, or a letter of
 * another script (approximated by every character past Latin-1's punctuation). */
static int loft_lx_alphabetic(int32_t c) {
    return loft_lx_ascii_alpha(c) || c == 0xAA || c == 0xB5 || c == 0xBA || (c >= 0xC0 && c != 0xD7 && c != 0xF7);
}

/* `str::trim().is_empty()` over bytes [p, p+n). */
static int loft_lx_blank(const uint8_t *p, size_t n) {
    size_t i = 0;
    while (i < n) {
        uint8_t c = p[i];
        size_t k = c < 0x80 ? 1 : c >= 0xF0 ? 4 : c >= 0xE0 ? 3 : c >= 0xC0 ? 2 : 1;
        if (i + k > n) k = n - i;
        uint32_t cp = k == 1 ? c : k == 2 ? (uint32_t) (c & 0x1F) : k == 3 ? (uint32_t) (c & 0x0F) : (uint32_t) (c & 0x07);
        for (size_t j = 1; j < k; j++) cp = (cp << 6) | (p[i + j] & 0x3F);
        if (!loft_lx_space(cp)) return 0;
        i += k;
    }
    return 1;
}

/* ── Putting a token (`put`, `put_string`, `put_identifier`, `put_none`) ─────────────────── */

static loft_lx_tok *loft_lx_put(loft_lx *x, int kind, uint32_t line, uint32_t col) {
    loft_lx_toks *o = x->out;
    if (o->n == o->cap) {
        o->cap = o->cap ? o->cap * 2 : 256;
        o->t = (loft_lx_tok *) realloc(o->t, o->cap * sizeof *o->t);
        if (!o->t) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_lx_tok *t = &o->t[o->n++];
    memset(t, 0, sizeof *t);
    t->kind = (uint8_t) kind;
    t->line = line;
    t->col = col;
    t->fmt = x->mode == LOFT_LX_FMT;
    return t;
}

static void loft_lx_put_text(loft_lx *x, int kind, const uint8_t *p, size_t n, uint32_t line, uint32_t col) {
    loft_lx_tok *t = loft_lx_put(x, kind, line, col);
    t->s_at = x->out->txt.len;
    t->s_len = n;
    loft_lx_buf_put(&x->out->txt, p, n);
}

/* `put_string`: a string closes in `Code` mode and opens a hole in `Formatting`; the kind says
 * which the scan did, `fmt` what the mode then was. */
static void loft_lx_put_string(loft_lx *x, loft_lx_buf *res, int hole, uint32_t line, uint32_t col) {
    loft_lx_put_text(x, hole ? LOFT_LX_STRHOLE : LOFT_LX_STR, res->p, res->len, line, col);
    free(res->p);
    memset(res, 0, sizeof *res);
}

static void loft_lx_put_none(loft_lx *x, uint32_t line, uint32_t col) {
    loft_lx_put(x, LOFT_LX_BAD, line, col);
}

/* A string opened a hole: `open_strings.push(kind)`, and the parser's reading of it starts. */
static void loft_lx_open_hole(loft_lx *x, int kind, int flag) {
    x->mode = LOFT_LX_FMT;
    x->in_format_expr = 1;
    if (x->nholes == x->hcap) {
        x->hcap = x->hcap ? x->hcap * 2 : 8;
        x->holes = (loft_lx_hole *) realloc(x->holes, x->hcap * sizeof *x->holes);
        if (!x->holes) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_lx_hole *h = &x->holes[x->nholes++];
    memset(h, 0, sizeof *h);
    h->kind = (uint8_t) kind;
    h->flag = (uint8_t) flag;
    h->phase = LOFT_LX_H_FIRST;
}

static void loft_lx_strip_push(loft_lx *x) {
    if (x->nstrip == x->scap) {
        x->scap = x->scap ? x->scap * 2 : 8;
        x->strip = (long *) realloc(x->strip, x->scap * sizeof *x->strip);
        if (!x->strip) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    x->strip[x->nstrip++] = -1;
}

static void loft_lx_strip_pop(loft_lx *x) {
    if (x->nstrip) x->nstrip--;
}

/* ── Escapes and characters ─────────────────────────────────────────────────────────────── */

/* `escape_seq`: ONE designator character at the cursor, the caller steps past the last one.
 * 0 at the end of the line. */
static int loft_lx_escape_seq(loft_lx *x, loft_lx_buf *res) {
    int32_t c = loft_lx_peek(x);
    if (c < 0) return 0;
    switch (c) {
    case '"': case '\'': case '\\': case '`': loft_lx_buf_byte(res, (uint8_t) c); break;
    case 't': loft_lx_buf_byte(res, '\t'); break;
    case 'r': loft_lx_buf_byte(res, '\r'); break;
    case 'n': case '\n': loft_lx_buf_byte(res, '\n'); break;
    case '0': loft_lx_buf_byte(res, 0); break;
    case 'x': {
        loft_lx_next(x);
        int32_t h1 = loft_lx_peek(x), h2 = -1;
        if (h1 >= 0) {
            loft_lx_next(x);
            h2 = loft_lx_peek(x);
        }
        if (h1 >= 0 && h2 >= 0 && loft_lx_hexval(h1) >= 0 && loft_lx_hexval(h2) >= 0) {
            int byte = (loft_lx_hexval(h1) << 4) | loft_lx_hexval(h2);
            loft_lx_buf_byte(res, byte < 0x80 ? (uint8_t) byte : '?');
        } else {
            loft_lx_buf_byte(res, '?');
        }
        break;
    }
    case 'u': {
        loft_lx_next(x);
        if (loft_lx_peek(x) != '{') { loft_lx_buf_byte(res, '?'); return 1; }
        loft_lx_next(x);
        uint32_t cp = 0;
        size_t digits = 0;
        int32_t h;
        while ((h = loft_lx_peek(x)) >= 0 && loft_lx_hexval(h) >= 0) {
            if (digits < 8) cp = (cp << 4) | (uint32_t) loft_lx_hexval(h);
            digits++;
            loft_lx_next(x);
        }
        if (loft_lx_peek(x) != '}' || digits == 0 || digits > 6) { loft_lx_buf_byte(res, '?'); return 1; }
        if (cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF)) loft_lx_buf_byte(res, '?');
        else loft_lx_buf_cp(res, cp);
        break;
    }
    default: loft_lx_buf_byte(res, '?');
    }
    return 1;
}

/* `char`: the cursor is past the opening `'`. */
static void loft_lx_char(loft_lx *x) {
    uint32_t line = x->line, col = x->pos;
    loft_lx_buf res = {0};
    int32_t c;
    while ((c = loft_lx_peek(x)) >= 0) {
        if (c == '\'') {
            loft_lx_next(x);
            int64_t code = 0;
            if (res.len) {
                size_t n;
                loft_lx tmp = *x;
                tmp.b = res.p;
                tmp.cur_end = res.len;
                code = loft_lx_cp_at(&tmp, 0, &n);
            }
            free(res.p);
            loft_lx_tok *t = loft_lx_put(x, LOFT_LX_CHAR, line, col);
            t->i = code;
            return;
        }
        if (c == '\\') {
            loft_lx_next(x);
            if (!loft_lx_escape_seq(x, &res)) break;
        } else {
            loft_lx_take(x, &res);
        }
        loft_lx_next(x);
    }
    free(res.p);
    loft_lx_put_none(x, line, col);
}

/* `hole_closes_on_this_line`: can the hole opened by the `{` just consumed close on this line? */
static int loft_lx_hole_closes(const loft_lx *x) {
    enum { C_CODE, C_STR, C_BACKTICK, C_CHAR };
    uint8_t stack[256];
    size_t sp = 0, deep = 0; /* frames beyond the array still count */
    stack[sp++] = C_CODE;
    size_t at = x->at, n;
    int32_t c;
#define LOFT_LX_TOP() (deep ? (int) C_CODE : (int) stack[sp - 1])
    while ((c = loft_lx_cp_at(x, at, &n)) >= 0) {
        at += n;
        if (c == '\\') {
            if (loft_lx_cp_at(x, at, &n) >= 0) at += n;
            continue;
        }
        int top = sp ? LOFT_LX_TOP() : C_CODE;
        size_t m;
        int32_t nx = loft_lx_cp_at(x, at, &m);
        if (top == C_CODE) {
            if (c == '/' && nx == '/') return 1;
            if (c == '{') { if (sp < 256) stack[sp++] = C_CODE; else deep++; }
            else if (c == '}') {
                if (deep) deep--; else if (sp) sp--;
                if (!sp && !deep) return 1;
            } else if (c == '"') { if (sp < 256) stack[sp++] = C_STR; else deep++; }
            else if (c == '`') { if (sp < 256) stack[sp++] = C_BACKTICK; else deep++; }
            else if (c == '\'') { if (sp < 256) stack[sp++] = C_CHAR; else deep++; }
        } else if (top == C_STR || top == C_BACKTICK) {
            if (c == '"' && top == C_STR) sp--;
            else if (c == '`' && top == C_BACKTICK) sp--;
            else if ((c == '{' || c == '}') && nx == c) at += m;
            else if (c == '{') { if (sp < 256) stack[sp++] = C_CODE; else deep++; }
        } else if (c == '\'') {
            sp--;
        }
    }
#undef LOFT_LX_TOP
    return 0;
}

/* ── Strings ────────────────────────────────────────────────────────────────────────────── */

/* `string`: the cursor is past the opening `"` (or the `}` that resumed it). */
static void loft_lx_string(loft_lx *x, uint32_t line, uint32_t col) {
    loft_lx_buf res = {0};
    for (;;) {
        /* `take_run`: up to a quote, a backslash, a control character or a brace. */
        size_t from = x->at;
        uint32_t chars = 0;
        while (x->at < x->cur_end) {
            uint8_t c = x->b[x->at];
            if (c == '"' || c == '\\' || c < 0x20 || c == '{' || c == '}') break;
            if ((c & 0xC0) != 0x80) chars++;
            x->at++;
        }
        if (x->at > from) {
            loft_lx_buf_put(&res, x->b + from, x->at - from);
            x->pos += chars;
        }
        int32_t c = loft_lx_peek(x);
        if (c < 0) break;
        if (c == '"') {
            x->mode = LOFT_LX_CODE;
            loft_lx_next(x);
            loft_lx_put_string(x, &res, 0, line, col);
            return;
        }
        if (c == '\\') {
            loft_lx_next(x);
            if (!loft_lx_escape_seq(x, &res)) break;
        } else if (c == '{') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '{') {
                loft_lx_buf_byte(&res, '{');
            } else if (!loft_lx_hole_closes(x)) {
                loft_lx_buf_byte(&res, '{');
                continue;
            } else {
                loft_lx_open_hole(x, LOFT_LX_SK_PLAIN, 0);
                loft_lx_put_string(x, &res, 1, line, col);
                return;
            }
        } else if (c == '}') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '}') loft_lx_buf_byte(&res, '}');
        } else {
            loft_lx_take(x, &res);
        }
        loft_lx_next(x);
    }
    free(res.p);
    loft_lx_put_none(x, line, col);
}

/* `close_nested`. */
static void loft_lx_close_nested(loft_lx *x, loft_lx_buf *res, int resumed, uint32_t line, uint32_t col) {
    if (resumed) x->mode = LOFT_LX_CODE;
    x->in_format_expr = 1;
    loft_lx_put_string(x, res, 0, line, col);
}

/* `close_backtick`. */
static void loft_lx_close_backtick(loft_lx *x, loft_lx_buf *res, int nested, int resumed, uint32_t line, uint32_t col) {
    loft_lx_strip_pop(x);
    if (nested) {
        loft_lx_close_nested(x, res, resumed, line, col);
        return;
    }
    x->mode = LOFT_LX_CODE;
    loft_lx_put_string(x, res, 0, line, col);
}

/* `string_nested`: a literal inside a `{…}` hole; the cursor is past its opening quote. */
static void loft_lx_string_nested(loft_lx *x, int escaped_delim, int resumed) {
    uint32_t line = x->line, col = x->pos;
    loft_lx_buf res = {0};
    int32_t c;
    while ((c = loft_lx_peek(x)) >= 0) {
        if (c == '"') {
            loft_lx_next(x);
            loft_lx_close_nested(x, &res, resumed, line, col);
            return;
        }
        if (c == '\\') {
            loft_lx_next(x);
            if (escaped_delim && loft_lx_peek(x) == '"') {
                loft_lx_next(x);
                loft_lx_close_nested(x, &res, resumed, line, col);
                return;
            }
            if (!loft_lx_escape_seq(x, &res)) break;
        } else if (c == '{') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '{') {
                loft_lx_buf_byte(&res, '{');
            } else if (!loft_lx_hole_closes(x)) {
                loft_lx_buf_byte(&res, '{');
                continue;
            } else {
                loft_lx_open_hole(x, LOFT_LX_SK_NESTED, escaped_delim);
                loft_lx_put_string(x, &res, 1, line, col);
                return;
            }
        } else if (c == '}') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '}') loft_lx_buf_byte(&res, '}');
        } else {
            loft_lx_take(x, &res);
        }
        loft_lx_next(x);
    }
    free(res.p);
    loft_lx_put_none(x, line, col);
}

/* `backtick_line_start`: the leading spaces of the line just entered, less the literal's dedent. */
static void loft_lx_backtick_line_start(loft_lx *x, loft_lx_buf *cur) {
    size_t spaces = 0;
    while (loft_lx_peek(x) == ' ') {
        spaces++;
        loft_lx_next(x);
    }
    int32_t p = loft_lx_peek(x);
    int settles = !(p < 0 || p == '`');
    size_t strip = 0;
    if (x->nstrip) {
        long *slot = &x->strip[x->nstrip - 1];
        if (settles && *slot < 0) *slot = (long) spaces;
        strip = *slot < 0 ? 0 : (size_t) *slot;
    }
    for (size_t i = strip; i < spaces; i++) loft_lx_buf_byte(cur, ' ');
}

/* The lines of a backtick literal's first segment joined: `skip_last_blank` for a closed
 * literal, whose closing line is layout. */
static void loft_lx_join(loft_lx_buf *lines, size_t n, int closed, loft_lx_buf *result) {
    for (size_t i = 0; i < n; i++) {
        int blank = loft_lx_blank(lines[i].p, lines[i].len);
        if (i == 0) {
            if (closed) {
                if (!blank) loft_lx_buf_put(result, lines[i].p, lines[i].len);
                continue;
            }
            if (blank) continue;
        } else if (closed && i == n - 1 && blank) {
            continue;
        }
        if (result->len) loft_lx_buf_byte(result, '\n');
        loft_lx_buf_put(result, lines[i].p, lines[i].len);
    }
}

typedef struct {
    loft_lx_buf *v;
    size_t n, cap;
} loft_lx_lines;

static void loft_lx_lines_push(loft_lx_lines *l, loft_lx_buf *cur) {
    if (l->n == l->cap) {
        l->cap = l->cap ? l->cap * 2 : 8;
        l->v = (loft_lx_buf *) realloc(l->v, l->cap * sizeof *l->v);
        if (!l->v) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    l->v[l->n++] = *cur;
    memset(cur, 0, sizeof *cur);
}

static void loft_lx_lines_free(loft_lx_lines *l) {
    for (size_t i = 0; i < l->n; i++) free(l->v[i].p);
    free(l->v);
}

/* `backtick_string`: the cursor is past the opening backtick. */
static void loft_lx_backtick_string(loft_lx *x, int nested) {
    uint32_t line = x->line, col = x->pos;
    loft_lx_lines lines = {0};
    loft_lx_buf cur = {0};
    for (;;) {
        int32_t c = loft_lx_peek(x);
        if (c == '`') {
            loft_lx_next(x);
            loft_lx_lines_push(&lines, &cur);
            loft_lx_buf result = {0};
            loft_lx_join(lines.v, lines.n, 1, &result);
            loft_lx_lines_free(&lines);
            loft_lx_close_backtick(x, &result, nested, 0, line, col);
            return;
        } else if (c == '{') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '{') {
                loft_lx_buf_byte(&cur, '{');
            } else if (!loft_lx_hole_closes(x)) {
                loft_lx_buf_byte(&cur, '{');
                continue;
            } else {
                loft_lx_lines_push(&lines, &cur);
                loft_lx_buf result = {0};
                loft_lx_join(lines.v, lines.n, 0, &result);
                loft_lx_lines_free(&lines);
                loft_lx_open_hole(x, LOFT_LX_SK_BACKTICK, nested);
                loft_lx_put_string(x, &result, 1, line, col);
                return;
            }
        } else if (c == '}') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '}') loft_lx_buf_byte(&cur, '}');
        } else if (c == '\\') {
            loft_lx_next(x);
            if (!loft_lx_escape_seq(x, &cur)) {
                loft_lx_strip_pop(x);
                free(cur.p);
                loft_lx_lines_free(&lines);
                loft_lx_put_none(x, line, col);
                return;
            }
        } else if (c >= 0) {
            loft_lx_take(x, &cur);
        } else {
            loft_lx_lines_push(&lines, &cur);
            if (!loft_lx_advance_line(x)) {
                loft_lx_strip_pop(x);
                loft_lx_lines_free(&lines);
                loft_lx_put_none(x, line, col);
                return;
            }
            loft_lx_backtick_line_start(x, &cur);
            int32_t d;
            while ((d = loft_lx_peek(x)) >= 0 && d != '`' && d != '{' && d != '}' && d != '\\') {
                loft_lx_take(x, &cur);
                loft_lx_next(x);
            }
            continue;
        }
        loft_lx_next(x);
    }
}

/* `drop_trailing_blank_line`. */
static void loft_lx_drop_trailing_blank(loft_lx_buf *cur) {
    size_t i = cur->len;
    while (i > 0 && cur->p[i - 1] != '\n') i--;
    if (i == 0) return;
    for (size_t j = i; j < cur->len; j++) {
        if (cur->p[j] != ' ' && cur->p[j] != '\t') return;
    }
    cur->len = i - 1;
}

/* `backtick_string_resume`: after the `}` that closed a hole of a backtick literal. */
static void loft_lx_backtick_resume(loft_lx *x, int nested) {
    uint32_t line = x->line, col = x->pos;
    loft_lx_buf cur = {0};
    for (;;) {
        int32_t c = loft_lx_peek(x);
        if (c == '`') {
            loft_lx_next(x);
            loft_lx_drop_trailing_blank(&cur);
            loft_lx_close_backtick(x, &cur, nested, 1, line, col);
            return;
        } else if (c == '{') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '{') {
                loft_lx_buf_byte(&cur, '{');
            } else if (!loft_lx_hole_closes(x)) {
                loft_lx_buf_byte(&cur, '{');
                continue;
            } else {
                loft_lx_open_hole(x, LOFT_LX_SK_BACKTICK, nested);
                loft_lx_put_string(x, &cur, 1, line, col);
                return;
            }
        } else if (c == '}') {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '}') loft_lx_buf_byte(&cur, '}');
        } else if (c == '\\') {
            loft_lx_next(x);
            if (!loft_lx_escape_seq(x, &cur)) {
                loft_lx_strip_pop(x);
                free(cur.p);
                loft_lx_put_none(x, line, col);
                return;
            }
        } else if (c >= 0) {
            loft_lx_take(x, &cur);
        } else {
            loft_lx_buf_byte(&cur, '\n');
            if (!loft_lx_advance_line(x)) {
                loft_lx_strip_pop(x);
                free(cur.p);
                loft_lx_put_none(x, line, col);
                return;
            }
            loft_lx_backtick_line_start(x, &cur);
            continue;
        }
        loft_lx_next(x);
    }
}

/* `resume_string`: the `}` that closed a hole resumes the string that opened it. */
static void loft_lx_resume_string(loft_lx *x) {
    x->in_format_expr = 0;
    if (!x->nholes) {
        loft_lx_string(x, x->line, x->pos);
        return;
    }
    loft_lx_hole h = x->holes[--x->nholes];
    x->popped = 1;
    if (h.kind == LOFT_LX_SK_BACKTICK) loft_lx_backtick_resume(x, h.flag);
    else if (h.kind == LOFT_LX_SK_NESTED) loft_lx_string_nested(x, h.flag, 1);
    else loft_lx_string(x, x->line, x->pos);
}

/* ── Numbers ────────────────────────────────────────────────────────────────────────────── */

/* `ret_number`. */
static void loft_lx_ret_number(loft_lx *x, uint64_t r, uint32_t line, uint32_t col, int start_zero) {
    loft_lx_tok *t;
    if (r > (uint64_t) INT64_MAX) {
        t = loft_lx_put(x, LOFT_LX_INT, line, col);
        t->zero = (uint8_t) start_zero;
    } else if (r > (uint64_t) INT32_MAX) {
        t = loft_lx_put(x, LOFT_LX_LONG, line, col);
        t->i = (int64_t) r;
    } else {
        t = loft_lx_put(x, LOFT_LX_INT, line, col);
        t->i = (int64_t) r;
        t->zero = (uint8_t) start_zero;
    }
}

/* `val.parse::<u64>()` of a number's digit text. */
static int loft_lx_parse_u64(const char *v, uint64_t *r) {
    uint64_t a = 0;
    if (!*v) return 0;
    for (; *v; v++) {
        if (*v < '0' || *v > '9') return 0;
        uint64_t d = (uint64_t) (*v - '0');
        if (a > (UINT64_MAX - d) / 10) return 0;
        a = a * 10 + d;
    }
    *r = a;
    return 1;
}

/* `wide_suffix`: an `l` after an integer literal, not starting a longer word. */
static int loft_lx_wide_suffix(loft_lx *x, uint64_t r, uint32_t line, uint32_t col) {
    if (loft_lx_peek(x) != 'l') return 0;
    int32_t n = loft_lx_peek2(x);
    if (loft_lx_ascii_alnum(n) || n == '_') return 0;
    loft_lx_next(x);
    loft_lx_tok *t = loft_lx_put(x, LOFT_LX_WIDE, line, col);
    t->i = r > (uint64_t) INT64_MAX ? 0 : (int64_t) r;
    return 1;
}

typedef struct {
    char *p;
    size_t len, cap;
} loft_lx_str;

static void loft_lx_str_push(loft_lx_str *s, char c) {
    if (s->len + 2 > s->cap) {
        s->cap = s->cap ? s->cap * 2 : 32;
        s->p = (char *) realloc(s->p, s->cap);
        if (!s->p) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    s->p[s->len++] = c;
    s->p[s->len] = 0;
}

static void loft_lx_str_cat(loft_lx_str *s, const loft_lx_str *t) {
    for (size_t i = 0; i < t->len; i++) loft_lx_str_push(s, t->p[i]);
}

/* `get_number`: the digits (separators dropped), and whether more than one group was cut. */
static void loft_lx_get_number(loft_lx *x, loft_lx_str *number) {
    int hex = 0, prev_digit = 0;
    int32_t c;
    loft_lx_str_push(number, 0);
    number->len = 0;
    while ((c = loft_lx_peek(x)) >= 0) {
        if (c >= '0' && c <= '9') {
            loft_lx_str_push(number, (char) c);
            loft_lx_next(x);
            prev_digit = 1;
        } else if ((c == 'x' || c == 'b' || c == 'o') && !hex && number->len == 1 && number->p[0] == '0') {
            hex = c == 'x';
            loft_lx_str_push(number, (char) c);
            loft_lx_next(x);
            prev_digit = 0;
        } else if (hex && loft_lx_hexval(c) >= 0) {
            loft_lx_str_push(number, (char) c);
            loft_lx_next(x);
            prev_digit = 1;
        } else if (c == '_') {
            loft_lx_next(x);
            int32_t n = loft_lx_peek(x);
            int next_digit = (n >= '0' && n <= '9') || (hex && loft_lx_hexval(n) >= 0);
            if (!(prev_digit && next_digit)) {
                while (loft_lx_peek(x) == '_') loft_lx_next(x);
            }
            prev_digit = 0;
        } else {
            break;
        }
    }
}

/* `hex_parse`, `bin_parse`, `oct_parse`: wrapping, as the Rust's release build computes them. */
static int loft_lx_radix_parse(const char *v, unsigned radix, uint64_t *r) {
    uint64_t a = 0;
    for (; *v; v++) {
        int d = loft_lx_hexval((unsigned char) *v);
        if (d < 0 || (unsigned) d >= radix) return 0;
        a = a * radix + (uint64_t) d;
    }
    *r = a;
    return 1;
}

/* `val.parse::<f64>()` over the forms a number's text takes here: digits, a `.`, an exponent. */
static int loft_lx_float_text(const char *v) {
    int digits = 0;
    for (const char *p = v; *p; p++) {
        if (*p >= '0' && *p <= '9') digits++;
        else if (*p != '.' && *p != 'e' && *p != '+' && *p != '-') return 0;
    }
    return digits > 0;
}

/* `decimal`: a number of decimal digits alone, read without building its text. */
static int loft_lx_decimal(loft_lx *x, uint32_t line, uint32_t col) {
    size_t i = x->at;
    while (i < x->cur_end && x->b[i] >= '0' && x->b[i] <= '9') i++;
    if (i < x->cur_end) {
        uint8_t a = x->b[i];
        if (a == '.' || a == '_' || (a >= 'a' && a <= 'z') || (a >= 'A' && a <= 'Z')) return 0;
    }
    size_t len = i - x->at;
    if (!len) return 0;
    int start_zero = x->b[x->at] == '0';
    uint64_t v = 0;
    for (size_t k = x->at; k < i; k++) {
        uint64_t d = (uint64_t) (x->b[k] - '0');
        if (v > (UINT64_MAX - d) / 10) return 0;
        v = v * 10 + d;
    }
    if (v > (uint64_t) INT64_MAX) return 0;
    x->at = i;
    x->pos += (uint32_t) len;
    loft_lx_ret_number(x, v, line, col, start_zero);
    return 1;
}

/* The previous token put: `self.peek` while the next one is scanned. */
static int loft_lx_prev_is(const loft_lx *x, int kind) {
    return x->out->n && x->out->t[x->out->n - 1].kind == kind;
}

/* `number`. */
static void loft_lx_number(loft_lx *x) {
    uint32_t line = x->line, col = x->pos;
    if (loft_lx_decimal(x, line, col)) return;
    loft_lx_str val = {0};
    loft_lx_get_number(x, &val);
    int f = 0;
    int prev_field_dot = loft_lx_prev_is(x, LOFT_LX_DOT);
    if (loft_lx_peek(x) == '.') {
        loft_lx_next(x);
        if (loft_lx_peek(x) == '.') {
            loft_lx_next(x);
            uint64_t r;
            if (loft_lx_parse_u64(val.p, &r)) loft_lx_ret_number(x, r, line, col, val.p[0] == '0');
            else loft_lx_ret_number(x, 0, line, col, 0);
            loft_lx_put(x, 3 /* .. */, line, col);
            free(val.p);
            return;
        }
        int32_t m = loft_lx_peek(x);
        int member_dot = m >= 0 && (loft_lx_alphabetic(m) || m == '_');
        if (prev_field_dot || member_dot) {
            uint32_t dl = x->line, dc = x->pos;
            uint64_t r;
            if (loft_lx_parse_u64(val.p, &r)) loft_lx_ret_number(x, r, line, col, val.p[0] == '0');
            else loft_lx_ret_number(x, 0, line, col, 0);
            loft_lx_put(x, LOFT_LX_DOT, dl, dc);
            free(val.p);
            return;
        }
        loft_lx_str_push(&val, '.');
        f = 1;
        loft_lx_str part = {0};
        loft_lx_get_number(x, &part);
        if (!part.len) {
            free(part.p);
            free(val.p);
            loft_lx_put_none(x, line, col);
            return;
        }
        loft_lx_str_cat(&val, &part);
        free(part.p);
    }
    int32_t e = loft_lx_peek(x);
    if (e == 'e' || e == 'E') {
        f = 1;
        loft_lx_str_push(&val, 'e');
        loft_lx_next(x);
        int32_t s = loft_lx_peek(x);
        if (s == '-' || s == '+') {
            loft_lx_next(x);
            loft_lx_str_push(&val, (char) s);
        }
        loft_lx_str ex = {0};
        loft_lx_get_number(x, &ex);
        if (!ex.len) {
            free(ex.p);
            free(val.p);
            loft_lx_put_none(x, line, col);
            return;
        }
        loft_lx_str_cat(&val, &ex);
        free(ex.p);
    }
    if (f) {
        int32_t c = loft_lx_peek(x);
        if (loft_lx_ascii_alpha(c) && c != 'f' && c != 'd') {
            int32_t n;
            while ((n = loft_lx_peek(x)) >= 0 && (loft_lx_ascii_alnum(n) || n == '_')) loft_lx_next(x);
        }
        int ok = loft_lx_float_text(val.p);
        char *end = NULL;
        if (loft_lx_peek(x) == 'f') {
            loft_lx_next(x);
            float r = ok ? strtof(val.p, &end) : 0.0f;
            if (!ok || *end) r = 0.0f;
            loft_lx_tok *t = loft_lx_put(x, LOFT_LX_SINGLE, line, col);
            t->f = (double) r;
        } else if (loft_lx_peek(x) == 'd') {
            loft_lx_next(x);
            double r = ok ? strtod(val.p, &end) : 0.0;
            if (!ok || *end) r = 0.0;
            loft_lx_tok *t = loft_lx_put(x, LOFT_LX_DOUBLE, line, col);
            t->f = r;
        } else {
            double r = ok ? strtod(val.p, &end) : 0.0;
            if (!ok || *end) r = 0.0;
            loft_lx_tok *t = loft_lx_put(x, LOFT_LX_FLOAT, line, col);
            t->f = r;
            t->zero = val.p[0] == '0';
        }
        free(val.p);
        return;
    }
    if (val.len >= 2 && val.p[0] == '0' && (val.p[1] == 'x' || val.p[1] == 'b' || val.p[1] == 'o')) {
        uint64_t r = 0;
        unsigned radix = val.p[1] == 'x' ? 16 : val.p[1] == 'b' ? 2 : 8;
        if (!loft_lx_radix_parse(val.p + 2, radix, &r)) r = 0;
        if (!loft_lx_wide_suffix(x, r, line, col)) loft_lx_ret_number(x, r, line, col, 0);
        free(val.p);
        return;
    }
    uint64_t r;
    if (loft_lx_parse_u64(val.p, &r)) {
        if (loft_lx_wide_suffix(x, r, line, col)) {
            free(val.p);
            return;
        }
        if (!x->in_format_expr && loft_lx_ascii_alpha(loft_lx_peek(x))) {
            int32_t n;
            while ((n = loft_lx_peek(x)) >= 0 && (loft_lx_ascii_alnum(n) || n == '_')) loft_lx_next(x);
        }
        loft_lx_ret_number(x, r, line, col, val.p[0] == '0');
    } else {
        loft_lx_ret_number(x, 0, line, col, 0);
    }
    free(val.p);
}

/* ── The scan (`scan_once`) ─────────────────────────────────────────────────────────────── */

/* The token a punctuation spelling of one or two characters is, or -1. */
static int loft_lx_token(const char *s, size_t n) {
    for (int k = 0; k < LOFT_LX_SPACE; k++) {
        const char *t = loft_lx_spell[k];
        if (strlen(t) == n && memcmp(t, s, n) == 0) return k;
    }
    if (n == 2 && s[0] == '/' && s[1] == '/') return 1000; /* the comment marker */
    return -1;
}

static int loft_lx_keyword(const uint8_t *s, size_t n) {
    for (int k = LOFT_LX_KW0; k < LOFT_LX_IDENT; k++) {
        const char *t = loft_lx_spell[k];
        if (strlen(t) == n && memcmp(t, s, n) == 0) return k;
    }
    return -1;
}

#define LOFT_LX_SCAN_TOKEN 0
#define LOFT_LX_SCAN_END 1
#define LOFT_LX_SCAN_COMMENT 2

static int loft_lx_scan_once(loft_lx *x) {
    if (x->mode != LOFT_LX_FMT) {
        for (;;) {
            int32_t c = loft_lx_peek(x);
            if (c >= 0) {
                if (c != ' ' && c != '\t' && c != '\r') break;
                loft_lx_next(x);
            } else if (loft_lx_more_lines(x)) {
                size_t ln = x->next_line;
                if (x->line == 0 && x->le[ln] - x->ls[ln] >= 3 && memcmp(x->b + x->ls[ln], "#!/", 3) == 0) {
                    x->line++;
                    x->next_line++;
                    continue;
                }
                loft_lx_enter(x);
                x->line++;
                x->pos = 1;
            } else {
                break;
            }
        }
    }
    uint32_t line = x->line, col = x->pos;
    int32_t c = loft_lx_peek(x);
    if (c < 0) {
        if (loft_lx_more_lines(x)) {
            loft_lx_enter(x);
            x->line++;
            x->pos = 1;
            loft_lx_put(x, LOFT_LX_LINEEND, x->line, x->pos);
            return LOFT_LX_SCAN_TOKEN;
        }
        return LOFT_LX_SCAN_END;
    }
    if (c >= '0' && c <= '9') {
        loft_lx_number(x);
    } else if (c == '"') {
        loft_lx_next(x);
        if (x->in_format_expr) loft_lx_string_nested(x, 0, 0);
        else loft_lx_string(x, line, col + 1);
    } else if (c == '`') {
        loft_lx_next(x);
        int nested = x->in_format_expr;
        loft_lx_strip_push(x);
        loft_lx_backtick_string(x, nested);
    } else if (c == '\'') {
        loft_lx_next(x);
        loft_lx_char(x);
    } else if (c == ' ' || c == '\t') {
        loft_lx_next(x);
        loft_lx_put(x, LOFT_LX_SPACE, line, col);
    } else {
        char one = (char) c;
        int single = c < 0x80 ? loft_lx_token(&one, 1) : -1;
        if (single >= 0) {
            loft_lx_next(x);
            int tok = single;
            int32_t d = loft_lx_peek(x);
            if (d >= 0) {
                char two[2] = {one, (char) d};
                int dbl = d < 0x80 ? loft_lx_token(two, 2) : -1;
                if (dbl >= 0) {
                    loft_lx_next(x);
                    tok = dbl;
                } else if (x->mode == LOFT_LX_FMT && single == LOFT_LX_RBRACE) {
                    loft_lx_resume_string(x);
                    return LOFT_LX_SCAN_TOKEN;
                }
            }
            if (tok == 1000) {
                x->at = x->cur_end;
                return LOFT_LX_SCAN_COMMENT;
            }
            loft_lx_put(x, tok, line, col);
        } else if (c == '\\' && x->in_format_expr) {
            loft_lx_next(x);
            if (loft_lx_peek(x) == '"') {
                loft_lx_next(x);
                loft_lx_string_nested(x, 1, 0);
            } else {
                loft_lx_put_none(x, line, col);
            }
        } else {
            size_t i = x->at;
            while (i < x->cur_end && (loft_lx_ascii_alnum(x->b[i]) || x->b[i] == '_')) i++;
            size_t len = i - x->at;
            if (!len) {
                loft_lx_next(x);
                loft_lx_put_none(x, line, col);
            } else {
                int kw = loft_lx_keyword(x->b + x->at, len);
                const uint8_t *w = x->b + x->at;
                x->at = i;
                x->pos += (uint32_t) len;
                if (kw >= 0) loft_lx_put(x, kw, line, col);
                else loft_lx_put_text(x, LOFT_LX_IDENT, w, len, line, col);
            }
        }
    }
    return LOFT_LX_SCAN_TOKEN;
}

/* ── The parser's mode switches around a hole (src/parser/objects.rs `parse_string`) ─────── */

static int loft_lx_is_operand(int k) {
    return k == LOFT_LX_IDENT || (k >= LOFT_LX_INT && k <= LOFT_LX_STRHOLE) || k == LOFT_LX_CHAR ||
           k == LOFT_LX_KW0 + 10 || k == LOFT_LX_KW0 + 11 || k == LOFT_LX_KW0 + 12; /* true false null */
}

/* `radix_for`: a letter (or `json`) that closes a spec. */
static int loft_lx_radix_name(const loft_lx *x, const loft_lx_tok *t) {
    if (t->kind != LOFT_LX_IDENT) return 0;
    const char *s = (const char *) x->out->txt.p + t->s_at;
    size_t n = t->s_len;
    if (n == 1) return strchr("jJxXbBoedf", s[0]) != NULL && s[0] != 'B';
    if (n == 4) {
        char l[4];
        for (int i = 0; i < 4; i++) l[i] = (char) (s[i] >= 'A' && s[i] <= 'Z' ? s[i] + 32 : s[i]);
        return memcmp(l, "json", 4) == 0;
    }
    return 0;
}

static void loft_lx_after(loft_lx *x, size_t ti, long owner);

/* The `}` closing a hole was read as code: `set_mode(Formatting)` exchanges it for the rest of
 * the string, which belongs to the hole around this one. */
static void loft_lx_resume_now(loft_lx *x, long owner) {
    x->mode = LOFT_LX_FMT;
    size_t before = x->out->n;
    x->popped = 0;
    loft_lx_resume_string(x);
    size_t end = x->out->n;
    for (size_t k = before; k < end; k++) loft_lx_after(x, k, owner - 1);
}

/* A token at depth 0 of a width that cannot continue its expression ends it: the parser reads it
 * as code, then goes back to text for the radix letter. */
static void loft_lx_width_end(loft_lx *x, loft_lx_hole *h, const loft_lx_tok *t, long owner) {
    x->mode = LOFT_LX_FMT;
    h->phase = LOFT_LX_H_AFTER;
    if (t->kind == LOFT_LX_RBRACE) loft_lx_resume_now(x, owner);
}

/* What the parser does with token `ti`, read inside the hole `owner` (an index into the open
 * holes, -1 outside every hole). */
static void loft_lx_after(loft_lx *x, size_t ti, long owner) {
    if (owner < 0 || (size_t) owner >= x->nholes) return;
    loft_lx_hole *h = &x->holes[owner];
    loft_lx_tok t = x->out->t[ti];
    int k = t.kind;
    switch (h->phase) {
    case LOFT_LX_H_FIRST:
        /* `parse_string`'s loop: `set_mode(Code)`, then `whitespace()` drops a space token. */
        x->mode = LOFT_LX_CODE;
        h->phase = LOFT_LX_H_EXPR;
        h->depth = 0;
        if (k == LOFT_LX_SPACE) return;
        loft_lx_after(x, ti, owner);
        return;
    case LOFT_LX_H_EXPR:
        if (k == LOFT_LX_LPAREN || k == LOFT_LX_LBRACKET || k == LOFT_LX_LBRACE) {
            h->depth++;
        } else if (k == LOFT_LX_RPAREN || k == LOFT_LX_RBRACKET) {
            if (h->depth) h->depth--;
        } else if (k == LOFT_LX_RBRACE) {
            if (h->depth) h->depth--;
            else loft_lx_resume_now(x, owner);
        } else if (k == LOFT_LX_COLON && h->depth == 0) {
            x->mode = LOFT_LX_FMT;
            h->phase = LOFT_LX_H_SPEC;
        }
        return;
    case LOFT_LX_H_SPEC:
        /* A fill: any token but the ones that begin flags or end the spec. */
        if (k < LOFT_LX_IDENT && k != LOFT_LX_RBRACE && k != LOFT_LX_DOT && k != 27 /* < */ &&
            k != 29 /* > */ && k != 37 /* ^ */ && k != 14 /* + */ && k != 16 /* - */ && k != 41 /* # */) {
            h->phase = LOFT_LX_H_FLAGS;
            return;
        }
        h->phase = LOFT_LX_H_FLAGS;
        loft_lx_after(x, ti, owner);
        return;
    case LOFT_LX_H_FLAGS: {
        if (k == 27 || k == 37 || k == 29 || k == 14 || k == 41 || k == LOFT_LX_DOT) return;
        int width = (k < LOFT_LX_IDENT && k != LOFT_LX_RBRACE && k != LOFT_LX_DOT) ||
                    (k == LOFT_LX_IDENT && !loft_lx_radix_name(x, &t)) || k == LOFT_LX_INT ||
                    k == LOFT_LX_FLOAT;
        if (width) {
            x->mode = LOFT_LX_CODE;
            h->phase = LOFT_LX_H_WIDTH;
            h->depth = 0;
            h->operand = 0;
            loft_lx_after(x, ti, owner);
        } else {
            h->phase = LOFT_LX_H_AFTER;
        }
        return;
    }
    case LOFT_LX_H_WIDTH:
        if (k == LOFT_LX_LPAREN || k == LOFT_LX_LBRACKET || k == LOFT_LX_LBRACE) {
            h->depth++;
            h->operand = 0;
        } else if (k == LOFT_LX_RPAREN || k == LOFT_LX_RBRACKET || (k == LOFT_LX_RBRACE && h->depth)) {
            if (h->depth) h->depth--;
            if (!h->depth) h->operand = 1;
        } else if (h->depth) {
            return;
        } else if (k == LOFT_LX_RBRACE) {
            loft_lx_width_end(x, h, &t, owner);
        } else if (loft_lx_is_operand(k)) {
            if (h->operand) loft_lx_width_end(x, h, &t, owner);
            else h->operand = 1;
        } else if (k == LOFT_LX_COLON || k == 4 /* , */ || k == 11 /* ; */ || k == LOFT_LX_SPACE ||
                   k == LOFT_LX_LINEEND || k == LOFT_LX_BAD) {
            loft_lx_width_end(x, h, &t, owner);
        } else {
            h->operand = 0;
        }
        return;
    default:
        return;
    }
}

/* The lines of `b`, as `str::lines` splits a file's text. */
static void loft_lx_split(loft_lx *x) {
    size_t cap = 64, n = 0, i = 0;
    x->ls = (size_t *) malloc(cap * sizeof *x->ls);
    x->le = (size_t *) malloc(cap * sizeof *x->le);
    if (!x->ls || !x->le) { fputs("loft: out of memory\n", stderr); abort(); }
    while (i < x->len) {
        size_t s = i;
        while (i < x->len && x->b[i] != '\n') i++;
        size_t e = i;
        if (i < x->len) i++;
        if (e > s && x->b[e - 1] == '\r') e--;
        if (n == cap) {
            cap *= 2;
            x->ls = (size_t *) realloc(x->ls, cap * sizeof *x->ls);
            x->le = (size_t *) realloc(x->le, cap * sizeof *x->le);
            if (!x->ls || !x->le) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        x->ls[n] = s;
        x->le[n] = e;
        n++;
    }
    x->nlines = n;
}

/* Every token of the source text `b`, the last one `LOFT_LX_END` where the input ends. */
static void loft_lex(const uint8_t *b, size_t len, loft_lx_toks *out) {
    loft_lx x;
    memset(&x, 0, sizeof x);
    x.b = b;
    x.len = len;
    x.out = out;
    loft_lx_split(&x);
    for (;;) {
        size_t before = out->n;
        long top = (long) x.nholes - 1;
        x.popped = 0;
        int r = loft_lx_scan_once(&x);
        if (r == LOFT_LX_SCAN_END) break;
        if (r == LOFT_LX_SCAN_COMMENT) continue;
        long owner = top - (x.popped ? 1 : 0);
        size_t end = out->n;
        for (size_t k = before; k < end; k++) loft_lx_after(&x, k, owner);
    }
    loft_lx_put(&x, LOFT_LX_END, x.line, x.pos);
    free(x.ls);
    free(x.le);
    free(x.holes);
    free(x.strip);
}

#endif
#endif
