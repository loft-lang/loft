// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `LOFT_TIMING`'s per-pass clock for a whole-program step.  Here, in `opt`, so both the
//! compiler's own steps and the rewrites below it can time their passes.

use std::fmt::Write as _;

/// The time each pass of a whole-program step took, for `LOFT_TIMING`.  Inert (no clock
/// read, no allocation) unless `on`.
pub struct PassLaps {
    last: Option<std::time::Instant>,
    laps: Vec<(&'static str, f64)>,
}

impl PassLaps {
    pub fn new(on: bool) -> Self {
        PassLaps {
            last: on.then(std::time::Instant::now),
            laps: Vec::new(),
        }
    }

    pub fn lap(&mut self, pass: &'static str) {
        if let Some(last) = self.last {
            let now = std::time::Instant::now();
            self.laps
                .push((pass, now.duration_since(last).as_secs_f64() * 1000.0));
            self.last = Some(now);
        }
    }

    pub fn report(&self, step: &str, definitions: u32) {
        if self.last.is_some() {
            let total: f64 = self.laps.iter().map(|(_, ms)| ms).sum();
            let mut line = format!("LOFT_TIMING {step} defs={definitions} total={total:.2}ms");
            for (pass, ms) in &self.laps {
                let _ = write!(line, " {pass}={ms:.2}ms");
            }
            eprintln!("{line}");
        }
    }
}
