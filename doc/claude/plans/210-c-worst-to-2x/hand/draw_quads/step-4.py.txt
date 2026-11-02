# D: loop versioning per row — the inlined accessors' bounds tests (0 <= x < w, 0 <= y < h,
# idx < len) are affine in px over px in [0, gw): test the row's two ends once, then a loop
# with none.  Same reads and writes in the same order, so a canvas aliasing the sheet is safe.
import sys; p=sys.argv[1]; s=open(p).read()
f0 = s.index("static void n_draw_quads__inv__fr(loft_ref v0_cv, loft_ref v1_atlas, loft_ref v2_quads, int64_t v3_colour, int64_t is0, int64_t is1, int64_t is2, int64_t is3, loft_vh hi0, loft_vh hi1, loft_vh hi2) {")
f1 = s.index("\n}\n", f0)
body = s[f0:f1]
a = """            v16__range_end_2 = v4_gw;
            v17_px_index = INT64_C(-1);"""
assert body.count(a) == 1
fast = """            {
                int64_t gy = loft_wrap_add(v11_sy, v15_py), ty = loft_wrap_add(s9, v15_py);
                if (gy >= 0 && gy < s5 && ty >= 0 && ty < is3 && v9_sx >= 0 && v9_sx + v4_gw <= s4
                    && s8 >= 0 && s8 + v4_gw <= is2
                    && gy * s4 + v9_sx + v4_gw <= hi1.len && ty * is2 + s8 + v4_gw <= hi2.len) {
                    const uint8_t *sp = hi1.base + (size_t) (gy * s4 + v9_sx) * 8;
                    uint8_t *tp = hi2.base + (size_t) (ty * is2 + s8) * 8;
                    for (int64_t px = 0; px < v4_gw; px++) {
                        int64_t sv; memcpy(&sv, sp + px * 8, 8);
                        if (sv != 0) memcpy(tp + px * 8, &v3_colour, 8);
                    }
                    goto cont_2;
                }
            }
"""
body = body.replace(a, fast + a)
s = s[:f0] + body + s[f1:]
open(p,'w').write(s)
