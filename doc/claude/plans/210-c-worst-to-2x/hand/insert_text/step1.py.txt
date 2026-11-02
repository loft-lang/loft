import sys
s=open(sys.argv[1]).read()
a=s.index('static loft_ref n_insert_text__fr(loft_ref v0_d, int64_t v1_a, loft_str v2_s, int64_t v3_style, loft_ref v4___retbuf) {')
b=s.index('\n}\n',a)
body=s[a:b]
n0=body.count('loft_copy_record(loft_types, 103, ')
body=body.replace('loft_copy_record(loft_types, 103, v26_p, v31__elm_6, 6);','memcpy(loft_rec_ptr(v31__elm_6), loft_pa26_5, 24);')
body=body.replace('loft_copy_record(loft_types, 103, v18_newp, v29__elm_4, 6);','memcpy(loft_rec_ptr(v29__elm_4), loft_rec_ptr(v18_newp), 24);')
body=body.replace('loft_copy_record(loft_types, 103, v18_newp, v32__elm_7, 6);','memcpy(loft_rec_ptr(v32__elm_7), loft_rec_ptr(v18_newp), 24);')
assert body.count('loft_copy_record(loft_types, 103, ')==0, n0
open(sys.argv[2],'w').write(s[:a]+body+s[b:])
