// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-TableHandle` — the C runtime's table handle answers only while its store's table
//! generation is the one it was resolved under, so every runtime path that changes a table's
//! shape or replaces a store's content must give the store a new generation.  A path that
//! forgets is silent (a stale handle hashes with an old seed, reads an old directory), so two
//! source scans make forgetting loud, and the emission pins check the writer's one decision.
//! Design: bench/portal/analysis/table-handle.md; cell tests/scripts/159-table-handle.loft.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn runtime_headers() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/c");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).expect("src/c") {
        let p = e.expect("entry").path();
        if p.extension().is_some_and(|x| x == "h") {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, fa::read_to_string(&p).expect("read header")));
        }
    }
    out
}

/// Every function definition in `src`: the offset its header starts at, and its name.  A header
/// may run over several lines; it is a definition when a `{` ends it before any `;`.
fn definitions(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let starts = line.starts_with("static ")
            || line.starts_with("LOFT_HOT_INLINE ")
            || line.starts_with("LOFT_COLD ")
            || line.starts_with("LOFT_TEXT_INLINE ");
        if starts && line.contains('(') {
            let rest = &src[at..];
            let brace = rest.find('{');
            let semi = rest.find(';');
            if let Some(b) = brace
                && semi.is_none_or(|s| b < s)
            {
                let name = line
                    .split('(')
                    .next()
                    .and_then(|h| h.split_whitespace().last())
                    .unwrap_or("?")
                    .trim_start_matches('*')
                    .to_string();
                out.push((at, name));
            }
        }
        at += line.len();
    }
    out
}

/// The C function a byte offset of `src` lies in.
fn enclosing_function(src: &str, at: usize) -> String {
    definitions(src)
        .into_iter()
        .take_while(|(start, _)| *start < at)
        .last()
        .map_or_else(|| "?".to_string(), |(_, n)| n)
}

/// The body of function `name` in `src` (its header through the `}` at column 0).
fn function_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = definitions(src)
        .into_iter()
        .find(|(_, n)| n == name)
        .map(|(s, _)| s)
        .unwrap_or_else(|| panic!("no definition of {name}"));
    let rest = &src[start..];
    &rest[..rest.find("\n}\n").map_or(rest.len(), |i| i + 3)]
}

/// The table header fields a handle caches have one writer, `loft_h_set_header`, which gives
/// the store a new generation.
#[test]
fn only_the_header_writer_writes_a_table_header() {
    let mut raw = Vec::new();
    for (name, src) in runtime_headers() {
        for (at, _) in src.match_indices("loft_set_u32(") {
            let call = &src[at..at + src[at..].find(';').unwrap_or(0)];
            let fields = ["LOFT_H_STRIDE_FLD", "LOFT_H_SEED_FLD", "LOFT_AR_DIR_FLD"];
            // the field is the call's third argument
            let third = call.split(',').nth(2).unwrap_or("").trim();
            if fields.iter().any(|f| third.starts_with(f))
                && enclosing_function(&src, at) != "loft_h_set_header"
            {
                raw.push(format!(
                    "{name}: {} in {}",
                    call,
                    enclosing_function(&src, at)
                ));
            }
        }
    }
    assert!(
        raw.is_empty(),
        "a table header field written outside `loft_h_set_header` — a table handle would not \
         see the change (`@FR-R-TableHandle`):\n{}",
        raw.join("\n")
    );
}

