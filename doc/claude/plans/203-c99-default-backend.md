// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN203 — the C99 backend as the only compiled target

The plan is [loft-lang/plans#203](https://github.com/loft-lang/plans/issues/203): its phases,
status and dependencies live there.  This page holds what the issue does not: the measured
position the plan starts from, the shape of each phase's work in this tree, and the decisions
each phase has to take.  The backend itself is [C_BACKEND.md](../C_BACKEND.md); the structure
it was built on is [@PLN200](200-backend-structure.md) B5.

## The position (measured 2026-10-26)

| what | result |
|---|---|
| corpus sweep, 2832 programs, strict / default / verify | 2469 match, 0 differ, 0 cc-error |
| the same under AddressSanitizer + UBSan, and under `LOFT_POISON` / `LOFT_POISON_CLAIM` | 0 faults |
| Linux x86-64, gcc 13, glibc | 2510 match, 0 differ, 0 cc-error |
| Windows, MinGW gcc 15, 74 programs | 74 match |
| the 22 corpus programs needing `--lib`, with their `@ARGS` | 22 match |
| the fixture libraries' own tests (pure loft) | match |
| libraries without a Rust crate | random 0.4.0, regex 0.6.0, crypto 0.4.0, imaging 0.5.0 |

The 363 the sweep refuses are the 341 expected-error programs and the 22 `--lib` programs the
sweep cannot pass arguments to.  What the sweep cannot see, found by hand in the evaluation:

- a `#native` function of a Rust-crate library is emitted as the unloaded stub — the reason
  the libraries are leaving their crates (`[c] shim`, @PLN24 arc D and E);
- `eprint` / `eprintln`, `is_symlink`, `store_upgrade_owed_*` are refused by name;
- a fault prints its message without the `-->` line and the call chain;
- the C binary reads neither `LOFT_TIMEOUT` nor `LOFT_MEMORY_LIMIT`;
- `par` runs its rows in order;
- nothing drives the C: `--emit-c` writes a file, `cc` is the user's.

## The phases, in this tree

**C1 — the driver.**  `src/main.rs`'s `--native` arm (the cache, the run, the `--out` copy)
gains a C branch beside the rustc one; the key is the C text, `c_shim::cc_identity`, and the
shim archives (`c_shim::archive`) and `[c] libs` the unit names in `/* link: */` lines.  The
run bounds move into the C runtime: `LOFT_TIMEOUT` as `timeout.rs`'s watchdog (a thread under
POSIX, `alarm` without), `LOFT_MEMORY_LIMIT` as `Stores`' ceiling with the type-naming report,
and the poison modes as a run-time switch read once at start instead of `-D` macros, so one
binary serves `c_sweep.py --poison`.  The logger (`logger.rs`'s `log.conf` resolution) and
`LOFT_REQUIRE_NATIVE`'s refusal to fall back follow.

**C2 — diagnostics parity.**  `loft_frame_site` already records the call line; the fault
printer in `loft_rt.h` grows the interpreter's three parts (`diagnostic_render.rs`: the
`error:` line, the `--> file:line:col` excerpt, the frame chain) and the recoverable faults
(`runtime_error.rs`' kinds and exit codes).  The sweep gains `--stderr`, comparing the two
streams after the interpreter-only lines are dropped.

**C3 — the stdlib census.**  A script over `default/*.loft` pairs each `#rust` template with
its arm in `src/c/body*.rs` (the operator names are computed, so the pairing is by the
`n_`/`t_` key, as `make ops-census` pairs operators) and lists the templates with neither an
arm nor a refusal row; a guard in `tests/c_emit.rs` holds the list at zero.

**C4 — `par` with threads.**  `body/par.rs` queues a worker per row today; the runtime gains a
pool (`loft_cc.h` chooses C11 `<threads.h>`, pthreads, or the sequential form under
`-DLOFT_NO_POSIX`), per-worker stores as `parallel.rs` mints them, and the process-wide tables
— the generator table, the frame stack, the open-file table, the `#c` text buffers — become
per-thread.  THREADING.md's guards are the acceptance, with the `par` bench rows.

*C4's design.*  The runtime's process-wide state (33 variables and the function-local caches —
the store table, the frame stack, the generator and evacuation tables, the parse and JSON error
lists, the par buffers, the heap counter) becomes per-thread under one macro, `LOFT_TLS`
(`_Thread_local`, GNU `__thread`, MSVC `__declspec(thread)`), and empty where the build has no
threads (`-DLOFT_STRICT`, `-DLOFT_NO_POSIX`, WASI), which keeps the sequential form.  What
stays shared is the program's: the open files, the lazy bindings, the arguments, the logger.
A worker starts from a COPY of its parent's store table — every entry live and pinned, so it
reads the parent's stores through the same buffers and never frees, grows or reuses one
(@FR-C-Impure: a worker that writes shared state has no defined behaviour) — and makes its own
stores past them, freed when it ends.  Tier 1 runs on threads the workers whose result is a
scalar, a text or nothing, each writing its own rows of the parent's result buffer; a worker
returning a record or a function value stays sequential until tier 2 adopts the worker's stores
into the parent and translates its references (`StoreRebase`, as `parallel.rs` does).  The rows
split into `min(threads, rows)` contiguous slices, as `par_pool::parallel_workers` splits them.  Tier 2
is the performance stream's (157-native-4x, owner 2026-10-26).

**C5 — libraries and hosts.**  The decision: a library compiled whole into each program (the
C does this today; the cost is compile time per program, bounded by the cache) or a C archive
per library (`c_shim::archive`'s shape over the library's own loft, needing a stable C ABI for
loft functions, which `@PLN199`'s cdylib layer defined for Rust).  Measure compile time on the
largest consumer before choosing.  `engine_host` and placement (PLACEMENT.md) become C worker
processes over the same protocol.  The remaining Rust-crate libraries: `graphics` over the
OS's windowing, GL and audio through `[c] libs`; `web` and `server` over BearSSL's TLS engine
(vendored like its primitives, with a minimal trust store) or the OS's; `ssh` parked.  The link
modes of @PLN195 have their C form on POSIX hosts (`c_driver::Placement`): a program written
out opens its packages' libraries at start through loft's search order, `--static` keeps a shim
inside and `--dynamic` beside, and a missing library stops the program naming the package and
where it searched (`native::a_shipped_c_library_travels_with_the_program`); Windows still links
them beside the binary, which C8 takes.  The two link-mode tests over `native_scalar_pkg` are
`[native] crate` tests and move to `--native-rust` with C9.  The `in_binary` libraries (natives inside
loft's own binary): the C runtime carries `lib/process`'s and `lib/git`'s
(`loft_process.h`, POSIX hosts); an in-binary native it does not carry refuses the unit by
name.  `engine_host`'s kernel is carried (`loft_engine.h`: listener, connector, UDP sync, debug
channel, `http_fetch`); its build swap and background rebuild are not — the swap needs the
world written and read through the store converter (@PLN196) for a type known only at run time.
HTTPS is BearSSL's engine vendored as crypto's primitives are (owner, 2026-10-27), built once
into a cached archive (C_BACKEND.md § HTTPS), and every fetch — whole-image, paged by range,
`http_fetch` — goes through the one HTTP client over it; C8 adds the Windows certificate store, which the
system-bundle probe does not read.  The live tier (flips, live edits, a breakpoint's eval) embeds the interpreter in a
compiled program, which the C does not: after C9 the edit loop runs interpreted, and the
`engine_host_*` scenarios that drive a live edit keep an interpreted leg.

**C6 — the wasm and browser targets.**  The runtime under `-DLOFT_NO_POSIX` is the start: the
WASI libc gives files for `--native-wasm`; the browser needs the host imports the Rust writer
declares (`emit_file_header`'s `loft_gl` block), the `loft_start` export, the html shell and
the threaded page (`html_embed.rs`, `html_fonts.rs`).  `c_shim::wasm_objects` and
`src/c/wasm/` already carry a package's C into both targets.

**C7 — performance.**  `scripts/native_ratio.sh` and `bench/stats.py` gain a C lane
(`--emit-c` + `cc -O3 -DLOFT_SHIP_PROBE` once the ledger exists), the perf portal a column;
the `FnPlan` families of @PLN200 B3.4 move as C's rows ask.  The bar is the Rust writer's: a
routine whose `cship` time is over 3.5× its Rust twin is a bug, one issue per problem class
(bench/README.md § When a gap is a bug), from the moment the initial store port has landed;
the work before that is bench/portal/analysis/c-store-gaps.md.

**C8 — platforms and CI.**  MSVC: the macro layer's `_MSC_VER` arms built and swept on the
Windows runner; the C sweep as a job in `ci.yml` beside the native leg, ASan and poison in the
nightly; `scripts/test_subjects.sh` gains `c`.

**C10 — what a user installs.**  The C stack moves the toolchain a user needs from rustup to the
system's C tools, and the pages a user reads must say so before C9 ships: `README.md`'s install
section, `doc/install.html`'s source, and the messages the driver prints when a tool is missing
(`c_driver::build`'s "needs a C compiler", the wasm route's clang / `wasm-ld` / sysroot refusals)
each name the package to install on the reader's OS.  The full set on Debian/Ubuntu (owner,
2026-10-26): `apt install clang lld wasi-libc libclang-rt-dev-wasm32` beside the host `cc`
(`build-essential`) — clang for wasm32, `lld` for `wasm-ld`, `wasi-libc` the `--native-wasm`
sysroot, `libclang-rt-dev-wasm32` its compiler-rt builtins.  The macOS (Homebrew `llvm`, `lld`,
`wasi-libc`) and Windows (MinGW-w64 gcc, or MSVC; LLVM for wasm) lists are measured on those
runners before they are written down, and what each piece buys is stated per target (host
binary: `cc` only; `--native-wasm`: all four; `--html`: clang and `lld`).

**C9 — the flip.**  `--native` is the C backend; `--native-rust` the Rust writer for one
release (the tests of the `[native] crate` mechanism itself — a package cdylib linked by C-ABI,
a remapped symbol, a crate linked statically — run there and leave with it: the C does not
bridge a crate, it waits for the library's own C); then `src/generation/`, `native_adopt.rs`, `compiled_stdlib*.rs`, `native_build.rs`,
the `#rust` templates and NATIVE.md's rustc content go to history.

## Decisions this plan takes

- The memory model stays loft's ([@PLN200 B5](200-backend-structure.md), owner): the C
  runtime ports `store.rs` and the keyed layouts, never its own.
- A library's native part is ANSI C the package ships (`[c] shim`), vendored and audited where
  one exists (crypto's BearSSL and Monocypher), written by hand only where none fits (the
  regex engine), and in loft wherever loft can express it — the C backend then writes the C.
- Compilers: `cc` for the host, `clang` for wasm32 (`LOFT_WASM_CC`); gcc has no wasm backend
  and the small C compilers have none, so there is no third choice to design for.
