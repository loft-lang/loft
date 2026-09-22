<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 167 — `&` links into a store: narrow integers at their width, text links carry their place kind

## Status

**Open — design settled with the owner 2026-09-23, no implementation.**  Every measurement
the design rests on is recorded below and was taken on both backends at
`tuxedo-quality-2026-09-21` @ `53d8d5d4a`.  Tracker: [@PLN167](https://github.com/loft-lang/plans/issues/167).
Closes loft#1566 (`D-bind-38`), loft#1567 (`D-bind-39`), loft#1602 and loft#1603.

## Goal

A `&` link reaches every place `(B-Ref-Lvalue)` names — a variable, a field, an element — for
every type, on both backends, with the type alone choosing how a read or write through the
link is done and the `DbRef` or pointer alone choosing where.

## Effort + design

- **Effort:** M (P0 XS · A S+S+XS · B S+XS+XS · C M+S+S · D XS)
- **Design:** ✓ — the three decisions are made; two open questions are sized by P0
- **Last touched:** 2026-09-23

## What is measured, and what the rules say

The rules: `(B-Ref-Lvalue)` names a field and an element as linkable with no exception for a
type; `(B-Ref-Repoint)` re-points a link between places of one `τ`; `(B-Ref-Reshape)` refuses
a link it cannot honour rather than copying quietly.  `D-bind-38` and `D-bind-39` record the
two refusals as deviations, both open.

**How a link is represented today, for the types that link** (`c = &x; c = &p.n; c = &v[2]`):

```rust
let mut var_c: *mut i64 = std::ptr::addr_of_mut!(var_x);                         // a local
var_c = … stores.store_mut(&__ed).addr_mut::<i64>(__ed.rec, __ed.pos) as *mut i64; // a field
var_c = … stores.vec_get_or_raise_runtime(&__vr, 8, __vi) … as *mut i64;          // an element
let mut var_d: *mut u8  = … addr_mut::<u8>(…);                                    // an enum field
```

One pointer per link, re-pointed freely between a local and a store place; the interpreter is
one `DbRef` into the stack store or a heap store, read and written through the op the inner
type picks (`OpGetInt`/`OpSetInt`, `OpGetEnum`/`OpSetEnum`, …).  **No stack-versus-store kind
exists anywhere for these types, and none is needed: the type gives the width, the pointer or
`DbRef` gives the place.**

**Why narrow integers cannot join that today** (`D-bind-39`): a `u8` local is widened — `let mut
var_x: i64` on native, an 8-byte frame slot on the interpreter (`OpVarInt`/`OpPutInt` are the only
frame-slot ops) — while a `u8` field or element is 1 byte in a store (`OpGetByte`/`OpSetByte`,
`addr_mut::<u8>`).  One type, two widths.  A link that writes 8 bytes into a store overwrote the
neighbouring bytes (the measured `2042` for `250`); a link that writes 1 byte into a widened
`i8` local leaves its upper bytes stale.

**Why text cannot** (`D-bind-38`): a text local is a Rust `String` (`*mut String` when linked;
`OpGetStackText` on the interpreter), a text field is a string record in a store, read today as
`store.get_str(store.get_u32_raw(db.rec, db.pos + off)).to_string()`.  Two representations
differing in kind, not width — and `&text` is the hidden channel of every text-returning
function (`__retbuf`), so its stack form cannot change.

**Two neighbours found while measuring, both pre-existing on `main`:**

- loft#1602 (silent-wrong): a `&` *parameter* refuses a field or element for every type
  (`Parser::is_addressable` → `is_place_read`, which admits only struct-shaped places; its own
  doc says a scalar field "copies for now — a later rung") — except `&text`, whose exemption
  copies the field into a work text and never writes it back: `app(o.s)` with
  `fn app(t: &text) { t = t + "!" }` leaves `o.s` unchanged, no diagnostic, both backends.
- loft#1603: a `&`-bound scalar local handed on to a `&` parameter (`c = &p.n; bump(c)`) runs
  on the interpreter and fails rustc on native (`&mut *mut i64` for `&mut i64`): the native
  call emitter knows a plain local, a forwarded `&` parameter and a tuple link
  (`unsafe { &mut *var_b }`), not a scalar link.

## The three decisions

1. **Narrow integers: store a narrow local at its declared width.  No kind, no branch, no
   second instance.**  Once a `u8` local is a `u8` Rust variable (widened at each use, narrowed
   through the range check every write already makes) and the interpreter reads and writes its
   slot at one byte, `&u8` is a byte pointer or a byte-wide `DbRef` access everywhere — exactly
   what `&integer` is today.  The refusal in `is_narrow_store_place` then has nothing left to
   protect and is retired.  Rejected: a runtime width on the link (a branch on every access,
   and native has no stack store to branch on), and a kind in the type (refuses cross-kind
   re-pointing for no reason once the widths agree).
2. **Text: the kind is static, so it lives in the type; the stack kind keeps its shortcut.**
   `RefVar(Text)` records whether the link names a stack text or a store text.  The stack kind
   is today's `*mut String`, untouched.  The store kind is a `DbRef` plus field: its read is the
   field read above **minus the `.to_string()`** — a `&str` into the store — so every borrowing
   consumer (formatting, comparison, `len`, @PLN158 W2's `text_borrowed` walk) takes either kind
   with no copy; its write is the setter `o.s = "dd"` already emits.  Consequences: a `&text`
   parameter called with both kinds is instantiated once per kind (the @PLN165 machinery), and
   the stack instance is byte-identical to today's function; re-pointing one link across kinds
   is a type change and is refused with the cure named; a store-text `&str` is a view into a
   store, so it joins the `(B-Ref-Reshape)` disturbance check (store memory moves on growth).
   Rejected: a store for the stack on native (every linked local a store access, or a runtime
   branch on every link access); copy-in/write-back at the call (the copy shows inside the
   callee, which `(B-Ref-Reshape)` forbids doing quietly).
3. **A `&` parameter takes a field or element through the lowering the `&` binding already
   has** (`expressions.rs`, the `heap_ref` arm that turns `OpGetInt(p, off)` into a store
   `DbRef`; native's `&mut (expr)` argument arm is already there for it).  The refusal was the
   stale half of a rung built only at the bind.

## Composition matrix — Stage A

Build every cell as a `/tmp` probe on `--interpret` first, expected values hand-computed,
then both backends; the cells graduate to `tests/scripts/167-*.loft`.  Run
`python3 scripts/matrix_axes.py file <guard>` on each finished guard: the axes below are
what the design claims to hold fixed, and the tool says which the cells actually reach.

| Axis (tool id) | Values the cells must reach |
|---|---|
| place kind (A3 argument spelling) | `local` · `field` · nested field (`o.inner.k`) · `element` (`v[i]`, `i` constant and variable) · `tuple-element` (expected: still refused — `D-bind-39` says a tuple place is read element-wise; confirm, do not widen) |
| type (A7 element type) | `narrow-int`: `u8` · `i8` · `u16` · `i32` (forced 4) · `integer limit(0, 7)` · `text` — and the controls `integer`, `float`, `enum`, `struct`, which must not move |
| nullability (A5) | `τ` and `τ?` (a nullable narrow slot reserves one code for the sentinel: `byte_width(true)`) |
| whose container (A2 provenance) | `local-literal` · `parameter` · `callee-return` — a link into a parameter's field outlives the frame, a callee's does not |
| context (A4, A9) | top-level · `loop-body` (re-linked every pass) · `call-argument` (the `&` parameter path) |
| link operation — *declared, not tool-measured* | read · write · `+=` · re-point to the same kind · re-point across kinds (text: refused) · pass to a `&` parameter · forward a `&` parameter to another |
| value at the edge — *declared* | the type's min and max (an `i8` local holding `-1` written through the link is THE cell for decision 1: it fails on a widened slot) · null · empty text · a text that reallocates its buffer |
| aliasing — *declared* | the callee reads the same place by another route during the call (`brighten(p.r, p)` prints `p.r` inside: must read the written value) |
| disturbance — *declared* | a store-text link held across a growth of its own store (refused) · across a growth of another store (allowed) |
| backend | both, and the ABI: a `&u8` parameter is `&mut u8` on native after A, so a native cdylib built before A cannot be called by a program built after — the published-lib gate (`scripts/revalidate_libs_local.sh`) is part of A's verification |

The four *declared* axes are ones `matrix_axes.py` has no vocabulary for, so its report cannot
vouch for them: each guard states, in its head comment, which of their values every cell
reaches, and the review reads that list against the cells by hand.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — the width probe: hand-edit one emitted program so its linked `u8`/`i8` locals are `u8`/`i8` and confirm a byte pointer reads and writes them, min/max/null included; list what the interpreter needs (new `OpVarByte`/`OpPutByte` frame ops, or narrow access through the stack-store `DbRef`) and size A2 | this file | the edited program prints the matrix's edge cells under rustc; the op inventory is written into A2's row | Open |
| **A1** — native: a narrow local's Rust type is its width, widened at each use, narrowed through the existing range check on each write | decision 1 | `--native-emit` diff over `tests/scripts/`: only narrow-local declarations and their uses move; script corpus + bench hashes unchanged; `range_arith` pins unmoved; published-lib gate green | Open |
| **A2** — interpreter: a narrow local's slot is read and written at its width (the cure P0 chose) | decision 1, P0 | value parity of the corpus across backends; `a-link-to-a-narrow-integer-local-reads-and-writes-it.loft` unmoved | Open |
| **A3** — retire `is_narrow_store_place`; `&u8` to a field or element links | decision 1 | `tests/scripts/167-a-narrow-link-into-a-store.loft`, both backends, falsified against A2's tip; `a-link-to-a-narrow-integer-store-place-is-refused.loft` retired; `D-bind-39` closed | Open |
| **B1** — a `&` parameter takes a scalar field or element (the bind's lowering at the call; `is_addressable` asks the place set `is_amp_place` asks) | decision 3 | `bump(p.n)`, `bump(v[i])`, `bump(o.inner.k)` write through, both backends; the refusal keeps a literal and a call result out | Open |
| **B2** — loft#1603: a `&`-bound scalar local passed to a `&` parameter compiles on native (`unsafe { &mut *var_c }`, the tuple arm's form) | loft#1603 | the issue's cell answers `6 11` on both backends; falsified against B1's tip | Open |
| **B3** — loft#1602, the loud stopgap: the `&text` exemption becomes the refusal every other type gets, until C3 makes it link | loft#1602 | `app(o.s)` refused naming the cure; falsified (interpreter answered `cc` silently before) | Open |
| **C1** — `RefVar(Text)` carries its kind; the bind `t = &o.s` / `t = &v[0]` makes the store kind; reads and writes through it on both backends; cross-kind re-point refused | decision 2 | `tests/scripts/167-a-text-link-into-a-store.loft`, both backends; the stack-kind emission of every text-returning guard byte-identical before/after (`--native-emit` diff); `D-bind-38` closed | Open |
| **C2** — the store-text slice joins the disturbance check | decision 2 | a cell that dangles under `LOFT_POISON=1` before the check is refused after; growth of another store stays allowed | Open |
| **C3** — a `&text` parameter instantiated per kind; `app(o.s)` links, `app(local)` unchanged | decision 2, @PLN165 | the aliasing cell reads the written value inside the callee; the stack instance byte-identical to C1's; loft#1602 closed | Open |
| **D** — formal + docs: `binding.md` gains the text-kind rule and closes `D-bind-38`/`39`; `DIAGNOSTICS.md` rows for the two new refusals; `CHANGELOG.md` | — | `rule_tags.py check` + `registers`; `check_doc_drift.sh` | Open |

## Phase ordering

1. P0, because it sizes A2 and can kill decision 1 for the cost of a compile.
2. A1 → A2 → A3: native first, since its emission diff is the exact comparison; the
   interpreter follows against parity; the refusal comes off last, when both agree.
3. B1 → B2 → B3: independent of A, and B3 stops the silent loss before C exists.
4. C1 → C2 → C3: C1 needs nothing from A; C3 needs the instantiation entry point @PLN165 E
   leaves, so it waits for that arc if E moves it.
5. D closes with the last of A3/C3.

## Open design questions

1. **A2's cure** — new narrow frame-slot ops, or narrow access through the stack-store `DbRef`
   with `OpGetByte`/`OpSetByte`?  The first is faster and more code; the second reuses the
   field ops that already carry the `min` offset and the nullable twins.  P0 answers it.
2. **Tuple members** — a tuple local is a frame blob (`element_stack_size`); does storing a
   narrow member at width change that layout, and is a store tuple's member already narrow?
   Measure in P0 before A1 touches tuples; the safe answer is to leave tuple members widened
   and out of scope, since a tuple place is refused as a link target anyway.
3. **Which cure for the `&text` parameter when only ONE kind is ever passed** — none: one
   instance, as today.  Instantiation is triggered by the second kind at a call site.

## Cross-arc dependencies

- @PLN165 (generics): C3 reuses per-instance substitution; E1 (a stdlib generic reserving a
  built-in's name) is the nearest moving part.
- @PLN158 W2 (`@FR-R-TextBorrow`): the store-kind read must hand `text_borrowed` a `&str`
  exactly as the stack kind does, or the walk loses its borrow.
- `COMPATIBILITY.md`: A1 changes the native ABI of every `&u8`/`&i8`/`&u16`/`&i32` parameter;
  a language change owes the published-lib gate (`revalidate_libs_local.sh`) before its PR.

## See also

- `doc/claude/formal/binding.md` — `(B-Ref-Lvalue)`, `(B-Ref-Repoint)`, `(B-Ref-Reshape)`,
  `D-bind-38`, `D-bind-39`.
- loft#1566, loft#1567 (the two refusals), loft#1602, loft#1603 (the parameter path).
- `src/parser/mod.rs` `is_amp_place` / `is_narrow_store_place` / the `&`-argument refusal;
  `src/parser/expressions.rs` the `heap_ref` bind arm; `src/generation/calls.rs` the
  `&`-argument arms; `src/generation/dispatch.rs` the link binds; `src/state/codegen.rs`
  the `RefVar` read dispatch.
- @PLN165, @PLN158.
