<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Windows session — what to do when temporary Windows access arrives

Companion to [WINDOWS.md](WINDOWS.md) (the reference doc on Windows compatibility state).  This file is the **session-prep checklist**: priority-ordered investigations, time budget, what to verify FIRST so the session yields signal even if it's short.

## Time budget

A single 1-day session can realistically:
- Verify 2-3 of the G1-G4 gaps (each is "reproduce + capture real error + decide on fix").
- NOT ship all gap fixes; the session's goal is **verified Windows error output**, not closure.

A 2-day session can typically close 1-2 gaps end-to-end (reproduce → fix → un-skip).

Plan for under-2-days first.  Most gap fixes that look "1-day" reveal Windows-toolchain quirks that eat the rest.

## Pre-flight checklist (5-10 min on the laptop)

```powershell
# 1. Rust stable with the MSVC target (default on Windows host)
rustup default stable
rustup target list --installed  # confirm x86_64-pc-windows-msvc

# 2. MSVC build tools — link.exe must be on PATH
where.exe link
# If missing: install "Build Tools for Visual Studio 2022" → "Desktop development with C++"

# 3. cargo-nextest (CI uses this, local repro should match)
cargo install cargo-nextest --locked

# 4. gh CLI for diffing against CI runs
where.exe gh
# If missing: winget install --id GitHub.cli

# 5. Clone + initial build (catches base-toolchain issues immediately)
git clone https://github.com/loft-lang/loft
cd loft
cargo build --release
# 6. a C compiler for --native: MSVC cl, or gcc 16+ (WINDOWS.md § The C compiler on a Windows runner)
```

If any pre-flight step fails, that's the FIRST gap to investigate — base-toolchain breakage gates everything else.

## Priority queue — investigate in this order

### Priority 1 — verify the v2 probe result ✅ DONE (2026-05-30, local host)

**Outcome: PASSES.**  Re-ran on a real Windows host (rustc 1.96.0 MSVC):
`multiplayer_v2` 3/3, `multiplayer_v3` 2/2, `multiplayer_v5` 5/5 — all 10
P229b tests green, no code change, no `cfg_attr(windows, ignore)` gate left.
@P229b confirmed incidentally fixed.  (Original probe text retained below for
context.)

A probe commit (libraries3 `baa9c3e2`, 2026-05-29) temporarily un-ignored `v2_single_client_completes_game` on Windows to surface real `diagnose_listen_failure` output.

```powershell
cd loft
cargo nextest run --test multiplayer_v2 v2_single_client_completes_game
```

Three outcomes:
- **PASSES** → @P229b was incidentally fixed sometime in the last 3 weeks.  Un-ignore all 10 P229b tests across `multiplayer_v{2,3,5}.rs`.  Update G1 in WINDOWS.md.
- **FAILS with port-bind error** in the captured stderr → the 2026-05-21 hypothesis confirms; apply the SO_REUSEADDR / server-binds-:0-itself fix described in WINDOWS.md G1.
- **FAILS with `code: 206` or another spawn error** → same problem space as PR #228 (cmdline overflow).  The spawn-cmdline of the server subprocess is short, so this would point to something else like a missing DLL path or working-dir issue.  Capture the full stderr and investigate.

### Priority 4 — ~~G4 `parallel { }` worker stack snapshot~~ — FULLY CLOSED 2026-05-30

Both halves verified clean:
- Interpreter half: local Windows host (2026-05-30) — `80-parallel-block.loft`
  + `81-parallel-outer-vars.loft` exit 0; `tests/threading.rs` 47/47.
- Native half: `windows-latest` CI runner (run 26689698213, 2026-05-30) — both
  scripts compiled + executed under `--native`, exit 0.  The runner does not
  enforce WDAC, so unsigned binaries run normally.

`@P229` G4 is fully closed.  No action needed here.

### Priority 5 — opportunistic checks

While the laptop is available, also probe:
- `loft install <pkg>` against a real (non-fixture) registry — @P332 was fixed assuming `LOFT_HOME` plumbing works on Windows; not verified end-to-end.
- `--html` build path on Windows (untested per WINDOWS.md compatibility table — needs `wasm-opt` available).
- Any test marked `STATUS_HEAP_CORRUPTION` previously surfaced (Windows allocator is stricter than Linux's, may catch new OOB bugs).

## What NOT to do

- **Don't try to fix everything.**  The session is finite.  Verified output beats unverified fixes.
- **Don't trust the leading-hypothesis labels in WINDOWS.md / PROBLEMS.md.**  All gap hypotheses are "code review only, unverified" (per the 2026-05-21 note).  PR #228 just proved one of them (P229b implicit assumption about subprocess spawn) was incomplete — the real cause was elsewhere.  Capture the actual Windows error output before adopting any fix.

## After the session

For each gap touched, update:
1. WINDOWS.md — move from `## Known gaps` to `## Previously fixed Windows-only issues` if closed.
2. PROBLEMS.md — close the P-issue with the verified diagnosis.
3. `tests/multiplayer_v{2,3,5}.rs` — un-ignore the affected tests if G1 fixed.
4. CHANGELOG_TECHNICAL.md — note the Windows-specific fix.

If gaps were verified-but-not-fixed (real error captured, fix needs more time), update the corresponding WINDOWS.md gap section with the captured error message so the next session starts from real data, not hypothesis.

## See also

- [WINDOWS.md](WINDOWS.md) — full reference for compatibility state, gap details, runbooks.
- [PROBLEMS.md](PROBLEMS.md) — @P229 (parallel + multiplayer), @P332 (install), @P333 (lib fixtures).
- [TESTING-history.md](TESTING-history.md) — the closed @P229b row.
- [WINDOWS_SESSION-history.md](WINDOWS_SESSION-history.md) — the G2 / G3 priorities of the
  rustc writer's `--native`, and the native-execution blocker they met.
