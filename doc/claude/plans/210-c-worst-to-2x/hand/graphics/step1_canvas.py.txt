# Step 1 (canvas): filled() reserves `count` once, then one store loop — no doubling copies, no regrowth.
import re, sys
s = open(sys.argv[1]).read()
start = s.index("static loft_ref n_filled__fr(int64_t v0_count, int64_t v1_value, loft_ref v3_fi_out) {")
end = s.index("\n}\n", start) + 3
body = '''static loft_ref n_filled__fr(int64_t v0_count, int64_t v1_value, loft_ref v3_fi_out) {
    loft_vec_clear_release(loft_types, loft_ntypes, v3_fi_out);
    if (v0_count <= 0) return v3_fi_out;
    loft_vec_reserve_run(v3_fi_out, v0_count, 8);
    loft_vh h = loft_vh_of(v3_fi_out);
    uint8_t *p = h.base;
    for (int64_t i = 0; i < v0_count; i++) memcpy(p + (size_t) i * 8, &v1_value, 8);
    loft_set_u32(loft_store_of(v3_fi_out), h.rec, 4, (uint32_t) v0_count);
    return v3_fi_out;
}
'''
s = s[:start] + body + s[end:]
open(sys.argv[2], "w").write(s)
