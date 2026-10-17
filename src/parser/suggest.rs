// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! What a tool reads back from a parse: the resolved occurrences an LSP asks for (@PLN115)
//! and the similar-name suggestions a diagnostic offers for an unknown function, field or
//! type.

use super::{Parser, Position};

impl Parser {
    /// Plan-07 phase 5 suggestions.  Find a similar user-defined
    /// function name (typed without the `n_` prefix) that might be
    /// the correct spelling.  Returns `None` when no candidate is
    /// within Levenshtein distance 2.
    ///
    /// Uses the same `suggest_similar` primitive as the existing
    /// variable-suggestion path at `parser/objects.rs::known_var_or_type`.
    /// @PLN115 — the resolved identifier occurrences recorded during the last
    /// parse (empty unless `record_resolutions` was set).  Drives the LSP's
    /// precise, non-lexical navigation.
    #[must_use]
    pub fn resolutions(&self) -> &[crate::resolution::Occurrence] {
        &self.resolutions
    }

    /// @PLN115 — turn occurrence recording on (default off).  The LSP parse path
    /// (S3) flips this before `parse_source` so navigation can resolve by binding
    /// identity; every normal compile leaves it off, keeping `record` a dead branch.
    pub fn set_record_resolutions(&mut self, on: bool) {
        self.record_resolutions = on;
    }

    /// @PLN115 — record one resolved occurrence, gated: a single predictable
    /// branch when `record_resolutions` is off (every normal compile), so it is
    /// zero-cost there.  A pure side-append — it changes no parse decision.  Wired
    /// to the resolution chokepoints starting in S2 (`parse_var` locals).
    pub(super) fn record(&mut self, pos: &Position, len: u16, res: crate::resolution::Resolution) {
        self.record_occurrence(pos, len, res, false);
    }

    /// @PLN115 tail — record a binding's DECLARATION occurrence (a parameter's
    /// signature name, a `for` / lambda binder).  Same gate as [`Self::record`],
    /// but flagged `declaration` so a consumer knows the binding's declaration is
    /// captured and a precise rename is complete.
    pub(super) fn record_decl(
        &mut self,
        pos: &Position,
        len: u16,
        res: crate::resolution::Resolution,
    ) {
        self.record_occurrence(pos, len, res, true);
    }

    pub(super) fn record_occurrence(
        &mut self,
        pos: &Position,
        len: u16,
        res: crate::resolution::Resolution,
        declaration: bool,
    ) {
        if self.record_resolutions {
            self.resolutions.push(crate::resolution::Occurrence {
                line: pos.line,
                col: pos.pos,
                len,
                res,
                declaration,
            });
        }
    }

    pub fn suggest_function_name(&self, name: &str) -> Option<String> {
        let candidates_owned: Vec<String> = self
            .data
            .user_fn_d_nrs()
            .iter()
            .filter_map(|&d_nr| {
                let n = self.data.def(d_nr).name();
                if let Some(stripped) = n.strip_prefix("n_") {
                    // Skip synthetic lambda names — they're not user-typeable.
                    if stripped.starts_with("__lambda_") {
                        None
                    } else {
                        Some(stripped.to_string())
                    }
                } else {
                    None
                }
            })
            .collect();
        let candidates: Vec<&str> = candidates_owned.iter().map(String::as_str).collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates).map(String::from)
    }

    /// Plan-07 phase 5 suggestions.  Find a similar field name on
    /// the given struct definition.  Skips synthetic compiler-
    /// generated attributes (those starting with `_` or `#`).
    pub fn suggest_field_name(&self, struct_d_nr: u32, name: &str) -> Option<String> {
        if struct_d_nr == u32::MAX {
            return None;
        }
        let candidates_owned: Vec<&str> = self
            .data
            .def(struct_d_nr)
            .attributes
            .iter()
            .filter_map(|a| {
                if a.name.starts_with('_') || a.name.starts_with('#') {
                    None
                } else {
                    Some(a.name.as_str())
                }
            })
            .collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates_owned).map(String::from)
    }

    /// Plan-07 phase 5 suggestions.  Find a similar type name —
    /// thin wrapper around `Data::suggest_type_name` so callers in
    /// the parser don't have to thread `self.data` explicitly.
    pub fn suggest_type_name(&self, name: &str) -> Option<String> {
        self.data.suggest_type_name(name)
    }
}
