<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 49 — Execution timeout: guaranteed termination + on-timeout diagnostics

**Status — DONE 2026-05-29.**  All phases (T1+T2+T3+T4 + native subprocess arming) shipped.

Reference for the shipped feature moved to
[`../../../TESTING.md` § Execution timeout](../../../TESTING.md#execution-timeout-loft_timeout----timeout).
This file is a closure record only.

## What shipped

| Phase | Commit / Date | What |
|---|---|---|
| **T1** — watchdog hard-kill | 2026-05-27 | Watchdog thread fires at `T + grace`, dumps shared breadcrumb, then `process::abort()` — terminates even when stuck in Rust/native/syscall. |
| **T2** — cooperative rich diagnostic | 2026-05-27 | `deadline_reached()` → `graceful_exit()` (exit 124) at parse-entry, lexer-recovery, run-interpret, native fn-entry checkpoints.  Each updates the shared breadcrumb so T1's hard-kill has actionable context even if T2 misses.  NOT per-opcode: a tight runtime loop with no fn call has no checkpoint and falls to T1. |
| **T3** — CLI / env + `--tests` default | 2026-05-27 | `--timeout <secs>` + `LOFT_TIMEOUT` env var; default ON (300s) under `--tests` / `loft test` (idempotent — explicit values override). |
| **T4** — harness subprocess isolation | 2026-05-29 | `.config/nextest.toml` per-profile `slow-timeout`: 300s on `default`, 600s on `ci`, `terminate-after = 1`.  nextest escalates SIGTERM → SIGKILL on the hung TEST PROCESS, localizing a hang to one test binary; other test binaries continue. |
| **Native subprocess arming** | 2026-05-29 | `src/generation/mod.rs` `emit_main_bootstrap` injects `loft::timeout::arm(env_timeout_secs(), env_grace_secs())` at the top of the generated `fn main()`.  When `loft <prog>` spawns the native child, the child self-arms from the inherited `LOFT_TIMEOUT` env var (no-op when unset). |

## Verification

- **T1 hard-kill (native, no fn-call checkpoints):**
  ```
  $ LOFT_TIMEOUT=2 timeout 8 ./target/release/loft /tmp/hang.loft
  [timeout] hard-kill after 2s+2s grace: phase=run-native fn=main file=/tmp/hang.loft:1
  ```
  Hang in a while-true loop with no fn calls — falls through to T1 at T+grace=4s.
- **T2 cooperative (native, with fn-call checkpoints):**
  ```
  $ LOFT_TIMEOUT=2 timeout 8 ./target/release/loft /tmp/hang_with_calls.loft
  [timeout] deadline reached after 2s (graceful): phase=run-native fn=helper file=/tmp/hang_with_calls.loft:1
  ```
  Hang in a while-true loop that calls `helper()` — T2 fires at T=2s with the active fn's breadcrumb.
- **No timeout armed:**
  ```
  $ timeout 2 ./target/release/loft /tmp/hang.loft
  # outer `timeout 2` had to SIGTERM (exit 143) — loft did not self-terminate, as expected.
  ```

## Tools added during this plan

| Tool | Status | Used for |
|---|---|---|
| `--timeout <secs>` CLI flag | New (`src/main.rs`) | Arm the watchdog from the command line. |
| `LOFT_TIMEOUT` env var | New (`src/timeout.rs::env_timeout_secs`) | Same as `--timeout`, but inherited by subprocesses (native binary spawn picks it up automatically via the arming injected by codegen). |
| `LOFT_TIMEOUT_GRACE` env var | New (`src/timeout.rs::env_grace_secs`) | Override the default 2s grace period before the hard kill. |
| `LOFT_TIMEOUT_CLEAN_EXIT` env var | New (`src/timeout.rs::print_breadcrumb_and_abort`) | Replace `process::abort()` with `process::exit(124)` for CI runs that prefer clean exit codes over core dumps. |
| `nextest slow-timeout` | New (`.config/nextest.toml`) | Per-test-binary watchdog at the test runner level — defense in depth above the in-process T1. |

## See also

- [`../../../TESTING.md` § Execution timeout](../../../TESTING.md#execution-timeout-loft_timeout----timeout) — user-facing reference for shipped flag + env var + auto-arming.
- `src/timeout.rs` — watchdog, deadline atomics, breadcrumb store.
- `src/state/mod.rs`, `src/codegen_runtime.rs`, `src/lexer.rs` — checkpoint call sites.
- `src/generation/mod.rs::emit_main_bootstrap` — native subprocess arming injection.
- `.config/nextest.toml` — T4 slow-timeout config.
- [`../../../PROBLEMS.md`](../../../PROBLEMS.md) — `@P356/#9` and other hang P-issues this closed.
