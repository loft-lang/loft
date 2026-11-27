# @PLN91 — the scope pass and what follows it, transcribed

A child of [README.md](README.md) and [main-switch.md](main-switch.md) phase 1 step 2.  The
goal: every program gate E agrees on RUNS through the loft path — elaborator, scope pass, slot
assignment, IR passes, bytecode writer, runner — and prints what `loft --interpret` prints
(the `front` layer of `selfhost/bench/run_image_survey.py`).

## Where it stands

| layer | measured | command |
|---|---|---|
| front (the whole loft path) | 52 of 1188 programs agree; 842 stop at the scope pass, 286 at the elaborator, 8 other | `run_image_survey.py --front` |
| scope gate (Rust's parsed image in, per definition) | the slice: 142 of 2277 programs, 19 878 of 37 762 definitions agree; the transcription (`LOFT_SCOPE_PORT=1`, step 2): 238 programs, 24 435 of 39 960 definitions (libraries' included), 140 differ | `scope_survey.py` |

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

## Passes

Step 4's second half: the IR passes `compile::close_program` runs after the scope pass,
transcribed one module per Rust module (`selfhost/src/ir_<module>.loft`, the same function names)
and called from `ir_passes::close_program` in Rust's order.  Shared tree helpers are
`ir_pass_tree.loft` (a node rebuilt over new children, `add_unique`, and `relayout`, which lays the
frame out again with `scope::intervals` / `scope::assign_slots` as Rust's
`compute_function_intervals` / `assign_function_slots` do).  A pass that cannot carry a definition
refuses it by name; none needs to yet.

The gate is `selfhost/bench/passes_survey.py` (Rust's image right after the scope pass, through the
loft passes, against Rust's image after `close_program`, per definition).  Each commit was checked
against the previous verdict file: no agreeing definition lost.

| measured | programs agree | definitions agree | differ |
|---|---|---|---|
| no pass (baseline) | 1 067 / 2 277 | 34 122 / 37 796 | 3 674 |
| the transcribed passes below | 1 843 / 2 277 | 36 413 / 37 796 | 1 383 |

**Transcribed:** `arg_copy` (judged by `opt_writes::blocks_plain`, the C writer's port of
`writes_store`), `leaf_inline` (ranges and non-sentinel facts from `opt_range` / `opt_non_sentinel`;
the IR phase asks the latter with the program-wide parameter fact off, `new_ir_proof`),
`zero_count`, `exp_pow`, `forward_walk`, `self_replace`, `in_range` (with its field clause, which
`forward_walk` asks), `single_use` (with `same_read`'s `pure`/`discharge`/`plain_read`),
`discharge_into`, `keyed_entry` (with `keyed_place::site`), `text_len`, and `complete_write`'s
fresh-field flags.

**Not transcribed**, with what the remaining differences rank them by (first-difference shapes of
the last run, ≈800 of the 1 383 classified):

| pass | differ (≈) | what it needs first |
|---|---|---|
| `value_record` | 450 | the program plan (`opt_vr*.loft` port it for the C writer: check it matches `value_records_twinned`), then `value_record.rs` itself (3 300 lines) |
| `loop_buffer` | 210 | `hoist::loop_buffers`, and `buffer_owner::owners`' two unported owners: `hoist::invariant_literals` and `hoist::element_first` |
| `elem_first` | 30 | `hoist::element_first` (≈1 000 lines with its helpers, `namings_avoid_place` among them) |
| `tail_self` | 22 | `tail_frame.rs` |
| `loop_record`, `lift_reuse` | 20 | `buffer_owner::owners` (as `loop_buffer`); tuple lifts (as `value_record`) |
| `same_read` | 10 | the pass itself (its guard, get-or-insert and match clauses) |
| `fill_loop` | — | `Data::vector_element_type` and `Stores::vector` over the image's types |

Also open: `complete_write`'s stored fact (`Definition::no_prefill`, decided by
`loop_records::group_covers_type`) is not computed — the gate does not print it, so the writers
read Rust's; `ledger_sites` (inert without `LOFT_LEDGER`) and `sync_capture_ownership` are not
called.  The survey takes ≈80 s at `--jobs 5`; `LOFT_PASSES_BIN` reuses a built `passed`.
