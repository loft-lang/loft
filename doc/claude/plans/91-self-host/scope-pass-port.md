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
