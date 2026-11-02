# step 4: the result vector built IN its destination — b64_decode's `out` is its caller's
# return buffer (cleared, then filled through the window), and base64_to_bytes hands its own
# return buffer down as b64_decode's; neither copies the finished vector (`loft_vec_add`).
import sys
src, dst = sys.argv[1], sys.argv[2]
c = open(src).read()
def fn(sig):
    s = c.index(sig); e = c.index("\n}\n", s); return s, e
s, e = fn("static loft_ref n_b64_decode__fr(loft_str v0_b64, loft_ref v1___retbuf, loft_ref v2_vals) {")
f = c[s:e]
old = '''    loft_database_room(loft_types, loft_ntypes, &v29___vdb_2, 4, 112);
    v11_out = loft_field(v29___vdb_2, 0);'''
assert f.count(old) == 1
f = f.replace(old, '''    loft_vec_clear_release(loft_types, loft_ntypes, v1___retbuf);
    v11_out = v1___retbuf;''')
old = '''        loft_vec_clear_release(loft_types, loft_ntypes, v1___retbuf);
        loft_vec_add(loft_types, 30, v1___retbuf, v11_out);
        loft_free_var(&v29___vdb_2);
        v39___ret_tail_4 = v1___retbuf;'''
assert f.count(old) == 1
f = f.replace(old, '''        v39___ret_tail_4 = v1___retbuf;''')
c = c[:s] + f + c[e:]
s, e = fn("static loft_ref n_base64_to_bytes__fr(loft_str v0_b64, loft_ref v1___retbuf, loft_ref v7___ref_2) {")
f = c[s:e]
old = '''    if (v3___ref_1.store_nr == 0xFFFF) {
        v3___ref_1 = loft_ref_null();
        loft_database_typed(&v3___ref_1, 4, 112);
    }
    loft_frame_site(405);
    v2_r = n_b64_decode(v0_b64, v3___ref_1, v7___ref_2);'''
assert f.count(old) == 1
f = f.replace(old, '''    loft_frame_site(405);
    v2_r = n_b64_decode(v0_b64, v1___retbuf, v7___ref_2);''')
old = '''        loft_vec_clear_release(loft_types, loft_ntypes, v1___retbuf);
        loft_vec_add(loft_types, 30, v1___retbuf, v2_r);
        loft_free_var(&v3___ref_1);
        loft_free_var(&v4__vec_1);'''
assert f.count(old) == 1
f = f.replace(old, '''        loft_free_var(&v4__vec_1);''')
c = c[:s] + f + c[e:]
open(dst, "w").write(c)
