<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 219 — Rust's `&mut` tree idioms in loft

## Status

Active.  The semantics of a link to an out-of-line field is settled and built (loft#2433, loft#2434,
C77 amended); a regression of that build on `&` parameters, the parameter half, the
walk-or-write warning and the O(1) unlink are open.  Work lives on `tuxedo-quality`.

## Goal

A Rust programmer's tree and list code — the `&mut Option<Box<T>>` slot cursor and a slot handed
to a function — compiles in loft in its `&` spelling, warns when a walk forgot its `&`, and costs
what Rust's costs.

## Why

The owner's ruling (C77, amended 2026-11-06): Rust programmers must feel at home in loft, whose
syntax stays close to Rust's even where the semantics differ.  So the shapes a Rust programmer
writes with `&mut` are admitted ahead of C77's narrowing of `&`.  The rules already said so for
every other kind of place: `(B-Ref-Lvalue)` names "variable / field / element", and
`inc(o.y)`, `inc(w[1])`, `grow(o.v)`, `rename(o.s)`, `reset(o.inner)` all pass a place to a `&`
parameter today.  Only the out-of-line field was the exception.

## The two spellings side by side

| Rust | loft |
|---|---|
| `&mut Option<Box<Tree>>` | `&Tree?` |
| `slot = &mut n.left` (re-point) | `cur = &cur.left` |
| `*slot = x` / `slot.take()` | `cur = x` (no deref, no take) |
| `slot.as_mut().unwrap()`, `is_some_and(…)` | `cur.left != null`, `cur.k` |
| `take_min(&mut node.right)` | `take_min(cur.right)` |
| `Some(n) if n.k == k => break` | `while cur != null && cur.k != k` |

loft's version is about a third shorter and reads as the textbook algorithm.  Rustc is more
precise in three places, each with its answer here:

1. **Walk versus write.**  Rust's types separate `slot = &mut …` from `*slot = …`; loft has no
   deref (`(B-Ref-AnnotationOnly)`), so `cur = &cur.left` walks and `cur = cur.left` unlinks.  A
   lookup that forgot one `&` unlinked three nodes of eight and answered `true`, silently, on
   both backends.  Answer: the walk-or-write warning (W).
2. **The call site.**  `take_min(cur.right)` does not show the write that `&mut` shows.  True of
   every `&` parameter in loft and a deliberate choice; an editor inlay hint is the place for it,
   not this plan.
3. **Cost.**  Rust's `take()` is an O(1) move; loft's `cur = cur.right` copies the subtree into
   the slot and releases the old node.  Answer: the move op (M).

## Design

**Field links (built).**  A `&` local only ever bound to out-of-line fields and written whole
keeps the FIELD in a slot local set before each bind; reads see the child through it; a whole
write is the field write `h.f = v` (`OpSetChildRec` through the slot) and the link re-reads the
child.  Pass 1 records, per function and link name, what the link is bound to and whether it is
written whole (`Parser::slot_links`).  A link bound to a whole variable at one `&` bind and to a
field at another has no one place for a whole write and is refused with the cure in the message.
Rules: `(B-OutOfLine)`, `(B-Ref-Write)`.

