// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Native-artifact identity & cache coherence

> Why a freshly-built `#native` package can still abort a native consumer's link
> with `found crates (libloading and libloading) with colliding StableCrateId
> values`, and the identity model that prevents it. (Investigated 2026-06-15 on
> the `crawler`/graphics consumer; `libloading` was the concrete collider.)
>
> **Resolution (see the end of this section): link the package's cdylib by C-ABI so
> its Rust deps stay private — the StableCrateId class is eliminated by construction.
> The build-identity exploration below is recorded but SUPERSEDED.**

## The shape of the problem

A native consumer (`loft --native` / `--check`) generates a Rust program and
compiles it, linking against `loft`, each `#native` package's prebuilt rlib
(`loft_graphics_native`, …), and those packages' transitive deps. Some deps are
**shared** with loft's own — sharply `libloading` (loft dlopens cdylibs with it;
a graphics stack pulls it via `glutin` for GL). rustc requires that any crate
appearing twice in one link have ONE identity: same name+version => same
`StableCrateId` => the copies must be byte-identical (same SVH). Two `libloading
0.8.9` rlibs built under **different RUSTFLAGS** (loft's `.cargo/config` `mold`
link-arg vs a package built `-g`) share a `StableCrateId` but differ in SVH ->
**collision, link aborts**. Same mechanic, rustc-version trigger -> `E0514 ...
incompatible version of rustc`.

## Two identities, two jobs - keep them separate

An artifact carries two orthogonal properties. Collapsing them into one staleness
number is the root of every recurrence; the fix is to give each its own job.

| | **Build identity** - *how* it was built | **Codegen hash** - *what* loft generates |
|---|---|---|
| Captures | rustc version + effective RUSTFLAGS | loft-ffi source / the generated-ABI surface (`LOFT_FFI_FINGERPRINT`) |
| Its job | **pick the right rlib for the job, fast** - the one built for *my* toolchain | **the deeper test** - has loft's *code generation* changed since this was built? |
| How checked | a path lookup (`.../<build-id>/` present?) - no hashing | a content-hash compare |
| When | **always**, first - you can never link the wrong-toolchain rlib | only to validate an already-selected rlib |
| Resolved by | building *that* identity if its slot is empty | rebuilding when loft's codegen moved |

The two answer different questions: build identity decides **which** artifact can
even link with me; the codegen hash decides **whether** that artifact is current.
We **always want the right rlib for the job, quickly** (a path-keyed lookup), and
the **hash is the deeper test - it fires when our code generation changes** (a new
loft-ffi ABI / generated-code contract), not on every run.

Today both are collapsed into one number: the staleness key is
`loft_ffi_fingerprint` (the loft-ffi *source* hash, deliberately invariant across
debug/release so a CI job reuses one `~/.loft/build-cache`). It carries the codegen
identity but is **blind to rustc + flags** - so a `-g` loft and a `mold` loft are
one identity, a package built by one is judged fresh by the other, its shared deps
keep the wrong flags -> collision. Confirmed: graphics' `.loft-build-fp` stamp
(@23:23) and a fresh stamp from this session's loft are the **same**
`1813746251070023740`, though one's `libloading` is `-g` (246 KB) and the other
`mold` (168 KB).

## When do we load what (today)

1. **Storage** - one rlib per package per profile, at
   `native_target_root(pkg_dir)/release/lib<crate>.rlib` (registry installs
   redirect the root to `~/.loft/build-cache/<pkg>-<ver>/`; monorepo
   `lib/<pkg>/native/` keeps in-tree `target/`). A rebuild **clobbers** the one slot.
2. **Staleness on use** (`add_native_extern_flags`, `src/native_utils.rs`) - rebuild
   iff the rlib is missing OR `!native_artifact_fingerprint_matches(dir,
   loft_build_fingerprint())`.
