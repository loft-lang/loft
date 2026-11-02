# step 2: `np += [p]` of a record type that owns no heap (Piece: three integers) copies its 24 bytes
# from the address the loop holds (loft_pa26_5) into the pushed slot — no loft_copy_record call.
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
s=edit(s,'n_insert_text__fr',[('            loft_copy_record(loft_types, 103, v26_p, v31__elm_6, 6);\n',
 '            memcpy(loft_rec_ptr(v31__elm_6), loft_pa26_5, 24);\n')])
open(sys.argv[2],'w').write(s)
