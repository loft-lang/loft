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
