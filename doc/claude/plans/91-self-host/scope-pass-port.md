# @PLN91 — the scope pass and what follows it, transcribed

A child of [README.md](README.md) and [main-switch.md](main-switch.md) phase 1 step 2.  The
goal: every program gate E agrees on RUNS through the loft path — elaborator, scope pass, slot
assignment, IR passes, bytecode writer, runner — and prints what `loft --interpret` prints
(the `front` layer of `selfhost/bench/run_image_survey.py`).

## Where it stands

| layer | measured | command |
|---|---|---|
| front (the whole loft path) | 799 of 1 189 programs agree; 105 stop at the scope pass, 281 at the elaborator; 0 differ; 2 faults, no leak, neither the scope pass's answer (step 14) | `run_image_survey.py --front` |
| scope gate (Rust's parsed image in, per definition) | the transcription (`LOFT_SCOPE_PORT=1`, step 14): 1 951 of 2 282 programs, 37 018 of 40 088 definitions (libraries' included), 0 differ | `scope_survey.py` |

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

**Step 1 (2026-11-27)** — `check` and the scan core transcribed: `scope_check.loft` (`check`'s
per-function loop and its rewrite order), `scope_phase.loft` (`run_scan_phase`),
`scope_scan.loft` (`scan`, `scan_inner`, `Scopes`), `scope_scan_set.loft`, `scope_free_vars.loft`
(with `insert_free` and the `returns.rs` helpers it calls), `scope_get_free_vars.loft`.  The scan
cannot run without three more modules, so their common paths came along: `scope_convert.loft`,
`scope_branches.loft` (`scan_if`), `scope_args.loft` (`scan_args`); `scope_pure.loft` is
`pure_reuse::effect_free`, which the reuse refusal needs to name its sites.  Selected with
`LOFT_SCOPE_PORT=1` in `compare/scoped.loft`; the slice stays the default.  `LOFT_SCOPE_ALL_REASONS=1`
reports every refusal a function meets, not only the first.

What is carried: every scalar, text and `&`-to-scalar/text binding, the scope numbering and
splits, the text exit frees and `__ret_N`/`__blk_N`/`__cond_N` temps, the text return through
the caller's buffer, `__lift_N` text copies, the `__ncc_N` text frees.  What refuses, by name:
every branch Rust reaches only for a record, collection, tuple, generator handle or closure
(`scan_set`'s store branches, `get_free_vars`' store releases, the record returns, the heap
argument lifts), and each settled-IR or whole-program rewrite where its Rust trigger is in the
body.  Two facts the image does not carry are refused where they matter: `was_loop_var` (a
non-text dep registered by a bind) and `text_payload_views` (a `text_mirror` block).

Measured on the whole corpus (`scope_survey.py`, `LOFT_SCOPE_PORT=1`):

| pass | programs agree | definitions agree | unscoped | differ |
|---|---|---|---|---|
| the slice (`scope.loft`) | 142 / 2277 | 19 878 / 37 762 | 16 780 | 1 104 |
| the transcription | 127 / 2277 | 19 725 / 37 762 | 17 898 | 139 |

18 262 definitions agree under both; 1 463 only under the transcription, 1 616 only under the
slice (435 of those are closure drop cascades the transcription refuses wholesale, 354 record
bindings, 281 hidden return buffers).  All 139 differing definitions are ones Rust adds or
renames after the loop (134 closure cascades `mark_borrowed_captures` renames `_retired`, 5
`shared_call::variant` definitions `pure_reuse` adds): no definition the transcription lays
out differs from Rust's.

Step 2's order — the refusals ranked by the definitions where they are the ONLY blocker family
(`LOFT_SCOPE_ALL_REASONS=1`; any = where the family appears at all):

| family | sole | any |
|---|---|---|
| records: drops, witness, backings, `owner_handover`, record returns | 4 394 | 14 908 |
| closures and fn-refs (`capture_*`, `closure_keep`; 888 of it `mark_borrowed_captures`) | 913 | 2 230 |
| vectors: backings, the vector clauses of `drops`, the vector rewrites | 729 | 9 331 |
| generator handles (`handles.rs`) | 225 | 524 |
| tuples (`tuple_members.rs`) | 172 | 1 707 |
| `pure_reuse` (sites) | 130 | 721 |
| `text_replace::admit` | 107 | 320 |
| heap arguments (`args.rs`, `inline_returns.rs`, `arm_lift.rs`) | 104 | 2 355 |
| keyed collections | 63 | 890 |

The survey looked an unscoped definition up by its name cut at the first `#`, so a dispatch or
generic instance (`t_18Circle#Tri#integer_tag`) counted as differ; fixed, which moved ≈620 of
the slice's differ to unscoped.

**Step 2 (2026-11-27) — records.**  A record (or record-enum) local is carried through the scan
in Rust's order of decisions: `scope_scan_set.loft` (`scan_set` for every kind; a collection,
tuple, handle, fn-ref or `&` binding still refuses), the record leg of `get_free_vars` with
`pair_literal_buffers`, the record return legs of `free_vars` and `insert_free` (the null-arm
join hoist, loft#754 / loft#793, `sink_return_branch`), the branch and loop reconciles of
`owned_refs` / `construction_backing` / `view_backing`, and `scan_if`'s `deferred_first_bind`.
New files, named after the Rust modules: `scope_drops.loft` (`drop_hook`, `displaced_drop`,
`in_place_rebuild`), `scope_witness.loft` (`owner_witness_locals` as a question,
`nullable_view_locals`, the displacement predicates), `scope_backings.loft`,
`scope_buffers.loft` (`tail_calls`, `reuse_record_buffers` with its lazy mints),
`scope_capture_builds.loft` (`adopted_work_refs`), `scope_join_rewrite.loft`
(`write_out_joined_copies`).  The phase mints the hidden return buffer's two witnesses
(`__rbo_`, `__rbw_`) and computes `null_led_first`, `literal_moves`, `view_temps`.  The pass
lays out every function but the standard library's, as Rust does, and the survey compares
the libraries' definitions too.

A fact a pass refused before the scan would fill stays empty and is read as Rust reads it; the
refusal is placed where it could fire: a local of a type with a drop cascade (the hand-offs,
per-path flags and hooks), a local whose assignments mix ownership (the owner witness), a
record view of a container the body may disturb (the view walk, a superset), a view handed to
a `&` parameter (`view_rebind_locals`).  The settled-IR record rewrites refuse where their
trigger is: `copy_view`, `rebind_place`, `return_field`, `exit_record`, `place_result` /
`literal_append`, `insert_evacuations`, `keep_placed_results`, `lastuse_reclaim` (vector
backings only), and before the scan `move_elide` (on by default, a local record copied out).
Two facts the image does not hold are refused where read: `Function::is_arm_consumed`, and
`Data::value_structs` — not refusable at all, so a `value struct` binding shows as differ
(`R-ValueRecord`, 1 definition).

A summary of a callee (`use_analysis::return_ownership`) reads the callee's body as Rust has
it at that moment: SETTLED for a function before this one.  Where this pass refused such a
callee, the image holds only its parser body, so the oracle asked about a value whose call
closure reaches one is refused by name (`check::stale_callees`, `Scopes::stale_defs`).  It
shrinks as callees are carried.

| pass | programs agree | definitions agree (step-1 set) | unscoped | differ |
|---|---|---|---|---|
| the slice | 142 / 2277 | 19 878 / 37 762 | 16 780 | 1 104 |
| step 1 | 127 / 2277 | 19 725 / 37 762 | 17 898 | 139 |
| step 2 | 238 / 2277 | 22 622 / 37 762 | 15 000 | 140 |

With the libraries' 2 198 definitions: 24 435 of 39 960 agree (1 813 of the libraries').  No
definition step 1 agreed on is lost (`--verdicts` files compared after every change).  Overlap
with the slice: 18 876 under both, 3 746 only under the transcription, 1 002 only under the
slice — 435 closure drop cascades, 112 generator returns, 90 `pure_reuse`, 55 tuple returns, 50
`text_replace`, 46 `loop_kernel_rewrite`, 41 vector return buffers, 35 `copy_view`, 33 keyed
returns; the records family is no longer the gap, so step 3 is not yet due.

