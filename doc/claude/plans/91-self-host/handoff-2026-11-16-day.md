<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
<!-- size-exempt: a dated handoff record, read once (DOC_QUALITY § Maintainer docs 2) -->
# Handoff 2026-11-16 (day): gate E has no differ; the 2515 leak fixed; a vector-parameter port in flight

Branch `tuxedo-pln204-int32` @ 4676cb84e, pushed.  Goal (owner, /goal): the full loft
interpreter, the in-between data of the two implementations compared at every layer.  PR #2516
(loft3's gather) carries this branch up to a57ab211f.

## Where it stands (suite pin 508580956)

| Layer | Gate | Position |
|---|---|---|
| front end (gate E) | `selfhost/front/elab_gate.py` | **762 of 2125 agree, 1363 refused, 0 differ**; baseline committed (52c35b4a7) |
| writer, plain programs | `bgen_survey.py` | 9091 of 9093 functions, 1316 of 1317 streams, 0 differ |
| writer, test functions | `bgen_survey.py --functions` | 5758 of 5758 functions, 2619 of 2660 streams (the 41: natives of `--lib tests/lib` packages, known) |
| runner on Rust's code | `run_image_survey.py [--functions]` | 1316 of 1317 programs / 3967 of 3978 test functions |
| runner on loft's code | `run_image_survey.py --written [--functions]` | 1315 of 1317 programs / the test-function layer was still running at handoff |

The writer and runner baselines are re-written at the new pin in the WORKTREE
`../loft2-repin` (detached at 52c35b4a7, `target/release/loft` symlinked to this tree's) —
**not committed**.  The `image` layer straddled a `cargo build` of the shared binary (a
`src/parser/control.rs` change that only touches record return delivery), so re-run the four
`run_image_survey.py` layers there or here before committing them; the two `bgen_survey.py`
layers were written before that build and can be copied over as they are.  The chain's log and
script: the session scratchpad `repin.log` / `repin.sh` (gone with the session; the commands are
`--write-baseline` per layer, in the order of the table, under `ulimit -v 8000000`).

## Done this session

- **Gate E's three differs** (1bed3fb68): `rebuild_call_deps` on a declared bind (O-Latest
  2132); `object_into` carries Rust's rebuild watch — a collection field naming the local or a
  view of it abandons the in-place attempt, and a parameter rebound by such a literal runs the
  attempt first (`param_rebuild_attempt`) so its `__vdb_N` / `_elm_N` stay declared
  (F-ParamRebind); `field_vector` carries `parse_vector`'s own-buffer arm inside that attempt
  only (refused by name elsewhere) and reads every item before building any element
  (139-drop-cascade).  Found on the way and closed by the same watch: a plain local rebuilt by a
  literal reading a view of itself was built IN PLACE by the port (no refusal) where Rust builds
  apart.
- **loft#2515 follow-up** (4676cb84e, sent to loft3 for `tuxedo-pr2516-next`): PR #2516's suite
  was red on the 2515 cell under `LOFT_LEAK_CHECK=1` — one `D` per `elem_or` call with the `??`
  default taken.  Cause was mine (72a943412): the promotion rung kept the call's container out of
  the buffer, but `ls_can_be_record_buffer` still read it as a candidate, so nothing freed it.
  One helper `work_ref_of_another_record` now answers both; `LOFT_NO_RET_CONTAINER_COPY=1`
  switches the delivery half off (STORE_RECORDS.md).  Checker: CLEAN with the cure, CAUGHT under
  each switch — the earlier MISS is closed by main's records.  The plain `loft --tests` matrix
  had skipped the leak gate: **score a store-lifetime cell with `LOFT_LEAK_CHECK=1`**.

## In flight (uncommitted in the tree; also `~/workspace/.pln91-wip/vector-param-rebind.patch`)

The rebound-parameter family's first member: `v = [..]` on a visible vector parameter
(`vector<T>` or `vector<T>?`) rebinds locally — `el_ex_assign` takes the entry witness
(`rebind_witness`) before the literal is read; `vector_build` gives the rebound parameter a fresh
backing (`OpFreeRefIfDistinct(v, witness)`, then `literal_store(…, rebind: true)`: skip-free,
inline) — Rust's `vector_db_init` rebind arm.  New helpers: `el_vars::rebind_orig`,
`el_ir::sets_var`.  The inline-ref null placement now asks Rust's question
(`inline_ref_set_in`: the first statement holding a `Set` of the ref, else the fallback), which
moves where a store only `OpDatabase` fills is nulled — **this touches every program with an
inline ref, so gate E must be run in full before committing**.

Probes (Rust's values hand-checked: `32 109 201 8 q2 1 5`, callers untouched):
`selfhost` scratch files `vp1.loft` / `vp2.loft` — re-create from the shapes in
`tests/scripts/87-p2-reassign-locality.loft` (twice, in a branch, record and text elements,
`vector<integer>?`, in a loop).  Before the last edit the only differences were the `__vdb`
null's position (fixed by `sets_var`, unmeasured) and the `?` parameter (now admitted,
unmeasured).  An elaborator build with both was running at handoff.

Steps: build the elaborator, `one.sh`-compare the probes, run gate E full (`--jobs 6`, `ulimit -v
6000000`, `LOFT_SUITE_DIR` at the pin), diff verdicts against 52c35b4a7's baseline, commit with
the baseline re-written.

## Open, in order

1. The in-flight port above, then the rest of the rebound-parameter family (by right-hand side:
   a name 7, a text literal 8 — check why a text parameter reaches the heap refusal —, a struct
   literal 5, a binary operator 3).
2. Commit the writer/runner baselines at the pin (see above).
3. Gate E families in `--plan` order: a function returning a vector from this tail (59), a
   `return` with a value of this type (79), an assignment to a place that is not a plain local
   (65), an assignment to a nullable place (76), an operator with no builtin candidate (45).
4. `store_records.rs` is 6545 lines, over the 5000 ceiling — any checker addition waits on its
   split (`split-file` skill, a pure move).
5. The 2512/2514 closure cells fail on this tree (their fix bc393168b is on
   `tuxedo-pr2513-next`); they pass once main has it — not this branch's to fix.

## Method notes

- One program against kept tools in seconds: build `elab` once (`loft --native-c --ship --out
  <dir> selfhost/front/elab.loft`, ~6 min), copy `defs/`, `std.store(.dschema)` from a
  `LOFT_ELAB_KEEP=1` run, then per program: `LOFT_EMIT_PARSED=<img> loft <f>`; `defs <img>
  program <std>` (Rust's report) against `elab <img> <f> <std>` (loft's), diffed.
- `loft --interpret selfhost/front/elab.loft …` printed nothing: native builds are the probe tool.
- Rust's `LOFT_VAR_TABLE=<fn>` lists a function's variables in creation order — it is what
  showed the abandoned attempt's `_vec_1` / `__vdb_1` / `_elm_3`.
- Never rebuild `target/release/loft` while a survey runs from a worktree that symlinks it.
