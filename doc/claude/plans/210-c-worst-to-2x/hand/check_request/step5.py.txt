# A read-only lookup answers its argument's field as a VIEW: `pa_get` returns `e.value` without
# copying it into a buffer its caller (`pa_text`) mints per call.
import sys
from lib import *
src = open(sys.argv[1]).read()
def pg(b):
    return sub1(b, '''                    loft_database_typed(&v2___retbuf, 16, 101);
                    t7 = loft_field(v6_e, 16);
                    loft_copy_record(loft_types, 101, t7, v2___retbuf, 2);
                    loft_text_free(&v7__mv_value_1);
                    loft_free_var_distinct(&v9___ref_2, v2___retbuf);
                    t8 = v2___retbuf;''', '''                    t8 = loft_field(v6_e, 16);''')
def pt(b):
    b = sub1(b, '''    if (v2___ref_1.store_nr == 0xFFFF) {
        loft_database_typed(&v2___ref_1, 16, 101);
    } else {
        if (!loft_reset_pool_root(v2___ref_1, 101, 16, loft_dflt101, 16)) loft_remove_claims(loft_types, 101, v2___ref_1);
    }
''', '')
    b = b.replace('loft_free_var_distinct(&v3__match_subj_1, v2___ref_1);\n', '')
    return b
src = edit_fn(src, 'static loft_ref n_pa_get__fr(', pg)
src = edit_fn(src, 'static loft_text n_pa_text__fr(', pt)
open(sys.argv[2], 'w').write(src)
