# fill_polygon steps, cumulative.  usage: step_poly.py N in.c out.c
#  1: hline's span loop as fill_rect's steps 2+3: versioned on width >= 0, the `idx < n` guard
#     turned into the span's end, a plain store loop
#  2: polygon_crossings (the branch where the output store differs from pts'): pc_a / pc_b
#     addresses from the held header's base (pts is not grown in the loop; i and (i+1)%n are
#     proven in range), no null-element fallback; pc_ty / pc_by copy the address they name
#  3: `(i + 1) % n` without the null/zero test (n > i >= 0 inside the loop)
#  4: the buffer's double clear at entry becomes one
import sys
N = int(sys.argv[1]); s = open(sys.argv[2]).read()
def defn(name):
    i = s.index(name + "(")
    while True:
        j = s.index("\n", i)
        if s[j - 1] == "{": return i
        i = s.index(name + "(", j)
def rep(old, new, start, count=1):
    global s
    for _ in range(count):
        k = s.index(old, start); s = s[:k] + new + s[k + len(old):]
if N >= 1:
    h = defn("static inline void t_6Canvas_hline__inv__fr")
    a = s.index("    v12__next_1 = v9__range_start_1;\n", h)
    b = s.index("    brk_1:;\n", a) + len("    brk_1:;\n")
    new = '''    if (is1 >= 0) {
        int64_t row = loft_wrap_mul(v3_hy, is1);
        int64_t xe = v10__range_end_1;
        if (v8_hl_n - row < xe) xe = v8_hl_n - row;
        uint8_t *p = h3.base + (size_t) row * 8;
        for (int64_t x = v9__range_start_1; x < xe; x++) memcpy(p + (size_t) x * 8, &v4_color, 8);
        return;
    }
'''
    s = s[:a] + new + s[a:]
if N >= 2:
    c = defn("static loft_ref n_polygon_crossings__fr")
    e = s.index("    } else {\n        ph12 = loft_ph_of", c)
    seg = s[c:e]
    seg = seg.replace('''            v10___ncc_1 = (LOFT_PROVED(loft_vh_same(h1, v0_pc_pts), "polygon_crossings:h1"), loft_vh_get(h1, 16, v8_i));
            if (v10___ncc_1.rec != 0) {
                v9_pc_a = v10___ncc_1;
            } else {
                loft_database_typed(&v28___ref_p2_1, 16, 134);
                loft_set_int(v28___ref_p2_1, 0, INT64_C(0));
                loft_set_int(v28___ref_p2_1, 8, INT64_C(0));
                v9_pc_a = v28___ref_p2_1;
            }
            loft_pa9_3 = loft_rec_ptr(v9_pc_a);''', '''            loft_pa9_3 = h1.base + (size_t) v8_i * 16;''')
    seg = seg.replace('''            v13___ncc_3 = (LOFT_PROVED(loft_vh_same(h1, v0_pc_pts), "polygon_crossings:h1"), loft_vh_get(h1, 16, t2));
            if (v13___ncc_3.rec != 0) {
                v11_pc_b = v13___ncc_3;
            } else {
                loft_database_typed(&v29___ref_p2_2, 16, 134);
                loft_set_int(v29___ref_p2_2, 0, INT64_C(0));
                loft_set_int(v29___ref_p2_2, 8, INT64_C(0));
                v11_pc_b = v29___ref_p2_2;
            }
            loft_pa11_5 = loft_rec_ptr(v11_pc_b);''', '''            loft_pa11_5 = h1.base + (size_t) t2 * 16;''')
    seg = seg.replace('''                v16_pc_ty = v9_pc_a;
            } else {
                v16_pc_ty = v11_pc_b;
            }
            loft_pa16_7 = loft_rec_ptr(v16_pc_ty);''', '''                loft_pa16_7 = loft_pa9_3;
            } else {
                loft_pa16_7 = loft_pa11_5;
            }''')
    seg = seg.replace('''                v17_pc_by = v11_pc_b;
            } else {
                v17_pc_by = v9_pc_a;
            }
            loft_pa17_9 = loft_rec_ptr(v17_pc_by);''', '''                loft_pa17_9 = loft_pa11_5;
            } else {
                loft_pa17_9 = loft_pa9_3;
            }''')
    assert seg.count("loft_rec_ptr") == 0, seg.count("loft_rec_ptr")
    # loft_pa_i64's null-pointer test goes too: the pointers are never null now
    seg = seg.replace("loft_pa_i64(loft_pa", "loft_pa_i64nn(loft_pa")
    if N >= 3:
        seg = seg.replace("            v12___ncc_2 = loft_rem(t3, v5_pc_n);", "            v12___ncc_2 = t3 % v5_pc_n;")
    if N >= 4:
        seg = seg.replace("    loft_vec_clear_release(loft_types, loft_ntypes, v4_pc_out);\n    loft_vec_clear_release(loft_types, loft_ntypes, v4_pc_out);\n", "    loft_vec_clear_release(loft_types, loft_ntypes, v4_pc_out);\n", 1)
    s = s[:c] + seg + s[e:]
    k = s.index("LOFT_HOT_INLINE double loft_pa_f64(")
    s = s[:k] + "LOFT_HOT_INLINE int64_t loft_pa_i64nn(const uint8_t *p, uint32_t off) { int64_t v; memcpy(&v, p + off, 8); return v; }\n" + s[k:]
open(sys.argv[3], "w").write(s)
