// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! What counts as a doc comment, and what does not.
//!
//! The library review reported 334 published `pub fn` as "carrying no doc comment".  Almost
//! all were documented and the reader could not see them: it accepted only `///`, while the
//! sources overwhelmingly use `//` (936 against 54 across the distribution), and it stopped
//! at a blank line, which those sources routinely leave above a declaration.
//!
//! ⚠ The negative cells are the ones that matter.  Widening a reader is easy to overdo, and
//! the first attempt here did: catching only `---` headings let `palette_r`'s documentation
//! begin *"── Palette ───…"*, because a second heading spelling uses box-drawing rules.  A
//! heading swept into a block becomes the first sentence of one arbitrary member's docs, and
//! a block of only blank comment lines would count as documentation that is not there.

/// The plain `loft def` rendering: the signature line, then any doc lines indented under it,
/// then a `→ file:line`.  Read as text rather than JSON so this test needs no extra crate.
fn def_output(name: &str, file: &str) -> String {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(["def", name, file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run loft def");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The doc lines of the FIRST entry — everything between its signature and its `→` location.
fn first_doc(name: &str, file: &str) -> Vec<String> {
    let text = def_output(name, file);
    let mut doc = Vec::new();
    let mut in_entry = false;
    for line in text.lines() {
        if line.starts_with("fn ") {
            if in_entry {
                break;
            }
            in_entry = true;
        } else if in_entry {
            if line.trim_start().starts_with('\u{2192}') {
                break;
            }
            if !line.trim().is_empty() {
                doc.push(line.trim().to_string());
            }
        }
    }
    doc
}

#[test]
fn a_plain_slash_slash_comment_is_documentation() {
    let doc = first_doc("abs", "default/01_code.loft");
    assert!(
        doc.iter().any(|l| l.contains("Absolute value")),
        "`//` above a `pub fn` is how these sources document; got {doc:?}"
    );
}

#[test]
fn a_blank_line_does_not_end_the_block() {
    // `exp` is written heading / comment / blank / `pub fn`.
    let doc = first_doc("exp", "default/01_code.loft");
    assert!(
        doc.iter().any(|l| l.contains("2.71828")),
        "a blank line between the comment and the declaration must not drop it; got {doc:?}"
    );
}

#[test]
fn a_section_heading_is_not_the_first_sentence_of_a_doc() {
    for (name, file) in [
        ("exp", "default/01_code.loft"), // `// --- … ---`
        (
            "palette_r",
            "lib/audience_crystal/src/audience_crystal.loft",
        ), // `// ── … ──`
    ] {
        let doc = first_doc(name, file);
        assert!(
            !doc.first()
                .is_some_and(|l| l.contains("---") || l.contains('─')),
            "{name}'s doc begins with a section heading: {doc:?}"
        );
    }
}

#[test]
fn a_function_with_no_comment_stays_undocumented() {
    // `palette_g` sits directly below a `}` — the true negative the widening must preserve.
    let doc = first_doc(
        "palette_g",
        "lib/audience_crystal/src/audience_crystal.loft",
    );
    assert!(
        doc.is_empty(),
        "a declaration with no comment above it must report none; got {doc:?}"
    );
}
