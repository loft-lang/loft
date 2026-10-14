// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN38 phase 01 — Tier 1 (`IntegrityOnly`) end-to-end exercise.
//!
//! Round-trips `Store::open_durable`: open → write → clean drop → reopen
//! detects clean state.  Then validates the corruption-detection path
//! (kill-skipped drop, mid-write torn writes, callback semantics).

#![cfg(feature = "mmap")]

use loft::file_access as fa;
use loft::store::{DurabilityMode, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

type CorruptionCb = Box<dyn Fn(&Path) -> std::io::Result<()>>;

/// Per-test scratch directory.  Cleared on entry so a previous failed run
/// doesn't leave stale `.dmeta` sidecars behind.
fn scratch(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("loft-store-durable-tier1")
        .join(test_name);
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn dmeta_path(main: &Path) -> PathBuf {
    let mut p = main.as_os_str().to_owned();
    p.push(".dmeta");
    PathBuf::from(p)
}

/// Build a callback that initialises a brand-new store (or rebuilds an
/// existing one) by opening it with `Store::open` and dropping immediately.
/// `Store::open` creates the file if it doesn't exist and writes the legacy
/// signature.  Counts invocations into a shared atomic so tests can assert
/// "callback fired exactly N times."
fn init_callback(counter: Arc<AtomicUsize>) -> CorruptionCb {
    Box::new(move |path: &Path| {
        counter.fetch_add(1, Ordering::SeqCst);
        let path_str = path.to_str().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path not utf-8")
        })?;
        // Open creates the file if missing and writes the legacy signature.
        // Drop the store immediately — that's all the callback needs to do
        // to initialise from-scratch state.  open_durable's
        // write_initial_sidecar path then captures CRC + length.
        let _ = Store::open(path_str);
        Ok(())
    })
}

// ── round-trip ──────────────────────────────────────────────────────────────

#[test]
fn fresh_path_triggers_callback_and_subsequent_open_is_clean() {
    let dir = scratch("fresh_path_triggers_callback");
    let main = dir.join("data.store");
    let counter = Arc::new(AtomicUsize::new(0));

    // First open: file doesn't exist → callback fires → init → sidecar
    // written → retry succeeds.
    let store = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: init_callback(counter.clone()),
        },
    )
    .expect("first open should succeed after callback");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    drop(store);

    // Sidecar must now exist.
    assert!(
        fa::exists(dmeta_path(&main)),
        "sidecar should exist after clean drop"
    );

    // Second open: clean state → callback does NOT fire.
    let store2 = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: init_callback(counter.clone()),
        },
    )
    .expect("second open should succeed without callback");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "callback must NOT fire on a clean-state reopen"
    );
    drop(store2);
}

#[test]
fn clean_drop_writes_sidecar_with_current_crc() {
    let dir = scratch("clean_drop_writes_sidecar_with_current_crc");
    let main = dir.join("data.store");
    let counter = Arc::new(AtomicUsize::new(0));

    {
        let _store = Store::open_durable(
            &main,
            DurabilityMode::IntegrityOnly {
                on_corruption: init_callback(counter.clone()),
            },
        )
        .expect("open should succeed");
    } // Drop runs here — sidecar should be rewritten.

    // Re-open: validate path should report clean.  If the sidecar wasn't
    // correctly rewritten, callback would fire.
    let counter2 = Arc::new(AtomicUsize::new(0));
    let _store = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: init_callback(counter2.clone()),
        },
    )
    .expect("re-open should succeed");
    assert_eq!(
        counter2.load(Ordering::SeqCst),
        0,
        "callback must NOT fire when the previous drop closed cleanly"
    );
}

// ── corruption-detection paths ──────────────────────────────────────────────

// NOTE: a "kill simulation via mem::forget + append-to-file" test was
// considered but cannot work in release builds.  Appending raw bytes to
// a legacy-format Store file extends it past the in-store record chain,
// and the next `Store::open` calls `fl_rebuild()` which walks block
// headers — encountering a zero-sized header in the appended garbage
// causes `pos += 0`, an infinite loop in release (`debug_assert!`
// catches it only in debug).  The `deleted_sidecar_triggers_callback`
// and `corrupted_sidecar_triggers_callback` tests below cover the
// "skip Drop" semantic without that hazard.

#[test]
fn deleted_sidecar_triggers_callback() {
    let dir = scratch("deleted_sidecar_triggers_callback");
    let main = dir.join("data.store");
    let counter = Arc::new(AtomicUsize::new(0));

    {
        let _store = Store::open_durable(
            &main,
            DurabilityMode::IntegrityOnly {
                on_corruption: init_callback(counter.clone()),
            },
        )
        .expect("initial open");
    }
    counter.store(0, Ordering::SeqCst); // Reset.

    // Wipe the sidecar — simulates someone deleting `.dmeta` between runs.
    fa::remove_file(dmeta_path(&main)).expect("remove sidecar");

    let _store2 = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: init_callback(counter.clone()),
        },
    )
    .expect("reopen after sidecar wipe");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "callback must fire when sidecar is missing"
    );
}

