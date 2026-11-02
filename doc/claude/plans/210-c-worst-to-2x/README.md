<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN210 — the worst C ship routines to 2× their Rust twin

**Status — ACTIVE.**  The worst routines are hand-priced; the mechanisms below are built in
yield order.  Tracker: [loft-lang/plans#210](https://github.com/loft-lang/plans/issues/210).

## Goal

Every library-bench routine under 2× its Rust twin on the C ship lane, under gcc AND clang,
reached by GENERAL optimisations with the memory model first
([`bench/portal/analysis/README.md`](../../../../bench/portal/analysis/README.md)).  A hand
form shows what the emitted C must become; the build makes the C writer emit it.

## The hand forms

Kept only for the routines over 3× (the list below): few enough that keeping them is cheap,
and each is a target the build works toward.  A routine under 3× gets its pricing in
`c-libraries.md`, not a form here.

`hand/<routine>/` keeps, per routine, what a person wrote while pricing it: each edited C unit
as a diff against the emitted base (`*.c.diff`), the step scripts, the driver cut (the
library's `bench.loft` / `bench.rs` cut down to the one routine), the twin variants used, and
`RESULT.md` (baseline, profile, the step table, the mechanism per step).  `FORMS.txt` names the
loft commit and the library commit the base was emitted from.

- **Export** a pricing directory: `python3 bench/portal/hand_forms.py export WORK_DIR
  doc/claude/plans/210-c-worst-to-2x/hand/<routine> <library package> --commit <sha>`.
- **Restore** the forms: `hand_forms.py restore hand/<routine> BASE_DIR OUT_DIR`, where
  BASE_DIR holds the base units emitted again under the names `FORMS.txt` lists; a diff that
  no longer applies is a form to RE-BASE (the emitted C moved under it).
- **The forms stay until their mechanism is built.**  Building a phase means the writer emits
  the form (diff the emitted unit against the restored step) and the ratio moves as priced.
  Re-base a form only when the emitted C changed significantly underneath it; most changes
  are this plan's own phases, which move the C TOWARD the forms.

Baseline (gcc ship, `bench/stats.py --lanes rust,cship`, tip `3cfb11fb9`): median 1.93× over
42 routines; over 3×: `insert_text` 5.11, `request` 4.58, `check_request` 4.58,
`flow_layout_full` 4.32, `encode_bytes` 4.04, `fill_polygon` 3.63, `canvas` 3.57, `fronds` 3.29,
`fill_rect` 3.20, `req_state_b64` 3.19, `draw_quads` 3.10, `invert` 3.05.

## Where each routine lands by hand

| routine | as emitted | by hand | steps | form |
|---|--:|--:|---|---|
| zttext `insert_text` | 4.55× | 0.91× | flat record copy (−76 %), vectors built in the return buffer (−18 %) | [`hand/insert_text`](hand/insert_text/RESULT.md) |
| pluginabi `check_request` | 4.45× | 0.29× | `(R-DecodeView)` structure clause alone; the stepwise path reaches 1.70× | [`hand/check_request`](hand/check_request/RESULT.md) |
| pluginabi `request` | 5.2× | 2.1× | byte classifier as a table, push length at exit, `(R-LitDiv)`, `(R-Destination)` two levels → 3.2×; the library's single decode → 2.1× | [`hand/request`](hand/request/RESULT.md) |
| cbor `encode_bytes` | 4.37× | 1.58× | `loft_ref` layout (gcc), window open across the item loop, no dead call buffer | [`hand/encode_bytes`](hand/encode_bytes/RESULT.md) |
| cbor `encode` / `decode` | 2.79× / 2.86× | 1.97× / 1.35× | the `loft_ref` layout alone (gcc) | same |
| graphics `fill_rect` | 3.0× / 4.2× | 1.51× / 1.51× | `(R-Fill)` guarded clause | [`hand/graphics`](hand/graphics/RESULT.md) |
| graphics `fill_polygon` | 3.1× / 3.9× | 1.69× / 1.28× | `(R-Fill)` in `hline`, element address from the held header with the index proven | same |
| graphics `canvas` | 3.1× | 2.1× | a comprehension with an invariant body as reserve + fill | same |
| drawing `fronds` | 3.00× | 1.13× | `(R-ElemFirst)`, `(R-MoveAppend)` | [`hand/fronds`](hand/fronds/RESULT.md) |
| drawing `fill_circle` | 3.57× / 4.21× | 1.48× / 1.83× | `(R-Fill)` | same |
| text2d `draw_quads` | 2.93× / 3.48× | 1.18× / 1.19× | loop versioning of affine bounds tests | [`hand/draw_quads`](hand/draw_quads/RESULT.md) |
| zttext `flow_layout_full` | 5.7× | 1.94× | `slice_runs`' result built outside the store (−54 % instructions), the resolver's `Style` kept across frames, a character range to text in one pass | [`hand/flow_layout_full`](hand/flow_layout_full/RESULT.md) |
| zttext `invert` | 2.57× | 1.21× | flat record copy (−43 % cycles); the replaced document reused by the callee | [`hand/invert`](hand/invert/RESULT.md) |

Ratios are gcc, or gcc / clang where both were priced; `fronds` and `fill_circle` are
instruction ratios (measured on an E-core), the rest cycles or time.

## Phases, in yield order

Each phase is one general mechanism.  It is validated by (a) the emitted C of every routine
it names matching that routine's hand step, (b) the same hashes, (c) the routine's ratio on
both compilers, and (d) the C corpus sweep (`scripts/c_sweep.py`) and the C test binaries.

| # | mechanism | kind | routines it moves (hand price) | state |
|---|---|---|---|---|
| P1 | **`loft_ref` passed without a store-forwarding stall under gcc**: gcc spills the 12-byte ref as an 8-byte and a 4-byte store and reloads `rec\|pos` as one 8-byte load spanning both | NEW, representation | `encode_bytes` 4.37 → 2.37, `encode` → 1.97, `decode` 2.86 → 1.35, `check_request` −19 to −25 % cycles; likely every gcc row passing refs through calls | **BUILT** (`d9d106cb2`): the 16-byte pad, chosen over the reorder by the A/B over 122 routines (pad gcc 0.969 / clang 0.999, reorder 0.968 / 1.013).  pluginabi `request` +19 % on gcc is branch placement in crypto `b64_value` (the same instructions, 2× the mispredictions), which P9 removes |
| P2 | **`(R-Fill)` on the C**, with its guarded clause (`t = base + i; if t < N { v[t] = c }`) | port (native: `opt::hoist`) | `fill_rect` → 1.51 both, `fill_polygon` → 1.83 / 2.29, `fill_circle` → 1.48 / 1.83; likely `clear`, `fill_triangle` | **BUILT** (`ecb39db3f`, `dd63ae3b0`): `fill_head` / `fill_tail`, `loft_vh_fill_*` |
| P3 | **`(R-DecodeView)` and its structure clause on the C** | port (native: `src/lower/decode_view.rs`, `decode_structure.rs`) | `check_request` → 0.29 | open — settles the open `(R-DecodeView)` vs `(R-Transient)` decision: 0.29× against 0.41× |
| P4 | **`(R-FlatCopy)`'s emitted clause on the C**: a record owning no heap copied by `memcpy`, no `loft_copy_record` + `loft_copy_claims` (≈ 196 instructions each on a 24-byte `Piece`) | port (native: `hoist::flat_copy`) | `insert_text` 4.55 → 1.10, `invert` 2.57 → 1.21; likely `delete_range`, `patch` | **BUILT** (`ecb39db3f`): `loft_copy_flat` |
| P4b | **`(R-Transient)`'s producer half for a returned vector**: a vector a callee builds and its caller only reads is built outside the store (a kept C array) | rule written, opt-in WIP (`LOFT_TRANSIENT=1` emits nothing different here) | `flow_layout_full` 5.1 → 2.14 (−54 % instructions) | open — the structure clause (P3) wins for a decoder, this wins for a builder the caller walks |
| P5 | **loop versioning of affine bounds tests**: one test of a row's two ends at loop entry selects a test-free copy | NEW (beside `(R-BoundedNest)`) | `draw_quads` → 1.18 / 1.19; likely `fill_rect`, `hline` | open |
| P6 | **`(R-ElemFirst)` and `(R-MoveAppend)` on the C** | port (native: `opt::hoist`) | `fronds` 3.00 → 1.13 | open |
| P7 | **a record element's address from the held header, the index PROVEN in range** (`(R-InRange)` extended to a bound held in a local and to `(i + 1) % n`) — static, no run-time test per access | extension | `fill_polygon` 1.83 → 1.69 gcc, 2.29 → 1.28 clang | open |
| P8 | **`(R-AppendTwin)`'s window clause across a loop** of window-twin calls, and no call buffer for a call the window twin replaced | extension | `encode_bytes` 2.37 → 1.58 | **BUILT** (the loop clause, `FnWriter::open_loop_windows`): cbor `encode_bytes` −27 % gcc / −7 % clang, `encode` −2 %; the dead call buffer not yet |
| P9 | **a pure if-chain classifier over a byte argument as a 256-entry table** | NEW | `request` 5.2 → 3.5 (branch misses 36 M → 1.8 M) | **BUILT** as `(R-Tabulate)` (`opt::tabulate`): the body evaluated in the compiler for every byte, eight branches the floor (the hex libraries' six-branch direction maps measured +7 % on hex_field `trace`); `request` −35 % gcc / −49 % clang, crypto `base64_to_bytes` −22 % / −33 % |
| P10 | **a comprehension with an invariant body as reserve + fill** | NEW | `canvas` 3.1 → 2.1 | LIBRARY: `[x; n]` with a run-time `n` is already one resize and a block fill on the C, so graphics `filled(count, value)` can be `[value; count]`; the comprehension clause only if another routine needs it |
| P11 | **a returned literal's vector field built in the caller's buffer** (a block swap inside one store at the return) | NEW | `insert_text` 1.10 → 0.91 | open; the destination store grows inside the callee, so a header held into it is derived again after growth |
| P12 | smaller ones: `(R-Push)`'s length kept in the header and written at loop exit; `(R-Destination)` for a vector across a two-level return chain; `(R-LitDiv)` on the C | port / extension | `request` −2 to −7 % each | open |
| P13 | **`(R-FnRefBuffer)` across frames**: the resolver's result record kept across `token_width` calls (undoing it at the end costs +27 % cycles) | extension | `flow_layout_full` −7 % instructions | open |
| P14 | **`(R-PushFill)`'s run clause for a text target**: a character-vector range turned into text in one pass | extension | `flow_layout_full` 2.14 → 1.94 | open |
| P15 | **the callee half beside `(R-RebindBuffer)`**: `cur = apply_op(cur, op)` — the callee answers inside the document being replaced, so its buffer is not copied | NEW | `invert` 2.57 → 2.33 | open |

**The cross-call facts these phases rest on** ([`formal/facts.md`](../../formal/facts.md),
[#2351](https://github.com/loft-lang/loft/issues/2351)) — built before P5, P9 and P11–P15, so no
phase writes its own callee walk:

| # | what | closes | validated by |
|---|---|---|---|
| F1 | the routine `facts(kind, reach)` with the `writes`, `mints`, `observable`, `fault` kinds; the eleven analyses moved onto it one at a time | D-fact-1 | **BUILT** S1–S6 (`b32585027`, `7f4177193`): byte-identical emission (2,938 + 2,976 units), census identical; S7 waits for one long-lived handle per writer (D-fact-1) |
| F2 | the `entry` kind's interval domain, read by both writers: `(R-RangedCall)` on the C | D-fact-2 | the census gains `R-RangedCall` rows on the C; `draw_quads` step 1 |
| F3 | one twin key `(function, assumptions)` and one registry; `__nn`, `__rg`, `__inv`, `__kh`, `__ap`, `__d` re-keyed | D-fact-3 | the emitted C and Rust byte-identical before and after |
| F4 | `(R-InlineLeaf)`'s accessor clause (`Fact-Relational`): a leaf reading a record parameter's fields and one vector element is inlined | — | P5's versioning then works inside one body |

Made redundant by a larger step on the same routine (priced, not needed there): `(R-TextBorrow)`
for fn-ref call arguments and a returned work text as the caller's buffer (`flow_layout_full`,
both under P4b); the match-binding clause of `(R-TextBorrow)` and `pa_get` answering a field
(`check_request`, both under P3).

**Measured to be worth nothing** (do not build for these rows): `(R-PathRelease)` on the C (gcc
already folds a release of a local null on that path); `head__ap` forced inline (−10 %
instructions, +15–20 % cycles on gcc: the ref spills and stalls again).

## Library-side (the library's own stream; not this plan's code)

- pluginabi `request` decodes every argument twice (`pa_is_base64`, then `pa_cbytes`): 3.2× → 2.1×.
- graphics `Canvas.data` holds 8-byte pixels where the twin holds 4-byte ones; the rest of
  `fill_rect`'s and `canvas`' gap is that bandwidth (a twin with 8-byte pixels matches the hand C).
- graphics `filled()` doubles because comprehensions were slow on the interpreter; after P10
  it can go back to the comprehension.

## Open questions

- P1: pad or reorder, and whether a per-compiler layout is acceptable — decided by the
  whole-table A/B on both compilers, not by these rows.
- `request` after P9 and the library's fix: a last-use MOVE of a vector or record into a field
  or element (it exists for text) is estimated at ≈ 1.8×; not priced.

## Documentation

- `bench/portal/analysis/c-libraries.md` — the table and each built row.
- `doc/claude/formal/rewrites.md` — each new rule (P5, P9, P10, P11) and each port's C clause.
- `doc/claude/C_BACKEND.md` — the C writer's rewrites and their switches; P1's ref layout.
- `doc/claude/PERF_PORTAL.md` — regenerated after the arc.

## See also

@PLN203 (the C99 backend as the only compiled target) · @PLN185 (a slow routine is an ordinary
bug) · @PLN158 (the release performance pass) ·
[`bench/portal/analysis/top5-memory.md`](../../../../bench/portal/analysis/top5-memory.md) ·
[`bench/portal/analysis/transient.md`](../../../../bench/portal/analysis/transient.md).
