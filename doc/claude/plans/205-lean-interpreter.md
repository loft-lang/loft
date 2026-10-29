<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 205 — A lean interpreter: features as prebuilt parts loaded on demand

[@PLN205](https://github.com/loft-lang/plans/issues/205).  Builds on
[@PLN200](https://github.com/loft-lang/plans/issues/200) (the layers, its B4 crates and B6
minimal binary — see [200-backend-structure.md](200-backend-structure.md)) and
[@PLN191](https://github.com/loft-lang/plans/issues/191) (`loft-runtime`); meets
[@PLN203](203-c99-default-backend.md) at the C backend.

## Status

Active.  The owner took the four recommendations under [Decisions](#decisions) (2026-10-27).
The `server` port that was on `firewall02-scripting` (Rust natives inside the binary) is
stopped: its C half and its guard move into L1, its Rust half is dropped; the work is kept at
the local ref `refs/keep/firewall02-server-in-binary-wip`.

**L0a — answered yes.**  A library whose native part is C behind `loft-ffi/include/loft_ffi.h`,
shipped prebuilt in `prebuilt/<triple>/`, runs on all three backends with nothing compiled to
use it: the interpreter loads it as it loads a Rust cdylib (the bridge ABI is one C signature),
the Rust writer links it by its typed symbols, and the C backend now resolves each symbol on
the first call (`library_native` in `src/c/mod.rs`, `src/c/loft_ffi_host.h`).  Guard:
`tests/ffi_c_library.rs` (binary text both ways, integers, a boolean, a float, a vector).
Its follow-ups, done: a program written out with `--out` binds the library by loft's search
order, beside it, rather than by the path it was built at (`loft_cboundlibs`,
`loft_ffi_sym`); a vector or record answer converts on all three backends (a record answer
read as garbage on the two that took it, loft#2224); `loft_ffi.h` carries the store number
and the answer builders the Rust crate has (`loft_ffi_store_nr`, `loft_ffi_alloc_record`,
`loft_ffi_vector_from_bytes`); and a missing or stale prebuilt library is refused naming it,
with no `cargo` advice (`native_build::unbuildable_native_reason`).  Guards in
`tests/ffi_c_library.rs`.  Still open: the manifest's `[native]` line that names no crate to
mark C-ABI natives wants a name of its own (`[native] prebuilt = true` or similar).

**L1.1 — the component loader.**  `[native] component = "<name>"` names a library loft ships
(`src/components.rs`: `LOFT_COMPONENTS`, `<binary>/../lib/loft/components/`,
`<binary>/../components/`, then `components/` in each directory a compiled program searches);
the manifest reads it as the native library `loft_<name>` with the C-ABI link, so the three
backends resolve it through `native_build::prebuilt_native_lib` as they resolve a package's own
`prebuilt/<triple>/`, and a missing one is never built — each backend refuses naming the
component and where it looked.  Guard: `tests/ffi_c_library.rs`
(`a_component_answers_on_every_backend`, `a_missing_component_is_refused_by_name`).

**L1.2 — `libloft_net`.**  `src/c/components/net.c` over the runtime's `loft_web.h`,
`loft_https.h` and the TLS archive, built by the `loft-components` tool (`src/c/component.rs`;
`make components` → `target/components/`, `make install` → `<prefix>/lib/loft/components/`):
a 215 KB library exporting the 22 `n_*` symbols and the two registrations, nothing else,
rebuilt only when its sources, the compiler or the loft-ffi ABI change.  The in-binary path is
gone: `src/web_client.rs`, the interpreter's registration, the Rust runtime's typed twins and
the C writer's `loft_web_*` map.  A browser page keeps its own (`web_browser.rs`, the
`runtime_abi::BROWSER_RUNTIME_FNS` rows the writer reads only for that target).  Guard:
`tests/web_natives.rs` (fixture `[native] component = "net"`), which builds the component with
`loft-components` first, so CI needs no step of its own.  Not on Windows: the C runtime's
host layer there is @PLN203 C8's, so the web library stays on its Rust crate (web 0.4.5) until
the component builds there — L1.4 waits for it.

**L1.3 — the server's natives in `libloft_net`.**  `src/c/loft_server.h`: the HTTP listener, the
request parser, the responses, the WebSocket server and the event pump, in C over the runtime's
`loft_stream`; a TLS listener over `tls/loft_tls.c`'s server session (BearSSL's server files, an
EC or RSA certificate, a key that does not match refused).  The 32 `n_tcp_*` / `n_ws_*`
exports join the web's 22.  The frame codec is one (`loft_ws_write_frame` /
`loft_ws_read_frame` in `loft_https.h`), shared by the web client and the server.  The parked
Rust port never landed, so nothing in the binary is removed.  Guard: `tests/server_natives.rs`
(fixture `[native] component = "net"`): a script of requests, a binary body, caller headers,
WebSockets, the event pump, and TLS over both certificate kinds, the same on all three
backends.

**L0b — answered yes.**  `LOFT_EMIT_FROM_IMAGE=1` makes `--emit-c` write from the image
(`ir_read::image_round_trip`); `scripts/image_probe.py` compares that with the C written from
memory over every program.  Over all 2893 programs (CI, linux-probe run 40595296631): 2525
byte-identical, 368 refused alike, none differing.  The first sample's 12 differences were all
in constants, and both causes are fixed:
- the image carries no built constants (the compile pass built them, in the interpreter's
  layer).  The builder moved to `const_literal::build_const_vectors`, which the image reader
  now calls too;
- the C written from memory was not reproducible either: the C backend copies a constant's
  store words into the unit, and the store came from the free pool with stale bytes in it.
  A constant store is now allocated fully defined (`Stores::database_defined`).

## Goal

The owner's rule (2026-10-27): the interpreter **does not compile anything on the fly** and **does
not carry routines a program may never use**.  So the `loft` binary holds the language — lexer,
parser, scopes, IR, optimiser, stores, interpreter, standard library — and the ways of running a
program in it (`run`, `test`, `debug`, `repl`).  Everything else is a PREBUILT part:

- built ahead of time by loft's release CI (or a library's own CI), for each platform;
- loaded only when a program or a command first needs it;
- refused BY NAME when it is missing for the platform — never compiled as a fallback.

