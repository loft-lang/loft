#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Write the null-text matrix cells: every text VALUE through every PLACE a text can be held,
read by every text OPERATION, with each answer computed here by hand.

`(L-Null-Text)`: a text means the same thing wherever it is held.  Absent (`null`) is one
value and a byte string — empty, or holding NUL bytes anywhere — is another, so the answers
below depend on the value alone, never on the place.  A place that changes an answer is the
defect this matrix exists to show.

usage: null_text_matrix.py SUITE_DIR [--pending VALUE,…]
  writes SUITE_DIR/rules/layout/L-Null-Text/null-text-through-<place>.loft
  --pending   values whose functions carry `@EXPECT_FAIL` (a known wrong answer not fixed yet)
"""
import argparse
from pathlib import Path

# name -> (loft expression, the value it denotes: None for null, else the text)
VALUES = {
    "null": ("nothing()", None),
    "empty": ('""', ""),
    "x": ('"x"', "x"),
    "nul": ('"\\0"', "\0"),
    "slice": ('"a\\0b"[1..2]', "\0"),
    "nulnul": ('"\\0\\0"', "\0\0"),
    "anul": ('"a\\0"', "a\0"),
    "bytes0": ("text_from_bytes(zero_byte())", "\0"),
}

# name -> (declarations, body of `fn through(t: text) -> text`)
PLACES = {
    "a-parameter": ("", "t"),
    "a-local": ("", "l = t;\n  l"),
    "an-accumulator": ("", 'acc = "";\n  acc += t;\n  acc'),
    "a-field": ("struct F { t: text? }\n", "f = F { t: t };\n  f.t"),
    "a-vector-element": ("", "v: vector<text?> = [t];\n  v[0]"),
    "an-appended-element": ("", "v: vector<text?> = [];\n  v += [t];\n  v[0]"),
    "a-tuple-member": ("", "p = (t, 1);\n  p.0"),
    "a-hash-key": (
        "struct K { k: text, n: integer }\n",
        'h: hash<K[k]> = [K { k: t, n: 1 }];\n  for e in h { return e.k; }\n  "MISSING"',
    ),
    "a-sorted-key": (
        "struct K { k: text, n: integer }\n",
        's: sorted<K[k]> = [K { k: t, n: 1 }];\n  for e in s { return e.k; }\n  "MISSING"',
    ),
    "a-yield": (
        "fn gen(t: text) -> iterator<text> { yield t; }\n",
        'for y in gen(t) { return y; }\n  "MISSING"',
    ),
    "a-par-result": (
        "fn ident(t: text?) -> text? { t }\n",
        'v: vector<text?> = [t];\n  for e in v par(r = ident(e), 1) { return r; }\n  "MISSING"',
    ),
    "a-saved-image": (
        "struct R { k: integer, t: text? }\n",
        'p = "{temp_dir() ?? "/tmp"}/loft_null_text_{now()}_{ticks()}.store";\n'
        "  h: hash<R[k]> = [R { k: 1, t: t }];\n"
        '  assert(store_persist_copy(h, p), "the image is written");\n'
        "  w: hash<R[k]> = [];\n"
        '  assert(store_load(w, p), "the image loads");\n'
        "  delete(p);\n"
        '  delete("{p}.dschema");\n'
        "  w[1].t",
    ),
}


def show(v):
    if v is None:
        return "N"
    return "'" + v.replace("\0", "~0") + "'"


def expected(v):
    """The answer of `sig` (below) for the value `v`, computed from the rules, not a run."""
    if v is None:
        parts = dict(isnull="true", not_="true", co=show("d"), len_="0", size="0", loop="0",
                     cat="N", addc="N", addt="N", eqe="false", lta="true", lte="true",
                     fmt="'[null]'", trim="N", low="N", rep="N", split="0", sw="true",
                     find="null", i0="", sub="N")
    else:
        b = v.encode()
        find = v.find("x")
        first = v[:1]
        parts = dict(
            isnull="false", not_="false", co=show(v), len_=str(len(v)), size=str(len(b)),
            loop=str(len(v)), cat=show("<" + v + ">"), addc=show(v + "x"), addt=show(v + "yz"),
            eqe="true" if v == "" else "false", lta="true" if b < b"a" else "false",
            lte="false", fmt=show("[" + v + "]"), trim=show(v.strip(" \t\n\r")),
            low=show(v.lower()), rep=show(v.replace("q", "r")),
            split="0" if v == "" else str(len(v.split(","))), sw="true",
            find="null" if find < 0 else str(find),
            # `t[0]` of a NUL is the null CHARACTER (code point 0), which formats as nothing.
            i0="" if first in ("", "\0") else first.replace("\0", "~0"),
            sub=show(b[0:1].decode()),
        )
    return ("isnull={isnull} not={not_} co={co} len={len_} size={size} loop={loop} cat={cat} "
            "addc={addc} addt={addt} eqe={eqe} lta={lta} lte={lte} fmt={fmt} trim={trim} "
            "low={low} rep={rep} split={split} sw={sw} find={find} i0={i0} sub={sub}").format(**parts)


COMMON = '''fn nothing() -> text? { null }

fn zero_byte() -> vector<u8> {
  z: vector<u8> = [];
  z += [0];
  z
}

// The text with each NUL byte written `~0`, read by byte so a NUL never becomes a character.
fn esc(t: text) -> text {
  r = "";
  for i in 0..size(t) {
    if t.byte_at(i) == 0 { r += "~0"; } else { r += t[i..i + 1]; }
  }
  r
}

fn show(t: text?) -> text { if t == null { "N" } else { "'" + esc(t) + "'" } }

// Every reader of a text, as one line.
fn sig(t: text) -> text {
  n = 0;
  for _c in t { n += 1; }
  a = t;
  a += 'x';
  b = t;
  b += "yz";
  p = "<" + t + ">";
  d = "d";
  co = show(t ?? d);
  f = show("[{t}]");
  tr = show(t.trim());
  lo = show(t.to_lowercase());
  q = "q";
  rr = "r";
  re = show(t.replace(q, rr));
  e = "";
  ea = "a";
  sw = t.starts_with(e);
  x = "x";
  fi = "{t.find(x)}";
  i0 = esc("{t[0]}");
  sub = show(t[0..1]);
  sp = len(split(t, ','));
  "isnull={t == null} not={!t} co={co} len={len(t)} size={size(t)} loop={n} cat={show(p)} addc={show(a)} addt={show(b)} eqe={t == e} lta={t < ea} lte={t < e} fmt={f} trim={tr} low={lo} rep={re} split={sp} sw={sw} find={fi} i0={i0} sub={sub}"
}
'''


# The build before a null text was an identity (sec#61's switch): every place answers the four
# rows holding a lone NUL piece as null there, on both backends.
RECEIPT = ("// @falsified-at: c138d6feb — interpret exit 1 -> 0, native exit 1 -> 0; each place's four\n"
           "//   rows holding a lone NUL piece (nul, slice, bytes0, nulnul's first byte) answered null")
PATCH_RECEIPT = ("// @falsified-by: tests/falsified/null-text-through-a-field.patch — interpret exit 1 -> 0, "
                 "interpret 4 assertion failures -> 0, native exit 1 -> 0, native 8 assertion failures -> 0")


def cell(place, decls, body, pending):
    title = place.replace("-", " ")
    out = [
        "// Copyright (c) 2026 Jurjen Stellingwerff",
        "// SPDX-License-Identifier: LGPL-3.0-or-later",
        "//",
        f"// @FR-L-Null-Text, @C171: a text means the same thing wherever it is held — here, {title}.",
        "// Absent (`null`) is one value; a byte string, empty or holding NUL bytes anywhere, is",
        "// another, so every reader's answer depends on the value alone.  Generated by",
        "// loft `scripts/null_text_matrix.py`, which computes each expected line by hand; edit",
        "// the generator, not this file.",
        "//",
        "// Axes this file MOVES:  the value (null, \"\", \"x\", NUL as a literal, a slice and a",
        "//                        byte buffer, two NULs, a trailing NUL) and every reader.",
        f"// Axes it HOLDS FIXED:   the place ({title}).",
        "//",
        RECEIPT,
    ] + ([PATCH_RECEIPT] if place == "a-field" else []) + [
        "// CHANNEL: the asserts.",
        "// ARMED: no instrument.",
        "// WITNESS: each message names the value, the place and every reader's answer.",
        "// HOLDS: the null, \"\", \"x\" and trailing-NUL rows, which answer the same in every place.",
        "",
        decls + COMMON,
        "fn through(t: text) -> text {",
        "  " + body,
        "}",
        "",
    ]
    for name, (expr, value) in VALUES.items():
        if name in pending:
            out.append(f"// @EXPECT_FAIL: {name} through {title}")
        out += [
            f"fn test_{name.replace('-', '_')}() {{",
            f"  got = sig(through({expr}));",
            f'  assert(got == "{expected(value)}", "{name} through {title}: {{got}}");',
            "}",
            "",
        ]
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("suite")
    ap.add_argument("--pending", default="")
    a = ap.parse_args()
    pending = {p for p in a.pending.split(",") if p}
    unknown = pending - VALUES.keys()
    if unknown:
        raise SystemExit(f"unknown value(s): {', '.join(sorted(unknown))}")
    d = Path(a.suite) / "rules/layout/L-Null-Text"
    d.mkdir(parents=True, exist_ok=True)
    for place, (decls, body) in PLACES.items():
        (d / f"null-text-through-{place}.loft").write_text(cell(place, decls, body, pending), encoding="utf-8")
    print(f"{len(PLACES)} cells × {len(VALUES)} values in {d}")


if __name__ == "__main__":
    main()
