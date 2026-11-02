# step 1: Del / Sty take the handed-over document: d is dead in the caller (cur = apply_op(cur, op)),
# so the result is d's own record with its pieces replaced — the 80 KB character buffer is not copied
# (the twin moves d.buf).  apply_op answers it as is; invert's rebind sees its own record.
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
s=edit(s,'n_delete_range__fr',[('''    if (v3___retbuf.store_nr == 0xFFFF) {
        t15 = (uint8_t) (0);
    } else {
        t15 = (uint8_t) (v3___retbuf.rec != 0);
    }
    if (t15 == 1) {
    } else {
        loft_database_typed(&v3___retbuf, 8, 104);
    }
    t16 = loft_field(v3___retbuf, 0);
    t17 = loft_field(v0_d, 0);
    loft_vec_replace(loft_types, loft_ntypes, 6, t16, t17);
    t18 = loft_field(v3___retbuf, 4);
''','''    v3___retbuf = v0_d;
    t18 = loft_field(v3___retbuf, 4);
''')])
s=edit(s,'n_set_style__fr',[('''    if (v4___retbuf.store_nr == 0xFFFF) {
        t21 = (uint8_t) (0);
    } else {
        t21 = (uint8_t) (v4___retbuf.rec != 0);
    }
    if (t21 == 1) {
    } else {
        loft_database_typed(&v4___retbuf, 8, 104);
    }
    t22 = loft_field(v4___retbuf, 0);
    t23 = loft_field(v0_d, 0);
    loft_vec_replace(loft_types, loft_ntypes, 6, t22, t23);
    t24 = loft_field(v4___retbuf, 4);
''','''    v4___retbuf = v0_d;
    t24 = loft_field(v4___retbuf, 4);
''')])
s=edit(s,'n_apply_op__fr',[('''        if (t10.store_nr != 0xFFFF && t10.store_nr == v0_d.store_nr) {
            v15___ret_tail_2 = loft_ref_null();
            loft_bind_copy(loft_types, 104, &v15___ret_tail_2, t10);
        } else { v15___ret_tail_2 = t10; }''','        v15___ret_tail_2 = t10;'),
 ('''        if (t15.store_nr != 0xFFFF && t15.store_nr == v0_d.store_nr) {
            v16___ret_tail_3 = loft_ref_null();
            loft_bind_copy(loft_types, 104, &v16___ret_tail_3, t15);
        } else { v16___ret_tail_3 = t15; }''','        v16___ret_tail_3 = t15;')])
s=edit(s,'n_invert__fr',[('        if (!(t8.store_nr != v3_cur.store_nr','        if (t8.store_nr == v3_cur.store_nr && t8.rec == v3_cur.rec && t8.pos == v3_cur.pos) {} else\n        if (!(t8.store_nr != v3_cur.store_nr')])
open(sys.argv[2],'w').write(s)
