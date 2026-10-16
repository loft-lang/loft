// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The advice lints a function body earns after it is parsed — a byte index driven by a
//! character length, a cognitive complexity past the limit, too many parameters, a cluster of
//! trailing boolean flags.  Advice never gates (DIAGNOSTICS.md § Two tiers).

use super::{Level, Parser, Type, Value, diagnostic_format};

impl Parser {
    /// @PLN110 3a — warn on `for i in 0..len(s) { … s.byte_at(i) … }`.
    ///
    /// `len(text)` counts CHARACTERS while `byte_at` indexes BYTES, so this loop
    /// stops one byte short per multi-byte character. It fails in the worst way
    /// available: no diagnostic, no fault, a shorter buffer with plausible contents
    /// that ASCII input never exposes. The published `cbor` library shipped exactly
    /// this in its RFC 8949 text encoder — encoding `"José"` produced a short buffer,
    /// `decode` reported success with an empty string, and a signature over the
    /// round-trip still verified, because both sides ran the same truncating encoder.
    ///
    /// This is the `byte_at` sibling of the `text[i]` lint in `fields.rs`: the same
    /// units error, the same bound, a different read. Advisory (the types are
    /// correct) — the fix is `0..size(s)` for a byte walk, or `for c in s`.
    pub(super) fn warn_text_len_byte_index(&mut self, d_nr: u32, code: &Value) {
        if self.first_pass || self.default || !crate::keys::text_index_units_lint_enabled() {
            return;
        }
        let Value::Call(_, args) = code else { return };
        if self.data.def(d_nr).original_name() != "byte_at" || args.len() != 2 {
            return;
        }
        let Value::Var(iv) = args[1].unspan() else {
            return;
        };
        let Some(bound) = self.vars.loop_len_bound(*iv) else {
            return;
        };
        if crate::parser::operators::vec_key(&args[0], &self.data) != Some(bound) {
            return;
        }
        let iname = self.vars.name(*iv).to_string();
        diagnostic!(
            self.lexer,
            Level::Warning,
            code = "text-index-char-bound",
            "index `{iname}` walks `0..len(text)` (a character count) but `byte_at({iname})` \
             reads bytes — this truncates by one byte per multi-byte character and is silent \
             on ASCII"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "walk bytes with `0..size(text)`".to_string(),
            condition: Some("you meant BYTES, not characters".to_string()),
            edit: None,
            concept: "len vs size",
            concept_ref: "@F97",
        });
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title: "iterate the characters with `for c in text`".to_string(),
            condition: None,
            edit: None,
            concept: "len vs size",
            concept_ref: "@F97",
        });
    }

    /// Advice: this function's cognitive complexity has passed [`crate::keys::COMPLEXITY_ADVICE_AT`]
    /// — a whole algorithm in one body.
    ///
    /// Cognitive, so DEPTH is what costs: a construct is charged `1 + nesting`, an `else if`
    /// chain once per link at its own depth, and a flat `match` once however many arms it
    /// has. Eight sequential `if`s score 8 while three nested ones score 6, which is the
    /// ordering that matters — "many branches" is not "hard to follow", and only the second
    /// is worth interrupting anyone about.
    ///
    /// Counted at parse time because that is the only place the author's structure still
    /// exists: loft has no AST between the parser and the Value IR, and on the IR five `??`
    /// discharges with no author-written branch score 10 while one `for x in v` scores 5 — a
    /// reading that charges people for writing idiomatic loft.
    ///
    /// Names the deepest nesting line, because "score 93" is not actionable and "your
    /// deepest nesting is 7 levels at line 412" is.
    pub(super) fn warn_function_complexity(&mut self) {
        if self.first_pass || self.default || !crate::keys::complexity_lint_enabled() {
            return;
        }
        // Only whoever can act on it.  The cure this advice names — lift the innermost
        // part into its own function — is an edit to the function's OWN source, so a
        // consumer importing a library gets a nudge about code they cannot change:
        // three of the six bundled libraries printed one into every build that `use`d
        // them, the Lexer chapter of the reference among them.  `source_is_owned` is
        // relative to what is being built, so the library's own author still sees it
        // when they compile the library.  Same gate, same reason, as
        // `advise_group_apart`.
        if !self
            .data
            .source_is_owned(self.data.def(self.context).source)
        {
            return;
        }
        let score = self.complexity.get(&self.context).copied().unwrap_or(0);
        if score < crate::keys::COMPLEXITY_ADVICE_AT {
            return;
        }
        let (depth, line) = self
            .cc_deepest
            .get(&self.context)
            .copied()
            .unwrap_or((0, 0));
        let name = self.data.def(self.context).original_name().clone();
        let at = crate::keys::COMPLEXITY_ADVICE_AT;
        // The whole body is parsed before the score exists, so the cursor has
        // drifted to the NEXT definition — the caret pointed at the following
        // `fn` while the prose named this one, which reads as a diagnostic about
        // a function that is fine.  Point at the definition's own position.
        let at_pos = self.data.def(self.context).position;
        diagnostic_at!(
            self.lexer,
            &at_pos,
            Level::Advice,
            code = "function-complexity",
            "`{name}` scores {score} for control-flow complexity (nudge at {at}) — its \
             deepest nesting is {depth} levels, at line {line}"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: format!("lift the innermost part at line {line} into its own function"),
            condition: Some("nesting is what the score charges — a long flat sequence of branches costs almost nothing".to_string()),
            edit: None,
            concept: "functions",
            concept_ref: "@F16",
        });
    }

    /// Advice: this function asks its callers for too many separate things.
    ///
    /// Counts only REQUIRED parameters — a defaulted one costs the caller nothing, and
    /// hidden ones (`__retbuf`, work buffers) were injected by the compiler, not written.
    /// Kept separate from the complexity nudge because the burdens differ: parameters are
    /// what a caller carries and the fix is a struct; nesting is what a reader carries and
    /// the fix is an extracted function. A function can be trivial to read and still hard to
    /// call — `th_subdiv` takes 12 with a complexity of 2 — which is exactly the case a
    /// combined score misses.
    pub(super) fn warn_parameter_count(&mut self) {
        if self.first_pass || self.default || !crate::keys::param_count_lint_enabled() {
            return;
        }
        let def = self.data.def(self.context);
        // @PLN128 arc D — a `#c` binding's parameter list is the C function's,
        // not the author's, so both cures below are impossible: a struct cannot
        // cross the boundary at all (`c_signature::shape_of` refuses a record,
        // because a store may move it), and a default does not change the arity
        // the C signature declares — it only hides the parameter from this
        // count. Every Fortran routine trips the nudge (`dgemm_` takes 13), so
        // leaving it on would mean advising every numeric binding to do
        // something it cannot do.
        if !def.c_sig.is_empty() {
            return;
        }
        let required = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && matches!(a.value, Value::Null))
            .count() as u32;
        if required < crate::keys::PARAM_ADVICE_AT {
            return;
        }
        let name = def.original_name().clone();
        let at = crate::keys::PARAM_ADVICE_AT;
        // Emitted after the body is parsed — see `warn_function_complexity`.
        let at_pos = def.position;
        diagnostic_at!(
            self.lexer,
            &at_pos,
            Level::Advice,
            code = "too-many-parameters",
            "`{name}` takes {required} required parameters (nudge at {at}) — every caller \
             has to get all {required} right, in order"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "group the parameters that travel together into a struct".to_string(),
            condition: Some("some of them are one thing the caller already has".to_string()),
            edit: None,
            concept: "struct records",
            concept_ref: "@F12",
        });
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "give the optional ones defaults so callers can leave them out".to_string(),
            condition: Some("some have a value that is right most of the time".to_string()),
            edit: None,
            concept: "default parameters",
            concept_ref: "@F17",
        });
    }

    /// Advice: these trailing boolean parameters want defaults.
    ///
    /// Fires only on a CLUSTER — two or more trailing booleans, none defaulted. One trailing
    /// flag is ordinary; two is where a call site turns into `f(x, true, false)` and stops
    /// saying anything at the point a reader meets it.
    ///
    /// This advertises a feature rather than reporting a fault, which is why it must stay
    /// quiet: the shape is rare (96.9% of real loft has none), and a nudge that fired on the
    /// common case would be suppressed — taking the thing it was advertising with it.
    ///
    /// Adopting it costs nothing under the compatibility promise, and the message says so,
    /// because that is the part people do not know: giving an existing parameter a default is
    /// additive, so every call that passes it today keeps working unchanged.
    pub(super) fn warn_boolean_flag_cluster(&mut self) {
        if self.first_pass || self.default || !crate::keys::default_params_lint_enabled() {
            return;
        }
        let def = self.data.def(self.context);
        let mut trailing = 0u32;
        for a in def.attributes().iter().rev() {
            if a.hidden {
                continue;
            }
            if matches!(a.typedef.base(), Type::Boolean) && matches!(a.value, Value::Null) {
                trailing += 1;
            } else {
                break;
            }
        }
        if trailing < crate::keys::BOOL_FLAG_ADVICE_AT {
            return;
        }
        let name = def.original_name().clone();
        // Emitted after the body is parsed — see `warn_function_complexity`.
        let at_pos = def.position;
        diagnostic_at!(
            self.lexer,
            &at_pos,
            Level::Advice,
            code = "trailing-boolean-parameters",
            "`{name}` ends with {trailing} boolean parameters — a call reading \
             `{name}(…, true, false)` says nothing about which flag is which"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title:
                "give them defaults (`loud: boolean = false`) so callers pass only what they change"
                    .to_string(),
            condition: None,
            edit: None,
            concept: "default parameters",
            concept_ref: "@F17",
        });
    }
}
