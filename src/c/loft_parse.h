#ifdef LOFT_PARSE
/* ── `T.parse(text)` (database/format.rs `record_from_text`, structures.rs `read_into` and
 * `walk_parsed_into`) ─────────────────────────────────────────────────────────────────────
 * The text is read in the lenient dialect and written into a new store as it is read: a
 * collection from an array element by element, a record from an object member by member,
 * every other value through the tree `loft_json_parse` builds.  What a type accepts, and what
 * an absent or misfit value leaves in its slot, is read from a table the emitter writes from
 * the program's own types (`loft_ptype`), one row per type the parse reaches. */
#define LOFT_P_INT 0
#define LOFT_P_LONG 1
#define LOFT_P_SINGLE 2
#define LOFT_P_FLOAT 3
#define LOFT_P_BOOL 4
#define LOFT_P_TEXT 5
#define LOFT_P_CHAR 6
#define LOFT_P_BYTE 7
#define LOFT_P_SHORT 8
#define LOFT_P_SHORTRAW 9
#define LOFT_P_NINT 10
#define LOFT_P_NINTRAW 11
#define LOFT_P_RECORD 12
#define LOFT_P_ENUM 13
#define LOFT_P_VECTOR 14
#define LOFT_P_DBREF 15
#define LOFT_P_CHILD 16
#define LOFT_P_OTHER 17
#define LOFT_P_KEYED 18
/* The `keyed` of a linked vector's slot list (`Parts::Array`): its records are its own. */
#define LOFT_P_ARRAY 6

/* A declared default (`Field::default`), as the literal the walk reads it from. */
#define LOFT_PD_NONE 0
#define LOFT_PD_INT 1
#define LOFT_PD_BOOL 2
#define LOFT_PD_NUMBER 3
#define LOFT_PD_STR 4

typedef struct {
    const char *name;
    uint16_t content, pos;
    uint8_t nullable;
    uint8_t skip;     /* written by no absent pass: the `enum` tag */
    uint8_t type_tag; /* the `type` field at 0 that a prefill stamps with the record's type */
    uint8_t dkind;
    int64_t di;
    uint64_t dbits; /* a NUMBER default's bits */
    const char *ds;
    size_t dslen;
    /* A member of a linked group: its members, itself first, the record type, whether a vector
     * member holds the records and whether they are `vector<S?>` elements. */
    const loft_member *grp;
    uint8_t ngrp;
    uint16_t gelem;
    uint8_t gheld, gnull;
} loft_pfield;

typedef struct {
    uint8_t kind;
    uint16_t size;
    uint16_t elem;               /* a collection's content */
    int32_t min;                 /* a narrow integer's lowest value */
    uint8_t nullable;            /* a narrow integer's nullable encoding */
    uint16_t n;                  /* fields, or variants */
    const loft_pfield *fields;
    const char *const *vnames;   /* an enum's variant names, by value from 1 */
    const uint16_t *vtypes;      /* their record types, 0xFFFF for a unit variant */
    uint8_t strict_enum;         /* a non-null field of it the text leaves absent fills nothing */
    uint8_t zero_default;        /* its default is all zero bytes */
    const uint8_t *prefill;      /* a record's prefill image (`Stores::set_default_value`) */
    uint8_t variant;             /* a record that is an enum's variant (`Parts::EnumValue`) */
    const char *name;            /* the type's name, as a JsonValue walk's message says it */
    uint8_t keyed;               /* a keyed collection's kind (`LOFT_KEYED_*`) */
    const loft_key *keys;        /* its key parts */
    uint8_t nk;
    uint8_t linked;              /* its record type is linked (`loft_hash_entry`) */
    uint8_t some;                /* a `__nullable<S>`'s `Some` value; 0 for any other enum */
    uint16_t payload_tp, payload_pos; /* the `Some` variant's `payload` field */
} loft_ptype;

/* The program's type table (`loft_types`), which a keyed collection's insert releases a
 * displaced record through; written after the runtime, beside `loft_ptypes`. */
static const loft_tdesc *loft_parse_types(void);

/* `WalkErr`: the offset an error reports and its path, segments in the order they were put in
 * front (the innermost first). */
typedef struct { size_t at; loft_text *seg; size_t n, cap; } loft_werr;
typedef struct { loft_werr *e; size_t n, cap; } loft_werrs;

static loft_werr *loft_werrs_push(loft_werrs *l, size_t at) {
    if (l->n == l->cap) {
        l->cap = l->cap ? l->cap * 2 : 4;
        l->e = (loft_werr *) realloc(l->e, l->cap * sizeof *l->e);
        if (!l->e) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_werr *e = &l->e[l->n++];
    memset(e, 0, sizeof *e);
    e->at = at;
    return e;
}

/* The path's next segment out: `e.path.insert(0, seg)`. */
static void loft_werr_front(loft_werr *e, const char *seg, size_t n) {
    if (e->n == e->cap) {
        e->cap = e->cap ? e->cap * 2 : 4;
        e->seg = (loft_text *) realloc(e->seg, e->cap * sizeof *e->seg);
        if (!e->seg) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_text t = { 0, 0, 0 };
    loft_push(&t, seg, n);
    e->seg[e->n++] = t;
}

static void loft_werr_index(loft_werr *e, size_t idx) {
    char b[32];
    int n = snprintf(b, sizeof b, "[%zu]", idx);
    loft_werr_front(e, b, (size_t) n);
}

static void loft_werrs_free(loft_werrs *l) {
    for (size_t i = 0; i < l->n; i++) {
        for (size_t k = 0; k < l->e[i].n; k++) loft_text_free(&l->e[i].seg[k]);
        free(l->e[i].seg);
    }
    free(l->e);
    memset(l, 0, sizeof *l);
}

/* `format_walk_err`: "line L:C path:" and the path, a field joined by `.`, an index not. */
static void loft_walk_err_text(loft_text *out, const uint8_t *text, size_t len, size_t at, const loft_werr *e) {
    size_t cap = at < len ? at : len;
    size_t line = 1, col_start = 0;
    for (size_t i = 0; i < cap; i++)
        if (text[i] == '\n') { line++; col_start = i + 1; }
    char b[96];
    int n = snprintf(b, sizeof b, "line %zu:%zu path:", line, cap - col_start + 1);
    loft_push(out, b, (size_t) n);
    if (!e) return;
    for (size_t k = e->n; k > 0; k--) {
        const loft_text *s = &e->seg[k - 1];
        if (k < e->n && !(s->len && s->p[0] == '[')) loft_push_char(out, '.');
        loft_push(out, s->p, s->len);
    }
}

/* The walk's state: the type table, and the field a non-null enum the text left empty
 * (`parse_unfilled`). */
typedef struct {
    const loft_ptype *P;
    const char *unfilled;
} loft_pw;

/* `@FR-D-Json`: `JsonValue`, the one enum whose default is a variant (`JNull`, tag 1). */
#define LOFT_P_JNULL 1
static int loft_p_is_json(const loft_pw *w, uint16_t tp) {
    return tp != 0xFFFF && w->P[tp].kind == LOFT_P_ENUM && strcmp(w->P[tp].name, "JsonValue") == 0;
}

static int loft_p_coll(const loft_pw *w, uint16_t tp) {
    return tp != 0xFFFF && (w->P[tp].kind == LOFT_P_VECTOR || w->P[tp].kind == LOFT_P_KEYED);
}

/* `field_ref`: field `field` of the `rec_tp` at `to`; `to` itself for no field. */
static loft_ref loft_p_field_ref(const loft_pw *w, loft_ref to, uint16_t rec_tp, uint16_t field) {
    if (field == 0xFFFF || rec_tp == 0xFFFF || w->P[rec_tp].kind != LOFT_P_RECORD) return to;
    to.pos += w->P[rec_tp].fields[field].pos;
    return to;
}

static int loft_p_field_nullable(const loft_pw *w, uint16_t rec_tp, uint16_t field) {
    if (rec_tp == 0xFFFF || field == 0xFFFF || w->P[rec_tp].kind != LOFT_P_RECORD) return 1;
    return field >= w->P[rec_tp].n || w->P[rec_tp].fields[field].nullable;
}

static void loft_p_u32(loft_ref r, uint32_t v) {
    loft_set_u32(loft_store_of(r), r.rec, r.pos, v);
}

static void loft_p_bytes(loft_ref r, const void *v, size_t n) {
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos), v, n);
}

