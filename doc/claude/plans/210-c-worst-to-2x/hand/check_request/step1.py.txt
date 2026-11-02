# (R-PathRelease): an exit releases only the locals that can hold a store on some path to it.
import re, sys
from lib import *
src = open(sys.argv[1]).read()
ASSIGN = re.compile(r'(?:\b(v\d+_\w+) = (?!loft_ref_null\(\)|\{ 0xFFFF)|loft_database_typed\(&(v\d+_\w+)|loft_slice_take\([^;]*&(v\d+_\w+))')
FREE = re.compile(r'^\s*loft_free_var_distinct\(&(v\d+_\w+), [^;]*\);\n', re.M)
def prune(body):
    lines = body.split('\n')
    # arm starts: top-level `if (v7_major ==`
    arm_starts = [k for k, l in enumerate(lines) if l.startswith('    if (v7_major == ')]
    first = arm_starts[0]
    pro = set()
    for l in lines[:first]:
        for m in ASSIGN.finditer(l): pro |= {g for g in m.groups() if g}
    out = []; live = set(pro); removed = 0
    for k, l in enumerate(lines):
        if k in arm_starts: live = set(pro)
        m = re.match(r'^\s*loft_free_var_distinct\(&(v\d+_\w+), ', l)
        if m and k > first and m.group(1) not in live:
            removed += 1; continue
        for mm in ASSIGN.finditer(l): live |= {g for g in mm.groups() if g}
        out.append(l)
    sys.stderr.write(f'  removed {removed} releases\n')
    return '\n'.join(out)
for sig in ['static loft_ref n_read_value__fr(', 'static loft_tup0 n_read_value__d__fr(']:
    src = edit_fn(src, sig, prune)
open(sys.argv[2], 'w').write(src)
