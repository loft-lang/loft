#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN183 P0 probe 2 — each construct, resolved to its catalogue entry by hand.

For each row of `constructs.tsv`: the entry named by hand must exist and its body must show
the spelling (so the answer is read from the catalogue, not invented) — or the row records
the gap (`body-gap`: the entry is the right one and does not show it; `no-entry`: nothing
covers the construct).  Each line also reports
how many entries a plain text search of the bodies' code would offer — the ambiguity the
parse, not the text, has to resolve.  Exit 1 when a row's outcome is not the one recorded: a gap opened, or one closed.
"""
import json, os, re, sys
here = os.path.dirname(os.path.abspath(__file__))
root = os.path.abspath(os.path.join(here, '..', '..', '..', '..'))
cat = json.load(open(os.path.join(root, 'index', 'features.json')))
body = {('F' if e['kind'] == 'feature' else 'I') + str(e['number']): e['body'] for e in cat}

def code_of(text):
    """The code a body shows: its fenced blocks and its inline code spans."""
    blocks = re.findall(r'```[a-z]*\n(.*?)```', text, re.S)
    spans = re.findall(r'`([^`\n]+)`', re.sub(r'```.*?```', '', text, flags=re.S))
    return '\n'.join(blocks + spans)

bad, rows = [], 0
for line in open(os.path.join(here, 'constructs.tsv')):
    if line.startswith('#') or not line.strip():
        continue
    construct, spelling, entry, expected, _how = line.rstrip('\n').split('\t')
    rows += 1
    offered = sorted((k for k, b in body.items() if spelling in code_of(b)),
                     key=lambda k: (k[0], int(k[1:])))
    if entry == '—':
        verdict = 'no-entry'
    elif entry not in body:
        verdict = 'missing'
    elif spelling not in code_of(body[entry]):
        verdict = 'body-gap'
    else:
        verdict = 'ok'
    if verdict != expected:
        bad.append(f'{construct}: {verdict}, recorded as {expected} — update the row '
                   f'(a gap closed, or the catalogue moved)')
    print(f'{construct:32} {entry:>4}  {verdict:9} text search offers {len(offered):3}')
print(f'{rows} constructs')
for b in bad:
    print('  ' + b)
sys.exit(1 if bad else 0)
