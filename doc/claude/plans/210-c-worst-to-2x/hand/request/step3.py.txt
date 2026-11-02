# step 3: (R-LitDiv) on the C — `v / 65536` and `v / 256` with a non-zero literal divisor and a
# dividend proven non-null (wrapping arithmetic over `?? 0` results) are plain divisions; the
# `?? 0` after them is dead.
import sys
src, dst = sys.argv[1], sys.argv[2]
c = open(src).read()
s = c.index("static loft_ref n_b64_decode__fr(loft_str v0_b64, loft_ref v1___retbuf, loft_ref v2_vals) {")
e = c.index("\n}\n", s)
f = c[s:e]
n = f.count("loft_div(v38_v, INT64_C(65536))") + f.count("loft_div(v38_v, INT64_C(256))")
assert n == 2, n
f = f.replace("loft_div(v38_v, INT64_C(65536))", "(v38_v / INT64_C(65536))").replace("loft_div(v38_v, INT64_C(256))", "(v38_v / INT64_C(256))")
c = c[:s] + f + c[e:]
open(dst, "w").write(c)