Step 3's order — the families by definitions where they are the only blocker:

| family | sole | top refusals |
|---|---|---|
| records (the refusals above) | ≈1 570 | the view walk 360, `copy_view` 470, `sink_return_branch`'s non-sinkable arms, `reassociate_coalesce_chains` 160, drop cascades 195 |
| closures and fn-refs | ≈1 170 | `mark_borrowed_captures` 888 |
| vectors | ≈1 000 | `vector_rebind_release`, the vector bindings and exits |
| heap arguments | ≈470 | `inline_struct_return` 365 |
| a stale callee summary | 260 | shrinks with every family carried |
| keyed, generators, `text_replace`, `pure_reuse`, tuples | 200, 190, 180, 120, 65 | |

**Step 3 (2026-11-27) — fn-refs, value structs, text replace, the cascades' deciders.**  New
files: `scope_value_struct.loft` (`value_struct::value_struct_copy`, run by `check` before the
loop on the parser's bodies; a summary it asks of a body a refused pre-loop rewrite would change
is refused), `scope_text_replace.loft` (`text_replace::admit` after the loop, on the settled
bodies), `scope_capture_adoption.loft` (the decidable part of `mark_borrowed_captures`:
`capture_attr_is_cascade_relevant`, `prune_empty_cascades`, the `cascade_before_record_frees`
trigger).  A fn-ref local in a frame with no closure record of its own is carried:
`register_binding`'s record order, `closure_keep::fnref_call_rebind` (`__fdisp_N`, in
`scope_convert.loft`), `get_free_vars`' fn-ref release; a frame `closure_keep_set` gates (a
holder shared) is refused.  The image's `Variable.arm_consumed` and `Definition.value_struct`
are read.

Two post-loop decisions read facts of OTHER functions' settled bodies, which a refused function
does not have.  Each is taken twice — the refused functions' unknowns read one way, then the
other — and a definition the two answers disagree on is refused by name: the pruned cascades
(a call of a cascade whose pruning is undecided), and `safe_params` (a text-field write through
a parameter whose refused callers decide it; a hook or cascade counts as called by any refused
body, which may hold the call its parser body lacks).

