#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Generate the ..rest store-lifetime probe corpus. One shape per file. Each carries a
# hand-computed expected value (@EXPECT) and a leak expectation (@LEAK ok|leak) that the
# runner checks against the ground-truth interpreter leak-check on BOTH backends.
import os, textwrap

OUT = os.path.join(os.path.dirname(__file__), "probes")
os.makedirs(OUT, exist_ok=True)

# Common type + helper preludes, keyed so probes only pull what they use.
DETAIL = "enum Detail { Ship { carrier: text }, Pickup { store: integer } }"
POINT = "struct P { x: integer, y: integer }"
# Payload-variant tokens only (unit variants leak independently — a separate pre-existing bug).
TOKP = "enum Tok { Kw { word: text }, Ident { name: text }, Num { value: integer } }"
STMT = "enum Stmt { LetS { name: text, count: integer }, Other { count: integer } }"

probes = []  # (name, desc, expect, leak, body)

def add(name, desc, expect, leak, body):
    probes.append((name, desc, expect, leak, textwrap.dedent(body).strip() + "\n"))

# ── AXIS A: element type × rest usage (rest is the payload; bare head, no escape) ──
# usage: return rest, len(rest), pass rest to a fn, index rest, rest unused
for et, prelude, mk in [
    ("scalar", "", "[10, 20, 30, 40]"),
    ("text",   "", '["a", "b", "c", "d"]'),
    ("struct", POINT, "[P{x:1,y:1}, P{x:2,y:2}, P{x:3,y:3}, P{x:4,y:4}]"),
    ("enum",   DETAIL, "[Ship{carrier:\"A\"}, Pickup{store:2}, Pickup{store:3}, Pickup{store:4}]"),
]:
    vt = {"scalar":"integer","text":"text","struct":"P","enum":"Detail"}[et]
    # A1 return rest (len 3) — bare-binding fallback avoids the separate []-heap native bug
    add(f"a-{et}-return", f"return rest ({et})", "3", "ok", f"""
        {prelude}
        fn f(v: vector<{vt}>) -> vector<{vt}> {{ match v {{ [h, ..rest] => rest, w => w }} }}
        fn main() {{ r = f({mk}); assert(len(r) == 3, "len={{len(r)}}"); print("PASS"); }}
    """)
    # A2 len(rest) in-arm
    add(f"a-{et}-len", f"len(rest) in-arm ({et})", "3", "ok", f"""
        {prelude}
        fn f(v: vector<{vt}>) -> integer {{ match v {{ [h, ..rest] => len(rest), _ => -1 }} }}
        fn main() {{ assert(f({mk}) == 3, "n={{f({mk})}}"); print("PASS"); }}
    """)
    # A3 pass rest to a fn
    add(f"a-{et}-pass", f"pass rest to a fn ({et})", "3", "ok", f"""
        {prelude}
        fn nt(w: vector<{vt}>) -> integer {{ len(w) }}
        fn f(v: vector<{vt}>) -> integer {{ match v {{ [h, ..rest] => nt(rest), _ => -1 }} }}
        fn main() {{ assert(f({mk}) == 3, "n={{f({mk})}}"); print("PASS"); }}
    """)
    # A4 rest unused, return const
    add(f"a-{et}-unused", f"rest materialised but unused ({et})", "42", "ok", f"""
        {prelude}
        fn f(v: vector<{vt}>) -> integer {{ match v {{ [h, ..rest] => 42, _ => -1 }} }}
        fn main() {{ assert(f({mk}) == 42, "n={{f({mk})}}"); print("PASS"); }}
    """)
    # A5 REUSE axis (the axis the corpus originally MISSED) — call the ..rest fn TWICE on the
    # SAME subject. The materialisation must not free a VIEW of the subject's elements: a
    # struct-enum LINKED-element read derefs the record pointer, so freeing the copy-temp frees
    # the subject's record → corruption seen only on the 2nd call. Both calls must agree.
    add(f"a-{et}-reuse", f"call ..rest fn twice on the same subject ({et})", "3", "ok", f"""
        {prelude}
        fn f(v: vector<{vt}>) -> integer {{ match v {{ [h, ..rest] => len(rest), _ => -1 }} }}
        fn main() {{ s = {mk}; a = f(s); b = f(s); assert(a == 3 && b == 3, "reuse: {{a}},{{b}}"); print("PASS"); }}
    """)

