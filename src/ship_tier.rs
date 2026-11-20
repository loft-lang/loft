// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native generators: the build tier a process writes for (`--ship`)

//! Whether this process writes shipped artefacts (`--ship`, NATIVE.md § Optimisation tiers).
//!
//! The fact is set while the command line is read, before anything is parsed, and read by
//! every layer that leaves something out of a shipped build: the C writer keeps a shipped
//! unit's functions whole (`c::functions_whole`), and the parser adds no development check
//! to it (`Parser::arm_drop_marks`).  It lives in the core layer so that both can ask.

use std::sync::atomic::{AtomicBool, Ordering};

static SHIPPED_UNITS: AtomicBool = AtomicBool::new(false);

/// Every unit this process writes from now on is a shipped artefact's.
pub fn ship_units() {
    SHIPPED_UNITS.store(true, Ordering::Relaxed);
}

/// Whether this process writes shipped artefacts.
#[must_use]
pub fn shipped_units() -> bool {
    SHIPPED_UNITS.load(Ordering::Relaxed)
}
