# B: the element read/write after `0 <= idx < h.len` is proven (gx, gy >= 0 tested; idx < len
# tested just above) — no loft_vh_index fold or range test, a direct base + idx*8 access.
import sys; p=sys.argv[1]; s=open(p).read()
for a,b in [("loft_vh_get_int(h3, 8, v3_gp_idx, 0)", "loft_vh_get_int_in(h3, v3_gp_idx)"),
            ("loft_vh_set_int(h3, 8, v4_sp_idx, 0, v3_color)", "loft_vh_set_int_in(h3, v4_sp_idx, v3_color)")]:
    assert a in s, a; s=s.replace(a,b)
helper = """
LOFT_HOT_INLINE int64_t loft_vh_get_int_in(loft_vh h, int64_t f) { int64_t v; memcpy(&v, h.base + (size_t) f * 8, 8); return v; }
LOFT_HOT_INLINE void loft_vh_set_int_in(loft_vh h, int64_t f, int64_t v) { memcpy(h.base + (size_t) f * 8, &v, 8); }
"""
k = s.index("LOFT_HOT_INLINE int64_t loft_vh_get_int(")
s = s[:k] + helper + s[k:]
open(p,'w').write(s)
