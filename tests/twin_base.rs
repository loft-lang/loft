// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Base`'s twin clause — the EMISSION pins.  A callee twin (`__inv`, `@FR-R-Inputs`)
//! takes the element BASE of each header it is handed (`__ib_k: *const u8`), a view of that
//! path inside the twin shares it (`__vb_N`), and the twin's fused reads and writes take the
//! one-load form (`get_elem_at` / `vec_set_at`) instead of resolving the store per element.
//! A caller whose loop holds a base passes it; one whose loop grows a store derives one from
//! the held header AT the call.  `LOFT_NO_TWIN_BASE=1` restores the header-only twin.  The
//! cell corpus (`tests/scripts/157-twin-base.loft`) says the VALUES hold on both backends,
//! in both switch states and under the falsifiers; this pins what is emitted and runs it.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-twin-base.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_TWIN_BASE")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_twin_base_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            src.to_str().unwrap(),
        ],
        env,
    );
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
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

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_cells_hold_on_both_backends_in_both_switch_states_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_TWIN_BASE", "1"))] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ("LOFT_HOIST_VERIFY", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
}

#[test]
fn a_twin_takes_a_base_beside_each_header_and_reads_and_writes_through_it() {
    let rust = emit("on", &[]);
    let get = body(&rust, "t_4TbCv_tb_get__inv");
    assert!(
        get.contains("__ih_0: vector::VecHeader, __ib_0: *const u8"),
        "the read twin takes no base:\n{get}"
    );
    // The view `d = &self.data` shares the twin's base under its own key …
    assert!(
        get.contains("let __vb_") && get.contains("= __ib_0; //@FR-R-Base view base"),
        "the view inside the twin does not share the base:\n{get}"
    );
    // … and the element read is the one-load form.
    assert!(
        get.contains("get_elem_at::<i64, false>(&__vh_"),
        "the read twin still resolves the store per element:\n{get}"
    );
    assert!(!get.contains("get_elem_hoisted"), "{get}");
    let set = body(&rust, "t_4TbCv_tb_set__inv");
    assert!(
        set.contains("vec_set_at::<i64, false>("),
        "the write twin still resolves the store per element:\n{set}"
    );
    assert!(!set.contains("vec_set_hoisted"), "{set}");
    // Two headers, two bases, in the headers' order.
    let masked = body(&rust, "t_4TbCv_tb_masked__inv");
    assert!(
        masked.contains("__ih_1: vector::VecHeader, __ib_0: *const u8, __ib_1: *const u8"),
        "the two-header twin does not take both bases:\n{masked}"
    );
    // A twin calling a twin hands its own base on.
    let twice = body(&rust, "t_4TbCv_tb_twice__inv");
    assert!(
        twice.contains("__ih_0, __ib_0)"),
        "the nested twin call does not hand the base on:\n{twice}"
    );
}

#[test]
fn a_caller_passes_its_held_base_or_derives_one_from_the_header_at_the_call() {
    let rust = emit("callers", &[]);
    let main = body(&rust, "n_main");
    let calls: Vec<&str> = main
        .lines()
        .filter(|l| l.contains("t_4TbCv_tb_get__inv(cell"))
        .collect();
    // t1's loop grows no store: the base it holds is passed as a `__vb_N`.
    assert!(
        calls.iter().any(|l| {
            let args = &l[l.find("tb_get__inv(cell").unwrap()..];
            args.contains(", __vh_") && args.contains(", __vb_")
        }),
        "no call passes a held base:\n{}",
        calls.join("\n")
    );
    // t4's loop appends what it read: it holds no base, so the call derives one from the
    // header it does hold.
    assert!(
        calls
            .iter()
            .any(|l| l.contains("vector::vec_base(&__vh_") && l.contains("&stores.allocations))")),
        "no call derives a base from its header:\n{}",
        calls.join("\n")
    );
}

#[test]
fn the_switch_restores_the_header_only_twin() {
    let rust = emit("off", &[("LOFT_NO_TWIN_BASE", "1")]);
    assert!(!rust.contains("__ib_"), "a twin base survived the switch");
    assert!(
        !rust.contains("view base for"),
        "a shared view base survived the switch"
    );
    let get = body(&rust, "t_4TbCv_tb_get__inv");
    assert!(
        get.contains("get_elem_hoisted") && !get.contains("get_elem_at"),
        "the switch does not restore the store-resolving read:\n{get}"
    );
}
