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

## Attempts and closures

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
