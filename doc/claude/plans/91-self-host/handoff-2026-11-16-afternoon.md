<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-16 (afternoon): gate E at 851 of 2158, a fast loop for the long tail

Branch `tuxedo-pln204-int32`, pushed; PR #2518 (loft3's gather) carries it up to 77024883f
(819 of 2151) and re-wrote gate E's baseline at ITS suite pin 898bc05 — after it merges, rebase
and re-write the baseline at the pin main then has.  Goal (owner, /goal): the full loft
interpreter, the in-between data of the two implementations compared at every layer.

## Where it stands (suite pin 508580956)

| Layer | Gate | Position |
|---|---|---|
| front end (gate E) | `selfhost/front/elab_gate.py` | **851 of 2158 agree, 1307 refused, 0 differ** (818 of the 2125 outside the cells; 765 at the day's start); baseline committed per code commit |
| writer / runner | `bgen_survey.py`, `run_image_survey.py` | as `handoff-2026-11-16-day.md`; their re-pinned baselines are still uncommitted in `../loft2-repin` |

The corpus is 2143 because the gate has a part of its own now: `selfhost/front/cells/*.loft`
(33 cells, all agreeing) — one shape per cell that the corpus does not hold ALONE.

## What gate E's remaining 1342 are

A long tail, not a few families: 301 programs are blocked by ONE refusal each, and those
are almost all different shapes (`--plan` admits five or six programs per family at best).
The measured rate with the loop below is about ten programs an hour.

**A lifted refusal uncovers differences.**  A program refused for one construct was never
compared in its other functions.  Of the eleven refusals lifted this session, most uncovered
one or more constructs the port had answered WRONGLY without refusing — a line marker, a
borrow list, a block type, a row order.  So after every batch the whole corpus is run again:
`differ` must be 0 and no `agree` may drop before the commit.  A difference found this way is
either ported (the usual case — each was one Rust function) or refused by name.

## The loop (`selfhost/front/elab_probe.py one | where | fam` carries the three probe scripts)

- **A development build of the elaborator** — `loft --native-c --out <dir>
  selfhost/front/elab.loft` (no `--ship`): about 7 minutes at load 4, half the optimised one.
- **One program, both reports diffed** (`one.sh`): `LOFT_EMIT_PARSED=<img> loft <f>`; `defs <img>
  program <std>` against `<elab> <img> <f> <std>`.  `defs`, `std.store` come from a
  `LOFT_ELAB_KEEP=1` gate run.
- **`LOFT_ELAB_WHERE=1`** (new, `elab.loft`): the source LINE each refusal was first met on, to
  stderr.  `where.sh` prints that line; `fam.sh "<refusal text>"` collects the lines behind one
  refusal over the corpus — the first thing to run on a family.
- **`LOFT_ELAB_DEFS=<exe> LOFT_ELAB_BIN=<exe>`** (new, `elab_gate.py`): a corpus run with tools
  built before — about four minutes.  `--check` / `--write-baseline` refuse them and build their
  own (about 25 minutes): that run is made once per commit, after it.
- **`LOFT_ELAB_VERDICTS=<file>`** per run; two files diffed name the verdict changes.
- **`LOFT_TRACE_MINT=1`** on the Rust side names the call chain that minted a store type — it
  settled a row-order question in one run where three probes had not.

Order of a batch: `fam.sh` → a probe in `scratchpad/p/` hand-checked against `loft --interpret`
→ Rust's report read for the shape → the Rust function read (never guessed) → the port →
development build → probe → corpus run → cell → commit → real gate → baseline commit.

**Traps met.**  The gate's own build reads `selfhost/src` in its first two minutes: no edit
there until its `loft --native-c --ship` process is older than that.  A loft local bound to
a function's answer can be a VIEW of the argument (`tail = el_ir::unspan(bl.operators[-1])`):
read what you need from it BEFORE the statement it views is rewritten.  The first pass often
answers `NdNull` with a type — a test of the CODE's shape there must ask the syntax instead.

## Done this session (commits 45d19cd75 … d874c229c)

Admitted: a vector parameter rebound by a literal and from a vector; a struct-enum parameter
rebound; `&text` written whole; a record into an element at an unproven index; `return null`
from a text function; formatted variant records and vectors of records; a field or element
handed to a `&` parameter; a vector `if` tail answering parameters; `!` (postfix) on a struct
enum; a literal rebinding a local it reads.

Rust rules ported because a lifted refusal showed the port differing: `@FR-E-Left`
(`fix_place_before_value`); the block-closing line rule (only a `;`-ended last statement moves
the line to the `}`); `parse_var`'s `into` arm asked of the exact type; a view answered through
`x?` published on the first pass; `vector<T>` definitions at a parameter's body entry and at a
declared comprehension; format-hole rows on both passes; `c ?? Rec {..}` borrowing what its
subject's read borrows; a `-> E?` body over a variant tail; a `w!` match subject's borrow.

## Later the same session (commits 56f990798 … fc10743c3)

`null` into a declared local of any type; prefix `!` on a record and a struct enum; `return
null` and a literal tail for `vector<T>?`, `return null` for a struct-enum `E?`; a tuple bound
to a declared tuple of other scalar member types (`tuple_convert`); `vector<integer?>`,
`vector<float?>`, `vector<boolean?>` (stored as the scalar's vector — the base
`vector<float>` row on the first pass, its definition on the second); a keyed local returned,
a keyed collection bound from a call with scalar arguments, a keyed literal at a tail
(`__kvb_N`); `destructure_constant_member`.

## Open, in order

1. **Out-of-line children** (`@FR-L-OutOfLine`) are carried in their first increments — layout
   (`child_rec<S>`, struct and struct enum), read, literal child, whole write, the demote
   (`h = N { next: h }`).  Still refused by name: `move_child` (`t.left = t.left.right`),
   `write_back_call` (`t.left = insert(t.left, k)`), a `&` to a child, a child given a value
   that is no literal, local or fresh call.  The notes below are the probe that started it.
   (Before: the largest family, 75
   programs meet "a type whose fields contain it again", 64 "an omitted field of this type").
   Rust's shape, from a probe: the field's store type is `child_rec<Tree>` (`ChildRec(100)`,
   4 bytes); a read is `OpRefFromChildRec(OpGetField(t, pos, type))`; an omitted child is
   `OpSetChildRec(field, OpNullRefSentinel(), type)`; a literal's FIRST child literal is built
   in place (`__ref_p2_N = OpChildRecInPlace(field, type)`, then its fields), a later one apart
   in an "Object" block handed to `OpSetChildRec`.  Read `typedef.rs` / `objects.rs` for which
   child is built in place before porting.
2. Next by blocked programs: a vector literal mixing struct literals and other items (33), a
   slice of a vector that is no local of plain scalars (27), an index that is a range, a key
   list or a limit (20), `vector<S?>` / `vector<text?>` / `vector<u8?>` (41, each its own
   layout), a keyed bind from a call with a record argument (`protectable_ref_args`).
3. **A loft language defect met in the port itself: loft#2520** (filed, not fixed).  In
   `el_ex_place::fields_at`, a `match ol! { .. }` over a `ir::Node?` local bound from
   `out_of_line_init`, in a branch that never runs, corrupts the sibling local `at`: the
   interpreter faults in three seconds, the C backend answers wrong.  `on = ol!; match on`
   and a helper function (`el_ir::empty_insert`) are both fine; the issue holds the boundary
   table and the recipe.  No standalone reduction yet — the elaborator is the repro.  Until
   it is fixed, do not `match` directly over a `x!` unwrap in the port.
4. Known differences OUTSIDE the corpus, found by probes and not yet fixed: `R { c: [] }` with
   `c: vector<boolean>` mints the `vector<boolean>` row on the first pass where Rust does not
   (`LOFT_TRACE_MINT` on both sides will name it).
   Also: `q = same(P { a: 2 })` with `fn same(p: P) -> P { p }` — Rust types `q` without the
   literal's own `__ref_p2_N`, the port lends it (the buffered-call arm of `call_dependencies`).
3. Families probed and left: a `text?` field or element as a `&text` argument (Rust calls a
   store-text twin `n_f@st1`); `vv[0] += [9]` on a vector of vectors (does Rust mint `_elm_N`
   there?); a vector tail that is a recursive call arm, `return q ?? [..]`, `return a + [b]`;
   `text? + text` (an "Add text" block under `OpConvBoolFromText`); a record literal written
   into a field or an element (four sub-refusals, each its own shape).
4. Everything in `handoff-2026-11-16-day.md` § Open that is not above (the writer/runner
   baselines, `store_records.rs` over the ceiling).
