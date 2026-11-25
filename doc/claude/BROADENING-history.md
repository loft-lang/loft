<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# BROADENING history

Records moved out of [BROADENING.md](BROADENING.md).

## Differentiator #4 as it read until 2026-11-24

Before the rustc writer's retirement (@PLN225, `@C170`) a library's native part could be a Rust
crate; since then it is C (`#c`, a `[c] shim`) and `[native] crate` is refused.

4. **It inherits the Rust ecosystem — and, more to the point, Rust's
   *stability*.**  loft is *built on* Rust, so it does not re-implement numpy /
   requests / regex — it **binds the crate** (`ndarray`, `reqwest`, `regex`,
   `polars`, …) and gets all of crates.io.  But the **bigger reason than saved
   code is what comes *with* the library: memory safety and maturity.**  A
   well-grounded Rust crate brings its battle-testing *and* the guarantee that it
   **cannot segfault loft** the way a buggy C extension hard-crashes the Python
   interpreter (safe Rust has no UB; panics unwind), and `cargo` brings
   reproducible, lock-filed builds instead of Python's wheel / ABI / manylinux
   roulette.  So "Python has an ecosystem" cuts *both* ways — Python inherits C's
   libraries **and** C's crash surface; loft inherits Rust's libraries **minus**
   that instability.  This is the
   [GOALS § Purpose](GOALS.md#purpose--what-loft-is-for) "software that does not
   fall over" aim **extended to the whole dependency surface** — which is *why* the
   crates must be **well-grounded** (the reliability filter: you inherit stability
   only from crates that have it) and bound **one per real need**, the dogfood way,
   not via a speculative auto-binder.

   On functionality alone the boilerplate is *still* wrong: Python's "rich
   ecosystem" is itself mostly a **binding** story (numpy/scipy = C/Fortran, torch =
   C++, polars / pydantic-core / cryptography = *Rust*; the C-API is glue), so loft
   plays the same move on the more modern host — and because loft *is* Rust, the
   binding is **in-language** (loft value ↔ Rust value, no C ABI), so it is
   lower-friction.  The honest gap is binding **ergonomics, not library existence**.
   Mechanism: PACKAGES.md + § Native-library execution model below.

   **The complementary route — the prebuilt cdylib (`@PLN21`) — is keyed on the *C
   ABI*, and that broadens #4 from "Rust's ecosystem" to "*every native ecosystem,
   via the C ABI*."**  A registry library's cdylib links the PUBLISHED, versioned
   loft-ffi C-ABI (proven rustc-independent — `nm` shows zero loft symbols, only
   system NEEDED libs), is validated by a *content* fingerprint of that ABI
   (`cache::loft_ffi_fingerprint`, **not** a toolchain hash), and is `dlopen`'d.  So it
   ships as a per-platform **prebuilt that needs no toolchain to *use*** — and its
   native source can be **any language that emits a C-ABI `.so` implementing the loft-
   ffi contract** (Rust ergonomically today via the macros; C / C++ / Zig directly once
   a `loft-ffi.h` lands).  Where CPython / Node tie native extensions to *their own* ABI
   **and** to a build of that runtime, loft's is the bare C ABI + a versioned
   fingerprint — runtime-version- *and* source-language-independent.

   This does **not** weaken the stability story, because **stability is being
   *well-grounded*, not the source language.**  The "C's crash surface" point above is
   about *un*grounded native code; a decades-hardened C library (sqlite, zlib, libpng)
   is as stable as safe Rust — and a large share of Rust's "native" crates are precisely
   Rust *wrapping those very C libraries*.  So the C-ABI prebuilt model just distributes
   the stable native substrate (mature C, Rust-over-C, or pure Rust) uniformly and
   toolchain-free; the reliability filter stays **well-grounded** (mature / tested /
   maintained).  Memory-safe Rust is *one* route to grounding for new code; a hardened C
   library is another.  The binary structure merges anything that speaks the contract;
   what *earns* the guarantee is grounding, not language.

## The build fingerprint as it read for rustc-built artifacts (until 2026-11-24)

#### The build fingerprint — correctness crux

A native library artifact is generated Rust that `extern`-links `libloft.rlib`.
It is valid only against the exact loft build whose rlib it links.  Change loft's
codegen OR its runtime ABI OR rustc → an old artifact mis-links.  This is the
root cause of the recurring "generated rust-code error" (the same class as tests
needing `make rebuild-native-cdylibs`).

Cache key requirements:

- **MUST fold**: the loft rlib CONTENT hash (memoised once per process in
  `native_utils::native_cache_key`, `src/native_utils.rs`; already done by BUILD2
  for the test-binary cache) + rustc version + target + feature-set.
- **Must NOT fold**: git-HEAD `BUILD_ID` (unchanged across uncommitted rebuilds)
  or mtime (over-invalidates / fragile).

Two enforcement points:

1. **Nuke-on-recompile** (startup self-check): compare current loft build
   fingerprint against a stored marker; on mismatch, clear the native artifact
   cache.  Fast cleanup of rebuild orphans.
2. **Fingerprint in every per-artifact cache key** (lazy backstop).  Together
   these make `make rebuild-native-cdylibs` obsolete.

Known gap to close: `@P341` native-package rlib path still folds **mtime** (per
PERFORMANCE-history.md § BUILD2 notes) — that is the specific hole behind hitting the
error too often.

#### Three-layer model

| Layer | Mechanism | Goal |
|---|---|---|
| rlib-hash in each artifact key | cache key invalidation | correctness — never link a stale artifact |
| nuke-on-recompile (startup marker check) | cache sweep on next startup | fast cleanup of rebuild orphans |
| idle-TTL GC (24 h, touch-on-use) | background eviction | space — unused library sets age out |

Idle-TTL (touch-on-use) is the primary eviction policy.  The current
`cache::prune_program_cache` (oldest-first size-cap) stays as a runaway backstop.

#### Validation layer and developer-vs-customer / daily-builds framing

The build fingerprint is eventually owned by a library **validation layer**: an
artifact's validity = content-hash · target · features · loft-build-fingerprint ·
(eventually) signature.  The cache then becomes dumb storage + the idle-TTL
janitor; the nuke trigger is subsumed by validation (stale-fingerprint artifacts
fail `is_valid` → never linked → swept).

It is one fingerprint serving different audiences on different timelines:
- **loft developers** need it now (rlib changes constantly, often uncommitted;
  git-HEAD `BUILD_ID` is useless here).
- **customers on releases** are covered today by `LOFT_VERSION`.
- **customers on daily builds** will need the fingerprint (same/rolling version,
  different codegen → `LOFT_VERSION` fails).

Building it now for developers IS the customer/daily-build mechanism.  This
dovetails with reproducible builds: BUILD2 confirmed the rlib is
byte-deterministic (`src/native_utils.rs`), so the fingerprint is a meaningful
stamp.

Note on binary-vs-rlib: the loft binary hash is the human-meaningful "which loft
build" identity and co-varies with the rlib, but the **rlib hash is the term that
strictly guarantees link compatibility** (the artifact links the rlib, not the
executable).

_Tactical now (developer-facing, local)_: per-library native artifact cache with
rlib-hash key + nuke trigger + idle-TTL; extract a single `loft_build_fingerprint()`
reusing BUILD2's memoised rlib hash so it has a clean seam to move into the
validation layer.

First concrete step: audit every native-artifact cache key for mtime/git-HEAD
usage (`@P341` is the known offender) + extract `loft_build_fingerprint()`.
