// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Per-unit CPU accounting for the test suite.
//!
//! # Why wall clock cannot answer "what is slow"
//!
//! Every attribution this repo has attempted from wall time has been wrong, and each was
//! wrong in a DIFFERENT way — which is what makes a dedicated instrument worth its code:
//!
//! * **JUnit `time` under load measures contention, not work.**
//!   `a_declared_font_reaches_the_emitted_page` reads 230 s inside a contended run and
//!   0.9 s alone: a 255× inflation that says nothing about the test.
//! * **Re-running one test in isolation measures the BUILD.**  `cargo nextest run -E
//!   test(X)` first makes 236 test binaries current, so a run after any source edit
//!   charges the rebuild to the test.  Measured: the same test reported 19.1 s that way
//!   and **0.096 s** once everything was built.
//! * **A number in a document is a claim with a date on it.**  `CI_BUDGET.md` recommended
//!   optimising two round-trip tests at 136 s; re-measured, the pair runs in 0.139 s, and
//!   a change was made and reverted on the strength of the stale figure.
//!
//! # What this records instead
//!
//! CPU time, from `getrusage`, split into this process and its REAPED CHILDREN — 185 of
//! the 230 test binaries spawn `loft`, `rustc` or a browser, so the child half is most of
//! the work and is exactly what wall clock cannot separate from waiting.  CPU time is
//! contention-invariant: it does not change because another gate is running.
//!
//! Wall is recorded too, because the RATIO is itself the diagnosis — `wall ≫ cpu` means
//! waiting (contention, I/O, a sleep), `wall ≈ cpu` means real work.
//!
//! # Granularity
//!
//! The unit is a LABEL the caller chooses, not a nextest test, because the suite's biggest
//! blind spot is that `loft_suite` is one nextest test containing ~895 corpus programs:
//! individually invisible, and their output interleaves into one unreadable bucket.  Label
//! each program and both problems go away.
//!
//! Off unless `LOFT_TEST_TIMING` names a file, so an ordinary run pays two `getrusage`
//! calls per unit and writes nothing.  Rows are appended with `O_APPEND` in one `write`,
//! which the kernel keeps atomic below `PIPE_BUF` — so parallel test processes may share
//! one file without interleaving.

use std::time::{Duration, Instant};

/// Which process tree a CPU reading is about.  Spelled here rather than as `libc`'s constants
/// because `libc` is a `cfg(unix)` dependency: naming its types in a signature is what made
/// this file fail to COMPILE on Windows, where `getrusage` does not exist and the crate is not
/// linked (`error[E0433]: cannot find module or crate 'libc'`, the nightly's windows leg).
#[derive(Clone, Copy)]
pub enum Who {
    /// This process.
    Own,
    /// Children already reaped.
    Kids,
}

/// CPU consumed so far by `who`.
///
/// Unix reads it from `getrusage`.  Everywhere else there is no equivalent this file can reach
/// without a new dependency, so it reads ZERO — and [`Unit::finish`] says so once per armed run
/// rather than writing plausible zeros into a report nobody can tell from a fast test.
// @PLN184 C2 exemption candidate: `getrusage` (own and reaped-children CPU) has no Windows equivalent here; Windows substitute: the wall column, with the CPU columns said to be unmeasured
#[cfg(unix)]
fn cpu(who: Who) -> Duration {
    let who = match who {
        Who::Own => libc::RUSAGE_SELF,
        Who::Kids => libc::RUSAGE_CHILDREN,
    };
    // SAFETY: `getrusage` writes a plain POD struct; the zeroed value is a valid
    // `rusage`, and failure leaves it zeroed, which reads as "no CPU" rather than as
    // garbage.  A wrong-but-monotonic zero is the safe failure for a REPORT.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(who, &raw mut ru) } != 0 {
        return Duration::ZERO;
    }
    let secs = |t: libc::timeval| {
        Duration::from_secs(t.tv_sec.max(0) as u64) + Duration::from_micros(t.tv_usec.max(0) as u64)
    };
    secs(ru.ru_utime) + secs(ru.ru_stime)
}

// @PLN184 C2 exemption candidate: `getrusage` (own and reaped-children CPU) has no Windows equivalent here; Windows substitute: the wall column, with the CPU columns said to be unmeasured
#[cfg(not(unix))]
fn cpu(_who: Who) -> Duration {
    Duration::ZERO
}

/// An open measurement.  Take one before the work, call [`finish`](Self::finish) after.
pub struct Unit {
    wall: Instant,
    own: Duration,
    kids: Duration,
}

impl Unit {
    /// Begin measuring.  Cheap enough to wrap every corpus program: two `getrusage`
    /// calls, no allocation, and no file touched unless recording is armed.
    #[must_use]
    pub fn start() -> Self {
        Self {
            wall: Instant::now(),
            own: cpu(Who::Own),
            kids: cpu(Who::Kids),
        }
    }

    /// Close the measurement and append `label`'s row when `LOFT_TEST_TIMING` is set.
    ///
    /// Child CPU counts only children already REAPED, so the caller must have waited for
    /// what it spawned — every corpus runner does, since it needs the exit status.
    pub fn finish(self, label: &str) {
        let Some(path) = std::env::var_os("LOFT_TEST_TIMING") else {
            return;
        };
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let wall = ms(self.wall.elapsed());
        let own = ms(cpu(Who::Own).saturating_sub(self.own));
        let kids = ms(cpu(Who::Kids).saturating_sub(self.kids));
        // A platform with no `getrusage` writes wall-clock and two zeros, which is a report
        // that reads as "this test used no CPU" — indistinguishable from a very fast one.  Say
        // it once per armed run, and only when armed, so an ordinary run stays silent.
        // @PLN184 C2 exemption candidate: `getrusage` (own and reaped-children CPU) has no Windows equivalent here; Windows substitute: the wall column, with the CPU columns said to be unmeasured
        #[cfg(not(unix))]
        {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                eprintln!(
                    "loft: LOFT_TEST_TIMING is armed on a platform without `getrusage` — the \
                     wall column is real and the two CPU columns are 0, not measured."
                );
            });
        }
        // Tabs, and the label last, so a label containing a tab cannot shift a number
        // into the wrong column — the failure would be silent and the numbers plausible.
        let row = format!(
            "{wall:.1}\t{own:.1}\t{kids:.1}\t{:.1}\t{label}\n",
            own + kids
        );
        use std::io::Write as _;
        if let Ok(mut f) = loft::file_access::open_with(
            path.as_os_str(),
            std::fs::OpenOptions::new().create(true).append(true),
        ) {
            let _ = f.write_all(row.as_bytes());
        }
    }
}