**Disturbance through views (built, loft#2434).**  An event made through a view or a link is the
event on the place the view names (`ViewWalk::through_views`); `(B-Disturb)` says so.

**Slot parameters — the exact link (P).**  A `&` parameter whose type holds itself out of line
(`&Tree?`, `&Tree`) receives the ADDRESS of its place, as every `&` parameter does: a stack cell
for a variable, the field for an out-of-line field.  The two hold different things — a 12-byte
reference, a 4-byte record number — so a read or a whole write through such a parameter tests
which one it has.  The test lives only in those parameters; no other program pays for it (C77's
cost clause).  Inside the callee the parameter is a field link from its first line, so a
re-point (`slot = &slot.left`) and a whole write before or after it use one path.  Copy-in,
copy-out at the caller was rejected: the write would land at the return rather than when it is
made, which `inc(o.y)` does not do.

**The walk-or-write warning (W).**  A whole write `cur = cur.f` through a field link, inside a
loop, that discards a sibling subtree not known to be null.  The cures it names: `cur =
&cur.f` to step; `cur.g = null;` before the write to drop that subtree on purpose.

| shape | loop | discards a sibling | verdict |
|---|---|---|---|
| lookup or insert walk that forgot the `&` | yes | yes | warns |
| `remove_key`'s unlink after its walk | no | — | silent |
| `take_min`'s `m = m.right` after its loop | no | — | silent |
| a list run removed, `while cur != null && cur.v < 0 { cur = cur.next }` | yes | no | silent |
| BST trim dropping left subtrees on purpose | yes | yes | warns; `cur.left = null;` first clears it |
| a forgotten `&` on one step outside a loop | no | — | missed (indistinguishable from an unlink) |

**The O(1) unlink (M).**  `OpMoveChildRec(field, src_field, tp)`: detach the child at
`src_field` without releasing it, install its record in `field`, release the record `field`
held.  Emitted only where the source is provably inside the record being replaced: the value is
a chain of out-of-line reads rooted at the field's own child — `cur = cur.right` on a field link,
`h.f = h.f.g` on a field.  Anything else keeps `OpSetChildRec`'s copy.

## Composition matrix

Axes: the link (local field link · `&` parameter · mixed), the place (out-of-line field · whole
variable · inline field), the write (null · own child · call result · literal · after a clear),
the position (loop · straight line), another view or link live across the write, the backend.

| cell | state |
|---|---|
| local link: unlink, clear, two-link delete, fill an empty slot, call result, write after clear, link bound in a loop body, 19 999 inserts + 6 667 deletes | green both backends — `B-Ref-Write/2433-a-whole-write-through-a-link-…` |
| mixed variable/field link written whole | refused — `B-Ref-Write/2433-a-link-to-a-variable-and-a-field-…` |
| a view or link live across a write through another view or link | green — `B-Disturb/2433-a-write-through-a-view-or-link-…`; a second live `&` link is refused as for the direct write |
| `&` parameter given a slot link, loop runs ≥ 1 time (`take_min`) | answers right today |
| `&` parameter given a slot link, loop runs 0 times | **interp panics, C loses the write** (the regression, P1) |
| `&` parameter whole-written null on entry (`clear(n)`) | **silent: tree unchanged** (P) |
| `&` parameter given the field directly, `take_min(cur.right)` | refused today (P) |
| a view live across a call that unlinks through a `&` parameter | not built (P) |
| forgotten `&` in a walk | silent tree damage (W) |
| unlink cost | O(subtree) copy (M) |

## Phases

Each phase names the comparison that can make it go red.

- **P1 — the parameter regression** (S).  The slot rewrite fires on a `&` parameter and leaves the
  first write before any re-point with no field.  Interim fix: no slot rewrite on an argument, so
  the parameter keeps today's path.  Red: the zero-iteration `take_min` cell, both backends.
- **W — walk-or-write warning** (S).  The six rows above as cells of one fixture with the exact
  set of warned locals pinned (`tests/dead_code_lint.rs` style).  Red: the forgotten-`&` cell
  stays silent, or a silent row warns.
- **P — slot parameters, exact link** (M).  Probe first: the address test in a hand-written
  callee on both backends.  Then the caller side (field argument, slot-link argument, variable
  argument) and the callee side (field link from the first line).  Red: the four parameter
  cells above, the view-across-the-call cell, and the existing `&` parameter cells unchanged.
- **M — the move op** (M).  Hand form first (loft-optimize step 4): the detach-install-release
  sequence written into the generated C for bench 21's delete, timed against the copy; build
  only if the prize is real.  Then the op on all three backends, a `LOFT_NO_MOVE_CHILD` switch,
  `make surface-gen` and the wasm rlib.  Red: switch on and off answer alike, live-record counts
  equal, the bench delete moves.
- **B — bench 21 in the Rust shape** (S).  `take_min` restored as a helper so the loft and Rust
  sources match routine for routine; hashes unchanged (`d0fc3f4 829a302c 41ceb783 3b0656d6`);
  emitted C read for each routine; the row recorded.  Last measured (C against Rust):
  insert 1.76×, query 0.47×, delete 3.77×, list 3.91×.

## Also open from the same arc

The natural-code list this plan grew out of, outside its core:

- **R-Demote cells and census** — cells under `rules/rewrites/R-Demote/` (correctness and the
  efficiency scale); `scripts/rewrite_census.py --bless-unless-drop`.
- **The read-only cursor `cur = t`** — the spelling nearly everyone writes for a lookup copies
  the whole tree at the bind; elide the copy where nothing writes through the local.
- **Recursive construction `N { v, next: mk(n - 1) }`** — O(n²); a design question
  (`(O-Buffer)`).
- **Enum lists `l = Cons { v, next: l }`** — O(n²).
- **list_ops at 3.91×** — find the mechanism behind it.
- **Before the PR** — RULE_CROSSINGS ledger rows for the pairs these fixes crossed, `make
  check-rlib` (the wasm rlib was stale), `make ci`.

## Documentation

- `formal/binding.md` `(B-OutOfLine)`, `(B-Disturb)` — done; `(B-Ref-Write)` gains the `&`
  parameter on a field (P).
- `formal/calls.md` `(F-ParamRef)` — a field place as the argument (P).
- `formal/rewrites.md` — the move rewrite's entry and the Applied-by table (M).
- `DIAGNOSTICS.md` — the walk-or-write code (W); `lost-write`'s fourth shape — done.
- `BOTH_BACKEND_SWITCHES.md` — `LOFT_NO_MOVE_CHILD` (M).
- `DESIGN_DECISIONS_OWNERSHIP.md` C77 — done.
- `doc/00-vs-rust.html` — the slot cursor beside Rust's (B).
- `bench/README.md` row 21 — the Rust-shaped routines (B).
- `RULE_CROSSINGS-history.md` — the verified pairs.
- `CHANGELOG.md` — the user-visible change: tree code in Rust's shape.

## See also

- loft#2433, loft#2434 — the two defects this arc fixed.
- [`DESIGN_DECISIONS_OWNERSHIP.md` § C77](../DESIGN_DECISIONS_OWNERSHIP.md).
- [`formal/binding.md`](../formal/binding.md) — `(B-Ref-Write)`, `(B-OutOfLine)`,
  `(B-Disturb)`, `(B-Ref-Reshape)`.
- `bench/21_tree/` — the routines and their Rust twin.
