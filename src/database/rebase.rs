// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Translating a `par` worker's `DbRef`s into the parent's store numbering after the
//! parent adopts the worker's stores.  A table over store numbers only, so it sits with
//! the stores rather than with the parallel runtime that builds it (@PLN191).

use crate::keys::DbRef;

// ── Plan-06 phase 2 — store-rebase infrastructure ─────────────────────────────
//
// Today's `copy_from_worker` (src/database/allocation.rs:409) deep-copies
// every struct result from a worker's WorkerStores into the parent's
// result vector via `copy_block` + `copy_claims`.  For a 100K-element
// result of 256-byte structs that's 25.6 MB of memcpy per parallel call.
//
// Phase 2's `StoreRebase` replaces deep-copy with **store adoption +
// DbRef translation**: the worker's allocated stores are moved into
// the parent's allocations table (store ownership transferred); any
// DbRef referring to those stores from the worker's local namespace
// is translated to the parent's new store_nr.  Bytes never move.
//
// See plan-06 DESIGN.md D2.1 (slot-marker design) and DESIGN.md D11c
// (the per-field worker-own / parent-shared / cross-worker translation
// rule).

/// Translates DbRefs across the worker→parent adoption boundary.
///
/// Built once per `parallel_for` call: every worker-allocated store
/// is adopted into the parent (via `Stores::adopt_store`), recording
/// its worker-local store_nr → parent-side store_nr mapping in a
/// per-worker `StoreRebase`.  When the parent later inspects a DbRef
/// the worker handed back, it calls `StoreRebase::translate` to rewrite
/// the `store_nr` field.
///
/// The translation is per-worker: each worker's DbRefs come from a
/// different worker-local namespace, so the rebase map is constructed
/// fresh for each worker.
///
/// Currently unused at runtime — phase 2b's narrow path
/// (`copy_from_worker_unowned`) handles owned-free struct returns
/// without needing the rebase walk.  The full step 2b will use this
/// for owned-field structs (text, refs).
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct StoreRebase {
    /// Maps worker-local `store_nr` → parent-side `store_nr` for stores
    /// adopted from one worker's WorkerStores.  A DbRef whose `store_nr`
    /// is **not** in the map either points at a parent-shared store
    /// (the original parent allocations the worker only read, store_nr
    /// < parent_store_count) or at a worker-local store that wasn't
    /// adopted (which would be a codegen bug).
    pub map: std::collections::HashMap<u16, u16>,
    /// Number of parent-side stores at the moment this rebase was
    /// constructed (before any worker output was adopted).  Set via
    /// `with_parent_count`; used by `translate` to apply the
    /// 3-category rule from DESIGN.md D11c:
    ///   - `db.store_nr` in `map` → worker-own, translate
    ///   - `db.store_nr < parent_store_count` → parent-shared, pass through
    ///   - otherwise → cross-worker (codegen bug); panic in debug,
    ///     log + pass through in release
    pub parent_store_count: u16,
}

#[allow(dead_code)]
impl StoreRebase {
    /// Construct an empty rebase map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an empty rebase map with the parent-side store count
    /// captured at the moment of construction.  Adoption monotonically
    /// increases the parent's store count, so any pre-existing parent
    /// store has a `store_nr < parent_store_count` — which is how
    /// `translate` separates parent-shared refs from worker-own ones.
    #[must_use]
    pub fn with_parent_count(parent_store_count: u16) -> Self {
        Self {
            map: std::collections::HashMap::new(),
            parent_store_count,
        }
    }

    /// Record a worker-local → parent-side translation.
    pub fn add(&mut self, worker_local: u16, parent_nr: u16) {
        self.map.insert(worker_local, parent_nr);
    }

    /// Translate a DbRef from the worker's local namespace to the
    /// parent's namespace.  Implements the 3-category rule from
    /// DESIGN.md D11c:
    ///
    /// | `db.store_nr` shape | Category | Action |
    /// |---|---|---|
    /// | In `map` | Worker-own | Translate to parent-side `store_nr` |
    /// | `< parent_store_count` | Parent-shared | Pass through |
    /// | Otherwise | Cross-worker | Debug-panic; release-pass-through |
    ///
    /// The cross-worker case is a codegen bug — workers should never
    /// hand out DbRefs into a sibling worker's stores.  Phase 5's
    /// auto-light analyser tightens this further; until then we keep
    /// the release path defensive (pass through unchanged).
    #[must_use]
    pub fn translate(&self, db: &DbRef) -> DbRef {
        if let Some(&parent_nr) = self.map.get(&db.store_nr) {
            return DbRef {
                store_nr: parent_nr,
                rec: db.rec,
                pos: db.pos,
            };
        }
        if db.store_nr < self.parent_store_count {
            // Parent-shared (input store, stdlib const, parent-allocated
            // read-only).  Pass through.
            return *db;
        }
        // Cross-worker: codegen bug.  Debug builds panic so the bug
        // surfaces immediately; release builds log + pass through so a
        // released runtime degrades gracefully on a corner the test
        // suite missed.
        debug_assert!(
            false,
            "StoreRebase::translate cross-worker DbRef: \
             store_nr={} not in rebase map and not parent-shared \
             (parent_store_count={})",
            db.store_nr, self.parent_store_count
        );
        eprintln!(
            "loft: StoreRebase cross-worker DbRef store_nr={} (parent_store_count={}); passing through unchanged",
            db.store_nr, self.parent_store_count
        );
        *db
    }
}