static int loft_pw_walk(loft_pw *w, const loft_jv *v, uint16_t tp, uint16_t rec_tp, uint16_t field, loft_ref to,
                        size_t at, loft_werrs *errs);

/* `write_declared_default`: the field's declared default walked into `slot` as if the text
 * had spelled it; 0 for a field without one, or one its type does not take. */
static int loft_pw_declared(loft_pw *w, uint16_t rec_tp, uint16_t field, loft_ref slot) {
    if (rec_tp == 0xFFFF || field == 0xFFFF || w->P[rec_tp].kind != LOFT_P_RECORD || field >= w->P[rec_tp].n)
        return 0;
    const loft_pfield *f = &w->P[rec_tp].fields[field];
    if (f->dkind == LOFT_PD_NONE) return 0;
    loft_jv v;
    memset(&v, 0, sizeof v);
    switch (f->dkind) {
    case LOFT_PD_INT: v.kind = LOFT_JV_INT; v.i = f->di; break;
    case LOFT_PD_BOOL: v.kind = LOFT_JV_BOOL; v.b = f->di != 0; break;
    case LOFT_PD_NUMBER: v.kind = LOFT_JV_NUMBER; memcpy(&v.n, &f->dbits, 8); break;
    default: v.kind = LOFT_JV_STR; loft_push(&v.s, f->ds, f->dslen); break;
    }
    loft_werrs errs = { 0, 0, 0 };
    int ok = loft_pw_walk(w, &v, f->content, rec_tp, field, slot, 0, &errs);
    loft_werrs_free(&errs);
    loft_jv_free(&v);
    return ok;
}

/* `set_default_value_nullable`: the absent value of `tp` at `rec`, the prefill's when `final`
 * is 0, the value a parse hands out (`Absent::Final`) otherwise. */
static void loft_pw_default(loft_pw *w, uint16_t tp, int nullable, int final, loft_ref rec) {
    if (tp == 0xFFFF || rec.rec == 0 || rec.store_nr == 0xFFFF) return;
    const loft_ptype *t = &w->P[tp];
    switch (t->kind) {
    case LOFT_P_INT:
    case LOFT_P_LONG: {
        int64_t v = nullable ? INT64_MIN : 0;
        loft_p_bytes(rec, &v, 8);
        return;
    }
    case LOFT_P_CHAR: {
        int32_t v = 0;
        loft_p_bytes(rec, &v, 4);
        return;
    }
    case LOFT_P_SINGLE: {
        float v = nullable ? NAN : 0.0f;
        loft_p_bytes(rec, &v, 4);
        return;
    }
    case LOFT_P_FLOAT: {
        double v = nullable ? NAN : 0.0;
        loft_p_bytes(rec, &v, 8);
        return;
    }
    case LOFT_P_BOOL: *loft_at(loft_store_of(rec), rec.rec, rec.pos) = nullable ? 255 : 0; return;
    case LOFT_P_TEXT:
        /* A plain text a parse hands out holds the empty text, not the null (loft#875). */
        if (!nullable && final) loft_set_text(rec, 0, loft_lit("", 0));
        else loft_p_u32(rec, 0);
        return;
    case LOFT_P_ENUM:
        /* A struct-enum's slot is written whole: its null is tested on more than the tag
         * (loft#2189). */
        if (t->size > 1) memset(loft_at(loft_store_of(rec), rec.rec, rec.pos), 0, t->size);
        *loft_at(loft_store_of(rec), rec.rec, rec.pos) = !nullable && loft_p_is_json(w, tp) ? LOFT_P_JNULL : 0;
        return;
    case LOFT_P_BYTE: *loft_at(loft_store_of(rec), rec.rec, rec.pos) = t->nullable ? 255 : 0; return;
    case LOFT_P_SHORT: {
        uint16_t v = t->nullable ? 0 : 1;
        loft_p_bytes(rec, &v, 2);
        return;
    }
    case LOFT_P_SHORTRAW: {
        uint16_t v = t->nullable ? 0xFFFF : 0;
        loft_p_bytes(rec, &v, 2);
        return;
    }
    case LOFT_P_NINT: {
        int32_t v = t->nullable ? INT32_MIN : 0;
        loft_p_bytes(rec, &v, 4);
        return;
    }
    case LOFT_P_NINTRAW: loft_p_u32(rec, t->nullable ? 0xFFFFFFFFu : 0); return;
    case LOFT_P_RECORD:
        if (t->zero_default) {
            memset(loft_at(loft_store_of(rec), rec.rec, rec.pos), 0, t->size);
            return;
        }
        if (!final && t->prefill) {
            loft_p_bytes(rec, t->prefill, t->size);
            return;
        }
        for (uint16_t f = 0; f < t->n; f++) {
            const loft_pfield *fd = &t->fields[f];
            loft_ref slot = rec;
            slot.pos += fd->pos;
            if (fd->type_tag) {
                uint16_t v = (uint16_t) (tp + 1);
                loft_p_bytes(slot, &v, 2);
                continue;
            }
            if (final && loft_pw_declared(w, tp, f, slot)) continue;
            loft_pw_default(w, fd->content, fd->nullable, final, slot);
        }
        return;
    case LOFT_P_DBREF: {
        uint32_t z[3] = { 0, 0, 0 };
        loft_p_bytes(rec, z, 12);
        return;
    }
    case LOFT_P_CHILD: loft_p_u32(rec, 0); return;
    case LOFT_P_VECTOR:
    case LOFT_P_KEYED:
        /* An absent `vector<T>?` a parse hands out is the absent collection (loft#917). */
        loft_p_u32(rec, nullable && final ? 0xFFFFFFFFu : 0);
        return;
    default: return;
    }
}

/* `write_absent_value`: what a slot the text gave no usable value holds — the field's declared
 * default, else its type's absent value for the field's declared nullability. */
static void loft_pw_absent(loft_pw *w, uint16_t tp, uint16_t rec_tp, uint16_t field, loft_ref slot) {
    if (loft_pw_declared(w, rec_tp, field, slot)) return;
    int nullable = loft_p_field_nullable(w, rec_tp, field);
    /* `@FR-D-Json`: a `JsonValue` the text gives no value holds `JNull`, unless it is a field
     * declared `JsonValue?`. */
    if (loft_p_is_json(w, tp) && !(rec_tp != 0xFFFF && field != 0xFFFF && nullable)) {
        *loft_at(loft_store_of(slot), slot.rec, slot.pos) = LOFT_P_JNULL;
        return;
    }
    /* `@FR-D-NoEnumF`: a non-null enum field has no value to take; the parse made none. */
    if (!nullable && !w->unfilled && tp != 0xFFFF && w->P[tp].kind == LOFT_P_ENUM && w->P[tp].strict_enum)
        w->unfilled = w->P[rec_tp].fields[field].name;
    loft_pw_default(w, tp, nullable, 1, slot);
}

