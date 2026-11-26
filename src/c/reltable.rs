// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 c

//! `@FR-R-CTable`: a parted function's runs of releases are written as data.
//!
//! A function `(R-CSplit)` writes in parts keeps its locals in a frame struct (`loft_fr`), and
//! each exit of a part releases what the function still owns, one statement per variable
//! (`(R-CTail)` cannot share them: a part cannot jump into another).  A long function's C is
//! then mostly releases — the loft-written elaborator's `assignment_inner` held 163 103 of them
//! in runs of eight or more, from 7 567 distinct statements.
//!
//! This pass reads a function's finished parts and writes each such run as one call,
//! `loft_release_run(cx, loft_fr, <alphabet>, <run>, n)` (`loft_rt.h`): the alphabet holds each
//! distinct release once (its kind, its variable's place in the frame struct, the place of the
//! value a distinct release spares, the slot a work text is parked in), and a run is the list of
//! its statements' indices, a run that ends another run reusing that run's tail.  The loop runs
//! the statements the run held, in their order, so the pass changes where a release is written,
//! never which releases run — the invariant of `(R-CTail)`.
//!
//! A statement joins a run only in one of the four forms the writer emits, each on its own line
//! and naming frame fields alone; anything else ends the run.  `LOFT_NO_C_RELTABLE=1` turns the
//! pass off (the bisect step).

use std::collections::HashMap;
use std::fmt::Write as _;

/// The shortest run written as a table: below it the statements cost less than the loop.
const MIN_RUN: usize = 8;

fn off() -> bool {
    crate::env_once!(std::env::var_os("LOFT_NO_C_RELTABLE").is_some())
}

/// One release, as the alphabet holds it: the kind (`loft_release_run`'s switch), the field it
/// releases, and the witness field (the distinct form) or the parking slot (a work text).
#[derive(Clone, PartialEq, Eq, Hash)]
struct Release {
    kind: u8,
    field: String,
    other: String,
}

/// A frame field `loft_fr->NAME` as `NAME`, else `None`.
fn field(s: &str) -> Option<&str> {
    let name = s.trim().strip_prefix("loft_fr->")?;
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then_some(name)
}

/// The address of a frame field `&loft_fr->NAME` as `NAME`, else `None`.
fn addr(s: &str) -> Option<&str> {
    field(s.trim().strip_prefix('&')?)
}

