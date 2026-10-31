// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The closure-copy share count has exactly ONE use, and this test keeps it that way.
//!
//! `src/database/closure_share.rs` holds the only reference count in the store layer.  It
//! exists for one case — a fn-ref read out of a struct field and kept (`g = h.f`) shares the
//! stores its closure record owns, and the last holder releases them (loft#2273) — and the
//! ownership model forbids it everywhere else: a store has ONE owner, decided statically.  A
//! count is the easy way out of every "who frees this?" question, which is why it must not
//! spread.  This test fails the moment a name of the mechanism appears outside its two sites:
//!
//! - the share is RAISED in `closure_share.rs` alone (by `copy_closure_record`, the body of
//!   `OpCopyClosure`), through a function private to that file;
//! - the share is TAKEN BACK in `Stores::free_named` alone (`allocation.rs`).
//!
//! If this fails because you added a use: do not widen the allow-list.  Find the owner of the
//! store instead (`doc/claude/formal/ownership.md`), or raise it with the project owner — a
//! second use is a change to the ownership model, not a refactor.

use loft::file_access;
use std::path::{Path, PathBuf};

/// Every `.rs` and `.h` file under `dir` (the C backend's runtime carries a twin of the count),
/// as `(path relative to the crate, contents)`.
fn rust_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in file_access::read_dir(dir).expect("read src") {
        let path: PathBuf = entry.os_spelling();
        if file_access::is_dir(&path) {
            rust_files(&path, out);
        } else if file_access::has_extension(&path, "rs") || file_access::has_extension(&path, "h")
        {
            let text = file_access::read_to_string(&path).expect("read source");
            let rel = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, text));
        }
    }
}

/// One file of the crate, read.
fn crate_file(rel: &str) -> String {
    file_access::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The files that name `needle`, sorted.
fn files_naming(needle: &str) -> Vec<String> {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut hits: Vec<String> = files
        .into_iter()
        .filter(|(_, text)| text.contains(needle))
        .map(|(path, _)| path)
        .collect();
    hits.sort();
    hits
}

#[test]
fn the_share_is_raised_in_one_file() {
    let rust: Vec<String> = files_naming("share_with_closure_copy")
        .into_iter()
        .filter(|f| f.ends_with(".rs"))
        .collect();
    assert_eq!(
        rust,
        vec!["src/database/closure_share.rs"],
        "the closure-copy share is raised outside closure_share.rs — see this file's doc"
    );
}

#[test]
fn the_share_is_taken_back_only_by_free_named() {
    let rust: Vec<String> = files_naming("release_closure_share")
        .into_iter()
        .filter(|f| f.ends_with(".rs"))
        .collect();
    assert_eq!(
        rust,
        vec![
            "src/database/allocation.rs",
            "src/database/closure_share.rs"
        ],
        "the closure-copy share is released outside Stores::free_named — see this file's doc"
    );
    let alloc = crate_file("src/database/allocation.rs");
    assert_eq!(
        alloc.matches("release_closure_share(").count(),
        1,
        "free_named is the one caller of release_closure_share"
    );
}

#[test]
fn the_c_twin_is_reached_only_at_its_three_sites() {
    // The C runtime's `closure_shares`: declared in `loft_store.h`, reset and taken back in
    // `loft_rt.h` (`loft_store_new`, `loft_free_ref`), raised in `loft_keyed.h`
    // (`loft_copy_closure`, the body of `OpCopyClosure`).
    let c: Vec<String> = files_naming("closure_shares")
        .into_iter()
        .filter(|f| f.starts_with("src/c/"))
        .collect();
    assert_eq!(
        c,
        vec![
            "src/c/loft_keyed.h",
            "src/c/loft_rt.h",
            "src/c/loft_store.h"
        ],
        "the C closure-copy share count is named outside its three sites"
    );
    let rt = crate_file("src/c/loft_rt.h");
    assert_eq!(
        rt.matches("closure_shares").count(),
        4,
        "loft_rt.h names the share count at the reset, and at the release: its comment, one \
         check, one decrement"
    );
}

#[test]
fn the_count_is_reached_only_through_its_module() {
    // The field and its type: declared in `mod.rs`, initialised at the three `Stores`
    // constructors (`mod.rs` twice, the worker stores in `allocation.rs`), read in the module.
    for needle in ["closure_shares", "ClosureShares"] {
        let rust: Vec<String> = files_naming(needle)
            .into_iter()
            .filter(|f| f.ends_with(".rs"))
            .collect();
        assert_eq!(
            rust,
            vec![
                "src/database/allocation.rs",
                "src/database/closure_share.rs",
                "src/database/mod.rs",
            ],
            "`{needle}` is named outside the closure-share module and its constructors"
        );
    }
}
