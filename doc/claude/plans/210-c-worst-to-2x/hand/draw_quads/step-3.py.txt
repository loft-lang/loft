# C: `gp_d[idx] ?? 0` — the element's null test (an integer vector element may hold the
# null sentinel); dropped: priced as the integer-null cost, the twin has no null.
import sys; p=sys.argv[1]; s=open(p).read()
a="""            v5___ncc_1 = (LOFT_PROVED(loft_vh_same(h3, v4_gp_d), "get_pixel:h3"), loft_vh_get_int_in(h3, v3_gp_idx));
            if (v5___ncc_1 != LOFT_NULL) {
                t2 = v5___ncc_1;
            } else {
                LOFT_COLD_PATH();
                t2 = INT64_C(0);
            }"""
assert a in s
s=s.replace(a, """            t2 = loft_vh_get_int_in(h3, v3_gp_idx);""")
open(p,'w').write(s)
