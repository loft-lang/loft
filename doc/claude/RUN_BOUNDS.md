<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Run bounds — what stops a run that goes wrong

loft runs are unbounded by default.  These are the bounds that stop a run which loops, grows
without limit or reads outside its store, and the rule that cleans up what a run leaves in the
temp directory.  Running the suite: [RUNNING_TESTS.md](RUNNING_TESTS.md).

| Bound | Catches | On by default |
|---|---|---|
| [Execution timeout](#execution-timeout-loft_timeout----timeout) | a run that does not finish | under `--tests` / `loft test` (300 s) |
| [Store-memory ceiling](#store-memory-ceiling-loft_memory_limit) | a run whose heap grows without limit | under `--tests` / `loft test` (2 GiB); any run with `LOFT_MEMORY_LIMIT` set |
| [Hang guard](#hang-guard-loft_max_ops) | names the LOOP a run is stuck in | only in a build with debug assertions on |
| [Debug boundary checks](#debug-boundary-checks-debug-builds-only) | a read or pop outside its record or stack | only in a build with debug assertions on |

## Scratch hygiene — what loft writes to a temp directory, and what removes it

Every native compile, html export and test probe lands in the temp directory (`TMPDIR`; under
`make ci` the per-checkout `/var/tmp/loft-test-scratch-<checkout>.<cksum>`, the one tag the Makefile, `find_problems.sh` and `disk_headroom.sh` all derive), and each family has one rule
that removes it — scratch nobody removes grows into hundreds of GB.

| what | who writes it | what removes it |
|---|---|---|
| `loft_native_bin_<pid>`, `loft_native_<pid>.rs` | a `--native` run's compile | the run itself when it ends normally; a run killed from OUTSIDE (a `timeout` wrapper, a harness kill, Ctrl-C) cannot, so **every native compile first sweeps the artefacts of dead processes** (`platform::reclaim_dead_native_scratch`, silent) |
| `loft_native_cache_<checkout>/loft_native_<stem>{.rs,_bin,_bin.key}` | the native test harness (`tests/native.rs`): this checkout's per-program binary cache, in a directory named for the checkout so nothing else ever writes there | **the harness itself, at the start of a run against a loft build the directory has not seen** (`platform::sweep_own_native_cache`: the rlib's path and content hash are the stamp in `.build`; a new stamp drops every entry older than two minutes — a concurrent shard's are younger — so the tmpfs never holds two builds' binaries).  Only its own directory, so never another checkout's or another process's work.  `sweep_scratch.sh` runs its dead-pid rule inside it and removes the whole directory when nothing in it moved for a day |
| `loft_test_native_<stem>_<key>_bin` (built in `loft_test_native_<pid>/`, published by rename) | `--tests --native`, a per-PROGRAM binary cache keyed by the native cache key — never a shared path a sibling process writes (loft#1626) | the low-space reclaim (aged entries) and `sweep_scratch.sh --days` |
| `<dir>/.loft/cache/<entry>` | the program cache a test writes beside its probe — every probe has a fresh name, so the cache only grows | `sweep_scratch.sh` (entries older than a day, in a temp dir); `scripts/disk_janitor.sh` (under a checkout's `tests/`, older than two days) |
| `loft_html_*`, `loft_p*`, `loft_rebuild_*`, `loft-*` | the html, probe, rebuild and serve suites | `sweep_scratch.sh` (older than a day) |
| `~/.cache/loft-falsify/<ref>{,-target}` (`LOFT_FALSIFY_CACHE`) | `make falsify` control builds | the script itself, LRU to `LOFT_FALSIFY_KEEP` after a successful build; `sweep_scratch.sh` a control unused for `--falsify-days` (7), a failed build's included |
| `~/.cache/tmp/claude-<uid>/<project>/<session>`, or `/tmp/claude-<uid>/…` where `TMPDIR` is unset | the agent harness's per-session scratch | `make sweep-scratch` (nothing in it changed for two weeks — two days on a RAM tmpfs); within those two days, the agent that wrote it — see below |
| `target/debug/deps` | cargo: every test binary of every dependency hash ever built (tens of GB per checkout) | `scripts/disk_janitor.sh`, beside every build (`cargo sweep --time 3`, `--time 1` under 50 GB free), on every checkout, under cargo's own build lock; `make sweep-target` by hand |
| `target/*/incremental` | cargo: the incremental compilation cache (9 GB measured) | `scripts/disk_janitor.sh` (sessions older than a day, under the build lock); `scripts/disk_headroom.sh`, whole, when the disk is short — it costs a rebuild's time, nothing else |
| `<package>/native-auto/libloft_auto_<pkg>_<fp>.so` | the native package loader (`native_lib::cached_or_build_shared_cdylib`): one cdylib per loft build and consumer type layout — about 110 MB each for the fixture packages under `tests/fixtures/` | `native_lib::prune_artifacts`, after a build in the same directory: the newest 8 stay (`KEEP_ARTIFACTS`).  Bounded, but at that size 8 per fixture is about 0.9 GB each and about 10 GB over the fixtures, and nothing takes the tail of a fixture no test rebuilds |
| `~/.cache/loft/stdlib-<key>.store` | the startup cache (`cache::stdlib_cache_path`): one image, about 7 MB, per distinct stdlib source set | `save_stdlib_cache` keeps the 16 most recently used images (`KEEP_STDLIB_IMAGES`); `cache_gc` and `sweep_scratch.sh` do not look in `~/.cache/loft` |
| `/var/tmp/loft-test-scratch-<checkout>.<cksum>` as a whole | ONE gate run's fixtures and native test cache (23 GB measured in a single run — today's entries, which the day-old rule keeps) | `scripts/disk_headroom.sh`, when the disk is still short after the steps above and NO gate of this checkout is alive (its pid file and `.ci-running`, liveness-tested): a finished run's fixtures are garbage, the next run writes fresh ones, and the native cache is rebuilt |

**A session scratch on a RAM-backed `/tmp` is memory, not disk.**  A control worktree's
`target/` or a probe directory's `.loft/` cache left there counts against the memory a gate's
compile needs, and the system's out-of-memory daemon then kills cargo with nothing else running
(measured: two gates in a row, 4 GB of one session's scratch — a 2.4 GB control
build and 1.4 GB of program caches).  `df -h /tmp` before a gate; clear a scratch worktree's
build with `cargo clean --manifest-path <worktree>/Cargo.toml` and remove scratch `.loft/`
caches when the probes are done.  Build a scratch worktree OUTSIDE a RAM `/tmp` (under
`~/.cache`) in the first place.

**A direct run does not get the gate's redirect.**  `find_problems.sh` and the gate export
`TMPDIR` and `LOFT_TMPDIR` as the checkout's `/var/tmp/loft-test-scratch-<checkout>.<cksum>`;
a bare `cargo test`, `loft --native` probe or `perf record` started by hand inherits `/tmp`.
On a RAM tmpfs that is where the large writers fail: `ownership_drop_gate`'s four native chunks
write about 1.3 GB each at once and stop on `Disk quota exceeded (os error 122)`, and a chunk
that panics leaves its directory behind.  Export the same two variables for a direct run
(`TMPDIR=/var/tmp/loft-test-scratch-<tag> LOFT_TMPDIR=<same>`, the tag as `find_problems.sh`
derives it), and keep profiles, emitted programs and their binaries in a disk directory, removed
when the measurement is recorded.

**The builds run the janitor.**  `scripts/disk_janitor.sh` starts, detached, beside whatever
starts a build: `find_problems.sh`, `bench/stats.py`, and every build-shaped command an agent
runs, through a user-level Claude Code hook (`make janitor-install`; the disk is a machine
concern, so the hook is not in the repo's settings).  At most hourly, at once under 50 GB free,
it applies the rules above to EVERY loft checkout and worktree of the box, not only the one
that started it. The checkout's cargo build locks are taken without waiting, so a build that
starts meanwhile waits the seconds a sweep takes, and a checkout whose lock is held is left for
the next round.  Measured: the disk ran from 18 GB free to 0 during one test run, with
117 GB of artefacts no build had used for two days and 21 GB of finished runs' binaries in
`~/.cache/loft-scratch`, which no rule was pointed at; the 19 native failures of that run were
the full disk.  A detached run takes seconds and the caller waits about 75 ms.  Below 15 GB after
a run it raises a desktop notification naming the largest directories; `~/.cache/loft-janitor/log`
records every run that freed space.

**A build cleans up after the build before it, but only its own.**  The 4 259 binaries (13 GB
of a 16 GB tmpfs) measured in one sweep were one checkout's native test cache compiled against
the day's earlier loft builds by bare `cargo test` runs, and no rule removed them: each was
younger than a day, and the harness only ever overwrote the entries a run reached.  The cache
now lives in a per-checkout directory and forgets an older build at the next run's start; the
boundary is the directory, not the file name, which is what keeps a sibling checkout's live
run and every other program's files out of reach.

**One native corpus run does not fit a 16 GB tmpfs.**  A test binary is stripped
(`platform::native_strip_symbols`) and still 12–14 MB — loft's own runtime at `opt-level=0` is
most of it — so the roughly 1 800 programs of `native_scripts_*` need about 22 GB, and the
harness keeps every one for the next run.  Its preflight (`tests/native.rs`,
`platform::native_compile_space_ok`) reclaims only artefacts of OLDER builds, and when that is not
enough it SKIPS the test; the shard summary counts each skip as `compile failed`, so a run that
filled the tmpfs reads as hundreds of compile errors.  Read `SKIP … low temp space` in the output
before reading the count.  Until the harness evicts its own least-recently-used entries before it
skips, run the corpus with `TMPDIR` on the disk (`find_problems.sh` does) — a test binary run by
hand without it lands on the tmpfs.

**Every gate makes room before it starts.**  `make ci`, `make test`, `make quick` and
`find_problems.sh` (both entry points) run `scripts/disk_headroom.sh` (`make disk-headroom` by
hand), which reclaims in the order of what it costs to lose — the standing sweep, the
incremental caches, this checkout's whole gate scratch, `cargo sweep` — until 20 GB is free,
prints one line when it acted and nothing when it did not, and REFUSES below 2 GB so the gate
stops there instead of reporting truncated object files and missing rlibs as test failures
(measured: 71 of 5312 red with 2 MB free).  Beside it, `scripts/sweep_scratch.sh`
alone runs on the checkout's scratch at the start of every gate, and `make sweep-scratch` runs
it on the checkout's scratch and on `TMPDIR` with the session prune, printing `df` after.  All
of them touch only loft's own names, only dead pids or aged entries, and never a sibling
checkout's gate scratch.

**And an agent session sweeps after every turn.**  `scripts/tmp_headroom.sh` is the harness's
`Stop` and `SubagentStop` hook (`.claude/settings.json`): the standing sweep over `/tmp` and
`TMPDIR` with the session prune, then — when `/tmp` is below 4 GB and no cargo, nextest or
rustc process is alive — the suites' native binary caches (`loft_native_cache_<checkout>/`,
14 GB measured after one `cargo nextest` run started by hand with `TMPDIR` unset, and the
`loft_test_native_*_bin` entries; the next run rebuilds them), then `disk_headroom.sh` for
the checkout's disk.  A test run started by hand should set `TMPDIR` to the checkout's gate
scratch, as `find_problems.sh` does, so its cache lands on the disk and not on the tmpfs.  The two fill
differently and each kills the box in its own way: `/tmp` is a 16 GB tmpfs on the Lima box,
and a full one stops the harness from capturing any command's output, while a full disk fakes
a red gate (both measured, once each way, in the session that added it).  Silent
when nothing needed doing; `/tmp/loft_tmp_headroom.last` is the stamp of its last run.  It
never touches a live run's caches or another program's files, and it does not replace the
20 GB floor a gate refuses under — lowering `LOFT_GATE_MIN_FREE_GB` for one run is what let
the second fill happen.

## Store-memory ceiling (`LOFT_MEMORY_LIMIT`)

The sibling of the execution timeout, for the failure it cannot catch.  A corrupted length does
not always end in a bad dereference — often it ends in an **allocation**, which reaches tens of
GiB in seconds.  The kernel's OOM killer is no diagnostic: it reports only that a process died,
and may kill a bystander instead of the culprit.

So the ceiling lives inside loft, where the thing being allocated still has a name.  When a
store growth would cross it the run stops **at that growth** — the store is left exactly as it
was — and prints what filled the heap:

```
loft: store memory limit reached — 1.7 GiB in use, limit 2.0 GiB

  the growth that crossed it
    a store of type `main_vector<Cell>` (kt=78) growing 1.7 GiB → 4.0 GiB
    the store was allocated at pc=8001

  where the memory is
    main_vector<Cell>        kt=78        1.7 GiB  in 1 store
    ?                        kt=65535     9.6 KiB  in 2 stores

  One type holding nearly all of it in ONE store is a runaway length;
  the same total spread over very many stores is a leak.
```

That last line is the point of the breakdown: **one store vs very many** separates a runaway
length from a leak, which you would otherwise have to go and measure.

| Mechanism | Default | How |
|---|---|---|
| Ordinary run | **off** | loft is unbounded by default; a real program may want the machine |
| Under `--tests` / `loft test` | **on, 2 GiB** | a test wanting tens of GiB is a bug either way |
| Env var | — | `LOFT_MEMORY_LIMIT=<size>` — `2G`, `512M`, `64M`, or `0` to remove it |

A limit that cannot be parsed is reported and the default is **kept** — a typo must never
silently remove the ceiling.  A repeat-run harness for a corruption repro also caps the PROCESS
(`ulimit -v`): the runaway is not necessarily the process the kernel kills.

Implementation is `src/store_budget.rs` (accounting, ceiling, report) with the accounting
asserted at every site in `src/store.rs` that allocates, reallocates or frees a store buffer:
`new`, `open`, `resize_store`, `shrink_to`, `clone_locked`, `snapshot_copy` and `Drop`.  Type
names reach the report through `Stores::publish_type_names`, called once per run and inert when
no ceiling is set.  Guarded by `tests/store_memory_limit.rs`.

**No `file:line` in the report, deliberately.**  The interpreter's span table records CALL
SITES only, so resolving an arbitrary allocation pc through it returns the nearest span
*below* — routinely in an unrelated function.  A diagnostic that sends the reader to the wrong
file costs more than one that stays quiet, so the report prints the pc and stops there;
`LOFT_STORES=timeline` resolves the same pc against the denser per-run table.

## Hang guard (`LOFT_MAX_OPS`)

> **Reach for `perf` FIRST when the hang is in a build you already have.**  This guard needs a
> rebuild with debug assertions on (below), which is minutes; a running process can be sampled
> in seconds and needs nothing:
>
> ```bash
> <the hanging command> & BGPID=$!
> sleep 6 && perf record -F 199 -g -p $BGPID -o /tmp/hang.data -- sleep 6
> kill -9 $BGPID; perf report -i /tmp/hang.data --stdio --no-children | head -15
> ```
>
> `gdb -p` needs ptrace, which a restricted box refuses; `perf` does not, and it works on a
> release build.  It buys what `LOFT_MAX_OPS` buys: the loop, not just the fact.
>
> ⚠ **Read the answer as a LOCATION, not a cause.**  Samples all in `State::execute_argv` say
> an interpreter loop is not terminating; they do not say the DEFECT is in the interpreter.  A
> compile-time decision can build a loop that cannot end — a `for` that walks a materialised
> copy while its body empties the original, with no compile-time symptom at all.  When the
> samples name a runtime loop, ask what COMPILED that loop differently, and bisect the change
> set by building and timing.

The interpreter counts executed operations and, on reaching the ceiling, panics with the last
sixteen ops, each resolved to `function+offset: OpName`.  That trail is the value of it: a
timeout says a run did not finish, this says which loop it did not finish in.

⚠ **"Debug-assertions only" does NOT mean "any debug build" — by default it is no build you
have.**  `Cargo.toml` carries

```toml
[profile.dev.package.loft]
debug-assertions = false        # ~270x on the hot-path store guards
```

so the loft LIBRARY is compiled without them, and the guard is absent from the binary
`cargo build --bin loft` produces AND from the test binaries: an infinite loop under
`LOFT_MAX_OPS=100000` runs until `LOFT_TIMEOUT` kills it.  The same holds for every other
`#[cfg(debug_assertions)]` item in `src/`, including `check_arg_ref_allocs` and
`check_ref_leaks`; the store LEAK check is not gated and is unaffected.

To use it, flip that one line to `true` and rebuild into a separate `--target-dir`, so the main
target tree is not invalidated:

```bash
sed -i 's/^debug-assertions = false/debug-assertions = true/' Cargo.toml   # revert after
cargo build --bin loft --target-dir /tmp/loft-dbg
LOFT_MAX_OPS=100000 /tmp/loft-dbg/debug/loft --interpret --path . prog.loft
```

(`--path .` because the stdlib is found relative to the binary.)

| Mechanism | Default | How |
|---|---|---|
| Any interpreter run, debug assertions ON | **on, 4e9 ops** | roughly a minute of debug-assertion interpretation |
| Debug assertions OFF (every release build) | **off** | the counter does not exist |
| Env var | — | `LOFT_MAX_OPS=<count>`, or `0` to remove it |

A count cannot tell a long run from a hung one, and its message says "hung".  So the default
sits well above the project's own suite — a ceiling that legitimate tests cross turns the
debug-assertions gate into a known red, and a gate read that way stops being run.  Set it
*low* (`LOFT_MAX_OPS=50000000`) when hunting a hang and you want the trail sooner.  An
unparseable value is reported and the default is **kept**, the rule `LOFT_MEMORY_LIMIT` follows.

Implementation: `crate::keys::max_ops` (one cached env read), read once outside the dispatch
loop in `src/state/mod.rs`; the trail is the `trail_pos` / `trail_op` ring beside it.

### Reading the debug-assertions gate locally

The nightly gate is

```bash
RUSTFLAGS='-C debug-assertions=on' LOFT_STORE_GUARD=1 CARGO_TARGET_DIR=/tmp/loft-da \
  cargo test --release --no-fail-fast --lib --test issues --test wrap --test strings \
  --test frame_vars -- --skip library_suite
```

and a separate target dir outside the repo gives reds that are not the code:

- **Tests that SPAWN the built binary** — a binary outside the checkout cannot find `default/`,
  so the subprocess exits non-zero and the test's `assert!(out.status.success())` fires.  CI
  builds into the repo's own `target/` and has no such failure.  Separate the populations
  before believing a red: an assert inside `src/` is the gate speaking; a test-file `assert!` on
  a spawned process's exit status is probably the target dir.
- **Replaying ONE guard file** on that binary —
  `LOFT_STORE_GUARD=1 /tmp/loft-da/release/loft --path <checkout> --tests [--native] tests/scripts/<guard>.loft`:
  1. Without `--path <checkout>` it exits 1 with *"cannot load standard library"* before any
     cell has run.
  2. `--tests` takes the file IMMEDIATELY after it (§ [Execution timeout](#execution-timeout-loft_timeout----timeout));
     `--tests --errors=compact <file>` runs the WHOLE corpus until the watchdog ends it.

Read a red as the guard's own only when it names cells and its sibling cells pass.

## Execution timeout (`LOFT_TIMEOUT` / `--timeout`)

Guards against hangs that would wedge `cargo test` or `find_problems.sh`.  The deadline is
layered:

- **Cooperative diagnostic check** fires at `T` (the requested deadline) — at every loft
  checkpoint (fn-entry on both backends, the lexer recovery loop), and on the interpreter at
  ANY op: the watchdog sets the dispatch loop's stop flag at `T`, which the loop tests after
  every op anyway (`@FR-R-DispatchStop`), so a loop that calls nothing stops cleanly too and
  names the frame that runs.  Raises a typed `Timeout`,
  dumps the call stack (interpreter: `crash_tail` + `StackFrame`; native: the `CALL_STACK`
  thread-local), and exits cleanly with `124`.
- **Watchdog hard-kill** fires at `T + grace` (grace 2 s, `LOFT_TIMEOUT_GRACE`) — a background
  thread calls `std::process::abort()`, so the run ends even when stuck in Rust, native code or
  a blocking syscall (on the interpreter, only those: a loft loop stops at `T`).  It prints a breadcrumb so the kill is still informative:

  ```
  [timeout] hard-kill after 300s+2s grace: phase=run-interpret fn=helper952 \
            file=tests/14-workflow.loft:31 entry=test_the_stuck_one
  ```

  `fn`/`file` name the loft function the run was in, per call on both backends; `entry` names
  the entry point it was reached from, which under `--tests` is the TEST — the field that says
  which of the swept-in files is responsible.  `phase=parse` means a hung compile.

| Mechanism | Default | How |
|---|---|---|
| Explicit CLI flag | (off) | `loft --timeout <secs> <program.loft>` |
| Env var | (off) | `LOFT_TIMEOUT=<secs> loft <program.loft>` |
| Auto-arming under `--tests` / `loft test` | **on, 300s** | `loft --tests <file>` arms 300s unless explicitly overridden |

⚠ **`--tests` takes its path as the next NON-FLAG argument, and it only steps over
`--native`, `--no-warnings` and `--deny-warnings` on the way.**  `--interpret` is not in that
list, so `loft --tests --interpret <file>` does not run `<file>` — the path stays at its default
`.` and the WHOLE TREE runs, `target/` included, for many minutes.  Put the backend flag first
(`loft --interpret --tests <file>`) or leave it off.  The tell is a single-file run that does
not finish; the parse site is the `--tests` arm in `src/main.rs`.

Implementation lives in `src/timeout.rs` (watchdog thread, deadline atomics, breadcrumb store),
with checkpoint calls in `src/state/mod.rs` (interpreter dispatch), `src/codegen_runtime.rs`
(`cr_check_deadline()` injected at native fn-entry and loop back-edges) and `src/lexer.rs`
(parse-time recovery loop).

**Under cargo test** a hang inside an in-process harness (`tests/wrap.rs`) is bounded by
nextest instead: `.config/nextest.toml` gives every test process a `slow-timeout` (300 s
default, 600 s in the `ci` profile, `terminate-after = 1`), after which nextest kills that one
process and the rest of the suite continues.

**The limit is a hang guard, and a healthy test stays under HALF of it.**
`scripts/test_duration_gate.py` runs right after CI's Test step and fails the leg when any
test's slowest attempt (a retried test is judged on its first try) passed half the `ci`
profile's `slow-timeout`.  A test over that line is made cheaper — chunked (`native_scripts_NN`,
`loft_suite_NN`, the drop gate's native chunks), parallelised, or given a deadline on every
wait — and the limit is never raised: a raised limit only postpones the same kill while every
run gets slower.

---

## Debug boundary checks (debug builds only)

Three checks catch the commonest runtime bug patterns at the first bad access, before the
corruption spreads:

| Check | File | Catches | Builds |
|---|---|---|---|
| `store_nr < allocations.len()` | `src/database/allocation.rs` `Stores::store()` | a DbRef pointing at a store that does not exist | every build (a slice-index panic) |
| `fld + size ≤ record_size` | `src/store.rs` `addr()` / `addr_mut()` | a field access past the end of a claimed record (e.g. a wrong `pos` in a returned DbRef) | debug assertions on |
| `stack.pos ≥ size_of::<T>()` | `src/database/mod.rs` `get<T>()` | stack underflow from popping more bytes than were pushed (e.g. a wrong native-function argument order) | debug assertions on |

⚠ The last two — and every other lib-side `debug_assert!` / `#[cfg(debug_assertions)]` check
(the two-pass contract, `Store::valid` / `validate`, codegen sanity asserts, the `[set_var]`
width warnings) — are **silent during ordinary `cargo test` runs on every platform**, in both
dev and `--release`, because of the `Cargo.toml` line in § [Hang guard](#hang-guard-loft_max_ops).
Only the cargo-fuzz target (which forces `-Cdebug-assertions`) and an explicit calibration run
check them — [DEBUG_STORES.md § The debug-assertions calibration run](DEBUG_STORES.md#the-debug-assertions-calibration-run-target-da).
A green suite is therefore no evidence for an invariant only these checks assert.  Latent
out-of-bounds writes inside `Store` are also absorbed by Linux's allocator slack (16-byte chunk
minimum) and caught by Windows as `STATUS_HEAP_CORRUPTION (0xc0000374)` at deallocation;
[TEST_ENVIRONMENTS.md § Occasional valgrind pass](TEST_ENVIRONMENTS.md#occasional-valgrind-pass-linux)
surfaces them on Linux.
