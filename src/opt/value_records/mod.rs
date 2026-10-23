// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-ValueRecord` — the functions whose no-heap record result, and the parameters whose
//! record argument, travel BY VALUE (a tuple of the fields) instead of through a store
//! record.  The IR pass `value_record` and the native generator both read the answer.

pub mod layout;
