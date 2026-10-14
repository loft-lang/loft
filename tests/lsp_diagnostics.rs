// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 S2 — the loft-side diagnostics path for loft-lsp.
//
// loft ALREADY carries positioned, coded diagnostics (@I75; @PLN102 arc-E): a
// Parser with the stdlib loaded, then `parse_source(buffer)`, collects each error
// as a `DiagEntry { level, line, col, message, code }` into `p.diagnostics` — no
// stderr, no exit.  The LSP maps `DiagEntry` -> LSP `Diagnostic` at S3.
//
// Recipe: a FRESH stdlib-loaded parser per parse.  A parser cannot be re-parsed:
// loft registers every definition per source (that is how files read each other
// on `use`), so a second `parse_source` on the same parser re-registers and
// conflicts ("Cannot redefine 'main'").  A fresh `parse_dir("default") +
// parse_source(buf)` is the correct model — ~80 ms, within the LSP budget, and it
// cannot carry state across edits.

use loft::diagnostics::Level;
use loft::file_access as fa;
use loft::parser::Parser;

/// The loft-lsp diagnostics recipe: fresh stdlib-loaded parser -> parse buffer ->
/// the error/fatal entries as `(level, line, col, message)`.
fn diagnose(buf: &str) -> Vec<(Level, u32, u32, String)> {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p.parse_source(buf, "buf.loft", false);
    p.diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= Level::Error)
        .map(|e| (e.level, e.line, e.col, e.message.clone()))
        .collect()
}

#[test]
fn clean_buffer_has_no_diagnostics() {
    let e = diagnose("fn main() {\n  print(\"hi\")\n}\n");
    assert!(
        e.is_empty(),
        "a valid program yields no error diagnostics, got {e:?}"
    );
}

#[test]
fn syntax_error_is_correctly_positioned() {
    let e = diagnose("fn main() {\n  let x =\n}\n");
    assert!(!e.is_empty(), "an incomplete `let` must error");
    let (lvl, line, col, msg) = &e[0];
    assert!(*lvl >= Level::Error);
    assert_eq!(*line, 2, "the missing `;` is on line 2, got {e:?}");
    assert!(*col > 0, "carries a column");
    assert!(msg.contains("token"), "names the missing token: {msg}");
}

#[test]
fn unknown_symbol_is_reported_at_the_reference_site() {
    let e = diagnose("fn main() {\n  nope(3)\n}\n");
    assert!(!e.is_empty(), "calling an undefined fn must error");
    let (lvl, line, col, msg) = &e[0];
    assert!(*lvl >= Level::Error);
    assert!(msg.contains("nope"), "names the offending symbol: {msg}");
    // Dogfood finding (b), S2 — FIXED: a deferred/semantic error (an unknown
    // call is type-checked AFTER its arguments are parsed) now reports at the
    // offending identifier's own position, not the cursor's drifted resting
    // place at the enclosing item's terminator.  `nope` is at line 2, col 3
    // (two-space indent) — not line 3 (`}`).  `call()` stamps the identifier's
    // `name_pos` into every "Unknown function" diagnostic via `diagnostic_at!`.
    assert_eq!((*line, *col), (2, 3), "caret sits on `nope`, got {e:?}");
}

#[test]
fn unknown_call_carries_a_structured_suggestion() {
    // Step A: the "did you mean 'X'" fix is machine-readable on the DiagEntry
    // (`suggestion`), not only in the prose — so codeAction needs no parsing.
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p.parse_source("fn main() {\n  nope(3)\n}\n", "buf.loft", false);
    let e = p
        .diagnostics
        .entries()
        .iter()
        .find(|e| e.message.contains("nope"))
        .expect("an unknown-fn diagnostic");
    assert_eq!(
        e.suggestion.as_deref(),
        Some("move"),
        "the suggested replacement is structured: {:?}",
        e.suggestion
    );
}

