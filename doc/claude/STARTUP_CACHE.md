<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The startup cache

What loft caches between runs so a program starts without re-parsing, how to turn it off, and
why a benchmark must know which loft it is measuring.  Part of measuring performance:
[PERFORMANCE.md](PERFORMANCE.md).

The whole-program startup cache skips ALL parsing — stdlib + lazily-`use`d
libs + user file — on warm runs by writing a binary bundle (content-addressed
`.store` + `.manifest`) to `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`).
It is default-on (@PLN11 G2 Track 1).

## What is cached

The bundle contains the fully-parsed IR (`Data`) for the entire program prefix:
every `default/*.loft` file, every `use`-d library, and the user script.  It is
keyed on a drift manifest holding a SHA-256 of each parsed source's bytes
(`cache::file_hash`), so any source edit invalidates the cache automatically.

## Default-on behaviour and overrides

`cache::program_cache_enabled()` (`src/cache.rs`) implements the precedence
order:

1. `LOFT_NO_CACHE` (non-empty) → **off** — the explicit kill switch for
   production scripts that must never read/write bundles.
2. `LOFT_PROGRAM_CACHE` (non-empty) → **on** — explicit force; used by the
   cache's own tests to override the two dev defaults below.
3. `CARGO_MANIFEST_DIR` present → **off** — auto-disables inside
   `cargo run` / `cargo test`.  The compiler-debug loop and the entire
   integration-test suite never read/write bundles with zero per-test wiring.
4. **the binary lives in a `target/{debug,release}/` tree → off** — the OTHER
   half of the same compiler-debug loop, and the half that surprises people.
   Rule 3 only catches an invocation *Cargo* made; running `target/release/loft`
   by hand sets no variable, and that is what iterating on the compiler actually
   looks like.  Keyed on the binary's own path (`running_a_dev_build`), so it
   also covers `CARGO_TARGET_DIR=target-da`.
5. otherwise → **on** — the default for installed / real invocations.

## Programs whose libraries build native, or have dependencies (loft#1684)

A program that `use`s a library with an auto-native build — every registry library, by
default — is cached like any other: the bundle carries the functions' native marks, and the
manifest records the cdylibs they dispatch to (`alib`) and the context that marked them
(`actx`: `LOFT_NO_NATIVE_LIBS`, `LOFT_FORCE_NATIVE_BUILD_FAIL`, `--html`).  A warm load needs
a matching context and every recorded cdylib on disk, else it is a miss before anything is
committed and the cold path rebuilds.  The key names the search path AS GIVEN: registering a
library's `[dependencies]` appends directories mid-parse (the registry root, a path dep's
parent), and a key taken after the parse was one no warm load computes.  Before both, `use
graphics;` alone re-parsed the program and its libraries on every launch — 1.19 s and 81 MB
against 0.11 s and 25 MB warm.  A cold run also verifies and parses the registry index once
per process instead of once per `use` lookup (six times for graphics).

**`LOFT_TRACE_WARM=1`** names the verdict of every warm load — `[warm] hit: <bundle>`, or the
gate that missed (no manifest at the computed path, a build signature or stdlib key that
differs, a changed source, a native-library context that differs, a recorded cdylib that is
gone) — and at a save, the search path the key used beside the one the parse ended with.

## Which loft am I measuring? (rule 4 is a trap for benchmarks)

Rule 4 means **a binary you built from source pays the cold cost on every run,
forever** — by design, so a parser change is never answered by a stale bundle.  It
also means the two binaries on your machine have startup costs that differ by
roughly the warm/cold ratio below, and nothing in the output says which one you ran.

That is not hypothetical: loft#864 was filed as *"every invocation pays full
process-spawn + compile overhead, give me a warm session or a daemon"*, on numbers
measured with a from-source build.  The same programs on the installed binary ran
several times faster, because the cache the report needed was already there and
rule 4 had switched it off.  **Benchmark the installed binary, or say which one you
measured.**

| invocation | program cache | a rerun of an unchanged program |
|---|---|---|
| `loft prog.loft` (installed) | on | warm — no parsing at all |
| `target/release/loft prog.loft` | **off** (rule 4) | full stdlib parse, every run |
| `cargo run -- prog.loft` | off (rule 3) | full stdlib parse, every run |
| `LOFT_NO_CACHE=1 loft prog.loft` | off (rule 1) | full stdlib parse, every run |

**How to tell which you got, in one command.**  `LOFT_TIMING=1` prints
`parse_default=<n>ms`: a couple of ms is a warm bundle load, tens of ms is a cold
parse of `default/`.  It is the cheapest hit/miss check there is, and it works on
any build.

