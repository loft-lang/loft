// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a library's compiled artifact, adopted or built

//! The driver's one question about a library this run: adopt its compiled artifact, build
//! one, or interpret it.  The interpreter answers the first half
//! (`native_adopt::adopt_shared_cdylib`) and the Rust writer builds
//! (`native_lib::build_shared_artifact`); neither names the other, so the two meet here.

use crate::data::Data;
use crate::database::Stores;
use crate::native_adopt::Adopt;
use std::collections::HashSet;

/// @PLN11 Arc N / N3 (Step 4 — **dev-interpret-on-edit**) — decide how the library
/// at `pkg_dir` should run this invocation, and build its cdylib only when warranted.
///
/// Returns:
/// - `Ok(Some(so))` — dispatch native against the cdylib at `so` (fresh cached, or
///   just built);
/// - `Ok(None)` — **interpret this run** (the library is being actively edited);
/// - `Err(_)` — a build was attempted and failed (the caller interprets + warns).
///
/// The policy reconciles "a library is always native" with "no `rustc` per save":
/// 1. **Fresh artifact** (`.so` exists, fingerprint matches, source not newer) →
///    native, no work.
/// 2. **No artifact yet, or `loft` itself changed** (fingerprint mismatch) → **build
///    eagerly** → native.  First use of a library, and a deployed/stable dep, pay the
///    one-time build and run native from the start (this is also why the parity tests,
///    which wipe `native-auto/` then run once, still dispatch native).
/// 3. **Stale artifact** (an `.so` exists but the source is newer — the library was
///    edited) → consult the last-run source hash:
///    - *changed since the previous run* → the edit loop is live → **interpret this
///      run** (`Ok(None)`); record the new hash.  No `rustc`.
///    - *unchanged since the previous run* → editing has settled → **rebuild** → native.
///
/// So an edit-run-edit-run loop interprets each time (no `rustc`); the first run after
/// you stop editing rebuilds and the library is native again.  (Polish: do that
/// rebuild in the background so even the settling run never blocks — see the plan's
/// Step 4 option 3.)
///
/// # Errors
/// Propagates a build failure from [`build_shared_cdylib`].
pub fn cached_or_build_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
    pending: &[String],
) -> Result<Option<std::path::PathBuf>, String> {
    match crate::native_adopt::adopt_shared_cdylib(data, stores, export_set, pkg_dir, pending)? {
        Adopt::Fresh(so) => Ok(Some(so)),
        Adopt::Interpret => Ok(None),
        Adopt::Build(key) => crate::native_lib::build_shared_artifact(
            data,
            stores,
            export_set,
            pkg_dir,
            &key.out_dir,
            &key.stem,
            key.fp,
        )
        .map(Some),
    }
}
