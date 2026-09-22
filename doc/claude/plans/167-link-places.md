<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 167 — `&` links into a store: narrow integers at their width, text links carry their place kind

## Status

**Open — design settled with the owner 2026-09-22; P0 measured.**  Every measurement
the design rests on is recorded below and was taken on both backends at
`tuxedo-quality-2026-09-21` @ `53d8d5d4a`.  Tracker: [@PLN167](https://github.com/loft-lang/plans/issues/167).
Closes loft#1566 (`D-bind-38`), loft#1567 (`D-bind-39`), loft#1602, loft#1603 and loft#1604.

## Goal

A `&` link reaches every place `(B-Ref-Lvalue)` names — a variable, a field, an element — for
every type, on both backends, with the type alone choosing how a read or write through the
link is done and the `DbRef` or pointer alone choosing where.

## Effort + design

- **Effort:** M (P0 XS · A XS+M+S+XS · B S+XS+XS · C M+S+S · D XS) — A1 grew from S to M
  in P0, see § P0
- **Design:** ✓ — the three decisions are made; P0 answered open question 1 and refined decision 1
- **Last touched:** 2026-09-22

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

## P0 — what the width probe measured (2026-09-22)

**Decision 1 holds, with one correction: "at its width" means "in its type's FIELD
encoding", not a Rust `i8`.**  A narrow store place is not two's complement at its width.  It
is biased by the type's minimum, and a nullable place reserves a sentinel code.  The one home
for the encoding is `data::NarrowIntKind::of(width, nullable, narrow_vec, unsigned_wide)`:

| kind | stored | null | types |
|---|---|---|---|
| `Byte` | `v - min` as `u8` | — | `u8`, `i8`, `limit(0, 7)` |
| `ByteNullable` | `v - min` as `u8` | `255` | `u8?`, `i8?` |
| `ShortFull` (read) / `ShortRaw` (write) | `v - min` as `u16` | — | `u16`, `i16` |
| `Short` | `v - min + 1` as `u16` | `0` | `u16?`, `i16?` |
| `Int4` | `v` as `i32` | `i32::MIN` | `i32`, `i32?` |
| `Int4Full` / `Int4Raw` | `v` as `u32` | `u32::MAX` (nullable) | `u32` |

So an `i8` local holding `-1` as a Rust `i8` is the byte `0xFF`, and a store `i8` place holding
`-1` is the byte `0x7F`.  One `*mut u8` cannot read both.  Every narrow LOCAL therefore holds the
FIELD encoding of its type — the kind with `narrow_vec = false`, which reads every place of the
type correctly, because the element and field kinds of one type store the same bytes and differ
only in how a non-null read treats the top code.

**Native, proven by hand.**  An emitted program was edited so `x: u8`, `y: i8` and `n: u8?` are
`u8` locals holding the field encoding, and ONE `*mut u8` link was re-pointed between each local
and a field of the same type (`addr_mut::<u8>`).  Under rustc it printed every edge cell right:
the `u8` max `255` written into a field, the `i8` min `-128` into a local and max `127` into a
field, `u8?` null read from a local, its max `254` written and read, a null written into a
field and read back by the store's own `get_byte` as code `255`.

**Interpreter — open question 1 answered: two fused frame ops.**  The interpreter keeps every
integer local in an 8-byte slot and has only `OpVarInt` / `OpPutInt` for it.  Reading a narrow
slot through `OpCreateStack` + `OpGetByte` works, but every read site would emit two ops and
every write site would have to push the `DbRef` BEFORE the value, which reorders `set_var`.
`OpVarNarrow(pos, min, kind)` / `OpPutNarrow(pos, min, kind)` drop in wherever `OpVarInt` /
`OpPutInt` stand for a narrow local, and do their encoding through the SAME `Store` setters and
getters the field ops call, on a `DbRef` naming the slot — so the encoding has one home.  The
slot stays 8 bytes, which leaves the frame layout (`SLOTS.md`) and every slot offset untouched;
the encoded bytes sit at its start, where a link's `DbRef` points.  `Store::read` / `write` are
unaligned-safe, so a 2- or 4-byte access at any slot offset is sound.

**Scope of "a narrow local".**  A local and a by-value PARAMETER both: a parameter is linked or
handed to a `&` parameter as often as a local is.  A by-value narrow parameter keeps its
calling convention (`i64` on native, the plain value in the interpreter's frame) and the callee
re-encodes it once at entry, so no caller and no calling convention changes.  Tuple members stay
widened (open question 2's safe answer: a tuple place is refused as a link target anyway).

**What P0 found beside the question.**

- **loft#1604 (silent-wrong, both backends, on `main`):** a write through a `&` link or `&`
  parameter to a narrow integer is never range-checked.  `c = &w; c = c + 10` leaves a `u8`
  holding `260` where `w = w + 10` is refused, and `c += 100` on an `i8` link leaves `220` where
  `y += 100` takes the slot's default `0`.  The narrowing refusal and the range guard are asked
  of the target's type, which is `RefVar(Integer(…))` for a link, and neither looks inside it.
  Decision 1 relies on every write to a narrow slot being checked, so this is fixed first (A0).
- **The size of A1.**  Native writes a local's name at ~200 sites across `generation/`, and the
  hoist and twin machinery reads locals by name too.  A narrow Rust type makes a missed decode a
  rustc type error at most of them, which is what makes the change tractable — and why the
  local is a `u8`/`u16`/`i32`/`u32` rather than an `i64` holding the encoded bytes, which would
  compile everywhere and read wrong silently.
- **No published library declares a narrow `&` parameter** (0 hits over every `loft-libs-*`
  checkout; the corpus has one, `set_200`), so A1's ABI change reaches no shipped cdylib.  The
  published-lib gate still runs.

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
| **P0** — the width probe: hand-edit one emitted program so its linked `u8`/`i8` locals are at width and confirm a byte pointer reads and writes them, min/max/null included; list what the interpreter needs and size A2 | this file | the edited program prints the matrix's edge cells under rustc; the op inventory is written into A2's row | **Done** 2026-09-22 — § P0: the local holds the FIELD encoding; A2 = two fused frame ops |
| **A0** — loft#1604: the narrowing refusal and the range guard look through a link, so a write through `&u8` (a local link or a parameter) is checked like a write to the `u8` | loft#1604, `(B-Ref-Uniform)` | the issue's cell: `x = x + 10` through a link or parameter refused, `+=` takes the slot's default, both backends; falsified against P0's tip | Open |
| **A1** — native: a narrow local's Rust type is its FIELD encoding's storage type (`u8`/`u16`/`i32`/`u32`), decoded at each use, encoded at each write; a narrow by-value parameter re-encoded once at entry | decision 1, § P0 | `--native-emit` diff over `tests/scripts/`: only narrow-local declarations and their uses move; script corpus + bench hashes unchanged; `range_arith` pins unmoved; published-lib gate green | Open |
| **A2** — interpreter: `OpVarNarrow(pos, min, kind)` / `OpPutNarrow(pos, min, kind)` read and write a narrow local's 8-byte slot in the field encoding through the `Store` getters and setters the field ops call; a narrow by-value parameter re-encoded at entry; the debugger and every other frame reader decode | decision 1, § P0 | value parity of the corpus across backends; `a-link-to-a-narrow-integer-local-reads-and-writes-it.loft` unmoved; `LOFT_VERIFY_STACK` sweep clean | Open |
| **A3** — retire `is_narrow_store_place`; `&u8` to a field or element links | decision 1 | `tests/scripts/167-a-narrow-link-into-a-store.loft`, both backends, falsified against A2's tip; `a-link-to-a-narrow-integer-store-place-is-refused.loft` retired; `D-bind-39` closed | Open |
| **B1** — a `&` parameter takes a scalar field or element (the bind's lowering at the call; `is_addressable` asks the place set `is_amp_place` asks) | decision 3 | `bump(p.n)`, `bump(v[i])`, `bump(o.inner.k)` write through, both backends; the refusal keeps a literal and a call result out | Open |
| **B2** — loft#1603: a `&`-bound scalar local passed to a `&` parameter compiles on native (`unsafe { &mut *var_c }`, the tuple arm's form) | loft#1603 | the issue's cell answers `6 11` on both backends; falsified against B1's tip | Open |
| **B3** — loft#1602, the loud stopgap: the `&text` exemption becomes the refusal every other type gets, until C3 makes it link | loft#1602 | `app(o.s)` refused naming the cure; falsified (interpreter answered `cc` silently before) | Open |
| **C1** — `RefVar(Text)` carries its kind; the bind `t = &o.s` / `t = &v[0]` makes the store kind; reads and writes through it on both backends; cross-kind re-point refused | decision 2 | `tests/scripts/167-a-text-link-into-a-store.loft`, both backends; the stack-kind emission of every text-returning guard byte-identical before/after (`--native-emit` diff); `D-bind-38` closed | Open |
| **C2** — the store-text slice joins the disturbance check | decision 2 | a cell that dangles under `LOFT_POISON=1` before the check is refused after; growth of another store stays allowed | Open |
| **C3** — a `&text` parameter instantiated per kind; `app(o.s)` links, `app(local)` unchanged | decision 2, @PLN165 | the aliasing cell reads the written value inside the callee; the stack instance byte-identical to C1's; loft#1602 closed | Open |
| **D** — formal + docs: `binding.md` gains the text-kind rule and closes `D-bind-38`/`39`; `DIAGNOSTICS.md` rows for the two new refusals; `CHANGELOG.md` | — | `rule_tags.py check` + `registers`; `check_doc_drift.sh` | Open |

## Phase ordering

1. P0, because it sizes A2 and can kill decision 1 for the cost of a compile.  Done.
2. A0 before everything: it is a silent-wrong on `main`, and A1's encode-at-write assumes every
   write to a narrow slot has already been range-checked.
3. B1 → B2 → B3 next: independent of A, smaller, and B3 stops loft#1602's silent loss.
4. A1 → A2 → A3: native first, since its emission diff is the exact comparison; the
   interpreter follows against parity; the refusal comes off last, when both agree.
5. C1 → C2 → C3: C1 needs nothing from A; C3 needs the instantiation entry point @PLN165 E
   leaves, so it waits for that arc if E moves it.
6. D closes with the last of A3/C3.

## Open design questions

1. **A2's cure** — *answered by P0*: two fused frame ops that call the field ops' `Store`
   getters and setters on a `DbRef` naming the slot, so the encoding keeps one home and every
   `OpVarInt` / `OpPutInt` site for a narrow local swaps one op for one op.
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