/// A static's address `&NAME` as `NAME`, else `None`.
fn slot(s: &str) -> Option<&str> {
    let name = s.trim().strip_prefix('&')?;
    (!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then_some(name)
}

/// The release statement on `line`, in one of the four forms the writer emits over frame
/// fields; `None` for any other line, which ends a run.
fn release(line: &str) -> Option<Release> {
    let t = line.trim();
    let (kind, args) = [
        ("loft_free_var(", 0u8),
        ("loft_free_var_distinct(", 1),
        ("loft_wpark_put(", 2),
        ("loft_text_free(", 3),
    ]
    .iter()
    .find_map(|(head, k)| Some((*k, t.strip_prefix(head)?.strip_suffix(");")?)))?;
    let parts: Vec<&str> = args.split(", ").collect();
    if parts.first() != Some(&"loft_fr->cx") {
        return None;
    }
    match (kind, parts.len()) {
        (0 | 3, 2) => Some(Release {
            kind,
            field: addr(parts[1])?.to_string(),
            other: String::new(),
        }),
        (1, 3) => Some(Release {
            kind,
            field: addr(parts[1])?.to_string(),
            other: field(parts[2])?.to_string(),
        }),
        (2, 3) => Some(Release {
            kind,
            field: addr(parts[2])?.to_string(),
            other: slot(parts[1])?.to_string(),
        }),
        _ => None,
    }
}

/// `parts` — the text `(R-CSplit)` wrote for function `fr` (its frame struct `<fr>__frame`) —
/// with every run of [`MIN_RUN`] or more releases written as one `loft_release_run` call over
/// tables placed after the frame struct.  `parts` as it is when the pass is off, finds no run,
/// or cannot place its tables.
pub(crate) fn tables(fr: &str, parts: &str) -> String {
    if off() {
        return parts.to_string();
    }
    let frame = format!("struct {fr}__frame {{");
    let lines: Vec<&str> = parts.split_inclusive('\n').collect();
    // The struct's end: the tables follow it, beside every name they take the place of.
    let Some(open) = lines.iter().position(|l| l.trim_end() == frame) else {
        return parts.to_string();
    };
    let Some(close) = lines[open..].iter().position(|l| l.trim_end() == "};") else {
        return parts.to_string();
    };
    let after_struct = open + close;
    // The runs: (first line, its releases).
    let mut runs: Vec<(usize, Vec<Release>)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let start = i;
        let mut run = Vec::new();
        while i < lines.len()
            && let Some(r) = release(lines[i])
        {
            run.push(r);
            i += 1;
        }
        if run.len() >= MIN_RUN {
            runs.push((start, run));
        }
        if i == start {
            i += 1;
        }
    }
    if runs.is_empty() {
        return parts.to_string();
    }
    crate::rewrite_census::fired("R-CTable", runs.len());
    let mut alphabet: Vec<Release> = Vec::new();
    let mut index: HashMap<Release, usize> = HashMap::new();
    let mut coded: Vec<Vec<usize>> = Vec::with_capacity(runs.len());
    for (_, run) in &runs {
        coded.push(
            run.iter()
                .map(|r| {
                    *index.entry(r.clone()).or_insert_with(|| {
                        alphabet.push(r.clone());
                        alphabet.len() - 1
                    })
                })
                .collect(),
        );
    }
    if alphabet.len() > usize::from(u16::MAX) {
        return parts.to_string();
    }
    // The run store: the longest runs first, and a run that ends one already stored is that
    // one's tail.
    let mut order: Vec<usize> = (0..coded.len()).collect();
    order.sort_by_key(|&k| std::cmp::Reverse(coded[k].len()));
    let mut store: Vec<usize> = Vec::new();
    let mut placed: Vec<(usize, usize)> = vec![(0, 0); coded.len()];
    let mut starts: Vec<(usize, usize)> = Vec::new(); // (offset, length) of each stored run
    for k in order {
        let run = &coded[k];
        let reuse = starts.iter().find_map(|&(o, n)| {
            (n >= run.len() && store[o + n - run.len()..o + n] == run[..])
                .then_some(o + n - run.len())
        });
        let at = reuse.unwrap_or_else(|| {
            let o = store.len();
            store.extend_from_slice(run);
            starts.push((o, run.len()));
            o
        });
        placed[k] = (at, run.len());
    }
    let mut tables_text = String::new();
    let _ = write!(tables_text, "static const loft_rel loft_rels_{fr}[] = {{");
    for (n, r) in alphabet.iter().enumerate() {
        let sep = if n % 4 == 0 { "\n    " } else { " " };
        let other = match r.kind {
            1 => format!("offsetof(struct {fr}__frame, {}), 0", r.other),
            2 => format!("0, &{}", r.other),
            _ => "0, 0".to_string(),
        };
        let _ = write!(
            tables_text,
            "{sep}{{ {}, offsetof(struct {fr}__frame, {}), {other} }},",
            r.kind, r.field
        );
    }
    let _ = write!(
        tables_text,
        "\n}};\nstatic const uint16_t loft_relrun_{fr}[] = {{"
    );
    for (n, v) in store.iter().enumerate() {
        let sep = if n % 24 == 0 { "\n    " } else { "" };
        let _ = write!(tables_text, "{sep}{v},");
    }
    tables_text.push_str("\n};\n");
    // The text: the tables after the struct, each run replaced by its call.
    let mut out = String::with_capacity(parts.len() / 2);
    let mut next_run = 0;
    let mut i = 0;
    while i < lines.len() {
        if next_run < runs.len() && runs[next_run].0 == i {
            let (at, n) = placed[next_run];
            let indent: String = lines[i].chars().take_while(|c| c.is_whitespace()).collect();
            let _ = writeln!(
                out,
                "{indent}loft_release_run(loft_fr->cx, loft_fr, loft_rels_{fr}, loft_relrun_{fr} + {at}, {n});"
            );
            i += runs[next_run].1.len();
            next_run += 1;
            continue;
        }
        out.push_str(lines[i]);
        if i == after_struct {
            out.push_str(&tables_text);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_run_of_releases_becomes_one_call_over_tables() {
        let mut parts = String::from("struct n_f__fr__frame {\n    loft_cx *cx;\n};\n");
        parts.push_str("static LOFT_NOINLINE int n_f__fr__part0(void *loft_frv) {\n");
        for i in 0..6 {
            let _ = writeln!(
                parts,
                "        loft_free_var(loft_fr->cx, &loft_fr->v{i}___ref_1);"
            );
        }
        parts.push_str(
            "        loft_free_var_distinct(loft_fr->cx, &loft_fr->v7___ref_2, loft_fr->v1___ret_1);\n",
        );
        parts.push_str("        loft_wpark_put(loft_fr->cx, &n_f__fr__loft_wp_n_f_3, &loft_fr->v8___work_1);\n");
        parts.push_str("        loft_text_free(loft_fr->cx, &loft_fr->t9);\n");
        parts.push_str("        return 1;\n");
        // a short run stays as written
        parts.push_str(
            "        loft_free_var(loft_fr->cx, &loft_fr->v0___ref_1);\n        return 0;\n}\n",
        );
        let out = tables(n_f(), &parts);
        assert!(
            out.contains("static const loft_rel loft_rels_n_f__fr[] = {"),
            "{out}"
        );
        assert!(out.contains("{ 1, offsetof(struct n_f__fr__frame, v7___ref_2), offsetof(struct n_f__fr__frame, v1___ret_1), 0 },"), "{out}");
        assert!(
            out.contains(
                "{ 2, offsetof(struct n_f__fr__frame, v8___work_1), 0, &n_f__fr__loft_wp_n_f_3 },"
            ),
            "{out}"
        );
        assert!(
            out.contains("        loft_release_run(loft_fr->cx, loft_fr, loft_rels_n_f__fr, loft_relrun_n_f__fr + 0, 9);\n        return 1;"),
            "{out}"
        );
        // the tables follow the struct
        assert!(out.find("};\nstatic const loft_rel").is_some(), "{out}");
        // the short run is untouched
        assert!(out.contains(
            "        loft_free_var(loft_fr->cx, &loft_fr->v0___ref_1);\n        return 0;"
        ));
    }

    #[test]
    fn a_run_that_ends_another_reuses_its_tail() {
        let mut parts = String::from("struct n_g__fr__frame {\n    loft_cx *cx;\n};\n");
        for k in [10, 8] {
            for i in (10 - k)..10 {
                let _ = writeln!(parts, "    loft_free_var(loft_fr->cx, &loft_fr->v{i});");
            }
            parts.push_str("    return 0;\n");
        }
        let out = tables("n_g__fr", &parts);
        // ten stored; the eight are its last eight
        assert!(out.contains("loft_relrun_n_g__fr + 0, 10);"), "{out}");
        assert!(out.contains("loft_relrun_n_g__fr + 2, 8);"), "{out}");
        assert_eq!(
            out.matches("},").count() - 0,
            10,
            "the alphabet holds each release once: {out}"
        );
    }

    fn n_f() -> &'static str {
        "n_f__fr"
    }
}
