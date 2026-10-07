// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! F11 (error-path state) regressions — see doc/claude/STABILITY_SWEEP.md
//! § F11.  Each test guards one probed break:
//!
//! * F11-2 — `files()` must return EVERY directory entry; an unstat-able
//!   one (dangling symlink) degrades to `Format.NotExists`, it does not
//!   truncate the listing (was: a mid-loop `return false` in `get_dir`
//!   silently dropped everything sorting after the failing entry).
//! * F11-4 — driver-mode native must anchor relative paths at the
//!   PROGRAM's directory, like the interpreter (was: the generated
//!   artifact anchored at its own dir, so file I/O silently landed in
//!   /tmp).  The assertion is the parity invariant itself, so the test
//!   holds whichever backend the driver picks (a rustc-mismatch fallback
//!   to interp still anchors at the program dir).

use loft::compile;
use loft::file_access as fa;
use loft::parser::Parser;
use loft::state::State;

fn test_tmp() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
    let _ = fa::create_dir_all(&dir);
    dir
}

/// Parse + run `src` in-process on the interpreter; panics on parse
/// errors, returns the runtime error (None = clean run, asserts passed).
fn run_interp(src: &str, name: &str) -> Option<String> {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(src, name, false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "program must parse: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data, &mut p.database);
    let mut data = p.data;
    let mut state = State::new(p.database);
    compile::byte_code(&mut state, &mut data);
    state.execute_argv("main", &data, &[]);
    state
        .database
        .runtime_error
        .take()
        .map(|e| format!("{e:?}"))
}

// @PLN184 C2 exemption candidate: a dangling symlink needs the symlink privilege on Windows; Windows substitute: none
#[cfg(unix)]
#[test]
fn f11_2_listing_survives_a_dangling_symlink() {
    let dir = test_tmp().join(format!("f11_dirlist_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).unwrap();
    fa::write(dir.join("a.txt"), "a").unwrap();
    fa::symlink("/nonexistent_f11_target", dir.join("b_dangle")).unwrap();
    fa::write(dir.join("c.txt"), "c").unwrap();

    let src = format!(
        r#"fn main() {{
  list = file("{}").files();
  assert(list.len() == 3, "every entry listed, got {{list.len()}}");
}}
"#,
        dir.display()
    );
    let err = run_interp(&src, "<f11-2>");
    assert!(err.is_none(), "listing must be complete: {err:?}");
    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn f11_4_driver_mode_anchors_at_the_program_dir() {
    let loft_bin = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/{}/loft",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    ));
    if !fa::exists(&loft_bin) {
        eprintln!("skipping: loft binary not built");
        return;
    }
    let dir = test_tmp().join(format!("f11_anchor_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).unwrap();
    let prog = dir.join("anchor_prog.loft");
    let marker = format!("f11_marker_{}.txt", std::process::id());
    fa::write(
        &prog,
        format!(r#"fn main() {{ file("{marker}").write("anchored"); }}"#),
    )
    .unwrap();
    // Run from the REPO root (not the program dir): the marker must land
    // next to the PROGRAM, whatever backend the driver picks.
    let status = std::process::Command::new(&loft_bin)
        .arg("--no-warnings")
        .arg(&prog)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOFT_OFFLINE", "1")
        .status()
        .expect("run loft");
    assert!(status.success(), "program must run clean");
    assert!(
        fa::exists(dir.join(&marker)),
        "the marker must land in the program's dir ({})",
        dir.display()
    );
    assert!(
        !fa::exists(std::path::Path::new("/tmp").join(&marker)),
        "the marker must NOT land at the artifact anchor (/tmp)"
    );
    let _ = fa::remove_dir_all(&dir);
}
