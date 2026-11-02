# F1: the span loop's per-element index test hoisted to one range test (the index is affine in the
# loop variable and the range is non-empty and known); the body a plain store gcc can vectorise.
import sys
from fn import span
s = open(sys.argv[1]).read(); a, b = span(s, "n_pil_hline__inv__fr"); f = s[a:b]
i = f.index("    v12__next_1 = v9__range_start_1;\n"); j = f.index("    brk_1:;\n") + len("    brk_1:;\n")
new = """    if (v9__range_start_1 <= v10__range_end_1) {
        int64_t lo_i = v8_hl_base + v9__range_start_1, hi_i = v8_hl_base + v10__range_end_1;
        if (lo_i >= 0 && hi_i < h3.len) {
            int64_t *dp = (int64_t *) h3.base;
            for (int64_t x = lo_i; x <= hi_i; x++) dp[x] = v4_ink;
        } else {
            for (int64_t x = v9__range_start_1; x <= v10__range_end_1; x++)
                loft_vh_set_int(h3, 8, v8_hl_base + x, 0, v4_ink);
        }
    }
"""
f = f[:i] + new + f[j:]
open(sys.argv[2], "w").write(s[:a] + f + s[b:])
