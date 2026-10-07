// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN184 Track W — the platform-sensitive scripts, run under the emulated Windows host
//! (`LOFT_POISON_HOST=windows`) on both backends.  The layer applies Windows' rules over the
//! real file system (BOTH_BACKEND_SWITCHES.md § Falsifier: the emulated Windows host), so a
//! script that depends on Linux's rules, or a runtime path that bypasses `file_access`, fails
//! here instead of first on the daily windows-latest run.
//!
//! This is the sweep, never the whole suite twice: the scripts below each exercise a
//! program's file handling, and the list is the one place that says which.  The same scripts
//! also run without the switch in `wrap` and `native`.  On a Windows host the switch changes
//! nothing, so there the same cells run against real Windows.
//!
//! @falsified-by: tests/falsified/windows_rules.patch — the runtime hands a program its host's
//!   rules again (`path_sep()` the host separator, `given` the native spelling, no name or case
//!   refusal, `\` not a separator).  The platform guard as it stood at 08b0ad037 then fails, under
//!   the switch on Linux, exactly the six cells windows-latest failed in windows-probe run
//!   37060992821 (separator, given directories, listing, case, names, backslash) and holds the
//!   same two (a held file, `PATH`), on both backends; without the switch it passes 8 of 8, as
//!   Linux did.  The current guard fails 5 of 9 under the switch.

use loft::file_access as fa;
use std::process::Command;

fn run_under_the_emulated_host(script: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for backend in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .current_dir(root)
            .env("LOFT_POISON_HOST", "windows")
            .env("LOFT_TIMEOUT", "240")
            .args([backend, "--tests", &format!("tests/scripts/{script}")])
            .output()
            .expect("run loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            out.status.success() && stdout.contains("test result: ok."),
            "{script} {backend} under LOFT_POISON_HOST=windows:\n{stdout}\n{stderr}"
        );
    }
}

macro_rules! under_the_emulated_host {
    ($($name:ident => $script:literal,)*) => {
        $(
            #[test]
            fn $name() {
                run_under_the_emulated_host($script);
            }
        )*
    };
}

under_the_emulated_host! {
    path_helpers => "108-path-helpers.loft",
    fread_inferred_size => "113-fread-inferred-size.loft",
    file_sync => "114-file-sync.loft",
    snapshot_roundtrip => "115-snapshot-roundtrip.loft",
    file_append_log => "116-file-append-log.loft",
    scalar_file_read => "158-scalar-file-read.loft",
    foreign_file => "174-foreign-file.loft",
    files => "19-files.loft",
    source_dir => "191-source-dir.loft",
    binary => "20-binary.loft",
    file_error_paths => "296-file-error-paths.loft",
    file_result => "42-file-result.loft",
    native_file_builtins => "430-native-file-builtins.loft",
    file_write_result => "561-file-write-result.loft",
    file_read_missing_null => "562-file-read-missing-null.loft",
    list_dir_survives_file_read => "586-h6-list-dir-survives-file-read.loft",
    sequential_file_lines => "60-sequential-file-lines.loft",
    amp_file_parameter => "753-amp-file-parameter.loft",
    unbound_file_read_only_vector => "899-unbound-file-read-only-vector.loft",
    unbound_file_read_temp => "899-unbound-file-read-temp.loft",
    stdlib_file_worked_examples => "946-stdlib-file-worked-examples.loft",
    a_buffered_read_moves_the_cursor => "a-buffered-file-read-moves-the-cursor-it-would-have.loft",
    a_cwd_test_file_anchors_at_the_cwd => "a-cwd-test-file-anchors-at-the-cwd-on-both-backends.loft",
    file_lines_keeps_its_line_rules => "a-file-lines-keeps-its-line-rules.loft",
    a_program_cannot_tell_which_platform => "a-program-cannot-tell-which-platform-it-runs-on.loft",
    a_scalar_file_read_writes_the_record_only => "a-scalar-file-read-is-a-write-of-the-file-record-only.loft",
    a_vector_is_written_as_its_stored_bytes => "a-vector-is-written-to-a-file-as-its-stored-bytes.loft",
    file_seek_and_position => "h11-file-seek-and-position.loft",
    one_file_handle_reads_and_writes => "one-file-handle-reads-and-writes-and-a-short-read-is-null.loft",
    the_reference_file_writes_its_widths => "the-reference-file-writes-the-widths-it-lists.loft",
}

