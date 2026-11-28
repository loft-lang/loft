# @PLN91 — the scope pass and what follows it, transcribed

A child of [README.md](README.md) and [main-switch.md](main-switch.md) phase 1 step 2.  The
goal: every program gate E agrees on RUNS through the loft path — elaborator, scope pass, slot
assignment, IR passes, bytecode writer, runner — and prints what `loft --interpret` prints
(the `front` layer of `selfhost/bench/run_image_survey.py`).

## Where it stands

| layer | measured | command |
|---|---|---|
| front (the whole loft path) | 882 of 1 201 programs agree; 55 stop at the scope pass, 243 at the elaborator, 17 at the bytecode writer; 0 differ; 3 faults (two `par` bodies and a timeout), no leak, none the scope pass's answer (step 17) | `run_image_survey.py --front` |
| scope gate (Rust's parsed image in, per definition) | the transcription (`LOFT_SCOPE_PORT=1`, step 17): 2 117 of 2 311 programs, 39 430 of 40 531 definitions (libraries' included), 0 differ | `scope_survey.py` |

The unscoped definitions, by what stops them: a compiler-made reference (≈13 500), a
collection local (≈8 300), a text or reference returned past a local to free (646), a `&`
variable (111).

## Why a transcription, not more slices

`scope.loft` is a re-derivation of the pass (its own numbering and exit sweep), carried kind by
kind.  It reached half the definitions; the rest need the pass's store-lifetime decisions
(`owner_handover`, `args`, `witness`, `stored_refs`, the buffers and the drops), and each slice
on a re-derived base meets Rust's structure as a new difference.  The C writer reached byte
identity because it was transcribed module by module under the same names
([12-c-generator.md](12-c-generator.md)); this follows that method (the port rule in
[1a-parser.md § The porting method](1a-parser.md)).

## What is transcribed, in lines of Rust

| part | Rust | lines |
|---|---|---|
| the scope pass | `src/scopes.rs`, `src/scopes/*.rs` | 40 600 |
| the analyses it asks | `src/use_analysis*`, `pure_reuse`, `bound_append`, `spent` | 13 200 |
| the rewrites `check` runs per function | `place_result` … `wrapper_inline` (19 modules) | 10 300 |
| slots and the variable facts | `src/variables/` | 9 100 |
| the IR passes `close_program` runs | `value_record`, `leaf_inline`, `same_read`, `ctor_literal`, … (11 modules) | 8 500, plus `compile.rs` and the `opt` parts |

Where a module is already ported for the C writer (`opt_vr*.loft`, `opt_*`), it is reused, never
written twice.

## Steps, each measured

1. **The scan phase core** — `check`'s per-function loop, `run_scan_phase` (`phase.rs`),
   `scan.rs`, `scan_set.rs`, `free_vars.rs`, `get_free_vars.rs`, in `scope_*.loft` files named
   after them; every call into a module not yet transcribed refuses its function by name.
   Measured with `LOFT_SCOPE_BIN` pointing at a `scoped` built on the new pass: the definitions
   it agrees on, against the slice's.
2. The modules the step-1 refusals rank first, one at a time, each gated.
3. When the transcription agrees on every definition the slice does, it replaces `scope.loft`.
4. Slots (`src/variables/`), then the IR passes — the front layer re-measured after each.

The gates: `scope_survey.py --check` (no agreeing definition lost), then the front layer.

## Built

Steps 1–13 are in [scope-pass-port-history.md](scope-pass-port-history.md).

