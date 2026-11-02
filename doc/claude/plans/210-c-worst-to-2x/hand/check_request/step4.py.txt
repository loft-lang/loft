# (R-TextBorrow) match-binding clause: `CText { value }` bound in a match on a read-only
# subject is a VIEW of the field, not an owned copy (pa_get's key compare, pa_text's value).
import sys
from lib import *
src = open(sys.argv[1]).read()
def pg(b):
    b = sub1(b, '''                loft_text_free(&t6);
                t6 = loft_text_own(dv_text(loft_field(v6_e, 0), 4));
                loft_text_move(&v7__mv_value_1, &t6);
                loft_text_free(&t6);
                if (loft_str_eq(loft_view(&v7__mv_value_1), v1_key)) {''',
'''                if (loft_str_eq(dv_text(loft_field(v6_e, 0), 4), v1_key)) {''')
    b = sub1(b, '''            loft_text_set(&v7__mv_value_1, loft_lit("", 0));
            t4 = loft_get_enum''', '''            t4 = loft_get_enum''')
    return b
def pt(b):
    b = sub1(b, '''        loft_text_free(&t4);
        t4 = loft_text_own(dv_text(v3__match_subj_1, 4));
        loft_text_move(&v4__mv_value_1, &t4);
        loft_text_free(&t4);
        loft_text_set(&(*v5___work_1), loft_lit("", 0));
        loft_format_text(&(*v5___work_1), loft_view(&v4__mv_value_1), INT64_C(0), 2, 32);''',
'''        loft_text_set(&(*v5___work_1), loft_lit("", 0));
        loft_format_text(&(*v5___work_1), dv_text(v3__match_subj_1, 4), INT64_C(0), 2, 32);''')
    return b
src = edit_fn(src, 'static loft_ref n_pa_get__fr(', pg)
src = edit_fn(src, 'static loft_text n_pa_text__fr(', pt)
open(sys.argv[2], 'w').write(src)
