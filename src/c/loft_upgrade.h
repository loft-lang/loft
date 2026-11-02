#ifdef LOFT_UPGRADE
/* ── The store upgrade (src/database/upgrade_map.rs, upgrade.rs, layout_parse.rs; @PLN196) ──────
 * A whole-image load of a store written under another layout CONVERTS it, as the interpreter
 * does (`@FR-L-Sound`, STORE_MODEL.md D30): the store's `.dschema` dump is read back into a
 * layout description, the program's own description is read from the text the emitter wrote
 * (`loft_upgrade`), the two give the upgrade map — where every new field's value comes from, how
 * each collection is rebuilt, or the first change it refuses and why — and the converter builds
 * a NEW store record by record: old bytes read only through the old description, new records
 * written only through the program's own allocate-and-file pair (`loft_pw_new`, `loft_pw_finish`,
 * the parse writer's `record_new` / `record_finish`).  An added field whose default is not a
 * constant is owed to the program's drain (`__upgrade_drain`, the `store_upgrade_owed_*`
 * natives), as is each `upgrade` routine call.  A refusal leaves the loaded value as it was. */

/* ── A growable byte buffer for the texts the map builds (paths, reasons, closures); its type
 * is declared beside the loader (`loft_image.h`) ── */

static void loft_ub_put(loft_ub *b, const char *s, size_t n) {
    if (b->n + n + 1 > b->cap) {
        b->cap = (b->n + n + 1) * 2 + 32;
        b->p = (char *) realloc(b->p, b->cap);
        if (!b->p) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    memcpy(b->p + b->n, s, n);
    b->n += n;
    b->p[b->n] = 0;
}

static void loft_ub_puts(loft_ub *b, const char *s) { loft_ub_put(b, s, strlen(s)); }

static void loft_ub_printf(loft_ub *b, const char *fmt, ...) {
    char small[256];
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(small, sizeof small, fmt, ap);
    va_end(ap);
    if (n < 0) return;
    if ((size_t) n < sizeof small) { loft_ub_put(b, small, (size_t) n); return; }
    char *big = (char *) malloc((size_t) n + 1);
    if (!big) { fputs("loft: out of memory\n", stderr); abort(); }
    va_start(ap, fmt);
    vsnprintf(big, (size_t) n + 1, fmt, ap);
    va_end(ap);
    loft_ub_put(b, big, (size_t) n);
    free(big);
}

static const char *loft_ub_str(const loft_ub *b) { return b->p ? b->p : ""; }
static void loft_ub_free(loft_ub *b) { free(b->p); memset(b, 0, sizeof *b); }

static char *loft_up_dup(const char *s, size_t n) {
    char *d = (char *) malloc(n + 1);
    if (!d) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(d, s, n);
    d[n] = 0;
    return d;
}

/* ── A layout description (`LayoutDesc`, src/database/descriptor.rs) ── */
#define LOFT_LN_NAME 0 /* a name with no node: a reference target outside the closure */
#define LOFT_LN_BASE 1
#define LOFT_LN_RECORD 2
#define LOFT_LN_EVALUE 3
#define LOFT_LN_CHOICES 4
#define LOFT_LN_BYTE 5
#define LOFT_LN_SHORT 6
#define LOFT_LN_INT 7
#define LOFT_LN_SHORTRAW 8
#define LOFT_LN_INTRAW 9
#define LOFT_LN_VECTOR 10
#define LOFT_LN_ARRAY 11
#define LOFT_LN_ITER 12
#define LOFT_LN_REF 13
#define LOFT_LN_CHILD 14

/* `BaseKind`, numbered as the seven seed rows of every type table. */
#define LOFT_LB_INTEGER 0
#define LOFT_LB_LONG 1
#define LOFT_LB_SINGLE 2
#define LOFT_LB_FLOAT 3
#define LOFT_LB_BOOLEAN 4
#define LOFT_LB_TEXT 5
#define LOFT_LB_CHARACTER 6

/* `Iterated`'s kinds. */
#define LOFT_LI_SORTED 0
#define LOFT_LI_ORDERED 1
#define LOFT_LI_HASH 2
#define LOFT_LI_INDEX 3
#define LOFT_LI_RADIX 4
#define LOFT_LI_TRIE 5

#define LOFT_LD_NONE 0xFFFFu

/* `LayoutField`. */
typedef struct {
    char *name;
    uint16_t pos, content, target;
    uint8_t nullable;
    uint16_t *group;
    uint16_t ngroup;
} loft_lf;

/* One `LayoutNode`, with its name, size, dump line and the facts beside it. */
typedef struct {
    char *name;
    char *line;      /* its dump line, as the sidecar wrote it */
    char *placement; /* a keyed collection's placement token when it is not this build's */
    uint8_t kind, base, ikind, nullable, tag, linked, old_order;
    int32_t start;
    uint16_t size, elem;
    loft_lf *f;
    uint16_t nf;
    uint16_t *vt; /* a value enum's variant types (LOFT_LD_NONE for a unit variant) */
    char **vn;    /* … and names */
    uint16_t nv;
    uint16_t *keys; /* a keyed collection's key fields */
    uint8_t *asc;
    uint16_t nk;
    uint16_t prog; /* the program's type number of this node (the program's description only) */
} loft_ln;

typedef struct { loft_ln *n; uint16_t count; } loft_ld;

static void loft_ld_free(loft_ld *d) {
    for (uint16_t i = 0; i < d->count; i++) {
        loft_ln *n = &d->n[i];
        free(n->name);
        free(n->line);
        free(n->placement);
        for (uint16_t k = 0; k < n->nf; k++) { free(n->f[k].name); free(n->f[k].group); }
        free(n->f);
        free(n->vt);
        for (uint16_t k = 0; k < n->nv; k++) free(n->vn[k]);
        free(n->vn);
        free(n->keys);
        free(n->asc);
    }
    free(d->n);
    memset(d, 0, sizeof *d);
}

static uint16_t loft_ld_id(const loft_ld *d, const char *name, size_t len) {
    for (uint16_t i = 0; i < d->count; i++)
        if (strlen(d->n[i].name) == len && memcmp(d->n[i].name, name, len) == 0) return i;
    return LOFT_LD_NONE;
}

static uint16_t loft_ld_add_name(loft_ld *d, const char *name, size_t len) {
    d->n = (loft_ln *) realloc(d->n, ((size_t) d->count + 1) * sizeof *d->n);
    if (!d->n) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_ln *n = &d->n[d->count];
    memset(n, 0, sizeof *n);
    n->name = loft_up_dup(name, len);
    n->elem = LOFT_LD_NONE;
    n->prog = LOFT_LD_NONE;
    return d->count++;
}

/* `matching`: the index of the closer of an opener before `s[from..]`; `->` closes nothing. */
static long loft_ld_matching(const char *s, size_t len, size_t from) {
    int depth = 0;
    for (size_t i = from; i < len; i++) {
        char c = s[i];
        if (c == '<' || c == '[' || c == '(' || c == '{') depth++;
        else if (c == '>' && i > 0 && s[i - 1] == '-') continue;
        else if (c == '>' || c == ']' || c == ')' || c == '}') {
            if (depth == 0) return (long) i;
            depth--;
        }
    }
    return -1;
}

/* `split_top`: the parts of `s` split at every `, ` outside brackets, as (start, len) pairs. */
typedef struct { size_t at, len; } loft_span;

static loft_span *loft_ld_split(const char *s, size_t len, size_t *count) {
    size_t cap = 4, n = 0, start = 0;
    loft_span *out = (loft_span *) malloc(cap * sizeof *out);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    int depth = 0;
    for (size_t i = 0; i < len; i++) {
        char c = s[i];
        if (c == '<' || c == '[' || c == '(' || c == '{') depth++;
        else if (c == '>' && i > 0 && s[i - 1] == '-') {}
        else if (c == '>' || c == ']' || c == ')' || c == '}') depth--;
        else if (c == ',' && depth == 0 && i + 1 < len && s[i + 1] == ' ') {
            if (n == cap) { cap *= 2; out = (loft_span *) realloc(out, cap * sizeof *out); }
            if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
            out[n].at = start;
            out[n].len = i - start;
            n++;
            start = i + 2;
            i++;
        }
    }
    if (n == cap) { cap++; out = (loft_span *) realloc(out, cap * sizeof *out); }
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    out[n].at = start;
    out[n].len = len - start;
    *count = n + 1;
    return out;
}

static int loft_ld_prefix(const char *s, size_t len, const char *pre) {
    size_t p = strlen(pre);
    return len >= p && memcmp(s, pre, p) == 0;
}

static int loft_ld_suffix(const char *s, size_t len, const char *suf) {
    size_t p = strlen(suf);
    return len >= p && memcmp(s + len - p, suf, p) == 0;
}

/* A decimal number spanning exactly `s[0..len]`. */
static int loft_ld_num(const char *s, size_t len, long long *out) {
    if (len == 0 || len > 20) return 0;
    char b[24];
    memcpy(b, s, len);
    b[len] = 0;
    char *end = 0;
    long long v = strtoll(b, &end, 10);
    if (!end || *end != 0) return 0;
    *out = v;
    return 1;
}

/* The parse of one dump: the line being read, and why it did not read. */
typedef struct { loft_ld *d; const char *line; loft_ub *err; } loft_ldp;

static int loft_ldp_fail(loft_ldp *p, const char *what) {
    if (p->err->n == 0) loft_ub_printf(p->err, "the store's layout line `%s` %s", p->line, what);
    return 0;
}

static int loft_ldp_type(loft_ldp *p, const char *s, size_t len, uint16_t *out) {
    uint16_t id = loft_ld_id(p->d, s, len);
    if (id == LOFT_LD_NONE) return loft_ldp_fail(p, "names a type the dump does not describe");
    *out = id;
    return 1;
}

/* `Cx::fields`: `name@pos:type, …`. */
static int loft_ldp_fields(loft_ldp *p, loft_ln *n, const char *s, size_t len) {
    if (len == 0) return 1;
    size_t count = 0;
    loft_span *sp = loft_ld_split(s, len, &count);
    n->f = (loft_lf *) calloc(count, sizeof *n->f);
    if (!n->f) { fputs("loft: out of memory\n", stderr); abort(); }
    n->nf = (uint16_t) count;
    int ok = 1;
    for (size_t i = 0; i < count && ok; i++) {
        const char *f = s + sp[i].at;
        size_t fl = sp[i].len;
        const char *at = (const char *) memchr(f, '@', fl);
        const char *colon = at ? (const char *) memchr(at, ':', fl - (size_t) (at - f)) : 0;
        long long pos;
        if (!at || !colon || !loft_ld_num(at + 1, (size_t) (colon - at - 1), &pos) || pos < 0 || pos > 0xFFFF) {
            ok = loft_ldp_fail(p, "has a field that does not read");
            break;
        }
        loft_lf *lf = &n->f[i];
        lf->name = loft_up_dup(f, (size_t) (at - f));
        lf->pos = (uint16_t) pos;
        lf->target = LOFT_LD_NONE;
        const char *tp = colon + 1;
        size_t tl = fl - (size_t) (tp - f);
        if (tl == 6 && memcmp(tp, "#65535", 6) == 0) lf->content = LOFT_LD_NONE;
        else ok = loft_ldp_type(p, tp, tl, &lf->content);
    }
    free(sp);
    return ok;
}

/* `[0, 2]`. */
static int loft_ldp_plain_keys(loft_ldp *p, loft_ln *n, const char *s, size_t len) {
    if (len < 2 || s[0] != '[' || s[len - 1] != ']') return loft_ldp_fail(p, "has keys that are not a list");
    s++;
    len -= 2;
    n->keys = (uint16_t *) malloc((len / 3 + 2) * sizeof *n->keys);
    n->asc = (uint8_t *) malloc(len / 3 + 2);
    if (!n->keys || !n->asc) { fputs("loft: out of memory\n", stderr); abort(); }
    if (len == 0) return 1;
    size_t i = 0;
    while (i <= len) {
        size_t j = i;
        while (j < len && s[j] != ',') j++;
        long long k;
        if (!loft_ld_num(s + i, j - i, &k) || k < 0 || k > 0xFFFF) return loft_ldp_fail(p, "has a key that is not a field");
        n->keys[n->nk] = (uint16_t) k;
        n->asc[n->nk++] = 1;
        if (j >= len) break;
        i = j + 2;
    }
    return 1;
}

/* `[(0, true), (1, false)]`. */
static int loft_ldp_flagged_keys(loft_ldp *p, loft_ln *n, const char *s, size_t len) {
    if (len < 2 || s[0] != '[' || s[len - 1] != ']') return loft_ldp_fail(p, "has keys that are not a list");
    s++;
    len -= 2;
    n->keys = (uint16_t *) malloc((len / 8 + 2) * sizeof *n->keys);
    n->asc = (uint8_t *) malloc(len / 8 + 2);
    if (!n->keys || !n->asc) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t i = 0;
    while (i < len) {
        if (s[i] != '(') return loft_ldp_fail(p, "has a key that is not a pair");
        size_t j = i + 1;
        while (j < len && s[j] != ',') j++;
        long long k;
        if (j + 2 > len || !loft_ld_num(s + i + 1, j - i - 1, &k) || k < 0 || k > 0xFFFF)
            return loft_ldp_fail(p, "has a key that is not a field");
        const char *flag = s + j + 2;
        size_t rest = len - (j + 2);
        int asc;
        if (rest >= 5 && memcmp(flag, "true)", 5) == 0) { asc = 1; i = j + 2 + 5; }
        else if (rest >= 6 && memcmp(flag, "false)", 6) == 0) { asc = 0; i = j + 2 + 6; }
        else return loft_ldp_fail(p, "has a key order that is not a boolean");
        n->keys[n->nk] = (uint16_t) k;
        n->asc[n->nk++] = (uint8_t) asc;
        if (i < len) {
            if (i + 2 > len || s[i] != ',' || s[i + 1] != ' ') return loft_ldp_fail(p, "has keys that are not a list");
            i += 2;
        }
    }
    return 1;
}

/* This build's placement token of each keyed kind (`crate::placement`), and the baseline that
 * renders as no suffix. */
#define LOFT_PL_BASELINE "1"
#define LOFT_PL_HASH "3"
#define LOFT_PL_INDEX "2"
#define LOFT_PL_RADIX "1"
#define LOFT_PL_TRIE "2"

/* `placement_of`: 1 with `*foreign` set to a malloc'd token when the suffix records another
 * build's placement (0 when it is this build's), 0 when the suffix is malformed. */
static int loft_ld_placement(const char *suffix, size_t len, const char *own, char **foreign) {
    *foreign = 0;
    int own_base = strcmp(own, LOFT_PL_BASELINE) == 0;
    if (len == 0) {
        if (!own_base) *foreign = loft_up_dup(LOFT_PL_BASELINE, 1);
        return 1;
    }
    if (!loft_ld_prefix(suffix, len, ",placement=")) return 0;
    const char *tok = suffix + 11;
    size_t tl = len - 11;
    if (!(tl == strlen(own) && memcmp(tok, own, tl) == 0) || own_base) *foreign = loft_up_dup(tok, tl);
    return 1;
}

#define LOFT_KEY_ORDER ",order=1"

/* `Cx::collection`: `kind<E>(args)`. */
static int loft_ldp_collection(loft_ldp *p, loft_ln *n, const char *s, size_t len) {
    const char *lt = (const char *) memchr(s, '<', len);
    if (!lt) return loft_ldp_fail(p, "has parts this build does not know");
    size_t kl = (size_t) (lt - s);
    const char *rest = lt + 1;
    size_t rl = len - kl - 1;
    long close = loft_ld_matching(rest, rl, 0);
    if (close < 0) return loft_ldp_fail(p, "has an unbalanced `<`");
    if (!loft_ldp_type(p, rest, (size_t) close, &n->elem)) return 0;
    const char *args = rest + close + 1;
    size_t al = rl - (size_t) close - 1;
    if (al < 2 || args[0] != '(' || args[al - 1] != ')') return loft_ldp_fail(p, "misses its `(…)`");
    args++;
    al -= 2;
    if ((kl == 6 && memcmp(s, "vector", 6) == 0) || (kl == 5 && memcmp(s, "array", 5) == 0)) {
        if (!loft_ld_prefix(args, al, "elem_size=")) return loft_ldp_fail(p, "misses its elem_size");
        n->kind = kl == 6 ? LOFT_LN_VECTOR : LOFT_LN_ARRAY;
        return 1;
    }
    if (!loft_ld_prefix(args, al, "keys=")) return loft_ldp_fail(p, "misses its keys");
    const char *keys = args + 5;
    const char *es = 0;
    for (const char *q = keys; q + 11 <= args + al; q++)
        if (memcmp(q, ",elem_size=", 11) == 0) { es = q; break; }
    if (!es) return loft_ldp_fail(p, "misses its keys");
    size_t keys_len = (size_t) (es - keys);
    const char *size_and_tag = es + 11;
    size_t st = al - (size_t) (size_and_tag - args);
    if (loft_ld_suffix(size_and_tag, st, LOFT_KEY_ORDER)) st -= strlen(LOFT_KEY_ORDER);
    const char *comma = (const char *) memchr(size_and_tag, ',', st);
    const char *tag = comma ? comma : size_and_tag + st;
    size_t tag_len = comma ? st - (size_t) (comma - size_and_tag) : 0;
    const char *own = "";
    int flagged = 1;
    n->kind = LOFT_LN_ITER;
    if (kl == 6 && memcmp(s, "sorted", 6) == 0) n->ikind = LOFT_LI_SORTED;
    else if (kl == 7 && memcmp(s, "ordered", 7) == 0) n->ikind = LOFT_LI_ORDERED;
    else if (kl == 4 && memcmp(s, "hash", 4) == 0) { n->ikind = LOFT_LI_HASH; own = LOFT_PL_HASH; flagged = 0; }
    else if (kl == 5 && memcmp(s, "index", 5) == 0) { n->ikind = LOFT_LI_INDEX; own = LOFT_PL_INDEX; }
    else if (kl == 7 && memcmp(s, "spatial", 7) == 0) { n->ikind = LOFT_LI_RADIX; own = LOFT_PL_RADIX; flagged = 0; }
    else return loft_ldp_fail(p, "has a collection kind this build does not know");
    if (!(flagged ? loft_ldp_flagged_keys(p, n, keys, keys_len) : loft_ldp_plain_keys(p, n, keys, keys_len))) return 0;
    if (!*own) {
        if (tag_len) return loft_ldp_fail(p, "carries a placement on a sorted collection");
        return 1;
    }
    if (!loft_ld_placement(tag, tag_len, own, &n->placement)) return loft_ldp_fail(p, "has a malformed placement");
    return 1;
}

/* `Cx::parts`: one line's layout part. */
static int loft_ldp_parts(loft_ldp *p, loft_ln *n, const char *s, size_t len) {
    if (len == 4 && memcmp(s, "base", 4) == 0) {
        static const char *const names[] = { "integer", "long", "single", "float", "boolean", "text", "character" };
        for (int b = 0; b < 7; b++)
            if (strcmp(n->name, names[b]) == 0) {
                n->kind = LOFT_LN_BASE;
                n->base = (uint8_t) b;
                return 1;
            }
        return loft_ldp_fail(p, "is `base` but not one of the seven base types");
    }
    if (len == 7 && memcmp(s, "dbref12", 7) == 0) { n->kind = LOFT_LN_REF; return 1; }
    if (loft_ld_prefix(s, len, "struct{") && s[len - 1] == '}') {
        n->kind = LOFT_LN_RECORD;
        return loft_ldp_fields(p, n, s + 7, len - 8);
    }
    if (loft_ld_prefix(s, len, "enumvalue[")) {
        const char *q = s + 10;
        const char *close = (const char *) memchr(q, ']', len - 10);
        long long tag;
        if (!close || close + 1 >= s + len || close[1] != '{' || s[len - 1] != '}' ||
            !loft_ld_num(q, (size_t) (close - q), &tag) || tag < 0 || tag > 255)
            return loft_ldp_fail(p, "is a malformed enumvalue");
        n->kind = LOFT_LN_EVALUE;
        n->tag = (uint8_t) tag;
        return loft_ldp_fields(p, n, close + 2, (size_t) (s + len - 1 - (close + 2)));
    }
    if (loft_ld_prefix(s, len, "enum{") && s[len - 1] == '}') {
        n->kind = LOFT_LN_CHOICES;
        const char *inner = s + 5;
        size_t il = len - 6;
        if (il == 0) return 1;
        size_t count = 0;
        loft_span *sp = loft_ld_split(inner, il, &count);
        n->vt = (uint16_t *) malloc(count * sizeof *n->vt);
        n->vn = (char **) malloc(count * sizeof *n->vn);
        if (!n->vt || !n->vn) { fputs("loft: out of memory\n", stderr); abort(); }
        for (size_t i = 0; i < count; i++) {
            n->vt[i] = LOFT_LD_NONE;
            n->vn[i] = loft_up_dup(inner + sp[i].at, sp[i].len); /* resolved after the last line */
        }
        n->nv = (uint16_t) count;
        free(sp);
        return 1;
    }
    static const char *const narrow[] = { "int4raw(", "shortraw(", "int4(", "short(", "byte(" };
    static const uint8_t nkind[] = { LOFT_LN_INTRAW, LOFT_LN_SHORTRAW, LOFT_LN_INT, LOFT_LN_SHORT, LOFT_LN_BYTE };
    for (int k = 0; k < 5; k++) {
        if (!loft_ld_prefix(s, len, narrow[k]) || s[len - 1] != ')') continue;
        const char *q = s + strlen(narrow[k]);
        size_t ql = (size_t) (s + len - 1 - q);
        const char *null = 0;
        for (const char *r = q; r + 6 <= q + ql; r++)
            if (memcmp(r, ",null=", 6) == 0) { null = r; break; }
        long long start;
        if (!loft_ld_prefix(q, ql, "start=") || !null || !loft_ld_num(q + 6, (size_t) (null - q - 6), &start))
            return loft_ldp_fail(p, "is a malformed narrow integer");
        size_t fl = ql - (size_t) (null + 6 - q);
        if (fl == 4 && memcmp(null + 6, "true", 4) == 0) n->nullable = 1;
        else if (!(fl == 5 && memcmp(null + 6, "false", 5) == 0)) return loft_ldp_fail(p, "has a null flag that is not a boolean");
        n->kind = nkind[k];
        n->start = (int32_t) start;
        return 1;
    }
    if (loft_ld_prefix(s, len, "childrec<") && s[len - 1] == '>') {
        n->kind = LOFT_LN_CHILD;
        return loft_ldp_type(p, s + 9, len - 10, &n->elem);
    }
    if (loft_ld_prefix(s, len, "trie<")) {
        const char *rest = s + 5;
        size_t rl = len - 5;
        long close = loft_ld_matching(rest, rl, 0);
        if (close < 0) return loft_ldp_fail(p, "is a malformed trie");
        const char *open = 0;
        for (long i = close - 1; i >= 0; i--)
            if (rest[i] == '[') { open = rest + i; break; }
        long long key;
        if (!open || rest[close - 1] != ']' || !loft_ld_num(open + 1, (size_t) (rest + close - 1 - open - 1), &key) || key < 0)
            return loft_ldp_fail(p, "has a malformed trie key");
        n->kind = LOFT_LN_ITER;
        n->ikind = LOFT_LI_TRIE;
        n->keys = (uint16_t *) malloc(sizeof *n->keys);
        n->asc = (uint8_t *) malloc(1);
        if (!n->keys || !n->asc) { fputs("loft: out of memory\n", stderr); abort(); }
        n->keys[0] = (uint16_t) key;
        n->asc[0] = 1;
        n->nk = 1;
        if (!loft_ldp_type(p, rest, (size_t) (open - rest), &n->elem)) return 0;
        if (!loft_ld_placement(rest + close + 1, rl - (size_t) close - 1, LOFT_PL_TRIE, &n->placement))
            return loft_ldp_fail(p, "has a malformed trie placement");
        return 1;
    }
    return loft_ldp_collection(p, n, s, len);
}

/* `LayoutDesc::key_orders` / `orders`: does a key over `elem` order by `placement::KEY_ORDER`? */
static int loft_ld_orders(const loft_ld *d, uint16_t t, int depth) {
    if (t >= d->count) return 0;
    const char *nm = d->n[t].name;
    if (strcmp(nm, "float") == 0 || strcmp(nm, "single") == 0 || strcmp(nm, "text") == 0) return 1;
    if (d->n[t].kind == LOFT_LN_RECORD && depth < 8)
        for (uint16_t i = 0; i < d->n[t].nf; i++)
            if (loft_ld_orders(d, d->n[t].f[i].content, depth + 1)) return 1;
    return 0;
}

static int loft_ld_key_orders(const loft_ld *d, const loft_ln *n) {
    if (n->elem >= d->count) return 0;
    const loft_ln *e = &d->n[n->elem];
    if (e->kind != LOFT_LN_RECORD && e->kind != LOFT_LN_EVALUE) return 0;
    for (uint16_t k = 0; k < n->nk; k++)
        if (n->keys[k] < e->nf && loft_ld_orders(d, e->f[n->keys[k]].content, 0)) return 1;
    return 0;
}

/* `LayoutDesc::from_dump`: the dump (from its `@endian` line) read into `d`; 0 with `err` set when
 * a line does not read.  Ids are given in line order. */
static int loft_ld_parse(loft_ld *d, const char *dump, loft_ub *err) {
    memset(d, 0, sizeof *d);
    const char *own = "@endian\tlittle";
    {
        uint16_t probe = 1;
        if (*(const uint8_t *) &probe == 0) own = "@endian\tbig";
    }
    const char *nl = strchr(dump, '\n');
    size_t first = nl ? (size_t) (nl - dump) : strlen(dump);
    if (first != strlen(own) || memcmp(dump, own, first) != 0) {
        loft_ub_printf(err, "the store was written on a host of the other byte order");
        return 0;
    }
    /* Pass 1: every type's name, so a part can name a type whose line comes later. */
    const char *at = nl ? nl + 1 : dump + first;
    const char *lines = at;
    while (*at) {
        const char *end = strchr(at, '\n');
        size_t len = end ? (size_t) (end - at) : strlen(at);
        if (len) {
            const char *tab = (const char *) memchr(at, '\t', len);
            if (!tab) {
                loft_ub_printf(err, "the store's layout has a line that is not a layout line");
                return 0;
            }
            if (loft_ld_id(d, at, (size_t) (tab - at)) != LOFT_LD_NONE) {
                loft_ub_printf(err, "the store's layout describes `%.*s` twice", (int) (tab - at), at);
                return 0;
            }
            uint16_t id = loft_ld_add_name(d, at, (size_t) (tab - at));
            d->n[id].line = loft_up_dup(at, len);
        }
        if (!end) break;
        at = end + 1;
    }
    (void) lines;
    /* Pass 2: each line's size and parts. */
    for (uint16_t id = 0; id < d->count; id++) {
        loft_ln *n = &d->n[id];
        const char *line = n->line;
        const char *t1 = strchr(line, '\t');
        const char *t2 = t1 ? strchr(t1 + 1, '\t') : 0;
        long long size;
        loft_ldp p = { d, line, err };
        if (!t2 || !loft_ld_prefix(t1 + 1, (size_t) (t2 - t1 - 1), "size=") ||
            !loft_ld_num(t1 + 6, (size_t) (t2 - t1 - 6), &size) || size < 0 || size > 0xFFFF)
            return loft_ldp_fail(&p, "has no size");
        n->size = (uint16_t) size;
        if (!loft_ldp_parts(&p, &d->n[id], t2 + 1, strlen(t2 + 1))) return 0;
    }
    /* `resolve_variants`: a struct-enum's variants are all types (an `enumvalue` whose tag is its
     * position plus one, or a record holding only the tag field), a value enum's none. */
    for (uint16_t id = 0; id < d->count; id++) {
        loft_ln *n = &d->n[id];
        if (n->kind != LOFT_LN_CHOICES) continue;
        int explicit_ = 0, all = 1;
        for (uint16_t i = 0; i < n->nv; i++) {
            char *v = n->vn[i];
            char *eq = strchr(v, '=');
            const char *ty = v;
            size_t tl = strlen(v);
            if (eq) {
                explicit_ = 1;
                ty = eq + 1;
                tl = strlen(ty);
            }
            uint16_t t = loft_ld_id(d, ty, tl);
            int typed = 0;
            if (t != LOFT_LD_NONE) {
                const loft_ln *tn = &d->n[t];
                if (tn->kind == LOFT_LN_EVALUE) typed = tn->tag == i + 1;
                else if (tn->kind == LOFT_LN_RECORD) typed = tn->nf > 0 && strcmp(tn->f[0].name, "enum") == 0;
            }
            if (!typed) t = LOFT_LD_NONE;
            all &= t != LOFT_LD_NONE;
            if (eq) *eq = 0;
            n->vt[i] = t;
        }
        if (!all) {
            if (explicit_) {
                loft_ub_printf(err, "the store's layout type `%s` names a variant type that is not one of its variants", n->name);
                return 0;
            }
            for (uint16_t i = 0; i < n->nv; i++) n->vt[i] = LOFT_LD_NONE;
        }
    }
    /* loft#2237 — a keyed line without the key-order suffix its key takes was written under the
     * old order. */
    for (uint16_t id = 0; id < d->count; id++) {
        loft_ln *n = &d->n[id];
        if (n->kind != LOFT_LN_ITER || !(n->ikind == LOFT_LI_SORTED || n->ikind == LOFT_LI_ORDERED || n->ikind == LOFT_LI_INDEX))
            continue;
        int ordered = loft_ld_suffix(n->line, strlen(n->line), LOFT_KEY_ORDER ")");
        int takes = loft_ld_key_orders(d, n);
        if (takes && !ordered) n->old_order = 1;
        else if (!takes && ordered) {
            loft_ub_printf(err, "the store's layout type `%s` carries a key order its key does not take", n->name);
            return 0;
        }
    }
    return 1;
}

/* `LayoutDesc::apply_facts`: the facts the dump does not render — linked types, nullable fields,
 * reference targets (a target outside the closure is a name with no node) and index groups. */
static int loft_ld_facts(loft_ld *d, const char *facts, loft_ub *err) {
    const char *at = facts;
    while (at && *at) {
        const char *end = strchr(at, '\n');
        size_t len = end ? (size_t) (end - at) : strlen(at);
        if (len) {
            const char *tab = (const char *) memchr(at, '\t', len);
            if (!tab) {
                loft_ub_printf(err, "the store's layout has a fact that does not read");
                return 0;
            }
            const char *what = at;
            size_t wl = (size_t) (tab - at);
            const char *fact = tab + 1;
            size_t fl = len - wl - 1;
            if (fl == 6 && memcmp(fact, "linked", 6) == 0) {
                uint16_t id = loft_ld_id(d, what, wl);
                if (id == LOFT_LD_NONE) {
                    loft_ub_printf(err, "the store's layout facts name no type `%.*s`", (int) wl, what);
                    return 0;
                }
                d->n[id].linked = 1;
            } else {
                /* The field is after the last top-level dot. */
                long dot = -1;
                int depth = 0;
                for (size_t i = 0; i < wl; i++) {
                    char c = what[i];
                    if (c == '<' || c == '[' || c == '(' || c == '{') depth++;
                    else if (c == '>' || c == ']' || c == ')' || c == '}') depth--;
                    else if (c == '.' && depth == 0) dot = (long) i;
                }
                if (dot < 0) {
                    loft_ub_printf(err, "the store's layout fact `%.*s` names no field", (int) wl, what);
                    return 0;
                }
                uint16_t id = loft_ld_id(d, what, (size_t) dot);
                if (id == LOFT_LD_NONE) {
                    loft_ub_printf(err, "the store's layout facts name no type `%.*s`", (int) dot, what);
                    return 0;
                }
                uint16_t target = LOFT_LD_NONE;
                if (loft_ld_prefix(fact, fl, "target=")) {
                    target = loft_ld_id(d, fact + 7, fl - 7);
                    if (target == LOFT_LD_NONE) target = loft_ld_add_name(d, fact + 7, fl - 7);
                }
                loft_ln *n = &d->n[id];
                if (n->kind != LOFT_LN_RECORD && n->kind != LOFT_LN_EVALUE) {
                    loft_ub_printf(err, "the store's layout type `%s` has no fields", n->name);
                    return 0;
                }
                const char *fname = what + dot + 1;
                size_t fnl = wl - (size_t) dot - 1;
                loft_lf *f = 0;
                for (uint16_t i = 0; i < n->nf && !f; i++)
                    if (strlen(n->f[i].name) == fnl && memcmp(n->f[i].name, fname, fnl) == 0) f = &n->f[i];
                if (!f) {
                    loft_ub_printf(err, "the store's layout type `%s` has no field `%.*s`", n->name, (int) fnl, fname);
                    return 0;
                }
                if (fl == 8 && memcmp(fact, "nullable", 8) == 0) f->nullable = 1;
                else if (target != LOFT_LD_NONE) f->target = target;
                else if (loft_ld_prefix(fact, fl, "group=")) {
                    const char *g = fact + 6;
                    size_t gl = fl - 6, i = 0;
                    f->group = (uint16_t *) malloc((gl / 2 + 2) * sizeof *f->group);
                    if (!f->group) { fputs("loft: out of memory\n", stderr); abort(); }
                    while (i <= gl) {
                        size_t j = i;
                        while (j < gl && g[j] != ',') j++;
                        long long v;
                        if (!loft_ld_num(g + i, j - i, &v) || v < 0 || v > 0xFFFF) {
                            loft_ub_printf(err, "the store's layout has a bad group `%.*s`", (int) gl, g);
                            return 0;
                        }
                        f->group[f->ngroup++] = (uint16_t) v;
                        if (j >= gl) break;
                        i = j + 1;
                    }
                } else {
                    loft_ub_printf(err, "the store's layout has an unknown fact `%.*s`", (int) fl, fact);
                    return 0;
                }
            }
        }
        if (!end) break;
        at = end + 1;
    }
    return 1;
}

/* `recorded_layout` over a whole `.dschema` text: its description with its facts, the root it
 * names (a version-1 sidecar names none: the one type nothing refers to), the release that wrote
 * it and the slot its references name (-1: none recorded).  0 with `err` set otherwise. */
typedef struct { uint16_t root; long slot; char version[64]; } loft_ld_head;

static int loft_ld_unreferenced(const loft_ld *d, uint16_t *root) {
    uint8_t *ref = (uint8_t *) calloc(d->count ? d->count : 1, 1);
    if (!ref) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint16_t i = 0; i < d->count; i++) {
        const loft_ln *n = &d->n[i];
        for (uint16_t k = 0; k < n->nf; k++) if (n->f[k].content < d->count) ref[n->f[k].content] = 1;
        for (uint16_t k = 0; k < n->nv; k++) if (n->vt[k] < d->count) ref[n->vt[k]] = 1;
        if (n->elem < d->count) ref[n->elem] = 1;
    }
    int found = 0;
    for (uint16_t i = 0; i < d->count; i++)
        if (d->n[i].kind != LOFT_LN_NAME && !ref[i]) { *root = i; found++; }
    free(ref);
    return found == 1;
}

