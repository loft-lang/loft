<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN54 — Sanitizer coverage expansion

## Status — CLOSED 2026-07-10 (sanitizer stack shipped; S9 toolchain-blocked, spun out)

Opened 2026-05-31 as the successor to
[@PLAN53](../finished/53-sanitizer-ci-lever/README.md) (base sanitizer CI stack,
Wave 1 + Wave 2 kickoff), owning the Wave-2 sanitizer-coverage items that are not
fuzzing.  **Eight of nine slices are green on `main`; the ninth is a toolchain
block, deferred with a one-line reason (the plan's own exit criterion accepts
that) — so it closes.**

- **✅ DONE on `main`:** S1 (macOS-ARM sanitizer leg) · S2 (TSan, 0 races) ·
  S3 (`LOFT_POISON` both halves + gate) · S5 (Miri store-unit set + whole-corpus
  debug-asserts gate) · S6 (native-backend ASan) · S7 (nightly failure notifier).
- **✅ S4 (LSan → `detect_leaks=1`) — DONE, unblocked by @PLN85.**  S4's blocker
  was the *"REAL, GROWING production leak"* (a native-`text` result in a user fn's
  implicit tail-return, never freed).  **@PLN85 closed that class** (the
  owned-text-return family) and the nightly `asan-leak-gate` in `miri.yml` now runs
  **`detect_leaks=1` enforcing at baseline 0, green** over the whole corpus +
  libraries on both backends — S4's exit criterion ("`detect_leaks=1` passes
  corpus-wide in CI") is met.  (The per-slice notes below predate the fix and still
  read "gate stays `detect_leaks=0`" — stale.)
- **⏸ S8 (MSan) — deferred** (needs a fully-instrumented dep closure; low marginal
  value over Miri + the debug-asserts gate).  Meets the exit criterion (deferred
  with a one-line reason).
- **⏭ S9 (mixed-boundary @C71 cdylib ASan) — SPUN OUT (toolchain-blocked).**  The
  ASan-injection mechanism landed (`build_shared_cdylib` + the `LOFT_NATIVE_ASAN`
  gate); end-to-end is blocked on the **curve25519-proc-macro `E0463`** in the
  cross-target ASan cdylib build (the host proc-macro is not on the target `-L`) —
  a *toolchain* limitation, not loft code.  A probe-grounded candidate fix (design
  in [NATIVE_ASAN_DESIGN.md](NATIVE_ASAN_DESIGN.md) § S9) is unvalidated.  It was
  routed in from [@PLN11](../11-data-as-store/README.md) N5 (the mixed-boundary
  soundness A-leg); re-open it there or as its own focused plan when the `E0463`
  block is cleared.  The @C71 cross-boundary surface stays sanitizer-uncovered until
  then (documented gap, not silent).

The finishing PR closes the tracker issue (`Closes @PLN54`).

**Ledger vs the CI reality (grepped 2026-07-09), highest-value first:**

- **S2 (TSan) — ✅ DONE (2026-07-09).** The `tsan` job in `miri.yml` (nightly)
  is loft's first data-race coverage. The `threading` + `threading_chars` suites
  are **TSan-clean (0 races)** over real rayon-pool `par` workers, confirming the
  store-isolation model (THREADING.md) holds under real concurrency; a throwaway
  positive control (two threads racing one byte) confirmed TSan fires, so the
  clean run is non-vacuous. Needs `-Zbuild-std` + target-scoped sanitizer flag
  (§ Concrete steps S2).
