// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1063 — a stdlib `#rust` body may not name a Rust item the wasm runtime lacks.
//!
//! The stdlib declares ONE surface for every target: `default/*.loft` carries a
//! `#rust"…"` template, and the native generator substitutes it verbatim for
//! `--native`, `--native-wasm` and `--html` alike. The wasm shapes link a runtime rlib
//! built `--no-default-features --features random` (`WasmRuntimeShape::features`), so a
//! template naming an item behind `#[cfg(feature = "mmap")]` compiles on the desktop and
//! fails to LINK on wasm — as a raw `error[E0599]: no associated function named
//! durable_seal found for struct loft::store::Store`, against a struct the loft program
//! never mentions, at a line of generated source the author cannot see.
//!
//! The cure is the shape `Stores::bind_path` already used: a `#[cfg(not(feature = …))]`
//! sibling that ANSWERS on the target without the feature, so one declaration keeps
//! working everywhere and the caller reads the result. This test is what keeps a newly
//! gated body from silently re-opening the hole — it is a property of the sources rather
//! than a list of names, so it also covers items that do not exist yet.
//!
//! Scope and its limit: the scan reads `#[cfg]` attributes textually, so it catches the
//! shape that actually bit (a feature gate directly above the definition, with no
//! fallback sibling). It is a tripwire on the known failure, not a cfg evaluator.

use loft::file_access as fa;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Cargo features present in a wasm shape's runtime rlib, from
/// `WasmRuntimeShape::features` — the single source both wasm shapes are built from.
const WASM_FEATURES: &[&str] = &["random", "wasm-native-threads"];

/// Every `.rs` file under `src/`, read as text.
fn rust_sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if fa::is_dir(&p) {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                // Two source files are not valid UTF-8 throughout; lossy is fine
                // for an attribute scan.
                if let Ok(bytes) = fa::read(&p) {
                    out.push((p, String::from_utf8_lossy(&bytes).into_owned()));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&workspace_root().join("src"), &mut out);
    out
}

/// The `#rust"…"` (and `#iterator"…"`) templates in `default/*.loft`, each paired with
/// the loft function it belongs to — the name a failure has to report, since that is the
/// only part of this an author writes.
fn stdlib_templates() -> Vec<(String, String)> {
    let dir = workspace_root().join("default");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read default/")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "loft"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = fa::read_to_string(&f).expect("read stdlib file");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !(line.starts_with("#rust") || line.starts_with("#iterator")) {
                continue;
            }
            // The declaration this template implements is the nearest `fn` above it;
            // a template belongs to the item it FOLLOWS (see `script.rs` on the
            // post-fix `#rust`), so the search stops at the previous template.
            let mut owner = String::from("<unknown>");
            for prev in lines[..i].iter().rev() {
                if prev.starts_with("#rust") || prev.starts_with("#iterator") {
                    break;
                }
                let t = prev.trim_start();
                let t = t.strip_prefix("pub ").unwrap_or(t);
                if let Some(rest) = t.strip_prefix("fn ") {
                    owner = rest
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("")
                        .to_string();
                    break;
                }
            }
            out.push((owner, (*line).to_string()));
        }
    }
    out
}

/// The `#[cfg(feature = "X")]` names attached to the ~8 lines above `line_idx` and to the
/// enclosing `impl`, i.e. the gates a definition sits behind.
fn feature_gates_above(lines: &[&str], line_idx: usize) -> Vec<String> {
    let mut feats = Vec::new();
    let start = line_idx.saturating_sub(8);
    for l in &lines[start..line_idx] {
        let t = l.trim_start();
        if !t.starts_with("#[cfg") {
            continue;
        }
        // A `not(feature = …)` gate is the FALLBACK arm — it is what makes the item
        // present on a target lacking the feature, so it never counts as a gate.
        if t.contains("not(feature") {
            continue;
        }
        let mut rest = t;
        while let Some(p) = rest.find("feature = \"") {
            rest = &rest[p + "feature = \"".len()..];
            if let Some(end) = rest.find('"') {
                feats.push(rest[..end].to_string());
                rest = &rest[end..];
            } else {
                break;
            }
        }
    }
    feats
}

/// Does `item` have at least one definition in `src/` that a wasm build keeps?
/// `None` when no definition is found at all (the item is out of scope — it may come
/// from a dependency or be a variant/associated const the scan does not model).
fn wasm_reachable(sources: &[(PathBuf, String)], item: &str) -> Option<bool> {
    let mut found_any = false;
    for (_, text) in sources {
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let t = l.trim_start();
            let is_def = [
                "fn ",
                "pub fn ",
                "const ",
                "pub const ",
                "static ",
                "pub static ",
            ]
            .iter()
            .any(|k| {
                t.strip_prefix(k).is_some_and(|r| {
                    r.split(|c: char| !(c.is_alphanumeric() || c == '_')).next() == Some(item)
                })
            });
            if !is_def {
                continue;
            }
            found_any = true;
            let gates = feature_gates_above(&lines, i);
            if gates.iter().all(|f| WASM_FEATURES.contains(&f.as_str())) {
                return Some(true);
            }
        }
    }
    found_any.then_some(false)
}

/// Every `crate::`-qualified item a stdlib template names must survive the wasm build.
///
/// @C87 — the `#rust"…"` template path is kept: this walks every stdlib template, so a change
/// that retired the path for per-Op emitters fails here.
///
/// The failure this prevents is not subtle once seen, and impossible to see before:
/// `store_durable_seal` ran on `--interpret` and `--native` and made `--native-wasm`
/// fail inside rustc, naming `loft::store::Store`.
#[test]
fn stdlib_rust_templates_resolve_on_wasm() {
    let sources = rust_sources();
    assert!(
        sources.len() > 50,
        "expected to scan loft's src/ tree, found {} files",
        sources.len()
    );
    // Positive control: the scan must be able to SEE a feature gate. `open_durable` is
    // mmap-only with no fallback sibling — if this ever reads as wasm-reachable the
    // detector has gone blind and a green result below means nothing.
    assert_eq!(
        wasm_reachable(&sources, "open_durable"),
        Some(false),
        "positive control failed: the cfg scan no longer sees `#[cfg(feature = \"mmap\")]`"
    );

    let mut offenders: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (owner, body) in stdlib_templates() {
        let mut rest = body.as_str();
        while let Some(p) = rest.find("crate::") {
            rest = &rest[p + "crate::".len()..];
            let path: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            let item = path.rsplit("::").next().unwrap_or("").to_string();
            if item.is_empty() {
                continue;
            }
            if wasm_reachable(&sources, &item) == Some(false) {
                offenders
                    .entry(owner.clone())
                    .or_default()
                    .push(format!("crate::{path}"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "stdlib `#rust` template(s) name a Rust item the wasm runtime rlib does not have \
         (built --no-default-features --features random). On `--native-wasm` / `--html` \
         this surfaces as a raw rustc E0599 against loft's own crate (loft#1063).\n\
         Give the item a `#[cfg(not(feature = \"…\"))]` sibling that answers on a target \
         without the feature — the shape `Stores::bind_path` and \
         `Store::durable_seal` use.\n\n{}",
        offenders
            .iter()
            .map(|(fnname, items)| format!("  {fnname}: {}", items.join(", ")))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
