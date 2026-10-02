#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN183 P0 probe 1 — is every catalogue entry in exactly one overview group?

Reads `index/features.json` (the generated mirror of loft-lang/features) and `grouping.tsv`
beside this script.  Exit 1 and name the entry when one is in no group, in two, or when the
grouping names an entry the catalogue does not have.
"""
import json, os, sys
from collections import defaultdict
here = os.path.dirname(os.path.abspath(__file__))
root = os.path.abspath(os.path.join(here, '..', '..', '..', '..'))
cat = json.load(open(os.path.join(root, 'index', 'features.json')))
ids = {('F' if e['kind'] == 'feature' else 'I') + str(e['number']) for e in cat}
placed = defaultdict(list)
titles = {}
for line in open(os.path.join(here, 'grouping.tsv')):
    if line.startswith('#') or not line.strip():
        continue
    key, title, entry = line.rstrip('\n').split('\t')
    placed[entry].append(key)
    titles[key] = title
bad = []
bad += [f'{e}: in no group' for e in sorted(ids - placed.keys())]
bad += [f'{e}: in {len(g)} groups ({", ".join(g)})' for e, g in sorted(placed.items()) if len(g) > 1]
bad += [f'{e}: not in the catalogue' for e in sorted(placed.keys() - ids)]
carried = {('F' if e['kind'] == 'feature' else 'I') + str(e['number']): e.get('group', '') for e in cat}
bad += [f'{e}: the issue carries group "{carried.get(e, "")}", the table "{g[0]}"'
        for e, g in sorted(placed.items()) if carried.get(e, '') != g[0]]
counts = defaultdict(int)
for e, g in placed.items():
    counts[g[0]] += 1
for key, title in titles.items():
    print(f'{counts[key]:4}  {title}')
print(f'{len(ids):4}  entries in the catalogue, {len(placed)} placed')
for b in bad:
    print('  ' + b)
sys.exit(1 if bad else 0)
