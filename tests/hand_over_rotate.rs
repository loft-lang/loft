// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-HandOver`'s rotate clause — the EMISSION pins.  An append of a copy to a vector,
//! then the drop of its oldest element past a depth K by a suffix-keep callee, is emitted as
//! ONE `stores.rotate_into` guarding S and the callee's `__nk` twin, with the statements as
//! written in its `else`; `LOFT_NO_HAND_OVER=1` emits no site.  The cell corpus
//! (`tests/scripts/158-rotate.loft`) says the VALUES hold on both backends, in both switch
//! states and under the falsifiers; this pins what is emitted.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-rotate.loft";

/// Per function: the rotate sites it emits.  `push3s`' S reads the vector, so it has none;
/// `push_n`'s element owns a text, which `rotate_into` declines at run time, not here.
const EXPECTED: &[(&str, usize)] = &[
    ("n_push3", 1),
    ("n_push2", 1),
    ("n_push1", 1),
    ("n_push_n", 1),
    ("n_push3s", 0),
];

/// The `__nk` twins the sites call: each suffix-keep callee once, without its keep.
const TWINS: &[&str] = &["n_drop_old__nk", "n_drop_n__nk"];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_HAND_OVER");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_str()
        .unwrap()
        .to_string()
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_rotate_{}_{tag}.rs", std::process::id()));
    let out_s = out.to_str().unwrap().to_string();
    let status = loft(&["--native-emit", &out_s, &cells()], env);
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
    rust
}

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

fn sites(rust: &str) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
        }
        *map.entry(current.clone()).or_default() += line.matches("//@FR-R-HandOver rotate").count();
    }
    map
}

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let c = cells();
    a.push(&c);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("rotate ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_cells_answer_the_interpreter_in_both_switch_states_under_the_falsifiers() {
    let oracle = run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_HAND_OVER", "1"))] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            let got = run(&["--native"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}

#[test]
fn each_push_emits_exactly_the_sites_predicted() {
    let got = sites(&emit("on", &[]));
    for (name, n) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or(0),
            *n,
            "{name}: rotate sites"
        );
    }
}

#[test]
fn each_twin_is_emitted_once_without_its_keep() {
    let rust = emit("twins", &[]);
    for twin in TWINS {
        assert_eq!(
            rust.matches(&format!("\nfn {twin}(")).count(),
            1,
            "{twin} is emitted exactly once"
        );
        let b = body(&rust, twin);
        assert!(
            !b.contains("keep_vector_range"),
            "{twin} keeps no range — the rotate did:\n{b}"
        );
    }
}

#[test]
fn the_switch_emits_no_site() {
    let rust = emit("off", &[("LOFT_NO_HAND_OVER", "1")]);
    assert_eq!(rust.matches("//@FR-R-HandOver rotate").count(), 0);
    assert_eq!(rust.matches("__nk(").count(), 0);
}