/* `write_absent_at`: a collection field's absent marker goes to the field's own slot. */
static void loft_pw_absent_at(loft_pw *w, uint16_t tp, uint16_t rec_tp, uint16_t field, loft_ref to) {
    loft_ref slot = loft_p_coll(w, tp) ? loft_p_field_ref(w, to, rec_tp, field) : to;
    loft_pw_absent(w, tp, rec_tp, field, slot);
}

/* `write_absent_fields`: every field of `tp` at `base` no member named (`found`, one flag per
 * field, or none at all). */
static void loft_pw_absent_fields(loft_pw *w, uint16_t tp, loft_ref base, const uint8_t *found) {
    const loft_ptype *t = &w->P[tp];
    for (uint16_t f = 0; f < t->n; f++) {
        if (t->fields[f].skip || (found && found[f])) continue;
        loft_ref slot = base;
        slot.pos += t->fields[f].pos;
        loft_pw_absent(w, t->fields[f].content, tp, f, slot);
    }
}

static int loft_pw_unfilled_record(const loft_pw *w, uint16_t rec_tp, uint16_t field) {
    return field != 0xFFFF && loft_p_field_nullable(w, rec_tp, field);
}

/* `record_new` / `record_finish` on a plain vector: the slot of one more element, and the
 * element counted. */
/* `record_new`: an element of the collection at field `field` of the `rec_tp` at `to` — a
 * vector's appended slot, a sorted's staged one past its end, a hash's entry, an index's or a
 * tree's record of its own. */
static loft_ref loft_pw_new(loft_pw *w, loft_ref to, uint16_t rec_tp, uint16_t field) {
    uint16_t coll = field == 0xFFFF ? rec_tp : w->P[rec_tp].fields[field].content;
    const loft_ptype *c = &w->P[coll];
    loft_ref d = loft_p_field_ref(w, to, rec_tp, field);
    uint32_t size = w->P[c->elem].size;
    if (c->kind != LOFT_P_KEYED) return loft_vec_append(d, size);
    if (c->keyed == LOFT_KEYED_SORTED) return loft_sorted_new(d, size);
    if (c->keyed == LOFT_KEYED_HASH) return loft_hash_entry(d, size, c->linked);
    return loft_entry_new(d, size);
}

/* `record_finish`: element `rec` filed — a vector's length taken, a keyed collection's record
 * put under its key (`@FR-Col-Insert`: an equal key's element is replaced and released). */
static void loft_pw_finish(loft_pw *w, loft_ref to, uint16_t rec_tp, uint16_t field, loft_ref rec) {
    uint16_t coll = field == 0xFFFF ? rec_tp : w->P[rec_tp].fields[field].content;
    const loft_ptype *c = &w->P[coll];
    loft_ref d = loft_p_field_ref(w, to, rec_tp, field);
    const loft_tdesc *T = loft_parse_types();
    /* A member of a linked group: the record enters every member (`link_siblings`). */
    if (field != 0xFFFF && rec_tp != 0xFFFF && w->P[rec_tp].kind == LOFT_P_RECORD
        && w->P[rec_tp].fields[field].ngrp) {
        const loft_pfield *g = &w->P[rec_tp].fields[field];
        if (c->kind != LOFT_P_KEYED) loft_vec_finish(d);
        loft_group_finish(T, g->gelem, g->gheld, g->gnull, to, rec, g->grp, g->ngrp);
        return;
    }
    if (c->kind != LOFT_P_KEYED) {
        loft_vec_finish(d);
        return;
    }
    switch (c->keyed) {
    case LOFT_KEYED_SORTED: loft_sorted_finish(d, w->P[c->elem].size, c->keys, c->nk); break;
    case LOFT_KEYED_HASH: (void) loft_hash_insert(T, c->elem, d, rec, c->keys, c->nk, 0); break;
    case LOFT_KEYED_INDEX: (void) loft_ix_insert(T, c->elem, d, rec, c->keys, c->nk, 0); break;
    case LOFT_KEYED_ORDERED: (void) loft_ordered_insert(T, c->elem, d, rec, c->keys, c->nk, 0); break;
    case LOFT_P_ARRAY: loft_array_append(d, rec); break;
    default:
        (void) loft_rx_insert_record(T, c->elem, d, rec, c->keys, c->nk, c->keyed == LOFT_KEYED_TRIE, 0);
        break;
    }
}

/* `Parsed::as_i64` / `as_f64`. */
static int loft_jv_i64(const loft_jv *v, int64_t *out) {
    if (v->kind == LOFT_JV_INT) { *out = v->i; return 1; }
    if (v->kind != LOFT_JV_NUMBER) return 0;
    double x = v->n;
    if (x != x) *out = 0;
    else if (x >= 9223372036854775807.0) *out = INT64_MAX;
    else if (x <= -9223372036854775808.0) *out = INT64_MIN;
    else *out = (int64_t) x;
    return 1;
}

static int loft_jv_f64(const loft_jv *v, double *out) {
    if (v->kind == LOFT_JV_NUMBER) { *out = v->n; return 1; }
    if (v->kind == LOFT_JV_INT) { *out = (double) v->i; return 1; }
    return 0;
}

/* The code point a valid UTF-8 text holds at `*k`, `*k` moved past it. */
static uint32_t loft_utf8_next(const uint8_t *s, size_t len, size_t *k) {
    uint32_t c = s[*k];
    size_t n = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
    if (*k + n > len) n = len - *k;
    if (n > 1) c &= 0xFFu >> (n + 1);
    for (size_t i = 1; i < n; i++) c = (c << 6) | (s[*k + i] & 0x3Fu);
    *k += n;
    return c;
}

#define LOFT_PW_MISFIT(errs, at) (loft_werrs_push((errs), (at)), 0)

/* `walk_primitive_into`. */
static int loft_pw_primitive(const loft_jv *v, uint16_t kind, loft_ref to, size_t at, loft_werrs *errs) {
    int64_t n;
    double x;
    switch (kind) {
    case LOFT_P_INT:
    case LOFT_P_LONG:
        if (!loft_jv_i64(v, &n)) return LOFT_PW_MISFIT(errs, at);
        loft_p_bytes(to, &n, 8);
        return 1;
    case LOFT_P_CHAR: {
        uint32_t cp;
        if (v->kind == LOFT_JV_STR) {
            /* One character, as `to_json` writes it. */
            const uint8_t *s = (const uint8_t *) v->s.p;
            size_t len = v->s.len, k = 0;
            if (!len) return LOFT_PW_MISFIT(errs, at);
            cp = loft_utf8_next(s, len, &k);
            if (k != len) return LOFT_PW_MISFIT(errs, at);
        } else {
            if (!loft_jv_i64(v, &n) || n < 0 || n > 0xFFFFFFFFll) return LOFT_PW_MISFIT(errs, at);
            cp = (uint32_t) n;
        }
        loft_p_bytes(to, &cp, 4);
        return 1;
    }
    case LOFT_P_SINGLE: {
        if (!loft_jv_f64(v, &x)) return LOFT_PW_MISFIT(errs, at);
        float f = (float) x;
        loft_p_bytes(to, &f, 4);
        return 1;
    }
    case LOFT_P_FLOAT:
        if (!loft_jv_f64(v, &x)) return LOFT_PW_MISFIT(errs, at);
        loft_p_bytes(to, &x, 8);
        return 1;
    case LOFT_P_BOOL:
        if (v->kind != LOFT_JV_BOOL) return LOFT_PW_MISFIT(errs, at);
        *loft_at(loft_store_of(to), to.rec, to.pos) = (uint8_t) (v->b != 0);
        return 1;
    case LOFT_P_TEXT:
        if (v->kind != LOFT_JV_STR) return LOFT_PW_MISFIT(errs, at);
        loft_set_text(to, 0, loft_view(&v->s));
        return 1;
    default: return LOFT_PW_MISFIT(errs, at);
    }
}

