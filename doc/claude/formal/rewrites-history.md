<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native rewrites — the record of attempts, and where each invariant is held

The companion of [rewrites.md](rewrites.md), in the shape of
[ownership-history.md](ownership-history.md): a rule says what must hold; this file says
which site actually holds it, what was tried against it, and what a sabotage measured.  Read
it before attacking a rule with a sabotage — two of the three sabotages of 2026-09-09
stayed GREEN, and each time the reason was that the invariant was held by a site other
than the one attacked.  A green sabotage is a finding about WHERE the invariant lives, and
this is where it is written down.

## Where each invariant is held (2026-09-09)

| rule | held by | a sabotage that turns it red | a sabotage that stays green, and why |
|---|---|---|---|
| R-Switch | every hoisted read's `VERIFY` monomorph (`get_elem_hoisted`, `vec_set_hoisted_or_raise_runtime`, `hoisted_scalar_verify`, `push_hoisted`) | any stale holder: the first read panics under `LOFT_HOIST_VERIFY=1` | — |
| R-Header | `hoist::vector_candidates` + `blocks_header_hoist`'s allow-list | shift a fused write's field offset (P4b's falsifier: writes land in neighbours) | — |
| R-Scalar | `hoist::WriteSet::evicts` at ANALYSIS time (caller side) | `evicts` answering `false`: eight P4c cells red | the CALLEE-side filter in `callee_inputs_inner` skipped: green, because the caller's write set already carries the callee's writes through `callee_writes` — the callee-side filter is a second assertion, kept to keep a twin's signature free of inputs no caller can hold |
| R-Inputs | the twin's parameter WIRING (`push_twin_frames` naming `__is_k` / `__ih_k` in the order `twin_call_inputs` passes them) | rotate a callee's two same-typed inputs: the composite cell red, the verifier panics `hoisted 3, now 4` | the rebound test on the input candidates skipped: green, because a loop that re-points a link is declined by the GATE (`OpCreateStack` takes a reference operand and is not store-free) before the rebound test is asked; reversing ALL inputs does not compile (c10's twin takes a `u8` and an `f64`) — a refusal, not a measurement |
| R-Push / R-Refresh | `Stores::push_hoisted`'s fast path: the length written to the header AND the record | skip the record write-back: the constant-fill cell red (`len(lay.best)` reads the record), the verifier panics at the next push | — |
| R-State | the ORDER of `hoist::hoistable`'s collectors (reads, callee inputs, then pushes removing themselves from the read list) | run the push collector first: a pushed path gets a plain header too, and a twin handed the plain one reads one push behind (the 2026-09-09 emission, caught by the verifier) | held by order, not by construction; the closure is the state builder (below) |
| R-Alias | `hoist::owned_local` / `hoist::retbuf_var` in `hoistable`'s admission block | drop a view candidate's exclusion: an alias of a pushed local reads a moved record | — |
| R-Base | `hoist::LoopHoist::growth_free` (decided before the analysis' early returns — a pixel loop has no record scalars to hoist and returns there) and the base bindings in `begin_vector_hoist`, one per header of a growth-free frame or per HELD header of a growth-free inner loop; `vector::vec_base` / `get_elem_at` / `Stores::vec_set_at` | `vec_base` answering one element past element 0: every cell of `V-ak-vector-base-cells.loft` answers a neighbour's value (`b1 900` for 910) — the VALUE channel, at once; a store that grows under a base: `LOFT_HOIST_VERIFY=1` panics at the first read (`hoisted vector base is stale`) | the growth-free test placed AFTER the scalar early return (the first build): green everywhere and the resample's loops never bound a base — the profile, not a cell, said so (probe 130 ms unchanged) |
| R-Counter / R-LitDiv | `non_sentinel::seed_range_counters` over `hoist::range_counters`, the fill rewrite's own parser moved verbatim into one home; `int_arith::literal_divisor_form` before the proof-gated table | `LOFT_NN_VERIFY=1` asserts every seeded counter at its use; an inclusive range to the type maximum stepping through the non-null add is loft#1525's edge and is excluded by the literal-below-maximum test | the integer closure (`+`/`-`/`*` over proven operands counted proven): `LOFT_NN_VERIFY=1` green on every cell because no cell overflows — the divergence it would open is after a reported fault, which no verify run reaches; declined on C85, its price measured in § V-aj, and closed by the owner on 2026-09-15 (DESIGN_DECISIONS.md C120: no value may differ after a fault — a range PROOF is the admissible successor) |
| R-ValueRecord | `hoist::value_shape` (what a value leaf is) and the positional `hoist::site_walk` (where a tuple may stand), read by the gate's fixpoint AND by the emitter's `value_locals_in` / `value_view_leaves`, so admission and emission are one computation; the fn-ref arms come from `fnref::dispatch_arms`, the emitter's own scan | admit an `Operand` position (pass a tuple where a record is wanted): the generated crate does not compile — measured as 376 rustc errors over the corpus before the site gate held (§ V-ah stage 1); judge the view leaf by its dep list instead of the ownership oracle: `q1021`'s `__ret_1` (`Own::Join`) is read as a tuple and the default it minted is nobody's (measured: five corpus scripts, before the oracle stood here); a value local bound by the whole-record COPY arm (a generic instance's statement join): the crate does not compile — `expected DbRef, found (i64,)` ×6 on `tests/docs/25-generics.loft`, the heavy shard's `native_dir` (t14, 2026-09-14); a `__lift_` temp read as a view leaf: every value exact and one record leaked per call under `LOFT_NATIVE_LEAK_CHECK`, on `const` parameters too (t15) | the by-RECORD fn-ref test that stood beside the `FnRef` scan: green on every cell and wrong on a lambda reaching a dispatch through a typed variable alone — the second spelling the one-home refactor removed; the lift's VIEW verdict — the oracle derives an un-minted `x = a` from `a` (Borrowed) while the lowering mints and the record form hands the store up: green on every cell because no cell selected a by-value parameter, caught by a probe's leak check the morning the generics doc failed to compile |
| R-LoopBuffer | `hoist::loop_buffers` (a `__vdb` minted under a `Loop`, backing ONE vector of no-heap elements, every mention its init family or a free, its `Set(v, Null)` outside the loop; the emitter drops buffers other rewrites own — § V-x, § V-z, § V-j, § V-u), the `OpDatabase` emitter's reuse arm (`vector::vector_buffer_reset`) and the block loop's dropped `OpSetInt4` zero | the length reset sabotaged into a no-op: `l1 315 20 51` for `210 10 -4` — every sum and length accumulates and the stale probe reads real elements (the VALUE channel, at once); a `text` vector admitted: `LOFT_NATIVE_LEAK_CHECK` sees each iteration's strings stranded | the hand-patched resample emission (F3, 2026-09-15) kept the field zero as well as the store on one draft: the vector record was orphaned inside the store and every iteration claimed a new one — green on the hash, growing the store per pixel, caught by reading the emission before running it |
| R-PushFill | `hoist::push_loop` (the counted pushes at the body's top level, `simple_invariant` for the end and the fill value, the early-exit and other-write walks over the body) and `Output::push_fast_path` / `push_fast_path_tail` (`stores.push_fill`, `vector::reserve_more`, the counters through `range_tail` shared with R-Fill) | the fill's length bumped one short: `p1 4 28` for `5 35` — the VALUE channel, at once; `LOFT_HOIST_VERIFY=1` re-derives the push header at the fill and panics on a stale one | the first recogniser counted the path's own `OpPreAllocVector` as a foreign statement because the parser emits it BEFORE the push it reserves for, so every constant push loop took the reserve and none the fill — green on every cell (the reserve is exact), caught by reading the resample's emission for the `__pf_` line that was not there |
| R-Invariant | `hoist::invariant_chains` (the maximal chains of the loop's own body, `arith_chain` over the admitted ops and the banned leaves — `rebound_vars` plus `non_sentinel::collect_escapes`, the proof's collector, now reading a by-reference argument through its `OpCreateStack` spelling) and the emitter's `begin_vector_hoist` / `emit_invariant_use` (the memo pair declared at the loop that spells the chain; the first use evaluates, every later use answers) | the memo made to record the sentinel after its first evaluation: `m1 [0,0,0,0]` for `[5310,6420,11970,13080]` — the VALUE channel at the first cell (2026-09-15) | a memo declared one loop OUT stays green on every cell (the value is the same wherever the pair is declared) and costs the whole gain — the flag's state at the inner loop's entry is unknown, so the test stays in every tap (95.1 against 92.7 ms/op on the resample probe); the cells cannot see placement, only `tests/invariant_arith.rs`'s count of memos per function can |
| R-Wrapper | `hoist::one_op_wrapper`'s ORIGIN test (`def.source() == STD_SOURCE`) | remove it: a user one-op function inlined, the wasm live-dispatch probe counts 0 dispatches (D-rw-1, the gate that found it) | — |
| R-Fuse (mirror clause) | `fusable_int`'s mirror of a literal-left comparison and `ops::fused_cmp`'s `GT`/`GE` kinds | `3 < a` left as `LT`, or `3 <= a` mirrored to `GT`: `test_literal_on_the_left` red (2026-10-01) | — |
| R-FirstClaim | `generate_call`'s skip of an `OpPreAllocVector` with n <= 11 over a plain local, and `vector_append`'s own 11-element first claim, which the skip relies on | `vector_append` claiming nothing for an absent vector: the first reserved cell red with the elision on, every reserved cell green with it off (only the never-reserved field cell red) | the skipped op answering `integer` and its operand left pushed: both green — a statement's type is not read and a loop's jumps reset the stack, so neither is a defect this rule can cause |
| R-LoopSlot | `slot_alias::range_slot_aliases` (the one home: the `OpCreateStack` and second-`Set` declines), `assign_slots_v2`'s placement, `generate_set` / `gen_rotated_loop` dropping the copy where the positions agree, `frame_view`'s one-value pairs | the address test ignored: `&integer` cell 100 after one round; the copy dropped without the alias: 310 for 20, on both layouts; `frame_view` without the pair: an edit of `i` dropped from undo after a step | the index-written-in-the-body decline: no loft spelling writes a range's index, so no cell can reach it |
| R-Rotate | `gen_rotated_loop` (entry jump, test at the bottom, forward-patched `continue`s via `Stack::patch_continues`) | the entry jump lost: 5 rounds of a zero-round loop; continues never patched: 55 for 37; the test jumping to the entry: never runs a body (timeout) | — |
| R-CallLine | `State::call_line`, the frame's `call_pos` | — (the stack-trace cells pin the line; the derivation is the lookup the call made) | the first unpinned timing read the hash loop +14 % at identical instruction counts: code and frame layout, not the rule (PERFORMANCE.md § Measuring an interpreter change) |
| R-DispatchStop | `Stores::raise_runtime_error` (the one store of a runtime error, which sets the flag), `State::invoke_native`'s post-check, the `parallel` block's post-join check, `State::lean_stop` | the setter not setting the flag: the op-raise cell red (a native raise stays caught by `invoke_native`'s own check); `invoke_native` ignoring a frame yield: `dispatch_reentry` red; ignoring a worker fatal: six loft#1053/#1056 cells red; the `parallel` block's check removed: its loft#1053 cell red; the cold path not halting: both raise cells red (2026-10-01) | `enable_debug` / `arm_profiler` setting the flag: no loft path arms a debugger inside an op, so no cell reaches it |

## Attempts and closures

- **2026-09-30 — `(R-Cold)`'s `#[cold]` half (823989c77).**  `get_elem_hoisted_cold`,
  `text_elem_cold` and `vec_set_hoisted_cold` carried `#[inline(never)]` without `#[cold]`.
  LLVM then weighed the out-of-range branch evenly and, once 7be88b9d4 (@PLN174 F1–F3) added
  never-taken foreign-record branches to `Store::read`/`addr`/`valid`/`elem_base`/`bytes_of`,
  if-converted bench 10's per-element bounds test into `setns`/`setb`/`cmpb`/`jne`: 25 % more
  instructions, 10_sort 2.82x -> 3.71x of its Rust twin (the release candidate's ratio-gate
  red).  Removing the five branches one at a time moved nothing but `valid` (3.35x); all five
  restored 2.82x — the signature of a threshold, not of one cost.  Adding `#[cold]`:
  instructions 1.359e9 -> 1.170e9, ratio 3.74x -> 2.78x on the candidate.

- **D-rw-1 — a user one-op function inlined as a stdlib wrapper (2026-09-09, CLOSED).**  The
  recogniser asked the body's SHAPE and not the definition's ORIGIN.  Found by the GitHub
  gate's wasm live-dispatch probe on the rebased tree; the local curated set never runs it.
  Closed by the origin test; cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  The third form of one lesson: a peephole matching an op shape must also ask what the shape
  stands for — the TYPE (P4c), the CONTAINER (§ V-m), the ORIGIN (here).
- **One path, two holders (2026-09-09, CLOSED by ordering; the structural closure is
  queued).**  The first push loop beside a twin call: `hoistable` collected the push paths
  BEFORE § V-p's callee inputs, the input collector added the pushed path again as a plain
  header, `begin_vector_hoist` bound both, and the twin was handed the plain one — one push
  behind.  No value cell saw it; `LOFT_HOIST_VERIFY=1` did, on the callee-inputs guard
  (`hoisted {len 12}, now {len 13}`).  Closed by running the push block after every
  collector that can add a read candidate.  This is the defect R-State was written for, and
  the reason the chapter now has a hoist-state section: the per-rewrite rules were each
  right and together said nothing about the same path being claimed twice.  The closure by
  construction — `hoistable` as ONE state builder — and the emission audit that checks the
  emitted routines against R-State are both queued in @PLN157's README.
- **Two green sabotages (2026-09-09).**  Recorded in the table above (R-Scalar's callee-side
  filter, R-Inputs' rebound test).  Each was a claim in DESIGN.md's `@falsified-at` draft
  that the measurement refused; the guards carry the sabotage that measured instead.
- **A rewrite this chapter does not own (2026-09-09).**  A push whose VALUE reads the pushed
  vector (`px += [px[len(px) - 1]? + d]`) is lowered by the PARSER as `OpDatabase ·
  OpAppendVector` — a whole-vector COPY per iteration — before the push, so the push tier
  never sees it (cell c3 of `V-q-hoisted-push-cells.loft`).  `lock_ribbons`' running sum is
  quadratic in the point count through it.  A lowering shared by both backends belongs to
  [collections.md](collections.md), not here; noted so the next reader does not look for it
  in the emitter.

## Shipped: the emission audit (2026-09-09).  Queued: the state builder

1. **The emission audit** (@PLN157 § V-r, SHIPPED 2026-09-09): `scripts/emission_audit.py
   <emitted.rs>` checks R-State, R-Refresh and R-Inputs structurally over a `--native-emit`
   output — the prelude lines bind holders keyed by their path EXPRESSION text, the reads,
   writes, pushes, `.len` uses and twin calls name them, and a template append on a held
   path is a violation; `tests/emission_audit.rs` runs it over every cell corpus and the
   in-repo bench in the gate.  Falsified against the collector order that produced the
   double holder: flagged at emission, with no run.  This is the plan's validation goal: as
   the emitted routines grow, their assumptions stay checkable.
2. **The state builder** (M): `hoist::hoistable` today is three collectors and two `retain`s
   whose ORDER holds R-State.  A `LoopState` with `hold(path, Holder)` as the only insert (a
   second holder for a path is an error, not a shadow), `evict(scalar)`, and
   `admit_mover(path)` applying R-Alias; the gate, the read collectors, the callee inputs
   and the pushes each call it; `begin_vector_hoist` iterates the map.
   Behaviour-preserving (the loft-codegen skill's Mode B: byte-identical emission over the
   cell corpora before and after), and the precondition for the next mover — the
   self-reading push once the parser stops copying, a bulk fill for a constant
   comprehension.

## Deviations carried by rewrites.md until 2026-09-29

Closed entries moved here from the rules chapter's register (RELEASE.md § 5b), as written.

- **D-rw-7 — OPENED AND CLOSED 2026-09-29 (loft#1741).**  `(R-MoveLast)`'s build-into-the-
  field form: a vector local whose last use is a field store was built INTO that field where
  the local was declared, and a binding of the field's root between the two — `m9 = mo`, a
  call result, an arm of an `if` — replaced the record the elements were built in.  The store
  then answered an empty or stale vector on both backends with no diagnostic, and a FRESH root
  was used before its `let` on `--native` (E0425).  Both lowerings carried it, the replace
  (`m9.b = rsl`) and the append (`m9.b += rsl`): each guarded that the root EXISTS at the build
  by the absence of a later `OpDatabase`, and a binding by assignment allocates nothing, while
  their read guard counted `Var` reads and a rebind names its target as an id.  **Fix.**  A
  binding of the root in the source's build region declines the rewrite (`binds_var`, the
  `Set` twin of `refs_var`), and the append path's prescan records each binding's order.  The
  rewrite census holds its admissions.  Guard
  `tests/scripts/a-field-store-lands-in-the-record-bound-at-the-store.loft`.
- **D-rw-6 — OPENED AND CLOSED 2026-09-28 (loft#1729).**  `(R-Const)` makes a constant's view
  B-Copy's copy the moment it is "handed to a parameter the callee writes", and a top-level
  vector constant handed there went through as the VIEW: the callee's first write reached the
  write-locked constant store and a development run halted (*"write to a constant"*), on both
  backends.  loft#1686 had covered the bind (`c = CODES` copies); the argument was the other
  road.  **Fix.**  `Parser::constant_arg_needs_copy`: at a loft-defined callee's non-`const`,
  non-`&` vector parameter, an `OpConstRef` argument travels as a copy
  (`materialize_collection_value`) when the callee writes that parameter
  (`callee_param_writes`) — read only for a callee parsed before the caller, since the copy
  mints its temporaries on both parser passes and pass 1 has no later body; a forward or
  recursive callee is assumed to write.  A callee that only reads still receives the view.
  Guard `tests/scripts/1729-a-constant-handed-to-a-writing-parameter-travels-as-a-copy.loft`.

- **D-rw-5 — OPENED AND CLOSED 2026-09-21 (loft#1574).**  `(R-InPlaceLiteral)` rebuilds a
  literal into a whole local in place, and `(E-Asgn)` says its right-hand side is computed
  before the store.  The re-init comes first, and #330 lifts an initialiser that reads the
  local above it.  A COLLECTION member cannot be lifted: it is primed with its field place and
  its literal writes through the field, after the re-init.  So `s = Bx { v: [s.n + 1], n: 2 }`
  read the cleared record: `1` for `6`, an element `null`, a comprehension over the local's own
  vector an empty vector, and with `n` written first, the NEW `n`.  Silent, on both backends,
  and older than @PLN164.  Found while probing `D-heap-25`'s vector-member cell.  Closed: the
  parser watches for the author naming the local while such a member is parsed
  (`Parser::rebuild_watch`, set in `var_usages`).  If it does, the literal is parsed again with
  the in-place hint declined, built apart and bound — the retry the parser already took for a
  postfix.  Two defects on that road were fixed with it.  The lexer's interpolation state was
  not replayed with the tokens, so a literal parsed twice refused every `"…{x}…"` inside it
  (guard `tests/scripts/a-literal-parsed-twice-keeps-its-interpolations.loft`).  And the
  work-ref the literal builds into keeps naming the binding's store for reuse on the next pass
  (loft#1513), so in a loop the re-init cleared the record the literal was about to read:
  `Scopes::scan_set` now gives that reuse up for a construction that reads its binding.
  Guard `tests/scripts/1574-a-literal-rebuilt-into-a-local-is-computed-from-the-old-record.loft`.
  The same read through a call that returns its argument (`s = me(Bx { … s.n … })` in a loop)
  was `D-op-11`, closed the same day.

- **D-rw-4 — OPENED AND CLOSED 2026-09-21 (loft#1571).**  `(R-InPlaceLiteral)` stages every
  field expression that may read the place, and the staging asked whether an expression NAMED
  the destination.  It missed three other names a place has.  For an ELEMENT destination the
  place is the whole variable, and that branch tested only a direct read of `v`, so a view
  local (`p = v[0]; v[0] = Pt { x: p.y, y: p.x }`) was never staged.  A view that reaches the
  destination through another view, such as a loop variable over the container, was missed by
  the one-level deps test.  And a heap PARAMETER, where the destination is a caller's
  (`sw(v, v[0])`), has no deps in the callee at all.  Each answered `2,2` for `2,1` on both
  backends, silently: the element spelling since @PLN164 C1, the field spelling through a
  parameter (`w.p = Pt { … }` inside `sw3(w, w.p)`) since the field road was built.  Found
  while probing `D-heap-24`'s control, a record rebuilt from its own fields.  Closed at the
  staging: a naming of any variable in `Function::store_viewers(root)` counts as a read of the
  place.  That is `(R-ElemFirst)`'s answer to the same question for a growth (`D-rw-3`), moved
  out of `hoist::destination_views` so that both readers ask one home.  Guard
  `tests/scripts/1571-a-literal-written-in-place-reads-the-old-record-through-any-view.loft`
  (16 cells and 3 controls: views, parameters, a loop variable, a nested literal, narrow-int,
  float, enum and text fields).

- **D-rw-3 — OPENED AND CLOSED 2026-09-17 (loft#1553).**  `(R-ElemFirst)` moves the append's
  mint — its GROWTH — to the temp's declaration, and the gate asked only whether a statement in
  between NAMED `out`.  A view of `out`'s element is another variable: `e = out[0]; ws = [];
  ws += […]; w = e.fid; out += [F { fpts: ws }]` left `e` a view (the scope pass saw the growth
  at the append, after the read), and on `--native` it read the vector record the early mint
  had relocated — `4609434218613702656` for `100` at the eleventh element, silently, since
  @PLN157 § V-z.  Found by @PLN164 E-2b's growth-boundary sweep.  Closed at the gate: the
  window may not read a local whose deps close over `out`'s store or, where that store is a
  caller's, a heap parameter; a view of a sibling field is spared by the scope pass's own place
  model.  Guard `tests/scripts/an-element-view-read-before-an-element-first-append-is-not-moved.loft`;
  cells `g25`–`g27` of `164-element-place.loft` for the field destination.

- **D-rw-2 — OPENED AND CLOSED 2026-09-17 (loft#1552).**  `(R-ElemFirst)` says the temp is consumed
  exactly once, and the gate counted its READS but never its BINDINGS: `p: vector<Pt> = [];
  if c { p = mk(n) }; out += [Op { pts: p }]` bound `p` to the early element's field at the
  declaration, the conditional rebind pointed `p` at the call's store, and the suppressed copy
  left the element holding the empty vector — `len(out[0].pts)` 0 where the interpreter says 3
  (`1000` for `1093` in the guard), silently, on `--native` since @PLN157 § V-z.  Found by
  @PLN164 E-2's cell `g13`, which widened the same gate.  Closed at the gate: the declaration
  must be the temp's only binding.  Guard
  `tests/scripts/a-rebound-element-first-temp-keeps-its-copy.loft`.

- **D-rw-1 — CLOSED 2026-09-09.**  `one_op_wrapper` asked the body's SHAPE and not
  the definition's ORIGIN, so a user function whose body is one op (`fn reader(w:
  W) -> integer { w.a }`) was emitted as its op; the wasm live-dispatch probe
  flipped it to the interpreter and counted 0 dispatches where it expected 2.  The
  rule was always (R-Wrapper)'s "stdlib"; the code now asks `def.source() ==
  STD_SOURCE`, and cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  Found by the GitHub gate on the rebased tree (run 34323456806); the local
  curated set never runs the wasm probe.

## Prose moved out of rewrites.md on 2026-09-29

Measurement stories, pricing, sabotage receipts, dated status and "first build" narratives
lifted out of the rule sections of [rewrites.md](rewrites.md) (DOC_QUALITY § Maintainer docs 4),
as written and under the heading they stood in.  The rule text left in each section states the
outcome; the numbers and the story are here.

### Preamble

- **Why a chapter.** Every rewrite rests on ONE invariant, and until now each lived as prose in its own design section (`doc/claude/plans/157-native-4x-drawing/ DESIGN.md`), with no name a code site could cite.
- Measured: `src/generation/ hoist.rs` cited zero rules while enforcing eight, and the first defect of the family — a user function inlined as a stdlib wrapper (D-rw-1 below) — was the rule's own qualifier ("stdlib") that the code had never asked.

### Every rewrite is switchable and falsifiable

- loft#1647 was that case: a lazily minted return buffer (`O-LazyBuffer`) met a double release older than it and a loop read its first pass's value on both backends — six days on main, seen by no cell, found by a consumer's suite.
- Its switch told the difference at once (`LOFT_NO_LAZY_BUFFER=1` restored the value), which is the A/B's whole argument: for this family the switch

### The hoist state — what the rewrites compose through

- Measured: the first emission of a push loop beside a twin call gave one path TWO holders, because the push collector ran before the twin-input collector; the verifier caught it and the ordering closed it, but nothing in the per-rewrite rules had said it was wrong ([rewrites-history.md](rewrites-history.md)).

### A vector reached by a pure path has one header for a loop that cannot move it

- names; admitted 2026-09-18, the crossing loop's `pg_table[i]?` had kept its loop on headers alone)
- Measured, hashes unchanged — on `bench/stats.py` (7 samples pinned to the fastest core, each ±0.5 % or tighter): `push` 48.8 → 8.0 µs (0.47× its Rust twin), `comprehension` 62.1 → 11.0 µs (9.53× → 1.70×), `grid` 69.6 → 23.9 µs (8.10× → 2.81×); and `f32_build`, A/B on one build through the switch, 739 → 147 µs (−80 %, eighteen pushes a pass through one window).
- Every other row of both lanes within noise, and the one that read slower in an UNPINNED loop — `record_append`, bimodal at 69k / 76–79k — is byte-identical in its emission and reads 68.4–69.5k (±0.4 %) pinned: not reproduced, and its cause not established.
- THE FORM IS LOAD-BEARING: the first hand-price kept the header's own `len` as the counter and handed the header to the growth arm by reference — its address escaped, LLVM kept it on the stack, and every push loaded and stored the length through memory: 22.8 µs.
- The window is three scalars, the growth arm takes the length BY VALUE and answers a fresh window by value: 10–13 µs.
- Sabotage, measured (each admission clause off in turn): "names the pushed vector" — d1 answers `51 102` for `51 2601`; "names a variable that may view it" — d4 answers 780 for 2340; "writes a store" — NOTHING CHANGED, so no cell guards that clause and none is claimed to: every second grower that could be built declines the loop's whole hoist before this admission is asked, which makes the clause a second fence behind the push header's own gate, kept because it can only decline.
- ⚠ A prediction that failed and was measured rather than explained: a result vector's pushed root depends on a store witness (`__vdb_1`) that is the hidden return buffer the CALLER handed in, and `hoist::owned_local` reads any `__vdb*` dep as "the store it owns" — so such a root passes as an exclusive owned local and the window is admitted beside a parameter read.
- Whether a caller can make the two one store was built both ways — the result replacing its own source (once, twice, in a loop) and the result built in an element slot of the container its source is an element of, that store reallocating under the call — and every one holds under the verifier: the window needs no reader of the vector's LENGTH and no store growth, and neither needs the parameter to live in another store.
- Hand-priced on the emitted Rust before any emitter code (the build loop edited by script: reserved for its trips, a `push_window` opened on the header, each arm's mint taking `base + len·32` and zeroing it through the window, its finish `len += 1`, the close after every copy the guarded chain emits): 35.9 → 20.4 µs, hash `1493`; built, 23.4 µs — the difference was the fast path staying a CALL until `#[inline(always)]` (`Stores::push_record_windowed`: one growth test `len >= cap`, the slot at `len·size + 8`, the growth arm outlined beside it).
- Sabotage, measured: the finish emitted as nothing, r1 answers `0 0 0 0` for `300 7261 6400 6566` on the built form; the switch restores the interpreter's answer.
- Hand-priced in four forms before any emitter code, `record_update`, hash unchanged: today 34.9 µs; the join KEPT and its result identity-tested against the header 21.4 (−39 %); the index tested first 12.1 (−65 %); that, with the join's temp still assigned in range, 12.2; and that through an `inline(always)` helper answering `Option<(DbRef, T)>`, 12.0–12.4 — the one built, measured 11.6–12.3 (pinned: 4.62× → 1.60× of Rust, range 1.59–1.61).

### A record local declared in a loop keeps its store

- **In words.** 2026-09-18, the `(R-LoopBuffer)` shape for a record.
- Measured on the probe (`s = S1 { a: k }; t += s.a`, 200 000 passes): 7 600–8 500 → 480–540 µs per 200 000 passes, 39 → 2.4 ns per pass (the kept record's field write and read no longer resolve a store per pass either), value unchanged.

### An in-place scalar set disturbs no header

- — 9.0 µs against 6.6 on `join`
- Measured: the stdlib `join` 11.0 → **6.6 µs** (−40 %, 2.11× → ~1.15× of its twin), hash `2df9`.
- Until 2026-09-22 (@PLN158 round 3, P1) the record had to be ALL-SCALAR, on the reading that a store hosting no vector could not be named by a header; the invariant-path argument is the one that holds, and the restriction had kept `map_set`'s loop — three `m.chunks[i]?` joins over `Chunk { …, hexes: vector<Hex> }` — on no header at all, each join a store resolution and `len(m.chunks)` a call per iteration.
- Found by a five-variant one-statement bisect under `LOFT_TRACE_RECPTR=1`; priced by deleting the write-back (−26 %); built, `entity_tick` 205.4 → 157.7 µs (−23 %), 3.19× → 2.45× of the Rust reference (4.67× before this arc).
- Measured on the crawler's `read_i16_vec` shape — `while i < n { out += [read_one_i16(bf)]; … }`, the read in a helper — together with two runtime costs of the read itself (a `SipHash` of the name `text` in `is_text_type` and a heap buffer per read, both gone): `binary_read` **6.20 → 2.87 ms**, 32.8× → ~14×, hash `fa34c62d78`.

### A record view carries its address

- Measured on the `wide_line` probe: 7 800 → 5 060 ns/op (−35 %), 2.9× → 1.9× the reference; hand-priced first at −25 % for the reads and −15 % for the callee's inputs.
- The first build admitted a return BUFFER's null pre-init (`__ref_p2_N = null`) as a binding and did not read the literal's mint into it (`OpDatabaseNP(buf, tp)`, a native op writing its first operand, not a `Set`) as a rebind, so the literal's field writes went through a null address and were dropped — a library's `mk()` answered a record of zeros; the suite caught it the same day (the 47 golden, the #672 parity test) and the two clauses above are its receipt, with cells r14/r15.
- Hand-priced on the emitted Rust before anything was built (`record_walk`, 32.0 µs per call): the address from the base and the index −40 %, the element's `DbRef` no longer built −49 %, the index stepped unchecked −62 % (1.12× the Rust reference).
- Built, the first two arrive together — once the address stops depending on the `DbRef`, LLVM drops the `DbRef` where nothing else reads it — and the third is `(R-Counter)`'s iteration clause: `record_walk` 31.97 → 16.15 µs (2.98× → 1.50×), `tuple_kernel` −20 % (2.02× → 1.62×), `entity_tick` −30 % (its inner scan).
- The form matters and was measured: a first build took the address from the `DbRef` — an identity (`rec_ptr` is the store's pointer plus `rec * 8 + pos`, the base the same pointer plus `rec * 8 + 8`) guarded by a test of its store and record — and ran **+46 % slower** than the store resolution it replaced, because the tests and the fallback hid the induction from the optimiser; the index form is what ships.
- Priced first by hand-binding the sub-record (`p = v.pos`, −52 %); built, 70.3 → 37.4 µs (−47 %), 4.7×.
- Hand-priced on the emitted Rust (−38…46 %), then built: `record_append` 126.7 → 69.1 µs (3.55× → 1.95×), `mesh_emit` 189.3 → 107.2 µs (4.98× → 2.83×; it stood at 18.3× before `(R-Mint)`'s field clause).
- **The first sabotage of this clause changed nothing** — with the window's verdict blinded to everything but the element's own writes, every cell stayed green, because a stale address needs the store's memory to MOVE inside the window and nine small elements never reallocate: the cells could not fail, so they proved nothing.
- `m4b` (sixty nested points per element, 120 elements) is the cell that can: under the sabotage it answers `120 125094 21420` for `120 127140 21420`, and `LOFT_HOIST_VERIFY=1` panics naming the stale address.
- `chunk_lookup` 1,417 → 1,097 µs (−23 %), 3.67× → 2.84× of the Rust reference.
- `enum_match` 111.6 → 40.4 µs (−64 %), 12.4× → 4.5× of the Rust reference.

### A stdlib one-op wrapper is its op

- Before that `for i in 0..len(m.chunks)` kept `len` a CALL per iteration — a store resolution and a length load — beside the header the loop already held for `m.chunks` (`map_set`'s bound, and 88 such loops in the corpus and the libraries).

### A callee's invariant inputs cross the call

- Measured before the code by hand-editing the emitted Rust: the twin and a full inline of the callee cost the same (`composite` 4.2× → 2.33× of Rust), because rustc inlines the small twin — so the rewrite passes values and never clones IR; shipped, the consumer's `composite` row measures 2.34× (446k → 237k ns/op), `render_lock` / `render_marks` −8 %.

### A push keeps its own header current

- Shipped: the consumer's `lock_curved` 5.64× → 3.84×, `lock` 4.39× → 3.51× of Rust, hashes unchanged.
- Before they joined, every boolean or enum push in a loop re-entered the runtime per element and blocked the loop's reservation: hex_field's `hexset_chunk` (`for _ in 0..n { cells += [false]; }`) is the whole cell set of every `HexSet`, and hex_place `field_union` went 3.76 → 2.18 ms per op with it (−42 %, hash unchanged; hand-priced at 2.25).

### A minted element is a record no holder can name

- Measured: `mesh_emit` 708.7 → 189.5 µs per call (−73 %, the figure its paired-variant probe priced), 18.3× → 4.9× of the Rust reference, hashes unchanged.

### A record append emits through its push header

- Shipped: standalone `smooth` −32 % (3 070 → 2 090 ns/op), consumer `smooth` 11.0× → 4.6× and `fronds` 11.1× → 8.4× of Rust on the measuring box, 14/14 hashes unchanged.

### A mint group outside any held header binds its own

- Hand-priced first on the lean emission (H7 −15 µs, H8 the heap clause −12 µs, together −33 µs), then built: `fronds` 101.7 → 69.6 µs per call (2.71× → 1.86× of the Rust reference), hash `ebcfd875` on every run, the hand ceiling met.

### An integer operator whose result provably fits emits the plain operator

- Measured: r1/r4 (declared `limit` and `u8` parameters) fully plain, the join's `* 256` and `+` plain with the index `p + 1` over a plain integer rightly checked, cbor's `read_value` +8 plain ops, values unchanged.
- Sabotage, measured: the spare-code exclusion removed, c8's `after = x + 5` on an overflowed `limit(-100, 100) size(1)` answers `-9223372036854775803` for null and the verifier panics — the first cell written for it (an `i32`) could not fail, because `i32` is the signed-32 template and excluded a clause earlier.
- Measured on `composite_layer`: 24 of its 36 checked operators take the plain form and the row does NOT move — the twelve that remain (`j * lw + i`, `x0 + i`, `y0 + j`, the accessors' `by * width + bx`) have record-scalar or parameter operands no static proof can bound, and the ceiling (every operator plain, −33 %) is carried by exactly those (class split: those six −35 %, the counters −5 %, the divisions −3 %).
- That is what `(R-GuardedChain)` is for.
- **The counted-counter clause was INERT until 2026-09-21** (loft#1558).
- It is listed above and was written, but no counted loop ever got a range: the seeding looked for the counter's SEED inside the loop, and the parser emits `index = <start>` as the statement BEFORE it, so `seeds` was always 0. The seed is now looked for in the whole FUNCTION
- Nothing made the gap visible, because the one pin that would have shown it (a4's `wrapping_mul` on `i * i`) was recording a plain form supplied by `(R-GuardedChain)`'s duplicated copy, not by this proof: **a pin can borrow another rewrite's evidence and read as proof that its own clause works.** It surfaced only when the guard's profitability gate declined that loop and took the borrowed evidence with it.
- Measured against the clause inert, same lane and invocation: `smooth` +12.9 %, `fill_circle` +2.9 %, `hash` +1.8 %, `render_marks` +1.5 %, `resize` +1.1 %, every other row within its swing.
- Measured: the stdlib bench's `char_walk` (`n += 1` / `n += 2` under `for c in src`) 12.5 → 10.45 µs (−17 %), hash `2e18`; `LOFT_HOIST_VERIFY=1` compares every plain step with its template.

### A counted loop's index chains run plain behind a guard

- Measured, `composite` 103 → 68 µs per call (−34 %, the hand ceiling for those six operators met), hash exact; the trace shows the guard admitting the graphics library's `fill_rect`, `fill_triangle`, `resample`, `mat4_mul`, `sphere` loops as well.

### A dying temporary's elements move into the append that consumes them

- All three lifetime conditions were falsified on builds that lacked them (2026-09-11): the scope-end-only release corrupted a recycled store slot on a6f30ae7 — values right, teardown walked another store's live records (the red `native_scripts` gate) — the host-death build WITHOUT the OpDatabase re-arm corrupted c21 (a store panic at rec 472: the reuse clear had reclaimed the placement the guard still trusted), and a corpus struct named `T` was walked through `main_vector<T>`'s typevar field (the c19–c22 cells).
- The host-death form reuses the placement across an enclosing loop's entries — `fronds` 351k → 295k ns/op (−16 %), the hand ceiling met exactly.
- Shipped: standalone `fronds` −11 % (396–400k → 354–359k ns/op), the hand-measured ceiling reached exactly; hash `ebcfd875` on every run.

### A loop over a split takes its pieces from the text

- Priced by hand patch first (−1.85 µs), then built: `parse` 19.21 → **17.44 µs (−9 %), 298k → 268k instructions per parse**, 2.93× → 2.63× its reference on x86-64, hash `33f6d2b8`; with the switch set the bench's emission is byte-identical to the build before the rule.

### A split bound to a name is a table of its pieces

- Priced by hand first on the `split` row: 21.2 → 6.3 µs (−70 %, hash `21bd`), and 3.3 with the `?`-discharged piece borrowed instead of copied twice — the twin is 2.7.

### A walk of texts borrows each element

- The standard library's `join` paid 30.8 µs for 2 000 words against its Rust twin's 5.7.
- Priced by hand first: `word_count` 58.5 → 44.3 ns/op (−24 %; the temp alone −14 %), `hash_text_keys` 657 → 603 µs (−8 %).

### A character walk steps an ASCII byte in one move

- Together 23.5 → 12.9 µs on the `char_walk` row (4.6× → ~2.5×); the two checked accumulator adds of the body are what remains, C120's territory.

### A format appended to a text is written into it

- The probe (a 120-character escape loop, `--native-release`): 1660 → 410–560 ns per call; the library rows, re-measured on the same box: html `escape_html` 10.3× → 2.64×, markdown `html_escape` 8.6× → 1.71×, zttext `materialise` 23.8× → 6.87× and `flow_layout_full` 107× → 14.8× (bench/portal/analysis/libraries-wide.md).

### A text copied byte by byte is one append

- The probe (256-byte texts appended 64 000 times, `--native-release`): 2.8–3.0 → 0.34–0.50 ns a byte.
- cbor `encode_bytes`, re-measured on the same box: **94.7× → 43.1×** of Rust — the copy is gone and what the row still pays is the buffer's own growth (each doubling claims, zero-fills and relocates) and the per-text work around the copy (bench/portal/analysis/libraries-wide.md).

### A vector copied element by element is one append

- zttext `insert_text` (`nb = []; for c in d.buf { nb += [c] }`), the insert-only bench, `--native-release`: 25.4 → 5.95 ms per op, hash unchanged — on top of the heap fact that `character` owns none (55.2 → 25.4 ms), which had hidden most of this rewrite's gain behind a per-element claims walk (bench/portal/analysis/libraries-wide.md).

### A lookup by one integer key takes the typed entry

- The typed entry is the table's header reads, the digest inline, and the walk compiled for the key's kind: **613 → 444 instructions a lookup**, `hash_find` −13 %, `hash_update` −14 % on x86-64.

### A result vector adopts the return buffer

- Shipped: standalone `fronds` −9.5 % (381–396k → 350–357k ns/op) and `smooth` −14.4 % (2 205 → 1 887), hashes exact, cells leak-free under poison.

### A nest whose arithmetic cannot fault runs plain

- The ledger had the row as "priced negative, the gap is the language's semantics" from an attempt that made the bound a SEPARATE, CHECKED pass and edited `t_7integer_sum` while the row runs its callee twin; and an i128 accumulator that would have closed the gap by CHANGING the answer for a prefix overflow that later cancels was rejected by the owner on principle (slower on many targets) — and is unnecessary.
- Priced on the emitted twin before any emitter code, then built: `sum` 11.9 → 3.7 µs, hash unchanged, pinned **6.45× → 2.06×** of Rust (range 2.06–2.08); the built form matches the price to the microsecond.

### A local that never leaves the frame is the caller's buffer

- Measured on the probe (`fn f(salt) { v: vector<integer> = []; …; len(v) }`, 2 M calls, `--native-release`): 81 → 33–34 ns a call.
- Census of the library corpus (2026-09-25): 371 vector locals declared `[]`, 77 with no escaping mention by the crude test, 194 results (another rule's), 90 handed to a call — the by-value clause above admits those whose callee answers no view of the parameter.
- The probe on the release tier, 2 M calls: **78–91 → 29–36 ns a call**, the hand-written caller-buffer form 26.
- Census of the walk over six libraries (2026-09-25): 98 locals promoted (hex_body 15, hex_terrain 25, cbor 4, graphics 9, hex_field 16, drawing 29); over eight libraries, library code alone, 82 promoted once the by-value clause admitted the hand-offs whose callee answers no view (28 hand-offs still decline: a native, a `&` parameter, a callee answering a view) and the element-field clause kept 37 for the emitter.
- Two things the first measurement taught, both on the drawing bench (2026-09-25/26): the native emitter must read the promoted parameter as EXCLUSIVE where a rewrite only needs nothing else to reach the store — `hoist::work_buffer_arg`, the attribute's mark looked up by argument position, admitted beside an owned local at the push window, the mint window and the loop hoist's mover set — because taken for a possibly aliased view it declined the push window, and `render_marks` and `resize` ran 1.5× SLOWER; and a local the emitter already builds inside an appended element (`fronds`' `fd_wid`) must stay a local, or the buffer costs a copy and the loop its hoists (`fronds` 1.95× → 4.3×).
- The rows, re-measured clean on the converged tree (2026-09-25, the committed row without the rule → now): hex_body `rig_world_frame3` **9.54× → 5.20×** (its twelve scratch vectors), cbor `encode` **18.0× → 11.4×** and `encode_bytes` **43.1× → 24.9×** (the map encoder's key tables), graphics `fill_rect` 4.05× → 3.05×, `draw_line` 2.70× → 2.06×, `blend_pixel` 2.11× → 1.30×, `fill_triangle` 3.13× → 2.87×, drawing `composite` 1.56× → 1.27×; `fronds`, `render_marks` and `resize` within noise of their committed rows once the two lessons above were applied; hex_body `bone_shape_has` 4.84× → 5.44× was the one row worse, and its cause is the rule's own shape: a thin wrapper called once per element that calls `rig_world_seg` (three buffers) mints those buffers per call one level up exactly as the callee did, and pays the callee's clear and witness for nothing.
- Measured clean (2026-09-26, the row above → with the clause): `bone_shape_has` **5.44× → 2.68×**, same hash, 11.2 → 5.5 ms against Rust's 2.07 — the three per-query mints at ~67 ns each were half the query, not the sixth the hand-price guessed from an assumed 1 µs row; every other row of the five libraries within noise.

### A frame no registrant can reach carries no buffer guard

- Measured on `fib(30)`: 11.3 → 7.4 ms (−34 %), 4.1× → ~2.5× of its Rust twin, hash `cb228`.
- The cells also found a leak of their own, unrelated to the rule: @PLN150's borrowed-return marker taken only on a MATCH stayed armed past an unrelated copy and matched a reused slot number later (`158-fnref-borrowed-marker.loft`); it is now taken by the first copy after the call, as the interpreter's is.

### A value already home is not copied to a second one

- Measured on the sphere shape (three tuple builders into a builder into an append, two million vertices, this box, idle): 0.27 → 0.17 s; `LOFT_NO_VALUE_RECORD=1` 0.43 s.
- Measured on the crawler's `rivers_test` (a 115 200-, an 86 400- and two 14 878-element table): the emitted Rust 61.7 MB → 4.9 MB, `init` 1.06 M lines → 17 k, and a cold `--native` run from more than 20 minutes (unfinished) to 16.4 s.
- zttext's insert loop (`io_d = insert_text(io_d, …)` inside `insert_op`): 5.95 → 5.01 ms per op, hash unchanged; two admissions over the census programs (zttext, hex_body).
- **Hand-priced 2026-09-24** on the bench row's emitted Rust (the exit's mint and `buf` copy struck, `pieces` delivered into `d`'s own record, `return d`; the call site unchanged — its identity test already accepts a result that is its argument): **61–64 ms → 4.2–4.6 ms per op (−93 %)**, hash `cb409877` unchanged, native leak check clean; against the twin's 0.68 ms the row goes 138× → ~6.5×, the remainder being the per-edit rebuild of 5 000 pieces, which the twin pays too.
- **BUILT 2026-09-24** (`LOFT_NO_REBIND_PLACE`, `LOFT_TRACE_REBIND`), in three pieces.
- and a body whose LAST statement is an explicit `return` now takes the buffer road at all — until then only a bare tail did, so every `return S { … }`-ending function, the shape the libraries write, minted a store per exit and its callers copied.
- Measured on the zttext bench, same binary (`LOFT_NO_REBIND_PLACE=1` 88.5 ms/op, 211×): `delete_range` 4.1 ms/op (**138× → 9.6×**, the twin's lane noisy at ±5 %), `insert_text` 95× → 63× (its own `nb` copy of the buffer remains, the library's algorithm), `invert` 118× → 99× — `apply_op`'s exits are CHAINS (`return delete_range(d, …)`), the next admission to write: a chain whose callee is rebind-safe on the parameter it is handed and whose buffer is handed on answers the buffer too.
- **Hand-priced 2026-09-24** on `truncate_to`'s emitted Rust: the rebuild — a store for `te_new`, a copy of every kept entry into it, a SECOND hidden store for the rebind, the entries copied again, the old field cleared, copied back, both stores freed: three deep copies of every kept element, each claiming its two inner vectors — replaced by one in-place compaction (release each dropped element where it stands, one block move of the kept run, the length set): **3.2–3.8 ms → 414–435 µs per op (−87 %)**, hash `31aefae` unchanged, clean under the leak check, `LOFT_POISON` and `LOFT_STRICT_STORES`; against the twin's 13.8 µs the row goes 207× → ~30×, and what remains is the per-push `Stroke` construction — `(R-MoveAppend)` / `(R-Place)`'s class, not this rule's.
- Measured on the portal's consumer lane, same box: `truncate_to` **3.62 ms → 195 µs per op, 259× → 14.0×** of Rust (−95 %: the row's 250 drop-oldest rebuilds per op and its 30 truncates are all compacted; the hand-price replaced the truncate alone).
- Sabotage receipts: the guard's `hi <= len(V)` test struck, the beyond-length cell (c7) keeps the vector whole instead of padding, on both backends; the "t named nowhere else" test struck, the pin sees c14 compacted.
- **Hand-priced 2026-09-24** on the row's emitted Rust: the two tables built once per process (a `OnceLock` holding the first build's `DbRef`, minted with a null buffer so the store is its own) **117 → 7.0 ms per op**, and the callers' hidden buffer struck as well (`(O-LazyBuffer)`'s mint before the call and free at every exit, ~74 ns a pair over 38 000 calls) **→ 4.3 ms (−96 %)**; hash `53aaaae5` unchanged, the leak check naming exactly the two resident tables and nothing else; against the twin's 1.31 ms the row goes 86× → ~3.3×.
- Found on the way: a TOP-LEVEL constant bound to a local and written (`c = NAMES; c[0] = 5`, `c += [7]`) or handed to a parameter the callee writes (`f(NAMES)`) had no copy road and halted on the lock — the bind was given its copy by loft#1686 and the argument by loft#1729 (deviation D-rw-6, closed).
- Measured on the portal's text2d lane, same box: `write_text` **165.3 → 4.23 ms per op, 125× → 3.23×** of Rust (the hand-price said ~3.3×; the rest of the row is `set_pixel`, the call class), the other four rows unmoved.
- Sabotage receipts: every argument position read as a pure read fails c7 on the interpreter with the write-locked panic; the one-literal test struck answers 5 for 7 in c16 on both backends.
- Hand-priced (the four arm copies struck, the arms answering their sources): 6.62 → 2.86 ms per op, hash `91eac327` unchanged.
- **BUILT 2026-09-26** (`src/copy_view.rs`, after `(R-Const)` on the settled IR; `LOFT_NO_COPY_VIEW`, `LOFT_TRACE_COPY_VIEW`): measured clean **22.62× → 9.94×** (6.65 → 2.92 ms), same hash, the other graphics and drawing rows within noise.
- Sabotage receipt: condition (3) struck, k4 reads 99 for 1 and k5 1003 for 3 on both backends.
- **Hand-priced 2026-09-24** on `mat4_transform`'s emitted Rust: `p` carried as three `f64` locals across the 500 000-call loop, the callee answering its tuple into them and `tv` crossing as scalars, **45 → 3.1–4.0 ms per op (−92 %)**, hash `1871b40b` unchanged, the leak check silent; against the twin's 1.85 ms the row goes 22× → ~1.7–2.2×.
- MEASURED on the mesh3d bench, same binary, the twin at 1.99 ms: `mat4_transform` **9.9 → 2.64 ms per op (−73 %, 5.0× → 1.32×)** — the hand-priced form to the number; `sphere` 6.5 → 4.05 ms (23× → 13×, its `vertex` and `normalize3` parameters tuples); `mesh_to_floats` 12.7 → 8.4 ms; `mat4_mul` 51 → 44 ms (its `mb` parameter, `(R-Rebind)` declining it still).
- Consumer `slope_path_with_undo`: hand-priced 565 → 297 µs per op on the emitted Rust, built 572 → **293 µs (−49 %)**, hash equal.
- Written 2026-09-24 from the wide pass's measurements, ahead of the code, so that the code changes to match them; `(R-Rebind)` was built the same day, `(R-ValueLocal)`, `(R-Const)` and `(R-Compact)`'s contiguous-range form the day after (their entries above); `(R-Compact)`'s filter, prepend and identity forms are NOT BUILT.

### Deviations

- **OPEN: 0** (2026-09-28).

### Date stamps lifted from the rule prose

Each entry is the stamped phrase as it stood; the rule sentence it sat in remains in
[rewrites.md](rewrites.md) without the date.

- (Every rewrite is switchable and falsifiable) `(C122, owner 2026-09-15.)`
- (A record view carries its address) `**In words.** 2026-09-18, the drawing library's crossing loop`
- (A stdlib one-op wrapper is its op) `and, since 2026-09-22 (@PLN158 round 3), a pure`
- (A minted element is a record no holder can name) `*The own-buffer mint (2026-09-27):*`
- (A minted element is a record no holder can name) `*The field clause (2026-09-21, @PLN158 R1):*`
- (A record append emits through its push header) `*The heap clause (2026-09-18):*`
- (A mint group outside any held header binds its own) `@PLN157 (2026-09-18).`
- (An integer operator whose result provably fits emits the plain operator) `**The type clause, in words.** 2026-09-22, the C129/C120`
- (An integer operator whose result provably fits emits the plain operator) `**In words.** 2026-09-20. C120 declined`
- (An integer operator whose result provably fits emits the plain operator) `**The accumulator clause** (2026-09-22, @PLN158 round 4):`
- (A counted loop's index chains run plain behind a guard) `**In words.** 2026-09-20. `(R-BoundedNest)`'s method`
- (A format appended to a text is written into it) `**BUILT** (2026-09-25, `Parser::format_append_in_place``
- (A text copied byte by byte is one append) `**BUILT** (2026-09-25, `src/byte_copy.rs``
- (A vector copied element by element is one append) `**BUILT** (2026-09-26, `src/vec_copy.rs``
- (A nest whose arithmetic cannot fault runs plain) `**The reduction clause, in words.** 2026-09-22, @PLN158 (`
- (A local that never leaves the frame is the caller's buffer) `**The pool** (2026-09-27, loft#1697;`
- (A local that never leaves the frame is the caller's buffer) `**BUILT** (2026-09-25, `src/parser/work_buffer.rs``
- (A value already home is not copied to a second one) `**The nested clause** (2026-09-28; cells`
- (A value already home is not copied to a second one) `**The body clause** (2026-09-26, loft#1697;`
- (A value already home is not copied to a second one) `**The own-buffer clause** (2026-09-26, `LOFT`
- (A value already home is not copied to a second one) `The wide pass (2026-09-24, `bench/portal/analysis/`
- (A value already home is not copied to a second one) `on 2026-09-24)`
- (A value already home is not copied to a second one) `**BUILT 2026-09-25, the CONTIGUOUS-RANGE form**`
- (A value already home is not copied to a second one) `**BUILT 2026-09-25** (`LOFT_NO_CONST_VIEW``
- (A value already home is not copied to a second one) `**BUILT 2026-09-25** (`LOFT_NO_VALUE_LOCAL``
- (A value already home is not copied to a second one) `the terminal copy (2026-09-27, @PLN158;`

## 2026-09-30 — `(R-LazySplit)` and the absent text (loft#1795)

The NULL text was the edge the lazy split's first build got wrong: it answered no pieces, by
analogy with the empty text, where `split` then answered one — `len(null)` was 1, so its
trailing-piece rule fired — and lazy native disagreed with the interpreter AND with its own
switch-off form (`count=0` for `count=1`).  The cell that found it passed a real null (s2,
s21); the first s2 passed `nothing ?? ""`, an empty text, and saw nothing.  The iterator was
then made to answer the null text as one piece, matching `split`.  loft#1795 made `len` and
`size` of an absent text 0, as `for c in t` over it already was, and the iterator, `split`
and the split table now all answer no pieces.
