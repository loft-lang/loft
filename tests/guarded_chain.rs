// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-GuardedChain` — the EMISSION pins.  A counted loop's admitted chains run plain
//! behind a guard evaluated once at the loop's entry (`//@FR-R-GuardedChain guard`): an
//! innermost loop is emitted twice (`plain chains` copy + checked `else` arm), a loop with
//! loops inside once with each admitted op branching on the guard (`(if __gc_N {`).
//! A loop with ONE admitted operator declines on PROFITABILITY: the guard is a fixed cost
//! per loop ENTRY against a saving of one null test per operator per ITERATION, and an
//! innermost loop is emitted twice, which in a small hot function costs the inline.
//! `LOFT_NO_GUARDED_CHAIN=1` restores the checked loops.  The cell corpus
//! (`tests/scripts/157-guarded-chain.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