/* `write_narrow_value`: `n` through the slot's encoding (a value out of range takes the
 * type's default, as the setters do). */
static void loft_pw_narrow(const loft_ptype *t, int64_t n, loft_ref to) {
    int32_t v = (int32_t) (uint32_t) (uint64_t) n;
    switch (t->kind) {
    case LOFT_P_BYTE: loft_set_byte_v(to, 0, t->min, v == INT32_MIN ? LOFT_NULL : v); break;
    case LOFT_P_SHORT: loft_set_short(to, 0, t->min, v == INT32_MIN ? LOFT_NULL : v); break;
    case LOFT_P_SHORTRAW: loft_set_short_raw(to, 0, t->min, v == INT32_MIN ? LOFT_NULL : v); break;
    case LOFT_P_NINT: loft_p_bytes(to, &v, 4); break;
    default: loft_p_u32(to, n == INT64_MIN ? 0xFFFFFFFFu : (uint32_t) (uint64_t) n); break;
    }
}

/* `walk_parsed_struct`: an object's members into the record of `tp` at `to`. */
static int loft_pw_struct(loft_pw *w, const loft_jv *v, uint16_t tp, loft_ref to, size_t at, loft_werrs *errs) {
    if (v->kind != LOFT_JV_OBJECT) return LOFT_PW_MISFIT(errs, at);
    const loft_ptype *t = &w->P[tp];
    uint32_t fld = to.rec == 0 ? 0 : to.pos;
    uint32_t rec = to.rec == 0 ? loft_claim(loft_store_of(to), (t->size + 7u) / 8u) : to.rec;
    uint8_t *found = (uint8_t *) calloc(t->n ? t->n : 1, 1);
    if (!found) { fputs("loft: out of memory\n", stderr); abort(); }
    for (size_t m = 0; m < v->nkids; m++) {
        const loft_text *name = &v->keys[m];
        for (uint16_t f = 0; f < t->n; f++) {
            const loft_pfield *fd = &t->fields[f];
            if (strlen(fd->name) != name->len || memcmp(fd->name, name->p, name->len) != 0) continue;
            loft_ref target = to;
            if (!loft_p_coll(w, fd->content)) {
                target.rec = rec;
                target.pos = fld + fd->pos;
            }
            size_t seen = errs->n;
            int ok = loft_pw_walk(w, &v->kids[m], fd->content, tp, f, target, v->key_offs[m], errs);
            found[f] = 1;
            if (!ok) {
                for (size_t k = seen; k < errs->n; k++) loft_werr_front(&errs->e[k], name->p, name->len);
                free(found);
                return 0;
            }
            break;
        }
    }
    loft_ref base = to;
    base.rec = rec;
    base.pos = fld;
    loft_pw_absent_fields(w, tp, base, found);
    free(found);
    return 1;
}

/* `walk_parsed_into`: tree `v` into the `tp` at `to`; 0 with the misfit in `errs`. */
static int loft_pw_walk(loft_pw *w, const loft_jv *v, uint16_t tp, uint16_t rec_tp, uint16_t field, loft_ref to,
                        size_t at, loft_werrs *errs) {
    if (v->kind == LOFT_JV_NULL) {
        loft_pw_absent_at(w, tp, rec_tp, field, to);
        return 1;
    }
    const loft_ptype *t = &w->P[tp];
    int64_t n;
    switch (t->kind) {
    case LOFT_P_INT: case LOFT_P_LONG: case LOFT_P_SINGLE: case LOFT_P_FLOAT:
    case LOFT_P_BOOL: case LOFT_P_TEXT: case LOFT_P_CHAR:
        return loft_pw_primitive(v, t->kind, to, at, errs);
    case LOFT_P_VECTOR:
    case LOFT_P_KEYED: {
        if (v->kind != LOFT_JV_ARRAY) return LOFT_PW_MISFIT(errs, at);
        loft_p_u32(loft_p_field_ref(w, to, rec_tp, field), 0);
        for (size_t i = 0; i < v->nkids; i++) {
            loft_ref res = loft_pw_new(w, to, rec_tp, field);
            size_t seen = errs->n;
            if (!loft_pw_walk(w, &v->kids[i], t->elem, t->elem, 0xFFFF, res, at, errs)) {
                for (size_t k = seen; k < errs->n; k++) loft_werr_index(&errs->e[k], i);
                return 0;
            }
            loft_pw_finish(w, to, rec_tp, field, res);
        }
        return 1;
    }
    case LOFT_P_RECORD: {
        /* A `Type{…}` reads as its body: the schema fixes the type. */
        const loft_jv *body = v->kind == LOFT_JV_CTOR ? &v->kids[0] : v;
        if (body->kind == LOFT_JV_OBJECT && loft_pw_unfilled_record(w, rec_tp, field))
            loft_pw_absent_fields(w, tp, to, 0);
        return loft_pw_struct(w, body, tp, to, at, errs);
    }
    case LOFT_P_ENUM: {
        /* @PLN25 E2: a `vector<S?>` element reads a present object as the `Some` variant, the
         * object S's fields in its `payload`. */
        if (t->some && (v->kind == LOFT_JV_OBJECT || v->kind == LOFT_JV_CTOR)) {
            *loft_at(loft_store_of(to), to.rec, to.pos) = t->some;
            loft_ref payload_to = to;
            payload_to.pos += t->payload_pos;
            return loft_pw_walk(w, v, t->payload_tp, rec_tp, field, payload_to, at, errs);
        }
        const char *name;
        size_t len;
        const loft_jv *payload = 0;
        if (v->kind == LOFT_JV_STR || v->kind == LOFT_JV_IDENT) {
            name = v->s.p ? v->s.p : "";
            len = v->s.len;
        } else if (v->kind == LOFT_JV_CTOR) {
            name = v->s.p ? v->s.p : "";
            len = v->s.len;
            payload = &v->kids[0];
        } else if (v->kind == LOFT_JV_OBJECT && v->nkids == 1) {
            name = v->keys[0].p ? v->keys[0].p : "";
            len = v->keys[0].len;
            payload = &v->kids[0];
        } else {
            return LOFT_PW_MISFIT(errs, at);
        }
        /* A qualified tag (`Enum.Variant`) is matched on its last segment. */
        for (size_t i = len; i > 0; i--)
            if (name[i - 1] == '.') { name += i; len -= i; break; }
        uint16_t enum_tp = 0xFFFF;
        uint8_t val = 0;
        if (!(len == 4 && memcmp(name, "null", 4) == 0)) {
            for (uint16_t k = 0; k < t->n; k++)
                if (strlen(t->vnames[k + 1]) == len && memcmp(t->vnames[k + 1], name, len) == 0) {
                    val = (uint8_t) (k + 1);
                    enum_tp = t->vtypes[k];
                    break;
                }
            /* An unknown tag in a declared field is a value that did not fit. */
            if (val == 0 && field != 0xFFFF && rec_tp != 0xFFFF) {
                loft_pw_absent_at(w, tp, rec_tp, field, to);
                return LOFT_PW_MISFIT(errs, at);
            }
        }
        *loft_at(loft_store_of(to), to.rec, to.pos) = val;
        if (enum_tp != 0xFFFF && w->P[enum_tp].size > 1) {
            loft_pw_absent_fields(w, enum_tp, to, 0);
            if (payload) return loft_pw_walk(w, payload, enum_tp, rec_tp, field, to, at, errs);
        }
        return 1;
    }
    case LOFT_P_BYTE: case LOFT_P_SHORT: case LOFT_P_SHORTRAW: case LOFT_P_NINT: case LOFT_P_NINTRAW:
        if (!loft_jv_i64(v, &n)) return LOFT_PW_MISFIT(errs, at);
        loft_pw_narrow(t, n, to);
        return 1;
    default: return LOFT_PW_MISFIT(errs, at);
    }
}

