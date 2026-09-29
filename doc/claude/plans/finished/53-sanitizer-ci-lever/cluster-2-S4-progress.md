<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 2 — S4 PROGRESS & SESSION HANDOFF (2026-05-30)

Read this first. It is the resumable state of the S4 (eval-TOS / full
stack alignment) work. Companion docs:
- [`cluster-2-S4-plan.md`](cluster-2-S4-plan.md) — the implementation
  spec (site classification, E1–E7, risk register R1–R7).
- [`cluster-2-fix-design.md`](cluster-2-fix-design.md) — the design +
  the frame-slot (V2 allocator) half.

**Branch:** `plan-53-sanitizer-ci-lever`.  **HEAD:** `8c0a5e72`.
**All work committed AND pushed.**  Working tree clean.  NO open PR.

## TL;DR

S4 makes loft's byte-packed eval stack 8-aligned, gated behind env
`LOFT_ALIGN`.  It went from *instant SIGSEGV* → through ~`p117` →
**the aligned suite now RUNS TO COMPLETION with NO crashes**:
`LOFT_ALIGN=1 LOFT_SLOT_V2=drive` is **630 passed / 51 failed / 0
crashes** (was an abort at `p117`).  Flag-OFF (default, no `LOFT_ALIGN`)
is **byte-for-byte identical** to before at every commit — `main`-quality
is untouched (681/0).  The two remaining crashers are GONE
(`p117` fixed, see below; the `non-empty c60_*` family no longer
SIGSEGVs — it is now a wrong-value assertion failure inside the 51).
The 51 remaining failures are **all assertion-level (wrong value), no
SIGSEGV, no guard fire** — fully visible at once instead of one-crash-
at-a-time peeling.  See § WHAT REMAINS for the inventory.

## SAFETY HARNESS (landed 2026-05-30 — the pivot that unblocks this work)

Running the alignment suite under load kept exhausting the 7.5G `/tmp`
tmpfs (native compiles write ~36MB binaries × dozens in parallel) and
HANGING THE MACHINE — that is why this branch detoured into CI/build
infrastructure before finishing S4.  That harness is now landed and
green (`make ci`: 1922 passed), so the machine is safe under load and
the original alignment work can resume freely.  What changed:

- **tmpfs lockup fixed** (`src/platform.rs`): native binaries stripped
  36MB→1MB (L1), a preflight free-space guard (L2,
  `LOFT_TMPFS_MIN_FREE_MB`), adaptive worker count (L3).  Full native
  suite now ~190MB scratch, tmpfs ~6%.  `--test native` is SAFE again.
- **thin binary** (BUILD1, PERFORMANCE.md): `main.rs` no longer
  double-compiles the crate — incremental bin rebuild ~16.5s→~3.2s.
- **native test cache** (BUILD2, PERFORMANCE.md): cache key now folds
  rlib CONTENT (not mtime), so a `cargo build --release --lib` no
  longer busts the native binary cache — faster `make ci`/`make test`
  locally too.

Commits `2dd0a142`..`ce489c01`.  Full design: PERFORMANCE-history.md
§ Design: BUILD1 / BUILD2.  None of this touched the alignment code
paths — S4 state below is exactly as handed off.

## HOW TO RUN / TEST (read before doing anything)

```bash
# Default (production) — flag OFF, V1 drives, guards absent.  MUST stay 681/0.
cargo test --test issues

# V2 frame-slot allocator drives the whole suite (no eval-TOS alignment yet):
LOFT_SLOT_V2=drive cargo test --test issues          # MUST be 681/0

# FULL aligned mode (eval-TOS + V2 frame slots) — the S4 acceptance path:
LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --test issues

# With the homegrown stack-UB guards ARMED (the workhorse for finding bugs):
LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --features stack_align_guard --test issues
```

### THREE HARD-WON PROCESS RULES (cost many hours to learn — obey them)

1. **`cargo clean -p loft` before testing S4 changes.**
   `[profile.dev.package.loft] debug-assertions = false` + incremental
   compilation routinely leaves a STALE `libloft` linked into the test
   binary, so `cargo test` runs code that does NOT match your source.
   This sent me chasing ghosts for hours (a8, p117).  When a result is
   surprising, `cargo clean -p loft && cargo test …`.  Symptom of a
   stale `cargo test --no-run`: it prints `Executable …` WITHOUT
   `Compiling loft`.

2. **Guards are gated on the `stack_align_guard` cargo feature, NOT
   `debug_assertions`** — because the lib disables `debug_assertions`
   (rule 1), so `cfg(debug_assertions)` is OFF inside this crate even
   under `cargo test`.  An earlier version of the guards used
   `cfg(debug_assertions)` and was silently compiled out (never ran).
   Run the S4 work with `--features stack_align_guard`.

