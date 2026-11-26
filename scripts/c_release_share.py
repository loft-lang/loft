#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN229 D — how much of a program's C is releases, and where: per C function, its bytes, its
release statements (loft_free_var, its distinct form, loft_wpark_put, loft_text_free) and the
runs of eight or more a release table could take, split by whether the function is an
(R-CSplit) part (its locals in loft_fr) or not.  Reads written C units:

    scripts/c_release_share.py UNIT.c ...

(a split build kept with LOFT_KEEP_NATIVE_C=1 LOFT_C_UNITS=1, or one --native-emit file)."""
import re, sys, collections
rel = re.compile(r'^\s*(loft_free_var_distinct|loft_free_var|loft_wpark_put|loft_text_free)\((loft_fr->cx|cx), ')
head = re.compile(r'^[A-Za-z].*\) \{$')
tot = collections.Counter(); forms = collections.Counter()
def flush(name, body):
    if name is None: return
    kind = 'part' if '__part' in name else ('framed-main' if 'loft_fr' in ''.join(body[:40]) else 'plain')
    b = sum(len(l) + 1 for l in body)
    tot[(kind, 'bytes')] += b; tot['all-bytes'] += b
    run = []
    for l in body + ['']:
        m = rel.match(l)
        if m:
            run.append(l)
        else:
            if run:
                rb = sum(len(x) + 1 for x in run)
                tot[(kind, 'release')] += rb
                if len(run) >= 8:
                    tot[(kind, 'run8')] += rb
                    for x in run:
                        forms[(kind, rel.match(x).group(1), rel.match(x).group(2))] += 1
            run = []
if sys.argv[1:2] in (['--help'], ['-h']):
    print(__doc__)
    sys.exit(0)
for f in sys.argv[1:]:
    name, body = None, []
    for l in open(f, encoding='utf-8', errors='replace'):
        l = l.rstrip('\n')
        if head.match(l):
            flush(name, body); name, body = l, []
        elif name is not None:
            body.append(l)
            if l == '}':
                flush(name, body); name, body = None, []
    flush(name, body)
A = tot['all-bytes']
print(f"C in functions: {A/1048576:.1f} MB")
for k in ('part', 'framed-main', 'plain'):
    b, r, r8 = tot[(k,'bytes')], tot[(k,'release')], tot[(k,'run8')]
    print(f"{k:12} {b/1048576:6.1f} MB  release {r/1048576:6.1f} MB ({100*r/max(b,1):3.0f}%)  in runs>=8 {r8/1048576:6.1f} MB = {100*r8/A:3.0f}% of all C")
print("release forms in runs>=8 (kind, form, context): count")
for (k, fm, ctx), n in sorted(forms.items(), key=lambda x: -x[1])[:10]:
    print(f"  {k:12} {fm:24} {ctx:12} {n}")