# ── AXIS B: head capture escape (the leak zone) ──
# B1 variant-head field capture, UNUSED, return int (control — clean)
add("b-varhead-unused-int", "variant head capture unused, len(rest) int", "3", "ok", f"""
    {TOKP}
    fn nt(w: vector<Tok>) -> integer {{ len(w) }}
    fn f(ts: vector<Tok>) -> integer {{ match ts {{ [Kw {{ word }}, ..rest] => nt(rest), whole => -1 }} }}
    fn main() {{ ts = [Kw{{word:"let"}}, Num{{value:1}}, Num{{value:2}}, Num{{value:3}}]; assert(f(ts) == 3, "n={{f(ts)}}"); print("PASS"); }}
""")
# B2 variant-head field ESCAPES as promoted text, rest UNUSED (the e1 leak)
add("b-varhead-escape-text-unused", "captured head field escapes (&text), rest unused — sub-class B, FIXED", "let", "ok", f"""
    {TOKP}
    fn f(ts: vector<Tok>) -> text {{ match ts {{ [Kw {{ word }}, ..rest] => word, whole => "none" }} }}
    fn main() {{ ts = [Kw{{word:"let"}}, Num{{value:1}}]; assert(f(ts) == "let", "w={{f(ts)}}"); print("PASS"); }}
""")
# B3 variant-head field ESCAPES as promoted text, rest USED (does using rest change it?)
add("b-varhead-escape-text-used", "captured field + rest into a FORMATTED text (owned, not promoted &text)", "let3", "ok", f"""
    {TOKP}
    fn f(ts: vector<Tok>) -> text {{ match ts {{ [Kw {{ word }}, ..rest] => "{{word}}{{len(rest)}}", whole => "none" }} }}
    fn main() {{ ts = [Kw{{word:"let"}}, Num{{value:1}}, Num{{value:2}}, Num{{value:3}}]; assert(f(ts) == "let3", "w={{f(ts)}}"); print("PASS"); }}
""")
# B4 captured field escapes into an ENUM, rest USED (the p1 leak — must stay CORRECT value)
add("b-varhead-escape-enum-used", "captured field + nt(rest) into an enum (p1) — sub-class A, FIXED", "3", "ok", f"""
    {TOKP}
    {STMT}
    fn nt(w: vector<Tok>) -> integer {{ len(w) }}
    fn parse(ts: vector<Tok>) -> Stmt {{ match ts {{ [Kw {{ word }}, ..rest] => LetS {{ name: word, count: nt(rest) }}, whole => Other {{ count: -1 }} }} }}
    fn cnt(s: Stmt) -> integer {{ match s {{ LetS {{ name, count }} => count, Other {{ count }} => count }} }}
    fn main() {{ ts = [Kw{{word:"let"}}, Num{{value:1}}, Num{{value:2}}, Num{{value:3}}]; assert(cnt(parse(ts)) == 3, "c={{cnt(parse(ts))}}"); print("PASS"); }}
""")
# B5 captured field escapes into a STRUCT, rest used
add("b-varhead-escape-struct-used", "captured field + rest into a struct — sub-class A, FIXED", "3", "ok", f"""
    {TOKP}
    struct R {{ name: text, count: integer }}
    fn nt(w: vector<Tok>) -> integer {{ len(w) }}
    fn f(ts: vector<Tok>) -> R {{ match ts {{ [Kw {{ word }}, ..rest] => R {{ name: word, count: nt(rest) }}, whole => R {{ name: "x", count: -1 }} }} }}
    fn main() {{ ts = [Kw{{word:"let"}}, Num{{value:1}}, Num{{value:2}}, Num{{value:3}}]; assert(f(ts).count == 3, "c={{f(ts).count}}"); print("PASS"); }}
""")
# B6 head field captured but NOT escaping, returns rest itself
add("b-varhead-return-rest", "variant head capture unused, return rest vector", "3", "ok", f"""
    {TOKP}
    fn f(ts: vector<Tok>) -> vector<Tok> {{ match ts {{ [Kw {{ word }}, ..rest] => rest, whole => whole }} }}
    fn main() {{ ts = [Kw{{word:"let"}}, Num{{value:1}}, Num{{value:2}}, Num{{value:3}}]; assert(len(f(ts)) == 3, "n={{len(f(ts))}}"); print("PASS"); }}
""")

