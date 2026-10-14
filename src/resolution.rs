// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I91 — Editor tooling: language server (LSP), debug adapter (DAP), resolution index
//! @PLN115 — the parse-time resolution index (position → resolved binding).
//!
//! The parser records, during parse, what each identifier occurrence resolves to,
//! so tooling (loft-lsp) can answer "what is the symbol at (line, col)?" and "where
//! are all occurrences of THIS binding?" PRECISELY, not lexically by name.  Gated
//! by `Parser.record_resolutions` (default off, zero-cost) — see
//! `doc/claude/plans/115-resolution-index/design.md`.
//!
//! S1 (this): the model, inert — nothing records yet.  S2 wires the first hook.

/// One resolved identifier occurrence recorded during parse.  `(line, col)` are
/// 1-based (the parser's own position convention); `len` is the name's char count,
/// so a consumer has the full `[col, col+len)` reference span without re-deriving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub line: u32,
    pub col: u32,
    pub len: u16,
    pub res: Resolution,
    /// True when this occurrence is the binding's DECLARATION (a parameter's
    /// signature name, a `for` / lambda binder) rather than a use.  Lets a
    /// consumer confirm the declaration is captured — the soundness gate for a
    /// precise rename — without the `name =` text heuristic assignment-locals use.
    pub declaration: bool,
}

/// What an identifier occurrence binds to.  A `Local` is keyed on its binding
/// IDENTITY — `(fn_def, var_nr)` — not a name, so two same-named locals in
/// different functions are distinct and shadowing / method references are exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A top-level definition — fn / struct / enum / typedef / constant / interface.
    Global(u32),
    /// A local variable or parameter; its type is `def(fn_def).variables().tp(var_nr)`.
    Local { fn_def: u32, var_nr: u16 },
    /// A field access `expr.field` on a struct/enum type.
    Field { type_def: u32, attr: u16 },
    /// A method call `expr.method(…)` on a receiver type.
    Method { recv_type: u32, method_def: u32 },
    /// A name that bound to nothing (a typo, or @C80 value-undefinedness).
    Unresolved,
}