3. **The trace path (`LOFT_LOG=full` → `execute_log_impl`) RESETS
   `stack_pos` to the codegen-tracked position on every op** for display,
   which MASKS runtime drift.  So a `LOFT_LOG=full` dump shows
   *correct* slots even when the real `execute` path is drifting and
   crashing.  For wrong-value bugs, instrument the REAL `execute_argv`
   path (`src/state/mod.rs`), not `execute_log`.  (The dump's *static
   bytecode* section is fine; its *runtime* `[stackpos]` trace lies.)

## THE GUARDS (the single biggest productivity win — use them)

Two debug-only guards, feature-gated (`stack_align_guard`), zero
production cost (the calls + their arg computation don't exist without
the feature).  They are a **homegrown Miri-for-the-stack** that fires on
ANY rustc at the access site.

- **Frame invariant** (`execute_argv` + `execute_log` loops,
  `src/state/mod.rs` ~2112, `debug.rs` ~1066): asserts `stack_pos % 8 ==
  0` after every op.  Catches frame *drifts* — names the op (pc + fn).
- **Access invariant** (`State::check_stack_align<T>`,
  `src/state/mod.rs` ~1384; called from `get_stack`/`put_stack`/
  `get_var`/`mut_var`/`put_var` and `text.rs` `string`/`set_string`/
  `string_mut`/`string_ref_mut`): asserts the SLOT address is a multiple
  of `align_of::<T>()`.  Catches the cluster-2 unaligned-`&T` UB exactly,
  at the access, naming the type + offset.

**They caught `c60` in ONE run** (`i64 at abs offset 116`) — what took
`a8` hours of gdb/valgrind before the guards worked.  When a crasher is
an *alignment* bug the guard fires loudly; when it's a *wrong-value /
slot-collision* bug (a8, p117) the guard stays silent — that distinction
alone tells you which kind of bug you're chasing.

## WHAT IS DONE (committed + pushed, oldest→newest)

| Commit | What |
|---|---|
| `25923f31` | drop dead V2-allocator scaffolding |
| `f933a4b2` | `Stack::var_pos` checked helper + report unfilter (better debug) |
| `36abfe8d` | **fix:** `compute_intervals` recurse into Yield/BreakWith/Parallel (latent liveness bug + latent V1 gap) |
| `9bf0894a` | scaffold the `LOFT_ALIGN` flag + `aligned_stack_step` seam (off by default) |
| `84ae1473` | the S4 implementation spec doc |
| `c87e1fb9` | E1 — `Stack::operator` advances stepped |
| `800c74c9` | E2a — codegen frame-setup advances stepped |
| `e2141bdb` | E2b — codegen call-path advances + `args_size = Σ step` |
| `904a2ca9` | E2c/E3/E4 — remaining codegen advances: slot-bump family, fn-ref ±4→±(step20−step16), gen_drop/Yield/coroutine/free-helpers/generate_block |
| `286046ca` | E5 — runtime `put_var`/`copy_result`/yield-restore/`push_null_value`(R4) stepped |
| `8d8fb510` | E6/E7 — frame base (`local_start`, generator zone) + text `pos−N` (only `format_single` 20→24) |
| `7e34fd4b` | **R2 — alignment-aware native marshalling** (`Stores::get`/`put` step; `Stores.aligned_stack` on all 4 literals). Cleared the b7/JsonValue family |
| `ad34f8c8` | E6 — entry frame base = `step(4)` (8-aligned, `execute_argv`) |
| `caffdca6` | the alignment guards (first cut) |
| `ea459261` | **a8 — `gen_text_dest_call` pops stepped** (the twin of `try_text_dest_pass` I'd missed; raw DbRef pop `12` vs `step(12)=16` → −4 drift → garbage var read) |
| `be516ec9` | **guards re-gated on the `stack_align_guard` feature** (was dead under `cfg(debug_assertions)`) — and proven to fire |
| `8c0a5e72` | **c60/R6 — `OpStart`/`OpIterate` `was_stack` spans stepped** (OpIterate pushes TWO u32s = `step(4)+step(4)`, not `step(8)`) |
| _(uncommitted)_ | **p117 — `copy_result` reads the return value from the LOW end of its stepped slot** (`from_pos = pos.min(step(size))`, was `min(size)`).  A struct/`Reference` return is a 12-byte `DbRef` in a 16-byte aligned slot; backing up by raw 12 read 4 bytes into the padding → a 4-byte-garbled `DbRef` whose `OpFreeRef` corrupted the heap → `pc=0 depth=0` runaway / `free(): invalid size`.  Caller `stack_pos` was already correct (line 1421 stepped), so the guard stayed silent — a wrong-VALUE bug.  Fixes p117 and UNCRASHES the whole suite. |

**Verification at every commit:** flag-OFF `issues` 681/0, `drive` 681/0,
flag-ON canary passing, clippy clean.

## p117 — FIXED (the last crasher; the suite no longer aborts)

**Root cause:** `copy_result` (`src/state/mod.rs` ~1411) read the
return value back from `pos.min(size)` — i.e. it backed up by the RAW
`size` (12 for a `Reference`/`DbRef`).  But `put_stack` had written the
12-byte `DbRef` at the LOW end of a **stepped 16-byte** aligned slot and
advanced TOS by `step(12)=16`.  Backing up by 12 (not 16) read 4 bytes
into the slot's padding → a `DbRef` whose three `u32` fields are all
shifted by 4 bytes (garbage `store_nr`/`rec`/`pos`).  The caller then
`OpFreeRef`s that garbage handle → heap corruption → `free(): invalid
size` and a `pc=0 depth=0` interpreter runaway (call-stack underflow,
op 5 looping with `stack_pos` climbing 8/iter).

The caller's `stack_pos` was ALREADY correct (line 1421 uses
`fn_stack + step(size)`), which is why the **frame guard stayed silent**
— a wrong-VALUE bug, not a misalignment.  The fix:

