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
// approved exemption: `getrusage` (own and reaped-children CPU) has no Windows equivalent here; Windows substitute: the wall column, with the CPU columns said to be unmeasured
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

// approved exemption: `getrusage` (own and reaped-children CPU) has no Windows equivalent here; Windows substitute: the wall column, with the CPU columns said to be unmeasured
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
        // approved exemption: `getrusage` (own and reaped-children CPU) has no Windows equivalent here; Windows substitute: the wall column, with the CPU columns said to be unmeasured
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

/// What one corpus chunk did, script by script, against the budget a chunk is allowed.
///
/// A chunk is ONE nextest test holding hundreds of programs, so "TIMEOUT" says nothing about
/// which of them took the time.  This prints a line per program as it finishes (wall and
/// CPU), and a WATCHDOG prints — the moment the chunk's wall clock passes the budget — the
/// program still running and the slowest ones so far, which reaches the report even when
/// nextest kills the test at its limit.  [`Suite::finish`] fails the chunk when its CPU
/// passed the budget: CPU is the chunk's own work, so the verdict does not depend on what
/// else the machine runs; `wall ≫ cpu` in the report is contention, `wall ≈ cpu` is work.
pub struct Suite {
    name: String,
    budget: Duration,
    start: Unit,
    rows: std::sync::Arc<std::sync::Mutex<Vec<(String, Duration, Duration)>>>,
    /// The CPU each row is CHARGED against the budget: its own, or a parallel batch's
    /// divided by the jobs it ran at once ([`Suite::end_parallel`]).
    charged: std::sync::Arc<std::sync::Mutex<Duration>>,
    current: std::sync::Arc<std::sync::Mutex<Option<(String, Instant)>>>,
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The budget of one corpus chunk: nextest stops a test at 300 s, and a chunk keeps 80 s of
/// that to spare.
pub const CHUNK_BUDGET: Duration = Duration::from_secs(220);

/// The slowest rows, longest first, as report lines.
fn slowest(rows: &[(String, Duration, Duration)], n: usize) -> String {
    let mut sorted: Vec<&(String, Duration, Duration)> = rows.iter().collect();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.2));
    sorted
        .iter()
        .take(n)
        .map(|(l, w, c)| {
            format!(
                "    {:7.1}s cpu {:7.1}s wall  {l}",
                c.as_secs_f64(),
                w.as_secs_f64()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Suite {
    /// Begin a chunk named `name`, with `budget` and its watchdog.
    #[must_use]
    pub fn start(name: &str, budget: Duration) -> Self {
        let suite = Suite {
            name: name.to_string(),
            budget,
            start: Unit::start(),
            rows: std::sync::Arc::default(),
            charged: std::sync::Arc::default(),
            current: std::sync::Arc::default(),
            done: std::sync::Arc::default(),
        };
        let (rows, current, done) = (
            suite.rows.clone(),
            suite.current.clone(),
            suite.done.clone(),
        );
        let label = suite.name.clone();
        let began = suite.start.wall;
        std::thread::spawn(move || {
            while began.elapsed() < budget {
                if done.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if done.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let running = current.lock().map_or_else(
                |_| "?".to_string(),
                |c| match &*c {
                    Some((l, t)) => format!("{l} (running {:.1}s)", t.elapsed().as_secs_f64()),
                    None => "nothing".to_string(),
                },
            );
            let rows = rows.lock().map(|r| r.clone()).unwrap_or_default();
            let cpu: Duration = rows.iter().map(|r| r.2).sum();
            eprintln!(
                "{label}: past its {}s budget on the wall clock — {} programs done ({:.1}s cpu \
                 between them), now running {running}.  Slowest so far:\n{}",
                budget.as_secs(),
                rows.len(),
                cpu.as_secs_f64(),
                slowest(&rows, 8)
            );
        });
        suite
    }

    /// `label` begins; the watchdog names it while it runs.
    pub fn begin(&self, label: &str) -> Unit {
        if let Ok(mut c) = self.current.lock() {
            *c = Some((label.to_string(), Instant::now()));
        }
        Unit::start()
    }

    /// `label` is done: print its line, keep its row, and record it for `LOFT_TEST_TIMING`.
    pub fn end(&self, label: &str, unit: Unit) {
        self.end_parallel(label, unit, 1);
    }

    /// [`Suite::end`] for a batch that ran up to `lanes` jobs at once (the native runner's
    /// parallel `rustc`): its CPU is printed whole and charged to the budget divided by
    /// `lanes`, the time it holds the chunk on an unloaded machine.
    pub fn end_parallel(&self, label: &str, unit: Unit, lanes: usize) {
        let wall = unit.wall.elapsed();
        let cpu = cpu(Who::Own).saturating_sub(unit.own) + cpu(Who::Kids).saturating_sub(unit.kids);
        let lanes = u32::try_from(lanes.max(1)).unwrap_or(u32::MAX);
        if let Ok(mut c) = self.charged.lock() {
            *c += cpu / lanes;
        }
        let shared = if lanes > 1 {
            format!(
                " ({lanes} at once: {:.1}s charged)",
                (cpu / lanes).as_secs_f64()
            )
        } else {
            String::new()
        };
        println!(
            "  {:6.1}s cpu {:6.1}s wall  {label}{shared}",
            cpu.as_secs_f64(),
            wall.as_secs_f64()
        );
        if let Ok(mut r) = self.rows.lock() {
            r.push((label.to_string(), wall, cpu));
        }
        if let Ok(mut c) = self.current.lock() {
            *c = None;
        }
        unit.finish(label);
    }

    /// No budget: a run that is not a chunk (the nightly whole corpus) still reports.
    #[must_use]
    pub fn unbounded(name: &str) -> Self {
        Suite::start(name, Duration::from_secs(u64::from(u32::MAX)))
    }

    /// Close the chunk: print its totals and slowest programs, and answer an error when its
    /// CPU passed the budget.
    pub fn finish(self) -> std::io::Result<()> {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
        let wall = self.start.wall.elapsed();
        let cpu = cpu(Who::Own).saturating_sub(self.start.own)
            + cpu(Who::Kids).saturating_sub(self.start.kids);
        let rows = self.rows.lock().map(|r| r.clone()).unwrap_or_default();
        // Charged: each row's CPU, a parallel batch's divided by its lanes; what no row holds
        // (the runner's own setup) is charged whole.
        let in_rows: Duration = rows.iter().map(|r| r.2).sum();
        let charged = self.charged.lock().map_or(cpu, |c| *c) + cpu.saturating_sub(in_rows);
        let report = format!(
            "{}: {} programs, {:.1}s cpu, {:.1}s charged, {:.1}s wall, budget {}s.  Slowest:\n{}",
            self.name,
            rows.len(),
            cpu.as_secs_f64(),
            charged.as_secs_f64(),
            wall.as_secs_f64(),
            self.budget.as_secs(),
            slowest(&rows, 8)
        );
        println!("{report}");
        // A platform without `getrusage` reads zero CPU, so it is judged on nothing here.
        if charged > self.budget {
            return Err(std::io::Error::other(format!(
                "over budget — a program that needs this much is wrong, not the budget.\n{report}"
            )));
        }
        Ok(())
    }
}

impl Drop for Suite {
    /// A chunk that returns early on a failing program stops its watchdog too.
    fn drop(&mut self) {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
