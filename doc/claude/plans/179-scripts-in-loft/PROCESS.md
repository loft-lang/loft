<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN179 strand 4 — `Command` + `run`: a subprocess under the two rules

The design for running a program from loft, owner-directed 2026-09-30, reversing
[@PLN119 § Why not a subprocess primitive](../119-out-of-process-libraries/README.md) under
two rules that keep what it protected: **a value can never become syntax** (the command is a
typed format string), and **a stream is never left without a reader** (`run` drains every
pipe).  The plan that owns it is [README.md](README.md).

**Where it lives (owner, 2026-09-30): a `process` LIBRARY, invisible on use, like `regex`** —
not the stdlib.  Running a program is outside loft's ordinary surface (a game never does it)
and impossible in the browser, so it must not sit where every program sees it.  The library
opts into auto-use (`[triggers] enabled = true`): its trigger surface is derived from its own
`pub` declarations — a TYPE trigger for `Command` and METHOD triggers for what hangs off it
(`run`, `lines`) — so `c: Command = "git log -n {n}"` loads the library and nothing in the
script names it; `process::` stays the spelling that always works.  A free function is not a
trigger, which is why the whole surface hangs off the type.  Its natives live INSIDE the loft
binary (`[native] in_binary = true`, the `engine_host` shape): a subprocess is a host
privilege the binary grants, and the drainer threads below are the binary's to own.  The
wasm builds carry none of them, so `make surface-gen` records the surface unavailable on
`--html` and WASI and admission refuses it there by name, exactly as a `#c` call is refused.

**4a — the `Command` type (S).**  A stdlib struct that opts into typed format strings:
`lit` splits the author's bytes into words on whitespace (a quoted span in the literal stays
one word); `hole_text` / `hole_int` / `hole_float` / `hole_boolean` append ONE argv word —
never split, never quoted, joined onto the open word when the literal touches it
(`--format={fmt}` is one word); `hole_text` with `null` omits the word, so an optional flag
composes without an `if`.  Two typed holes carry the cases the argv rule alone does not
cover, the way `SqlIdent` does for a table name: `args(v: vector<text>)` splices a list, and
**a value that begins with `-` is refused unless it came through `flag(v)`** — because an
option is syntax too (@PLN119's `git -c core.sshCommand=…` point survives the reversal as a
hole type rather than a closed vocabulary).  Gate: a matrix of literal/hole compositions
against the argv each must produce, hand-computed, on both backends.

**4b — `run`, and the output channel (S–M).**  Two shapes of caller, one mechanism.  A
script that wants an OK notice writes `r = c.run(); assert(r.ok, r.stderr)` and reads
`Run { ok, code, stdout, stderr }`, everything collected.  A script facing a lot of output —
`git log`, a `cargo build` that talks on stderr, a `find` over a tree — must consume it as it
arrives with flat memory, and must be unable to deadlock however slowly it consumes.  The
evaluation, with the shapes loft has:

| shape | reads as | verdict |
|---|---|---|
| collect everything (`c.run()`) | one value | right for the notice; wrong past a few MB |
| a loft generator (`c.lines() -> iterator<text>`, `yield` per line) | the native loft loop | laziness on `--native` is the CL-9 subset today (COROUTINE.md), so a generator can run the child to completion before the first line — a risk to measure, not a mechanism to rely on |
| a callback (`c.run(fn(line) { … })`) | inversion of control | control flow (break, early return, an error) goes through a closure; declined |
| a handle with `read_line` / `wait` (`p = c.spawn()`) | Python's `Popen` | the cursor the caller forgets stderr on — the deadlock the rule forbids; declined as the public shape |
| a polled read with a wait (`p.next_line(ms)`) | `host_input(wait_ms)` | the engine-loop shape; nothing here loops per frame |