**`LOFT_TIMING=1` also reports the EXTERNAL build tools, with a reason each** (loft#1238).
A `--html` or `--native-wasm` build shells out to `cargo`, `rustc` and `wasm-opt`, and until
now the only channel was a per-event stderr line — right for a spawned process, useless for a
reader watching a 25-second `--html` and wanting to know what it is doing.  The report is
per-invocation, slowest first, and it names WHY each ran:

```
[loft-build] --html: 3 external invocation(s), 25.79s total
[loft-build]     25.61s  cargo     libloft.rlib (wasm32-unknown-unknown)  loft's own build fingerprint moved (loft was rebuilt) — rlib is stale
[loft-build]      0.18s  wasm-opt  prog_opt.wasm                          always runs — --asyncify is required for frame yield, not an optimisation
[loft-build]      0.00s  lock      html                                   waiting for the global build lock
```

**What it made visible.**  `loft_build_fingerprint()` is a CONTENT HASH of `libloft.rlib` /
the loft executable, so it moves on every loft rebuild — and it gates loft's own wasm runtime
rlib.  The first `--html` / `--native-wasm` run after ANY loft source change therefore pays a
full `cargo build` of that rlib (25.6 s here); the second is 0.55 s.  That is the price of the
guarantee that a codegen change reaches an already-built artifact, not a defect, but it was
invisible and it is why a wasm-shaped test can look like a hang.

**`loft cache warm` builds that set up front, and `make ci` runs it** (loft#1238).  The cost
above is unavoidable per artifact, but WHO pays it is not: left alone it lands on whichever test
reaches the artifact first, while the rest queue on the global build lock, and the charge shows
up as that test blowing a per-test deadline it has nothing to do with.  `loft cache warm --from
tests` builds them once, before the parallel section, with the same calls a test would make — so
the artifacts are stamped with the same key and every test afterwards takes the ordinary hit
path.  0.3–0.5 s once warm, and it never fails the caller: a package that cannot be warmed is
simply built by the test that needs it, exactly as before.

**Scope is the whole design of that command.**  The first version warmed every stale tree under
`~/.loft/build-cache/`, which on this box is 61 of 71 — generations belonging to OTHER loft
builds, none of which the run would touch — and was still going minutes later.  The set that
costs a test its budget is the INTERSECTION of two smaller ones: packages the corpus actually
`use`s, and trees already on disk under a previous key.  Neither alone is right — used-but-absent
is a first build no warming can avoid, and stale-but-unused is somebody else's cache.

⚠ **Warm with the RELEASE binary.**  The cdylib key is profile-independent (`BUILD_ID` is the
git-HEAD id, so debug/release/test share it), but `loft_build_fingerprint` — which gates the wasm
runtime rlib — is a content hash of the binary, so a debug-warmed rlib leaves a release-driven
test (`html_asyncify` runs `target/release/loft`) still cold.

**The lock is timed apart from the build on purpose.**  Under a parallel runner every
wasm-shaped process reaches `/tmp/loft-native-build.lock` at once after a loft rebuild, so a
single run's wall-clock can be almost entirely SOMEONE ELSE'S build.  Folding the two together
would name `cargo` for time cargo did not spend in this process.

Silent unless armed, and silent when nothing ran — a build that reused every artifact prints
no report at all.

**`LOFT_STDLIB_CACHE` is the narrower fallback, not an addition.**  It caches
`default/` only, and `main.rs` engages it **just when the program cache is off** —
so on an installed binary setting it changes nothing, while on a from-source build
it recovers most (not all) of what rule 4 gave up.  A measurable win from setting it
is therefore itself a signal that the program cache was disabled.

**`loft test` engages it too, and there it pays per FILE** (loft#925).  A suite
builds one parser per test file — deliberately, so one file's definitions cannot
leak into the next — and each of those parsers used to re-parse `default/` from
scratch.  For a directory of N test files that is N cold stdlib parses of a
directory that provably did not change between them.  `src/test_runner.rs` now asks
`warm_load_stdlib` first and falls through to the cold parse on a miss, exactly as
`main.rs` does; on a 21-file synthetic that is 2.33 s → 1.86 s.  The control matters
for reading that number: before the change, setting `LOFT_STDLIB_CACHE=1` on a
`loft test` run did **nothing at all**, because the runner never called the API.

## The shared library base (loft#925)

The larger half was NOT that cache: a `use`d LIBRARY was loaded from source once
per test file — *twice* per file, because both parse passes re-run the use region,
`Data::reset` having cleared the loaded-library map between them.

**Measured before the fix** (best of 3, idle box; the generator is on loft#925).  A
package of M modules behind an aggregator, N test files that all `use` it, beside a
control package whose N test files `use` nothing:

| N (M = 25) | `use` | control |   | M (N = 20) | per file |
|---|---|---|---|---|---|
| 1 | 0.10 s | 0.03 s |   | 10 modules | 0.039 s |
| 5 | 0.35 s | 0.13 s |   | 25 modules | 0.065 s |
| 10 | 0.66 s | 0.23 s |   | 50 modules | 0.122 s |
| 20 | 1.36 s | 0.45 s |   | | |

Dead linear in N with **zero amortization** — marginal 0.068 s/file against the
control's 0.022 s/file — and the per-file cost proportional to library size, so a
suite paid the **product** of the two.  That is what made it superlinear in project
growth: a new module slowed every test file, a new test file re-paid for every module.

**What it does now.**  Files are grouped by their leading `use` region *verbatim* —
the text, not an interpretation of it, so the parser stays the authority on what
those lines mean and a shared key is a shared library set by construction.  The
second file of a group triggers one parse of that region alone (`Parser::parse_as`,
the whole-program path fed a string instead of a file), and every file in the group
then starts from a copy of it: `Data` cloned, the schema re-installed, plus the
`use`-path and native-registration state a `use` would have produced.  `Data`
carries a `preloaded_uses` map that `reset` re-seeds, which is the one mechanism
that makes a `use` of an already-loaded library a no-op instead of a file read.

**After** (same box, same generator):

| N (M = 25) | before | after |   | M (N = 20) | before | after |
|---|---|---|---|---|---|---|
| 1 | 0.07 s | 0.06 s |   | 10 modules | 0.80 s | 0.23 s |
| 20 | 1.32 s | 0.43 s |   | 25 modules | 1.32 s | 0.43 s |
| 40 | 2.68 s | 0.74 s |   | 50 modules | 2.44 s | 0.81 s |

The marginal per-file cost drops from 0.068 s to 0.0155 s and stops tracking the
library — 3.0× at 25 modules, 3.6× at 40 files.  On the consumer that reported it
(dryopea: 81 test files, 1161 tests, one group) the suite went **238 s → 209 s**,
i.e. the ~31 s the issue predicted, with byte-identical output.

Three decisions are load-bearing and easy to undo by accident:

- **A group of one gets no base.**  The base is built when a SECOND file asks for
  the same region; before that the file parses exactly as it always did.  Building
  eagerly doubled `loft test <one-file>` (0.07 s → 0.13 s) — the tight inner loop of
  development, and a group of one by definition.
- **A seeded file skips the stdlib warm load.**  The base already holds the stdlib,
  so loading the bundle per file only decoded it for `seed_from` to discard —
  that redundant decode was most of what a seeded file still paid (it is the
  difference between the 3.0× above and the 1.5× without it).
- **A leading `#cwd` is part of the region, not a reason to refuse one.**  All 81
  of dryopea's test files open with it; a scanner that gave up there measured
  perfectly on the synthetic and saved the reporting consumer nothing.

`LOFT_NO_TEST_BASE=1` turns the sharing off — the control half of an A/B on one
binary, and what the equivalence guard in `tests/test_base_equivalence.rs` compares
against.  `LOFT_TEST_BASE_REPORT=1` says on stderr which regions got a shared base,
which is how that guard knows it is not comparing a run to itself.

Not covered, and still open: repeated `loft test <one-file>` invocations, which
need a keyed on-disk bundle whose key covers every library source plus the resolved
dependency graph (loft#930 is the reminder of what an incomplete key costs).

Two things had made this un-reproducible outside the reporting consumer, both worth
knowing when cutting a suite-shaped benchmark: `loft test` refuses multiple file
arguments (loft#916 — the suite form is a DIRECTORY, which only became nameable when
`resolve_test_target` stopped appending `.loft` to one), and a package needs
`[library] entry = …` or its own `src/` is not a library its tests can `use`.

## Invalidation

`build_signature()` folds together:

- The cache-format version constant, loft version, and git HEAD (`BUILD_ID`).
- The running binary's mtime (`binary_signature_tag()`) so an *uncommitted*
  compiler rebuild invalidates bundles.  `BUILD_ID` (git HEAD) alone does not
  change across uncommitted edits; the mtime addition closes that gap.

## Eviction

`cache::prune_program_cache()` is called after each cold save.  It evicts the
oldest `(.store + .manifest)` pairs until the cache directory is under
`LOFT_CACHE_MAX_MB` (default **512 MiB**).

## See also — the startup-cache design

Full design, E1/E2/E3 arc, and the zero-copy follow-up: see
[`plans/11-data-as-store/README.md`](plans/11-data-as-store/README.md).

---