## Where the size is (measured 2026-10-27)

The release binary on this box is 33 MB; 19.8 MB of that is machine code, 6.5 MB symbol tables
(a stripped build drops them).  Of loft's own code, by the layers `scripts/crate_layers.py` holds:

| group | loft code | external crates that leave with it |
|---|---|---|
| language core: lexer, parser, scopes, data, stores, interpreter (stays) | ≈ 12.2 MB | — |
| backends: Rust writer (`generation`, `create`, `native_lib`, `stdlib_codegen`), `lower`, C writer (`c`), their drivers and `codegen_runtime` | ≈ 2.7 MB | — |
| tools: language server, docs site, fuzzing, API reports, live reload, placement host | ≈ 1.1 MB | — |
| package manager: registry, install, self-update | ≈ 0.4 MB | ureq, zip, tar, flate2, ed25519 |
| networking, processes, git | ≈ 0.2 MB | rustls, ring, webpki, rustls-native-certs |

Measured with `nm -S` over `target/release/loft`, grouped by `loft::<module>`; generic code
instantiated for a module (`core`, `alloc`, `hashbrown`, ≈ 5 MB) is not attributed and leaves in
proportion.  So the lean binary is roughly a third smaller, not a tenth of the size: the language
itself is most of it.  What changes more than the size is what the binary CARRIES — no TLS stack,
no code generators, no package downloader, no compiler on the run path.

## The three kinds of part

### Runtime components — native code a loft program calls

HTTP and TLS (client and server), WebSockets, the engine kernel (`engine_host`), processes, git:
today Rust inside the binary (`web_client`, `engine_host`, `process_run`, `git_query`, `tls`)
with a C twin for the C backend (`src/c/loft_web.h`, `loft_engine.h`, `loft_process.h`,
`tls/loft_tls.c` over vendored BearSSL).

- **One implementation, in C**, the C twins promoted to the only one.  `libloft_net` (HTTP,
  TLS, WebSockets, the engine kernel) and `libloft_process` (processes, git).
- **The ABI is `loft-ffi`'s**, which is already plain C layout: a text argument is pointer plus
  length, a text result a `LoftStr {ptr, len}` (binary-safe), a vector a `LoftStore` + `LoftRef`
  with the store's callbacks.  Written down once as `loft_ffi.h`.  A library's `#native`
  declarations do not change: web 0.5.0's and server 0.7.5's are the guards' fixtures word for
  word.
- **Interpreter:** opens the component on the first call of one of its symbols, as it opens a
  declared optional C library today (`c_libs`, `loft_c_lazy_sym`).
- **C backend:** links the component's static archive.  **Rust backend:** declares the symbols
  `extern "C"` and links the same archive.  Three backends, one build of the code.
