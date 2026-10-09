// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): the structural limits it enforces

//! The one number loft's structural limits share: 300.
//!
//! A program may nest an expression 300 levels deep, declare 300 fields in a struct, take 300
//! parameters, bind 300 variables in a function and hold 300 elements in a tuple.  Every one of
//! these works at 300 — on both backends, on the test runner's thread and under the sanitizer
//! builds — and the 301st is refused at compile time with a message naming the limit.  One
//! number, so it can be remembered, and well inside what every pass after the parser can hold.
//!
//! What counts as one level of depth: each nested expression (a bracket, a call argument, a
//! block, a branch, a vector element, a struct field value), each operator in a chain
//! (`a + b + c` is three levels deep at `c`), and each postfix step (`a.b.c`, `x.f().g()`).
//!
//! Not limited by it: the size of a collection (a vector's length, a text's), the number of
//! definitions, functions or files.  An enum's variants have their own cap, 254, because a
//! variant is stored in one byte.

/// The limit.  The 301st level, field, parameter, variable or tuple element is refused.
pub const LIMIT: u32 = 300;

/// The deepest expression tree the recursive passes after the parser accept (the scope pass,
/// `scopes/scan.rs`, and the live-interval pass, `variables/intervals.rs`): past it they stop
/// with an internal error rather than overflow the stack.  A program within [`LIMIT`] never
/// reaches it — measured, every form builds at most two tree levels per level of depth, and
/// the scope pass's rewrites (a method receiver into a temporary) at most double that — so it
/// is a safety net, not a limit a program meets.  The parser refuses a tree deeper than half of
/// it (`TREE_DEPTH / 2`) as a backstop for a shape the depth count does not see.
pub const TREE_DEPTH: usize = 8 * LIMIT as usize;

/// Refuse definition `d_nr`: it needs more than 64 KiB of stack for its parameters, its
/// variables and the values its expressions hold while they run.  A technical cap, like the
/// enum's: the interpreter addresses a frame with 16-bit positions.  [`LIMIT`] does not
/// multiply into it — 300 variables each holding a 300-element tuple is far past it — so a
/// frame past it is refused on its own, wherever a pass first meets it: the slot layout for
/// parameters and variables, the code generator for the values pending in an expression.
pub fn frame_too_large(data: &crate::data::Data, d_nr: u32) -> ! {
    let d = data.def(d_nr);
    crate::crash_report::limit_exceeded(
        d.position(),
        &format!(
            "`{}` needs more than 64 KiB of stack for its parameters, its variables and the \
             values its expressions hold — move part of the work into a function of its own, \
             or keep large tuples in a struct or a vector",
            d.display_name()
        ),
    )
}
