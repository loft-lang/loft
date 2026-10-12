<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-09` — release state (prep done 2026-09-04)

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

The gate is the same: stability, not features. What ships is the store-lifetime and
reference-correctness body of work of the cycle — the two checkouts' streams joined on
`pr/join-2026-09-02` — and the release is gated on that being clean.

**Tracker state.** 22 open issues, **every one `fixed-pending-merge`** — fixed on the branch,
closing on merge via their `Fixes #N` trailers. The four the release pass itself filed closed
the same day: #1356 (a struct or vector yielded from a generator's LOOP body compiles on
`--native` as a per-yield snapshot, and the eager factory runs the generator's tail — it had
dropped every statement after the last yield, leaking each persistent heap local and losing a
`print`), #1357 (every text buffer a frame mints is released — the sibling checkout's fix,
cherry-picked as `3ece3109`), #1358 (a capturing closure in a collection is refused by design,
@C116, with the struct-field route named) and #1359 (the warm cache carries its import tables,
IR cache format 4). `#1342`'s library half — 34 undocumented types across six `loft-libs-*`
repos — is done on their `doc-types-2026-09` branches and lands with their next releases.

**Gate evidence on the tag candidate `e77ef442`** (deliberate runs, per § The nightlies):

| gate | result |
|---|---|
| `make ci` | ALL GATES PASSED, 4698/4698, 30 skipped, 1 flaky (`engine_host_connector::browser_kernel_one_script_differential`, green on its retry — the browser-differential class) |
| `M-leaks` | `LOFT_STORES=warn` over 1123 scripts: 0 leaking; `LOFT_LOG=stores` on the two threading scripts: nothing unfreed (`target/m-leaks-candidate.log`) |
| `M-wasm` | `make wasm-html-test` 32 passed; `make gallery` built and every asset probed (`target/m-wasm-candidate.log`) |
| `M-libs` | `revalidate_libs_local.sh`: 42 pass, 0 runtime/env, 0 compile-break (`target/m-libs-candidate.log`) |
| `M-valgrind` | **GREEN** — 1195 runs (1159 interpreter, 36 native), 0 invalid accesses, 0 definitely lost, 0 timed out (`target/vg-2026-09-candidate/`) |

The commits after the measured tree (`e77ef442` → `68948bbf`) touch this record, the checklist,
one test's failure message, the four wasm skip comments and the `Makefile`'s scratch prune — no
`src/` — and `make ci` is green on `68948bbf` (4698/4698).

**The valgrind red of the morning was #1357, and it is fixed rather than waved through.**
Eleven files lost one Rust `String` per call on the interpreter — nine the class #568 had
closed in July without reaching these shapes, two the text ledger could not see. The sibling
checkout closed eight distinct shapes where each asks its own question (`scopes.rs`,
`control.rs`, `parser/mod.rs`, `collections.rs`, `use_analysis.rs`), made the ledger
process-wide (a `par` worker's buffers count) and reporting under `--tests`, and found the
last red file was the TEST RUNNER — it returned after the first yield frame instead of
resuming `main`, so only the sweep's `--tests` path showed it. Guard
`tests/scripts/1357-every-text-buffer-a-frame-mints-is-released.loft` +
`tests/text_buffer_ledger.rs` (99 orphans on the pre-fix binary → 0). The sweep is one
command (`scripts/valgrind-sweep.sh`) and runs nightly in `miri.yml`; CI's LSan gate still
suppresses the class on a premise `scripts/valgrind.supp` records as measured false, so the
sweep is what sees it. `M-valgrind` carries a tick for the first time in any release.

**The reference is reviewed at its current source, 40 chapters of 40**, and the pass found
five caveats promising limitations the language no longer has — each cited an issue that had
since closed, each kept its sentence when its tag was stripped. Settled by running every
claim on both backends (REFERENCE_REVIEW.md, above the watermark table). Documentation
review steps 1–4 and 8 done; `doc_history_report` was run and the docs at its head stay,
for these reasons: `PAR_PRESENTATION` (24%) is a presentation, not a contract doc;
`USER_FACING` (19%) is a census whose dated table IS its state; `formal/closures.md` (17%)
already has its companion and what remains flagged are live pointers and the `OPEN: 0`
header; `formal/matching.md` (13%) carries the @PLN35 "SHIPPED" provenance block, a
companion move deferred to a doc pass rather than done under a release; `WINDOWS.md` (11%)
likewise.

**`M-ignores` evidence for the owner's sign-off.** 33 `#[ignore]`s, every one in
`tests/ignored_tests.baseline` with a reason that names the run it rides — benchmarks and
measurements by hand (`--ignored --nocapture`), the two release-gate sweeps nightly in
`miri.yml`'s `release-gate-sweeps`, the differential oracle nightly in `ci.yml`, two Windows
`cfg_attr` ignores whose guard is the resource cap — held by
`doc_hygiene::{ignored_tests_baseline_is_current, every_ignore_reason_says_how_it_runs}`. The
two that had guarded defects (#1358's canary, #1359's round trip) are live tests now. Skip
lists: every suite list is EMPTY; `html_wasm`'s four platform limits stay, each with its end
condition (`server` on wasm — no listener by construction; `hex_world` on node — no
filesystem; `imaging` and `input` on wasmtime — no canvas codec / the graphics crate absent
from the sysroot). Four inert entries went this cycle: `447` (#1356 fixed), `web/http.loft`
twice (the file lives in `tests-network/`, which no suite walks), `191-source-dir` (#268
closed in June, and the sweep never reached its directory), `19-threading` (runs green under
wasmtime). TESTING.md § Every skip says why, how it runs instead, and when it ends is the
rule's home.

**Still owner-only and manual** (§ No Automated Releases): the merge via PR — after which
`release-gate.yml` is on the default branch and `make release-gate` can run — the `M-ignores`
sign-off above, the tag push, the draft build, validation and publish.
