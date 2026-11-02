# S2: `for f in fronds(..) { fd_out += [f] }` → an append twin n_fronds__ap that appends into the
# caller's fd_out (its reads of fd_out offset by the entry length); no sub-result, no copy_record.
import sys
from fn import span
s = open(sys.argv[1]).read()
a, b = span(s, "n_fronds__fr"); f = s[a:b]
ap = f.replace("n_fronds__fr(", "n_fronds__ap(", 1)
def rep(t, o, n, c=1):
    assert t.count(o) == c, (o, t.count(o)); return t.replace(o, n)
# prologue: keep the caller's buffer, remember where it started
ap = rep(ap, """    loft_vec_clear_release(loft_types, loft_ntypes, v7___retbuf);
    if (v7___retbuf.store_nr == 0xFFFF || v7___retbuf.rec == 0) { loft_database_room(loft_types, loft_ntypes, &v7___retbuf, 4, 184); } else { loft_vec_clear_release(loft_types, loft_ntypes, v7___retbuf); }
    v8_fd_out = v7___retbuf;
""", """    v8_fd_out = v7___retbuf;
    int64_t ap_base0 = loft_vec_len(v8_fd_out);
""")
ap = rep(ap, "        loft_vec_replace(loft_types, loft_ntypes, 183, v7___retbuf, v8_fd_out);\n", "")
ap = rep(ap, "    v75_fd_base = t_6vector_len(v8_fd_out);\n", "    v75_fd_base = t_6vector_len(v8_fd_out) - ap_base0;\n")
ap = rep(ap, "        v81___ncc_16 = loft_vec_get(v8_fd_out, 8, v79_k);\n", "        v81___ncc_16 = loft_vec_get(v8_fd_out, 8, ap_base0 + v79_k);\n")
# the recursive call in both functions: append into fd_out directly
i = ap.index("        v88__vector_2 = n_fronds("); j = ap.index("        brk_4:;\n", i) + len("        brk_4:;\n")
call = "        n_fronds__ap(t198, t199, t200, t201, v86_fd_sub, v5_pw, v6_ph, v8_fd_out);\n"
ap = ap[:i] + call + ap[j:]
i = f.index("        v88__vector_2 = n_fronds("); j = f.index("        brk_4:;\n", i) + len("        brk_4:;\n")
f = f[:i] + call + f[j:]
proto = "static loft_ref n_fronds__ap(double v0_x1, double v1_y1, double v2_x2, double v3_y2, loft_ref v4_sp, int64_t v5_pw, int64_t v6_ph, loft_ref v7___retbuf);\n"
s = s[:a] + proto + ap + f + s[b:]
open(sys.argv[2], "w").write(s)
