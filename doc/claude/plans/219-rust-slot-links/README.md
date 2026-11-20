<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 219 — Rust's `&mut` tree idioms in loft

Tracker: [@PLN219](https://github.com/loft-lang/plans/issues/219).

## Status

**SHIPPED 2026-11-20.**  A Rust programmer's tree and list code — the `&mut Option<Box<T>>` slot
cursor and a slot handed to a function — compiles in loft in its `&` spelling, a walk that forgot
its `&` is warned about, and the performance suite holds programs that build their own structures
the natural way (`bench/21_tree`, `22_expr`, `23_lru`) against Rust.  Owner ruling C77, amended
2026-11-06: Rust programmers must feel at home in loft.

This file is the closure record.  How it works today is in:

| what | home |
|---|---|
| a `&` link to an out-of-line field, and a write through it | [formal/binding.md](../../formal/binding.md) `(B-OutOfLine)`, `(B-Ref-Write)`, `(B-Disturb)` |
| a `&` parameter handed a field (the slot parameter) | [formal/calls.md](../../formal/calls.md) `(F-ParamRef)` |
| the two spellings side by side, as a program that runs | `doc/00-vs-rust.html#slots`, `tests/comparisons/slots.loft` |
| the `walk-or-write` warning and its two cures | [DIAGNOSTICS.md](../../DIAGNOSTICS.md) |
| the rewrites that make such code cost what Rust's does | [formal/rewrites.md](../../formal/rewrites.md): `(R-MoveChild)`, `(R-HostStore)`, `(R-NoThreads)`, `(R-ExpPow)`, and the clauses of `(R-CopyView)`, `(R-InPlaceLiteral)`, `(R-Demote)`, `(R-Range)`, `(R-ForwardWalk)`, `(R-Counter)`, `(R-PushFill)`, `(R-TypedKeyed)`, `(R-SameRead)`, `(R-GuardedChain)`, `(R-Version)` |
| their bisect switches | [BOTH_BACKEND_SWITCHES.md § Layout: out-of-line fields](../../BOTH_BACKEND_SWITCHES.md#layout-out-of-line-fields); the C-only ones with their rules in `formal/rewrites.md` |
| the decision | [DESIGN_DECISIONS_OWNERSHIP.md § C77](../../DESIGN_DECISIONS_OWNERSHIP.md) |
| the bench rows | [bench/README.md](../../../../bench/README.md) rows 21 to 23 |

## What shipped

| phase | outcome | guard |
|---|---|---|
| field links (loft#2433) | a `&` local bound only to out-of-line fields keeps the FIELD; a whole write is the field write; a link bound to a variable at one bind and a field at another is refused with its cure | `binding/B-Ref-Write/2433-…` |
| disturbance through views (loft#2434) | an event made through a view or a link is the event on the place it names | `binding/B-Disturb/2433-…` |
| P — slot parameters | a `&` parameter of a type held out of line points at a slot cell and writes the field when the write is made, on three backends | `calls/F-ParamRef/a-slot-parameter-writes-the-field-it-is-handed` |
| W — walk-or-write | a whole write `cur = cur.f` inside a loop the link was bound outside of, discarding a sibling subtree not known to be null, warns | `tests/scripts/a-forgotten-amp-in-a-walk-warns-walk-or-write.loft`, pinned in `tests/dead_code_lint.rs` |
| B — the bench programs | 21_tree's `build_rec`, `enum_list`, `lookup_copy`; 22_expr; 23_lru | the portal |
| M — the O(1) unlink | `(R-MoveChild)`: the surviving child is installed by its number | `binding/B-OutOfLine/an-unlink-moves-and-a-literal-is-built-in-its-child` |

One shape the warning cannot see: a forgotten `&` on a single step outside a loop reads the same
as an unlink.

## The C against Rust — what the hand forms priced and the compiler now emits

Every routine over the bar was first written by hand into the form the compiler should emit,
one mechanism per step, and timed against its Rust twin ([hand/RESULT.md](hand/RESULT.md): the
method, the step tables and the profiles; `hand/<routine>/` the forms as diffs of the emitted
unit; `bash hand/compare.sh.txt <routine>`).  Each mechanism then became a general rewrite with
a formal rule — none names a routine.

**Built, and what the compiler now emits** (median of 7, C ÷ Rust, the same drivers and flags;
each emitted unit checked for the hand form's shape — `loft_demote_rec`, `loft_move_child_rec` /
`loft_slot_move`, `loft_child_rec_in_place`, `loft_store_buf` + `loft_adopt_child_rec`,
`loft_release_rec_if_distinct` for a dropped result, no
`loft_bind_copy` of a walk cursor, `#define LOFT_NO_THREADS`):

| rewrite | rule | switch | guard |
|---|---|---|---|
| 6 | `(R-NoThreads)` | `LOFT_NO_THREAD_FACT` | `tests/c_emit.rs` `a_unit_that_starts_no_thread_keeps_no_thread_locals` |
| 4, direct | `(R-Demote)` as `OpDemoteRec` | `LOFT_NO_DIRECT_DEMOTE` | `rewrites/R-Demote/a-front-push-moves-the-old-node-whole-a-records-or-a-variants` |
| 1 | `(R-CopyView)` walk clause | `LOFT_NO_COPY_VIEW_WALK` | `rewrites/R-CopyView/a-cursor-that-only-walks-is-a-view` |
| 2 | `(R-MoveChild)` | `LOFT_NO_MOVE_CHILD` | `binding/B-OutOfLine/an-unlink-moves-and-a-literal-is-built-in-its-child` |
| 3 | `(R-InPlaceLiteral)` out-of-line clause | `LOFT_NO_CHILD_LITERAL` | the same cell |
| 5, write-back half | `(R-HostStore)` | `LOFT_NO_HOST_STORE` | `rewrites/R-HostStore/a-recursion-builds-its-result-in-the-field-it-is-written-to` |
| 5, callee half (no drop) | `(R-HostStore)` literal clause, `(R-Place)` branch exits, `(R-InPlaceLiteral)` variant clause | `LOFT_NO_HOST_STORE`, `LOFT_NO_VARIANT_INPLACE` | `rewrites/R-HostStore/a-result-built-in-its-hosts-store-is-adopted-and-released-once` |
| 5, the release by record | `(R-HostStore)` result clause | `LOFT_NO_HOST_RESULT` | `rewrites/R-HostStore/a-result-built-in-its-hosts-store-is-adopted-and-released-once` |
| 4, variant clause | `(R-Demote)` variant clause | `LOFT_NO_DEMOTE` | `rewrites/R-Demote/a-front-push-moves-the-old-node-whole-a-records-or-a-variants` |
| 1, `match` payload | `(R-CopyView)` walk clause, step locals | `LOFT_NO_COPY_VIEW_WALK` | `rewrites/R-CopyView/a-walk-over-a-list-is-a-view-of-the-list-it-was-bound-to` |

| routine | before | now | hand form |
|---|--:|--:|--:|
| tree_delete | 3.73× | **0.73×** | 0.58× |
| list_ops | 4.00× | **0.80×** | 0.57× |
| lookup_copy | 1340× | **0.97×** | 1.23× |
| build_rec | 3.16× | **1.94×** | 1.65× |
| expr_build | 10.4× | **0.68×** | 1.46× |
| expr_simplify | 13.3× | **1.04×** | 0.91× |
| lru_mixed | 3.53× | 3.21× | 3.05× |
| enum_list | ~6000× | **0.83×** | 0.56× |

**Which lane these tables are.**  The judged number is the C SHIP lane (`cship`) ÷ the Rust
twin, per routine, on every core and library bench (PERFORMANCE.md § What is judged).  The
tables in this section were taken on the CHECKED lane (`c`, clang, Apple Silicon), which keeps
every fault check the ship lane drops, so they overstate the judged gap — most where a routine is
integer arithmetic under checks (graphics `blend_pixel` 3.88× checked, 1.18× ship).  The
rewrites lower both lanes.

**The routines over 2.7× (checked C ÷ Rust twin, core and library benches).**  Each was written by
hand first; the general change each form priced, by reach:

| routine | base | hand | mechanism → general change | status |
|---|--:|--:|---|---|
| sort | 6.70× | 1.45× | every held element access tested its index twice → one test (runtime); a counter stepped down once per trip of `for _ in 0..x` from `x` is ≥ 1 in the body → `(R-Range)` relational step + index clauses (`_nn` accessors) | **built: 1.44×** |
| cbor decode | 12.3× | 1.88× | `(R-CSplit)` splits a hot recursive function for MSVC's sake on every compiler → split only under `_MSC_VER`; a parked text buffer taken on every call → take it on the path that writes it | **split built: 2.13×**; the park open |
| hex_field doc_read | 8.70× | 2.01× | a float read per call through `fread` → `(R-TypedRead)` float clause; several zero-push loops → `(R-PushFill)` several-vector clause; `x.f = ctor(…)` minted + copied → `(R-Place)` guarded-mint clause | **float clause built: 3.84×**; the rest open |
| graphics clear | 6.96× | 1.97× | the bulk fill stores one `memcpy` per element, which clang does not vectorise → typed-store fill (runtime, `loft_fill8`) | **built: 1.81×** (fill_circle 1.50×, fill_star 1.20×) |
| drawing render_marks, resize | 7.09×, 6.38× | 2.09×, 2.33× | `(R-GuardedChain)`'s plain copy is identical to the checked one (the held read's index is emitted from a clone, so the admitted operators are never found) → fix the identity; an overflow-only accumulator; small scratch vectors as frame arrays | **identity built: 4.38×, 4.37×** (render_lock 3.49×); the rest open |

Second round, built (C ÷ Rust): `(R-Range)` open-end counter clause — a counted loop's step plain
whatever its end (sum_loop 4.01× → 2.96×, render_marks 4.38× → 3.36×, resize 4.32× → 3.33×,
render_lock 3.49× → 2.77×); `(R-SpecChain)` — a checked integer chain in a loop runs plain with
one fallback flag (sum_loop → 1.00×, index_read 3.01× → 1.00×, binary_read 3.29× → 1.90×);
`(R-Range)` callee-step clause — `(q, r) = f(q, r, …)` whose results move each by a literal
(hex_neighbor 4.15× → 1.23×); `(R-GuardedChain)` version clause — a loop whose only calls are
versioned sites runs a plain copy calling the fast twins (draw_quads 4.51× → 2.71×,
render_stage 2.54× → 1.98×, composite 1.30× → 1.12×, crawler build_vis 1.48× → 1.26×);
`(R-Version)` nest clause — an inner loop's calls decided once per outer entry, at the corners
of the counters' box (draw_quads 2.70× → 1.48×).

On the C ship lane the seven routines that stood over 2.7× all came under it on both
compilers, and a full clang pass read 217 routines at a median of 1.19×
([handoff-2026-11-08.md](handoff-2026-11-08.md);
[the second host's pass](../../../../bench/portal/analysis/hosts-2026-11-09.md)).

Not a cost, measured: the per-call frame (`loft_frame_push`/`pop`, `build_rec` step 2: 0 %) and
inlining the reference accessors (`lru_mixed` step 3: 2.5 %).

## Left to the performance loop

Making a row fast is the ordinary performance loop (`loft-optimize`), not a plan; these are
handed to it.  The four with a named mechanism are rows of
[PERFORMANCE.md § Open performance work](../../PERFORMANCE.md#open-performance-work).

- **random `get`, 2.72× (clang ship)** — arithmetic over two record fields; the twin inlines
  it and keeps the state in registers.  `bench/portal/hand_price_c.sh` does not link a
  library's C shim: build a hand form with the library's `c/*.c` on the command line.
- **`lru_mixed`, 2.6×** — what is left is the backward-shift removal (`loft_h_remove_at`, about
  a quarter of the routine).  Price a branch-free shift on today's layout first; the twin's
  removal (a tombstone unless a neighbour is empty) changes the table in both runtimes and in
  the image, which is the owner's decision.
- **zttext `materialise`, 2.0× / 2.6×** — a text clause of `(R-AppendTwin)`: hand-priced 1.60×
  gcc, 1.80× clang.
- **graphics `canvas` and `clear`, about 2.1×** — the floor while pixels are `vector<integer>`:
  `u32` cannot hold opaque white (`(N-Reserve)`), so the width is a language question.
- **Priced by hand on the checked lane** (the forms were not kept; the prices are).  `exp`
  as its own operator is built since, as `(R-ExpPow)`, and so are the abs clause, the text
  window and the keyed removal; the others are not: `(R-Invariant)` hoist clause and the guarded
  copy an inlined callee never gets (blur_region 4.98× → 1.99×, render_stage 3.85× → 1.75×); a
  sticky-flag sum guarded by an element-size scan (the resample → ~2.1×); `(R-GuardedChain)`
  guard hoisted to the outermost loop its leaves are linear in (draw_quads 4.30× → 1.23×,
  write_text 3.03× → 0.78×); an accumulator clause for variable steps (draw_line 3.03× → 1.32×,
  draw_bezier 3.72× → 2.45×); a comparison clause for `(R-Range)` and the non-null proof asking
  the range proof at a call (blend_pixel 3.68× → 1.53×); `(R-RangedCall)` loop guard for nests
  whose counters start at a field (roof_cone, roof_match, field_union, stencil_rotate → 1.6–2.5×),
  the accumulator rule under a guarded copy's facts (edgeset_count 3.92× → 1.01×), an entry guard (`day` 2.83× → 1.24×, terrain_surface_at
  3.56× → 2.45×); `(R-PushFill)` nested window (grid 4.02× → 0.76×); `(R-Range)` abs clause and
  loop-end divisor (fov_rays 3.39× → 1.69×); a text clause for `(R-AppendTwin)` and a text window
  (materialise 3.11× → 1.82×); the hash table's per-probe cost (lru_mixed 3.20× → 2.64×);
  `exp` as its own operator (approach 2.87× → 0.92× — changes the last bit of a float: an owner's
  decision).

## The rest of the arc's list, as it ended

- **`h[k]!` handed to a parameter that keeps a reference** (`push_front(c, c.nodes[k]!)`,
  once refused as outliving `_ncr_1`) compiles and answers right on both backends: the `!`
  temp is a view of the container, as `(B-View)` says (loft#2404, guard
  `binding/B-View/2404-a-discharged-element-is-a-view-whatever-the-declaration-order`).
- **The strict-mode store count** (`O-NoDiverge/1328`, `/1329`, `O-Proxy/1320` under
  `LOFT_STRICT_STORES=1`) no longer reports an exhausted store table, on either backend.
- **A structure with shared nodes or back-links** — owned children mixed with `reference<T>`
  — is exercised by 23_lru only; a graph or parent pointers have no cell yet.

## Documentation

Every line is in the tree: `formal/binding.md` `(B-OutOfLine)`, `(B-Disturb)`;
`formal/calls.md` `(F-ParamRef)`; `formal/rewrites.md` `(R-MoveChild)` and the rest of the
table above; `DIAGNOSTICS.md` `walk-or-write` and `lost-write`'s fourth shape;
`BOTH_BACKEND_SWITCHES.md`; `DESIGN_DECISIONS_OWNERSHIP.md` C77; `doc/00-vs-rust.html#slots`;
`bench/README.md` with `bench/portal/routines.tsv` and `classes.tsv`;
`RULE_CROSSINGS-history.md`; `CHANGELOG.md` ("Tree code in Rust's shape").

## The record

[handoff-2026-11-06.md](handoff-2026-11-06.md) the language half,
[handoff-2026-11-07.md](handoff-2026-11-07.md) the judged lane and the seven over 2.7×,
[handoff-2026-11-08.md](handoff-2026-11-08.md) where they ended and the lessons;
[hand/](hand/RESULT.md) the hand forms.

## See also

- loft#2433, loft#2434 — the two defects this arc fixed.
- [`formal/binding.md`](../../formal/binding.md) — `(B-Ref-Write)`, `(B-OutOfLine)`,
  `(B-Disturb)`, `(B-Ref-Reshape)`.
- `bench/21_tree/` — the routines and their Rust twin.
