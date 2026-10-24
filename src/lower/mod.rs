// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a compiled target's decisions, as data

//! The `lower` layer (@PLN200 B3, `@C153`): what a COMPILED target decides about a program
//! before any text is written — which functions twin, which records travel apart, which views
//! decode in place, which walks stream — held as plain data that every emitter reads.  The
//! Rust writer (`generation`) spells these decisions; a second emitter reads the same ones.
//! Nothing here writes target text, names the Rust writer, or names the interpreter.
pub mod stream_walk;