3. **Build gate** (`auto_build_native`, `src/extensions.rs`) - rebuild iff the
   `.loft-build-fp` sidecar != `loft_ffi_fingerprint()`; on build, re-stamp and pass
   loft's `LOFT_BUILD_RUSTFLAGS` to the package cargo (#274).
4. **Link** - `--extern` the package rlib, `-L dependency=` its deps, **pin shared
   crates to loft's copy**. The pin fixes the *name*; it cannot override the SVH the
   package rlib's metadata demands (compiled against its own `libloading`, found via
   `-L`) -> collision.
5. **Interpreter path is separate** - the dlopen'd cdylib (`pending_native_libs`)
   never enters the rlib link, which is why the interpreter runs graphics while
   `--native` aborts.

## Failure paths

| # | Failure | Trigger | Why today's gate misses it |
|---|---|---|---|
| F1 | `colliding StableCrateId` | shared dep built under different RUSTFLAGS (package vs consumer-loft) | key is flag-blind |
| F2 | `E0514 incompatible rustc` | shared dep / `loft` built by a different rustc | key is rustc-blind |
| F3 | `E0463 can't find crate` | no rlib for this target/identity was ever built (e.g. the wasm rlibs) | not every artifact kind is auto-built |
| F4 | silent stale reuse | codegen hash matches, build identity differs (F1/F2's mechanism) | no build-identity comparison |
| F5 | `fp == 0` "match anything" | loft can't hash its own rlib -> matcher returns `true` | a fallback that reuses *any* artifact |
| F6 | two-check inconsistency | `add_native_extern_flags` checks `loft_build_fingerprint`; the stamp uses `loft_ffi_fingerprint` | masked because `auto_build_native` is the real gate |

## Can we have multiple rlibs beside each other?

- **In one link - NO.** rustc forbids two crates sharing a `StableCrateId`; a
  consumer compile may contain exactly one `libloading`. This is why "namespace
  incidental deps to coexist" cannot be applied uniformly - RUSTFLAGS-level
  namespacing would also split `loft`/`loft-ffi`, whose `Stores`/`DbRef` types
  **must** unify across loft and every package (the *contract* crates are
  non-negotiably shared, one identity).
- **In the cache - YES, and that is the fix.** Key storage by **build identity**:
  `~/.loft/build-cache/<pkg>-<ver>/<build-id>/release/lib<crate>.rlib`, where
  `<build-id>` is a short token of (rustc-version + effective RUSTFLAGS). N
  toolchain/flag-sets then coexist as N cached artifacts; the consumer loads the
  one matching **its** identity, building it on demand. Switching toolchains stops
  clobbering (no churn) **and** stops colliding (the loaded artifact's shared deps
  match the consumer by construction).

## The design - build identity selects (fast, always); codegen hash validates (deep)

> **SUPERSEDED** by the Resolution at the end.  Explored and partly built (it fixes
> flag-axis collisions like `libloading`) but it is whack-a-mole: `log` collides on the
> *profile* axis next.  Kept as the design record so the build-id approach is not
> re-attempted.

Two separate, legible axes - never folded into one number:

- **Build identity = the fast selector.** The consumer computes its `<build-id>`
  from stamps already present (`LOFT_BUILD_RUSTC`, `LOFT_BUILD_RUSTFLAGS`) and
  resolves the package rlib under that directory - a path lookup, no hashing,
  **always**: *the right rlib for the job, quickly.* An empty slot => build *this*
  identity (leave the others alone). A mismatch is diagnosable: "graphics here is
  rustc-1.95/`-g`; you are rustc-1.96/`mold` - building that variant."
- **Codegen hash = the deeper test.** Within the selected identity, the existing
  `loft_ffi_fingerprint` (source hash) answers *"has loft's code generation changed
  since this rlib was built?"* - the case where even the right-toolchain rlib is
  stale (new loft-ffi ABI / generated-code contract). The expensive, occasional
  check; it fires **when our codegen changes**, not on every run.

So **build identity decides *which* artifact (fast, always); the codegen hash
decides *whether it is current* (deep, on change).** The collision becomes
structurally impossible - a consumer can only ever load an artifact built under its
own toolchain+flags - without forcing one global build (a moving target under the
floating-stable toolchain) and without one-slot clobber-churn.

### Cache lifecycle - discard lazily, after convergence

The identity-keyed slots are a **multi-producer store**, not one mutable slot. Two
producers on different toolchains - this laptop on rustc-1.96/`mold`, a Mac laptop on
1.95 with its own flags - each build and use the slot that is *totally right for
them*; both are live and correct at the same time, and the cache can even be shared
across machines, since each producer's build identity selects its own.

Discarding is therefore **lazy and deferred, never eager**: a slot is not dead just
because it is not *my* current identity - another producer may still be on it. The
old variants become collectable only once the producers **fold to a common rustc**
(the heterogeneity that justified them is gone) and the superseded slots stop being
touched. So prune by **disuse** - a last-access timestamp bumped on every load,
reaping slots untouched past a threshold (or keep-N-recent) - **not** by
"not-the-current-one." A still-diverged producer is never robbed of its working
artifact, and cleanup happens on its own after convergence.

### What it closes
F1/F2/F4 structurally (an incompatible artifact is a *different cache slot*, never
loaded). F5 - the build-identity path is derived from stamps, so "match anything"
stops gating shared artifacts. F3/F6 - route every artifact kind (incl. the wasm
rlibs) through the same identity-keyed resolution and use one key on both sides.

### Implementation touch points (not yet built)
- `src/cache.rs`: add `build_identity()` -> short token (rustc + flags); keep
  `loft_ffi_fingerprint` as the codegen freshness hash - **do not fold them.**
- `src/extensions.rs` (`auto_build_native`, `native_target_root`): key the package
  target dir by `build_identity()`; build + stamp per identity.
- `src/native_utils.rs` (`add_native_extern_flags`): resolve under the consumer's
  `build_identity()`; align its freshness key with the stamp (closes F6).
- Wasm rlib paths (Makefile + `tests/html_wasm.rs`) join the scheme so F3 stops
  being a manual rebuild.
- GC by **disuse** (a last-access reaper, e.g. `loft cache gc` / age-out), never an
  eager clobber - heterogeneous producers coexist and a superseded build identity is
  reaped only once a common-rustc convergence stops touching it.
- **Discarded alternative:** folding rustc+flags into `loft_ffi_fingerprint` (one
  number). It detects the mismatch but forces a clobber-rebuild on every toolchain
  switch (churn) and conflates "which artifact" with "is it fresh" - the opposite of
  the fast-select / deep-validate split.

---

## Resolution: separate the API id from the Rust part (link the cdylib by C-ABI)

The build-identity design above (rebuild the package so its shared deps MATCH loft's) was
implemented and partly works — but it is whack-a-mole, and that is structural.  Verified
end-to-end on the crawler:

- keying rebuilt graphics into its slot with loft's flags (#274) and **`libloading`
  vanished** — it was a *flags* mismatch (`-g` vs `mold`).
- A NEW collision surfaced at once: **`log`** — same version/features/rustflags/rustc, but
  a different **profile** hash.  The next would be build-script reproducibility.  Matching
  *every* SVH-affecting input is the fragile path.

**The reframe:** the collision exists ONLY because native-compile links the package as a
Rust *rlib* (`extern crate loft_graphics_native`), pulling its whole crate graph into the
consumer's rustc link where it overlaps loft's.  Link the package's **cdylib by C-ABI**
instead and that graph is sealed in the `.so` — it never enters the link, so the class is
gone BY CONSTRUCTION, for any toolchain / flags / profile / reproducibility.

| | **API id** — crosses the boundary | **Rust part** — stays private |
|---|---|---|
| What | loft-ffi C-ABI: `gl_*` as `extern "C"` symbols + loft-ffi types as opaque pointers | the package's crate graph (`libloading`, `log`, `glutin`, ...) + its rustc/flags/profile |
| Must match | only the **loft-ffi version** (already gated by the codegen hash) | **nobody** — sealed in the `.so` |

Proven by the rebuilt artifact: `gl_*` are exported as plain C symbols
(`#[no_mangle] pub unsafe extern "C"`, via loft-ffi-macros), and `nm -D ...so | grep -c
'U .*(libloading|log)' == 0` — the deps are statically bundled inside the `.so`, not
leaked.  Linking it brings ZERO Rust crates into the consumer.  It is also what the
**interpreter already does** (dlopen the `.so`, call the C symbols), so this UNIFIES the
two native-package paths; the codegen even has a non-rlib call mode already (the
wasm-browser path emits `#[link(...)]` imports, not `krate::sym`).

### Consequence: the build-identity axis disappears for native packages
With the Rust part private, the package's rustc/flags/profile are **irrelevant** — so the
build-identity keying, the multi-producer cache lifecycle, and the lazy GC above are all
**unnecessary**.  F1/F2/F4 close *structurally* (no shared rlib to collide), not by
matching.  The only remaining staleness axis is the **codegen hash** (loft-ffi ABI),
which already gates the cdylib.

### The codegen change — SHIPPED (host-native executable backend)
The native executable backend (`output_native` / `output_native_reachable`) links a `[native]
crate` package's cdylib `.so` by C-ABI; it no longer pulls the package's rlib into the
consumer's rustc link, so the StableCrateId collision class is gone by construction.  Gated on
`Output::native_cabi` (set by `native_utils::native_cabi_enabled()` — on everywhere except
Windows, which stays on the rlib path until C-ABI dylib import-library linking is built).  The
codegen and the linker flags read the *same* helper so they never disagree.

- **`emit_file_header`** (`native_cabi` arm): emits `unsafe extern "C" { … }` declaring each
  body-less `#native` fn (`code() == Null`) that belongs to a `[native] crate` package (has a
  `native_symbol_crates` entry — a stem/dlopen native, `[library] native = "…"`, has none and
  stays on its existing route, not declared/linked here).  The signature is derived from the
  loft types to match exactly what `output_native_direct_call` marshals: a `loft_ffi::LoftStore`
  first param when the fn writes the store (a Reference arg, or a Vector/Reference return);
  text → `*const u8, usize`; vector → `*const ELEM, u32`; Reference → `loft_ffi::LoftRef`; text
  return → `loft_ffi::LoftStr`; Vector/Reference return → `loft_ffi::LoftRef`; scalar widths per
  `is_wide`.  **No reachability filter** — the fn emitter emits a wrapper (a native call) for
  *every* body-less native in range, so each needs a decl (a genuinely-uncalled one is a
  harmless dead extern).  **`#[link_name = "<sym>"]` + a `__cabi_<sym>` local alias** because a
  bare `#native` defaults the symbol to the fn's own `n_<name>`, which would otherwise shadow
  the generated `n_<name>` wrapper (E0428).
- **call site**: `native_cabi` emits the unqualified `__cabi_<sym>` alias (resolved by the
  link), not `krate::sym`.  `output_native_direct_call`'s body is unchanged — its
  `transmute_copy`s are now harmless identities (the consumer has only loft's `loft_ffi`, named
  via `--extern loft_ffi`, since the cdylib's copy is sealed in the `.so`).
- **which symbol** (loft#907): `#[link_name]` names the fn the LIBRARY says implements the
  binding, which is not always the `#native` string.  See § Which symbol a `#native` binding
  links, below.
- **link** (`native_utils::add_native_extern_flags`, `target.is_none() && native_cabi_enabled()`):
  `-L native=<so dir> -l dylib=<stem> -Clink-arg=-Wl,-rpath,<so dir>`, not `--extern
  <ident>=<rlib>`.  Keyed on **`loft_ffi_fingerprint()`** (the ABI hash, matching what
  `auto_build_native` stamps) — NOT `loft_build_fingerprint`: the `.so` is ABI-sealed and stays
  valid across loft rebuilds.
- The rlib is **still built** (`auto_build_native` builds `.so` + rlib) and still *linked* by
  wasm32-wasip2 (cross-compiled rlib), the library-cdylib path, and Windows.  Dropping the rlib
  build awaits converting those remaining paths; the win here is removing it from the native
  executable consumer's link, the one place the collision arose.
- The trade: the `.so` is a runtime/dynamic dependency (RPATH / shipped beside the binary)
  rather than statically baked in — the same model the interpreter already needs.

**Verified** (2026-06-15): a store-mutating round-trip — imaging `save_png` + `load_png` (both
Reference-arg/`LoftStore`) — native-compiles, links `libloft_imaging.so` by C-ABI, runs the
store mutation through the handle, and prints output identical to `--interpret`
(`saved=true reloaded=2x1`).  Full native test suite green.

**Remaining paths + residual gaps** — the library-cdylib path (the viewer collision),
wasm32-wasip2, and Windows still link the rlib; plus same-symbol cross-package
disambiguation, `make install` `.so` packaging (`$ORIGIN` RPATH + copy), the
boolean→`u8` ABI, and the `prebuilt/` + `fp == 0` resolution edges — are tracked in
**@PLN26** ([loft-lang/plans#26](https://github.com/loft-lang/plans/issues/26)).

### Which symbol a `#native` binding links (loft#907)

**`#native "sym"` is an API id, not the name of the Rust fn behind it.**  A native library
registers its implementations by loft symbol —
`loft_register_bridges! { "sym" => other__loft_bridge }` — and nothing requires `other == sym`.
So a `#native` string names a *binding*; only the library's own table names the *function*.

The two backends used to answer that differently, and the disagreement was silent:

| | how it resolved `#native "sym"` |
|---|---|
| `--interpret` | the loaded cdylib's `loft_register_bridges_v1` table — the library's own answer |
| `--native` (before) | `#[link_name = "sym"]` — whatever the cdylib exports under that name |

A C-ABI link matches on **name alone**, so where the two differed the call was marshalled into a
different function with no error, no warning and no failed link.  In the published `graphics`
that was ten functions: each store-aware one has loft's `(LoftStore, LoftRef)` entry point at
`n_<x>` and an older raw `(ptr, count)` fn under the `#native` name, so arguments arrived shifted
by a register — `save_png` answered `false` and wrote nothing, the WebGL upload calls uploaded
nothing.

**Both backends now read one source.**  `extensions::resolve_native_impl_symbols` runs after
`load_all` and before any codegen, asks the loaded cdylibs which fn implements each symbol
(`dladdr` on the registered bridge pointer names it — `#[loft_native]` emits `X__loft_bridge`
beside `X`, so stripping the suffix gives the implementation), and records **only the differing
entries** in `Data::native_impl_symbols`.  `Data::link_symbol` is the one accessor codegen emits
through, on both the C-ABI `#[link_name]` and the rlib `krate::sym` path.

- A **clean binding** — the loft symbol equals the implementing fn — maps to itself and nothing
  changes.  That is the only shape `loft-ffi-build`'s generator can produce, so a library that
  derives its registration from its `#native` annotations is clean by construction; a remap only
  ever comes from a hand-written `loft_register_bridges!` list.
- A cdylib that is **absent, un-loadable, or predates the bridge registry** cannot be resolved and
  keeps the literal name.  Not a silent wrong answer: the interpreter reports the missing
  registration at load (loft#886) and a call panics rather than answering.
- `dladdr` is required to hit the bridge **exactly** (`dli_saddr == ptr`); it otherwise reports the
  nearest preceding symbol, which would hand codegen a neighbouring function's name.  A near miss
  leaves the symbol alone.

Guards: `tests/lib/native_remap_pkg` is a `[native] crate` fixture in this shape which exports a
DECOY under each `#native` name, so a regression *answers* (-1000 / -2000) instead of failing to
link — the way this reached users.  `native_scalar_pkg` is the clean-binding control.
