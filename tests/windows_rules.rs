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
