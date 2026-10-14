// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 — the agent-facing code-intelligence CLI (`loft symbols` / `def` /
// `hover`), the shell frontend to the same `loft::lsp` accessors the LSP server
// gives editors.  Drives the real `loft` binary and asserts the `--json` output.

use loft::json::{self, Parsed};

fn run(args: &[&str]) -> (String, bool) {
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(args)
        .output()
        .expect("spawn loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.success(),
    )
}

fn field_str(v: &Parsed, key: &str) -> Option<String> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).and_then(|(_, _, val)| {
            if let Parsed::Str(s) = val {
                Some(s.clone())
            } else {
                None
            }
        }),
        _ => None,
    }
}

fn as_array(s: &str) -> Vec<Parsed> {
    match json::parse(s).expect("valid json") {
        Parsed::Array(a) => a,
        other => panic!("expected a JSON array, got {other:?}"),
    }
}

#[test]
fn symbols_lists_a_files_top_level_defs() {
    let (stdout, ok) = run(&["symbols", "tests/scripts/06-structs.loft", "--json"]);
    assert!(ok, "`loft symbols` exits 0");
    let items = as_array(&stdout);
    let got: Vec<(String, String)> = items
        .iter()
        .filter_map(|it| Some((field_str(it, "kind")?, field_str(it, "name")?)))
        .collect();
    assert!(
        got.iter().any(|(k, n)| k == "struct" && n == "Pos"),
        "outline includes `struct Pos`: {got:?}"
    );
    assert!(
        got.iter().any(|(k, n)| k == "fn" && n == "main"),
        "outline includes `fn main`: {got:?}"
    );
}

#[test]
fn def_resolves_a_stdlib_method_by_name() {
    // `starts_with` is a METHOD (`t_<LEN>text_starts_with`) — name-lookup that the
    // hover path can't do, but `def` surfaces via its method scan.
    let (stdout, ok) = run(&["def", "starts_with", "--json"]);
    assert!(ok, "`loft def` exits 0 when the symbol resolves");
    let items = as_array(&stdout);
    assert!(!items.is_empty(), "resolves `starts_with`");
    let sig = field_str(&items[0], "signature").unwrap_or_default();
    assert!(sig.contains("starts_with"), "the signature names it: {sig}");
    // `file` is a NATIVE absolute path, so it separates with `\` on Windows —
    // compare separator-agnostically rather than assuming POSIX.  (The `file://`
    // URIs the LSP hands editors are normalised to `/` at the producer; this
    // field deliberately stays native for a CLI consumer.)
    assert!(
        field_str(&items[0], "file")
            .unwrap_or_default()
            .replace('\\', "/")
            .contains("default/"),
        "points into the stdlib source: {items:?}"
    );
}

#[test]
fn def_unknown_symbol_exits_nonzero() {
    let (_stdout, ok) = run(&["def", "zzz_not_a_symbol_anywhere"]);
    assert!(!ok, "an unknown symbol exits non-zero");
}