# ── AXIS C: name:pat element capture escape ──
# C1 name:pat capture escapes + ..rest
add("c-namepat-escape", "name:pat ELEMENT capture escapes + ..rest (clean — element, not field)", "UPS", "ok", f"""
    {DETAIL}
    fn f(ds: vector<Detail>) -> Detail {{ match ds {{ [tok:Ship, ..rest] => tok, _ => Pickup {{ store: -1 }} }} }}
    fn main() {{ ds = [Ship{{carrier:"UPS"}}, Pickup{{store:1}}, Pickup{{store:2}}];
      match f(ds) {{ Ship {{ carrier }} => assert(carrier == "UPS", "c={{carrier}}"), _ => assert(false, "x") }} print("PASS"); }}
""")

# ── AXIS D: nesting / multi-arm / subject ──
# D1 two ..rest arms (multi-arm; first-match-wins, both materialise)
add("d-two-rest-arms", "two arms each with ..rest", "103", "ok", """
    fn f(v: vector<integer>) -> integer { match v { [a, b, ..r] => len(r) + 100, [a, ..r] => len(r), _ => -1 } }
    fn main() { assert(f([1,2,3,4,5]) == 103, "n={f([1,2,3,4,5])}"); print("PASS"); }
""")

# ── AXIS E: the pre-existing []-empty-heap-vector-in-match-arm native bug (isolated) ──
add("e-empty-heap-vec-match", "[] empty heap vector as a match _ arm (no ..rest)", "0", "ok", f"""
    {POINT}
    fn pick(n: integer, src: vector<P>) -> vector<P> {{ match n {{ 0 => src, _ => [] }} }}
    fn main() {{ a = [P{{x:1,y:1}}]; assert(len(pick(9, a)) == 0, "n={{len(pick(9,a))}}"); print("PASS"); }}
""")
add("e-empty-scalar-vec-match", "[] empty scalar vector as a match _ arm (control)", "0", "ok", """
    fn pick(n: integer, src: vector<integer>) -> vector<integer> { match n { 0 => src, _ => [] } }
    fn main() { a = [1]; assert(len(pick(9, a)) == 0, "n={len(pick(9,a))}"); print("PASS"); }
""")
# D2 subject is a local (not a param)
add("d-subject-local", "subject is a local", "3", "ok", f"""
    {TOKP}
    fn f() -> text {{ ts = [Kw{{word:"hi"}}, Num{{value:1}}, Num{{value:2}}, Num{{value:3}}]; match ts {{ [Kw {{ word }}, ..rest] => "{{word}}{{len(rest)}}", whole => "none" }} }}
    fn main() {{ assert(f() == "hi3", "r={{f()}}"); print("PASS"); }}
""")
# D3 ..rest arm inside a match nested in another match
add("d-nested-match", "..rest inside a match returned from an outer match", "3", "ok", f"""
    fn inner(v: vector<integer>) -> integer {{ match v {{ [h, ..r] => len(r), _ => -1 }} }}
    fn f(n: integer, v: vector<integer>) -> integer {{ match n {{ 0 => inner(v), _ => -9 }} }}
    fn main() {{ assert(f(0, [1,2,3,4]) == 3, "n={{f(0,[1,2,3,4])}}"); print("PASS"); }}
""")
# D4 mixed name:pat + ..rest, capture escapes
add("d-namepat-plus-rest-escape", "name:pat head + ..rest, name escapes as text — sub-class B, FIXED", "hi", "ok", f"""
    {TOKP}
    fn f(ts: vector<Tok>) -> text {{ match ts {{ [k:Kw {{ word }}, ..rest] => word, whole => "none" }} }}
    fn main() {{ ts = [Kw{{word:"hi"}}, Num{{value:1}}, Num{{value:2}}]; assert(f(ts) == "hi", "r={{f(ts)}}"); print("PASS"); }}
""")

# Emit files + manifest.
manifest = []
for name, desc, expect, leak, body in probes:
    header = f"// @DESC {desc}\n// @EXPECT {expect}\n// @LEAK {leak}\n"
    path = os.path.join(OUT, name + ".loft")
    with open(path, "w") as fh:
        fh.write(header + body)
    manifest.append((name, expect, leak, desc))

with open(os.path.join(os.path.dirname(__file__), "manifest.tsv"), "w") as fh:
    fh.write("name\texpect\tleak\tdesc\n")
    for row in manifest:
        fh.write("\t".join(row) + "\n")
print(f"wrote {len(probes)} probes to {OUT}")
