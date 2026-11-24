// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-SplitTable` — the EMISSION pins.  A `parts = text.split(c)` bound to a local
//! and then only `len`'d, indexed or walked collects its pieces as `Vec<&str>` at the bind
//! and never calls `t_4text_split`; a parameter nothing writes is borrowed and every other
//! source is copied at the bind; the walk's loop variable is a borrowed `&str`; the raising
//! and the nullable element read take their own getters; the buffer the call would have
//! filled is never minted; a returned, appended, written, handed-away, copied or rebound
//! vector, a variable separator, a bind read outside its block and a generator keep the
//! vector; `LOFT_NO_SPLIT_TABLE=1` restores the vector everywhere.  The guard
//! (`tests/scripts/158-split-table.loft`) says the VALUES hold on both backends; this pins
//! what is emitted.
