# step 4 (R-Transient): slice_runs' result is built outside the store — a frame-kept C array of
# {text, style}; each slot's text keeps its room across calls.  token_width and place_line's two
# passes read it.  No store vector, no Run record, no text claim per run.
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
helper='''
typedef struct { loft_text str; int64_t style; } tr_run;
typedef struct { tr_run *a; int32_t n, init, cap; } tr_runs;
static inline tr_run *tr_push(tr_runs *r) {
    if (r->n == r->cap) { r->cap = r->cap ? r->cap * 2 : 4; r->a = realloc(r->a, (size_t) r->cap * sizeof(tr_run)); }
    if (r->n == r->init) { r->a[r->init].str = (loft_text) { 0, 0, 0 }; r->init++; }
    return &r->a[r->n++];
}
static void slice_runs_tr(loft_ref d, int64_t a, int64_t b, tr_runs *out) {
    out->n = 0;
    if (b <= a) return;
    int64_t cum = 0;
    loft_ref pv = loft_field(d, 4);
    int64_t np = t_6vector_len(pv);
    loft_vh hp = loft_vh_of(pv);
    loft_ref bv = loft_field(d, 0);
    loft_vh hb = loft_vh_of(bv);
    for (int64_t i = 0; i < np; i++) {
        uint8_t *pa = loft_rec_ptr(loft_vh_get(hp, 24, i));
        int64_t ps = cum, pe = loft_wrap_add(cum, loft_pa_i64(pa, 8));
        int64_t lo = n_clampi(a, ps, pe), hi = n_clampi(b, ps, pe);
        if (lo < hi) {
            tr_run *e = tr_push(out);
            bst_into(bv, loft_pa_i64(pa, 0) + (lo - ps), hi - lo, &e->str, hb);
            e->style = loft_pa_i64(pa, 16);
        }
        cum = pe;
    }
}
'''
a=s.index('\nstatic loft_ref n_slice_runs__fr(loft_ref v0_d, int64_t v1_a, int64_t v2_b, loft_ref v18___vdb_1) {')
s=s[:a]+helper+s[a:]
# token_width: whole body
tw='''static double n_token_width__fr(loft_ref v0_d, loft_ref v1_tok, loft_fn v2_resolve, loft_fn v3_measure, loft_ref v6___ref_1) {
    static tr_runs runs; static loft_ref keep_st = { 0xFFFF, 0, 0 };
    (void) v6___ref_1;
    if (loft_get_bool(v1_tok, 17) == 1) return 0.0;
    double w = 0.0;
    slice_runs_tr(v0_d, loft_get_int(v1_tok, 0), loft_get_int(v1_tok, 8), &runs);
    for (int32_t i = 0; i < runs.n; i++) {
        loft_ref t5 = keep_st;
        int64_t t7 = (int64_t) loft_stores_allocated;
        loft_ref t6 = loft_call0b(v2_resolve, runs.a[i].style, keep_st);
        if (t6.store_nr != 0xFFFF && t6.store_nr == keep_st.store_nr && t6.rec == keep_st.rec && t6.pos == keep_st.pos) {
        } else if (loft_fnref_minted(t6, (uint64_t) t7, v2_resolve.clos)) {
            loft_fnref_adopt(t6);
            if (t5.store_nr != 0xFFFF && t5.store_nr != t6.store_nr) loft_free_ref(t5);
            keep_st = t6;
        } else {
            loft_database_typed(&keep_st, 32, 118);
            loft_copy_record(loft_types, 118, t6, keep_st, 4);
        }
        w += loft_call1(v3_measure, loft_view(&runs.a[i].str), keep_st);
    }
    return w;
}
'''
import re
m=re.search(r'\nstatic double n_token_width__fr\(loft_ref v0_d[^;{]*\) \{\n', s)
a=m.start()+1; b=s.index('\n}\n',a)+3
s=s[:a]+tw+s[b:]
# place_line pass 1
s=edit(s,'n_place_line__fr',[
 ('            v18_hruns = n_slice_runs(v0_d, t5, t6, v19___ref_1);\n','            slice_runs_tr(v0_d, t5, t6, &pl_runs);\n'),
 ('            v20__range_end_2 = t_6vector_len(v18_hruns);\n','            v20__range_end_2 = pl_runs.n;\n'),
 ('            h1 = loft_vh_of(v18_hruns);\n',''),
 ('''                v24___ncc_2 = (LOFT_PROVED(loft_vh_same(h1, v18_hruns), "place_line:h1"), loft_vh_get(h1, 16, v22_ri));
                if (v24___ncc_2.rec != 0) {
                    t10 = v24___ncc_2;
                } else {
                    loft_database_typed(&v60___ref_p2_2, 16, 106);
                    loft_set_text(v60___ref_p2_2, 8, loft_lit("", 0));
                    loft_set_int(v60___ref_p2_2, 0, INT64_C(0));
                    t10 = v60___ref_p2_2;
                }
                t11 = loft_get_int(t10, 0);
''','''                t11 = pl_runs.a[v22_ri].style;
'''),
 ('            v37_pruns = n_slice_runs(v0_d, t17, t18, v25___ref_2);\n','            slice_runs_tr(v0_d, t17, t18, &pl_runs);\n'),
 ('            v40__range_end_4 = t_6vector_len(v37_pruns);\n','            v40__range_end_4 = pl_runs.n;\n'),
 ('            h2 = loft_vh_of(v37_pruns);\n',''),
 ('''                v44___ncc_4 = (LOFT_PROVED(loft_vh_same(h2, v37_pruns), "place_line:h2"), loft_vh_get(h2, 16, v42_rj));
                if (v44___ncc_4.rec != 0) {
                    v43_r = v44___ncc_4;
                } else {
                    loft_database_typed(&v63___ref_p2_4, 16, 106);
                    loft_set_text(v63___ref_p2_4, 8, loft_lit("", 0));
                    loft_set_int(v63___ref_p2_4, 0, INT64_C(0));
                    v43_r = v63___ref_p2_4;
                }
''','''                tr_run *trr = &pl_runs.a[v42_rj];
'''),
 ('                t22 = loft_get_int(v43_r, 0);\n','                t22 = trr->style;\n'),
 ('                v46_rw = loft_call1(v8_measure, loft_get_text(v43_r, 8), v45_st);','                v46_rw = loft_call1(v8_measure, loft_view(&trr->str), v45_st);'),
 ('                t30 = t_4text_len(loft_get_text(v43_r, 8));','                t30 = t_4text_len(loft_view(&trr->str));'),
 ('                t32 = (int64_t) (loft_get_int(v43_r, 0));','                t32 = (int64_t) (trr->style);'),
 ('    static loft_ref keep_l1','    static tr_runs pl_runs;\n    static loft_ref keep_l1'),
])
open(sys.argv[2],'w').write(s)
