<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# A loft interpreter built and run without Rust — the design

The objective of [README.md](README.md) is an interpreter written in loft with no Rust code
left.  [rust-dependencies.md](rust-dependencies.md) is the register of what the bytecode runner
still takes from Rust and the order those dependencies end in.  This page is the design that
register serves: what "without Rust" has to mean for a build, the three levels it is reached
in, what each level needs, what is measured today, the decisions each level raises, and how
each is checked.  Everything below that says "measured" was measured on 2026-11-15 on branch
`tuxedo-pln204-int32`.

## Contents
- [Three levels of "without Rust"](#three-levels-of-without-rust)
- [Where it stands](#where-it-stands)
- [Level 1 — a build that needs only a C compiler](#level-1--a-build-that-needs-only-a-c-compiler)
- [Level 2 — source in, run, with no Rust in the binary's path](#level-2--source-in-run-with-no-rust-in-the-binarys-path)
- [Level 3 — loft rebuilds itself](#level-3--loft-rebuilds-itself)
- [What crosses every level](#what-crosses-every-level)
- [The order, and what can run in parallel](#the-order-and-what-can-run-in-parallel)
- [Decisions to take](#decisions-to-take)
- [How each level is checked](#how-each-level-is-checked)

## Three levels of "without Rust"

"Built without Rust" names three different guarantees.  They are reached in order, and each is
useful on its own:

| Level | The guarantee | Who it serves | What still uses Rust |
|---|---|---|---|
| **L1 — build** | The interpreter binary is built from C sources with a C compiler alone: no `cargo`, no `rustc`, no `loft` binary on the build machine. | anyone installing loft, a packager, a board without a Rust toolchain | writing the C sources (a developer's step), writing the program's run image |
| **L2 — run** | That binary takes a `.loft` source and runs it: the front end, the bytecode writer and every native it reaches are in the binary, in loft or in the C runtime. | every user running a program | regenerating the C when loft's own sources change |
| **L3 — self-build** | loft's own sources are compiled to that C by loft: a change to the language is made and built with no Rust anywhere, and stage 2 of the bootstrap equals stage 1. | the language's developers | nothing — Rust is retired (README § The path, stage 6) |

The levels are separate because their costs are of different sizes.  L1 is a packaging step
over what exists.  L2 is the front end — the long pole.  L3 is the C generator, the second
long pole.  Holding them apart lets L1 ship long before the front end is done, and makes it
plain at every point which guarantee a build gives.

## Where it stands

| Part | State | Measured by |
|---|---|---|
| the runner (all 366 operators, the natives a corpus program reaches) | **3234 of 3303** corpus programs and test functions agree with `loft --interpret` — 1195 plain programs, 2108 `fn test_*` functions run one per program; 12 fault, 25 stop on a feature not carried (lambda generators, a native called through a function value, reference rendering) | `selfhost/bench/run_image_survey.py --functions` |
| the runner's speed | 0.86 of `loft --interpret`'s time (geometric mean, 94 routines, one core); `par` on one thread | `selfhost/bench/bench_speed.py` |
| the engine-host kernel on the runner | a game server and its clients over sockets, identical to the interpreter | [1b-fill-port.md](1b-fill-port.md); `lib/engine_host/tests/local_kernel.loft` |
| the bytecode writer in loft | 8506 of 8611 functions byte-identical (the last census; 104 not written, 1 differs) | `selfhost/bench/bgen_survey.py` |
| the run image's tables in loft | code, starts, natives, operators, layouts, shared bodies, generators, function values; open: constants, `par` workers, sidecars, the schema, operator numbers, debug data | `bgen_check --stream` ([rust-dependencies.md](rust-dependencies.md) step 2) |
| the lexer and parser in loft | tokens equal on 1845 of 1850 files, syntax on 1836 of 1843 | [1a-parser.md § Where it stands](1a-parser.md#where-it-stands) gates T, S |
| the elaborator in loft | IR equal on 718 of 1825 programs; 60 of 379 formal rules carried for every cell | gate E |
| the scope pass in loft | a first slice (`scope.loft`, 867 lines, against `src/scopes/`'s 39 k) | — |
| the IR passes `close_program` runs | not ported | — |
| the C generator in loft | not started | — |

The sizes the remaining ports face, in lines of Rust: the parser 130 k, the scope pass 39 k,
the shared decisions `src/opt/` 33 k, the C writer `src/c/*.rs` 34 k, the lowering `src/lower/`
8 k, the bytecode writer `src/state/codegen*.rs` 7.5 k, `src/data.rs` 16 k.  The loft side
written so far: `selfhost/src/` 71 k lines (the elaborator 35 k, the bytecode writer 4.8 k).
The C runtime (`src/c/*.h`, 36 k lines) is C already and is not ported.

## Level 1 — a build that needs only a C compiler

### The seed

The runner is a loft program the C backend compiles (`loft --native-c --ship`).  What reaches
the C compiler is three things:

| File | Made by | Size (measured) |
|---|---|---|
| the runner's C unit, the C runtime inlined | `loft --native-c --ship --native-emit` over `selfhost/bench/run_image_run.loft` | 26 MB, 217 k lines |
| the crt shim (`selfhost/crt/shim/crt.c`) | hand-written C, includes the runtime headers | 3.1 k lines + `src/c/*.h` |
| the compiled standard-library bodies the shim includes (`loft_bodies.c`) | `loft --emit-c-bodies` | 161 kB |

Captured once, these are a **seed**: C that builds the interpreter on any machine with a C
compiler.  **Probed (the design's load-bearing premise):** the three files and `src/c/*.h` were
copied to an empty directory and built under `env -i PATH=/usr/bin:/bin` — no `loft`, nothing
calling `cargo` — with two commands:

```sh
cc -std=c99 -w -O2 -fPIC -I gen -c selfhost/crt/shim/crt.c -o crt.o
cc -std=c99 -w -pthread -O2 -DLOFT_BIND_LIBS gen/run_image_run.c crt.o -o run_image_run -ldl -rdynamic -lm
```

58 s on one core.  On a sample of 35 corpus programs the seed-built runner's output and exit
code were byte-identical to the loft-built runner's (stderr included), and stdout and exit code
equal to `loft --interpret`'s on all 35.  L1 is therefore packaging, not porting.

### What L1 has to add

1. **A seed target.**  `make seed` writes the three files and a `seed/build.sh` (the two
   commands, the platform's flags: `-ldl` and `-rdynamic` on POSIX, the Windows CRT's on
   Windows, LTO and `-O3` as the shipped tier — [NATIVE.md § Optimisation
   tiers](../../NATIVE.md#optimisation-tiers--semantics-runs-performance-lanes-shipped-binaries)).
   The absolute paths the unit carries in its `/* link: */` comments are replaced by the seed's
   relative layout.
2. **A seed check.**  A gate that regenerates the seed and compares it with the published one:
   the seed is derived, so it may never be edited by hand, and a change to the runner's
   sources that is not reflected in the seed is a red build.
3. **Where the seed lives.**  26 MB of generated C changes on nearly every runner commit.
   Committed to the repository it would dominate the history.  Recommended: a release artifact
   (a tarball beside the binaries, its hash in the release record), regenerated and checked by
   the release gate; [Decisions to take](#decisions-to-take) 1.
4. **A stable image format.**  At L1 the runner still runs images the Rust front end writes
   (`loft --emit-run-image`): a seed-built runner and a newer `loft` must agree on the format.
   The image carries a schema (`selfhost/src/ir.loft` `RunImage`, `make ir-schema-regen`); it
   needs a version word the runner checks, refusing an image of another version by name rather
   than misreading it.

### What L1 gives

A binary built with a C compiler alone that runs every program a run image can describe —
a player for compiled loft programs.  A game, a tool or a service ships as `run_image_run` plus
its `.run` image, on any machine with a C compiler, and the engine-host kernel already runs
on it.  The source-to-image step still needs the Rust `loft`.

## Level 2 — source in, run, with no Rust in the binary's path

L2 puts the front end, the bytecode writer and the image's tables into the same binary, all
loft compiled through the seed.  Its stages are [rust-dependencies.md](rust-dependencies.md)
steps 1–5; the design of each, and what L2 adds beyond them:

### The front end (the long pole)

| Stage | Rust it ports | State | What finishes it |
|---|---|---|---|
| lexer, parser | `src/lexer.rs`, `src/parser/` (syntax) | gates T and S near 100 % | the `\0` literal rows, the refusal cells |
| elaborator (names, types, calls, dispatch) | `src/parser/` (semantics), `src/typedef.rs`, `src/data.rs` | gate E 718 / 1825 | the families `elab_gate.py --plan` ranks: dispatch, tuples, closures, calls, interfaces ([1a-parser.md](1a-parser.md)) |
| scope pass (lifetimes, frees, witnesses) | `src/scopes/` (39 k) | first slice | the whole pass, gated by the image before and after it |
| slot assignment, the variable facts | `src/variables/` | partial (`frame.loft`) | each variable's slot and flags (`stack_allocated`, `skip_free`, `lazy_buffer`, `view_elided`, `linked_narrow`, witnesses) — [rust-dependencies.md](rust-dependencies.md) item 14 |
| the IR passes `close_program` runs | `src/compile.rs` (`ledger_sites`, `arg_copy`, `value_record`, `leaf_inline`, `tail_self`, … `text_len`, `loop_buffer`) and the parts of `src/opt/` they call | not ported | each pass's admissions diffed against `make rewrite-census` before its IR is |
| bytecode writer | `src/state/codegen.rs` | 99 % of functions | the 104 functions and the text-view rewrite ([handoff-2026-11-14-bytecode-writer.md](handoff-2026-11-14-bytecode-writer.md)) |
| the image's tables, in memory | `src/ir_store.rs` | about half | constants, workers, sidecars, schema, operator numbers, debug data |

The rule for all of it is the plan's: a faithful port, the IR and the bytes identical to Rust's,
a difference a defect.  That rule is what makes the oracle exact; it also means **the target
moves**: every change to the Rust compiler while the port is open is a change the port owes.
[Decisions to take](#decisions-to-take) 3.

### The standard library at start-up

The Rust `loft` parses `default/*.loft` on every run and keeps a cache of the result
(`LOFT_STDLIB_CACHE`).  The L2 binary parses it with the loft front end — slower than Rust's,
measured on no machine yet — so the stdlib's elaborated IR and its images should be built into
the binary as constant data (a store image, as `selfhost/src/ir.loft`'s codec writes) and the
front end started from it, as the Rust interpreter starts from its cache.  The compiled
standard-library bodies (`loft_bodies.c`) stay as they are: C in the seed.

### Natives

Everything a program calls by native number must be in loft or in the C runtime:

| Group | State | L2 work |
|---|---|---|
| the stdlib natives a corpus program reaches | carried (`natives.loft`) | — |
| reflection, `stack_trace`, process and environment, the engine-host kernel | carried | `stack_trace`'s variables (the bytecode writer's tables) |
| a native called through a function value; lambda generators | stop the runner (25 corpus tests) | carry them |
| `process_run`, `git_query`, lazy stores, `store_load_url`, live rebuild and swap, host input and output | not carried | the C runtime has most (`loft_process.h`, `loft_lazy.h`, `loft_engine.h`'s swap): marshalled as the kernel's are |
| the 12 faults the survey names | runner defects | fixed one by one, each with its cell |

### Libraries

A library is loft source plus, for some, C (`[c]`, a shim of the package's own) — on
`origin/main` `crypto`, `random`, `regex`, `graphics`, `imaging`, `audio`, `input`, `screen`;
`web`, `server`, `ssh` reach the `net` component, which is C (README strand 10).  At L2:

1. **A library's `[c]` shim must load into the runner.**  The runner calls a program's own `#c`
   functions by symbol (`c_call`, `dlsym`); a library's shim is a separate object.  The design:
   the L2 binary builds a package's shim with the C compiler the way `c_shim.rs` does today
   (spawning `cc` through the C runtime's process routines) and opens it (`dlopen`; `LoadLibrary`
   on Windows), then binds the package's `#c` declarations to it by symbol.  This is the C
   driver's job moved into loft, and the first piece of L3's driver.
2. **No library code compiled for an interpreted script** (1b-fill-port.md difference 9).  A
   `use`d library runs as bytecode on the runner.  This is a speed difference only: the
   compiled form comes with L3's generator.  The library-test harness
   (`selfhost/bench/run_image_libtests.py`) compares the runner, libraries as bytecode, with the
   interpreter, libraries compiled — which found a silent wrong answer in the compiled path
   (18ae180b3).
3. **What remains Rust in the libraries** is strand 10's list: `plugin_runtime`'s crates, the
   `bench.rs` twins, the library CI's cargo build.  None is on the interpreter's path.

### `par`

Results are right; the workers run on one thread.  The threaded design is
[1b-par-threads.md](1b-par-threads.md).  It is not on L2's path — a correctness level — but
it is on the path to the interpreter being as fast as the Rust one.

### What L2 gives

`loft prog.loft` with no Rust anywhere at run time: the seed builds the binary, the binary
reads source.  Developing loft itself still regenerates the seed with the Rust toolchain.

## Level 3 — loft rebuilds itself

L3 is strand 12 of [README.md](README.md): the C generator written in loft, so the seed is
regenerated by the seed-built binary.

| Part | Rust it ports | Gate |
|---|---|---|
| the shared decisions (`hoist`, `range`, `writes`, `push_window`, `store_text_borrow`, `apart`, `value_records`, `tail_self`, `ret_adopt`, `non_sentinel`, …) | `src/opt/` (33 k) | each pass's admissions and declines equal to `make rewrite-census` / `make decline-census` and its `LOFT_TRACE_*` lines, before its C is compared |
| the C writer | `src/c/*.rs`, `src/c/body/*.rs` (34 k) | the `--emit-c` unit byte-identical per program, counted over the corpus |
| the compiled bodies fragment | `src/c/bodies.rs` | `loft_bodies.c` byte-identical |
| the driver: shim builds, the link, the C compiler's flags per tier | `src/c_driver.rs`, `src/c_shim.rs` | the binary the driver builds behaves as the Rust-driven one (the seed check of L1) |

**The bootstrap:** stage 1 is the loft-written compiler compiled to C by the Rust generator
(today's seed, with the C generator in it); stage 2 is the same sources compiled by stage 1.
The gate is that stage 2's C for itself equals stage 1's.  From then on the seed is
regenerated by loft, and Rust can be removed (README § The path, stage 6).

## What crosses every level

- **The oracle.**  Every gate compares with the Rust implementation: `loft --interpret` for
  answers, `--emit-image` / `--emit-run-image` for IR and bytes, `--emit-c` for C.  The oracle
  stays until the gates have held over the corpus; after Rust is retired the comparison is
  against recorded answers — the language suite's expectations, `tests/scripts`' outputs
  pinned (strand 11).  Retiring the oracle before the answers are recorded loses the ability
  to tell a regression from a change.
- **The C runtime is shared and stays C.**  The interpreter and every compiled program run on
  `src/c/*.h`.  A defect there is a defect of both; the runner's survey and the C backend's
  sweep (`scripts/c_sweep.py`) both police it.
- **Memory and time of the loft front end.**  The compiled elaborator has driven one corpus
  program to 11 GB ([1a-parser.md](1a-parser.md)).  The front end's peak memory and its time
  per program are measured beside its correctness gate from the first L2 build, or L2 ships
  an interpreter that cannot compile the programs it is meant to run.
- **Portability.**  The seed is C99 with the runtime's platform switches (`LOFT_POSIX_HOST`,
  Windows, WASI, `LOFT_STRICT`).  L1's build script carries the platform's link flags; the
  shim load of L2 needs `dlopen` or its Windows twin, absent under `LOFT_STRICT` and WASI,
  where a library with C cannot be used (the same limit the C backend has).

## The order, and what can run in parallel

```
L1 seed target + check + image version ──► (ships: a Rust-free player)
                                            │
L2: lexer/parser ─► elaborator ─► scope pass ─► slots ─► IR passes ─┐
     bytecode writer (104 left) ─► image tables in memory ──────────┼─► source in, run
     natives (lambda generators, fn-value natives, process, lazy)   │
     library [c] shims loaded by the runner ────────────────────────┘
L3: src/opt/ ─► C writer ─► bodies ─► driver ─► bootstrap fixpoint ─► Rust retired
```

- L1 first: it is days, not months, and it changes what can be shipped.
- In L2 the front end is the critical path; the bytecode writer, the image tables, the natives
  and the library shims are independent of it and of each other.
- L3's decisions (`src/opt/`) are shared with L2's IR passes: a decision ported once serves the
  bytecode (L2) and the C (L3).  Port each where L2 needs it first.

## Decisions to take

1. **Where the seed lives** — committed C (simple, heavy history), a release artifact
   (recommended: the history stays small, the release gate checks it), or a separate repository.
2. **The image format's versioning at L1** — a version word and a refusal, or the runner and
   `loft` released only together.  Recommended: the version word; it is one field and it makes
   the L1 player independent of the compiler's release.
3. **The moving target** — while the front end is being ported, every Rust compiler change is
   owed twice.  Options: freeze the Rust front end's semantics for the port's duration (only
   fixes, each mirrored), or let it move and count the debt per change.  Recommended: a change
   to the Rust front end names its loft twin in the same commit once the loft stage it touches
   passes its gate on that construct — a change to a stage not yet ported costs nothing.
4. **The stdlib at start-up** — parsed per run, or built into the binary as constant data.
   Recommended: built in, measured against parsing before it is decided.
5. **When the oracle is retired** — after which gates hold, over which corpus, and how the
   answers are recorded.

## How each level is checked

| Level | Check | Command |
|---|---|---|
| L1 | the seed builds with a C compiler alone | `env -i PATH=/usr/bin:/bin sh seed/build.sh` |
| L1 | the seed-built runner equals the loft-built one | the survey with `--runner` pointing at the seed build |
| L1 | the published seed equals a fresh one | the seed check, in the release gate |
| L2 | each front-end stage's IR equals Rust's | gates T, S, E ([1a-parser.md](1a-parser.md)); `selfhost/compare/` |
| L2 | the bytes and tables equal | `bgen_survey.py`, `bgen_check --stream` |
| L2 | source in, run, over the corpus | the survey started from sources (rust-dependencies.md step 4) |
| L2 | the libraries | `run_image_libtests.py` over `origin/main` worktrees of the `loft-libs-*` repositories — never a local checkout, which can lag (the 2026-11-15 run read checkouts 16 and 60 commits behind) |
| L3 | the decisions, then the C | `make rewrite-census` / `make decline-census` traces; the `--emit-c` unit per program |
| L3 | the bootstrap | stage 2's C for itself equals stage 1's |
