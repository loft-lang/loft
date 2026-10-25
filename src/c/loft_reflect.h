#ifdef LOFT_REFLECT
/* ── Type reflection (native.rs `reflect_type_into`, `type_named_in`) ──────────────────────
 * Each type's answer is the interpreter's, read back by the emitter into `loft_rtypes`
 * (`reflect.rs`); a `TypeInfo` is built from its row by the writes `reflect_type_into` makes,
 * in its order: the record, its name, kind, size, collection and element, then the field,
 * variant and key vectors each claimed whole and filled. */
typedef struct { const char *name, *type_name; int64_t pos; uint8_t kind, nullable; } loft_rfield;
typedef struct { const char *name; int64_t pos; uint8_t asc; } loft_rkey;
/* A data field as `reflect_field_at` reads it: `cls` 1 integer, 2 long, 3 float, 4 single,
 * 5 boolean, 6 character, 7 text, 8 byte, 9 short, 10 raw short, 11 int, 0 no scalar. */
typedef struct { uint16_t pos, content; uint8_t kind, cls; int32_t start; uint8_t nullable, inline_rec; } loft_rdata;
typedef struct {
    const char *lookup;         /* the type's own name, as `Stores::name` finds it */
    const char *name;           /* the name reflection reports */
    uint8_t kind;
    int64_t size;
    const char *element;
    uint8_t collection;
    int nf;                     /* -1: no field vector */
    const loft_rfield *f;
    int nv;
    const char *const *v;
    int nk;
    const loft_rkey *k;
    int nd;
    const loft_rdata *d;
} loft_rtype;
/* The reflection records' layout: TypeInfo's type and the four sizes, then each position. */
typedef struct {
    uint16_t ti_tp;
    uint32_t ti_bytes, fi_bytes, vi_bytes, ki_bytes;
    uint32_t ti_name, ti_kind, ti_size, ti_fields, ti_variants, ti_element, ti_collection, ti_keys;
    uint32_t fi_name, fi_type, fi_pos, fi_kind, fi_null, vi_name, vi_tag, ki_name, ki_pos, ki_asc;
    uint16_t vf_tp;
    uint32_t vf_bytes, vf_kind, vf_null, vf_i, vf_f, vf_t;
} loft_rlayout;

/* `Store::set_str`: a text record of `n` bytes in `s`, its number answered. */
static uint32_t loft_rstr_n(loft_store_t *s, const char *t, size_t n) {
    uint32_t srec = loft_st_claim(s, (uint32_t) ((n + 15) / 8));
    loft_set_u32(s, srec, 4, (uint32_t) n);
    if (n) memcpy(loft_at(s, srec, 8), t, n);
    return srec;
}

static uint32_t loft_rstr(loft_store_t *s, const char *t) {
    return loft_rstr_n(s, t, strlen(t));
}

static void loft_rint(loft_store_t *s, uint32_t rec, uint32_t at, int64_t v) {
    memcpy(loft_at(s, rec, at), &v, 8);
}

/* A vector of `n` elements of `bytes`, claimed whole, its length written and its handle at
 * `(rec, at)`; the vector's record answered. */
static uint32_t loft_rvec(loft_store_t *s, uint32_t rec, uint32_t at, uint32_t n, uint32_t bytes) {
    uint32_t words = (n * bytes + 15) / 8 + 1;
    uint32_t v = loft_claim(s, words);
    loft_set_u32(s, v, 4, n);
    loft_set_u32(s, rec, at, v);
    return v;
}

static loft_ref loft_reflect_build(const loft_rlayout *L, const loft_rtype *t, const char *name) {
    loft_ref out = loft_ref_null();
    loft_database(&out, ((L->ti_bytes + 7) / 8) * 8);
    loft_store_t *s = loft_store_of(out);
    uint32_t r = out.rec, p = out.pos;
    loft_set_u32(s, r, 4, L->ti_tp);
    loft_set_u32(s, r, p + L->ti_name, loft_rstr(s, name));
    *loft_at(s, r, p + L->ti_kind) = t->kind;
    loft_rint(s, r, p + L->ti_size, t->size);
    loft_set_u32(s, r, p + L->ti_fields, 0);
    loft_set_u32(s, r, p + L->ti_variants, 0);
    loft_set_u32(s, r, p + L->ti_keys, 0);
    *loft_at(s, r, p + L->ti_collection) = t->collection;
    loft_set_u32(s, r, p + L->ti_element, loft_rstr(s, t->element));
    if (t->nf >= 0) {
        uint32_t v = loft_rvec(s, r, p + L->ti_fields, (uint32_t) t->nf, L->fi_bytes);
        for (int i = 0; i < t->nf; i++) {
            uint32_t at = 8 + (uint32_t) i * L->fi_bytes;
            loft_set_u32(s, v, at + L->fi_name, loft_rstr(s, t->f[i].name));
            loft_set_u32(s, v, at + L->fi_type, loft_rstr(s, t->f[i].type_name));
            loft_rint(s, v, at + L->fi_pos, t->f[i].pos);
            *loft_at(s, v, at + L->fi_kind) = t->f[i].kind;
            *loft_at(s, v, at + L->fi_null) = t->f[i].nullable;
        }
    }
    if (t->nv >= 0) {
        uint32_t v = loft_rvec(s, r, p + L->ti_variants, (uint32_t) t->nv, L->vi_bytes);
        for (int i = 0; i < t->nv; i++) {
            uint32_t at = 8 + (uint32_t) i * L->vi_bytes;
            loft_set_u32(s, v, at + L->vi_name, loft_rstr(s, t->v[i]));
            loft_rint(s, v, at + L->vi_tag, i + 1);
        }
    }
    if (t->nk >= 0) {
        uint32_t v = loft_rvec(s, r, p + L->ti_keys, (uint32_t) t->nk, L->ki_bytes);
        for (int i = 0; i < t->nk; i++) {
            uint32_t at = 8 + (uint32_t) i * L->ki_bytes;
            loft_set_u32(s, v, at + L->ki_name, loft_rstr(s, t->k[i].name));
            loft_rint(s, v, at + L->ki_pos, t->k[i].pos);
            *loft_at(s, v, at + L->ki_asc) = t->k[i].asc;
        }
    }
    return out;
}

