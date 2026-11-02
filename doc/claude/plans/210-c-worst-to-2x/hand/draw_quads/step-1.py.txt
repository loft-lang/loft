# A: the pixel index `gy * width + gx` as a wrapping add: both operands non-null and the
# dominating bounds tests (0 <= gx < width, 0 <= gy < height) bound the sum.
import sys; p=sys.argv[1]; s=open(p).read()
for a in ["v3_gp_idx = loft_add(t9, v1_gx);", "v4_sp_idx = loft_add(t7, v1_sx);"]:
    assert a in s, a
    s=s.replace(a, a.replace("loft_add(", "loft_wrap_add("))
open(p,'w').write(s)
