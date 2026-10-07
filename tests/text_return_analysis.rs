// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN85 text-return analysis framework — verify the SHADOW analysis beside
//! the tests.  The framework (`Parser::classify_text_return`) is the single
//! selector that will replace the stacked per-shape promotion predicates (2d
//! native-call, 3a view-of-local, 3b user-call, 3c if/match arm) and encode the
//! p281 borrow exclusion.  It is NOT yet wired to codegen — this test proves its
//! verdicts are correct so it can be extended to cover more cases before the
//! switch.
//!
//! The corpus (`framework/corpus.loft`) carries one fn per shape with the
//! known-correct verdict in a `// VERDICT: <kind>` comment above the `fn`.
//! Running the binary with `LOFT_TRA_DUMP=1` prints `TRA <fn> => <kind>` per
//! text-returning fn (read-only; no codegen change).  This test diffs the two.

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn corpus_path() -> std::path::PathBuf {
    workspace_root().join(
        "doc/claude/plans/85-store-lifetime-retirement/probes/text-tail-return/framework/corpus.loft",
    )
}

/// Parse `// VERDICT: X` / `fn NAME` pairs from the corpus.
fn expected_verdicts(corpus: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pending: Option<String> = None;
    for line in corpus.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("// VERDICT:") {
            pending = Some(rest.trim().to_string());
        } else if let Some(rest) = t.strip_prefix("fn ")
            && let Some(v) = pending.take()
        {
            // fn name = up to the first '<' (generics) or '(' (params).
            let name: String = rest
                .chars()
                .take_while(|c| *c != '<' && *c != '(')
                .collect();
            out.push((name.trim().to_string(), v));
        }
    }
    out
}

#[test]
fn text_return_analysis_matches_corpus() {
    let corpus = corpus_path();
    let text = fa::read_to_string(&corpus).expect("read corpus.loft");
    let expected = expected_verdicts(&text);
    assert!(
        expected.len() >= 15,
        "corpus should annotate many cases; found {}",
        expected.len()
    );

    // LOFT_TRA_DUMP names a FILE the compiler appends verdicts to — a
    // deterministic channel (loft's stderr races with `process::exit` and
    // truncates unreliably).
    let dump = std::env::temp_dir().join(format!("loft_tra_{}.txt", std::process::id()));
    let _ = fa::remove_file(&dump);
    let out = Command::new(loft_bin())
        .env("LOFT_TRA_DUMP", &dump)
        // Force a fresh parse: the program cache is content-keyed, so a warm hit
        // would skip the parse (and the dump).
        .env("LOFT_NO_CACHE", "1")
        .arg("--interpret")
        .arg(&corpus)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft binary");
    assert!(
        out.status.success(),
        "corpus should run cleanly; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dumped = fa::read_to_string(&dump).unwrap_or_default();
    let _ = fa::remove_file(&dump);

    // Collect `TRA <fn> => <verdict>` lines, stripping a leading `n_` so generic
    // monomorphs (n_f_*) match the source name.
    let mut actual: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in dumped.lines() {
        if let Some(rest) = line.strip_prefix("TRA ")
            && let Some((fun, verdict)) = rest.split_once(" => ")
        {
            let fun = fun.strip_prefix("n_").unwrap_or(fun);
            actual.insert(fun.to_string(), verdict.trim().to_string());
        }
    }

    let mut failures = Vec::new();
    for (fun, exp) in &expected {
        match actual.get(fun) {
            Some(got) if got == exp => {}
            Some(got) => failures.push(format!("{fun}: expected {exp}, got {got}")),
            None => failures.push(format!("{fun}: expected {exp}, got <none>")),
        }
    }
    assert!(
        failures.is_empty(),
        "text-return verdicts diverged from corpus:\n  {}\n--- full TRA dump ---\n{dumped}",
        failures.join("\n  ")
    );
}
