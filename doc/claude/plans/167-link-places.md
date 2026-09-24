<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 167 — `&` links into a store: narrow integers at their width, text links carry their place kind

## Status

**Open — P0, A0–A3, B0–B4 and C1–C3 landed (A0–A2 and B 2026-09-22, A3 and C1 2026-09-23, C2
and C3 2026-09-24); D remains, and C3's function-value spelling is open as D-bind-57 / loft#1656.**  Every measurement the design rests on is recorded below and was taken on both backends
at `tuxedo-quality-2026-09-21` @ `53d8d5d4a`.  Tracker: [@PLN167](https://github.com/loft-lang/plans/issues/167).
Closes loft#1566 (`D-bind-38`), loft#1567 (`D-bind-39`), loft#1602, loft#1603, loft#1604 and loft#1605.

**Arc C is loft3-ca's (2026-09-23), and the owner has RULED that it is built to decision 2.**
They had started loft#1566 independently, on `tuxedo-1562-layout-gate` @
`979a77118..df6155481`, before reading this plan — and what they built is decision 2's
*rejected* option, measured rather than argued: a runtime-kind `codegen_runtime::TextLink` (a
pointer to a `String` **or** a slot `DbRef`, branched on at every access) whose `edit()` guard
copies a slot's text out and writes it back at the statement end.  That is both rejected shapes
at once — a runtime branch per link access, and copy-in/write-back — and it also cost the stack
kind its byte-identical emission, since every user `&text` became a `TextLink`.  The owner's
ruling (2026-09-23) is **rebuild per decision 2**, and those four commits are reverted at
`ed429181d`; they are joined nowhere and are not to be picked.  C1 starts at its open probe —
which sites rebuild a `Text`'s `Deps` from scratch.

The episode is worth keeping for the reason it happened, not for who was right: the design was
decided and written down a day earlier, and it was re-derived anyway because the work was found
through the ISSUE (loft#1566) and the issue does not say that a plan owns it.  An issue a plan
has taken is worth a line saying so.

Two findings of theirs are representation-independent and stand either way: `is_amp_place` admits
a text place, and `for c in t` over any `&text` (a plain parameter included) was an ICE at
`collections.rs` `iter_text`, because the loop set-up tested `Type::Text` without unwrapping
`RefVar` at four sites — fixed at `40488e943` (`walks_text`, guard
`a-text-walk-over-a-link-reads-the-linked-text.loft`, falsified on both backends).  A third,
their `D-bind-53`, is fixed here instead — see A3's row — and their hunk was reverted with the
TextLink commits, so `D-bind-54` is the only version.

## Goal

A `&` link reaches every place `(B-Ref-Lvalue)` names — a variable, a field, an element — for
every type, on both backends, with the type alone choosing how a read or write through the
link is done and the `DbRef` or pointer alone choosing where.

## Effort + design

- **Effort:** M (P0 XS · A XS+M+S+XS · B S+XS+XS · C M+S+S · D XS) — A1 grew from S to M
  in P0, see § P0
- **Design:** ✓ — the three decisions are made; decision 1's scope settled 2026-09-22 (linked locals only)
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

**Why narrow integers could not join that** (`D-bind-39`, closed 2026-09-23 by A1–A3): a `u8` local
was widened — `let mut var_x: i64` on native, an 8-byte frame slot on the interpreter
(`OpVarInt`/`OpPutInt` were the only frame-slot ops) — while a `u8` field or element is 1 byte in a
store (`OpGetByte`/`OpSetByte`, `addr_mut::<u8>`).  One type, two widths.  A link that writes 8
bytes into a store overwrote the neighbouring bytes (the measured `2042` for `250`); a link that
writes 1 byte into a widened `i8` local leaves its upper bytes stale.  **A1/A2 dissolved this**: a
LINKED narrow local holds its type's field encoding, so there is one width, not two, and A3 then
found that what still blocked the store place was not representation at all but two gates written
to this paragraph's premise.

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

1. **Narrow integers: a narrow local that can be linked holds its type's FIELD encoding.  No
   kind on the link, no branch on access, no second instance of a `&` function.**  A `u8` field
   is one byte biased by the type's minimum, with a sentinel code when nullable
   (`NarrowIntKind`, § P0); once a linked `u8` local holds that same byte and the interpreter
   reads and writes its slot in that encoding, `&u8` is a byte pointer or a byte-wide `DbRef`
   access everywhere — exactly what `&integer` is today.  The refusal in
   `is_narrow_store_place` then has nothing left to protect and is retired.  Rejected: a runtime
   width on the link (a branch on every access, and native has no stack store to branch on); a
   kind in the type (refuses cross-kind re-pointing for no reason once the encodings agree); a
   Rust `i8`/`i16` local (two's complement, which a store `i8` is not — P0 measured `-1` as
   `0x7F` in the store and `0xFF` in the register).
   **DECIDED 2026-09-22 with the owner: only a LINKED narrow local changes** — a local (or by-value
   parameter) whose address is taken by a `&` bind, a `&` argument or a re-point.  Revisable: if
   the two shapes prove costlier than the uniform one, the first option below is the fallback,
   and nothing in the link itself depends on the choice.  The two options as weighed:
   - *every* narrow local, whether or not anything links it: one shape, no per-variable fact,
     and the whole corpus exercises the new encoding — at the price of every narrow read and
     write in a loop such as `for b in px { t += b }`, which is where the drawing bench spends
     its time, and ~200 native sites that write a local's name (plus the hoist and twin
     machinery, which reads locals by name);
   - *only* a local whose address is taken — a `&` bind or a `&` argument names it, a fact the
     parser has per variable: the link stays uniform, because a linked local is always in the
     field encoding, but the emitter holds two shapes for a narrow local and picks by that fact,
     and almost nothing exercises the new one (the corpus has ONE linked narrow local, no
     published library declares a narrow `&` parameter) so the guards carry the weight.  It also
     leaves a closure capture and a generator frame (R9) untouched.
   **The instrument that makes the corpus exercise the new shape anyway:** a generation-time
   switch, `LOFT_LINK_ALL_NARROW=1`, treats EVERY narrow local as linked.  The corpus compiled
   natively under it turns each site that reads an encoded local as its value into a rustc type
   error (`u8` where an `i64` is wanted), and its value cells under both backends check the
   decode; shipped code marks only what is linked.  A1 lands with a clean corpus under the
   switch, and the switch stays as the bisect step for a wrong narrow value.
   **Measured 2026-09-22:** the whole script corpus under `LOFT_LINK_ALL_NARROW=1` passes on
   BOTH backends but for `931b-i32-accepted-forms-run`, whose subject the switch inverts by
   construction — its cell asserts *"a local keeps the 8-byte slot"* and reads `705032704` for
   `5000000000` once every narrow local is encoded.  That is the one expected difference, and
   no other cell moved.  A compiler temp is never linked and is spared by the switch too:
   without that exclusion 7 native and 1 interpreted scripts fail, because a `__ncc_N` /
   `__dn4_N` holds a value the narrowing has not yet checked.
   **Cells the scope decision owes, whichever way it goes** (2026-09-22): a narrow by-value
   PARAMETER linked in its callee (re-encoded at entry — only if linked, under the second
   option); a `&u8` parameter RE-POINTED to a local of the callee (`c = &y`: a re-point takes an
   address too); a LOOP VARIABLE linked in the body; a `par` worker's capture (R9's shape); a
   local first bound inside an `if` ARM and read after it — the pre-declaration writes `0`, and
   a zero byte decodes as the type's MINIMUM (`-128` for an `i8` no arm bound); the decode and
   encode seams `n: integer = x`, `x = n as u8`, `x + 1`, `"{x}"`; a narrow RETURN, which native
   already answers as a two's-complement Rust `i8` (a third representation, kept: it is the
   convention between loft functions); a null landing in a linked `u8?` as the sentinel code,
   and `x == null` testing that code; and every reader of a frame slot that is not the program —
   `loft debug` / `setValue`, `introspect`, reflection, `LOFT_VAR_TABLE`, `LOFT_VERIFY_STACK`'s
   shadow tags (the falsifier: an `OpPutNarrow` write tags the slot and an 8-byte read must be
   reported).  Unaffected: `(R-Scalar)`'s hoist temps (wide, never linked), tuple members
   (refused as targets; a whole-tuple link reads members through the element ops), and
   `range_arith`'s `type_range` (the value range is the encoding's invariant).  The "linked"
   fact is recorded on pass 1 and read on pass 2, as `ref_linked_tuple_locals` already is.
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
   **Amended 2026-09-23 (C1's probe): the kind rides on the VARIABLE, not in the `Deps` of the
   linked `Text`.**  The probe this plan set for C1 (§ Where the text kind can ride) failed:
   `Type::with_deps`, `Type::without_deps` and `make_independent` rebuild a `Text`'s deps, the IR
   codec reads back only the dep items, and 56 sites build `Type::Text(Deps::none())` fresh — each
   of which would drop a side flag in silence, reverting a store link to the stack kind with a
   wrong read and no diagnostic.  A link exists only in a variable (a local bound by `&`, a
   parameter), so the fact is `Variable::store_text_link`, the shape decision 1 already chose for
   `linked_narrow`, and it is carried through the IR snapshot.  What decision 2 decided is
   unchanged: the kind is static, there is no runtime branch, the stack kind is byte-identical,
   the store kind reads through the field read and writes through the field's setter, and a
   cross-kind bind is refused.  Realised in the PARSER: every mention of a store-kind link is
   spelled as the field it names (`OpGetText(OpVarRef(t), 0)`), so the emitters add only the
   link's `DbRef` declaration and bind.  Rejected alongside the two above: a runtime-kind link
   (a `TextLink` naming a `String` or a slot, a branch on every access, the slot's text written
   back at the end of each statement) — built on this branch as WIP for loft#1566 and reverted
   (ed429181d) for being the first rejected option in another spelling.
   **A per-variable fact is only as durable as the CODEC**: a warm program-cache load is a
   second decoder of the same state, and `linked_narrow` had no codec field — a warm run read a
   linked `i8` holding `-7` as `-135` (loft#1650).  A new per-variable flag owes a snapshot field
   and a cold-vs-warm cell in the commit that introduces it.
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

## Review cases (loft3-ff, 2026-09-22) — each one is a cell its phase owes

A sibling stream read the plan and sent eleven cases, the first two measured.  Each is
recorded here with the phase that must carry it as a cell, so none depends on memory.

| # | case | phase | status |
|---|---|---|---|
| R0 | **Two `&mut` to one place (loft#1605).**  Native passes a `&τ` argument as `&mut τ`: `add2(x, x)` is E0499 today (loud), and once B1 admits a store place, `add2(p.n, p.n)` or `add2(v[i], v[j])` with `i == j` compiles to two `&mut` from raw addresses — UB under `noalias`.  B2's proposed `&mut *var_c` makes the same aliasing reachable from a link and its target (`add2(c, x)` with `c = &x`).  Cure: a scalar `&τ` parameter is `*mut τ` on native, the type a `&` local link already has — so a link, a local and a place are all passed as one pointer and aliasing is defined.  Cells: the same local twice, the same field twice, `v[i]`/`v[j]` with `i == j` at run time, a link and its target together, `brighten(p.r, p)` reading the place by another route inside the call. | **B0** (new, before B1) | Open |
| R1 | `limit(lo, hi)` with `lo != 0` is width AND offset: `type Lim = integer limit(1000, 1100)`, `c = &q; c = 1050; c = &o.l; c = 1020`, then read `q` and `o.l` directly; also `limit(-100, 100) size(1)`. | A1–A3 | answered by P0 (the local holds the FIELD encoding); the cell is owed by A3 |
| R2 | The range promise through a link, which `range_arith`'s `type_range` relies on for plain operators: `fn set(c: &u8, k: integer) { c = k }`, `set(x, 300)`, then `x * 2` in the caller. | A0 | closed by A0: the write is refused at compile time; a compound step takes the slot default |
| R3 | A store-text read fed into a write to the SAME store in one statement (`t = &o.a`: `o.b = t`, `o.a = t`, `t = t`, `t += t`): the setter's claim can move the store under the `&str` mid-copy, and on native a `&str` from `stores` alive across `store_mut` only borrow-checks if raw.  Copy to a `String` first when the destination text is in the same store, or claim before reading. | C1 | Open |
| R4 | `(B-Disturb)`'s "overwriting a place is not disturbing it" is wrong for a BORROWED text read: `o.a = "…"` frees the old string record, so a `&str` taken through a store link and still alive (a W2 `text_borrowed` walk, `for c in t { o.a = … }`) dangles.  A write to the linked text place, through the link or directly, ends every borrowed read of it — a `binding.md` clause, not only a check. | C2, D | **Answered** (C2, 2026-09-24): measured no dangle on either backend under `LOFT_POISON` + `LOFT_STRICT_STORES` — `(I-Text)` binds a walk's source once, so the body's write cannot reach what is walked and no clause is owed. `loop-source-written` now also reaches a write through the link (`t = …` inside `for c in t`); writing the field by its own name while walking the link stays unreported, the lint's documented lower bound |
| R5 | Per-kind instantiation holes: (a) a fn-ref `g = app; g(o.s); g(local)` — one fn-ref type, and a `CallRef` cannot pick an instance by kind (refuse, or carry the kind in `fn(&text)`); (b) a library's `pub fn app(t: &text)` in a prebuilt cdylib has only the stack instance (PLACEMENT.md); (c) `__retbuf` stays the stack kind; (d) a recursive `&text` function calling itself with the other kind — the instances close transitively. | C3 | Open |
| R6 | Cross-kind in forms other than a sequential re-point: a join `t = if c { &a } else { &o.s }`, and a `&text` parameter re-pointed inside the store instance to one of its locals.  Both need the named refusal. | C1 | Open |
| R7 | Null against empty through a store link: a null text field read as `t == null`, and `t = null` written through the link; can the stack kind (a `String`) hold the distinction?  A parity cell. | C1 | Open |
| R8 | `u8` against `u8?`: is a `&u8` bound or re-pointed to a `u8?` place one `τ` under `(B-Ref-Repoint)`?  (`u8?` is 1 byte with range `0..=254`, `ByteNullable`; `u8` is `Byte` — same width, different null code.) | A3 | Open |
| R9 | The context axis lacks a closure capture and a generator frame: A1 changes the capture box's type and a coroutine frame's field types for a narrow local.  Cells: a `u8` captured by a lambda and also linked; a `u8` live across a `yield`. | A1, A2 | Open |
| R10 | B1 and the callee clause: `bump(v[i], v)` where `bump` appends to `v` must be refused (`(B-Ref-Reshape)` already names `f(v[i], v)`), so the new `&` argument enters `disturbed_params_map` as a plain parameter does; also `bump(v[-1])`, `bump(v[10])` on a length-3 vector (raise parity), and `bump(h[k].n)` with the key absent. | B1 | Open |
| R11 | Overwrite after an absent bind: `c = &o.inner.n` while `o.inner` is a null nullable record, then `o.inner = Inner { n: 1 }`, then read `c` and write through it.  `(B-Disturb)` says overwriting a place does not disturb it and `(B-Ref-Read)` says the link reads the source's CURRENT value (1), but both backends keep the absence resolved at the bind (a `rec == 0` `DbRef`, a null pointer) and answer null — agreeing, and both off the rule.  Either the rule gains "a link bound to an absent place stays absent" (the owner's call) or the bind refuses an absent place it cannot re-resolve. | rule question → D | Open |
| R12 | Diagnostic parity for an absent link: a direct `v[i] = x` / `v[i]` with `i` possibly out of range carries the fault-site / may-be-null diagnostic; `c = &v[i]; c = x` and `bump(v[i])` must say as much, or B4's dropped write is silent where the direct spelling speaks (B1 warns only for a non-null `&τ` parameter). | B4 follow-up | Open |
| R13 | Growth while an absent link is live: `c = &v[10]`, `v += [...]` to 11 elements, read `c` — `(B-Ref-Reshape)` must refuse the growth, and an absent link must not read as "no container" to the walk; also `c = &v[-1]` on an EMPTY vector, then `v += [x]`. | B4 follow-up | Open |
| R14 | **Spare-code types** (found by the switch, 157's c8): a non-null `limit(-100, 100) size(1)` or `limit(-1000, 1000) size(2)` LOCAL answers null after an overflow (C85), and a FIELD of the type answers the sentinel as a value — `0` and `64535` — on `main`, both backends (loft#1615). A1 gives the linked local the local's answer (two spare-code kinds; a spare-byte link reads and writes through the nullable pair, since `OpSetByte`'s `as i32` turns `i64::MIN` into `0`; `OpGetShortSpare` decodes the two-byte code). A3's `&` to such a FIELD will read the field's wrong answer until loft#1615 is fixed; take it before A3, or A3's cell for this type fails against C85. | A3, loft#1615 | **Closed** 22feb4e2f — `data::NarrowSlot::of_slot` is the one home for which read and write ops a store PLACE takes and with which `min`, asked by `Parser::get_val` and `Parser::set_field_check` alike, so the field now answers what the local answers. Guard `1615-every-narrow-slot-answers-the-types-default-for-an-unfitting-value.loft`. **Superseded 2026-09-23 by C127**: such a type has no null at all — a value that does not fit takes the type's DEFAULT, uniformly in every slot — so the spare-code encodings are retired and the guard pins the default instead. Two claims in this row were also wrong: a spare-code type IS reachable as a narrow-VECTOR element (`narrow_vec` asks about the FIELD's type, not the element's, so `vector<Spare8>` is one), and the `debug_assert` written on the opposite belief fired under `-C debug-assertions=on` |
| R15 | **A wide bias** (found while measuring R14's domain): every 1- and 2-byte store op took its type's minimum as `const i16`, so a narrow type whose minimum falls outside i16 — `limit(40000, 40100) size(1)`, `limit(-40000, 20000) size(2)` — had its bias truncated at EMISSION.  The interpreter read one wrong number for every value of the type and `--native` did not compile (loft#1620).  The two frame ops carry the same operand, so a LINKED narrow local of such a type SIGSEGV'd.  Widening it moved a fact five sites re-derive (how many bytes a constant operand occupies) plus one hand-rolled emission no ladder covers. | A1, A3, loft#1620 | **Closed** db6551ffb + f3ed908a4 — guard `1620-a-narrow-fields-bias-is-four-bytes-wide.loft` |

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **P0** — the width probe: hand-edit one emitted program so its linked `u8`/`i8` locals are at width and confirm a byte pointer reads and writes them, min/max/null included; list what the interpreter needs and size A2 | this file | the edited program prints the matrix's edge cells under rustc; the op inventory is written into A2's row | **Done** 2026-09-22 — § P0: the local holds the FIELD encoding; A2 = two fused frame ops |
| **A0** — loft#1604: the narrowing refusal and the range guard look through a link, so a write through `&u8` (a local link or a parameter) is checked like a write to the `u8` | loft#1604, `(B-Ref-Uniform)` | the issue's cell: `x = x + 10` through a link or parameter refused, `+=` takes the slot's default, both backends; falsified against P0's tip | **Done** e0cb48964 |
| **A1** — native: a LINKED narrow local's Rust type is its field encoding's storage type (`u8`/`u16`/`i32`/`u32`), decoded at each read, encoded at each write; a linked by-value parameter re-encoded once at entry by a shadowing `let` | decision 1, § P0 | the guard on both backends plain and under the switch; the corpus under `LOFT_LINK_ALL_NARROW=1` | **Done** 545243bdb + 1fbca93eb — `src/narrow.rs` is the one home for the bytes (both backends), `data::NarrowSlot` the one home for which kind a type takes, `Variables::linked_narrow_slot` the flag (set on pass 2 at the three `&` sites; no pass-1 record needed, the emitters run after the parse). Guard `167-a-linked-narrow-local-holds-its-field-encoding.loft`. The switch exercises every USER narrow local; a compiler temp is never linked and is spared. Two cells differ under the switch by design: `931b`'s "a local keeps the 8-byte slot" (an unlinked `i32` truncates to 4 bytes only under the switch) — recorded here so a sweep reads it as expected |
| **A2** — interpreter: `OpVarNarrow(pos, min, kind)` / `OpPutNarrow(pos, min, kind)` read and write a linked narrow local's 8-byte slot in the field encoding through `crate::narrow`; a linked by-value parameter re-encoded at entry; a link to a narrow place reads and writes through the kind's own field op (`NarrowSlot::get_op`/`set_op`) | decision 1, § P0 | value parity of the corpus across backends; the old local-link guard unmoved | **Done** with A1, and the FRAME READERS with it. A census of every non-codegen reader of a frame slot found four sites wrong and two right, measured rather than assumed: `render_frame_local` (behind `:vars`, the DAP and RPC surfaces, the browser debugger and the REPL's frame seed) rendered the stored CODE — `127` for an `i8` holding `-1`, `50` for a `limit(1000, 1100)` holding `1050`, `255` for a `u8?` holding null; `read_variable_value` carried the same into userland through `stack_trace()`, with no debugger attached; `set_frame_literal` / `set_frame_value` wrote a WIDE value the next read decoded, so typing `-5` resumed the run with `123`; and the bare-local watchpoint snapshotted eight bytes of a one-byte slot and reported `127 → 133 → 121` for `-1 → 5 → -7`. `eval_frame_reenter` and the REPL's expression evaluation were MEASURED and are correct untouched — the census predicted otherwise, and the probe is what settled it. `u8` and `u16` were right at every site throughout, their bias being zero, which is why each cell uses `i8`, `limit(1000, 1100)` or `u8?` as well. Guards: `tests/frame_readers.rs` (the four debugger entries, driving the real CLI over a pipe) and `tests/scripts/167-a-frame-reader-decodes-a-linked-narrow-local.loft` (the `stack_trace()` entry, both backends). `State::frame_narrow` is the one lookup for a reader that has a NAME, `FrameEntry::var_nr` for one that has the frame view. Two gaps recorded, neither narrow-specific: `read_variable_value` has no `Optional` arm at all (every nullable scalar reaches `stack_trace()` as `<unsupported>`), and the debugger silently discards an edit to any nullable local (loft#1629) |
| **A3** — retire `is_narrow_store_place`; `&u8` to a field or element links | decision 1 | `tests/scripts/1567-a-link-to-a-narrow-integer-store-place-reads-and-writes-it.loft`, both backends; `a-link-to-a-narrow-integer-store-place-is-refused.loft` retired; `D-bind-39` closed | **Done** — and A1/A2 had already done the representation half, so the work was finding TWO GATES WRITTEN TO THE OLD PREMISE, each citing `D-bind-39` by number and so reading as settled. (1) `Parser::scalar_place_ref` matched an `OpGet*` of exactly two arguments, and a narrow read carries a third (the `min` its encoding is biased by), so the place came back `None`, the `&` was silently dropped and the local was typed a plain `int` — which is why writes were "lost": there was no link, only a copy. (2) `set_var`'s re-point test read `byte_width(false) == 8`, so with (1) fixed a re-point fell through to the write-through path and wrote the 12-byte stack CELL through the kind's own `set_op` — the interpreter panicked in the store for an element and wrote the CONST store for a field, while native, which routes a re-point by the value's shape, was already right. Both refusal sites (the `&` bind and the `&` argument) ride the one fix; `is_narrow_store_place` is gone. 17 cells, both backends, neighbours and lengths read back in each.  **And lifting the refusal admitted a silent-wrong one layer down** (`D-bind-54`, 911f96983, found by loft3-ca on the wide and text spellings): `p = &v[0].f` was given the ELEMENT as its place — the field operand was not mis-offset, it was DROPPED, so two different fields of one element produced byte-identical IR and `&v[0].a` and `&v[0].b` both wrote `c`, while `&v[0].c` was right BY ACCIDENT because its offset is zero. The narrow spelling could not reach that path until this row lifted the refusal, so shipping A3 alone would have made a FIX introduce a silent-wrong — which is `(B-Ref-Reshape)`'s own objection to a link that cannot be honoured, so it was closed here rather than filed. The cure names the field unless its operand is literally zero, written once for reads of ANY arity, so a narrow read's third operand (the `min`) rides the same code instead of a second arm that would have to agree with the first forever. 28 more cells. **The lesson arc C owes:** B3's `&text`-place refusal is the same kind of lid, so measure what C1 ADMITS, not only what it builds |
| **B0** — loft#1605: a scalar `&τ` parameter is `*mut τ` on native, the type a `&` local link already has; a local argument passes `addr_of_mut!`, a link passes itself, a forwarded parameter passes itself | R0, loft#1605 | the R0 cells on both backends, native compiling each; `add2(x, x)` answers `12` on both; falsified against A0's tip | **Done** — `generation::is_raw_scalar_ref` is the one predicate; parameters take the local-link read, write and re-point arms. Found and fixed with it: a `&boolean?` link read the two-state `bool`, so `l == null` was never true (silent, on `main`), and a `&boolean?` parameter in `if b` / `b && …` did not compile — `infer_type` now reports a scalar link's read as the linked type. Guard `1605-one-place-reaches-two-ref-parameters.loft` |
| **B1** — a `&` parameter takes a scalar field or element (the bind's lowering at the call; `is_addressable` asks the place set `is_amp_place` asks) | decision 3 | `bump(p.n)`, `bump(v[i])`, `bump(o.inner.k)` write through, both backends; the refusal keeps a literal and a call result out | **Done** — `Parser::scalar_place_ref` is the one lowering, asked by the bind and by `convert`; the argument check admits a scalar place (it refused a narrow store place in the bind's words until A3 lifted both refusals together); the `(B-Ref-Reshape)` call-site half counts a scalar `&` parameter as one that names an element (R10); a possibly-absent element to a non-null `&τ` gets its own warning (the N-Store cure `?` would turn the place into a value). Guards `167-a-ref-parameter-links-a-scalar-field-or-element.loft`, `167-a-ref-parameter-refuses-a-place-it-cannot-link.loft` |
| **B2** — loft#1603: a `&`-bound scalar local passed to a `&` parameter compiles on native — after B0 the link IS the parameter's type and is passed as it is | loft#1603 | the issue's cell answers `6 11` on both backends | **Done with B0** — the issue's cell is in B0's guard |
| **B3** — loft#1602, the loud stopgap: a text FIELD or ELEMENT handed to a `&text` parameter is refused, naming the cure | loft#1602 | `app(o.s)` refused; falsified (both backends answered `cc` silently before) | **Done** — narrowed by measurement: a TEMPORARY keeps its read-only work copy, because the stdlib's `directory(v: &text = "")` and its two siblings are called with literals by design; only a place (`Parser::is_text_place`) is refused. Guard `1602-a-text-place-to-a-ref-text-parameter-is-refused.loft` |
| **B4** — a link to an ABSENT scalar place (`c = &v[10]`, `bump(v[10])`, a field of a null record) reads null and drops its writes on native as on the interpreter (C80: nothing stops a running calculation); native panicked in the allocator (`index 65535`), on `main` too for the bind | found in B1 (R10's raise-parity cell) | the bind and the argument cells answer alike on both backends: a read is null, a write lands nowhere, a later read is still null; no panic | **Done** — `output_place_pointer` yields a null pointer for an absent place; a scalar link's read answers `generation::absent_link_value` (each getter's `rec == 0` answer) and its write is dropped; a `&boolean` reads its storage byte in every case. No measurable cost on a hot `&` accumulator. Guard `167-a-link-to-an-absent-place-reads-null.loft` |
| **C1** — `RefVar(Text)` carries its kind; the bind `t = &o.s` / `t = &v[0]` makes the store kind; reads and writes through it on both backends; cross-kind re-point refused | decision 2 | `tests/scripts/167-a-text-link-into-a-store.loft`, both backends; the stack-kind emission of every text-returning guard byte-identical before/after (`--native-emit` diff); `D-bind-38` closed | **Done** — `Variable::store_text_link` (decision 2's amendment); the parser spells every mention as the field, `OpGetText(OpVarRef(t), 0)`, and binds the place `scalar_place_ref` gives; native declares the link a `DbRef`, the interpreter re-points it by `OpPutRef`. `scripts/introspect_diff.sh` before/after: 1 799 of 1 801 corpus files byte-identical, the two differing being the new guards. R3, R6 (refused, both orders and per path) and R7 answered in `167-a-text-link-into-a-store.loft` / `167-a-text-link-keeps-its-kind.loft`, both backends under `LOFT_POISON=1`. A store-kind link handed to a `&text` parameter stays refused (D-bind-55, C3). Asked on the union with loft#1651's retired-`__retbuf` fix (the next return-delivery change should know it was asked): `make ci` at 286a7cb80, 5 296/5 296, and zero stack-store free refusals (`BUG (#306)`) in the whole log — the two do not meet. |
| **C2** — the store-text slice joins the disturbance check | decision 2 | a cell that dangles under `LOFT_POISON=1` before the check is refused after; growth of another store stays allowed | **Done** — wider than the row: NO scalar or text place link reached `(B-Ref-Reshape)` (A3, B1 and C1 lower the `&` to a `RefVar` local, and the walk opened views only for `Reference`/`Enum`/`Vector` and refused only `is_amp_link`), so `c = &v[1]; v += [x]; c = 99` lost the write on both backends — `D-bind-56`. `scopes::is_place_link` at both readers, and `scopes::link_set_repoints` (the interpreter's re-point test, now shared) so a write through the link after the event is a USE rather than a re-bind. Guards `167-a-scalar-or-text-link-refuses-a-disturbance-of-its-container.loft` (12 cells) and `…-survives-what-does-not-disturb-it.loft`, both backends. R4 measured: no dangle — a text walk binds its source once |
| **C3** — a `&text` parameter instantiated per kind; `app(o.s)` links, `app(local)` unchanged | decision 2, @PLN165 | the aliasing cell reads the written value inside the callee; the stack instance byte-identical to C1's; loft#1602 closed | **Done** (2026-09-24) — `parser/store_text.rs`: the instance is a clone minted in `after_pass2` AFTER @PLN104's promotion (so it clones the final signature), keyed `n_f@st<mask>` (`@` is in no other key — `#` was, in a dispatch stub's `n_hit__dyn_E#E`, which the byte-identity diff caught), its `&text` parameters flagged `store_text_link`, reads → `OpGetText(OpVarRef(t), 0)`, the five write shapes → the twin op on one work text + `OpSetText`; calls whose `&text` argument is a place op other than `OpCreateStack` retargeted, transitively. K1–K12 all answered, both backends: K7 (fn-ref) refused — D-bind-57 / loft#1656, silent-wrong on `main`; K10 refused by the callee clause's new text spelling; K9 found D-bind-58 (a `&text?` parameter did not compile on native) and closed it; K12 `a_store_text_instance_reads_the_same_warm`. Guards `1602-a-ref-text-parameter-links-a-text-field-or-element.loft` (8 fns) and `…-refuses-what-it-cannot-link.loft`; loft#1602's stopgap guard retired into the first |
| **D** — formal + docs: `binding.md` gains the text-kind rule and closes `D-bind-38`/`39`; `DIAGNOSTICS.md` rows for the two new refusals; `CHANGELOG.md` | — | `rule_tags.py check` + `registers`; `check_doc_drift.sh` | **Narrow half DONE**; the text half travels with C3 (loft3-ca).  `D-bind-39` closed with A3.  `binding.md` needed no new rule — decision 1 gives a narrow link no kind, so `(B-Ref-Lvalue)` already described it — but the REPRESENTATION did: `layout.md` `(L-Narrow-Linked)` now states that a narrow local a `&` can reach holds the FIELD encoding, that the obligation runs to every READER of the slot and not to the emitters alone, and that `u8`/`u16` hide a broken one because their bias is zero.  Cited at its seven enforcing sites, so *"who enforces this?"* is a grep.  `DIAGNOSTICS.md`: no new narrow refusal exists — A0–A3 REMOVED one — but `narrow-fallback`'s subject widened, so its row now says the step reads through a link and through a `&` parameter.  `CHANGELOG.md` carries both user-visible halves: `&` reaching a narrow field or element, and the debugger no longer misreporting such a local.  The two refusals D's row names are both text-side (B3's message, C1's cross-kind re-point).  ⚠ The CHANGELOG entry SHOWS code, which is published code, so every line of it was run on both backends before it shipped — and three of the four things I first wrote were wrong, each the mistake a reader would make: a `&` parameter is called WITHOUT `&`, `p = p + 3` inside it is refused because `p + 3` is an `integer` (the compound step `p += 3` is the shape), and a link carries its target's type so a `&u8` cannot be re-pointed at an `i8`.  The entry now shows the spelling and names both refusals rather than leaving them to be discovered.  No new guard: `1567-a-link-to-a-narrow-integer-store-place-reads-and-writes-it.loft` already pins every one of those cells (28 asserts — the `u8` field, the `i8` element, the `&u8` parameter, the re-point, the `limit(1000, 1100)` field), so a second file would duplicate rather than cover |

## C3 — cases written before the build (loft3-ca, 2026-09-24)

Route (the plan's, confirmed by a census): the store instance is a CLONE of the stack
definition, minted in `after_pass2` beside @PLN104's targeted promotion (post-H5, so the
pass-1/pass-2 def count is untouched), with its `&text` parameter flagged
`store_text_link` and its body rewritten to C1's spelling; the calls that pass a store place
are retargeted to it.  Census over the 206 functions with a `&text` parameter in `default/`,
`tests/scripts` and `tests/docs`: the parameter is WRITTEN through exactly five shapes —
`Set` (449), `OpAppendStackText` (83), `OpFormatStackInt` (48), `OpFormatStackText` (26),
`OpClearStackText` (10) — and every other mention is a read (`OpConvBoolFromText`,
`OpGetTextSub`, `OpEqText`, `t_4text_*`, formatting, a `&text` argument, a `&` link).  A
closed mapping, so the rewrite is total: a read becomes `OpGetText(OpVarRef(t), 0)`, a write
the field's setter through one work text.  Which kind a call passes is read off the argument
(`OpCreateStack(x)` or a stack link/parameter is the stack kind; a place op or a store-kind
link/parameter is the store kind).

| # | case | expected |
|---|---|---|
| K1 | `app(o.s)`, `app(v[1])`, `app(v[1].s)` (a field at a non-zero offset) | the write lands in that place; neighbours untouched |
| K2 | aliasing: `peek(o.s, o)` writes `t` then reads `o.s` inside the callee | the callee reads its own write (no copy-in/write-back) |
| K3 | `app(l)` with `l = &o.s` | lands in `o.s` (lifts D-bind-55's link face) |
| K4 | forwarding `fwd(o.s)` where `fwd(t)` calls `app(t)` | `app`'s store instance is minted transitively |
| K5 | one function called with BOTH kinds | stack instance's emission byte-identical to today's |
| K6 | recursion `rec(o.s, n)` calling `rec(t, n-1)` | instance closes on itself (R5d) |
| K7 | fn-ref `g = app; g(o.s)` | refused, named (R5a: a `CallRef` cannot pick an instance) |
| K8 | a store instance whose body captures `t` in a closure or binds `u = &t` | measured; the answer is the rule's or a named refusal |
| K9 | a `text?` field passed and written null / tested `== null` | parity with the direct field (R7) |
| K10 | `grow(v[0], v)` where `grow` appends to `v` | refused by `(B-Ref-Reshape)`'s callee clause (R10) |
| K11 | a hidden text-return buffer (`__work_ret`) | stays the stack kind (R5c) |
| K12 | warm program cache | a warm run answers what a cold run does (loft#1650's lesson) |

## Phase ordering

1. P0, because it sizes A2 and can kill decision 1 for the cost of a compile.  Done.
2. A0 before everything: it is a silent-wrong on `main`, and A1's encode-at-write assumes every
   write to a narrow slot has already been range-checked.
3. B0 → B1 → B2 → B3 next: independent of A, smaller, and B3 stops loft#1602's silent loss.
   B0 comes first because B1 is what makes loft#1605's aliasing compile.
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

## Where the text kind can ride (for C1, measured 2026-09-22)

`Type::RefVar(Box<Type>)` is matched at **328** sites, 77 of them for text, so a second field
on `RefVar` would touch every one.  The precedent for a fact that must travel with a type
without changing its shape is `ConstParams` (`data.rs`): *"carried BESIDE the parameter types,
not as a wrapper on them — plan 40 rules out a `Type::Const`, because `Type` is matched in
hundreds of places."*  The text kind fits the same slot: `Type::Text(Deps)` already carries a
side structure that `Type::is_equal` ignores, so a store-kind flag inside the `Deps` of a linked
`Text` leaves every shape test, every `is_equal` and every `&text` match unchanged, and is
read only where the kind decides something — the two emitters' read and write arms, the
re-point refusal, and C3's instantiation key.  To be probed in C1 before it is built: whether
any site rebuilds a `Text`'s `Deps` from scratch (`Deps::none()`, `depending(v)`), which would
drop the flag silently.

**Answered by C1 (2026-09-23): it does** — `with_deps`, `without_deps`, `make_independent`, the
codec, and 56 fresh `Type::Text(Deps::none())` constructions.  The kind rides on the variable
instead (decision 2's amendment).

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