**Step 14 (2026-11-28) — the owner witness, the value-form remainder, the drops arc begun.**
`witness::owner_witness_locals` (loft#1336, `@FR-O-Witness`) is carried at every site Rust
has it.  The phase mints each mixed-ownership local's `__own_<name>` before the scan (the
local never-free, `Variable.owner_witness` set, `view_bind_bases` joined into its deps, the
null-init in Rust's prepend order); `scan_set` keeps it in step per `witness_set_kind` (the
guarded `release_witness`, `witness_points_at`, `MintUnless` with its temps,
`transfer_lift_to_witness`), a split copy inherits it, `scan`'s in-place rebuild arm
(loft#1510) re-points it, `get_free_vars` releases through it at a scope exit, and
`drops::witness_hook` is transcribed.  `witness_aliases` and `return_moved_holders` feed hooks
only and are not recorded; a captured local that could take a witness refuses by name (its
clauses read `capture_build_backings`).

`arm_lift`'s value-form remainder is carried: a reassignment `sink_set_into_arms` declines takes
`scan_set`'s generic legs.  Two facts made it differ.  `pair_literal_buffers`' rebound
parameters include a local renamed onto the return buffer (`is_promoted_return_buffer`), so
the construction buffers a promoted buffer adopts take no free.  And `written_out` is keyed as
Rust keys it, by the node of the code the scan was handed: a `Set` inside a value `scan_set`
built itself (a written-out branch, a copied bind) is never recorded (`Scopes::built_depth`),
and `rewrite_written_out` matches a value's text BEFORE its children are rewritten.  The port's
text key had written out a `??` chain's subject nested in a sunk branch (`c131`).

The drops arc begins with the records no copy hands a drop on: `drop_hook` answers the cascade;
`displaced_drop` takes Rust's `__disp_N` snapshot, placed by `convert` around an in-place
rebuild; `drop_handoff_node`'s construction arm arms the adopted work-refs per statement and
early for a loop body; `hooks_follow_returned_frees` guards a hook ahead of a returned
`OpFreeRefIfDistinct`.  `scope_handoff.loft` is `handoff.rs` begun (`copy_moves_drop_from`,
`copy_carries_drop`, `copy_record_handoff`).  Refused by name where `handed_off` would take a
flag: a whole-value copy that moves a drop (`holds_caller_record` read as false, a superset),
a droppable record copied into a buffer or container, a droppable first bind Rust writes out
per arm (`arm_source_outlives_loop`), a closure record rebuilt.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 13 | 1 872 / 2 282 | 36 359 / 40 088 | 0 | 769 |
| + the owner witness | 1 911 | 36 545 | 0 | 780 |
| + the value-form remainder | 1 943 | 36 642 | 0 | 793 |
| + drops without a copy hand-off | 1 951 / 2 282 | 37 018 / 40 088 | 0 | 799 |

No definition that agreed was lost, nor a whole-path program; the faults are step 7's (one
whole-path run timed out `164-element-place` under build load: it agrees alone, in step 13's
time).  The stale-callee cascade: 172 definitions refused for nothing else, in 37 programs.

Sole refusal families now (programs): the view walk 71, drops 39, closures 16 (113 counting
`displaced_drop`'s closure-record rebuild with them), `callref_collection_join_base` 11.  By
first refusal: closure records 133, the view walk 72, `copy_record_handoff` 21,
`callref_collection_join_base` 16, the droppable first bind written out 10, `copy_moves_drop_from`
7.  The drops arc continues with the hand-offs themselves: `mint_handoff_flag` and the per-path
pairs (`per_path_handoffs`, `loop_self_stopping_copies`, `caller_record_locals`,
`arm_container_handoffs`, `local_copy_pairs`), `drop_handoff_node`'s copy arms, `scan_set`'s
`stops_target` / inherited flag writes, then `arm_source_outlives_loop`; closures follow.

**Step 15 (2026-11-28) — the drops arc finished: the hand-offs, the returned records, the
collections of droppables.**  `scope_handoff.loft` is `handoff.rs` whole: `copy_moves_drop_from`
with `caller_record_locals` (`Function::holds_caller_record`, kept as a `Scopes` field — read
only inside the pass), `per_path_handoffs`, `loop_self_stopping_copies`, `per_path_stops`,
`arm_container_handoffs`, `handle_handoff`, `local_copy_pairs`, `assigns_a_self_stopping_copy`,
the collection hand-offs (`collection_copy_handoff`, `_ends`, `_backings`,
`collection_handoffs`), `handoff_target`, `collect_drop_transferred`, `copy_record_handoff`,
`drop_handoff_node` with all three arms, `arm_handoff_flags` and `mint_handoff_flag`.  A pair is
`destination * 65536 + source`, Rust's `(u16::MAX, source)` with `scope_fn::NONE`.  The phase
mints the `__hoff_` flags in Rust's order and heads the body with their `false` inits (sorted,
not deduplicated, as Rust collects `handed_off.values()`); `convert` arms each statement's
hand-offs with the arm-lift temps as the per-path set, and a loop arms its body's early;
`scan`'s call arm sets an arm copy's flag (`arm_handoff_flags`) and gives a regrown collection
its release back (`regrown_flag`); `scan_set` writes `stops_target`
(`copy_flagged_on_target`, `witnessed_copy_stops_itself`), the inherited flag and the retire —
`witness_update` nested as Rust nests it, `Insert([prev, write])` — mints the written-out arms'
flags, and records loft#1628's `witness_aliases` with their rebind hooks (`__hdsp_`).
`arm_source_outlives_loop` (with `loop_refills`) admits the droppable first bind written out per
arm, `lift_join_arm_tails` moves a lifted copy's release per path and records `join_holders`, and
`lease.rs`'s `copy_reach` / `leases_whole` join `opt_lease.loft`.

