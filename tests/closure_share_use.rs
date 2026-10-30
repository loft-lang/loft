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

use std::fs;
use std::path::Path;

/// Every `.rs` file under `dir`, as `(path relative to the crate, contents)`.
fn rust_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read src") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).expect("read source");
            let rel = path
                .strip_prefix(env!("CARGO_MANIFEST_DIR"))
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, text));
        }
    }
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
    assert_eq!(
        files_naming("share_with_closure_copy"),
        vec!["src/database/closure_share.rs"],
        "the closure-copy share is raised outside closure_share.rs — see this file's doc"
    );
}

#[test]
fn the_share_is_taken_back_only_by_free_named() {
    assert_eq!(
        files_naming("release_closure_share"),
        vec![
            "src/database/allocation.rs",
            "src/database/closure_share.rs"
        ],
        "the closure-copy share is released outside Stores::free_named — see this file's doc"
    );
    let alloc = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/database/allocation.rs"),
    )
    .expect("read allocation.rs");
    assert_eq!(
        alloc.matches("release_closure_share(").count(),
        1,
        "free_named is the one caller of release_closure_share"
    );
}

#[test]
fn the_count_is_reached_only_through_its_module() {
    // The field and its type: declared in `mod.rs`, initialised at the three `Stores`
    // constructors (`mod.rs` twice, the worker stores in `allocation.rs`), read in the module.
    for needle in ["closure_shares", "ClosureShares"] {
        assert_eq!(
            files_naming(needle),
            vec![
                "src/database/allocation.rs",
                "src/database/closure_share.rs",
                "src/database/mod.rs",
            ],
            "`{needle}` is named outside the closure-share module and its constructors"
        );
    }
}
