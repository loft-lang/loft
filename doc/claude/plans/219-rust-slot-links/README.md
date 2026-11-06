<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 219 — Rust's `&mut` tree idioms in loft

## Status

Active.  The semantics of a link to an out-of-line field is settled and built (loft#2433, loft#2434,
C77 amended), and so is the parameter half (P, which absorbed P1): a `&` parameter of a type held
out of line takes a field and writes it.  The bench programs that expose user-built structures
(B) and the walk-or-write warning (W) are built, and the C routines over 3× Rust are priced by
hand (§ The C to 2×): seven rewrites to build.  Open: those, and § Also open.  Work lives on `tuxedo-quality`.

## Goal

A Rust programmer's tree and list code — the `&mut Option<Box<T>>` slot cursor and a slot handed
to a function — compiles in loft in its `&` spelling and warns when a walk forgot its `&`; and the
performance suite holds programs that build their own structures the natural way, so the portal
shows what they cost against Rust.  Making them fast is the ordinary performance loop
(`loft-optimize`), not this plan's.

## Why

loft ships its data structures (`vector`, `hash`, `index`, `sorted`, …) and gave a user little
room to build their own for a custom situation.  Field links and the textbook recursion fill most
of that gap: a user's own tree, trie or list is written directly, at a cost near Rust's.  What is
left of the gap is in § Also open — building a structure is still O(n²) in its two most natural
spellings, and a structure with shared nodes or back-links (a graph, parent pointers) mixes owned
children with `reference<T>`, which no cell or bench exercises yet.

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

**Slot parameters — the exact link (P, built).**  A `&` parameter whose type is held out of line
(`Data::slot_link_def`: `&S` / `&S?` with `S` on a size cycle) points at a SLOT CELL, as every `&`
parameter points at a cell: the caller's variable, or a cell of the caller's frame holding the
address of an out-of-line field with `keys::SLOT_FIELD_BIT` in its position (`OpSlotTag`).  A read
through the parameter answers the child a tagged cell names (`slot_read`; interpreter
`OpGetSlotRef`, C `loft_slot_read`, rustc `stores.slot_read`); a whole write is the field write
(`slot_write` → `set_child_rec`, an owned value given up there).  `slot = &slot.left` sets a cell
of the callee's own frame and re-points the parameter to it; the scope pass places that cell at
function scope.  A field link handed to such a parameter keeps its field (`SlotLinkFacts::passed`,
also set for a callee pass 1 cannot resolve yet) and re-reads it after the call.  The call
disturbs the field it hands over as a write that may be null (`released_children`).  The test
lives only in those parameters (C77's cost clause).  Copy-in, copy-out at the caller was
rejected: the write would land at the return rather than when it is made.

**The walk-or-write warning (W, built).**  A whole write `cur = cur.f` through a field link or a
slot parameter, inside a loop the link was bound outside of, that discards a sibling subtree
not known to be null (`walk-or-write`, DIAGNOSTICS.md).  The cures it names: `cur =
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
| `&` parameter given a field, a field link (0, 1, 2 iterations), a forwarder, a whole variable; null on entry; insert walks; a callee declared later | green, three backends — `calls/F-ParamRef/a-slot-parameter-writes-the-field-it-is-handed` |
| a view live across a call that unlinks through a `&` parameter (the child, below it, through a link) | green, same cell |
| a `&` variable drained to null by its callee (`while u != null { take_min(u) }`) | green, three backends, leak gate — the same suite cell: a record write-back releases the displaced store unless the new value lives in it, borrowed value or owned |
| forgotten `&` in a walk | warns `walk-or-write` — `tests/scripts/a-forgotten-amp-in-a-walk-warns-walk-or-write.loft`, exact set pinned in `tests/dead_code_lint.rs` |
| unlink cost | O(subtree) copy (M) |

## Phases

Each phase names the comparison that can make it go red.  In order:

- **B — built.**  21_tree gained `build_rec`, `enum_list`, `lookup_copy` and its `take_min`
  helper; 22_expr and 23_lru are new.  Measured C against Rust (`stats.py --only 21,22,23`):
  `enum_list` 6273×, `lookup_copy` 1033×, `expr_simplify` 13.7×, `expr_build` 10.7×,
  `tree_delete` 3.9×, `lru_mixed` 3.5×; the rest within 2.3×.  Making them fast is the portal's.
- **P — built** (absorbed P1), and **W — built**: § Design.

The O(1) unlink (§ Design, M) is no longer a phase: it is the portal's work once 21_tree's delete
row asks for it — hand form first, the op on all three backends behind `LOFT_NO_MOVE_CHILD`.
Last measured, C against Rust: insert 1.83×, query 2.25×, delete 3.93×, list 4.14×.

## The C to 2× — hand forms and the rewrites they price

Every C routine over 3× its Rust twin was written by hand into the form the compiler should
emit, one mechanism per step, and timed against the twin (`hand/RESULT.md`: the method, the
step tables and the profiles; `hand/<routine>/` the forms as diffs of the emitted unit, kept to
compare the built rewrites with — `bash hand/compare.sh.txt <routine>`).  Seven of eight land under 2×
by hand; the eighth (`lru_mixed`) does not.

| routine | base | by hand | mechanisms (§ below) |
|---|--:|--:|---|
| tree_delete | 3.65× | 0.65× | 2, 3 |
| list_ops | 4.10× | 0.57× | 1, 4 |
| lookup_copy | 1405× | 1.24× | 1 |
| enum_list | 6798× | 0.58× | 1, 4 |
| build_rec | 3.14× | 1.93× (1.76× with 6) | 5, 6 |
| expr_build | 10.4× | 1.64× (1.43× with 6) | 5 |
| expr_simplify | 13.4× | 1.17× (0.93× with 6) | 5 |
| lru_mixed | 3.50× | 3.16× (2.95× with 6) | 7 |

The rewrites, each general — none names a routine — with the formal rule it extends or adds, the
conditions it must prove, and what it reaches beyond the bench.  The order is the price.

1. **A copied cursor that only walks is a view** — `(R-CopyView)`, a WALK clause.  Today the rule
   admits a local bound by B-Copy's copy and only READ, never rebound.  The clause admits rebinds
   to a child read of the local itself (`cur = cur.left`, `cur = cur.next`) and, the same way, a
   `match` payload bound from it that is only read or rebound into it (`Cons { v, next } =>
   cur = next`): a child of a view of the source is a view of the source.  Conditions: the rule's
   (1)–(3) over the whole loop — nothing in the live range writes, moves or frees the source's
   records (B-Disturb's events), the source closes over parameters or over a local not written
   there.  Price: `lookup_copy` 1405× → 1.24×, `list_ops` 4.0× → 2.3×, `enum_list`'s walks O(n²)
   → O(n).  Reach: the read-only cursor nearly every first lookup is written with, and every walk
   over a user's list or tree by `match`.
2. **The unlink moves** — `OpMoveChildRec(field, src_field, tp)`, the move op (M) of § Design, a
   new entry in `formal/rewrites.md` beside `(R-Demote)`.  `h.f = <a chain of out-of-line reads
   rooted at h.f's own child>` — `cur = cur.left` through a field link, `slot = slot.right`
   through a slot parameter (`loft_slot_move`) — detaches the child's record number, installs it
   in the field and releases only the replaced record; `OpSetChildRec` copied the surviving
   subtree into a fresh record, its claims too, then released the old one.  Condition: no view
   of the replaced child is live across the write — `(B-OutOfLine)` keeps that view reading the
   field's new value, which a move would not (the walk already knows those views).  Price:
   `tree_delete` 3.76× → 2.03×.  Reach: every delete from a user's tree or list.  Switch
   `LOFT_NO_MOVE_CHILD`.
3. **A literal written into an out-of-line field is built there** — `(R-InPlaceLiteral)`, an
   out-of-line clause.  Today the rule writes a literal into an inline field or an element and
   declines an out-of-line field, so `cur = Tree { k: k, left: null, right: null }` through a
   field link builds the literal in a fresh store, copies it in with `OpSetChildRec` and frees
   the store.  The clause builds it in the field's child record (`OpChildRec`, then the field
   writes); a present child is the existing in-place case.  Price: `tree_delete` 2.03× → 0.65×
   (one store per insert was the larger half); `tree_insert` takes the same path.  Reach: every
   insert into a user's tree, every `t.left = Tree { … }`.