/// `@FR-Path-Utf8` — a name loft text cannot spell is LISTED (with U+FFFD) and never REACHED.
/// The directory holds two such names that show alike (`a?.txt`), a real name spelled with
/// U+FFFD on disk (`c?.txt`, its own exact spelling) and a plain one.  A loft program cannot
/// make such a name, so the fixture does (`platform::name_that_is_not_text`).  Both backends,
/// with and without the emulated host; the expected lines are written out by hand.
#[test]
fn a_name_that_is_not_text_is_listed_and_never_reached() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("loft_not_text_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch dir");
    let one = dir.join(loft::platform::name_that_is_not_text("a", ".txt"));
    if let Err(e) = fa::write(&one, "one") {
        // macOS (APFS) holds only valid UTF-8 names: there the rule has nothing to refuse.
        eprintln!("this file system refuses a name that is not text ({e}); nothing to check");
        let _ = fa::remove_dir_all(&dir);
        return;
    }
    let two = dir.join(loft::platform::another_name_that_is_not_text("a", ".txt"));
    fa::write(&two, "two").expect("second name");
    fa::write(dir.join("c\u{FFFD}.txt"), "exact").expect("exact name");
    fa::write(dir.join("b.txt"), "plain").expect("plain name");
    let d = dir.to_string_lossy().replace('\\', "/");
    let program = dir.with_extension("loft");
    fa::write(
        &program,
        format!(
            r#"fn main() {{
  d = "{d}";
  names = list_dir(d) ?? [];
  println("listed {{len(names)}}: {{names}}");
  paths = 0;
  for f in file(d).files() {{ paths += 1; }}
  println("files {{paths}}");
  bad = "{{d}}/a\u{{FFFD}}.txt";
  println("exists {{exists(bad)}}");
  println("content {{file(bad).content() ?? "null"}}");
  println("write {{file(bad).write("y").ok()}}");
  println("delete {{delete(bad).ok()}}");
  println("exact {{file("{{d}}/c\u{{FFFD}}.txt").content() ?? "null"}}");
  println("plain {{file("{{d}}/b.txt").content() ?? "null"}}");
}}
"#
        ),
    )
    .expect("program");
    let want = "listed 4: [\"a\u{FFFD}.txt\",\"a\u{FFFD}.txt\",\"b.txt\",\"c\u{FFFD}.txt\"]\n\
                files 4\nexists false\ncontent null\nwrite false\ndelete false\n\
                exact exact\nplain plain\n";
    for host in ["", "windows"] {
        for backend in ["--interpret", "--native"] {
            let out = Command::new(env!("CARGO_BIN_EXE_loft"))
                .current_dir(root)
                .env("LOFT_POISON_HOST", host)
                .env("LOFT_TIMEOUT", "240")
                .args([backend, &program.to_string_lossy()])
                .output()
                .expect("run loft");
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert_eq!(stdout, want, "{backend} host={host:?}\n{stderr}");
            assert_eq!(
                stderr
                    .matches("holds a name that is not valid text")
                    .count(),
                1,
                "one line names the directory ({backend} host={host:?}):\n{stderr}"
            );
            // Nothing was reached: both files keep their content, and no third `a?.txt`.
            assert_eq!(fa::read_to_string(&one).unwrap(), "one");
            assert_eq!(fa::read_to_string(&two).unwrap(), "two");
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
        }
    }
    let _ = fa::remove_dir_all(&dir);
    let _ = fa::remove_file(&program);
}
