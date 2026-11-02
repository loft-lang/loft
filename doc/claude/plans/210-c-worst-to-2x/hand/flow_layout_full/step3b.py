# step 3b: steps 1+2 applied in place_line — measure reads the run text as a view; the two Style lifts kept across calls
import sys; sys.path.insert(0,'.'); from fn import edit
s=open(sys.argv[1]).read()
s=edit(s,'n_place_line__fr',[
 ('''                loft_text_set(&v64___work_p2_1, loft_lit("", 0));
                loft_text_concat(&v64___work_p2_1, loft_get_text(v43_r, 8));
                v46_rw = loft_call1(v8_measure, loft_view(&v64___work_p2_1), v45_st);''',
  '''                v46_rw = loft_call1(v8_measure, loft_get_text(v43_r, 8), v45_st);'''),
 ('    v77___lift_1 = loft_ref_null();\n','    static loft_ref keep_l1 = { 0xFFFF, 0, 0 }; static loft_ref keep_st = { 0xFFFF, 0, 0 };\n    v77___lift_1 = keep_l1; v45_st = keep_st;\n'),
 ('    loft_free_var(&v77___lift_1);\n','    keep_l1 = v77___lift_1;\n'),
 ('    loft_free_var(&v45_st);\n','    keep_st = v45_st;\n')])
open(sys.argv[2],'w').write(s)
