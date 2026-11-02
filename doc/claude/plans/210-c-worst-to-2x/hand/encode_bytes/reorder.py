# reorder.py IN OUT — loft_ref as {rec, pos, store_nr}: every positional loft_ref initialiser made
# designated first, then the typedef's field order changed.
import re, sys
src = open(sys.argv[1]).read()
pat = re.compile(r'(loft_ref\s+\w+(?:\[\d*\])?\s*=\s*|\(loft_ref\)\s*|return\s+\(loft_ref\)\s*|\},\s*\w+\s*=\s*)\{')
out = []; i = 0; n = 0
def split_top(s):
    parts, d, cur = [], 0, ''
    for ch in s:
        if ch in '([{': d += 1
        if ch in ')]}': d -= 1
        if ch == ',' and d == 0: parts.append(cur); cur = ''
        else: cur += ch
    parts.append(cur); return parts
for m in pat.finditer(src):
    pass
pos = 0
res = []
for m in pat.finditer(src):
    start = m.end() - 1
    if start < pos: continue
    d = 0; j = start
    while True:
        if src[j] == '{': d += 1
        elif src[j] == '}':
            d -= 1
            if d == 0: break
        j += 1
    body = src[start+1:j]
    parts = split_top(body)
    if len(parts) == 3 and '.' not in parts[0].strip()[:1] and '{' not in body:
        res.append(src[pos:start]); res.append('{ .store_nr = %s, .rec = %s, .pos = %s }' % tuple(p.strip() for p in parts)); n += 1
        pos = j + 1
res.append(src[pos:])
src = ''.join(res)
src = src.replace('{ { 0xFFFF, 0, 0 },', '{ { .store_nr = 0xFFFF, .rec = 0, .pos = 0 },')
old = 'typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; } loft_ref;'
assert old in src
LAYOUTS = {'reorder': 'typedef struct { uint32_t rec; uint32_t pos; uint16_t store_nr; } loft_ref;',
           'split': 'typedef struct { uint32_t rec; uint16_t store_nr; uint32_t pos; } loft_ref;'}
if len(sys.argv) > 3:
    src = src.replace(old, LAYOUTS[sys.argv[3]])
open(sys.argv[2], 'w').write(src)
print('designated', n, file=sys.stderr)