**Design: one drainer, two views.**  Underneath every run, the natives start a reader thread
per pipe the moment the child exists, feeding stdin beside them, so no pipe ever lacks a
reader — that is rule 2, enforced once, in the binary, and no surface above it can undo it.
Above it, `run()` collects, and **`lines(which: Stream = Stream.Out)` is a native-backed
cursor** — a `Lines` value with `next() -> text?` that the `for line in c.lines()` loop
drives, yielding each line as it arrives while the other pipe keeps collecting — not a loft
generator, so its laziness does not depend on the backend.  `Stream.Err` and `Stream.Both`
(a `(Stream, text)` per line, in arrival order) serve the tools that talk on stderr.  When
the loop ends the same value carries `code`, `ok` and whatever the other pipe collected.
Back-pressure is the pipe's own: the queue between drainer and cursor is bounded, a slow
consumer makes the child block on `write` as it would under a shell, and the other pipe is
still drained, so it is slow and not stuck.  A loop left early (a `break`, a `return`, a
panic) drops the cursor, and the drop ends the child (`SIGTERM`, then the kill-after-grace
`LOFT_TIMEOUT` already has) — no zombie, no reader thread outliving its loop.  A collected
stderr nobody reads is bounded by an option (`Streams.Collect | Inherit | Discard`), and
`Inherit` is the one that passes a tool's own progress through to the person watching.
`c.bytes()` is the same cursor over chunks for binary output (`git show` of an image), and
`run(c, input: text)` feeds stdin from a value.  A capability group `process#update` (running a program acts on the world; the right vocabulary is read, update, append) keeps a
sandboxed script from running anything ungranted (SANDBOX.md S1).

**Gate, before any port calls it** — the probes that can fail, on both backends: (1) a child
writing 1 MiB to stderr while the parent reads stdout by line, against the same child under
`subprocess.Popen(...).stdout.read()`, which hangs; (2) the first line of a 10-second
producer arrives before the producer exits, by timestamp (the laziness claim); (3) 1 GB
through `lines()` with flat RSS (the back-pressure claim); (4) a `break` after one line
leaves no child behind and returns within the grace; (5) the OK notice, `c.run()` of `true`,
timed against Python's `subprocess.run` — the performance axis on the cheapest call.

**4c — recording (S) — BUILT (2026-10-08, `src/process_run.rs` § recording).**
`LOFT_RUN_RECORD=<dir>` writes each collecting run as `<dir>/<NNN>-<program>/{argv, stdin,
stdout, stderr, code}` — `argv` one word per line, numbered in the order the runs happened —
and `LOFT_RUN_REPLAY=<dir>` answers each run from the first unused entry whose `argv` and
`stdin` match (a repeated call walks its entries in order and stays on the last; a call with
no entry answers -1 and says so on stderr, so a twin goes red rather than running the live
tool).  Inside `run`, so every port is twin-able offline; `start()` is never recorded.  The
original's side reads the same directory through `tests/comparisons/scripts/replay_tool.sh`,
one bash shim installed under each tool's name in `tests/comparisons/scripts/bin/`, and
`script_twin.sh --replay <dir>` arms both sides at once.  The format is plain files on
purpose: a recording is a committed fixture, read and refreshed by hand — by running the
ORIGINAL through the shim in record mode against a scratch repository, which is how
`check_bundle_fresh`'s four cases were made.

**4d — the tools, in the order the work list ranks them.**  The first port over `run` is
`scripts/check_bundle_fresh` (2026-10-08): `git diff --name-only {base}...HEAD` as a
`Command`, twinned on four recordings, both backends, faster than the bash original.  Until
`process` is published a port spells `use process::*;` (the trigger surface fires from the
registry index) and `scripts/loft.toml` names the library by path.  `git` first (`lib/git` rewritten
over `run`, twinned against its natives, which then retire); `gh` and `cargo` as the
originals call them; `curl` through the `web` library where the script already speaks
JSON.  Each lands with its first consumer ported and twinned; an interface nobody calls is
green by construction and is not a phase.