/* `reflect_field_at`: the ValueInfo of the scalar that begins at `position` of the `kt` record
 * at `base` in `value`'s store — `OtherKind` (14) when no data field begins there or there is
 * no record to read; `is_null` when the scalar there holds loft's null. */
static loft_ref loft_reflect_field_at(const loft_rlayout *L, const loft_rtype *T, uint32_t nt, loft_ref value,
                                      uint32_t base, uint16_t kt, int64_t position) {
    const loft_rdata *e = 0;
    if (kt < nt && position >= 0 && position <= 0xFFFF)
        for (int i = 0; i < T[kt].nd; i++)
            if (T[kt].d[i].pos == (uint16_t) position) { e = &T[kt].d[i]; break; }
    uint8_t kind = e && value.rec != 0 ? e->kind : 14;
    loft_ref out = loft_ref_null();
    loft_database(&out, ((L->vf_bytes + 7) / 8) * 8);
    loft_store_t *s = loft_store_of(out);
    loft_set_u32(s, out.rec, 4, L->vf_tp);
    *loft_at(s, out.rec, out.pos + L->vf_kind) = kind;
    loft_rint(s, out.rec, out.pos + L->vf_i, 0);
    double zero = 0.0;
    memcpy(loft_at(s, out.rec, out.pos + L->vf_f), &zero, 8);
    loft_set_u32(s, out.rec, out.pos + L->vf_t, 0);
    int is_null = 0;
    if (kind != 14) {
        loft_store_t *vs = loft_store_of(value);
        const uint8_t *a = loft_at(vs, value.rec, base + e->pos);
        int64_t iv = 0;
        double fv = 0.0;
        loft_text tv = { 0, 0, 0 };
        int has_t = 0;
        switch (e->cls) {
        case 1: case 2: memcpy(&iv, a, 8); is_null = iv == INT64_MIN; break;
        case 3: memcpy(&fv, a, 8); is_null = fv != fv; break;
        case 4: { float x; memcpy(&x, a, 4); fv = (double) x; is_null = x != x; break; }
        case 5: iv = a[0] & 1; is_null = a[0] == 255; break;
        case 6: { uint32_t c; memcpy(&c, a, 4); iv = c; is_null = c == 0; break; }
        case 7: {
            uint32_t r;
            memcpy(&r, a, 4);
            loft_str t = loft_store_str(vs, r);
            is_null = loft_str_is_null(t);
            if (!is_null && r != 0) { tv = loft_text_own(t); has_t = 1; }
            break;
        }
        case 8:
            is_null = e->nullable && a[0] == 255;
            iv = is_null ? INT32_MIN : (int64_t) a[0] + e->start;
            break;
        case 9: {
            uint16_t r;
            memcpy(&r, a, 2);
            iv = r ? (int64_t) r + e->start - 1 : INT32_MIN;
            is_null = r == 0;
            break;
        }
        case 10: {
            uint16_t r;
            memcpy(&r, a, 2);
            iv = (int64_t) r + e->start;
            is_null = iv == -32768;
            break;
        }
        case 11: { int32_t x; memcpy(&x, a, 4); iv = x; is_null = x == INT32_MIN; break; }
        default: break;
        }
        if (!is_null) {
            loft_rint(s, out.rec, out.pos + L->vf_i, iv);
            memcpy(loft_at(s, out.rec, out.pos + L->vf_f), &fv, 8);
            if (has_t) loft_set_u32(s, out.rec, out.pos + L->vf_t, loft_rstr_n(s, tv.p ? tv.p : "", tv.len));
        }
        loft_text_free(&tv);
    }
    *loft_at(s, out.rec, out.pos + L->vf_null) = (uint8_t) is_null;
    return out;
}

/* `reflect_field_path_into`: each step but the last descends into an inline record. */
static loft_ref loft_reflect_field_path_at(const loft_rlayout *L, const loft_rtype *T, uint32_t nt, loft_ref value,
                                           loft_ref path, uint16_t kt) {
    int64_t n = loft_vec_len(path);
    if (n == 0) return loft_reflect_field_at(L, T, nt, value, value.pos, kt, -1);
    loft_store_t *ps = loft_store_of(path);
    uint32_t inner = loft_collection_rec(ps, path.rec, path.pos);
    uint16_t cur = kt;
    uint32_t pos = value.pos;
    for (int64_t i = 0; i + 1 < n; i++) {
        int64_t step;
        memcpy(&step, loft_at(ps, inner, 8 + (uint32_t) i * 8), 8);
        const loft_rdata *e = 0;
        if (cur < nt && step >= 0 && step <= 0xFFFF)
            for (int j = 0; j < T[cur].nd; j++)
                if (T[cur].d[j].pos == (uint16_t) step) { e = &T[cur].d[j]; break; }
        if (!e || !e->inline_rec) return loft_reflect_field_at(L, T, nt, value, pos, cur, -1);
        pos += e->pos;
        cur = e->content;
    }
    int64_t last;
    memcpy(&last, loft_at(ps, inner, 8 + (uint32_t) (n - 1) * 8), 8);
    return loft_reflect_field_at(L, T, nt, value, pos, cur, last);
}
#endif