- **S3 (`LOFT_POISON`) — ✅ FULLY CLOSED (2026-07-09).** Both halves + the CI
  gate. **Store half:** poison-on-free (`keys.rs::poison_enabled` +
  `allocation.rs::free_named`, both backends); the 23-bug campaign drove
  `LOFT_POISON=1` green, re-verified today (1498/1498). **CI gate:** the nightly
  `poison` job in `miri.yml` (that green is no longer an undefended one-off).
  **Stack half:** built the sound way ([STACK_POISON_DESIGN.md](STACK_POISON_DESIGN.md))
  — the literal "poison freed slots" is unsound (the pop primitive returns a
  reference into the vacated region; the return value transiently lives there),
  so poison at **reserve** instead: `State::reserve_frame` fills its
  freshly-reserved (provably-dead, above-TOS) region with the sentinel, so any
  read of an unwritten frame slot is loud. One chokepoint (`reserve_frame`),
  sound by construction, interpreter-only (native uses Rust's own stack).
  Green-drive 1498/1498 (no false positive) + positive control
  (`reserve_poison_fires_on_uninit_slot_read`) fires. Residual (documented, not
  silent): within-scope zone-1 slot reuse needs an interval-end hook with no
  runtime event; the non-`reserve_frame` reserve paths (par/coroutine/reenter);
  both complementary to `LOFT_UAF_GEN` (stale-DbRef gen-stamping).
- **S6 (native ASan) — ✅ DONE (2026-07-09).** `LOFT_NATIVE_ASAN=1` compiles the
  `--native` generated binary with `-Zsanitizer=address`; the in-process
  interpreter `asan` job was blind to that separate process. Validated 14/14
  curated corpus + positive control; nightly `native-asan` job in `miri.yml`.
  ASan tolerates the uninstrumented libloft (no `-Zbuild-std`). See
  [NATIVE_ASAN_DESIGN.md](NATIVE_ASAN_DESIGN.md).
- **S9 (cdylib mixed-boundary ASan) — mechanism landed; NO validated design yet.** The
  @C71 path (an interpreted script sharing its `*mut Stores` with a compiled cdylib
  by raw pointer) is the one cross-boundary surface no in-process sanitizer sees.
  Cdylib injection shares S6's `LOFT_NATIVE_ASAN` gate; blocked on the
  curve25519-proc-macro `E0463` in the cross-target ASan cdylib build. The obvious
  `-L host deps` fix was **probed + falsified** (double-std / ABI cascade); a
  probe-grounded candidate exists but is unvalidated — the genuinely hard, unfinished
  part of S6+S9. See [NATIVE_ASAN_DESIGN.md](NATIVE_ASAN_DESIGN.md) § S9.
- **S4 (LSan triage) — ✅ ROOT-CAUSED (2026-07-09, Mac session); found a REAL
  growing production leak — gate stays off until it's FIXED.** LSan runs on
  macOS-ARM (verified with a `Box::leak` positive control), and unlike Linux,
  release frames symbolize — so the class the Linux session was blocked on is named.
  **Two classes:** (1) the **intentional, documented `ir_read` `Box::leak`** of IR
  names (`&'static str`, interner-style, bounded, freed at exit — ir_read.rs:25-27);
  16 `ir_read`/`ir_schema`/`ir_store` round-trip lib tests exercise it by design.
  (2) a **REAL, GROWING PRODUCTION leak** — an earlier note wrongly called this a
  harness artifact; it rested on unrepresentative probes (`+=`, a str-view). A
  boundary matrix (both cells output-verified) isolates it: a native fn's `text`
  result used as a user function's **implicit tail-return** is never registered as
  owned → ~2 allocs leak **per call, growing** (`fn f()->text{…u.to_json()}` →
  N=10:20, N=100:200 leaked; rebinding to a local before `return` → 0; `to_upper()`
  bound directly is also clean). Likely a caller-bind / `OpReturn` owned-placeholder
  gap in `state/codegen.rs`. `free_text` itself works. **Gate stays `detect_leaks=0`
  until (2) is FIXED** (stability policy: fix + regression test, not suppress); then
  Class 1 gets a narrow exclusion/`leak:ir_read`. The fix is codegen/ownership work
  (loft-codegen discipline + `--native` cross-check). A `fn main(){test();}` harness
  wrapper was tried and REVERTED (did not reduce the leak → confirms the codegen
  cause). See [REMAINING_DESIGN.md](REMAINING_DESIGN.md) § S4.
- **S1 (macOS-ARM sanitizer leg) — ✅ DONE (2026-07-09), validated on a real Mac.**
  The `miri` and `asan` jobs in `miri.yml` now run a `[ubuntu-latest, macos-latest]`
  OS matrix (`fail-fast: false`), closing the ARM residual the founding @P383 (an
  ARM-only incident) exposed. Validated on `aarch64-apple-darwin` before landing:
  Miri store-unit set **6/6 clean**, ASan corpus sweep **1494 pass / 0 fail**, and
  the raw-pointer-OOB positive control **fires on arm64** (the ARM Apple ASan
  runtime is active — non-vacuous). One finding, catalogued + handled: ASan's
  per-frame redzones (~6× on ARM) defeat the 8 MB-stack calibration of the
  `deep_nesting_in_sandboxed_def_is_a_clean_error_not_a_crash` guard test —
  128-deep recursion overflows the thread stack before the depth guard bails. Pure
  ASan-instrumentation artifact (production frames ~10 KB, guard fires at ~1.3 MB),
  a functional guard test not UAF/OOB coverage, still run unsanitized on the
  macOS-ARM full suite (`v2-validation.yml`) + the ubuntu ASan leg — so the macOS
  ASan leg skips just that one test with no memory-safety-coverage loss.
- **S7 (nightly notifier) — ✅ DONE (2026-07-09).** The `notify` job in `miri.yml`
  reads per-JOB conclusions and opens/updates/closes ONE deduped GitHub issue on any
  red gate (logic validated locally; first-run issue-file confirms on merge).
- **S5 (grow Miri set) — ✅ DONE (2026-07-09), REFRAMED.** Miri-on-loft costs
  ~5-10 min/test (interpreter-under-interpreter; the p213 sibling timed out at
  240s), so growing the slow full-program set is infeasible. **S5a:** re-targeted
  Miri at the unsafe STORE layer — 6 direct store-unit tests (claim/free-coalesce/
  resize/adopt/rebase/borrow), all Miri-clean in **~8s** (vs 240s+), curated set
  4→10. **S5b:** the higher-ROI complement — a whole-corpus **`debug-asserts`**
  gate (loft's ~200 internal `debug_assert!`s, off in every normal build, checked
  only by cargo-fuzz before) — green over `--lib`+`--test issues`. More UB +
  invariant coverage for less CI time than the original plan.
- **S8 (MSan) — defer** (needs a fully-instrumented dep closure; low marginal value
  over Miri + the DA gate).

**Recommended entry point (updated 2026-07-09):** **S1, S2, S3, S5, S6, S7 are DONE**
(macOS-ARM sanitizer leg · TSan · both poison halves + gate · reframed Miri + new
debug-asserts gate · native ASan · nightly notifier); **S4 is ROOT-CAUSED** and
surfaced a REAL growing production leak (native-fn `text` result in implicit
tail-return position never freed — a codegen/ownership gap). **S8** deferred.
Remaining implementation: **fix the S4 Class-2 leak** (codegen/ownership + regression
test; then flip `detect_leaks=1` with a narrow Class-1 `ir_read` exclusion) and
**S9** (mixed-boundary cdylib ASan — a focused session; candidate design in
NATIVE_ASAN_DESIGN.md, medium-high risk).

## Goal

Expand the sanitizer CI coverage that @PLAN53 established, closing the
platform, tool-class, and corpus blind spots that survive the Wave-1+kickoff
stack.  Exit criterion: each item below is either green on `main` or explicitly
deferred with a one-line reason.

**Design for the remaining arcs (S9, S4, S5, S1, S7, S8):**
[REMAINING_DESIGN.md](REMAINING_DESIGN.md) — per-arc invariant + concrete steps +
positive control + CI shape + effort, grounded in this plan's toolchain findings.
S9's deep analysis is in [NATIVE_ASAN_DESIGN.md](NATIVE_ASAN_DESIGN.md).

## Sub-arcs

| Item | Description | Exit criterion | Priority |
|---|---|---|---|
| **S1** | **macOS-ARM sanitizer leg** — the founding @P383 incident surfaced exclusively on macOS-ARM, so a ubuntu-only nightly would not have caught it.  **✅ DONE 2026-07-09, validated on a real Mac:** the `miri` + `asan` jobs run a `[ubuntu-latest, macos-latest]` OS matrix.  On `aarch64-apple-darwin`: Miri store-unit set 6/6 clean, ASan sweep 1494 pass / 0 fail, raw-pointer-OOB positive control fires on arm64.  The macOS ASan leg skips one deep-nesting guard test whose 8 MB-stack calibration ASan's frame redzones defeat (an instrumentation artifact, not a loft bug — see README § S1 / the asan job comment). | macOS-ARM *sanitizer* leg green on `main`, validated on a real Mac; nightly badge reflects it. | ✅ Done |
| **S2** | **ThreadSanitizer (TSan)** — add a `tsan` job to `miri.yml` running the parallel/threading suite under `RUSTFLAGS=-Zsanitizer=thread`.  loft executes real parallel workloads via `par`/`par_light` under a store-isolation model (THREADING.md); zero data-race coverage exists today (Miri runs stacked-borrows-off; ASan/guard are not race detectors).  **✅ DONE 2026-07-09: `tsan` job built + validated — `threading`+`threading_chars` TSan-CLEAN (0 races), positive control confirms it fires; needs -Zbuild-std + target-scoped flag (§ Concrete steps S2).** | TSan job green on `main`; any races found catalogued or fixed. | ✅ Done |
| **S3** | **`LOFT_POISON=1` arena poison-on-free keystone — ✅ STORE-RECORD HALF BUILT** (2026-06-29, @PLN85 fuzz-proof: `keys.rs::poison_enabled` + the `allocation.rs::free_named` poison block; both backends — native calls the same `free_named`; positive control proven — exposed a SILENT use-after-free (`elem_accumulate-none`) the cross-backend differential alone missed. **✅ FULLY CLOSED (2026-07-09):** store half + nightly `poison` CI gate in `miri.yml` + stack half. Stack half = poison at **reserve** not free ([STACK_POISON_DESIGN.md](STACK_POISON_DESIGN.md)): `reserve_frame` sentinel-fills its provably-dead reserved region (interpreter-only); green-drive 1498/1498 + positive control fires. See § Concrete steps S3.3.) Fill freed store records + freed stack slots with a sentinel value on free, turning silent store-internal use-after-free (the @P377/@P378 dangling-`DbRef` family) into loud, deterministic garbage at the dangling read — on any rustc, no nightly.  This is the blind spot Miri/ASan/Valgrind all share (loft's arena "free" is not a libc `free()`). | `LOFT_POISON=1 cargo test` green; @P377/@P378-class reads produce sentinel-value panics rather than silent stale data.  **Also unblocks @PLN53 F4.** | ✅ Done |
| **S4** | **Triage the LeakSanitizer baseline** — understand each allocation class, fix the avoidable leaks, turn `detect_leaks=1` on.  **✅ ROOT-CAUSED 2026-07-09 (Mac session):** (1) intentional documented `ir_read` `Box::leak` of `&'static str` IR names (interner-style, bounded); (2) a **REAL, GROWING production leak** — a native fn's `text` result in a user function's **implicit tail-return** is never registered as owned → ~2 allocs/call, growing (rebinding to a local fixes it); likely a caller-bind/`OpReturn` owned-placeholder gap in `state/codegen.rs`.  **Gate stays off until (2) is FIXED** (fix + regression test, not suppress), then Class 1 gets a narrow exclusion.  See REMAINING_DESIGN.md § S4. | ASan `detect_leaks=1` passes corpus-wide in CI, or each surviving allocation class has a one-line accepted-leak annotation. | Root-caused✅ real-bug-found |
| **S5** | **Grow the Miri curated set** beyond the current 4 tests (p213 + clusters 3/4/5).  **✅ DONE 2026-07-09, REFRAMED after finding Miri-on-loft = ~5-10 min/test (interpreter-under-interpreter).  S5a: re-targeted at the unsafe STORE layer — 6 direct store-unit tests, Miri-clean in ~8s (curated set 4→10).  S5b: added a whole-corpus `debug-asserts` gate (loft's ~200 internal invariants, off in normal builds) — the fast, broad complement to the slow Miri set.** | Miri curated set ≥ 8 tests; job runtime ≤ 20 min on ubuntu. | ✅ Done (a+b) |
| **S6** | **Native-backend ASan** — instrument the `--native` codegen runtime under ASan (currently ASan instruments only the in-process interpreter; the `--native` path is uninstrumented).  **✅ DONE 2026-07-09: `LOFT_NATIVE_ASAN=1` → `-Zsanitizer=address` on the generated binary; nightly `native-asan` job; 14/14 curated corpus clean + positive control fires; ASan tolerates the uninstrumented libloft (no -Zbuild-std). See NATIVE_ASAN_DESIGN.md.** | At least one native-mode test corpus passes under ASan; any findings catalogued or fixed. | ✅ Done |
| **S7** | **Failure→issue notifier for the nightly** — a CI job that opens/updates a deduped GitHub issue when the nightly fails, reading per-*job* conclusions (not the overall run status, which `continue-on-error` holds green even when matrix legs are red). | Nightly failure automatically surfaces as a tracked GitHub issue within 24 h of the failing run.  **✅ DONE 2026-07-09: `notify` job — per-JOB conclusions, deduped open/update/close; jq logic validated locally.** | ✅ Done |
| **S8** | **MSan (MemorySanitizer) corpus-wide** — uninitialised-read detection beyond what Miri covers.  Painful setup (needs a fully instrumented std); lower priority. | MSan job passes the interpreter subset or deferred with a one-line setup-cost note. | Low |
| **S9** | **Mixed-boundary (@C71) cdylib ASan** — instrument the auto-built native-library cdylib *and* the interpreter host under ASan together, covering the [@PLN11](../11-data-as-store/README.md) @C71 mixed path: an interpreted script shares its `*mut Stores` with a compiled library cdylib by **raw pointer** (zero-marshalling) — the one cross-boundary surface no current sanitizer sees (ASan = interpreter targets only; the `stack_align_guard` sweep can't see spawned binaries; Miri can't `dlopen` a cdylib).  Propagate `-Zsanitizer=address` into `build_shared_cdylib` when the host is ASan-instrumented, + a nightly job.  **Routed in from @PLN11 N5** (mixed-boundary soundness — the D + E legs landed there, this A leg was tooling-blocked).  Shares the ASan-on-a-generated-build mechanism with **S6**.  **Mechanism landed (build_shared_cdylib ASan injection, LOFT_NATIVE_ASAN gate); end-to-end BLOCKED on the curve25519-proc-macro E0463 in the cross-target ASan cdylib build (host proc-macro not on the target -L); fix + CI-job design in NATIVE_ASAN_DESIGN.md.** | the interp-script + native-lib mixed corpus (the `tests/n3_parity.rs` shapes) passes under ASan; a cross-boundary UAF/OOB on the shared store is caught, not silent. | Mechanism✅ blocked |

## Concrete steps to finish

Ordered by value-per-effort. Each step names the exact file, command, and
acceptance check. **CI convention** (established by @PLAN53): nightly
tool-class sanitizers live in [`.github/workflows/miri.yml`](../../../../.github/workflows/miri.yml)
(non-blocking — a red nightly never blocks a merge); the one cheap per-PR
sanitizer is the `guard` job in [`.github/workflows/ci.yml`](../../../../.github/workflows/ci.yml).
The in-process interpreter suites `--lib --test issues --test wrap --test
strings --test frame_vars` are the sanitizer-relevant surface (native / wasm /
html tests spawn separate uninstrumented binaries and add no coverage — mirror
the existing `asan` / `guard` job filters).

### S3 — `LOFT_POISON` keystone (do FIRST; ~½ day) — nearly done

1. **Re-verify green on current `main`.** ✅ **Done 2026-07-09 — GREEN
   (1498/1498).** `LOFT_POISON=1 cargo test --release --no-fail-fast --lib
   --test issues --test wrap --test strings --test frame_vars` → 0 failures
   across lib + issues + wrap + strings + frame_vars, re-confirming the
   2026-07-03 green still holds after the ~15 intervening commits (incl. the
   @PLN97 store loaders). *Known non-poison noise on a dev box:*
   `codegen_emitter::p310_*` fails on a STALE cached native cdylib (`loft_ffi`
   StableCrateId collision) — it fails identically WITHOUT poison, is
   environmental (a build/toolchain limitation, not a memory bug), and CI's
   clean build does not hit it; clear locally with `make rebuild-native-cdylibs`.
2. **Add the CI gate.** ✅ **Done 2026-07-09** — the `poison` job in `miri.yml`
   (nightly, stable toolchain — poison needs no sanitizer/nightly): `cargo
   nextest run --profile ci --release --lib --test issues --test wrap --test
   strings --test frame_vars -E 'not test(library_suite)'` under `env:
   LOFT_POISON: '1'`. Validated green locally under the exact command before
   landing (1497/1497). *Follow-on (not blocking):* promote to a per-PR `ci.yml`
   job once its wall-clock on a runner is measured acceptable; and reintroduce a
   store-UAF (revert one `OpFreeRefIfDistinct` guard from fuzz-proof-gate.md) as
   a one-off to confirm the gate turns red (the positive control).
3. **Poison unwritten frame slots at RESERVE** (S3's second half) — ✅ **DONE
   2026-07-09** ([STACK_POISON_DESIGN.md](STACK_POISON_DESIGN.md) — design +
   closeout). The literal "poison freed slots" is *unsound* (the pop primitive
   returns a reference into the vacated bytes; the return value transiently lives
   in the region being vacated), so the sound hook is **reserve**, not free:
   `State::reserve_frame` (state/mod.rs:1591) fills its freshly-reserved
   (above-TOS, provably-dead) region with `0xDEADBEEF` under
   `keys::poison_enabled()`, so a read of any slot the frame has not yet written
   (uninitialized, or a cross-frame stale read) hits the sentinel — a `DbRef`
   read gets `store_nr=0xBEEF`, wildly out of range. **Invariant:** at reserve
   every not-yet-written slot holds the sentinel; definite assignment (`OpInit*`
   before read) means a correct program never observes it. **Chokepoint N = 1**
   (`reserve_frame`), interpreter-only (native uses Rust's own stack). Validated:
   green-drive **1498/1498** under `LOFT_POISON=1` (no false positive — the
   soundness + definite-assignment predictions hold), and the positive control
   `frame_vars::reserve_poison_fires_on_uninit_slot_read` fires under the flag
   (skips without). Ships inside the existing nightly `poison` job automatically.
   Residual (documented, not silent): within-scope zone-1 slot reuse (needs an
   interval-end hook — no runtime event); the non-`reserve_frame` reserve paths
   (par/coroutine/`reenter_ret`); both complementary to `LOFT_UAF_GEN` (stale
   *DbRef* gen-stamping).

### S2 — ThreadSanitizer — ✅ DONE 2026-07-09

The `tsan` job in `miri.yml` (nightly) — loft's only data-race coverage. **Result:
the `threading` (47) + `threading_chars` (49) suites are TSan-CLEAN — 0 race
reports** over real rayon-pool `par`/`par_light` workers, confirming the
isolation model (deep-copied per-worker `Stores` via `clone_for_worker`,
non-overlapping tiled writes through a shared raw pointer, joined before the
buffer is read — THREADING.md) holds under real concurrency.

Non-vacuity proven: a throwaway positive control (two `std::thread`s racing one
byte in loft's shared-raw-pointer shape) made TSan fire (exit 66), so the clean
run is real coverage, not an inactive detector.

Toolchain notes baked into the job (learned the hard way): TSan needs an
**instrumented std** (`-Zbuild-std` + the `rust-src` component) — unlike ASan,
linking the precompiled std ABI-mismatches (`mixing -Zsanitizer`); and the
sanitizer flag must be **target-scoped** via
`--config 'target.x86_64-unknown-linux-gnu.rustflags=["-Zsanitizer=thread"]'`
(NOT global `RUSTFLAGS`), else host proc-macro dylibs get sanitized and mismatch.
`cargo test` (not nextest) — the exact incantation validated locally.
`parallel_rebase` is intentionally excluded: it is single-threaded unit tests of
the rebase machinery (no `run_parallel_*` yet), so it adds no race coverage.
**Acceptance MET:** job green; zero findings; positive control confirms it fires.

### S4 — LeakSanitizer triage (~1 day; unblocks a stricter ASan)

1. Reproduce the baseline: build the `asan` job's target locally with
   `ASAN_OPTIONS='detect_leaks=1'` and capture the ~108 live-at-exit stacks.
2. Classify each allocation class (intentional `OnceLock`/lazy-static/interner
   vs avoidable store/String leak). Fix the avoidable ones.
3. Add `lsan_suppressions.txt` for the accepted classes; flip the `asan` job in
   `miri.yml` from `detect_leaks=0` to `detect_leaks=1` +
   `LSAN_OPTIONS=suppressions=lsan_suppressions.txt`. **Acceptance:** ASan
   `detect_leaks=1` passes corpus-wide, or each survivor has a one-line
   accepted-leak annotation.

### S6 + S9 — ASan over a generated build (design + status: [NATIVE_ASAN_DESIGN.md](NATIVE_ASAN_DESIGN.md))

One shared mechanism: `LOFT_NATIVE_ASAN=1` threads `-Zsanitizer=address` (+
nightly rustc) into the two rustc sites that compile generated native code.
Opt-in, off by default.

- **S6 (native ASan) — ✅ DONE 2026-07-09.** Injection at the standalone native
  binary compile (`src/main.rs`, `loft_native_bin_<pid>` — per-PID, uncached).
  Key finding: **ASan tolerates linking the generated crate against the
  uninstrumented `libloft.rlib`** (no `-Zbuild-std`, unlike TSan), so it's a
  ~10-line flag injection. Validated: ASan runtime active on the generated binary
  (verbosity banner); **green-drive 14/14 curated store-heavy scripts** (incl. the
  `131`/`132` UAF regression scripts) ASan-clean; positive control (raw-pointer
  OOB) fires. CI: the nightly `native-asan` job in `miri.yml`.
- **S9 (@C71 cdylib mixed-boundary) — mechanism landed; end-to-end BLOCKED.**
  Injection into `native_lib::build_shared_cdylib` (same `LOFT_NATIVE_ASAN` gate).
  Architecture validated (an ASan loft binary — 870 `__asan` symbols — drives the
  `datalib` mixed path up to the cdylib build), but the ASan cdylib build fails
  **`E0463: can't find crate for curve25519_dalek_derive`**: libloft's proc-macro
  dep is a HOST artifact in `target/release/deps`, while the cross-target ASan
  cdylib's `-L dependency` points at `target/<triple>/release/deps` which lacks
  it (the same curve25519-proc-macro class the interpreter `asan` job sidesteps).
  **The "host `deps/` on `-L`" fix was PROBED and FALSIFIED** (double-std +
  `-Zsanitizer` ABI-mismatch cascade); libloft needs ~6 host proc-macros. The
  probe-grounded candidate (link the complete-deps stable libloft, ASan the
  cdylib, load into the ASan host) is unvalidated — see
  [NATIVE_ASAN_DESIGN.md](NATIVE_ASAN_DESIGN.md) § S9. **Acceptance:** S6 met; S9
  = cross-boundary UAF/OOB caught, once a validated cdylib-deps approach lands
  (a focused session, not a one-liner).

### S5 — ✅ DONE 2026-07-09, REFRAMED (Miri is the wrong *granularity* to scale)

**Finding:** Miri-on-loft is interpreter-under-interpreter — ~5-10 min/test (the
p213 sibling timed out at 240s), because all 4 seeds run WHOLE loft programs via
`code!`. Growing that set is infeasible (8 programs ≈ 40-80 min), and ASan already
sweeps the whole corpus fast for OOB/UAF, so Miri's job is a small focused canary.

- **S5a — re-target Miri at the unsafe STORE layer.** The UB lives in a few
  `unsafe` store functions (claim / free-coalesce / resize+zero / cross-store adopt
  / store-rebase walk); reach them with DIRECT Rust unit tests over hand-built
  `Stores` — no stdlib parse, no execution. Added 6 (`store::tests::*`,
  `database::*`) to the `--exact` list (`--lib`): Miri-clean in **~8s** (vs 240s+),
  curated set 4→10. Deliberately-UB store tests (`*_panics`, the dangling-pointer
  positive control) excluded — Miri would correctly flag their by-design UB.
- **S5b — the higher-ROI complement: a whole-corpus `debug-asserts` gate.** loft's
  ~200 internal `debug_assert!`s (H5 numbering law, `Store::valid()`/`addr()`
  bounds, poison/UAF invariants) are compiled OUT of every normal test build
  (`[profile.dev.package.loft]`) and were checked ONLY by cargo-fuzz. New nightly
  `debug-asserts` job runs the corpus under `-C debug-assertions=on` at near-native
  speed — green over `--lib`+`--test issues` (after skipping one release-mode test).
  Catches invariant regressions the moment they land — far more per CI-minute than
  more slow Miri programs. Expansion to wrap/strings awaits the known stdlib
  slot-width DA items.

**Acceptance MET:** Miri set ≥ 8 (now 10) with the growth fast; plus a new
whole-corpus invariant gate the original plan didn't have.

### S1 — macOS-ARM sanitizer leg (mostly moot; ~½ day or DEFER)

`v2-validation.yml` already runs the full suite on macOS-ARM; only the
*sanitizer* leg is ubuntu-only. Either add `macos-latest` to the `asan`/`miri`
jobs (ASan + Miri both run on macOS-ARM) or **defer with a one-line note** that
the founding @P383 platform risk is already covered by the full-suite macOS-ARM
run. **Acceptance:** macOS-ARM sanitizer leg green, or the deferral note landed.

### S7 — nightly failure→issue notifier (~½ day)

Add a `notify` job to `miri.yml` that reads per-*job* conclusions (not the
overall run status — `continue-on-error`/`fail-fast:false` hold it green) via
the GitHub API and opens/updates a deduped issue on any red leg.
**Acceptance:** a forced nightly failure surfaces as a tracked issue within 24 h.

### S8 — MSan (stretch; DEFER)

Needs a fully instrumented std (heavy upstream setup). Keep deferred with the
one-line setup-cost note until the above land.

## Cross-arc dependencies

- **@PLN53 F4** — blocked on S3 (`LOFT_POISON` keystone) landing.
- **[@PLN11](../11-data-as-store/README.md) N5** — S9 is the routed-in A-leg of @PLN11's mixed-boundary soundness work; the D (differential parity) + E (Goal-E store guard) legs already landed there, so S9 is the remaining (tooling-blocked) sanitizer leg.
- **@PLAN53** (closed) — shipped the `fuzz/` crate, direct structure-fuzz targets, and the base CI stack this plan extends.

## See also

- [`plans/finished/53-sanitizer-ci-lever/`](../finished/53-sanitizer-ci-lever/README.md) — predecessor plan; shipped Wave 1 + Wave 2 kickoff; historical record of the 5-cluster fix arc and CI stack bring-up.
- [`plans/53-program-level-fuzzing/`](../53-program-level-fuzzing/README.md) — sibling spinoff; owns program-level fuzzing (F1-F5); F4 depends on S3 (`LOFT_POISON`).
- [`doc/claude/TESTING.md`](../../TESTING.md) — sanitizer-CI documentation.
- [`.github/workflows/miri.yml`](../../../../.github/workflows/miri.yml) — the nightly Miri + ASan + toolchain-matrix workflow this plan extends.
- [`doc/claude/THREADING.md`](../../THREADING.md) — store-isolation threading model that S2 (TSan) exercises.
- [`doc/claude/DATABASE.md`](../../DATABASE.md) — store allocator + arena semantics relevant to S3 (`LOFT_POISON`).
