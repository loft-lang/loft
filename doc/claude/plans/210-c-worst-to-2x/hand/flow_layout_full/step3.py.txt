# step 3: the run's text goes from the builder's work buffer straight into the record's text field:
# buf_slice_text's returned copy (t4) and the caller's move into `s` are gone.
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
helper='''
static inline void bst_into(loft_ref v0_buf, int64_t start, int64_t count, loft_text *out, loft_vh hi0) {
    loft_text_set(out, loft_lit("", 0));
    for (int64_t j = start; j < start + count; j++) {
        uint32_t c = loft_get_char(loft_vh_get(hi0, 4, j), 0);
        loft_push_code(out, c != 0 ? c : loft_char_of(INT64_C(0)));
    }
}
'''
a=s.index('\nstatic loft_ref n_slice_runs__fr(loft_ref v0_d, int64_t v1_a, int64_t v2_b, loft_ref v18___vdb_1) {')
s=s[:a]+helper+s[a:]
s=edit(s,'n_slice_runs__fr',[
 ('''            loft_text_free(&t10);
            t10 = n_buf_slice_text__inv(t5, t8, t9, &v16___work_c1, h2);
            loft_text_move(&v15_s, &t10);
            loft_text_free(&t10);
''','''            bst_into(t5, t8, t9, &v16___work_c1, h2);
'''),
 ('loft_set_text(v17__elm_1, 8, loft_view(&v15_s));','loft_set_text(v17__elm_1, 8, loft_view(&v16___work_c1));')])
open(sys.argv[2],'w').write(s)