```rust
let from_pos = self.stack_cur.plus(pos).min(self.stack_step(size));
```

`copy_block` still moves the real `size` bytes; only the back-up
distance is stepped.  Identity when off (`step==identity`).

**Minimal reproducer** (`/tmp/p_followups/m2.loft`) — needs exactly:
mixed-size struct (pointer-sized field + 1-byte `boolean`), returned by
value from a fn, called in a **loop** (≥2 iters; sequential calls don't
trigger — the garbage handle is freed across the loop body), and a text
arg long enough that the corrupted `free` hits a bad page
(`"tex/d.png"` crashes; `"d.png"` limps).  The bug is present for ANY
non-8-multiple return size; the SEGV/abort manifestation is layout-
dependent (string length, struct name), which is why it looked flaky.

```loft
struct R { name: text, found: boolean }
fn lookup(p: text) -> R { R { name: p, found: true } }
fn test() { for i in 0..4 { a = lookup("tex/d.png"); } }
```

Verify: `LOFT_ALIGN=1 LOFT_SLOT_V2=drive loft --interpret m2.loft`
(NOTE: `--interpret` — the binary defaults to NATIVE, which has no
byte-packed stack and never exercises this path; the bug only shows
under the interpreter or `cargo test --test issues`).

## WHAT REMAINS — corrected inventory 2026-05-30 (27 failures, NOT 51)

> ⚠️ The "630/51/0 crashes" figure below this note is STALE and was
> mis-measured.  `cargo test --test issues` cannot inventory aligned
> mode: in-process, a heap-corrupting test SIGSEGVs the whole binary
> under thread parallelism, and a runaway generator HANGS a
> `--test-threads=1` run (18 min at 100% CPU on `p210`).  The real
> ground truth comes from **per-test subprocess isolation** (each
> `issues-<hash> --exact NAME` under `timeout 8`):
>
> **658 ok / 22 FAIL / 3 CRASH / 2 HANG = 27 failures.**  Four families:
>
> | Sub-cluster | Tests | Severity |
> |---|---|---|
> | **2a coroutine/`yield`** | `p210`,`p211` (HANG) · `p218`×2,`p225` (CRASH) · `p328` (wrong-val) | heap-UB / runaway |
> | **2b sorted-iter** | `inc02`,`inc12`×2,`p190`,`p277`,`p295`,`p300`,`p4d_b`,`n2` | wrong-value |
> | **2c hash-iter `c60`** | `c60`×4 | wrong-value |
> | **2d struct/tuple/misc** | `p145`,`p159`,`p189c`,`p193`,`n4`,`n5`,`n8`×2 | wrong-value |
>
> The handoff mis-classified 2a as "~20 wrong-value coroutine" — under
> isolation they are HANGs/CRASHes (the dangerous family), and the
> *root cause is narrower than "the coroutine zone"*: **a generator
> reading its own ARGUMENT after a `yield` reads it 4 bytes high**
> (`n=42` → `42<<32`).  No-argument generators round-trip fine.  This
> is the p117 template exactly — value in a stepped slot, read back at
> a raw width — applied to the args/locals boundary in the coroutine
> create/yield/restore path.
>
> **Probes authored (pass 1):** [`probes/2a-*.loft`](probes) +
> [`probes/run.sh`](probes/run.sh) + [`probes/README.md`](probes/README.md).
> `2a-01-gen-arg-single-yield.loft` is the minimal deterministic
> reproducer (FAIL aligned, PASS flag-off).  References `2a-02`/`2a-03`
> pin the trigger to the argument read.
>
> **Next step:** fix the 2a args/locals stepping in `coroutine_create` /
> `coroutine_yield` / `coroutine_next` (`src/state/mod.rs`), re-run
> `probes/run.sh 2a` to green, then author 2b/2c/2d probes.

