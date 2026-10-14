// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): the structural limits it enforces

//! The one number loft's structural limits share: 300 (@C149).
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

/// The largest frame, in bytes: positions are 16-bit, so a frame past 64 KiB is refused.
pub const FRAME: u32 = u16::MAX as u32;

/// The limits one program is compiled under ([`crate::data::Data::limits`]): [`LIMIT`] and
/// [`FRAME`], unless a test LOWERS them.  Raising either is never possible — they are what
/// every later pass is built to hold.  A lowered limit lets a guard prove the refusal and the
/// fit at a small size: the same relations, without compiling a program 300 levels deep, which
/// costs time that grows with the square of the depth.  `// @LIMIT: n` and `// @FRAME_LIMIT: n`
/// set them for one corpus script; `LOFT_LIMIT` / `LOFT_FRAME_LIMIT` for a `loft` run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The count: nesting levels, fields, parameters, variables, tuple elements.
    pub count: u32,
    /// The largest frame in bytes.
    pub frame: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            count: LIMIT,
            frame: FRAME,
        }
    }
}

impl Limits {
    /// The defaults, each lowered to the value given — never raised past it.
    #[must_use]
    pub fn lowered(count: Option<u32>, frame: Option<u32>) -> Self {
        Limits {
            count: count.map_or(LIMIT, |c| c.min(LIMIT)),
            frame: frame.map_or(FRAME, |f| f.min(FRAME)),
        }
    }

    /// [`Limits::lowered`] from `LOFT_LIMIT` and `LOFT_FRAME_LIMIT`, for a `loft` run.
    #[must_use]
    pub fn from_env() -> Self {
        let read = |k: &str| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.trim().parse::<u32>().ok())
        };
        Limits::lowered(read("LOFT_LIMIT"), read("LOFT_FRAME_LIMIT"))
    }

    /// [`Limits::lowered`] from a corpus script's `// @LIMIT: n` and `// @FRAME_LIMIT: n`
    /// lines, read in its first 20 lines as `@ARGS` is.
    #[must_use]
    pub fn from_annotations(source: &str) -> Self {
        let read = |key: &str| {
            source
                .lines()
                .take(20)
                .find_map(|l| l.trim().strip_prefix(key))
                .and_then(|v| v.trim().parse::<u32>().ok())
        };
        Limits::lowered(read("// @LIMIT:"), read("// @FRAME_LIMIT:"))
    }

    /// Each limit the lower of the two — a second source can only lower what the first set.
    #[must_use]
    pub fn min(self, other: Limits) -> Self {
        Limits {
            count: self.count.min(other.count),
            frame: self.frame.min(other.frame),
        }
    }

    /// The frame cap in words a message can say: "64 KiB" at the default.
    #[must_use]
    pub fn frame_text(self) -> String {
        if self.frame == FRAME {
            "64 KiB".to_string()
        } else {
            format!("{} bytes", self.frame)
        }
    }
}

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
            "`{}` needs more than {} of stack for its parameters, its variables and the \
             values its expressions hold — move part of the work into a function of its own, \
             or keep large tuples in a struct or a vector",
            d.display_name(),
            data.limits.frame_text()
        ),
    )
}

/// Most frames a loft call stack may hold — `main` included, since `main` is a frame like any
/// other and `stack_trace()` reports it as one.
///
/// Set below the store stack limit (~8000 bytes / ~8 bytes per frame) so the depth check fires
/// before a store out-of-bounds panic.  Both backends enforce it against the same quantity:
/// `fn_call` reads `call_stack.len()`, the generated binary reads its shadow stack in
/// `cr_call_push` (loft#1058).
pub const MAX_CALL_DEPTH: u32 = 10_000;