A closure record's cascade is final only once the building frame's capture verdicts are in, and
every closure record with a cascade has a capture that holds a store, so all of them (1 037
definitions, the largest sole refusal) wait for the closure frames to be carried: those frames
are refused for the whole closure machinery (`capture_builds`, `closure_keep`, the closure
record's drop hook and hand-offs, `stored_refs`, the captured owner witness) and mostly for
vectors as well.  4 struct cascades the slice and step 2 agreed on are now refused: each calls a
closure cascade whose pruning a refused frame decides (they agreed because Rust did not prune).

The survey pairs a definition only Rust's image holds — a retired cascade (`<name>_retired`), a
`shared_call` variant (`<callee>__given_<f>`) — with the refusal that decides it.

| pass | programs agree | definitions agree (step-1 set) | all definitions | unscoped | differ |
|---|---|---|---|---|---|
| step 2 | 238 / 2277 | 22 622 / 37 762 | 24 435 / 39 960 | 15 524 | 1 |
| step 3 | 256 / 2277 | 22 926 / 37 762 | 24 744 / 39 960 | 15 216 | 0 |

Overlap with the slice: 18 898 under both, 980 only under the slice — 438 closure cascades,
112 generator returns, 90 `pure_reuse`, 55 tuple returns, 47 `loop_kernel_rewrite`, 41 vector
return buffers, 36 `copy_view`, 33 keyed returns, 24 fn-ref returns; step 3 of the plan is not
yet due.  Sole refusals now: the closure cascades 1 037, `inline_struct_return` 390, a stale
callee summary 269, `copy_view` 306 (195 join, 111 variable), the view walk 153, `pure_reuse`
120, `drop_hook` 87, a `&` variable 84, `rebind_place` 70.

Two loft defects met, both backends, worked around in the transcription (not filed yet):
- silent: `fn keep() -> vector<integer> { v: vector<integer> = [1, 2, 3]; v = [for k in v { k }]; v }`
  answers `[]`, not `[1,2,3]` (in `main`, without the return, it answers right);
- a double free: `enum Node { Leaf { n: integer }, Span { pos: Pos, inner: Node }, Set { var:
  integer, inner: Node } }` with `struct Pos { file: text, line: integer }`, walked by
  `kids(n)` (`[inner]`), `[for c in kids { visit(c) }]` and a rebuild `Set { var: var, inner:
  ks[0]! }` where `visit` answers `Span { pos: pos, inner: visit(inner) }` — "a store was freed
  twice" on `Set { 1, Span { Pos { "f", 3 }, Leaf { 7 } } }`.

The walks rebuild each variant in place, as `scope_wrapper` does.

**Step 4 (2026-11-27) — vectors and the settled rewrites the whole path stops at.**  Vector
locals are carried (1bd9b9c66: bindings, rebind releases, lazy buffer mints, confinement,
last-use reclaim, `bound_append`).  Then, each gated: `scope_join_rewrite.loft` gains
`reassociate_coalesce_chains`; `scope_buffer_detach.loft` (`detach_owned_buffers`, after the
loop); `scope_take_local.loft` (`(R-MoveLast)`'s local clause), `scope_loop_kernel.loft`
(`loop_kernel_rewrite`) and `scope_byte_copy.loft` (`byte_copy`'s text clause, with
`compact::match_loop`) run by `check` on the settled body in Rust's order; `scan_args` hoists the
guarded slot mints of a `&` argument (`is_slot_hoisted`, @C118); a rebound parameter is released
at the exit (`OpFreeRefIfDistinct` against its entry witness).  `Function::was_loop_var` is read
off the body (`scope_phase::loopish_in`): only a variable set inside a loop or a `par` body, an
iterator's, or the `#fields` walk's `StructField` binding may be one.

The compact / slice_into trigger ("a vector appended to") had hidden two rewrites; each is now
named on its own: `move_elide::elide_borrows` (refused by name, a superset of
`use_analysis::elision_plans`: a local vector filled by one append from a variable or its field)
and `take_local` (transcribed).  Still refused by name: `byte_copy`'s vector clause (it needs
`vec_copy::shared_element_type` and `text_run`).

| measured | programs agree | definitions agree | differ |
|---|---|---|---|
| step 3 | 256 / 2277 | 24 744 / 39 960 | 0 |
| vector locals (1bd9b9c66) | 427 / 2277 | 27 130 / 39 960 | 0 |
| + join rewrite, buffer detach, take_local, slot mints | 436 | 27 630 | 0 |
| + byte_copy | 469 | 27 882 | 0 |
| + was_loop_var read off the body | 477 | 27 923 | 0 |
| + rebound parameters | 498 | 28 099 | 0 |
| + loop kernels | 518 / 2277 | 28 205 / 39 960 | 0 |

No definition that agreed was lost at any step (`--verdicts` files compared).  The whole path
(`run_image_survey.py --front`, 1 188 programs) went 142 → 147 agree with the join rewrite,
take_local and slot mints in, and → 221 with the rest of the step (d3f88a747): 0 differ, leaks
29 → 6, faults 3 → 1.  One leak is new, a program that reaches the runner for the first time
(`R-StackBase/a-stack-that-grows-mid-call-keeps-every-frame.loft`).

What stops the most programs now (scope gate, programs whose ONLY refusal it is): the view walk
55, `inline_struct_return` 43, `pure_reuse` 33, `arm_lift::arm_bind` (a heap local as a branch
arm) 30, `vec_copy` 21.  By first refusal: the view walk 533 programs, `inline_struct_return`
408, `move_elide` 399, `pure_reuse` 326, a stale callee summary 315, keyed bindings 189,
`arm_bind` 177, `elide_borrows` 148, closure records 138.  The view walk is
`view_walk.rs` + `disturbance.rs` (≈3 200 lines, with the scan's materialise legs); `vec_copy`
and `repeat_run`'s local clause need the schema's vector type (`Data::vector_element_type`,
ported privately in `ir_fill_loop.loft`).

A loft defect met, both backends, worked around in `scope_byte_copy::chain` (not filed yet): a
record local first bound to a vector ELEMENT, rebound in a loop to a literal that holds its own
old value, loses that value on the second pass — `cond = tests[2]!; while k >= 0 { cond = Pair
{ c: Leaf { n: k }, t: cond }; k -= 1; }` answers `(0 null)`, not `(0 (1 3))`; one pass, or a
first bind from a literal, answers right.

**Step 5 (2026-11-27) — the view walk's question, `vec_copy`.**  `scope_view_walk.loft` is
`view_walk::collect_views_to_materialise` with `disturbance.rs`'s producers, asked as the
question the scan's materialise legs need answered: is a view used after a disturbance of its
container?  Where the answer is no the function is carried; where it may be yes it is refused
by name ("a view used after a disturbance of its container (the materialise legs)") until those
legs are transcribed.  The walk is Rust's — the frames, the value-branch `Set`, the `if` arms
from one state, the loop's head shake and back edge with its provisional records,
`record_target` with `resolve_view_root` and `maybe_own`, `note_uses` — and every SPARING is
left out (the literal key and index, the whole-container link, the payload slot handles, the
callee-argument discount), so its answer is a superset of Rust's.  The open views are kept
exactly Rust's, because `resolve_view_root` follows the first open entry of a view and a
superset of entries would resolve a place to another root.  Facts the image does not hold:
`is_iteration_source` is read off the `_vector_N` name the three loop-source sites mint; a
keyed binding is a candidate view (`keyed_views`, `amp_container_link`); a whole record written
over a place the walk cannot size (`OpCopyRecord`, a tagged slot's clear) is the whole variable.
The callee half (`disturbed_params_map`) is `opt_vr_disturb::params_map`, built once by `check`
before its loop.  `scope_vec_copy.loft` is `vec_copy::rewrite`, run after `byte_copy`
(`ir_fill_loop::vector_element_type` made `pub` for it).  `arm_lift::arm_bind`'s decision is
transcribed in `scope_args.loft` (`call_arm_binds`, `var_arm_binds`): a heap branch arm refuses
only where Rust gives it a binding of its own (a named call answering a borrowed or joined
record, or an owned one with no binding named; a plain record or vector variable a bind copies),
since `lift_join_arm_tails` and the lists it leaves are not transcribed; `whole_value_hoists` is
read as every `??` hoist, a superset.

| measured | programs agree | definitions agree | differ |
|---|---|---|---|
| step 4 | 518 / 2277 | 28 205 / 39 960 | 0 |
| + the view walk's question | 564 | 28 598 | 0 |
| + `vec_copy` | 591 | 28 783 | 0 |
| + `arm_bind`'s decision | 635 / 2277 | 28 962 / 39 960 | 0 |

The view walk still refuses 269 definitions (75 programs); in 64 of those programs Rust's own
walk materialises a view (`LOFT_DEBUG_F8`), so the materialise legs are what is left of it (the
superset's own excess is 10 programs).  Programs whose only refusal family it is: 11.  Sole
families now, by programs: `inline_struct_return` 75, `pure_reuse` 42, `place_result` 17,
`arm_lift` 15 (the lifts Rust makes), `copy_view` 14, the view walk 11.

The whole path (`run_image_survey.py --front`, 1 188 programs, with the runner's @C171 null-text
fixes and three tool-leak workarounds beneath it, which alone read 228): 262 agree with the view
walk's question and `vec_copy`, 278 with `arm_bind`'s decision; 0 differ, 0 fault, 0 leak.  The
view walk is now the first refusal of 9 programs (132 before).  By first refusal:
`inline_struct_return` 82, `move_elide` 67, keyed bindings 63, `pure_reuse` 51, `drop_hook` 42,
`arm_bind` (Rust's own lifts) 27, closure records 26, `copy_view` (join) 23, `elide_borrows` 22,
`exit_vector` 20.

**Step 6 (2026-11-27) — inline returns, `pure_reuse`, `forward_result`, keyed locals.**
`scope_inline_returns.loft` is `inline_returns.rs` whole: `inline_struct_return` answering the lifted
temp's type (the fn-ref call through `fnref_target` with its capture and witness gates, the `??`
and `!` blocks with `ncc_flat_arms` and the local-arm bases, the monomorph facts read off a
callee's body — `return_sites`, `site_is_fresh`, `monomorph_return_is_fresh`, the delegated and
fn-ref-slot chains — and the native constructors).  `scan_args` makes every lift Rust makes in
its order (#248's ordering lift, the A5.6 / P135 / @C118 hoists with their final lift, the
`Insert`-tail lift, the plain lift with `mark_lift_handoff` and `home_lasts_as_its_holder`, the
coalesce arm, the generator handle) and `keep_operand_order` (`@FR-E-Left`); `new_lift_var` takes
the pending join witnesses, `lift_set` the literal accumulator and the minted-store pairing, the
`Drop` arm of the scan lifts a discarded result, and a consumed lift is read back as null after
its call.  A body a refused callee would answer for (`stale_defs`) refuses only where the answer
reads it: a scalar or text result answers `None` before any body is read.  Refused by name: the
host-store literal clause (`host_store::literal_clause`, a call's answer lifted into a child
field), the `&` write-back copy (`amp_writeback_owned_copy`), a tuple literal's member lifts.
`scope_pure_reuse.loft` is `pure_reuse::rewrite_program` with `shared_call::total` and
`sharing_of`, run by `check` on the parser's bodies before `bound_append`; a sharing wrapper's
variant (a new definition) and a total call's vector handed in (`hand_in`) refuse by name.
`scope_forward_result.loft` is `(R-ForwardResult)` whole: the port had no refusal for it, and the
`pure_reuse` trigger had hidden the one program that reaches it.  Keyed locals take the collection
legs (the @P302 clear, the self-dep and minting-null-init releases); a keyed type whose records
hold a generator refuses by name, and a local keyed record stored through a parameter's reference
is `insert_evacuations`' trigger.  A `par` loop's element variable (`_<name>_par<N>`) is a loop
variable.

| measured | programs agree | definitions agree | differ |
|---|---|---|---|
| step 5 | 635 / 2277 | 28 962 / 39 960 | 0 |
| + `inline_struct_return` and the lifts | 719 | 29 620 | 0 |
| + `pure_reuse`, `forward_result` | 788 | 29 878 | 0 |
| + keyed locals | 871 / 2277 | 30 227 / 39 960 | 0 |

No definition that agreed was lost at any step.  The whole path: 278 → 312 → 348 → 394 of 1 188
agree, 0 differ, 0 fault; one leak is new — a program the scope pass now carries reaches the exit
leak check for the first time, and the three records it leaves are the elaborator's own `Ex`
(`el_ex_base`), a tool leak beneath the scope pass.  By first refusal now: `move_elide` 70,
`drop_hook` 42, `arm_bind` 35, `copy_view` (join) 27, closure records 26, `literal_host` 26,
`elide_borrows` 26, `exit_vector` 22.  `move_elide` and `elide_borrows` are one port: the use
analysis' `analyze_fn` (`collect_uses`, the parser's `find_written_vars`) beneath `move_plans` and
`elision_plans`, then `move_elide.rs` and `construct_elide.rs`.

Two programs leave the agree column without a wrong answer: `store_iter_scratch.loft` prints its
store report, which inside `run_front` lists the runner's own store too (`stores`), and
`store_handoff_residency.loft` reaches a node the bytecode writer in loft does not carry
(`unwritten`).

A loft defect met, silent, both backends, worked around in the port (not filed yet) — move
elision builds a record over the local its own source views: `scanned = mk(i); match scanned {
Span { pos, inner } => match inner { Ins { items } => { ops: vector<Node> = [for x in items { x
}]; scanned = Ins { items: ops }; } } }` answers one item where `items` held two
(`LOFT_NO_MOVE_ELIDE=1` answers right).  `scan_set` builds that node in a call
(`scope_fn::insert_of`), which no construct rewrite reaches.

**Step 7 (2026-11-27) — the settled rewrites, the move elisions, the arm lifts.**  Measured
against loft-suite `45cb6a2` and a binary with loft#2668 fixed (dc44e24bd); the step-6 baseline
re-taken there reads 871 of 2 282 programs, 30 296 of 40 088 definitions, and 400 of 1 189 on the
whole path.  New files, each a Rust module transcribed whole: `scope_slice_text.loft`,
`scope_slice_into.loft`, `scope_repeat_run.loft` (a run into a local whose `vector<elem>` the
schema does not hold refuses: Rust's `Stores::vector` mints it), `scope_literal_append.loft`,
`scope_literal_host.loft` (`Function::is_ref_named` is not in the image: an admitted literal in a
program with a `reference<T>` field refuses), `scope_copy_view.loft` (the plain, join and walk
clauses), `scope_compact.loft` (`match_loop` is `scope_byte_copy`'s, now `pub`),
`scope_use_analysis.loft` (`collect_uses`, `caller_stores_stable`, `elision_plans`, `move_plans`;
the survival split is off by default, so `loop_survives` is false), `scope_move_elide.loft`
(`elide_borrows`, `move_elide`'s record shape) and `scope_construct_elide.loft` (B1.3b–d).  The
move elisions run in `check` over every function on the parser's bodies, `elide_borrows` first, as
Rust's do; `Variable::viewed` (every earlier bind's deps) is not in the image, so B1.3c/B1.3d ask
`touches_store` twice — the current deps, and every multiply-bound local as a possible viewer — and
refuse where the answers differ.  `place_result`'s trigger is narrowed to its callee clause (a
local bound from a loft call with a hidden return buffer, copied into an appended element).
`arm_lift::lift_join_arm_tails` is transcribed for record arms (`arm_bind`'s `Bind`, the owned
call tails, the deps rewrite, `whole_value_hoists_in`); a vector variable arm (`CopyVector`) and a
lifted copy that moves a drop refuse.  `sink_set_into_arms` and `rewrite_written_out` are carried
where the scan writes a reassignment out per arm (keyed by the binding and its value's text where
Rust keys by address, then the rescan with `sunk`); a reassignment kept in its value form beside
that rule refuses (the promoted return buffer's arm work-refs read otherwise).  A copy that frees
its source (`0x8000`) disarms its `__ref_N` work-ref (`drop_transferred`).  `body_block_mut`'s
`Insert([Set(w, null), Block])` form is read by the lazy mints, the record-buffer reuse and the
entry prepends (`f29_build` had lost its lazy mint).  The loft#2667 workaround in
`scope_byte_copy` and the loft#2668 one in `scope_scan_set` are gone.

| measured | programs agree | definitions agree | differ |
|---|---|---|---|
| step 6, re-taken | 871 / 2 282 | 30 296 / 40 088 | 0 |
| + `slice_text`, `slice_into`, `repeat_run`, `literal_append`, `literal_host` | 927 | 30 664 | 0 |
| + `copy_view` | 978 | 31 172 | 0 |
| + `move_elide`, `elide_borrows` | 1 067 | 31 766 | 0 |
| + `compact`, the hoisted body block | 1 145 | 32 148 | 0 |
| + the arm lifts | 1 212 | 32 500 | 0 |
| + the copy that frees its source | 1 241 | 32 663 | 0 |
| + the written-out reassignments | 1 260 / 2 282 | 32 729 / 40 088 | 0 |

No definition that agreed was lost at any step.  The whole path: 400 → 416 → 434 → 470 → 511 →
513 → 525 → 556 of 1 189, 0 differ.  Two faults are new and neither is a wrong answer of
the scope pass: `2568-parallel-arms-run-at-once` asserts — the runner in loft runs `parallel`
arms one after another, so an arm that waits for a later one times out — and
`a-function-of-many-record-temporaries-compiles-in-seconds` takes 106 s in the development-built
`run_front` against the survey's 60 s (its output is right).  The leaks the whole path now
reports are the tools' own stores, counted because the runner shares the C runtime with the
program it runs: an `NdSet` per `??` bind out of `scope_join_rewrite::reassoc_walk` (24 programs,
worked around below), the elaborator's `Ex` (two programs) and two `NdIf` records in
`2591-a-name-an-alternation-captured-ends-with-its-arm` (not traced yet).

Sole refusal families now (scope gate, programs): `was_loop_var` 28, the view walk's materialise
legs 26, `unnameable_borrow_source` 21, `place_result` 17, `rebind_place` 15,
`insert_evacuations` 15, a value-form reassignment beside `sink_set_into_arms` 12.

A loft defect met, a leak, both backends, worked around in `reassoc_walk`: a local bound from a
join of a parameter and a call's fresh record is not released where the function answers another
value — `node = if deep { rebuild(n, A { x: 7 }) } else { n }; r = alt(node); if r != null { r! }
else { node }` leaves one `S` store at exit (`--interpret` and `--native-c`).

**Step 8 (2026-11-28) — the image facts, the borrow sources, `rebind_place`, `host_store`, the
`@FR-H-RefHome` legs, the `&` bindings.**  Four facts the scope pass reads are now in the image,
each written by `ir_store`, read back by `ir_read` and set by the loft elaborator where it has
them: `Variable.was_loop_var` (byte 64; the record stride 64 → 72; `el_vars::stored` sets it off
`loop_vars`), `Variable.place_buffer` and `Variable.ref_home` (bytes 65, 66) and
`Definition.ref_evacuations` (four integers each, at byte 200; the definition's flags move up
by 4).  The last three are `stored_reference_homes`' marks, which the loft elaborator does not
make: it refuses the hoisted homes and the placements (`el_refhome`), but not every evacuation,
so a program whose parser records one and which the elaborator accepts reads none on the whole
path — owed to the elaborator, not seen on the corpus yet.
`scope_phase::loopish_in` and the par-variable name test are gone.

New files, each a Rust module transcribed whole: `scope_host_store.loft` (`host_store.rs`, the
write-back and literal clauses, after the loop in definition order; a decision that reads a
refused callee's body refuses its function), `scope_host_result.loft` (`host_result::admit`;
its rewrite is not transcribed, so a function it admits refuses), `scope_rebind_place.loft`
(`rebind_place.rs`, after `literal_append`, on the body put in place first because the oracle
reads the definition's own code), `scope_stored_refs.loft` (`keep_placed_results`,
`insert_evacuations`).  In `scan_args`, `inline_built_borrow_source` (`hoist_built_block`) and
`unnameable_borrow_source` (`lift_view_deps`, `own::bracket_can_name`) replace the superset
refusal.  `own::displaced_owned_slots` (`reassign_sites_of`'s owned-then-borrowed slots) feeds
`scan_set`'s `local_source` strip; the right-hand sides it classifies take the stale-callee
guard.  `last_use::reclaim_safe` and `store_confinement` read `is_ref_home`.  Three refusals
were wider than the legs behind them and are narrowed by measurement: a `&` binding to a record
or vector takes the record and vector legs (`publishes_through_ref`), a `&` local releases
nothing at a scope exit (no arm of the sweep matches `RefVar`), and a value-form reassignment
beside the written-out rule is carried but where the binding is the frame's promoted buffer or
an arm is a `??` chain (those still differ: the construction work-refs a promoted buffer takes
over, and a sink the port declines where Rust writes the arms out, `c131` `rb = rs as Circle`).

`Variable::viewed` is still not in the image: `rebind_place` asks the callee's parameter viewers
twice (current and parse-time deps; then every multiply-bound local added) and refuses where the
answers differ.  Carrying `rebind_place` exposed a lag behind Rust: `join_rewrite`'s owner block
for a minting call arm declines a callee answering a borrowed view (loft#2665), now ported.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 7 | 1 260 / 2 282 | 32 729 / 40 088 | 0 | 556 |
| + `was_loop_var` in the image | 1 288 | 32 791 | 0 | 574 |
| + the borrow sources, `host_store`, `host_result`'s admission | 1 313 | 32 901 | 0 | 578 |
| + `rebind_place` | 1 349 | 33 208 | 0 | 590 |
| + `place_buffer`, `ref_home`: `keep_placed_results`, the reclaim and confinement legs | 1 366 | 33 359 | 0 | |
| + `ref_evacuations`: `insert_evacuations` | 1 386 | 33 449 | 0 | |
| + the value-form reassignment, narrowed | 1 396 | 33 485 | 0 | 602 |
| + `displaced_owned_slots` | 1 408 | 33 533 | 0 | |
| + `&` bindings to a record or vector, `&` locals at a scope exit | 1 440 / 2 282 | 33 677 / 40 088 | 0 | 619 |

No definition that agreed was lost at any step (`--verdicts` compared after each); the whole
path lost no program either, and its faults and leak are step 7's.  Whole-path runs and their
images must come from one `loft` binary: a rebuild with a new image layout mid-run faults every
program (the runner refuses the image), so the gates here ran a frozen copy of the binary.

Sole refusal families now (scope gate, programs): the view walk's materialise legs 44,
`owner_witness_locals` (a record local whose assignments mix ownership) 22, `place_result` 22,
the value-form reassignment (promoted buffer, `??` arm) 9, `callref_collection_join_base` 8, a
fn-ref return's join hooks 4, then 3 each: `return_field`, `keep_viewed_displaced`,
`construct_elide`'s `Variable::viewed`, `amp_writeback_owned_copy`.  By first refusal the
stale-callee summary leads (346 programs), then closure records 143, tuple bindings 132,
`return_field` 123, closure cascades 121.  `Variable::viewed` is now the one parse-time fact
three ports approximate (`construct_elide`, `rebind_place`'s callee viewers): the next image
fact to add.

A loft defect met, both backends (a lint, so the front end): `dead-assignment` calls an init
dead when a `while` body assigns the variable unconditionally, though a loop that runs no turn
reads the init — `hit = false; k = 0; while k < len(v) { k += 1; hit = true; } hit` warns on
`hit = false` (removing it as told breaks the empty case).  Worked around in
`scope_host_store::literal_clause` (a count).

**Step 9 (2026-11-28) — loft#2671, `Variable::viewed`, `return_field`.**  Re-taken on
`c19671e61` (the loft#2671 fix beneath step 8): 1 440 of 2 282 programs, 33 677 of 40 088
definitions, 0 differ — the corpus (suite `45cb6a2`) holds no join return of the issue's shape.
`free_vars.rs`' new join leg is transcribed all the same (`view_sources`, the backings released
unless returned, the guards re-keyed on the value returned, `LOFT_NO_JOIN_RETURN_BACKINGS`): a
probe of the issue's two shapes differed before and agrees now.

`Variable::viewed` is in the image (Variable byte 48, a `vector<integer>`; the flags after it
move up by four, the stride stays 72), written from `Function::viewed_of`, read back by
`set_viewed` and set by the loft elaborator from its own `viewed`.  `scope_fn::viewers_of` is
`Function::viewers_of`; `construct_elide`'s reorders and `rebind_place`'s callee viewers read it,
and their two-answer approximations and refusals are gone.

`scope_return_field.loft` is `return_field.rs` whole (`@FR-R-ReturnField`, the plain and the
presence-tested exits), run after `literal_append`.  Its gain is mostly the stale-callee
cascade: a function refused for `return_field` refused every later caller whose ownership
summary reads it.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 8, re-taken on `c19671e61` | 1 440 / 2 282 | 33 677 / 40 088 | 0 | 619 |
| + loft#2671's join leg | 1 440 | 33 677 | 0 | |
| + `Variable::viewed` in the image | 1 444 | 33 684 | 0 | 620 |
| + `return_field` | 1 508 / 2 282 | 34 197 / 40 088 | 0 | 647 |

No definition that agreed was lost at any step, nor a program on the whole path; its faults and
leak are step 7's.

**The stale-callee refusal** (`use_analysis::return_ownership: a callee laid out earlier was
refused`, the first refusal of 274 programs) is not a mechanism of its own.  Rust's ownership
oracle reads a callee's return summary off the callee's body as it stands when the caller is
laid out — settled, for a callee laid out earlier.  Where this pass refused that callee the
image holds only its parser body, so the caller is refused by name rather than answered from
the wrong body.  It never stands alone: no program's refusals are this family only.  Its 517
definitions refused for nothing else come back as the families that refuse their callees are
carried — `return_field` alone took it from 675 definitions to 517.  The families behind it now,
by programs that have a stale-callee definition: a returned join's hooks (`returns`) 106,
`exit_vector` 106, drop cascades 83, `scan` 68, `const_fn` 62.  `exit_vector.rs` (678 lines)
is the next transcription with that leverage.

Sole refusal families now (scope gate, programs): the view walk's materialise legs 46,
`place_result` 23, `owner_witness_locals` 22, the value-form remainder 10,
`callref_collection_join_base` 8, a fn-ref return's join hooks 4, `keep_viewed_displaced` 4,
`amp_writeback_owned_copy` 3.  The owner witness is about thirty sites (the mint before the
scan, `view_bind_bases`, the per-`Set` release and alias hooks in `scan_set`, the scope-end
`release_witness`, the split propagation) rather than one module; the view walk's legs need its
sparings transcribed first, so that the walk answers exactly where it now answers a superset.

**Step 10 (2026-11-28) — `exit_vector`, loft#2673/#2674, `const_fn`.**  `scope_exit_vector.loft`
is `exit_vector.rs` whole (`(R-ExitVector)`): the admission records each site by its path
through the body, as Rust's does, and the plans are applied by those paths; the wrapper's field
type is compared as `Type`'s equality reads it (an integer's `forced_size` left out).
`scope_const_fn.loft` is `const_fn.rs` whole (`@FR-R-Const`: the twin copy and the read-only
call's constant view, with `use_analysis::read_only_uses`, whose admitted calls Rust keys by
address and the port by path).  The image carries the one fact it needs,
`Definition.const_linked` (byte 214, `literal_const != u32::MAX`; `ir_read` relinks the twin by
name), because a twin pass 1 made and pass 2 unlinked keeps its `__const_` name.  One case is
refused by name: a twin copy whose element is a tuple (`Data::vector_element_type` reaches a
tuple through `type_elm`, which the port does not derive).

The Rust fixes for loft#2673 and loft#2674 (fx `4867bd642..e792815ae`) are cherry-picked
beneath, and the port follows their scope-pass halves with their switches: a `??` hoist promoted
onto the return buffer takes no plain transition free where it has an entry witness
(`LOFT_NO_NCC_ENTRY_GUARD`); `delivered_work_ref` needs every return to hand the parameter back
(`LOFT_NO_DELIVERED_EVERY_RETURN`); a first bind of a delivering call is not hoisted out of its
preamble (`LOFT_NO_DELIVERED_FIRST_ADOPT`).  Without the port they made five definitions differ.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 9 | 1 508 / 2 282 | 34 197 / 40 088 | 0 | 647 |
| + `exit_vector` | 1 563 | 34 521 | 0 | 673 |
| + loft#2673/#2674 and the port's halves | 1 563 | 34 521 | 0 | (with the next) |
| + `const_fn` (`const_linked` in the image) | 1 604 / 2 282 | 34 800 / 40 088 | 0 | 689 |

No definition that agreed was lost at any step, nor a whole-path program.  The loft elaborator
does not carry the parser halves of loft#2673/#2674 (`ref_return`'s buffer note, the self-copy
guard); the whole path showed no difference for it on the corpus.  The stale-callee cascade: 376 definitions are
refused for nothing else (517 at step 9), in 223 programs (340); its first refusal now stands in
160 programs.  Behind it, by programs: a returned join's hooks (`returns`: keyed, generator,
tuple and fn-ref returns) 103, drop cascades 81, `scan` 62, `scan_set` 46, `arm_lift` 43, the owner
witness 43, `exit_record` 33, tuple members 30.  By first refusal: closure records 143, tuple
bindings 134, closure cascades 121, drop hooks 85, generator handles 71, keyed and generator
returns 70 each.

Sole refusal families now (programs): the view walk's materialise legs 52, `place_result` 29,
`owner_witness_locals` 23, the value-form remainder 11, `callref_collection_join_base` 9.

**Step 11 (2026-11-28) — `exit_record`, the `returns` family, handles, the keyed branch.**
`scope_exit_record.loft` is `exit_record.rs` whole (`(R-ExitVector)`'s record clause), run by
`check` after `exit_vector`; `scope_exit_vector` gains Rust's `Ops::lookup` and `exit_guard` for
it.  `free_vars` no longer refuses a fn-ref, keyed, tuple or generator return (none builds the
`materialized_view_return` copy `move_join_hooks_into_arms` and its siblings read), and the legs
those returns reach are transcribed: loft#1142's keyed join, the @P329 tuple-of-text staging,
@P378(b)'s lifted-condition `if` (missing before for records and vectors too),
`renumber_text_buffer` for a tuple's members, `insert_free`'s fn-ref block hoist (loft#1469), and
`(G-Hold)`'s function-exit release of a generator's handle parameters.  `scope_handles.loft` is
`handles.rs` whole: `retain_shared_handles` runs before the scan in place of the refusal of every
body with a handle local, a handle binding takes `scan_set`'s rebind release (loft#835) and
`get_free_vars`' scope-exit release.  `rewrite_keyed_replace_branch` (loft#1320) is in
`scope_args.loft` beside the other `arm_lift` ports.  `scan_set`'s `@FR-B-View` lift dep now
uses the whole `Type::with_deps` port (`scope_inline_returns::with_deps`, which gains its
`RefVar` and `Tuple` legs): its own copy knew records and vectors only, so a keyed local bound
to a projection of a lifted call kept owning.

Refused by name, in the phase: a generator handle moved into a field or an element
(`handoff::handle_handoff`: `collect_drop_transferred` and the arm hand-off flag are not
transcribed), and a handle local given both a member's handle and one of its own
(`mixed_handle_views`' per-path flag).

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 10 | 1 604 / 2 282 | 34 800 / 40 088 | 0 | 689 |
| + `exit_record` | 1 615 | 34 852 | 0 | |
| + fn-ref returns | 1 620 | 34 878 | 0 | |
| + keyed returns, `with_deps` | 1 662 | 35 292 | 0 | |
| + tuple returns, @P378(b) | 1 662 | 35 354 | 0 | 704 |
| + generator functions | 1 662 | 35 575 | 0 | |
| + `handles.rs`, handle locals | 1 704 | 35 818 | 0 | |
| + `rewrite_keyed_replace_branch` | 1 712 / 2 282 | 35 835 / 40 088 | 0 | 720 |

No definition that agreed was lost at any step, nor a whole-path program; its faults and leak
are step 7's.  The stale-callee cascade: 165 definitions refused for nothing else (376 at step
10), in 58 programs; behind it, by programs, `arm_lift` 23, the owner witness 16,
`place_result` 14, tuple members 8.  By first refusal: tuple bindings 127, closure records 126,
the view walk 63, drop hooks 57, the owner witness 45, `place_result` 43.

Sole refusal families now (programs): tuples 123 (392 definitions: `scan_set`'s tuple
binding, `register_binding`'s member backings, `tuple_owned_elem_frees`), the view walk's
materialise legs 60, `place_result` 38, drop cascades 32, the owner witness 32, `arm_lift` 27
(the value-form remainder 25), `callref_collection_join_base` 10.  Tuples are the next lever:
`tuple_members.rs` (533 lines) with the tuple legs of `scan`, `scan_set`, `args` and
`get_free_vars`.

Two loft defects met, both backends:
- silent: a vector literal assigned to a field of an element whose index is a field read is built
  in place, its destination cleared before a call inside the literal reads it —
  `d.vs[p.i].xs = [first(d, 0)]` stores `[-1]` where `first` reads `d.vs[0].xs[0]` (7); with
  `k = 0; d.vs[k].xs = …` it is right.  Worked around in `scope_exit_record::apply` by binding
  the value first;
- a parse error: a struct literal of a struct declared LATER in the file does not parse when a
  field value holds a range whose start is an integer literal written tight —
  `Ro { ok: [for _ in 0..n { true }] }` reports "Expect token ;" at `n`; `0 .. n`, `z..n`, or
  the struct declared first all parse.  This is step 10's `scope_const_fn` failure: the
  qualified call was not the cause.

**Step 12 (2026-11-28) — tuples.**  `scope_tuple_members.loft` is `tuple_members.rs` whole (the
owned-element and text-member frees, the claimant disarms, `tuple_call_mints` and its branch
join, `member_mint`, `member_write`, `tuple_claimants_in`, `written_tuple_members_in`).  The
tuple facts are `Scopes` fields (`tuple_call_mint`, `tuple_member_now`, `tuple_moved`,
`tuple_depth`, `written_tuple_members`, `tuple_claimants`, `owned_handle_members`; a per-member
map is a `scope_scan::TMap`), and the tuple legs are transcribed where Rust has them: `scan_set`
(the unbox dep, the tuple transition free, the member backings and call mints, the first
binding's depth, the member backings' turn in the sweep), `register_binding` (the member
backings, `backings::member_backing`), `get_free_vars` (`claimants_after_their_tuples`,
`outer_tuple_backings`, the tuple leg), `scan_if` (the per-arm reconcile, a statement `if`'s
claimant join and disarms), the loop and `Iter` reconciles, `convert` (the member moves a
statement arms, loft#1645's per-path flags, which the phase sets false at the body's head) and
`scan`'s member write.  Refused by name where a drop rides the move:
`call_minted_member_handoff` (a call-minted member of a type with a drop cascade) and a vector
member of droppable elements; `drop_bearing_source`'s tuple member (`tuple_member_backing`,
which reads `Function::tuphold_origin` and `Vars::tuple_backings`) and
`lift_tuple_call_members` still refuse.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 11 | 1 712 / 2 282 | 35 835 / 40 088 | 0 | 720 |
| + `tuple_members.rs`, the tuple legs | 1 834 / 2 282 | 36 238 / 40 088 | 0 | 747 |

No definition that agreed was lost, nor a whole-path program.  The stale-callee cascade: 155
definitions refused for nothing else, in 53 programs.

Sole refusal families now (programs): the view walk's materialise legs 60, `place_result` 38,
the owner witness 35, drop cascades 34, `arm_lift` 31, closures 15.  By first refusal: closure
records 128, the view walk 63, drop hooks 57, the owner witness 46, `place_result` 43.

Drops and closures are ONE arc.  Of the 144 programs a closure refusal stops, 122 are also
stopped by a drop refusal: a closure record's capture cascade is a drop cascade, and its
hand-offs are `handoff.rs`'s.  Only 15 programs are closures alone, and 40 are drops alone (68
with other families).  Its Rust: `drops.rs` (770 lines, about half ported) and `handoff.rs`
(1 009: the hand-offs, the per-path flags, `collect_drop_transferred`, `arm_container_handoffs`,
`collection_handoffs`), then `capture_adoption.rs` (1 832, ported only for its decidable
cascade parts), `capture_builds.rs` (1 505), `closure_keep.rs` (1 347) and
`rebound_captures.rs` (799): about 7 000 lines.  The drop machinery comes first, because the
closure records ride on it; the closures can then be taken module by module.

**Step 13 (2026-11-28) — `place_result`, loft#2677.**  `scope_place_result.loft` is
`src/place_result.rs`'s rewrite whole (`@FR-R-Place`, `@FR-R-MoveLast`): the top-level bind
(`placeable_bind`, `admit`, the walk with its `St`/`Pos`/`Flow` states, `destination`, `apply`),
the callee clause (`callee_writes_buffer_at_every_exit` with the chain exits and the four exit
spellings, `tail_fresh_object_workref`), and the loop clause (`admitted_loops`, `admit_loop`,
`lazy_mint_path`, `outside_the_turn`, the turn's `LoopCx`, `apply_loop`).  Sites are recorded by
their path through the body, spans transparent, as Rust's are; the loop plans are applied first
and the top-level admission reads their result.  `check` runs it after `insert_evacuations` and
before `exit_vector`; the refusal in `record_triggers` and the `placeable_binds` superset are
gone.  One case is refused by name: a callee whose exits the admission reads and which this pass
refused (Rust reads its settled body; the image holds its parser body) — it fires on no corpus
program.  The preview (`admits`, asked by the copy notice) and `callee_keeps_an_offered_record`
(the parser's stored-reference placement) are asked outside the pass and are not transcribed.

The Rust fix for loft#2677 (`17ccb4ca9`) is beneath, with the parser fixes for loft#2678 and
loft#2679 (`611224532`), and the port follows its scope-pass half: `scan`'s `Drop` arm peels the
statement's `Span` off a lifted argument's `Insert` before looking for the discarded call at its
end (`LOFT_NO_DROP_SPAN_PEEL`).  The loft#2677 cell agrees with the switch on and off.  The
binary is re-frozen from that tree.

| measured | programs agree | definitions agree | differ | whole path |
|---|---|---|---|---|
| step 12 | 1 834 / 2 282 | 36 238 / 40 088 | 0 | 747 |
| + `place_result` | 1 872 | 36 359 | 0 | 767 |
| + loft#2677 and the port's half | 1 872 / 2 282 | 36 359 / 40 088 | 0 | 768 |
| + the loop-record workaround in `ir_ctor_literal` | 1 872 / 2 282 | 36 359 / 40 088 | 0 | 769 |

No definition that agreed was lost, nor a whole-path program.  The whole path's leak on
`B-Scope/2591-…` (the loft-written tool's own `NdIf`s, loft#2677's subject) is gone with the
fix.  `157-a-constructor-call-builds-its-literal-in-place` reached the runner with
`place_result` and leaked one store of the tool's own (`ir_ctor_literal::Pairs`): a loft defect, both backends — a record local
made fresh in a loop body is a loop record (`@FR-R-LoopRecord`), and a `return` at the head of
the body, before its mint, does not release the record the previous turn kept.  Worked around in
`ir_ctor_literal::rewrite_gated` (the empty set ends the rounds in the loop's condition), and the
program agrees.

Sole refusal families now (programs): the view walk 61, the owner witness 38, drop cascades 37,
`arm_lift` 33, closures 16.  By first refusal: closure records 143, closure cascades 121, drop
hooks 86, the stale callee 71, the view walk 69, the owner witness 50.

The view walk is the next lever but not one transcription.  Of the 76 programs it refuses, Rust's
walk answers a view in 65 (`LOFT_DEBUG_F8`), so making the walk exact frees at most 10 of the 61
it alone stops; the rest need the materialise legs (`scan_set`'s dep strip, respell and copy, the
arm lift's copy, the split's order copy in `branches`, `scan`'s use, the phase's materialise
buffers and `materialised_views`).  An exact walk is `view_walk.rs` (2 088 lines),
`disturbance.rs` (1 109) and `places.rs` (381), and reads five facts the image does not carry:
`is_iteration_source`, `text_payload_views`, `group_write_views`, `is_amp_container_link` and
`keyed_views`.

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
