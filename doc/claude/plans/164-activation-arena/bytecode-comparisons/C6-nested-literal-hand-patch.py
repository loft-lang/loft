#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C6 ceiling, measured by hand: a nested record literal written straight into the
element field it is copied to.

    C6-nested-literal-hand-patch.py in.rs out.rs

`in.rs` is a `loft --native-release --native-emit` emission.  The shape replaced is

    let _pre_N = { //Object_M: ref(T)["__ref_p2_K"]
      var___ref_p2_K = OpDatabaseNP(cell,var___ref_p2_K, TP_i32);
      <field writes through var___ref_p2_K>
      var___ref_p2_K
      } /*Object_M: …*/;
    OpCopyRecord(cell,_pre_N, <the element field's place>, TP_i32);

and it becomes the field writes through the place itself.  Sound for this measurement
only where the literal's fields read nothing of the element (true of the drawing
library's `Paint { pk: Stroked, c1: 0, c2: 0, spec: [] }`), and where every field is
written (the literal names all four).
"""
import re
import sys

src = open(sys.argv[1]).read()
pat = re.compile(
    r'let (_pre_\d+) = \{ //Object_\d+: ref\(\w+\)\["(__ref_p2_\d+)"\]\n'
    r'(\s*)var_\2 = OpDatabaseNP\(cell,var_\2, (\d+)_i32\);\n'
    r'(?P<writes>(?:.*\n)*?)'
    r'\s*var_\2\n'
    r'\s*\} /\*Object_\d+: [^\n]*\*/;\n'
    r'(\s*)OpCopyRecord\(cell,\1, (?P<place>\{\{ let _v_v1 = \(var__elm_\d+\); DbRef \{[^\n]*?\} \}\}), \4_i32\);\n'
)
count = 0


def repl(m):
    global count
    count += 1
    buf = m.group(2)
    writes = m.group('writes').replace(f'(var_{buf})', f'(_c6_{buf})')
    indent = m.group(6)
    return (f'{{ let _c6_{buf}: DbRef = {m.group("place")};\n'
            f'{writes}{indent}}}\n')


out = pat.sub(repl, src)
open(sys.argv[2], 'w').write(out)
print(f'rewrote {count} nested literals', file=sys.stderr)
