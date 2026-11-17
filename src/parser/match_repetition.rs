// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Slice-pattern repetition `( [name:] V )*` with its separator, and alternation — a single
//! element `(A | B)`, a sequence `(A B | C)`, an optional group — with the captures each binds.
//! A part of the match subject split out of `control.rs`; the others are listed there.

use super::control::{AltSpan, SepSpec, match_arm_types_unify};
use super::{
    DefType, I32, Level, LexItem, Parser, Type, Value, diagnostic_format, v_block, v_if, v_loop,
    v_set,
};
use crate::data::Deps;

impl Parser {
    /// @PLN35 Phase 6 (L3.4, P-Rep) — a repetition `[ head…, ( [name:] V )*[(Sep)] [tail…]
    /// [, ..rest] ]` / `…+`.  The body `V` is one variant of the struct-enum element type; the
    /// repetition matches the MAXIMAL run of `V` starting at `head_len` (the count of fixed head
    /// elements already parsed).  A runtime run-loop counts that run into `end`; `name` (if given)
    /// collects the run `v[head_len .. end]` as a FRESH `vector<ElemType>`.  Fixed `tail`
    /// elements after the group are matched from the END when nothing follows them, so the run
    /// must reach exactly `len - tail_len`; with a `..rest` after them, or in a cursor match, they
    /// follow the run (`v[end + j]`) and the rest picks up `v[end + tail_len .. len]`.  `*` =
    /// zero-or-more; `+` ⇒ `end > head_len` (≥1 body element).  The run-loop lives INSIDE the arm
    /// condition (before the bindings materialise), yielding the match boolean.  Assumes the
    /// lexer is at the `(`; consumes through `]`.
    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::too_many_lines,
        reason = "one repetition: the head, the run loop inside the arm condition, the separator, the tail read from the end or after the run, and the rest share the length arithmetic"
    )]
    pub(super) fn parse_slice_repetition(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        head_len: i32,
        borrow_src: u16,
        cond: &mut Option<Value>,
        bindings: &mut Vec<Value>,
        group_cap: Option<String>,
    ) {
        self.lexer.token("(");
        // Optional `name:` capture prefix.
        let mut cap_name: Option<String> = None;
        if let Some(named) = self.lexer.peek_named_arg() {
            cap_name = Some(named);
            self.lexer.has_identifier();
            self.lexer.token(":");
        }
        // @C161 — a capture of the repetition GROUP, `g:( V )*`, is what `(P-Rep)` says a run
        // yields: the elements it collected, which is what `( g: V )*` binds.  Written both ways
        // (`g:( x: V )*`), both names take the run.
        let cap_names: Vec<String> = cap_name.into_iter().chain(group_cap).collect();
        // The body variant.
        let vname = self.lexer.has_identifier().unwrap_or_default();
        let real = if self.lexer.has_token("::") {
            self.lexer.has_identifier().unwrap_or_else(|| vname.clone())
        } else {
            vname.clone()
        };
        let mut vdef = self.data.variant_of(e_nr, &real);
        if vdef == u32::MAX {
            vdef = self.data.def_nr(&real);
        }
        let valid = vdef != u32::MAX
            && self.data.def_type(vdef) == DefType::EnumValue
            && self.data.def(vdef).parent() == e_nr;
        if !valid && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "'{}' is not a variant of {}",
                real,
                self.data.def(e_nr).name()
            );
        }
        let disc = if valid {
            self.variant_disc(e_nr, true, vdef, &real)
        } else {
            0
        };
        // @PLN35 slice 2 — `( V { field, … } )*` captures a per-iteration FIELD PROJECTION: each
        // named field collects into its own fresh `vector<field_type>` (vs a `name:` prefix, which
        // collects whole elements), whatever its type (`@FR-P-Rep-Ty`).  The run tag-tests `V`, so
        // every element carries the field.  A name that is not a field of `V` is rejected.
        let mut field_caps: Vec<(String, usize, Type)> = Vec::new();
        if self.lexer.has_token("{") {
            if valid {
                while let Some(fname) = self.lexer.has_identifier() {
                    let found = {
                        let vd = self.data.def(vdef);
                        vd.attributes[1..]
                            .iter()
                            .enumerate()
                            .find(|(_, a)| a.name == fname)
                            .map(|(i, a)| (i + 1, a.typedef.clone()))
                    };
                    match found {
                        Some((attr_idx, ftype)) => {
                            field_caps.push((fname, attr_idx, ftype));
                        }
                        None if !self.first_pass => {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "`{fname}` is not a field of {real}"
                            );
                        }
                        None => {}
                    }
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.token("}");
            } else {
                // Invalid variant already diagnosed above; skip the body to recover.
                let mut depth = 1i32;
                while depth > 0 {
                    if self.lexer.has_token("{") {
                        depth += 1;
                    } else if self.lexer.has_token("}") {
                        depth -= 1;
                    } else if matches!(self.lexer.peek().has, LexItem::None) {
                        break;
                    } else {
                        self.lexer.cont();
                    }
                }
            }
        }
        self.lexer.token(")");
        let plus = self.lexer.has_token("+");
        if !plus {
            self.lexer.token("*");
        }
        // @PLN35 Phase 6.2 — an optional separator group `(Sep)` after the `*`/`+`: the run
        // becomes `V (Sep V)*`.  `Sep` is ONE variant, CONSUMED between elements but never
        // captured (`name` collects only the V's — a stride-2 read skips the separators).
        let sep_disc = self.parse_repetition_separator(e_nr);
        // After the group: a comma-separated tail of fixed elements — literals (`")"`),
        // bare-name binds (`x`), and variant sub-patterns (`V { f }` / bare `V`) — then
        // optionally `, ..[name]`.
        //
        // Count the tail elements FIRST (a robust link/revert look-ahead), and whether a rest
        // follows them, so each is matched at a position known while it parses.  Where the tail
        // is read from depends on what comes after it (`@FR-P-Seq`, `@FR-P-Whole`): with nothing
        // after it the arm consumes the whole input, so the tail sits at the END,
        // `-(tail_len - j)`; with a `..rest` after it, or in a cursor match (which consumes a
        // prefix), the tail follows the RUN, at `end + j`, and whatever is left over is the
        // rest's or stays unconsumed.
        let mut rest_follows = false;
        let tail_len = {
            let save = self.lexer.link();
            let mut n = 0i32;
            while self.lexer.has_token(",") {
                if self.lexer.peek_token("..") {
                    rest_follows = true;
                    break;
                }
                // Skip one element up to the next top-level `,` or `]` (tracking bracket depth so
                // a variant's `{ f, g }` counts as one element).
                let mut depth = 0i32;
                loop {
                    match &self.lexer.peek().has.clone() {
                        LexItem::None => break,
                        LexItem::Token(t) if *t == "{" || *t == "[" || *t == "(" => {
                            depth += 1;
                            self.lexer.cont();
                        }
                        LexItem::Token(t) if *t == "}" || *t == "]" || *t == ")" => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            self.lexer.cont();
                        }
                        LexItem::Token(t) if *t == "," && depth == 0 => break,
                        _ => self.lexer.cont(),
                    }
                }
                n += 1;
            }
            self.lexer.revert(save);
            n
        };
        // `end` = the cursor after the last matched V; a forward tail reads relative to it.
        let end_var = self.create_unique("rep_end", &I32);
        self.vars.defined(end_var);
        let tail_forward = rest_follows || self.match_cursor.is_some();
        // Parse the tail with known positions.  A literal / variant sub-pattern contributes a
        // CONDITION (AND'd after the run bool, inside the run's own block, where `end` is set);
        // a bare-name bind and a variant's field binds are BINDINGS (run once the arm commits).
        let mut tail_conds: Vec<Value> = Vec::new();
        let mut has_rest = false;
        let mut rest_name: Option<String> = None;
        let mut j = 0i32;
        while self.lexer.has_token(",") {
            if self.lexer.has_token("..") {
                has_rest = true;
                rest_name = self.lexer.has_identifier();
                break;
            }
            let pos = if tail_forward {
                self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(j),
                    I32.clone(),
                    I32.clone(),
                )
            } else {
                Value::Int(-(tail_len - j))
            };
            if self.peek_is_slice_literal() {
                let mut lit = Value::Null;
                let lit_tp = self.expression(&mut lit);
                match self.build_literal_match(v, elm_size, elm_tp, &pos, &lit, &lit_tp) {
                    Some(tc) => tail_conds.push(tc),
                    None if !self.first_pass => self.slice_literal_mismatch(elm_tp, &lit_tp),
                    None => {}
                }
            } else if matches!(self.pattern_variant_enum(elm_tp), Some((te, true))
                if matches!(&self.lexer.peek().has, LexItem::Identifier(id)
                    if self.data.variant_of(te, id) != u32::MAX))
            {
                // A variant sub-pattern `V { f }` / bare `V`, matched at `pos`.  The DIRECT read
                // (as the head sub-pattern path uses) drives both the tag-test and the field binds.
                let elem_read = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
                let mut te_binds: Vec<Value> = Vec::new();
                let mut te_conds: Vec<Value> = Vec::new();
                let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                if let Some(tag) = self.parse_field_sub_pattern(
                    elem_read,
                    elm_tp,
                    &mut te_binds,
                    &mut te_conds,
                    &mut aliases,
                ) {
                    tail_conds.push(tag);
                }
                tail_conds.append(&mut te_conds);
                // The names a sub-pattern bound belong to this arm; its end restores them.
                self.pattern_binds_pending.append(&mut aliases);
                bindings.append(&mut te_binds);
            } else if let Some(name) = self.lexer.has_identifier() {
                let bind_var = self.pattern_binding(&name, elm_tp);
                if bind_var != u16::MAX {
                    self.vars.defined(bind_var);
                    let elem_read = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
                    bindings.push(v_set(bind_var, elem_read));
                    self.mark_slice_element_view(bind_var, elm_tp, borrow_src);
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "unexpected element after a repetition `( … )*` — expected a literal, a bind name, a variant sub-pattern, or `..rest`"
                    );
                }
                // Recover to `]` so `token("]")` below succeeds and this is the primary error.
                while !self.lexer.peek_token("]") && !matches!(self.lexer.peek().has, LexItem::None)
                {
                    self.lexer.cont();
                }
                break;
            }
            j += 1;
        }
        self.lexer.token("]");

        // @PLN35 PC — a CURSOR-mode repetition PREFIX-consumes: the run begins at the
        // ABSOLUTE index `pos + head_len` and, on a match, advances `cursor.pos` to the
        // run end — matching the MAXIMAL V run and LEAVING any following non-V tokens
        // (a plain vector must instead consume through to its end).  `base` is that
        // absolute start: plain `head_len` in vector mode (so the emitted IR stays
        // byte-identical there), a `pos + head_len` temp in cursor mode.  A fixed tail
        // follows the run (`tail_forward`), and the cursor advances past it too.
        let cursor_ctx = self.match_cursor;
        let (base_val, base_setup) = if let Some((_, _, _, pos_var)) = cursor_ctx {
            let b = self.create_unique("rep_base", &I32);
            self.vars.defined(b);
            let add = self.conv_op(
                "+",
                Value::Var(pos_var),
                Value::Int(head_len),
                I32.clone(),
                I32.clone(),
            );
            (Value::Var(b), Some(v_set(b, add)))
        } else {
            (Value::Int(head_len), None)
        };

        let mut run_ops: Vec<Value> = Vec::new();
        if let Some(bs) = base_setup {
            run_ops.push(bs);
        }
        run_ops.push(v_set(end_var, base_val.clone()));
        if let Some(sep) = &sep_disc {
            // Separated `V (Sep V)*`: match the first V, then loop `(Sep V)` pairs.
            // First V: `if head_len < len && tag(v[head_len]) == disc { end = head_len + 1 }`.
            let first_v = {
                let lc = self.cursor_len(v);
                let non_empty = self.conv_op("<", base_val.clone(), lc, I32.clone(), I32.clone());
                let elem = self.read_slice_elem(v, elm_size, elm_tp, base_val.clone());
                let tag = self.elem_tag_int(elem);
                let is_v = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
                let one_more = if cursor_ctx.is_some() {
                    self.conv_op(
                        "+",
                        base_val.clone(),
                        Value::Int(1),
                        I32.clone(),
                        I32.clone(),
                    )
                } else {
                    Value::Int(head_len + 1)
                };
                v_if(
                    non_empty,
                    v_if(is_v, v_set(end_var, one_more), Value::Null),
                    Value::Null,
                )
            };
            // `(Sep V)*`: `loop { if len <= end+1 break; if tag(v[end]) != sep break;
            //             if tag(v[end+1]) != disc break; end += 2 }`.
            let need_pair_brk = {
                let lc = self.cursor_len(v);
                let next = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(1),
                    I32.clone(),
                    I32.clone(),
                );
                let short = self.conv_op("<=", lc, next, I32.clone(), I32.clone());
                v_if(short, Value::Break(0), Value::Null)
            };
            let sep_brk = {
                let is_sep = self.sep_match_cond(sep, v, elm_size, elm_tp, Value::Var(end_var));
                v_if(is_sep, Value::Null, Value::Break(0))
            };
            let v_brk = {
                let next = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(1),
                    I32.clone(),
                    I32.clone(),
                );
                let elem = self.read_slice_elem(v, elm_size, elm_tp, next);
                let tag = self.elem_tag_int(elem);
                let is_v = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
                v_if(is_v, Value::Null, Value::Break(0))
            };
            let bump2 = {
                let b = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(2),
                    I32.clone(),
                    I32.clone(),
                );
                v_set(end_var, b)
            };
            let sep_loop = v_loop(vec![need_pair_brk, sep_brk, v_brk, bump2], "rep sep run");
            run_ops.push(first_v);
            run_ops.push(sep_loop);
        } else {
            // Contiguous `V*`: `loop { if len <= end break; if tag(v[end]) != disc break; end += 1 }`.
            let len_brk = {
                let lc = self.cursor_len(v);
                let at_end = self.conv_op("<=", lc, Value::Var(end_var), I32.clone(), I32.clone());
                v_if(at_end, Value::Break(0), Value::Null)
            };
            let tag_brk = {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, Value::Var(end_var));
                let tag = self.elem_tag_int(elem);
                let is_v = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
                v_if(is_v, Value::Null, Value::Break(0))
            };
            let step = {
                let bump = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(1),
                    I32.clone(),
                    I32.clone(),
                );
                v_set(end_var, bump)
            };
            run_ops.push(v_loop(vec![len_brk, tag_brk, step], "rep run"));
        }

        // Match boolean (after the loop): no-rest requires the run to reach exactly the start of
        // the fixed tail (`end == len - tail_len`); a rest just needs room for the head
        // (`head_len <= len`).  `+` additionally requires a non-empty run (`end > head_len`).
        let base = if tail_forward && tail_len > 0 {
            // The tail follows the run: it needs its elements after `end` (`end + t <= len`).
            let lc = self.cursor_len(v);
            let need = self.conv_op(
                "+",
                Value::Var(end_var),
                Value::Int(tail_len),
                I32.clone(),
                I32.clone(),
            );
            self.conv_op("<=", need, lc, I32.clone(), I32.clone())
        } else if has_rest || cursor_ctx.is_some() {
            // A cursor PREFIX-consumes: the run took the maximal V prefix and any
            // non-V tail simply stays unconsumed, so the arm matches whenever the head
            // fits (`base <= len`).  A vector `..rest` matches on the same "room for the
            // head" test.  Only a vector WITHOUT a rest must reach the fixed tail
            // exactly (`end == len - tail_len`, the whole-consume boundary).
            let lc = self.cursor_len(v);
            self.conv_op("<=", base_val.clone(), lc, I32.clone(), I32.clone())
        } else {
            let lc = self.cursor_len(v);
            let boundary = self.conv_op("-", lc, Value::Int(tail_len), I32.clone(), I32.clone());
            self.cl("OpEqInt", &[Value::Var(end_var), boundary])
        };
        let match_bool = if plus {
            // `end > base`, written `base < end` — `>` has no Int form (swapped `<`).
            let gt = self.conv_op(
                "<",
                base_val.clone(),
                Value::Var(end_var),
                I32.clone(),
                I32.clone(),
            );
            v_if(base, gt, Value::Boolean(false))
        } else {
            base
        };
        // AND the tail conditions (literal matches + variant tag-tests) AFTER the run boolean —
        // it already guarantees the tail's elements exist, and `&&` short-circuits so a
        // too-short slice never reads out of range.  They sit INSIDE the run's block, where
        // `end` is set, which a forward tail reads.  The tail BINDINGS (bare-name binds, variant
        // field binds) were pushed to `bindings` during the tail parse above.
        let mut match_bool = match_bool;
        for tc in tail_conds {
            match_bool = v_if(match_bool, tc, Value::Boolean(false));
        }
        run_ops.push(match_bool);
        let arm_cond = v_block(run_ops, Type::Boolean, "rep cond");
        *cond = match cond.take() {
            Some(existing) => Some(v_if(existing, arm_cond, Value::Boolean(false))),
            None => Some(arm_cond),
        };

        // Captures materialise AFTER the condition set `end` (bindings run once the arm commits).
        // `name` = the run `v[head_len .. end]`; a SEPARATED run puts the V's at every other index
        // (V Sep V Sep …), so it reads with a stride of 2 to skip separators (a bare run: 1).
        let cap_step = if sep_disc.is_some() { 2 } else { 1 };
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        for name in cap_names {
            let cap_var = self.pattern_binding(&name, &vec_tp);
            self.vars.defined(cap_var);
            if !self.first_pass && cap_var != u16::MAX {
                self.materialize_named_rest(
                    v,
                    elm_tp,
                    elm_size,
                    cap_var,
                    &vec_tp,
                    base_val.clone(),
                    Value::Var(end_var),
                    cap_step,
                    bindings,
                );
            }
        }
        // @PLN35 slice 2 — each `{ field }` projects the run's field into its own vector.
        for (fname, attr_idx, ftype) in field_caps {
            let fvec_tp = Type::Vector(Box::new(ftype.clone()), Deps::none());
            let proj_var = self.pattern_binding(&fname, &fvec_tp);
            self.vars.defined(proj_var);
            if !self.first_pass && proj_var != u16::MAX {
                self.materialize_field_projection(
                    v,
                    elm_size,
                    elm_tp,
                    vdef,
                    attr_idx,
                    &ftype,
                    proj_var,
                    &fvec_tp,
                    base_val.clone(),
                    Value::Var(end_var),
                    cap_step,
                    bindings,
                );
            }
        }
        // The first element after the run AND its fixed tail — where a rest begins, and where a
        // cursor stops without one.
        let after_tail = if tail_forward && tail_len > 0 {
            self.conv_op(
                "+",
                Value::Var(end_var),
                Value::Int(tail_len),
                I32.clone(),
                I32.clone(),
            )
        } else {
            Value::Var(end_var)
        };
        if let Some(name) = rest_name {
            let rest_var = self.pattern_binding(&name, &vec_tp);
            self.vars.defined(rest_var);
            if !self.first_pass && rest_var != u16::MAX {
                let hi_val = self.cursor_len(v);
                self.materialize_named_rest(
                    v,
                    elm_tp,
                    elm_size,
                    rest_var,
                    &vec_tp,
                    after_tail.clone(),
                    hi_val,
                    1,
                    bindings,
                );
            }
        }

        // @PLN35 PC — advance the cursor by the consumed prefix.  On a plain vector
        // there is no cursor and nothing to advance.  On a cursor the run consumed up
        // to `end`; a trailing `..rest` additionally consumed the remainder, so the
        // cursor lands on the source end.  Mirrors the fixed-arity advance in
        // `parse_vector_match` (including the PC5 `farthest` high-water update).
        if let Some((cursor_var, cursor_def, pos_field, pos_var)) = cursor_ctx {
            let _ = pos_var;
            let advance_to = if has_rest {
                let lc = self.cursor_len(v);
                let t = self.create_unique("rep_adv", &I32);
                self.vars.defined(t);
                bindings.push(v_set(t, lc));
                Value::Var(t)
            } else {
                after_tail
            };
            let adv = self.set_field(
                cursor_def,
                pos_field,
                0,
                Value::Var(cursor_var),
                advance_to.clone(),
            );
            bindings.push(adv);
            if let Some(ff) = self.match_cursor_farthest {
                let old_far = self.get_field(cursor_def, ff, Value::Var(cursor_var));
                let is_less = self.conv_op(
                    "<",
                    old_far.clone(),
                    advance_to.clone(),
                    I32.clone(),
                    I32.clone(),
                );
                let maxed = v_if(is_less, advance_to, old_far);
                let set_far = self.set_field(cursor_def, ff, 0, Value::Var(cursor_var), maxed);
                bindings.push(set_far);
            }
        }
    }

    /// @PLN35 Phase 6.2 — parse an OPTIONAL separator group after a repetition's `*` / `+`,
    /// consumed BETWEEN elements but never captured.  It is a variant TAG `(Comma)` OR a LEXEME
    /// literal `(",")` (a comma-separated token grammar → `(arg)*(",")`).  `None` = no `(` follows.
    pub(super) fn parse_repetition_separator(&mut self, e_nr: u32) -> Option<SepSpec> {
        if !self.lexer.peek_token("(") {
            return None;
        }
        self.lexer.token("(");
        if self.peek_is_slice_literal() {
            // Lexeme separator `(",")` — matched by `#lexeme`/scalar equality per element.
            let mut lit = Value::Null;
            let lit_tp = self.expression(&mut lit);
            self.lexer.token(")");
            return Some(SepSpec::Lexeme(lit, lit_tp));
        }
        let sname = self.lexer.has_identifier().unwrap_or_default();
        let real = if self.lexer.has_token("::") {
            self.lexer.has_identifier().unwrap_or_else(|| sname.clone())
        } else {
            sname.clone()
        };
        let mut sdef = self.data.variant_of(e_nr, &real);
        if sdef == u32::MAX {
            sdef = self.data.def_nr(&real);
        }
        let valid = sdef != u32::MAX
            && self.data.def_type(sdef) == DefType::EnumValue
            && self.data.def(sdef).parent() == e_nr;
        if !valid && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "separator '{}' is not a variant of {}",
                real,
                self.data.def(e_nr).name()
            );
        }
        let disc = if valid {
            self.variant_disc(e_nr, true, sdef, &real)
        } else {
            0
        };
        // A `{ … }` field body on the separator is deferred (like the repetition body): skip it.
        if self.lexer.has_token("{") {
            let mut depth = 1i32;
            while depth > 0 {
                if self.lexer.has_token("{") {
                    depth += 1;
                } else if self.lexer.has_token("}") {
                    depth -= 1;
                } else if matches!(self.lexer.peek().has, LexItem::None) {
                    break;
                } else {
                    self.lexer.cont();
                }
            }
        }
        self.lexer.token(")");
        Some(SepSpec::Variant(disc))
    }

    /// @PLN35 Phase 6.2 — the boolean "does `v[pos]` match the separator" for a repetition
    /// separator: a variant TAG test, or a `#lexeme`/scalar equality for a lexeme separator.
    pub(super) fn sep_match_cond(
        &mut self,
        sep: &SepSpec,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: Value,
    ) -> Value {
        match sep {
            SepSpec::Variant(disc) => {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, pos);
                let tag = self.elem_tag_int(elem);
                self.cl("OpEqInt", &[tag, Value::Int(*disc)])
            }
            SepSpec::Lexeme(lit, lit_tp) => self
                .build_literal_match(v, elm_size, elm_tp, &pos, lit, lit_tp)
                .unwrap_or(Value::Boolean(false)),
        }
    }

    /// @PLN35 Phase 4.3 — a WHOLE-SLICE multi-element alternation
    /// `[ (seq₁ | seq₂ | …) [, ..rest] ]`, where each branch is a SEQUENCE of variant
    /// sub-patterns of (possibly) different width.  Dispatches PREDICTIVELY on the leading
    /// tags: an ordered `if / else if` over PURE per-branch conditions
    /// (`len {==|>=} wᵢ && tag(v[0])==d₀ && tag(v[1])==d₁ && …`), the first match committing.
    /// Captures bind conditionally (option-promoted across branches).  Assumes the lexer is at
    /// the `(` and the alternation is the WHOLE slice content (head empty); builds the arm
    /// `cond` + `bindings` and consumes through `]`.  `..rest` from the runtime cursor is
    /// Phase 4.3 step 5 (deferred here with a diagnostic).
    #[allow(clippy::too_many_arguments)]
    #[expect(
        clippy::too_many_lines,
        reason = "one ordered if/else over the alternatives: each branch's pure condition and the captures promoted across branches are built together"
    )]
    pub(super) fn parse_multi_element_alternation(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        head_len: i32,
        borrow_src: u16,
        cond: &mut Option<Value>,
        bindings: &mut Vec<Value>,
        group_cap: Option<&str>,
    ) {
        self.lexer.token("(");
        // branches[i] = the sequence for branch i: [(disc, variant_def, fields)] per position.
        let mut branches: Vec<Vec<(i32, u32, Vec<(String, usize, Type)>)>> = Vec::new();
        let whole_tp = Type::Enum(e_nr, true, crate::data::Deps::none());
        loop {
            let mut branch: Vec<(i32, u32, Vec<(String, usize, Type)>)> = Vec::new();
            loop {
                // `@FR-P-Cap` — `x: V` captures the element a position matched, as in a
                // single-element alternation; `(x: A)?` binds `x : A?` (`@FR-P-Opt`).
                let capture = self.lexer.peek_named_arg();
                if capture.is_some() {
                    self.lexer.has_identifier();
                    self.lexer.token(":");
                }
                let Some(vid) = self.lexer.has_identifier() else {
                    break;
                };
                let vname = if self.lexer.has_token("::") {
                    self.lexer.has_identifier().unwrap_or_else(|| vid.clone())
                } else {
                    vid.clone()
                };
                let mut vdef = self.data.variant_of(e_nr, &vname);
                if vdef == u32::MAX {
                    vdef = self.data.def_nr(&vname);
                }
                let valid = vdef != u32::MAX
                    && self.data.def_type(vdef) == DefType::EnumValue
                    && self.data.def(vdef).parent() == e_nr;
                if !valid && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        vname,
                        self.data.def(e_nr).name()
                    );
                }
                let disc = if valid {
                    self.variant_disc(e_nr, true, vdef, &vname)
                } else {
                    0
                };
                let mut fields: Vec<(String, usize, Type)> = Vec::new();
                if let Some(cap) = capture {
                    fields.push((cap, usize::MAX, whole_tp.clone()));
                }
                if self.lexer.has_token("{") {
                    while let Some(fname) = self.lexer.has_identifier() {
                        let attr = if valid {
                            let vd = self.data.def(vdef);
                            vd.attributes[1..]
                                .iter()
                                .enumerate()
                                .find(|(_, a)| a.name == fname)
                                .map(|(i, a)| (i + 1, a.typedef.clone()))
                        } else {
                            None
                        };
                        match attr {
                            Some((idx, ty)) => fields.push((fname, idx, ty)),
                            None => {
                                if valid && !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "variant {} has no field '{}'",
                                        vname,
                                        fname
                                    );
                                }
                            }
                        }
                        if !self.lexer.has_token(",") {
                            break;
                        }
                    }
                    self.lexer.token("}");
                }
                if valid {
                    branch.push((disc, vdef, fields));
                }
                if self.lexer.peek_token("|") || self.lexer.peek_token(")") {
                    break;
                }
            }
            if !branch.is_empty() {
                branches.push(branch);
            }
            if !self.lexer.has_token("|") {
                break;
            }
        }
        self.lexer.token(")");
        if branches.is_empty() {
            return;
        }

        // @PLN35 Phase 5 (P-Opt) — a trailing `?` makes the group OPTIONAL: append an EMPTY
        // branch (width 0).  It always matches (`len >= 0`), so the arm never fails and, when the
        // real branches do not match, the cursor stays put and every capture reads null — exactly
        // `(a)?`'s "try a; else bind null, cursor unmoved" contract, expressed as `(a | ε)`.
        let optional = self.lexer.has_token("?");
        if optional {
            branches.push(Vec::new());
        }

        // Optional `, ..rest`, or — after an OPTIONAL group — fixed TAIL elements, then `]`.
        let mut rest_name: Option<String> = None;
        let mut tail_conds: Vec<Value> = Vec::new();
        let mut tail_len: i32 = 0;
        if self.lexer.has_token(",") {
            if self.lexer.has_token("..") {
                rest_name = self.lexer.has_identifier();
            } else if optional && self.match_cursor.is_none() {
                tail_len = self.parse_optional_group_tail(
                    e_nr,
                    v,
                    elm_size,
                    elm_tp,
                    borrow_src,
                    &mut tail_conds,
                    bindings,
                );
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a multi-element alternation `( … | … )` is the whole slice pattern — only \
                     `, ..rest` may follow it"
                );
                self.skip_rest_of_slice();
                return;
            }
        }
        self.lexer.token("]");
        let has_rest = rest_name.is_some();
        if head_len > 0 && !optional {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a multi-element alternation `( … | … )` is the whole slice pattern — it \
                     cannot follow fixed elements"
                );
            }
            return;
        }

        // `@FR-P-Opt` mid-slice (loft#2048) — after fixed head elements, or before fixed tail
        // ones, the group is GREEDY, as `(a)?` is: a branch is taken when its tags match at
        // the group's position, the first such in order, and the length must then fit what
        // follows — the tail, read from the end, or the rest.  (The whole-slice form below
        // includes the length in the branch test; with nothing after the group the two agree.)
        if head_len > 0 || tail_len > 0 {
            let mut branch_conds: Vec<Value> = Vec::new();
            for branch in &branches {
                let w = branch.len() as i32;
                let lc = self.cursor_len(v);
                let mut c =
                    self.conv_op("<=", Value::Int(head_len + w), lc, I32.clone(), I32.clone());
                for (j, (disc, _, _)) in branch.iter().enumerate() {
                    let elem =
                        self.read_slice_elem(v, elm_size, elm_tp, Value::Int(head_len + j as i32));
                    let tag = self.elem_tag_int(elem);
                    let t = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
                    c = v_if(c, t, Value::Boolean(false));
                }
                branch_conds.push(c);
            }
            // The taken branch's length requirement: exactly `head + w + tail`, or at least
            // `head + w` before a rest.
            let mut fits = Value::Boolean(false);
            for (i, branch) in branches.iter().enumerate().rev() {
                let need = head_len + branch.len() as i32 + tail_len;
                let lc = self.cursor_len(v);
                let ok = if has_rest {
                    self.conv_op("<=", Value::Int(need), lc, I32.clone(), I32.clone())
                } else {
                    self.conv_op("==", lc, Value::Int(need), I32.clone(), I32.clone())
                };
                fits = v_if(branch_conds[i].clone(), ok, fits);
            }
            for tc in tail_conds {
                fits = v_if(fits, tc, Value::Boolean(false));
            }
            *cond = match cond.take() {
                Some(existing) => Some(v_if(existing, fits, Value::Boolean(false))),
                None => Some(fits),
            };
            self.bind_alternation_captures(
                &branches,
                &branch_conds,
                head_len,
                v,
                elm_size,
                elm_tp,
                borrow_src,
                bindings,
            );
            if let Some(name) = group_cap {
                let span = AltSpan { head_len, optional };
                self.bind_alternation_group(
                    name,
                    &branches,
                    &branch_conds,
                    span,
                    v,
                    elm_size,
                    elm_tp,
                    bindings,
                );
            }
            if let Some(name) = rest_name {
                self.bind_alternation_rest(
                    &name,
                    &branches,
                    &branch_conds,
                    head_len,
                    v,
                    elm_size,
                    elm_tp,
                    bindings,
                );
            }
            return;
        }

        // Per-branch PURE predicate: length + leading-tag sequence.  With a rest the branch may
        // be a PREFIX (`wᵢ <= len`); without, it must be the whole slice (`len == wᵢ`).
        let mut branch_conds: Vec<Value> = Vec::new();
        for branch in &branches {
            let w = branch.len() as i32;
            let lc = self.cursor_len(v);
            let mut c = if has_rest {
                self.conv_op("<=", Value::Int(w), lc, I32.clone(), I32.clone())
            } else {
                self.conv_op("==", lc, Value::Int(w), I32.clone(), I32.clone())
            };
            for (j, (disc, _, _)) in branch.iter().enumerate() {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(j as i32));
                let tag = self.elem_tag_int(elem);
                let t = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
                c = v_if(c, t, Value::Boolean(false)); // c && t
            }
            branch_conds.push(c);
        }
        // arm cond = OR of the branch predicates, AND'd into any existing cond.
        let mut disjunction = branch_conds[0].clone();
        for bc in &branch_conds[1..] {
            disjunction = v_if(disjunction, Value::Boolean(true), bc.clone());
        }
        *cond = match cond.take() {
            Some(existing) => Some(v_if(existing, disjunction, Value::Boolean(false))),
            None => Some(disjunction),
        };

        self.bind_alternation_captures(
            &branches,
            &branch_conds,
            0,
            v,
            elm_size,
            elm_tp,
            borrow_src,
            bindings,
        );
        if let Some(name) = group_cap {
            let span = AltSpan {
                head_len: 0,
                optional,
            };
            self.bind_alternation_group(
                name,
                &branches,
                &branch_conds,
                span,
                v,
                elm_size,
                elm_tp,
                bindings,
            );
        }
        if let Some(name) = rest_name {
            self.bind_alternation_rest(
                &name,
                &branches,
                &branch_conds,
                0,
                v,
                elm_size,
                elm_tp,
                bindings,
            );
        }
    }

    /// The fixed TAIL after an optional group (`[head…, (G)?, t₁, …, tₙ]`, loft#2048): each element
    /// read from the END (`v[-(n - k)]`) — the length the group leaves is checked by the caller —
    /// as a bare name binding the element, a `_`, or a variant pattern (`V { f }`, `V | W`).
    /// Its conditions go to `tail_conds`; returns `n`.  The lexer stops at the `]`.
    #[allow(clippy::too_many_arguments)]
    fn parse_optional_group_tail(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        borrow_src: u16,
        tail_conds: &mut Vec<Value>,
        bindings: &mut Vec<Value>,
    ) -> i32 {
        let total = self.count_slice_tail_elements();
        for k in 0..total {
            let pos = Value::Int(-(total - k));
            if self.lexer.has_wildcard() {
                // a wildcard: the position only
            } else if self.lexer.peek_named_arg().is_none()
                && let LexItem::Identifier(id) = self.lexer.peek().has.clone()
                && id.chars().next().is_some_and(char::is_lowercase)
            {
                self.lexer.has_identifier();
                let bind_nr = self.pattern_binding(&id, elm_tp);
                self.vars.defined(bind_nr);
                let val = self.read_slice_elem(v, elm_size, elm_tp, pos);
                bindings.push(v_set(bind_nr, val));
                self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
            } else {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, pos);
                self.parse_slice_alternation_element(
                    e_nr,
                    &elem,
                    Some(borrow_src),
                    bindings,
                    tail_conds,
                    false,
                );
            }
            if k + 1 < total {
                self.lexer.token(",");
            }
        }
        total
    }

    /// The captures of a multi-element alternation or an optional group (`parse_multi_element_
    /// alternation`): the names of every branch, each read at its branch's position past the
    /// `head_len` fixed elements before the group, under that branch's selection — a name only
    /// some branches bind is `τ?` (`@FR-P-Alt-Diff`, `@FR-P-Opt-Ty`).
    #[allow(clippy::too_many_arguments)]
    fn bind_alternation_captures(
        &mut self,
        branches: &[Vec<(i32, u32, Vec<(String, usize, Type)>)>],
        branch_conds: &[Value],
        head_len: i32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        borrow_src: u16,
        bindings: &mut Vec<Value>,
    ) {
        // Captures: union the names; a name in every branch at a compatible type stays that
        // type, a name in only some promotes to `option<T>`.  Each capture reads at ITS
        // branch's position, guarded by that branch's predicate (untaken → null via else).
        let mut names: Vec<(String, Type)> = Vec::new();
        for branch in branches {
            for (_, _, fields) in branch {
                for (fname, _, ftype) in fields {
                    if let Some((_, seen)) = names.iter().find(|(n, _)| n == fname) {
                        if !self.first_pass && !match_arm_types_unify(seen, ftype) {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "alternation capture '{}' is {} in one branch but {} in another",
                                fname,
                                ftype.source_name(&self.data),
                                seen.source_name(&self.data)
                            );
                        }
                    } else {
                        names.push((fname.clone(), ftype.clone()));
                    }
                }
            }
        }
        for (fname, ftype) in &names {
            let present_in_all = branches.iter().all(|br| {
                br.iter()
                    .any(|(_, _, fields)| fields.iter().any(|(n, _, _)| n == fname))
            });
            let var_type = if present_in_all {
                ftype.clone()
            } else {
                Self::capture_optional(ftype)
            };
            let v_nr = self.create_unique(&format!("mv_{fname}"), &var_type);
            if v_nr == u16::MAX {
                continue;
            }
            self.vars.defined(v_nr);
            let mut acc = self.absent_capture(&var_type);
            for (bi, branch) in branches.iter().enumerate().rev() {
                if let Some((pos, vdef, attr)) =
                    branch.iter().enumerate().find_map(|(j, (_, vd, fields))| {
                        fields
                            .iter()
                            .find(|(n, _, _)| n == fname)
                            .map(|(_, a, _)| (j, *vd, *a))
                    })
                {
                    let elem = self.read_slice_elem(
                        v,
                        elm_size,
                        elm_tp,
                        Value::Int(head_len + pos as i32),
                    );
                    let read = if attr == usize::MAX {
                        elem
                    } else {
                        self.pattern_field_value(vdef, attr, elem)
                    };
                    acc = v_if(branch_conds[bi].clone(), read, acc);
                }
            }
            bindings.push(v_set(v_nr, acc));
            self.name_capture(fname, v_nr);
            // Heap capture = a borrowed view of the subject element (mirror the field-sub-pattern
            // path): skip its free, record the borrow dep — through an `option<T>` wrapper too.
            if !matches!(ftype.base(), Type::Text(_)) {
                self.vars.set_skip_free(v_nr);
            }
            let bound_tp = Self::element_view_of(&self.vars.tp(v_nr).clone(), borrow_src);
            self.vars.set_type(v_nr, bound_tp);
        }
    }

    /// @C161, `@FR-P-Cap` over a GROUP — `g:( … )?` / `g:( … | … )` binds the sub-slice the
    /// taken branch matched, `v[head_len .. head_len + width]`, as a FRESH vector (the store
    /// `(P-Rest)` gives a rest, independent of the subject).  An OPTIONAL group whose real
    /// branches all failed took its empty branch, and then `g` is null, as `(P-Opt)` makes every
    /// capture inside it: the binding is `vector<T>?`, its null the vector-header sentinel the
    /// parser writes for `x = null` on a nullable vector.
    #[allow(clippy::too_many_arguments)]
    fn bind_alternation_group(
        &mut self,
        name: &str,
        branches: &[Vec<(i32, u32, Vec<(String, usize, Type)>)>],
        branch_conds: &[Value],
        span: AltSpan,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        bindings: &mut Vec<Value>,
    ) {
        let end_var = self.create_unique("grp_end", &I32);
        self.vars.defined(end_var);
        // end = head + the width of the FIRST branch that holds (ordered choice, `(P-Alt)`).
        let mut end_acc = Value::Int(span.head_len);
        for (i, bc) in branch_conds.iter().enumerate().rev() {
            end_acc = v_if(
                bc.clone(),
                Value::Int(span.head_len + branches[i].len() as i32),
                end_acc,
            );
        }
        bindings.push(v_set(end_var, end_acc));
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        let bind_tp = if span.optional {
            Type::optional(vec_tp.clone())
        } else {
            vec_tp.clone()
        };
        let cap_var = self.pattern_binding(name, &bind_tp);
        self.vars.defined(cap_var);
        if self.first_pass || cap_var == u16::MAX {
            return;
        }
        self.materialize_named_rest(
            v,
            elm_tp,
            elm_size,
            cap_var,
            &vec_tp,
            Value::Int(span.head_len),
            Value::Var(end_var),
            1,
            bindings,
        );
        if span.optional {
            let built = self.vars.tp(cap_var).clone();
            if let Some(&db) = built.depend().first() {
                let absent = self.conv_op(
                    "==",
                    Value::Var(end_var),
                    Value::Int(span.head_len),
                    I32.clone(),
                    I32.clone(),
                );
                let null_header = self.cl(
                    "OpSetInt4",
                    &[Value::Var(db), Value::Int(0), Value::Int(-1)],
                );
                bindings.push(v_if(absent, null_header, Value::Null));
            }
            self.vars.set_type(cap_var, Type::optional(built));
        }
    }

    /// The `..rest` after a multi-element alternation or an optional group: `v[pos .. len]`,
    /// `pos` the end of WHICHEVER branch was taken (`head_len + wᵢ`).
    #[allow(clippy::too_many_arguments)]
    fn bind_alternation_rest(
        &mut self,
        name: &str,
        branches: &[Vec<(i32, u32, Vec<(String, usize, Type)>)>],
        branch_conds: &[Value],
        head_len: i32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        bindings: &mut Vec<Value>,
    ) {
        let pos_var = self.create_unique("alt_pos", &I32);
        self.vars.defined(pos_var);
        // pos = if b0 { w0 } else if b1 { w1 } … else 0 (0 unreachable — cond already gated).
        let mut pos_acc = Value::Int(0);
        for (i, bc) in branch_conds.iter().enumerate().rev() {
            pos_acc = v_if(
                bc.clone(),
                Value::Int(head_len + branches[i].len() as i32),
                pos_acc,
            );
        }
        bindings.push(v_set(pos_var, pos_acc));

        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        let rest_var = self.pattern_binding(name, &vec_tp);
        self.vars.defined(rest_var);
        if !self.first_pass && rest_var != u16::MAX {
            let hi_val = self.cursor_len(v);
            self.materialize_named_rest(
                v,
                elm_tp,
                elm_size,
                rest_var,
                &vec_tp,
                Value::Var(pos_var),
                hi_val,
                1,
                bindings,
            );
        }
    }

    /// @PLN35 Phase 4 (P-Alt) — parse a single-element alternation
    /// `( V1 { f… } | V2 { f… } | … )` in a slice element position and emit its
    /// tag-disjunction condition (into `elem_conds`) + shared-slot capture bindings
    /// (into `bindings`).  Each alternative is a variant sub-pattern of the element
    /// enum; the element matches if ANY branch's tag matches, and each capture is
    /// read from WHICHEVER variant matched, at THAT variant's own offset (a
    /// conditional-offset read `f = if tag==V1 { f@V1 } else if tag==V2 { f@V2 } …`).
    /// Enum tags are disjoint, so ordered choice reduces to a disjunction here.
    ///
    /// A capture bound by every branch takes the join of its types (`@FR-P-Alt-Same`); one
    /// bound by only some is `τ?`, null when another branch matched (`@FR-P-Alt-Diff`).  A
    /// branch may capture the whole element it matched, `a:V | b:W` (`@FR-G-Pat-Prec`).  A
    /// varying-width MULTI-element alternative is `parse_multi_element_alternation`.  `elem`
    /// is the element value at this position; it is cloned for each tag test and field read.
    ///
    /// `parens` is false for a TUPLE element (`(Circle { r } | Square { r }, k)`, @PLN186 step
    /// 4): the alternatives stand bare between the element separators, and each name joins the
    /// arm's pending names so the arm's end restores what it meant before.
    #[expect(
        clippy::too_many_lines,
        reason = "one tag disjunction: each alternative's condition and its conditional-offset captures are built from the same branch list"
    )]
    pub(super) fn parse_slice_alternation_element(
        &mut self,
        e_nr: u32,
        elem: &Value,
        borrow_src: Option<u16>,
        bindings: &mut Vec<Value>,
        elem_conds: &mut Vec<Value>,
        parens: bool,
    ) {
        if parens {
            self.lexer.token("(");
        }
        // (disc, variant_def_nr, fields: [(name, attr_idx, type)]) — an `attr_idx` of
        // `usize::MAX` is a `name:` capture of the WHOLE element (`@FR-P-Cap`).
        let mut alts: Vec<(i32, u32, Vec<(String, usize, Type)>)> = Vec::new();
        let elem_tp = Type::Enum(e_nr, true, crate::data::Deps::none());
        loop {
            // `(G-Pat-Prec)` — `a:V | b:W` is `(a:V) | (b:W)`: a branch may capture the element
            // it matched.  Read as a variant name, `a` then `:` was a parse error.
            let branch_capture = self.lexer.peek_named_arg();
            if branch_capture.is_some() {
                self.lexer.has_identifier();
                self.lexer.token(":");
            }
            let Some(vname) = self.lexer.has_identifier() else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expect a variant name in an alternation `( … | … )`"
                    );
                }
                break;
            };
            let mut vdef = self.data.variant_of(e_nr, &vname);
            if vdef == u32::MAX {
                vdef = self.data.def_nr(&vname);
            }
            let valid = vdef != u32::MAX
                && self.data.def_type(vdef) == DefType::EnumValue
                && self.data.def(vdef).parent() == e_nr;
            if !valid && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is not a variant of {}",
                    vname,
                    self.data.def(e_nr).name()
                );
            }
            // Only touch the definition table when the variant resolved — `def(u32::MAX)`
            // panics (Unknown definition).  An invalid branch still consumes its `{ … }`
            // so the loop makes progress, but contributes no disc / fields / alt entry.
            let disc = if valid {
                self.variant_disc(e_nr, true, vdef, &vname)
            } else {
                0
            };
            let mut fields: Vec<(String, usize, Type)> = Vec::new();
            if let Some(cap) = branch_capture {
                fields.push((cap, usize::MAX, elem_tp.clone()));
            }
            if self.lexer.has_token("{") {
                while let Some(fname) = self.lexer.has_identifier() {
                    let attr = if valid {
                        let vd = self.data.def(vdef);
                        vd.attributes[1..]
                            .iter()
                            .enumerate()
                            .find(|(_, a)| a.name == fname)
                            .map(|(i, a)| (i + 1, a.typedef.clone()))
                    } else {
                        None
                    };
                    // `f: name` binds the field under another name, and `f: _` does not bind it
                    // — the spellings a fixed slice element takes (`@FR-P-Cap`); a nested pattern
                    // after the `:` is the fixed element's form, refused here by name.
                    let mut bind = Some(fname.clone());
                    if self.lexer.has_token(":") {
                        if self.lexer.has_wildcard() {
                            bind = None;
                        } else if let LexItem::Identifier(id) = self.lexer.peek().has.clone()
                            && id.chars().next().is_some_and(char::is_lowercase)
                        {
                            self.lexer.has_identifier();
                            bind = Some(id);
                        } else {
                            if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "a field of an alternation branch binds a name (`{fname}: x`) \
                                     or nothing (`{fname}: _`); a nested pattern belongs in a \
                                     fixed element"
                                );
                            }
                            self.skip_rest_of_slice();
                            return;
                        }
                    }
                    match attr {
                        Some((idx, ty)) => {
                            if let Some(b) = bind {
                                fields.push((b, idx, ty));
                            }
                        }
                        None => {
                            if valid && !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "variant {} has no field '{}'",
                                    vname,
                                    fname
                                );
                            }
                        }
                    }
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.token("}");
            }
            if valid {
                alts.push((disc, vdef, fields));
            }
            if !self.lexer.has_token("|") {
                break;
            }
        }
        if parens {
            self.lexer.token(")");
        }
        if alts.is_empty() {
            return;
        }

        // Tag-disjunction: OR of each branch's tag test on the element.
        let mut cond = {
            let tag = self.elem_tag_int(elem.clone());
            self.cl("OpEqInt", &[tag, Value::Int(alts[0].0)])
        };
        for (disc, _, _) in &alts[1..] {
            let tag = self.elem_tag_int(elem.clone());
            let t = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
            cond = v_if(cond, Value::Boolean(true), t);
        }
        elem_conds.push(cond);

        // Capture unification (P-Alt-Same ⊔ / P-Alt-Diff N-Join).  Union the capture
        // names across all branches; a name in EVERY branch (compatible type) binds at
        // that type, a name in only SOME becomes `option<T>` — the absent branches read
        // null through the else-chain.  Order follows first appearance.
        let mut names: Vec<(String, Type)> = Vec::new();
        for (_, _, fields) in &alts {
            for (fname, _, ftype) in fields {
                if let Some((_, seen)) = names.iter().find(|(n, _)| n == fname) {
                    if !self.first_pass && !match_arm_types_unify(seen, ftype) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "alternation capture '{}' is {} in one branch but {} in another",
                            fname,
                            ftype.source_name(&self.data),
                            seen.source_name(&self.data)
                        );
                    }
                } else {
                    names.push((fname.clone(), ftype.clone()));
                }
            }
        }

        for (fname, ftype) in &names {
            let present_in_all = alts
                .iter()
                .all(|(_, _, fields)| fields.iter().any(|(n, _, _)| n == fname));
            let var_type = if present_in_all {
                ftype.clone()
            } else {
                Self::capture_optional(ftype)
            };
            let v_nr = self.create_unique(&format!("mv_{fname}"), &var_type);
            if v_nr == u16::MAX {
                continue;
            }
            self.vars.defined(v_nr);
            // if tag==V0 { f@V0 } else if tag==V1 { f@V1 } … else <null>.  A branch
            // lacking `f` is simply skipped — its tag falls through to the null else,
            // which is exactly the `option<T>` null a partial-overlap capture wants.
            let mut acc = self.absent_capture(&var_type);
            for (disc, vdef, fields) in alts.iter().rev() {
                if let Some((_, attr_idx, _)) = fields.iter().find(|(n, _, _)| n == fname) {
                    let read = if *attr_idx == usize::MAX {
                        elem.clone()
                    } else {
                        self.pattern_field_value(*vdef, *attr_idx, elem.clone())
                    };
                    let tag = self.elem_tag_int(elem.clone());
                    let test = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
                    acc = v_if(test, read, acc);
                }
            }
            bindings.push(v_set(v_nr, acc));
            let before = self.name_capture(fname, v_nr); // stays in scope for the arm body (as sub-patterns do)
            if !parens {
                self.pattern_binds_pending.push((fname.clone(), before));
            }
            // A heap capture is a borrowed VIEW of the subject element: skip its free
            // and record the borrow dep on the subject source (mirrors the field-
            // sub-pattern path); a text payload is an OWNED copy (freed normally).
            // The underlying `ftype` drives this even when the slot is `option<T>`.
            if !matches!(ftype.base(), Type::Text(_)) {
                self.vars.set_skip_free(v_nr);
            }
            if let Some(src) = borrow_src {
                let bound_tp = Self::element_view_of(&self.vars.tp(v_nr).clone(), src);
                self.vars.set_type(v_nr, bound_tp);
            }
        }
    }
}
