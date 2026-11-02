# leave-one-out on s5: the Style records minted and freed per call again (step 2 / 3b keeps undone)
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
s=edit(s,'n_token_width__fr',[('    return w;','    loft_free_var(&keep_st);\n    return w;')])
s=edit(s,'n_place_line__fr',[('    keep_l1 = v77___lift_1;\n','    loft_free_var(&v77___lift_1);\n    keep_l1 = v77___lift_1;\n'),
 ('    keep_st = v45_st;\n','    loft_free_var(&v45_st);\n    keep_st = v45_st;\n')])
open(sys.argv[2],'w').write(s)