static int loft_ld_read_sidecar(loft_ld *d, const char *text, const char *what, loft_ld_head *h, loft_ub *err) {
    memset(h, 0, sizeof *h);
    h->slot = -1;
    h->root = LOFT_LD_NONE;
    const char *fence = strstr(text, "\n--\n");
    if (!fence || strncmp(text, "LOFT-DSCHEMA v", 14) != 0) {
        loft_ub_printf(err, "%s.dschema is not a layout file", what);
        return 0;
    }
    const char *root = 0;
    size_t root_len = 0;
    for (const char *line = strchr(text, '\n'); line && line < fence;) {
        const char *start = line + 1, *end = strchr(start, '\n');
        if (!end || end > fence) end = fence;
        size_t len = (size_t) (end - start);
        if (loft_ld_prefix(start, len, "root=")) { root = start + 5; root_len = len - 5; }
        else if (loft_ld_prefix(start, len, "slot=")) {
            long long v;
            if (loft_ld_num(start + 5, len - 5, &v)) h->slot = (long) v;
        } else if (loft_ld_prefix(start, len, "version=")) {
            size_t vl = len - 8 < sizeof h->version - 1 ? len - 8 : sizeof h->version - 1;
            memcpy(h->version, start + 8, vl);
            h->version[vl] = 0;
        }
        line = end < fence ? end : 0;
    }
    const char *dump = fence + 4;
    const char *facts = strstr(dump, "--facts--\n");
    char *dtext = loft_up_dup(dump, facts ? (size_t) (facts - dump) : strlen(dump));
    int ok = loft_ld_parse(d, dtext, err);
    free(dtext);
    if (ok && facts) ok = loft_ld_facts(d, facts + 10, err);
    if (!ok) return 0;
    if (root && root_len) h->root = loft_ld_id(d, root, root_len);
    if (h->root == LOFT_LD_NONE && !loft_ld_unreferenced(d, &h->root)) h->root = LOFT_LD_NONE;
    if (h->root == LOFT_LD_NONE) {
        loft_ub_printf(err, "%s.dschema does not say which of its types is the store's root", what);
        return 0;
    }
    return 1;
}

