# step3: the array arm's unused call buffer (v23___ref_8, a store minted per arm and freed) dropped:
# the window twin writes into the caller's buffer, so the buffer the plain call needed is dead.
import sys
s = open(sys.argv[1]).read()
a = """                                if (v23___ref_8.store_nr == 0xFFFF) {
                                    v23___ref_8 = loft_ref_null();
                                    loft_database_typed(&v23___ref_8, 4, 112);
                                }
                                t23 = loft_field(loft_vec_get(v18__mv_items_1, 16, v22_i_1), 0);
                                t24 = n_encode__ap"""
n = s.count(a); assert n >= 1, n
s = s.replace(a, """                                t23 = loft_field(loft_vec_get(v18__mv_items_1, 16, v22_i_1), 0);
                                t24 = n_encode__ap""")
open(sys.argv[2], 'w').write(s)
print('step3 sites', n, file=sys.stderr)
