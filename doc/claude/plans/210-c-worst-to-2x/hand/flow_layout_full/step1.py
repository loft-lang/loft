# step 1: token_width hands the run's stored text to measure as a view (no work-text copy, no lift copy)
import sys
s=open(sys.argv[1]).read()
old='''        loft_text_set(&v13___work_p2_1, loft_lit("", 0));
        loft_text_concat(&v13___work_p2_1, loft_get_text(v10_r, 8));
        loft_text_set(&v18___lift_2, loft_view(&v13___work_p2_1));
'''
assert s.count(old)==1, s.count(old)
s=s.replace(old,'')
old2='t10 = loft_call1(v3_measure, loft_view(&v18___lift_2), v17___lift_1);'
assert s.count(old2)==1
s=s.replace(old2,'t10 = loft_call1(v3_measure, loft_get_text(v10_r, 8), v17___lift_1);')
open(sys.argv[2],'w').write(s)
