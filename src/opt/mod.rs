// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The `opt` layer (@PLN200): the analyses of the IR that more than one backend asks — the
//! interpreter's bytecode emitter, the IR-phase rewrites and the native generator.  An
//! optimisation the interpreter can use lives here, never in a backend's writer, so a second
//! code generator reads the same answers without carrying the Rust writer.  Each item moved
//! from `generation/` keeps a re-export at its old path.

pub mod attrs;