#[test]
fn corrupted_sidecar_triggers_callback() {
    let dir = scratch("corrupted_sidecar_triggers_callback");
    let main = dir.join("data.store");
    let counter = Arc::new(AtomicUsize::new(0));

    {
        let _store = Store::open_durable(
            &main,
            DurabilityMode::IntegrityOnly {
                on_corruption: init_callback(counter.clone()),
            },
        )
        .expect("initial open");
    }
    counter.store(0, Ordering::SeqCst);

    // Flip a byte in the sidecar's payload_crc region.  That breaks the
    // payload-CRC check (sidecar's header_crc covers only bytes 0..12, so
    // changing payload_crc bytes 32..36 doesn't invalidate the header_crc;
    // it invalidates the CRC-vs-main-file comparison).
    let meta = dmeta_path(&main);
    let mut bytes = fa::read(&meta).expect("read sidecar");
    bytes[32] ^= 0xFF;
    fa::write(&meta, &bytes).expect("write corrupted sidecar");

    let _store2 = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: init_callback(counter.clone()),
        },
    )
    .expect("reopen after sidecar corruption");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "callback must fire when sidecar's payload_crc is wrong"
    );
}

// ── callback error semantics ────────────────────────────────────────────────

#[test]
fn callback_returning_error_propagates_to_caller() {
    let dir = scratch("callback_returning_error_propagates");
    let main = dir.join("data.store");

    let err = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: Box::new(|_path: &Path| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic callback failure",
                ))
            }),
        },
    )
    .expect_err("callback's error must surface");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}

#[test]
fn callback_that_leaves_main_file_unrecoverable_hits_recursion_cap() {
    // The recursion cap exists so a buggy callback can't infinite-loop
    // open_durable.  Verify it fires when the callback returns Ok but
    // leaves the main file in a state that still fails validation.
    //
    // After @PLAN38 phase 01's auto-rewrite-sidecar behaviour, a no-op
    // callback against a sidecar-only-corruption is NOT a recursion-cap
    // case anymore — the framework rewrites the sidecar from the main
    // file's current state and the next validate returns Clean.  To
    // actually trip the cap we need the main file to be missing or
    // corrupt after the callback runs.
    let dir = scratch("callback_leaves_main_unrecoverable");
    let main = dir.join("data.store");

    // Step 1: initial open creates a valid store.
    {
        let setup_counter = Arc::new(AtomicUsize::new(0));
        let _store = Store::open_durable(
            &main,
            DurabilityMode::IntegrityOnly {
                on_corruption: init_callback(setup_counter),
            },
        )
        .expect("initial open");
    }

    // Step 2: corrupt the sidecar.
    let meta = dmeta_path(&main);
    let mut bytes = fa::read(&meta).expect("read sidecar");
    bytes[32] ^= 0xFF; // corrupt payload_crc
    fa::write(&meta, bytes).expect("write corrupted sidecar");

    // Step 3: open with a callback that DELETES the main file rather
    // than rebuilding it.  The post-callback `has_main` check skips
    // sidecar rewrite; recurse; validate still Corrupt; recursion cap.
    let cb_count = Arc::new(AtomicUsize::new(0));
    let cb_count_clone = cb_count.clone();
    let main_clone = main.clone();
    let err = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: Box::new(move |_path: &Path| {
                cb_count_clone.fetch_add(1, Ordering::SeqCst);
                let _ = fa::remove_file(&main_clone);
                Ok(())
            }),
        },
    )
    .expect_err("destructive callback must NOT cause an infinite loop");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        cb_count.load(Ordering::SeqCst),
        1,
        "callback must run exactly once before the recursion cap kicks in"
    );
}

// ── format compatibility ────────────────────────────────────────────────────

#[test]
fn legacy_file_opened_via_open_durable_migrates_via_callback() {
    let dir = scratch("legacy_file_opened_via_open_durable_migrates");
    let main = dir.join("data.store");

    // Pre-create the main file using the legacy (non-durable) path —
    // no sidecar exists.
    {
        let _legacy = Store::open(main.to_str().unwrap());
    }
    assert!(fa::exists(&main), "legacy main file should exist");
    assert!(!fa::exists(dmeta_path(&main)), "no sidecar yet");

    let counter = Arc::new(AtomicUsize::new(0));
    let _store = Store::open_durable(
        &main,
        DurabilityMode::IntegrityOnly {
            on_corruption: init_callback(counter.clone()),
        },
    )
    .expect("legacy → durable migration should succeed");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "callback fires once to materialise the sidecar"
    );
    assert!(
        fa::exists(dmeta_path(&main)),
        "sidecar created during migration"
    );
}
