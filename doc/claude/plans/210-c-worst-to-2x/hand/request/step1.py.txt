# step 1: b64_value — a pure function of one integer whose result is constant-derived over a
# small domain (0..255, every other input -> -1) — answered from a 256-entry table built once.
import sys
src, dst = sys.argv[1], sys.argv[2]
c = open(src).read()
s = c.index("static int64_t n_b64_value__fr(int64_t v0_c) {")
e = c.index("\n}\n", s) + 3
def val(x):
    if 65 <= x <= 90: return x - 65
    if 97 <= x <= 122: return x - 71
    if 48 <= x <= 57: return x + 4
    if x in (43, 45): return 62
    if x in (47, 95): return 63
    return -1
tab = ", ".join(str(val(i)) for i in range(256))
body = f'''static const int8_t loft_tab_b64_value[256] = {{ {tab} }};
static int64_t n_b64_value__fr(int64_t v0_c) {{
    if ((uint64_t) v0_c < 256) return loft_tab_b64_value[v0_c];
    return (-(INT64_C(1)));
}}
'''
c = c[:s] + body + c[e:]
open(dst, "w").write(c)