/// Every function that gives a store a new buffer either makes the store's content anew
/// through a path that takes a new table generation, keeps the content as it was, or releases
/// the store.  A new site fails here until it is classified.
#[test]
fn every_store_content_replacement_gives_a_new_table_generation() {
    // Calls that give the store a new generation (`loft_h_reshaped` itself or a path through it).
    const RESHAPES: [&str; 5] = [
        "loft_h_reshaped(",
        "loft_st_init(",
        "loft_st_resync(",
        "loft_store_new(",
        "loft_store_make(",
    ];
    // Growth and shrinking that keep every byte the store held.
    const KEEPS: [&str; 3] = ["loft_map_resize", "loft_resize_store", "loft_st_shrink_to"];
    // The store is gone after it: its slot's next store is made through `loft_store_make`.
    const RELEASES: [&str; 2] = ["loft_st_close_mapped", "loft_store_drop"];
    let mut unclassified = Vec::new();
    for (name, src) in runtime_headers() {
        for pat in ["->buf = ", ".buf = "] {
            for (at, _) in src.match_indices(pat) {
                let f = enclosing_function(&src, at);
                if KEEPS.contains(&f.as_str()) || RELEASES.contains(&f.as_str()) {
                    continue;
                }
                let body = function_body(&src, &f);
                if !RESHAPES.iter().any(|r| body.contains(r)) {
                    unclassified.push(format!("{name}: {f}"));
                }
            }
        }
    }
    unclassified.sort();
    unclassified.dedup();
    assert!(
        unclassified.is_empty(),
        "a store's buffer replaced without a new table generation (`loft_h_reshaped`) — a table \
         handle on it would answer from the old content:\n{}",
        unclassified.join("\n")
    );
    // The content swap exchanges two stores whole, buffers included, without assigning one.
    let rt = runtime_headers()
        .into_iter()
        .find(|(n, _)| n == "loft_rt.h")
        .unwrap()
        .1;
    assert!(
        function_body(&rt, "loft_swap_in").contains("loft_h_reshaped("),
        "loft_swap_in exchanges two stores' content: both take a new table generation"
    );
}

const CELL: &str = "tests/scripts/159-table-handle.loft";

fn emit_c(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELL);
    let out =
        std::env::temp_dir().join(format!("loft_table_handle_{}_{tag}.c", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-c")
        .arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_NO_TABLE_HANDLE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-c --native-emit");
    assert!(
        fa::exists(&out),
        "no C emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let c = fa::read_to_string(&out).expect("read the emitted C");
    let _ = fa::remove_file(&out);
    c
}

/// The body of an emitted program function (`n_<name>__fr`).
fn emitted<'a>(c: &'a str, name: &str) -> &'a str {
    function_body(c, &format!("{name}__fr"))
}

#[test]
fn the_keyed_operations_on_one_place_share_one_handle() {
    let c = emit_c("on", &[]);
    // bench 23's `put` shape: the lookup and the insert of linked records
    let add = emitted(&c, "n_add");
    assert_eq!(
        add.matches("loft_hh hh").count(),
        1,
        "add: one handle\n{add}"
    );
    assert!(
        add.contains("loft_hh_find_i64(&hh") && add.contains("loft_hh_insert(&hh"),
        "{add}"
    );
    // the table emptied and refilled between uses: lookups and the insert on one handle
    let refill = emitted(&c, "n_refill");
    assert_eq!(
        refill.matches("loft_hh hh").count(),
        1,
        "refill: one handle\n{refill}"
    );
    assert!(
        refill.matches("loft_hh_find_i64(&hh").count() >= 2,
        "{refill}"
    );
    // owned elements live in an inline table the handle does not cover: the plain calls
    let evict = emitted(&c, "n_evict_put");
    assert!(
        !evict.contains("loft_hh "),
        "evict_put's elements are owned: no handle\n{evict}"
    );
    assert!(evict.contains("loft_hash_remove_i64("), "{evict}");
    // a generator's resume body is assembled from its frame: it takes no handle
    let resume = &c[c
        .find("(void *frp, void *out) {\n    loft_gen")
        .expect("a generator resume body")..];
    let resume = &resume[..resume.find("\n}\n").unwrap_or(resume.len())];
    assert!(
        resume.contains("loft_h_find_i64(") && !resume.contains("loft_hh_"),
        "the generator's keyed lookups keep the plain calls\n{resume}"
    );
    // a function with no keyed operation declares none
    assert!(
        !emitted(&c, "n_push").contains("loft_hh "),
        "push makes no keyed operation"
    );
}

#[test]
fn the_switch_restores_the_plain_calls() {
    let c = emit_c("off", &[("LOFT_NO_TABLE_HANDLE", "1")]);
    let program = &c[c.find("struct lt_0 {").unwrap_or(0)..];
    assert!(
        !program.contains("loft_hh hh"),
        "LOFT_NO_TABLE_HANDLE=1 declares no handle"
    );
    assert!(
        emitted(&c, "n_add").contains("loft_hash_insert("),
        "the switch emits the plain insert as before"
    );
}
