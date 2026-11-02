# step 2: token_width keeps the resolver's Style record across calls (no mint+free per token)
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
s=edit(s,'n_token_width__fr',[('    v17___lift_1 = loft_ref_null();\n','    static loft_ref keep_st = { 0xFFFF, 0, 0 };\n    v17___lift_1 = keep_st;\n'),
 ('    loft_free_var(&v17___lift_1);\n','    keep_st = v17___lift_1;\n')])
open(sys.argv[2],'w').write(s)