/* ── The upgrade map (`UpgradeMap::derive_with`, src/database/upgrade_map.rs) ── */
#define LOFT_HOW_SAME 0
#define LOFT_HOW_RECORD 1
#define LOFT_HOW_WIDEN 2
#define LOFT_HOW_VARIANTS 3
#define LOFT_HOW_REFERENCE 4
#define LOFT_HOW_ELEMENTS 5

#define LOFT_FF_OLD 0
#define LOFT_FF_DEFAULT 1
#define LOFT_FF_GROUP 2
#define LOFT_FF_LAYOUT 3

/* `FieldFrom`. */
typedef struct { uint8_t from; uint16_t field, source; uint32_t pair; } loft_ff;

/* `Pair`: an (old, new) pair of description ids and how a value crosses it. */
typedef struct {
    uint16_t old, nw;
    uint8_t how;
    loft_ff *plan;
    uint16_t nplan;
    int32_t *vto;   /* per old variant its new discriminant index, -1 when it is gone */
    int64_t *vpair; /* per old variant the pair of its fields, -1 for none */
    uint16_t nv;
    uint32_t elem;
} loft_upair;

typedef struct { uint16_t old, nw; uint32_t at; } loft_umemo;

typedef struct {
    const loft_ld *od, *nd;
    /* `routines`: per new record type with an `upgrade` routine, its `old` struct (new ids). */
    const uint16_t *rt_new, *rt_old;
    uint32_t nrt;
    loft_upair *p;
    uint32_t np, cap;
    loft_umemo *memo;
    uint32_t nm, mcap;
    /* `projections`: per new record type, the pair filling its routine's `old`. */
    uint16_t *proj_new;
    uint32_t *proj_pair;
    uint32_t nproj;
    char **cl_old, **cl_new;
    /* `whole_copies`: an unchanged record or collection may be copied as it is; off, only a
     * leaf is, and everything else is carried through the description. */
    int whole_copies;
    /* The first refusal, `path: reason`; the converter runs no map that has one. */
    loft_ub refusal;
} loft_upmap;

static void loft_um_free(loft_upmap *m) {
    for (uint32_t i = 0; i < m->np; i++) { free(m->p[i].plan); free(m->p[i].vto); free(m->p[i].vpair); }
    free(m->p);
    free(m->memo);
    free(m->proj_new);
    free(m->proj_pair);
    if (m->cl_old) for (uint16_t i = 0; i < m->od->count; i++) free(m->cl_old[i]);
    if (m->cl_new) for (uint16_t i = 0; i < m->nd->count; i++) free(m->cl_new[i]);
    free(m->cl_old);
    free(m->cl_new);
    loft_ub_free(&m->refusal);
}

static const loft_ld *loft_um_desc(const loft_upmap *m, int nw) { return nw ? m->nd : m->od; }

static int loft_um_has_node(const loft_ld *d, uint16_t id) { return id < d->count && d->n[id].kind != LOFT_LN_NAME; }

/* `Deriver::name`: a type's name, `#<id>` for one the description has none for. */
static void loft_um_name(const loft_upmap *m, int nw, uint16_t id, loft_ub *out) {
    const loft_ld *d = loft_um_desc(m, nw);
    if (id < d->count) loft_ub_puts(out, d->n[id].name);
    else loft_ub_printf(out, "#%u", (unsigned) id);
}

static const char *loft_um_name_of(const loft_upmap *m, int nw, uint16_t id) {
    const loft_ld *d = loft_um_desc(m, nw);
    return id < d->count ? d->n[id].name : "?";
}

static int loft_um_same_name(const loft_upmap *m, uint16_t old, uint16_t nw) {
    if (old >= m->od->count || nw >= m->nd->count) return 0;
    return strcmp(m->od->n[old].name, m->nd->n[nw].name) == 0;
}

static int loft_um_strcmp(const void *a, const void *b) { return strcmp(*(char *const *) a, *(char *const *) b); }

/* `Deriver::closure`: the dump lines of `id`'s whole closure, sorted, with the facts the dump
 * does not render; equal for two types iff a value of one can be copied as the other. */
