// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! loft#806 — a crash report must ATTRIBUTE the fault, not merely announce it.
//!
//! Two of its three lines were unreadable, and each cost real time on #806:
//!
//! * `last op: (opcode dispatch) (op=249)` — a number names nothing a reader can
//!   act on. The opcode's name was known all along (`Data::operator_name`); it
//!   was simply never on a path a signal handler can reach, since a handler
//!   cannot allocate, lock, or borrow what the crashing thread may hold.
//! * `at default/05_coroutine.loft:18:27` — a stdlib line the program never
//!   calls. The span lookup answers "the last statement recorded at or before
//!   pc", the table is sparse, and with nothing nearer it reaches arbitrarily far
//!   back — then states the result as flatly as a true one. That is worse than
//!   silence: silence makes you look, a confident wrong answer sends you away.
//!
//! Its OWN test binary, like `crash_report_file.rs` beside it and for the same
//! reason: `install` resolves the report destination once per process, and both
//! facts under test here are published through process-wide `OnceLock`s. A
//! sibling test that installs or publishes first would pin what this one asserts
//! — which is exactly what happened when this started life in that file.

// @PLN184 C2 exemption candidate: `fork` and the POSIX signal handler the crash reporter rides (unix-only by design) have no Windows equivalent; Windows substitute: none
#![cfg(unix)]

use loft::file_access as fa;
use std::path::PathBuf;

/// Fault a forked child for real and answer how it died.
///
/// A null dereference rather than `raise`: the handler is armed `SA_RESETHAND`,
/// so only a FAULTING INSTRUCTION re-executes and re-raises. `install` runs in
/// the parent, before the fork — resolving the path allocates, and after a fork
/// only async-signal-safe calls are sound.
fn crash_a_child() -> libc::c_int {
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let devnull = libc::open(c"/dev/null".as_ptr().cast(), libc::O_WRONLY);
            if devnull >= 0 {
                libc::dup2(devnull, libc::STDERR_FILENO);
            }
            std::ptr::null_mut::<u8>().write_volatile(1);
            libc::_exit(0);
        }
        let mut status: libc::c_int = 0;
        assert!(
            libc::waitpid(pid, &raw mut status, 0) == pid,
            "waitpid failed"
        );
        status
    }
}

#[test]
fn the_report_names_the_opcode_and_states_a_missing_source_span() {
    let dir = std::env::temp_dir().join(format!("loft-crash-806-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let target: PathBuf = dir.join("report.txt");
    // SAFETY: single-threaded here; the fork happens below.
    unsafe {
        std::env::set_var("LOFT_CRASH_FILE", &target);
    }
    loft::crash_report::install("loft");

    // The table a running interpreter publishes. Only the one entry matters; the
    // rest are the empty names an opcode number nothing uses would have.
    let mut names = vec![""; 256];
    names[249] = "OpAppendStackText";
    loft::crash_report::set_op_names(|| names);
    // The LABEL here is the generic one the interpreter actually passes, so a
    // report naming the op proves the name came from the TABLE, not the caller.
    loft::crash_report::set_context(8121, 249, "(opcode dispatch)", 700, "");

    let status = crash_a_child();
    assert!(
        libc::WIFSIGNALED(status),
        "the child must die of the signal — the handler reports, it does not swallow"
    );

    let report = fa::read_to_string(&target).expect("crash report");
    assert!(
        report.contains("OpAppendStackText"),
        "the report must name the opcode; `op=249` is not a diagnostic.\ngot:\n{report}"
    );
    // No span table was published here, so the report must SAY the pc has no
    // source — not omit the line and leave the reader unable to tell whether it
    // was looked for and missing, or never looked for at all.
    assert!(
        report.contains("no source span covers this pc"),
        "an absent source span must be stated, not silently skipped.\ngot:\n{report}"
    );

    let _ = fa::remove_dir_all(&dir);
}
