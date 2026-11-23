// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-GuardFree` — the EMISSION pins.  A function from which nothing that registers a
//! fn-ref return buffer is reachable, cycles included, constructs no `FnRefBufGuard` and
//! keeps its depth push; a function that dispatches a fn-ref, calls one that does (at any
//! depth), hands a capture back through a join tail, or is a generator keeps its guard;
//! `LOFT_NO_GUARD_FREE=1` restores the guard on every non-leaf frame.  The guard
//! (`tests/scripts/158-guard-free.loft`) scores the rule on the leak channel; this pins
//! what is emitted, in both tiers, and the C backend's form of the guard (the frame's
//! `loft_nfnrefs` mark and its `loft_fnref_release`).
