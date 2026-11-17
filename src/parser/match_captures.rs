// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The bindings a match pattern makes: the field bindings of a struct-enum arm, forward captures,
//! slice elements read by position, the rest of a slice, and literal-value matches.
//! A part of the match subject split out of `control.rs`; the others are listed there (`@F29`).

use super::control::{SliceGroupKind, max_lookahead};
use super::{
    DefType, I32, Level, LexItem, Parser, Type, Value, diagnostic_format, v_block, v_if, v_loop,
    v_set,
};
use crate::data::Deps;
use std::collections::HashSet;

impl Parser {
    /// Parse field bindings for a struct-enum match arm.
    pub(super) fn parse_match_enum_field_bindings(
        &mut self,
        variant_def_nr: u32,
        pattern_name: &str,
        subject_val: &Value,
        arm_stmts: &mut Vec<Value>,
        field_conditions: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) {
        self.lexer.token("{");
        let mut seen_fields: HashSet<String> = HashSet::new();
        while let Some(field_name) = self.lexer.has_identifier() {
            if !self.first_pass && seen_fields.contains(&field_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "duplicate field binding '{}' in match arm",
                    field_name
                );
            }
            seen_fields.insert(field_name.clone());

            let attr_idx_and_type = {
                let variant_def = self.data.def(variant_def_nr);
                variant_def.attributes[1..]
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == field_name)
                    .map(|(i, a)| (i + 1, a.typedef.clone()))
            };

