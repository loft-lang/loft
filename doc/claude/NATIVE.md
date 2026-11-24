
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Native builds

`--native` compiles a program through C: the C backend writes the program as C11, the host's C
compiler (`LOFT_CC`, `CC`, else `cc`) builds it, and the binary is cached and run.  The same C
reaches every compiled target — `--native-wasm` and `--html` through clang for WASI
([WASM.md](WASM.md), [HTML_EXPORT.md](HTML_EXPORT.md)), `--native-android` through the NDK's
clang — and a `use`d library runs as a hosted C unit on the interpreter's stores.  The two
backends are the interpreter and the C (`@C170`, [DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md)).

This doc is about native BUILDS: the tiers a build is made in, when a run falls back to the
interpreter, how a library's compiled unit is adopted, built and collected, how a native
program links its libraries, and the Android target.  How the C is written — its model, its
parts, its refusals, its rewrites — is [C_BACKEND.md](C_BACKEND.md).  The `#rust "…"` templates in
`default/*.loft` are the interpreter's operator source (`create.rs` generates `src/fill.rs` from
them); no compiled target reads them.  The rustc writer that `--native` used before, its
architecture and its open work are the record in
[NATIVE-history.md § The rustc writer](NATIVE-history.md#the-rustc-writer-as-nativemd-described-it-until-pln225-retired-it).

---

## Optimisation tiers — semantics runs, performance lanes, shipped binaries

Three tiers, and the split is deliberate (the owner's rule):

| tier | what runs it | frames | C compiler |
|---|---|---|---|
| **semantics** | `--native`, the test runner's native corpus and fixtures | named frames (`LOFT_CALL_CHAIN`) | `-O1` — the compile is the cost that matters |
| **performance** | `bench/stats.py`'s `c` and `cship` lanes, a consumer's `--ship` bench | lean | `-O3` |
| **shipped** | `--ship` on every compiled target; a library's hosted unit (`c::hosted::build_artifact`) | lean | `-O3` |

`--native-debug` is the semantics tier at `-O0 -g`, its debug information naming the loft
source.  The tier's flags are part of the binary's cache key (`c_driver::cc_flags`).

**`--ship` is the build a program ships as**, on whichever target the run names: the host
binary by default, `--native-wasm`, `--html` or `--native-android`.  Each gets reachable-only
emission and the shipped tier; `--html` keeps its named frames.  Native libraries are linked
INTO the program unless `--dynamic` or the program's `#link dynamic` says otherwise
(`native_utils::link_static_unless_named`).  Every fault check stays.  `--ship` is refused
where nothing is built: with `--interpret`, `--tests`, `--emit-c` (C source only) and an
`--html --debug` page.

A semantics build may also reshape its C for the compiler's sake: the C backend writes an
over-budget function as a chain of parts there and never in a shipped unit
([C_BACKEND.md](C_BACKEND.md) § Large functions are split into parts).

A performance lane measures exactly what a shipped binary gets, and never the semantics
build — the RUNTIME included: `bench/stats.py` (and through it the portal and `perf-check`)
runs the loft binary `scripts/lean_build.sh` builds, `make install`'s feature list, which leaves
out the debugging aids (`strict-stores`).  The measurements behind the split are in
[NATIVE-history.md § The optimisation tiers, measured](NATIVE-history.md#the-optimisation-tiers-measured).
`--lean` remains the way to strip the tier from a `--native` build; `--html` keeps its named
frames (the browser panic hook's frame block is a pinned contract).

**The semantics tier is an open program; the other two are closed programs (the owner's ruling).**  A
build that ships the live tier parses the program again at run time and may flip or replace any
function (`Data::open_world`), so a whole-program rewrite that needs the closed program — one
that changes a signature, inlines a callee or picks a call target (`@FR-R-ValueRecord`'s IR
pass) — does not apply there: the dynamic build need not be fully optimised.  The performance
lanes and `--ship` are closed and get every such rewrite.  So a check of such a rule runs a
closed build (`--lean`): a semantics run never reaches it.  A library's hosted unit is open in
the same sense: an interpreter the writer never sees calls its exports ([C_BACKEND.md § A library
as a hosted unit](C_BACKEND.md#a-library-as-a-hosted-unit)).

**The tier that does not exist yet.**  Every tier above keeps the null-sentinel and overflow
checks that make an integer fault a reported null rather than a wrapped number
(DESIGN_DECISIONS.md @C129, @C120): the checks are the semantics, and they stay the default
because a starting programmer must never be handed a random-looking number they cannot debug.
What may come, and is deliberately not built: an OPT-IN "proven program" tier for a game that
has already run fine with the checks on — licensed by the evidence of its own fault-free runs
under the checked tier, never by a declaration — that emits the plain arithmetic the processor
does.  Its shape (the owner's): a RELEASE BUILD PASS FOR GAMES.  A library published to the
registry always carries the checks; a game that has passed extended in-house testing under the
checked build is compiled once more from every source it uses — its libraries recompiled at
that level inside the game's build, never taken as published binaries — into the less safe
artefact that ships to Steam or the browser.  A build, not a switch, and licensed by the
testing's fault-free ledger: `--ship <ledger>`.  The ledger itself is recorded already:
`LOFT_LEDGER=<file>` on a checked run, either backend (BOTH_BACKEND_SWITCHES.md).  Precompiled
libraries serve quick development only; the release build recompiles every library from source,
because the checks it drops live inside the library code.  Its design is @PLN201
([plans/201-ship-tier](plans/201-ship-tier/README.md)): a ledger of the checked runs' fault
sites, named on the IR so it holds for any code generator, and the build that drops a check the
ledger licenses.  Its speed is measured already: `bench/stats.py`'s `cship` lane builds the C
under `LOFT_RELEASE_PASS_PROBE=1`, the tier with every site licensed, and reports its ratio per
routine beside the checked `c` lane ([bench/README.md](../../bench/README.md) § The C lanes).  Until
it exists, the only way a check is retired is a PROOF that the value cannot be the sentinel
(`@FR-R-Counter`, the range proofs @PLN157 queues), which is portable by construction and — the
point — retires the check inside a library's ordinary build, so every consumer gets it with the
checks on.

## Loop-invariant vector headers

A loop the C can prove writes **no store** derives each vector's `(store_nr, record, length)`
once, before the loop, and reads its elements against that header instead of re-deriving all
three per element ([C_BACKEND.md](C_BACKEND.md) § The model).
`LOFT_NO_VECTOR_HOIST=1` holds no header, so one binary carries both halves of an A/B; it is the
first bisect step for a `--native`-only wrong answer in a loop that indexes a vector.
`LOFT_HOIST_VERIFY=1` is the falsifier of the hoist family.  Every such switch, with its
falsifier, is in [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md).

## Native→interpreter fallback, and `LOFT_REQUIRE_NATIVE`

A default `loft <file>` run prefers native but **degrades to the interpreter** rather
than failing when native is genuinely unavailable.  This keeps loft turnkey on a box
without a working C compiler, but it can silently mask a performance regression — a
program you *think* runs native is quietly interpreting.  There are exactly two places
native can degrade (both in `src/main.rs::main`):

| Where | Trigger | Default behaviour |
|---|---|---|
| **Library adoption** | a `use`d library whose hosted unit won't build (`Err`) or is being edited (`Ok(None)` dev-interpret) | the library interprets; `Err` warns, dev-interpret is silent |
| **Main-program `'native` block** | no C compiler on a cache miss (@C102), or the C backend refuses the program | runs the program on the interpreter, saying why where native was asked for (`--native`) or in a source checkout, silently from a release |

Set **`LOFT_REQUIRE_NATIVE=1`** (the inverse of `LOFT_NO_NATIVE_LIBS`) to turn **every**
one of those fallbacks into a **hard error that names the reason**, so a performance run
can never silently interpret.  Off by default.  Enforced at one chokepoint per location: the
library loop, and a single post-`'native`-block check (each fallback records *why* in
`native_fallback_reason`; the chokepoint reports it, and a catch-all still errors loudly if a
future fallback forgets to record one).  `--check` runs are exempt (they report parse status,
not execution).  Guards: `tests/n3_use_native.rs::require_native_*`.

**`--interpret` is NOT the escape hatch for a library that will not build**, and the
refusal says so.  `--interpret` chooses the interpreter for **your program**; a `use`d
library still builds its unit, so a broken library keeps failing under it.  The switch
that makes every library interpret is **`LOFT_NO_NATIVE_LIBS=1`**.  When you change that
message, check the cure by running it: `LOFT_FORCE_NATIVE_BUILD_FAIL=1` reproduces the
refusal on any program.

## N9 — native-library shared-store dispatch (C71)

An interpreted program that `use`s a library dispatches the library's compiled functions over
the **shared store** and interprets the rest — byte-identical to the all-interpreted run.  The
compiled form is a hosted C unit working on the interpreter's own stores; what it is, how it is
generated and what it refuses is [C_BACKEND.md § A library as a hosted
unit](C_BACKEND.md#a-library-as-a-hosted-unit).  The decision is automatic and invisible (`use
<lib>` is native; `LOFT_NO_NATIVE_LIBS` opts out), with a dev-interpret-on-edit fallback so an
actively-edited library never pays a C compile per save
(`native_artifact::cached_or_build_shared_cdylib`): a fresh unit is adopted, a missing or
stale-loft one is built, and one whose sources changed since the previous run interprets that
run and rebuilds once the edits settle.  `native_gate::native_compilable` decides which
functions cross; the rest interpret.  Soundness: the mixed interp+native boundary is
parity-checked (`tests/n3_parity.rs`, interp≡mixed≡native).

**An artifact is never inspected at a path a rebuild-and-load can follow at.**  macOS
dyld caches a loaded image BY PATH for the process lifetime, and its `dlclose` is a no-op — so a
second `dlopen` of the same path returns the FIRST image even after the file was rebuilt.  The
layout-adoption probe (`native_adopt::artifact_matches_layout`, which reads the unit's
`LAYOUT_FP_SYMBOL`) therefore asks through a RELOCATED artifact — hard-linked, or copied when
linking fails, into a throwaway `.layout-probe-<pid>-<seq>/` — and never at the artifact's own
path.  There is one entry point.  Linux keys `dlopen` on (dev, inode), so only macOS needs it,
but every platform takes the same path.

**A degraded probe never degrades to the answer a fix removed.**  When the relocation
cannot be made, the answer is *cannot adopt*, which rebuilds — never a probe in place and never
"declares no layout" (which means ADOPT).  Two rules:

- **A fallback is never the behaviour a fix removed.**  Degrade toward the
  expensive-but-correct answer (rebuild), never toward the cheap one that was the bug.
  A silent fallback is the same defect twice: the wrong answer, and no way to see it.
  A relocation that fails says so once per process and names the cure.
- **Relocation that costs nothing cannot fail for lack of room.**  The probe
  hard-links first (O(1), no space, no bytes read); the byte copy is the fallback for a
  filesystem without links.  A link shares the artifact's inode, which is safe under both
  keying schemes — the rebuild publishes a NEW inode by rename, and where the caller adopts
  instead, the bytes are identical anyway.

Guards: `tests/n3_parity.rs::a_dependency_edit_invalidates_its_dependents_cdylib` is the
end-to-end shape of the macOS trap; `a_dependency_edit_reaches_a_dependent_when_the_layout_probe_cannot_relocate`
and `an_unrelocatable_layout_probe_rebuilds_instead_of_adopting` set
`LOFT_FORCE_PROBE_RELOCATE_FAIL=1`, which makes both relocation legs fail on every platform.

**Probe before marking.**  Marking a function for unit dispatch is a commitment: the
bytecode calls its bridge symbol, and if nothing wires that symbol the call reaches a panic stub.
A unit can build cleanly and still be one this process cannot dispatch through — built by a
different loft, missing a system library at load, or replaced by a concurrent `loft` between the
freshness check and the load.  `native_adopt::probe_and_mark_exports` therefore `dlopen`s the
artifact, `dlsym`s each bridge, and marks only what resolves; a symbol that does not resolve
leaves its function **interpreting**.  The marking can come out partial, and that is correct.
The handle is kept for the process, so a later prune or a rebuild by another process cannot
change what this one dispatches to.  A fallback is reported once per library with a count —
the program's results are unchanged, only slower; under `LOFT_REQUIRE_NATIVE=1` it is a hard
error.  Guards: `tests/n3_use_native.rs::an_unwirable_cdylib_interprets_instead_of_panicking`
and `::a_partially_exporting_cdylib_marks_only_what_resolves`.

**The artifact sweep owns one family, not the directory.**  `prune_artifacts` bounds
`native-auto/` at `KEEP_ARTIFACTS` (eight) by mtime, and takes only the family it built
(`opt::native_key::auto_cdylib_stem`).  The directory is not the auto-native builder's alone: a
package with a `[c] shim` builds its content-keyed shim library there exactly once, so it is
permanently the oldest file — and a shim is the only definition of the package's `#c` symbols,
with nothing that could interpret in its place.

**Seeing and collecting the caches — `loft cache`.**  The sweep runs only after a
successful build in that directory, so `loft cache` is the command that shows and reduces the
rest:

* `loft cache status` — the footprint per area, and what is reclaimable.  Read-only,
  so it is also the dry run for `prune`.
* `loft cache prune` — drop it.  `--all` takes the live generation too.

The two areas are decided differently, and the report says which is which:

| area | test | certainty |
|---|---|---|
| `build-cache/<pkg>-<ver>/` | its build fingerprint ≠ this loft's `native_artifact_cache_key()` | **exact** — the same comparison the cache lookup makes, so the tree can never be selected again |
| `registry/*/native-auto/` | beyond `KEEP_ARTIFACTS` newest per family | **conservative** — the artifact name folds the consumer's `layout_fp`, of which there is an open set, so reachability is not decidable from the name |

No age bound is used: "older than the running loft binary" dates everything from a freshly
built binary.  For the same reason `prune` **refuses to run from a binary that is not the
installed `loft`** (`--force` overrides): a development build has its own `BUILD_ID`, so it
judges the installed loft's live generation unusable *by itself*, and deleting it costs every
project on the machine a cold rebuild.

## A native program's libraries

**A dynamic program binds its libraries at start (@PLN195).**  The link names no package
library and writes no rpath; `main` binds each library from the search order in
[LOFT_DESIGN.md § Shipping a dynamic program](LOFT_DESIGN.md#a-native-build-links-its-libraries-beside-the-binary-or-into-it),
and the driver hands a program it runs the build's library directories in `LOFT_LIB_PATH`.
`--out <dir>` writes the program and every library file it links into one directory that runs
wherever it is copied.  `--static` / `#link static` links the libraries in instead.  Guard:
`tests/native.rs::a_dynamic_program_finds_its_libraries_where_loft_looks`.

**A C library a package ships travels with the program (@PLN197).**  A `[c] shim` or a file
beside the package is never linked through a path.  A dynamic program binds it like a package
library (a symbol its handle lacks falls back to the `#c` resolver, since `#c` symbols are
attributed per package).  A static one links its archive into the binary (`c_shim::archive`, by
full path — `ld64` picks a same-stem `.dylib` over `-l static=`) when every library of the
package has one (`c_shim::archived_package`), and binds the rest.  On Windows (MSVC target) that
archive is compiled by `cl.exe` + `lib.exe` (`platform::shim_archiver`): the shared shim is a
MinGW build whose objects `link.exe` cannot take.  A prebuilt C library has no static form there
(beside a DLL, a `.lib` is its import library), and `windows-gnu` keeps the shim beside the
binary.  Guard: `tests/native.rs::a_shipped_c_library_travels_with_the_program`.

A library's native part is a C `[c] shim`; a Rust native part (`[library] native`, `[native]
crate`) is refused where its manifest is read.  The identity model that keeps a native package's
link and cache coherent is [NATIVE_ARTIFACT_IDENTITY.md](NATIVE_ARTIFACT_IDENTITY.md).

## Verification

**A rewrite both backends run lands with its switch A/B green.** The interpreter is the
oracle only for a GENERATOR rewrite; one the parser or scope pass applies (the ownership /
buffer family) gives both backends the same answer, right or wrong, so its falsifier is the
switch itself: the corpus and the consumer suites run with it off and on, and any difference
is a defect (`.github/workflows/switch-ab.yml`; the rule is `(R-Switch)`'s both-backend
clause in [formal/rewrites.md](formal/rewrites.md)).  A C-only rewrite is checked against the
interpreter: `scripts/c_sweep.py` runs every corpus program both ways
([C_BACKEND.md](C_BACKEND.md) § Running it).

## Android target

`loft --native-android [out.apk|out.so] prog.loft` builds the program as an Android app through
the C backend: the C unit, loft's Android entry (`src/c/android/loft_android_main.c`) and the
NDK's app glue, compiled by the NDK's clang and packaged as a signed APK by `src/android.rs`
(design: [plans/106-android-build-target/c-backend.md](plans/106-android-build-target/c-backend.md)).
Env: `ANDROID_NDK_HOME`, `ANDROID_HOME`, `JAVA_HOME`; `LOFT_ANDROID_TARGET` (default
`aarch64-linux-android`; `x86_64-linux-android` for the KVM emulator), `LOFT_ANDROID_API`
(default 24).  Needs a `fn main`.  A library's `[c] shim` compiles for the device: `graphics`
(0.13 and later) draws through EGL and OpenGL ES 3, `audio` plays through miniaudio, an
`https://` fetch runs the TLS client against Android's CA store, and the app carries the files
its program names (@C164).  Checked by `tests/android_target.rs` (the build),
`scripts/c_sweep.py --android` (every corpus program on an emulator against the interpreter)
and `scripts/android_guards.py` (drawing, input, sound, files and HTTPS).

## See also
- [C_BACKEND.md](C_BACKEND.md) — how the C is written: its model, parts, rewrites and refusals
- [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) — the switches of the rewrites only the C applies
- [BOTH_BACKEND_SWITCHES.md](BOTH_BACKEND_SWITCHES.md) — the switches of the lowering both backends share
- [PERFORMANCE.md](PERFORMANCE.md) — benchmark results and performance designs
- [COMPILER.md](COMPILER.md) — compiler pipeline: lexer, parser, IR, bytecode
- [INTERMEDIATE.md](INTERMEDIATE.md) — IR Value tree structure
- [DATABASE.md](DATABASE.md) — runtime data store and type schema

---

## Open work

Enhancements on a complete, graceful core: a construct the library dispatch cannot cross
**interprets** (correct, just not native-accelerated), so none is a correctness gap.

| Item | Status |
|---|---|
| **Closures across the library boundary** | A library function taking or returning a closure (a `__closure` parameter) interprets: `native_gate::native_compilable` excludes it.  Crossing it needs the `__closure` record marshalled over the bridge. |
| **Background build** | The first run after edits settle rebuilds the library's unit in the foreground (`native_artifact::cached_or_build_shared_cdylib`); moving that to the background would let even the settling run never block. |
