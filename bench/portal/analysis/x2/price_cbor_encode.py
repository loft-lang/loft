#!/usr/bin/env python3
"""X2 price on cbor `encode_map`'s emit loop (cbor.loft:135-143): vector headers held across
a loop that writes only `buf` — its own appends and those of the recursive `encode` it calls.
Both emissions of the function (plain and `__ap`) are edited; every edit asserts the line it
replaces.  Emit and build as bench/portal/hand_price.sh describes for a package.

    price_cbor_encode.py in.rs out.rs R     ranks, koff, klen — vector<integer>, another
                                            type than the callee's return buffer
    price_cbor_encode.py in.rs out.rs K     keybuf — vector<u8>, the return buffer's own type,
                                            another STORE (a local the callee is never handed)
    price_cbor_encode.py in.rs out.rs RK    both"""
import sys, re
if len(sys.argv) != 4 or sys.argv[1] in ("-h", "--help"):
    print(__doc__)
    sys.exit(0 if "--help" in sys.argv or "-h" in sys.argv else 2)
src, dst, mode = sys.argv[1:4]
L = open(src, encoding="utf-8").read().split('\n')
n_fn = 0
for name in ('fn n_encode_map(', 'fn n_encode_map__ap('):
    i = next(k for k, l in enumerate(L) if l.startswith(name))
    j = next(k for k in range(i + 1, len(L)) if L[k].startswith('fn '))
    a = next(k for k in range(i, j) if 'cbor.loft:135' in L[k])
    hdr = []
    for v in ('ranks', 'koff', 'klen', 'keybuf'):
        hdr.append(f'  let __xh_{v} = vector::vec_header(&(var_{v}), &stores.allocations); let __xb_{v}: *const u8 = vector::vec_base(&__xh_{v}, &stores.allocations);')
    done = 0
    k = a
    while k < j:
        l = L[k]
        m = re.match(r'^(\s*)let _pre_(\d+) = vector::get_vector\(&\(var_(ranks|koff|klen)\), \(8_i64\) as u32, \((var_i__2)\), &stores.allocations\);$', l)
        if m and 'R' in mode:
            ind, v = m.group(1), m.group(3)
            nxt = L[k + 1]
            mm = re.match(r'^\s*let mut (var___ncc_\d+): i64 = \{\{let db = \(_pre_\d+\); .*\}\};$', nxt)
            assert mm, nxt[:160]
            L[k] = ''
            L[k + 1] = f'{ind}let mut {mm.group(1)}: i64 = unsafe {{ vector::get_elem_at::<i64, false>(&__xh_{v}, __xb_{v}, &(var_{v}), (8_i64) as u32, var_i__2, (0_i64) as u32, i64::MIN, &stores.allocations) }};'
            done += 1
        m = re.match(r'^(\s*)let _pre_(\d+) = vector::get_vector\(&\(var_keybuf\), \(1_i64\) as u32, \((ops::op_add_long_nn\(\(var_oi\), \(var_t\)\))\), &stores.allocations\);$', l)
        if m and k > a and 'K' in mode:
            ind = m.group(1)
            nxt = L[k + 1]
            mm = re.match(r'^\s*let mut (var___dn4_\d+): i64 = \{\{let db = \(_pre_\d+\); .*get_byte.*\}\};$', nxt)
            assert mm, nxt[:160]
            L[k] = ''
            L[k + 1] = (f'{ind}let mut {mm.group(1)}: i64 = {{ let __bi: i64 = {m.group(3)}; let __bl = i64::from(__xh_keybuf.len); '
                        f'let __bf = if __bi < 0 {{ __bi + __bl }} else {{ __bi }}; if __bf >= 0 && __bf < __bl {{ i64::from(unsafe {{ __xb_keybuf.add(__bf as usize).read() }}) }} else {{ i64::MIN }} }};')
            done += 10
        k += 1
    assert done == (3 if 'R' in mode else 0) + (10 if 'K' in mode else 0), (name, done)
    L[a] = L[a] + '\n' + '\n'.join(hdr)
    n_fn += 1
assert n_fn == 2
open(dst, 'w', encoding="utf-8").write('\n'.join(L))
