#!/usr/bin/env python3
"""@PLN182 P0 — the name census: which candidate operator-method names are already METHODS
(first parameter `self`) in loft's stdlib or in any library's public API, and with what
signature.  Sources: default/*.loft (the stdlib) and doc/claude/LIBRARIES.md (`make
libcatalogue`: every library, published and origin/main)."""
import re, glob, collections, sys
CANDIDATES = """compare Ordering then cmp order equals eq same less less_than lt before after greater
plus minus times divided_by over remainder modulo mod rem negate negated neg opposite
add sub subtract multiply mul divide div scale scaled
at get set element set_element item lookup find slice range key_range span between
index put insert next to_text set_at power bit_and bit_or bit_xor bit_not shift_left shift_right""".split()
rx = re.compile(r'pub fn ([a-z_][a-z0-9_]*)\s*(?:<[^>]*>)?\s*\(\s*(self|both)\s*:\s*([^,)]+)([^)]*)\)\s*(->\s*[^{#]*)?')
hits = collections.defaultdict(list)
def scan(text, origin):
    lib = origin
    for line in text.splitlines():
        m = re.match(r'- \*\*([a-z0-9_-]+)\*\*', line.strip())
        if m and origin == 'catalogue': lib = m.group(1)
        mm = rx.search(line)
        if mm and mm.group(1) in CANDIDATES:
            sig = f"{mm.group(1)}(self: {mm.group(3).strip()}{mm.group(4)}) {(mm.group(5) or '').strip()}"
            hits[mm.group(1)].append((lib if origin == 'catalogue' else origin, re.sub(r'\s+', ' ', sig)))
for f in sorted(glob.glob('default/*.loft')):
    scan(open(f).read(), 'stdlib:' + f.split('/')[-1])
scan(open('doc/claude/LIBRARIES.md').read(), 'catalogue')
for name in CANDIDATES:
    uses = sorted(set(hits.get(name, [])))
    print(f"{name:12} {len(uses):3}  " + ('; '.join(f"{o}: {s}" for o, s in uses[:6]) + (' …' if len(uses) > 6 else '') if uses else 'free'))
