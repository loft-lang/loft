<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The startup cache — history

What [STARTUP_CACHE.md](STARTUP_CACHE.md) no longer says because it no longer holds.

## The development-build exemption, until 2026-10 (loft#1762)

Until loft#1762 the policy had five rules, and a development build was exempt:

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

The exemption existed so a compiler change being debugged could never be answered by a stale
bundle.  It was kept after its reason was answered on the facts — both cache keys fold in
`binary_signature_tag`, the placed-library list rides the manifest, and the warm load replays
each diagnostic's fixes (loft#1129) — until the owner flipped it (2026-09-30, after the
2026.10 freeze), with `LOFT_NO_CACHE` as the off switch made visible in `loft --help`,
DEBUG.md and RUNNING_TESTS.md.  Decision: @C133.

### Which loft am I measuring? (rule 4 is a trap for benchmarks)

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

## The wasm runtime rlib build, retired with the rustc writer (@PLN225)

While the rustc writer built `--html` and `--native-wasm`, those builds also ran `cargo` to
build loft's own wasm runtime rlib, and the `LOFT_TIMING` report showed it.  The text as it
stood:

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

⚠ **Warm with the RELEASE binary.**  The cdylib key is profile-independent (`BUILD_ID` is the
git-HEAD id, so debug/release/test share it), but `loft_build_fingerprint` — which gates the wasm
runtime rlib — is a content hash of the binary, so a debug-warmed rlib leaves a release-driven
test (`html_asyncify` runs `target/release/loft`) still cold.
