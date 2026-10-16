// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::Parser;

/// loft#1648 — the question `h5_divergence_report` asks about every loaded file, exercised
/// where a parse cannot be raced.
///
/// The report itself is only reachable by making the two passes disagree, which is exactly
/// what nobody can do on demand for this defect — so the decidable half is split out and
/// tested here.  All four answers matter: the two that report, and the two that must stay
/// SILENT, because a stat that cannot answer is not evidence of a change and saying
/// otherwise would send a reader hunting a file nobody touched.
fn tmp(name: &str, body: &str) -> String {
    let p = std::env::temp_dir().join(format!(
        "loft_h5_{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    crate::file_access::write(&p, body).expect("write probe file");
    p.to_string_lossy().into_owned()
}

#[test]
fn a_file_written_after_the_parse_began_is_named() {
    let f = tmp("moved", "before");
    let mut p = Parser::new();
    p.pass_started = Some(std::time::SystemTime::now());
    // mtime resolution is finer than this everywhere loft builds, but a rewrite in the
    // same instant would make the cell vacuous rather than wrong — so step past it.
    std::thread::sleep(std::time::Duration::from_millis(20));
    crate::file_access::write(&f, "after").expect("rewrite probe file");
    assert_eq!(
        p.sources_changed_during_parse(std::iter::once(f.as_str())),
        vec![f.clone()],
        "a source rewritten under the two passes must be named"
    );
    let _ = crate::file_access::remove_file(&f);
}

#[test]
fn a_file_untouched_since_the_parse_began_is_not_named() {
    let f = tmp("still", "unchanged");
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut p = Parser::new();
    p.pass_started = Some(std::time::SystemTime::now());
    assert!(
        p.sources_changed_during_parse(std::iter::once(f.as_str()))
            .is_empty(),
        "an untouched source must not be named — reporting it would point the reader at \
         a file nobody wrote"
    );
    let _ = crate::file_access::remove_file(&f);
}

#[test]
fn a_file_that_cannot_be_stat_ed_is_not_evidence() {
    let mut p = Parser::new();
    p.pass_started = Some(std::time::SystemTime::now());
    assert!(
        p.sources_changed_during_parse(std::iter::once("/nonexistent/loft/h5/probe.loft"))
            .is_empty(),
        "a failed stat says nothing about whether the file moved"
    );
}

#[test]
fn without_a_recorded_start_nothing_is_claimed() {
    let f = tmp("nostart", "x");
    let p = Parser::new();
    assert!(
        p.pass_started.is_none(),
        "a fresh parser has no start instant"
    );
    assert!(
        p.sources_changed_during_parse(std::iter::once(f.as_str()))
            .is_empty(),
        "with no instant to compare against there is no answer, and silence is the honest one"
    );
    let _ = crate::file_access::remove_file(&f);
}
