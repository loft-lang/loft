// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR)

//! @PLN91 — the reimplementation contract's per-function facts: how a function behaves as its
//! callers and the later passes see it, beyond its code.  Each is asked of the function by the
//! one predicate its call sites ask, so the line is what this front end acts on, not a second
//! derivation.  `LOFT_REPORT_CONTRACT=1` prints one `[contract]` line per program function where
//! the parser image is written (`LOFT_EMIT_PARSED`); the loft-written front end prints the same
//! line from its own predicates, and `elab_gate.py` / `elab_probe.py` compare the two.

use super::{DefType, HashMap, Parser, Value};
use std::fmt::Write as _;

impl Parser {
    /// One `[contract]` line per function of the program (no standard-library definition), in
    /// definition order:
    ///
    /// - `buffer` — the hidden parameter the result is delivered through (`-`: none);
    /// - `bufferless_text` — answers a text with no `&text` work buffer;
    /// - `borrowed_view` — the result views a visible parameter's store;
    /// - `fnref_return` — some `return` hands back a function value;
    /// - `adopts_fresh` — the caller adopts the store the result names;
    /// - `store_text_instance` — the store-text twin of a `&text` callee;
    /// - `one_of_several` — the result may be one of several arguments;
    /// - `forwards_foreign` — the result is a store the function does not own;
    /// - `buffer_exits` — `fills` (every exit answers the buffer), `exits` (some answers a store
    ///   the caller never owns), `-` (not placeable);
    /// - `wrapper` — the one operator a one-op wrapper forwards to (`-`: none);
    /// - `rebinds` — the visible parameters the body rebinds with a store it owns.
    pub fn contract_report(&self) -> String {
        let mut out = String::new();
        let mut rebinds_cache = HashMap::new();
        for d_nr in 0..self.data.definitions() {
            let def = self.data.def(d_nr);
            if def.def_type() != DefType::Function || def.is_stdlib() || *def.code() == Value::Null
            {
                continue;
            }
            let attrs = def.attributes();
            let buffer = def
                .hidden_return_buffer_attr()
                .and_then(|a| attrs.get(a))
                .map_or_else(|| "-".to_string(), |a| a.name.clone());
            let exits = match self.callee_buffer_exits(d_nr, &mut Vec::new()) {
                Some(false) => "fills",
                Some(true) => "exits",
                None => "-",
            };
            let wrapper = crate::ir_shape::one_op_wrapper(&self.data, d_nr).map_or_else(
                || "-".to_string(),
                |(op, _)| self.data.def(op).name().to_string(),
            );
            let rebinds =
                crate::parser::callee_param_rebinds_owned(d_nr, &self.data, &mut rebinds_cache);
            let named = |keep: &dyn Fn(usize) -> bool| -> String {
                let names: Vec<&str> = (0..attrs.len())
                    .filter(|&i| keep(i))
                    .map(|i| attrs[i].name.as_str())
                    .collect();
                if names.is_empty() {
                    "-".to_string()
                } else {
                    names.join(",")
                }
            };
            let rebinds = named(&|i| rebinds.get(i).copied().unwrap_or(false));
            let b = |v: bool| u8::from(v);
            let _ = writeln!(
                out,
                "[contract] fn={} buffer={buffer} bufferless_text={} borrowed_view={} \
                 fnref_return={} adopts_fresh={} store_text_instance={} one_of_several={} \
                 forwards_foreign={} buffer_exits={exits} wrapper={wrapper} rebinds={rebinds}",
                def.name(),
                b(def.returns_bufferless_text()),
                b(def.returns_borrowed_view()),
                b(def.has_fnref_return_site()),
                b(def.return_adopts_fresh_store()),
                b(def.is_store_text_instance()),
                b(crate::use_analysis::returns_one_of_several_args(
                    &self.data, d_nr
                )),
                b(self.callee_forwards_foreign_store(d_nr)),
            );
        }
        out
    }
}