/* `JParser::opening`. */
#define LOFT_OPEN_ARRAY 0
#define LOFT_OPEN_OBJECT 1
#define LOFT_OPEN_WORD 2
#define LOFT_OPEN_OTHER 3
static int loft_jp_opening(const loft_jp *p) {
    if (loft_jp_is(p, '[')) return LOFT_OPEN_ARRAY;
    if (loft_jp_is(p, '{')) return LOFT_OPEN_OBJECT;
    if (p->pk.kind == LOFT_JT_IDENT && p->lenient) return LOFT_OPEN_WORD;
    return LOFT_OPEN_OTHER;
}

static int loft_pr_into(loft_pw *w, loft_jp *p, loft_werrs *errs, size_t value_at, uint16_t tp, uint16_t rec_tp,
                        uint16_t field, loft_ref to, size_t at, loft_jfault *f);

/* `read_array`: the `[` is at the cursor. */
static int loft_pr_array(loft_pw *w, loft_jp *p, loft_werrs *errs, uint16_t tp, uint16_t content, uint16_t rec_tp,
                         uint16_t field, loft_ref to, size_t at, loft_jfault *f) {
    size_t start, next;
    int more = loft_jp_open_array(p, &start, &next);
    loft_p_u32(loft_p_field_ref(w, to, rec_tp, field), 0);
    (void) tp;
    size_t idx = 0;
    while (more) {
        loft_ref res = loft_pw_new(w, to, rec_tp, field);
        size_t seen = errs->n;
        if (!loft_pr_into(w, p, errs, next, content, content, 0xFFFF, res, at, f)) return 0;
        /* The element itself did not fit when an error's path is still empty. */
        int misfit = 0;
        for (size_t k = seen; k < errs->n; k++) {
            if (errs->e[k].n == 0) misfit = 1;
            loft_werr_index(&errs->e[k], idx);
        }
        if (misfit) loft_pw_default(w, content, 1, 1, res);
        loft_pw_finish(w, to, rec_tp, field, res);
        int r = loft_jp_next_item(p, start, &next, f);
        if (r < 0) return 0;
        more = r;
        idx++;
    }
    return 1;
}

/* `read_struct`: the `{` is at the cursor. */
static int loft_pr_struct(loft_pw *w, loft_jp *p, loft_werrs *errs, uint16_t tp, uint16_t rec_tp, uint16_t field,
                          loft_ref to, loft_jfault *f) {
    size_t start;
    int more = loft_jp_open_object(p, &start);
    if (loft_pw_unfilled_record(w, rec_tp, field)) loft_pw_absent_fields(w, tp, to, 0);
    const loft_ptype *t = &w->P[tp];
    uint32_t fld = to.rec == 0 ? 0 : to.pos;
    uint32_t rec = to.rec == 0 ? loft_claim(loft_store_of(to), (t->size + 7u) / 8u) : to.rec;
    /* Which fields the text gave: on the stack for an ordinary record. */
    uint8_t found_local[64];
    uint8_t *found = found_local;
    if (t->n > sizeof found_local) {
        found = (uint8_t *) calloc(t->n, 1);
        if (!found) { fputs("loft: out of memory\n", stderr); abort(); }
    } else {
        memset(found_local, 0, sizeof found_local);
    }
    /* Members usually come in declaration order (`to_json` writes them so): the search for a
     * name starts at the field after the last one found. */
    uint16_t next = 0;
    int ok = 1;
    while (more) {
        loft_text name = { 0, 0, 0 };
        size_t key_at, value_at;
        if (!loft_jp_member(p, &name, &key_at, &value_at, f)) { loft_text_free(&name); ok = 0; break; }
        uint16_t hit = 0xFFFF;
        for (uint16_t j = 0; j < t->n; j++) {
            uint16_t k = (uint16_t) (next + j < t->n ? next + j : next + j - t->n);
            if (strlen(t->fields[k].name) == name.len && memcmp(t->fields[k].name, name.p, name.len) == 0) {
                hit = k;
                next = (uint16_t) (k + 1 < t->n ? k + 1 : 0);
                break;
            }
        }
        if (hit != 0xFFFF) {
            const loft_pfield *fd = &t->fields[hit];
            loft_ref target = to;
            if (!loft_p_coll(w, fd->content)) {
                target.rec = rec;
                target.pos = fld + fd->pos;
            }
            size_t seen = errs->n;
            if (!loft_pr_into(w, p, errs, value_at, fd->content, tp, hit, target, key_at, f)) {
                loft_text_free(&name);
                ok = 0;
                break;
            }
            int misfit = 0;
            for (size_t k = seen; k < errs->n; k++) {
                if (errs->e[k].n == 0) misfit = 1;
                loft_werr_front(&errs->e[k], name.p, name.len);
            }
            if (!misfit) found[hit] = 1;
        } else {
            loft_jv skip;
            memset(&skip, 0, sizeof skip);
            int r = loft_jp_value(p, value_at, &skip, f);
            loft_jv_free(&skip);
            if (!r) { loft_text_free(&name); ok = 0; break; }
        }
        loft_text_free(&name);
        int r = loft_jp_next_member(p, start, f);
        if (r < 0) { ok = 0; break; }
        more = r;
    }
    if (ok) {
        loft_ref base = to;
        base.rec = rec;
        base.pos = fld;
        loft_pw_absent_fields(w, tp, base, found);
    }
    if (found != found_local) free(found);
    return ok;
}

/* `read_into`: the value at the cursor into the `tp` at `to`, written as it is read; 0 for a
 * document that is not well-formed (the fault in `f`). */
static int loft_pr_into(loft_pw *w, loft_jp *p, loft_werrs *errs, size_t value_at, uint16_t tp, uint16_t rec_tp,
                        uint16_t field, loft_ref to, size_t at, loft_jfault *f) {
    const loft_ptype *t = &w->P[tp];
    int open = loft_jp_opening(p);
    int record = t->kind == LOFT_P_RECORD;
    if ((t->kind == LOFT_P_VECTOR || t->kind == LOFT_P_KEYED) && open == LOFT_OPEN_ARRAY)
        return loft_pr_array(w, p, errs, tp, t->elem, rec_tp, field, to, at, f);
    if (record && open == LOFT_OPEN_OBJECT) return loft_pr_struct(w, p, errs, tp, rec_tp, field, to, f);
    /* A number into a numeric field: read straight into the slot through the writer the walk
     * would reach (`loft_pw_primitive`); the number reader sets only the kind and the value,
     * so the general value is neither cleared nor freed. */
    if ((t->kind == LOFT_P_INT || t->kind == LOFT_P_LONG || t->kind == LOFT_P_FLOAT || t->kind == LOFT_P_SINGLE)
        && (p->pk.kind == LOFT_JT_NUM || (p->pk.kind == LOFT_JT_TOK && p->pk.tok == '-'))) {
        loft_jv n;
        n.kind = LOFT_JV_NULL;
        n.i = 0;
        n.n = 0.0;
        if (!loft_jp_number(p, &n, f)) return 0;
        loft_pw_primitive(&n, t->kind, to, at, errs);
        return 1;
    }
    loft_jv v;
    memset(&v, 0, sizeof v);
    if (record && open == LOFT_OPEN_WORD) {
        size_t off;
        /* The tag is informational for a record: its type is the schema's. */
        if (loft_jp_word(p, &v, &off)) {
            loft_jv_free(&v);
            return loft_pr_struct(w, p, errs, tp, rec_tp, field, to, f);
        }
    } else if (!loft_jp_value(p, value_at, &v, f)) {
        loft_jv_free(&v);
        return 0;
    }
    loft_pw_walk(w, &v, tp, rec_tp, field, to, at, errs);
    loft_jv_free(&v);
    return 1;
}

