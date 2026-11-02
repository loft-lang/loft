# (R-DecodeView) instance 1: a CText/CBytes field of a decoded value holds a SPAN of the frame
# in the record's spare word (offset 8): INT64_MIN | lo << 32 | n.  The owned field stays empty.
import re, sys
from lib import *
src = open(sys.argv[1]).read()
HELP = r'''
/* hand: (R-DecodeView) span helpers */
static const uint8_t *dv_frame;
static inline const uint8_t *dv_bytes(loft_ref v) {
    loft_store_t *s = loft_store_of(v);
    return loft_elem_base(s, loft_collection_rec(s, v.rec, v.pos));
}
static inline loft_str dv_text(loft_ref r, uint32_t fld) {
    int64_t w = loft_get_int(r, 8);
    if (w < 0) { loft_str t = { (const char *) dv_frame + ((uint64_t) w >> 32 & 0x7FFFFFFF), (size_t) (w & 0xFFFFFFFF) }; return t; }
    return loft_get_text(r, fld);
}
'''
src = sub1(src, 'static loft_tup0 n_read_value__d(loft_ref v0_bytes, int64_t v1_pos, int64_t v2_depth, loft_ref v3___retbuf);\n',
           'static loft_tup0 n_read_value__d(loft_ref v0_bytes, int64_t v1_pos, int64_t v2_depth, loft_ref v3___retbuf);\n' + HELP)
def rvd(b):
    # bytes arm: the slice into the field becomes the span word
    b = sub1(b, '''        t65 = (loft_field(loft_field(v3___retbuf, 0), 4));
        loft_vec_slice(t65, v0_bytes, v25__slice_lo_1, v26__slice_hi_1, 1);''',
'''        loft_set_int(t54, 8, INT64_MIN | (v25__slice_lo_1 << 32) | (v26__slice_hi_1 - v25__slice_lo_1));''')
    # text arm: no slice store, no text copy, no store text; validated once
    b = sub1(b, '''        loft_slice_take(loft_types, &v71___vdb_2, 4, 112);
        v69____iter_vec_2 = loft_field(v71___vdb_2, 0);''', '')
    b = sub1(b, '''        loft_vec_slice_view(v69____iter_vec_2, v0_bytes, v30__slice_lo_2, v31__slice_hi_2, 1);
        t77 = v69____iter_vec_2;
        loft_text_free(&t88);
        t88 = loft_text_from_bytes(t77);
        loft_text_move(&v91___work_p2_1, &t88);
        loft_text_free(&t88);
        loft_set_text(t76, 4, loft_view(&v91___work_p2_1));''',
'''        {
            int64_t sn = v31__slice_hi_2 - v30__slice_lo_2;
            if (sn > 0 && !loft_utf8_valid(dv_bytes(v0_bytes) + v30__slice_lo_2, (size_t) sn)) sn = 0;
            loft_set_int(t76, 8, INT64_MIN | (v30__slice_lo_2 << 32) | (sn < 0 ? 0 : sn));
        }''')
    return b
src = edit_fn(src, 'static loft_tup0 n_read_value__d__fr(', rvd)
src = edit_fn(src, 'static loft_ref n_pa_get__fr(', lambda b: sub1(b,
    't6 = loft_text_own(loft_get_text(loft_field(v6_e, 0), 4));', 't6 = loft_text_own(dv_text(loft_field(v6_e, 0), 4));'))
src = edit_fn(src, 'static loft_text n_pa_text__fr(', lambda b: sub1(b,
    't4 = loft_text_own(loft_get_text(v3__match_subj_1, 4));', 't4 = loft_text_own(dv_text(v3__match_subj_1, 4));'))
src = edit_fn(src, 'static loft_text n_check_request__fr(', lambda b: sub1(b,
    '    v2___ref_1 = loft_ref_null();\n', '    v2___ref_1 = loft_ref_null();\n    dv_frame = dv_bytes(v0_frame);\n'))
open(sys.argv[2], 'w').write(src)