4. **`(R-Demote)` emitted directly, and for enum variants** — the rule already moves the old
   record into a child; its emission goes through a temporary record (`loft_place_record`), two
   `loft_move_field`s and `loft_free_record_in`.  Direct: claim the child in the same store, copy
   the record's bytes (its handles move with them — same store, nothing re-claimed), write the
   child's number into the field.  And a VARIANT clause: `l = Cons { v, next: l }` for a
   struct-enum, where today a work store is built, the whole list copied into it and copied
   back.  Price: `list_ops` 2.28× → 0.57×; `enum_list`'s build O(n²) → O(n).  Reach: every list
   or stack a user grows at its front, struct or enum.
5. **A record-valued recursion builds in its caller's store** — the destination store, the
   `(O-Buffer)` question of § Also open answered: a new rule, `(R-HostStore)`, next to
   `(R-Destination)` (which binds a buffer to an element slot) and `(R-RetAdopt)` (vectors).
   A call whose record result becomes an out-of-line child — `t.left = insert_rec(t.left, k)`,
   `Add { l: gen(…), r: gen(…) }`, `a = simplify(l)` then a node over it — is handed a buffer
   naming the host's STORE with no record; the callee claims its result there, the caller ADOPTS
   it by writing its number into the field, a result the caller drops is released record by
   record (`OpSetChildRec`'s null branch, not a store free), and `return a` of such a result
   hands its reference back instead of copying it into the buffer.  Today every record result
   is a store of its own, and every composition copies the child subtree into the parent's
   store.  Conditions: the callee returns its argument or a record it built in the buffer
   (`write_back_call`'s `returns_borrowed_view` + the hidden buffer) — adopting by number any
   other record of the store would make one record a child twice; the field write recognises
   the argument handed back inline (one load and compare) rather than inside
   `loft_set_child_rec`.  Price (both with 6): `expr_build` 8.7× → 1.43×, `expr_simplify` 11.6× → 0.93×,
   `build_rec` 2.68× → 1.76×.  Reach: recursive construction of any user tree — the
   @PLN91 self-host's `enum Node` (`NdIf { cond, t, f }`, `NdSet { inner }`) built by its
   recursive-descent parser is exactly this shape.
6. **No thread-locals in a program that starts no thread** — an emission switch, not an IR
   rewrite: the C runtime keeps its store table and frame stack thread-local (`LOFT_TLS`), and on
   macOS every such access is a call (`_tlv_get_addr`, 13 % of `build_rec`'s profile).  A
   program with no `par`, no coroutine worker and no server thread emits `#define
   LOFT_NO_THREADS` (the runtime's existing switch) before the runtime.  Price: −10 % to −21 % on
   every routine here, macOS; Linux's thread-locals cost a register offset, so expect less
   there (unmeasured).  Home: `C_BACKEND.md`, with the whole-program fact the emitter asks.
7. **lru_mixed — the hash table, not a rewrite.**  Two general rewrites priced:
   `(R-TypedKeyed)`'s REMOVAL clause — `h[k] = null` probes once by the key and deletes from that
   bucket (`remove(find(k))` probed again by the record's digest): −12 %; and a lookup of the key
   the previous statement added, with no write between, answers the record the insert filed —
   a new clause of `(R-SameRead)`, across one statement: −6 %.  With 6 the routine is at 2.95×.
   What is left is the table's own cost per operation — the bucket names a record and the key is
   compared in it (two dependent loads where the twin's table holds the key), the insert
   digests the record through the generic key walker (`loft_ref_digest`; a typed insert clause
   of `(R-TypedKeyed)`), and every reference write marks its target for `(H-Removed)`
   (`loft_st_note_referenced`, 9 %) and every read tests it.  No hand form of those was written;
   each is a runtime change to price next, the key stored beside the tag in the bucket first.

Not a cost, measured: the per-call frame (`loft_frame_push`/`pop`, `build_rec` step 2: 0 %) and
inlining the reference accessors (`lru_mixed` step 3: 2.5 %).

## Also open from the same arc

The natural-code list this plan grew out of, outside its core:

- **R-Demote cells and census** — cells under `rules/rewrites/R-Demote/` (correctness and the
  efficiency scale); `scripts/rewrite_census.py --bless-unless-drop`.
- **The read-only cursor `cur = t`** — priced: § The C to 2×, rewrite 1.
- **Recursive construction `N { v, next: mk(n - 1) }`** and **enum lists `l = Cons { v, next:
  l }`** — O(n²); priced: § The C to 2×, rewrites 4 and 5.
- **list_ops at 3.91×** — the portal's, like every row B adds.
- **`h[k]!` makes a copy a reference cannot keep** — `push_front(c, c.nodes[k]!)` is refused
  ("would outlive the record it names … `_ncr_1`") while the nullable warning names `!` as the
  cure; `(B-View)` makes the `?`-discharged element read a view, and `!` should match it.  Found
  writing 23_lru, which uses the `if n != null` guard instead.
- **Strict-mode store count** — under `LOFT_STRICT_STORES=1`, `O-NoDiverge/1328`, `/1329` and
  `O-Proxy/1320` report "store table exhausted by the checking mode, not by a leak" on this
  branch and not on the installed 0e95172cd; none of them uses `&`.  Unattributed.
- **Before the PR** — `make ci`.

## Documentation

- `formal/binding.md` `(B-OutOfLine)`, `(B-Disturb)` — done, the `&` parameter on a field
  included.
- `formal/calls.md` `(F-ParamRef)` — a field place as the argument — done.
- `formal/rewrites.md` — the move rewrite's entry and the Applied-by table (M).
- `DIAGNOSTICS.md` — the walk-or-write code (W); `lost-write`'s fourth shape — done.
- `BOTH_BACKEND_SWITCHES.md` — `LOFT_NO_MOVE_CHILD` (M).
- `DESIGN_DECISIONS_OWNERSHIP.md` C77 — done.
- `doc/00-vs-rust.html` — the slot cursor beside Rust's (B).
- `bench/README.md` — rows 22 and 23, row 21's new routines; `bench/portal/routines.tsv` and
  `classes.tsv` (B).
- `RULE_CROSSINGS-history.md` — the verified pairs.
- `CHANGELOG.md` — the user-visible change: tree code in Rust's shape.

## See also

- loft#2433, loft#2434 — the two defects this arc fixed.
- [`DESIGN_DECISIONS_OWNERSHIP.md` § C77](../../DESIGN_DECISIONS_OWNERSHIP.md).
- [`formal/binding.md`](../../formal/binding.md) — `(B-Ref-Write)`, `(B-OutOfLine)`,
  `(B-Disturb)`, `(B-Ref-Reshape)`.
- `bench/21_tree/` — the routines and their Rust twin.
