#!/usr/bin/env python3
"""(R-DecodeView) on check_request, priced on the emitted Rust: the decoded texts and byte
strings are held as SPANS of the frame — (offset, length) — instead of owned copies.

The region is emitted as twins (the rule's own shape): `n_decode_dv` → `n_read_value__rg_dv` →
`n_read_value__d_dv` write a span where the owned form claims a text or a vector, and
`n_pa_text_dv` → `n_pa_get__vr_dv` take the frame and read a span's bytes in place.  Only
`check_request` calls the twins, so every other row keeps the owned form and its hash.

The span is written as the CInt variant (tag 3, the i64 at +8 = offset << 32 | length): the
twin layout's cost, with no type in the schema for it — the release walk sees an integer and
frees nothing, which is the rule's "a span field frees no bytes".  A text span is admitted
only when its bytes are valid UTF-8, (R-DecodeView)'s condition (5); otherwise the owned form
is built, as the rule says.

    price_decode_view.py <emitted.rs> <out.rs>
"""
import re
import sys

if len(sys.argv) != 3 or sys.argv[1] in ("-h", "--help"):
    print(__doc__)
    sys.exit(0 if "--help" in sys.argv or "-h" in sys.argv else 2)
src = open(sys.argv[1], encoding="utf-8").read()


def body(name):
    """The whole text of `fn <name>(` up to its closing `} /*block_1: …*/` line."""
    start = src.index(f"fn {name}(")
    start = src.rindex("\n", 0, start) + 1
    nxt = re.compile(r"\n(// loft:[^\n]*\n)?(#\[inline\]\n)?fn |\nconst |\nstatic ")
    return src[start:nxt.search(src, start + 1).start() + 1]


def once(text, old, new):
    assert text.count(old) == 1, (old[:80], text.count(old))
    return text.replace(old, new)


# ── the decoder twin ─────────────────────────────────────────────────────────────────
d = body("n_read_value__d")
d = d.replace("fn n_read_value__d(", "fn n_read_value__d_dv(", 1)
d = d.replace("n_read_value__d(cell,", "n_read_value__d_dv(cell,")
lines = d.split("\n")


def line_of(marker, after=0):
    for i in range(after, len(lines)):
        if marker in lines[i]:
            return i
    raise SystemExit(f"no line with {marker!r}")


SPAN = ("{ let db = var___retbuf; let st = stores.store_mut(&db); "
        "st.set_int(db.rec, db.pos + 8, (var_argpos << 32) | var_arg); "
        "st.set_byte(db.rec, db.pos, 0, 3); }")

# major 2 — the byte string: no holder record, no slice, no move.  Drop line 253's claim
# and slice; the Decoded write keeps its shape with a span where the vector was moved in.
import os
PART = os.environ.get("DV_PART", "all")   # all | bytes | text: which half of the form to price
if PART in ("all", "bytes"):
    a = line_of("cbor.loft:253")
    b = line_of("cbor.loft:255", a)
    del lines[a + 1:b]
    mv = line_of("stores.move_vector(", a)
    assert "4_u8" in lines[mv + 1]
    lines[mv] = "      " + SPAN
    del lines[mv + 1]

# major 3 — the text: a span when the bytes are UTF-8, the owned text otherwise.
t = line_of("synth text dest_", line_of("cbor.loft:261"))
assert "set_str" in lines[t + 4] and "5_u8" in lines[t + 5]
owned = "\n".join(lines[t:t + 6])
lines[t:t + 6] = [owned] if PART == "bytes" else [
    "      { let __vb = stores.allocations[__vh_1.store_nr as usize].elem_base(__vh_1.rec);"
    " let __s = unsafe { std::slice::from_raw_parts(__vb.add(var_argpos as usize), var_arg as usize) };"
    " if std::str::from_utf8(__s).is_ok() " + SPAN + " else {",
    owned,
    "      } }",
]
d = "\n".join(lines)

rg = body("n_read_value__rg")
rg = rg.replace("fn n_read_value__rg(", "fn n_read_value__rg_dv(", 1)
rg = rg.replace("n_read_value__d(cell,", "n_read_value__d_dv(cell,")

dec = body("n_decode")
dec = dec.replace("fn n_decode(", "fn n_decode_dv(", 1)
dec = once(dec, "n_read_value__rg(cell,", "n_read_value__rg_dv(cell,")