#[test]
fn superseded_steer_carries_a_structured_suggestion_on_the_call_name() {
    // @PLN102 arc C + step B: a call to a `#superseded "Y"` symbol from owned source
    // steers toward Y, positioned on the call NAME and carrying Y as a structured
    // suggestion — so a codeAction can offer "Change to `new_add`" that replaces the
    // right token (the LSP diagnose path the server pushes).
    // `#superseded` marks the PRECEDING def, so `old_add` is the superseded one.
    let src = "fn old_add(a: integer, b: integer) -> integer { new_add(a, b) }\n\
               #superseded \"new_add\"\n\
               fn new_add(a: integer, b: integer) -> integer { a + b }\n\
               fn main() -> integer { old_add(1, 2) }\n";
    let diags = loft::lsp::diagnose(src, "buf.loft", "default");
    let e = diags
        .entries()
        .iter()
        .find(|e| e.message.contains("is superseded"))
        .expect("the steer fires from owned source");
    // Advice, so an editor renders it as a Hint (severity 4) rather than a problem:
    // the old form keeps working, and gating on it would fail a shipped library's own
    // CI the moment loft supersedes a symbol it uses.
    assert_eq!(e.level, Level::Advice, "the steer is Advice, not a Warning");
    assert_eq!(
        e.suggestion.as_deref(),
        Some("new_add"),
        "the successor is a structured suggestion: {:?}",
        e.suggestion
    );
    // The caret is on the call name `old_add` (line 4), not the drifted statement end.
    assert_eq!(e.line, 4, "steer sits on the call line");
    let line4 = src.lines().nth(3).unwrap();
    assert!(
        line4[e.col as usize - 1..].starts_with("old_add"),
        "the caret is on `old_add`: col {} in {line4:?}",
        e.col
    );
}

// ── @PLN131: the fix reaches the editor, and the code links to its documentation ──

/// A fix with no `edit` must still be VISIBLE in an editor.
///
/// A quick-fix needs an `edit`, and most fixes have none — they name a rewrite the compiler
/// cannot place. Those used to reach the CLI's `--explain` and nowhere else, which became a
/// real regression once the messages stopped carrying their own cure: the editor showed a
/// diagnostic that deliberately no longer said what to write. `relatedInformation` is where
/// an editor shows detail that is not itself a problem, so that is where they go — with the
/// condition, which is the thing a reader affirms.
#[test]
fn a_fix_without_an_edit_still_reaches_the_editor() {
    // An upper-case local — `upper-case-local`, whose one fix names the rewrite ("rename it
    // to lower_case") and cannot place it: the rename touches every reference, not the
    // declaration alone, so no quick-fix can carry it.
    //
    // This was `redundant-coalesce` until loft#1003 gave that one an edit, which is exactly
    // the change the assertion below is written to catch.  `EDIT_BLOCKED` in
    // `tests/e1_code_set.rs` is the list to pick the next fixture from.
    let src = "fn main() { MAX_SIZE = 10; println(\"{MAX_SIZE}\"); }\n";
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("load stdlib");
    p.parse_source(src, "buf.loft", false);
    let entry = p
        .diagnostics
        .entries()
        .iter()
        .find(|e| e.code == Some("upper-case-local"))
        .expect("the upper-case-local lint must fire");
    assert!(
        !entry.fixes.is_empty(),
        "the diagnostic must carry fixes for the editor to show"
    );
    assert!(
        entry.fixes.iter().all(|f| f.edit.is_none()),
        "this fixture is chosen BECAUSE neither fix spells an edit — if that changes, it \
         stops testing the path that has no quick-fix"
    );
    assert!(
        entry
            .fixes
            .iter()
            .all(|f| !f.concept.is_empty() && !f.concept_ref.is_empty()),
        "each fix must name its concept and door, since that is what the editor renders"
    );
}

/// Every coded diagnostic points at documentation that exists.
///
/// `codeDescription` is what turns the concept handle from CLI text into one click. The
/// anchor is asserted against the local file rather than the network: a door onto nothing is
/// what this plan refuses, and a URL nobody checks is exactly how one appears.
#[test]
fn the_code_links_to_an_anchor_that_exists() {
    let doc = fa::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("doc/claude/DIAGNOSTICS.md"),
    )
    .expect("DIAGNOSTICS.md");
    assert!(
        doc.contains("\n## The codes\n"),
        "the `#the-codes` anchor the LSP links to must exist in DIAGNOSTICS.md"
    );
}
