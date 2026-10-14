// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 — the workspace reverse index (`loft::lsp::WorkspaceIndex`) that drives
// find-references.  Lexes the `.loft` tree, so comments/strings are excluded; the
// overlay reflects unsaved buffers.

use loft::file_access as fa;
use std::path::PathBuf;

use loft::lsp::{WorkspaceIndex, identifier_at};

fn temp_ws(name: &str) -> PathBuf {
    // A unique dir per test — the tests run in parallel and must not share files.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    fa::create_dir_all(&dir).unwrap();
    // `area` in a comment must NOT be counted; the def + the call must be.
    fa::write(
        dir.join("a.loft"),
        "fn area(w: integer) -> integer {\n  // area is w times w\n  w * w\n}\n",
    )
    .unwrap();
    fa::write(dir.join("b.loft"), "fn main() {\n  print(area(3))\n}\n").unwrap();
    dir
}

#[test]
fn references_span_files_and_skip_comments() {
    let wi = WorkspaceIndex::build(temp_ws("refws_span").to_str().unwrap());
    let refs = wi.references("area");
    assert_eq!(
        refs.len(),
        2,
        "the def + the call, comment excluded: {refs:?}"
    );
    assert!(
        refs.iter()
            .any(|r| r.file.ends_with("a.loft") && r.line == 1),
        "the definition in a.loft line 1: {refs:?}"
    );
    assert!(
        refs.iter()
            .any(|r| r.file.ends_with("b.loft") && r.line == 2),
        "the call in b.loft line 2: {refs:?}"
    );
    assert!(
        wi.references("nowhere_symbol").is_empty(),
        "an unknown name has no references"
    );
}

#[test]
fn overlay_reflects_unsaved_edits() {
    let dir = temp_ws("refws_overlay");
    let wi = WorkspaceIndex::build(dir.to_str().unwrap());
    // Overlay a.loft with a version that renames `area` -> `zone`.
    let a_path = dir.join("a.loft").to_string_lossy().into_owned();
    let overlays = vec![(a_path, "fn zone(w: integer) -> integer { w }\n".to_string())];

    let area = wi.references_overlaid("area", &overlays);
    assert!(
        area.iter().all(|r| !r.file.ends_with("a.loft")),
        "a.loft's `area` is gone in the overlay: {area:?}"
    );
    assert!(
        area.iter().any(|r| r.file.ends_with("b.loft")),
        "b.loft's `area` call stays (from disk): {area:?}"
    );
    let zone = wi.references_overlaid("zone", &overlays);
    assert!(
        zone.iter().any(|r| r.file.ends_with("a.loft")),
        "the overlaid `zone` appears: {zone:?}"
    );
}

#[test]
fn identifier_at_reads_the_cursor_token() {
    let text = "fn area(w: integer) { w * w }\n";
    assert_eq!(identifier_at(text, 1, 4).as_deref(), Some("area"));
    // A cursor just past a name still anchors on it (the right-edge rule).
    assert_eq!(identifier_at(text, 1, 8).as_deref(), Some("area"));
    // Leading whitespace with nothing to the left → no identifier.
    assert!(identifier_at("   x = 1", 1, 1).is_none());
}
