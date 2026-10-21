// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN199 G0b-2 — the code a library artifact carries does not depend on how the program
//! around it numbers types and functions.  Every program of the corpus is generated as a
//! library artifact twice: alone, and behind a prefix that declares structs, an enum and
//! functions first, so each of its types and functions takes another number.  The two texts
//! must be the same outside `init` (the program's own type table): a literal id left in the
//! code shows up as the first line that differs.

mod common;
use common::cached_default;
use loft::file_access as fa;

const SHIFT: &str = "struct ShiftA { a: text, b: float, c: boolean }\n\
                     struct ShiftB { d: vector<ShiftA>, e: integer }\n\
                     enum ShiftE { One, Two }\n\
                     fn shift_a(x: integer) -> integer { x + 1 }\n\
                     fn shift_b(x: integer) -> integer { x * 2 }\n";

/// The library code of `src`, parsed after `prefix`, with `init` cut out.
fn code_of(prefix: &str, src: &str, name: &str) -> Option<String> {
    let (data, db) = cached_default();
    let mut p = loft::parser::Parser::new();
    p.data = data;
    p.database = db;
    if !prefix.is_empty() {
        p.parse_str(prefix, "shift", false);
    }
    let first = p.data.definitions();
    p.parse_str(src, name, false);
    if p.diagnostics.lines().iter().any(|l| l.starts_with("Error")) {
        return None;
    }
    loft::scopes::check(&mut p.data, &mut p.database);
    let mut state = loft::state::State::new(p.database);
    loft::compile::byte_code(&mut state, &mut p.data);
    let entry: Vec<u32> = (first..p.data.definitions())
        .filter(|&d| {
            let def = p.data.def(d);
            def.def_type() == loft::data::DefType::Function
                && def.name().starts_with("n_")
                && !def.name().starts_with("n_shift_")
                && *def.code() != loft::data::Value::Null
        })
        .collect();
    if entry.is_empty() {
        return None;
    }
    let src = loft::native_lib::library_code_rs(&p.data, &state.database, &entry);
    let start = src.find("\nfn init(")?;
    let end = start + src[start..].find("\n}\n")? + 3;
    Some(without_comments(&format!(
        "{}{}",
        &src[..start],
        &src[end..]
    )))
}

/// `src` without its `/* … */` and `// …` comments: the generator annotates blocks with
/// their loft type, ids included, and a comment is not what the artifact runs.
fn without_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find(['/', '"']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with('"') {
            // A string literal: copied whole, escapes included.
            let mut end = 1;
            let bytes = tail.as_bytes();
            while end < bytes.len() && bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let end = (end + 1).min(tail.len());
            out.push_str(&tail[..end]);
            rest = &tail[end..];
        } else if let Some(body) = tail.strip_prefix("/*") {
            rest = body.find("*/").map_or("", |e| &body[e + 2..]);
        } else if tail.starts_with("//") {
            rest = tail.find('\n').map_or("", |e| &tail[e..]);
        } else {
            out.push('/');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Where `a` and `b` first differ: the text around the first differing character of the first
/// differing line, as `…before⟨a | b⟩`.
fn first_difference(a: &str, b: &str) -> String {
    let Some((x, y)) = a.lines().zip(b.lines()).find(|(x, y)| x != y) else {
        return "the texts differ in length".to_string();
    };
    let at = x
        .char_indices()
        .zip(y.chars())
        .find(|((_, c), d)| c != d)
        .map_or(x.len().min(y.len()), |((i, _), _)| i);
    let from = x[..at].char_indices().rev().nth(70).map_or(0, |(i, _)| i);
    let tail = |s: &str| s[at..].chars().take(12).collect::<String>();
    format!("{}⟨{} | {}⟩", &x[from..at], tail(x), tail(y))
}

#[test]
fn library_code_does_not_name_the_programs_numbers() {
    let mut files: Vec<String> = fa::read_dir("tests/scripts")
        .expect("tests/scripts")
        .into_iter()
        .map(|e| e.os_spelling().to_string_lossy().into_owned())
        .filter(|p| p.ends_with(".loft"))
        .collect();
    files.sort();
    files.extend(
        common::corpus::suite_programs()
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned()),
    );
    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk = files.len().div_ceil(threads).max(1);
    let results: Vec<(usize, Vec<String>)> = std::thread::scope(|s| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    let (mut compared, mut differ) = (0, Vec::new());
                    for f in part {
                        let Ok(src) = fa::read_to_string(f) else {
                            continue;
                        };
                        let alone = std::panic::catch_unwind(|| code_of("", &src, "lib"));
                        let shifted = std::panic::catch_unwind(|| code_of(SHIFT, &src, "lib"));
                        let (Ok(Some(a)), Ok(Some(b))) = (alone, shifted) else {
                            continue;
                        };
                        compared += 1;
                        if a != b {
                            differ.push(format!("{f}: {}", first_difference(&a, &b)));
                        }
                    }
                    (compared, differ)
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let compared: usize = results.iter().map(|r| r.0).sum();
    let differ: Vec<String> = results.into_iter().flat_map(|r| r.1).collect();
    assert!(
        compared > 100,
        "only {compared} corpus programs generated — the guard reads nothing"
    );
    assert!(
        differ.is_empty(),
        "{} of {compared} programs generate library code that names this program's numbers:\n{}",
        differ.len(),
        differ.join("\n")
    );
}
