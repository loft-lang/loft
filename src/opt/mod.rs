// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The `opt` layer (@PLN200, `@C153`): the analyses of the IR that more than one backend asks — the
//! interpreter's bytecode emitter, the IR-phase rewrites and the native generator.  An
//! optimisation the interpreter can use lives here, never in a backend's writer, so a second
//! code generator reads the same answers without carrying the Rust writer.  Each item moved
//! from `generation/` keeps a re-export at its old path.

pub mod attrs;
pub mod body_writes;
pub mod char_walk;
pub mod frame_calls;
pub mod header;
pub mod iteration;
pub mod laps;
pub mod loop_records;
pub mod names;
pub mod native_key;
pub mod non_sentinel;
pub mod range;
pub mod tail_self;
pub mod text_borrow;
pub mod value_records;
pub mod writes;

/// Forget every analysis memo: the range analysis's call memo (keyed by a definition's
/// address) and the non-null analysis's answers (keyed by its number) are facts about ONE
/// program, and the next program a process compiles names other functions by the same
/// addresses and numbers (loft#2122, loft#2142).  Called by the first pass of a program that
/// asks them, `leaf_inline::rewrite_program`, and by `generation::Output::new`.
pub(crate) fn begin_program() {
    non_sentinel::begin_generation();
    range::begin_generation();
}
