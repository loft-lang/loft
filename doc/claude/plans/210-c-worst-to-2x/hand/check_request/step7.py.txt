# ALTERNATIVE — (R-Transient) T2: the decoded tree built outside the store (lt_read: scalars in a
# C struct, texts/bytes views of the frame, one child array per map), ONE decode (PureReuse),
# `.ok` read and the map walked for "op".  argv[3] = "arena" puts the arrays in loft_ta_*.
import sys
from lib import *
src = open(sys.argv[1]).read()
arena = len(sys.argv) > 3 and sys.argv[3] == 'arena'
lt = open('/home/jurjen/workspace/loft-c-perf/target-exp/wt-rebase/target-exp/ab/pricing/transient.c').read()
if arena:
    lt = lt.replace('#ifndef LT_ARENA', '#define LT_ARENA 1\n#define LT_ALLOC(sz) loft_ta_alloc(sz)\n#ifndef LT_ARENA')
    lt = lt.replace('        free(v->kids);\n', '')
SITE = r'''
static loft_text lt_check_request(loft_ref frame) {
''' + ('    loft_ta_mark_t m = loft_ta_mark();\n' if arena else '') + r'''
    lt_dec d = lt_decode(frame);
    loft_text res;
    if (!d.ok) res = loft_text_own(loft_lit("malformed-frame", 15));
    else {
        loft_text op = { 0, 0, 0 };
        if (d.v.kind == 6)
            for (uint32_t k = 0; k < d.v.n; k++) {
                lt_val *kv = &d.v.kids[2 * k];
                if (kv->kind == 4 && kv->n == 2 && memcmp(kv->p, "op", 2) == 0) {
                    lt_val *vv = &d.v.kids[2 * k + 1];
                    if (vv->kind == 4 && vv->n) loft_push(&op, (const char *) vv->p, vv->n);
                    break;
                }
            }
        res = n_valid_op__fr(loft_view(&op)) ? loft_text_own(loft_lit("", 0)) : loft_text_own(loft_lit("unknown-op", 10));
        loft_text_free(&op);
    }
''' + ('    loft_ta_back(m);\n' if arena else '    lt_free(&d.v);\n') + r'''    return res;
}
'''
src = sub1(src, 'static loft_text n_check_request__fr(loft_ref v0_frame) {\n',
    lt + SITE + 'static loft_text n_check_request__fr(loft_ref v0_frame) {\n    return lt_check_request(v0_frame);\n')
open(sys.argv[2], 'w').write(src)