- **Shipped with loft**, in `<prefix>/lib/loft/<version>/`, found relative to the executable.
  Networking is loft's own (its URL loads, remote stores and package downloads use the same
  client), so it ships with loft rather than with the `web` package.  A component only one
  library needs ships in that library's package instead, prebuilt per platform by its CI
  ([Decisions](#decisions) 2).

### Backends — the code generators

The C writer, the Rust writer, the browser target (`--html`, `--native-wasm`) and Android.

- **Contract: the program image.**  After the front end and the optimiser, the program is a
  `Data` plus its type schema, and `ir_store::save_bundle` / `ir_read::open_bundle_into` already
  write and read exactly that (the start-up cache's stdlib bundle;
  `cargo test --release --test ir_schema_roundtrip` holds the codec over the stdlib and every
  program of the suite).  The driver writes the image; the backend reads it and writes `.c` or
  `.rs`.
- **Form:** a shared library per backend with one C entry point
  (`loft_backend_v1(image, options, out)`), each linking its own copy of the crates it reads
  (`core`, `front`, `opt`, `lower`) — @PLN200 B4 makes those crates.
- **What each carries:** the C backend its runtime headers (`loft_rt.h` …) and the prebuilt
  runtime components; the Rust backend the `loft-runtime` rlib a generated crate links, and it
  still needs `rustc` on the machine — that is the Rust target's nature, not a compile on the
  interpreter's path.
- **Versioning:** the image names the IR schema version and the loft build id; a backend from
  another build refuses it by name.

### Tools — commands that do not run a program

Package manager (`install`, `publish`, the registry, `self-update`), language server,
documentation site, fuzzing, API surface and diff, the reports (`ops-census` …).  Each is an
executable `loft-<tool>`; `loft <tool>` runs it, as `git` and `cargo` dispatch subcommands.
Each links only the crates it needs.

### What stays inside `loft`

The language and the four ways of running a program in it: `run`, `test` (its interpreter leg),
`debug` (the live debugger is the interpreter's own frame), `repl`.  Plus the standard library,
including its natives and the compiled standard library (`compiled_stdlib_gen`), because every
program uses them.  The loader for prebuilt parts, the `#c` caller, and the cdylib loader while
libraries with Rust crates remain.

## Compiles on the fly that go away

| today | becomes |
|---|---|
| a `use`d library is compiled with `rustc` the first time an interpreted program runs it (@C71) | the library runs interpreted, unless its package carries a prebuilt artifact for this platform (built by the library's CI at publish, @PLN200 B7's "a library's publish builds that library's object") |
| a library's `[c] shim` is compiled with `cc` into a shared object for the interpreter (`c_shim`) | the package carries the shim prebuilt per platform; the C backend still compiles it into the unit (compiling is that command's job) |
| the TLS archive is built with `cc` on first use (`src/c/tls.rs`, `~/.loft/c-tls/`) | prebuilt in `libloft_net` |

## Decisions

Taken by the owner as recommended (2026-10-27).

1. **How a backend gets the program — the image, or a shared Rust library.**  The alternative
   to the image is a Rust `dylib` of the core that the binary, the backends and the tools all
   link dynamically (sound when every part comes from one release build, as here).  It saves the
   copies of the core each part carries on disk, and the serialisation.  **Taken: the
   image.**  It is a contract with a codec that is already gated over every program, a backend
   can be run and tested on a saved image alone, and nothing ties the parts to one `rustc`.
2. **Where a runtime component ships — with loft, or with its library's package.**
   **Taken: with loft for what loft itself also uses (networking, processes), in the
   package for the rest** (a graphics or imaging library's native part).
3. **@C71 changes** from "libraries compile, scripts interpret" to "libraries run prebuilt or
   interpreted".  A contract change, so it is a new decision entry replacing @C71.
   **Taken**; the speed of a compiled library is kept wherever its CI publishes the
   artifact.
4. **Which commands stay in the binary.**  **Taken:** `run`, `test`, `debug`, `repl`
   stay; everything else moves to `loft-<tool>`.  The REPL's line editor (`rustyline`) is the
   one dependency that keeping `repl` holds in.

## Phases

Each phase runs the old path beside the new one and compares exactly before the old one goes.

| phase | change | check — what goes red if it is wrong |
|---|---|---|
| L0a | probe: a C-written `n_*` native behind `loft_ffi.h`, built as a shared library, called by its `#native` declaration on all three backends — text with NUL and 0xff bytes both ways, an integer, a boolean, a `vector<u8>` argument | the probe's lines on `--interpret`, `--native-rust`, `--native-c`, hand-fixed |
| L0b | probe: the C backend fed an image reloaded from disk instead of the in-memory `Data` | byte-identical `.c` over the whole corpus (`scripts/c_sweep.py`'s program list) |
| L1 | `libloft_net` from `loft_web.h`, the server's C natives and `loft_tls.c` (+ BearSSL's server half); `web` and `server` use it on every backend; `web_client.rs` and the interpreter's own TLS for these natives go | `tests/web_natives.rs`, `tests/server_natives.rs` (HTTP, binary bodies, one WebSocket, the event pump, TLS over EC and RSA certificates) on three backends; web's and server's own test suites |
| L2 | the engine kernel, URL loads and remote stores, processes and git onto the components | the `engine_host_*` suites, `c_https`, `lib_process` on three backends |
| L3 | the C backend as a part reading the image | L0b's comparison, then the C sweep in its three modes |
| L4 | the Rust backend as a part; `loft-runtime` shipped with it | `tests/native.rs`, the native corpus, the bench hashes |
| L5 | the tools out as `loft-<tool>` | each tool's tests run against the executable |
| L6 | libraries prebuilt or interpreted (@C71's replacement); the registry stores per-platform artifacts | a library with an artifact runs it, one without runs interpreted, the same lines either way |
| L7 | the lean build: the moved features leave `loft`'s default features; release CI builds and ships every part for every platform; a missing part answers one line naming it | the lean binary's size against this plan's table; the corpus and the suite on the lean build; a missing-part refusal per kind |

L0a and L0b come first because each can kill its half of the design for the price of a probe.
L1 needs only L0a.  L3 and L4 need @PLN200 B4's crates.

## L1 — `libloft_net`, cut into steps

What L0a settled: a component is a shared library behind `loft_ffi.h`, loaded or linked by
symbol on every backend.  L1 makes the first one and moves `web` and `server` onto it.

| step | change | check |
|---|---|---|
| L1.1 | the component loader: `[native] component = "net"` in a package's manifest names a library loft ships, found beside the executable (`<exe>/../lib/loft/<version>/`, and `target/components/<triple>/` for a development build); the three backends resolve it as they resolve `prebuilt/<triple>/`.  `make components` builds it, never `loft` itself.  Also takes L0a's follow-ups: the marker line, the C unit binding by search order instead of an absolute path, the refusals that name `cargo` | `tests/ffi_c_library.rs` with the fixture's library as a component; a missing component refused by name on all three |
| L1.2 | `libloft_net` holds `web`'s natives — `loft_web.h` and `loft_https.h` compiled as they are over `loft_cc.h` + `loft_text.h` (the text core, moved out of `loft_rt.h` for this), with `tls/loft_tls.c` and BearSSL inside, every symbol hidden but the 22 typed `n_*` exports and the two registrations; built by `make components`, and CI builds it before the tests — and in the same step the in-binary path goes (`web_client.rs`, the interpreter's `web` registration, the Rust runtime's typed twins, the C writer's in-binary map and `LOFT_WEB` in a unit).  One step, because wherever a symbol exists both ways the in-binary one wins on every backend, so the two cannot run side by side: the comparison is the guard's hand-fixed lines before and after | `tests/web_natives.rs` (12 lines, the pong, both close frames) on three backends through the component; web's own suite on its testbed with `[native] component = "net"` |
| L1.3 | `server`'s natives in it, from the parked port (`refs/keep/firewall02-server-in-binary-wip`: the C half, BearSSL's server files, the guard and its test PKI) | `tests/server_natives.rs` on three backends |
| L1.4 | the web and server packages declare the component (loft-libs-net, a PR per library), once a loft release ships it | web's and server's own suites on their testbeds |

## Open questions

- Windows: the components need MSVC or MinGW builds in release CI (@PLN203 C8 builds the C
  backend's Windows leg).
- The browser bundle (`doc/pkg/loft_bg.wasm`) is an interpreter of its own; its networking is
  `doc/loft-web.js`.  It is not affected, but its feature list should follow L7's.
- The cdylib loader (`native-extensions`) stays while any published library keeps a Rust crate;
  it leaves when the last one has moved (`regex`, `crypto`, `graphics` per @PLN200 B5's route).

## Documentation

What the work updates outside this plan when it finishes:

- `doc/claude/PACKAGES.md` — prebuilt per-platform artifacts in a package; the component ABI.
- `doc/claude/LIBRARY_AUTHORING.md` — a native part is C behind `loft_ffi.h`, built by the
  library's CI.
- `doc/claude/C_BACKEND.md` and `doc/claude/NATIVE.md` — backends as parts, the image contract.
- `doc/claude/DESIGN_DECISIONS_PLATFORM.md` — the entry replacing @C71.
- `doc/claude/RELEASE_PUBLISH.md` — the release bundle's parts per platform.
- `CHANGELOG.md` — the lean binary and the `loft <tool>` dispatch.
