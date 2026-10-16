#!/usr/bin/env python3
"""The four hand prices on zttext `invert` (over-9x.md § zttext `invert`), applied to the
emitted Rust of the zttext bench.  Each lever is an edit to one function; every edit asserts
the text it replaces, so a stale emission fails loudly instead of pricing nothing.

    L1  `cur = apply_op(cur, op)` hands `cur` as apply_op's buffer, apply_op hands it on to
        its chain callee, and the chain's delivery takes that result as the buffer it is
    L2  the prepend `tmp += g; tmp += result; result = tmp` done in result's own record
    L3  `np += [p]` in insert_text as 24 bytes between the two record addresses (BUILT as
        `(R-FlatCopy)`'s emitted clause: price it on an emission from before that build)
    L4  slice_runs holds `d.pieces`' header across its loop

    cd <loft-bench-libs>/loft-libs-core/zttext
    LOFT_NATIVE_NO_CACHE=1 LOFT_NATIVE_RUSTC_ARGS=/tmp/b.args loft --native-release bench/bench.loft --n 2
    loft --native-release --native-emit /tmp/z.rs bench/bench.loft
    price_invert.py /tmp/z.rs /tmp/z_l1234.rs L1 L2 L3 L4
    bench/portal/hand_price.sh /tmp/z.rs /tmp/z /tmp/b.args
    bench/portal/hand_price.sh /tmp/z_l1234.rs /tmp/z_l1234 /tmp/b.args
    taskset -c 2 /tmp/z --n 20 | grep ^invert; taskset -c 2 /tmp/z_l1234 --n 20 | grep ^invert
"""
import re
import sys


def function(s, name):
    """The span of `fn name(` up to the next top-level `fn`."""
    a = s.index(f"fn {name}(")
    b = s.index("\nfn ", a + 10)
    return a, b


def replace_in(s, name, old, new, count=1):
    a, b = function(s, name)
    body = s[a:b]
    found = body.count(old)
    if found != count:
        sys.exit(f"{name}: expected {count} of {old!r}, found {found}")
    return s[:a] + body.replace(old, new) + s[b:]


def l1(s):
    s = replace_in(
        s,
        "n_invert",
        "let _src = n_apply_op(cell, var_cur, var_op, var___ref_3);",
        "let _src = n_apply_op(cell, var_cur, var_op, var_cur);",
    )
    a, b = function(s, "n_apply_op")
    body = s[a:b]
    for n, callee in [(1, "n_insert_text"), (2, "n_delete_range"), (3, "n_set_style")]:
        pat = re.compile(r"(let _src = " + callee + r"\(cell, var_d, [^;]*?), var___ref_" + str(n) + r"\);")
        body, k = pat.subn(r"\1, var___retbuf);", body)
        if k != 1:
            sys.exit(f"n_apply_op: the call to {callee} was not found")
    guard = "if _src.store_nr == u16::MAX || _src.store_nr == _dst.store_nr || _src.store_nr != var_d.store_nr {"
    if body.count(guard) != 3:
        sys.exit("n_apply_op: the chain deliveries were not found")
    body = body.replace(guard, "if true {")
    return s[:a] + body + s[b:]


def l2(s):
    a, _ = function(s, "n_invert")
    st = s.index("        var___vdb_2 = OpDatabaseNP(cell,var___vdb_2, 116_i32);", a)
    end_marker = "          var_tmp = DbRef::NULL\n;"
    en = s.index(end_marker, st) + len(end_marker)
    prepend = """        { // L2: g's elements inserted at the front of result, in result's own record
          let __gl = vector::length_vector(&var_g, &stores.allocations) as i64;
          for __k in 0..__gl {
            let __src = vector::get_vector(&var_g, 32_u32, __k, &stores.allocations);
            let __slot = vector::insert_vector(&var_result, 32_u32, __k, &mut stores.allocations);
            stores.copy_block(&__src, &__slot, 32);
            stores.copy_claims(&__src, &__slot, 101_u16);
          }
        }
"""
    return s[:st] + prepend + s[en:]


def l3(s):
    return replace_in(
        s,
        "n_insert_text",
        "OpCopyRecord(cell,var_p, var__elm_6, 16486_i32);",
        "{ let __d = unsafe { __pw_5.base.add(__pw_5.len as usize * 24) } as *mut u8; "
        "if !__pa_6.is_null() && !__pw_5.locked { unsafe { std::ptr::copy_nonoverlapping(__pa_6, __d, 24); } } "
        "else { OpCopyRecord(cell,var_p, var__elm_6, 16486_i32); } }",
    )


def l4(s):
    path = "{{ let _v_v1 = (var_d); DbRef {store_nr: _v_v1.store_nr, rec: _v_v1.rec, pos: _v_v1.pos + (4_i64) as u32} }}"
    s = replace_in(
        s,
        "n_slice_runs",
        f"var___ncc_1 = {{vector::get_vector(&({path}), (24_i64) as u32, (var_i), &stores.allocations)}};",
        "var___ncc_1 = {vector::get_vector_hoisted::<false>(&__hp_vh, &__hp_vd, (24_i64) as u32, (var_i), &stores.allocations)};",
    )
    loop = "    'l4: loop { //For loop_4"
    return replace_in(
        s,
        "n_slice_runs",
        loop,
        f"    let __hp_vd: DbRef = {path}; let __hp_vh = vector::vec_header(&__hp_vd, &stores.allocations); // L4\n" + loop,
    )


LEVERS = {"L1": l1, "L2": l2, "L3": l3, "L4": l4}

if len(sys.argv) < 4 or sys.argv[1] in ("-h", "--help") or not set(sys.argv[3:]) <= set(LEVERS):
    print(__doc__)
    sys.exit(0 if sys.argv[1:2] in (["-h"], ["--help"]) else 2)
text = open(sys.argv[1], encoding="utf-8").read()
for lever in sys.argv[3:]:
    text = LEVERS[lever](text)
open(sys.argv[2], "w", encoding="utf-8").write(text)