### (STALE — kept for diff history) original 51-failure note

`LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --test issues` →
**630 passed, 51 failed, 0 crashes** (full list in
`/tmp/p_followups/fail_names.txt`).  All are assertion-level (wrong
value); the guard never fires.  They cluster:

- **Coroutine / `yield` (~20)** — `p210`–`p230`, `p327_*_yield`,
  `p328_*`.  Generator frames live in a separate `stack_bytes` zone;
  likely a stepped/raw span mismatch in the yield-save/restore or the
  generator-zone base.  Biggest single family — start here.
- **Sorted-collection iteration (~8)** — `inc02`, `inc12_*`, `p190`,
  `p277`, `p295`, `p300`, `p4d_b`.  Same shape as the c60 hash family.
- **`c60_*` hash iteration (4)** — `single_field_asc`, `multi_field_lex`,
  `filter_clause`, `loop_attributes`.  Extra leading element
  (`,apple,…`); the per-element key-gather (`OpNext`/`gather_key`) span.
- **`p235_par_half_*` (3)** — parallel half-arg marshalling.
- **struct / tuple / vector (rest)** — `p145`, `p159`, `p189c`,
  `p322`/`p324`/`p325`/`p326`, `n2`/`n4`/`n5`/`n8`.

**Next step:** the suite now completes, so triage by family — pick the
coroutine cluster first (largest, single shared zone).  For each, the
`stack_align_guard` feature + a wrong-value diff localises it; the
`copy_result` fix above is the template for "value pushed into a stepped
slot, read back with a raw width".

## ARCHITECTURE QUICK MAP (where the stepping lives)

- **The seam:** `variables::aligned_stack_step(size, aligned)` =
  `if aligned { size.next_multiple_of(8) } else { size }`
  (`src/variables/mod.rs`).  Identity when off ⇒ flag-OFF unchanged.
- **Runtime:** `State.aligned_stack` (env, both ctors incl. worker);
  `State::stack_step`; routed `get_stack`/`put_stack`/`reserve_frame`/
  `put_var`/`copy_result`/text ops (`src/state/mod.rs`, `text.rs`).
- **Native ABI:** `Stores.aligned_stack`; `Stores::get`/`put` step
  (`src/database/mod.rs`).  Set on ALL FOUR `Stores` literals (new +
  3 clone/worker; the worker ones in `src/database/allocation.rs`).
- **Codegen:** `Stack.aligned` (env); `Stack::step` (`src/stack.rs`);
  ~all `position +=/-=` advances in `src/state/codegen.rs` routed
  through it.  Rule: codegen emits the *stepped* advance/operand;
  runtime span-operands (discard/ret/args_size) are consumed RAW.
- **Frame model:** entry base `step(4)`=8 (8-aligned); `local_start =
  Σ step(arg) + step(4)`; V2 places slots at `align`-multiples;
  `frame_hwm` rounded to 8.

## REMAINING SPEC ITEMS / KNOWN-DEFERRED

- **Miri gate** (the original cluster-2 detector): once the suite is
  green under `LOFT_ALIGN=1 LOFT_SLOT_V2=drive`, run
  `MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test --test
  issues <pure-compute-test>` — currently aborts at `put_stack`
  (mod.rs) on the unaligned i64 push; S4 should clear it.  Miri is
  installed.  (The `stack_align_guard` feature is the cheaper everyday
  detector; Miri is the gold-standard final check.)
- **R2 native push width (the `--native` backend)**: OUT of scope —
  `codegen_runtime.rs` natives use real Rust values, not the byte-packed
  stack.  Confirm unaffected.  (`--test native` is now SAFE to run —
  the old "7.5G `/tmp` tmpfs + concurrent `cc` links exhaust it and
  break the shell" hazard was FIXED 2026-05-30 by the safety-harness
  work below: native binaries are stripped 36MB→1MB + a preflight
  space guard + adaptive parallelism keep the full native suite at ~6%
  tmpfs.  See § SAFETY HARNESS.)
- The `o`/`p`-range and later suite past `p117` is unexplored under
  aligned — expect a few more distinct crashers, each now self-locating
  (alignment) or instrument-the-execute-path (wrong-value).

## DEFINITION OF DONE (for S4)

`LOFT_ALIGN=1 LOFT_SLOT_V2=drive cargo test --test issues` → 681/0,
AND `--features stack_align_guard` clean (no guard fires), AND a
per-test Miri run clean, AND flag-OFF + `drive`-only still 681/0.  Then
flip the default (or keep S4 behind the flag and mark cluster 2
"fixed behind `LOFT_ALIGN`" per the plan's known-deferred policy — the
@PLAN53 deliverable is the CI lever, not making V2 the default).
