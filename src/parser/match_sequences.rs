// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Vector, slice and tuple matches: their element patterns, alternatives inside a sequence, and
//! the scalar condition chains a pattern lowers to.
//! A part of the match subject split out of `control.rs`; the others are listed there.

use super::control::{
    PatternArm, SliceGroupKind, SliceShape, SliceSubject, SubruleGroup, chain_pattern_arms,
    guarded_arm, mark_forward_call, match_subject_borrows_off, slice_arms_cover_every_length,
};
use super::{I32, Level, Parser, Type, Value, diagnostic_format, v_block, v_if, v_set};
use crate::data::Deps;
use crate::data::IntegerSpec;
use std::collections::HashSet;

impl Parser {
    #[expect(
        clippy::too_many_lines,
        reason = "one condition chain over a vector subject's arms: each arm's slice pattern, length test, element conditions and bindings extend it"
    )]
    pub(super) fn parse_vector_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
    ) -> Type {
        let elm_tp = subject_type.content();
        // A subject that is already a VARIABLE is matched as itself.  Through a `_match_subj`
        // alias, every capture was a view of a compiler temporary (`OpGetVector(_match_subj,
        // …)`), and a removal or growth of the variable in the arm never shook it
        // (`@FR-B-View` × `@FR-P-Cap-View`): `[first, ..] => { u.remove(0); first.x }` read
        // the element that moved into slot 0.  A computed subject still gets its temporary,
        // so it is evaluated once.
        let subject_var = match subject.unspan() {
            // An author's variable only: a compiler buffer (an iterator subject drained into
            // one) is filled by statements of its own around the match.
            Value::Var(x)
                if !matches!(self.vars.tp(*x).base(), Type::RefVar(_))
                    && !self.vars.is_compiler_generated(*x) =>
            {
                Some(*x)
            }
            _ => None,
        };
        let v = subject_var.unwrap_or_else(|| self.create_match_subject(subject_type));
        self.vars.defined(v);
        // `@FR-B-View`: the subject temp is a second name for the subject, so it borrows what
        // the subject borrows.  Pass 2 meets pass 1's variable, typed before the subject's
        // borrows were known (an iterator's buffer gains its `__vdb` only in pass 2), so they
        // are added here; left dep-less, the temp owned that store beside its `__vdb`.  A
        // subject matched as ITSELF has no temp: its borrows are already its own, and adding
        // them again made the author's vector a borrower (`v += […]` in an arm rebuilt it).
        if !self.first_pass && subject_var.is_none() && !match_subject_borrows_off() {
            for d in subject_type.depend() {
                self.vars.depend(v, d);
            }
        }
        let elm_size = Value::Int(self.element_store_size(&elm_tp));
        // @PLN35 P-Cap-View — a heap element view's borrow-dep must name the SUBJECT's
        // source var (the caller's param/local), not the internal `_match_subj` copy `v`,
        // so the return-ownership chain reaches the caller's value. Fall back to `v` when
        // the subject isn't a plain var.
        let borrow_src = self.match_borrow_source(&subject).unwrap_or(v);

        self.lexer.token("{");
        let mut result_type = Type::Void;
        let saved_null_arms = std::mem::replace(&mut self.null_literal_arms, 0);
        let mut arms: Vec<PatternArm> = Vec::new();
        // loft#1682 — a `PatternArm` carries no type; the join reconverts by this list.
        let mut arm_types: Vec<Type> = Vec::new();
        let mut has_wildcard = false;
        // `(M-Total)`'s vector clause — each arm's length shape beside whether it is guarded, in
        // the arms' order, so the set can be asked whether it covers every length.
        let mut shapes: Vec<(Option<SliceShape>, bool)> = Vec::new();
        // @PLN35 PC2 — statements hoisted BEFORE the arm if-chain (evaluated once, at match-level
        // scope so a binding is visible to every arm body — native scopes cond sub-blocks away).
        // A sub-rule call goes here: `name = rule(cursor)` runs unconditionally, then each arm's
        // cond null-checks `name` (mirrors the hand-form `r = rule(c); if r != null { … }`).
        let mut prechain: Vec<Value> = Vec::new();
        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            let mut bindings: Vec<Value> = Vec::new();
            let mut cond: Option<Value> = None;
            // Set by a `_` or bare-name arm head: this pattern matches every subject.  It only
            // makes the MATCH exhaustive if no guard follows, so `has_wildcard` is decided after
            // the guard is parsed — `_ if c` can fail, and counting it as total would let a
            // subject fall through with no arm selected (loft#839).
            let mut is_total = false;
            // @PLN35 L2 — conditions from element sub-patterns (`[Variant { f }, ..]`); empty for
            // bare-name / `..` / `_` slice patterns, so those stay byte-identical.
            let mut elem_conds: Vec<Value> = Vec::new();
            // `@FR-P-Multi` — the further `|` / `,` slice patterns of this arm, each its own
            // bindings and condition; the body runs under whichever matched (loft#2107).
            let mut alternatives: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
            let mut arm_shapes: Vec<Option<SliceShape>> = Vec::new();
            // Every pattern of the arm takes a token before its body runs.
            let mut arm_consumes = false;
            // `@FR-N-Match` — a nullable VECTOR subject takes the `null` arm and the `x: _`
            // capture every other subject takes (`@FR-P-Cap` at the arm root): without them the
            // arm head matched none of `[`, `_` or a bare name, and the match did not parse
            // (loft#2427).  The null arm tests the subject's sentinel; on a dense subject it
            // never matches.
            if self.lexer.has_token("null") {
                if !self.first_pass {
                    cond = self.null_test(Value::Var(v), subject_type, false);
                }
            } else if !matches!(subject_type.base(), Type::Vector(_, _))
                && self.lexer.peek_token("[")
            {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a keyed collection has no positions for a slice pattern to match; \
                         match it as a whole (`null`, `x: _`, `_`) and look elements up by key"
                    );
                }
                break;
            } else if self.lexer.has_token("[") {
                let subject = SliceSubject {
                    v,
                    elm_tp: &elm_tp,
                    elm_size: &elm_size,
                    borrow_src,
                };
                let first_names = self.pattern_binds_pending.len();
                arm_shapes.push(self.parse_slice_pattern(
                    &subject,
                    &mut bindings,
                    &mut cond,
                    &mut elem_conds,
                    &mut prechain,
                ));
                arm_consumes = self.slice_consumes;
                alternatives = self.parse_pattern_alternatives(
                    first_names,
                    &mut bindings,
                    |p| p.lexer.has_token("["),
                    |p, binds, conds| {
                        let mut cond_i = None;
                        let mut elem_i = Vec::new();
                        arm_shapes.push(p.parse_slice_pattern(
                            &subject,
                            binds,
                            &mut cond_i,
                            &mut elem_i,
                            &mut prechain,
                        ));
                        arm_consumes &= p.slice_consumes;
                        conds.extend(cond_i);
                    },
                );
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_total = true;
                } else {
                    // bare name — wildcard binding; `name: _` is the same capture spelled with
                    // its pattern (`@FR-P-Cap`).
                    let bind_nr = self.pattern_binding(&id, subject_type);
                    self.vars.defined(bind_nr);
                    // A KEYED subject's capture is a view of its store, bound once ahead of the
                    // arm chain: first bound inside an arm, it was pre-inited to null there, and
                    // a keyed null-init mints a store the view then overwrote — one orphaned
                    // store per match (loft#2427).
                    if matches!(subject_type.base(), Type::Vector(_, _)) {
                        bindings.push(v_set(bind_nr, Value::Var(v)));
                    } else {
                        prechain.push(v_set(bind_nr, Value::Var(v)));
                    }
                    is_total = true;
                    if self.lexer.has_token(":")
                        && self.lexer.has_identifier().as_deref() != Some("_")
                        && !self.first_pass
                    {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "a vector match arm captures the whole subject as `{id}: _`; a slice \
                             pattern binds its parts by name inside `[…]`"
                        );
                    }
                }
            } else {
                // Unrecognized arm head (not `[…]`, `_`, or a binding). No branch above
                // consumed a token, so break to make progress — the arm loop would
                // otherwise spin forever on this token in the first pass (diagnostics silent).
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected slice pattern '[...]', 'null' or '_' in vector match arm"
                    );
                }
                break;
            }
            // Parse the optional guard.  Captures bound by the pattern above are in scope
            // for it, and `chain_pattern_arms` assigns them before the guard runs.
            let mut guard_opt = if self.lexer.has_token("if") {
                let at = *self.lexer.pos();
                let mut guard = Value::Null;
                let gt = self.outside_cursor_mode(|p| p.expression(&mut guard));
                self.record_guard(&guard, at);
                if !self.first_pass && gt != Type::Boolean {
                    self.convert(&mut guard, &gt, &Type::Boolean);
                }
                Some(guard)
            } else {
                None
            };
            // @PLN35 — a PEG cursor arm ADVANCES the shared cursor from its `bindings`, which
            // run before the guard.  A guard that then fails would leave the cursor consumed
            // while the next arm re-matches from the wrong position, so the whole parse would
            // silently read the wrong tokens.  Refuse the combination rather than ship that:
            // the guard's test belongs in the arm body, where the cursor is already committed.
            if guard_opt.is_some() && self.match_cursor.is_some() {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a guard is not supported on a cursor match arm — it would advance the cursor before the guard is tested; test inside the arm body instead"
                    );
                }
                guard_opt = None;
            }
            if is_total && guard_opt.is_none() {
                has_wildcard = true;
            }
            // The arm's shapes, one per alternative, in the order the arms are pushed below.
            if arm_shapes.is_empty() {
                shapes.push((None, guard_opt.is_some()));
            } else {
                shapes.extend(arm_shapes.drain(..).map(|s| (s, guard_opt.is_some())));
            }
            self.expect_match_arm_arrow();
            let mut arm_code = Value::Null;
            // #556 — a `{ … }` arm body is a VALUE position (its result is the match value), so
            // parse it as a `match_arm` block with an INFERRED (`Unknown`) result type, exactly
            // as the enum/scalar match arm does.  Routing it through `self.expression` instead
            // delegated to `parse_block("block", …, &Type::Void)`, which DROPS the trailing
            // result expression (`{ c = 5; c }` → `c = 5; drop c`) — the block then yielded void,
            // so native delivered 0 (interpret happened to still surface the value).
            let arm_expected = self.match_arm_expected(&result_type);
            // `@FR-P-Rule` — a sub-rule called on this cursor in the body runs after the arm took
            // a token, so its edge is a consuming one.
            let consumed_cursor = self
                .match_cursor
                .filter(|_| arm_consumes)
                .map(|(cursor_var, ..)| cursor_var);
            self.cursor_consumed.extend(consumed_cursor);
            let mut arm_type =
                self.outside_cursor_mode(|p| p.parse_match_arm_body(&arm_expected, &mut arm_code));
            if consumed_cursor.is_some() {
                self.cursor_consumed.pop();
            }
            // The alternatives run the BODY under their own bindings, so they take it before
            // the first pattern's bindings fold into it below.
            let mut bare_body = if alternatives.is_empty() {
                Value::Null
            } else {
                arm_code.clone()
            };
            let bare_type = arm_type.clone();
            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_code, &arm_type);
            self.match_void_arm |= matches!(arm_type.base(), Type::Void);
            if Self::arm_settles_match(&result_type, &arm_type) {
                result_type = arm_type.clone();
            } else if let Some(joined) =
                self.join_null_tuple_arm(&result_type, &mut arm_code, &mut arm_type)
            {
                for (prev, prev_tp) in arms.iter_mut().zip(arm_types.iter_mut()) {
                    if self.arm_takes_join(prev_tp, &joined) {
                        let prev_orig = prev_tp.clone();
                        self.convert_arm_tail(&mut prev.code, &prev_orig, &joined);
                        *prev_tp = joined.clone();
                    }
                }
                result_type = joined;
            }
            arm_types.push(arm_type.clone());
            // Without a guard the bindings fold into the body exactly as before; with one they
            // stay separate so `chain_pattern_arms` can run them ahead of the guard.
            let arm_bindings = if guard_opt.is_some() {
                std::mem::take(&mut bindings)
            } else {
                if !bindings.is_empty() {
                    bindings.push(arm_code);
                    arm_code = v_block(bindings, arm_type.clone(), "slice_binding");
                }
                Vec::new()
            };
            // Each alternative runs a CLONE of the arm body under its own bindings, which
            // assign the shared slots the first pattern's do — as the tuple match does.
            if !alternatives.is_empty() && bare_type != arm_type {
                self.convert_arm_tail(&mut bare_body, &bare_type, &arm_type);
            }
            let alternative_arms: Vec<PatternArm> = alternatives
                .into_iter()
                .map(|(mut binds_i, conds_i)| {
                    let cond_i = conds_i
                        .into_iter()
                        .reduce(|a, b| v_if(a, b, Value::Boolean(false)));
                    let (code_i, bindings_i) = if guard_opt.is_some() || binds_i.is_empty() {
                        (bare_body.clone(), binds_i)
                    } else {
                        binds_i.push(bare_body.clone());
                        (
                            v_block(binds_i, arm_type.clone(), "slice_binding"),
                            Vec::new(),
                        )
                    };
                    PatternArm {
                        cond: cond_i,
                        guard: guard_opt.clone(),
                        bindings: bindings_i,
                        code: code_i,
                    }
                })
                .collect();
            arms.push(PatternArm {
                cond,
                guard: guard_opt,
                bindings: arm_bindings,
                code: arm_code,
            });
            for alt in alternative_arms {
                arm_types.push(arm_type.clone());
                arms.push(alt);
            }
            if has_wildcard {
                self.lexer.has_token(",");
                break;
            }
            self.match_arm_separator();
        }
        self.lexer.token("}");

        // @PLN35 Phase 2 (F6 / M-Total): one slice pattern is length-constrained, so it is
        // non-total — a well-typed vector can always have a length that arm does not match.
        // The match is exhaustive when its final arm is total (a `_` or a bare binding, both of
        // which set `has_wildcard` and force the arm last), or — `(M-Total)`'s vector clause,
        // loft-lang/plans#202 P4 — when the slice arms together cover every length: `[]` and
        // `[x, ..rest]`.  The last arm's test is then dropped as the fallback, as a `_`'s is,
        // so the join is not nullable.  Without either, a subject would fall through with no
        // arm selected; reject it statically, mirroring the enum exhaustiveness gate.
        let lengths_total =
            !has_wildcard && self.match_cursor.is_none() && slice_arms_cover_every_length(&shapes);
        let exhaustive = has_wildcard || lengths_total;
        if !self.first_pass && !exhaustive {
            diagnostic!(
                self.lexer,
                Level::Error,
                "match on vector is not exhaustive — a slice pattern can fail (a length no arm matches); add a '_ =>' or a bare-binding final arm, or arms that together cover every length (`[]` and `[x, ..rest]`)"
            );
        }

        // Build if-else chain from arms
        let has_null_arm = std::mem::replace(&mut self.null_literal_arms, saved_null_arms) > 0;
        self.type_null_arms(arms.iter_mut().map(|a| &mut a.code), &result_type);
        let fallback = if exhaustive {
            arms.pop().unwrap().code
        } else {
            self.null_value(&result_type)
        };
        let chain = chain_pattern_arms(arms, fallback, &result_type);
        let mut block_ops = if subject_var.is_some() {
            Vec::new()
        } else {
            vec![v_set(v, subject)]
        };
        block_ops.append(&mut prechain);
        block_ops.push(chain);
        // The tuple and vector chains never widened for a `null` arm, scalars included.
        if has_null_arm && crate::keys::pln25_dn1_enabled() && Self::null_arm_widens(&result_type) {
            result_type = Type::optional(result_type);
        }
        *code = v_block(block_ops, result_type.clone(), "vector_match");
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// One slice pattern `[ … ]` of a vector match arm, the `[` already taken: its bindings,
    /// its condition and the element sub-pattern conditions ANDed onto it, and any sub-rule
    /// call hoisted ahead of the arm chain.  Called once per pattern of an arm, so a
    /// multi-pattern arm (`@FR-P-Multi`) parses each of its slice patterns here.
    #[expect(clippy::too_many_lines, reason = "inherited from parse_vector_match")]
    fn parse_slice_pattern(
        &mut self,
        subject: &SliceSubject,
        bindings: &mut Vec<Value>,
        cond: &mut Option<Value>,
        elem_conds: &mut Vec<Value>,
        prechain: &mut Vec<Value>,
    ) -> Option<SliceShape> {
        let SliceSubject {
            v,
            elm_tp,
            elm_size,
            borrow_src,
        } = *subject;
        // `(M-Total)` — an element sub-pattern that can fail adds a condition here; the arm's
        // elements are total when none did.
        let conds_at_entry = elem_conds.len();
        // Parse slice pattern elements
        // @PLN35 Phase 4.3: set when a whole-slice MULTI-element alternation built the
        // arm `cond` itself (predictive dispatch); suppresses the fixed-length gate below.
        let mut multi_alt = false;
        let mut head: Vec<String> = Vec::new();
        let mut tail: Vec<String> = Vec::new();
        // How many fixed elements follow the `..`, known as soon as the rest is seen.
        let mut tail_total: i32 = 0;
        let mut has_rest = false;
        let mut rest_name: Option<String> = None;
        // How the rest was spelled (`..xs` or `xs:integer*`), for the second-rest refusal.
        let mut rest_label = String::new();
        // The fewest elements the rest may take: 1 for a scalar `xs:T+`, else 0.
        let mut rest_min: i32 = 0;
        // `@FR-P-Rule` — a cursor pattern is read in SEGMENTS: the fixed elements up to a
        // sub-rule are read from the position the segment starts at, and the sub-rule's own
        // advance starts the next segment.  `seg_start` is the head index of the current
        // segment's first element; outside a cursor, and before any sub-rule, it is 0 and every
        // read is the one it always was.
        let mut seg_start: usize = 0;
        let entry_cursor = self.match_cursor;
        loop {
            if self.lexer.has_token("]") {
                break;
            }
            // @PLN35 — classify a `( … )` group ONCE per element (a second look-ahead
            // over the same region corrupts the lexer replay buffer).  A `Repetition`
            // may sit MID-slice (fixed head before it); an `Alt` is whole-slice only, so
            // its branch keeps the `head.is_empty()` guard.
            // `@FR-P-Cap` over a GROUP, @C161 — `g:( … )?`, `g:( … | … )`, `g:( … )*`: the name
            // binds the sub-slice the group matched.  Read before the group is classified, so the
            // group's own handler binds it.  A cursor pattern's `name: rule` is not this.
            let mut group_cap: Option<String> = None;
            if !has_rest && self.match_cursor.is_none() && self.lexer.peek_named_arg().is_some() {
                let lnk = self.lexer.link();
                let name = self.lexer.has_identifier();
                self.lexer.token(":");
                if self.lexer.peek_token("(") {
                    group_cap = name;
                } else {
                    self.lexer.revert(lnk);
                }
            }
            let group_kind = if !has_rest && self.lexer.peek_token("(") {
                self.peek_group_kind()
            } else {
                SliceGroupKind::Other
            };
            if group_cap.is_some() && group_kind == SliceGroupKind::Other && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{}:( … )` captures a GROUP — a `( … )?`, `( … | … )` or `( … )*` over a \
                     struct-enum element; this `( … )` is none of those",
                    group_cap.as_deref().unwrap_or("")
                );
            }
            if self.lexer.has_token("..") {
                // @PLN35 Phase 2 (P-Rest) — `..name` with the name ADJACENT to `..` (no
                // comma) captures the tail sub-slice as a FRESH vector. `.., x` (a comma
                // before the name) stays a bare gap plus a tail binding. Named rest is
                // tail-only (enforced after the loop).
                let name = self.lexer.has_identifier();
                // `@FR-P-Rest` — `t` counts the FIXED patterns after the rest, so a
                // second variable-length part has no length of its own: the first takes
                // every element the fixed ones leave.  Accepting it dropped the first
                // rest's binding without a word (`[..a, ..b]` bound only `b`).
                if has_rest && !self.first_pass {
                    let second = format!("..{}", name.as_deref().unwrap_or(""));
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a slice pattern holds one variable-length part, and `{second}` is a second one after `{rest_label}` — the first already takes every element the fixed ones leave; drop one of them"
                    );
                }
                if has_rest {
                    // Declared so the arm body still parses: the refusal above is the
                    // one error, not an `Unknown variable` cascade behind it.
                    if let Some(n) = &name {
                        self.declare_refused_rest(n, elm_tp);
                    }
                } else {
                    rest_label = format!("..{}", name.as_deref().unwrap_or(""));
                    rest_name = name;
                }
                has_rest = true;
                // Everything from here to `]` is the fixed TAIL, and each of its
                // elements is read from the end.  Count them now so a variant
                // sub-pattern or a literal there knows its own index while it parses
                // (loft#1419); a bare name does not need it, which is why only bare
                // names used to be admitted.
                self.lexer.has_token(",");
                tail_total = self.count_slice_tail_elements();
            } else if self.match_cursor.is_some()
                && !has_rest
                && let Some((name, fn_nr, ret_tp, quant)) = self.peek_subrule_group()
            {
                // `@FR-P-Rule` × `@FR-P-Rep` / `@FR-P-Opt` — `(name: rule)*`, `+` or `?`: the
                // segment before it is tested and bound, the group runs the rule as often as it
                // matches (once at most for `?`), and the next segment starts where it stopped.
                self.flush_cursor_segment(
                    subject,
                    &mut head,
                    seg_start,
                    cond,
                    elem_conds,
                    conds_at_entry,
                    bindings,
                );
                let k = head.len() - seg_start;
                let consumed = !head.is_empty();
                let group = SubruleGroup {
                    name,
                    fn_nr,
                    ret_tp,
                    quant,
                };
                self.parse_subrule_group(subject, &group, k, consumed, cond, prechain);
                seg_start = head.len();
            } else if self.match_cursor.is_some()
                && !has_rest
                && let Some((name, fn_nr, ret_tp)) = self.peek_subrule_capture()
            {
                // `@FR-P-Rule` — a sub-rule invocation `name: rule` at any element of a cursor
                // pattern.  The segment before it is tested and bound first, then the rule runs
                // from the position after that segment, and the next segment is read from where
                // the rule left the cursor.
                self.flush_cursor_segment(
                    subject,
                    &mut head,
                    seg_start,
                    cond,
                    elem_conds,
                    conds_at_entry,
                    bindings,
                );
                let k = head.len() - seg_start;
                let consumed = !head.is_empty();
                self.parse_subrule_slice_element(
                    &name, fn_nr, &ret_tp, k, consumed, cond, prechain,
                );
                seg_start = head.len();
            } else if self.pattern_variant_enum(elm_tp).is_none()
                && let Some(is_rep) = self.peek_scalar_type_capture()
                && (is_rep || !has_rest)
            {
                // @PLN35 slice 1 — a scalar type-annotated capture `name:Type` (single —
                // a type-as-match that always holds for the element type) OR its
                // `name:Type*` / `+` repetition collected into a `vector<elm_tp>`.  Only
                // for a SCALAR element type; enum elements keep the `name:pat` /
                // `(x: V)*` paths below.
                if is_rep {
                    // `@FR-P-Rep-Scalar` — `Type` is the element type, so EVERY element
                    // matches it and the run is a TYPED REST: `(P-Rest)` with `t` the
                    // fixed patterns after it, whatever they are, and `+` asking one
                    // element more.  It is parsed as the rest it is, so its tail takes
                    // every element form a `..rest` tail takes (loft#1736).
                    let name = self.lexer.has_identifier().unwrap_or_default();
                    self.lexer.token(":");
                    let tname = self.lexer.has_identifier().unwrap_or_default();
                    let elm_name = elm_tp.source_name(&self.data);
                    if !self.first_pass && tname != elm_name {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "a scalar repetition `{name}:{tname}*` must match the vector's element type {elm_name}"
                        );
                    }
                    let plus = self.lexer.has_token("+");
                    if !plus {
                        self.lexer.token("*");
                    }
                    let label = format!("{name}:{tname}{}", if plus { "+" } else { "*" });
                    if has_rest {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "a slice pattern holds one variable-length part, and `{label}` is a second one after `{rest_label}` — the first already takes every element the fixed ones leave; drop one of them"
                            );
                        }
                        self.declare_refused_rest(&name, elm_tp);
                    } else {
                        has_rest = true;
                        rest_label = label;
                        rest_name = Some(name);
                        rest_min = i32::from(plus);
                        if self.lexer.has_token(",") {
                            tail_total = self.count_slice_tail_elements();
                        }
                    }
                    continue;
                }
                // Single `name:Type`: bind `name = v[pos]`; the annotation matches the
                // element type so no condition is needed (a "_" keeps position alignment
                // for following bare-name indices).
                let position = (head.len() - seg_start) as i32;
                let name = self.lexer.has_identifier().unwrap();
                self.lexer.token(":");
                let tname = self.lexer.has_identifier().unwrap_or_default();
                let elm_name = elm_tp.source_name(&self.data);
                if !self.first_pass && tname != elm_name {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a scalar capture `{name}:{tname}` must match the element type {elm_name}"
                    );
                }
                let bind_nr = self.pattern_binding(&name, elm_tp);
                self.vars.defined(bind_nr);
                let val = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                bindings.push(v_set(bind_nr, val));
                self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
                head.push("_".to_string());
            } else if !has_rest && self.lexer.peek_named_arg().is_some() {
                // @PLN35 Phase 2 (P-Cap) — `name:pat` element capture. Bind the whole
                // element to `name` (a VIEW of the subject, exactly the read a bare head
                // binding emits) AND require sub-pattern `pat` to match the same element.
                // Head position only (before any `..`); a `_` placeholder keeps the
                // position count aligned for following bare-name indices, as the variant
                // sub-pattern branch does.
                let name = self.lexer.has_identifier().unwrap();
                self.lexer.token(":");
                let position = (head.len() - seg_start) as i32;
                // Bind name = v[position] — the same read as the head-binding loop below.
                let bind_nr = self.pattern_binding(&name, elm_tp);
                self.vars.defined(bind_nr);
                let bval = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                bindings.push(v_set(bind_nr, bval));
                self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
                // Sub-pattern condition on the SAME element. Read v[position] AGAIN: the
                // binding var is assigned only in the arm body (after the condition runs),
                // so the condition must read the element directly, never the binding.
                let cread = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                let mut sub_conds: Vec<Value> = Vec::new();
                let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                if let Some(c) = self.parse_field_sub_pattern(
                    cread,
                    elm_tp,
                    bindings,
                    &mut sub_conds,
                    &mut aliases,
                ) {
                    elem_conds.push(c);
                }
                elem_conds.append(&mut sub_conds);
                // The names a sub-pattern bound belong to this arm; its end restores them.
                self.pattern_binds_pending.append(&mut aliases);
                head.push("_".to_string());
            } else if self.peek_is_variant_subpattern(elm_tp) {
                // Read v[pos] and tag-test + bind via parse_field_sub_pattern; a "_"
                // placeholder keeps the position count so following bare-name indices align.
                // A head element counts forward from 0; a tail element counts BACK from
                // the end, exactly as a bare tail name does.
                let position = if has_rest {
                    -(tail_total - tail.len() as i32)
                } else {
                    (head.len() - seg_start) as i32
                };
                let read = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                let mut sub_conds: Vec<Value> = Vec::new();
                let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                if let Some(c) = self.parse_field_sub_pattern(
                    read,
                    elm_tp,
                    bindings,
                    &mut sub_conds,
                    &mut aliases,
                ) {
                    elem_conds.push(c);
                }
                elem_conds.append(&mut sub_conds);
                // The names a sub-pattern bound belong to this arm; its end restores them.
                self.pattern_binds_pending.append(&mut aliases);
                if has_rest {
                    tail.push("_".to_string());
                } else {
                    head.push("_".to_string());
                }
            } else if group_kind == SliceGroupKind::Repetition {
                // @PLN35 Phase 6 — a repetition `[ head…, ( [name:] V )*[(Sep)] [tail…]
                // [, ..rest] ]` / `…+`.  `head` is any fixed prefix already parsed (its
                // binds/conds are in `bindings`/`elem_conds`); the run starts at `head_len`
                // and any fixed `tail` after the group is matched from the END.
                // `parse_slice_repetition` builds the run-loop cond + collection and
                // consumes through `]`, so break the element loop.
                if let Some((e_nr, true)) = self.pattern_variant_enum(elm_tp) {
                    let head_len = (head.len() - seg_start) as i32;
                    // Bind any BARE-NAME head element (a variant sub-pattern / literal
                    // already emitted its own bind/cond and left a "_").  These are safe:
                    // the arm's `cond` requires `len >= head_len` before bindings run.
                    for (i, name) in head.iter().enumerate().skip(seg_start) {
                        if name == "_" {
                            continue;
                        }
                        let bind_nr = self.pattern_binding(name, elm_tp);
                        self.vars.defined(bind_nr);
                        let at = Value::Int((i - seg_start) as i32);
                        let val = self.read_slice_elem(v, elm_size, elm_tp, at);
                        bindings.push(v_set(bind_nr, val));
                        self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
                    }
                    self.parse_slice_repetition(
                        e_nr,
                        v,
                        elm_size,
                        elm_tp,
                        head_len,
                        borrow_src,
                        cond,
                        bindings,
                        group_cap.take(),
                    );
                    multi_alt = true;
                } else {
                    if !self.first_pass {
                        let elm_name = self.slice_element_name(elm_tp);
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "a repetition `( … )*` slice element needs a struct-enum element type, not `{elm_name}`"
                        );
                    }
                    self.skip_rest_of_slice();
                }
                break;
            } else if group_kind == SliceGroupKind::Alt
                && (head.is_empty() || self.match_cursor.is_none())
            {
                // @PLN35 Phase 4.3 / 5 — a WHOLE-SLICE MULTI-element alternation
                // `[ (A B | C) [, ..rest] ]` OR an OPTIONAL group `[ (a)? [, ..rest] ]`
                // (a degenerate alternation `(a | ε)`).  Predictive dispatch on the leading
                // tags over a sequence per branch; it builds the arm `cond` itself and
                // consumes through `]`, so break the element loop and skip the length gate.
                if let Some((e_nr, true)) = self.pattern_variant_enum(elm_tp) {
                    // A fixed head before an optional group (loft#2048): its bare names
                    // bind as a repetition's head does — the arm's `cond` gates them.
                    let head_len = (head.len() - seg_start) as i32;
                    for (i, name) in head.iter().enumerate().skip(seg_start) {
                        if name == "_" {
                            continue;
                        }
                        let bind_nr = self.pattern_binding(name, elm_tp);
                        self.vars.defined(bind_nr);
                        let at = Value::Int((i - seg_start) as i32);
                        let val = self.read_slice_elem(v, elm_size, elm_tp, at);
                        bindings.push(v_set(bind_nr, val));
                        self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
                    }
                    self.parse_multi_element_alternation(
                        e_nr,
                        v,
                        elm_size,
                        elm_tp,
                        head_len,
                        borrow_src,
                        cond,
                        bindings,
                        group_cap.take().as_deref(),
                    );
                    multi_alt = true;
                } else {
                    if !self.first_pass {
                        let elm_name = self.slice_element_name(elm_tp);
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "an alternation `( … | … )` slice element needs a struct-enum element type, not `{elm_name}`"
                        );
                    }
                    self.skip_rest_of_slice();
                }
                break;
            } else if let Type::Tuple(elems) = elm_tp.base()
                && group_kind == SliceGroupKind::Other
                && self.lexer.has_token("(")
            {
                // `@FR-P-Point` × `(G-Pat-Group)` — a `( … )` with no pattern operator is
                // a TUPLE pattern, and over a tuple element it tests that one element,
                // position by position, exactly as a tuple subject's arm does.  It was
                // read as an alternation and refused (loft#1737).  A head element
                // counts forward from 0, a tail element back from the end.
                let elems = elems.clone();
                let position = if has_rest {
                    -(tail_total - tail.len() as i32)
                } else {
                    (head.len() - seg_start) as i32
                };
                let tmp = self.create_unique("slice_tuple", elm_tp);
                self.vars.defined(tmp);
                let mut t_binds: Vec<Value> = Vec::new();
                let mut t_conds: Vec<Value> = Vec::new();
                self.parse_tuple_pattern_elements(tmp, &elems, &mut t_binds, &mut t_conds);
                // The test reads the element itself, because the bindings run only once
                // the arm commits; the bindings read it again for the same reason.
                if !t_conds.is_empty() {
                    let mut test = t_conds.remove(0);
                    for c in t_conds {
                        test = v_if(test, c, Value::Boolean(false));
                    }
                    let read = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                    elem_conds.push(v_block(
                        vec![v_set(tmp, read), test],
                        Type::Boolean,
                        "tuple element test",
                    ));
                }
                if !t_binds.is_empty() {
                    let read = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                    bindings.push(v_set(tmp, read));
                    bindings.append(&mut t_binds);
                }
                if has_rest {
                    tail.push("_".to_string());
                } else {
                    head.push("_".to_string());
                }
            } else if !has_rest && self.lexer.peek_token("(") {
                // @PLN35 Phase 4 (P-Alt) — a parenthesized single-element alternation
                // `( V1 { f } | V2 { f } )` in a head slice-element position. Tag-test
                // the element against each branch and bind the shared captures from the
                // matching variant's offsets. A `_` placeholder keeps position alignment
                // for following bare-name indices, as the variant sub-pattern branch does.
                if let Some((e_nr, true)) = self.pattern_variant_enum(elm_tp) {
                    let position = (head.len() - seg_start) as i32;
                    let read = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(position));
                    self.parse_slice_alternation_element(
                        e_nr,
                        &read,
                        Some(borrow_src),
                        bindings,
                        elem_conds,
                        true,
                    );
                    head.push("_".to_string());
                } else {
                    if !self.first_pass {
                        let elm_name = self.slice_element_name(elm_tp);
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "an alternation `( … | … )` slice element needs a struct-enum element type, not `{elm_name}`"
                        );
                    }
                    self.skip_rest_of_slice();
                    break;
                }
            } else if self.peek_is_slice_literal() {
                // @PLN35 Phase 6.3 (P-Lit) — a LITERAL head element `[ 1, … ]` /
                // `[ "kw", … ]`.  On a SCALAR element it matches by direct EQUALITY
                // against `v[pos]`.  On a STRUCT-ENUM element (a token stream) it matches
                // against the variant's `#lexeme` field — so `"fn"` reads like the
                // grammar, standing in for `Keyword { name: "fn" }`.  A "_" placeholder
                // keeps following bare-name indices AND the length gate aligned, and the
                // condition is AND'd into the arm like a variant tag test.  In the TAIL
                // it reads at a negative index, the same way a bare tail name does.
                let position = if has_rest {
                    -(tail_total - tail.len() as i32)
                } else {
                    (head.len() - seg_start) as i32
                };
                let mut lit = Value::Null;
                let lit_tp = self.expression(&mut lit);
                match self.build_literal_match(
                    v,
                    elm_size,
                    elm_tp,
                    &Value::Int(position),
                    &lit,
                    &lit_tp,
                ) {
                    Some(c) => elem_conds.push(c),
                    None if !self.first_pass => self.slice_literal_mismatch(elm_tp, &lit_tp),
                    None => {}
                }
                if has_rest {
                    tail.push("_".to_string());
                } else {
                    head.push("_".to_string());
                }
            } else if let Some(id) = self.lexer.has_identifier() {
                if has_rest {
                    tail.push(id);
                } else {
                    head.push(id);
                }
            } else {
                // Unrecognized element token (`{`, an unsupported tail sub-pattern, …).
                // None of the branches above consumed anything, so this MUST break to make
                // progress — otherwise the loop spins forever on the same token in the
                // first pass (where diagnostics are silent).
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected identifier, literal, or '..' in slice pattern"
                    );
                }
                break;
            }
            self.lexer.has_token(",");
        }
        // @PLN35 Phase 4.3: a whole-slice multi-element alternation built the arm cond +
        // bindings itself (predictive dispatch), so skip the fixed-position length gate,
        // the head/tail binds, and the rest materialisation below.
        if !multi_alt {
            let fixed = (head.len() - seg_start + tail.len()) as i32;
            // Generate length condition
            let len_call = self.cursor_len(v);
            if let Some((cursor_var, cursor_def, pos_field, pos_var)) = self.match_cursor {
                // @PLN35 PC1 — cursor PREFIX-consume: need `fixed` elements from `pos`
                // (`pos + fixed <= len`), and on a match advance `cursor.pos` by `fixed`.
                // `pos_var` stays the ENTRY position, so every read AND this write share it.
                let ispec = Type::Integer(IntegerSpec {
                    min: 0,
                    max: 0,
                    not_null: false,
                    forced_size: None,
                });
                let need = self.conv_op(
                    "+",
                    Value::Var(pos_var),
                    Value::Int(fixed),
                    I32.clone(),
                    I32.clone(),
                );
                // ANDed after what the earlier segments tested (`@FR-P-Rule`), when a sub-rule
                // ended one.
                let mut gate = Value::Null;
                self.call_op(&mut gate, "<=", &[need, len_call], &[ispec.clone(), ispec]);
                *cond = Some(match cond.take() {
                    Some(c) => v_if(c, gate, Value::Boolean(false)),
                    None => gate,
                });
                let new_pos = self.conv_op(
                    "+",
                    Value::Var(pos_var),
                    Value::Int(fixed),
                    I32.clone(),
                    I32.clone(),
                );
                let adv = self.set_field(
                    cursor_def,
                    pos_field,
                    0,
                    Value::Var(cursor_var),
                    new_pos.clone(),
                );
                bindings.push(adv);
                // @PLN35 PC5 — maintain the farthest high-water mark: `farthest =
                // max(farthest, new_pos)`.  Monotonic across arms + sub-rules (the shared
                // cursor carries it), so after a failed parse it names the deepest token
                // reached.  Opt-in — only when the cursor has a `farthest` field.
                if let Some(ff) = self.match_cursor_farthest {
                    let old_far = self.get_field(cursor_def, ff, Value::Var(cursor_var));
                    let is_less = self.conv_op(
                        "<",
                        old_far.clone(),
                        new_pos.clone(),
                        I32.clone(),
                        I32.clone(),
                    );
                    let maxed = v_if(is_less, new_pos, old_far);
                    let set_far = self.set_field(cursor_def, ff, 0, Value::Var(cursor_var), maxed);
                    bindings.push(set_far);
                }
            } else if has_rest {
                // length >= fixed (+1 for a `xs:T+` rest)  →  fixed <= length
                self.call_op(
                    cond.get_or_insert(Value::Null),
                    "<=",
                    &[Value::Int(fixed + rest_min), len_call],
                    &[
                        Type::Integer(IntegerSpec {
                            min: 0,
                            max: 0,
                            not_null: false,
                            forced_size: None,
                        }),
                        Type::Integer(IntegerSpec {
                            min: 0,
                            max: 0,
                            not_null: false,
                            forced_size: None,
                        }),
                    ],
                );
            } else {
                // length == fixed
                self.call_op(
                    cond.get_or_insert(Value::Null),
                    "==",
                    &[len_call, Value::Int(fixed)],
                    &[
                        Type::Integer(IntegerSpec {
                            min: 0,
                            max: 0,
                            not_null: false,
                            forced_size: None,
                        }),
                        Type::Integer(IntegerSpec {
                            min: 0,
                            max: 0,
                            not_null: false,
                            forced_size: None,
                        }),
                    ],
                );
            }
            // Bind head elements: head[i] = v[i] (in a cursor, from the last segment's start)
            for (i, name) in head.iter().enumerate().skip(seg_start) {
                if name == "_" {
                    continue;
                }
                let bind_nr = self.pattern_binding(name, elm_tp);
                self.vars.defined(bind_nr);
                let at = Value::Int((i - seg_start) as i32);
                let val = self.read_slice_elem(v, elm_size, elm_tp, at);
                bindings.push(v_set(bind_nr, val));
                self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
            }
            // Bind tail elements: tail[j] = v[len - tail.len() + j]
            for (j, name) in tail.iter().enumerate() {
                if name == "_" {
                    continue;
                }
                let bind_nr = self.pattern_binding(name, elm_tp);
                self.vars.defined(bind_nr);
                let idx = Value::Int(-((tail.len() - j) as i32));
                let val = self.read_slice_elem(v, elm_size, elm_tp, idx);
                bindings.push(v_set(bind_nr, val));
                self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
            }
            // @FR-P-Rest — `..name` binds `name` to the FRESH sub-slice
            // `v[head_len .. len - tail_len]`, so fixed elements MAY follow the rest and
            // the middle shrinks by however many do.  Reuse the proven compile-time slice
            // materialisation (`materialize_iterator`): a minimal slice `Value::Iter` over the
            // index range, copied element-type-aware into a fresh vector (P-Cap-Fresh — the
            // result is INDEPENDENT of the subject, so it is safe to return or mutate).
            // Bounds are in range by the `fixed <= len` arm condition, which counts head AND
            // tail.  The tail elements are already bound above, at negative indices — the
            // same reads the un-named gap `[a, .., z]` has always used, which is why naming
            // the middle needs no machinery of its own.  A tail element is a BARE NAME:
            // a variant sub-pattern or literal after a `..` is not parsed in either
            // spelling (loft#1419), which is the one part of `t` still outstanding.
            if let Some(name) = rest_name.clone() {
                let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
                let rest_var = self.pattern_binding(&name, &vec_tp);
                self.vars.defined(rest_var);
                if !self.first_pass && rest_var != u16::MAX {
                    let lo_val = Value::Int((head.len() - seg_start) as i32);
                    let len_c = self.cursor_len(v);
                    let hi_val = self.conv_op(
                        "-",
                        len_c,
                        Value::Int(tail.len() as i32),
                        I32.clone(),
                        I32.clone(),
                    );
                    self.materialize_named_rest(
                        v, elm_tp, elm_size, rest_var, &vec_tp, lo_val, hi_val, 1, bindings,
                    );
                }
            }
        } // end `if !multi_alt`
        let elements_total = elem_conds.len() == conds_at_entry;
        // @PLN35 L2 — AND element sub-pattern conditions into the arm condition AFTER the
        // length check, so `&&` short-circuit never reads past the end.
        for ec in elem_conds.drain(..) {
            match cond.take() {
                // `c && ec` via short-circuit if (the AND idiom used elsewhere in match
                // lowering) — avoids reading v[pos] when the length check already failed.
                Some(c) => *cond = Some(v_if(c, ec, Value::Boolean(false))),
                None => *cond = Some(ec),
            }
        }
        // `@FR-P-Atomic` — a sub-rule moved the cursor while the arm was tested, so an arm that
        // fails puts it back where the match started.
        if (seg_start > 0 || self.slice_subrule)
            && let Some((cursor_var, cursor_def, pos_field, entry_pos)) = entry_cursor
        {
            let back = self.set_field(
                cursor_def,
                pos_field,
                0,
                Value::Var(cursor_var),
                Value::Var(entry_pos),
            );
            let failed = v_block(
                vec![back, Value::Boolean(false)],
                Type::Boolean,
                "sub-rule miss",
            );
            let tested = cond.take().unwrap_or(Value::Boolean(true));
            *cond = Some(v_if(tested, Value::Boolean(true), failed));
        }
        self.slice_consumes = !head.is_empty();
        self.slice_subrule = false;
        self.match_cursor = entry_cursor;
        if multi_alt || self.match_cursor.is_some() {
            None
        } else {
            let fixed = (head.len() + tail.len()) as i32;
            Some(SliceShape {
                min: if has_rest { fixed + rest_min } else { fixed },
                exact: !has_rest,
                total: elements_total,
            })
        }
    }

    /// Does the sub-pattern at the cursor list alternatives — a `|` before the `,`, `)`, `}`
    /// or `=>` that ends it, outside any group?  Looks ahead and puts the cursor back.
    pub(super) fn sub_pattern_lists_alternatives(&mut self) -> bool {
        let at = self.lexer.link();
        let found =
            self.lexer.recover_to(&[",", ")", "}", "=>", "|"]) && self.lexer.peek_token("|");
        self.lexer.revert(at);
        found
    }

    /// The elements of a tuple pattern `( p₀, p₁, … )` over the tuple held in `tmp`, the lexer just
    /// past the `(`; consumes through the `)`.  Each position is `_`, a binding, a literal or a
    /// variant sub-pattern — `@FR-P-Point`: a tuple element is ONE value.  A binding lands in
    /// `bindings`, a test in `elem_conds`.  Answers whether the pattern was refused.  Shared by a
    /// tuple SUBJECT and a tuple ELEMENT of a slice pattern, which `(G-Pat-Group)` tells apart
    /// from a group by its missing operators.
    #[expect(
        clippy::too_many_lines,
        reason = "moved whole out of `parse_tuple_match`"
    )]
    fn parse_tuple_pattern_elements(
        &mut self,
        tmp: u16,
        elem_types: &[Type],
        bindings: &mut Vec<Value>,
        elem_conds: &mut Vec<Value>,
    ) -> bool {
        let mut bad_pattern = false;
        // Element-by-element pattern
        for (i, elem_type) in elem_types.iter().enumerate() {
            // The break is NOT gated on the pass: both passes must walk the
            // arm the same way, or the first one wanders into positions the
            // second never visits and stops making progress.
            if i > 0 && !self.lexer.has_token(",") {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected ',' between tuple pattern elements"
                    );
                }
                break;
            }
            // A `..` rest is not part of the tuple design — arity is fixed
            // (TUPLES.md § "What is NOT supported"), so there is nothing for a
            // rest to stand for.  Refuse it by name and skip to the closing
            // `)`, rather than letting it fall through to the literal branch
            // below, where `expression` consumes the `..` but leaves the `)`
            // unclaimed and the arm loop spinning (loft#832).
            if self.lexer.peek_token("..") || self.lexer.peek_token("..=") {
                bad_pattern = true;
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a `..` rest pattern is not supported in a tuple pattern — \
                         a tuple's arity is fixed, so write every position, \
                         using `_` for the ones you do not bind"
                    );
                }
                self.lexer.has_token("..=");
                self.lexer.has_token("..");
                self.lexer.recover_to(&[")"]);
                break;
            }
            let elem_type = elem_type.clone();
            let elem_get = Value::TupleGet(tmp, i as u16);
            // @FR-P-Cap at a tuple element — `(whole: Circle { r }, k)` binds the element as a
            // bare name does, and the sub-pattern after the `:` still tests it.
            if let Some(name) = self.lexer.peek_named_arg()
                && Self::is_binding_name(&name)
            {
                self.lexer.has_identifier();
                self.lexer.token(":");
                let bind_nr = self.pattern_binding(&name, &elem_type);
                self.vars.defined(bind_nr);
                self.tuple_pattern_element(i, Some(bind_nr)); // @PLN187
                bindings.push(v_set(bind_nr, elem_get.clone()));
            }
            if self.peek_is_variant_subpattern(&elem_type) {
                self.tuple_pattern_element(i, None); // @PLN187 — a tag test reads the member
                // @FR-P-Point — a unit or struct variant is a point pattern over ONE
                // value, and a tuple element is one value, so an enum element takes
                // the forms a slice element takes: `(Fire, Wall { hp })` tag-tests
                // each position and binds the payload it names, through the one
                // lowering the top-level arm and the slice head share.
                let mut sub_conds: Vec<Value> = Vec::new();
                let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                if let Some(c) = self.parse_field_sub_pattern(
                    elem_get,
                    &elem_type,
                    bindings,
                    &mut sub_conds,
                    &mut aliases,
                ) {
                    elem_conds.push(c);
                }
                elem_conds.append(&mut sub_conds);
                // The names a sub-pattern bound belong to this arm; its end restores them.
                self.pattern_binds_pending.append(&mut aliases);
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    // element wildcard — no condition, no binding
                } else if id.starts_with(char::is_uppercase) {
                    // @FR-M-Unit — a capitalised name is a VARIANT of the element's
                    // enum, or it is nothing: the language never reads one as a
                    // variable (`Foo = 5` is unknown), so it cannot be a binding here
                    // either, and binding it would make the arm match every tuple.
                    // Refused by name, as a top-level arm refuses it.
                    bad_pattern = true;
                    if !self.first_pass {
                        if let Some((e_nr, _)) = self.pattern_variant_enum(&elem_type) {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant of {}",
                                id,
                                self.data.def(e_nr).name()
                            );
                        } else {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant — this tuple element is {}, \
                                 which has no variants; a binding is lower_case",
                                id,
                                elem_type.source_name(&self.data)
                            );
                        }
                    }
                    // A payload written after a name that is no variant is skipped
                    // whole, so the arm reaches its `=>` with one diagnostic, not a
                    // cascade.  Step INSIDE the group first: `recover_to` skips a
                    // nested group as a unit, so asked from the `{` it would walk
                    // past the matching `}` and on through the elements after it.
                    if self.lexer.has_token("{") {
                        self.lexer.recover_to(&["}"]);
                        self.lexer.has_token("}");
                    }
                } else {
                    // binding variable — always matches, captures element value
                    let bind_nr = self.pattern_binding(&id, &elem_type);
                    self.vars.defined(bind_nr);
                    self.tuple_pattern_element(i, Some(bind_nr)); // @PLN187
                    bindings.push(v_set(bind_nr, elem_get));
                }
            } else {
                // literal: build elem_get == literal condition
                self.tuple_pattern_element(i, None); // @PLN187 — a literal compares the member
                let negate = self.lexer.has_token("-");
                let lit: Value = if let Some(n) = self.lexer.has_integer() {
                    let v = n as i32;
                    Value::Int(if negate { -v } else { v })
                } else if let Some(n) = self.lexer.has_long() {
                    let v = n as i64;
                    Value::Long(if negate { -v } else { v })
                } else if let Some(n) = self.lexer.has_float() {
                    Value::Float(if negate { -n } else { n })
                } else if let Some(s) = self.lexer.has_cstring() {
                    Value::Text(s)
                } else if self.lexer.has_token("true") {
                    Value::Boolean(true)
                } else if self.lexer.has_token("false") {
                    Value::Boolean(false)
                } else {
                    let mut e = Value::Null;
                    self.expression(&mut e);
                    e
                };
                let mut elem_cond = Value::Null;
                self.call_op(
                    &mut elem_cond,
                    "==",
                    &[elem_get, lit],
                    &[elem_type.clone(), elem_type],
                );
                elem_conds.push(elem_cond);
            }
        }
        if !self.lexer.has_token(")") {
            bad_pattern = true;
            if !self.first_pass {
                // A `,` here means the pattern listed MORE elements than the
                // subject has; anything else is ordinary junk.  Naming the
                // arity is what tells the author which side to change.
                if self.lexer.peek_token(",") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "tuple pattern has more elements than the {}-element subject tuple",
                        elem_types.len()
                    );
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected ')' to close tuple pattern"
                    );
                }
            }
            // Skip the surplus so the arm reaches its `=>` — without this the
            // cursor stays parked on the `,` and the arm loop spins (loft#832).
            self.lexer.recover_to(&[")"]);
            self.lexer.has_token(")");
        }
        bad_pattern
    }

    /// `@FR-P-Multi` over a tuple subject — the further tuple patterns of one arm, after `|`
    /// or `,` (`(Circle { r }, Square { s }) | (Square { s }, Circle { r }) => r * s`).
    fn parse_tuple_alternatives(
        &mut self,
        tmp: u16,
        elem_types: &[Type],
        first_names: usize,
        first_bindings: &mut Vec<Value>,
        bad_pattern: &mut bool,
    ) -> Vec<(Vec<Value>, Vec<Value>)> {
        let mut bad = false;
        let alternatives = self.parse_pattern_alternatives(
            first_names,
            first_bindings,
            |p| p.lexer.has_token("("),
            |p, binds, conds| {
                if p.parse_tuple_pattern_elements(tmp, elem_types, binds, conds) {
                    bad = true;
                }
            },
        );
        *bad_pattern |= bad;
        alternatives
    }

    /// `@FR-P-Multi` — the further patterns of one arm after its first, listed with `|` or `,`,
    /// for a subject whose arms are not discriminant tests (a tuple, a plain struct).  `opens`
    /// consumes the start of an alternative and answers whether one is there (the cursor is put
    /// back when not); `parse_one` parses it into its own bindings and conditions.  A name the
    /// first pattern — or an earlier alternative — bound is LINKED: this alternative's own
    /// variable is copied into that slot, after `@FR-P-Alt-Same` checks the types meet, so the
    /// one body reads whichever alternative matched.  A name only some alternatives bind is
    /// `τ?` and null in the others (`@FR-P-Alt-Diff`); `first_bindings` gains those nulls for
    /// the first pattern.  `first_names` is where the first pattern's names start in
    /// `pattern_binds_pending`.  Answers each alternative's `(bindings, conditions)`; an empty
    /// list is a single-pattern arm.
    pub(super) fn parse_pattern_alternatives(
        &mut self,
        first_names: usize,
        first_bindings: &mut Vec<Value>,
        opens: impl Fn(&mut Self) -> bool,
        mut parse_one: impl FnMut(&mut Self, &mut Vec<Value>, &mut Vec<Value>),
    ) -> Vec<(Vec<Value>, Vec<Value>)> {
        let mut alternatives: Vec<(Vec<Value>, Vec<Value>, HashSet<String>)> = Vec::new();
        // name → (shared slot, its type); the first pattern's names, then each new one.
        let mut shared: Vec<(String, u16, Type)> = self.pattern_binds_pending[first_names..]
            .iter()
            .map(|(name, _)| {
                let v = self.vars.var(name);
                (name.clone(), v, self.vars.tp(v).clone())
            })
            .collect();
        let first_set: HashSet<String> = shared.iter().map(|(n, _, _)| n.clone()).collect();
        while (self.lexer.peek_token("|") || self.lexer.peek_token(","))
            && !self.lexer.peek_token("=>")
        {
            let at = self.lexer.link();
            self.lexer.has_token("|");
            self.lexer.has_token(",");
            if !opens(self) {
                self.lexer.revert(at);
                break;
            }
            let start = self.pattern_binds_pending.len();
            let mut binds_i: Vec<Value> = Vec::new();
            let mut conds_i: Vec<Value> = Vec::new();
            parse_one(self, &mut binds_i, &mut conds_i);
            let added: Vec<(String, Option<u16>)> =
                self.pattern_binds_pending.drain(start..).collect();
            let mut names_i: HashSet<String> = HashSet::new();
            for (name, before) in added {
                let own = self.vars.var(&name);
                names_i.insert(name.clone());
                if let Some((_, slot, slot_tp)) = shared.iter().find(|(n, _, _)| *n == name) {
                    // Linked the way the enum arm links (`join_later_sub_pattern_captures`):
                    // this pattern's own variable is copied into the first one's slot.
                    let (slot, slot_tp) = (*slot, slot_tp.clone());
                    let own_tp = self.vars.tp(own).clone();
                    self.shared_slot_accepts(&name, &own_tp, &slot_tp);
                    // The copy reads this pattern's variable: it is not a never-read binding.
                    self.var_usages(own, true);
                    binds_i.push(v_set(slot, Value::Var(own)));
                    self.name_capture(&name, slot);
                } else {
                    // A name no earlier pattern bound: its own variable is the shared slot,
                    // and the arm's end restores what the name meant before the arm.
                    shared.push((name.clone(), own, self.vars.tp(own).clone()));
                    self.pattern_binds_pending.push((name, before));
                }
            }
            alternatives.push((binds_i, conds_i, names_i));
        }
        if alternatives.is_empty() {
            return Vec::new();
        }
        // `@FR-P-Alt-Diff`: a name some alternative does not bind is `τ?`, null there.
        for (name, slot, tp) in &shared {
            let everywhere = first_set.contains(name)
                && alternatives
                    .iter()
                    .all(|(_, _, names)| names.contains(name));
            if everywhere {
                continue;
            }
            let opt = Self::capture_optional(tp);
            self.vars.set_type(*slot, opt.clone());
            if !first_set.contains(name) {
                first_bindings.push(v_set(*slot, self.absent_capture(&opt)));
            }
            for (binds_i, _, names) in &mut alternatives {
                if !names.contains(name) {
                    binds_i.push(v_set(*slot, self.absent_capture(&opt)));
                }
            }
        }
        alternatives
            .into_iter()
            .map(|(binds, conds, _)| (binds, conds))
            .collect()
    }

    /// Parse a `match` expression whose subject is a `Type::Tuple`.
    ///
    /// Arm syntax: `_ => expr` (wildcard) or `(pat0, pat1, ...) => expr` (element patterns).
    /// Element patterns: `_` (wildcard), `identifier` (binding), or a literal value.
    /// Arms are separated by `,` or `;` (optional after the last arm).
    #[expect(
        clippy::too_many_lines,
        reason = "one condition chain over tuple arms: each element pattern (wildcard, binding, literal) and the arm body extend it"
    )]
    pub(super) fn parse_tuple_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
    ) -> Type {
        let Type::Tuple(elem_types) = subject_type.base() else {
            unreachable!("parse_tuple_match called with non-tuple subject")
        };
        let elem_types = elem_types.clone();

        // Store the tuple in a temp var so elements can be read multiple times.
        let tmp = self.create_unique("match_tuple", subject_type);
        self.vars.defined(tmp);

        self.lexer.token("{");

        let mut arms: Vec<PatternArm> = Vec::new();
        // loft#1682 — a `PatternArm` carries no type; the join reconverts by this list.
        let mut arm_types: Vec<Type> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;
        let saved_null_arms = std::mem::replace(&mut self.null_literal_arms, 0);

        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            // Every iteration must consume at least one token.  A pattern the element
            // loop below cannot make sense of leaves the cursor parked mid-arm, and
            // `expect_match_arm_arrow`'s `recover_to` cannot rescue it: that helper
            // resynchronises, and returns WITHOUT consuming when the cursor already
            // sits on one of its stop tokens or on an unmatched closer.  The arm then
            // re-parses the same token forever — silently, because first-pass
            // diagnostics are suppressed (loft#832).  Compared against `arm_start` at
            // the bottom of the loop.
            let arm_start = self.lexer.at();

            let mut is_wildcard = false;
            // A pattern that was REFUSED binds nothing and tests nothing, which reads
            // exactly like `(_, _, _)` at the classification below — and a wildcard arm
            // ends the arm loop, so a rejected first arm would swallow every arm after
            // it and report a missing `}` instead of the refusal (loft#832).
            let mut bad_pattern = false;
            let mut bindings: Vec<Value> = Vec::new();
            let mut elem_conds: Vec<Value> = Vec::new();
            // `@FR-P-Multi` over a tuple subject: the further `|` / `,` tuple patterns of this
            // arm, each its own bindings and conditions (`parse_tuple_alternatives`).
            let mut alternatives: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();

            if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_wildcard = true;
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected '_' or a tuple pattern '(...)' in tuple match"
                    );
                }
            } else if self.lexer.has_token("(") {
                let first_names = self.pattern_binds_pending.len();
                if self.parse_tuple_pattern_elements(
                    tmp,
                    &elem_types,
                    &mut bindings,
                    &mut elem_conds,
                ) {
                    bad_pattern = true;
                }
                alternatives = self.parse_tuple_alternatives(
                    tmp,
                    &elem_types,
                    first_names,
                    &mut bindings,
                    &mut bad_pattern,
                );
                // All element positions were wildcards/bindings with no literal conditions.
                // The arm is effectively unconditional (wildcard) when there are no bindings
                // either; if there are bindings it acts like a wildcard-with-capture.
                if elem_conds.is_empty()
                    && bindings.is_empty()
                    && !bad_pattern
                    && alternatives.is_empty()
                {
                    is_wildcard = true;
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "expected '_' or a tuple pattern '(...)' in tuple match"
                );
            }

            // Optional guard clause — `tuple-arm ::= tuple-pattern [ guard ] '=>' expression`
            // (TUPLES.md § Grammar).  Element bindings are in scope for it, and
            // `chain_pattern_arms` assigns them before the guard runs.
            let guard_opt = if self.lexer.has_token("if") {
                let at = *self.lexer.pos();
                let mut g = Value::Null;
                let gt = self.expression(&mut g);
                self.record_guard(&g, at);
                if !self.first_pass && gt != Type::Boolean {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "guard must be boolean, got {}",
                        gt.source_name(&self.data)
                    );
                }
                Some(g)
            } else {
                None
            };

            if is_wildcard && guard_opt.is_none() {
                has_wildcard = true;
            }

            self.expect_match_arm_arrow();
            let mut arm_body = Value::Null;
            let arm_expected = self.match_arm_expected(&result_type);
            let mut arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_body);
            // The alternatives run the BODY under their own bindings, so they take it before
            // the first pattern's bindings fold into it below.
            let mut bare_body = if alternatives.is_empty() {
                Value::Null
            } else {
                arm_body.clone()
            };
            let bare_type = arm_type.clone();

            // Combine element conditions with AND (short-circuit: if a { b } else { false })
            let cond: Option<Value> = if elem_conds.is_empty() {
                None
            } else {
                let mut combined = elem_conds.remove(0);
                for c in elem_conds {
                    combined = v_if(combined, c, Value::Boolean(false));
                }
                Some(combined)
            };

            // Without a guard the bindings fold into the body exactly as before; with one they
            // stay separate so `chain_pattern_arms` can run them ahead of the guard.
            let (mut arm_body, arm_bindings) = if guard_opt.is_some() {
                (arm_body, bindings)
            } else if bindings.is_empty() {
                (arm_body, Vec::new())
            } else {
                bindings.push(arm_body);
                (
                    v_block(bindings, arm_type.clone(), "tuple_binding"),
                    Vec::new(),
                )
            };

            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_body, &arm_type);
            self.match_void_arm |= matches!(arm_type.base(), Type::Void);
            if Self::arm_settles_match(&result_type, &arm_type) {
                result_type = arm_type.clone();
            } else if let Some(joined) =
                self.join_null_tuple_arm(&result_type, &mut arm_body, &mut arm_type)
            {
                for (prev, prev_tp) in arms.iter_mut().zip(arm_types.iter_mut()) {
                    if self.arm_takes_join(prev_tp, &joined) {
                        let prev_orig = prev_tp.clone();
                        self.convert_arm_tail(&mut prev.code, &prev_orig, &joined);
                        *prev_tp = joined.clone();
                    }
                }
                result_type = joined;
            }
            // Each alternative runs a CLONE of the arm body under its own bindings, which
            // assign the same shared slots the first pattern's do — the hand-expanded form.
            // A tail the join above converted is converted in the clone the same way.
            if !alternatives.is_empty() && bare_type != arm_type {
                self.convert_arm_tail(&mut bare_body, &bare_type, &arm_type);
            }
            let alternative_arms: Vec<PatternArm> = alternatives
                .into_iter()
                .map(|(mut binds_i, conds_i)| {
                    let cond_i = conds_i
                        .into_iter()
                        .reduce(|a, b| v_if(a, b, Value::Boolean(false)));
                    let (code_i, bindings_i) = if guard_opt.is_some() || binds_i.is_empty() {
                        (bare_body.clone(), binds_i)
                    } else {
                        binds_i.push(bare_body.clone());
                        (
                            v_block(binds_i, arm_type.clone(), "tuple_binding"),
                            Vec::new(),
                        )
                    };
                    PatternArm {
                        cond: cond_i,
                        guard: guard_opt.clone(),
                        bindings: bindings_i,
                        code: code_i,
                    }
                })
                .collect();
            arm_types.push(arm_type.clone());
            arms.push(PatternArm {
                cond,
                guard: guard_opt,
                bindings: arm_bindings,
                code: arm_body,
            });
            for alt in alternative_arms {
                arm_types.push(arm_type.clone());
                arms.push(alt);
            }

            if has_wildcard {
                self.lexer.has_token(",");
                self.lexer.has_token(";");
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(",");
                self.lexer.has_token(";");
            } else {
                // optional arm separator
                self.lexer.has_token(",");
                self.lexer.has_token(";");
            }
            // Backstop for the invariant declared at `arm_start`: a whole arm parsed
            // without consuming a token means no later iteration can consume one
            // either, so stop instead of spinning.  The shapes known to reach here
            // are handled above with their own diagnostics; this catches the ones
            // nobody has written a probe for yet.  `token("}")` below reports the
            // arm loop's failure to reach a close brace, as the vector arm loop's
            // own no-progress `break` does.
            if self.lexer.at() == arm_start {
                break;
            }
        }
        self.lexer.token("}");

        // Build if-else chain (last arm is fallback / wildcard)
        let has_null_arm = std::mem::replace(&mut self.null_literal_arms, saved_null_arms) > 0;
        self.type_null_arms(arms.iter_mut().map(|a| &mut a.code), &result_type);
        let fallback = if has_wildcard {
            arms.pop().unwrap().code
        } else {
            self.null_value(&result_type)
        };
        let chain = chain_pattern_arms(arms, fallback, &result_type);
        // The tuple and vector chains never widened for a `null` arm, scalars included.
        if has_null_arm && crate::keys::pln25_dn1_enabled() && Self::null_arm_widens(&result_type) {
            result_type = Type::optional(result_type);
        }

        *code = v_block(
            vec![v_set(tmp, subject), chain],
            result_type.clone(),
            "tuple_match",
        );
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// Build a boolean condition for a single scalar pattern value.
    pub(super) fn build_scalar_cond(
        &mut self,
        cond: &mut Value,
        v: u16,
        subject_type: &Type,
        pat: Value,
    ) {
        // Reuse the same logic as build_scalar_chain for special block patterns.
        if let Value::Block(ref bl) = pat
            && bl.result == Type::Boolean
            && (bl.name == "range_pattern" || bl.name == "null_pattern" || bl.name == "or_pattern")
        {
            *cond = bl.operators[0].clone();
            return;
        }
        self.call_op(
            cond,
            "==",
            &[Value::Var(v), pat],
            &[subject_type.clone(), subject_type.clone()],
        );
    }

    /// Build the if-chain for a scalar match from collected arms.
    pub(super) fn build_scalar_chain(
        &mut self,
        v: u16,
        subject_type: &Type,
        has_wildcard: bool,
        result_type: &Type,
        mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)>,
    ) -> Value {
        // A bare `null` arm value (`false => null`) parses to `Value::Null`, which
        // lowers to NO push (Type::Void).  In a value-producing match the if-chain
        // join then reads an unwritten, value-sized slot — interp stack underflow
        // ("No elements left on the stack"), native a lost value.  Convert each
        // bare-null arm to the result type's typed null sentinel, the same
        // transform `parse_if` applies to a null branch and the
        // fallback gets just below.  `null_value` is a no-op (returns
        // `Value::Null`) for Void/Unknown result types, so a statement-style
        // match is untouched.  Compute the typed null once (it's the same for
        // every arm — `result_type` is fixed), releasing the `&mut self` borrow
        // before mutating `arms`.
        //
        // loft#936 — `null_value` and not `null`, at every branch-MERGE slot in
        // this file.  `null` also supplies a VARIABLE's default-init, where a
        // collection has to be an allocated empty store, so its catch-all
        // answers a bare `Value::Null` for the entire collection family and
        // silently reintroduces the no-push this paragraph exists to prevent.
        // A BLOCK-bodied null arm (`0 => { null }`) needs the same repair as the bare one:
        // it parses to `Value::Block` whose last operator is `Value::Null`, so a predicate
        // that only matches a bare `Value::Null` walks straight past it and the arm keeps a
        // value that pushes nothing.  The enum/struct match path already tests both forms;
        // this one tested only the bare form, so `match n { 0 => { null }, _ => { [n] } }`
        // answered null for EVERY n while the bare-arm spelling of the same function was
        // right — a wrong value with no diagnostic, on both backends.
        let first_pass = self.first_pass;
        self.type_null_arms(
            arms.iter_mut()
                .filter(|a| !Self::is_placeholder_arm(first_pass, &a.2))
                .map(|a| &mut a.1),
            result_type,
        );
        for arm in arms
            .iter_mut()
            .filter(|a| Self::is_placeholder_arm(first_pass, &a.2))
        {
            mark_forward_call(&mut arm.1);
        }
        // The same repair one width up, for the fn-ref result type.  A non-capturing
        // lambda arm is a bare `Value::Int(d_nr)` — eight bytes — where the join reads
        // the full twenty-byte fn-ref, so the arms of one choice leave different depths
        // and the read straddles.  The `if` spelling of the identical choice is correct
        // because its arm is a BLOCK typed `function(…)` whose result path completes the
        // pair; this chain puts the arm body in raw, so the widening happens here instead
        // (loft#1469).  Both the bare and the `{ … ; <lambda> }` block spelling, and the
        // wildcard arm too — it is still in `arms` at this point and becomes the fallback
        // just below.
        for arm in &mut arms {
            crate::parser::widen_bare_fn_ref(&mut arm.1, result_type);
        }
        let fallback = if has_wildcard {
            let (_, arm_code, _, _) = arms.pop().unwrap();
            arm_code
        } else {
            self.null_value(result_type)
        };

        let mut chain = fallback;
        for (pattern_val, arm_code, _, guard_opt) in arms.into_iter().rev() {
            if let Some(lit) = pattern_val {
                // range/null/or patterns stored as Block with Boolean result.
                if let Value::Block(ref bl) = lit
                    && bl.result == Type::Boolean
                    && (bl.name == "range_pattern"
                        || bl.name == "null_pattern"
                        || bl.name == "or_pattern")
                {
                    let range_cond = bl.operators[0].clone();
                    chain = match guard_opt {
                        Some(guard) => {
                            guarded_arm(range_cond, Vec::new(), guard, arm_code, chain, result_type)
                        }
                        None => v_if(range_cond, arm_code, chain),
                    };
                    continue;
                }
                let mut cond = Value::Null;
                let cond_tp = self.call_op(
                    &mut cond,
                    "==",
                    &[Value::Var(v), lit],
                    &[subject_type.clone(), subject_type.clone()],
                );
                if cond_tp == Type::Null {
                    chain = arm_code;
                } else {
                    chain = match guard_opt {
                        Some(guard) => {
                            guarded_arm(cond, Vec::new(), guard, arm_code, chain, result_type)
                        }
                        None => v_if(cond, arm_code, chain),
                    };
                }
            } else {
                // Wildcard or guarded wildcard (no pattern).
                chain = match guard_opt {
                    Some(guard) => v_if(guard, arm_code, chain),
                    None => arm_code,
                };
            }
        }
        chain
    }
}
