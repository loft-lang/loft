# fill_rect steps, cumulative.  usage: step_rect.py N in.c out.c
#  1: the index add `y*w + x` wraps (both operands proven non-null: a range index and a wrapping mul)
#  2: the store skips loft_vh_index: loop versioned on width >= 0 (then 0 <= idx, and idx < n is the
#     library's own guard), the store goes straight to base + idx*8
#  3: the library's per-pixel `idx < n` guard becomes the row's end (idx is monotone in x):
#     x_end = min(x1, n - y*w) per row, a plain store loop gcc/clang vectorise
import sys
N = int(sys.argv[1]); s = open(sys.argv[2]).read()
fn = s.index("loft_vh hi0) {", s.index("static void t_6Canvas_fill_rect__inv__fr(", s.index("static void t_6Canvas_fill_rect__inv__fr(") + 10))
old_add = "            v22_fr_idx = loft_add(t10, v21_fr_x);\n"
old_store = "(LOFT_PROVED(loft_vh_same(h3, v10_fr_d), \"fill_rect:h3\"), loft_vh_set_int(h3, 8, v22_fr_idx, 0, v5_color));"
i_add = s.index(old_add, fn); 
if N >= 1:
    s = s[:i_add] + old_add.replace("loft_add(", "loft_wrap_add(") + s[i_add + len(old_add):]
if N >= 2:
    i_st = s.index(old_store, fn)
    new = "if (is0 >= 0) memcpy(h3.base + (size_t) v22_fr_idx * 8, &v5_color, 8); else loft_vh_set_int(h3, 8, v22_fr_idx, 0, v5_color);"
    s = s[:i_st] + new + s[i_st + len(old_store):]
if N >= 3:
    # replace the whole inner x loop with a clamped plain loop when width >= 0
    a = s.index("        v20__next_2 = v17__range_start_2;\n", fn)
    b = s.index("        brk_2:;\n", a) + len("        brk_2:;\n")
    new = '''        if (is0 >= 0) {
            int64_t row = loft_wrap_mul(v16_fr_y, is0);
            int64_t xe = v18__range_end_2;
            if (v11_fr_n - row < xe) xe = v11_fr_n - row;
            uint8_t *p = h3.base + (size_t) row * 8;
            for (int64_t x = v17__range_start_2; x < xe; x++) memcpy(p + (size_t) x * 8, &v5_color, 8);
        } else {
''' + s[a:b] + "        }\n"
    s = s[:a] + new + s[b:]
open(sys.argv[3], "w").write(s)
