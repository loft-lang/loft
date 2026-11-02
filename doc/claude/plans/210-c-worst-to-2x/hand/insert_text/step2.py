import sys
s=open(sys.argv[1]).read()
a=s.index('static loft_ref n_insert_text__fr(loft_ref v0_d, int64_t v1_a, loft_str v2_s, int64_t v3_style, loft_ref v4___retbuf) {')
b=s.index('\n}\n',a)
body=s[a:b]
def rep(old,new):
    global body
    assert body.count(old)==1, old
    body=body.replace(old,new)
ensure='''    if (v4___retbuf.store_nr == 0xFFFF) {
        t24 = (uint8_t) (0);
    } else {
        t24 = (uint8_t) (v4___retbuf.rec != 0);
    }
    if (t24 == 1) {
    } else {
        loft_database_typed(&v4___retbuf, 8, 104);
    }
'''
rep(ensure,'')
HOLD='''{ loft_store_t *hs_ = loft_store_of(v4___retbuf); uint32_t hr_ = loft_st_claim(hs_, 2); loft_set_u32(hs_, hr_, 8, 0); %s = (loft_ref){ v4___retbuf.store_nr, hr_, 8 }; }'''
rep('    loft_database_room(loft_types, loft_ntypes, &v33___vdb_1, 4, 107);\n    v6_nb = loft_field(v33___vdb_1, 0);',
    ensure+'    '+HOLD%'v6_nb'+'\n')
rep('    loft_database_room(loft_types, loft_ntypes, &v34___vdb_2, 4, 108);\n    v22_np = loft_field(v34___vdb_2, 0);',
    '    '+HOLD%'v22_np'+'\n')
MOVE='''{ loft_store_t *ms_ = loft_store_of(v4___retbuf); loft_ref f_ = loft_field(v4___retbuf, %d);
      uint32_t old_ = loft_u32(ms_, f_.rec, f_.pos), new_ = loft_u32(ms_, %s.rec, %s.pos);
      loft_set_u32(ms_, f_.rec, f_.pos, new_);
      if (old_ != 0 && old_ != 0xFFFFFFFFu && old_ != new_) loft_st_delete(ms_, old_);
      loft_st_delete(ms_, %s.rec); }'''
rep('''    t25 = loft_field(v4___retbuf, 0);
    loft_vec_replace(loft_types, loft_ntypes, 6, t25, v6_nb);
    t26 = loft_field(v4___retbuf, 4);
    loft_vec_replace(loft_types, loft_ntypes, 103, t26, v22_np);''',
    '    '+MOVE%(0,'v6_nb','v6_nb','v6_nb')+'\n    '+MOVE%(4,'v22_np','v22_np','v22_np'))
open(sys.argv[2],'w').write(s[:a]+body+s[b:])
