// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The lifetime check (`@PLN228`): ONE opt-in analysis of a compiled program's store lifetimes,
//! with one report.  `LOFT_LIFETIME_CHECK=<file>` (`-` for standard error) runs, at the end of
//! the program's close, the store records' checks (STORE_RECORDS.md § The checks: leak, double
//! end, re-free, use after end, wrong end) and the scope check (`@FR-B-Scope`), and writes:
//!
//! - one line per problem, tab-separated: the program file of the function, the function, the
//!   rule it breaks, the kind, the site (`line N`, or `-`), and a one-line reason;
//! - a last line `# N problem(s) in M function(s) (kind=count, …)`, or `# no problems`.
//!
//! The lines are sorted, so two runs of one program print the same report and a change shows as
//! the lines it adds or removes.  Any compile reaches it, `--emit-image` included, which runs no
//! program: a gate that only compiles can ask it of every program.  It decides nothing the
//! compiler acts on, and it is off unless asked for.

use crate::data::Data;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One problem of the report.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    file: String,
    function: String,
    line: u32,
    kind: String,
    rule: &'static str,
    reason: String,
}

/// The formal rule each kind of problem breaks.
fn rule_of(kind: &str) -> &'static str {
    match kind {
        "leak" => "@FR-O-Complete",
        "re-free" => "@FR-H-FreeTwice",
        "use-after-end" => "@FR-H-Read",
        "out-of-scope" => "@FR-B-Scope",
        "copied-lease" => "@FR-H-Lease",
        // A double end and a wrong end both break the single owner's one release.
        _ => "@FR-O-Borrow",
    }
}

/// A store-records finding (`store-<kind> [store …] <reason> (line N)`) split into its kind,
/// its reason and its line (0 when it names none).
fn parse_record(finding: &str) -> (&str, &str, u32) {
    let rest = finding.strip_prefix("store-").unwrap_or(finding);
    let (kind, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    match rest.rsplit_once(" (line ") {
        Some((r, tail)) => match tail.strip_suffix(')').and_then(|n| n.parse().ok()) {
            Some(n) => (kind, r, n),
            None => (kind, rest, 0),
        },
        None => (kind, rest, 0),
    }
}

/// A store-records finding of definition `d` as a row.
fn record_row(data: &Data, d: u32, finding: &str) -> Row {
    let (kind, reason, line) = parse_record(finding);
    let def = data.def(d);
    Row {
        file: def.variables.file.clone(),
        function: def.name().to_string(),
        line,
        kind: kind.to_string(),
        rule: rule_of(kind),
        reason: reason.replace('\t', " "),
    }
}

/// The observer entry point, at the end of `close_program`.
pub fn observe(data: &Data) {
    let Some(out) = std::env::var_os("LOFT_LIFETIME_CHECK") else {
        return;
    };
    let mut rows: Vec<Row> = crate::store_records::findings(data)
        .iter()
        .map(|(d, f)| record_row(data, *d, f))
        .collect();
    for (d, f) in crate::scope_check::findings(data) {
        let def = data.def(d);
        rows.push(Row {
            file: def.variables.file.clone(),
            function: def.name().to_string(),
            line: f.line,
            kind: "out-of-scope".to_string(),
            rule: rule_of("out-of-scope"),
            reason: f.reason(&def.variables),
        });
    }
    rows.sort();
    rows.dedup();
    let mut text = String::new();
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut functions: Vec<(&str, &str)> = Vec::new();
    for r in &rows {
        let site = if r.line == 0 {
            "-".to_string()
        } else {
            format!("line {}", r.line)
        };
        let _ = writeln!(
            text,
            "{}\t{}\t{}\t{}\t{site}\t{}",
            r.file, r.function, r.rule, r.kind, r.reason
        );
        *kinds.entry(&r.kind).or_default() += 1;
        if !functions.contains(&(&r.file, &r.function)) {
            functions.push((&r.file, &r.function));
        }
    }
    if rows.is_empty() {
        text += "# no problems\n";
    } else {
        let per: Vec<String> = kinds.iter().map(|(k, n)| format!("{k}={n}")).collect();
        let _ = writeln!(
            text,
            "# {} problem(s) in {} function(s) ({})",
            rows.len(),
            functions.len(),
            per.join(", ")
        );
    }
    if out == "-" {
        eprint!("{text}");
    } else if let Err(e) = crate::file_access::write(std::path::Path::new(&out), &text) {
        eprintln!(
            "loft: LOFT_LIFETIME_CHECK: cannot write {}: {e}",
            out.to_string_lossy()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_record, rule_of};

    #[test]
    fn each_kind_names_the_rule_it_breaks() {
        assert_eq!(rule_of("leak"), "@FR-O-Complete");
        assert_eq!(rule_of("re-free"), "@FR-H-FreeTwice");
        assert_eq!(rule_of("use-after-end"), "@FR-H-Read");
        assert_eq!(rule_of("out-of-scope"), "@FR-B-Scope");
        assert_eq!(rule_of("double-end"), "@FR-O-Borrow");
        assert_eq!(rule_of("wrong-end"), "@FR-O-Borrow");
    }

    /// A record's finding splits into its kind, its reason and its line.
    #[test]
    fn a_record_finding_becomes_a_row() {
        assert_eq!(
            parse_record(
                "store-leak [store #4, copied] a path reaches a return with the store live (line 46)"
            ),
            (
                "leak",
                "[store #4, copied] a path reaches a return with the store live",
                46
            )
        );
        assert_eq!(
            parse_record("store-wrong-end [store #0, minted] no line here"),
            ("wrong-end", "[store #0, minted] no line here", 0)
        );
    }
}
