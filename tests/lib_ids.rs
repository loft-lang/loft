// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN199 G0b-2 — the code a library artifact carries does not depend on how the program
//! around it numbers types and functions.  Every program of the corpus is generated as a
//! library artifact twice: alone, and behind a prefix that declares structs, an enum,
//! functions and generics first, so each of its types, functions and type variables takes
//! another number.  The two texts
//! must be the same: a literal id left in the code shows up as the first line that differs.

mod common;
use common::cached_default;
use loft::file_access as fa;

const SHIFT: &str = "struct ShiftA { a: text, b: float, c: boolean }\n\
                     struct ShiftB { d: vector<ShiftA>, e: integer }\n\
                     struct ShiftG<T> { g: T }\n\
                     enum ShiftE { One, Two }\n\
                     fn shift_a(x: integer) -> integer { x + 1 }\n\
                     fn shift_b(x: integer) -> integer { x * 2 }\n\
                     fn shift_t<T>(x: T) -> vector<T> { [x] }\n\
                     fn shift_u<U>(x: U) -> U { x }\n";

/// The library code of `src`, parsed after `prefix`.
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
    Some(without_comments(&src))
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

/// The corpus, then the suite, sorted: the same list in every shard's process.
fn corpus() -> Vec<String> {
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
    files
}

const SHARDS: usize = 8;

/// Every `SHARDS`-th program from `shard` on.  A shard runs in its own process (nextest):
/// the parser keeps process-wide state, so two parses on two threads of one process are not
/// the same parse twice.
fn check_shard(shard: usize) {
    let (mut compared, mut differ) = (0, Vec::new());
    for f in corpus().iter().skip(shard).step_by(SHARDS) {
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
    assert!(
        compared > 20,
        "only {compared} programs generated — the guard reads nothing"
    );
    assert!(
        differ.is_empty(),
        "{} of {compared} programs generate library code that names this program's numbers:\n{}",
        differ.len(),
        differ.join("\n")
    );
}

macro_rules! shards {
    ($($name:ident = $k:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                check_shard($k);
            }
        )*
    };
}

shards!(
    library_code_does_not_name_the_programs_numbers_0 = 0,
    library_code_does_not_name_the_programs_numbers_1 = 1,
    library_code_does_not_name_the_programs_numbers_2 = 2,
    library_code_does_not_name_the_programs_numbers_3 = 3,
    library_code_does_not_name_the_programs_numbers_4 = 4,
    library_code_does_not_name_the_programs_numbers_5 = 5,
    library_code_does_not_name_the_programs_numbers_6 = 6,
    library_code_does_not_name_the_programs_numbers_7 = 7,
);
