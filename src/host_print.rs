// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I66 — Bytecode VM / executor: printing to the host's stdout and stderr

//! Printing to the host's stdout and stderr, and the answer to a closed stdout: the
//! interpreter and the C runtime's host calls both reach these, so the backends cannot answer a
//! broken pipe differently.

/// Write `s` to standard output, treating a CLOSED stdout as the ordinary end of a pipeline
/// rather than a fault.
///
/// `print!` panics on any write error, so `prog | head` — the most ordinary shell idiom there
/// is — ended in a Rust panic naming a `std` source line; and when stderr shared the closed
/// pipe the panic PRINTER failed too, the process ABORTED, and the crash reporter wrote a
/// report blaming a line of `default/01_code.loft` that has nothing wrong with it
/// (loft#1289).  A reader that stops reading is not this program's error: stop writing and
/// leave, which is what every other tool on a pipeline does.
///
/// Exit **0** rather than by signal.  `head` itself exits 0, and a pipeline written to stop
/// early is doing what it was written to do — under `set -o pipefail` a signal would report
/// the idiom as a failure.
///
/// The interpreter (`fill.rs`) and the `#rust` template the native generator emits both call
/// THIS, so the two backends cannot answer a broken pipe differently.
///
/// # Panics
///
/// On a write error that is NOT a broken pipe — a full disk, an I/O error on a redirect.
/// That is a real fault and keeps the loudness `print!` had; only the closed reader is
/// reinterpreted as an ending.
pub fn host_print(s: &str) {
    use std::io::Write;
    let broken = {
        let mut out = std::io::stdout().lock();
        match out.write_all(s.as_bytes()) {
            Ok(()) => false,
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => true,
            // Anything else — a full disk, an I/O error on a redirect — is a real fault and
            // keeps the loudness `print!` had.
            Err(e) => panic!("failed printing to stdout: {e}"),
        }
    };
    if broken {
        std::process::exit(0);
    }
}

/// End the run with exit 0 when `e` is a closed stdout — [`host_print`]'s answer, for a
/// writer that gets the error back instead of panicking (`loft introspect`, the
/// `loft package` summary).  Any other error returns, so the caller reports it as before.
pub fn end_on_closed_stdout(e: &std::io::Error) {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
}

/// Whether a panic message is `print!`'s report of a closed stdout, so the CLI's panic hook
/// can give its own `println!` sites [`host_print`]'s answer without rewriting each one.
///
/// It reads the OS error NUMBER, never the message text: the text is the platform's
/// (`Broken pipe` on Linux, `The pipe is being closed.` on Windows), and the number's kind is
/// what decides.  Any other message answers false — a full disk keeps the loud panic.
#[must_use]
pub fn is_closed_stdout_panic(msg: &str) -> bool {
    msg.strip_prefix("failed printing to stdout: ")
        .and_then(|rest| rest.rsplit_once("(os error "))
        .and_then(|(_, n)| n.strip_suffix(')'))
        .and_then(|n| n.parse::<i32>().ok())
        .is_some_and(|n| {
            std::io::Error::from_raw_os_error(n).kind() == std::io::ErrorKind::BrokenPipe
        })
}

pub use crate::platform::host_eprint;
