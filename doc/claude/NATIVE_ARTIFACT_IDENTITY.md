// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Native-artifact identity & cache coherence

A package's compiled form is a **hosted C unit**: its functions written as C and built by the C
compiler into a shared library in the package's `native-auto/`
([C_BACKEND.md § A library as a hosted unit](C_BACKEND.md#a-library-as-a-hosted-unit)).

- **One identity, in the name.**  The unit's file name carries the loft binary that built it,
  the library's sources and the build switches; any program run by that loft adopts it,
  whichever run built it, and the directory keeps the newest eight.  A unit another loft built
  is never adopted, and a release's fetched units carry the `loft_exe_id` they were built by
  ([RELEASE_PUBLISH.md § Library artifacts](RELEASE_PUBLISH.md#library-artifacts)).
- **Fresh across the graph.**  A unit carries its dependencies' functions too, so its
  freshness spans every package that contributed code
  ([PACKAGES.md § An auto-native artifact is stale when any source it CONTAINS is newer](PACKAGES.md#an-auto-native-artifact-is-stale-when-any-source-it-contains-is-newer)).
- **No toolchain axis.**  A C unit is linked by C ABI, so nothing about the compiler's version
  or flags has to match another artifact's.

A package's own Rust native library is refused (@PLN225).  The identity model it needed — the
`StableCrateId` collisions between a package's rlib and loft's own, the build-identity versus
codegen-hash split, the C-ABI link of the package's cdylib, and which symbol a `#native`
binding linked — is recorded in
[NATIVE_ARTIFACT_IDENTITY-history.md](NATIVE_ARTIFACT_IDENTITY-history.md).
