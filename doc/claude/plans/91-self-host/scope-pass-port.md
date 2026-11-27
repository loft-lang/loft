# @PLN91 — the scope pass and what follows it, transcribed

A child of [README.md](README.md) and [main-switch.md](main-switch.md) phase 1 step 2.  The
goal: every program gate E agrees on RUNS through the loft path — elaborator, scope pass, slot
assignment, IR passes, bytecode writer, runner — and prints what `loft --interpret` prints
(the `front` layer of `selfhost/bench/run_image_survey.py`).

## Where it stands

| layer | measured | command |
|---|---|---|
| front (the whole loft path) | 52 of 1188 programs agree; 842 stop at the scope pass, 286 at the elaborator, 8 other | `run_image_survey.py --front` |
| scope gate (Rust's parsed image in, per definition) | 142 of 2277 programs, 19 878 of 37 762 definitions agree; 16 671 unscoped, 1 213 differ | `scope_survey.py` |

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
