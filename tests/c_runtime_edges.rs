// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The C runtime on inputs at the edge of their range, built under AddressSanitizer and UBSan
//! (`common::c_sanitized`), where reading past a buffer, an arithmetic overflow and a loop that
//! never ends each show.  A host whose `cc` has no sanitizer runtime skips, saying so.
//!
//! A path that resolves to nothing is reached from a program, on both backends.  A null width
//! beside a radix marker and a text's room doubling past the largest size are not: the format
//! parser hands over a literal width with a zero pad, and only a 32-bit target's sizes double
//! that far.  Those two are cells of the runtime's own functions, called by a harness built
//! around the emitted unit — the 32-bit case in 64-bit sizes, the room past half of them.

mod common;

use common::c_sanitized as san;
use loft::file_access as fa;
use std::time::Duration;

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("loft-c-edges-{}-{name}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("scratch dir");
    d
}

/// Paths that name the working directory without naming a directory in it: `""`, `.` and
/// `q/..` resolve to nothing under `#cwd`.  `mkdir_all` of each answers `Ok` — the directory
/// is there — and `mkdir` of `""` makes nothing (`NotFound`), of the other two finds it there
/// (`Other`), and no `q` is made.  The interpreter, the compiled build and the compiled build
/// under the sanitizers print the same lines; the walk of a path's parents starts one past its
/// first character, which for a path of no characters was past its end.
#[test]
fn mkdir_of_a_path_that_names_the_working_directory() {
    let dir = scratch("mkdir");
    let prog = dir.join("mk.loft");
    fa::write(
        &prog,
        "fn main() {\n  println(\"all {mkdir_all(\\\"\\\")} {mkdir_all(\\\".\\\")} {mkdir_all(\\\"q/..\\\")}\");\n  println(\"one {mkdir(\\\"\\\")} {mkdir(\\\".\\\")} {mkdir(\\\"q/..\\\")} q {exists(\\\"q\\\")}\");\n}\n",
    )
    .expect("write mk.loft");
    let expect = "all Ok Ok Ok\none NotFound Other Other q false\n";
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args([backend, "--no-warnings", "mk.loft"])
            .current_dir(&dir)
            .env("LOFT_PATHS", "cwd")
            .env("LOFT_TIMEOUT", "300")
            .output()
            .expect("loft runs");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            expect,
            "{backend}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    if !san::available(&dir) {
        eprintln!("skipped the sanitized leg: no AddressSanitizer runtime for cc here");
        return;
    }
    let c = san::emit(&dir, &prog);
    let exe = san::build(&c, &c);
    let run = san::run(
        &exe,
        &[],
        &dir,
        &[("LOFT_PATHS", "cwd")],
        Duration::from_secs(60),
    );
    assert!(
        run.code == Some(0) && run.out == expect && !run.reported(),
        "sanitized: {:?}\n{}\n{}",
        run.code,
        run.out,
        run.err
    );
    let _ = fa::remove_dir_all(&dir);
}

/// A harness around the unit `fn main() {}` emits: `body` is its `main`, which calls the
/// runtime's own functions.  Built under the sanitizers; `None` where they are not here.
fn harness(name: &str, body: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let dir = scratch(name);
    if !san::available(&dir) {
        eprintln!("skipped: no AddressSanitizer runtime for cc here");
        return None;
    }
    let prog = dir.join("unit.loft");
    fa::write(&prog, "fn main() {\n  println(\"unit\");\n}\n").expect("write unit.loft");
    let unit = san::emit(&dir, &prog);
    let c = dir.join("harness.c");
    fa::write(
        &c,
        format!(
            "#define main loft_unit_main\n#include \"unit.c\"\n#undef main\nint main(void) {{\n{body}\n}}\n"
        ),
    )
    .expect("write harness.c");
    Some((san::build(&c, &unit), dir))
}

/// A number written with its radix marker and a zero pad (`{255:#06x}`, `0x00ff`) takes the
/// marker's width from the field's: a null width — no width — stays null and pads nothing,
/// `0xff`.  Subtracting from it wrapped, past the smallest number, to a width near the largest.
#[test]
fn a_radix_marker_beside_a_null_width_pads_nothing() {
    let Some((exe, dir)) = harness(
        "null-width",
        r#"    loft_text s = { 0, 0, 0 };
    loft_format_prefixed(0, &s, loft_lit(0, "0x", 2), loft_lit(0, "ff", 2), LOFT_NULL, 1, '0');
    loft_push(0, &s, "|", 1);
    loft_format_prefixed(0, &s, loft_lit(0, "0x", 2), loft_lit(0, "ff", 2), 6, 1, '0');
    printf("%.*s\n", (int) s.len, s.p);
    return 0;"#,
    ) else {
        return;
    };
    let run = san::run(&exe, &[], &dir, &[], Duration::from_secs(60));
    assert!(
        run.code == Some(0) && run.out == "0xff|0x00ff\n" && !run.reported(),
        "{:?} {}\n{}",
        run.code,
        run.out,
        run.err
    );
    let _ = fa::remove_dir_all(&dir);
}

/// A text whose room, doubled, would pass the largest size asks for exactly what it needs —
/// here more than can be had, so the run stops with `loft: out of memory` at once.  The
/// doubling wrapped to zero and doubled zero for ever: the loop a 32-bit target meets once a
/// text's room passes 2 GiB, met here in 64-bit sizes with the room past half of them.
#[test]
fn a_text_whose_room_would_double_past_the_largest_size_is_out_of_memory() {
    let Some((exe, dir)) = harness(
        "reserve",
        r#"    loft_text t = { 0, 0, 0 };
    t.cap = SIZE_MAX / 2 + 1;
    loft_reserve(0, &t, SIZE_MAX / 2 + 2);
    puts("reserved");
    return 0;"#,
    ) else {
        return;
    };
    let run = san::run(&exe, &[], &dir, &[], Duration::from_secs(30));
    assert!(
        !run.timed_out
            && run.code != Some(0)
            && run.out.is_empty()
            && run.err.contains("loft: out of memory")
            && !run.reported(),
        "timed out {} {:?} {}\n{}",
        run.timed_out,
        run.code,
        run.out,
        run.err
    );
    let _ = fa::remove_dir_all(&dir);
}