            match attr_idx_and_type {
                Some((attr_idx, field_type)) => {
                    self.check_visibility("pattern-field", variant_def_nr, attr_idx);
                    self.sandbox_read_member(variant_def_nr, &field_name, subject_val);
                    let field_read = self.get_field(variant_def_nr, attr_idx, subject_val.clone());
                    if self.lexer.has_token(":") {
                        // `@FR-P-Point` — a bare lowercase NAME as a field's sub-pattern is a
                        // binding under that name (`Circle { radius: r }`), total like the
                        // shorthand `{ radius }`, and never a comparison with a variable of that
                        // name in scope.  Read through as a value, it compared against an outer
                        // `r` (a silent wrong arm) or refused an unknown one.  Only the plain
                        // name followed by `,` / `}` — `r..5`, `_`, a variant and a literal keep
                        // their sub-pattern meaning.
                        if let Some(bind_name) = self.field_pattern_rename() {
                            self.bind_match_field_capture(
                                variant_def_nr,
                                attr_idx,
                                &bind_name,
                                &field_type,
                                subject_val,
                                arm_stmts,
                                name_aliases,
                            );
                        } else if let Some(cond) = self.parse_field_sub_pattern(
                            field_read,
                            &field_type,
                            arm_stmts,
                            field_conditions,
                            name_aliases,
                        ) {
                            field_conditions.push(cond);
                        }
                    } else {
                        self.bind_match_field_capture(
                            variant_def_nr,
                            attr_idx,
                            &field_name,
                            &field_type,
                            subject_val,
                            arm_stmts,
                            name_aliases,
                        );
                    }
                }
                None => {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "variant {} has no field '{}'",
                            pattern_name,
                            field_name
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

    /// Pass 1 over a capture list whose subject's enum is not declared yet — `is V { a, b }`, or
    /// a `match` arm `V { a, b: c } =>`: consume the braces when they have a capture list's
    /// SHAPE — names, each optionally renamed, separated by `,` — and leave anything else (a
    /// body, a sub-pattern) where it is, answering `None`.  The same lookahead the known-enum
    /// path uses, minus the field names it cannot see yet.
    ///
    /// Each captured name is bound, for the body after it, to the capture the known-enum path
    /// creates (`_mv_<name>_N`, created in the same order), at `Unknown`: pass 2 knows the
    /// enum and refines that same variable's type.  Stepped over without a binding, the body
    /// read the name as whatever it means OUTSIDE the arm — a function of that name became a
    /// function value, and `limit + 1` was refused on pass 1, where pass 2 would have known
    /// better (loft#2326).  A `_` binds nothing.  Answers the names re-pointed and what each
    /// named before, for the caller's end of scope to restore.
    pub(super) fn bind_forward_capture_list(&mut self) -> Option<Vec<(String, Option<u16>)>> {
        if !self.lexer.peek_token("{") {
            return None;
        }
        let link = self.lexer.link();
        self.lexer.token("{");
        let mut binds: Vec<String> = Vec::new();
        loop {
            let Some(field) = self.lexer.has_identifier() else {
                self.lexer.revert(link);
                return None;
            };
            let mut bind = field;
            if self.lexer.has_token(":") {
                let Some(alias) = self.lexer.has_identifier() else {
                    self.lexer.revert(link);
                    return None;
                };
                bind = alias;
            }
            // `_` lexes as a name, and binds nothing.
            if bind != "_" {
                binds.push(bind);
            }
            if self.lexer.has_token("}") {
                break;
            }
            if !self.lexer.has_token(",") {
                self.lexer.revert(link);
                return None;
            }
        }
        let mut aliases = Vec::new();
        for name in binds {
            let v = self.create_unique(&format!("mv_{name}"), &Type::Unknown(0));
            self.vars.mark_blind_capture(v);
            if v == u16::MAX {
                continue;
            }
            self.vars.defined(v);
            let old = self.name_capture(&name, v);
            aliases.push((name, old));
        }
        Some(aliases)
    }

    /// After `is V {`, is `field: name` (then `,` or `}`) an attempted RENAME of a field of `V`,
    /// rather than a body opening with a declaration?  Only when `field` is a field of the
    /// variant and `name` is not a type — `{ radius: integer }` stays a body.  Called inside a
    /// lookahead the caller reverts, after `field` has been read.
    pub(super) fn is_capture_rename_attempt(
        &mut self,
        variant_def_nr: u32,
        field: Option<&str>,
    ) -> bool {
        let Some(field) = field else {
            return false;
        };
        // The first attribute is the discriminant; a unit variant has none at all (loft#2168).
        let is_field = self
            .data
            .def(variant_def_nr)
            .attributes
            .iter()
            .skip(1)
            .any(|a| a.name == field);
        if !is_field || !self.lexer.has_token(":") {
            return false;
        }
        let Some(alias) = self.lexer.has_identifier() else {
            return false;
        };
        Self::is_binding_name(&alias)
            && self.data.def_nr(&alias) == u32::MAX
            && (self.lexer.peek_token(",") || self.lexer.peek_token("}"))
    }

    /// A field sub-pattern that is a plain binding NAME (`Circle { radius: r }`) — a rename of
    /// the capture, `@FR-P-Point` — consumed and answered; anything else (`r..5`, `_`, a
    /// variant, a literal) is left unread for the sub-pattern parse.  Only a name followed by
    /// `,` or `}` qualifies, and the lexer is linked only when a name is next, so a numeric
    /// range is never re-lexed.  One home for the single-pattern and the listed-pattern arm.
    pub(super) fn field_pattern_rename(&mut self) -> Option<String> {
        if !matches!(
            &self.lexer.peek().has,
            LexItem::Identifier(id) if Self::is_binding_name(id)
        ) {
            return None;
        }
        let link = self.lexer.link();
        let name = self.lexer.has_identifier().unwrap_or_default();
        if self.lexer.peek_token(",") || self.lexer.peek_token("}") {
            Some(name)
        } else {
            self.lexer.revert(link);
            None
        }
    }

    /// A lowercase identifier other than `_` — the spelling that BINDS in a pattern.
    pub(super) fn is_binding_name(name: &str) -> bool {
        name != "_"
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
    }

    /// The discriminant integer for `variant_def_nr` (a variant of enum `e_nr`).
    /// Struct-enum variants store it in their own `attributes[0]`; unit variants
    /// and plain enums read it from the parent enum's attribute for the name.
    /// Mirrors the inline lookup the first pattern of an arm uses, factored out so
    /// the extra patterns of a `@PLN35` multi-pattern arm resolve their discs the
    /// same way.
    pub(crate) fn variant_disc(
        &self,
        e_nr: u32,
        is_struct: bool,
        variant_def_nr: u32,
        variant_name: &str,
    ) -> i32 {
        if is_struct
            && let Some(first) = self.data.def(variant_def_nr).attributes().first()
            && let Value::Enum(nr, _) = first.value
        {
            return i32::from(nr);
        }
        if let Some(a_nr) = self.data.def(e_nr).attr_names.get(variant_name)
            && let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value
        {
            return i32::from(nr);
        }
        0
    }

    /// The enum whose VARIANTS a pattern names when it is written against a value of type
    /// `tp` — `@FR-M-Unit` / `@FR-M-Variant` asked of a SLOT type rather than of a value.  The
    /// second half of the answer says whether it is a struct-enum, which is what the branches
    /// that need a payload (a repetition, an alternation, a `#lexeme` literal) require.
    ///
    /// A NULLABLE element names the same enum.  `@FR-L-Null` gives `E?` the layout of `E` —
    /// variants are numbered from 1 because 0 is the absent value — so the tag test a variant
    /// pattern builds already answers false for an absent element, which is exactly
    /// `(M-Variant)`: an absence is no variant.  Asking `Type::Enum` of the WRAPPER instead
    /// made a `vector<Tok?>` stop being a token stream: `[Id { x }]` was a parse error naming
    /// nothing, and the unit spelling `[Id]` fell through to the bare-name branch and became a
    /// BINDING that matched every element, absent ones included (loft#1410).
    ///
    /// The tagged `__nullable<S>` is NOT such an enum: its discriminant is `@FR-L-Null-Tag`'s
    /// presence bit and its two variants are the compiler's, so a pattern naming one would ask
    /// a variant question of an absence bit.  `None` keeps the element's own refusal there,
    /// which names the type the author wrote rather than the synthetic.
    pub(super) fn pattern_variant_enum(&self, tp: &Type) -> Option<(u32, bool)> {
        // Through a `&` too: a tuple element taken from a `&T` binding is that `T`'s value,
        // and a variant pattern asks its enum exactly as a direct subject does (loft#1526).
        // A value typed as one VARIANT (`w = Vn { rs: 7 }` infers `Vn`) is still a record of
        // its enum, tag and all, so its patterns name the enum's variants — the mapping the
        // top-level subject dispatch in `parse_match_inner` makes.  Asked of `Enum` alone, a
        // tuple element of variant type refused `(Vn { rs }, k)` as "Vn, which has no variants".
        if let Type::Reference(d_nr, _) = tp.peel_link()
            && self.data.def_type(*d_nr) == DefType::EnumValue
        {
            return Some((self.data.def(*d_nr).parent(), true));
        }
        // A `reference<E>` to a struct-enum (the recursive shape loft#1579's cycle refusal asks
        // for, `Add { l: reference<Expr>, … }`) is a pointer to a record of `E`: its read is
        // the record (`OpGetDbRef`), so a variant pattern asks it exactly as it asks an
        // inline field.  Unrecognised, `l: Lit { v: 0 }` fell to the scalar path, which built
        // a `Lit` and tested THAT: the arm never matched on the interpreter, `--native`
        // refused its own output, and the binding form did not parse (loft#1870).
        if let Type::Reference(d_nr, _) = tp.peel_link()
            && self.data.def_type(*d_nr) == DefType::Enum
            && self.data.def(*d_nr).attributes().iter().any(|a| {
                matches!(a.value, Value::Enum(_, _))
                    && self.data.def_type(self.data.variant_of(*d_nr, &a.name))
                        == DefType::EnumValue
            })
        {
            return Some((*d_nr, true));
        }
        let Type::Enum(e_nr, is_struct, _) = tp.peel_link() else {
            return None;
        };
        if self.nullable_payload_struct(*e_nr).is_some() {
            return None;
        }
        Some((*e_nr, *is_struct))
    }

    /// How a slice element's type is SPELLED to the author.  The tagged `__nullable<S>` is the
    /// compiler's spelling of `S?` and never belongs in a message: a repetition over a
    /// `vector<S?>` reported *"'S' is not a variant of __nullable<S>"*, which names a type the
    /// program cannot write and a question the author did not ask (loft#1410).
    pub(super) fn slice_element_name(&self, elm_tp: &Type) -> String {
        if matches!(elm_tp, Type::Enum(_, true, _))
            && let Some(struct_d) = self.data.nullable_struct_payload(elm_tp)
        {
            return format!("{}?", self.data.def(struct_d).name());
        }
        elm_tp.source_name(&self.data)
    }

    /// Consume a match arm's body without parsing it — for an arm whose PATTERN was refused, so
    /// the names that pattern would have bound do not come back as a second, misleading
    /// *"Unknown variable 'x'"* pointing into a body that is fine.  Stops after the arm's own
    /// `,`, or at the `}` that closes the match.
    pub(super) fn skip_match_arm_body(&mut self) {
        let mut depth = 0i32;
        loop {
            match &self.lexer.peek().has.clone() {
                LexItem::None => break,
                LexItem::Token(t) if depth == 0 && *t == "," => {
                    self.lexer.cont();
                    break;
                }
                LexItem::Token(t) if depth == 0 && *t == "}" => break,
                LexItem::Token(t) if *t == "{" || *t == "(" || *t == "[" => {
                    depth += 1;
                    self.lexer.cont();
                }
                LexItem::Token(t) if *t == "}" || *t == ")" || *t == "]" => {
                    depth -= 1;
                    self.lexer.cont();
                }
                _ => self.lexer.cont(),
            }
        }
    }

    /// @PLN35 L2 — is the next element a VARIANT sub-pattern of the element enum type
    /// (`Ship { carrier }` / bare `Ship`)?
    ///
    /// Peeks without consuming, so a plain binding name still falls through to the
    /// binding branch.  Asked in the TAIL as well as the head: `(P-Rest)` counts the fixed
    /// patterns after the rest and `(P-Point)` makes a variant one of them, so the tail
    /// takes the forms the head does — it just reads at a negative index (loft#1419).
    pub(super) fn peek_is_variant_subpattern(&mut self, elm_tp: &Type) -> bool {
        // @FR-P-Point — a plain struct `S { f… }` is a point pattern too: the element's own
        // struct name opens it, and `parse_field_sub_pattern` parses it as a field's would —
        // over a `S?` element's tagged slot and a forward-declared `S` alike.
        let rec = self
            .tagged_pointer_type(elm_tp)
            .map(|(_, pointer)| pointer)
            .or_else(|| self.forward_field_type(elm_tp));
        let elm_tp = rec.as_ref().unwrap_or(elm_tp);
        if self.undeclared_struct_pattern(elm_tp) {
            return true;
        }
        if let Type::Reference(s_nr, _) = elm_tp.peel_link()
            && self.data.def_type(*s_nr) == DefType::Struct
        {
            return matches!(&self.lexer.peek().has, LexItem::Identifier(id) if *id == self.data.def(*s_nr).name());
        }
        let Some((elm_e_nr, _)) = self.pattern_variant_enum(elm_tp) else {
            return false;
        };
        if let LexItem::Identifier(pname) = &self.lexer.peek().has.clone() {
            self.data.variant_of(elm_e_nr, pname) != u32::MAX
        } else {
            false
        }
    }

    /// How many fixed elements follow a slice `..`, counted without consuming them.
    ///
    /// A tail element is read from the END (`-(tail_len - j)`), so its position is not known
    /// until the whole tail has been seen — which is why only a BARE NAME was ever accepted
    /// there.  A bare name defers trivially, because the bind loop runs after `]`; a variant
    /// sub-pattern or a literal emits its test AS it parses and needs the index in hand
    /// (loft#1419).  Counting first gives every branch the index it needs, so the tail admits
    /// the element forms the head does — which is what `(P-Rest)` says, `t` being the count of
    /// fixed patterns after the rest, with `(P-Point)` making a variant one of them.
    ///
    /// ONE look-ahead over the region, reverted before the real parse.  Deliberately not a
    /// second peek over ground `peek_group_kind` has already covered: that is what corrupts
    /// the replay buffer, and this runs where no group peek has been.
    pub(super) fn count_slice_tail_elements(&mut self) -> i32 {
        let link = self.lexer.link();
        let mut depth = 0i32;
        let mut commas = 0i32;
        let mut seen_any = false;
        loop {
            if matches!(self.lexer.peek().has, LexItem::None) {
                break;
            }
            if depth == 0 && self.lexer.peek_token("]") {
                break;
            }
            if self.lexer.peek_token("{")
                || self.lexer.peek_token("(")
                || self.lexer.peek_token("[")
            {
                depth += 1;
                seen_any = true;
            } else if self.lexer.peek_token("}")
                || self.lexer.peek_token(")")
                || self.lexer.peek_token("]")
            {
                depth -= 1;
            } else if depth == 0 && self.lexer.peek_token(",") {
                commas += 1;
            } else {
                seen_any = true;
            }
            self.lexer.cont();
        }
        self.lexer.revert(link);
        if seen_any { commas + 1 } else { 0 }
    }

    /// Recover from a REFUSED slice element by swallowing the rest of the pattern through its
    /// closing `]` — what the group parsers do on the paths that succeed.
    ///
    /// Without it the refusal is followed by a first-pass cascade (*"Expect token ,"*, *"Expect
    /// token }"*, *"unexpected ')'"*) that aborts the run before the second pass emits the real
    /// reason, so the author is told about a comma instead of about the element type.  Measured
    /// on a repetition over a NON-enum element, which has always landed here.
    /// Pass 1's reading of a pattern over a subject it cannot type yet (loft#2522): every token
    /// up to the arm's `=>` at bracket depth 0.  The names the pattern binds are bound at an
    /// unknown type, as `bind_forward_capture_list` binds a variant's, so the body that reads
    /// them parses; pass 2 binds them by the real pattern.
    pub(super) fn skip_unresolved_pattern(&mut self) {
        let mut depth = 0i32;
        loop {
            match &self.lexer.peek().has.clone() {
                LexItem::None => break,
                LexItem::Token(t) if depth == 0 && (*t == "=>" || *t == "->") => break,
                LexItem::Token(t) if *t == "[" || *t == "(" || *t == "{" => {
                    depth += 1;
                    self.lexer.cont();
                }
                LexItem::Token(t) if *t == "]" || *t == ")" || *t == "}" => {
                    depth -= 1;
                    self.lexer.cont();
                }
                LexItem::Identifier(name) if name != "_" && name != "if" => {
                    let name = name.clone();
                    self.lexer.cont();
                    // A name followed by `(` or `{` is a variant or a struct, not a capture.
                    if !self.lexer.peek_token("(") && !self.lexer.peek_token("{") {
                        // Not marked defined: pass 2 binds the name to a variable of its own
                        // pattern, and this one stays unknown and unread.
                        let v = self.create_var(&name, &Type::Unknown(0));
                        if v != u16::MAX {
                            let before = self.name_capture(&name, v);
                            self.pattern_binds_pending.push((name, before));
                        }
                    }
                }
                _ => self.lexer.cont(),
            }
        }
    }

    pub(super) fn skip_rest_of_slice(&mut self) {
        while !self.lexer.peek_token("]") && !matches!(self.lexer.peek().has, LexItem::None) {
            self.lexer.cont();
        }
        self.lexer.token("]");
    }

    /// The integer-discriminant read `OpConvIntFromEnum(OpGetEnum(elem, 0))` for a
    /// struct-enum element value — the same tag read a top-level struct-enum match
    /// emits, used by a slice-element alternation to tag-test each branch.
    pub(crate) fn elem_tag_int(&mut self, elem: Value) -> Value {
        let get_enum = self.cl("OpGetEnum", &[elem, Value::Int(0)]);
        self.cl("OpConvIntFromEnum", &[get_enum])
    }

    /// @PLN35 Phase 4.3 — the single `read(pos)` seam for a slice element.  EVERY element
    /// read in a slice pattern goes through here: `get_field(elm_type, whole, OpGetVector(v,
    /// size, pos))`.  `pos` is a `Value` — a compile-time `Int` for a fixed position today, a
    /// runtime `Var` (or `pos + offset`) once a variable-width alternation advances a cursor.
    /// Funnelling all reads here is what lets a streamed/accumulating backing (Phase 7)
    /// replace the read without touching the match logic — so do NOT open-code
    /// `OpGetVector` + `get_field` at a slice element site; call this.
    pub(super) fn read_slice_elem(
        &mut self,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: Value,
    ) -> Value {
        // @PLN35 PC1 — in CURSOR mode a forward read `v[i]` is relative to the cursor's current
        // `pos`, so it becomes `source[pos + i]`.  Only non-negative literal positions are offset
        // (a negative/tail read has no meaning for a prefix cursor and is rejected upstream).
        let pos = match self.match_cursor {
            Some((_, _, _, pos_var)) if matches!(&pos, Value::Int(i) if *i >= 0) => {
                self.conv_op("+", Value::Var(pos_var), pos, I32.clone(), I32.clone())
            }
            _ => pos,
        };
        let get = self.cl("OpGetVector", &[Value::Var(v), elm_size.clone(), pos]);
        // @FR-H-Stride — the WIDTH of the read is the declared TYPE's.  The general path below
        // reaches `get_val` through `get_field(_, usize::MAX, _)`, which re-derives the type from
        // the DEF (`attr_type` answers `def.returned` for `usize::MAX`) — and a def cannot carry a
        // width, because the one `integer` def serves all seven.  So a narrow element was read
        // EIGHT bytes wide however the vector was declared, and `[a, .., z]` over a `vector<u8>`
        // answered `21542142465` = `0x05_04_03_02_01`, the five elements swallowed whole
        // (loft#1420).  `get_val` already selects the narrow read op from the spec when the alias
        // is absent; it just has to be given the type the caller still holds.
        if let Some((_, nullable, _)) = crate::data::Data::narrow_vector_element(elm_tp) {
            self.expr_not_null = !nullable;
            self.expr_not_null_name.clear();
            return self.get_val(elm_tp, nullable, 0, get, u32::MAX);
        }
        // A TUPLE element is stored as a `__tuple<…>` record and lives on the stack as its
        // members, so the read unboxes the element's DbRef member by member — the one read
        // `v[i]` makes.  Handed the DbRef itself, a `vector<(…)>` slice binding read garbage on
        // the interpreter's stack layout and failed to compile on `--native` (loft#1737).
        if let Type::Tuple(elems) = elm_tp.base() {
            return self.unbox_tuple_from_dbref(get, elems);
        }
        let td = self.data.type_def_nr(elm_tp);
        self.get_field(td, usize::MAX, get)
    }

    /// @PLN35 Phase 7 — the LENGTH half of the match Cursor seam (`read_slice_elem` is the READ
    /// half).  The match engine asks the subject "how long are you?" ONLY through here, so a future
    /// STREAMING cursor can answer differently (pull-to-exhaustion / `max_lookahead`-bounded)
    /// without any match logic changing.  The vector cursor emits `OpLengthVector`, byte-identical
    /// to the inline calls this replaced.
    pub(super) fn cursor_len(&mut self, v: u16) -> Value {
        self.cl("OpLengthVector", &[Value::Var(v)])
    }

    /// @PLN35 Phase 7 (Path B, step 2a) — materialise a coroutine `iterator<T>` match subject into a
    /// fresh `vector<T>` buffer (EAGER pull), so the existing vector-match machinery runs over it.
    /// Emits `gen = subject; done = false; buf = []; while !done { x = next(gen); if exhausted(gen)
    /// { done = true } else { buf += [x] } }` and returns `(buf_var, setup_ops)`.  The pull uses
    /// explicit `OpCoroutineNext`/`OpCoroutineExhausted` in a `while` — a `for` over a STORED
    /// coroutine hangs.  A lazy per-read pull is the step-2b refinement behind the Cursor seam.
    pub(super) fn collect_iterator_subject(
        &mut self,
        subject: Value,
        iter_tp: &Type,
        elm_tp: &Type,
        match_pos: &crate::lexer::Position,
    ) -> (u16, Type, Vec<Value>) {
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        let buf = self.create_unique("stream_buf", &vec_tp);
        self.vars.defined(buf);
        let gen_var = self.create_unique("stream_gen", iter_tp);
        self.vars.defined(gen_var);
        let done = self.create_unique("stream_done", &Type::Boolean);
        self.vars.defined(done);
        let x = self.create_unique("stream_x", elm_tp);
        self.vars.defined(x);
        // A RECORD-valued yield hands back a DbRef into the coroutine's own frame, which
        // lives in the STACK store — `x` names it, and the append below deep-copies it into
        // the buffer (`OpCopyRecord`), so the buffer owns the copy and `x` owns nothing.
        // Scope cleanup did not know that and emitted `OpFreeRef(_stream_x_1)` at the end of
        // every pull iteration, whole-store freeing a stack-record ref: `BUG (#306)` on each
        // run of `match <iterator<StructEnum>>`, and only the store-0 guard stopped it from
        // taking the eval stack with it (loft#920).  Same fact as `elm` below, one step
        // earlier in the same loop.
        //
        // Gated on the record case rather than set for every element type: `skip_free` is one
        // bit for all free kinds, and a `text` element's `x` holds a String the caller DOES
        // own — suppressing its `OpFreeText` would trade this wrong free for a leak of one
        // string per yield.  A scalar emits no free at all, so it never reaches either.
        if matches!(
            elm_tp.base(),
            Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
        ) {
            self.vars.set_skip_free(x);
        }
        let ed_nr = self.data.type_def_nr(elm_tp);
        let elm = self.create_unique("stream_elm", &Type::Reference(ed_nr, Deps::none()));
        self.vars.defined(elm);
        // `elm` is a TRANSIENT ref to the record just appended into `buf` — it belongs to the
        // buffer, not to `elm`.  Without skip_free, scope cleanup emits `OpFreeRef(elm)` after the
        // append and frees that record (harmless for an inline int, but it frees the string for a
        // `text` element — the null-value bug).  The comprehension avoids this via a buffer-db dep.
        self.vars.set_skip_free(elm);

        // `OpCoroutineNext` value_size: packed `(channel_tag << 8) | byte_size` of the yield type
        // (same encoding `next(gen)` computes — collections.rs `iterator`).
        let byte_size = i32::from(crate::variables::size(
            elm_tp,
            &crate::data::Context::Argument,
        ));
        let channel_tag = crate::coroutine_layout::channel_tag(elm_tp);
        let value_size = (channel_tag << 8) | byte_size;

        // `vector_db` takes the ELEMENT type: handed `vec_tp`, the buffer's store was typed
        // `vector<vector<E>>`, and its release could not see the records it holds.
        let mut setup: Vec<Value> = self.vector_db(elm_tp, buf);
        setup.push(v_set(gen_var, subject));
        setup.push(v_set(done, Value::Boolean(false)));

        let next_call = self.cl(
            "OpCoroutineNext",
            &[Value::Var(gen_var), Value::Int(value_size)],
        );
        let exhausted = self.cl("OpCoroutineExhausted", &[Value::Var(gen_var)]);
        // The append a `[for x in <iter> { x }]` comprehension emits, through its one home:
        // this buffer carried its own copy, which wrote a `vector<T>` element as a scalar and
        // so refused every element type but a scalar, text or struct-enum (loft#1737).
        let mut append_ops = self.append_element_ops(buf, elm_tp, elm, Value::Var(x));
        // `(G-Own)`: a value the generator hands over is `x`'s own, so the append MOVES it
        // into the buffer — the copy frees `x`'s store (`COPY_FREE_SOURCE`) and runs no drop
        // hook, since the resource went with the copy the buffer releases.  `x` stays
        // `skip_free`: nothing is left for its scope end to release.
        if crate::coroutine_layout::yield_handed_over(elm_tp) {
            let copy_d = self.data.def_nr("OpCopyRecord");
            for op in &mut append_ops {
                if let Value::Call(d, args) = op.unspan_mut()
                    && *d == copy_d
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == x)
                    && let Some(Value::Int(tp)) = args.get_mut(2).map(Value::unspan_mut)
                {
                    *tp |= i32::from(crate::keys::COPY_FREE_SOURCE);
                }
            }
        }
        let append = v_block(append_ops, Type::Void, "stream append");
        // `@FR-P-IterBound` — the pull is BOUNDED by `max_lookahead`, and exceeding it is a
        // defined runtime error, never a hang (loft#1678).  The subject is materialised before
        // the patterns run (`35p-iterator-match.loft`), so an endless source had no ceiling and
        // filled memory on both backends — 2 GB in 18 s on `--native`.  The count is of
        // elements APPENDED, and the stop is `panic`, the language's defined error: it names
        // the `match`, the bound and the switch, and runs on both backends alike.  The bound is
        // a compile-time constant, `LOFT_MAX_LOOKAHEAD` (default one million, `0` = unbounded):
        // far above any pattern a program matches over a finite source, far below an
        // allocation that takes the machine down.
        let limit = max_lookahead();
        let append = if limit > 0 {
            let n = self.create_unique("stream_n", &I32);
            self.vars.defined(n);
            setup.push(v_set(n, Value::Int(0)));
            let bump = self.cl("OpAddInt", &[Value::Var(n), Value::Int(1)]);
            let over = self.cl("OpLtInt", &[Value::Int(limit), Value::Var(n)]);
            let stop = Value::Call(
                self.data.def_nr("n_panic"),
                vec![
                    Value::str(&format!(
                        "a `match` over an iterator read more than {limit} elements (max_lookahead) — \
                         the source may be endless; bound it, or raise LOFT_MAX_LOOKAHEAD"
                    )),
                    Value::str(match_pos.file.as_str()),
                    Value::Int(match_pos.line as i32),
                ],
            );
            v_block(
                vec![v_set(n, bump), v_if(over, stop, Value::Null), append],
                Type::Void,
                "stream bounded append",
            )
        } else {
            append
        };
        let loop_body = vec![
            v_if(Value::Var(done), Value::Break(0), Value::Null),
            v_set(x, next_call),
            v_if(exhausted, v_set(done, Value::Boolean(true)), append),
        ];
        setup.push(v_loop(loop_body, "stream pull"));
        // `@FR-B-View`: the buffer views the `__vdb` store `vector_db` minted for it, and its
        // type now says so.  The match subject is typed from what this returns, so handing back
        // the dep-less `vec_tp` made `_match_subj_N` an owner of `__vdb_N`'s store beside
        // `__vdb_N` itself — two frees of one store at the match's end.
        let buf_tp = self.vars.tp(buf).clone();
        (buf, buf_tp, setup)
    }

    /// @PLN35 (`@FR-P-Cap-Fresh`) — materialise a named `..rest` sub-slice `v[lo .. hi]` into a
    /// FRESH independent `vector<T>` (`rest_var`), by reusing the proven compile-time
    /// slice-materialise path
    /// (`materialize_iterator` over a minimal index-range `Iter`).  `lo`/`hi` are `Value`s: a
    /// compile-time `Int(head_len)` / `len − tail_len` for a fixed-arity slice, or a runtime
    /// cursor `Var(pos)` / `len` once a variable-width alternation determines the head width
    /// (Phase 4.3 step 5).  Reads from the subject SOURCE var `v` (not the `_match_subj` copy),
    /// matching the `x = v[lo..hi]` path.  Pushes the materialisation into `bindings`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn materialize_named_rest(
        &mut self,
        v: u16,
        elm_tp: &Type,
        elm_size: &Value,
        rest_var: u16,
        vec_tp: &Type,
        lo_val: Value,
        hi_val: Value,
        step: i32,
        bindings: &mut Vec<Value>,
    ) {
        // A pattern binding is a NEW binding (`@FR-B-Scope`): the arm binds its names fresh, and
        // an earlier `match` that bound the same name in a block that has ended left only a type
        // carrying ITS store's dep.  Read through that dep, `vector_needs_db` lowered this bind as
        // a reassignment (`OpClearVector` + refill) with no declaring `Set`, so the scope pass had
        // nothing to split and native — which declares the first binding inside its own arm —
        // could not see the name: two `match`es binding one `..rest` or `(xs: V)*` did not
        // compile.  Starting from the owned type makes it the fresh bind it is.
        self.vars
            .set_type(rest_var, vec_tp.with_deps(&Deps::none()));
        let lo_slot = self.create_unique("rest_lo", &I32);
        let hi_slot = self.create_unique("rest_hi", &I32);
        let idx = self.create_unique("rest_idx", &I32);
        let null_idx = self.null(&I32);
        let init = Value::Insert(vec![
            v_set(lo_slot, lo_val),
            v_set(hi_slot, hi_val),
            v_set(idx, null_idx),
        ]);
        // index step: `idx = !idx ? lo : idx + step ; if hi <= idx break ; idx`.  `step` is 1 for
        // a contiguous sub-slice (`..rest`, a bare `(V)*` run) and 2 for a SEPARATED run
        // `(V)*(Sep)` — the V's sit at every other index (V Sep V Sep …), so a stride of 2 skips
        // the separators.  `!idx` is a NULL test (the null sentinel is i32::MIN, not 0), so a `lo`
        // of 0 seeds correctly and never re-triggers the first-iteration branch.
        let bump = self.conv_op(
            "+",
            Value::Var(idx),
            Value::Int(step),
            I32.clone(),
            I32.clone(),
        );
        let not_idx = self.single_op("!", Value::Var(idx), I32.clone());
        let advance = v_set(idx, v_if(not_idx, Value::Var(lo_slot), bump));
        let past_end = self.conv_op(
            "<=",
            Value::Var(hi_slot),
            Value::Var(idx),
            I32.clone(),
            I32.clone(),
        );
        let brk = v_if(past_end, Value::Break(0), Value::Null);
        let idx_block = v_block(
            vec![advance, brk, Value::Var(idx)],
            I32.clone(),
            "Iter range",
        );
        // Element read must match the slice-subscript builder in `fields.rs`
        // (`parse_vector_index`): a linked struct derefs its record pointer, a
        // base/primitive wraps with `get_val` (e.g. `OpGetInt`/`OpGetText`), a
        // tuple unboxes, and an inline struct stays the RAW record DbRef so
        // `materialize_iterator`'s `OpCopyRecord` deep-copies it.
        let elm_type_def = match self.data.type_elm(elm_tp) {
            u32::MAX => self.data.source_nr(0, "reference"),
            e => e,
        };
        let known = self.data.def(elm_type_def).known_type();
        let elm_read = if self.database.is_linked(known) {
            self.cl("OpVectorRefNullable", &[Value::Var(v), idx_block])
        } else {
            let mut r = self.cl(
                "OpGetVectorNullable",
                &[Value::Var(v), elm_size.clone(), idx_block],
            );
            if self.database.is_base(known) {
                r = self.get_val(elm_tp, matches!(elm_tp, Type::Optional(_)), 0, r, u32::MAX);
            } else if let Type::Tuple(elems) = elm_tp {
                let elems = elems.clone();
                r = self.unbox_tuple_from_dbref(r, &elems);
            }
            r
        };
        let next = v_block(vec![elm_read], elm_tp.clone(), "Vector Index");
        let mut mat = Value::Iter(
            u16::MAX,
            Box::new(init),
            Box::new(next),
            Box::new(Value::Null),
        );
        // A VIEW-read element type (a DbRef INTO the subject) needs a borrow dep on the transient
        // per-iteration read temp so the free-analysis frees each read once (via the copy source),
        // not double-freeing the subject; text/scalar take NO dep (owned copy / no DbRef).
        let elm_borrowed = Self::element_view_of(elm_tp, v);
        let iter_tp = Type::Iterator(Box::new(elm_borrowed), Box::new(Type::Null));
        self.materialize_iterator(
            &mut mat,
            &iter_tp,
            &Value::Var(rest_var),
            vec_tp,
            rest_var,
            "=",
        );
        // `rest`'s elements are deep-copied (OpCopyRecord) into its own store — INDEPENDENT of the
        // subject — so reset the ELEMENT type to the clean `elm_tp` while PRESERVING `rest`'s
        // vector-level deps (the fresh-store dep `materialize_iterator` set); leaving the element
        // borrow-dep would make the analysis treat `rest` as borrowing the subject → a leak.
        if let Type::Vector(_, vdeps) = self.vars.tp(rest_var).clone() {
            self.vars
                .set_type(rest_var, Type::Vector(Box::new(elm_tp.clone()), vdeps));
        }
        bindings.push(mat);
    }

