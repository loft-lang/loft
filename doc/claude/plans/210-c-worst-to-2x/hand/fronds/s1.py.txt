# S1: pts/wid built in place in the fd_out element's fields (no scratch stores, no vec_add).
import sys
s = open(sys.argv[1]).read()
from fn import span
a, b = span(s, "n_fronds__fr")
f = s[a:b]
old_mint = """            loft_database_room(loft_types, loft_ntypes, &v95___vdb_4, 4, 175);
            v51_fd_pts = loft_field(v95___vdb_4, 0);
            loft_database_room(loft_types, loft_ntypes, &v96___vdb_5, 4, 105);
            v52_fd_wid = loft_field(v96___vdb_5, 0);
"""
new_mint = """            loft_vec_prealloc(v8_fd_out, 1, 8);
            t148 = loft_vec_append(v8_fd_out, 8);
            v74__elm_7 = t148;
            loft_set_int4(v74__elm_7, 0, INT64_C(0));
            loft_set_int4(v74__elm_7, 4, INT64_C(0));
            loft_vec_finish(v8_fd_out);
            v51_fd_pts = loft_field(v74__elm_7, 0);
            v52_fd_wid = loft_field(v74__elm_7, 4);
"""
assert f.count(old_mint) == 1; f = f.replace(old_mint, new_mint)
old_add = """            loft_vec_prealloc(v8_fd_out, 1, 8);
            t148 = loft_vec_append(v8_fd_out, 8);
            v74__elm_7 = t148;
            loft_set_int4(v74__elm_7, 0, INT64_C(0));
            loft_set_int4(v74__elm_7, 4, INT64_C(0));
            t149 = loft_field(v74__elm_7, 0);
            loft_vec_add(loft_types, 153, t149, v51_fd_pts);
            t150 = loft_field(v74__elm_7, 4);
            loft_vec_add(loft_types, 3, t150, v52_fd_wid);
            loft_vec_finish(v8_fd_out);
"""
assert f.count(old_add) == 1; f = f.replace(old_add, "")
open(sys.argv[2], "w").write(s[:a] + f + s[b:])