The returned-record family of `returns.rs`: `move_join_hooks_into_arms` (the armed
`materialized_view_return`, rewritten into the return), `return_copies_whole_local`,
`return_copies_view_holders`, `return_moved_holders`, read through `Delivered`'s `arm_dropped`
and `moved_out` (`unless_moved`) at every hook leg and the null-arm join.  The collections:
`drop_hook`'s collection arm, `vector_call_rebind`, `promoted_vector_refill`,
`remints_literal_backing`, `call_minted_member_handoff` and a tuple's vector member moved
(`tuple_member_now`); `write_out_joined_copies` writes a join copied into a droppable container
out per arm (`sink`).  `copy_hands_off` and `appends_to_element` read
`type_owns_droppable_anywhere` / `owns_droppable` (`opt_vr_leaves::owns_droppable`) where a
cascade stand-in had counted a vector element as droppable.

One fact is still not in the image: a tuple member's backing (`tuple_member_backing` reads
`Function::tuphold_origin` and `Vars::tuple_backings`).  It answers `None` where the image settles
it — a parameter's member, a member that is no heap record, a member whose type names no dep
(`tuple_backings` records the deps the members spelled, so it holds none there) — and refuses by
name otherwise (2 programs).  Two port defects fixed on the way: `arm_bind` read the owner-witness
map as a value set, and `scan_set` spread `witness_update` where Rust nests each write.

A loft defect met, both backends: a nullable local returned on one path and copied into a field
on another (`return N { f: d! }` beside `d`) leaks its store on the copying path.  The tool's
`scope_end_drop` and `witness_hook` had that shape and leaked their own `NdIf` nodes on ten
whole-path programs; they are spelled around it (the unflagged path answers the call itself).

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 14 | 1 951 / 2 282 | 37 018 / 40 088 | 0 | 799 |
| + the hand-offs, the returned records | 1 971 | 37 271 | 0 | 811 |
| + the collections, the joined copies | 1 996 / 2 282 | 37 454 / 40 088 | 0 | 814 |

No definition that agreed was lost, nor a whole-path program; no whole-path leak.  The
stale-callee cascade: 123 definitions refused for nothing else, in 29 programs.

No drop refusal is left but the keyed collection's generator walk (1 program) and the tuple
member's backing (2).  Sole refusal families now (programs): closures 119, the view walk 73,
`use_analysis` 18 (`callref_collection_join_base` 11, `lease_calls` 6), `args` 10.  By first
refusal: closure records 133, the view walk 73, `callref_collection_join_base` 16,
`view_rebind_locals` 9.