    /// @PLN35 slice 2 — collect a per-iteration FIELD projection from a struct-enum repetition
    /// run `( V { field } )*`.  Like `materialize_named_rest`, but the per-element read projects
    /// `variant.field` (`get_field`) instead of the whole element, so the result is a fresh
    /// `vector<field_type>`.  The run already tag-tested every element against
    /// `variant_def_nr`, so the field read is valid at each index.  A scalar/text field is an
    /// OWNED value; a heap field is a view into the subject and is read under a borrow dep, as a
    /// whole element is — either way `materialize_iterator` copies it into the projection.
    /// Reads `v[lo..hi]` with `step` (2 for a separated run `(V)*(Sep)`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn materialize_field_projection(
        &mut self,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        variant_def_nr: u32,
        attr_idx: usize,
        field_type: &Type,
        proj_var: u16,
        proj_vec_tp: &Type,
        lo_val: Value,
        hi_val: Value,
        step: i32,
        bindings: &mut Vec<Value>,
    ) {
        // A fresh binding, for `materialize_named_rest`'s reason.
        self.vars
            .set_type(proj_var, proj_vec_tp.with_deps(&Deps::none()));
        let lo_slot = self.create_unique("proj_lo", &I32);
        let hi_slot = self.create_unique("proj_hi", &I32);
        let idx = self.create_unique("proj_idx", &I32);
        let null_idx = self.null(&I32);
        let init = Value::Insert(vec![
            v_set(lo_slot, lo_val),
            v_set(hi_slot, hi_val),
            v_set(idx, null_idx),
        ]);
        // Same index step as `materialize_named_rest`: `idx = !idx ? lo : idx + step; if hi <= idx
        // break; idx`.  `!idx` is a NULL test (sentinel i32::MIN), so a `lo` of 0 seeds correctly.
        let bump = self.conv_op(
            "+",
            Value::Var(idx),
            Value::Int(step),
            I32.clone(),
            I32.clone(),
        );
        let not_idx = self.single_op("!", Value::Var(idx), I32.clone());
        let advance = v_set(idx, v_if(not_idx, Value::Var(lo_slot), bump));
        let past_end = self.conv_op(
            "<=",
            Value::Var(hi_slot),
            Value::Var(idx),
            I32.clone(),
            I32.clone(),
        );
        let brk = v_if(past_end, Value::Break(0), Value::Null);
        let idx_block = v_block(
            vec![advance, brk, Value::Var(idx)],
            I32.clone(),
            "Iter range",
        );
        // Read the enum element at `idx` the same way the head sub-pattern path does
        // (`read_slice_elem` → an enum value `get_field` can read from), then project the field.
        let elm_read = self.read_slice_elem(v, elm_size, elm_tp, idx_block);
        let field_read = self.get_field(variant_def_nr, attr_idx, elm_read);
        let next = v_block(vec![field_read], field_type.clone(), "Field projection");
        let mut mat = Value::Iter(
            u16::MAX,
            Box::new(init),
            Box::new(next),
            Box::new(Value::Null),
        );
        // `@FR-H-Alloc` — a HEAP field (a vector, a struct, a struct-enum) is a VIEW into the
        // subject's store, exactly as a whole element is, so the per-element read carries the
        // same borrow dep `materialize_named_rest` gives it: typed as an owned value, its
        // scope-end free released the SUBJECT's record on every iteration (loft#1735).  Each one
        // is deep-copied into the projection; a scalar or text field is unchanged.
        let field_borrowed = Self::element_view_of(field_type, v);
        let iter_tp = Type::Iterator(Box::new(field_borrowed), Box::new(Type::Null));
        self.materialize_iterator(
            &mut mat,
            &iter_tp,
            &Value::Var(proj_var),
            proj_vec_tp,
            proj_var,
            "=",
        );
        // The copies are the projection's own, so its ELEMENT type drops the borrow again while
        // keeping the vector-level deps `materialize_iterator` set — as `materialize_named_rest`.
        if let Type::Vector(_, vdeps) = self.vars.tp(proj_var).base().clone() {
            self.vars
                .set_type(proj_var, Type::Vector(Box::new(field_type.clone()), vdeps));
        }
        bindings.push(mat);
    }

    /// @PLN35 Phase 6.3 — is the next token the start of a LITERAL slice element (`[ 1, … ]`,
    /// `[ "kw", … ]`, `[ 'c', … ]`, `[ -1, … ]`)?  A pure peek — consumes nothing.
    pub(super) fn peek_is_slice_literal(&self) -> bool {
        matches!(
            self.lexer.peek().has,
            LexItem::Integer(..)
                | LexItem::Long(_)
                | LexItem::Float(..)
                | LexItem::Single(_)
                | LexItem::CString(_)
                | LexItem::Character(_)
        ) || self.lexer.peek_token("-")
    }

    /// @PLN35 Phase 6.3 — may a literal of `lit_tp` match a slice element of `elm_tp`?  Same
    /// category (numeric / text / character / boolean), allowing the int→float widening the
    /// expression parser already applies to a numeric literal.
    fn slice_literal_compatible(elm_tp: &Type, lit_tp: &Type) -> bool {
        matches!(
            (elm_tp.base(), lit_tp.base()),
            (Type::Integer(_), Type::Integer(_))
                | (Type::Float, Type::Float | Type::Integer(_))
                | (Type::Single, Type::Single | Type::Float | Type::Integer(_))
                | (Type::Text(_), Type::Text(_))
                | (Type::Character, Type::Character)
                | (Type::Boolean, Type::Boolean)
        )
    }

    /// @PLN35 Phase 6.3 (`#lexeme`) — desugar a LITERAL slice element `[ "fn", … ]` on a
    /// struct-enum (token) element into a match against the `#lexeme` field.  A bare `"fn"`
    /// matches iff the element is SOME variant whose `#lexeme` field (compatible with the
    /// literal's type) equals the literal — an OR over the eligible variants of
    /// `tag(v[pos]) == disc && v[pos].<lexeme> == lit`.  The field read sits inside the tag
    /// test's `then`, so a non-matching variant never reads at the wrong offset.  Returns the
    /// condition, or `None` if the enum has no `#lexeme` field the literal can match.
    #[allow(clippy::too_many_arguments)]
    fn build_lexeme_literal_match(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: &Value,
        lit: &Value,
        lit_tp: &Type,
    ) -> Option<Value> {
        let variant_names: Vec<String> = self
            .data
            .def(e_nr)
            .attributes()
            .iter()
            .map(|a| a.name.clone())
            .collect();
        let mut acc: Option<Value> = None;
        for vname in variant_names {
            let vdef = self.data.variant_of(e_nr, &vname);
            if vdef == u32::MAX {
                continue;
            }
            // The variant's `#lexeme` field (skip attr 0, the discriminant) of a type the
            // literal can match.
            let lex = self
                .data
                .def(vdef)
                .attributes()
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, a)| a.lexeme && Self::slice_literal_compatible(&a.typedef, lit_tp))
                .map(|(i, a)| (i, a.typedef.clone()));
            let Some((lex_idx, lex_tp)) = lex else {
                continue;
            };
            let disc = self.variant_disc(e_nr, true, vdef, &vname);
            let tag_elem = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
            let tag = self.elem_tag_int(tag_elem);
            let tag_eq = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
            let field_elem = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
            let field = self.get_field(vdef, lex_idx, field_elem);
            let field_eq = self.conv_op("==", field, lit.clone(), lex_tp, lit_tp.clone());
            let branch = v_if(tag_eq, field_eq, Value::Boolean(false)); // tag && field
            acc = Some(match acc {
                Some(a) => v_if(a, Value::Boolean(true), branch), // a || branch
                None => branch,
            });
        }
        acc
    }

    /// @PLN35 Phase 6.3 — the equality condition for a LITERAL element at `pos` (a position
    /// VALUE, so callers pass `Int(i)` for a head element or `Int(-(tail_len-j))` for a tail
    /// element read from the end).  A SCALAR element compares directly; a STRUCT-ENUM element
    /// matches its `#lexeme` field.  `None` = the literal cannot match this element type.
    pub(super) fn build_literal_match(
        &mut self,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: &Value,
        lit: &Value,
        lit_tp: &Type,
    ) -> Option<Value> {
        if let Some((e_nr, true)) = self.pattern_variant_enum(elm_tp) {
            self.build_lexeme_literal_match(e_nr, v, elm_size, elm_tp, pos, lit, lit_tp)
        } else if Self::slice_literal_compatible(elm_tp, lit_tp) {
            let read = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
            Some(self.conv_op("==", read, lit.clone(), elm_tp.clone(), lit_tp.clone()))
        } else {
            None
        }
    }

    /// @PLN35 — the diagnostic when a literal slice element cannot match `elm_tp` (raised when
    /// `build_literal_match` returns `None`): a `#lexeme` hint for a struct-enum, a type mismatch
    /// for a scalar.
    pub(super) fn slice_literal_mismatch(&mut self, elm_tp: &Type, lit_tp: &Type) {
        if let Some((e_nr, true)) = self.pattern_variant_enum(elm_tp) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "{} has no `#lexeme` field a {} literal can match — mark a field `#lexeme` or write the variant pattern",
                self.data.def(e_nr).name(),
                lit_tp.source_name(&self.data)
            );
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a {} literal cannot match a {} slice element",
                lit_tp.source_name(&self.data),
                elm_tp.source_name(&self.data)
            );
        }
    }

    /// @PLN35 — ONE lexical look-ahead (no parser side effects) at a `(` in a slice pattern
    /// that classifies the group in a single save/revert.  (Two sequential peeks over the same
    /// region corrupt the lexer's replay buffer — a later peek reads stale state — so all the
    /// group kinds share this one scan.)  Skips an optional `name:` capture prefix and a
    /// balanced `{ … }` body, then decides from the token following the FIRST variant
    /// sub-pattern — stopping there so a non-routed group `(A | B)` does not over-scan and
    /// shift a downstream diagnostic column (`revert` restores `peek` but, by design, not the
    /// forward scan `position` that error rendering reads):
    ///   • another identifier    → branch 1 is a sequence         → `Alt` (multi-element)
    ///   • `)` then `?`           → optional `(A)?`                → `Alt` (degenerate `(A|ε)`)
    ///   • `)` then `*` / `+`     → repetition `(A)*` / `(A)+`     → `Repetition`
    ///   • anything else (`|`, `)` bare, `,`) → single-element alt / bare group → `Other`
    pub(super) fn peek_group_kind(&mut self) -> SliceGroupKind {
        let save = self.lexer.link();
        self.lexer.cont(); // past `(`
        let mut kind = SliceGroupKind::Other;
        if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
            self.lexer.cont(); // first identifier (a `name:` prefix or the variant itself)
            if self.lexer.peek_token(":") {
                // it was a `name:` capture prefix — the variant name follows.
                self.lexer.cont(); // `:`
                if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                    self.lexer.cont(); // variant name
                }
            }
            if self.lexer.peek_token("::") {
                self.lexer.cont();
                if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                    self.lexer.cont();
                }
            }
            if self.lexer.peek_token("{") {
                let mut depth = 0i32;
                loop {
                    match &self.lexer.peek().has.clone() {
                        LexItem::Token(t) if *t == "{" => {
                            depth += 1;
                            self.lexer.cont();
                        }
                        LexItem::Token(t) if *t == "}" => {
                            depth -= 1;
                            self.lexer.cont();
                            if depth == 0 {
                                break;
                            }
                        }
                        LexItem::None => break,
                        _ => self.lexer.cont(),
                    }
                }
            }
            if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                // a second variant name → branch 1 is a sequence (multi-element alt).
                kind = SliceGroupKind::Alt;
            } else if self.lexer.peek_token(")") {
                // `(A)` — the suffix after `)` decides: `?` optional, `*`/`+` repetition.
                self.lexer.cont();
                if self.lexer.peek_token("?") {
                    kind = SliceGroupKind::Alt;
                } else if self.lexer.peek_token("*") || self.lexer.peek_token("+") {
                    kind = SliceGroupKind::Repetition;
                }
            }
        }
        self.lexer.revert(save);
        kind
    }

    /// @PLN35 slice 1 — is the upcoming slice element a scalar type-annotated capture
    /// `name : Type` (optionally with a `*` / `+` repetition postfix)?  Returns
    /// `Some(is_repetition)` when the shape matches, else `None`.  ONE save/revert look-ahead —
    /// robust to the `peek_named_arg` that follows it now the `cont()` replay-buffer duplicate
    /// is fixed (see `lexer::test::link_revert_repeatable_same_region`).
    pub(super) fn peek_scalar_type_capture(&mut self) -> Option<bool> {
        if !matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
            return None;
        }
        let save = self.lexer.link();
        let mut res = None;
        self.lexer.cont(); // name
        if self.lexer.peek_token(":") {
            self.lexer.cont(); // `:`
            // A `_` after `:` is the wildcard sub-pattern (`name:_`), not a type — leave it to
            // the `name:pat` path.  A real type name identifies the scalar capture.
            if matches!(&self.lexer.peek().has, LexItem::Identifier(id) if id != "_") {
                self.lexer.cont(); // Type
                // A `{` after the name opens a plain-struct PATTERN (`p: Pt { x: 3 }`): that is
                // `name: pattern`, not a type annotation.
                if !self.lexer.peek_token("{") {
                    res = Some(self.lexer.peek_token("*") || self.lexer.peek_token("+"));
                }
            }
        }
        self.lexer.revert(save);
        res
    }

    /// A second variable-length part of a slice pattern is refused; its name is still declared
    /// (as the `vector` it would have bound) so the arm body parses without a cascade.
    pub(super) fn declare_refused_rest(&mut self, name: &str, elm_tp: &Type) {
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        let nr = self.pattern_binding(name, &vec_tp);
        self.vars.defined(nr);
    }
}
