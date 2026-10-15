// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! How many workers a test may spawn BESIDE the other tests nextest runs at once (@PLN193 P0).
//!
//! nextest runs `NEXTEST_TEST_THREADS` test processes side by side, and a test that sizes an
//! inner pool from `available_parallelism()` multiplies that: six native corpus chunks each
//! running 24 `rustc` beside a 24-slot pool and a drop-gate chunk's 24 spawned `loft`s was the
//! 184 contending threads that hung the box on 2026-10-15.  A process that spawns a pool must
//! know its SHARE of the box, and that share is declared ONCE, in `.config/nextest.toml`:
//! every test that spawns a pool carries `threads-required = <UNDER_NEXTEST>` there, so
//! nextest counts its workers against the run's thread budget, and the pool here is sized to
//! the same number.  `native::pool_tests_declare_their_lanes_to_nextest` holds the two in step.
//!
//! Outside nextest (`cargo test`, a test binary run by hand) nothing else shares the box and
//! the pool takes the cores, as before.

/// The lanes an inner pool takes under nextest — the value `threads-required` carries for the
/// pool-spawning tests in `.config/nextest.toml`.
pub const UNDER_NEXTEST: usize = 8;

/// The workers an inner pool may run: at most `cap`, never more than the box's cores, and
/// [`UNDER_NEXTEST`] at most when nextest runs other tests beside this one (`NEXTEST=1`).
pub fn lanes(cap: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    let share = if std::env::var_os("NEXTEST").is_some() {
        UNDER_NEXTEST.min(cores)
    } else {
        cores
    };
    share.min(cap).max(1)
}
