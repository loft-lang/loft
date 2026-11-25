<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
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
| R-Counter / R-LitDiv | `non_sentinel::seed_range_counters` over `hoist::range_counters`, the fill rewrite's own parser moved verbatim into one home; `int_arith::literal_divisor_form` before the proof-gated table | `LOFT_NN_VERIFY=1` asserts every seeded counter at its use; an inclusive range to the type maximum stepping through the non-null add is loft#1525's edge and is excluded by the literal-below-maximum test | the integer closure (`+`/`-`/`*` over proven operands counted proven): `LOFT_NN_VERIFY=1` green on every cell because no cell overflows — the divergence it would open is after a reported fault, which no verify run reaches; declined on @C85, its price measured in § V-aj, and closed by the owner on 2026-09-15 (DESIGN_DECISIONS.md @C120: no value may differ after a fault — a range PROOF is the admissible successor) |
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
| R-StackBase | `State::stack_base` re-derived in the two constructors, `grow_stack`, `claim_in_stack` and `restore_checkpoint`; `Store::stack_buffer` refused by `claim_grow`, `adopt_image`, `reclaim_tail`, `take_slot`, `take_store`; kept by `snapshot_copy` | `grow_stack` not re-deriving: the guard aborts (exit 134, heap corruption); `claim_in_stack` not re-deriving: `par_nested` red; the checkpoint restore not re-deriving: the rpc continue-after-step-back cell red; the refusal disabled: its unit test red (2026-10-01) | `grow_stack` not re-deriving stayed GREEN on the first three cells: the stale base kept using the freed buffer consistently, so values agreed — caught only once a cell wrote a slot through the store and read it back the fast way.  And the first refusal run found the convention false: `execute_at_text` claims in the stack store |
| R-DispatchPublish | `crash_report::LeanSource` registered by the lean loop; `current_ctx` the one derivation every reader uses | the loop not registering: the guard names a stale op mid-run; the drop not publishing: the after-run context is empty (2026-10-01) | priced at −3 to −6 % before the cached stack base and measured after it at −1.2 % (geomean) with −5 % instructions: the per-op write was off the critical path once the stack's load chain was gone |
| R-CodeBase | `State::code_base`/`code_len`, set by the two constructors and `State::edit_code`; `code_put`, `code_add`, `code_add_str` write only through it | the writer keeping the old length: new code unreachable (out-of-range read); the writer keeping the old base, set once from null: the first result wrong (the stdlib compile already moves the buffer); a write bypassing the writer: the source scan red (2026-10-01) | measured first across a rebase, which credited it with −4.2 %: 129 commits of main were −5.6 % on the same set.  Against the same tree it is +0.9 %, inside the noise.  And it flipped an inlining decision: `vec_get_or_raise` (raises included) stopped being inlined into the vector ops and cost `sort` 5 % of its cycles until its in-range path was split off from the raises |
| R-ElementPath | `State::vec_get_or_raise`'s in-range path; `vector::append_slot_in_capacity` + `append_capacity` (the two loft#810 refusals' one home); `Stores::append_with`, each branch completing its append; `Store::valid`'s size read under `debug_assertions` only | read: the compare taking `<=` (the out-of-range cell), a fixed element size (the struct and text cells abort).  Append: the length derived with a fixed size (the i32 cell), a full vector taken as fitting (a store bounds abort), the fast path writing no element (the integer cell).  The raw read past the store still refusing: its unit test, falsified by removing the field read's bound (2026-10-01) | dropping the size read alone made the push ops SLOWER (f32 append +8 % cycles with −4 % instructions): it changed what LLVM inlined, and a non-inlined helper's `DbRef` answer came back through memory as narrow stores re-read as one wide load.  Found with `ld_blocks.store_forward` (22 M against 2 M), after two source readings that named the wrong helper — the event has no precise sampling here, so its samples skid.  Outlining the strict-store report from `Stores::store`/`store_mut` was tried and measured slower on every bench (more callers inlined it); kept only for the append ops (`append_store`) |
| R-InlineLeaf | `leaf_inline::admit` (the scalar-leaf shape), `expand` (literal and plain-variable arguments in place, others bound in order), `fold_int` (checked, never regrouped) | a reassigned parameter substituted: the run panics in `remap`; the arguments bound in reverse: the order cell reads `[2,1]`; an overflowing fold: the unit test (2026-10-01) | priced at 72× from hand-written forms, built at 79×: the hand form had folded `73856093 ^ … ^ 584449537` into one constant by regrouping, which the per-result sentinel test makes inexact |
| R-MaskRange | `Reduce::masks` (statement-by-statement facts inside an inlined body), `redundant_mask` (`m = 2^k - 1`), `Reduce::coalesce`; `range::op_range`'s `OpEorInt` clause | a mask that is not all ones removed: `part(k)` reads 5; the fallback dropped without the non-null proof: the null-seed hash reads null; xor ranged with a negative operand: `negxor` reads -2 (2026-10-01) | the proposal claimed both reductions held inside `seed_hash` without inlining; the fallback does not (a parameter may hold the sentinel), and `(R-Range)`'s flow-insensitive facts refuse a local that reads itself, so the facts are carried through the body instead |
| R-SingleUse | `Reduce::single_use` on an inlined body's last assignment and its result | two reads substituted: the run panics on the stale local (2026-10-01) | — |
| R-ScaleFold | `Reduce::scale`, `scale_folds` | a scale by a non-power of two: `third(9)` reads `…568` for `…577` (2026-10-01); `third(10)` could not catch it, its two forms agree | the first cell picked agreed in both forms — a guard for a rounding rule needs an input where the forms differ, found by search |
| R-FastTable | `create::generate_code_into` (two tables, `get_stack_m::<F, _>` respelling), `State::op_table`, the lean loop | the fast table in every run: `state::fast_table_tests` red; the direct push planted into the checked table: the census share fell from 92.6 % to 6.6 % (2026-10-01) | the first census guard asserted a byte count over a threshold and passed both plants: `State` methods push through `put_stack` too, so only the SHARE separates; and the census runs in the full loop, which never dispatched the fast table, so the table choice needed its own pin.  A `const` table has no fixed address, so the pin needed `static` tables |
| R-FrameHeadroom | `State::record_frame_headroom`, `push_frame` (every frame push: 15 sites routed), `put_stack_m::<true, _>`; the table in `WorkerProgram` and `ParallelCtx` | under `LOFT_HEADROOM_VERIFY=1`: `push_frame` making no room fails the room-in-buffer check; the height recorded as the margin alone fails the position check; a frame pushed around `push_frame` fails the source scan (2026-10-01) | the value cells passed both runtime plants: the store grows by 7/3, so a push past the room landed in slack.  The falsifier came from the stop flag: held set, the cold path runs after every op at no cost when off |
| R-ForwardWalk | `forward_walk::match_walk` (the forward walk's lowered shape, a scalar read over the element accepted), `check_body`, `rewrite_in` | the `&` clause dropped: `both(w, w)` reads 2 rounds for 5 — a silent wrong answer; the read-only clause dropped: the pin sees `handed_on` rewritten (2026-10-02) | the first matcher accepted only a bare element reference and so missed every scalar walk, the commonest kind; and it declined a walk appending to a local because the scope pass had promoted that local to a result-buffer parameter — the alias question is about `&` parameters only.  The index-assignment clause is defensive: no loft source assigns `p#index` (`#remove` names the vector, which the read-only clause declines), so no plant can fail on it alone |
| R-SameRead | `same_read::discharge`, `same` (positions ignored), `pure` (every call a pure operator), `bind_statement` | the default ignored: `two_defaults` reads 11 for 12; any call taken as pure: `written_between` reads 198 for 103 — both silent (2026-10-02) | bound nothing at first: the two `v[i]?` of one statement carry different source positions, so exact equality failed; and it only looked at assignments, while the commonest site is a function's result expression |
| R-InlineLeaf (tuple clause) | `leaf_inline::admit`'s result clause, `pure` and `remap` over a tuple literal | — : the plant admitting any tuple passed, because a tuple holding heap is a record type (`__tuple<…>`) the scalar-result check already declines (2026-10-02) | the cell written to falsify the all-scalars condition could not: the language never types such a tuple `Type::Tuple`; the condition stays as a defence and is recorded as one |
| R-CtorLiteral | `ctor_literal::admit` (the literal group, scalar fields over scalar parameters), `expand` / `literal_pushes` (a vector literal's pushes retargeted at the field), `buffer_only_pooled` + `retire_buffer` (the pooled buffer's prep and frees) | the literal's first push dropped: c1 red on both backends (2026-10-08) | — |
| R-LoopRecord (refill clause) | `hoist::loop_record_zeros` (every mint heads a complete group; every vector-field zero is a group's) and the `(R-RefillBuffer)` empty in `emit.rs` | the empties not emitted: every VALUE holds and the live-record count grows two records a pass (r6, c9) — the literal's zero strands the kept vector in its store (2026-10-08) | the value cells alone: a stranded vector answers nothing wrong, so the guard carries a live-record cell |
| R-InRange | `in_range::match_range`, `record_discharge` (the bare element read of `v` at `i`), `forward_walk::check_body` | another vector's read accepted: `another_vector` reads null for 15; the body check dropped: the cleared vector reads null for 10 — both silent (2026-10-02); a scalar discharge accepted: passes, the shape excludes it first | priced first over scalars too (`index_read` −20 %) — not exact: a scalar `v[i]?` also discharges a null VALUE, so the rule shrank to record elements before it was built |
| R-SingleUse (statement clause) | `single_use::rewrite_program`: a `_comp_N` temporary assigned once, read once, pure, read after pure operands only | any element taken as pure: the element with a side effect moved, the pin red (2026-10-02) | fired nowhere at first, twice over: a comprehension's statements sit in a `Loop` node, which the walk did not visit, and the element value under its block is `Span`-wrapped |
| R-DischargeInto | `discharge_into::discharge_into` (the shape and the two target clauses), the single-use count in `rewrite_in` | the null test inverted reads `[]` for `[yy]`; the default dropped reads `[null]` — both silent (2026-10-02); the target-in-read, target-in-default and single-use clauses: planted away, all green — DEFENSIVE, the parser routes an assignment naming its target through a work buffer | `tests/text_borrow.rs` pinned the discharge TEMP as a `&str`; with the temp gone the pin asks the temps of the `LOFT_NO_DISCHARGE_INTO=1` emission and the locals of both |
| R-KeyList | `Stores::get_keys` (cache or compute), `determine_keys_for` (fills it), `State::stack_keys` (copies the kinds out) | a stale cache (every key as text): the keyed bench panics in `allocation.rs`; under `LOFT_KEY_LIST_VERIFY=1` it stops at the first lookup naming `hash<E[id]>` (2026-10-02) | priced at ~7 % from the profile, measured −15 to −22 % on the keyed lookups: the per-call walk through `key_contents_for_field` cost more than its own frames showed.  Built EAGER first (filled in `determine_keys_for`): +171 front-end allocations on the tiny program, the frontend pin red, and the schema round-trip red (a reloaded schema had not filled it) — made lazy (`OnceLock`, reset by `determine_keys_for`) and equality-blind, as `TypeFacts` is |
| R-ByteCopy (vector clause) | `byte_copy::guarded`'s range guard (`0 <= off`, `lo <= hi`, `hi <= len(v) - off`) before `OpSliceVector` | the upper test struck: v5's length reads 3 for 5 on both backends (the slice clamps where the loop pushed defaults) | — |
| R-TextRun | `text_run::inert` (the statements between the bytes and the text read may not write a variable the guard reads) and `replace_read`'s guarded `Set` | the protected-variable test struck: t6 reads "world" for "hello" on both backends | — |
| R-RepeatRun | `repeat_run`'s fill total (`OpAppendCopy` handed the run's count) | the fill handed `count - 1`: r1's matrix reads 15 elements for 16 on both backends | — |
| R-WorkerLean | `State::worker_lean_ok` (admission) and `lean_stop(true)` (a worker's stop leaves the fault takeover to the main loop) | the worker ignoring its stop flag: `runtime_errors::i1056…` and placement_parity's two library-fault cells red on `--interpret` | the remembered lookup answering its stored definition for any position: green, because every `par` loop builds fresh worker States, so a worker State only ever enters one function |
| R-StartStep | `start_step` (the iterator's shape), `State::start_step_holds` (the rest of the loop) and `gen_rotated_loop`'s seed with its entry past `pre` | the entry on the step: s1 reads 39 for 42; `continue`s patched past the step, or the seed dropped: s2 / s1 never end (timeout) | the rest-of-loop check struck: green, because a body reading the index reads the right value either way and no spelling writes it |

## Attempts and closures

- **2026-10-04 — `(R-Cold)`'s fold clause.**  bench/10_sort's insertion sort reads
  `arr[j - 1]` in its test and again in its body.  LLVM merged the two in-range loads, but
  the outlined re-derivation (`get_elem_hoisted_cold`, a call into the rlib that LLVM must
  assume writes) stayed twice, which kept the inner loop at 17 instructions a step with the
  store's lock flag tested inside.  With every index resolved inline (`vector::elem_index`)
  the loop is 13 instructions and the flag is unswitched out: 1.905 → 1.64 M ns per op
  (−14 %), hash unchanged — the price the hand-merged read had shown (1.94 → 1.62 M), now
  reached with no emitter rule, for every repeated element read.  An interleaved A/B of the
  two rlibs over lanes 01–16 (same emitted Rust, five runs each) moved sort −13.8 %,
  bfs_flow −9.3 %, collatz −8.4 % and nothing else past noise, except `join`: the text
  reader folded the same way cost it +14 % (8.47 → 9.67 µs), and +10 % with the hot test
  kept first and only the negative arm inline behind `cold_path()`.  The text reader keeps
  `text_elem_cold`; the clause names the exception.

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

## Deviations carried by rewrites.md, @PLN211 arc C′

### D-phase-4 — CLOSED 2026-11-06 (@PLN211 c4-view S3) — `(R-ElemFirst)`'s view-exit form stays in both writers
- **Violates:** `R-Phase` (one place: no writer clause for a form the IR does not take)
- **Where:** `generation::emit` (the element-first statement handling), `c::body::elem_first` —
  both now find only the pairs the IR declines: a temp whose function also returns it inside
  its record, which `element_first` pairs only with the view plan (`ValueRecords::views`).
- **Effect:** the interpreter keeps the temp-store build and the copy for that form; on the
  benches neither writer applies it (no bench pair needs the plan), in the cells it is
  `164-element-place.loft` g1, g2, g4, g10, g16 (the `parse_circle` shape), g20.
- **Removal:** waits for D-phase-5: once the IR spells a view leaf, the exit is a tuple element
  naming the temp and the pass takes the form with no plan (`plans/211-rule-phases/c4-view-plan.md`
  S3), and both writers' clauses go.
- **Closed by:** c4-view S1 made the view leaf an IR tuple element, so `opt::elem_first` pairs the view-exit temp with no plan; S3 deleted both writers' clauses (`src/c/body/elem_first.rs`, `generation::emit`'s element-first statement handling) with byte-identical emission over the corpus.


### D-phase-3 — CLOSED 2026-11-04 (@PLN211 arc C′3) — `(R-KeyedEntry)`'s subscript clause stays in both writers
- **Violates:** `R-Phase` (one place: no writer clause for a form the IR does not take)
- **Where:** `generation::emit` (`output_keyed_place`'s subscript clause), the C's keyed-place arm
- **Effect:** the interpreter claims, writes and files on `coll[k] = T{…}` where the writers
  overwrite the entry in place.
- **Removal:** the IR form, `opt::keyed_entry`'s staging for one more host (arc C′3).
- **Closed by:** `opt::keyed_entry::subscript_replacement` — the subscript clause in the IR (the literal staged, `keyed_entry_ints(coll, tp, 65535, k…)` into a fresh entry variable, the non-key fields written), the analysis `keyed_place` moved from `lower` to `opt`; both writers' subscript branches deleted, `R-KeyedEntry/subscript` applied by `ir`.  `(R-InPlaceLiteral)`'s keyed clause (the non-heap-free cases) stays the writers'.

### D-phase-2 — CLOSED 2026-11-04 (@PLN211 arc C′2) — buffer ownership is derived per consumer
- **Violates:** `R-Phase` (one place: no decision recomputed per consumer)
- **Where:** `opt::loop_buffer::rewrite_program` (five rules' analyses to leave their buffers),
  `generation::Output` (the loop-buffer exclusions, `hoist_owned`), the C's own reads
- **Effect:** none while the derivations agree; a rule added to one list and not another would
  hand one buffer to two rewrites.
- **Removal:** one `opt::hoist::buffer_owner` answering the owning rule per witness (arc C′2).
- **Closed by:** `opt::buffer_owner::owners` — the one answer, per witness, of the rule that owns a buffer, in a stated priority (inline buffer, hoisted literal, element-first pair, move host, adopted witness); `opt::loop_buffer` reads it, and rustc's own loop-buffer exclusions went with its clause.  rustc's `kept.rs` keeps a coarser function-level check (a kept twin is not built where an inline buffer or an element-first pair applies) for `(R-Kept)`, a rule that stays in rustc for a stated reason.  The census unchanged.

### D-phase-1 — CLOSED 2026-11-04 (@PLN211 arc C′1) — the C recognises the loop buffer's IR form
- **Violates:** `R-Phase` (one place: no writer compensation)
- **Where:** `c::body::emptied_either_way`, called by `known_empty` for `(R-PushFill)`'s run clause
- **Effect:** none on values; the C keeps its push-run reservation after `(R-LoopBuffer)`'s
  `if OpRefIsNull … else OpClearVector` declaration.
- **Removal:** a front fact in `opt` — a vector slot is empty after a statement that empties it
  on every path — read by every writer (arc C′1).
- **Closed by:** `opt::empty_slot` — `known_empty` moved to the front, a slot emptied by a statement that empties it on every path (its zero, a clear of it, an `if` whose arms both do); the C's function is gone, the C's `R-PushFill/run` census unchanged.

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

- Together 23.5 → 12.9 µs on the `char_walk` row (4.6× → ~2.5×); the two checked accumulator adds of the body are what remains, @C120's territory.

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

- (Every rewrite is switchable and falsifiable) `(@C122, owner 2026-09-15.)`
- (A record view carries its address) `**In words.** 2026-09-18, the drawing library's crossing loop`
- (A stdlib one-op wrapper is its op) `and, since 2026-09-22 (@PLN158 round 3), a pure`
- (A minted element is a record no holder can name) `*The own-buffer mint (2026-09-27):*`
- (A minted element is a record no holder can name) `*The field clause (2026-09-21, @PLN158 R1):*`
- (A record append emits through its push header) `*The heap clause (2026-09-18):*`
- (A mint group outside any held header binds its own) `@PLN157 (2026-09-18).`
- (An integer operator whose result provably fits emits the plain operator) `**The type clause, in words.** 2026-09-22, the @C129/@C120`
- (An integer operator whose result provably fits emits the plain operator) `**In words.** 2026-09-20. @C120 declined`
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

## 2026-10-05 — `(R-RefillText)` proposed and built

Priced first on `msg_ping` (x86-64, 17.4 → 4.78 ms by hand), then built in the five steps the
proposal named: `Store::refill_str` with unit tests over its three cases; the census guard,
whose planted defect (the text release dropped, nothing refilling) read 31 → 3001 records
on both backends before any rewrite existed; the admission traced and pinned while it
emitted nothing; the twin and the site behind `LOFT_NO_REFILL_TEXT`, falsified by the twin
releasing nothing (native 31 → 3001) and by (b)'s whole-body checks removed (native 11 → 1001);
then the row, 18.03 → 4.89 ms.  The first trace declined `msg_ping` itself as "an exit that is
not the literal": a literal with no work text is lowered as `return { …; b }`, which the exit
check then learned.  An earlier draft pinned the text clause on `(R-RefillBuffer)`'s return
buffer; the release removed is the pool's `OpClear` (`@FR-H-ClearRelease`).  Found on the
way: a text field reassigned on a live record leaks the text it replaces, on both backends
(loft#1873) — the same slot invariant, at every `OpSetText` site.

## 2026-10-08 — a constructor's call is its literal

`(R-CtorLiteral)` and `(R-LoopRecord)`'s refill clause, from `rustc-vs-twin.md`'s first
mechanism (a copy where Rust moves).  hex_recover `forms_upto` built each candidate's two
vectors in pooled buffers and `form_new` copied them into its `Form`.  Priced by hand
(`bench/portal/hand_price.sh`, cycles per op on an E-core, the n=12 minus n=2 difference):
inlining `form_new` alone −2 %, building the vectors in the fields −31 %.  A runtime fast path
in `Stores::vector_add` (no-heap elements, room in a distinct destination: one block copy) was
priced first, since a runtime fix reaches every program: it bought −4 % here, so the copy
routine was not where the prize was.  Built, the two rules give `forms_upto` −24.5 % cycles on
`--native-release` (4.95M → 3.74M per op, hash 26284275 unchanged) and −54 % on the
interpreter (13.0 → 5.9 ms per op).  The literal form needed the refill clause first: written
directly in a loop, `Fm { lens: [..] }` minted a store a pass and ran 27 % SLOWER than the call,
which reused a pooled return buffer; with the clause it runs −47 %.
The rewrite census caught the rule's first condition set too wide: it also took scalar-only
constructors, and hex_form's `plan_new(0, 0, 20, 16, 0, false)` — a library `pub fn` the IR
value-record pass may not reshape, which the native `(R-ValueLocal)` then carried as a register
tuple — became a store-minted literal (`R-ValueLocal` −1 in six libraries).  The rule now needs
a vector-literal argument; c12 pins the scalar case on the trace.

Widening the condition to "the record holds a vector" (dryopea's `undo_entry_empty()`, a
constructor with no arguments) measured the next trap: inside a loop whose local is NOT a loop
record, the literal mints a store every pass where the call reused a pooled one — +33 % cycles on
a probe whose local is copied out (`last = ef`), +18 % on `truncate_to` before the refill clause
admitted `OpNewRecord`/`OpFinishRecord` on the record's own vector fields.  The rule therefore
reads the loop records of the rewritten body and restores the call where the local is not one
(c13).  With the gate, `truncate_to` and `reload_and_record` move −0.6 % cycles (neutral: the
call form already refilled a kept store), `forms_upto` keeps its gain.  `truncate_to`'s cost is
the copy of each entry into the timeline and the store allocator's per-claim cost (claim 13 %,
copy walk 13 %, frees 8 %), not the entry's own construction.

## 2026-10-08 — `(R-Mint)`'s text clause

A loop appending records with text fields while reading another vector declined its holds on
the element's text write.  Admitted as a claim in the mint's own store: zttext `invert` −13.5 %,
`materialise` −7.7 %, arguments `parse` −1.7 % cycles.  The first build took an element
`(R-RefillText)` builds into a KEPT slot as fresh too: every value held and two refill guards'
live-record counts grew (`a-buffer-of-text-records-is-refilled-where-it-is`,
`a-kept-buffers-elements-are-refilled-in-their-slots`) — the kept slot's old text was never
released.  Kept elements are now named to the hoist (`HoistOwned::kept_elems`) and decline.  Five
emission pins had recorded the old decline as a prediction (`discharge_set` c3, `record_push`
m7, `complete_write` c4, `copy_in_place` c4, `enum_record` e3); each cell answered the same with
the clause on and off under `LOFT_HOIST_VERIFY=1`, `LOFT_POISON=1` and `LOFT_POISON_CLAIM=1`
before its prediction moved.

## 2026-10-13 — `(R-WorkBuffer)`'s result clause

The unit the worklist carried as S1c, proposed here as "the pooled clause, for
`(R-RefillText)`'s collection clause": a return buffer whose elements the callee keeps is only
kept across calls the caller's buffer survives, and zttext's `token_width` — called per token
— minted it per activation.  Re-priced on the emission of `40baa214d` with one static buffer in
`token_width`: `flow_layout_full` 14.36 → 11.60 ms (−19 %; the −8 % priced before predates
`(R-FnRefValue)`).  Built as a clause of `(R-WorkBuffer)` and not of `(R-RefillText)`: what the
caller holds is a buffer whatever the callee does with it, and the rule's transitive clause and
pool already carry one up the call tree.  Built: 14.35 → 11.26 ms on a performance core (−21.5 %),
25.3 → 20.4 ms on an efficiency core, hash `eb888d85` unchanged; `place_line`'s three run buffers
went with it.  Every other row of the lane is level and its functions are emitted byte for byte
as before.

Three things the build measured.  The null road's backing is minted from the ELEMENT type
(`vector_db` takes it): minted from the vector type it registered a second wrapper type per
buffer, and `(R-RefillBuffer)`'s live test then never matched.  A view bound inside the value
that yields it (`r = runs[i]?` binds its temporary inside the block whose value it is) is found
one round after the bind that needs it, so a round that found a view is run again whatever it
answered.  And without the runs-again condition a reader called once from `main` parked its
result's store there for the whole run — sound, paid for by nothing, and not where the source
ends the value; clause (d) is that measurement.

Two rules met the clause in the census (`make rewrite-census`), and both were settled where
they stand.  `(R-RetAdopt)` read "the first local bound to a `__vdb` backing" as the result
local's own bind; a work buffer's null road binds a PARAMETER to a backing ahead of it, so a
function with one lost its adoption (markdown `extract_headings` under this clause — and
12_drawing's `lock_ribbons` before it, whose four work buffers had kept it from adopting since
the rule shipped).  The gate skips a parameter's bind (`hoist::ret_adopt`; pin
`tests/retbuf_adopt.rs` `n_tagged`).  And a constant `split` whose vector the native build
never makes was handed a caller's buffer nothing fills, minted once per caller where the
function had minted none: the clause declines it (`hoist::split_call`).

## 2026-10-13 — `(R-WorkBuffer)`'s function-value clause (loft#2001)

Asked what could go wrong if the rule said what the code did — a function passed by name is
promoted, although the decline list read "a function whose address is taken" — the routes were
probed: by name, a local, a record field, a vector element; a leaf, a forwarder, a value-record
answer, a work buffer beside a return buffer.  Three things came out.  The address test sees a
`FnRef` node only, and a function passed by name is a bare definition number.  The backends
differed in what the call hands: the interpreter one store per hidden parameter per call,
`--native` the null sentinel — so a forwarder with a loop, reached by value, minted a store per
leaf call (4 million leaf calls: 0.38 s by value against 0.11 s direct, 0.35 s with the whole
rule off).  And where the function value's signature answers a vector, the dispatch handed
every hidden vector parameter the return buffer's store, a work buffer included: `tagged`'s
scratch `ids` and its result were one vector record — `3:5/1005|0/0|1010/2|` for
`2:5/10|1005/1|` with no diagnostic, a panic on a scratch value read as a record number in a
variant.  The first probe of that shape answered right by accident: its scratch element equalled
the record field it overwrote.  Built: the frame lends (`FnRefWork`); the by-value forwarder runs
0.11 s.

## 2026-10-13 — `(R-Header)`'s apart clause

The worklist's X2, priced first by hand on cbor `encode_map`'s emit loop (−20 %: −16 % from
`ranks`, `koff`, `klen`, −6.5 % from `keybuf`).  The plan was a write set for a callee that
grows its vector return buffer, taken by fixpoint over a recursion, and a store-identity test
for `keybuf`.  Reading `(R-Header)`'s function clause — "growth of OTHER stores is no concern of
a header" — gave the smaller statement: a store nothing in the loop is handed cannot be
reached by what the loop calls, so no callee needs a verdict.  Built: 10.52 → 8.42 ms, the
hand price exactly, all four vectors.

The first build kept a header for a vector a CLOSURE pushed to (`c6 303` where the file says
622; the verifier named the two lengths): the chain end of a local vector is its `__vdb`
witness, which `StoreFacts::fresh` takes as its own before asking whether anything is
captured, and the capture is recorded on the local.  The clause now drops a store when ANY
name in its alias set is captured or a `&` link.  A text value read from an element is no
handle on the store and is not counted as a mention (`size(names[i] ?? "")` binds one).

Measured wrong on the way: two binaries run side by side on one core through
`paste <(a) <(b)` read a row +15 % slower whose code was byte for byte the same; run one
after the other the row is level.

## 2026-10-14 — `(R-FileHold)` and `(R-InlineLeaf)`'s reader clause

The worklist's X8, priced on the crawler lane's `read_i16_vec` (`while i < n { out +=
[read_one_i16(bf)]; … }`, 150 000 values a load): 1.47 → 0.84 ms by hand, with bare locals
and with a guard value whose drop writes the record — the same figure, so the guard was built
and no exit is enumerated.  Built: 1.47 → 0.84 ms, the hand price, hash unchanged.

Priced beside it and not built: the cursor kept in the HANDLE, the record's two fields left
alone while a handle is open — 1.47 → 1.01 ms, for every scalar read on both backends and
with no gate.  It moves where `f#next` and `f#index` are read from (two parser forms, the
stdlib's `position` and `seek`, twenty-odd runtime sites across the two backends and the
host bridge), which is a change of the cursor's model; the hold changes none of it.

Found on the way, in a cell first written with `g = f` (loft#2005): a copy of a `File` record made
after its first read shares the handle and keeps its own `#next`, and the two backends then
read different bytes through the copy — the interpreter seeks to the copy's own position
(`-47`), native reads at the handle's (`-44`) and reports the copy's.  `heap.md` states one
handle and one position.  Independent of the hold, which declines a loop naming two `File`
variables; the cell names one record through two parameters instead.

loft#2005 closed the other way the same day, which retired the handle-cursor price: on native
a copy made BEFORE the first read already had a cursor of its own (each record opens its own
handle), so "one position per handle" would have made `g = f` mean two things by when it ran.
Each record keeps its position; native's read now places the handle at `#next` first
(`LoftFile::place`, a compare when it is there), as writes, seeks and the interpreter did.

## 2026-10-21 — `(R-HandOver)`: the truncating take, built and removed

Beside the rotate clause, the take for `truncate_to(h, c); h.entries += [e]` was built: before
the truncation, element c's vector blocks were taken (its slots set to 0, so the keep freed
nothing of them) and the copy of `e` wrote into them (`hand_over_detach` / `hand_over_copy`).
Measured on dryopea `truncate_to` against the rotate alone: −16.6 % with it, −17.1 % without
(the routine looped alone: 0.87 s against 0.84 s, with the bound test emitted inline so the
call ran only on the 30 truncating pushes of 300).  `history_push`'s own code grew from 13.3 %
to 18.5 % of the profile to carry the take and the conditional copy, and the take saved only a
claim and a free: the exact-size free lists already hand the freed block to the next claim.
Removed; the general rule's status line records the reason.

While bisecting it, `Store::get_u32_raw` was found split from its `#[inline]` by a function
inserted between the attribute and the item: a +24 % slowdown on `truncate_to` from one
out-of-line raw read, found by a flat profile of the two runtimes (8.6 % of samples in a
symbol the fast build does not have).

## (R-LoopRecord) in the writers

Until 2026-11-04 `(R-LoopRecord)` was two writer clauses; @PLN211 moved it into the IR
(`opt::loop_record`) and deleted both.  What they were, as `rewrites.md` stated them:

Sites: `hoist::loop_records`, the `Value::Loop`
emission in `emit.rs` (prelude and postlude), `output_block` (the two dropped statements),
`ops::misc_ops` (the kept-record mint).  Cells `tests/scripts/157-loop-record.loft` l1–l15,
pins `tests/loop_record.rs`.  The refill clause: a kept record's vectors keep their records
and capacity, so a literal of vectors refills them where the plain form claimed new ones
every pass.  Sites `hoist::loop_record_zeros` (the groups and their zeros) and the
`(R-RefillBuffer)` empty in `emit.rs`; cells `tests/scripts/157-a-loop-record-keeps-its-vectors.loft`,
pins `tests/ctor_literal.rs`.

**On the C (a port).**  The C writer does not ask
`crate::opt::loop_records::loop_records`, so a record literal in a loop mints a store, claims
its vectors and frees it every pass.  The port is the same three moves in the C's spelling: the
mint is taken only where the local holds no store (`if (r.store_nr == 0xFFFF)`), a refill
record's literal zero of a vector field (`loft_set_int4(r, off, 0)`, in `field_zeros`) becomes
`loft_vec_clear` of that field on a kept pass — length 0, block kept — and the end-of-body
free moves after the loop.  Priced on hex_recover's `forms_upto` (gcc), on top of the run's
header clause: 0.17 s → 0.11 s, **−35 %**; together 0.30 → 0.11 s (0.37×, ≈ 6.3× → ≈ 2.3×
the Rust twin).  Resetting the store (`loft_reset_pool_root`) instead measured NOTHING — the
cost is the two vectors' claims, not the store — and so did skipping the run's reservation on
a kept pass and hinting `head_step` inline.  Built to the hand form (0.12 s against 0.11:
the fresh flag below): `src/c/body/loop_record.rs` reads the shared analysis once per
function; the mint is `((v.store_nr == 0xFFFF) ? (mint, lrf = 1) : (lrf = 0))`, each vector-field
zero `if (lrf) zero; else loft_vec_clear(field)` — a fresh mint keeps its zero, because the C's
claim is not promised to be zeroed — and the declaring block's `Set(v, null)` and end-of-pass
free are skipped by node address, with one free after every copy of the keeping loop
(`loop_stmt`).  The refill records only: a plain no-heap record keeps the plain mint on the C,
where a reset measured nothing.  Bench (A/B, both clauses off against on): hex_recover
`forms_upto` **0.302×** gcc (1045 → 315 µs, ≈ 2.5× the twin), **0.234×** clang (799 → 187 µs,
≈ 1.5×).  Switch `LOFT_NO_LOOP_RECORD_REFILL=1` (and `LOFT_NO_LOOP_RECORD`); census
`R-LoopRecord/refill`.  Guard: `157-a-loop-record-keeps-its-vectors.loft` and
`157-loop-record.loft` now run on the C too (both pass, and under `LOFT_NATIVE_LEAK_CHECK`) —
a kept pass that does not empty its vectors fails r1 with 102280.

## (R-SpecChain) measured

@PLN219, 2026-11 (C ÷ Rust): sum_loop 2.96× → 1.00×, index_read 3.01× → 1.00×, binary_read
3.29× → 1.89×, binary_map 1.78× → 0.84×.  The product clause: with the overflow builtin,
`par_reduce`'s `h * 1103515245` cost a high multiply on its chain (1.90× → 2.58×); keeping
`loft_mul` for a product by a literal, 1.85×.

## `(R-NoThreads)` while the runtime kept thread-locals

Until @PLN226 removed `LOFT_TLS`, `LOFT_NO_THREADS` also made the runtime's store table, frame
stack and pools plain statics instead of thread-locals.  On macOS every store access then
reached a thread-local through a call (`_tlv_get_addr`, 13 % of a record-building recursion's
profile), and the rule's gain was that cost: on Apple silicon, clang, 21_tree `build_rec`
−14.5 %, every routine of @PLN219's hand pricing −10 % to −21 %.  With the state in the store
context (`loft_cx`) the rule keeps only the thread it does not start.

## `(R-DecodeView)` on the C

Both clauses were built first in the rustc writer, which measured them on pluginabi
`check_request` (over-9x.md, one core): the view clause 2.57 → 1.64 ms per op (−36 %, against
−46 % priced), the structure clause 4.66× → 0.94× of the Rust twin (1.58 → 0.32 ms against the
twin's 0.34 ms; the hand price of the same form was 0.30 ms), every hash unchanged.  The writer
went with @PLN225, and the C took the emission over on 2026-11-25 (`src/c/body/decode_view.rs`,
`src/c/body/decode_structure.rs`).

Measured on the C, `--ship` (`bench/stats.py --routine check_request --lanes rust,cship`, five
samples, one pinned core), 2026-11-25: the rule off 290.7 µs per op against the twin's
242.5 µs (1.20×), the view clause alone 290.3 µs (1.17×), both clauses 288.4 µs (1.19×), hash
unchanged in every lane.  The lane does not move because no root is admitted: the cbor
`read_value` the bench links reads a decoded byte string, which instance 1 declines.  The
guard's own decoder, 200 000 lookups of one of three keys in its F1 frame (`--ship`, five runs
each, one core): 100 ms with the rule off, 92 ms with the view clause alone, 46 ms with both,
the answers the same.

## Retired with the rustc writer (@PLN225 R5)

Rules only the rustc writer applied.  Over the whole corpus the C backend admitted none of them
and its emitted C did not move when their decision code was removed, so they were removed with
the writer ([plans/225-retire-rustc-writer/rewrite-audit.md](../plans/225-retire-rustc-writer/rewrite-audit.md)).
Each is recorded as `rewrites.md` last stated it.

### `(R-FoldCompare)`

      (R-FoldCompare) a text predicate P ∈ { ==, !=, starts_with, ends_with, contains }
                     one of whose operands is a case FOLD F(t), F ∈ { to_lowercase,
                     to_uppercase }, where the fold's value has that ONE mention (a
                     synthesised temporary, or a local whose every binding is such a
                     fold or `fold + literal` and whose every other mention is a text
                     VALUE read as an operand of such a P), is answered WITHOUT
                     building the folded text: over the span P inspects, F is applied
                     byte by byte while every byte of that span and of the other
                     operand is ASCII; the FIRST non-ASCII byte met in either operand
                     takes the written form (the fold built, P applied) for that
                     evaluation.  A `+ literal` on the folded operand is compared as
                     the fold followed by the literal.  The other operand is read as
                     written.  Any other mention of the fold — a bind read elsewhere,
                     a return, a link, a capture, a non-text-value op — declines.

**In words.** `h.to_lowercase().starts_with(prefix)` is a question, not a text: the fold
exists only to be compared.  Unicode case is not byte-local (U+212A KELVIN SIGN lowercases
to ASCII `k`, `İ` lowercases to two scalars, `ß` uppercases to `SS`), so the byte-wise form
is sound only while both operands' inspected bytes are ASCII — one `is_ascii()` over the
span — and the fallback is the program as written, so no answer can change.  Reach: every
case-insensitive lookup in the text-scan class.  Priced on server `header`
(`bench/portal/analysis/over-9x.md`): with the walked line borrowed, 9.43 → 3.52 M ns/op.
Generation time, `--native` first (`src/generation/ops/text_ops.rs`, beside the
`starts_with` emitter; the folding helper beside `lazy_split` in `src/codegen_runtime.rs`);
the IR phase only on broad evidence (`(Perf-Order)`).  Switch `LOFT_NO_FOLD_COMPARE`; trace
`LOFT_TRACE_FOLD_COMPARE` names each admitted predicate and each declined fold.  Falsifier:
hand-computed cells on both backends — ASCII hit and miss, a prefix longer than the line,
the Kelvin sign in the line against an ASCII prefix (must match), `İ` within the span (two-
scalar fold, the written form), `ß` under `to_uppercase` against `SS`, an empty operand, a
fold bound to a local and read twice (declines); the switch A/B over `server`'s bench hash;
`(R-TextBorrow)`'s pins with this rule off.

**BUILT** for a fold that is a synthesised temporary (`codegen_runtime::fold_compare`; the
emitter `FoldCompareEmitter` in `src/generation/ops/text_ops.rs` and `pre_eval` ask ONE
predicate, `fold_compare_site`, which admits only operands with no work to lift, so the
fold's block is neither lifted nor built; guard
`loft-suite/rules/rewrites/R-FoldCompare/a-text-predicate-over-a-case-fold-builds-no-fold.loft`).  Still PROPOSED:
the operand clause below.

      (R-FoldCompare) OPERAND CLAUSE — an operand of an admitted P may be a text LOCAL x
                     that is never built, when ALL of:
                     (1) x has exactly ONE binding, `x = F'(s) + l` or `x = F'(s)`, F' a
                         fold, l a text LITERAL, s a text PARAMETER or LOCAL;
                     (2) every other mention of x is an operand of an admitted P, or x's
                         own release;
                     (3) s holds at every such P the value it held at the binding: no
                         path from the binding to a P writes s — no assignment, no
                         append, no `&s` hand-off, no call handed s's work buffer;
                     (4) the binding and every P that reads x lie in one activation
                         of one function, and that function is not a generator.
                     Then each P reads x as the PAIR (F'(s), l), F'(s) followed by l:
                     on the fast path the ASCII test spans the inspected bytes of
                     s and of l as well as the fold's own operand.  The written form,
                     taken from the first non-ASCII byte, builds F'(s) + l at most
                     ONCE per binding of x (a frame slot filled on first use), so no
                     run builds it more often than the program as written.
                     Declines, keeping x built: a second binding; any other mention —
                     a return, a format, a store, an append, `len`/`size`, an index, a
                     call argument, a capture, a link `&x`; an s that is a field or an
                     element (a store write between binding and P could change it); a
                     non-literal l; a generator.

**In words.**  `prefix = name.to_lowercase() + ":"` exists only to be compared, like the fold
the rule already removes; reading it as the pair it was built from removes the text and
both of its allocations.  (3) is what makes deferring the fold sound: F' is pure, so F'(s)
at the P equals F'(s) at the binding exactly when s did not change in between, and a local
or parameter can be checked for that by its writes, where a field cannot.  The once-per-
binding slot keeps a non-ASCII run from rebuilding x on every P.  Priced on server
`header` (`bench/portal/analysis/over-9x.md`, lever D): −14 %.  Switch and trace are the
rule's own.  Falsifier: hand-computed cells on both backends — the `header` shape; s written
between binding and P (declines); x formatted once besides the compare (declines); x bound
on two branches (declines); s non-ASCII with the P in a loop (the written form, built
once — `LOFT_ALLOC_SITES` counts one build).

### `(R-Leaf)`

      (R-Leaf)       a function whose body calls no user function and no fn-ref (a store
                     bracket, as in (R-LeafChain), is none) — a LEAF — is emitted without a shadow-stack frame push and without a fn-ref
                     buffer guard: it cannot recurse, cannot reach stack_trace / assert /
                     panic, and cannot sit between the frame that pushed a buffer and
                     the one that releases it.  The live-flip check stays.

**In words.** @PLN157 N4.  A runtime fault inside a leaf keeps its exact position
and loses only the innermost frame NAME from the chain.  Switch
`LOFT_NO_LEAF_PRELUDE`.  Site: `Output::is_elidable_leaf` and its use in
`Output::output_function`.  Its IR form, inlining the leaf for both backends, is `(R-InlineLeaf)`.

### `(R-LeafChain)`

      (R-LeafChain)  in the LEAN tier, a function whose whole call tree is FRAMELESS is
                     emitted as a leaf is: every user function it reaches, transitively,
                     has a loft body, none of them lies on a cycle, and none calls a
                     fn-ref, runs `parallel` or yields.  A native user-level callee
                     (no loft body) makes the tree opaque, so the caller keeps its frame —
                     except a store BRACKET the compiler emits around an argument
                     (`set_store_lock`, `protect_store_frees`, `unprotect_store_frees`),
                     which sets a store flag and calls no loft code.
                     The named tiers keep every non-leaf frame.

**In words.** @PLN157 queue row 11.  Such a function cannot be re-entered while it runs, so
the depth cap needs no entry for it: a recursion that passes through it is still counted at
the recursive function's own frame, and the cap's report names that function — the nearest
frame — where the interpreter names the innermost call.  Nothing beneath it can push a fn-ref
buffer, so its buffer guard would always drop empty.  The lean frame carries no name, which is
why the rule is lean-only: in a named tier the frame is also what `stack_trace()` and a
panic's frame block read.  Switch `LOFT_NO_LEAF_CHAIN` (one step finer than
`LOFT_NO_LEAF_PRELUDE`).  Site: `Output::is_frameless_chain`.

### `(R-LitDiv)`

      (R-LitDiv)     a division or remainder by a LITERAL that is neither 0 nor -1 emits
                     as one sentinel test and the plain operator — `if x == MIN { MIN }
                     else { x / k }` — which is the guarded template's exact value: the
                     null a `/` mints comes from a zero divisor, from `MIN / -1`, or from
                     a null operand, and the literal rules out the first two at
                     generation time.  It needs no proof of the dividend, which is what
                     lets it fire on an arithmetic result the proof never trusts; the
                     template's fault note is dropped with it, since it fires only when
                     the result is the sentinel while neither operand is.

### `(R-RebindBuffer)`

#### A rebind hands the displaced store to the call

      (R-RebindBuffer) on `--native`, the rebind `x = f(…)` of an owned local from a
                     direct call that answers a FRESH store (`Own::Owned`) through a
                     hidden buffer local `b` — a compiler local nothing but this call and
                     its scope-exit free names — ADOPTS the result and hands `x`'s
                     previous store to `b`, the buffer of the call's next execution,
                     instead of exchanging the result into `x`'s store
                     (`(H-SwapRebind)`).  Run-time conditions, each falling back to the
                     exchange: the result has a store of its own (not null, not `x`'s),
                     `x` held one, `b` is null or IS the result's store (so no live
                     buffer is dropped), and `x`'s store is not the caller's entry buffer
                     (a return-buffer local).  Static conditions: `x` is not an argument
                     (the return-buffer attribute with its entry witness excepted), not
                     captured, not never-free, not witnessed, not a coroutine field; the
                     rebind sits inside a loop (the handed store is a buffer only if the
                     call runs again); and no local borrowing `x`'s or `b`'s store is named
                     at or after the top-level statement holding the rebind.  At its
                     release `b` PARKS the store it holds where the exchange would have
                     parked its released one (`(R-RefillBuffer)`'s spare), so the stores a
                     function leaves behind for the next mint are the exchange's.

**In words.**  `c = mul(a, c)` in a loop built each result in a buffer, then exchanged the
two stores' contents so that `c` kept its slot — a check of both stores' flags, the swap,
the park of the released store and a take of it at the next mint, every iteration.  Nothing
needs `c`'s SLOT to stay put when nothing else names it: the result is fresh, so `c` can
simply take it, and the store `c` held — whose value the rebind ends — is exactly a buffer
of the right type for the next call, which the refilling callee (`(R-RefillBuffer)`) writes
in place.  The two stores take turns by name instead of by content.  The scope-exit free of
`b` releases whichever one `c` does not hold.  Effect: mesh3d's `mat4_mul` loop 54 → 39 ns
per product (−29 %, priced −29 %).  Switch `LOFT_NO_REBIND_BUFFER=1`, read where the rebind
is emitted; trace `LOFT_TRACE_REBIND_BUFFER=1` names each decline.  Sites:
`Output::rebind_buffer_for`, the call-return arm in `generation/dispatch.rs`, the body
pre-pass that marks the buffers (`Output::rebind_handed`, so a release emitted before the
rebind — an early `return` inside the loop — parks too), and `Stores::park_or_free`.  Guard
`tests/scripts/a-rebind-hands-the-displaced-store-to-the-call.loft`, pin
`tests/rebind_buffer.rs`.

### `(R-TextClear)`

#### An emptied text keeps its buffer

      (R-TextClear)  `x = ""` where x is a declared text LOCAL whose native slot is a
                     `String`, or a `&text` PARAMETER, empties x IN PLACE — the same empty
                     text, the buffer's capacity kept — rather than binding a fresh empty
                     `String`.  The value is the empty text either way; only the
                     capacity differs, which no program can observe (R-Escape).  A first
                     bind (the declaration), a text PARAMETER passed by value, a generator
                     field and a narrow-encoded local keep the fresh `String`.

**In words.**  A work text — a call's text result buffer, a format's work text, an
accumulator — is emptied once per use and refilled.  Bound to a fresh `String` each time,
it dropped its allocation and claimed and regrew a new one on every pass: zttext's
`materialise` paid that per piece, its callee emptying the caller's result buffer through
the `&text` parameter.  Emptied in place the buffer is claimed once per activation.
`materialise` 500 → 206 µs, `seg` 950 → 665 µs on arm64 macOS, hashes unchanged.
Switch `LOFT_NO_TEXT_CLEAR`; census `R-TextClear`, one per emission.  Cells
`loft-suite/attic/scripts/an-emptied-text-keeps-no-old-content.loft` (c1–c6: every cell refills a
text that held MORE than it is refilled with; with the clear replaced by a no-op, c1
reads `abcdeabcdefg…`).  Sites: the two `@FR-R-TextClear` arms of
`Output::output_set_inner`.

### `(R-StreamWalk)`

#### A vector its caller walks once is never built

      (R-StreamWalk)  `v = P(a…); for i in 0..len(v) { … }` — a call of a function P that
                      answers `vector<T>` for a record T, bound to a local that the next
                      statement mentioning it walks once by index — runs as P's STREAM TWIN
                      with the walk's body as its closure: P's own body runs, and each append
                      it would have made hands the values it would have stored in the fields
                      the body reads to the body, at that append.  No vector, no record and no
                      text of an element is built.  Admitted when: P's result vector is
                      mentioned only by its buffer's plumbing, by its appends (a mint, a set of
                      each field from a value, a finish) and by `return`; every field the body
                      reads is set by every append and is an `integer`, `float`, `single` or
                      `text`, whose round trip through the store answers the value stored; the
                      body mentions the vector, the index and the element only as `v[i]?` read
                      for a field; nothing in it leaves the walk (`break`, `continue`,
                      `return`, `yield`); P and the body are QUIET — every write lands in a
                      place the function owns (a local borrowing only what it owns, a buffer
                      it is handed to write), every call is of a quiet function, every
                      function value dispatches only to quiet functions without captures; and
                      neither the body nor a statement between the call and the walk writes a
                      variable the call reads or a local borrowing from one.  The local is
                      otherwise mentioned only by its null init and frees.  Native release
                      build only.

**In words.** The plain form runs P to its end — a buffer refilled, a record claimed and a
text copied in per element — and then reads each element back once.  The twin interleaves
P's steps with the body's, which can change a value only if one side sees the other's
progress.  P is quiet, so it reports, prints and writes nothing the body can read; the
body is quiet and writes nothing the call reads, so P reads at each step what it would
have read before the body ever ran — and P reaches data only through its arguments, which
is why the arguments and every view of them are what the body must leave alone.  The
values handed over are the values the append would have stored, and each admitted field
kind reads back what was stored, so the body reads the same values in the same order.  A
field the body does not read is not handed over, so the twin need not compute it.  The
call moves down to the walk, past statements that write nothing it reads.  The twin is
emitted without loop hoists, since a header it held across its loop would not survive a
body that grows a store.

**BUILT** (`generation::stream_walk` the plan, `generation::stream_walk_emit` the twins and
the walks; `LOFT_NO_STREAM_WALK=1`, `LOFT_TRACE_STREAM_WALK=1`).  zttext's
`flow_layout_full` (`slice_runs` walked by `token_width` and twice by `place_line`):
**4.60× → 2.98× of Rust**, hash unchanged.  Not admitted yet: a walk by `for e in v`, a
narrow integer, enum, boolean or character field, a body that reads the index, a walk
that leaves early.  Guard `tests/scripts/a-walk-of-a-built-vector-streams-its-elements.loft`
(`@falsify-tier: ship`), pin `tests/stream_walk.rs`.

### `(R-ViewReturn)`

#### A lookup's found entry is answered as a view the caller reads in place

      (R-ViewReturn) a function F whose result at some exit is a VIEW V of a sub-record
                     that the parser materialises (a buffer minted, V deep-copied into it
                     — `return e.value` for `e` an element of a parameter's vector) is
                     given a TWIN that answers (address, owned): a materialised exit
                     answers V's address and `false`, every other exit the store it
                     built and `true`.  A call takes the twin, and its caller mints no
                     buffer and frees the result only when owned, when ALL of:
                     (1) F writes no store but those it minted itself, calls no user
                         function, and names its buffer only in its materialised exits;
                     (2) the call's arguments are variables or reads — the stores V
                         lies in are held by the caller across the call's statement;
                     (3) the WINDOW — the statements after the call in its block —
                         writes no store and calls no user function, and reads the
                         result only through getters that copy (a tag, a scalar, a
                         text);
                     (4) the result's variable is bound once and named only in the
                         window; the buffer only in its prep, the call and its frees.
                     Every exit keeps the ownership it had, so a built result (a
                     not-found literal) is freed as before.

**In words.**  `(R-ReturnField)` answers a field of an OWNED local as its store at a position
and declines a parameter, because a view of a parameter's record dangles the moment the
caller frees or grows that record.  Here the referent is the caller's own argument and the
result is read only in the statements right after the call, which change nothing, so the
dangling window is empty by construction.  The twin decides per call site, so a caller that
holds the result longer, or writes in between (the guard's `text_then_clear`), keeps the
copy.  Priced on pluginabi `check_request` (over-9x.md): −10 % — the mint, the deep copy and
its text claim per lookup; `pa_get` is the shape of every `match`-based finder.  The
ownership question it raised is settled by @C122 and @C139: a representation is the
compiler's wherever the program's results are kept.  Switch `LOFT_NO_VIEW_RETURN`; trace
`LOFT_TRACE_VIEW_RETURN` names each call site's admission or declining condition.  Built:
`src/generation/view_return.rs`; guard `loft-suite/rules/rewrites/R-ViewReturn/a-found-entry-is-read-where-it-lies.loft`.

### `(R-MoveAppend)`

#### A dying temporary's elements move into the append that consumes them

      (R-MoveAppend) in `for f in call(…) { V += [f] }` where the call MINTS its result
                     (a borrowed-view return declines), the loop variable's ONLY use
                     after its binding is that one append (a read after it, a second
                     append, an append under a further loop all decline), V is an owned
                     local plain vector of a PLAIN STRUCT element never rebound in the
                     function, and the call's hidden buffer serves nothing else — the
                     buffer is PLACED as a record inside V's own `__vdb` store at the
                     loop (V is in scope there, so its store exists on every path), the
                     append relocates the element's bytes and ZEROES the source when
                     source and destination share that store (anything else keeps the
                     deep copy, which is always correct), and the buffer's free is a
                     record-level release — the deep walk of what the loop did not
                     move, then the record's block — inside the store that lives on.
                     Three lifetime conditions bound the placement.  (1) The placed
                     record dies WITH ITS HOST STORE: the release is injected before
                     EVERY free of the host `__vdb` — the early dead-after-last-read
                     site as much as scope end, since a later store can recycle a dead
                     host's slot and a deferred release then deletes a record inside
                     whatever owns it — and the buffer attr's own scope-end free is
                     the null-guarded backstop that covers an ADOPTED `__retbuf` host,
                     which no in-function site frees.  Between host deaths an
                     enclosing loop REUSES the placement (the callee's entry clear
                     resets its length).  (2) A host store CLEARED whole re-arms the
                     guard: `OpDatabase`'s reuse arm reclaims the placement with the
                     clear, so the buffer var is nulled at that site or the next call
                     delivers into unclaimed bytes.  (3) The buffer's type must be V's
                     ELEMENT's own wrapper — the lookup is by name, and a struct named
                     like a stdlib type variable finds the GENERIC template whose
                     field still carries the typevar, which a record-level walk
                     misreads (such a name declines the pairing).

**In words.** @PLN157 § V-j.  The deep copy's cost was never the bytes: each appended
element re-CLAIMED its inner vectors in the destination's store, copied them, and freed
the source's — two claims and two frees per element (23 % of the `fronds` row).  Placing
the buffer where the elements must end up makes the callee's own delivery put them there,
and the append is then a shallow relocation whose heap handles never change store.  The
zeroed source is what the temporary's clear and free are allowed to walk.  The gates are
under-approximations on purpose; the one that carries values is use-after — falsified by
removing it, the read-after-append cell answers the zeroed source (`3 409 45` →
`3 409 0`).  Composes with `(R-PushRec)`: a no-heap element's slot comes from the
push header and the move lands in it (the c16 cell).  Switch `LOFT_NO_MOVE_APPEND`; the value
cells run under `LOFT_POISON=1` and both leak checks.  Sites: `hoist::move_appends`,
`hoist::pair_for_block`, `Output::move_pair_for_block`, `Output::active_move_pair`, the
placement in `Output::output_block`, the move arm in `OpCopyRecordEmitter`, the record
free in `OpFreeRefEmitter`, the null decl in `Output::emit_null_dbref`,
`Stores::place_record_in`, `Stores::move_record_shallow`, `Stores::free_record_in`.

### `(R-FaultFlag)`

#### A `for` statement runs fast under a fault flag, and as written when it faults

      (R-FaultFlag)  a `for` statement whose every node is a literal, a local, a `Set` of a
                     scalar local nothing takes the address of, a block, an `if`, a loop, a
                     `break` / `continue` that stays inside it, or an OPERATOR whose `#rust`
                     template touches no state once its recoverable raise is taken out, and
                     which holds a DISCHARGE ON A RECURRENCE — one whose tested value
                     depends, through the statement's assignments, on a local its result
                     is written into — runs a fast copy first.  In it a
                     `??` / `?` discharge (`if not-null(n) { n } else { d }`) sets a flag where
                     `n` is null and answers `n`; a template's `s.raise_recoverable(k)` sets the
                     flag instead of raising.  Each fault form answers exactly the exact form's
                     value while the flag stays clear.  An integer operator keeps its checked
                     form: its null tests are branches the processor predicts, off the value
                     path, and a flag computed per operator measured slower (`02_sum_loop`
                     1.81× → 2.24×).  A store READ (a getter or length, by name) and a call to a
                     loft function whose own body passes the walk without a raise are admitted
                     in a statement that writes no store.
                     After the copy a set flag restores every written local declared before the
                     statement and runs the statement as written, so a fault is answered and
                     reported once, by the exact statement.  The copy is skipped under
                     `--dev-soft-halt` (an overflow REPORTS there) and at a site that has rerun
                     8 times.  Inside the copy a constant range of 1 to 64 rounds over a body
                     free of loops, `break`, `continue` and `return`, of at most 48 nodes, is
                     unrolled, the counters left where the loop leaves them.

**In words.** A discharge or a checked operator compiles to a select or a branch on a loop's
critical path, and a select in a constant loop's body keeps LLVM from unrolling it, so one
call's divisions cannot overlap the next call's.  The fast copy moves every such test off the
path into one flag, read once after the statement.  Its only effect is on the statement's
scalar locals, which a rerun restores, so a set flag leaves no trace: this is @C120's rule
kept (every observable value as the interpreter answers it, faults included), with the fact
established after a speculative run instead of before the arithmetic.  `06_newton_sqrt`:
1.67× → 1.14× of Rust.  Switch `LOFT_NO_FAULT_FLAG=1`; `LOFT_TRACE_FAULT_FLAG=1` names each
admission and each decline with its reason.  Sites: `generation/fault_flag.rs`
(`fault_flag_admits`, `fault_flag_statement`, `fault_flag_unrolled`), the fault forms in
`emit.rs` (the discharge), `calls.rs` (the raise), the runtime helpers `ops::fault_flag_*`.  Guard
`loft-suite/rules/rewrites/R-FaultFlag/a-for-statement-run-under-a-fault-flag-answers-as-written.loft`.

### `(R-GroupPush)`

#### A mint group outside any held header binds its own

      (R-GroupPush)  a mint GROUP — OpPreAllocVector(P, n, size) · n × (e = OpNewRecord(P)
                     · writes into e · OpFinishRecord(P, e)) — over a bare-variable plain
                     vector P (the (R-Mint) shape) whose path holds NO record-push header
                     where it stands, whose every mint qualifies under (R-PushRec), and
                     whose every OTHER statement up to the n-th finish is one the header
                     admission lets through (an in-place set, a store-free builder, a
                     § V-d fresh delivery, a nested mint on another path) with P never
                     rebound and never minted or reserved outside the count, binds a
                     push header of its own right after its reservation and emits its
                     mints and finishes through it (R-PushRec), the header dying at the
                     n-th finish; every read of P inside the group stays a store read.
                     The reservation STAYS — it is what gives the header its capacity —
                     and a growth inside the group is (R-Refresh)'s.  A builder that may
                     write a store, a mint or finish on P nested where the count cannot
                     see it, and a group that runs out before its n-th finish each keep
                     the templates, which resolve per element and are always right.

**In words.** @PLN157.  `(R-PushRec)` gave the header to a mint INSIDE a loop
that holds one; every other append — a literal group at function level, a group inside a
loop that declined, a group on a per-pass local — paid the `record_new` dispatch, the prefill
and the `record_finish` dispatch per element: the drawing library's `fronds` built its 1 296
points per call that way (`fd_pts += [pt(…), pt(…)]`, `fd_pts` declared per pass), 15 µs of a
102 µs call.  The group's own reservation already guarantees the slots, so the header derived
after it is exact for the whole group, and the group is straight-line code the parser emits —
the same producer `(R-Mint)` trusts.
The audit reads the group's close marker (`// @FR-R-GroupPush group __ph_N closed`) because
the header's Rust binding outlives the group.  Switch `LOFT_NO_GROUP_PUSH`; falsifier
`LOFT_HOIST_VERIFY=1` (the header re-derived at the slot and the finish; the growth cell g6
is the one that moves the record mid-group).  Cells `tests/scripts/157-group-push.loft`
g1, g3–g7, g10; pins `tests/group_push.rs`, and `tests/record_push.rs` re-derived where the
group reaches a cell the loop declined (c8, c11, c15).  Sites: `hoist::mint_group`,
`Output::bind_group_push`, `Output::close_groups_before`, `Output::hoist_tiers`, the
`GROUP_CLOSE` rule in `scripts/emission_audit.py`.

### `(R-RefillBuffer)`

#### A return buffer refills the store a rebind released

      (R-RefillBuffer) on `--native`, the store `(H-SwapRebind)` releases — holding the
                     rebound variable's previous value, a root of its own type that
                     nothing references — is KEPT instead of freed (one at a time; a
                     second release frees the first).  A function's hidden return buffer
                     whose every mint is the buffer's null-check prologue or a bare
                     `OpDatabase`, each heading a literal group that writes every field
                     (`(R-CompleteWrite)`'s coverage) of a REFILLABLE type — every field
                     owns no heap, or is a vector whose elements own none — mints with
                     `OpDatabaseRefill`: a null buffer takes the kept store of the same
                     type as it stands, and the group's zero of each vector field empties
                     that vector in place (`clear_vector`) for the fill that follows.
                     Every other mint is fresh and the empty is a no-op on it.  A text, a
                     keyed collection, a nested record that owns heap, a mint the walk
                     cannot see the group of, and a `par` worker's stores decline.
                     THE IN-PLACE CLAUSE: an empty followed by a repeat literal of the
                     same field (`n` copies of one scalar, `(R-PushFill)`'s form) is one
                     overwrite when the kept vector holds exactly `n` elements of that
                     width — every element written, the length already `n`, so the
                     answer is the empty-and-fill's; any other length, a foreign or
                     locked vector, empties and fills.

**In words.** The `(R-LoopRecord)` shape across a call.  A builder called in a loop,
`c = mul(a, c)`, minted a store for its result every call, claimed its vector and filled
it; the rebind then exchanged it into `c` and freed the store holding `c`'s old value.  The
two stores now take turns: the one released is the next call's buffer.  It is exact because
the literal is complete — every scalar is written again — and a refillable type's only heap
is vectors the group empties and refills, so nothing of the old value survives and nothing
old is left unreachable in the store.  A kept store is the runtime's, not a leak, and is
skipped by the exit census.  Effect on mesh3d's `mat4_mul` loop: priced by hand at −52 %
of the call.  Switch `LOFT_NO_REFILL_BUFFER=1`, read where the buffer is emitted and at run
time.  The in-place clause skips the reset, the reserve and the two header derivations of a
fill whose length the kept vector already has: `mat4_mul` −17 % (priced −24 %), switch
`LOFT_NO_REFILL_IN_PLACE=1`, site `Stores::fill_exact`, guard
`tests/scripts/a-refilled-repeat-literal-is-overwritten-in-place.loft`.  Sites: `hoist::refill_buffers`, `ops::misc_ops` (the mint), the block loop in
`emit.rs` (the vector-field empty), `Stores::take_spare` / `park_spare`.  Guard
`tests/scripts/a-return-buffer-refills-the-store-a-rebind-released.loft`, pin
`tests/refill_buffer.rs`.

### `(R-RefillText)`

#### A pooled buffer's texts are refilled in their slots

      (R-RefillText) at a call site whose result buffer b is a POOLED call buffer —
                     minted once per activation by a zero-filling OpDatabase and
                     released before every later call by OpClear(b, tp) (the
                     (R-Reuse) pool, @FR-H-ClearRelease) — the release is not
                     emitted, and the call goes to the callee's REFILL TWIN, whose
                     text sets into its own return buffer refill the slot: the
                     slot's block is written over when the new text fits its claim
                     (len + 8 ≤ 8 × the block's header words), released and claimed
                     anew when it does not, claimed when the slot is 0; a null text
                     releases the slot's block.  Admitted where (a) tp's heap is
                     TEXT ONLY — every heap slot of the type, nested inline records
                     flattened, is a text (no collection, no reference, no enum
                     payload that owns heap); (b) the callee builds its result as a
                     record LITERAL into its own buffer on EVERY exit — `return b`,
                     or a block whose value is b — the literal writing every text
                     slot of tp exactly once, and nothing else in the callee names
                     the buffer; and (c) b is named by nothing but its pool
                     statement, this call's buffer argument and its releases, and
                     the result bound from the call is only READ (a reading OpGet*,
                     the source of a copy that keeps its source).  Where the site
                     would call an `__inv` or `__rg` twin it releases first instead,
                     in the pool statement's order.

**In words.**  A pooled buffer's release walk frees every text the previous result held, and
the callee then claims a fresh block for every text of the new one: per call, one walk and
one claim-and-free per text field, for blocks that are the same size each time more often
than not.  Writing over the old block when the new text fits keeps the bytes where they are
and drops both halves.  What makes it sound is ONE invariant, enforced at three points:
**a text slot read by the refill holds 0 or a text block that slot owns.**

- *The buffer's history* — (c): its mint zero-fills it (a slot is 0) and every later write
  is the refill twin's literal (a slot owns its block).  A no-prefill mint
  (`OpDatabaseNP`, `(R-CompleteWrite)`) or an element slot handed in as the buffer
  (`(R-Place)`, `(R-ElemFirst)`) leaves BYTES there, which a refill would read as a block
  number; neither is a pooled buffer, and neither can reach a refill twin, because the
  decision is the SITE's, not the callee's.  A twin that mints its own buffer keeps the
  zero-filling mint for the same reason.
- *The callee* — (b): every exit is a literal writing every text slot, so after the call no
  slot still holds the previous result's text (no stale value is visible, none leaks); a
  forwarded exit (`return g(…)`), a copy into the buffer (`OpCopyRecord`, which claims fresh
  blocks over the old ones) or an early exit that writes nothing declines.  The parser
  writes every field of a literal, a defaulted text included, so what (b) protects is the
  live-record count, not a value.
- *The type* — (a): a refilled text slot is the only heap the walk would have freed; a
  collection or a heap-owning enum payload would be left for the walk and is out of scope
  until a clause of its own (vector-build.md's text-bearing elements).

Why a TWIN and not the callee itself: the same callee is also called with buffers that do
not keep the invariant (a fresh element slot, a no-prefill mint), and one emitted body cannot
tell them apart; a twin, chosen where the buffer's history is known, can.  It is the shape
`(R-Inputs)` (`__inv`) and `(R-RangedCall)` (`__rg`) emit.  A previous result read through a
borrow after the next call is no new hazard: the release walk freed its block at the same
point, and the scope pass never pools a site whose result a later text still depends on (it
mints per call there instead).

Native only; the interpreter keeps the walk and the fresh claims and is the oracle.  An
armed release no call takes is an emission error, never a silent leak.  Effect on
game_protocol `msg_ping` (x86-64): 18.0 → 4.89 ms, 7.55× → 2.05× Rust, hash unchanged.
`flow_layout_full`, pluginabi `check_request` and cbor `decode` need the collection clause
after it (vectors of text-bearing records).  Switch `LOFT_NO_REFILL_TEXT=1`; trace
`LOFT_TRACE_REFILL_TEXT=1` names each pooled site's verdict.  Sites: `hoist::refill_text_sites`
and `hoist::refill_text_callee` (the admission), `Stores::text_slots` (a), the block loop in
`emit.rs` (the pool statement), `user_fn_call_body` (the call), `substitute_template_body`
(the twin's text set), `Store::refill_str` (the write).  Guard
`tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft` — value cells and a
live-record census (`m1`), with three planted defects under `tests/falsified/` — and pin
`tests/refill_text.rs`.

      (R-RefillText), the collection clause: a function whose hidden return buffer b is
                     a (R-RefillBuffer) heap-element buffer — a one-field wrapper of
                     vector<E>, E a record of scalars and texts — KEEPS b's elements
                     where the mint keeps b (refill_keeps, the test OpDatabaseRefill
                     asks): the entry sets the length to 0 and keeps K, the old length,
                     in place of the releasing clear; a text set into an element the
                     build appended refills the block its slot owns when the slot is
                     under K and claims past it; and every exit releases the texts of
                     the kept slots [len, K) the call did not reuse.  Admitted where the
                     buffer is cleared once, at entry; every append mints its element
                     with OpNewRecord, emitted without the prefill (R-CompleteWrite);
                     and every other mention of b, its field views and its appended
                     elements is a reservation, a length, an element read, an
                     append's finish, an element's own set or read, the field's zero,
                     a self-replace, a release or a return.

**In words.**  The same invariant, one level down: a text slot read by the refill holds 0 or
a block that slot owns.  Slots under the length at entry are live elements, which own their
texts; past the length a removal can leave a stale copy of a live element's handle, and fresh
capacity holds whatever the store left there, so neither is trusted and K is the length, not
the capacity.  The length stays true during the build — a read of the vector mid-build sees
only what this call appended — which is why each kept slot still goes through the append (a
form that overwrote kept elements in place under the old length priced −20 % against this
form's −16.5 %, and answers a mid-build read with stale elements).  The vector grows only
when its length reaches its capacity, which is at least K, so a slot past K is never one a
growth copied.  A second clear inside the build would reset the store under the kept slots;
a prefilling mint would write 0 over a kept text and strand its block; a removal, an insert
or any use the whitelist does not name declines.  Effect on zttext `flow_layout_full`
(x86-64): 26.0 → 22.7 ms, 14.15× → 12.35× Rust, hash unchanged.  Switch
`LOFT_NO_REFILL_ELEMENTS=1`; `LOFT_TRACE_REFILL_TEXT=1` names each heap-element buffer's
verdict.  Sites: `hoist::keep_elements` (the admission), `Stores::refill_keep_open` /
`refill_keep_close` (entry and exit), `codegen_runtime::RefillKeepGuard` (the exit on every
return), `clear_vector` in `text.rs` (the entry), `substitute_template_body` (the element's
text set).  Guard `tests/scripts/a-kept-buffers-elements-are-refilled-in-their-slots.loft`,
with two planted defects under `tests/falsified/`; pin `tests/refill_text.rs`.

      (R-RefillText), the vector clause: at a pooled call site whose buffer b's type has
                     for heap only vector fields of plain elements (no text, no
                     collection, no linked group in an element; no other heap field —
                     Stores::vector_slots), and whose callee (R-RefillBuffer) refills its
                     buffer with elements that own no heap, the release is not emitted
                     and the plain callee is called.  Admitted under (c) without its
                     "only READ": the buffer named by nothing but its pool statement,
                     the call's buffer argument and its releases, the result bound once.

**In words.**  The refilling callee keeps a live buffer (`refill_keeps`) and empties each
vector field where it stands; the release before the call freed exactly the vectors it would
have kept, and the callee's appends then claimed new ones.  The invariant the emptying reads
is that a vector slot holds 0, the absent mark, or a vector that slot owns — and every write
the language makes to a vector field keeps that, which is why the result may be written,
grown or passed on between calls, unlike a text clause's refilled slot.  No twin is needed:
an emission of the callee that does not refill mints over the live buffer, and that mint
clears its store.  Native only; the interpreter keeps the release and is the oracle.  Effect
on hex_recover `forms_upto` (x86-64): 2.00 → 1.02 ms, 12.96× → 6.62× Rust, hash unchanged.
Switch `LOFT_NO_REFILL_VECTORS=1`; `LOFT_TRACE_REFILL_TEXT=1` prints `admitted (vectors)`.
Sites: `hoist::refill_vector_callee`, the clause choice in `refill_text_site_declines`,
`RefillTextSites::vectors`, `refill_text_call` in `calls.rs`.  Guard
`tests/scripts/a-pooled-buffers-vectors-are-emptied-in-place.loft`; pin
`tests/refill_text.rs` (the values pass either way, so the pin is the falsifier).

### `(R-BoundedNest)`'s guarded nest (the bare `R-BoundedNest`)

      (R-BoundedNest) an innermost counted loop whose body is ONE accumulate over
                     `?`-discharged element reads — `acc = acc + term`, the term a
                     chain of `+`, `-`, `*` and negation over literals, the loop's
                     counters, variables the loop does not write, and reads
                     `v[chain]?` of `integer` vectors — runs with the processor's
                     PLAIN operators when a guard, evaluated once at the loop's
                     entry, proves that no operation in it can fault: the range's
                     ends are not the sentinel and the trip count is positive; no
                     invariant is the sentinel; every vector read has a known
                     element bound (no stored null); and the MAGNITUDE BOUND of every
                     chain and of `|acc| + trips × bound(term)` — literals by value,
                     counters by the larger range end, reads by their vector's bound,
                     `+`/`-` summing and `*` multiplying, each step checked — fits
                     the type.  A bound that fits means every true intermediate
                     fits, so the plain operator answers exactly what the checked
                     template would; otherwise the checked loop runs, unchanged.
                     An element bound is a fact about VALUES, taken once at the
                     outermost loop whose body neither writes the vector (an element
                     or field set through it, an append, a rebind of its root) nor
                     hands it to a call — never at the nest's own prelude.
                     When every read's chain names the counter at most ONCE (so it
                     is affine, and its extremes over the range are its values at
                     the two ends), the guard also requires both ends in [0, len)
                     for every read; then each read is one load through the held
                     base with no bounds test and no null select — the element
                     bound has already ruled out a stored null.  A read outside the
                     range at either end declines the nest whole.

**In words.**  @PLN157's guarded plain nest, the admissible successor @C120 names: the
integer closure is not taken (R-Counter) because an overflow's null must propagate on both
backends, so the plain form is admitted only where a fact ESTABLISHED before the arithmetic
runs says no overflow can occur — which is exactly what the guard checks, in the same
magnitude-bound arithmetic the rule states, with `checked_*` operators whose failure is the
decline.  The reads stay what they were (a bounds-tested load and the discharge's null
test); what goes plain is the index chain, the product and the accumulate — the taps of a
resample, where they were 12 checked operators per tap and ~55 checking instructions in
front of the arithmetic.  The bound's placement is the cost model: one linear pass per pass
of the nest's enclosing loop, against the nest's taps under it; at the nest's own prelude
it would be the nest's cost again, which is why a function-level nest with no enclosing
loop declines.  A stored null, an accumulator near the maximum, a null invariant and a
vector written by the enclosing loop are the cells that must DECLINE
(`tests/scripts/157-bounded-nest.loft` n2, n3, n7c, n8, n16, n17); a read one past the end
and a chain that goes negative decline the RAW form and answer through the checked loop
(n19, n23), while the last element, a negative coefficient and a constant index are in range
(n18, n20, n22) and a counter named twice is not affine (n21).  Switches
`LOFT_NO_BOUNDED_NEST` and, one step finer, `LOFT_NO_NEST_RAW_READS` (the plain arm keeps its
bounds-tested reads); falsifier `LOFT_HOIST_VERIFY=1` (every plain operator's answer is
compared with the checked template's at the operator — `ops::nest_verify` — and every raw read
with the checked read); trace `LOFT_TRACE_NEST=1`.  Sites: `hoist::bounded_nest`,
`hoist::nest_read_paths`, `Output::nest_fast_path`, `nest_bound_expr`, `nest_chain_at`,
`ops::int_arith::nest_form`, `ops::vector_ops` (the raw read), `Output::output_if_inner` (the
select), `vector::abs_bound_i64`.

### `(R-Header)`'s function clause (`R-Header/function`)

      The FUNCTION clause: a body that reads a
                     vector PARAMETER twice or more through an element address, and
                     leaves that parameter as it found it — never rebinds it, never
                     hands it to a native that is not a reader, never frees it, and
                     hands it (or a heap value that may name its store: a parameter, a
                     view, a link — never a local's own fresh store or a work buffer)
                     only to callees of which the same holds at that position, a
                     recursive call answering for itself — binds its header once at
                     entry, and every read in the body, loops included, serves from it;
                     in a twin body that already receives the parameter's header (the
                     twin clause's `__ih_k`) that header IS the binding, never a second
                     lookup.  Growth of OTHER stores is no concern of a header (its record and
                     length are the vector's own), so a body that builds its result
                     beside the reads keeps them; only a BASE would need the store's
                     memory still, and the clause binds none — a loop inside that is
                     growth-free derives its base off the held header.
                     

- **The function clause takes a base, for `(R-Base)`.**  Beside the function-clause header
  `(R-Header)` emits (`src/generation/mod.rs`), `vec_base` is emitted under the base's own
  condition — no growth of that vector anywhere in the function — and the `?? default`
  element read outside a loop takes it as the in-loop read does.  Cells: a read past the end
  (the default answers); a function that appends to the vector it reads (declines).  cbor
  `decode`: 36 byte-read sites, 6.49 → 5.15 M on the ladder.

### `(R-Header)`'s kept clause (`R-Header/kept`)

      (R-Header), the KEPT clause: a vector field p.f of a plain-record PARAMETER p
                     that a body never rebinds is KEPT by that body when everything
                     in it that could change the field's header — its record or its
                     length — is one of: (a) a record append to p.f itself (the
                     OpNewRecord … OpFinishRecord group) as a statement outside any
                     loop; (b) a statement that is a call to a loft function, or
                     the assignment of one; nothing else.  A store-free op, a scalar
                     setter and a heap-free record copy change no header; any other
                     writer mentions no variable that may name p's record or the
                     vector — p, a variable whose type reaches p's record type, a
                     vector of the field's element type — unless it is its own
                     store's; a call that is not a statement leaves every such
                     argument untouched (the function clause's question); no `CallRef`,
                     `par` or `yield`, and nothing it calls can lock or unlock a
                     store (the header carries the lock).  A kept field the body
                     READS — a walk, an element, a length, or a call that hands p to
                     a function keeping the field at that position — has ONE holder:
                     bound at entry where the body HANDS IT DOWN to a callee's kept
                     twin and the reads it serves, as emitted, are two or more;
                     received from the caller by `&mut` in the body's KEPT TWIN.  Every read
                     serves from the holder; after each op of (a) and each call of
                     (b) the holder is derived again, except a call to the callee's
                     kept twin, which hands the holder down and so keeps it current
                     itself.  No base is kept: a claim anywhere may move the store's
                     memory, never a record number or a length.  A body some rewrite
                     emits a RUN of statements in — the hand-over's rotate, an
                     element-first build, `(R-Apart)`, a stream walk — keeps nothing:
                     the run is emitted outside the statements a holder is refreshed
                     after.  A kept call takes the ranged form of the kept twin
                     (`__rg__kh`) where `(R-RangedCall)` admits it.

**The kept clause, in words.**  L7 (`bench/portal/analysis/worklist.md`): lane 17's
`slope_path_with_undo` walks `m.m_chunks` four times a step in four callees
(`map_get_hex` twice, `map_ensure_chunk`, `map_set_hex`), each deriving the header from the
store, and one of them appends to it.  The function clause cannot hold it (a callee writes the
vector) and the twin clause cannot hand it over (the callee grows a store).  A header changes
only where something names its record: the apart clause's mention argument, applied to a
whole body, with the appends that DO name it refreshing the holder at their own site
(`(R-Refresh)`).  Hand price −13.5 %.  Switch `LOFT_NO_KEPT_HEADER=1`; falsifier
`LOFT_HOIST_VERIFY=1` (every read of a holder asserts it against a fresh derivation);
`LOFT_TRACE_KEPT=1` names each candidate key and its verdict.  Sites: `kept::kept_paths` (the
verdict; `kept::untouched_field` asks a plain callee, and sees a local VIEW of the field, which
`param_untouched` does not), `Output::begin_kept_headers` (the holders, in the plain body, its
ranged variant and the `__kh` twin), `Output::kept_after_statement` (the refresh, and a `for`
walk's hidden vector served from the holder), `Output::kept_twin_call` (the hand-down),
`Output::kh_live_check` (a twin the live tier flips into the interpreter refreshes before it
returns).  Guard `tests/scripts/a-kept-vector-field-follows-its-appends-across-calls.loft`
(falsified twice: no refresh after the append group, no refresh after a plain call that may
append); pin `tests/kept_header.rs`.  `Output::kept_check_taken` stops the generation when a
statement that changes a held field was taken by another rewrite — the net under the run-emitting
list, which caught the element-first build.  The versioned copy of `(R-Alias)` does not count a
path held only by a kept holder as held: the copy buys more than a base there (crawler's
`build_vis_texture` keeps its push window in it; withheld, 1.14× → 2.94× of its twin).  A
plain body binds only a holder it hands down, counted as emitted (`Output::kept_effective_uses`:
a callee `kept_of` declines, or a loop's call that takes the `__inv` twin, serves nothing) —
own reads alone are what the loop's and the function clause's headers already serve lazily,
and an eager holder of a field read on one branch cost hex_body's `bone_shape_has` 7.7 %.
A held read of the raising `OpGetVector` reports an index outside the header as the unfused
read does (`@FR-E-Report`, `FusedElementReadEmitter`); the nullable read answers it silently.
A loop that changes no kept field copies the holder into its own header local
for its extent (`Output::kept_loop_copy`): read through the `&mut`, the header stayed in memory
across the loop's calls (zttext's `materialise`, +11 % until it did).  Measured with
`bench/stats.py` against `LOFT_NO_KEPT_HEADER=1` (7 samples, every hash equal):
`slope_path_with_undo` −11.0 %, zttext `materialise` −8.9 % and `seg` −4.4 %, stage
`pack_instances` −4.1 %, `resolve_move` −2.5 %; over the 47 routines of the lanes it reaches
most, geomean −0.6 %.

### `(R-RecPtr)`'s index clause (`R-RecPtr/index`)

      THE INDEX CLAUSE: where the binding is `r = v[IDX]?` —
                     `{ t = v[IDX]; if t { t } else { ?'s record } }` — over a path whose
                     header AND element base the frame holds, with IDX a repeatable
                     expression (pure operators and discharges): IDX is evaluated once,
                     before the statement, and when it is in range of the header's length
                     r is the vector's own record at `8 + IDX * size` and its address
                     `base + IDX * size`; out of range — past the end, below zero, null —
                     the binding runs as written, reading the named index, and the record
                     `?` builds is resolved as any view's.  That record lives in a store of
                     its own, so no text field of r is served by the base's span.  

### `(R-RecPtr)`'s nested clause (`R-RecPtr/nested`)

      THE NESTED CLAUSE: a VECTOR field of r — `r.items`, or through
                     inline sub-records `r.pos.items` at the summed offset — read by
                     a length (`len`, OpLengthVector) or an element read (OpGetVector,
                     OpGetVectorNullable, the element clause's write) takes its header
                     through the address: the field's vector record one load at it,
                     the length one read of that record, at each use, so a write to
                     the field before the read is seen.  The null address holds the
                     absent vector.  A view whose only uses are such reads takes its
                     address on this clause alone when two reads share the one store
                     resolution the address costs, or one under the base clause, where
                     the address is free; that use enters the verdict before its
                     growth, free and rebind conditions, as the text clause's does.
