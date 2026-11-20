<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `lib/process/` — subprocess primitive

**DONE — closed 2026-11-19.**  Shipped as `lib/process` (with `lib/git` on top of it), built under @PLN179 strand 4 and @PLN119 arc F; the reference is the library's own source docs (`lib/process/src/process.loft`) and its tests.

- **The primitive:** a typed `Command` (a format string whose holes never become syntax; no shell), `run()` with input and a `timeout`, `start()` with `wait` / `alive` / `stop` (a stop takes the whole tree), and the streaming read `lines()` / `err_lines()`.  `Run` answers `ok`, `code` (`128 + n` for a signal), both streams and `timed_out`.
- **The payoff this plan named:** `tools/viewer/refresh.sh` is deleted (`tools/viewer/refresh.loft` reads git through `lib/git`); `tools/indexer/scan.sh` is deleted and `scan.loft` asks `git::tracked_files()`, so the tracked-file workaround is gone.  A loft-written `make` driver is writable now; porting the repo's own tooling is @PLN179's work, not this plan's.
- **Verified at the close** (loft `tuxedo-pr2438-post` @ 1a06bab05): `tests/lib_process.rs` 13 of 13 on the interpreter and compiled, the emulated Windows host included; `make index` and the viewer refresh run.
- **Differs from the sketch below:** the API is `Command.run()` rather than `run(cmd, args)`, and the host side is the loft binary's own `src/process_run.rs` (and `loft_process.h` on C), not a separate package with a Rust bridge.

What follows is the plan as it was opened.

## Why

Loft has no subprocess primitive today.  Every loft tool that
needs to call an external command works around it via a bash
wrapper script:

- `tools/viewer/refresh.sh` exists ONLY because the viewer
  can't shell out to `git diff` / `git log`.  ~140 lines of
  bash that loft could own directly.
- `make index` runs `tools/indexer/scan.sh` BEFORE the loft
  scanner because scan.loft can't call `git ls-files` to match
  bash's tracked-file set.  The @PLAN37 phase 07 test gate has
  a "filter loft to bash-tracked files" workaround for this
  exact reason.
- `make ci` is shell-only — couldn't be ported to a loft
  driver script if anyone wanted to.

The single largest architectural unlock for loft tools.

## Surface

```loft
struct Process {
  stdout: text,
  stderr: text,
  code:   integer,
  signal: integer  // 0 if exited cleanly
}

pub fn run(cmd: text, args: vector<text>) -> Process;
pub fn run_with_input(cmd: text, args: vector<text>, stdin: text) -> Process;
```

Streaming variant for large outputs (e.g. `git log -p`):

```loft
pub fn spawn(cmd: text, args: vector<text>) -> ProcessHandle;
pub fn read_line(self: ProcessHandle) -> text;  // null at EOF
pub fn wait(self: ProcessHandle) -> integer;    // exit code
```

## What ships

- `lib/process/src/process.loft` — the API above, declared as
  `#native` fns binding to a Rust host bridge.
- Host bridge (Rust): `std::process::Command::new(cmd).args(args).output()` for
  `run`; `Command::spawn()` + `BufReader` for `spawn`.
- Tests: smoke (`echo hello`), arg-quoting (`echo "a b" "c"`),
  stdin (`cat`), exit code (`false`), stderr (`grep` on
  empty), signal (`kill -9`).

## Security model

- `run()` invokes `cmd` directly via `execve` — NOT a shell.
  No shell injection by construction (args are a vector, not
  a concatenated string).
- No `run_shell()` variant.  If users need shell features
  (pipes, glob, `&&`), they pass `["sh", "-c", "<one-liner>"]`
  explicitly and own the escape.
- File: `doc/claude/SECURITY.md` (new doc) lists the
  process-execution attack surface and what's deliberately
  out of scope (no setuid, no env clearing, no chroot).

## Consumer changes once shipped — ALL DONE (2026-08-11, @PLN119 arc F)

This plan is superseded, and every consumer it named has been served by the
typed-library route rather than by `run()`: `tools/viewer/refresh.sh` is deleted,
and the indexer asks `git::tracked_files()` instead of mirroring `.gitignore` in
a skip list.  See `doc/claude/plans/119-out-of-process-libraries/`.

The original list:

- `tools/viewer/refresh.sh` deletes; `tools/viewer/src/main.loft`
  reads git state directly via `process::run("git", [...])`.
- `tools/indexer/src/scan.loft` adds a `git_tracked_files()` fn
  that mirrors bash's `git ls-files` enumeration; the test
  gate's "filter to bash-tracked files" workaround comes out.
- `make ci`, `make ship`, etc. become writable in pure loft.

## Effort

M (1-3 days).  Native bridge is ~50 lines of Rust;
streaming variant is the bulk.  Tests + docs another day.

## Cross-references

- [@PLAN37 phase 07](../../plans/42-tracker-index/07-loft-native-scanner.md)
  — driver use case (untracked-files parity).
- [@PLAN35](../../plans/finished/35-branch-review-viewer/README.md)
  — `tools/viewer/refresh.sh` is the largest pre-existing
  workaround this lib lifts.
- [STDLIB.md § Open work](../../STDLIB.md#open-work)
  — sibling stdlib gaps surfaced by the same dogfood pass.
