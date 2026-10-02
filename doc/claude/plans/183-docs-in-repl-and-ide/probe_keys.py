#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN183 § 1 — every feature entry names its keys, and no key has two owners.

Reads `keys.tsv` beside this script and `index/features.json`.  Exit 1 when a key is listed by
two entries, when a feature entry outside the maintainers' group has no keys, or when an entry
named here is not in the catalogue — and when the table and the issues' own `keys` disagree.
"""
import json, os, sys
from collections import defaultdict
here = os.path.dirname(os.path.abspath(__file__))
root = os.path.abspath(os.path.join(here, '..', '..', '..', '..'))
cat = json.load(open(os.path.join(root, 'index', 'features.json')))
features = {'F' + str(e['number']) for e in cat if e['kind'] == 'feature'}
owners, listed = defaultdict(list), set()
for line in open(os.path.join(here, 'keys.tsv')):
    if line.startswith('#') or not line.strip():
        continue
    entry, keys = line.rstrip('\n').split('\t')
    listed.add(entry)
    for k in keys.split():
        owners[k].append(entry)
bad = [f'{k}: owned by {", ".join(o)}' for k, o in sorted(owners.items()) if len(o) > 1]
inside = {'F' + str(e['number']) for e in cat if e['kind'] == 'feature' and e.get('group') == 'inside'}
bad += [f'{e}: no keys' for e in sorted(features - listed - inside, key=lambda x: int(x[1:]))]
bad += [f'{e}: not in the catalogue' for e in sorted(listed) if not e.startswith('NEW-') and e not in features]
carried = {'F' + str(e['number']): ' '.join(e.get('keys', [])) for e in cat if e['kind'] == 'feature'}
table = {}
for line in open(os.path.join(here, 'keys.tsv')):
    if not line.startswith('#') and line.strip():
        entry, keys = line.rstrip('\n').split('\t')
        table[entry] = keys
bad += [f'{e}: the issue carries "{carried.get(e, "")}", the table "{k}"' for e, k in table.items() if carried.get(e, '') != k]
print(f'{len(owners)} keys over {len(listed)} entries')
for b in bad:
    print('  ' + b)
sys.exit(1 if bad else 0)
