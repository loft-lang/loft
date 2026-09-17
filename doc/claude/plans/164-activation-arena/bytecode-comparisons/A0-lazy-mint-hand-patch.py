#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The A0 ceiling, measured by hand: every function-entry buffer mint moved to its first use.

    A0-lazy-mint-hand-patch.py in.rs out.rs

`in.rs` is a `loft --native-release --native-emit` emission.  A prelude line
`var___ref_N = OpDatabase(cell,var___ref_N, T);` (before the function's first `// loft:`
comment) is removed, its `stores.null_named(…)` declaration becomes `DbRef::NULL`, and every
body line that hands `var___ref_N` to a call gets a null-guarded mint in front of it.  The
exits' `OpFreeRef` already ignores a null store, so no free changes.  § P0b has the numbers.
"""
import re
import sys

src = open(sys.argv[1]).read().split('\n')
out = []
fn = None
seen_loft = False
pending = {}   # var -> its mint expression
moved = 0
for line in src:
    m = re.match(r'fn (\w+)\(', line)
    if m:
        fn = m.group(1)
        seen_loft = False
        pending = {}
    if '// loft:' in line:
        seen_loft = True
    mm = re.match(r'\s*(var___ref_\d+) = (OpDatabase(?:NP)?\(cell,\s*var___ref_\d+, \d+_i32\));\s*$', line)
    if fn and not seen_loft and mm:
        pending[mm.group(1)] = mm.group(2)
        moved += 1
        continue
    if pending and seen_loft:
        body = line.lstrip()
        if not (body.startswith('if (') or body.startswith('OpFreeRef')):
            for v, mint in pending.items():
                if re.search(r'[,(]\s*' + v + r'\)', line):
                    indent = line[:len(line) - len(body)]
                    out.append(f'{indent}if {v}.store_nr == u16::MAX {{ {v} = {mint}; }}')
    out.append(line)

# a moved mint's declaration must hold the null sentinel, not an allocated empty slot
lazy = {}
fn = None
for line in out:
    m = re.match(r'fn (\w+)\(', line)
    if m:
        fn = m.group(1)
    for v in re.findall(r'if (var___ref_\d+)\.store_nr == u16::MAX \{ \1 = OpDatabase', line):
        lazy.setdefault(fn, set()).add(v)
final = []
fn = None
for line in out:
    m = re.match(r'fn (\w+)\(', line)
    if m:
        fn = m.group(1)
    mm = re.match(r'(\s*let mut (var___ref_\d+): DbRef = )stores\.null_named\("var___ref_\d+"\);\s*$', line)
    if mm and mm.group(2) in lazy.get(fn, ()):
        line = mm.group(1) + 'DbRef::NULL;'
    final.append(line)
open(sys.argv[2], 'w').write('\n'.join(final))
print(f'moved {moved} entry mints', file=sys.stderr)