# ── the readers: the frame is a parameter, a span's bytes are read where they lie ──────
FRAME = ("let __fh = vector::vec_header(&(var___frame), &stores.allocations); "
         "let __fb = vector::vec_base(&__fh, &stores.allocations);")


def span_bytes(holder):
    return ("{ let __sp = { let db = " + holder + "; stores.store(&db).get_int(db.rec, db.pos + 8) }; "
            "unsafe { std::slice::from_raw_parts(__fb.add((__sp >> 32) as usize), (__sp & 0xffff_ffff) as usize) } }")


g = body("n_pa_get__vr")
g = g.replace("fn n_pa_get__vr(cell: &std::cell::UnsafeCell<Stores>, mut var_m: DbRef,",
              "fn n_pa_get__vr_dv(cell: &std::cell::UnsafeCell<Stores>, var___frame: DbRef, mut var_m: DbRef,", 1)
g = once(g, "  let stores: &mut Stores = unsafe { &mut *cell.get() };\n",
         "  let stores: &mut Stores = unsafe { &mut *cell.get() };\n  " + FRAME + "\n")
m = re.search(r"\n( *)if \(\(\(\(\{\{ let _v_v1 = \(\(_pre_24\) as u8\); if _v_v1 == 255 \{ i64::MIN \} else \{ i64::from\(_v_v1\) \} \}\}\) as i64\) == \(\(5_i64\) as i64\)\) as u8\) == 1 \{ //match_arm_10: void\n.*?\n.*?\n.*?if \(\(ops::op_eq_text\(\(&var__mv_value_1\), \(var_key\)\)\) as u8\) == 1 \{ //block_12: never\n", g)
assert m, "pa_get arm"
ind = m.group(1)
g = g[:m.start()] + (
    f"\n{ind}if _pre_24 == 3 {{ //match_arm_10: void (span)\n"
    f"{ind}  {{ //block_11: void\n"
    f"{ind}    if {span_bytes('var_e')} == var_key.as_bytes() {{ //block_12: never\n"
) + g[m.end():]

p = body("n_pa_text")
p = p.replace("fn n_pa_text(cell: &std::cell::UnsafeCell<Stores>, mut var_m: DbRef,",
              "fn n_pa_text_dv(cell: &std::cell::UnsafeCell<Stores>, var___frame: DbRef, mut var_m: DbRef,", 1)
p = once(p, "  let stores: &mut Stores = unsafe { &mut *cell.get() };\n",
         "  let stores: &mut Stores = unsafe { &mut *cell.get() };\n  " + FRAME + "\n")
p = once(p, "n_pa_get__vr(cell, var_m, var_key)", "n_pa_get__vr_dv(cell, var___frame, var_m, var_key)")
old = re.search(r"== \(\(5_i64\) as i64\)\) as u8\) == 1 \{\{ //match_arm_3: never\n.*?\n(.*?\n.*?\n.*?\n.*?\n)", p)
assert old, "pa_text arm"
p = p.replace(old.group(0), old.group(0).replace("((5_i64) as i64)", "((3_i64) as i64)"), 1)
p = once(p, "var__mv_value_1 = {{let db = (var__match_subj_1); if db.rec == 0 { loft::state::STRING_NULL } else { let store = stores.store(&db); store.get_str(store.get_u32_raw(db.rec, db.pos + (4_i64) as u32)) } }}.to_string();",
         "let __sv: &[u8] = " + span_bytes("var__match_subj_1") + ";")
p = once(p, "          var___work_1.clear();\n          ops::format_text(&mut var___work_1, &var__mv_value_1, 0_i64, 2, 32);",
         "          var___work_1.clear();\n          var___work_1.push_str(unsafe { std::str::from_utf8_unchecked(__sv) });")

# ── check_request calls the twins ─────────────────────────────────────────────────────
c = body("n_check_request")
c2 = once(c, "n_decode(cell, var_frame, var__reuse_buf_5)", "n_decode_dv(cell, var_frame, var__reuse_buf_5)")
if PART != "bytes":
    c2 = once(c2, "n_pa_text(cell, {{", "n_pa_text_dv(cell, var_frame, {{")
src = src.replace(c, c2)
src += "\n" + "\n".join([d, rg, dec, g, p])
open(sys.argv[2], "w", encoding="utf-8").write(src)
