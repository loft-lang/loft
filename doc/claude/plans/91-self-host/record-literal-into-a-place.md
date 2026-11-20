# @PLN91 — a record literal built apart for a field place of a local (prepared subject)

**The family:** "a record literal written into a field or an element" (15 programs at try
f3ff4b511, 6 held by it alone).  This page takes its largest part, "a vector literal field of a
place in a local": 4 of the 6 sole programs.  The refusal is `el_ex_place.loft` ~1164.  The
port already builds the literal apart when the place's root is a PARAMETER; for a LOCAL root it
refuses, because the head (the null-inits at the top of the body) did not match Rust's.  This
page measures that head.

**The rule:** `@FR-R-InPlaceLiteral` — a record literal written into a field is built in the
field, unless an item of a vector-literal field reads the place (`v: [b.inner.n]`), or the
place is an element (`s.els[1] = …`), or an item is itself a record literal.  Then Rust builds
the record apart (`__ref_p2_N`: the vector field emptied, one push per item, the other fields)
and copies it in (`OpCopyRecord`).  The port's `apart` decision already says this.

## Rust's head: the preamble by descending number, the unused `_elm_N` after the first

Pass 1 registers an `_elm_N` for the vector literal.  On pass 2 the built-apart road does not
use it, but it stays declared (`ref(boolean)` for scalar items, `ref(R)` for record items), and
it takes a null-init in the head.  Measured on the four functions below (numbers from
`LOFT_VAR_TABLE=<fn>`):

```
m2   b.inner = Vf { v: [b.inner.n, 8], n: 3 }
     vars  0 b, 1 __ref_1, 2 _elm_1, 3 __work_1, 4 __ref_p2_1, 5 _elm_2
     head  __ref_p2_1(4)  _elm_1(2)  __ref_1(1)  __work_1 = ""
m3   b.inner = Vf { v: [len(b.inner.v), b.inner.v[0]], n: 3 }
     vars  0 b, 1 __ref_1, 2 _elm_1, 3 __work_1, 4 _build_src_1, 5 __vdb_1, 6 __ref_p2_1, 7 _elm_2
     head  __ref_p2_1(6)  _elm_1(2)  __vdb_1(5)  __ref_1(1)
m10  r.inner = Rv { rs: [R { n: 1 }, R { n: 2 }], k: 5 }
     vars  0 r, 1 _elm_1, 2 _elm_2, 3 __work_1, 4 __ref_p2_1, 5 __ref_p2_2, 6 _elm_3
     head  __ref_p2_2(5)  _elm_2(2)  __ref_p2_1(4)  __work_1 = ""
stages-copies (a-literal-reading-its-places-own-collection-keeps-it.loft:109)
     head  __ref_p2_21  _elm_5  _elm_7  _elm_8  _elm_11  _elm_14  _elm_17 …
```

Read off all four: **the work-ref preamble in descending variable number (`@C168`,
expressions.rs ~889, each `ls.insert(0, …)`), with every unused `_elm_N`, in ascending number,
placed directly after the highest-numbered entry.**  `_elm_N` is not emitted by that preamble
loop (`LOFT_TRACE_PREAMBLE=1` lists no `_elm_`), so it is inserted after the loop, at index 1.
The `_elm_N` pass 2 does use (m10's `_elm_1 = OpNewRecord(__ref_p2_1, …)` inside the build) has
no head entry.

## What to code

At the refusal (el_ex_place.loft ~1164): drop the parameter-only condition, and give the local
root the head the parameter root already gets, ordered as above.  The port's parameter branch
emits the same entries; what changes is where the `_elm_N` null-inits go: after the
highest-numbered preamble entry, ascending among themselves.  If the port's head is built by a
general preamble, that is one rule there, not a special case here.

## Probes (the sole programs; each is a suite cell with hand-computed answers)

```
kr-suite/rules/rewrites/R-InPlaceLiteral/a-literal-field-of-plain-items-is-built-in-its-place.loft   m2, m9, m10 (m3 too: refused, not sole-held)
kr-suite/rules/heap/H-ClearRelease/a-reused-vector-or-place-releases-what-it-held.loft:69            n_test_an_in_place_literal_in_a_loop
kr-suite/rules/ownership/O-One-Kind/a-removed-temporary-store-answers-as-the-store-it-stands-for.loft:45   n_test_a_literal_written_into_its_element
kr-suite/rules/rewrites/R-InPlaceLiteral/a-literal-reading-its-places-own-collection-keeps-it.loft:135      n_test_a_field_literal_stages_copies_of_its_heap_reads
```

Three of the sole functions have no unused `_elm_N` in their head, only the descending
preamble: m9 (`s.els[1] = Vf { … }`, an element place) `__ref_p2_1, __work_1`;
n_test_a_literal_written_into_its_element `__ref_p2_1, __vdb_1, __work_1`;
n_test_an_in_place_literal_in_a_loop `__ref_1, __ref_p2_1, __vdb_1, __work_3, __work_2,
__work_1`.  Their `_elm_1 = OpNewRecord(<vector>, …)` is a USED element temp, so for them only
the condition is lifted.  Check each against `probe.py FILE --show <fn>` after the change.
The other sub-shapes of the family, "a place it is built apart for" (11 sites) and "a field
that is no plain scalar" (5), are not on this page.