Closures are the next lever, and they are one transcription of about 5 500 lines:
`capture_builds.rs` (1 505: `capture_build_backings` builds `CaptureBuilds`, twelve facts —
the backing each capture named at its build, the builds in a loop or on some path, the adopted
stores, the join arms — read only inside the pass), `capture_adoption.rs` (1 832, its decidable
cascade parts ported), `closure_keep.rs` (1 347) and `rebound_captures.rs` (799), with
`owner_slot_handoff`, `displaced_drop`'s closure rebuild and the owner witness's captured-local
clauses.  133 programs stop first at `capture_build_backings`; 119 at closures alone.

**Step 16 (2026-11-28) — loft#2684 followed; the closures begun: the capture builds, the keep
set, the owner slot, the witness's capture clauses.**  Re-taken first on a binary frozen from
97319770e: `get_free_vars` follows loft#2684 (a202c48ad) — a local only the returned ARGUMENT
names is released at the exit by store identity, `OpFreeRefIfDistinct(local, buffer)`
(`LOFT_NO_PROMOTED_RETURN_IDENTITY_FREE`; the attic's `h9-tail-field-of-call` agrees with the
switch on and off).  loft#2681 is the IR passes' (already ported); c549e0df5 and fa74668d5 change
comments only.  The gates were unchanged by the new binary: 1 996 programs, 37 454 definitions,
whole path 814.