/* `Store::parse_errors`: what the parse that built a store's root reported, by store. */
typedef struct { loft_text *e; size_t n; uint16_t tp; } loft_perrs;
/* A header built without `loft_cc.h` (the replays of tests/c_emit.rs) keeps one copy of its
 * state: no threads. */
#ifndef LOFT_TLS
#define LOFT_TLS
#endif
static LOFT_TLS loft_perrs *loft_store_perrs = 0;
static LOFT_TLS size_t loft_store_nperrs = 0;

static void loft_perrs_clear(loft_perrs *pe) {
    for (size_t i = 0; i < pe->n; i++) loft_text_free(&pe->e[i]);
    free(pe->e);
    pe->e = 0;
    pe->n = 0;
    pe->tp = 0xFFFF;
}

static loft_perrs *loft_perrs_of(uint16_t store_nr) {
    if (store_nr >= loft_store_nperrs) {
        size_t n = (size_t) store_nr + 16;
        loft_store_perrs = (loft_perrs *) realloc(loft_store_perrs, n * sizeof *loft_store_perrs);
        if (!loft_store_perrs) { fputs("loft: out of memory\n", stderr); abort(); }
        for (size_t i = loft_store_nperrs; i < n; i++) {
            loft_store_perrs[i].e = 0;
            loft_store_perrs[i].n = 0;
            loft_store_perrs[i].tp = 0xFFFF;
        }
        loft_store_nperrs = n;
    }
    return &loft_store_perrs[store_nr];
}

static void loft_perrs_reset(uint16_t nr) {
    if (nr < loft_store_nperrs) loft_perrs_clear(&loft_store_perrs[nr]);
}

static void loft_perrs_swap(uint16_t a, uint16_t b) {
    loft_perrs_of(a > b ? a : b);
    loft_perrs t = loft_store_perrs[a];
    loft_store_perrs[a] = loft_store_perrs[b];
    loft_store_perrs[b] = t;
}

/* `deliver_parse_errors`: a write of a whole `tp` record into another store's root brings the
 * errors of the parse that built it, or none when no parse did.  `fresh`: the root was just
 * minted for this copy (a bind), so it is a `tp` by construction. */
static void loft_perrs_deliver(loft_ref data, loft_ref to, uint16_t tp, int fresh) {
    if (data.store_nr == to.store_nr || data.store_nr >= loft_nstores || to.store_nr >= loft_nstores) return;
    if (to.rec != 1 || to.pos != 8) return;
    loft_perrs_of(to.store_nr > data.store_nr ? to.store_nr : data.store_nr);
    loft_perrs *dest = &loft_store_perrs[to.store_nr];
    uint16_t known = loft_stores[to.store_nr].known;
    uint16_t root = known == 0xFFFF ? dest->tp : known;
    if (!fresh && root != tp) return;
    loft_perrs *src = &loft_store_perrs[data.store_nr];
    int parsed = data.rec == 1 && data.pos == 8 && src->tp == tp;
    loft_text *copy = 0;
    size_t n = parsed ? src->n : 0;
    if (n) {
        copy = (loft_text *) calloc(n, sizeof *copy);
        for (size_t i = 0; i < n; i++) copy[i] = loft_text_own(loft_view(&src->e[i]));
    }
    loft_perrs_clear(dest);
    dest->e = copy;
    dest->n = n;
    dest->tp = parsed ? tp : 0xFFFF;
}

/* The parse table the last parse read, for the deliveries that ask a store's parsed type. */
static const loft_ptype *loft_ptab = 0;

/* Whether type `tp` is a vector variable's own store (`main_vector<T>`), emitted with the
 * table. */
static int loft_is_main_vector(uint16_t tp);

/* `parsed_vector`: the vector type a parse built `db`'s store for, when `db` is its root. */
static uint16_t loft_parsed_vector(loft_ref db) {
    if (db.store_nr == 0xFFFF || db.store_nr >= loft_store_nperrs || db.rec != 1 || !loft_ptab) return 0xFFFF;
    uint16_t tp = loft_store_perrs[db.store_nr].tp;
    return tp != 0xFFFF && loft_ptab[tp].kind == LOFT_P_VECTOR ? tp : 0xFFFF;
}

/* `deliver_vector_parse_errors`: `to`, a vector variable's whole value, given the content of
 * `data`.  A whole write takes the incoming value's errors or none; an append only when it
 * fills an empty vector no parse built from a whole parsed one. */
static void loft_perrs_deliver_vector(loft_ref data, loft_ref to, int whole) {
    if (data.store_nr == to.store_nr || to.store_nr == 0xFFFF || to.store_nr >= loft_nstores || to.rec != 1) return;
    if (!loft_stores[to.store_nr].live) return;
    if (loft_parsed_vector(to) == 0xFFFF && !loft_is_main_vector(loft_stores[to.store_nr].known)) return;
    uint16_t parsed = loft_parsed_vector(data);
    loft_perrs *dest = loft_perrs_of(to.store_nr);
    if (!whole && (parsed == 0xFFFF || dest->tp != 0xFFFF || loft_vec_len(to) != 0)) return;
    loft_text *copy = 0;
    size_t n = 0;
    if (parsed != 0xFFFF) {
        loft_perrs *src = &loft_store_perrs[data.store_nr];
        n = src->n;
        if (n) copy = (loft_text *) calloc(n, sizeof *copy);
        for (size_t i = 0; i < n; i++) copy[i] = loft_text_own(loft_view(&src->e[i]));
    }
    loft_perrs_clear(dest);
    dest->e = copy;
    dest->n = n;
    dest->tp = parsed;
}


/* `text_target`: the store a parse fills, its root holding the type's final defaults. */
static loft_ref loft_text_target(loft_pw *w, uint16_t tp) {
    uint32_t words = 1 + ((uint32_t) w->P[tp].size + 7) / 8;
    if (words < 8) words = 8;
    loft_ref r = loft_ref_null();
    loft_database(&r, (words - 1) * 8);
    loft_perrs_clear(loft_perrs_of(r.store_nr));
    loft_pw_default(w, tp, 1, 1, r);
    return r;
}

/* `record_from_text`: a new store holding the `tp` the text describes, its errors filed under
 * the record's `#errors` and `json_errors()`; the null reference for a non-null enum field the
 * text left without a value. */