static const char *loft_um_closure(loft_upmap *m, int nw, uint16_t id) {
    const loft_ld *d = loft_um_desc(m, nw);
    char ***cache = nw ? &m->cl_new : &m->cl_old;
    if (!*cache) {
        *cache = (char **) calloc(d->count ? d->count : 1, sizeof(char *));
        if (!*cache) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    if (id < d->count && (*cache)[id]) return (*cache)[id];
    uint8_t *seen = (uint8_t *) calloc(d->count ? d->count : 1, 1);
    uint16_t *stack = (uint16_t *) malloc(((size_t) d->count * 4 + 8) * sizeof *stack);
    if (!seen || !stack) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t sp = 0;
    if (id < d->count) stack[sp++] = id;
    while (sp) {
        uint16_t t = stack[--sp];
        if (t >= d->count || seen[t]) continue;
        seen[t] = 1;
        const loft_ln *n = &d->n[t];
        switch (n->kind) {
        case LOFT_LN_RECORD:
        case LOFT_LN_EVALUE:
            for (uint16_t k = 0; k < n->nf; k++)
                if (n->f[k].content != LOFT_LD_NONE && !seen[n->f[k].content]) stack[sp++] = n->f[k].content;
            break;
        case LOFT_LN_CHOICES:
            for (uint16_t k = 0; k < n->nv; k++)
                if (n->vt[k] != LOFT_LD_NONE && !seen[n->vt[k]]) stack[sp++] = n->vt[k];
            break;
        case LOFT_LN_VECTOR:
        case LOFT_LN_ARRAY:
        case LOFT_LN_CHILD:
        case LOFT_LN_ITER:
            if (n->elem != LOFT_LD_NONE && n->elem < d->count && !seen[n->elem]) stack[sp++] = n->elem;
            break;
        default: break;
        }
    }
    size_t cap = 16, nl = 0;
    char **lines = (char **) malloc(cap * sizeof *lines);
    if (!lines) { fputs("loft: out of memory\n", stderr); abort(); }
#define LOFT_UM_LINE(s)                                                                          \
    do {                                                                                         \
        if (nl == cap) { cap *= 2; lines = (char **) realloc(lines, cap * sizeof *lines); }      \
        if (!lines) { fputs("loft: out of memory\n", stderr); abort(); }                         \
        lines[nl++] = (s);                                                                       \
    } while (0)
    int has_ref = 0;
    for (uint16_t t = 0; t < d->count; t++) has_ref |= seen[t] && d->n[t].kind == LOFT_LN_REF;
    if (has_ref) LOFT_UM_LINE(loft_up_dup(nw ? "@ref new" : "@ref old", 8));
    for (uint16_t t = 0; t < d->count; t++) {
        if (!seen[t]) continue;
        const loft_ln *n = &d->n[t];
        if (n->kind != LOFT_LN_NAME) LOFT_UM_LINE(loft_up_dup(n->line, strlen(n->line)));
        if (n->kind == LOFT_LN_RECORD || n->kind == LOFT_LN_EVALUE)
            for (uint16_t k = 0; k < n->nf; k++) {
                loft_ub b = { 0, 0, 0 };
                if (n->f[k].nullable) {
                    loft_ub_printf(&b, "%s.%s?", n->name, n->f[k].name);
                    LOFT_UM_LINE(b.p);
                }
                if (n->f[k].target != LOFT_LD_NONE) {
                    loft_ub c = { 0, 0, 0 };
                    loft_ub_printf(&c, "%s.%s->", n->name, n->f[k].name);
                    loft_um_name(m, nw, n->f[k].target, &c);
                    LOFT_UM_LINE(c.p);
                }
            }
    }
#undef LOFT_UM_LINE
    qsort(lines, nl, sizeof *lines, loft_um_strcmp);
    loft_ub all = { 0, 0, 0 };
    loft_ub_puts(&all, "");
    for (size_t i = 0; i < nl; i++) {
        if (i) loft_ub_puts(&all, "\n");
        loft_ub_puts(&all, lines[i]);
        free(lines[i]);
    }
    free(lines);
    free(seen);
    free(stack);
    if (id < d->count) (*cache)[id] = all.p;
    else {
        /* An id the description has no row for: cached nowhere, so freed by the next call. */
        static LOFT_TLS char *loose = 0;
        free(loose);
        loose = all.p;
    }
    return all.p;
}

static void loft_um_refuse(loft_upmap *m, const char *path, const char *reason) {
    if (m->refusal.n) return;
    loft_ub_printf(&m->refusal, "%s: %s", path, reason);
}

static uint32_t loft_um_push(loft_upmap *m, uint16_t old, uint16_t nw, uint8_t how) {
    if (m->np == m->cap) {
        m->cap = m->cap ? m->cap * 2 : 16;
        m->p = (loft_upair *) realloc(m->p, m->cap * sizeof *m->p);
        if (!m->p) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_upair *p = &m->p[m->np];
    memset(p, 0, sizeof *p);
    p->old = old;
    p->nw = nw;
    p->how = how;
    uint32_t at = m->np++;
    for (uint32_t i = 0; i < m->nm; i++)
        if (m->memo[i].old == old && m->memo[i].nw == nw) { m->memo[i].at = at; return at; }
    if (m->nm == m->mcap) {
        m->mcap = m->mcap ? m->mcap * 2 : 16;
        m->memo = (loft_umemo *) realloc(m->memo, m->mcap * sizeof *m->memo);
        if (!m->memo) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    m->memo[m->nm].old = old;
    m->memo[m->nm].nw = nw;
    m->memo[m->nm].at = at;
    m->nm++;
    return at;
}

static void loft_um_unmemo(loft_upmap *m, uint16_t old, uint16_t nw) {
    for (uint32_t i = 0; i < m->nm; i++)
        if (m->memo[i].old == old && m->memo[i].nw == nw) { m->memo[i] = m->memo[--m->nm]; return; }
}

static int loft_um_routine(const loft_upmap *m, uint16_t nw, uint16_t *old) {
    for (uint32_t i = 0; i < m->nrt; i++)
        if (m->rt_new[i] == nw) { if (old) *old = m->rt_old[i]; return 1; }
    return 0;
}

static int loft_um_projected(const loft_upmap *m, uint16_t nw) {
    for (uint32_t i = 0; i < m->nproj; i++) if (m->proj_new[i] == nw) return 1;
    return 0;
}

/* `number_range`: the values a number's encoding holds; 0 for a type that is not a number. */
static int loft_um_range(const loft_ln *n, int64_t *lo, int64_t *hi, int *nullable) {
    int64_t s = n->start;
    switch (n->kind) {
    case LOFT_LN_BASE:
        if (n->base != LOFT_LB_INTEGER && n->base != LOFT_LB_LONG) return 0;
        *lo = INT64_MIN + 1; *hi = INT64_MAX; *nullable = 1; return 1;
    case LOFT_LN_BYTE: *lo = s; *hi = s + 255 - n->nullable; *nullable = n->nullable; return 1;
    case LOFT_LN_SHORT: *lo = s; *hi = s + 65534; *nullable = n->nullable; return 1;
    case LOFT_LN_SHORTRAW: *lo = s; *hi = s + 65535 - n->nullable; *nullable = n->nullable; return 1;
    case LOFT_LN_INT: *lo = (int64_t) INT32_MIN + 1; *hi = INT32_MAX; *nullable = n->nullable; return 1;
    case LOFT_LN_INTRAW: *lo = 0; *hi = (int64_t) UINT32_MAX - n->nullable; *nullable = n->nullable; return 1;
    default: return 0;
    }
}

/* `kind_word`. */
static const char *loft_um_kind_word(const loft_ln *n) {
    switch (n->kind) {
    case LOFT_LN_BASE:
        switch (n->base) {
        case LOFT_LB_TEXT: return "text";
        case LOFT_LB_BOOLEAN: return "boolean";
        case LOFT_LB_CHARACTER: return "character";
        case LOFT_LB_SINGLE:
        case LOFT_LB_FLOAT: return "a float";
        default: return "an integer";
        }
    case LOFT_LN_BYTE:
    case LOFT_LN_SHORT:
    case LOFT_LN_SHORTRAW:
    case LOFT_LN_INT:
    case LOFT_LN_INTRAW: return "an integer";
    case LOFT_LN_RECORD: return "a struct";
    case LOFT_LN_EVALUE: return "an enum variant";
    case LOFT_LN_CHOICES: return "an enum";
    case LOFT_LN_VECTOR:
    case LOFT_LN_ARRAY: return "a vector";
    case LOFT_LN_ITER:
        switch (n->ikind) {
        case LOFT_LI_SORTED:
        case LOFT_LI_ORDERED: return "a sorted";
        case LOFT_LI_HASH: return "a hash";
        case LOFT_LI_INDEX: return "an index";
        case LOFT_LI_RADIX: return "a spatial";
        default: return "a trie";
        }
    case LOFT_LN_REF: return "a reference";
    case LOFT_LN_CHILD: return "a captured closure";
    default: return "nothing";
    }
}

static int loft_um_is_data(const loft_lf *f) {
    return strcmp(f->name, "enum") != 0 && f->pos != 0xFFFF && f->name[0] != '#';
}

static int64_t loft_um_pair_as(loft_upmap *m, uint16_t old, uint16_t nw, const char *path, int whole);
static int64_t loft_um_pair(loft_upmap *m, uint16_t old, uint16_t nw, const char *path) {
    return loft_um_pair_as(m, old, nw, path, 1);
}

/* `Deriver::reference_target_kept`. */
static int loft_um_ref_kept(loft_upmap *m, const char *record, const loft_lf *of, const loft_lf *nf, const char *path) {
    loft_ub at = { 0, 0, 0 };
    loft_ub_printf(&at, "%s.%s", path, nf->name);
    int ok = 1;
    if (of->target == LOFT_LD_NONE) {
        loft_ub why = { 0, 0, 0 };
        loft_ub_printf(&why,
                       "the store's recorded layout does not say what `%s.%s` points at \xe2\x80\x94 it was written "
                       "before stores recorded it; load and save it once with the program that wrote it",
                       record, of->name);
        loft_um_refuse(m, loft_ub_str(&at), loft_ub_str(&why));
        loft_ub_free(&why);
        ok = 0;
    } else {
        loft_ub was = { 0, 0, 0 }, now = { 0, 0, 0 };
        loft_um_name(m, 0, of->target, &was);
        loft_um_name(m, 1, nf->target, &now);
        if (strcmp(loft_ub_str(&was), loft_ub_str(&now)) != 0) {
            loft_ub why = { 0, 0, 0 };
            loft_ub_printf(&why, "now points at a `%s` where the stored references point at a `%s`", loft_ub_str(&now),
                           loft_ub_str(&was));
            loft_um_refuse(m, loft_ub_str(&at), loft_ub_str(&why));
            loft_ub_free(&why);
            ok = 0;
        }
        loft_ub_free(&was);
        loft_ub_free(&now);
    }
    loft_ub_free(&at);
    return ok;
}

/* `Deriver::added_and_dropped`: 0 when a record loses and gains fields in one upgrade with no
 * routine to carry a moved value — refused as a rename. */
static int loft_um_added_dropped(loft_upmap *m, const char *name, uint16_t new_id, const loft_ln *on, const loft_ln *nn,
                                 const char *path) {
    loft_ub added = { 0, 0, 0 }, dropped = { 0, 0, 0 };
    for (uint16_t i = 0; i < nn->nf; i++) {
        if (!loft_um_is_data(&nn->f[i])) continue;
        int found = 0;
        for (uint16_t k = 0; k < on->nf && !found; k++)
            found = loft_um_is_data(&on->f[k]) && strcmp(on->f[k].name, nn->f[i].name) == 0;
        if (!found) { if (added.n) loft_ub_puts(&added, ", "); loft_ub_puts(&added, nn->f[i].name); }
    }
    for (uint16_t k = 0; k < on->nf; k++) {
        if (!loft_um_is_data(&on->f[k])) continue;
        int found = 0;
        for (uint16_t i = 0; i < nn->nf && !found; i++)
            found = loft_um_is_data(&nn->f[i]) && strcmp(on->f[k].name, nn->f[i].name) == 0;
        if (!found) { if (dropped.n) loft_ub_puts(&dropped, ", "); loft_ub_puts(&dropped, on->f[k].name); }
    }
    int ok = 1;
    if (added.n && dropped.n && !loft_um_routine(m, new_id, 0)) {
        loft_ub why = { 0, 0, 0 };
        loft_ub_printf(&why,
                       "`%s` loses %s and gains %s in one upgrade, which looks like a rename; keep the old field, "
                       "give the new one a default that reads it (`new: T = $.old`), upgrade, then remove the old one",
                       name, loft_ub_str(&dropped), loft_ub_str(&added));
        loft_um_refuse(m, path, loft_ub_str(&why));
        loft_ub_free(&why);
        ok = 0;
    }
    loft_ub_free(&added);
    loft_ub_free(&dropped);
    return ok;
}

static int loft_um_old_index(const loft_ln *on, const loft_lf *f) {
    for (uint16_t k = 0; k < on->nf; k++)
        if (loft_um_is_data(&on->f[k]) && strcmp(on->f[k].name, f->name) == 0) return k;
    return -1;
}

/* `Deriver::record`: the per-field plan of a record whose name did not change. */
static int loft_um_record(loft_upmap *m, const char *name, uint16_t new_id, const loft_ln *on, const loft_ln *nn,
                          const char *path, loft_ff **plan_out, uint16_t *nplan) {
    if (!loft_um_added_dropped(m, name, new_id, on, nn, path)) return 0;
    loft_ff *plan = (loft_ff *) calloc(nn->nf ? nn->nf : 1, sizeof *plan);
    if (!plan) { fputs("loft: out of memory\n", stderr); abort(); }
    int ok = 1;
    uint16_t np = 0;
    for (uint16_t at = 0; at < nn->nf; at++) {
        const loft_lf *f = &nn->f[at];
        if (!loft_um_is_data(f)) { plan[np++].from = LOFT_FF_LAYOUT; continue; }
        /* In each index group the FIRST member with stored data is converted; the others
         * receive its records by filing. */
        long first = -1;
        for (int g = -1; g < (int) f->ngroup; g++) {
            uint16_t mbr = g < 0 ? at : f->group[g];
            if (mbr == 0xFFFF || mbr >= nn->nf) continue;
            if (loft_um_old_index(on, &nn->f[mbr]) < 0) continue;
            if (first < 0 || mbr < first) first = mbr;
        }
        if (first >= 0 && first != at) {
            plan[np].from = LOFT_FF_GROUP;
            plan[np++].source = (uint16_t) first;
            continue;
        }
        int i = loft_um_old_index(on, f);
        if (i < 0) { plan[np++].from = LOFT_FF_DEFAULT; continue; }
        if (on->f[i].content < m->od->count && m->od->n[on->f[i].content].kind == LOFT_LN_REF &&
            !loft_um_ref_kept(m, name, &on->f[i], f, path)) {
            ok = 0;
            continue;
        }
        int whole = 1;
        for (uint16_t g = 0; g < f->ngroup; g++) whole &= f->group[g] == 0xFFFF;
        loft_ub fp = { 0, 0, 0 };
        loft_ub_printf(&fp, "%s.%s", path, f->name);
        int64_t pair = loft_um_pair_as(m, on->f[i].content, f->content, loft_ub_str(&fp), whole);
        loft_ub_free(&fp);
        if (pair < 0) { ok = 0; continue; }
        plan[np].from = LOFT_FF_OLD;
        plan[np].field = (uint16_t) i;
        plan[np++].pair = (uint32_t) pair;
    }
    if (!ok) { free(plan); return 0; }
    *plan_out = plan;
    *nplan = np;
    return 1;
}

/* `Deriver::project`: the pair filling an `upgrade` routine's `old` struct `o` from a stored
 * record of old type `old`. */
static int64_t loft_um_project(loft_upmap *m, uint16_t old, const loft_ln *on, uint16_t o, const char *path) {
    if (!loft_um_has_node(m->nd, o) || m->nd->n[o].kind != LOFT_LN_RECORD) {
        loft_um_refuse(m, path, "an upgrade routine's `old` is not a struct");
        return -1;
    }
    const loft_ln *nn = &m->nd->n[o];
    uint32_t at = loft_um_push(m, old, o, LOFT_HOW_SAME);
    loft_um_unmemo(m, old, o);
    loft_ff *plan = (loft_ff *) calloc(nn->nf ? nn->nf : 1, sizeof *plan);
    if (!plan) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint16_t k = 0; k < nn->nf; k++) {
        const loft_lf *f = &nn->f[k];
        if (!loft_um_is_data(f)) { plan[k].from = LOFT_FF_LAYOUT; continue; }
        loft_ub fp = { 0, 0, 0 };
        loft_ub_printf(&fp, "%s.%s", path, f->name);
        if (f->content < m->nd->count && m->nd->n[f->content].kind == LOFT_LN_REF) {
            loft_ub why = { 0, 0, 0 };
            loft_ub_printf(&why, "`%s.%s` is a reference, which an upgrade routine cannot read from the old store",
                           nn->name, f->name);
            loft_um_refuse(m, loft_ub_str(&fp), loft_ub_str(&why));
            loft_ub_free(&why);
            loft_ub_free(&fp);
            free(plan);
            return -1;
        }
        int i = loft_um_old_index(on, f);
        if (i >= 0) {
            int64_t pair = loft_um_pair_as(m, on->f[i].content, f->content, loft_ub_str(&fp), 0);
            if (pair < 0) { loft_ub_free(&fp); free(plan); return -1; }
            plan[k].from = LOFT_FF_OLD;
            plan[k].field = (uint16_t) i;
            plan[k].pair = (uint32_t) pair;
        } else {
            plan[k].from = LOFT_FF_DEFAULT;
        }
        loft_ub_free(&fp);
    }
    m->p[at].how = LOFT_HOW_RECORD;
    m->p[at].plan = plan;
    m->p[at].nplan = nn->nf;
    return at;
}

/* `Deriver::variants`. */
static int loft_um_variants(loft_upmap *m, uint32_t at, const loft_ln *on, const loft_ln *nn, const char *path) {
    int32_t *to = (int32_t *) malloc(((size_t) on->nv + 1) * sizeof *to);
    int64_t *pairs = (int64_t *) malloc(((size_t) on->nv + 1) * sizeof *pairs);
    if (!to || !pairs) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint16_t v = 0; v < on->nv; v++) {
        int i = -1;
        for (uint16_t k = 0; k < nn->nv && i < 0; k++) if (strcmp(nn->vn[k], on->vn[v]) == 0) i = k;
        to[v] = i;
        pairs[v] = -1;
        if (i < 0) continue;
        uint16_t ot = on->vt[v], nt = nn->vt[i];
        loft_ub vp = { 0, 0, 0 };
        loft_ub_printf(&vp, "%s.%s", path, on->vn[v]);
        if (ot != LOFT_LD_NONE && nt != LOFT_LD_NONE) {
            int64_t pr = loft_um_pair(m, ot, nt, loft_ub_str(&vp));
            if (pr < 0) { loft_ub_free(&vp); free(to); free(pairs); return 0; }
            pairs[v] = pr;
        } else if (ot != nt) {
            loft_um_refuse(m, loft_ub_str(&vp), "a variant gains or loses its fields");
            loft_ub_free(&vp);
            free(to);
            free(pairs);
            return 0;
        }
        loft_ub_free(&vp);
    }
    m->p[at].how = LOFT_HOW_VARIANTS;
    m->p[at].vto = to;
    m->p[at].vpair = pairs;
    m->p[at].nv = on->nv;
    return 1;
}

static int loft_um_sorted_kind(uint8_t k) { return k == LOFT_LI_SORTED || k == LOFT_LI_ORDERED; }

/* `Deriver::how`: how a value of old node `on` becomes one of new node `nn`, written into pair
 * `at`; 0 when refused. */
static int loft_um_how(loft_upmap *m, uint32_t at, uint16_t old, uint16_t nw, const char *path) {
    const loft_ln *on = &m->od->n[old], *nn = &m->nd->n[nw];
    const char *old_name = on->name, *new_name = nn->name;
    if ((on->kind == LOFT_LN_RECORD && nn->kind == LOFT_LN_RECORD) || (on->kind == LOFT_LN_EVALUE && nn->kind == LOFT_LN_EVALUE)) {
        if (strcmp(old_name, new_name) != 0) {
            loft_ub why = { 0, 0, 0 };
            loft_ub_printf(&why, "now holds a `%s` where the store holds a `%s`", new_name, old_name);
            loft_um_refuse(m, path, loft_ub_str(&why));
            loft_ub_free(&why);
            return 0;
        }
        loft_ff *plan = 0;
        uint16_t np = 0;
        if (!loft_um_record(m, new_name, nw, on, nn, path, &plan, &np)) return 0;
        m->p[at].how = LOFT_HOW_RECORD;
        m->p[at].plan = plan;
        m->p[at].nplan = np;
        uint16_t o;
        if (loft_um_routine(m, nw, &o) && !loft_um_projected(m, nw)) {
            loft_ub up = { 0, 0, 0 };
            loft_ub_printf(&up, "%s (upgrade)", path);
            int64_t pr = loft_um_project(m, old, on, o, loft_ub_str(&up));
            loft_ub_free(&up);
            if (pr < 0) return 0;
            m->proj_new = (uint16_t *) realloc(m->proj_new, ((size_t) m->nproj + 1) * sizeof *m->proj_new);
            m->proj_pair = (uint32_t *) realloc(m->proj_pair, ((size_t) m->nproj + 1) * sizeof *m->proj_pair);
            if (!m->proj_new || !m->proj_pair) { fputs("loft: out of memory\n", stderr); abort(); }
            m->proj_new[m->nproj] = nw;
            m->proj_pair[m->nproj++] = (uint32_t) pr;
        }
        return 1;
    }
    if (on->kind == LOFT_LN_CHOICES && nn->kind == LOFT_LN_CHOICES) {
        if (strcmp(old_name, new_name) != 0) {
            loft_ub why = { 0, 0, 0 };
            loft_ub_printf(&why, "now holds a `%s` where the store holds a `%s`", new_name, old_name);
            loft_um_refuse(m, path, loft_ub_str(&why));
            loft_ub_free(&why);
            return 0;
        }
        return loft_um_variants(m, at, on, nn, path);
    }
    if (on->kind == LOFT_LN_BASE && on->base == LOFT_LB_SINGLE && nn->kind == LOFT_LN_BASE && nn->base == LOFT_LB_FLOAT) {
        m->p[at].how = LOFT_HOW_WIDEN;
        return 1;
    }
    if ((on->kind == LOFT_LN_VECTOR || on->kind == LOFT_LN_ARRAY) && (nn->kind == LOFT_LN_VECTOR || nn->kind == LOFT_LN_ARRAY)) {
        loft_ub ep = { 0, 0, 0 };
        loft_ub_printf(&ep, "%s[]", path);
        int64_t e = loft_um_pair(m, on->elem, nn->elem, loft_ub_str(&ep));
        loft_ub_free(&ep);
        if (e < 0) return 0;
        m->p[at].how = LOFT_HOW_ELEMENTS;
        m->p[at].elem = (uint32_t) e;
        return 1;
    }
    if (on->kind == LOFT_LN_ITER && nn->kind == LOFT_LN_ITER) {
        int same = on->ikind == nn->ikind || (loft_um_sorted_kind(on->ikind) && loft_um_sorted_kind(nn->ikind));
        if (!same) {
            loft_ub why = { 0, 0, 0 };
            loft_ub_printf(&why, "changes from %s to %s", loft_um_kind_word(on), loft_um_kind_word(nn));
            loft_um_refuse(m, path, loft_ub_str(&why));
            loft_ub_free(&why);
            return 0;
        }
        loft_ub ep = { 0, 0, 0 };
        loft_ub_printf(&ep, "%s[]", path);
        int64_t e = loft_um_pair(m, on->elem, nn->elem, loft_ub_str(&ep));
        loft_ub_free(&ep);
        if (e < 0) return 0;
        m->p[at].how = LOFT_HOW_ELEMENTS;
        m->p[at].elem = (uint32_t) e;
        return 1;
    }
    if (on->kind == LOFT_LN_REF && nn->kind == LOFT_LN_REF) {
        m->p[at].how = LOFT_HOW_REFERENCE;
        return 1;
    }
    int64_t olo, ohi, nlo, nhi;
    int onull, nnull;
    if (loft_um_range(on, &olo, &ohi, &onull) && loft_um_range(nn, &nlo, &nhi, &nnull)) {
        loft_ub why = { 0, 0, 0 };
        if (nlo > olo || nhi < ohi)
            loft_ub_printf(&why, "narrows from `%s` to `%s`: a stored value may not fit", old_name, new_name);
        else if (onull && !nnull)
            loft_ub_printf(&why, "`%s` cannot hold null, and the stored `%s` can", new_name, old_name);
        if (why.n) {
            loft_um_refuse(m, path, loft_ub_str(&why));
            loft_ub_free(&why);
            return 0;
        }
        m->p[at].how = LOFT_HOW_WIDEN;
        return 1;
    }
    loft_ub why = { 0, 0, 0 };
    loft_ub_printf(&why, "changes from %s to %s", loft_um_kind_word(on), loft_um_kind_word(nn));
    loft_um_refuse(m, path, loft_ub_str(&why));
    loft_ub_free(&why);
    return 0;
}

static int loft_um_leaf(const loft_ln *n) {
    switch (n->kind) {
    case LOFT_LN_BASE:
    case LOFT_LN_BYTE:
    case LOFT_LN_SHORT:
    case LOFT_LN_SHORTRAW:
    case LOFT_LN_INT:
    case LOFT_LN_INTRAW: return 1;
    case LOFT_LN_CHOICES:
        for (uint16_t k = 0; k < n->nv; k++) if (n->vt[k] != LOFT_LD_NONE) return 0;
        return 1;
    default: return 0;
    }
}

/* `placement::hash_reads` / `trie_reads`: a keyed collection of another build's placement this
 * build can still walk. */
static int loft_um_readable(const loft_ln *n) {
    if (!n->placement || n->kind != LOFT_LN_ITER) return 0;
    if (n->ikind == LOFT_LI_HASH) return strcmp(n->placement, LOFT_PL_HASH) == 0 || strcmp(n->placement, "2") == 0;
    if (n->ikind == LOFT_LI_TRIE) return strcmp(n->placement, LOFT_PL_TRIE) == 0 || strcmp(n->placement, "1") == 0;
    return 0;
}

/* `Deriver::pair_as`: pair `old` with `nw` at `path`; its index, or -1 when refused.  `whole` off
 * never copies the value whole (a group's source member is filed record by record). */
static int64_t loft_um_pair_as(loft_upmap *m, uint16_t old, uint16_t nw, const char *path, int whole) {
    if (whole)
        for (uint32_t i = 0; i < m->nm; i++)
            if (m->memo[i].old == old && m->memo[i].nw == nw) return m->memo[i].at;
    if (!loft_um_has_node(m->od, old) || !loft_um_has_node(m->nd, nw)) {
        loft_um_refuse(m, path, "the layout does not describe this type");
        return -1;
    }
    const loft_ln *on = &m->od->n[old], *nn = &m->nd->n[nw];
    if (on->placement && !loft_um_readable(on)) {
        loft_ub why = { 0, 0, 0 };
        loft_ub_printf(&why,
                       "the store's %s was written by a build that lays it out differently (placement %s), which "
                       "this build cannot read",
                       loft_um_kind_word(on), on->placement);
        loft_um_refuse(m, path, loft_ub_str(&why));
        loft_ub_free(&why);
        return -1;
    }
    if (loft_um_same_name(m, old, nw) && ((m->whole_copies && whole) || loft_um_leaf(on)) &&
        strcmp(loft_um_closure(m, 0, old), loft_um_closure(m, 1, nw)) == 0)
        return loft_um_push(m, old, nw, LOFT_HOW_SAME);
    (void) nn;
    /* The slot is reserved first, so a recursive type pairs with itself. */
    uint32_t at = loft_um_push(m, old, nw, LOFT_HOW_SAME);
    return loft_um_how(m, at, old, nw, path) ? (int64_t) at : -1;
}

/* `UpgradeMap::derive_with` from `old` (rooted at `old_root`) to `nw` (at `new_root`). */
static void loft_um_derive(loft_upmap *m, const loft_ld *old, uint16_t old_root, const loft_ld *nw, uint16_t new_root,
                           const uint16_t *rt_new, const uint16_t *rt_old, uint32_t nrt, int whole_copies) {
    memset(m, 0, sizeof *m);
    m->whole_copies = whole_copies;
    m->od = old;
    m->nd = nw;
    m->rt_new = rt_new;
    m->rt_old = rt_old;
    m->nrt = nrt;
    loft_ub root = { 0, 0, 0 };
    const char *nn = loft_um_name_of(m, 1, new_root);
    if (strcmp(loft_um_name_of(m, 0, old_root), nn) == 0) loft_ub_puts(&root, nn);
    else loft_ub_printf(&root, "%s (now %s)", loft_um_name_of(m, 0, old_root), nn);
    (void) loft_um_pair(m, old_root, new_root, loft_ub_str(&root));
    loft_ub_free(&root);
}

/* The parse table the converter builds records through, written after it (`loft_ptypes`). */
static const loft_ptype *loft_up_ptypes(void);

/* ── What a converting load owes the program's drain (`Owed`, src/database/upgrade.rs) ── */
typedef struct { loft_ref rec; int64_t key; loft_ref old; } loft_uowe;
static LOFT_TLS loft_uowe *loft_uo;
static LOFT_TLS size_t loft_uo_n, loft_uo_cap, loft_uo_at;
static LOFT_TLS loft_ref loft_uo_from = { 0xFFFF, 0, 0 };
static LOFT_TLS int loft_uo_scratch = -1;

/* The `upgrade` routines the drain's wrapper registered (`Stores::upgrade_routine`). */
static LOFT_TLS uint16_t *loft_ur_tp, *loft_ur_old, *loft_ur_ver;
static LOFT_TLS size_t loft_ur_n;

static void loft_uo_push(loft_ref rec, int64_t key, loft_ref old) {
    if (loft_uo_n == loft_uo_cap) {
        loft_uo_cap = loft_uo_cap ? loft_uo_cap * 2 : 32;
        loft_uo = (loft_uowe *) realloc(loft_uo, loft_uo_cap * sizeof *loft_uo);
        if (!loft_uo) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_uowe o = { rec, key, old };
    loft_uo[loft_uo_n++] = o;
}

/* `reset_owed`: forget what a drain still owed, releasing the routines' scratch store. */
static void loft_uo_reset(void) {
    loft_uo_n = 0;
    loft_uo_at = 0;
    loft_uo_from = loft_ref_null();
    if (loft_uo_scratch >= 0) loft_store_drop((uint16_t) loft_uo_scratch);
    loft_uo_scratch = -1;
}

/* `owed_key` / `routine_key`: the key a drain matches an owed fill by. */
static int64_t loft_uo_key(uint16_t tp, uint16_t field) { return ((int64_t) tp << 16) | field; }

static int64_t loft_store_upgrade_owed_next(void) {
    if (loft_uo_at >= loft_uo_n) return -1;
    return loft_uo[loft_uo_at++].key;
}

static loft_ref loft_store_upgrade_owed_record(void) {
    return loft_uo_at > 0 && loft_uo_at <= loft_uo_n ? loft_uo[loft_uo_at - 1].rec : loft_ref_null();
}

static loft_ref loft_store_upgrade_owed_old(void) {
    return loft_uo_at > 0 && loft_uo_at <= loft_uo_n ? loft_uo[loft_uo_at - 1].old : loft_ref_null();
}

static loft_ref loft_store_upgrade_owed_from(void) { return loft_uo_from; }

static uint8_t loft_store_upgrade_owed_finish(void) {
    loft_uo_reset();
    return 1;
}

static void loft_store_upgrade_routine(int64_t tp, int64_t old, int64_t version) {
    /* A program can call this itself: numbers that name no type register nothing. */
    if (tp < 0 || old < 0 || version < 0 || tp >= (int64_t) loft_ntn || old >= (int64_t) loft_ntn ||
        version >= (int64_t) loft_ntn)
        return;
    for (size_t i = 0; i < loft_ur_n; i++)
        if (loft_ur_tp[i] == tp) { loft_ur_old[i] = (uint16_t) old; loft_ur_ver[i] = (uint16_t) version; return; }
    loft_ur_tp = (uint16_t *) realloc(loft_ur_tp, (loft_ur_n + 1) * sizeof *loft_ur_tp);
    loft_ur_old = (uint16_t *) realloc(loft_ur_old, (loft_ur_n + 1) * sizeof *loft_ur_old);
    loft_ur_ver = (uint16_t *) realloc(loft_ur_ver, (loft_ur_n + 1) * sizeof *loft_ur_ver);
    if (!loft_ur_tp || !loft_ur_old || !loft_ur_ver) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_ur_tp[loft_ur_n] = (uint16_t) tp;
    loft_ur_old[loft_ur_n] = (uint16_t) old;
    loft_ur_ver[loft_ur_n++] = (uint16_t) version;
}

/* ── The converter (`Stores::upgrade_store_run`) ── */

/* Where a collection's elements are filed: field `f` of the `tp` record `rec`, or the
 * collection `rec` of type `tp` itself (`f` 0xFFFF). */
typedef struct { loft_ref rec; uint16_t tp, f; } loft_ucont;

typedef struct { uint32_t old; loft_ref nw; } loft_urec;
typedef struct { loft_ref holder; uint32_t sn, rec, pos; } loft_ufix;

typedef struct {
    const loft_upmap *m;
    const loft_ld *od, *nd;
    const loft_ptype *P;
    const loft_tdesc *T;
    loft_pw w;
    long self_slot;
    uint16_t old_nr, dst_nr;
    /* old record number → the new record converted from it (open addressing; 0 is empty) */
    loft_urec *recs;
    size_t rcap, rn;
    loft_ufix *fix;
    size_t nfix, fcap;
    int projecting;
    const char *from; /* the release the store records */
    loft_ub err;
} loft_urun;

static void loft_uc_fail(loft_urun *r, const char *why) {
    if (!r->err.n) loft_ub_puts(&r->err, why);
}

static loft_urec *loft_uc_rec_slot(loft_urun *r, uint32_t old) {
    size_t i = ((size_t) old * 2654435761u) & (r->rcap - 1);
    while (r->recs[i].old != 0 && r->recs[i].old != old) i = (i + 1) & (r->rcap - 1);
    return &r->recs[i];
}

static void loft_uc_rec_put(loft_urun *r, uint32_t old, loft_ref nw) {
    if ((r->rn + 1) * 2 > r->rcap) {
        loft_urec *was = r->recs;
        size_t wcap = r->rcap;
        r->rcap = r->rcap ? r->rcap * 2 : 64;
        r->recs = (loft_urec *) calloc(r->rcap, sizeof *r->recs);
        if (!r->recs) { fputs("loft: out of memory\n", stderr); abort(); }
        for (size_t i = 0; i < wcap; i++) if (was[i].old) *loft_uc_rec_slot(r, was[i].old) = was[i];
        free(was);
    }
    loft_urec *s = loft_uc_rec_slot(r, old);
    if (s->old == 0) r->rn++;
    s->old = old;
    s->nw = nw;
}

static loft_urec *loft_uc_rec_get(loft_urun *r, uint32_t old) {
    if (!r->rcap || old == 0) return 0;
    loft_urec *s = loft_uc_rec_slot(r, old);
    return s->old == old ? s : 0;
}

static uint16_t loft_uc_prog(const loft_urun *r, uint16_t nid) { return nid < r->nd->count ? r->nd->n[nid].prog : 0xFFFF; }

static int loft_uc_convert(loft_urun *r, uint32_t pair, loft_ref from, loft_ref to, loft_ucont into);

/* `widen`: a number read through the old encoding and written through the new one. */
static void loft_uc_widen(loft_urun *r, const loft_ln *on, const loft_ln *nn, loft_ref from, loft_ref to) {
    loft_store_t *os = &loft_stores[from.store_nr], *ns = loft_store_of(to);
    uint8_t *src = loft_at(os, from.rec, from.pos), *dst = loft_at(ns, to.rec, to.pos);
    (void) r;
    if (on->kind == LOFT_LN_BASE && on->base == LOFT_LB_SINGLE) {
        float g;
        memcpy(&g, src, 4);
        double f = (double) g;
        memcpy(dst, &f, 8);
        return;
    }
    int64_t v;
    uint8_t b;
    uint16_t h;
    int32_t i;
    uint32_t u;
    switch (on->kind) {
    case LOFT_LN_BYTE:
        b = src[0];
        v = on->nullable && b == 255 ? INT64_MIN : (int64_t) b + on->start;
        break;
    case LOFT_LN_SHORT:
        memcpy(&h, src, 2);
        v = on->nullable && h == 0 ? INT64_MIN : h == 0 ? (int64_t) INT32_MIN : (int64_t) h + on->start - 1;
        break;
    case LOFT_LN_SHORTRAW:
        memcpy(&h, src, 2);
        v = on->nullable && h == 0xFFFF ? INT64_MIN : (int64_t) h + on->start;
        break;
    case LOFT_LN_INT:
        memcpy(&i, src, 4);
        v = on->nullable && i == INT32_MIN ? INT64_MIN : (int64_t) i;
        break;
    case LOFT_LN_INTRAW:
        memcpy(&u, src, 4);
        v = on->nullable && u == UINT32_MAX ? INT64_MIN : (int64_t) u;
        break;
    default: memcpy(&v, src, 8); break;
    }
    /* `write_narrow_value`: the value as the new encoding stores it; null is the wide null. */
    int32_t n32 = v == INT64_MIN ? INT32_MIN : (int32_t) v;
    int32_t min = nn->start;
    switch (nn->kind) {
    case LOFT_LN_BYTE:
        dst[0] = n32 == INT32_MIN ? 255 : (n32 >= min && n32 <= min + 255) ? (uint8_t) (n32 - min) : 0;
        return;
    case LOFT_LN_SHORT:
        h = n32 == INT32_MIN ? 0 : (n32 >= min && n32 <= min + 65534) ? (uint16_t) (n32 - min + 1) : 0;
        memcpy(dst, &h, 2);
        return;
    case LOFT_LN_SHORTRAW:
        h = n32 == INT32_MIN ? 0xFFFF : (n32 >= min && n32 <= min + 65535) ? (uint16_t) (n32 - min) : 0;
        memcpy(dst, &h, 2);
        return;
    case LOFT_LN_INT:
        memcpy(dst, &n32, 4);
        return;
    case LOFT_LN_INTRAW:
        u = v == INT64_MIN ? UINT32_MAX : (uint32_t) v;
        memcpy(dst, &u, 4);
        return;
    default: memcpy(dst, &v, 8); return;
    }
}

/* `old_elements`: the elements of the old collection at `from` (`loft_ref`s to their payloads),
 * in the order the new collection receives them; 0 with `*absent` set for an absent collection. */
static int loft_uc_ref_by_place(const void *a, const void *b) {
    const loft_ref *x = (const loft_ref *) a, *y = (const loft_ref *) b;
    if (x->rec != y->rec) return x->rec < y->rec ? -1 : 1;
    return x->pos < y->pos ? -1 : x->pos > y->pos;
}

static loft_ref *loft_uc_old_elements(loft_urun *r, const loft_ln *on, loft_ref from, uint32_t *n, int *absent) {
    loft_store_t *s = &loft_stores[from.store_nr];
    uint32_t cur = loft_u32(s, from.rec, from.pos);
    *n = 0;
    *absent = cur == 0xFFFFFFFFu;
    if (cur == 0 || *absent) return 0;
    loft_ref *out = 0;
    int k = on->kind == LOFT_LN_ITER ? on->ikind + 100 : on->kind;
    if (k == LOFT_LN_VECTOR || k == LOFT_LI_SORTED + 100) {
        uint32_t size = on->elem < r->od->count ? r->od->n[on->elem].size : 0, len = loft_u32(s, cur, 4);
        out = (loft_ref *) malloc(((size_t) len + 1) * sizeof *out);
        if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
        for (uint32_t i = 0; i < len; i++) { loft_ref e = { from.store_nr, cur, 8 + size * i }; out[i] = e; }
        *n = len;
    } else if (k == LOFT_LN_ARRAY || k == LOFT_LI_ORDERED + 100) {
        uint32_t len = loft_u32(s, cur, 4);
        out = (loft_ref *) malloc(((size_t) len + 1) * sizeof *out);
        if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
        for (uint32_t i = 0; i < len; i++) { loft_ref e = { from.store_nr, loft_u32(s, cur, 8 + 4 * i), 8 }; out[i] = e; }
        *n = len;
    } else if (k == LOFT_LI_HASH + 100) {
        /* In the source's own record order; an inline table lists its entries in insertion
         * order already (`(Col-Inline)`). */
        uint32_t cnt = 0;
        loft_hent *all = loft_h_entries_of(from, &cnt);
        out = (loft_ref *) malloc(((size_t) cnt + 1) * sizeof *out);
        if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
        for (uint32_t i = 0; i < cnt; i++) out[i] = all[i].e;
        free(all);
        if (!loft_h_hdr(s, cur).inl) qsort(out, cnt, sizeof *out, loft_uc_ref_by_place);
        *n = cnt;
    } else if (k == LOFT_LI_INDEX + 100 || k == LOFT_LI_RADIX + 100 || k == LOFT_LI_TRIE + 100) {
        uint32_t cnt = 0, *recs = k == LOFT_LI_INDEX + 100 ? loft_bt_records(from, &cnt) : loft_rx_records(from, &cnt);
        out = (loft_ref *) malloc(((size_t) cnt + 1) * sizeof *out);
        if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
        for (uint32_t i = 0; i < cnt; i++) { loft_ref e = { from.store_nr, recs[i], 8 }; out[i] = e; }
        free(recs);
        *n = cnt;
    }
    return out;
}

/* `collection_count`: how many elements the new collection at `slot` of type `tp` holds. */
static int64_t loft_uc_count(const loft_urun *r, loft_ref slot, uint16_t tp) {
    const loft_ptype *c = &r->P[tp];
    loft_store_t *s = loft_store_of(slot);
    if (c->kind == LOFT_P_VECTOR || (c->kind == LOFT_P_KEYED && (c->keyed == LOFT_KEYED_SORTED || c->keyed == LOFT_KEYED_ORDERED || c->keyed == LOFT_P_ARRAY))) {
        uint32_t cur = loft_collection_rec(s, slot.rec, slot.pos);
        return cur ? loft_u32(s, cur, 4) : 0;
    }
    if (c->kind != LOFT_P_KEYED) return 0;
    if (c->keyed == LOFT_KEYED_HASH) return loft_hash_len(slot);
    if (c->keyed == LOFT_KEYED_INDEX) return loft_bt_len(slot);
    return loft_rx_count(slot);
}

/* A ref inside the element staged at `stage` now names the filed element `filed`, at the same
 * offset into it. */
static void loft_uc_moved(loft_ref *d, loft_ref stage, loft_ref filed) {
    if (d->store_nr == stage.store_nr && d->rec == stage.rec && d->pos >= stage.pos) {
        d->store_nr = filed.store_nr;
        d->pos = filed.pos + (d->pos - stage.pos);
        d->rec = filed.rec;
    }
}

/* The key of `rec` under the collection's keys, as the duplicate refusal spells it. */
static void loft_uc_key_text(loft_ub *b, loft_ref rec, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(rec);
    for (int i = 0; i < nk; i++) {
        if (i) loft_ub_puts(b, ", ");
        loft_kv kv = loft_key_get(s, rec.rec, rec.pos, &keys[i]);
        if (kv.kind == LOFT_KV_TEXT) {
            loft_ub_puts(b, "\"");
            loft_ub_put(b, kv.s.p ? kv.s.p : "", kv.s.len);
            loft_ub_puts(b, "\"");
        } else if (kv.kind == LOFT_KV_INT) {
            loft_ub_printf(b, "%lld", (long long) kv.i);
        } else {
            loft_text t = { 0, 0, 0 };
            loft_push_shortest(&t, kv.kind == LOFT_KV_SINGLE ? (double) kv.g : kv.f, kv.kind == LOFT_KV_SINGLE);
            loft_ub_put(b, t.p ? t.p : "", t.len);
            loft_text_free(&t);
        }
    }
}

/* `convert_elements`: the old collection at `from` rebuilt element by element, each filed
 * through the program's own allocate-and-file pair. */
static int loft_uc_elements(loft_urun *r, uint32_t elem, loft_ref from, const loft_ln *on, loft_ucont into) {
    loft_ref parent = into.rec;
    uint16_t parent_tp = into.tp, field = into.f;
    loft_ref slot = into.f == 0xFFFF ? into.rec : loft_p_field_ref(&r->w, into.rec, into.tp, into.f);
    uint16_t coll = field == 0xFFFF ? parent_tp : r->P[parent_tp].fields[field].content;
    const loft_ptype *c = &r->P[coll];
    uint32_t n = 0;
    int absent = 0;
    loft_ref *els = loft_uc_old_elements(r, on, from, &n, &absent);
    if (absent) {
        loft_set_u32(loft_store_of(slot), slot.rec, slot.pos, 0xFFFFFFFFu);
        return 1;
    }
    uint32_t esize = c->elem != 0xFFFF ? r->P[c->elem].size : 0;
    int hash = c->kind == LOFT_P_KEYED && c->keyed == LOFT_KEYED_HASH;
    /* Sized for the whole collection first, nothing moves while it fills, so a reference taken
     * to an element — an owed fill, a mapped record — stays true. */
    if (n && hash) loft_hash_reserve(r->T, coll, slot, n);
    else if (n && (c->kind == LOFT_P_VECTOR || (c->kind == LOFT_P_KEYED && c->keyed == LOFT_KEYED_SORTED)))
        loft_vec_reserve(slot, (int64_t) n + 1, esize);
    const loft_upair *ep = &r->m->p[elem];
    uint16_t elem_tp = loft_uc_prog(r, ep->nw);
    int ok = 1;
    for (uint32_t i = 0; i < n && ok; i++) {
        loft_ref src = els[i];
        int64_t before = loft_uc_count(r, slot, coll);
        loft_ref rec = loft_pw_new(&r->w, parent, parent_tp, field);
        if (rec.rec == 0) { loft_uc_fail(r, "a collection of the new store could not take an element"); ok = 0; break; }
        if (esize) memset(loft_at(loft_store_of(rec), rec.rec, rec.pos), 0, esize);
        if (src.pos == 8 && !r->projecting) loft_uc_rec_put(r, src.rec, rec);
        size_t owed_from = loft_uo_n, fix_from = r->nfix;
        loft_ucont at = { rec, elem_tp, 0xFFFF };
        if (!loft_uc_convert(r, elem, src, rec, at)) { ok = 0; break; }
        /* The key, copied before filing: an inline table moves the staged entry into itself. */
        loft_kv kv[8];
        char *own[8];
        int nk = c->nk > 8 ? 8 : c->nk;
        if (hash && !c->linked)
            for (int k = 0; k < nk; k++) {
                kv[k] = loft_key_get(loft_store_of(rec), rec.rec, rec.pos, &c->keys[k]);
                own[k] = 0;
                if (kv[k].kind == LOFT_KV_TEXT) {
                    own[k] = loft_up_dup(kv[k].s.p ? kv[k].s.p : "", kv[k].s.len);
                    kv[k].s.p = own[k];
                }
            }
        loft_pw_finish(&r->w, parent, parent_tp, field, rec);
        loft_ref filed = rec;
        if (hash && !c->linked) {
            loft_ref found = loft_h_find(slot, c->keys, kv, nk);
            if (found.rec != 0) filed = found;
            for (int k = 0; k < nk; k++) free(own[k]);
        }
        if (filed.rec != rec.rec || filed.pos != rec.pos || filed.store_nr != rec.store_nr) {
            for (size_t k = owed_from; k < loft_uo_n; k++) {
                loft_uc_moved(&loft_uo[k].rec, rec, filed);
                loft_uc_moved(&loft_uo[k].old, rec, filed);
            }
            for (size_t k = fix_from; k < r->nfix; k++) loft_uc_moved(&r->fix[k].holder, rec, filed);
            loft_urec *m = src.pos == 8 ? loft_uc_rec_get(r, src.rec) : 0;
            if (m) loft_uc_moved(&m->nw, rec, filed);
        }
        if (loft_uc_count(r, slot, coll) == before) {
            loft_ub why = { 0, 0, 0 };
            if (field != 0xFFFF) loft_ub_printf(&why, "%s.%s", loft_tn[parent_tp], r->P[parent_tp].fields[field].name);
            else loft_ub_puts(&why, loft_tn[coll]);
            loft_ub_puts(&why, ": two stored records share the key [");
            loft_uc_key_text(&why, filed, c->keys, c->nk);
            loft_ub_puts(&why, "] under the new definition, so one of them would be lost");
            loft_uc_fail(r, loft_ub_str(&why));
            loft_ub_free(&why);
            ok = 0;
        }
    }
    free(els);
    return ok;
}

/* `version_triple` of the store's release, (0, 0, 0) when it is none. */
static void loft_uc_triple(const char *v, int64_t t[3]) {
    unsigned long u[3] = { 0, 0, 0 };
    if (!loft_version_triple(v, strlen(v), u)) u[0] = u[1] = u[2] = 0;
    for (int i = 0; i < 3; i++) t[i] = (int64_t) u[i];
}

/* `upgrade_scratch`: the store of the routines' records, made on first use with the `Release`
 * of the store's release as its first record. */
static uint16_t loft_uc_scratch(loft_urun *r, uint16_t version_tp) {
    if (loft_uo_scratch >= 0) return (uint16_t) loft_uo_scratch;
    uint32_t words = (8 + r->P[version_tp].size + 7) / 8;
    if (words < 2) words = 2;
    uint16_t nr = loft_store_scratch(words + 3);
    loft_store_t *s = &loft_stores[nr];
    uint32_t rec = loft_st_claim(s, words);
    loft_zero_fill(s, rec);
    int64_t t[3];
    loft_uc_triple(r->from, t);
    const loft_ptype *vt = &r->P[version_tp];
    for (uint16_t f = 0; f < vt->n && f < 3; f++) {
        uint8_t *at = loft_at(s, rec, 8 + vt->fields[f].pos);
        if (r->P[vt->fields[f].content].size == 4) { int32_t v = (int32_t) t[f]; memcpy(at, &v, 4); }
        else memcpy(at, &t[f], 8);
    }
    s->known = version_tp;
    loft_uo_scratch = nr;
    loft_ref from = { nr, rec, 8 };
    loft_uo_from = from;
    return nr;
}

/* `owe_routine`: the `old` record of `item`'s routine filled from the stored record at `from`
 * through projection pair `proj`, and the routine call owed to the drain. */
static int loft_uc_owe_routine(loft_urun *r, uint32_t proj, loft_ref from, loft_ref item, uint16_t tp, uint16_t nid) {
    uint16_t old_tp = 0xFFFF, version_tp = 0xFFFF;
    (void) nid;
    for (size_t i = 0; i < loft_ur_n; i++)
        if (loft_ur_tp[i] == tp) { old_tp = loft_ur_old[i]; version_tp = loft_ur_ver[i]; }
    if (old_tp == 0xFFFF || version_tp == 0xFFFF) return 1;
    uint16_t sc = loft_uc_scratch(r, version_tp);
    loft_store_t *s = &loft_stores[sc];
    uint32_t words = (8 + r->P[old_tp].size + 7) / 8;
    if (words < 2) words = 2;
    uint32_t rec = loft_st_claim(s, words);
    loft_zero_fill(&loft_stores[sc], rec);
    loft_ref old = { sc, rec, 8 };
    int was = r->projecting;
    r->projecting = 1;
    loft_ucont at = { old, old_tp, 0xFFFF };
    int ok = loft_uc_convert(r, proj, from, old, at);
    r->projecting = was;
    if (!ok) return 0;
    loft_uo_push(item, loft_uo_key(tp, 0xFFFF), old);
    return 1;
}

/* `convert`: the value at `from` (old layout) through pair `pair` into `to` (new layout). */
static int loft_uc_convert(loft_urun *r, uint32_t pair, loft_ref from, loft_ref to, loft_ucont into) {
    const loft_upair *p = &r->m->p[pair];
    const loft_ln *on = &r->od->n[p->old], *nn = &r->nd->n[p->nw];
    uint16_t new_tp = loft_uc_prog(r, p->nw);
    if (new_tp == 0xFFFF) { loft_uc_fail(r, "the program's layout names a type this build did not describe"); return 0; }
    switch (p->how) {
    case LOFT_HOW_SAME:
        loft_copy_bytes(to, 0, from, 0, nn->size);
        loft_copy_claims(r->T, new_tp, from, to);
        return 1;
    case LOFT_HOW_WIDEN: loft_uc_widen(r, on, nn, from, to); return 1;
    case LOFT_HOW_REFERENCE: {
        loft_store_t *s = &loft_stores[from.store_nr];
        uint32_t sn = loft_u32(s, from.rec, from.pos), rec = loft_u32(s, from.rec, from.pos + 4),
                 pos = loft_u32(s, from.rec, from.pos + 8);
        loft_store_t *t = loft_store_of(to);
        if ((uint16_t) sn == 0xFFFF && rec == 0) {
            loft_set_u32(t, to.rec, to.pos, 0xFFFF);
            loft_set_u32(t, to.rec, to.pos + 4, 0);
            loft_set_u32(t, to.rec, to.pos + 8, 0);
            return 1;
        }
        if (r->nfix == r->fcap) {
            r->fcap = r->fcap ? r->fcap * 2 : 16;
            r->fix = (loft_ufix *) realloc(r->fix, r->fcap * sizeof *r->fix);
            if (!r->fix) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        loft_ufix f = { to, sn, rec, pos };
        r->fix[r->nfix++] = f;
        return 1;
    }
    case LOFT_HOW_VARIANTS: {
        int raw = (int) *loft_at(&loft_stores[from.store_nr], from.rec, from.pos);
        int i = raw - 1;
        uint8_t *dst = loft_at(loft_store_of(to), to.rec, to.pos);
        if (i >= 0 && i < (int) p->nv && p->vto[i] >= 0) {
            if (p->vpair[i] >= 0) return loft_uc_convert(r, (uint32_t) p->vpair[i], from, to, into);
            *dst = (uint8_t) (p->vto[i] + 1);
        } else {
            *dst = 0;
        }
        return 1;
    }
    case LOFT_HOW_RECORD: {
        for (uint16_t i = 0; i < p->nplan; i++) {
            const loft_ff *ff = &p->plan[i];
            const loft_lf *nf = &nn->f[i];
            loft_ref field_to = to;
            field_to.pos += nf->pos;
            switch (ff->from) {
            case LOFT_FF_OLD: {
                loft_ref field_from = from;
                field_from.pos += on->f[ff->field].pos;
                loft_ucont c = { to, new_tp, i };
                if (!loft_uc_convert(r, ff->pair, field_from, field_to, c)) return 0;
                break;
            }
            case LOFT_FF_GROUP: break;
            case LOFT_FF_DEFAULT:
                /* Every added field is owed: the drain calls the fill of those whose default
                 * the program computes, and passes over the rest. */
                loft_uo_push(to, loft_uo_key(new_tp, i), loft_ref_null());
                loft_pw_absent(&r->w, loft_uc_prog(r, nf->content), new_tp, i, field_to);
                break;
            default:
                if (strcmp(nf->name, "enum") == 0 && nn->kind == LOFT_LN_EVALUE)
                    *loft_at(loft_store_of(field_to), field_to.rec, field_to.pos) = nn->tag;
                break;
            }
        }
        for (uint32_t k = 0; k < r->m->nproj; k++)
            if (r->m->proj_new[k] == p->nw) return loft_uc_owe_routine(r, r->m->proj_pair[k], from, to, new_tp, p->nw);
        return 1;
    }
    case LOFT_HOW_ELEMENTS: return loft_uc_elements(r, p->elem, from, on, into);
    default: return 0;
    }
}

/* `Release::is_newer_than`: this program's release is after `written` (any release is after a
 * store that records none). */
static int loft_up_newer(const char *mine, const char *written) {
    unsigned long me[3], w[3];
    if (!mine || !loft_version_triple(mine, strlen(mine), me)) return 0;
    if (!loft_version_triple(written, strlen(written), w)) return 1;
    return w[0] < me[0] || (w[0] == me[0] && (w[1] < me[1] || (w[1] == me[1] && w[2] < me[2])));
}

/* The program's own description (`loft_upgrade::desc`) read into `nd`, each type given its type
 * number, and its node for type `tp` in `*root`; 0 with `why` set. */
static int loft_up_own(const loft_upgrade *up, uint16_t tp, loft_ld *nd, uint16_t *root, loft_ub *why) {
    const char *facts = strstr(up->desc, "--facts--\n");
    char *dtext = loft_up_dup(up->desc, facts ? (size_t) (facts - up->desc) : strlen(up->desc));
    loft_ub perr = { 0, 0, 0 };
    int ok = loft_ld_parse(nd, dtext, &perr) && (!facts || loft_ld_facts(nd, facts + 10, &perr));
    free(dtext);
    if (!ok) {
        loft_ub_printf(why, "this program's own layout does not read: %s", loft_ub_str(&perr));
        loft_ub_free(&perr);
        return 0;
    }
    loft_ub_free(&perr);
    for (uint16_t i = 0; i < nd->count; i++)
        for (uint32_t k = 0; k < up->n; k++)
            if (strcmp(up->names[k], nd->n[i].name) == 0) { nd->n[i].prog = up->types[k]; break; }
    *root = LOFT_LD_NONE;
    for (uint16_t i = 0; i < nd->count && *root == LOFT_LD_NONE; i++)
        if (nd->n[i].prog == tp && nd->n[i].kind != LOFT_LN_NAME) *root = i;
    if (*root == LOFT_LD_NONE) {
        loft_ub_printf(why, "this program does not describe the store's root type");
        return 0;
    }
    return 1;
}

/* `upgrade_into_new_store` with `repoint_references`' checks: the store `old_nr` (rooted at
 * `old_root` of `m`'s old description) converted into a fresh store of root type `tp`, answered;
 * 0xFFFF with `why` set and nothing left allocated.  The references are not written yet: the
 * caller names the store they land in (`loft_up_repoint`). */
static uint16_t loft_up_convert(loft_urun *run, const loft_upmap *m, const loft_ld *od, const loft_ld *nd,
                                uint16_t new_root, uint16_t tp, uint16_t old_nr, long self_slot, const char *from,
                                loft_ub *why) {
    uint32_t words = (8 + (uint32_t) nd->n[new_root].size + 7) / 8;
    if (words < 2) words = 2;
    uint16_t dst_nr = loft_store_scratch(words + 3);
    loft_store_t *d = &loft_stores[dst_nr];
    uint32_t root = loft_st_claim(d, words);
    if (root != LOFT_ST_PRIMARY) {
        loft_ub_puts(why, "a fresh store did not hand out its first record");
        loft_store_drop(dst_nr);
        return 0xFFFF;
    }
    loft_zero_fill(d, root);
    /* The root record's back-pointer word travels with it. */
    loft_set_u32(d, root, 4, loft_u32(&loft_stores[old_nr], LOFT_ST_PRIMARY, 4));
    run->m = m;
    run->od = od;
    run->nd = nd;
    run->P = loft_up_ptypes();
    run->T = loft_parse_types();
    run->w.P = run->P;
    run->w.unfilled = 0;
    run->self_slot = self_slot;
    run->old_nr = old_nr;
    run->dst_nr = dst_nr;
    run->from = from;
    loft_ref src = { old_nr, LOFT_ST_PRIMARY, 8 }, to = { dst_nr, LOFT_ST_PRIMARY, 8 };
    loft_ucont at = { to, tp, 0xFFFF };
    int ok = loft_uc_convert(run, 0, src, to, at);
    if (!ok) loft_ub_puts(why, run->err.n ? loft_ub_str(&run->err) : "the conversion did not finish");
    /* Each converted reference must name a record the new store holds. */
    for (size_t i = 0; ok && i < run->nfix; i++) {
        loft_ufix *f = &run->fix[i];
        loft_ref named = { old_nr, f->rec, f->pos };
        if (loft_record_removed(named)) { f->sn = 0xFFFFFFFFu; continue; }
        if (self_slot < 0 || (long) (uint16_t) f->sn != self_slot) {
            loft_ub_puts(why, self_slot < 0 ? "the store holds references and does not record which store they name"
                                            : "the store holds a reference into another store");
            ok = 0;
        } else if (f->pos != 8) {
            loft_ub_puts(why, "a reference names a field inside a record, which an upgrade does not carry yet");
            ok = 0;
        } else if (!loft_uc_rec_get(run, f->rec)) {
            loft_ub_puts(why, "a reference names a record the store no longer holds");
            ok = 0;
        }
    }
    if (!ok) {
        loft_store_drop(dst_nr);
        return 0xFFFF;
    }
    loft_stores[dst_nr].known = tp;
    return dst_nr;
}

/* `repoint_references`: every converted reference written, its holder and the record it names
 * moved from the store they were built in (`built`) to the one they now live in (`lives`). */
static void loft_up_repoint(loft_urun *run, uint16_t built, uint16_t lives) {
    for (size_t i = 0; i < run->nfix; i++) {
        loft_ufix *f = &run->fix[i];
        loft_ref holder = f->holder;
        if (holder.store_nr == built) holder.store_nr = lives;
        loft_ref target = loft_ref_null();
        if (f->sn != 0xFFFFFFFFu) {
            target = loft_uc_rec_get(run, f->rec)->nw;
            if (target.store_nr == built) target.store_nr = lives;
        }
        loft_set_dbref(holder, 0, target);
    }
}

static void loft_up_run_free(loft_urun *run) {
    free(run->recs);
    free(run->fix);
    loft_ub_free(&run->err);
    memset(run, 0, sizeof *run);
}

/* `load_upgraded_from`: the image `bytes` (`n` long, taken over), whose `.dschema` text is
 * `sidecar` and records another layout, converted into the store `r` names.  1 when it took;
 * 0 with `why` set, `r`'s store left as it was and nothing owed. */
static int loft_up_load(loft_ref r, const loft_sidecar *sc, const char *sidecar, uint8_t *bytes, size_t n,
                        const char *what, loft_ub *why) {
    const loft_upgrade *up = sc->up;
    loft_uo_reset();
    loft_ld od, nd;
    memset(&od, 0, sizeof od);
    memset(&nd, 0, sizeof nd);
    loft_ld_head h;
    loft_upmap m;
    memset(&m, 0, sizeof m);
    loft_urun run;
    memset(&run, 0, sizeof run);
    uint16_t *rt_new = 0, *rt_old = 0;
    uint32_t nrt = 0;
    int ok = 0, have_map = 0;
    uint16_t old_nr = 0xFFFF, dst_nr = 0xFFFF, new_root = LOFT_LD_NONE;
    if (!loft_ld_read_sidecar(&od, sidecar, what, &h, why)) goto done;
    if (!loft_up_own(up, sc->tp, &nd, &new_root, why)) goto done;
    /* A store older than this program goes through the routines; one this release or a later
     * one wrote is converted by name alone. */
    if (loft_up_newer(up->version, h.version) && loft_ur_n) {
        rt_new = (uint16_t *) malloc(loft_ur_n * sizeof *rt_new);
        rt_old = (uint16_t *) malloc(loft_ur_n * sizeof *rt_old);
        if (!rt_new || !rt_old) { fputs("loft: out of memory\n", stderr); abort(); }
        for (size_t i = 0; i < loft_ur_n; i++) {
            uint16_t a = LOFT_LD_NONE, b = LOFT_LD_NONE;
            for (uint16_t k = 0; k < nd.count; k++) {
                if (nd.n[k].kind == LOFT_LN_NAME) continue;
                if (nd.n[k].prog == loft_ur_tp[i]) a = k;
                if (nd.n[k].prog == loft_ur_old[i]) b = k;
            }
            if (a == LOFT_LD_NONE || b == LOFT_LD_NONE) continue;
            rt_new[nrt] = a;
            rt_old[nrt++] = b;
        }
    }
    {
        char reason[160];
        if (loft_image_refusal(bytes, n, reason, sizeof reason)) {
            loft_ub_printf(why, "%s is not a store image", what);
            goto done;
        }
    }
    loft_um_derive(&m, &od, h.root, &nd, new_root, rt_new, rt_old, nrt, 1);
    have_map = 1;
    if (m.refusal.n) {
        loft_ub_puts(why, loft_ub_str(&m.refusal));
        goto done;
    }
    /* The old image in a slot of its own, read only through the old description. */
    old_nr = loft_store_scratch(4);
    {
        loft_ref o = { old_nr, LOFT_ST_PRIMARY, 8 };
        uint8_t *own = bytes;
        bytes = 0;
        if (!loft_image_adopt(o, own, n, 1, 0, -1, what, "store upgrade")) {
            loft_ub_printf(why, "%s is not a store image", what);
            goto done;
        }
    }
    dst_nr = loft_up_convert(&run, &m, &od, &nd, new_root, sc->tp, old_nr, h.slot, h.version, why);
    if (dst_nr == 0xFFFF) goto done;
    loft_store_drop(old_nr);
    old_nr = 0xFFFF;
    {
        /* `place_converted`: the new store's image adopted as `r`'s store, which keeps its type
         * and its liveness. */
        loft_store_t *d = &loft_stores[dst_nr];
        uint32_t live_end = loft_image_live_end(d);
        if (live_end == 0) {
            loft_ub_puts(why, "the converted store does not hold together");
            goto done;
        }
        uint32_t target = live_end > LOFT_IMG_MIN_WORDS ? live_end : LOFT_IMG_MIN_WORDS;
        uint8_t *img = (uint8_t *) calloc(target, 8);
        if (!img) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(img, d->buf, (size_t) live_end * 8);
        if (target > live_end) {
            int32_t free_words = -(int32_t) (target - live_end);
            memcpy(img + (size_t) live_end * 8, &free_words, 4);
        }
        if (!loft_image_adopt(r, img, (size_t) target * 8, 0, 0, -1, what, "store upgrade")) {
            loft_ub_puts(why, "the converted store could not be adopted");
            goto done;
        }
    }
    loft_store_drop(dst_nr);
    for (size_t i = 0; i < loft_uo_n; i++) {
        if (loft_uo[i].rec.store_nr == dst_nr) loft_uo[i].rec.store_nr = r.store_nr;
        if (loft_uo[i].old.store_nr == dst_nr) loft_uo[i].old.store_nr = r.store_nr;
    }
    loft_up_repoint(&run, dst_nr, r.store_nr);
    dst_nr = 0xFFFF;
    ok = 1;
done:
    if (!ok) {
        /* `refusal_for`: a store older than this program names its release and the routine that
         * could carry it across. */
        if (have_map && h.version[0] && loft_up_newer(up->version, h.version) && why->n) {
            loft_ub e = { 0, 0, 0 };
            loft_ub_printf(&e,
                           "the store was written by %s: %s; a routine `fn upgrade(from: Release, old: <the old fields>, "
                           "item: <the type>)` can carry it across",
                           h.version, loft_ub_str(why));
            loft_ub_free(why);
            *why = e;
        }
        if (old_nr != 0xFFFF) loft_store_drop(old_nr);
        if (dst_nr != 0xFFFF) loft_store_drop(dst_nr);
        loft_uo_reset();
    }
    free(bytes);
    free(rt_new);
    free(rt_old);
    loft_up_run_free(&run);
    if (have_map) loft_um_free(&m);
    loft_ld_free(&od);
    loft_ld_free(&nd);
    return ok;
}

/* `image_rebuild` for a type holding a stored reference: the live store `slot` carried through
 * the converter at the program's own layout, element by element (`whole_copies` off), so every
 * reference is re-pointed at the copy it lands in; the copy answered, 0xFFFF with `why` set. */
static uint16_t loft_up_rebuild(uint16_t slot, const loft_sidecar *sc, loft_ub *why) {
    loft_ld od, nd;
    memset(&od, 0, sizeof od);
    memset(&nd, 0, sizeof nd);
    uint16_t old_root, new_root, dst = 0xFFFF;
    if (!loft_up_own(sc->up, sc->tp, &od, &old_root, why) || !loft_up_own(sc->up, sc->tp, &nd, &new_root, why)) {
        loft_ld_free(&od);
        loft_ld_free(&nd);
        return 0xFFFF;
    }
    loft_upmap m;
    loft_um_derive(&m, &od, old_root, &nd, new_root, 0, 0, 0, 0);
    if (m.refusal.n) {
        loft_ub_puts(why, loft_ub_str(&m.refusal));
    } else {
        loft_urun run;
        memset(&run, 0, sizeof run);
        size_t owed = loft_uo_n;
        dst = loft_up_convert(&run, &m, &od, &nd, new_root, sc->tp, slot, slot, "", why);
        if (dst != 0xFFFF) loft_up_repoint(&run, dst, dst);
        /* The identity conversion adds no field, so it owes nothing. */
        loft_uo_n = owed;
        loft_up_run_free(&run);
    }
    loft_um_free(&m);
    loft_ld_free(&od);
    loft_ld_free(&nd);
    return dst;
}
#else
/* A unit that converts no store owes nothing (`Stores::owed_next` over an empty list). */
static int64_t loft_store_upgrade_owed_next(void) { return -1; }
static loft_ref loft_store_upgrade_owed_record(void) { return loft_ref_null(); }
static loft_ref loft_store_upgrade_owed_old(void) { return loft_ref_null(); }
static loft_ref loft_store_upgrade_owed_from(void) { return loft_ref_null(); }
static uint8_t loft_store_upgrade_owed_finish(void) { return 1; }
static void loft_store_upgrade_routine(int64_t tp, int64_t old, int64_t version) {
    (void) tp;
    (void) old;
    (void) version;
}
#endif