The image now carries `Function::join_owners` (`Variable.join_owners` at byte 52, the stride 80;
`scope_fn::join_owners`), which the capture analysis reads.  The loft elaborator does not set it
yet (its two Rust sites are `parser/expressions.rs`'s sunk vector arms and
`parser/operators.rs`'s owned-vector `??`), so on the whole path it reads empty.

`scope_capture_builds.loft` is `capture_builds.rs`'s facts: `CaptureBuilds` and
`capture_build_backings` with every walk it runs.  Rust keys `null_slot_sets` and
`arm_written_joins` by node address; the port keys them by the node's position in the same
pre-order walk (`Value::walk`, spans not counted).  `Function::materialised_views` reads as empty
(a frame with a view to materialise is refused before the scan).  `scope_closure_keep.loft` holds
`pass_confined_records` (the confinement walk, `returns_or_calls` over the stripped callee body,
`only_called_in`) and `closure_keep_set`; a GATED keep set is refused by the phase by name (its
per-run releases, `closure_keep_live` … `fnref_call_rebind`, are not transcribed).
`get_free_vars` runs `fnref_stand_down`; `captured_record_stand_down` is refused by name.
`scope_capture_adoption.loft` gains `record_adopts_capture`, `frame_owns_capture_store`,
`owner_slot_takes`, `join_arm_holders`; `scan`'s call arm carries `owner_slot_handoff`'s three
legs (loft#2131); `owner_witness_locals` its three capture clauses; `displaced_drop` its
closure-record clause (`closure_cascade_has_hooks`).

The capture facts were checked against Rust's fact by fact: a probe printed both sides
(`LOFT_TRACE_CAPTURE_BUILDS=1` — kept in the loft tool, the Rust half was a throwaway build) over
the 143 programs that build a closure: 792 definitions, identical but for 5 definitions of 2
programs where Rust drops a materialised view's backing — both refused by the view walk.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 15, re-taken on the new binary | 1 996 / 2 282 | 37 454 / 40 088 | 0 | 814 |
| + loft#2684 | 1 996 | 37 454 | 0 | — |
| + `capture_build_backings` | 1 998 | 37 458 | 0 | — |
| + `closure_keep_set`, `fnref_stand_down` | 2 006 | 37 477 | 0 | — |
| + the witness's capture clauses | 2 006 | 37 477 | 0 | — |
| + `owner_slot_handoff` | 2 006 | 37 481 | 0 | — |
| + `closure_cascade_has_hooks` | 2 006 / 2 282 | 37 481 / 40 088 | 0 | 817 |

No definition that agreed was lost, nor a whole-path program.

What stops the closure programs now, by first refusal: `free_record_in_omitting_arms` 63
(loft#1476: it needs `capture_store_adopters`, `link_written_closure_records`,
`record_store_leaves_frame`, `record_capture_slots` and the parser's `widen_bare_fn_ref`),
`capture_adoption_owns_free` 38 (the get_free_vars consumers: `escaping_record_holds`,
`_buffer`, `per_run_capture_owners`, `adoption_build_is_conditional` with
`free_unless_record_built` / `free_unless_leaver_holds`, `owning_record_locals`), the gated keep
set 11.  Behind them every closure record's drop cascade waits on `mark_borrowed_captures`
(1 072 definitions), and `rebound_captures.rs` is not begun.

**Step 17 (2026-11-28) — loft#2687, #2683, loft#2688/#2689 followed; the closures: the per-arm
record release, the capture adoption consumers, the borrow verdicts, the drop cascades.**
`scan_set` follows loft#2687 (`@FR-B-Scope`, `LOFT_NO_SPLIT_JOIN_STATEMENT_SCOPE`): a written-out
join that starts a new binding is not recorded for `rewrite_written_out`, and the binding is
declared at the statement.  After the rebase onto #2683 the port follows the scope-pass changes
under it: the displaced free disarms the record constructions' work-refs that name its store
(loft#2680, `construction_temps`, `disarm_at_release`), `reuse_record_buffers` declines a buffer
whose call is bound to a local with an unguarded free (`call_result_locals`), and
`witness_set_kind` reads a `!` discharge block as a mint where its temp holds a store of its own
(loft#2669; `bind_lowering::first_arm` transcribed as far as it decides there, refused by name
otherwise).  Then loft#2689 (`LOFT_NO_WITNESS_DELIVERED_MOVE`: a witnessed local that adopts a
delivered literal moves its buffer) and loft#2688 (`LOFT_NO_MOVE_PER_BINDING`:
`literal_moves_in` counts only the binds of the local's first binding).

Rust reads the definition's code during a scan (`data.def(d_nr).code()`): the parsed body at the
first scan, the previous scan's result at each rescan.  `Scopes::def_code` carries it; `check`
threads it through every `run_scan_phase`.

`free_vars` carries `free_record_in_omitting_arms` (loft#1476) with `capture_store_adopters`,
`link_written_closure_records`, `record_store_leaves_frame` (`returned_closure_records`,
`closure_records_of_source`), `record_capture_slots` and the parser's `widen_bare_fn_ref`.
`get_free_vars` carries its closure legs: the link-delivered records,
`closure_record_leaves_frame` (`record_leaves_frame_at`, `records_holding`,
`claimed_into_delivered`), the fn-ref's adoption and arm-release stand-downs,
`capture_adoption_owns_free` (`captured_only_by_confined`, `backs_an_adopted_capture`,
`capture_is_adopted`, `fn_ref_half_captured`), the conditional-build frees
(`adoption_build_is_conditional`, `owning_record_locals`, `free_unless_record_built`, run first,
with `captured_view_stand_down`), `escaping_record_holds(_buffer)`, `join_capture_witness`,
`reassigned_join_capture_slot` and `rebound_capture_slots`.  `scan_set` carries the record
rebuild prefix (loft#1388, the owner slot handed back, loft#2131); `displaced_drop` homes a
closure record's snapshot at the body; `arm_lift`'s join holders become join owners.

After the loop, `mark_borrowed_captures` computes the verdicts on each settled frame that builds
a record (`frame_verdicts`, with `group_is_pairwise_exclusive`, `group_leaver_is_per_run`,
`adoption_owner_index`), marks the borrowed attributes, scrubs the cascades' walks and strips the
displaced-record releases; the cascades are scanned in the loop like any function, and refused
after it only where a refused frame builds their record.  `cascade_before_record_frees` is
transcribed.  `Variable::promoted_from` is not in the image: it is read off the shadow's name
(`__bx_<arg>`, `__tp_<arg>`, the parser's only spellings).  The closure keep set's per-run
methods are still not transcribed (a gated keep set is refused), so `per_run_capture_owners` and
`free_unless_leaver_holds` are unreachable; `rebound_captures` and `owner_handover` are refused
where they fire.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 16, re-taken on a0b91cbb9 + loft#2687 | 2 006 / 2 282 | 37 481 / 40 088 | 0 | 817 |
| + the per-arm release, the adoption consumers, the verdicts | 2 011 | 37 622 | 0 | 818 |
| rebased on 743355252 (#2683), the replayed port | 2 032 / 2 308 | 38 106 / 40 585 | 13 | — |
| + #2683's scope changes followed | 2 036 / 2 307 | 38 099 / 40 565 | 0 | — |
| + the cascades admitted, `cascade_before_record_frees` | 2 113 / 2 307 | 39 349 / 40 450 | 0 | 881 / 1 201 |
| rebased on 8c4212608 (loft#2688, loft#2689), the replayed port | 2 114 / 2 311 | 39 416 / 40 531 | 14 | — |
| + loft#2689, loft#2688 followed | 2 117 / 2 311 | 39 430 / 40 531 | 0 | 882 / 1 201 |

The suite moved with the rebase (4daddf8), so the denominators move between rows; each row is
checked against the one above it.
No definition that agreed was lost, nor a whole-path program.  The 115 cascades
`prune_empty_cascades` retires answer under their `_retired` names on both sides.

What stops the closure programs now (definitions / programs): the cascades a refused frame's
verdicts decide 200 / 36, the gated keep set 101 / 30, `fnref_call_rebind` 56 / 19,
`captured_record_stand_down` 15 / 4, `rebound_captures` 14 / 4, `owner_handover` 12 / 5.

Two loft traps met: a comprehension over a call answering a nullable struct loses elements
(`[for i in 0..4 { shape(i) }]` with `shape` answering `G?`, both backends;
`captured_view_stand_down` builds its list by a loop), and `x ?? false || c` reads as
`x ?? (false || c)` (`frame_decided_cascades` had it).

## Passes

Step 4's second half: the IR passes `compile::close_program` runs after the scope pass,
transcribed one module per Rust module (`selfhost/src/ir_<module>.loft`, the same function names)
and called from `ir_passes::close_program` in Rust's order.  Shared tree helpers are
`ir_pass_tree.loft` (a node rebuilt over new children, `add_unique`, and `relayout`, which lays the
frame out again with `scope::intervals` / `scope::assign_slots` as Rust's
`compute_function_intervals` / `assign_function_slots` do).  A pass that cannot carry a definition
refuses it by name; `tail_self` refuses a function whose lowered body shows no jump (an internal
compiler error in Rust), and none does on the corpus.

The gate is `selfhost/bench/passes_survey.py` (Rust's image right after the scope pass, through the
loft passes, against Rust's image after `close_program`, per definition).  Both images are taken
under `--emit-image` (`LOFT_EMIT_SCOPED` exits after the scoped one): a plain run parses some
programs otherwise — no `synth text dest` around a library's text-returning call — so its scoped
image was not the input the closed image's passes had.  The gate also hands `passed` the closed
image, for the one fact the host sets between the two: `Definition::native`, the bridge symbol a
native library build exports (`native_adopt::mark_exports`), which the passes read and the scoped
image cannot hold.  Two facts the passes store are not in the definition report, so `passed`
compares them with the closed image itself and names a definition that differs on a `fact:` row
(`ir_passes::fact_rows`), which the survey counts as a difference: `Definition::no_prefill` and a
closure record's capture field layout.  Each commit was checked against the previous verdict
file: no agreeing definition lost.

| measured | programs agree | definitions agree | differ |
|---|---|---|---|
| no pass (baseline, an earlier corpus) | 1 067 / 2 277 | 34 122 / 37 796 | 3 674 |
| through `complete_write` (passes9) | 1 836 / 2 277 | 38 570 / 40 047 | 1 477 |
| + `loop_buffer` | 1 943 / 2 277 | 38 787 / 40 047 | 1 260 |
| + `value_record`, `tail_self` (passes10) | 2 164 / 2 277 | 39 804 / 40 047 | 243 |
| + `elem_first`, `fill_loop`, `loop_record`, `same_read`, `lift_reuse`, own's `borrow_base` | 2 265 / 2 277 | 40 030 / 40 047 | 17 |
| + `ctor_literal`, `tail_self`'s spanned delivery, the scoped image under `--emit-image` | 2 277 / 2 277 | 40 047 / 40 047 | 0 |
| + `no_prefill` and the closure layout compared (`fact:` rows) | 2 277 / 2 277 | 40 047 / 40 047 | 0 |

**Transcribed:** every pass `close_program` runs but `ledger_sites`: `sync_capture_ownership`
(`ir_capture_ownership`), `arg_copy` (judged by `opt_writes::blocks_plain`, the C writer's port
of `writes_store`), `value_record` (`ir_value_record`, `ir_vr_shape`, `ir_vr_plan`,
`ir_vr_rewrite`: the twinned plan `opt_vr::plan_twinned`, `split_public`, the callee and
parameter shapes, the admission and twin fixpoints, the carrier rewrite and the exits, the view
leaves read off `opt_vr_view`'s plan), `leaf_inline` (ranges and non-sentinel facts from
`opt_range` / `opt_non_sentinel`; the IR phase asks the latter with the program-wide parameter
fact off, `new_ir_proof`), `tail_self` (`ir_tail_self`, with `tail_frame`'s predicate in
`ir_tail_frame`), `zero_count`, `exp_pow`, `ctor_literal`, `forward_walk`, `same_read`,
`self_replace`, `in_range` (with its field clause, which `forward_walk` asks), `single_use`,
`discharge_into`, `fill_loop`, `keyed_entry` (with `keyed_place::site`), `elem_first` (with
`hoist::element_first` in `ir_elem_pairs`), `text_len`, `loop_buffer` (with
`hoist::loop_buffers` and `hoist::invariant_literals` in `ir_hoist`, `buffer_owner::owners` and
its element-first claim in `ir_buffer_owner`), `complete_write` (the fresh-field flags and the
stored `no_prefill` fact, from `ret_adopt::complete_writes` in `ir_loop_records`), `loop_record`
and `lift_reuse`.

**Not transcribed:** `ledger_sites` — inert without `LOFT_LEDGER`, which the gate does not set,
so nothing measures a port of it.  `Data::observes_entries` (set only by `loft test`'s coverage
run) is not in the image and reads as false in `ctor_literal`.

The survey takes ≈220 s at `--jobs 5` (the twinned plan is most of it); `LOFT_PASSES_BIN` reuses
a built `passed`, and the `defs` tool must be built from the same tree as the images
(`LOFT_SCOPE_DEFS`).

A loft trap met on the way: a `match` in statement position whose arm is a value without
`return` (`NdSpan { … } => ir::NdSpan { … },`) builds that value and drops it, with no
diagnostic, and control falls through to the code after the `match` — `tail_self`'s
`deliver_in` counted a jump it then discarded.  Loft drops the value of any expression statement
silently (`N::B { v: 7 };` too).

## The loft-in-loft list

`selfhost/bench/loft_in_loft.list` names the programs the loft-written toolchain carries end to
end, and `selfhost/bench/lil_check.py` holds them as a gate.  A listed program must pass both
halves:

- **interpreter** — the whole loft path (elaborator, scope pass, IR passes, bytecode writer,
  runner: `run_front.loft`, the `front` layer of `run_image_survey.py`) prints what
  `loft --interpret` prints and leaves no store the interpreter does not;
- **compiler** — the C generator in loft writes the Rust writer's unit byte for byte, and it
  builds and runs alike (`cgen_survey.py --run`).

`lil_check.py` checks the list and fails naming the half a program lost;
`lil_check.py --discover --write` re-pins it in the commit that moves it.  CI runs the check on
every pull request that touches the compiler or the port (`.github/workflows/loft-in-loft.yml`,
advisory).  The compiler half reads Rust's front-end image today: joining the loft front end to
the loft C generator is the step that makes it loft from source to C.