static loft_ref loft_record_from_text(const loft_ptype *P, loft_str text, uint16_t tp) {
    loft_pw w;
    w.P = P;
    w.unfilled = 0;
    loft_ptab = P;
    loft_jerr_clear();
    const uint8_t *b = (const uint8_t *) text.p;
    loft_ref into = loft_text_target(&w, tp);
    loft_werrs errs = { 0, 0, 0 };
    loft_jfault f;
    loft_jp p;
    size_t value_at = loft_jp_open(&p, b, text.len, 1);
    int ok = loft_pr_into(&w, &p, &errs, value_at, tp, tp, 0xFFFF, into, 0, &f) && loft_jp_finish(&p, &f);
    loft_jp_close(&p);
    loft_text *msgs = 0;
    size_t nmsg = 0;
    if (!ok) {
        /* A document that is not well-formed answers the type's defaults, as if none of it
         * had been read. */
        loft_free_ref(into);
        into = loft_text_target(&w, tp);
        msgs = (loft_text *) calloc(2, sizeof *msgs);
        loft_walk_err_text(&msgs[0], b, text.len, f.at, 0);
        nmsg = 1;
    } else if (errs.n) {
        msgs = (loft_text *) calloc(errs.n + 1, sizeof *msgs);
        for (size_t i = 0; i < errs.n; i++) loft_walk_err_text(&msgs[i], b, text.len, errs.e[i].at, &errs.e[i]);
        nmsg = errs.n;
    }
    loft_werrs_free(&errs);
    if (w.unfilled) {
        loft_free_ref(into);
        msgs = (loft_text *) realloc(msgs, (nmsg + 1) * sizeof *msgs);
        loft_text m = { 0, 0, 0 };
        loft_push_cstr(&m, "field `");
        loft_push_cstr(&m, w.unfilled);
        loft_push_cstr(&m, "` has no value and no default — the parse answers null");
        msgs[nmsg++] = m;
        for (size_t i = 0; i < nmsg; i++) {
            loft_jerr_push(loft_view(&msgs[i]));
            loft_text_free(&msgs[i]);
        }
        free(msgs);
        return loft_ref_null();
    }
    for (size_t i = 0; i < nmsg; i++) loft_jerr_push(loft_view(&msgs[i]));
    loft_perrs *pe = loft_perrs_of(into.store_nr);
    loft_perrs_clear(pe);
    pe->e = msgs;
    pe->n = nmsg;
    pe->tp = tp;
    return into;
}

/* `parse_errors_vector`: the errors of the parse that built the store `of` lives in, as a
 * fresh `vector<text>`. */
static loft_ref loft_parse_errors_of(loft_ref of) {
    loft_ref vec = loft_ref_null();
    loft_database(&vec, 4 * 8 - 8);
    loft_set_u32(loft_store_of(vec), vec.rec, vec.pos, 0);
    if (of.store_nr == 0xFFFF || of.store_nr >= loft_nstores || !loft_stores[of.store_nr].live) return vec;
    loft_perrs *pe = loft_perrs_of(of.store_nr);
    for (size_t i = 0; i < pe->n; i++) {
        loft_ref e = loft_vec_append(vec, 4);
        loft_set_text(e, 0, loft_view(&pe->e[i]));
        loft_vec_finish(vec);
    }
    return vec;
}

#ifdef LOFT_JV_SIZE
/* ── `T.parse(JsonValue)` (native.rs `n_struct_from_jsonvalue`) ─────────────────────────────
 * The two-stage form: every declared field looked up by name in the tree's object and written
 * from the member's variant, a member the field's type cannot hold reported as
 * "`Type.field: expected X, got Y`" and taken as absent.  Its messages are the record's
 * `#errors`, and join `json_errors()` without clearing it. */
static const char *loft_jd_name(uint8_t d) {
    static const char *names[] = { "JUnknown", "JNull", "JBool", "JNumber", "JString", "JArray", "JObject", "JInteger" };
    return names[d <= 7 ? d : 0];
}

/* `push_kind_mismatch`: a present member of another kind than the field takes. */
static void loft_jsv_mismatch(uint8_t actual, uint8_t expected, const char *sname, const char *fname) {
    if (actual == LOFT_JD_NULL || actual == expected) return;
    const char *en = expected == LOFT_JD_BOOL ? "JBool" : expected == LOFT_JD_NUMBER ? "JNumber"
        : expected == LOFT_JD_STRING ? "JString" : expected == LOFT_JD_ARRAY ? "JArray"
        : expected == LOFT_JD_OBJECT ? "JObject" : "?";
    loft_text m = { 0, 0, 0 };
    loft_push_cstr(&m, sname);
    loft_push_char(&m, '.');
    loft_push_cstr(&m, fname);
    loft_push_cstr(&m, ": expected ");
    loft_push_cstr(&m, en);
    loft_push_cstr(&m, ", got ");
    loft_push_cstr(&m, loft_jd_name(actual));
    loft_jerr_take(m);
}

/* `unwrap_long` (and `unwrap_int`): an integer exactly, a finite number truncated. */
static int loft_jsv_long(loft_ref sub, uint8_t d, const char *sname, const char *fname, int64_t *out) {
    if (d == LOFT_JD_INT) { *out = loft_get_int(sub, LOFT_JV_INT_VALUE); return 1; }
    loft_jsv_mismatch(d, LOFT_JD_NUMBER, sname, fname);
    if (d != LOFT_JD_NUMBER) return 0;
    double f = loft_get_float(sub, LOFT_JV_NUMBER_VALUE);
    if (!isfinite(f)) return 0;
    if (f >= 9223372036854775807.0) *out = INT64_MAX;
    else if (f <= -9223372036854775808.0) *out = INT64_MIN;
    else *out = (int64_t) f;
    return 1;
}

static int loft_jsv_float(loft_ref sub, uint8_t d, const char *sname, const char *fname, double *out) {
    if (d == LOFT_JD_INT) { *out = (double) loft_get_int(sub, LOFT_JV_INT_VALUE); return 1; }
    loft_jsv_mismatch(d, LOFT_JD_NUMBER, sname, fname);
    if (d != LOFT_JD_NUMBER) return 0;
    *out = loft_get_float(sub, LOFT_JV_NUMBER_VALUE);
    return 1;
}

/* `unwrap_char`: a one-character string, or a number as its code point. */
static int loft_jsv_char(loft_ref sub, uint8_t d, const char *sname, const char *fname, uint32_t *out) {
    if (d == LOFT_JD_STRING) {
        loft_str t = loft_get_text(sub, LOFT_JV_STRING_VALUE);
        size_t k = 0;
        if (t.len && !loft_str_is_null(t)) {
            uint32_t cp = loft_utf8_next((const uint8_t *) t.p, t.len, &k);
            if (k == t.len) { *out = cp; return 1; }
        }
        return 0;
    }
    int64_t n;
    if (!loft_jsv_long(sub, d, sname, fname, &n) || n < INT32_MIN || n > INT32_MAX) return 0;
    *out = (uint32_t) (int32_t) n;
    return 1;
}

/* `lookup_jobject_field`: the member named `name` of the object at `src`. */
static int loft_jsv_lookup(loft_ref src, const char *name, loft_ref *out) {
    uint32_t fr = loft_jv_kids(src, LOFT_JD_OBJECT);
    int64_t n = loft_jv_count(src, fr);
    loft_store_t *s = loft_store_of(src);
    size_t len = strlen(name);
    for (int64_t i = 0; i < n; i++) {
        uint32_t at = 8 + (uint32_t) i * LOFT_JF_SIZE;
        loft_str k = loft_store_str(s, loft_u32(s, fr, at + LOFT_JF_NAME));
        if (k.len == len && memcmp(k.p, name, len) == 0) {
            *out = src;
            out->rec = fr;
            out->pos = at + LOFT_JF_VALUE;
            return 1;
        }
    }
    return 0;
}

/* A JsonValue copied whole into the slot's store (loft#2187). */
static void loft_jsv_copy(loft_ref slot, loft_ref sub) {
    loft_jv tree;
    loft_jv_read(sub, &tree);
    loft_jv_into(slot, &tree);
    loft_jv_free(&tree);
}


static void loft_jsv_struct(loft_pw *w, loft_ref dest, uint16_t kt, loft_ref src);

/* `populate_vector_from_jarray`: the array's items appended to the vector at `handle`; an item
 * the element type cannot hold takes its absent value, an element kind the walk does not write
 * keeps the appended slot's bytes. */
static void loft_jsv_vector(loft_pw *w, loft_ref handle, uint16_t elem, loft_ref arr) {
    uint32_t ir = loft_jv_kids(arr, LOFT_JD_ARRAY);
    int64_t n = loft_jv_count(arr, ir);
    const loft_ptype *e = &w->P[elem];
    for (int64_t i = 0; i < n; i++) {
        loft_ref item = arr;
        item.rec = ir;
        item.pos = 8 + (uint32_t) i * LOFT_JV_SIZE;
        uint8_t d = loft_jd(item);
        loft_ref elm = loft_vec_append(handle, e->size);
        int64_t l;
        double x;
        uint32_t cp;
        switch (e->kind) {
        case LOFT_P_INT:
        case LOFT_P_LONG:
            if (loft_jsv_long(item, d, "vector", e->name, &l)) loft_p_bytes(elm, &l, 8);
            else loft_pw_absent(w, elem, 0xFFFF, 0xFFFF, elm);
            break;
        case LOFT_P_FLOAT:
            if (loft_jsv_float(item, d, "vector", e->name, &x)) loft_p_bytes(elm, &x, 8);
            else loft_pw_absent(w, elem, 0xFFFF, 0xFFFF, elm);
            break;
        case LOFT_P_BOOL:
            loft_jsv_mismatch(d, LOFT_JD_BOOL, "vector", e->name);
            if (d == LOFT_JD_BOOL)
                *loft_at(loft_store_of(elm), elm.rec, elm.pos) = *loft_at(loft_store_of(item), item.rec, item.pos + LOFT_JV_BOOL_VALUE);
            else loft_pw_absent(w, elem, 0xFFFF, 0xFFFF, elm);
            break;
        case LOFT_P_TEXT:
            loft_jsv_mismatch(d, LOFT_JD_STRING, "vector", e->name);
            if (d == LOFT_JD_STRING) {
                loft_text t = loft_text_own(loft_get_text(item, LOFT_JV_STRING_VALUE));
                loft_set_text(elm, 0, loft_view(&t));
                loft_text_free(&t);
            } else {
                loft_pw_absent(w, elem, 0xFFFF, 0xFFFF, elm);
            }
            break;
        case LOFT_P_CHAR:
            if (loft_jsv_char(item, d, "vector", e->name, &cp)) loft_p_bytes(elm, &cp, 4);
            else loft_pw_absent(w, elem, 0xFFFF, 0xFFFF, elm);
            break;
        case LOFT_P_BYTE: case LOFT_P_SHORT: case LOFT_P_SHORTRAW: case LOFT_P_NINT: case LOFT_P_NINTRAW:
            if (loft_jsv_long(item, d, "vector", e->name, &l)) loft_pw_narrow(e, l, elm);
            else loft_pw_absent(w, elem, 0xFFFF, 0xFFFF, elm);
            break;
        case LOFT_P_ENUM:
            if (loft_p_is_json(w, elem)) loft_jsv_copy(elm, item);
            break;
        case LOFT_P_RECORD:
            if (!e->variant) loft_jsv_struct(w, elm, elem, item);
            break;
        default: break;
        }
        loft_vec_finish(handle);
    }
}

/* `populate_struct_from_jsonvalue`: every field of the struct `kt` at `dest`, from `src`. */
static void loft_jsv_struct(loft_pw *w, loft_ref dest, uint16_t kt, loft_ref src) {
    const loft_ptype *t = &w->P[kt];
    if (t->kind != LOFT_P_RECORD || t->variant) return;
    for (uint16_t f = 0; f < t->n; f++) {
        const loft_pfield *fd = &t->fields[f];
        loft_ref slot = dest;
        slot.pos += fd->pos;
        loft_ref sub;
        int found = loft_jsv_lookup(src, fd->name, &sub);
        uint8_t d = found ? loft_jd(sub) : LOFT_JD_NULL;
        /* An omitted key and an explicit `null` are the declaration's question. */
        if (!found || d == LOFT_JD_NULL) {
            loft_pw_absent(w, fd->content, kt, f, slot);
            continue;
        }
        const loft_ptype *c = &w->P[fd->content];
        int64_t l;
        double x;
        uint32_t cp;
        switch (c->kind) {
        case LOFT_P_INT:
        case LOFT_P_LONG:
            if (loft_jsv_long(sub, d, t->name, fd->name, &l)) loft_p_bytes(slot, &l, 8);
            else loft_pw_absent(w, fd->content, kt, f, slot);
            break;
        case LOFT_P_FLOAT:
            if (loft_jsv_float(sub, d, t->name, fd->name, &x)) loft_p_bytes(slot, &x, 8);
            else loft_pw_absent(w, fd->content, kt, f, slot);
            break;
        case LOFT_P_BOOL:
            loft_jsv_mismatch(d, LOFT_JD_BOOL, t->name, fd->name);
            if (d == LOFT_JD_BOOL)
                *loft_at(loft_store_of(slot), slot.rec, slot.pos) = *loft_at(loft_store_of(sub), sub.rec, sub.pos + LOFT_JV_BOOL_VALUE);
            else loft_pw_absent(w, fd->content, kt, f, slot);
            break;
        case LOFT_P_TEXT:
            loft_jsv_mismatch(d, LOFT_JD_STRING, t->name, fd->name);
            if (d == LOFT_JD_STRING) {
                loft_text s = loft_text_own(loft_get_text(sub, LOFT_JV_STRING_VALUE));
                loft_set_text(slot, 0, loft_view(&s));
                loft_text_free(&s);
            } else {
                loft_pw_absent(w, fd->content, kt, f, slot);
            }
            break;
        case LOFT_P_CHAR:
            if (loft_jsv_char(sub, d, t->name, fd->name, &cp)) loft_p_bytes(slot, &cp, 4);
            else loft_pw_absent(w, fd->content, kt, f, slot);
            break;
        case LOFT_P_BYTE: case LOFT_P_SHORT: case LOFT_P_SHORTRAW: case LOFT_P_NINT: case LOFT_P_NINTRAW:
            if (loft_jsv_long(sub, d, t->name, fd->name, &l)) loft_pw_narrow(c, l, slot);
            else loft_pw_absent(w, fd->content, kt, f, slot);
            break;
        case LOFT_P_RECORD:
            if (!c->variant) {
                loft_jsv_struct(w, slot, fd->content, sub);
                break;
            }
            memset(loft_at(loft_store_of(slot), slot.rec, slot.pos), 0, c->size);
            break;
        case LOFT_P_ENUM:
            if (loft_p_is_json(w, fd->content)) loft_jsv_copy(slot, sub);
            else memset(loft_at(loft_store_of(slot), slot.rec, slot.pos), 0, c->size);
            break;
        case LOFT_P_VECTOR:
            loft_p_u32(slot, 0);
            if (d == LOFT_JD_ARRAY) loft_jsv_vector(w, slot, c->elem, sub);
            break;
        default:
            memset(loft_at(loft_store_of(slot), slot.rec, slot.pos), 0, c->size);
            break;
        }
    }
}

/* `n_struct_from_jsonvalue`: a new store holding the `kt` the tree at `src` describes. */
static loft_ref loft_struct_from_jsonvalue(const loft_ptype *P, loft_ref src, uint16_t kt) {
    loft_pw w;
    w.P = P;
    w.unfilled = 0;
    loft_ptab = P;
    uint32_t words = ((uint32_t) P[kt].size + 7) / 8 + 1;
    if (words < 2) words = 2;
    loft_ref r = loft_ref_null();
    loft_database(&r, (words - 1) * 8);
    size_t seen = loft_jerr_n;
    loft_jsv_struct(&w, r, kt, src);
    loft_perrs *pe = loft_perrs_of(r.store_nr);
    loft_perrs_clear(pe);
    pe->n = loft_jerr_n - seen;
    pe->e = pe->n ? (loft_text *) calloc(pe->n, sizeof *pe->e) : 0;
    for (size_t i = 0; i < pe->n; i++) pe->e[i] = loft_text_own(loft_view(&loft_jerr[seen + i]));
    pe->tp = kt;
    return r;
}
#endif
#endif
