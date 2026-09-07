// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Argument, DefType, Function, I32, Level, Parser, ToString, Type, Value, diagnostic_format,
    field_id, v_block, v_if, v_loop, v_set,
};
use crate::data::Deps;

/// The narrow-integer element KIND of a vector element type, with the `min` its ops take —
/// `None` for a wide (8-byte) element and for anything that is not an integer.
///
/// loft#1036 — the width comes from `byte_width` (through `vector_narrow_width`), the ONE
/// range→width home, exactly as the index READ (`get_val`) derives it.  Keying on
/// `forced_size` alone made an element declared `integer limit(10, 255)` answer `None`, so
/// its write fell back to a wide `OpSetInt` that never applied the `- min` ENCODE the 1-byte
/// `OpGetByte` read decodes with — every element read back exactly `lo` too high.
///
/// `narrow_vec` stays keyed on `forced_size`: it does not pick the WIDTH, it picks the
/// raw-vs-full ENCODING for a 2-byte element (`ShortRaw` for a `u16`-style alias,
/// `ShortFull` for a range that merely fits), and the storage side
/// (`Data::narrow_vector_content`) registers the matching Part.
fn narrow_elm_kind(elm_tp: &Type) -> Option<(crate::data::NarrowIntKind, i32)> {
    // A nullable narrow element (`vector<u8?>`) reserves a sentinel, so it needs the
    // nullable store op — a raw `OpSetByte` would write null's low byte `0`,
    // indistinguishable from the value 0.
    let (spec, nullable) = match elm_tp {
        Type::Integer(spec) => (*spec, false),
        Type::Optional(inner) => match &**inner {
            Type::Integer(spec) => (*spec, true),
            _ => return None,
        },
        _ => return None,
    };
    let narrow_vec = spec.forced_size.is_some() && spec.vector_narrow_width(nullable).is_some();
    let n = spec.vector_narrow_width(nullable)?;
    let kind = crate::data::NarrowIntKind::of(n, nullable, narrow_vec, spec.unsigned_wide());
    Some((kind, spec.usable_min(kind.reserves_sentinel())))
}

/// The store op that writes one NARROW-integer vector element — a `vector<u8>`, `<u16>`, or a
/// 4-byte `integer` subtype, nullable or not.  `None` for any other element type, leaving the
/// caller on its wide `set_field` / `OpSetInt` path.
///
/// Every site that BUILDS a vector routes its element write through here — the concrete
/// literal, append and slice through [`Parser::narrow_elm_set`], and a GENERIC body's
/// `out += [v]` through the monomorph rewrite (`primitive_setter_call`) — so the store op is
/// the exact twin of the index READ for each width and nullability: `OpSetByte` /
/// `OpSetShortRaw` / `OpSetInt4` for raw elements, `OpSetByteNullable` / `OpSetShort` for
/// nullable ones.  A site that misses it emits the wide 8-byte `OpSetInt` into a 1-byte
/// slot, so one write covers eight element slots — the slice half of #624, and the monomorph
/// half of loft#1378, whose setter keyed on the ALIAS def's `forced_size` and so wrote every
/// narrow element of a generic `vector<T>` eight bytes wide (`200 0` for two `u8`s).
pub(crate) fn narrow_elm_write(
    elm_tp: &Type,
    elm: Value,
    val: &Value,
    data: &crate::data::Data,
) -> Option<Value> {
    let (kind, min) = narrow_elm_kind(elm_tp)?;
    let d = data.def_nr(kind.set_op());
    if d == u32::MAX {
        return None;
    }
    let pos = Value::Int(0);
    Some(if kind.takes_min() {
        Value::Call(d, vec![elm, pos, Value::Int(min), val.clone()])
    } else {
        Value::Call(d, vec![elm, pos, val.clone()])
    })
}

/// The read twin of [`narrow_elm_write`]: the element slot `code` unpacked at the width and
/// encoding its type stores, for a GENERIC body's `v[i]` (`wrap_vector_get_val`).  `None` for
/// a wide element, which reads through `OpGetInt`.
pub(crate) fn narrow_elm_read(
    elm_tp: &Type,
    code: Value,
    data: &crate::data::Data,
) -> Option<Value> {
    let (kind, min) = narrow_elm_kind(elm_tp)?;
    let d = data.def_nr(kind.get_op());
    if d == u32::MAX {
        return None;
    }
    let pos = Value::Int(0);
    Some(if kind.takes_min() {
        Value::Call(d, vec![code, pos, Value::Int(min)])
    } else {
        Value::Call(d, vec![code, pos])
    })
}

// Lambda and vector expression parsing.

/// Which argument of an element read is its INDEX, for the ops that have one.
///
/// The two families differ in arity — `OpGetVector(r, size, index)` carries the element size
/// and `OpVectorRef(r, index)` does not — so the position is per op and not per family.
/// Reading it off the wrong one is not a wrong PLACE, because a missing argument declines,
/// but it is a place not named: the read then keeps pointing at the destination being built.
/// Each nullable peer reads at the same position as the op it mirrors.
fn element_index_arg(name: &str) -> Option<usize> {
    match name {
        "OpGetVector" | "OpGetVectorNullable" => Some(2),
        "OpVectorRef" | "OpVectorRefNullable" => Some(1),
        _ => None,
    }
}

/// One step of a PLACE — what `s.f`, `v[0]` and `v[i]` each contribute to the identity of the
/// location a build is assigned to, so *"is this read the destination?"* is an equality.
///
/// A nullable element read is the same place as a plain one, which is why both spellings
/// normalise to the element steps rather than carrying the op.  An element step is admitted
/// only for an index one statement cannot change — a constant, or a variable, since no
/// expression inside a single statement rewrites one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PlaceStep {
    Field(i32),
    ElemConst(i32),
    ElemVar(u16),
    Capture(i32),
}

impl Parser {
    /// The store op that writes one NARROW-integer vector element the concrete build sites
    /// emit — [`narrow_elm_write`] with the element in a local; see its doc for the contract.
    pub(crate) fn narrow_elm_set(&mut self, elm_tp: &Type, elm: u16, val: &Value) -> Option<Value> {
        narrow_elm_write(elm_tp, Value::Var(elm), val, &self.data)
    }

    /// Refuse a vector concatenation whose two sides store their INTEGER elements
    /// differently — a different width, or a different offset.
    ///
    /// `OpAppendVector` copies element BYTES: it is handed the destination's element
    /// type and never learns the source's, so it cannot re-encode.  That is right for
    /// every shape the type checker admits except this one: `u8` and `integer` are both
    /// "integer" to it, so `vector<u8> + vector<integer>` type-checked and then copied
    /// 8-byte elements into 1-byte slots — `[1,250] + [7,8]` answered `[1,250,7,0]`, and
    /// `vector<u8> + vector<i8>` answered `[1,250,123,133]` for `[-5,5]` because the two
    /// offsets differ.  Nothing reported it.
    ///
    /// Refusing rather than converting follows the scalar rule: `formal/types.md`
    /// (I-Narrow) makes a narrowing explicit (`as`), and every mixed concat is a
    /// narrowing in one direction or the other at the element level.  A literal append
    /// (`v += [9]`) is unaffected — the literal is already built in the destination's
    /// encoding — and so is any concat of two vectors of one type.
    fn refuse_mixed_element_encoding(&mut self, dest_tp: &Type, part_tp: &Type) {
        if self.first_pass {
            return;
        }
        let (Type::Vector(dest_c, _), Type::Vector(src_c, _)) = (dest_tp.base(), part_tp.base())
        else {
            return;
        };
        let int_of = |t: &Type| match t.base() {
            Type::Integer(spec) => Some((*spec, matches!(t, Type::Optional(_)))),
            _ => None,
        };
        let (Some((d_spec, d_null)), Some((s_spec, s_null))) = (int_of(dest_c), int_of(src_c))
        else {
            return;
        };
        let (d_w, s_w) = (d_spec.byte_width(d_null), s_spec.byte_width(s_null));
        // The offset only encodes anything at a narrow width; a wide element stores the
        // value raw, so two 8-byte specs of different ranges share one encoding.
        let offset_differs =
            d_w <= 4 && d_spec.part_min(d_w, d_null) != s_spec.part_min(s_w, s_null);
        if d_w == s_w && !offset_differs {
            return;
        }
        let how = if d_w == s_w {
            format!("both store their elements in {d_w} byte(s), but at different offsets")
        } else {
            format!("one stores its elements in {d_w} byte(s) and the other in {s_w}")
        };
        diagnostic!(
            self.lexer,
            Level::Error,
            "cannot concatenate `{}` with `{}` — {how}, and a concatenation copies element \
             BYTES, so the copied values would be wrong.  Append element by element \
             instead, which converts each value: `for x in <source> {{ <dest> += [x]; }}` \
             — with the checked cast inside the brackets if that step narrows \
             (`[(x as u8?) ?? 0]`)",
            dest_tp.source_name(&self.data),
            part_tp.source_name(&self.data),
        );
    }

    pub(crate) fn parse_append_vector(
        &mut self,
        code: &mut Value,
        tp: &Type,
        parts: &[(Value, Type)],
        orig_var: u16,
    ) -> Type {
        let mut ls = Vec::new();
        // Cluster I-d (@PLN85 cluster V / @PLN85 single-dep) — the store `orig_var` ends up
        // OWNING after this concat.  For `a = <call> + …` the first-operand adopt
        // (branch below) makes `a` hold the call's `["??"]` store, but `a`'s
        // pre-allocated `create_vector` backing is its current dep; the returned
        // type must carry the ADOPTED store's dep so `change_var` re-points `a` to
        // what it truly owns (else the orphaned backing leaks on escape — N).
        let mut adopt_dep: Option<u16> = None;
        let rec_tp = if let Type::Vector(cont, _) = tp {
            // @P314 — narrow-aware element type (see `append_elem_tp`).
            let cont = (**cont).clone();
            self.append_elem_tp(&cont)
        } else {
            i32::MIN
        };
        let var_nr = if orig_var == u16::MAX {
            let vec = self.create_unique("vec", tp);
            let elm_tp = tp.content();
            let db_ops = self.vector_db(&elm_tp, vec);
            // vector concat as an inline expression (not assigned to a variable)
            // creates a temporary with database allocation that corrupts the stack when
            // the result is used inside a compound assignment expression.
            // Emit an error so the user assigns to a variable first.
            if !db_ops.is_empty() && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "vector concatenation in an expression creates a temporary; \
                     assign to a variable first for correct results in compound expressions"
                );
            }
            for l in db_ops {
                ls.push(l);
            }
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(vec), code.clone(), Value::Int(rec_tp)],
            ));
            vec
        } else if let Value::Insert(elms) = code {
            for e in elms {
                ls.push(e.clone());
            }
            orig_var
        } else if matches!(self.vars.tp(orig_var), Type::RefVar(t) if matches!(**t, Type::Vector(_, _)))
        {
            // RefVar(Vector): append directly without an identity Set(v, Var(v)).
            // find_written_vars detects the write via the OpAppendVector in the parts loop.
            // The first operand `code` must BE the accumulator (`out = out + x`,
            // in-place grow); a DIFFERENT first operand (`out = a + x`) is a
            // REPLACEMENT.  A whole-value replacement IS expressible — `assign_refvar_vector`
            // lowers `out = a` to clear-and-refill of the shared store — but this path
            // parses the CONCAT, one operand at a time, and never learns whether the
            // statement was `=` or `+=`, so it cannot decide where the clear belongs.
            // Reject it and name the spelling that works; the old message told the author
            // the parameter "cannot be reassigned", which stopped being true (loft#772).
            if !self.first_pass && !matches!(code.unspan(), Value::Var(x) if *x == orig_var) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot build a concatenation directly into the `&` vector parameter \
                     '{0}'; assign the concatenation to a local first (`t = …; {0} = t;`) \
                     or append with `{0} += …`",
                    self.vars.name(orig_var)
                );
            }
            orig_var
        } else if matches!(code.unspan(), Value::Var(x) if *x != orig_var) {
            // The concat's first operand is a NAMED LOCAL other than the
            // accumulator (`v = a + b`, `u = v + [9]`).  Emitting `Set(v, Var(a))`
            // would repoint v's runtime slot at a's store — aliasing v to a,
            // mutating a when the parts append, AND orphaning the fresh
            // vector_db store create_vector allocated for v.  Deep-COPY a's
            // elements into v's own store instead (OpAppendVector deep-copies),
            // so a stays intact and v is independent.  The self-reference case
            // (`code == Var(orig_var)`, excluded by `x != orig_var`) keeps the
            // `Set(v, Var(v))` below so create_vector's self_ref / in-place path
            // still fires; a fresh-storage temp (Call/Block, not a Var) also
            // keeps `Set` so v adopts the temp's store with no extra copy.
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(orig_var), code.clone(), Value::Int(rec_tp)],
            ));
            orig_var
        } else {
            // `orig_var` ADOPTS a fresh-storage temp (`Set(v, Call/Block)`, no copy)
            // — or the self-ref `v = v + x` (code == Var(orig_var)).  For a CALL
            // adopt, record the call's hidden `["??"]` buffer as the store `v` now
            // owns so the return type below carries it (re-pointing `v` off its
            // orphaned create_vector backing).  Self-ref keeps its dep.
            if !self.first_pass && matches!(code.unspan(), Value::Call(_, _)) {
                adopt_dep = Self::collect_hidden_ref_args(code, &self.data)
                    .first()
                    .copied();
            }
            ls.push(v_set(orig_var, code.clone()));
            orig_var
        };
        for (val, part_tp) in parts {
            self.refuse_mixed_element_encoding(tp, part_tp);
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), val.clone(), Value::Int(rec_tp)],
            ));
        }
        if orig_var == u16::MAX {
            let res = self.vars.tp(var_nr).clone();
            ls.push(Value::Var(var_nr));
            *code = v_block(ls, res.clone(), "Append Vector");
            return res;
        }
        *code = Value::Insert(ls);
        match adopt_dep {
            Some(dep) => Type::Rewritten(Box::new(tp.depending(dep))),
            None => Type::Rewritten(Box::new(tp.clone())),
        }
    }

    pub(crate) fn parse_append_text(
        &mut self,
        code: &mut Value,
        tp: &Type,
        parts: &[(Value, Type)],
        orig_var: u16,
    ) -> Type {
        let mut ls = Vec::new();
        let var_nr = if orig_var == u16::MAX {
            let v = self.vars.work_text(&mut self.lexer);
            if matches!(self.vars.tp(v), Type::RefVar(_)) {
                ls.push(self.cl("OpClearStackText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendStackText", &[Value::Var(v), code.clone()]));
            } else if tp == &Type::Character {
                ls.push(self.cl("OpClearText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendCharacter", &[Value::Var(v), code.clone()]));
            } else {
                ls.push(self.cl("OpClearText", &[Value::Var(v)]));
                ls.push(self.cl("OpAppendText", &[Value::Var(v), code.clone()]));
            }
            v
        } else if matches!(self.vars.tp(orig_var), Type::RefVar(_)) {
            ls.push(self.cl("OpAppendStackText", &[Value::Var(orig_var), code.clone()]));
            orig_var
        } else {
            ls.push(self.cl("OpAppendText", &[Value::Var(orig_var), code.clone()]));
            orig_var
        };
        for (val, tp) in parts {
            // Unwrap `RefVar(inner)` for the type-dispatch check below.
            // A `&text` argument (parameter passed by reference) appears
            // as `Type::RefVar(Type::Text(_))` here but the OpAppend* ops
            // accept it directly via the same code path as plain `Text`.
            let dispatch_tp: &Type = if let Type::RefVar(inner) = tp {
                inner.as_ref()
            } else {
                tp
            };
            if matches!(self.vars.tp(var_nr), Type::RefVar(_)) {
                if *dispatch_tp == Type::Character {
                    ls.push(self.cl("OpAppendStackCharacter", &[Value::Var(var_nr), val.clone()]));
                } else if matches!(dispatch_tp, Type::Text(_)) {
                    ls.push(self.cl("OpAppendStackText", &[Value::Var(var_nr), val.clone()]));
                } else {
                    // @P274 — non-text/non-character parts (integer / float /
                    // bool / vector / reference / enum / …) need a format-
                    // dispatch step before append.  `OpAppendStackText`
                    // assumes its argument already evaluates to text on the
                    // stack; passing a raw `i64` from `headers.len()` is what
                    // tripped native E0614 (`type i64 cannot be dereferenced`)
                    // and SIGSEGV in interp.  Route through `append_data`,
                    // which is the same dispatch path used by `"…{x}…"`
                    // format-string interpolation and handles every formattable
                    // type via the matching `OpFormat*` op.
                    self.append_data(
                        dispatch_tp.clone(),
                        &mut ls,
                        var_nr,
                        u16::MAX,
                        val,
                        super::OUTPUT_DEFAULT,
                    );
                }
            } else if *dispatch_tp == Type::Character {
                ls.push(self.cl("OpAppendCharacter", &[Value::Var(var_nr), val.clone()]));
            } else if matches!(dispatch_tp, Type::Text(_)) {
                ls.push(self.cl("OpAppendText", &[Value::Var(var_nr), val.clone()]));
            } else {
                // @P274 — see RefVar branch above.
                self.append_data(
                    dispatch_tp.clone(),
                    &mut ls,
                    var_nr,
                    u16::MAX,
                    val,
                    super::OUTPUT_DEFAULT,
                );
            }
        }
        let tp = Type::Text(Deps::frame1(var_nr));
        if orig_var == u16::MAX || var_nr != orig_var {
            // A new work text was created (either no orig_var, or orig_var was a
            // Character variable) — wrap in a Block so the work text appears on the stack.
            ls.push(Value::Var(var_nr));
            *code = v_block(ls, tp.clone(), "Add text");
            return tp;
        }
        *code = Value::Insert(ls);
        Type::Rewritten(Box::new(tp))
    }

    /// Rewrite boolean operators into an `IF` statement to prevent the calculation of the second
    /// expression when it is unneeded.
    pub(crate) fn boolean_operator(
        &mut self,
        code: &mut Value,
        tp: &Type,
        precedence: usize,
        is_or: bool,
    ) {
        // An operand of `&&`/`||` is READ as a boolean, not stored — @FR-N-Store admits it,
        // as `convert_condition` does for the same reading in an `if`.
        if !self.convert_admitting(code, tp, &Type::Boolean) && !self.first_pass {
            self.can_convert(tp, &Type::Boolean);
        }
        let mut second_code = Value::Null;
        let mut parent_tp = Type::Unknown(0);
        let second_pos = self.lexer.peek_pos().clone();
        let second_type = self.parse_operators(
            &Type::Unknown(0),
            &mut second_code,
            &mut parent_tp,
            precedence + 1,
        );
        self.known_var_or_type(&second_code, &second_pos);
        if !self.convert_admitting(&mut second_code, &second_type, &Type::Boolean)
            && !self.first_pass
        {
            self.can_convert(&second_type, &Type::Boolean);
        }
        // `&&`/`||` do not route through `call_op_as`, so its deferral counter cannot see an
        // operand pass 1 failed to type — and `handle_operator` publishes `Type::Boolean` for
        // this expression whatever the operands did, which ERASES the evidence for everything
        // upstream.  An operand still `Unknown` here is that evidence, and this is the only
        // place it exists.  Pass 2 re-parses and types it properly, so it matters only where
        // the source is parsed once; see `Parser::unresolved_types` (loft#1170).
        if self.first_pass && (tp.is_unknown() || second_type.is_unknown()) {
            self.unresolved_types = self.unresolved_types.saturating_add(1);
        }
        // Both operands of `&&`/`||` are TRUTHINESS positions, so the result is a definite
        // two-state boolean — C73 (`&&`/`||`/`!` coerce `null` to `false`), which is why the
        // caller types this expression the non-null `Type::Boolean`.  The left operand becomes
        // the `if` CONDITION below and a jump coerces it (`OpGotoFalse` tests `!= 1`); the
        // right operand becomes a branch VALUE, which nothing coerces.  `convert` does not
        // close that: it inserts a real conversion for every OTHER nullable type reaching a
        // boolean position (`integer?` picks up `OpConvBoolFromInt`, whose `!= i64::MIN` is
        // already 0/1), but `boolean?` to `boolean` shares a base type, so it converts to
        // nothing at all.  `b == true` is the definite-iser — C73's raw compare answers
        // `false` for the 255 sentinel and is measured identical on both backends — and it is
        // applied to the one operand the jump never sees.  @FR-E-Truthy, the truthiness
        // exception to @FR-E-NullArg's contagion.
        // Not gated on `!first_pass`: a DEFAULT VALUE — a parameter's or a struct field's —
        // is parsed once, in pass 1, so a pass-2-only wrap left `fn f(b: boolean = t && m())`
        // answering null while every other position was fixed.
        //
        // `Type::Null` is the LITERAL spelling of the same operand (`t && null`), which
        // `convert` turns into `OpConvBoolFromNull` — the 255 sentinel — and then hands on
        // unchanged.  It reaches this position the same way and must answer the same
        // `false`; only the static type differs.  Every OTHER nullable type is already
        // definite by the time it arrives (`integer?` through `OpConvBoolFromInt`), so
        // these two are the whole domain.
        if matches!(&second_type, Type::Optional(inner) if **inner == Type::Boolean)
            || second_type == Type::Null
        {
            second_code = self.cl("OpEqBool", &[second_code, Value::Boolean(true)]);
        }
        *code = v_if(
            code.clone(),
            if is_or {
                Value::Boolean(true)
            } else {
                second_code.clone()
            },
            if is_or {
                second_code
            } else {
                Value::Boolean(false)
            },
        );
    }

    // <single> ::= '!' <expression> |
    //              '(' <expression> ')' |
    //              <vector> |
    //              'if' <if> |
    //              <identifier:var> |
    //              <number> | <float> | <cstring> |
    //              'true' | 'false' | 'null'
    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_single(
        &mut self,
        var_tp: &Type,
        val: &mut Value,
        parent_tp: &mut Type,
    ) -> Type {
        // The reserved-but-unbuilt name, in VALUE position.  The statement-position twin
        // lives in `parse_assign_op_inner`'s keyword chain; both are needed because a
        // keyword token reaches neither an identifier lookup nor a call, so every position
        // that wanted a value reported a missing `;` instead (loft#1167).
        if self.lexer.peek_token("debug_assert") {
            self.lexer.has_token("debug_assert");
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`debug_assert` is reserved for a future release and does nothing yet — \
                     use `assert(…)`, which is checked in every build"
                );
            }
            if self.lexer.has_token("(") {
                while !self.lexer.peek_token(")") && !self.lexer.peek_token(";") {
                    let mut arg = Value::Null;
                    self.expression(&mut arg);
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.has_token(")");
            }
            return Type::Void;
        }
        if self.lexer.has_token("!") {
            let operand_pos = self.lexer.peek_pos().clone();
            // The operand is a SUB-expression, so the assignment's destination hint does not
            // reach it (loft#1304 — see `Parser::prefix_operand`).
            let outer_prefix = std::mem::replace(&mut self.prefix_operand, true);
            let t = self.parse_part(var_tp, val, parent_tp);
            self.prefix_operand = outer_prefix;
            // A unary prefix operator must validate its operand like a binary
            // one does, else an undefined name (a pass-1 placeholder Var with no
            // slot) reaches codegen and panics instead of a clean "Unknown
            // variable" diagnostic (@PLN53 F1-1).
            self.known_var_or_type(val, &operand_pos);
            // #253: `!x` on a non-boolean reads as "is x null?" — the null
            // sentinel is in-band (LOFT.md "!value asymmetry").  On a `not null`
            // operand the value can never BE the sentinel, so `!x` is *always
            // false* — a silent no-op (`f = 0; if !f` never runs, because 0 is a
            // real value, not null).  Warn, don't error: a nullable operand
            // (`!both` in stdlib min/max) is a legitimate null test, and boolean
            // `!` is ordinary negation (`false` is a valid value there).
            let eff = if let Type::RefVar(inner) = &t {
                (**inner).clone()
            } else {
                t.clone()
            };
            let operand_not_null =
                self.expr_not_null || matches!(&eff, Type::Integer(spec) if spec.not_null);
            if !self.first_pass
                && operand_not_null
                && !matches!(eff, Type::Boolean | Type::Null | Type::Unknown(_))
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "redundant-null-negation",
                    "'!' on a 'not null' {} is always false — '!x' tests whether x \
                     is null, and a 'not null' value is never null",
                    t.name(&self.data)
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "compare the VALUE instead (`x == 0`)".to_string(),
                    condition: Some(
                        "you meant to test what the value IS, not whether it is present"
                            .to_string(),
                    ),
                    edit: None,
                    concept: "nullable values",
                    concept_ref: "@F1",
                });
            }
            let arg = val.clone();
            // LOFT.md § Conversions lists `!v` beside `if` / `while` / `assert` for the
            // any-type coercion, and the comment above states what `!x` means on a
            // non-boolean: *"is x null?"*.  A heap handle has no `Not` operator, so
            // without this the documented spelling was refused — `!v` on a vector read
            // *"No matching operator Not on vector<integer>"* while `if v` compiled.
            // Routed through the ONE condition coercion, so the two cannot part ways.
            if Self::is_heap_handle(&t) {
                let mut present = arg;
                self.convert_condition(&mut present, &t);
                *val = self.cl("OpNot", &[present]);
                return Type::Boolean;
            }
            self.call_op(val, "Not", &[arg], &[t])
        } else if self.lexer.has_token("~") {
            let operand_pos = self.lexer.peek_pos().clone();
            // The operand is a SUB-expression, so the assignment's destination hint does not
            // reach it (loft#1304 — see `Parser::prefix_operand`).
            let outer_prefix = std::mem::replace(&mut self.prefix_operand, true);
            let t = self.parse_part(var_tp, val, parent_tp);
            self.prefix_operand = outer_prefix;
            self.known_var_or_type(val, &operand_pos); // @PLN53 F1-1 (see `!` above)
            let arg = val.clone();
            self.call_op(val, "BitNot", &[arg], &[t])
        } else if self.lexer.has_token("-") {
            let operand_pos = self.lexer.peek_pos().clone();
            // The operand is a SUB-expression, so the assignment's destination hint does not
            // reach it (loft#1304 — see `Parser::prefix_operand`).
            let outer_prefix = std::mem::replace(&mut self.prefix_operand, true);
            let t = self.parse_part(var_tp, val, parent_tp);
            self.prefix_operand = outer_prefix;
            self.known_var_or_type(val, &operand_pos); // @PLN53 F1-1 (see `!` above)
            // @PLN102 pre-freeze — the leading `-` binds tighter than `**` (loft's uniform
            // rule: a unary prefix binds tighter than any binary op — the `-` is the sign of
            // its operand).  So `-2 ** 2` is `(-2) ** 2` = 4, NOT `-(2 ** 2)` = -4 as in
            // Python/maths (which treat `-` as a weaker OPERATOR).  For a *literal* base this
            // matches the intuition "-2 IS a number" and is unsurprising, so stay silent.
            // We warn only when the base is NOT a literal (`-x ** y`, `-f() ** y`), where the
            // `-` reads as an operator on a subexpression and the grouping is a genuine
            // footgun.  The grammar rule itself stays uniform (no special-case `**`
            // precedence).  `(-x) ** y` (the `-` parses inside the paren primary) and
            // `-(x ** y)` (the operand is a paren primary, so the next token is `)`/`;`, not
            // `**`) never reach here.
            let base_is_literal = matches!(
                val.unspan(),
                Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_)
            );
            if !self.first_pass && !base_is_literal && self.lexer.peek_token("**") {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "unary-minus-binds-tighter",
                    "`-x ** y` parses as `(-x) ** y` — the leading `-` binds to `x` as a sign \
                     (tighter than `**`), not `-(x ** y)`"
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "parenthesise as `-(x ** y)`".to_string(),
                    condition: Some("you meant to negate the POWER, not the base".to_string()),
                    edit: None,
                    concept: "operators",
                    concept_ref: "@F37",
                });
            }
            let arg = val.clone();
            self.call_op(val, "Min", &[arg], &[t])
        } else if self.lexer.has_token("(") {
            // loft#1067 — a tuple MEMBER's declared type is an expected type like any
            // other, so `t: (fn(integer) -> integer, integer) = (|x| { x * 2 }, 1)` can
            // say what `x` is.  Held back until loft#1069, because a `fn(…)` in a tuple
            // could not be CALLED back out of one whatever spelling put it there — so
            // threading the type here would have replaced a clean type error with a panic.
            //
            // Only the members on the way to a `fn(…)` seed anything: this deliberately
            // does not thread member types in general, which is a wider question about
            // tuple literal typing (loft#942/#943) and not this rule.
            //
            // The destination arrives on either channel.  A top-level literal is checked
            // against the declaration in `var_tp`; a NESTED one re-enters here through
            // `expression`, which starts its own `var_tp` from `Unknown`, so the member's
            // declared type comes on `⇐` instead.  Reading only `var_tp` is what made the
            // seeding per TOP-LEVEL member: `(fn(integer) -> integer, integer)` inferred
            // `|x|` while `((fn(integer) -> integer, integer), text)` did not (loft#1073).
            let tuple_members: Vec<Type> = match var_tp.base() {
                Type::Tuple(ms) => ms.clone(),
                _ => match self.expected.base() {
                    Type::Tuple(ms) => ms.clone(),
                    _ => Vec::new(),
                },
            };
            // Touch the `⇐` channel ONLY when this destination actually names a `fn(…)`
            // member.  A `(` also opens an ordinary parenthesised expression, and that
            // expression INHERITS the ambient expectation — clearing it unconditionally
            // silently retyped one (`115-snapshot-roundtrip` went from a text build to
            // "No matching operator '&' on 'text' and 'integer'").
            let seeding = tuple_members.iter().any(Self::seeds_tuple_member_hint);
            let saved_expected = if seeding {
                std::mem::replace(&mut self.expected, Type::Unknown(0))
            } else {
                Type::Unknown(0)
            };
            // Member 0 is parsed before the `,` proves this is a tuple at all.  Seeding it
            // is still safe: a parenthesised NON-tuple expression checked against a tuple
            // type is already an error, and only a `fn(…)`-typed member seeds.
            if seeding
                && tuple_members
                    .first()
                    .is_some_and(Self::seeds_tuple_member_hint)
            {
                self.expected = tuple_members[0].base().clone();
            }
            // An assignment's destination variable is the accumulator a heap-building RHS
            // adopts (#501's watermark reuse, and `parse_append_vector`'s `orig_var`).  A
            // parenthesised expression IS that whole value, so adopting is right there —
            // but a tuple MEMBER is not, and member 0 is the one parsed before the `,` can
            // say which this is.  Adopting it typed the destination as the MEMBER
            // (`t = ([10, 20], 9)` → "Variable 't' cannot change type from vector<integer>
            // to (vector<integer>, integer)", refusing a legal program) and, where the
            // member built through the append path instead, left the tuple element reading
            // null with no diagnostic at all (`t = (x + y, 9)`).  Ask the lexer first and
            // give a member its own temp; the built value returns on the normal channel, so
            // everything downstream is unchanged.
            let divert = matches!(val, Value::Var(_)) && self.lexer.peek_tuple_literal();
            let mut member0 = Value::Null;
            let t = if divert {
                let t = self.expression(&mut member0);
                *val = member0;
                t
            } else {
                self.expression(val)
            };
            if seeding {
                self.expected = Type::Unknown(0);
            }
            if self.lexer.has_token(",") {
                // T1.2: Tuple literal — (expr, expr, ...)
                //
                // A struct-literal member arrives as `Rewritten(Reference(S))` (#319).
                // That wrapper is a parse-internal marker saying the value was built in
                // place, not a type a member can HAVE, and every consumer that matches on
                // the constructor misses it: `set_field` refused "Cannot assign to field
                // '_0' of type S", `get_val` refused "Field access not supported on type
                // S", and a bare `t = (S { … }, k)` reached codegen as the internal error
                // "emit_tuple_put_ops: unsupported elem Rewritten(…)".  `parse_vector` and
                // `parse_vector_for` already peel it from a vector's ELEMENT type for the
                // same reason; a tuple member is the same fact one level in (loft#943).
                let mut values = vec![val.clone()];
                let mut types = vec![t.unrewritten()];
                loop {
                    if self.lexer.peek_token(")") {
                        break;
                    }
                    let mut v = Value::Null;
                    if seeding
                        && tuple_members
                            .get(values.len())
                            .is_some_and(Self::seeds_tuple_member_hint)
                    {
                        self.expected = tuple_members[values.len()].base().clone();
                    }
                    let t2 = self.expression(&mut v);
                    if seeding {
                        self.expected = Type::Unknown(0);
                    }
                    values.push(v);
                    types.push(t2.unrewritten());
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                if seeding {
                    self.expected = saved_expected;
                }
                self.lexer.token(")");
                if types.len() < 2 {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple literals require at least 2 elements"
                    );
                }
                // loft#1102 — a heap member is COPIED into the tuple, like the two sibling
                // constructors.  `@FR-T-Cons` said nothing about ownership, and the code
                // stored the local's handle, so `t = (vl, 9); vl[0] = 41` changed `t.0` —
                // while `S { v: vl }` and `[vl]` both answered the original.  `@FR-B-Copy`
                // is the contract a plain bind already has (*"the bound variable is
                // INDEPENDENT, and mutating it does NOT reach the source"*), and a
                // constructor handing a value to a new name is that same step.
                //
                // NOT when this tuple is an assignment TARGET.  A destructure's left side
                // (`(ca, cb) = t`) and a tuple-place write (`(s.a, s.b) = t`) are parsed by
                // this same branch and are then read back as a list of bare `Var`s; rewriting
                // a target into a copy block leaves that list EMPTY, which surfaces as
                // "Tuple arity mismatch: left has 0 names".  Only a tuple being CONSTRUCTED
                // as a value is a construction.
                if !self.lexer.peek_token("=") {
                    for (i, v) in values.iter_mut().enumerate() {
                        if let Some(owned) = self.tuple_member_owned_copy(v, &types[i]) {
                            types[i] = owned;
                        }
                    }
                }
                *val = Value::Tuple(values);
                Type::Tuple(types)
            } else {
                if seeding {
                    self.expected = saved_expected;
                }
                self.lexer.token(")");
                // @P395 — a parenthesised vector concat consumed by a trailing
                // `.method()` reused the OUTER assignment LHS as its accumulator
                // (the inner concat inherited `orig_var` from the live `val`),
                // lowering to a void `Value::Insert` that leaves no stack value;
                // the method then reads a misaligned slot → garbage value / the
                // `codegen.rs:2669` +8 drift.  This is the same shape the P103
                // guard in `parse_append_vector` already rejects for
                // `f([1,2] + [3])` (tests/scripts/102-expected-errors.loft):
                // inline vector concat consumed by a call/sub-expression must be
                // assigned to a variable first.  Fire the same clean error here
                // instead of silently corrupting.  Direct assignment `v = (a+c)`
                // (no trailing `.`) keeps its reuse path; `(a+c)[i]` indexing
                // already lowers correctly and is not guarded.
                if !self.first_pass && matches!(val, Value::Insert(_)) && self.lexer.peek_token(".")
                {
                    let tv = if let Type::Rewritten(inner) = &t {
                        (**inner).clone()
                    } else {
                        t.clone()
                    };
                    if matches!(tv, Type::Vector(_, _)) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "vector concatenation in an expression creates a temporary; \
                             assign to a variable first for correct results in compound expressions"
                        );
                    }
                }
                t
            }
        } else if self.lexer.peek_token("{") {
            self.parse_block("block", val, &Type::Unknown(0))
        } else if self.lexer.has_token("[") {
            // #432 — a bare vector literal in call-argument position arrives with
            // `var_tp` unknown (the parameter type is dropped by `expression`).
            // Build it at the parameter's element width via `vector_hint`, so a
            // `vector<u8>` parameter gets a 1-byte-stride literal instead of a
            // `vector<integer>` (8-byte) the callee silently reinterprets.  A
            // typed context already carries the type in `var_tp` and wins.  Take
            // (clear) the hint so it seeds only this outermost literal — nested
            // literals get their element type threaded through `var_tp`.
            // loft#699 — `vector_hint` answers "may the expected type OVERRIDE what the
            // elements infer?", and #432 answers that narrowly on purpose (a `vector<u8>`
            // parameter must win over the `vector<integer>` a bare `[10, 255]` infers,
            // while a generic or `single` element must not).  An EMPTY literal has no
            // elements to infer from, so there is nothing to override and nothing to get
            // wrong: the expected type is the only thing that can say what to build.
            // Without it `fn mk() -> vector<S> { [] }` typed as Unknown and folded to
            // `void`, so every heap element type but a narrow integer had no empty value
            // form at all — which is what a hoisted `= []` parameter default needs.
            // A KEYED expected type seeds whether or not the literal is empty (loft#703):
            // `[…]` infers `vector<K>`, which is not a narrower or wider version of
            // `hash<K[k]>` but a different container — so there is no inference to
            // override, only the one answer.
            let hint = if is_collection(&self.expected)
                && (self.lexer.peek_token("]") || is_keyed(&self.expected))
            {
                self.expected.without_deps()
            } else {
                self.vector_hint()
            };
            self.expected = Type::Unknown(0);
            // #501 — a vector literal parsed as an assignment RHS reuses the LHS var
            // (`val`) as its build accumulator (the watermark optimisation for
            // `v = [..]`).  Remember it, so that after parsing we can detect a trailing
            // `.method(..)` chain that makes the literal a RECEIVER — where that reuse
            // is wrong (the chain's result, assigned to the same LHS, would be
            // discarded → interpret #306 / native E0425).
            let orig_lhs = if let Value::Var(n) = val {
                Some(*n)
            } else {
                None
            };
            // loft#945 — a literal that pass 1 already found to be a RECEIVER takes neither
            // the reuse nor the LHS's type.  The variable table survives the pass boundary,
            // and after the chain the LHS holds the CHAIN's type, not the literal's:
            // `total = [1, 2, 3, 4].reduce(0, …)` leaves an integer, and
            // `d = [1, 2, 3].map(|x| { "n{x}" })` leaves a `vector<text>`.  Pass 2 then built
            // the literal against that and refused it — "cannot change type from integer to
            // vector<integer>", or "cannot store integer elements in a vector<text>".  Below
            // is where the chain is recognised and recorded; here is where the next pass
            // acts on it.  Building into a fresh accumulator is what the chain case does
            // anyway, so this only brings pass 2 forward to the same decision.
            let known_receiver =
                orig_lhs.is_some_and(|n| self.literal_chain_lhs.contains(&(self.context, n)));
            if known_receiver {
                *val = Value::Null;
            }
            let orig_lhs = if known_receiver { None } else { orig_lhs };
            let seeded;
            let unseeded = Type::Unknown(0);
            let elem_tp = if known_receiver {
                &unseeded
            } else if var_tp.is_unknown() && is_collection(&hint) {
                seeded = hint;
                &seeded
            } else {
                var_tp
            };
            let t = self.parse_vector(elem_tp, val, parent_tp);
            // The literal is now fully parsed (a safe point to peek — no lexer
            // backtrack).  If it reused the LHS var AND a `.method(..)` chain follows
            // (`[1,2,3].map(..)`), rename the accumulator to a fresh synthetic local so
            // the LHS is free to receive the chain's result, and wrap the (now void,
            // in-place) build so it YIELDS that local — making a literal receiver behave
            // exactly like a variable one.  Scoped to a `.` method chain: `.map` /
            // `.filter` / `.reduce` route the receiver through `parse_vector_method`,
            // and the map/filter cases keep the vector's element type so the LHS's
            // parsed type stays valid across passes.  (A trailing `[i]` index yields a
            // SCALAR, so the LHS's parsed vector type would clash with the index result
            // on the second pass — that rarer form keeps its existing clean "cannot
            // change type" diagnostic.)  Runs in both passes so `create_unique`
            // numbering stays aligned.
            if let Some(lhs) = orig_lhs
                && self.lexer.peek_token(".")
            {
                // loft#945 — record it, so the NEXT pass knows this literal is a receiver
                // before it starts building (see `known_receiver` above).  By the end of
                // this statement the LHS holds the CHAIN's type, which is not a type the
                // literal can be built against.
                self.literal_chain_lhs.insert((self.context, lhs));
                // Inherit the LHS's parsed vector type — it carries the `["__vdb_N"]`
                // borrow dep on the literal's backing store, so the chain BORROWS the
                // receiver and the backing is freed once at scope exit (matching a
                // variable receiver) instead of being double-owned (a store leak).
                let recv_tp = self.vars.tp(lhs).clone();
                let recv = self.create_unique("vec", &recv_tp);
                self.vars.defined(recv);
                // The renamed build immediately assigns `recv` (`recv = OpGetField(__vdb,0)`),
                // so its null-init must be a NON-allocating sentinel — otherwise the eager
                // `OpInitRef` store is orphaned when the build reassigns it (a 1-store leak).
                self.vars.mark_inline_ref(recv);
                crate::parser::collections::rename_var(val, lhs, recv);
                // The literal poisoned the LHS's inferred type (it typed it as the
                // vector); clear it so the outer assignment re-infers from the CHAIN
                // result.
                self.vars.set_type(lhs, Type::Unknown(0));
                let build = std::mem::replace(val, Value::Null);
                *val = v_block(vec![build, Value::Var(recv)], recv_tp.clone(), "Vector");
                recv_tp
            } else {
                t
            }
        } else if self.lexer.has_token("if") {
            self.parse_if(val)
        } else if self.lexer.has_token("match") {
            self.parse_match(val)
        } else if self.lexer.has_token("fn") {
            if self.lexer.peek_token("(") {
                self.parse_lambda(val)
            } else {
                // function references use the bare name, not 'fn name'.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Use the function name directly, without 'fn' prefix"
                );
                self.parse_fn_ref(val)
            }
        } else if self.lexer.has_token("||") {
            // Zero-parameter short lambda: || { body } — `||` already consumed, no closing `|`
            self.parse_lambda_short(val, false)
        } else if self.lexer.has_token("|") {
            // Short lambda with parameters: |x: T, …| { body } — opening `|` consumed
            self.parse_lambda_short(val, true)
        } else if self.lexer.has_token("sizeof") {
            self.lexer.token("(");
            self.parse_size(val)
        } else if self.lexer.has_token("type_name") {
            self.lexer.token("(");
            self.parse_type_name(val)
        } else if self.lexer.has_token("assert") {
            self.lexer.token("(");
            self.parse_intrinsic_call(val, "assert")
        } else if self.lexer.has_token("panic") {
            self.lexer.token("(");
            self.parse_intrinsic_call(val, "panic")
        } else if let Some((name, name_pos)) = self.lexer.has_identifier_pos() {
            // @PLN22 Phase 1 — when the receiver context (`parent_tp`) is not
            // itself an enum, supply the operand's expected enum so a bare
            // value-position variant resolves against it (variants are not in the
            // flat namespace).  `var_tp` carries the enum for typed-local decls,
            // typed reassignment, `==`, and struct-field init; `enum_hint` carries
            // it for call args / return body.  This only feeds parse_var's
            // last-resort variant branch — a name that resolves as a variable,
            // field, function, or `$` is handled by an earlier branch unaffected.
            if !self.enum_context(parent_tp) {
                if self.enum_context(var_tp) {
                    *parent_tp = var_tp.clone();
                } else if self.enum_context(&self.enum_hint()) {
                    *parent_tp = self.enum_hint();
                }
            }
            self.parse_var(val, &name, parent_tp, &name_pos)
        } else if self.lexer.peek_token("$") {
            let name_pos = self.lexer.peek_pos().clone();
            self.lexer.has_token("$");
            self.parse_var(val, "$", parent_tp, &name_pos)
        } else if let Some(nr) = self.lexer.has_integer() {
            *val = Value::Int(nr as i32);
            I32.clone()
        } else if let Some(nr) = self.lexer.has_long() {
            *val = Value::Long(nr as i64);
            crate::data::I64.clone()
        } else if let Some(nr) = self.lexer.has_float() {
            *val = Value::Float(nr);
            Type::Float
        } else if let Some(nr) = self.lexer.has_single() {
            *val = Value::Single(nr);
            Type::Single
        } else if let Some(s) = self.lexer.has_cstring() {
            // @PLN124 — does this string BUILD a value rather than render text?
            // `var_tp` carries the target for a typed-local decl, a typed
            // reassignment and a struct-field init; `expected` carries it for a
            // call argument and a return body — the same two-source rule the
            // bare-variant branch above uses, for the same reason.
            let mut target = self.interpolation_target(var_tp);
            // Not inside a HOLE: a hole is not the destination, so a string literal
            // in one does not inherit the destination's type.  Without this,
            // `q: SqlText = "{"seed"}"` checked the inner literal against `SqlText`
            // and it took the BUILD path, so a string that was plainly the author's
            // value came back as a second accumulator.  Only the `expected` source
            // is gated — `var_tp` is the type of a declaration written inside the
            // hole, which does name a destination — and only this reading of the
            // channel, because `expected` carries facts a hole legitimately needs
            // (a keyed lookup resolves its record type through it).
            if target == u32::MAX && !self.in_format_expr {
                target = self.interpolation_target(&self.expected);
            }
            // A string literal written INSIDE a `{…}` interpolates like any
            // other, and `parse_string` decides that from the mode — which by
            // now describes where the LEXER is, not this string: the enclosing
            // loop set `Code` to read its own expression after this literal was
            // already scanned. Ask the lexer whether THIS string still has a
            // hole open instead (loft#767).
            if self.lexer.nested_hole_open() {
                self.lexer.set_mode(crate::lexer::Mode::Formatting);
            }
            self.parse_string(val, &s, target)
        } else if let Some(nr) = self.lexer.has_char() {
            *val = self.cl("OpConvCharacterFromInt", &[Value::Int(nr as i32)]);
            Type::Character
        } else if self.lexer.has_token("true") {
            *val = Value::Boolean(true);
            Type::Boolean
        } else if self.lexer.has_token("false") {
            *val = Value::Boolean(false);
            Type::Boolean
        } else if self.lexer.has_token("null") {
            *val = Value::Null;
            Type::Null
        } else {
            Type::Unknown(0)
        }
    }

    // <fn-ref> ::= 'fn' <identifier>
    // Produces a Type::Function value whose runtime representation is the
    // definition number (d_nr) of the named function stored as an i32.
    pub(crate) fn parse_fn_ref(&mut self, code: &mut Value) -> Type {
        let Some(name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expect function name after fn");
            }
            return Type::Unknown(0);
        };
        // Try user function (n_<name>) first, then fall back to bare name.
        let d_nr = {
            let prefixed = format!("n_{name}");
            let nr = self.data.def_nr(&prefixed);
            if nr == u32::MAX {
                self.data.def_nr(&name)
            } else {
                nr
            }
        };
        if d_nr == u32::MAX {
            if !self.first_pass {
                if let Some(s) = self.suggest_function_name(&name) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown function '{name}' — did you mean '{s}'?"
                    );
                } else {
                    diagnostic!(self.lexer, Level::Error, "Unknown function '{name}'");
                }
            }
            return Type::Unknown(0);
        }
        if !self.first_pass && !matches!(self.data.def_type(d_nr), DefType::Function) {
            diagnostic!(self.lexer, Level::Error, "'{name}' is not a function");
            return Type::Unknown(0);
        }
        *code = Value::Int(d_nr as i32);
        self.data.def_used(d_nr);
        let n_args = self.data.attributes(d_nr);
        let arg_types: Vec<Type> = (0..n_args).map(|a| self.data.attr_type(d_nr, a)).collect();
        let ret_type = self.published_ret_type(d_nr, self.data.def(d_nr).returned().clone());
        Type::Function(arg_types, Box::new(ret_type), Deps::none())
    }

    // <lambda> ::= 'fn' '(' [<params>] ')' ['->' <type>] '{' <body> '}'
    // Produces Type::Function; runtime representation is d_nr as i32, same as fn-ref.
    // @F22 — closures & lambdas (value capture, cross-scope)
    /// @PLN86 D-cap-2 — a lambda created inside a sandboxed def is itself restricted:
    /// mark its def sandboxed under the enclosing def's profile so the admission walk
    /// DESCENDS into its body (checks its calls / fn-refs / raw-writes precisely) instead
    /// of treating it as an untagged leaf and rejecting every sandboxed lambda wholesale.
    /// Nested lambdas inherit transitively (each reads its immediate enclosing context's
    /// entry). A captured host fn-ref is still gated at its creation site in the enclosing
    /// def (the `cap = host_fn` Set that `referenced_defs` records). No-op outside a sandbox
    /// (`def_sandbox` is empty). Re-derived each pass, so verdicts stay pass-stable.
    fn mark_lambda_sandboxed(&mut self, enclosing: u32, lambda: u32) {
        if let Some(profile) = self.def_sandbox.get(&enclosing).cloned() {
            self.def_sandbox.insert(lambda, profile);
        }
    }

    /// How THIS scope names a capture it holds only because a nested lambda asked for it — a
    /// read of its own closure record — or `None` when it holds no such thing.
    ///
    /// The scope that BUILDS a closure record fills it from what it can name, and a relayed
    /// capture is not a local here: it arrived through this lambda's own `__closure`
    /// (loft#1236).  Pass 1 answers `None` (no record is synthesised yet) and emits nothing,
    /// which costs nothing — pass 1's IR is rebuilt in pass 2.
    pub(crate) fn relayed_capture_read(&mut self, name: &str) -> Option<Value> {
        if self.first_pass || self.closure_param == u16::MAX {
            return None;
        }
        let rec = self.data.def(self.context).closure_record();
        if rec == u32::MAX {
            return None;
        }
        let fnr = self.data.attr(rec, name);
        if fnr == usize::MAX {
            return None;
        }
        Some(self.get_field(rec, fnr, Value::Var(self.closure_param)))
    }

    /// The record and attribute index THIS scope names a relayed capture by, or `None` when it
    /// holds no such thing.  The write-side companion to [`Self::relayed_capture_read`].
    ///
    /// A by-VALUE capture is observed by copying the closure record's field back to the outer
    /// binding after the call.  For a lambda nested in a lambda the binding is not a variable
    /// of this scope at all, and the write-back was skipped — silently, so a nullable scalar
    /// (which is captured by value rather than boxed) accumulated nothing: `n? += x` four times
    /// read 0, on both backends.
    ///
    /// Answers the PLACE rather than emitting the write, because the value to write is not free
    /// to build: a caller that emits its side first and discards it on a `None` changes what
    /// other functions compile to — measured, as a native-only silent exit 1 in a script with no
    /// lambda in it.
    pub(crate) fn relayed_capture_attr(&mut self, name: &str) -> Option<(u32, usize)> {
        if self.first_pass || self.closure_param == u16::MAX {
            return None;
        }
        let rec = self.data.def(self.context).closure_record();
        if rec == u32::MAX {
            return None;
        }
        let fnr = self.data.attr(rec, name);
        if fnr == usize::MAX {
            return None;
        }
        Some((rec, fnr))
    }

    /// Install the capture scope a lambda body is parsed in, and return the enclosing one.
    ///
    /// The scope is the enclosing function's variables PLUS, when that function is itself a
    /// lambda, everything IT can see — a lambda nested in a lambda names the grandparent's
    /// locals just as the outer one does.  Without the second half the inner body could not
    /// RESOLVE such a name: where it happened to be a scalar or a collection the resolver made
    /// a fresh binding instead of refusing, so every write landed in a local that dies with the
    /// call and `total` read 0 where it owed 62 — silent, and `--native` would not compile it
    /// (loft#1236).
    ///
    /// The enclosing entries go in AFTER the enclosing function's own variables, so a name
    /// bound in the nearer scope shadows the further one.
    fn enter_capture_scope(
        &mut self,
        outer_vars: &Function,
        outer_context: u32,
    ) -> (Vec<(String, Type)>, std::collections::HashMap<String, u32>) {
        // @FR-L-CapRef — a `&T` parameter offers its POINTEE to a capture, so the peel belongs
        // HERE, at the
        // one place the capture context is built, rather than at each consumer.  `&` is a
        // channel to the CALLER's slot; the capture question — share or copy — is asked of
        // what it points at, and `(L-CapHeap)` / `(L-CapScalar)` then answer it exactly as
        // they do for the bare type.  Peeled at each reader instead, the storage half and
        // the reading half disagreed for a `&vector<τ>`: the body took the attribute's own
        // `Reference(elem)` type and `p += [9]` reported *"No matching operator 'Add' on
        // 'integer' and 'integer'"* — the ELEMENT's type, for an append to the collection
        // (loft#1276, the same shape loft#1209 had through `?`).
        let mut ctx: Vec<(String, Type)> = outer_vars
            .all_names_and_types()
            .into_iter()
            .map(|(n, t)| match t {
                Type::RefVar(inner) => (n, *inner),
                other => (n, other),
            })
            .collect();
        let mut owner: std::collections::HashMap<String, u32> = ctx
            .iter()
            .map(|(n, _)| (n.clone(), outer_context))
            .collect();
        for (name, tp) in &self.capture_context {
            if !ctx.iter().any(|(n, _)| n == name) {
                ctx.push((name.clone(), tp.clone()));
            }
            // Ownership stays where the BINDING is, even when the enclosing scope has a
            // variable of that name: a lambda that names a capture gets a placeholder in its
            // own table, and a placeholder is a way of reaching the binding rather than being
            // it.  Reading the table alone made the enclosing lambda the owner and boxed the
            // cell in the closure instead of in the frame that holds the variable.
            if let Some(d) = self.capture_owner.get(name) {
                owner.insert(name.clone(), *d);
            }
        }
        (
            std::mem::replace(&mut self.capture_context, ctx),
            std::mem::replace(&mut self.capture_owner, owner),
        )
    }

    /// Refuse a whole-value rebind, made inside a closure, of a captured heap PARAMETER
    /// (loft#1281, `D-clo-20`).
    ///
    /// `(F-ParamRebind)` makes such a rebind local to the callee, and the capture has no
    /// route back to the parameter's SLOT — which is the binding it would have to rebind.
    /// The closure record holds a COPY of the parameter's DbRef, so the write lands in the
    /// store that copy points at, and `(F-ParamHeap)` makes that store the CALLER's: the
    /// caller sees a replacement the rules say is local. Measured before this refusal:
    /// `fn f(p: vector<integer>) { g = fn() { p = [7,7]; }; g(); }` left the caller's
    /// `[1,2]` as `[7,7]`, on both backends, with nothing reported.
    ///
    /// Refusing rather than lowering it is the same call the `&`-scalar shape makes one
    /// rule over (DESIGN_DECISIONS C115 records both halves as one decided edge),
    /// and for the same reason: making it MEAN what it says needs the binding reachable
    /// from inside the closure plus a write-back, and the cell machinery that gives a
    /// mutated captured SCALAR exactly that cannot serve a heap value — reads in the
    /// enclosing body would see the cell while the caller still sees its own slot.
    ///
    /// Deliberately narrow, and each exclusion is a shape that works today:
    /// * a captured LOCAL — there is no caller for the rebind to reach past;
    /// * a `&` parameter — `(L-CapRef)` captures the pointee and the write-back to the
    ///   caller is the whole point of the annotation;
    /// * a scalar or text parameter — the cell machinery boxes it and the write reaches
    ///   the enclosing slot correctly;
    /// * every MUTATION-through — `p += [x]`, `p[i] = v`, `p.clear()` — which
    ///   `(L-CapHeap)` and `(F-ParamGrow)` say must reach the caller, and which do.
    fn reject_rebound_heap_parameter_captures(&mut self, lambda: u32) {
        let Some(names) = self.rebound_captures.remove(&lambda) else {
            return;
        };
        for name in names {
            let v_nr = self.vars.var(&name);
            if v_nr == u16::MAX || !self.vars.is_argument(v_nr) {
                continue;
            }
            let tp = self.vars.tp(v_nr).clone();
            if !matches!(
                tp.base(),
                Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Radix(_, _, _)
                    | Type::Trie(_, _, _)
                    | Type::Reference(_, _)
            ) {
                continue;
            }
            self.lexer.to(self.vars.var_source(v_nr));
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot replace the whole value of the captured parameter '{name}' from a \
closure — the closure shares the caller's storage, so the replacement would reach the \
caller, where a rebind of a parameter is local to this function. Change it in place if the \
caller should see it (`{name}.clear()`, `{name} += [...]`, `{name}[i] = ...`), \
or build a local and use that."
            );
        }
    }

    /// Take, in THIS scope, every capture the lambda just parsed reaches past it for.
    ///
    /// A closure record is filled by the scope that BUILDS it, from what that scope can name.
    /// An inner lambda capturing a grandparent's local therefore needs its enclosing lambda to
    /// have captured it as well — otherwise there is nothing here to fill the inner record
    /// from, and `emit_lambda_code` silently skips the field (loft#1236).
    ///
    /// Only names this scope does not already bind, and only ones its own capture scope offers:
    /// a name the inner lambda declared, or one that is a local here, relays nothing.
    fn relay_nested_captures(&mut self, inner: &[(String, Type)]) {
        for (name, _) in inner {
            if self.vars.var(name) != u16::MAX {
                continue;
            }
            let Some((_, ctype)) = self
                .capture_context
                .iter()
                .find(|(n, _)| n == name)
                .cloned()
            else {
                continue;
            };
            if !self.captured_names.iter().any(|(n, _)| n == name) {
                self.captured_names.push((name.clone(), ctype));
            }
        }
    }

    pub(crate) fn parse_lambda(&mut self, code: &mut Value) -> Type {
        let lambda_name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        let stored_name = format!("n_{lambda_name}");

        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(&lambda_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;
        // save outer scope variable names/types for capture detection.
        let (outer_capture, outer_owner) = self.enter_capture_scope(&outer_vars, outer_context);
        // clear captured_names so we collect only this lambda's captures.
        let outer_captured = std::mem::take(&mut self.captured_names);

        self.lexer.token("(");
        let mut arguments = Vec::new();
        self.parse_arguments(&lambda_name, &mut arguments);
        self.lexer.token(")");

        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, &lambda_name, &arguments)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            // …and the capture state, which the short form already restored here.  Left
            // standing, this lambda's `capture_context` is what the NEXT thing parsed sees as
            // its enclosing scope.
            self.capture_context = outer_capture;
            self.capture_owner = outer_owner;
            self.captured_names = outer_captured;
            return Type::Unknown(0);
        }
        let d_nr = self.context;
        self.mark_lambda_sandboxed(outer_context, d_nr);

        // Parse optional return type annotation.
        let result = if self.lexer.has_token("->") {
            self.parse_type_full(d_nr, true).unwrap_or(Type::Void)
        } else {
            Type::Void
        };
        // A lifetime-bearing tuple return is boxed exactly as a named function's is
        // (loft#1349) — see `Parser::boxed_tuple_return`.
        let result = self.boxed_tuple_return(result);
        if self.first_pass {
            self.data.set_returned(d_nr, result);
        }

        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        for (a_nr, a) in arguments.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
            }
        }
        // @PLN115 tail — record each `fn(e: T)` lambda parameter's DECLARATION (pass 2,
        // recording on), consuming the positions `parse_arguments` captured, now that
        // the lambda's def_nr is known.  Same arg-index == var_nr as a plain fn param.
        if !self.first_pass {
            for (idx, pos, len) in std::mem::take(&mut self.pending_param_positions) {
                self.record_decl(
                    &pos,
                    len,
                    crate::resolution::Resolution::Local {
                        fn_def: d_nr,
                        var_nr: idx,
                    },
                );
            }
        }

        // The codegen (line 40-46) reads definition attributes to assign argument positions.
        let outer_closure_param = self.closure_param;
        if !self.first_pass {
            let closure_rec = self.data.def(d_nr).closure_record();
            if closure_rec != u32::MAX {
                // #686 — repair any attribute pass 1 had to leave unresolved BEFORE the
                // body reads it for its type-check and storage shape.
                self.resolve_forward_captures(closure_rec);
                let closure_tp = Type::Reference(closure_rec, Deps::none());
                // Add as definition attribute so codegen positions it on the stack.
                self.data
                    .add_attribute(&mut self.lexer, d_nr, "__closure", closure_tp.clone());
                let v_nr = self.create_var("__closure", &closure_tp);
                self.vars.become_argument(v_nr);
                self.closure_param = v_nr;
            }
        }

        self.parse_code();
        self.closure_param = outer_closure_param;
        self.data.op_code(d_nr);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);

        // Plan-22 phase 02c (2026-05-12): collect mutations BEFORE
        // synthesize_closure_record so the synthesise step can
        // consult `data.def(d_nr).mutated_captures` to pick the
        // right attribute type (auto-Reference for mutated
        // Reference captures vs inline-bytes for everything else).
        // The collect helper accepts a missing closure_record and
        // derives captured names from the lambda's variable table
        // in that case.
        if !self.captured_names.is_empty() {
            collect_mutated_captures(&mut self.data, d_nr);
            // Plan-22 phase 02d-i (2026-05-12): accumulate scalar
            // mutated-captures onto the parent function's
            // `scalars_to_box` field.  Phase 02d-iii will use this
            // to rewrite outer bindings to hidden cells.  Detection-
            // only at this phase — no behavior change.
            accumulate_scalars_to_box(
                &mut self.data,
                outer_context,
                d_nr,
                &self.captured_names,
                &self.capture_owner,
            );
            // Plan-22 phase 02d-ii — ensure a `__cell_<T>` struct
            // exists for every scalar-typed mutated capture, so
            // 02d-iii's outer-binding rewrite has a target type to
            // allocate.  Idempotent across all lambdas in the
            // compilation; gated on first_pass.
            self.synthesize_cell_structs(d_nr);
            // Plan-22 phase 02d-iii.e — pre-box `captured_names`
            // entries for names in the parent's scalars_to_box
            // BEFORE `synthesize_closure_record` builds the
            // record's attributes.  Without this, pass 1 freezes
            // the attribute as the un-flipped scalar (Integer)
            // and the closure record's storage layout uses 8B
            // inline instead of 12B share-by-DbRef — runtime
            // then misroutes writes through `OpSetInt` and
            // crashes on the locked store.
            box_captured_names_for_outer_scalars(
                &mut self.captured_names,
                &self.data,
                outer_context,
                &self.capture_owner,
            );
            self.synthesize_closure_record(d_nr, &lambda_name);
            // #314: remember this lambda so the enclosing body's end
            // can reject shared-mutable-scalar captures once
            // `scalars_to_box` is complete.
            if self.first_pass {
                self.fn_lambdas.entry(outer_context).or_default().push(d_nr);
            }
        }
        let captured = std::mem::replace(&mut self.captured_names, outer_captured);

        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;
        self.capture_context = outer_capture;
        self.capture_owner = outer_owner;
        // The enclosing table is back, so a captured name can finally be asked what it is
        // bound to out here (loft#1281).
        self.reject_rebound_heap_parameter_captures(d_nr);
        self.relay_nested_captures(&captured);

        self.data.def_used(d_nr);

        // A lambda's own emit is the ONLY thing that may answer this question about it.
        // `emit_lambda_code` sets `last_closure_work_var` for a CAPTURING lambda and leaves
        // it alone for a non-capturing one, so a capturing lambda nested in the body just
        // parsed would otherwise still be the answer — and the assignment site would map
        // THIS fn-ref to a closure variable that lives in the inner lambda's table.  Native
        // then emits `var_??` for the closure argument and the program does not compile.
        // The named-function reset in `definitions.rs` states the same rule one scope out
        // (*"a lambda inside make_adder leaks last_closure_work_var into the next function
        // parsed"*); a lambda inside a lambda is the same leak within one body.
        self.last_closure_work_var = u16::MAX;
        self.emit_lambda_code(code, d_nr);

        // Build the user-visible function type from the declared arguments only.
        // Using data.attributes(d_nr) is wrong here: text_return() registers text work
        // variables (e.g. __work_1) as definition attributes for stack allocation, and
        // the second-pass closure injection also adds a hidden __closure attribute.
        // Neither should appear in the public Function type — only declared params do.
        let arg_types: Vec<Type> = arguments.iter().map(|a| a.typedef.clone()).collect();
        let ret_type = self.published_ret_type(d_nr, self.data.def(d_nr).returned().clone());
        // include the closure work var dep so that get_free_vars knows
        // a local ___clos_N variable owns the closure (and will free it).  Without
        // this dep, the Function arm in get_free_vars would emit a duplicate free.
        let dep = if self.last_closure_work_var == u16::MAX {
            Deps::none()
        } else {
            Deps::frame1(self.last_closure_work_var)
        };
        Type::Function(arg_types, Box::new(ret_type), dep)
    }

    // <short-lambda> ::= '||' ['->' type] block              (expect_close=false)
    //                  | '|' [param {',' param}] '|' ['->' type] block  (expect_close=true)
    // param ::= ident [':' type]
    // `expect_close` is true when the opening `|` was consumed (params may follow);
    // false when `||` was consumed (zero params, no closing `|`).
    // Types are inferred from `lambda_hint` (set by the call-site parser) when omitted.
    // Produces Type::Function; runtime representation is d_nr as i32, same as fn-ref.
    #[allow(clippy::too_many_lines)] // single context save/restore spans the whole body; splitting would need unsafe borrowing
    pub(crate) fn parse_lambda_short(&mut self, code: &mut Value, expect_close: bool) -> Type {
        let lambda_name = format!("__lambda_{}", self.lambda_counter);
        self.lambda_counter += 1;
        let stored_name = format!("n_{lambda_name}");

        // Capture hint types before entering the new context.
        let hint_params_ret = self.lambda_hint();
        let hint_params: Vec<Type> = if let Type::Function(pts, _, _) = &hint_params_ret {
            pts.clone()
        } else {
            Vec::new()
        };

        // Parse parameter list from `|p1 [: T], p2 [: T], …|`.
        // When expect_close=false (`||` was consumed), there are no params and no closing `|`.
        let mut param_names: Vec<String> = Vec::new();
        let mut param_types: Vec<Type> = Vec::new();
        if expect_close {
            while !self.lexer.peek_token("|") && !self.lexer.peek_token("{") {
                let Some(pname) = self.lexer.has_identifier() else {
                    break;
                };
                let idx = param_names.len();
                let tp = if self.lexer.has_token(":") {
                    // type annotations are not allowed in |x| short-form lambdas.
                    // Use the long form fn(x: type) -> ret { body } instead.
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Type annotations are not allowed in |x| lambdas — \
                         use fn({pname}: <type>) {{ ... }} instead \
                         (add `-> <ret>` only for non-void returns; \
                         `-> void` is not a valid type)"
                    );
                    // Consume the type token so parsing can continue.
                    let _ = self.lexer.has_identifier();
                    // Infer from hint to keep parsing viable.
                    hint_params.get(idx).cloned().unwrap_or(Type::Unknown(0))
                } else {
                    // Infer from hint.
                    hint_params.get(idx).cloned().unwrap_or(Type::Unknown(0))
                };
                param_names.push(pname);
                param_types.push(tp);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("|"); // consume closing `|`
        }

        // Build Argument list for function registration.
        let arguments: Vec<Argument> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, t)| Argument {
                name: n.clone(),
                typedef: t.clone(),
                default: Value::Null,
                constant: false,
                ref_pos: (0, 0),
                const_pos: (0, 0),
            })
            .collect();

        // Error on second pass for any parameter whose type is still Unknown.
        if !self.first_pass {
            for a in &arguments {
                if a.typedef.is_unknown() {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot infer type for lambda parameter '{}'; pass the lambda where the expected type is known, or use fn(name: <type>) {{ ... }} (add `-> <ret>` only for non-void returns)",
                        a.name
                    );
                }
            }
        }

        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(&lambda_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;
        // save outer scope variable names/types for capture detection.
        let (outer_capture, outer_owner) = self.enter_capture_scope(&outer_vars, outer_context);
        let outer_captured = std::mem::take(&mut self.captured_names);

        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, &lambda_name, &arguments)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            self.capture_context = outer_capture;
            self.capture_owner = outer_owner;
            self.captured_names = outer_captured;
            return Type::Unknown(0);
        }
        let d_nr = self.context;
        self.mark_lambda_sandboxed(outer_context, d_nr);

        // return-type annotations are not allowed in |x| short-form lambdas.
        let has_arrow = self.lexer.has_token("->");
        let result = if has_arrow {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Return-type annotations are not allowed in |x| lambdas — \
                 use fn(…) -> <ret> {{ ... }} instead"
            );
            self.parse_type_full(d_nr, true).unwrap_or(Type::Void)
        } else if let Type::Function(_, ret, _) = &hint_params_ret {
            *ret.clone()
        } else {
            Type::Void
        };
        if self.first_pass {
            // loft#945 — pass 1 takes the hint's return type too, not only an explicit
            // `->`.  Storing `Void` here and letting pass 2 force the hint meant the two
            // passes parsed the body against different return types: a text-returning
            // callback promotes a work buffer into a hidden PARAMETER while its body
            // parses, so the signature grew on pass 2 alone and
            // `xs.reduce("", |a, x| { "{a}{x}" })` died on the H5 two-pass contract.
            // Left `Void` only when nothing names the return — `block_result` infers it
            // from the body then, at the same point on both passes.
            let known = if has_arrow || !(result.is_unknown() || matches!(result, Type::Void)) {
                result.clone()
            } else {
                Type::Void
            };
            // A lifetime-bearing tuple return is boxed exactly as a named function's is
            // (loft#1349) — see `Parser::boxed_tuple_return`; the same step on both
            // passes, so the signature does not move between them.
            let known = self.boxed_tuple_return(known);
            self.data.set_returned(d_nr, known);
        } else if !result.is_unknown() && !matches!(result, Type::Void) {
            // On second pass, force-update the return type from hint or annotation.
            let boxed = self.boxed_tuple_return(result.clone());
            self.data.definitions[d_nr as usize].returned = boxed;
        }

        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        for (a_nr, a) in arguments.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
                // Force-update the data definition with the inferred type.
                // `set_attr_type` panics on non-unknown, so write directly.
                // (First pass stored Unknown(0); typedef.rs may have resolved that to a
                // concrete type before the second pass, so we can't rely on is_unknown().)
                if !a.typedef.is_unknown() {
                    self.data.definitions[d_nr as usize].attributes[a_nr].typedef =
                        a.typedef.clone();
                }
            }
        }

        // Mirror `parse_lambda` (the `fn(){}` form): on the second pass, add the
        // `__closure` attribute + set `closure_param` so the body reads captured
        // outer variables from the closure record.  Without this the short `|x|`
        // form could not capture (formal D-clo-1) — the two lambda syntaxes are now
        // captured identically (pure sugar), matching what a user expects.  INERT for
        // a non-capturing lambda: no captures ⇒ no closure record ⇒ `closure_rec ==
        // u32::MAX` ⇒ the block is a no-op, so an ordinary `|x| { x*2 }` map callback
        // lowers exactly as before.
        let outer_closure_param = self.closure_param;
        if !self.first_pass {
            let closure_rec = self.data.def(d_nr).closure_record();
            if closure_rec != u32::MAX {
                let closure_tp = Type::Reference(closure_rec, Deps::none());
                self.data
                    .add_attribute(&mut self.lexer, d_nr, "__closure", closure_tp.clone());
                let v_nr = self.create_var("__closure", &closure_tp);
                self.vars.become_argument(v_nr);
                self.closure_param = v_nr;
            }
        }

        // loft#945 — with no `-> τ` (the short form forbids one) and no return in the
        // hint that placed this lambda (`map`'s callback: its `U` is free), the body IS
        // the declaration.  `block_result` adopts the tail's type at the tail, where the
        // return machinery can still act on it; see `Parser::infer_ret_defs` for why
        // anywhere later is the H5 two-pass contract violation.
        // Nothing named the return: no `-> τ`, and the hint either is not a function type
        // at all (`Void` here) or names no return (`Unknown` — `map`'s callback).
        let infer_ret = !has_arrow && (result.is_unknown() || matches!(result, Type::Void));
        if infer_ret {
            self.infer_ret_defs.insert(d_nr);
        }
        self.parse_code();
        if infer_ret {
            self.infer_ret_defs.remove(&d_nr);
        }
        self.closure_param = outer_closure_param;
        self.data.op_code(d_nr);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);

        // synthesize closure record if any captures were detected.
        // Plan-22 phase 02c (2026-05-12): collect mutations BEFORE
        // synthesize_closure_record so the synthesise step can
        // consult `data.def(d_nr).mutated_captures` to pick the
        // right attribute type (auto-Reference for mutated
        // Reference captures vs inline-bytes for everything else).
        // The collect helper accepts a missing closure_record and
        // derives captured names from the lambda's variable table
        // in that case.
        if !self.captured_names.is_empty() {
            collect_mutated_captures(&mut self.data, d_nr);
            // Plan-22 phase 02d-i (2026-05-12): accumulate scalar
            // mutated-captures onto the parent function's
            // `scalars_to_box` field.  Phase 02d-iii will use this
            // to rewrite outer bindings to hidden cells.  Detection-
            // only at this phase — no behavior change.
            accumulate_scalars_to_box(
                &mut self.data,
                outer_context,
                d_nr,
                &self.captured_names,
                &self.capture_owner,
            );
            // Plan-22 phase 02d-ii — ensure a `__cell_<T>` struct
            // exists for every scalar-typed mutated capture, so
            // 02d-iii's outer-binding rewrite has a target type to
            // allocate.  Idempotent across all lambdas in the
            // compilation; gated on first_pass.
            self.synthesize_cell_structs(d_nr);
            // Plan-22 phase 02d-iii.e — pre-box `captured_names`
            // entries for names in the parent's scalars_to_box
            // BEFORE `synthesize_closure_record` builds the
            // record's attributes.  Without this, pass 1 freezes
            // the attribute as the un-flipped scalar (Integer)
            // and the closure record's storage layout uses 8B
            // inline instead of 12B share-by-DbRef — runtime
            // then misroutes writes through `OpSetInt` and
            // crashes on the locked store.
            box_captured_names_for_outer_scalars(
                &mut self.captured_names,
                &self.data,
                outer_context,
                &self.capture_owner,
            );
            self.synthesize_closure_record(d_nr, &lambda_name);
            // #314: remember this lambda so the enclosing body's end
            // can reject shared-mutable-scalar captures once
            // `scalars_to_box` is complete.
            if self.first_pass {
                self.fn_lambdas.entry(outer_context).or_default().push(d_nr);
            }
        }
        let captured = std::mem::replace(&mut self.captured_names, outer_captured);

        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;
        self.capture_context = outer_capture;
        self.capture_owner = outer_owner;
        // The enclosing table is back, so a captured name can finally be asked what it is
        // bound to out here (loft#1281).
        self.reject_rebound_heap_parameter_captures(d_nr);
        self.relay_nested_captures(&captured);

        self.data.def_used(d_nr);

        // See the twin in `parse_lambda`: only this lambda's own emit may answer.
        self.last_closure_work_var = u16::MAX;
        self.emit_lambda_code(code, d_nr);

        // The public Function type is the DECLARED parameters only — the first
        // `param_names.len()` attributes.  Later attributes are hidden injections
        // (text `__work_*` work-refs, the `__closure` record param added above for a
        // capturing lambda) and must NOT appear in the type, or the arity check at a
        // `.map(f)` call site would see the extra param and reject (matches the
        // `fn(){}` form, which builds its type from `arguments`, not `data.attributes`).
        let n_params = param_names.len();
        let arg_types: Vec<Type> = (0..n_params)
            .map(|a| self.data.attr_type(d_nr, a))
            .collect();
        let ret_type = self.published_ret_type(d_nr, self.data.def(d_nr).returned().clone());
        // include closure work var dep (same as fn-form lambda).
        let dep = if self.last_closure_work_var == u16::MAX {
            Deps::none()
        } else {
            Deps::frame1(self.last_closure_work_var)
        };
        Type::Function(arg_types, Box::new(ret_type), dep)
    }

    // emit the lambda value — plain Int(d_nr) for non-capturing
    // lambdas, or an Insert block that allocates and populates the closure record.
    #[allow(clippy::similar_names)]
    fn emit_lambda_code(&mut self, code: &mut Value, d_nr: u32) {
        let closure_rec_d = self.data.def(d_nr).closure_record();
        if closure_rec_d != u32::MAX && !self.first_pass {
            // A5.6-1/2 (16-byte fn-ref + embedded closure):
            // Allocate and populate the closure record at lambda DEFINITION time.
            // Embed the closure DbRef into a 16-byte fn-ref slot so the closure
            // travels with the fn-ref even when it escapes its defining scope.
            //
            // Layout of the 16-byte fn-ref frame slot:
            //   bytes  0.. 4: d_nr (i32, function definition number)
            //   bytes  4..16: closure DbRef (12 bytes; null = no closure)
            //
            // w (___clos_N) is added to work_refs so parse_code inserts Set(w,Null)
            // at the START of the enclosing function body, pre-reserving w's slot
            // in the outer scope — FreeStack cannot clobber it.
            //
            // The fn_ref_var (__fn_ref_N) holds [d_nr, closure DbRef] as 16 bytes.
            // At call sites, fn_call_ref reads the embedded DbRef and pushes it as
            // the hidden __closure arg automatically — no explicit injection needed.
            let rec_tp = Type::Reference(closure_rec_d, Deps::none());
            let w = self.create_unique("__clos", &rec_tp);
            self.vars.defined(w);
            // Register w as a work-ref so parse_code inserts Set(w,Null) at fn start.
            self.vars.add_to_work_refs(w);
            let tp_nr = i32::from(self.data.def(closure_rec_d).known_type());
            // Build fn_type for fn_ref_var: visible params (excluding __closure) + ret.
            let n_all_attrs = self.data.attributes(d_nr);
            let has_closure_attr =
                n_all_attrs > 0 && self.data.attr_name(d_nr, n_all_attrs - 1) == "__closure";
            let n_visible = if has_closure_attr {
                n_all_attrs - 1
            } else {
                n_all_attrs
            };
            let visible_params: Vec<Type> = (0..n_visible)
                .map(|aid| self.data.attr_type(d_nr, aid).clone())
                .collect();
            let ret_tp = self.published_ret_type(d_nr, self.data.def(d_nr).returned().clone());
            // fn-ref depends on closure work var `w` so that
            // get_free_vars does not emit OpFreeRef for the closure record
            // before the fn-ref escapes the defining scope.
            let fn_type = Type::Function(visible_params, Box::new(ret_tp.clone()), Deps::frame1(w));
            let mut alloc_steps: Vec<Value> = Vec::new();
            // Allocate and populate the closure record w.
            alloc_steps.push(crate::data::v_set(w, Value::Null));
            alloc_steps.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp_nr)]));
            let n_attrs = self.data.attributes(closure_rec_d);
            let mut captured_var_nrs: Vec<u16> = Vec::new();
            for aid in 0..n_attrs {
                let cap_name = self.data.attr_name(closure_rec_d, aid);
                let v_nr = self.vars.var(&cap_name);
                if v_nr != u16::MAX {
                    captured_var_nrs.push(v_nr);
                    // mark as captured so test_used does not emit
                    // a false "never read" warning.  Do NOT call var_usages —
                    // that would interfere with the dead-assignment check.
                    self.vars.set_captured(v_nr);
                }
                // loft#1236 — fill from however THIS scope NAMES the capture, and its own
                // closure field comes first: a lambda that names an outer binding gets a
                // placeholder variable in its own table, which pass 2 never assigns because
                // the read goes through the record.  Filling from that placeholder handed the
                // inner record an unallocated slot (an internal compiler error) and, where it
                // survived, an inline scalar where the outer half had a boxed cell.  A name
                // this scope really OWNS is not in its record, so it falls through to the
                // local — which is every one-level capture.
                let fill = self
                    .relayed_capture_read(&cap_name)
                    .or_else(|| (v_nr != u16::MAX).then_some(Value::Var(v_nr)));
                if let Some(fill) = fill {
                    if v_nr != u16::MAX {
                        // loft#1218 — give a NULL collection capture its slot BEFORE the fill
                        // below copies the handle, or there is nothing for the lambda to share.
                        let backing = self.null_capture_backing(v_nr);
                        alloc_steps.extend(backing);
                    }
                    alloc_steps.push(self.set_field_no_check(
                        closure_rec_d,
                        aid,
                        0,
                        Value::Var(w),
                        fill,
                    ));
                    // P259 / Plan-57 Phase B (Mechanism B): the closure record now
                    // holds a DbRef into the captured heap cell (`Reference(__cell_*,
                    // _)`) via the auto-Reference attribute, and the record OWNS that
                    // cell — `Stores::free_named`'s cascade (allocation.rs:301) frees
                    // it when the closure value dies.  No `OpIncRc` is emitted: the
                    // defining-frame `OpFreeRef` on the cell is suppressed in
                    // `get_free_vars` (scopes.rs) instead, so a captured cell survives
                    // a factory return without an rc bump.  This was the last
                    // load-bearing `OpIncRc`; dropping it unblocks removing the
                    // ref-count entirely (Phase C).
                }
            }
            self.last_closure_captured_vars = captured_var_nrs;
            // Block result: push d_nr (4B via OpConstInt) + closure DbRef (12B via OpVarRef).
            // Together these 16 bytes constitute the fn-ref slot value.
            alloc_steps.push(Value::FnRef(d_nr as i32, w, Box::new(fn_type.clone())));
            *code = crate::data::v_block(alloc_steps, fn_type, "fn_ref_with_closure");
            // A5.6-1/2: closure is embedded in fn-ref — no explicit call-site injection.
            self.last_closure_alloc = None;
            // propagate closure dep and work-buffer info to the
            // enclosing function's declared return type.  Two things are needed:
            // 1. The closure work var `w` in the Function dep list, so
            //    get_free_vars does not emit OpFreeRef for the closure record.
            // 2. The lambda's actual return type (with work-buffer deps), so
            //    try_fn_ref_call at the call site creates the right number of
            //    work buffers.  Without this, cross-scope fn-ref calls to
            //    text-returning lambdas crash because the work buffer is missing.
            if let Type::Function(params, _, _) = self.data.def(self.context).returned() {
                let params = params.clone();
                // H2 step 5: `w` is a FRAME var stored in the DEF-space home
                // (`Definition.returned`) — write it as a tagged
                // callee-frame note so readers decode the space instead of
                // guessing by attr-range position (`Deps::entries`).
                self.data.definitions[self.context as usize].returned =
                    Type::Function(params, Box::new(ret_tp), Deps::callee_frame1(w));
            }
            // record the work var so parse_assign can populate closure_vars
            // (used by write-back and native codegen's closure_var_of lookup).
            self.last_closure_work_var = w;
        } else {
            *code = Value::Int(d_nr as i32);
        }
    }

    /// The backing a NULL collection capture needs before the closure record can share it —
    /// empty for every other capture.
    ///
    /// A captured collection is shared as a DbRef, and `vector::is_absent_collection`'s header
    /// states what that DbRef is: *"a collection field or local is addressed by a DbRef aimed
    /// AT its 4-byte slot, not at the collection"*, with absence read out of the SLOT because
    /// the value-level null "can never appear there".  A `τ?` local still holding the
    /// value-level null has no slot to aim at, so the closure record was handed `DbRef::NULL`
    /// and the append inside the lambda had no destination: silently lost for a vector, a NULL
    /// DbRef fault for a keyed kind (loft#1218).
    ///
    /// So the local is given its slot here, marked ABSENT.  That changes the REPRESENTATION
    /// and not the value — `v == null` still answers true, because absence is what the slot
    /// now says — and an append on EITHER side then materialises in place, which is what an
    /// absent FIELD has always done (loft#1213).  Materialising the COLLECTION instead is the
    /// cheaper-looking alternative and is wrong: it would make `v == null` answer false
    /// because a lambda mentioned `v`.
    ///
    /// Not gated on the lambda MUTATING the capture.  `(L-CapHeap)` shares a captured heap
    /// value whatever the body does with it, and the mutation walker cannot see a collection
    /// append in any case: `v += [x]` lowers to `OpNewRecord` / `OpFinishRecord` on the
    /// capture and neither is in its mutating-op set, so it reported the element TEMP as the
    /// mutated name and the capture as untouched.  A read-only capture pays one store for a
    /// representation it shares with every other collection capture.
    fn null_capture_backing(&mut self, v_nr: u16) -> Vec<Value> {
        if self.first_pass || v_nr == u16::MAX || self.vars.is_argument(v_nr) {
            return Vec::new();
        }
        let tp = self.vars.tp(v_nr).clone();
        let (base, nullable) = tp.peel_optional();
        if !nullable || !Self::is_collection_type(base) {
            return Vec::new();
        }
        // A dep means the local already owns a backing — a `= []` capture, or one an earlier
        // statement built.  Its slot is already there to share.
        if !tp.depend().is_empty() {
            return Vec::new();
        }
        #[allow(clippy::cast_possible_wrap)]
        let absent = crate::keys::DbRef::ABSENT_REC as i32;
        if let Type::Vector(elm, _) = base {
            let elm = (**elm).clone();
            return self.vector_db_init(&elm, v_nr, absent, true);
        }
        // A KEYED local has no wrapper record: its store IS the collection, so the slot it must
        // gain is the one `OpDatabase` hands it.  Built with the guarded emission a keyed WRITE
        // already uses, and then marked ABSENT — the store exists to be shared, the collection
        // does not exist yet, and `h == null` is still true.
        //
        // The mark is what makes this the same change of REPRESENTATION the vector branch
        // above makes.  Without it the mint alone is a change of VALUE: `h == null` answers
        // false straight after the capture, which is the reading this whole function exists to
        // avoid.
        //
        // Guarded on the HANDLE, and NOT through `keyed_local_materialise`, whose guard is
        // `OpVectorIsNull` — the right test for a WRITE, which wants the empty collection built
        // whenever there is no collection.  Here the mint leaves the collection ABSENT on
        // purpose, so that test still answers true afterwards: a closure built inside a loop
        // would re-mint and re-MARK on every pass, and the second pass's mark would wipe what
        // the first one's appends had put in.  `OpRefIsNull` asks the question this site means
        // — does the local have a store at all — and is false from the first pass on.
        let Some(kt) = self.keyed_known_type(&tp) else {
            return Vec::new();
        };
        let mint = self.cl("OpDatabase", &[Value::Var(v_nr), Value::Int(i32::from(kt))]);
        let mark = self.cl(
            "OpSetInt4",
            &[Value::Var(v_nr), Value::Int(0), Value::Int(absent)],
        );
        let test = self.cl("OpRefIsNull", &[Value::Var(v_nr)]);
        vec![crate::data::v_if(
            test,
            Value::Insert(vec![mint, mark]),
            Value::Null,
        )]
    }

    /// Synthesize an anonymous struct definition for the captured variables
    /// of a lambda. Emits a diagnostic with the record layout for test verification.
    ///
    /// Plan-22 phase 02c (2026-05-12): for each mutated Reference
    /// capture (per phase 01's `mutated_captures` walker, populated
    /// in pass 1 by phase 02a), the attribute type is
    /// `Reference(d, [u16::MAX])` instead of `Reference(d, [])`.
    /// The non-empty dep activates phase 02b's auto-Reference
    /// storage encoding (12-byte DbRef + OpSetDbRef + OpGetDbRef
    /// instead of inline-bytes + OpCopyRecord + OpGetField).
    /// Mutations made inside the closure body propagate to the
    /// outer scope through the shared store record.
    ///
    /// The dep value `u16::MAX` is a SENTINEL meaning
    /// "auto-Reference share-marker" — it's not a real outer-var
    /// nr (we're inside the lambda's scope at synthesis time, so
    /// the outer var nr isn't directly accessible).  Phase 03
    /// (Case C) refines the dep value to the actual outer-var nr
    /// for proper liveness tracking.  For phase 02c's case-B
    /// scope (closure stays within capture's scope), the sentinel
    /// is sufficient — the closure record itself is freed at the
    /// outer scope's exit, after which the captured value is no
    /// longer reachable.
    /// Plan-22 phase 02d-ii — ensure a `__cell_<T>` struct exists
    /// for every scalar-typed mutated capture in
    /// `self.captured_names`.
    ///
    /// Idempotent: a cell struct created for one lambda is reused
    /// by every subsequent lambda that captures the same scalar
    /// type.  Gated on `self.first_pass` because struct definitions
    /// must be created in pass 1; pass 2 looks them up via
    /// `data.def_nr(name)` (no creation).
    ///
    /// Cells whose `cell_struct_name` returns `None` (Reference / Function / Vector /
    /// etc.) are skipped — phase 02d-iii's outer-binding rewrite detects the missing
    /// cell at the use site and falls back to stack-slot codegen.  No SCALAR is
    /// declined, so that fallback never carries one (`@FR-L-CapWrite`).
    fn synthesize_cell_structs(&mut self, lambda_d_nr: u32) {
        if !self.first_pass {
            return;
        }
        let captures = self.captured_names.clone();
        let mutated: Vec<String> = self.data.def(lambda_d_nr).mutated_captures().to_vec();
        for (name, tp) in &captures {
            if !mutated.iter().any(|m| m == name) {
                continue;
            }
            let Some(cell_name) = cell_struct_name(tp, &self.data) else {
                continue;
            };
            // Idempotent: skip if the cell struct already exists.
            if self.data.def_nr(&cell_name) != u32::MAX {
                continue;
            }
            let cell_d_nr = self
                .data
                .add_def(&cell_name, self.lexer.pos(), DefType::Struct);
            let value_tp = cell_value_type(tp);
            // The FLAG has to say what the TYPE says.  `add_attribute` marks every
            // attribute nullable, and a dense cell's `value` is not: that is the whole
            // reason a nullable capture takes a `__cell_opt_` of its own rather than
            // widening this one (`@FR-N-Store` — a `τ` field may not hold the sentinel).
            // Left disagreeing, the field's width is read two ways — the declared width
            // from the type, a sentinel-reserving one from the flag — which agree at 8
            // bytes and part company at every narrow width: a `u8` cell registered as
            // `byte<0,true>` in the compiler and `short<0,true>` in generated `init()`,
            // one byte against two, with the ids past it renamed (loft#739).
            let nullable = matches!(value_tp, Type::Optional(_));
            let a_nr = self
                .data
                .add_attribute(&mut self.lexer, cell_d_nr, "value", value_tp);
            self.data.set_attr_nullable(cell_d_nr, a_nr, nullable);
        }
    }

    /// Plan-22 phase 02d-iii.a — at the start of pass 2 for the
    /// parent function, replace each scalar local in
    /// `Definition.scalars_to_box` with its boxed
    /// `Reference(__cell_<T>, [])` type.  Runs AFTER the
    /// pass-1 vars are restored via `Function::append`, so the
    /// helper sees every local that pass 1 added (including the
    /// names queued by phase 02d-i's accumulator).
    ///
    /// Foundation step: NO read or write rewriting yet.
    /// Subsequent sub-phases (02d-iii.b read auto-deref, 02d-iii.c
    /// first-set allocation, 02d-iii.d closure-body write rewrite,
    /// 02d-iii.e type-check gate) build on this type-flip.
    ///
    /// Today's behavior with this commit alone: the variables
    /// table reports the boxed type, but every emit site still
    /// treats `n` as a stack-slot scalar.  Any function with
    /// mutating scalar captures will hit a type-mismatch
    /// diagnostic at the first `n = …` site (RHS scalar vs LHS
    /// `Reference(__cell_<T>, _)`).  No existing test exercises
    /// mutating-scalar-capture closures beyond the cells of
    /// `tests/mut_closure_matrix.rs`, which run by default, so the
    /// regression net stays green.
    ///
    /// `change_var_type` (in `src/parser/expressions.rs`) is
    /// taught to PRESERVE the flipped Reference type when the
    /// new type is the cell's value type; without that guard,
    /// `change_var(to, &s_type)` in `parse_assign_op` would
    /// revert `n` back to Integer on every `n = …` line.
    ///
    /// Idempotent + scoped:
    /// - No-op when `scalars_to_box` is empty (the common case
    ///   for every function in the standard library and existing
    ///   tests).
    /// - Skips arguments — boxing a function parameter would
    ///   change its call-site signature.  Argument boxing is a
    ///   follow-up sub-step (matrix row M / Case-B-on-arg uses
    ///   the explicit `Mutable<T>` path in phase 05).
    /// - Skips names whose `cell_struct_name` returns `None` — no
    ///   scalar type is among them.
    /// - Skips names already flipped on a re-entry (defensive).
    #[allow(
        dead_code,
        reason = "Helper exists for 02d-iii.e activation; only invoked from tests for now."
    )]
    pub(crate) fn flip_scalars_to_box_types(&mut self) {
        if self.context == u32::MAX || (self.context as usize) >= self.data.definitions.len() {
            return;
        }
        // Plan-22 phase 02d-vii's blanket "skip text when the parent returns text" is
        // RETIRED (#687).  It stood in for one real case — a text local that is the
        // function's RETURN SOURCE, which the text-return machinery has already given a
        // hidden `&text` out-parameter — and the `RefVar` test below names that case
        // directly.  As a proxy it was both too wide (it also skipped a text local the
        // function does not return, which boxes fine) and useless for a PARAMETER, which
        // has no indirection of its own to reuse.
        let names = self.data.def(self.context).scalars_to_box().to_vec();
        for name in &names {
            let mut v_nr = self.vars.var(name);
            if v_nr == u16::MAX {
                continue;
            }
            // #685 — an ARGUMENT cannot be flipped in place: its slot receives the
            // caller's scalar, and giving it a 12-byte cell DbRef type would change
            // the call ABI.  Promote it to a shadow LOCAL seeded from the argument
            // instead (the same move the text-argument promotion in
            // `parse_assign_op` makes), and flip that.  This runs before the body is
            // parsed, so the name remap sends every later read, write and capture to
            // the shadow — leaving the argument case indistinguishable from the local
            // case that already works for every boxable type.
            //
            // Without it the two halves of "is this capture boxed?" disagreed:
            // `box_captured_names_for_outer_scalars` gave the closure record a
            // 12-byte DbRef field while the argument stayed an 8-byte stack scalar,
            // so `emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of an 8-byte
            // slot and corrupted the fn-ref being built beside it.
            //
            // A `RefVar` argument is excluded: it already HAS its own indirection.  It is
            // either a user `&T` out-parameter (whose writes must reach the caller, so a
            // private cell would swallow them) or a text local the return machinery
            // promoted to a hidden `&text` out-parameter — and for that one the record
            // stores the value inline and the existing write-back propagates it, which is
            // the pairing `finalize_capture_storage` keeps in step (#687).
            // A DECLARED `&` SCALAR parameter written from inside a closure is the one
            // capture shape that cannot be made to mean what it says, and it is refused HERE
            // rather than allowed to compute quietly (loft#1276; the decision and why no code
            // change closes it are DESIGN_DECISIONS C115).
            //
            // Everything else about a `&` capture works, because `closure_attr_type` captures
            // the POINTEE: a `&S` / `&vector<τ>` shares its DbRef, so a field write, an
            // element write, an append and even a whole-record rebind all reach the caller
            // (`(L-CapHeap)`), and a `&integer` / `&text` READ copies at closure creation
            // (`(L-CapScalar)`).  A scalar WRITE is different in kind: the value lives in the
            // CALLER's slot, the capture holds a copy of it, and there is no shared record for
            // the write to land in.  The cell promotion below is what makes this work for a
            // plain local — and a cell cannot serve a `&`, because reads in the enclosing body
            // would then see the cell while the caller still sees its own slot.
            //
            // Measured before the refusal existed: `fn bump(p: &integer) { g = fn() { p += 1;
            // }; g(); p = p + 10; }` on `n = 5` answered 15 instead of 16 — the closure's
            // increment silently dropped, through a parameter whose whole purpose is the
            // write-back.
            //
            // A HIDDEN `&` argument is not a user parameter: `text_return` / `ref_return`
            // promote a returned text local to a hidden out-parameter, and that one is served
            // by the existing per-call write-back (#687).  The type cannot tell the two apart
            // — `fn f(a: &const integer)` is a declared `RefVar` too — so the definition's
            // `hidden` flag is asked, which is where the fact lives.
            let is_user_ref_param = matches!(self.vars.tp(v_nr), Type::RefVar(_))
                && self.vars.is_argument(v_nr)
                && self
                    .data
                    .def(self.context)
                    .attributes()
                    .get(v_nr as usize)
                    .is_some_and(|a| !a.hidden);
            if is_user_ref_param
                && let Type::RefVar(inner) = self.vars.tp(v_nr).clone()
                && cell_struct_name(&inner, &self.data).is_some()
            {
                self.lexer.to(self.vars.var_source(v_nr));
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot write to the `&` parameter '{name}' from a closure — a \
closure captures a scalar BY VALUE, so the write would not reach the caller. Capture a \
local copy and write it back after the closure runs: `local = {name}; …; {name} = local;`"
                );
                continue;
            }
            if self.vars.is_argument(v_nr) && !matches!(self.vars.tp(v_nr), Type::RefVar(_)) {
                // A value-const parameter is read-only, and the closure-side write never
                // reaches `validate_write`'s guard — inside the lambda `name` is a
                // capture, not a binding that carries the const flag.  This is the first
                // point that knows a closure mutates it, so the guard belongs here;
                // without it the promotion below would quietly hand the closure a
                // writable cell (the crash it replaced at least failed loudly).
                if self.vars.is_value_const(v_nr) {
                    // `const_report_var` — see loft#1250.  `name` is the captured
                    // spelling, which is the source one here; the KIND still has to come
                    // from the original, because a promoted `__tp_` local is not marked an
                    // argument and would demote "const parameter" to "const variable".
                    let report = self.vars.const_report_var(v_nr);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot modify {} '{}' from a closure; remove 'const' or \
                         capture a local copy",
                        self.const_noun(report),
                        self.vars.name(report)
                    );
                    continue;
                }
                match self.promote_boxed_scalar_arg(name, v_nr) {
                    Some(shadow) => v_nr = shadow,
                    None => continue,
                }
            }
            let original_tp = self.vars.tp(v_nr).clone();
            // Skip if already flipped.
            if boxed_cell_def(&original_tp, &self.data).is_some() {
                continue;
            }
            // Plan-22 phase 02d-iii.e / 02d-v / 02d-vi — all
            // boxable scalar types now flip cleanly:
            //
            // - Direct shapes (Integer / Float / Single /
            //   Character / Text / plain Enum): auto-deref
            //   produces `Call(OpGet<T>, [Var, 0])`,
            //   `maybe_prepend_cell_alloc`'s
            //   `extract_boxed_var_from_lhs` recognises this
            //   shape, `call_to_set_op` maps `OpGet<T>` →
            //   `OpSet<T>` for the write side.
            //
            // - Boolean: auto-deref wraps in
            //   `Call(OpEqInt, [Call(OpGetByte, [Var, 0, 0]),
            //   Int(1)])`; phase 02d-v's `extract_boxed_var_from_lhs`
            //   recognises this nested shape too; writes route
            //   through towards_set's existing boolean branch
            //   (collections.rs:493) which produces the right
            //   OpSetByte IR with bool→byte conversion.
            //
            // - Text: phase 02d-vi added a bypass guard in
            //   `parse_assign_op` that skips the text-special
            //   branch when the LHS is auto-deref'd boxed text.
            //   The general path then handles re-assignment
            //   (`log = "after"`) and append (`log += s` lowered
            //   to `log = log + s`) via `OpSetText` writes
            //   through the cell DbRef.
            let Some(cell_name) = cell_struct_name(&original_tp, &self.data) else {
                continue;
            };
            let cell_d_nr = self.data.def_nr(&cell_name);
            if cell_d_nr == u32::MAX {
                continue;
            }
            self.vars
                .set_type(v_nr, Type::Reference(cell_d_nr, Deps::none()));
        }
    }

    /// #685 — replace a boxable scalar ARGUMENT with a shadow local of the same
    /// type, seeded from the argument at function entry, and point `name` at it.
    /// Returns the shadow, or `None` for a type with no cell struct (the caller
    /// then leaves the argument alone, as before).
    ///
    /// This is what the hand-written workaround did — `acc = n;` as the first
    /// statement — so the shadow inherits whichever lowering the LOCAL case uses:
    /// a `__cell_<T>` for integer / float / boolean / character, or the text
    /// write-back path when the flip below skips text.  Nothing here needs to know
    /// which.
    ///
    /// The seed is emitted by the promoted-argument preamble in `parse_code`, which
    /// already exists for text-argument promotion.  The shadow is marked `defined`
    /// so the body's first write does not ALSO prepend an allocation
    /// (`maybe_prepend_cell_alloc`) — a second `OpDatabase` would replace the
    /// seeded cell and lose the argument's value.
    ///
    /// Const-ness travels with it: a `const` parameter's shadow stays const, so the
    /// guard that rejects mutating it still fires on the shadow instead of being
    /// silently dropped along with the argument.
    fn promote_boxed_scalar_arg(&mut self, name: &str, arg: u16) -> Option<u16> {
        let tp = self.vars.tp(arg).clone();
        cell_struct_name(&tp, &self.data)?;
        let shadow = self
            .vars
            .add_variable(&format!("__bx_{name}"), &tp, &mut self.lexer);
        if shadow == u16::MAX || shadow == arg {
            return None;
        }
        self.vars.set_promoted_from(shadow, arg);
        self.vars.defined(shadow);
        if self.vars.is_value_const(arg) {
            self.vars.set_value_const(shadow);
        }
        if self.vars.is_const_binding(arg) {
            self.vars.set_const_binding(shadow);
        }
        // The argument is now read only by the seed, which the preamble inserts
        // after `test_used` would otherwise have flagged it unused.
        self.vars.mark_used(arg);
        self.vars.remap_name(name, shadow);
        Some(shadow)
    }

    /// #314 (closed by decision — GOALS.md § "Stability trumps
    /// features"): a mutated scalar captured by MORE THAN ONE closure
    /// is rejected at compile time.
    ///
    /// The shape only worked through shared heap cells with no defined
    /// owner ("first death wins" — `free_named` frees the cell at the
    /// first record's death and silently no-ops for the rest), and the
    /// closure-record attribute types freeze at each lambda's pass-1
    /// epilogue while `scalars_to_box` keeps accumulating until the
    /// parent's body end — so a reader lambda parsed before the writer
    /// baked in the unboxed layout and crashed at runtime.  No
    /// consumer needs the shape; shared mutable state belongs in a
    /// struct, which also makes the sharing visible in the source.
    ///
    /// Runs at the parent's pass-1 body end — the first moment the
    /// accumulation is final — for the same parents whose locals
    /// `flip_scalars_to_box_types` flips in pass 2 (named fns; the
    /// caller in `definitions.rs` is the flip's sibling).  The
    /// single-closure accumulator (one record, one owner) stays
    /// supported.
    /// #687 — settle how each mutated scalar capture is STORED, now that the parent's
    /// pass-1 body is complete.
    ///
    /// A mutated capture is normally boxed into a shared `__cell_<T>` so writes from the
    /// closure and reads from the enclosing body see one location.  The exception is a
    /// binding that already has an indirection of its own: a text local the function
    /// RETURNS, which the text-return machinery promotes to a hidden `&text`
    /// out-parameter so the caller supplies the buffer.  That binding cannot also be a
    /// cell, and it does not need to be — the record stores the value inline and the
    /// existing per-call write-back propagates the closure's changes.
    ///
    /// Both halves must agree, and this is the first moment either could be right:
    /// `box_captured_names_for_outer_scalars` types the record's attribute at the
    /// LAMBDA's epilogue, where a to-be-returned text local still looks like a plain
    /// `Text`, while `flip_scalars_to_box_types` (pass 2) sees the finished `RefVar` and
    /// skips it.  That disagreement is what plan-22 02d-vii papered over with "skip text
    /// when the parent returns text" — too wide (it also skipped a text local the
    /// function does not return) and no help at all for a PARAMETER, which has no
    /// indirection to reuse.  Asking the binding instead covers all three.
    ///
    /// Runs before `fill_all`, so the corrected attribute is what gets laid out.
    pub(crate) fn finalize_capture_storage(&mut self, parent_d: u32) {
        if !self.first_pass
            || parent_d == u32::MAX
            || (parent_d as usize) >= self.data.definitions.len()
        {
            return;
        }
        let Some(lambdas) = self.fn_lambdas.get(&parent_d).cloned() else {
            return;
        };
        for name in self.data.def(parent_d).scalars_to_box().to_vec() {
            let v_nr = self.vars.var(&name);
            // `RefVar` is the fact: this binding already carries its own indirection.
            if v_nr == u16::MAX || !matches!(self.vars.tp(v_nr), Type::RefVar(_)) {
                continue;
            }
            let inline_tp = match self.vars.tp(v_nr) {
                Type::RefVar(inner) => (**inner).clone(),
                other => other.clone(),
            };
            for lam in &lambdas {
                let rec = self.data.def(*lam).closure_record();
                if rec == u32::MAX {
                    continue;
                }
                let a_nr = self.data.attr(rec, &name);
                if a_nr == usize::MAX {
                    continue;
                }
                // Only undo the provisional boxing; anything else is already right.
                let is_cell = boxed_cell_def(&self.data.attr_type(rec, a_nr), &self.data).is_some();
                if is_cell {
                    self.data.retype_capture_attr(rec, a_nr, inline_tp.clone());
                }
            }
        }
    }

    /// Box the capture attribute of every lambda NESTED inside this function's lambdas, for
    /// each name this function boxes.
    ///
    /// Pass 1 freezes a closure record's storage at the lambda's own end, and a nested lambda
    /// ends BEFORE the enclosing one — so when the enclosing lambda is what mutates the name,
    /// the inner record was laid out before anyone knew the binding would be boxed.  The inner
    /// half then read an inline scalar out of a field the outer half filled with a cell DbRef:
    /// `OpSetInt(clos, 0, OpGetDbRef(…))` on one side and `OpGetInt(closure, 0)` on the other,
    /// which wrote a handle through an integer setter and landed on the const store
    /// (loft#1236).
    ///
    /// The repair runs at the OWNER's body end, which is the first moment `scalars_to_box` is
    /// complete, and walks `fn_lambdas` transitively because a nested lambda is registered
    /// against its enclosing LAMBDA rather than against this function.  It is the mirror of
    /// [`Self::finalize_capture_storage`], which un-boxes at the same moment for the binding
    /// that turned out to carry its own indirection.
    pub(crate) fn box_nested_capture_attrs(&mut self, parent_d: u32) {
        if !self.first_pass
            || parent_d == u32::MAX
            || (parent_d as usize) >= self.data.definitions.len()
        {
            return;
        }
        let scalars = self.data.def(parent_d).scalars_to_box().to_vec();
        if scalars.is_empty() {
            return;
        }
        // Only lambdas NESTED inside this function's lambdas.  A direct child is
        // `finalize_capture_storage`'s to decide, and it has just run: re-boxing what it
        // deliberately un-boxed is two indirections for one binding, which is the crash #687
        // exists to prevent.
        let direct: Vec<u32> = self.fn_lambdas.get(&parent_d).cloned().unwrap_or_default();
        let mut todo: Vec<u32> = direct
            .iter()
            .filter_map(|d| self.fn_lambdas.get(d).cloned())
            .flatten()
            .collect();
        let mut seen: Vec<u32> = Vec::new();
        while let Some(lam) = todo.pop() {
            if seen.contains(&lam) || direct.contains(&lam) {
                continue;
            }
            seen.push(lam);
            if let Some(inner) = self.fn_lambdas.get(&lam) {
                todo.extend(inner.iter().copied());
            }
        }
        for name in &scalars {
            // The same exemption `finalize_capture_storage` applies: a binding that already
            // carries its own indirection (a hidden `&text` out-parameter) must stay inline.
            let v_nr = self.vars.var(name);
            if v_nr != u16::MAX && matches!(self.vars.tp(v_nr), Type::RefVar(_)) {
                continue;
            }
            for lam in &seen {
                let rec = self.data.def(*lam).closure_record();
                if rec == u32::MAX {
                    continue;
                }
                let a_nr = self.data.attr(rec, name);
                if a_nr == usize::MAX {
                    continue;
                }
                let tp = self.data.attr_type(rec, a_nr);
                if boxed_cell_def(&tp, &self.data).is_some() {
                    continue;
                }
                let Some(cell_name) = cell_struct_name(&tp, &self.data) else {
                    continue;
                };
                let cell_d_nr = self.data.def_nr(&cell_name);
                if cell_d_nr == u32::MAX {
                    continue;
                }
                self.data
                    .retype_capture_attr(rec, a_nr, Type::Reference(cell_d_nr, Deps::none()));
            }
        }
    }

    pub(crate) fn reject_shared_mutable_scalar_captures(&mut self, parent_d: u32) {
        if !self.first_pass
            || parent_d == u32::MAX
            || (parent_d as usize) >= self.data.definitions.len()
        {
            return;
        }
        let Some(lambdas) = self.fn_lambdas.remove(&parent_d) else {
            return;
        };
        let scalars = self.data.def(parent_d).scalars_to_box().to_vec();
        if scalars.is_empty() {
            return;
        }
        for name in &scalars {
            let capturers = lambdas
                .iter()
                .filter(|&&lam| {
                    let rec = self.data.def(lam).closure_record();
                    rec != u32::MAX
                        && (0..self.data.attributes(rec))
                            .any(|a_nr| self.data.attr_name(rec, a_nr) == *name)
                })
                .count();
            if capturers >= 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "variable `{name}` is mutated through a closure and captured by \
                     {capturers} closures; sharing a mutable variable between closures \
                     is not supported — hold the shared state in a struct field instead \
                     (e.g. `state = State {{ {name}: ... }}` captured by all closures)"
                );
            }
        }
    }

    /// The closure-record ATTRIBUTE type for a capture whose logical type is `tp` —
    /// the STORAGE encoding, which is deliberately not the same thing.
    ///
    /// P260 (2026-05-13): ALL Reference captures store as a 12-byte `Parts::DbRef`
    /// pointing at the live original (`typedef::fill_database`'s arm fires on non-empty
    /// deps).  Inline-byte storage was wrong even for a read-only capture — the closure
    /// then reads a stale snapshot when the outer scope mutates a non-scalar field of
    /// the source struct, and closure-side writes to compound fields (a vector field, a
    /// nested struct, an element) silently no-op against the inline copy.  The arm was
    /// once gated on `is_mutated(name)`, and that gate was wrong: this is an
    /// architectural decision ("don't deep-copy a possibly-large struct into a closure
    /// record"), not a property of whether THIS closure writes to the capture.
    ///
    /// @PLN93 (#511): a captured COLLECTION borrows the outer collection, so it takes
    /// the same shared-DbRef representation — the whole schema / read / write path on
    /// both backends then reuses the proven Reference-DbRef machinery, and the body
    /// recovers the real collection type from `capture_context` (`parser/objects.rs`).
    /// The `Reference` content def is inert for a DbRef field (never laid out inline);
    /// carrying the collection's content def keeps it meaningful.
    ///
    /// The share marker itself is only read by `fill_database` and by `generation`'s
    /// native mirror, and closure records are its sole producer, so none of this reaches
    /// user-defined struct fields.  Ownership (which marker) is decided later, after
    /// scope analysis — see `scopes::mark_borrowed_captures` (#682).
    fn closure_attr_type(&mut self, tp: &Type) -> Type {
        // A NULLABLE heap value takes the same storage as its dense twin, and that is the
        // whole of the rule rather than a special case: `S?` IS a `DbRef` whose `rec == 0`
        // means absent, so sharing it needs no wrapper.  Left to fall through, the `S?`
        // spelling kept its `__nullable<S>` enum type, was COPIED into the closure record
        // inline while `S` was SHARED as a DbRef, and the body's read then applied the
        // enum's payload offset on top of a record the write had placed without one — a
        // garbage read out of a lambda that looked like a capture problem (loft#1114).
        if let Some(d) = self.data.nullable_struct_payload(tp) {
            return Type::Reference(d, Deps::share_sentinel());
        }
        // …and the same for a nullable COLLECTION, which is the other half of that rule and
        // was the half left behind: `.base()`, so `vector<τ>?` / `hash<τ[k]>?` share exactly
        // as their dense twins do.  Unpeeled it fell to `_ => tp.clone()`, the attribute kept
        // the collection type and was stored INLINE, and the body's read came back an
        // `OpGetField` where the dense capture reads an `OpGetDbRef` — so an append inside
        // the lambda was taken for a STRUCT FIELD append, resolved its parent against
        // `Type::Null`, and asked `Data::def` for `u32::MAX`: an internal compiler error on
        // three lines of ordinary source (loft#1209).
        // @FR-L-CapRef — a `&T` parameter captures as its POINTEE, and that is the rule
        // rather than a
        // convenience: `&` is a channel to the CALLER's slot, so the capture question is
        // asked of what it points at.  `(L-CapHeap)` then shares a `&S` / `&vector<τ>`
        // exactly as its bare twin — the DbRef is the same one either way — and
        // `(L-CapScalar)` copies a `&integer` / `&text` at closure creation.  Unpeeled,
        // every `&` shape fell to `_ => tp.clone()`, kept `RefVar(τ)` as the attribute
        // type, and reached `get_val`'s catch-all: *"Field access not supported on type
        // &integer"*, on a lambda body containing no field access (loft#1276).
        //
        // ⚠ What this does NOT give back is the `&` itself.  A write THROUGH the capture
        // to the caller's slot — a scalar write, or a whole-value rebind of a heap
        // capture — needs the ref in the record and a write-back, which the language does
        // not have (DESIGN_DECISIONS C115); `reject_ref_capture_write` refuses those at the
        // capture site rather than letting them land in a copy.
        let tp = match tp {
            Type::RefVar(inner) => inner.as_ref(),
            other => other,
        };
        match tp.base() {
            Type::Reference(d, _) => Type::Reference(*d, Deps::share_sentinel()),
            Type::Hash(c, _, _)
            | Type::Sorted(c, _, _)
            | Type::Index(c, _, _)
            | Type::Radix(c, _, _)
            | Type::Trie(c, _, _) => Type::Reference(*c, Deps::share_sentinel()),
            Type::Vector(elm, _) => {
                Type::Reference(self.data.type_elm(elm), Deps::share_sentinel())
            }
            _ => tp.clone(),
        }
    }

    /// #686 — re-type any closure-record attribute that pass 1 had to leave UNRESOLVED,
    /// now that pass 2 knows what the capture really is.
    ///
    /// A capture whose type mentions a type declared LATER in the file is
    /// `Unknown(0)` when pass 1 freezes the record's attributes (`ch = w.chunks[1]`
    /// where `World` comes further down).  Pass 2 has the resolved type in
    /// `capture_context`, and the body is about to read the attribute for BOTH its own
    /// type-check (`parser/objects.rs`) and the field's storage shape — so the repair
    /// has to happen here, before `parse_code`, not at the record-synthesis epilogue
    /// that runs after the body.
    ///
    /// Attributes are matched by NAME against `capture_context`, which holds every
    /// enclosing binding: `captured_names` is still empty at this point (it fills as
    /// the body parses).  Only unresolved attributes are touched, so a record that
    /// pass 1 typed correctly is left exactly as it was.
    ///
    /// The record is then laid out here rather than by `fill_all`.  `fill_all` runs at
    /// the END of the pass, but `emit_lambda_code` needs the record's `known_type` for
    /// its `OpDatabase` the moment this lambda finishes — and `typedef` deliberately
    /// deferred the layout while the attribute was unresolved
    /// (`has_nameless_unknown_attr`), because registering it then bakes a field with no
    /// position.  Field positions still come from `Stores::finish()` at the end of the
    /// pass, so laying out early only fixes the ORDER, not the mechanism.
    fn resolve_forward_captures(&mut self, closure_rec: u32) {
        if self.first_pass || closure_rec == u32::MAX {
            return;
        }
        let unresolved: Vec<(usize, String)> = (0..self.data.attributes(closure_rec))
            .filter(|&a| self.data.attr_type(closure_rec, a).is_unknown())
            .map(|a| (a, self.data.attr_name(closure_rec, a)))
            .collect();
        if unresolved.is_empty() {
            return;
        }
        for (a_nr, name) in unresolved {
            let Some((_, ctype)) = self
                .capture_context
                .iter()
                .find(|(n, _)| *n == name)
                .cloned()
            else {
                continue;
            };
            if ctype.is_unknown() {
                continue;
            }
            ensure_tuple_defs_for_capture(&mut self.data, &mut self.lexer, &ctype);
            let attr_tp = self.closure_attr_type(&ctype);
            self.data.set_attr_type(closure_rec, a_nr, attr_tp);
        }
        if self.data.def(closure_rec).known_type() == u16::MAX
            && !(0..self.data.attributes(closure_rec))
                .any(|a| self.data.attr_type(closure_rec, a).is_unknown())
        {
            crate::typedef::fill_database(&mut self.data, &mut self.database, closure_rec);
            self.database
                .lay_out_record(self.data.def(closure_rec).known_type());
        }
    }

    fn synthesize_closure_record(&mut self, lambda_d_nr: u32, lambda_name: &str) {
        let record_name = lambda_name.replace("__lambda_", "__closure_");
        let captures = self.captured_names.clone();

        if self.first_pass {
            // Create the struct definition in the first pass.
            let record_d_nr = self
                .data
                .add_def(&record_name, self.lexer.pos(), DefType::Struct);
            for (name, tp) in &captures {
                // P216: ensure the synthetic `__tuple<…>` struct exists
                // for any tuple-typed capture before `fill_database`
                // walks the closure record's attributes — without this,
                // `type_elm(&Type::Tuple(_))` returns `u32::MAX` and the
                // closure record's tuple-typed attribute is silently
                // skipped at `typedef.rs:381`, leaving the closure
                // record with size 0 and the `OpDatabase` allocation
                // panicking with "Incomplete record" at
                // `src/store.rs:227`.
                ensure_tuple_defs_for_capture(&mut self.data, &mut self.lexer, tp);
                let attr_tp = self.closure_attr_type(tp);
                self.data
                    .add_attribute(&mut self.lexer, record_d_nr, name, attr_tp);
            }
            // Store the closure record def_nr on the lambda's definition.
            self.data.definitions[lambda_d_nr as usize].closure_record = record_d_nr;
        } else {
            let record_d_nr = self.data.def_nr(&record_name);
            if record_d_nr != u32::MAX {
                self.data.definitions[lambda_d_nr as usize].closure_record = record_d_nr;
            }
        }
    }

    // <for-vector> ::= 'for' <id> 'in' <range> ['if' <cond>] '{' <expr> '}'
    // Implements [for n in range { body }] vector comprehensions.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(crate) fn parse_vector_for(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &mut Type,
        val: &mut Value,
        is_var: bool,
        is_field: bool,
        block: bool,
        parent_tp: &Type,
    ) -> Type {
        let Some(src_id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect variable after for");
            return Type::Null;
        };
        // loft#915 — the name this comprehension's loop BINDS.  Its variable and its
        // `#index` / `#next` companions all hang off it, so a second comprehension over
        // the same name in one function shares nothing with the first.
        let id = self.vars.loop_binding(&src_id);
        self.lexer.token("in");
        let loop_nr = self.vars.start_loop();
        let mut expr = Value::Null;
        // Bind the range bounds to THIS comprehension.  `last_range_*` is parser-wide
        // state written by whichever range was parsed last, so it is only trustworthy
        // in the window between clearing it and reading it back: cleared here, it is
        // `Some` after the parse below exactly when the iterable IS a range
        // (`parse_in_range_body` is the only writer), so a text or keyed iterable
        // cannot inherit an earlier loop's bounds.
        // Snapshotting BEFORE the body is parsed matters just as much — the body
        // may contain ranges of its own, and they would otherwise be read as this
        // comprehension's length.
        self.last_range_from = None;
        self.last_range_till = None;
        let mut in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
        let range_bounds = self
            .last_range_from
            .clone()
            .zip(self.last_range_till.clone());
        let mut fill = Value::Null;
        if matches!(in_type, Type::Vector(_, _)) {
            let vec_var = self.create_unique("vector", &in_type);
            // The loop iterates THIS temp, so its identity is load-bearing: materialising it
            // would walk a copy while the body's `#remove` empties the original.
            self.vars.set_iteration_source(vec_var);
            in_type = in_type.depending(vec_var);
            fill = v_set(vec_var, expr);
            expr = Value::Var(vec_var);
        }
        let var_tp = self.for_type(&in_type);
        let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
            let pos_var = self.create_var(&format!("{id}#next"), &I32);
            self.vars.defined(pos_var);
            let index_var = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(index_var);
            (pos_var, Some(index_var))
        } else {
            let iv = self.create_var(&format!("{id}#index"), &I32);
            self.vars.defined(iv);
            (iv, None)
        };
        let for_var = self.create_loop_var(&id, &var_tp);
        // The body reads the name the program wrote (loft#915).
        if id != src_id {
            self.vars.set_name(&src_id, for_var);
        }
        self.vars.defined(for_var);
        let if_step = if self.lexer.has_token("if") {
            let mut if_expr = Value::Null;
            self.expression(&mut if_expr);
            if_expr
        } else {
            Value::Null
        };
        // Captured BEFORE `iterator` rewrites `create_iter`: the collection as written, so
        // the length-based break re-reads the SOURCE rather than the iterator state
        // (loft#1000). Mirrors the `for` statement's `orig_coll_expr`.
        let src_coll = expr.clone();
        let mut create_iter = expr;
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, pre_var);
        if !self.first_pass && iter_next == Value::Null {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Need an iterable expression in a for statement"
            );
            return Type::Null;
        }
        let for_next = v_set(for_var, iter_next);
        self.vars.loop_var(for_var);
        let in_loop = self.in_loop;
        self.in_loop = true;
        // Parse body as an expression-returning block: [for n in range { expr }]
        // @PLN25 — when the DECLARED element type is the synthetic `__nullable<S>`
        // enum (a `v: vector<S> = [for … { S{…} }]`, rewritten at the vector-type
        // chokepoint), pass it as the block's expected type so the body's `S{…}`
        // builds the `Some` variant (via parse_block's enum-hint) instead of a
        // bare `S` that then mismatches the rewritten vector type.  Other element
        // types keep `Unknown` — no behaviour change for non-nullable
        // comprehensions.
        let body_expected = if matches!(&*in_t, Type::Enum(e, true, _)
            if self.data.def(*e).name.starts_with("__nullable<"))
        {
            in_t.clone()
        } else {
            // @PLN25 storage-vs-access-nullability — INFERRED comprehensions stay DENSE
            // (the struct-literal PEEK is retired). A nullable element comes only from a
            // DECLARED `vector<?S>` (the first arm above).
            Type::Unknown(0)
        };
        let mut body = Value::Null;
        let body_type = self.parse_block("for", &mut body, &body_expected);
        // #319 — a struct-literal body returns `Rewritten(Reference(...))`.
        // The wrapper is a parse-internal marker, not an element type:
        // leaking it into the vector's element type broke every later
        // `qs[i] ?? …` on the comprehension result (the ncc temp lost its
        // dep chain and stack slot — "Incorrect var __ncc_N[65535]").
        *in_t = if let Type::Rewritten(t) = body_type {
            *t
        } else {
            body_type
        };
        // A comprehension body must produce an element value.  An empty body
        // `[for i in r {}]` (or a void-producing one) yields `Void`, which
        // downstream lifetime/codegen reads as an Unknown definition
        // (`control.rs` block_result, `data.rs::def`) and panics.  Report it
        // cleanly and recover with a dense element type so later passes don't crash.
        if matches!(*in_t, Type::Void) {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "comprehension body must produce a value — the `{{ … }}` after \
                     `for … in …` cannot be empty"
                );
            }
            *in_t = crate::data::I64.clone();
        }
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        // Finalise vector element type (same as parse_vector post-loop)
        let struct_tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        if !is_field {
            self.vars
                .change_var_type(vec, &struct_tp, &self.data, &mut self.lexer);
            self.data.vector_def(&mut self.lexer, in_t);
        }
        let tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        if self.first_pass {
            return tp;
        }
        // (I-Comp) A struct FIELD destination and a compound `+=` both read their target and
        // have no repoint to defer, so they take the other delivery: build into a buffer of
        // this comprehension's own and let the destination's assignment hand it over.  That
        // is exactly the route `map` takes below in `collections.rs`, which is why
        // `s.v = s.v.map(…)` and `a += a.map(…)` already answer correctly.  Pass-2 only —
        // the early return above means the two passes cannot disagree about the mint.
        if self.comprehension_needs_own_buffer(
            vec,
            val,
            is_var,
            is_field,
            &[&fill, &create_iter, &for_next, &if_step, &body],
        ) {
            let out_tp = Type::Vector(Box::new(in_t.clone()), Deps::none());
            let out = self.create_unique("vec", &out_tp);
            self.vars.defined(out);
            let out_elm = self.unique_elm_var(&out_tp, in_t, out);
            self.data.vector_def(&mut self.lexer, in_t);
            let vector_end = if matches!(in_type, Type::Vector(_, _)) {
                Some((src_coll.clone(), iter_var))
            } else {
                None
            };
            // Reset `val` so the build creates a fresh result vector instead of seeding it
            // with the destination — the same reset `parse_map` makes for the same reason.
            *val = Value::Null;
            return self.build_comprehension_code(
                out,
                &Value::Var(out),
                out_elm,
                in_t,
                &in_type,
                &var_tp,
                for_var,
                for_next,
                pre_var,
                vector_end,
                fill,
                create_iter,
                if_step,
                body,
                val,
                false,
                false,
                true,
                tp,
            );
        }
        let is_plain_local_target = !is_field && !matches!(val.unspan(), Value::Call(_, _));
        // (I-Comp) Both shortcuts below build THROUGH the destination, so neither can serve
        // a comprehension that reads it — `[for i in 0..a.len() { 5 }]` assigned to `a`
        // took the fill with a count read after the destination was already emptied.  The
        // general loop path below defers the repoint and answers these correctly; a
        // self-reading comprehension simply forgoes the optimisation.  Pass-2 only, so
        // skipping them cannot move the two passes apart.
        let self_read = self.comprehension_reads_target(
            vec,
            is_var,
            &[&fill, &create_iter, &for_next, &if_step, &body],
        );
        // O8.5 (loft#884): a comprehension whose body does not vary with the loop
        // variable IS a fill, so emit the repeat literal's one-template-plus-copy
        // instead of the per-element record protocol.  Tried before the unroll
        // below because a loop-invariant body would unroll to N copies of one
        // value — the same vector, built with N times the IR.
        //
        // Only where the vector being built is a PLAIN LOCAL — not a struct field,
        // not a captured collection, not an indexed element.  The container defect
        // that first forced this narrowing is fixed (loft#892: `OpAppendCopy` was
        // handed the enclosing record rather than the field's own handle, so
        // `Pair { a: [0; 3], b: [0; 3] }` built `a` with seven elements and `b` with
        // one), and `new_record` now derives one container for all three append ops.
        //
        // The restriction stays because the remaining question is a different one and
        // is unmeasured: a captured target may be a KEYED collection, where "append n
        // copies of one element" is not the same operation as n inserts — a hash or a
        // sorted set dedups them.  Widening this to addressed containers is worth
        // doing, but it needs that case decided and timed on its own, not inherited
        // from a bug fix.
        if is_plain_local_target
            && !self_read
            && let Some(fill) =
                self.try_const_fill_comprehension(range_bounds.as_ref(), &body, &if_step)
        {
            let parent_tp = &Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
            let (tp, ls) =
                self.build_vector_list(val, parent_tp, elm, vec, &fill, in_t, tp, is_var, is_field);
            *val = if !is_var && !is_field {
                v_block(ls, tp.clone(), "Const fill comprehension")
            } else {
                Value::Insert(ls)
            };
            return tp;
        }
        // O8.5: try const-unrolling for [for i in A..B [if cond] { expr(i) }].
        // If the range bounds are const and the body folds for every i,
        // emit a pre-computed literal vector instead of a runtime loop.
        if matches!(in_t, Type::Integer(_))
            && !self_read
            && let Some(unrolled) = self.try_const_unroll_comprehension(
                for_var,
                range_bounds.as_ref(),
                &body,
                &if_step,
                in_t,
            )
        {
            let parent_tp = &Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
            let (tp, ls) = self.build_vector_list(
                val, parent_tp, elm, vec, &unrolled, in_t, tp, is_var, is_field,
            );
            *val = if !is_var && !is_field {
                v_block(ls, tp.clone(), "Const comprehension")
            } else {
                Value::Insert(ls)
            };
            return tp;
        }
        // Second pass: build the append-in-loop bytecode.
        // For field assignments (vec == u16::MAX), pass the original field expression
        // so comprehension code can reference it instead of Value::Var(u16::MAX).
        let vec_expr = if vec == u16::MAX {
            val.clone()
        } else {
            Value::Var(vec)
        };
        self.build_comprehension_code(
            vec,
            &vec_expr,
            elm,
            in_t,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            pre_var,
            if matches!(in_type, Type::Vector(_, _)) {
                Some((src_coll.clone(), iter_var))
            } else {
                None
            },
            fill,
            create_iter,
            if_step,
            body,
            val,
            is_var,
            is_field,
            block,
            tp,
        )
    }

    /// Build the second-pass bytecode for a `[for ... { body }]` vector comprehension.
    // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_comprehension_code(
        &mut self,
        vec: u16,
        vec_expr: &Value,
        elm: u16,
        in_t: &Type,
        in_type: &Type,
        var_tp: &Type,
        for_var: u16,
        mut for_next: Value,
        pre_var: Option<u16>,
        // The SOURCE collection and its index variable, for length-based termination;
        // `None` when the source is not a vector.  The source EXPLICITLY, because
        // `vec_expr` is the DESTINATION being appended to: `map` builds a result vector
        // and hands that in, and taking ITS length would measure the thing that grows
        // each iteration.  Distinct from `pre_var` too, which is text's character index —
        // a text loop's `iter_var` is a byte POSITION and cannot answer "how many
        // elements so far" (loft#1000).
        vector_end: Option<(Value, u16)>,
        mut fill: Value,
        mut create_iter: Value,
        mut if_step: Value,
        mut body: Value,
        val: &mut Value,
        is_var: bool,
        is_field: bool,
        block: bool,
        mut tp: Type,
    ) -> Type {
        // Per-iteration: OpNewRecord / set_field / OpFinishRecord pattern.
        let ed_nr = self.data.type_def_nr(in_t);
        // @PLAN58 cluster IV: use the SAME element-type `known` as the proven
        // `vv += [inner]` path (`new_record`), not `vector(def(ed_nr).known_type)`
        // which over-wraps one level for a nested element — making `record_new`
        // stride the outer slot by the 4-byte handle while the read strides by 8
        // (off-by-one).  `vector_of(in_t)` gives the element type; for a sub-4
        // inner (boolean) pass the outer vector type so the handle strides by 4.
        let elem_known = self.vector_of(in_t);
        let known = Value::Int(i32::from(if elem_known == u16::MAX {
            0
        } else if matches!(in_t, Type::Vector(_, _))
            && self.database.size(self.database.content(elem_known)) < 4
        {
            self.database.vector(elem_known)
        } else {
            elem_known
        }));
        let fld = Value::Int(i32::from(u16::MAX));
        // The per-iteration yield slot.  When the body is a bare variable that is itself a
        // BORROW, this slot holds that borrow and must say so: a variable bound to a borrow
        // is a borrow, and one created with a bare element type carries no deps, so scope
        // handling reads it as an OWNER and emits `OpFreeRef` at the end of every iteration.
        //
        // `filter` is where that bites, because its body IS the loop element
        // (`body = Value::Var(for_var)` — the identity yield that makes it a filter rather
        // than a map): each kept element freed the SOURCE's record.  Silent on a scalar
        // element, whose copy is the value itself, and visible on a `vector<vector<T>>`,
        // where a later loop over the same source then yielded nothing at all — on both
        // backends.  `map` was never affected: its body is a call, whose result this slot
        // really does own.
        //
        // Narrowed to a bare `Var` on purpose: that is the only shape where the slot is an
        // ALIAS rather than a fresh value, so it is exactly the case where the free is wrong.
        let comp_tp = match &body {
            Value::Var(v) if *v != u16::MAX && !self.vars.tp(*v).depend().is_empty() => {
                in_t.depending(*v)
            }
            _ => in_t.clone(),
        };
        let comp_var = self.create_unique("comp", &comp_tp);
        // @P325 — coroutine comprehensions `[for v in gen() { … }]` had NO
        // termination check in the loop body (the `!matches!(Iterator)`
        // guard below skipped it entirely), so they ran forever appending
        // to the result vector until the underlying store overflowed its
        // 2 GiB word limit (`src/store.rs:643`).  Mirror the @P327 fix in
        // `collections.rs::iter_for`: when iterating a coroutine, emit
        // `OpCoroutineExhausted(__gen_N)` as the loop's break condition.
        // The generator var is the first arg of `OpCoroutineNext` inside
        // `for_next` (`Set(for_var, OpCoroutineNext(__gen_N, value_size))`).
        //
        // Peeled at all three levels, per `Value::unspan`'s rule for a site that
        // discriminates on specific variants: a `Span` around any of them hides the shape,
        // the generator var is not found, and the loop silently loses the break — which is
        // the unbounded append @P325 was.
        let coroutine_gen_var = if matches!(in_type, Type::Iterator(_, _))
            && let Value::Set(_, rhs) = for_next.unspan()
            && let Value::Call(_, next_args) = rhs.unspan()
            && let Some(Value::Var(v)) = next_args.first().map(Value::unspan)
        {
            *v
        } else {
            u16::MAX
        };
        // (I-Comp) A comprehension that READS its own destination cannot be built through
        // that destination.  The fresh store `create_vector` splices in for a `=` repoints
        // the variable BEFORE the loop, so the range bound, the source and the body all
        // read the empty result being built instead of the value they named — silently, and
        // on both backends.  Take that store's ops here instead and hold back the ONE op
        // that repoints the destination, so the loop appends through the store's own handle
        // while every read still resolves through the destination's previous store.
        //
        // Split out of `vector_db` rather than rebuilt, so the argument, keyed and rebind
        // guards — and the rebind pre-free — stay in their one home.  An empty answer means
        // there is no fresh store at this site, and nothing to defer.
        let deferred = if self.comprehension_reads_target(
            vec,
            is_var,
            &[&fill, &create_iter, &for_next, &if_step, &body],
        ) {
            let mut ops = self.vector_db(in_t, vec);
            ops.iter()
                .position(|o| matches!(o.unspan(), Value::Set(s, _) if *s == vec))
                .map(|at| {
                    let Value::Set(_, handle) = ops.remove(at).unspan().clone() else {
                        unreachable!("position matched a Set")
                    };
                    // Snapshot what the destination holds NOW, into a store of its own,
                    // and point every READ at the snapshot.  `OpDatabase` reuses the slot's
                    // current store (`clear` + `claim`), so on a second execution of this
                    // site the buffer store IS the one the destination was left pointing
                    // at — clearing it would empty the value the loop is about to read.
                    // The snapshot is taken before that clear, which is what makes the
                    // idiom survive a surrounding loop.  Same cure, same reason, as the
                    // trailing self-reference `v = a + v` in `create_vector`; one home with
                    // the literal's snapshot (`snapshot_read_destination`).
                    let mut snap = Vec::new();
                    let mut parts = [
                        &mut fill,
                        &mut create_iter,
                        &mut for_next,
                        &mut if_step,
                        &mut body,
                    ];
                    if let Some(snapshot) = self.snapshot_read_destination(
                        vec,
                        &Value::Var(vec),
                        is_var,
                        false,
                        in_t,
                        &mut parts,
                    ) {
                        snap.extend(snapshot);
                    }
                    snap.extend(ops);
                    (snap, *handle)
                })
        } else {
            None
        };
        // The container the loop appends through: the deferred store's own handle, or the
        // destination itself when nothing reads it back.
        let vec_expr = &match &deferred {
            Some((_, handle)) => handle.clone(),
            None => vec_expr.clone(),
        };
        let mut lp = vec![for_next];
        if matches!(in_type, Type::Text(_))
            && let Some(idx) = pre_var
        {
            // loft#755 — a comprehension / par materialisation over text
            // terminates on the POSITION, never on the character read: a NUL
            // the text really holds is `character`'s null.  Same fact, same
            // home as the plain `for c in s` loop.
            let tcn = self.data.def_nr("OpTextCharacterNullable");
            let coll = super::collections::find_text_coll(lp.first().unwrap_or(&Value::Null), tcn)
                .unwrap_or_else(|| create_iter.clone());
            for step in self.text_loop_break(&coll, idx) {
                lp.push(step);
            }
        } else if let Some((src, index_var)) = vector_end
            && matches!(in_type, Type::Vector(_, _))
        {
            // loft#1000 — a VECTOR ends on its LENGTH, never on the element's value.
            // The same rule the `for` STATEMENT already uses, and for the same reason:
            // a null the vector really holds shares the out-of-bounds sentinel, and a
            // `value struct` element is deep-copied into a fresh record on bind (@PLN101),
            // so the bound local is never null and a null test can never fire at all.
            // `map`, `filter` and the comprehension all route their break through here.
            for step in self.vector_loop_break(&src, index_var) {
                lp.push(step);
            }
        } else if !matches!(in_type, Type::Iterator(_, _)) {
            let mut test_for = Value::Var(for_var);
            self.convert(&mut test_for, var_tp, &Type::Boolean);
            test_for = self.cl("OpNot", &[test_for]);
            lp.push(v_if(
                test_for,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            ));
        } else if coroutine_gen_var != u16::MAX {
            let test_exhausted = self.cl("OpCoroutineExhausted", &[Value::Var(coroutine_gen_var)]);
            lp.push(v_if(
                test_exhausted,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            ));
        }
        if if_step != Value::Null {
            lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
        }
        lp.push(v_set(comp_var, body));
        lp.push(v_set(
            elm,
            self.cl(
                "OpNewRecord",
                &[vec_expr.clone(), known.clone(), fld.clone()],
            ),
        ));
        // @PLAN58 cluster IV: a NESTED comprehension's body is a vector (a 12-byte
        // DbRef handle).  The scalar `set_field(usize::MAX)` path emits `OpSetInt4`
        // (4 of 12 bytes) → eval-stack skew → garbage rec-id into the locked
        // CONST_STORE.  Deep-copy the inner record instead.  Scalar elements keep
        // `set_field`.
        if matches!(in_t, Type::Vector(_, _)) {
            lp.push(self.cl(
                "OpSetInt4",
                &[Value::Var(elm), Value::Int(0), Value::Int(0)],
            ));
            // The ELEMENT type of the outer vector — one derivation, shared with
            // the `+=` append and the slice.  Deriving it as
            // `vector(db_type(inner))` re-entered `db_type`'s scalar arm, which
            // sizes a narrow int as if it were nullable (`u8` -> 2, `u16` -> 4 ->
            // plain `integer`) and so named a DIFFERENT row than the one the
            // element was written into (loft#624 nested).
            let type_nr = Value::Int(i32::from(
                self.data
                    .vector_element_type(in_t, &mut self.database)
                    .unwrap_or(u16::MAX),
            ));
            lp.push(self.cl(
                "OpCopyRecord",
                &[Value::Var(comp_var), Value::Var(elm), type_nr],
            ));
        } else if let Some(op) = self.narrow_elm_set(in_t, elm, &Value::Var(comp_var)) {
            // A NARROW element gets the store op for its own width — the third site
            // `narrow_elm_set` exists for, beside the `+=` append and the slice.
            // `set_field` below dispatches on the element DEF, and a narrow integer is
            // an ALIAS of `integer`, so it picked the wide 8-byte `OpSetInt` for a
            // 4-byte slot: every element overwrote the next, and past the initial
            // allocation the write reached the vector's own bookkeeping and
            // `vector_add` stopped terminating. `[for i in 0..13 { i as i32 }]` hung
            // while 12 returned instantly, and the `+=` loop — already routed here —
            // was fine at any size (loft#869).
            lp.push(op);
        } else {
            lp.push(self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), Value::Var(comp_var)));
        }
        lp.push(self.cl(
            "OpFinishRecord",
            &[vec_expr.clone(), Value::Var(elm), known, fld],
        ));
        let mut for_steps: Vec<Value> = Vec::new();
        if fill != Value::Null {
            for_steps.push(fill);
        }
        if let Some(idx_var) = pre_var {
            for_steps.push(v_set(idx_var, Value::Int(0)));
        }
        for_steps.push(create_iter);
        for_steps.push(v_loop(lp, "For comprehension"));
        let mut ls: Vec<Value> = Vec::new();
        if block {
            ls.extend(self.vector_db(in_t, vec));
            // After vector_db, vec's type carries the db dependency.  Propagate that
            // into tp so that (a) the block's result type keeps the db alive until the
            // block exits, and (b) the caller receives the correct Vector<T,[db]> type,
            // preventing scopes from emitting a redundant OpFreeRef for the result variable.
            if let Type::Vector(elem, _) = &tp {
                tp = Type::Vector(elem.clone(), Deps::frame(self.vars.tp(vec).depend()));
            }
        }
        ls.extend(for_steps);
        if let Some((alloc, handle)) = deferred {
            // Allocate and zero the store first, run the loop, and only THEN point the
            // destination at what the loop built.  The ordering is the whole fix.
            for (i, op) in alloc.into_iter().enumerate() {
                ls.insert(i, op);
            }
            ls.push(v_set(vec, handle));
        } else if self.vector_needs_db(vec, in_t, is_var) {
            let db = self.insert_new(vec, elm, in_t, &mut ls);
            self.vars.depend(vec, db);
        } else if !is_field && !is_var && *val != Value::Null {
            ls.insert(0, v_set(vec, val.clone()));
        }
        if !is_var && !is_field {
            ls.push(Value::Var(vec));
        }
        *val = if block || (!is_var && !is_field) {
            v_block(ls, tp.clone(), "Vector comprehension")
        } else {
            Value::Insert(ls)
        };
        tp
    }

    /**
    Fill a structure (vector) with values. This can be done in different situations:
    - On a new variable, this creates a variable pointing to a structure with the vector.
    - As a stand-alone expression, this creates a new structure of type vector.
    - On an existing variable, this fills (or replaces) the vector with more elements.
    - On a field inside a structure, this fills any data structure with more elements.
    */
    // <vector> ::= '[' <expr> [ ';' <size-expr>]{ ',' <expr> [ ';' <size-expr> } ']'
    pub(crate) fn parse_vector(
        &mut self,
        var_tp: &Type,
        val: &mut Value,
        parent_tp: &Type,
    ) -> Type {
        let mut assign_tp = var_tp.content();
        // @PLN25 E2 — a KEYED collection's `content()` yields `Reference(__nullable<S>)`
        // (Hash/Sorted/Index wrap the content def in a Reference), but literal-element
        // construction needs the inline `Enum(.., true)` form so each `S{…}` builds the
        // `Some` variant (a vector's `content()` already yields `Enum(.., true)` directly,
        // which is why `vector<S>` fields work but keyed fields hit "cannot store S in
        // vector<__nullable<S>>").  Normalize so `hash<S[k]> += [S{…}]` and keyed-field
        // initialisers take the same Some-construction path as a vector.  Inert gate-off
        // (no content def is ever a `__nullable<` enum).
        if let Type::Reference(d, dep) = &assign_tp
            && self.data.def(*d).name.starts_with("__nullable<")
        {
            assign_tp = Type::Enum(*d, true, dep.clone());
        }
        // @PLN25 storage-vs-access-nullability — INFERRED literals stay DENSE. With no
        // declared element type, an inferred `v = [S{…}]` builds a dense `vector<S>`;
        // nullability is never inferred from a literal's shape (the old struct-literal
        // PEEK is retired). A nullable element comes only from a DECLARED `vector<?S>`
        // (the `?` opt-in), where the checking-mode element type + `(C-Var)` `S ⤳ ?S`
        // wraps each element. Keeps type formation substitution-stable (parametricity).
        // @P315 — `declared` is true when the element type comes from a typed
        // target (typed local / struct field), false when it is inferred from
        // an untyped literal.  A declared element type must NOT be silently
        // promoted to a wider type by `parse_item` (that changes the element
        // storage width and loses data); require an explicit `as` cast.
        // loft#944 — and NOT declared when it carries an unresolved member.  `declared`
        // means "the author wrote this element type, so do not silently widen it", and a
        // pass-1 type holding a forward-referenced member is not something the author
        // wrote — it is the parser's placeholder for it.  Treating it as declared made
        // pass 2 refuse its own resolved literal: "cannot store (integer, Q) elements in a
        // vector<(integer, unknown)>".  `is_unknown()` alone sees only the bare and
        // vector-wrapped forms.
        let declared =
            !assign_tp.is_unknown() && !crate::data::Data::type_has_unresolved(&assign_tp);
        let is_field = self.is_field(val);
        let is_var = matches!(val, Value::Var(_));
        // Empty `[]`.  A new variable / struct field keeps the lightweight
        // placeholder (its store is zero-initialised elsewhere), as does an
        // UNTYPED standalone `[]` (no element-type hint to size a store).  But a
        // TYPED standalone `[]` — e.g. `return []` with the function's return
        // type threaded in (@P365) — falls through to the normal construction
        // path so it materialises a REAL empty vector store; the placeholder
        // (`Insert([Null])`) otherwise lowers to `()` → native E0308 / interpret
        // garbage-handle crash.
        if self.lexer.peek_token("]") {
            if is_var {
                self.lexer.has_token("]");
                *val = Value::Insert(vec![]);
                return Type::Rewritten(Box::new(var_tp.clone()));
            }
            if is_field {
                // The field is already zero-initialized by OpDatabase; nothing to
                // emit.  Wrapping the OpGetField result in Value::Insert would
                // leave a dangling 12-byte DbRef on the expression stack.
                self.lexer.has_token("]");
                *val = Value::Insert(vec![]);
                return var_tp.clone();
            }
            if assign_tp.is_unknown() {
                self.lexer.has_token("]");
                *val = Value::Insert(vec![val.clone()]);
                return var_tp.clone();
            }
            // Typed standalone empty — fall through; `]` is consumed by the
            // `self.lexer.token("]")` at the end of the construction path.
        }
        let block = !is_field && !matches!(val, Value::Var(_));
        let vec = if is_field {
            u16::MAX
        } else if let Value::Var(nr) = val {
            *nr
        } else if is_keyed(var_tp) {
            // loft#703 — a keyed literal in VALUE position (`fn mk() -> hash<K[k]> { […] }`,
            // `f([K { … }])`) has no destination variable to build through, so give it one
            // of its own AT THE KEYED TYPE.  A `vector<K>` temp here is what made every
            // such position report "expected hash<K[k]>, got vector<K>".
            //
            // Minted as a FUNCTION-scoped work-ref in its own `__kvb_N` namespace: the
            // store is this accumulator's own (a keyed collection has no wrapper record
            // to hold it), so whoever owns the variable owns the store, and the
            // function-exit sweep is what frees it — a block-local temp left an argument
            // literal (`f([K { … }])`) leaking.  See `Function::work_kvb` for why
            // neither of the existing namespaces will do.
            //
            // WITHOUT the destination's deps, because they are the destination's and not
            // this accumulator's: the store is minted here.  A `??` default whose SUBJECT
            // is a borrowed field (`b.c ?? []`) reaches here with `var_tp` naming `b`, and
            // that dep is what the exit sweep reads as *"someone else owns this"* — so it
            // freed nothing and the literal leaked one store per evaluation, unbounded in
            // a loop.  The `vector` twin three lines below has always minted dep-free.
            let kvb_tp = var_tp.without_deps();
            self.vars.work_keyed(&kvb_tp, &mut self.lexer)
        } else {
            self.create_unique(
                "vec",
                &Type::Vector(Box::new(assign_tp.clone()), Deps::frame(parent_tp.depend())),
            )
        };
        let mut in_t = assign_tp.clone();
        let mut res = Vec::new();
        let elm = self.unique_elm_var(parent_tp, &assign_tp, vec);
        if is_field {
            // elm is a reference INTO an existing field's store — the owning struct's
            // variable already emits FreeRef at scope exit.  Suppress FreeRef for elm
            // to prevent a double-free.
            self.vars.set_skip_free(elm);
        }
        // A typed standalone empty `[]` (the @P365 fall-through above) has no
        // items and no `for` comprehension — skip straight to the empty build.
        if !self.lexer.peek_token("]") {
            // Handle [for n in range [if cond] { body }] vector comprehension
            if self.lexer.peek_token("for") {
                self.lexer.has_token("for");
                let tp = self
                    .parse_vector_for(vec, elm, &mut in_t, val, is_var, is_field, block, parent_tp);
                self.lexer.token("]");
                return tp;
            }
            if let Some(early) = self.collect_vector_items(elm, &mut in_t, declared, &mut res) {
                return early;
            }
        }
        // convert parts to the common type
        if in_t == Type::Null {
            return in_t;
        }
        // loft#703 — a KEYED destination keeps its own type.  The elements are added
        // THROUGH it: `new_record` reads the local's type to pick `hash::add` /
        // `sorted_new` / `tree::add`, which is how `h += [K { … }]` already builds one.
        // Retyping it to `vector<K>` here is what made `h: hash<K[k]> = [K { … }]`
        // report a type change, so loft had no way to write a non-empty keyed
        // collection as a VALUE at all — only through a keyed destination that already
        // existed (a struct-literal field, or a `+=` onto a built collection).
        let keyed_dest = self.keyed_local(vec);
        // The INFERRED half of loft#923's refusal: this literal's element type was never
        // written, so the declaration chokepoint in `definitions.rs` never saw it.  A keyed
        // destination is a different construct — `h = [K { … }]` builds THROUGH `h`
        // (loft#703) and its elements are `K`, not collections — so it is excluded here the
        // same way it is below.
        if !keyed_dest && self.refuse_keyed_vector_element(&in_t) {
            self.lexer.token("]");
            *val = Value::Insert(Vec::new());
            return Type::Unknown(0);
        }
        let struct_tp = Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()));
        if !is_field && !keyed_dest {
            self.vars
                .change_var_type(vec, &struct_tp, &self.data, &mut self.lexer);
            self.data.vector_def(&mut self.lexer, &in_t);
        }
        let tp = if keyed_dest {
            // `.base()` — a CONSTRUCTED literal is never absent, so it does not wear the
            // destination's nullability.  A keyed literal is built THROUGH its destination
            // (loft#703), so it reports the destination variable's type; taken whole, a
            // `hash<E[k]>?` destination made the literal's own type `Optional(Hash(…))`, and
            // loft#1210's `(N-Store)` append gate then read the construction as an
            // un-discharged nullable SOURCE and warned about code that is correct — a
            // `warning`, which is the tier that GATES a library's CI (loft#1229).  The vector
            // branch below has always constructed its type fresh, which is why only the keyed
            // spelling carried this.
            self.vars.tp(vec).base().clone()
        } else {
            Type::Vector(Box::new(in_t.clone()), Deps::frame(parent_tp.depend()))
        };
        // A literal that READS its destination reads what the destination held when the
        // statement began (`I-Comp`, @FR-O-Detach) — take the snapshot before the build.
        let snapshot = {
            let mut parts: Vec<&mut Value> = res.iter_mut().collect();
            self.snapshot_read_destination(vec, val, is_var, is_field, &in_t, &mut parts)
        };
        let (tp, mut ls) =
            self.build_vector_list(val, parent_tp, elm, vec, &res, &in_t, tp, is_var, is_field);
        if let Some(snapshot) = snapshot {
            self.build_snapshot_len = snapshot.len();
            for (i, op) in snapshot.into_iter().enumerate() {
                ls.insert(i, op);
            }
        }
        self.lexer.token("]");
        if block {
            *val = v_block(ls, tp.clone(), "Vector");
        } else {
            *val = Value::Insert(ls);
        }
        tp
    }

    /// Parse comma-separated vector items inside `[...]`, returning an early error type on failure.
    pub(crate) fn collect_vector_items(
        &mut self,
        elm: u16,
        in_t: &mut Type,
        declared: bool,
        res: &mut Vec<Value>,
    ) -> Option<Type> {
        loop {
            if let Some(value) = self.parse_item(elm, in_t, declared, res) {
                return Some(value);
            }
            if self.lexer.has_token(";")
                && let Some(value) = self.parse_multiply(res)
            {
                return Some(value);
            }
            if !self.lexer.has_token(",") {
                break;
            }
            if self.lexer.peek_token("]") {
                break;
            }
        }
        None
    }

    /// Build the instruction list for a parsed vector literal; returns `(tp, ls)`.
    #[allow(clippy::too_many_arguments)] // parser helper threading IR-construction params alongside &mut self; no sensible grouping reduces the count
    pub(crate) fn build_vector_list(
        &mut self,
        val: &mut Value,
        parent_tp: &Type,
        elm: u16,
        vec: u16,
        res: &[Value],
        in_t: &Type,
        mut tp: Type,
        is_var: bool,
        is_field: bool,
    ) -> (Type, Vec<Value>) {
        let mut ls = Vec::new();
        // loft#1160 / loft#1161 — a write spelled through a variant's payload BINDING means
        // what the same write spelled through the FIELD means, so resolve the binding back to
        // the field access it was projected from and build the ordinary field append.
        //
        // It has to happen HERE rather than at `new_record` below, because the two halves that
        // go wrong are decided on either side of that call.  Treated as a bare local, the
        // binding gets a store of its own minted for it (`vector_db`, and `vector_needs_db`
        // after) and is REBOUND to it, so the append landed in a fresh store the block threw
        // away — the write never reached the subject at all (loft#1161).  And `new_record` was
        // handed no field, so `Stores::record_finish` had no `other_indexes` to walk and the
        // record reached the member the binding named and no sibling (loft#1160).  One
        // substitution, above both, and everything downstream is on the path it already has
        // for a field.
        //
        // A capture spanning ALTERNATIVES (`is A | B { f }`) is deliberately absent from
        // `mv_field_origin`: it picks its origin from the runtime tag, so it has no one field
        // to be resolved to.
        let binding_origin = if self.first_pass || is_field {
            None
        } else {
            self.vars.mv_field_origin.get(&vec).cloned()
        };
        //
        // `vec` is deliberately KEPT — the binding is a real variable and everything below
        // that reads it (the pre-allocation, and the `vars` lookups under it) needs one; only
        // the two store-MINTING branches are suppressed, which is the half that was wrong.
        // Blanking it to `u16::MAX` instead reads as "no variable" to some of those lookups
        // and panics on `variables[65535]`.
        let substituted = binding_origin.is_some();
        let mut owned_origin;
        let mut owned_parent;
        let (val, parent_tp, is_var, is_field) = match binding_origin {
            Some((origin, origin_parent)) => {
                owned_origin = origin;
                owned_parent = origin_parent;
                (&mut owned_origin, &mut owned_parent, false, true)
            }
            None => (val, &mut parent_tp.clone(), is_var, is_field),
        };
        let parent_tp: &Type = parent_tp;
        // loft#944 — in pass 1 an element type naming a type declared LOWER in the file is
        // still a stub, so there is no record shape and every step below asks for one:
        // `new_record` reports Fatal, and the append path reaches `data.def(u32::MAX)`.
        // Nothing built here survives anyway — pass 1's IR is regenerated in pass 2, which
        // sees the resolved element and builds it properly.  Emit nothing rather than
        // half of it.
        if self.first_pass && crate::data::Data::type_has_unresolved(in_t) {
            return (tp, ls);
        }
        // loft#703 — a keyed literal in VALUE position allocates its accumulator's store
        // outright rather than as a side effect of the first `OpNewRecord`.  Two things
        // need it said: an EMPTY one has no element to allocate it at all (the temp
        // reached codegen with no definition — "Incorrect var _vec_1[65535] versus 8"),
        // and a store that appears only as a side effect is a store the scope pass never
        // sees the accumulator OWN, so an argument literal (`f([K { … }])`) leaked it.
        // A keyed LOCAL is excluded: its store comes from its own declaration, and a
        // second allocation here would orphan the first.
        let tuple_place = match val.unspan() {
            Value::TupleGet(t, i) => Some((*t, *i)),
            _ => None,
        };
        if tuple_place.is_some() && !self.first_pass {
            // The accumulator is SEEDED from the place, so its own null-init must not allocate
            // a store: `OpInitRef` claims one eagerly, the seed overwrites the handle one
            // statement later, and the claimed store is orphaned.  `--interpret` never noticed
            // (its init is lazy); `--native` leaked one per append.  Same precedent as
            // `vector_db`'s rebind backing, whose `OpDatabase` is likewise conditional.
            self.vars.mark_inline_ref(vec);
        }
        if !self.first_pass && !is_var && tuple_place.is_none() && self.keyed_local(vec) {
            let keyed_tp = self.vars.tp(vec).clone();
            if let Some(kt) = self.keyed_known_type(&keyed_tp) {
                ls.push(v_set(vec, Value::Null));
                ls.push(self.cl("OpDatabase", &[Value::Var(vec), Value::Int(i32::from(kt))]));
            }
        }
        // Only create a fresh database record here when the variable has no existing
        // one (dep is empty).  For `v += [...]` the variable already has a dep from
        // the initial `=` assignment; calling vector_db again would reset v to an
        // empty record and discard the existing elements.  create_vector handles
        // the `=` re-assignment case by calling vector_db unconditionally.
        //
        // loft#1219 — an empty dep is a PARSE-TIME stand-in for the runtime question
        // *does this variable already hold a store?*, and a local declared `= null` breaks
        // the correspondence: it never got the dep the comment above assumes, so the mint
        // lands at the APPEND site.  Outside a loop that is right and the site runs once;
        // inside one it re-executes per iteration, orphaning the previous store, and
        // `v: vector<integer>? = null; for i in 0..3 { v += [i] }` ended holding the LAST
        // element alone.  No parse-time predicate can answer it — the honest answer is "no"
        // on the first iteration and "yes" on every later one — so an APPEND asks at
        // runtime instead, and mints only into a slot that is still absent.
        //
        // A REPLACE keeps the unguarded mint: `v = [...]` in a loop must REBUILD each time,
        // and guarding it would turn a rebuild into an append.  `assign_replaces` is that
        // distinction and it is already threaded; `assign_target` pins the pair to THIS
        // variable, so a literal in some other position cannot read a stale flag.
        // @FR-O-Proxy asks alloc — an empty dep list stands in for *does this variable
        // already hold a store?*, and the answer decides whether to MINT one (and whether the
        // mint is guarded).  Nothing here releases; the loft#1219 note above is about a mint
        // that runs too often, not a free.
        if !substituted && self.vars.tp(vec).depend().is_empty() {
            let db_ops = self.vector_db(in_t, vec);
            let guarded_mint = !self.first_pass
                && !db_ops.is_empty()
                && !self.assign_replaces
                && vec != u16::MAX
                && self.assign_target == vec;
            if guarded_mint {
                // The `OpDatabase` is now conditional, so the backing's entry null-init has
                // to be the non-allocating sentinel — otherwise the path that skips the mint
                // leaks an eagerly-allocated store.  Same reason `vector_db`'s rebind branch
                // marks it, for the same conditional shape.
                if let Some(&db) = self.vars.tp(vec).depend().last() {
                    self.vars.mark_inline_ref(db);
                }
                let absent = self.cl("OpVectorIsNull", &[Value::Var(vec)]);
                ls.push(v_if(absent, Value::Insert(db_ops), Value::Null));
            } else {
                ls.extend(db_ops);
            }
        }
        // O8.1a: pre-allocate vector capacity when the element count is known
        // at compile time.  This eliminates resize calls in vector_append.  A keyed
        // local (loft#703) has no vector to size — its adds go through `hash::add` and
        // friends, which grow the keyed store themselves.
        if !self.first_pass && !res.is_empty() && vec != u16::MAX && !self.keyed_local(vec) {
            let ed_nr = self.data.type_def_nr(in_t);
            if ed_nr != u32::MAX {
                let known = self.data.def(ed_nr).known_type();
                if known != u16::MAX {
                    let elem_size = self.database.size(known);
                    if elem_size > 0 {
                        ls.push(self.cl(
                            "OpPreAllocVector",
                            &[
                                Value::Var(vec),
                                Value::Int(res.len() as i32),
                                Value::Int(i32::from(elem_size)),
                            ],
                        ));
                    }
                }
            }
        }
        ls.extend(self.new_record(val, parent_tp, elm, vec, res, in_t));
        if let Some((tuple_var, idx)) = tuple_place {
            // Write back ONLY when the place is still null.  The accumulator was SEEDED from
            // the place, so for a non-null element the two already name one store and the
            // append has landed in it — writing it back would put a second owner on a store
            // that has one.  A null element is the only case where the accumulator holds a
            // store the place has never seen.
            let slot = Value::TupleGet(tuple_var, idx);
            let test = self.cl("OpVectorIsNull", std::slice::from_ref(&slot));
            let put = Value::TuplePut(tuple_var, idx, Box::new(Value::Var(vec)));
            ls.push(crate::data::v_if(test, put, Value::Null));
        }
        if !substituted
            && !self.first_pass
            && vec != u16::MAX
            && !self.vars.is_argument(vec)
            && self.vector_needs_db(vec, in_t, is_var)
        {
            let db = self.insert_new(vec, elm, in_t, &mut ls);
            self.vars.depend(vec, db);
            tp = tp.depending(db);
        } else if !is_field && !is_var && *val != Value::Null {
            ls.insert(0, v_set(vec, val.clone()));
        }
        if !is_var && !is_field {
            ls.push(Value::Var(vec));
            for d in self.vars.tp(vec).depend() {
                tp = tp.depending(d);
            }
        }
        (tp, ls)
    }

    /// loft#884 — recognise a comprehension that is really a FILL, and describe it
    /// the way the repeat literal `[x; n]` is described: one template element
    /// followed by `Value::Return(count)`, which `new_record` lowers to a single
    /// `OpAppendCopy`.
    ///
    /// The test for "this is a fill" is that the body folds to a constant WITHOUT
    /// binding the loop variable. That one check carries both halves of the
    /// requirement: a body reading `i` cannot fold, and neither can a body that
    /// calls anything, so `[for _ in 0..n { f() }]` still calls `f()` n times.
    /// Loop-invariant is not enough on its own — only PURE is.
    ///
    /// `bounds` must be this comprehension's own range (see the snapshot in
    /// `parse_vector_for`); `None` means the iterable was not a range, and its
    /// length is then not `till - from`.
    ///
    /// Returns `None` for anything unrecognised, which just leaves the runtime
    /// loop in place.
    fn try_const_fill_comprehension(
        &mut self,
        bounds: Option<&(Value, Value)>,
        body: &Value,
        if_step: &Value,
    ) -> Option<Vec<Value>> {
        // A filter is not a fill: it decides per element how many there are.
        if *if_step != Value::Null {
            return None;
        }
        let (from, till) = bounds?;
        // A comprehension body arrives wrapped in the block `parse_block` built for
        // it, and `const_eval` has no arm for one — unwrap it here rather than
        // there, because teaching the shared folder about blocks also wakes O8.5's
        // const-UNROLL, which has never run for exactly this reason and is wrong in
        // positions this lowering declines (loft#892).  Exactly one operator: a
        // second one is a statement, and a statement is what a constant may not
        // stand in for.  Position markers are not operators.
        let expr = match body.unspan() {
            Value::Block(bl) => {
                let mut ops = bl
                    .operators
                    .iter()
                    .filter(|o| !matches!(o.unspan(), Value::Line(_)));
                let only = ops.next()?;
                if ops.next().is_some() {
                    return None;
                }
                only.unspan()
            }
            other => other,
        };
        let value = crate::const_eval::const_eval(expr, &self.data)?;
        // `till - from`, with the common `0..n` needing no subtraction at all so
        // the emitted count is the caller's own expression.
        let count = if matches!(
            crate::const_eval::const_eval(from, &self.data),
            Some(Value::Int(0))
        ) {
            till.clone()
        } else {
            self.conv_op("-", till.clone(), from.clone(), I32.clone(), I32.clone())
        };
        Some(vec![value, Value::Return(Box::new(count))])
    }

    /// O8.5: try to const-unroll a comprehension into a literal vector.
    /// Returns Some(vec of folded values) if successful, None to fall back to runtime loop.
    ///
    /// `bounds` is this comprehension's own range, snapshotted in
    /// `parse_vector_for` rather than read from `last_range_*` at this point: the
    /// body has been parsed by now, and any range inside it has already overwritten
    /// that parser-wide state.
    fn try_const_unroll_comprehension(
        &self,
        for_var: u16,
        bounds: Option<&(Value, Value)>,
        body: &Value,
        if_step: &Value,
        _in_t: &Type,
    ) -> Option<Vec<Value>> {
        use crate::const_eval::{const_eval, const_eval_with_var};
        let (from_e, till_e) = bounds?;
        let from = const_eval(from_e, &self.data)?;
        let till = const_eval(till_e, &self.data)?;
        let (from_i, till_i) = match (&from, &till) {
            (Value::Int(a), Value::Int(b)) => (*a, *b),
            _ => return None,
        };
        if from_i >= till_i {
            return Some(Vec::new()); // empty range
        }
        let count = (till_i - from_i) as u32;
        if count > 10_000 {
            return None; // S7: size limit
        }
        let has_filter = *if_step != Value::Null;
        let mut values = Vec::with_capacity(count as usize);
        for i in from_i..till_i {
            let iv = Value::Int(i);
            // Check filter condition if present.
            if has_filter {
                match const_eval_with_var(if_step, for_var, &iv, &self.data) {
                    Some(Value::Boolean(true)) => {}         // include
                    Some(Value::Boolean(false)) => continue, // skip
                    _ => return None,                        // filter not const-foldable
                }
            }
            let folded = const_eval_with_var(body, for_var, &iv, &self.data)?;
            values.push(folded);
        }
        Some(values)
    }

    /// loft#703 — is this literal being built straight into a KEYED accumulator?
    ///
    /// Such a variable owns a keyed store of its own (`hash::add` / `sorted_new` /
    /// `tree::add` add through it), so none of the vector backing applies: no `__vdb_N`
    /// store, no capacity pre-allocation, no `vector<T>` retype.  Asked in one place so
    /// the build and the type agree — a literal built as a vector under a variable still
    /// typed keyed reads its elements back through the wrong container.
    pub(crate) fn keyed_local(&self, vec: u16) -> bool {
        vec != u16::MAX && is_keyed(self.vars.tp(vec))
    }

    /// The empty keyed collection a NULLABLE keyed LOCAL must be given before a write can
    /// land in it — `if h == null { OpDatabase(h, …) }` — or `None` when `vec` is not one.
    ///
    /// `(N-Default)` states it on the COLLECTION and not on the kind: appending to a null
    /// collection builds the empty one first.  Two of the three shapes already keep it and
    /// the third could not.  A vector LOCAL is given a `__vdb_N` backing at the write
    /// ([`Parser::vector_db`]); a keyed FIELD holds the absent marker in a slot the runtime
    /// materialises in place (`collection_rec`, loft#1213).  A keyed LOCAL has neither: its
    /// store comes from its DECLARATION, and one declared `= null` is given no store at all,
    /// so the slot holds the null sentinel and every keyed accessor follows it as a record
    /// number — `h += [e]` and `h[k] = e` both reached "a NULL DbRef reached a store
    /// accessor" while the dense local one declaration over was correct.
    ///
    /// GUARDED, not unconditional, and that is the half a write site cannot supply for
    /// itself: the site can run many times, so an unguarded mint would build a fresh
    /// collection on every pass and throw away what the previous ones put in it.  Guarded,
    /// the mint is idempotent and a keyed local filled inside a loop keeps its records.
    ///
    /// An ARGUMENT is excluded: its store is the caller's, and a null one is the caller's
    /// answer to give.
    pub(crate) fn keyed_local_materialise(&mut self, vec: u16) -> Option<Value> {
        if self.first_pass
            || !self.keyed_local(vec)
            || !self.vars.tp(vec).peel_optional().1
            || self.vars.is_argument(vec)
        {
            return None;
        }
        let keyed_tp = self.vars.tp(vec).clone();
        let kt = self.keyed_known_type(&keyed_tp)?;
        self.keyed_place_materialise(&Value::Var(vec), kt, &keyed_tp)
    }

    /// The same guarded build for a keyed PLACE rather than a keyed local — the emission
    /// itself, so the two place-kinds cannot drift about what "build the empty one first"
    /// produces.
    ///
    /// A VARIABLE is repointed by `OpDatabase` directly, which takes the variable it fills.
    /// A TUPLE ELEMENT cannot be: it is a slot inside the tuple, and `OpDatabase` names a
    /// variable, so the store is built in a `__kvb_N` accumulator and put into the slot with
    /// the same `TuplePut` an ordinary `t.0 = h` uses. That assignment is what loft#1225's
    /// first half taught to accept a keyed collection at all — before it, this materialisation
    /// could not have been written, because the statement it ends with was an ICE.
    ///
    /// `None` for any other place. A struct FIELD is deliberately not here: its slot is
    /// addressable, so the runtime materialises it in place through `collection_rec`
    /// (loft#1213), and a second build from the parser would orphan the first.
    pub(crate) fn keyed_place_materialise(
        &mut self,
        place: &Value,
        kt: u16,
        keyed_tp: &Type,
    ) -> Option<Value> {
        let test = self.cl("OpVectorIsNull", std::slice::from_ref(place));
        let mint = match place.unspan() {
            Value::Var(v) => self.cl("OpDatabase", &[Value::Var(*v), Value::Int(i32::from(kt))]),
            Value::TupleGet(tuple_var, idx) => {
                let (tuple_var, idx) = (*tuple_var, *idx);
                let base = keyed_tp.base().clone();
                let kvb = self.vars.work_keyed(&base, &mut self.lexer);
                let db = self.cl("OpDatabase", &[Value::Var(kvb), Value::Int(i32::from(kt))]);
                Value::Insert(vec![
                    v_set(kvb, Value::Null),
                    db,
                    Value::TuplePut(tuple_var, idx, Box::new(Value::Var(kvb))),
                ])
            }
            _ => return None,
        };
        Some(crate::data::v_if(test, mint, Value::Null))
    }

    /// The registered database type for a KEYED collection type — the id that makes
    /// `record_new` / `record_finish` dispatch to `hash::add` / `sorted_new` /
    /// `tree::add` / the spatial index rather than to `vector_append`.
    ///
    /// Registration is idempotent (the same call the typedef walker makes), so asking
    /// is also how the type comes to exist.  `None` for a non-keyed type, or one whose
    /// content has no layout yet.
    ///
    /// **The one home for the question** — `Parser::keyed_type_id` delegates here.  One
    /// list of the keyed kinds, because a site that names four of the five reads as a
    /// complete rule and is not one: the field-replace site named three, so `spatial` and
    /// `trie` fields silently kept the whole-collection-assign defect it exists to fix
    /// (loft#922).
    ///
    /// Nullability-agnostic, and the peel lives HERE rather than in a note telling callers
    /// to do it — which is what `keyed_type_id`'s doc used to say, one contract per
    /// spelling.  `Optional(τ)` lays out exactly like `τ` (@FR-L-Null), so a `hash<S[k]>?`
    /// needs the same registered store type as its bare twin.  Answering `None` for the
    /// wrapper sends `new_record` to its `vector_of(in_t)` fallback, which registers a
    /// SECOND db type: the literal then builds into `vector<S>` while every read goes to
    /// `hash<S[k]>`, and the collection reads back empty.  Measured on a nullable keyed
    /// LOCAL — a shape [`is_keyed`] still refuses, so nothing reaches it that way today.
    pub(crate) fn keyed_known_type(&mut self, tp: &Type) -> Option<u16> {
        let tp = tp.base();
        let content = match tp {
            Type::Sorted(td, _, _)
            | Type::Hash(td, _, _)
            | Type::Index(td, _, _)
            | Type::Radix(td, _, _)
            | Type::Trie(td, _, _) => self.data.def(*td).known_type(),
            _ => return None,
        };
        if content == u16::MAX {
            return None;
        }
        Some(match tp {
            Type::Sorted(_, key, _) => self.database.sorted(content, key),
            Type::Hash(_, key, _) => self.database.hash(content, key),
            Type::Index(_, key, _) => self.database.index(content, key),
            Type::Radix(_, key, _) => self.database.spatial(content, key),
            Type::Trie(_, key, _) => self.database.trie(content, key),
            _ => return None,
        })
    }

    /// @FR-O-Proxy asks alloc — whether this vector needs a local `__vdb_N` backing store.
    /// The proxy narrows it to locals that own their storage; the answer allocates or does
    /// not, and the argument carve-out below exists to avoid an allocation, not a free.
    pub(crate) fn vector_needs_db(&self, vec: u16, in_t: &Type, is_var: bool) -> bool {
        is_var
            && *in_t != Type::Void
            && !self.keyed_local(vec)
            && self.vars.tp(vec).depend().is_empty()
            && !matches!(self.vars.tp(vec), Type::RefVar(_))
            // Argument vectors already have a caller-provided backing store; do not
            // allocate a local __vdb_N store that would be freed before the return.
            && !self.vars.is_argument(vec)
    }

    /// The PLACE an accessor expression names: the variable it is rooted at, and the field
    /// offsets walked from there.  `None` for anything that is not a variable or a chain of
    /// `OpGetField`s.
    ///
    /// Two things are deliberately ignored, because the same place written at two source
    /// positions differs in both: the `Span` wrappers (peeled at every level, not just the
    /// top) and the accessor's trailing type-id argument, which is a resolution artefact
    /// rather than part of the place.  Plain `PartialEq` on the expression sees both and so
    /// answers "different place" for `s.inner.v` on the left and `s.inner.v` on the right —
    /// the nesting is what makes it bite, since a one-level `s.v` has only a bare `Var`
    /// under it.
    pub(crate) fn field_place(&self, v: &Value) -> Option<(u16, Vec<PlaceStep>)> {
        match v.unspan() {
            Value::Var(x) => Some((*x, Vec::new())),
            Value::Call(d, args) if *d == self.data.def_nr("OpGetField") => {
                let Value::Int(off) = args.get(1)?.unspan() else {
                    return None;
                };
                let (root, mut path) = self.field_place(args.first()?)?;
                path.push(PlaceStep::Field(*off));
                Some((root, path))
            }
            // An ELEMENT step, so `xs[0].items` is a place of its own rather than nothing.
            // Both spellings of the read are the SAME place — a nullable read of an element
            // is that element — so they normalise to one step; the index decides which
            // element, and only an index this statement cannot change is admitted: a
            // constant, or a variable, which no expression inside one statement rewrites.
            // Anything else (a computed index) is not named and falls back to declining, as
            // every unnameable destination here does.
            Value::Call(d, args) if let Some(at) = element_index_arg(self.data.def(*d).name()) => {
                let step = match args.get(at)?.unspan() {
                    Value::Int(i) => PlaceStep::ElemConst(*i),
                    Value::Var(i) => PlaceStep::ElemVar(*i),
                    _ => return None,
                };
                let (root, mut path) = self.field_place(args.first()?)?;
                path.push(step);
                Some((root, path))
            }
            // A CAPTURED collection.  A closure reaches its capture through
            // `OpGetDbRef(__closure, <offset>)` — @PLN93's build-into-target writes straight
            // through it — so the destination is neither a variable nor a field chain, and a
            // build inside the closure could not be told from a read of something else.  It
            // IS a place: the closure record and the slot the capture sits in.
            Value::Call(d, args) if self.data.def(*d).name() == "OpGetDbRef" => {
                let Value::Int(off) = args.get(1)?.unspan() else {
                    return None;
                };
                let (root, mut path) = self.field_place(args.first()?)?;
                path.push(PlaceStep::Capture(*off));
                Some((root, path))
            }
            _ => None,
        }
    }

    /// The snapshot a collection build takes of a destination it READS, so its reads resolve
    /// through what the destination held when the statement began.
    ///
    /// `(I-Comp)` says a build reads what its destination held when the statement BEGAN, never
    /// the result being built, whichever part does the reading and however many times the
    /// statement runs.  A LITERAL is the same build without the loop and is held to the same
    /// sentence.  Building THROUGH the destination breaks it the moment the destination is
    /// detached — the `=` repoint (`vector_db`), a field's `OpClearVector` — or grown (`+=`):
    /// every read inside the parts then resolves through the empty or partially-built result,
    /// so `v = [v[1], v[0]]` answered `[0, 0]` and `v += [len(v), len(v)]` appended `2, 3`.
    /// Enforces @FR-O-Detach: the detach is sequenced AFTER every read of the destination by
    /// the value being built — here by hoisting those reads onto a copy taken before the
    /// first write, which is the same hoist the comprehension's deferred repoint already made
    /// for its own five parts.
    ///
    /// Returns the ops that take the snapshot — to run BEFORE anything detaches or grows the
    /// destination — having renamed every read of the destination in `parts` to the snapshot.
    /// Flat ops, not a block: the snapshot temp is read by the build's own parts, and a block
    /// would scope its `let` away from them on `--native`.  A literal puts them at the HEAD of
    /// its ops and records their count in [`Parser::build_snapshot_len`], which the sites that
    /// insert a detach at the head of a build's ops read as their insertion point.  `None`
    /// when nothing reads the destination, and always on pass 1: the snapshot is a pass-2
    /// temp like every other materialising temp here.
    ///
    /// Two destinations are left alone.  A KEYED one: its literal is built THROUGH it by
    /// construction (loft#703) and a vector copy is not its kind.  And a field reached through
    /// an ELEMENT (`xs[i].items`), which [`Self::field_place`] cannot name — the root-variable
    /// fallback `comprehension_needs_own_buffer` takes is over-wide (a sibling field matches),
    /// so renaming on it would redirect reads of the sibling too.
    pub(crate) fn snapshot_read_destination(
        &mut self,
        vec: u16,
        dest: &Value,
        is_var: bool,
        is_field: bool,
        in_t: &Type,
        parts: &mut [&mut Value],
    ) -> Option<Vec<Value>> {
        if self.first_pass || (vec != u16::MAX && self.keyed_local(vec)) {
            return None;
        }
        // Any destination that is not a bare VARIABLE is named by its PLACE, whether the
        // caller called it a field or not.  A CAPTURED collection is neither a variable nor a
        // field — a closure reaches it through `OpGetDbRef(__closure, <offset>)` — so gating
        // the place on `is_field` left it with no read test at all and the build read its own
        // emptied result (loft#1391).  A field whose place cannot be named still declines, as
        // it did.
        let field_place = match dest.unspan() {
            Value::Var(_) => None,
            _ if is_field => Some(self.field_place(dest)?),
            _ => self.field_place(dest),
        };
        let reads = {
            let ro: Vec<&Value> = parts.iter().map(|p| &**p).collect();
            match &field_place {
                Some(place) => ro
                    .iter()
                    .any(|v| v.any_node(&mut |n| self.field_place(n).is_some_and(|p| p == *place))),
                None => {
                    self.comprehension_reads_target(vec, is_var, &ro)
                        || self.comprehension_needs_own_buffer(vec, dest, is_var, is_field, &ro)
                }
            }
        };
        if !reads {
            return None;
        }
        let src_tp = Type::Vector(Box::new(in_t.clone()), Deps::none());
        let src = self.create_unique("build_src", &src_tp);
        self.vars.defined(src);
        let mut snap = self.vector_db(in_t, src);
        let elem_tp = self.append_elem_tp(in_t);
        snap.push(self.cl(
            "OpAppendVector",
            &[Value::Var(src), dest.clone(), Value::Int(elem_tp)],
        ));
        // Reads only: the build's WRITES are emitted from `vec` / `dest` after this, so the
        // rename cannot redirect an append.
        if let Some(place) = field_place {
            for part in parts.iter_mut() {
                part.map_nodes(&mut |n| {
                    if self.field_place(n).is_some_and(|p| p == place) {
                        *n = Value::Var(src);
                    }
                });
            }
        } else {
            // A `&` link shares the destination's store, so a read spelled through the link
            // is a read of the same place and has to be redirected with the destination's
            // own — `b = [(r[2] ?? 0), (r[0] ?? 0)]` under `r = &b`.  Renaming only the
            // destination's name left those reads pointing at the store the build is about
            // to clear, and the literal answered zeros.
            let mut names = vec![vec];
            names.extend(self.vector_link_partner_vars(vec));
            for name in names {
                for part in parts.iter_mut() {
                    crate::parser::collections::rename_var(part, name, src);
                }
            }
        }
        Some(snap)
    }

    /// Does a comprehension being built into `vec` READ `vec` itself?
    ///
    /// `I-Comp` ([`doc/claude/formal/iteration.md`]) builds a comprehension into a FRESH
    /// result vector and hands that over, so the destination still holds the value it had
    /// while the source, the range bound, the `if` guard and the body are evaluated.
    /// Building straight into the destination — the `#501` watermark reuse — keeps that
    /// promise only while nothing in the loop reads the destination back.
    ///
    /// Answered for a whole-value `=` into a LOCAL only ([`Parser::assign_target`] with
    /// [`Parser::assign_replaces`]): that is the assignment which repoints the destination
    /// at a fresh store, and so the only one with a repoint to DEFER.  The other two
    /// destinations read their target the same way but are emptied — or grown — by a
    /// different site, so they take the fresh-buffer route instead
    /// ([`Parser::comprehension_needs_own_buffer`]).
    ///
    /// `parts` are the comprehension's evaluated pieces.  The read test is
    /// [`Value::reads_var`], whose over-approximation costs this caller a deferred
    /// repoint and never a wrong answer.
    pub(crate) fn comprehension_reads_target(
        &self,
        vec: u16,
        is_var: bool,
        parts: &[&Value],
    ) -> bool {
        is_var
            && vec != u16::MAX
            && vec == self.assign_target
            && self.assign_replaces
            && (parts.iter().any(|v| v.reads_var(vec)) || self.parts_read_a_vector_link(vec, parts))
    }

    /// Do these parts read the destination through a `&` LINK rather than by its own name?
    ///
    /// A vector link shares the source's `DbRef`, so `r = &a` makes `r` and `a` one place and
    /// `a = [for i in 0..r.len() { (r[i] ?? 0) * 2 }]` reads what it is assigning.
    /// `(I-Comp)` is stated over the PLACE, not the spelling — and the clear that empties the
    /// destination reaches the link too, so without this the loop reads an emptied vector and
    /// the build answers `[]`.
    fn parts_read_a_vector_link(&self, vec: u16, parts: &[&Value]) -> bool {
        self.vector_link_partner_vars(vec)
            .into_iter()
            .any(|pv| parts.iter().any(|v| v.reads_var(pv)))
    }

    /// The variables a `&` link makes one place with `vec` — the source of a link, or every
    /// link taken to a source.
    fn vector_link_partner_vars(&self, vec: u16) -> Vec<u16> {
        if vec == u16::MAX {
            return Vec::new();
        }
        let name = self.vars.name(vec).to_string();
        let Some(partners) = self.amp_vector_link_partners.get(&(self.context, name)) else {
            return Vec::new();
        };
        partners
            .iter()
            .map(|p| self.vars.var(p))
            .filter(|v| *v != u16::MAX && *v != vec)
            .collect()
    }

    /// Does a comprehension need to be built into a BUFFER of its own, rather than through
    /// its destination?
    ///
    /// The other half of `I-Comp` beside [`Self::comprehension_reads_target`]. Two
    /// destinations read what they are being assigned and cannot be served by deferring a
    /// repoint, because neither HAS one to defer:
    ///
    /// * a struct FIELD (loft#1195) — the whole-vector field replace emits
    ///   `OpClearVector(s.v)` ahead of the comprehension's own ops, so the field is empty
    ///   before the loop reads it. `dest` is the field expression, and the comparison is by
    ///   EXPRESSION: reading a SIBLING field (`s.v = [for … s.w …]`) is correct today and
    ///   must keep its in-place build.
    /// * a compound `+=` into a local (loft#1196) — it appends into the destination's own
    ///   store, so a bound or source that reads the destination measures a length that the
    ///   loop itself is growing, and the loop never ends.
    ///
    /// Both are answered by building into a fresh local and letting the destination's own
    /// assignment deliver it — the route `map` and `filter` already take, which is why
    /// `s.v = s.v.map(…)` and `a += a.map(…)` are correct today while the comprehension
    /// spelling of each is not.
    pub(crate) fn comprehension_needs_own_buffer(
        &self,
        vec: u16,
        dest: &Value,
        is_var: bool,
        is_field: bool,
        parts: &[&Value],
    ) -> bool {
        if is_field {
            // Compared as a PLACE, so a nested destination (`s.inner.v`) matches its own
            // reads.  When the destination is an accessor shape `field_place` cannot read,
            // fall back to any read of its ROOT variable: that is over-wide — a sibling
            // field matches it — and over-wide costs a buffer, never a wrong answer.
            let Some(dest_place) = self.field_place(dest) else {
                return dest
                    .base_var()
                    .is_some_and(|root| parts.iter().any(|v| v.reads_var(root)));
            };
            return parts.iter().any(|v| {
                v.any_node(&mut |n| self.field_place(n).is_some_and(|p| p == dest_place))
            });
        }
        is_var
            && vec != u16::MAX
            && vec == self.assign_target
            && !self.assign_replaces
            && parts.iter().any(|v| v.reads_var(vec))
    }

    pub(crate) fn unique_elm_var(&mut self, parent_tp: &Type, assign_tp: &Type, vec: u16) -> u16 {
        let c_tp = parent_tp.content();
        let was = Type::Reference(
            if c_tp.is_unknown() {
                0
            } else {
                self.data.type_def_nr(&c_tp)
            },
            Deps::frame(parent_tp.depend()),
        );
        // @PLN25 E2 — an ANONYMOUS `{ … }` element against a nullable synth enum
        // `__nullable<S>` has no struct name to drive the transparent-construction
        // path (objects.rs:151, which keys on `S{` matching `__nullable<S>`), so it
        // falls to `parse_block`'s `Reference(r)` record-scan.  Type the element var
        // as the `Some` variant so that scan builds the present payload (the
        // discriminant defaults present via object_init), exactly as the NAMED
        // `S{ … }` path does.  Without this the var falls back to `was`
        // (`Reference(type_def_nr(parent_tp.content))`) and mis-resolves to an
        // arbitrary def → "Unknown field <wrong-def>.<field>".  Gate-inert: a
        // `__nullable<>` enum only exists when the E2 rewrite is active.
        let elm_tp = if let Type::Reference(rd, _) = assign_tp {
            // @PLN25 E2 — a keyed-collection field's `content()` is
            // `Reference(__nullable<S>)` (not `Enum(..)`), so an anon `{ … }`
            // element assigned to a `hash`/`sorted`/`index` field must resolve
            // against the `Some` variant too — same as the `Enum(syn,true)`
            // (vector-element) case below.  Without this it points at the enum
            // and `.field` fails ("Unknown field __nullable<S>.field").
            if self.data.def(*rd).name.starts_with("__nullable<") {
                Type::Reference(
                    self.data.variant_of(*rd, "Some"),
                    Deps::frame(parent_tp.depend()),
                )
            } else {
                assign_tp.clone()
            }
        } else if let Type::Enum(syn, true, _) = assign_tp
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            Type::Reference(
                self.data.variant_of(*syn, "Some"),
                Deps::frame(parent_tp.depend()),
            )
        } else if let Type::Vector(inner, _) = assign_tp {
            // #555 — a `vector<T>` element keeps its SPECIFIC type.  `was` routes through
            // `type_def_nr(vector<T>)`, which collapses EVERY vector to the one generic `vector`
            // source def (data.rs), so the element var's inner type is shared across all vector
            // slices in a function — two nested-vector slices then desync (the later one's first
            // element reads null).  Carrying `assign_tp`'s own inner type keeps them distinct.
            Type::Vector(inner.clone(), Deps::frame(parent_tp.depend()))
        } else {
            was
        };
        let elm = self.create_unique("elm", &elm_tp);
        // loft#664 — an element NEVER owns a store: its record is the slot the
        // enclosing `OpNewRecord` carved out of the container, and `OpFinishRecord`
        // commits it there.  That was encoded only as a DEPENDENCY on the container
        // VARIABLE, so a container with no variable — a vector inside an enum payload
        // is addressed by a field DbRef — left the dep list empty, and empty reads as
        // "owns its store": the answer came back WRONG rather than unknown.  State the
        // fact at the mint site instead, through the marker that already means
        // "borrow, don't allocate", so every consumer reads it rather than inferring
        // it from deps (or, as #660 had to, from the `_elm` name).  The dep below is
        // still recorded where a container variable exists — it names the borrow
        // SOURCE, which the marker does not.
        self.vars.mark_inline_ref(elm);
        if vec != u16::MAX {
            self.vars.depend(elm, vec);
        }
        self.vars.depend_on_all(elm, &parent_tp.depend());
        elm
    }

    pub(crate) fn parse_multiply(&mut self, res: &mut Vec<Value>) -> Option<Type> {
        let mut code = Value::Null;
        let tp = self.parse_operators(&Type::Unknown(0), &mut code, &mut Type::Null, 0);
        if !matches!(tp, Type::Integer(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a number as the object multiplier"
            );
            return Some(Type::Unknown(0));
        }
        res.push(Value::Return(Box::new(code)));
        None
    }

    // <item> ::== ['for' | <expr> ]
    pub(crate) fn parse_item(
        &mut self,
        elm: u16,
        in_t: &mut Type,
        declared: bool,
        res: &mut Vec<Value>,
    ) -> Option<Type> {
        // `elm` is offered as an in-place construction target so a struct element
        // builds straight into the slot `OpNewRecord` carved out (`parse_object`
        // takes that path whenever it is handed a `Var` that owns a store).  A
        // TUPLE element is not built as one record — `emit_tuple_set_ops` writes
        // each member at its own offset — so offering the slot lets the tuple
        // literal's FIRST member consume it: `[(S { … }, k)]` wrote S's fields
        // directly into the element and then handed that valueless statement list
        // to `OpCopyRecord` as its SOURCE.  Only the first member could ever fail
        // this way, because `(a, b, …)` parses member 0 into the caller's value and
        // every later member into a fresh one (loft#942).
        // Peel `Rewritten` first: a literal in RETURN position gets its element type
        // from the function's return type and arrives wrapped (the unwrap below runs
        // only AFTER the element is parsed, which is too late to decide this).
        let elem_is_tuple = {
            let mut t: &Type = in_t;
            while let Type::Rewritten(inner) = t {
                t = inner;
            }
            matches!(t.base(), Type::Tuple(_))
        };
        // A `(`-leading element is a parenthesised expression or a tuple literal, and
        // `(a, b, …)` parses member 0 into the caller's value while every later member
        // gets a fresh one.  Offering the slot therefore lets the FIRST member consume
        // it, and the type cannot tell us to stop: in return position the element type
        // is inferred FROM this literal, so it is still `Unknown` when member 0 is
        // seeded and only resolves by member 1 — which is exactly why the first member
        // was the only one that ever failed.  Keying on the token instead is decidable
        // at the one moment the decision has to be made.  A parenthesised struct
        // literal `[(S { … })]` loses the in-place build and takes the allocate-then-
        // copy path every non-first member already takes; it stays correct.
        let mut p = if elem_is_tuple || self.lexer.peek_token("(") {
            Value::Null
        } else {
            Value::Var(elm)
        };
        // #247: isolate THIS element's capturing-lambda signal.  A capturing
        // lambda makes `emit_lambda_code` set `last_closure_work_var` (and emit
        // a Block, not a bare FnRef); a non-capturing lambda / non-lambda leaves
        // it MAX.  Reset first so a prior element's value can't leak in.
        self.last_closure_work_var = u16::MAX;
        let mut t = if self.lexer.has_token("for") {
            //self.iter_for(&mut p)
            diagnostic!(
                self.lexer,
                Level::Error,
                "For inside a vector is not yet implemented"
            );
            return Some(Type::Unknown(0));
        } else {
            let mut parent_tp = Type::Null;
            // @PLAN58 III-a: propagate the declared element type `in_t` into the
            // element parse so a NESTED literal's inner elements adopt the
            // declared narrow width (`vector<vector<i32>>` → inner `[1,2]` types
            // its elements `i32`, not wide `integer`).  When `in_t` is Unknown
            // (untyped inferred literal) this is identical to the prior
            // `Type::Unknown(0)` behaviour.
            // loft#1067 — an element of a `vector<fn(…)>` is an inference context for the
            // same reason a call argument is: the declared element type names the
            // signature. Saved and restored rather than cleared, because an element is
            // parsed INSIDE whatever push the literal itself arrived under.
            let saved_expected = std::mem::replace(&mut self.expected, Type::Unknown(0));
            if Self::seeds_lambda_hint(in_t) {
                self.expected = in_t.base().clone();
            }
            let parsed = self.parse_operators(&in_t.clone(), &mut p, &mut parent_tp, 0);
            self.expected = saved_expected;
            parsed
        };
        let elem_capturing_lambda = self.last_closure_work_var != u16::MAX;
        if let Type::Rewritten(tp) = in_t {
            *in_t = *tp.clone();
        }
        if let Type::Rewritten(tp) = t {
            t = *tp.clone();
        }
        if in_t.is_unknown() {
            *in_t = t.clone();
        }
        if t.is_unknown() {
            t = in_t.clone();
        }
        // loft#944 — the same adoption for a type that is unresolved INSIDE rather than at
        // the top: a pass-1 element type carrying a forward-referenced member
        // (`(integer, unknown)`) is a placeholder, and the element the author actually
        // wrote is the resolved one.  Only when the other side is fully resolved, so this
        // cannot swap one placeholder for another.  Without it pass 2 refused its own
        // literal — "No common type (integer, Q) for vector (integer, unknown)".
        if crate::data::Data::type_has_unresolved(in_t)
            && !crate::data::Data::type_has_unresolved(&t)
        {
            *in_t = t.clone();
        } else if crate::data::Data::type_has_unresolved(&t)
            && !crate::data::Data::type_has_unresolved(in_t)
        {
            t = in_t.clone();
        }
        // loft#1232 — `(N-Store)` is owed of a LITERAL's elements too.  `(N-Dense)` says a
        // `vector<t>`'s elements are non-null unless the type is written `vector<t?>`, and the
        // rule was enforced at the scalar seam (`x: integer = n`) and at the append seam
        // (`c += n`) but nowhere inside a collection literal: `v: vector<integer> = [n]` with
        // `n: integer?` stored the null and `v[0]` read back null, silently, on both backends.
        //
        // This is the CURE the language itself names.  loft#1223 refuses the bare `c += n` and
        // sends the reader to `c += [n]` — the right spelling under @PLAN52's bracket rule, and
        // until now the un-diagnosed one — so closing that issue moved the reader from warned to
        // silent.  A diagnostic must not go quiet along the path it recommends.
        //
        // Asked HERE because this is the one point every literal shape passes through with both
        // types settled: a typed local (`v: vector<integer> = [n]`), a field assignment
        // (`d.c = [n]`), a constructor field (`D { c: [n] }`) and a NESTED literal all reach
        // `parse_item`, so the three shapes the issue lists and their nested forms need no
        // separate checks.  Placed BEFORE the conversion chain below, which is what was silent:
        // `convert` peels the `Optional` and stores the sentinel without a word.
        //
        // `n_store_violation` is the shared home, so this seam cannot drift from the other two —
        // it carries the warn/error split (a full-width slot reserves its null distinctly and
        // WARNS with the store proceeding; a narrow one has no room and errors) and it already
        // declines on a nullable target, which is what keeps `vector<t?>` and the synthetic
        // `vector<__nullable<S>>` quiet.
        // @FR-N-Store: the element is a slot; both halves of the rule are asked by the store
        // face at the `convert` below (lenient — loft#1232 — so a narrow element warns).  The
        // arms between here and there that bypass `convert` are the nullable-wrapper
        // spellings, whose slot holds absence by construction.
        let elem_index = res.len();
        let elem_slot = format!("element {elem_index} of this vector literal");
        if let (Type::Reference(t_nr, _), Type::Reference(in_nr, _)) = (&t, &in_t.clone())
            && let (Type::Enum(t_e, true, _), Type::Enum(in_e, true, _)) = (
                self.data.def(*t_nr).returned(),
                self.data.def(*in_nr).returned(),
            )
            && *t_e == *in_e
        {
            *in_t = Type::Enum(*t_e, true, Deps::none());
        } else if let (Type::Enum(t_e, true, _), Type::Enum(in_e, true, _)) = (&t, &*in_t)
            && *t_e == *in_e
        {
            // @PLN25 E2 — the appended element ALREADY is the vector's nullable
            // enum (`result += [sa_sp]` where `sa_sp: __nullable<S>` and the
            // vector is `vector<__nullable<S>>`).  No conversion is needed — store
            // it as-is; the generic `convert` below does not recognise
            // enum→same-enum and would fire the spurious "would lose precision"
            // diagnostic.  Mirrors the `Reference`-to-same-enum arm above for the
            // case where the element is typed as the enum directly.
        } else if matches!(&*in_t, Type::Enum(syn, true, _) if self.data.def(*syn).name.starts_with("__nullable<"))
            && matches!(t, Type::Null)
        {
            // @PLN25 E2 — a `null` element in a `vector<__nullable<S>>` (e.g.
            // `v += [null]`): the appended element is the Null variant
            // (discriminant 0).  OpNewRecord zero-inits the element to disc 0, so
            // emit an empty construction; the generic convert would otherwise
            // reject `null` → the synthetic enum and fire the "cannot store"
            // diagnostic.
            p = Value::Insert(Vec::new());
            t = in_t.clone();
        } else if !self.first_pass
            && let Type::Enum(syn, true, _) = in_t.base()
            && self.needs_nullable_wrap(*syn, &t)
            && !matches!(p, Value::Insert(_))
        {
            // @PLN25 single-payload — store a value spelled `S` or `S?` into a
            // `vector<__nullable<S>>` element (`v += [p]`, `v += [make()]`): set the
            // discriminant present and copy the dense `S` into the inline `payload` field.
            // `emit_nullable_slot_write` is the shared home — it stashes the source once and
            // supplies the runtime null test this arm used to be written without, which is
            // what an `S?` source needs: the value is only dense when it is present.
            let syn = *syn;
            let steps = self.emit_nullable_slot_write(syn, &Value::Var(elm), p.clone());
            p = Value::Insert(steps);
            t = in_t.clone();
        } else if matches!(t, Type::Null)
            && matches!(in_t.base(), Type::Enum(_, false, _))
            && !matches!(in_t, Type::Optional(_))
        {
            // A `null` into a DENSE value-enum element is a real precision loss: the slot
            // packs the raw discriminant byte and the author asked for a vector that holds
            // a `Color`, not the absence of one.  A NULLABLE element (`vector<Color?>`) is
            // a different question and takes the scalar `convert(Null, Enum)` typed-null
            // path below — `(L-Null)` covers every type that reserves a null VALUE, and a
            // value enum reserves two codes (`0` undefined, `255` null), which is why its
            // variants are numbered from 1.  Refusing there rejected the author's own
            // declared type while naming a type they had not written (loft#1416).
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot store null elements in a vector<{}> (would lose precision); \
                 declare the element nullable (`vector<{}?>`), or cast each element \
                 explicitly with 'as {}'",
                in_t.name(&self.data),
                in_t.name(&self.data),
                in_t.name(&self.data)
            );
        } else if self.first_pass
            && (crate::data::Data::type_has_unresolved(&t)
                || crate::data::Data::type_has_unresolved(in_t))
        {
            // loft#944 — neither side is a type yet.  A forward-referenced element member
            // (`vector<(integer, Q)>` with `Q` declared below) is `Unknown(stub)` for all of
            // pass 1, so `convert` fails and the arms below report a precision loss between
            // two spellings of the SAME type — an error that aborted the run before pass 2
            // could re-check against the resolved element.  Say nothing; pass 2 decides.
            // The struct-field store guard (`objects.rs`) already works this way.
        } else if !self.convert_store_lenient(&mut p, &t, in_t, &elem_slot, None) {
            if declared {
                // @P315 — the element type is DECLARED (typed local / struct
                // field).  A value that does not convert TO it must be cast
                // EXPLICITLY: silently promoting the declared element type
                // (e.g. Single→Float) would change the element storage WIDTH
                // (a vector<single> packs 4-byte slots; an 8-byte OpSetFloat
                // write overflows them → heap corruption) AND lose data
                // (float→single).  Consistent with scalar / local-vector
                // assignment, which already reject this.
                // loft#1146 — for `float` elements in a `vector<single>` the cheapest cure
                // is the literal SUFFIX, not a cast: `[1.0f, 2.0f]` is what `LOFT.md`'s own
                // example writes, and per-element `as single` was the only thing offered.
                // Named first because it costs no conversion; the cast stays for the case
                // where the elements are not literals.
                if matches!(t, Type::Float) && matches!(in_t.base(), Type::Single) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot store float elements in a vector<single> (would lose precision); \
                         write `single` literals with the `f` suffix (`[1.0f, 2.0f]`), or cast each \
                         element with 'as single'"
                    );
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot store {} elements in a vector<{}> (would lose precision); \
                     cast each element explicitly with 'as {}'",
                        t.name(&self.data),
                        in_t.name(&self.data),
                        in_t.name(&self.data)
                    );
                }
            } else if self.convert(&mut p, in_t, &t) {
                // INFERRED element type: widen to the common type
                // (e.g. [1, 2.0] → vector<float>).
                *in_t = t.clone();
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "No common type {} for vector {}",
                    t.name(&self.data),
                    in_t.name(&self.data)
                );
            }
        }
        if let Type::Enum(td_nr, true, _) = t
            && let Value::Enum(enum_nr, _) = &p
            && self.lexer.peek_token("{")
        {
            let mut ls = Vec::new();
            self.parse_enum_field(&mut ls, Value::Var(elm), td_nr, 0, *enum_nr);
            ls.push(p.clone());
            p = Value::Insert(ls);
        }
        // A collection takes non-capturing lambdas only (DESIGN_DECISIONS.md C116):
        // refuse the statically-detectable capturing shapes — a direct lambda, a
        // local holding one, a closure factory's return — at the literal.
        if !self.first_pass
            && matches!(in_t, Type::Function(_, _, _))
            && (elem_capturing_lambda || self.fn_ref_source_captures(&p))
        {
            self.refuse_capturing_closure_in_collection();
        }
        res.push(p.clone());
        None
    }

    /// Does this fn-ref SOURCE carry a captured environment?
    ///
    /// The statically-detectable shapes only — a direct capturing lambda (bare or wrapped
    /// in its closure-allocation block), a local holding one, and a call whose return type
    /// carries the closure work-var in its dep list (a closure FACTORY, which is otherwise
    /// indistinguishable from a plain `fn()->T` by signature). `false` for anything
    /// unrecognised, which is the answer that keeps a plain fn-ref working; a capturing
    /// source that slips through reaches the deferred layout rather than a wrong answer.
    ///
    /// One predicate, because two destinations ask the same question and must not answer
    /// differently: the collection LITERAL (#247) and the element ASSIGNMENT (loft#1072).
    /// The first half is [`find_capturing_fn_ref`], which walks the Block/Set wrappers a
    /// lambda arrives inside and knows the pass-1 marker (a bare `Int` d_nr whose def has
    /// a synthesized closure record); the second half is the two source shapes it cannot
    /// see through, a LOCAL and a CALL, where the capture is a fact about the source
    /// rather than about this expression.
    /// The ONE refusal for a capturing closure headed into a collection element,
    /// raised by the collection literal and by the element assignment alike.  A
    /// collection has one element layout and each capture set is its own record
    /// shape, so the element slot holds a plain fn-ref only; a struct FIELD holds a
    /// capturing closure, which is the route the message names
    /// (DESIGN_DECISIONS.md C116).
    pub(crate) fn refuse_capturing_closure_in_collection(&mut self) {
        diagnostic!(
            self.lexer,
            Level::Error,
            "a capturing closure cannot be stored in a collection: a collection has one \
             element layout and each capture set is its own record shape — hold it in a \
             struct field, or store a non-capturing fn that reads the state from a \
             struct field"
        );
    }

    pub(crate) fn fn_ref_source_captures(&self, p: &Value) -> bool {
        if super::find_capturing_fn_ref(&self.data, p).is_some() {
            return true;
        }
        match p.unspan() {
            Value::FnRef(_, clos_var, _) => *clos_var != u16::MAX,
            Value::Var(v) => self.closure_vars.contains_key(v),
            Value::Call(d_nr, _) => matches!(
                self.data.def(*d_nr).returned(),
                Type::Function(_, _, deps) if !deps.is_empty()
            ),
            _ => false,
        }
    }

    /// The four bytes to write into a fn-ref SLOT, projected out of whatever `src` is,
    /// plus the statements that must run before the write.
    ///
    /// A fn-ref slot in a vector element is four bytes holding the d_nr, while a fn-ref
    /// VALUE on the stack is a 20-byte pair — 8 bytes of d_nr plus a 12-byte closure
    /// DbRef. Writing the value straight through `OpSetInt4` takes the wrong four bytes
    /// (the high end of the slot, part of the closure DbRef) and stores a garbage d_nr
    /// that crashes when the element is later called or freed (#263). So each source
    /// shape is projected to its d_nr:
    ///
    /// * `Value::Int(d)` — what `parse_fn_ref` lowers a bare function name to. Already the
    ///   bare d_nr; write it.
    /// * `Value::Var(v)` — a fn-ref local. `FnRefDnr(v)` projects the d_nr via `OpVarInt`.
    /// * `Value::Call(…)` — a call that returns a fn-ref. Materialise it into a temp
    ///   first, then project that; the temp is marked `skip_free` because its closure half
    ///   is the null sentinel and scope-exit must never dereference it.
    ///
    /// Anything else is handed back untouched, which is what the struct-field path
    /// (`emit_fn_ref_field_write`) and the #247 guard are there to judge.
    pub(crate) fn fn_ref_slot_dnr(&mut self, src: &Value, in_t: &Type) -> (Value, Vec<Value>) {
        match src.unspan() {
            Value::Int(_) => (src.clone(), Vec::new()),
            Value::Var(v) if matches!(self.vars.tp(*v), Type::Function(_, _, _)) => {
                (Value::FnRefDnr(*v), Vec::new())
            }
            Value::Call(_, _) => {
                let fn_type = if let Type::Function(params, ret, _) = in_t {
                    Type::Function(params.clone(), ret.clone(), Deps::none())
                } else {
                    in_t.clone()
                };
                let tmp = self.create_unique("__fn_ref_tmp", &fn_type);
                self.vars.defined(tmp);
                self.vars.set_skip_free(tmp);
                if !self.first_pass {
                    self.change_var_type(tmp, &fn_type);
                }
                (Value::FnRefDnr(tmp), vec![v_set(tmp, src.clone())])
            }
            _ => (src.clone(), Vec::new()),
        }
    }

    pub(crate) fn is_field(&self, val: &Value) -> bool {
        // Plan-07 phase 1: unspan() so wraps on `.` (step 1.12) don't
        // hide the field-access shape from compound-assignment dispatch.
        if let Value::Call(o, _) = val.unspan() {
            *o == self.data.def_nr("OpGetField")
        } else {
            false
        }
    }

    /// A bare collection captured into a closure resolves, inside the closure body,
    /// to an `OpGetDbRef` of the closure-record field — a borrowed 12-byte DbRef, not
    /// a local `Var` (like a plain local) nor an `OpGetField` (like a struct field).
    /// It is still a DbRef-producing append lvalue: `coll += elem` must insert into the
    /// shared store the DbRef points at, exactly as a field target does.  Recognising it
    /// lets `parse_object` build a `Value::Insert` (not a fresh `Object`) and lets
    /// `new_record` emit `OpNewRecord`/`OpFinishRecord` against the captured DbRef.
    /// See doc/claude/plans/93-collection-capture/README.md.
    pub(crate) fn is_captured_dbref(&self, val: &Value) -> bool {
        matches!(val.unspan(), Value::Call(o, _) if *o == self.data.def_nr("OpGetDbRef"))
    }

    pub(crate) fn new_record_field_op(&mut self, val: &Value, parent_tp: &Type, op: &str) -> Value {
        if let Value::Call(_, ps) = val.unspan() {
            let parent = self.data.def(self.data.type_def_nr(parent_tp)).known_type();
            // @PLN25 single-payload: appending to a NESTED collection field of a `__nullable<S>`
            // element (`b.items += […]` where `b` is a nullable element) — the field-access
            // unwrap already made `ps[0]` the dense-`S` payload sub-ref and `ps[1]` its
            // S-relative offset, so resolve the field number against the payload's `S`, NOT the
            // enum (`field_nr(enum, S_offset)` = 0 → `OpNewRecord(field=0)` = the wrong field).
            // `key_owner` maps a synth `__nullable<S>` to its payload struct; identity otherwise.
            let parent = self.database.key_owner(parent);
            // loft#977 — the same fact for a USER struct-enum: `c.limbs` where `c: Shape`
            // and `limbs` lives in the `Circle` variant.  The enum type carries a variant
            // list and no fields, so resolving against it answers field 0 and a `u16::MAX`
            // field type, which `record_new` then uses as a type-table index.  Redirect to
            // the variant that declares the field, named by the offset AND the content type
            // the read (`OpGetField(base, pos, content)`) already resolved — two variants
            // each holding a collection put its handle at the same offset, so the offset
            // alone picks the wrong one.  Identity for a plain struct, so both halves of the
            // append still agree for every non-enum parent.
            let parent = if let Value::Int(pos) = ps[1]
                && let Some(Value::Int(content)) = ps.get(2)
                && let Ok(pos) = u16::try_from(pos)
                && let Ok(content) = u16::try_from(*content)
            {
                self.database.variant_owning_field(parent, pos, content)
            } else {
                parent
            };
            let field_nr = if let Value::Int(pos) = ps[1] {
                self.database.field_nr(parent, pos)
            } else {
                0
            };
            if op == "OpNewRecord" {
                self.cl(
                    "OpNewRecord",
                    &[
                        ps[0].clone(),
                        Value::Int(i32::from(parent)),
                        Value::Int(i32::from(field_nr)),
                    ],
                )
            } else {
                self.cl(
                    "OpFinishRecord",
                    &[
                        ps[0].clone(),
                        Value::Var(0), // placeholder, caller replaces with Value::Var(elm)
                        Value::Int(i32::from(parent)),
                        Value::Int(i32::from(field_nr)),
                    ],
                )
            }
        } else {
            Value::Null
        }
    }

    pub(crate) fn new_record(
        &mut self,
        val: &mut Value,
        parent_tp: &Type,
        elm: u16,
        vec: u16,
        res: &[Value],
        in_t: &Type,
    ) -> Vec<Value> {
        let mut ls = Vec::new();
        // A keyed LOCAL declared `= null` owns no store, so the record would be added
        // through the null sentinel.  Build the empty collection first — `(N-Default)` —
        // here rather than at each append spelling, because every one of them reaches this
        // function to add its record and no two reach it by the same route.
        if let Some(guard) = self.keyed_local_materialise(vec) {
            ls.push(guard);
        }
        let is_field = self.is_field(val);
        let ed_nr = self.data.type_def_nr(in_t);
        if ed_nr == u32::MAX && self.first_pass && crate::data::Data::type_has_unresolved(in_t) {
            // loft#944 — "never resolved" is the wrong word for it in pass 1: the element
            // names a type declared LOWER in the file, so it resolves at the end of this
            // pass and pass 2 builds the record normally.  This diagnostic is Fatal, so
            // firing it here aborted the whole compile on a program that is correct —
            // `v: vector<(integer, Q)> = [(71, Q { … })]` with `Q` below.  Emit nothing and
            // let pass 2 decide; a name that is still unresolved THERE reports below.
            return ls;
        }
        if ed_nr == u32::MAX {
            // The element type never resolved, so there is no record shape to build.  An
            // `assert_ne!` here made that an internal compiler error on ordinary source:
            // `v = [Nope { n: 1 }]` — one undefined name in a vector literal — was enough
            // to abort the compiler and send the reader looking for a compiler bug.
            //
            // The undefined name itself is always reported before this, so say nothing
            // about WHICH type is missing: by the time the element reaches here it is the
            // synthesised `never`, and naming that (or prescribing a `use` for it) points
            // at something the author never wrote.  Fatal because every caller below needs
            // a record shape, so the parse cannot usefully go on.
            diagnostic!(
                self.lexer,
                Level::Fatal,
                "cannot build this record — its type never resolved"
            );
            return ls;
        }
        // P188: when the LHS local is a keyed collection
        // (sorted/hash/index/spatial<T[key]>), the container type id
        // must be the keyed-collection's own known_type so OpNewRecord
        // dispatches to sorted_new / hash::add / tree::add / etc.
        // Falling back to `vector_of(in_t)` returns the wrap-`vector<T>`
        // id which would route through `Parts::Vector` → vector_append
        // and crash with index 65535.
        //
        // P188-followup: previously this used
        // `data.def(type_def_nr(lhs_tp)).known_type`, but
        // `type_def_nr` returns the GENERIC alias (`hash` / `index`)
        // not the specific `hash<Score[name]>` instantiation.  The
        // alias's `known_type` happened to be a vector type, so
        // record_finish dispatched through `Parts::Vector` instead of
        // `Parts::Hash`/`Parts::Index` — producing 6 records for 3
        // adds (vector_finish appends without dedup) and bypassing
        // tree::add entirely (1 record for 2 adds).  Fix: register
        // the keyed-collection db type directly (idempotent — same
        // call as gen_set_first_keyed_null and the typedef walker).
        // @PLN93 (#511): the keyed-collection type that drives the record-kind dispatch
        // (`record_new`/`record_finish` read it when the field is `u16::MAX`).  Normally it is
        // the LHS local's own type.  A captured-collection target (`val` = `OpGetDbRef`) has no
        // owning var, so read the collection type the append branch passed as `parent_tp`.
        let cap_target: Option<Value> =
            if !is_field && !self.first_pass && self.is_captured_dbref(val) {
                Some(val.clone())
            } else {
                None
            };
        let keyed_src: Option<Type> = if is_field || self.first_pass {
            None
        } else if vec != u16::MAX {
            Some(self.vars.tp(vec).clone())
        } else if cap_target.is_some() {
            Some(parent_tp.clone())
        } else {
            None
        };
        let lhs_known = match keyed_src.as_ref() {
            Some(tp) => self.keyed_known_type(tp),
            _ => None,
        };
        // #246: `vv[i] += [...]` — `is_field` is true (the LHS is an indexed
        // access) but the parent is a VECTOR, not a struct, so there is no
        // struct/field to locate.  `parse_part`'s `[` branch now records the
        // indexed container's type as `parent_tp`, so a `Type::Vector` parent is
        // the signal — covering both `vv[0]` and `h.vv[0]` (a vector element
        // reached through a struct field, where `parent_tp` would otherwise be
        // the stale struct from the preceding `.field`).  Append directly to the
        // inner vector that `val`'s read yields (`ps[0]`), with `fld = u16::MAX`,
        // mirroring the plain-local path — instead of routing through
        // `new_record_field_op` (struct-only, which mis-locates a field).
        let vector_elem_target: Option<Value> = if is_field
            && matches!(parent_tp, Type::Vector(_, _))
            && let Value::Call(_, ps) = val.unspan()
            && !ps.is_empty()
        {
            Some(ps[0].clone())
        } else {
            None
        };
        // The container new elements are appended INTO — derived ONCE, because all three
        // append ops must agree on it.  `OpNewRecord`/`OpFinishRecord` may name a struct
        // field indirectly (enclosing record plus field number, no resolved handle
        // needed), but `OpAppendCopy` copies WITHIN one vector, so it can only be handed
        // that vector's own handle.  Give the copy the enclosing record instead and it
        // appends into whichever vector that resolves to — the first vector field — so
        // `Pair { a: [0; 3], b: [0; 3] }` builds `a` with seven elements and `b` with one
        // (loft#892).
        let container: Value = if let Some(target) = &vector_elem_target {
            // The inner vector that `val`'s indexed read yields.
            target.clone()
        } else if let Some(target) = &cap_target {
            // The captured collection the `OpGetDbRef` points at.
            target.clone()
        } else if is_field {
            // `val` IS the field read (`OpGetField(record, pos, vector_tp)`), so it
            // already evaluates to the field's own handle.  Unspanned, because
            // `is_field` looks through a span and the container has to agree with the
            // test that selected it — otherwise a spanned field read (`q.a = [7; 3]`,
            // `q.a += [7; 3]`, `vv[i] += [7; 3]`) misses the match below, falls back to
            // `Value::Var(u16::MAX)`, and indexes the variable table out of bounds.
            val.unspan().clone()
        } else {
            Value::Var(vec)
        };
        // Only a struct field reached directly — not through an index, not through a
        // capture — can use the field-numbered form.
        let field_form = is_field && vector_elem_target.is_none() && cap_target.is_none();
        for p in res {
            // route through `vector_of` so narrow integer
            // aliases (i32, u8) produce the same narrow-element vector
            // db type as struct fields get via `fill_database`.  Without
            // this the literal-append path would register
            // `vector<integer>` (8-byte stride) into a narrow-registered
            // local, and reads would mis-align with writes.
            // @PLAN58 cluster-I (boolean outer-handle stride): for a nested
            // element (`in_t` is a vector), the OUTER vector stores 4-byte rec-id
            // HANDLES.  `vector_of(in_t)` yields the ELEMENT type whose content is
            // the inner scalar — so `record_new` strides the outer slot by the
            // inner scalar size.  ≥4 is fine, but a 1-byte `boolean` inner makes
            // adjacent handles OVERLAP.  When the inner content is <4 bytes, pass
            // the OUTER vector type (`vector(elem)`) so `record_new` strides by the
            // handle size (4).  Integer/single (≥4) and the inner scalar append
            // (`in_t` not a vector) are untouched.
            let elem_known = lhs_known.unwrap_or_else(|| self.vector_of(in_t));
            let known_tp = if matches!(in_t, Type::Vector(_, _))
                && self.database.size(self.database.content(elem_known)) < 4
            {
                self.database.vector(elem_known)
            } else {
                elem_known
            };
            let known = Value::Int(i32::from(known_tp));
            if let Value::Return(multiply) = p {
                ls.push(self.cl(
                    "OpAppendCopy",
                    &[container.clone(), *multiply.clone(), known],
                ));
                continue;
            }
            let fld = Value::Int(i32::from(u16::MAX));
            let app_v = if field_form {
                self.new_record_field_op(val, parent_tp, "OpNewRecord")
            } else {
                // `fld = u16::MAX` so `record_new` keys off `known` (the keyed db-type
                // resolved above), the same shape on every addressed container.
                self.cl(
                    "OpNewRecord",
                    &[container.clone(), known.clone(), fld.clone()],
                )
            };
            ls.push(v_set(elm, app_v));
            // @P380 (generalized, plan-58 cluster II): a freshly-created
            // vector-of-vectors element is a VECTOR HANDLE (rec-id at offset 0),
            // but `OpNewRecord` default-inits it with the mis-resolved inner
            // scalar's null sentinel.  For an 8-byte inner (`integer`/`float`)
            // the sentinel's low-32 bits are 0 (a harmless empty handle), but a
            // 4-byte `single` NaN (0x7FC00000) is a non-zero garbage rec-id →
            // wild `get_u32_raw` → SIGSEGV when later read as a rec-id.  Zero the
            // handle on EVERY construction path — the literal/`Insert` path lacked
            // it (only the copy branch below had it), so nested `single` literals
            // crashed.  No-op for the already-zero 8-byte cases.
            if !self.first_pass && matches!(in_t, Type::Vector(_, _)) {
                ls.push(self.cl(
                    "OpSetInt4",
                    &[Value::Var(elm), Value::Int(0), Value::Int(0)],
                ));
            }
            if matches!(
                in_t,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            ) {
                let inner_nr = match in_t {
                    Type::Reference(nr, _) => *nr,
                    _ => self.data.type_def_nr(in_t),
                };
                if let Value::Insert(steps) = p {
                    // Inline struct initialization: the steps already write fields into elm.
                    for l in steps {
                        ls.push(l.clone());
                    }
                } else {
                    // Source is a variable, field access, or function call — the bytes
                    // must be explicitly copied into the new element slot.
                    //
                    // When the source is a STRUCT-RETURNING function call (or an
                    // inline struct-literal Object block — same shape as
                    // `copy_ref::is_struct_returning_call`), set the high
                    // bit (0x8000) on the type parameter so OpCopyRecord
                    // also frees the callee's temporary source store after
                    // the deep copy.  Without this, `vec += [fn_call()]`
                    // leaks one store per call (the `__ref_N` that the
                    // callee's `OpDatabase` allocated to build the return
                    // value).  Mirrors the existing fix in `copy_ref` for
                    // the plain-variable assignment path.  Suppressed
                    // under WASM where frame yield/resume creates store
                    // aliases that cannot yet be tracked.
                    #[cfg(not(feature = "wasm"))]
                    let free_source_bit: i32 =
                        if !self.first_pass && self.is_struct_returning_call(p) {
                            0x8000
                        } else {
                            0
                        };
                    #[cfg(feature = "wasm")]
                    let free_source_bit: i32 = 0;
                    let type_nr = if self.first_pass {
                        Value::Int(i32::from(u16::MAX))
                    } else if matches!(in_t, Type::Vector(_, _)) {
                        // The ELEMENT type of the outer vector, from the shared
                        // resolver — the same id the literal, slice and comprehension
                        // paths use.  `vector(db_type(inner))` named a different row:
                        // `db_type`'s scalar arm sizes a narrow int as if nullable, so
                        // a `vector<vector<u8>>` element deep-copied against a
                        // `vector<short>` row (loft#624 nested).
                        let elem_id = self
                            .data
                            .vector_element_type(in_t, &mut self.database)
                            .unwrap_or(u16::MAX);
                        Value::Int(i32::from(elem_id) | free_source_bit)
                    } else {
                        Value::Int(
                            i32::from(self.data.def(inner_nr).known_type()) | free_source_bit,
                        )
                    };
                    // @P380 handle-zero is now hoisted above (after the element
                    // is created), covering this copy path AND the literal/`Insert`
                    // path; `remove_claims` here sees the already-zeroed handle.
                    ls.push(self.cl("OpCopyRecord", &[p.clone(), Value::Var(elm), type_nr]));
                }
            } else if let Value::Tuple(values) = p {
                // P189c — vector-element tuple literal.  Emit
                // per-attribute writes against the synthetic
                // `__tuple<T1,T2,…>` struct that P189's `tuple_def`
                // registered.  Mirrors the struct-literal path
                // (`Value::Insert` arm below) but tuple literals
                // arrive as `Value::Tuple([v0, v1, …])` without
                // pre-emitted `SetField` steps — `parse_single` at
                // src/parser/vectors.rs:223 builds a bare wrapper —
                // so we emit them here.  `ed_nr` is already the
                // synthetic struct's d_nr (`type_def_nr(Type::Tuple)`),
                // and `set_field` with an explicit attribute index
                // routes through the standard per-field layout
                // dispatch (Integer/Text/Reference/etc.).
                // @PLN25 — a member declared `S?` is STORED as the tagged `__nullable<S>`
                // (`formal/types.md`: an inline struct slot has no out-of-band absent value),
                // while the value was parsed against the spelling the author writes and so
                // arrives DENSE.  Build the tag here or presence is decided by the payload's
                // first byte — `S { a: 0, … }` reads back absent, and `null` reads present.
                // `emit_nullable_slot_write` is the shared home; a struct field holding the
                // same slot goes through it too.
                let src_elems = self.tuple_elements(in_t).unwrap_or_default();
                for (i, val) in values.iter().enumerate() {
                    if !self.first_pass
                        && let Type::Enum(syn, true, _) = self.data.attr_type(ed_nr, i)
                        && let Some(src_tp) = src_elems.get(i)
                        && self.needs_nullable_wrap(syn, src_tp)
                    {
                        let enum_kt = i32::from(self.data.def(syn).known_type());
                        let name = self.data.def(ed_nr).attributes[i].name.clone();
                        let pos = i32::from(
                            self.database
                                .position(self.data.def(ed_nr).known_type(), &name),
                        );
                        let slot = self.cl(
                            "OpGetField",
                            &[Value::Var(elm), Value::Int(pos), Value::Int(enum_kt)],
                        );
                        let write = self.emit_nullable_slot_write(syn, &slot, val.clone());
                        ls.extend(write);
                        continue;
                    }
                    ls.push(self.set_field(ed_nr, i, 0, Value::Var(elm), val.clone()));
                }
            } else if let Value::Insert(steps) = p {
                for l in steps {
                    ls.push(l.clone());
                }
            } else if self.is_null_source(p) && Self::is_collection_type(in_t.base()) {
                // A `null` ELEMENT of a collection-typed element (`vector<vector<T>?>`,
                // and the keyed kinds) is the EMPTY collection — the same rule a `null`
                // reaching a collection FIELD takes (loft#922), because the slot is the
                // same: a 4-byte record id where `0` already means "no records".
                //
                // Without this arm the element fell to the generic `set_field` below,
                // which wrote what `convert` had made of the `null`: a REFERENCE sentinel
                // (`OpNullRefSentinel`, a 16-byte DbRef with `store_nr = u16::MAX`), the
                // right null for a vector VARIABLE, whose slot is a DbRef.  Writing it
                // through the element's 4-byte setter aborted the compiler with an
                // internal assertion — `expected 8B on stack but … pushed 16B` — so
                // `vv += [null]` never reached a diagnostic, let alone a value.
                //
                // Telling this empty from an absent element is the same open question
                // the FIELD has, and has one home: loft#917's reader half.
                ls.push(self.cl(
                    "OpSetInt4",
                    &[Value::Var(elm), Value::Int(0), Value::Int(0)],
                ));
            } else if let Some(op) = self.narrow_elm_set(in_t, elm, p) {
                // @PLN25 item 2 / #624 — narrow integer element write, shared with
                // the slice-materialise site.  The fallback (an element outside the
                // narrow gate) keeps the wide `set_field` path below.
                ls.push(op);
            } else if matches!(in_t, Type::Function(_, _, _)) {
                // Plan-06 phase 4d.A.2 — a fn-ref vector element stores the 4-byte i32
                // d_nr, so the write is `OpSetInt4` (4 bytes) and not `OpSetInt` (8),
                // which would overflow into the next element's slot.  What to WRITE is
                // [`Self::fn_ref_slot_dnr`]'s job — the source is a 20-byte pair and the
                // slot is four bytes of it.  Capturing sources are rejected above (the
                // #247 guard), so discarding the closure half here is lossless.
                let (dnr_val, pre) = self.fn_ref_slot_dnr(p, in_t);
                ls.extend(pre);
                ls.push(self.cl("OpSetInt4", &[Value::Var(elm), Value::Int(0), dnr_val]));
            } else {
                ls.push(self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), p.clone()));
            }
            let finish = if field_form {
                let mut finish_v = self.new_record_field_op(val, parent_tp, "OpFinishRecord");
                // Replace placeholder Var(0) with the actual elm variable.
                if let Value::Call(_, ref mut args) = finish_v
                    && args.len() >= 2
                {
                    args[1] = Value::Var(elm);
                }
                finish_v
            } else {
                // Commit the new element into the container resolved above.
                self.cl(
                    "OpFinishRecord",
                    &[container.clone(), Value::Var(elm), known, fld],
                )
            };
            ls.push(finish);
        }
        ls
    }

    /// Return the database `known_type` of a `main_vector<T>` wrapper struct,
    /// registering it on the spot if it is still unassigned (`u16::MAX`).
    ///
    /// The wrapper is normally registered by `fill_all`'s sweep, which scans
    /// struct fields and (P191) function-local keyed collections — but NOT
    /// function-local plain vectors.  When the element type only resolves on
    /// pass 2 (a cross-package forward reference whose dependency is parsed
    /// after the importing package — #375), the `main_vector<T>` wrapper is
    /// born here during pass-2 codegen, AFTER this file's `fill_all` already
    /// ran, so it never receives a `known_type`.  Codegen would then bake an
    /// `OpDatabase(db_tp=u16::MAX)` operand and crash in `set_default_value` at
    /// runtime.  Mirror the keyed-collection codegen path (`database.hash` /
    /// `database.sorted` register on demand) by filling the wrapper here; the
    /// content type is fully resolved by pass 2.
    ///
    /// `fill_database` registers the wrapper struct and adds its `vector` field,
    /// but the field's POSITION is assigned by `finish_type` (run from
    /// `database.finish()`).  That finish normally runs at end of `parse_file` —
    /// AFTER the codegen that reads the position here — so without finishing now
    /// the `vector` field sits at `u16::MAX` and `OpGetField`/`get_field` read
    /// through a bogus offset, corrupting the interpreter free path (a heap
    /// write that SIGSEGVs at teardown after the correct value prints).  Finish
    /// immediately so the position is laid out before codegen consumes it.
    fn vector_wrapper_known_type(&mut self, vec_def: u32) -> u16 {
        let tp = self.data.def(vec_def).known_type();
        if tp != u16::MAX {
            return tp;
        }
        crate::typedef::fill_database(&mut self.data, &mut self.database, vec_def);
        self.database.finish();
        self.data.def(vec_def).known_type()
    }

    /// loft#1102 — rewrite a tuple member that names a heap LOCAL into an owned copy of it,
    /// answering the copy's type.
    ///
    /// A struct literal deep-copies a vector member into the field's own storage
    /// (`OpAppendVector(OpGetField(s, …), vl)`) and a vector literal copies its elements.  A
    /// tuple had no such storage to copy INTO — its element slot holds a `DbRef` — so it stored
    /// the source's handle and the tuple element and the local became two names for one store.
    /// Nothing at the construction said so, and `@FR-T-Cons` did not cover it.
    ///
    /// The store the copy needs does not have to belong to the TUPLE: a frame-local backing owns
    /// it and frees it at scope exit, exactly as a user-written `o: vector<T> = []; o += vl; o`
    /// does.  That is the shape emitted here, and it is the same one
    /// [`jo_copy_borrowed_arm_yield`](Self::jo_copy_borrowed_arm_yield) uses for a borrowed match
    /// arm.  Built at PARSE time and re-parsed each pass, so `create_unique` and `vector_db` stay
    /// pass-consistent.
    ///
    /// Only a NAMED non-argument local is copied.  A vector ARGUMENT aliases the caller by design
    /// (`@FR-B-Ref-Alias` — a parameter reaches the source), so copying it here would change what
    /// the caller sees; and a member that is already a fresh value has no source to diverge from.
    pub(crate) fn tuple_member_owned_copy(&mut self, val: &mut Value, tp: &Type) -> Option<Type> {
        if self.first_pass {
            return None;
        }
        // `@FR-T-Cons`: a heap element is COPIED into the tuple.  The source is a heap LOCAL,
        // or a member read off a tuple local — the whole-tuple bind and the destructure lower
        // onto this same copy (loft#1361), so the member of a tuple PARAMETER copies too:
        // `u = p` is a plain bind, and `@FR-B-Copy` has no parameter carve-out (`w = v` off a
        // vector parameter copies today).  A bare heap
        // ARGUMENT as a literal member keeps its documented alias.  Anything else is a fresh
        // value with no source to diverge from.
        let src = match val.unspan() {
            Value::Var(v) => {
                if *v >= self.vars.count() || self.vars.is_argument(*v) {
                    return None;
                }
                Value::Var(*v)
            }
            Value::TupleGet(base, i) => {
                if *base >= self.vars.count() {
                    return None;
                }
                Value::TupleGet(*base, *i)
            }
            _ => return None,
        };
        // The KEYED half of `D-tup-4`, closed with the copy the keyed family already has.
        // `OpReplaceKeyed` is what a STRUCT literal emits for a keyed member (`S { h: a }`),
        // and a tuple RETURN copies through the synthetic `__tuple<…>` record — so both
        // siblings `(T-Cons)` names were already independent and only the tuple LITERAL
        // aliased.  The register left this open because the shape was a codegen ICE; that is
        // fixed (loft#1225's `TuplePut` arm), and what remained was reaching for the copy.
        //
        // The copy keeps the SOURCE's nullability.  `keyed_known_type` is nullability-agnostic
        // so the store type is the same either way, but a tuple's element slot is `τ?` when
        // the source is, and a dense-typed copy loses its ownership dep entering that slot —
        // which leaks the copy's store.
        // The backing is created OWNED.  The member's type may carry the deps of the tuple it
        // was read from — a copied tuple's member depends on the literal's own backing — and a
        // backing that inherited them would be read as a borrow and never freed.
        let owned_create = tp.with_deps(&Deps::none());
        // The backing for a KEYED or STRUCT member is a function-scope work ref (`__ref_N`),
        // the same temp an inline record literal mints: declared at the body's top and freed
        // at function exit, so a copy built inside an ARGUMENT tuple (`show((s, 5))`) is
        // released too.  A block-scoped local was the block's own value and nothing freed it
        // there.  The vector branch's store is its `__vdb`, which already has that discipline.
        if is_keyed(tp) {
            let keyed_tp = owned_create.clone();
            let kt = self.keyed_known_type(&keyed_tp)?;
            let o = self.vars.work_refs(&keyed_tp, &mut self.lexer);
            if o == u16::MAX {
                return None;
            }
            let db = self.cl("OpDatabase", &[Value::Var(o), Value::Int(i32::from(kt))]);
            let copy = self.cl(
                "OpReplaceKeyed",
                &[src, Value::Var(o), Value::Int(i32::from(kt))],
            );
            let ops = vec![v_set(o, Value::Null), db, copy, Value::Var(o)];
            // DEPENDING on `o`, exactly as the vector branch's result does: that dep is what
            // tells the scope pass the copy's store has an owner in this frame.
            let owned_tp = keyed_tp.depending(o);
            *val.unspan_mut() = crate::data::v_block(ops, owned_tp.clone(), "tuple_member_copy");
            return Some(owned_tp);
        }
        // A STRUCT (or struct-enum) member: the record is copied into a frame-local backing
        // the same way (`OpDatabase` + `OpCopyRecord`, the shape a whole-value struct bind
        // emits).  A nullable source copies only when it holds a record — a null stays null,
        // which `OpCopyRecord` alone would turn into a default record.
        if let Some(d_nr) = match tp.base() {
            Type::Reference(d, _) | Type::Enum(d, true, _) => Some(*d),
            _ => None,
        } {
            // A member typed by a generic's TYPE VARIABLE is not copied here.  Its record
            // row is the template placeholder (`__typevar_T`), which no record may be
            // allocated with (loft#1070's guard), and what the member IS — a scalar with
            // nothing to copy, or a record — is decided per instantiation, after this parse.
            // So the generic keeps the pre-copy layout, the element slot holding the local's
            // handle: `formal/tuples.md` D-tup-9 (loft#1365) records that a struct-bound `T`
            // therefore still aliases, and names the monomorph-time copy as the cure.
            let kt = self.data.def(d_nr).known_type();
            if kt == u16::MAX {
                return None;
            }
            let o = self.vars.work_refs(&owned_create, &mut self.lexer);
            if o == u16::MAX {
                return None;
            }
            let db = self.cl("OpDatabase", &[Value::Var(o), Value::Int(i32::from(kt))]);
            let copy = self.cl(
                "OpCopyRecord",
                &[src.clone(), Value::Var(o), Value::Int(i32::from(kt))],
            );
            let fill = if matches!(tp, Type::Optional(_))
                && let Some(is_null) = self.null_test(src, tp, false)
            {
                crate::data::v_if(
                    is_null,
                    Value::Null,
                    crate::data::v_block(vec![db, copy], Type::Void, "tuple_member_copy_fill"),
                )
            } else {
                crate::data::v_block(vec![db, copy], Type::Void, "tuple_member_copy_fill")
            };
            let ops = vec![v_set(o, Value::Null), fill, Value::Var(o)];
            let owned_tp = owned_create.depending(o);
            *val.unspan_mut() = crate::data::v_block(ops, owned_tp.clone(), "tuple_member_copy");
            return Some(owned_tp);
        }
        // A nested TUPLE member with a heap leaf: hold the inner tuple once, then copy each
        // of ITS heap members through this same function — the copy is one level deep per
        // call and recursion reaches every leaf.
        if let Type::Tuple(elems) = tp
            && elems.iter().any(|e| !crate::data::is_scalar(e.base()))
        {
            // The hold BORROWS its source; it is the one branch here that mints no store.
            // The three above create a backing (`OpDatabase` + a copy) and so are created
            // with `owned_create`, whose empty dep list is `@FR-O-Proxy`'s proxy for "this
            // binding owns its store" — which is what places their free.  This hold only
            // names the source tuple so each member can be read once, so `@FR-O-Borrow`
            // applies instead: it is tracked and never frees, and `@FR-O-Derived` then
            // derives no free for it.  Created with the SOURCE's dep, since `with_deps`
            // carries a tuple's dep into every element and the free is decided per element.
            let base = match &src {
                Value::Var(v) => *v,
                Value::TupleGet(b, _) => *b,
                _ => return None,
            };
            let hold_tp = tp.depending(base);
            let h = self.create_unique("tuphold", &hold_tp);
            if h == u16::MAX {
                return None;
            }
            self.vars.defined(h);
            let mut members: Vec<Value> = Vec::with_capacity(elems.len());
            let mut types = elems.clone();
            for (i, t) in elems.iter().enumerate() {
                let mut m = Value::TupleGet(h, i as u16);
                if let Some(owned) = self.tuple_member_owned_copy(&mut m, t) {
                    types[i] = owned;
                }
                members.push(m);
            }
            let owned_tp = Type::Tuple(types);
            let ops = vec![v_set(h, src), Value::Tuple(members)];
            *val.unspan_mut() = crate::data::v_block(ops, owned_tp.clone(), "tuple_member_copy");
            return Some(owned_tp);
        }
        let Type::Vector(b, _) = tp else {
            return None;
        };
        let elm = (**b).clone();
        let o = self.create_unique("tupcopy", &owned_create);
        if o == u16::MAX {
            return None;
        }
        self.vars.defined(o);
        let mut ops = self.vector_db(tp, o);
        // The clear says once, where the replace is: a no-op on a fresh store and correct on a
        // reused one — the same reason the match-arm copy beside this one clears.
        ops.push(self.cl("OpClearVector", &[Value::Var(o)]));
        let elem_tp = self.append_elem_tp(&elm);
        ops.push(self.cl("OpAppendVector", &[Value::Var(o), src, Value::Int(elem_tp)]));
        ops.push(Value::Var(o));
        let owned_tp = Type::Vector(Box::new(elm), Deps::frame1(o));
        *val.unspan_mut() = crate::data::v_block(ops, owned_tp.clone(), "tuple_member_copy");
        Some(owned_tp)
    }

    /// The heap local a [`Self::tuple_member_owned_copy`] block was built from — `None` for
    /// any other value.
    ///
    /// That copy exists so a tuple ELEMENT does not become a second name for the member's
    /// store.  A tuple written as a function's RETURN tail is rewritten to a synthetic
    /// `__tuple<…>` record whose vector field is filled by `set_field_no_check`, and THAT
    /// copies into the record's own storage — so on the return path the member is copied into
    /// a frame-local backing and then copied again out of it.  The rewrite unwraps back to
    /// this source and lets the record's copy be the only one (loft#1109); the element is
    /// still copied, so `@FR-T-Cons` holds and the local cannot alias the tuple.
    ///
    /// Only the RETURN path may unwrap.  A tuple literal bound to a LOCAL has no second copy
    /// to fall back on, and dropping the backing there is exactly the aliasing loft#1102
    /// closed.
    ///
    /// One home, two readers, kept adjacent on purpose: the block is BUILT directly above and
    /// MATCHED here, so its shape and the matcher cannot drift into different files.
    pub(crate) fn tuple_member_copy_source(&self, val: &Value) -> Option<Value> {
        let Value::Block(b) = val.unspan() else {
            return None;
        };
        if b.name != "tuple_member_copy" {
            return None;
        }
        // The block ends `OpAppendVector(backing, source, elem_tp); backing` for a VECTOR
        // member, `OpReplaceKeyed(source, backing, tp); backing` for a KEYED one and
        // `OpCopyRecord(source, backing, tp)` (inside the fill block) for a STRUCT one, and the
        // SOURCE is the only part of any of them that the record's own copy still needs.  The
        // source sits at a DIFFERENT argument position, which is why this cannot be a
        // name-agnostic `args.get(1)`.
        // The STRUCT branch nests its `OpCopyRecord` one block down (the fill, guarded by a
        // null test when the source is nullable); its source is the first argument too.
        fn source_in(data: &crate::data::Data, ops: &[Value]) -> Option<Value> {
            ops.iter().rev().find_map(|op| match op.unspan() {
                Value::Call(d, args) => match data.def(*d).name() {
                    "OpAppendVector" => args.get(1).cloned(),
                    "OpReplaceKeyed" | "OpCopyRecord" => args.first().cloned(),
                    _ => None,
                },
                Value::Block(inner) if inner.name == "tuple_member_copy_fill" => {
                    source_in(data, &inner.operators)
                }
                Value::If(_, t, f) => source_in(data, std::slice::from_ref(t.as_ref()))
                    .or_else(|| source_in(data, std::slice::from_ref(f.as_ref()))),
                _ => None,
            })
        }
        source_in(&self.data, &b.operators)
    }

    pub(crate) fn vector_db(&mut self, assign_tp: &Type, vec: u16) -> Vec<Value> {
        self.vector_db_init(assign_tp, vec, 0, false)
    }

    /// [`Self::vector_db`] with the collection slot's initial value spelled out.
    ///
    /// `0` is the EMPTY collection and is what a backing minted for a write wants.
    /// `DbRef::ABSENT_REC` is the reserved id that means ABSENT (loft#917), and it is what a
    /// backing minted to give a NULL local something shareable wants: the local gains its
    /// slot without gaining a collection, so `v == null` still answers true —
    /// `vector::is_absent_collection` reads the slot, not the handle.
    ///
    /// `guarded` mints only into a local whose HANDLE is still null, and is for a site that
    /// can run more than once — a closure built inside a loop mints at every pass, and an
    /// unguarded mint then re-points the local at a fresh store and orphans what the previous
    /// pass put in it (the shape loft#1219 measured at the append site).  The test is
    /// `OpRefIsNull` and not `OpVectorIsNull`: the slot this writes says ABSENT, so the
    /// collection test answers true afterwards as well and would re-mint every time.  The
    /// backing is `mark_inline_ref` for the same reason the rebind branch above is — a
    /// CONDITIONAL `OpDatabase` leaves the untaken path holding an eagerly allocated store.
    pub(crate) fn vector_db_init(
        &mut self,
        assign_tp: &Type,
        vec: u16,
        slot_init: i32,
        guarded: bool,
    ) -> Vec<Value> {
        // @PLN87 P2.4 — a REBIND vector param (`v = [..]` whole-binding replace on
        // a visible vector param, marked via `ensure_rebind_witness`) DOES get a
        // fresh backing: it rebinds locally rather than appending to the caller's
        // store.  Every other argument keeps the caller-provided backing (no
        // local `__vdb` that would be freed before the return).
        let rebind = self.vars.rebind_orig(vec).is_some();
        // loft#703 — a keyed local already owns a keyed store; wrapping it in a
        // `vector<T>` backing repointed `h` at `OpGetField(__vdb, …)`, so the elements
        // went in through the keyed container and came back out through the vector one.
        if self.first_pass
            || vec == u16::MAX
            || (self.vars.is_argument(vec) && !rebind)
            || self.keyed_local(vec)
        {
            Vec::new()
        } else {
            let mut ls = Vec::new();
            let vec_def = self.data.vector_def(&mut self.lexer, assign_tp);
            let db = self
                .vars
                .work_vec_db(&Type::Reference(vec_def, Deps::none()), &mut self.lexer);
            // The rebind param's fresh backing is freed at function exit by the
            // param's own `OpFreeRefIfDistinct(v, witness)` (it frees v's store);
            // skip_free keeps scopes from ALSO freeing the `__vdb` (a double-free).
            if rebind {
                self.vars.set_skip_free(db);
                // inline_ref makes the `__vdb`'s ENTRY null-init a non-allocating
                // sentinel rather than `OpInitRef` (which eagerly allocates a store).
                // The null-init is hoisted to function scope, but the `OpDatabase`
                // that fills it sits at the rebind site — which may be CONDITIONAL
                // (`if c { v = [..] }`).  Without the sentinel, the untaken path
                // leaks the eagerly-allocated backing.  `OpDatabase` allocates fresh
                // from the sentinel when the rebind does run.
                self.vars.mark_inline_ref(db);
                // Pre-free a PRIOR rebind backing before repointing `vec` at this
                // fresh one: on the FIRST rebind `vec == witness` (the caller's
                // store) so it no-ops; on a REPEAT (`v = [..]; v = [..]`) `vec` is
                // the previous fresh `__vdb`, now orphaned, so it is freed — without
                // this, every repeat rebind leaks the prior backing.  Emitted INSIDE
                // vector_db's op list (not as an outer `Insert` wrap, which would
                // make `create_vector` re-fire and re-allocate), and BEFORE the
                // `OpDatabase` below that repoints `vec`.
                if let Some(orig) = self.vars.rebind_orig(vec) {
                    ls.push(self.cl("OpFreeRefIfDistinct", &[Value::Var(vec), Value::Var(orig)]));
                }
            }
            self.vars.depend(vec, db);
            let tp = self.vector_wrapper_known_type(vec_def);
            debug_assert_ne!(
                tp,
                u16::MAX,
                "Undefined type {} at {}",
                self.data.def(vec_def).name(),
                self.lexer.pos()
            );
            ls.push(self.cl("OpDatabase", &[Value::Var(db), Value::Int(i32::from(tp))]));
            // Reference to the vector field.
            ls.push(v_set(vec, self.get_field(vec_def, 0, Value::Var(db))));
            // Write the initial slot value into this reference.
            ls.push(self.set_field(vec_def, 0, 0, Value::Var(db), Value::Int(slot_init)));
            if guarded {
                self.vars.mark_inline_ref(db);
                let test = self.cl("OpRefIsNull", &[Value::Var(vec)]);
                return vec![crate::data::v_if(test, Value::Insert(ls), Value::Null)];
            }
            ls
        }
    }

    pub(crate) fn insert_new(
        &mut self,
        vec: u16,
        elm: u16,
        in_t: &Type,
        ls: &mut Vec<Value>,
    ) -> u16 {
        // determine the element size by the resulting type
        let vec_def = self.data.vector_def(&mut self.lexer, in_t);
        // Use work_vec_db (separate __vdb_N counter) so that these calls do NOT
        // consume __ref_N counter slots.  Both vector_db and insert_new contribute
        // to the __vdb_N namespace; at any given vector site exactly one of them
        // runs per pass (vector_db is guarded by !first_pass; insert_new is called
        // on first pass when vector_db has not yet created a dep, but on second pass
        // vector_needs_db returns false after vector_db ran, so insert_new is
        // skipped).  The __ref_N counter is reserved exclusively for add_defaults
        // and other return-value work-refs, ensuring ref_return can match the same
        // name across both passes.
        let db = self
            .vars
            .work_vec_db(&Type::Reference(vec_def, Deps::none()), &mut self.lexer);
        self.vars.depend(elm, db);
        self.vars.depend(vec, db);
        let known = Value::Int(i32::from(self.vector_wrapper_known_type(vec_def)));
        ls.insert(0, self.cl("OpDatabase", &[Value::Var(db), known]));
        // Reference to the vector field.
        ls.insert(1, v_set(vec, self.get_field(vec_def, 0, Value::Var(db))));
        // Write 0 into this reference.
        ls.insert(
            2,
            self.set_field(vec_def, 0, 0, Value::Var(db), Value::Int(0)),
        );
        db
    }

    pub(crate) fn type_info(&mut self, in_t: &Type) -> Value {
        Value::Int(i32::from(self.get_type(in_t)))
    }

    pub(crate) fn get_type(&mut self, in_t: &Type) -> u16 {
        if self.first_pass {
            return u16::MAX;
        }
        match in_t {
            Type::Integer(spec) => {
                // honour `forced_size` via
                // `vector_narrow_width`.  Gate covers 1/2/4 bytes.
                // 2-byte uses `Parts::ShortRaw` (direct encoding) —
                // parallel to `Parts::Byte` / `Parts::Int` so that
                // source literal vectors and destination fields share
                // encoding and `vector_add`'s raw-byte copy stays
                // valid.  Narrow path registers on demand so locals /
                // params / returns don't depend on a struct field
                // having registered the name first.
                // loft#1036 — one derivation, and it REGISTERS.  The width now comes
                // from `byte_width` (`forced_size` first, then the range), so the
                // `limit(lo, hi)` spelling lands in this arm too.  It used to fall to
                // a bounds-heuristic branch that only LOOKED UP a name — and looked up
                // `short<min,false>` (the `+1` sentinel encoding) where this arm
                // registers `short_raw` (direct), so the two spellings of one range
                // could not agree on a Part even when both found one.
                match spec.vector_narrow_width(false) {
                    Some(1) => self.database.byte(spec.min, false),
                    Some(2) => self.database.short_raw(spec.min, false),
                    Some(4) => self.database.int(spec.min, false),
                    _ => self.database.name("integer"),
                }
            }
            Type::Character => self.database.name("integer"),
            Type::Float => self.database.name("float"),
            Type::Single => self.database.name("single"),
            Type::Text(_) => self.database.name("text"),
            // Plan-06 phase 4d.A.2 — fn-ref in a vector stores as
            // 4-byte i32 d_nr.  Use the same DB type as `i32`
            // (signed 32-bit, registered via `database.int(0, false)`)
            // so vector storage uses the flat narrow-int path.  The
            // semantic difference (d_nr vs. integer) is recovered at
            // read-back time via fn-ref unbox.
            Type::Function(_, _, _) => self.database.int(0, false),
            Type::Reference(r, _) | Type::Enum(r, _, _) => self.data.def(*r).known_type(),
            Type::Hash(tp, key, _) => {
                let mut name = "hash<".to_string() + self.data.def(*tp).name() + "[";
                self.database
                    .field_name(self.data.def(*tp).known_type(), key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                // P190 — local-var hash iteration: register on demand.
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.hash(c_tp, key)
            }
            Type::Radix(tp, key, _) => {
                // @PLN48 — mirror Hash: resolve the spatial<T[…]> db type id, and
                // register it on demand for a local-only var (whose type would else
                // be absent from the schema, so iteration/`get_type` sees u16::MAX).
                let mut name = "spatial<".to_string() + self.data.def(*tp).name() + "[";
                self.database
                    .field_name(self.data.def(*tp).known_type(), key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.spatial(c_tp, key)
            }
            Type::Trie(tp, key, _) => {
                // The Radix shape, for a trie: resolve the registered id, and register
                // on demand for a local-only var whose type is otherwise absent from
                // the schema.  Same spelling `Stores::trie` uses.
                let name = format!("trie<{}[{key}]>", self.data.def(*tp).name());
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.trie(c_tp, key)
            }
            Type::Sorted(tp, key, _) => {
                let mut name = "sorted<".to_string() + self.data.def(*tp).name() + "[";
                field_id(key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                let mut ordered = "ordered<".to_string() + self.data.def(*tp).name() + "[";
                field_id(key, &mut ordered);
                let r = self.database.name(&ordered);
                if r != u16::MAX {
                    return r;
                }
                // P190 — local-var keyed collection iteration: the
                // sorted/ordered type wasn't pre-registered by
                // fill_database (which only runs on struct fields).
                // Register on demand here so OpIterate gets the right
                // db type id and `fill_iter` produces all 6 args.
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.sorted(c_tp, key)
            }
            Type::Index(tp, key, _) => {
                let mut name = "index<".to_string() + self.data.def(*tp).name() + "[";
                field_id(key, &mut name);
                let r = self.database.name(&name);
                if r != u16::MAX {
                    return r;
                }
                // P190 — same on-demand registration for local-var index.
                let c_tp = self.data.def(*tp).known_type();
                if c_tp == u16::MAX {
                    return u16::MAX;
                }
                self.database.index(c_tp, key)
            }
            Type::Vector(tp, _) => {
                // route through `vector_of` so narrow-alias
                // content (vector<i32>, vector<u8>) registers the same
                // narrow vector db_tp that `fill_database` registers for
                // struct fields.  Without this, locals / returns / file
                // writes fall back to `database.name(...)` which only
                // succeeds for pre-registered type names and returns
                // `u16::MAX` otherwise — triggering an index-out-of-bounds
                // in `assemble_write_data` when the resulting db_tp is
                // passed to the write path.
                self.vector_of(tp)
            }
            // @PLN25 — `Optional(τ)` is a compile-time nullability marker over τ's own
            // storage, so it names the SAME db type.  Its sibling resolvers
            // (`type_def_nr`, `type_elm`, `rust_type`, `element_stack_size`) all peel;
            // this one did not, so an `Optional`-typed collection answered `u16::MAX` —
            // the "no such type" sentinel — which callers read as "not a collection"
            // (loft#909).
            Type::Optional(inner) => self.get_type(inner),
            _ => u16::MAX,
        }
    }

    /// Classify a `c += e` source against the collection `dest` it is appended to — the one
    /// home for [`AppendSource`], so the routes below cannot disagree about which of them
    /// owns a statement, and a source none of them owns is NAMED rather than guessed at.
    ///
    /// The element test is asked of [`Parser::can_convert`], the predicate that already
    /// answers *"may this value satisfy that slot"* for arguments, returns and struct
    /// literals.  It has to be that one, because "the element type" has more than one
    /// spelling and a fresh `is_equal` knows only the first: a nullable record is
    /// `Reference(d)` where `content()` reads it off a keyed kind and `Enum(d, true, …)`
    /// where a vector carries it (@FR-L-Null), and `(C-Var)` makes a VARIANT satisfy its
    /// enum, so `vector<Tagged>` holds a `Named`.  Both spellings are live in the corpus.
    ///
    /// `Unknown` on either side answers [`AppendSource::Whole`] — a generic body carries
    /// placeholder types until monomorphisation, and refusing there would refuse the
    /// template rather than any of its instances.
    /// Does this source VALUE name a collection that ALREADY EXISTS — a binding, a field, a
    /// capture, a tuple member — rather than one constructed on the spot?
    ///
    /// [`Self::append_source`] answers with TYPES, and a keyed literal is built THROUGH its
    /// destination (loft#703), so `t.0 += [E { … }]` reports the destination's own
    /// `hash<E[k]>` and reads as `Whole`.  The refusal that fires on `Whole` is about MERGING
    /// two collections, which is a thing only a source that names one can be: three place
    /// kinds never reach it because they parse the literal per element first, and the fourth —
    /// a TUPLE ELEMENT, which builds the append through a `__kvb_N` accumulator — does, so a
    /// bracketed one-element append there was refused as a merge.
    pub(crate) fn source_names_a_collection(&self, val: &Value) -> bool {
        match val.unspan() {
            Value::Var(v) => *v != u16::MAX,
            Value::TupleGet(_, _) => true,
            Value::Call(d, _) => self.data.def(*d).name().starts_with("OpGet"),
            _ => false,
        }
    }

    pub(crate) fn append_source(&mut self, dest: &Type, s_type: &Type) -> AppendSource {
        let dest = dest.base();
        let content = dest.content();
        if s_type.is_unknown() || content.is_unknown() {
            return AppendSource::Whole;
        }
        if s_type.is_equal(dest) {
            return AppendSource::Whole;
        }
        if self.holds_element(&content, s_type) {
            return AppendSource::Element;
        }
        if is_keyed(dest)
            && let Type::Vector(elm, _) = s_type.base()
            && self.holds_element(&content, elm)
        {
            return AppendSource::ElementVector;
        }
        AppendSource::Unrelated
    }

    /// May a value of type `src` be ONE element of a collection whose element type is
    /// `content`?  Delegates to [`Parser::can_convert`] in both directions, because the two
    /// spellings of a record element are not ordered: a keyed kind reads its element off
    /// `content()` as `Reference(d)` while a vector carries the synthetic `Enum(d, true, …)`,
    /// and which side holds which depends on the destination's kind rather than on the value.
    pub(crate) fn holds_element(&mut self, content: &Type, src: &Type) -> bool {
        // `Unknown` is not a match, and saying so here is what keeps the predicate usable by a
        // REFUSAL.  [`Parser::can_convert`] answers TRUE for an unknown `test_type` — the right
        // answer for a validator, which must not report a generic body's placeholder as a
        // mismatch — but read as "this IS an element" it turns every unresolved element type
        // into a hit: a struct-enum's collection field resolves lazily, so `j.xs += [Item { … }]`
        // asked while `xs` was still `Unknown` looked like a bare element append and earned
        // @PLAN52's ambiguity refusal with the brackets already written.
        if content.is_unknown() || src.is_unknown() {
            return false;
        }
        content.is_equal(src) || self.can_convert(src, content) || self.can_convert(content, src)
    }

    // <children> ::=
}

/// Does this type name a KEYED collection — one whose elements are reached by key
/// rather than by position (`hash` / `sorted` / `index` / `spatial`)?
///
/// loft#703: a `[…]` literal infers `vector<T>`, and a keyed type is not a wider or
/// narrower version of that but a DIFFERENT container, so the two are never
/// interchangeable and every site that builds a literal has to ask which it is.
///
/// **The one home for this question.**  It is the union of the five kind rules, and all five
/// are cited on purpose: a sixth keyed kind must update exactly this function, so
/// `idx tag:@FR-Col-Spatial` has to return it.  Spelled inline at a call site instead, the
/// list drifts — there is nothing to keep the copies in step.
///
/// Enforces @FR-Col-Hash · @FR-Col-Sorted · @FR-Col-Index · @FR-Col-Spatial · @FR-Col-Trie.
///
/// ⚠ No rule names the KEYED FAMILY as a category, though that is the question 16 sites
/// actually ask — see doc/claude/formal/IMPLEMENTATIONS.md.
///
/// Nullability-agnostic: the operand is read through `.base()`.  A `hash<S[k]>?` is a keyed
/// collection that may be absent, and `Optional(τ)` occupies τ's storage exactly
/// (@FR-L-Null), so which keyed kind a type names does not depend on the wrapper.  Asking
/// bare instead answers "not keyed" for every `τ?`, and the call sites that gate a WRITE on
/// it then emit no write at all.
///
/// [`is_collection`] must peel on BOTH of its arms for the same reason.  The two differ by
/// the `Vector` variant and by nothing else — a difference on the nullability axis makes a
/// `vector<τ>?` the one collection `is_collection` denies, which is loft#1207.
impl Parser {
    /// Refuse a KEYED collection in a vector ELEMENT position, and say so once.
    ///
    /// `true` when it fired, so the caller can abandon the construction.
    ///
    /// One home for a refusal two sites have to make: the DECLARED spelling
    /// (`vector<hash<E[k]>>`, caught while the type is parsed) and the INFERRED one
    /// (`hs = [mk(1), mk(2)]`, which writes no type and so passed the first site by).  A
    /// keyed collection has no element form anything can write — measured: giving the
    /// element the collection's own registered row gets past the type table and then
    /// corrupts the block chain, so the storage has no room for one either (loft#923,
    /// loft#1298).
    ///
    /// Named by its KIND, not by `Type::name`: a keyed type's registered name carries its
    /// key list in the schema's own spelling (`sorted<E,[("k", true)]>`), which is not what
    /// the author wrote and not something to hand back to them.
    pub(crate) fn refuse_keyed_vector_element(&mut self, tp: &Type) -> bool {
        let kind = match tp.base() {
            Type::Hash(_, _, _) => "hash",
            Type::Sorted(_, _, _) => "sorted",
            Type::Index(_, _, _) => "index",
            Type::Radix(_, _, _) => "spatial",
            Type::Trie(_, _, _) => "trie",
            _ => return false,
        };
        diagnostic!(
            self.lexer,
            Level::Error,
            "a `{kind}` cannot be a vector ELEMENT — a keyed collection has no element \
             form anything can write, so `vector<{kind}<…>>` could only ever be declared \
             and stay empty. Hold it in a struct and make a vector of THAT: the extra \
             record is what the element would have been anyway."
        );
        true
    }
}

pub(crate) fn is_keyed(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
    )
}

/// Does this type name any collection a `[…]` literal can build — keyed or vector?
///
/// Enforces @FR-Col-Store, whose store-backed set is exactly
/// `Parts::{Vector, Hash, Sorted, Radix, Trie}`.  Checklist #4 in
/// doc/claude/formal/IMPLEMENTATIONS.md: this is the `is_keyed` set plus `Vector`, and the
/// two differ by that one variant BY DESIGN — not a drifted copy of each other.
///
/// That "one variant" is the whole difference, which is why the `Vector` arm reads
/// `tp.base()` exactly as [`is_keyed`] does.  While it matched bare, the two predicates
/// disagreed on a second axis nothing documented: a `vector<τ>?` was the one collection this
/// answered "no" for, so `towards_set`'s collection interception — asked in PASS 1, before
/// any `!first_pass` route can claim the statement — let a nullable vector append fall
/// through to the generic operator lookup and be refused as *"No matching operator 'Add'"*
/// (loft#1207).
pub(crate) fn is_collection(tp: &Type) -> bool {
    is_keyed(tp) || matches!(tp.base(), Type::Vector(_, _))
}

/// What a `c += e` source IS, relative to the collection it is being appended to.
///
/// @FR-Col-Insert is stated over `c += [ rec, … ]` — a record joins the collection — and the
/// routes that implement it each serve ONE spelling of that source: the whole collection
/// (concatenation), a single element, or a vector of elements aimed at a keyed kind.  Naming
/// the three in one place is what makes the FOURTH answer expressible: a source the
/// destination cannot hold at all.
///
/// Without that answer the routes are a partial list with no `else`, and each of the two
/// failure directions has shipped.  A source matching no route reached whichever route tests
/// LEAST — the vector single-element push, which compares nothing — and was written raw into
/// an element slot: a `float` read back as its IEEE-754 bits, a `text` panicking the
/// allocator, `--native` refusing to compile the Rust it emitted (loft#1215).  A keyed
/// destination has no such catch-all, so the same source fell past every route to a statement
/// that emitted no write, and the append vanished with `len` reading 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppendSource {
    /// The source IS the collection — `v += other_v`, concatenation.
    Whole,
    /// The source is ONE element.  A keyed kind places it by key; a vector requires the
    /// `+= [elem]` brackets (@PLAN52), so for a vector this is a diagnosis, not a route.
    Element,
    /// A vector OF elements aimed at a KEYED destination — loft#1159's bulk fill.
    ElementVector,
    /// Nothing the destination can hold.
    Unrelated,
}

/// P216: walk a captured variable's `Type` and call `tuple_def` for
/// every `Type::Tuple` (including nested) so the synthetic
/// `__tuple<…>` struct exists by the time `fill_database` walks the
/// closure record's attributes.  Without this, a tuple-typed capture
/// surfaces `u32::MAX` from `type_elm` and the attribute is silently
/// skipped — closure record allocates with size 0 → `OpDatabase`
/// panics "Incomplete record" / native dispatches with field offsets
/// at `u16::MAX` → silent corruption.
fn ensure_tuple_defs_for_capture(
    data: &mut crate::data::Data,
    lexer: &mut crate::lexer::Lexer,
    tp: &Type,
) {
    data.ensure_tuple_defs(lexer, tp);
}

/// Plan-22 phase 02d-ii — canonical cell-struct name for a scalar
/// type.
///
/// Returns the conventional name `__cell_<T>` used to box a scalar
/// capture into a 1-field record so closure mutations propagate
/// back through the auto-Reference path (phase 02b/02c encoding).
///
/// Returns `None` for a type that takes no cell at all — a reference, a collection, a
/// function.  Every SCALAR gets one, at every integer width: `@FR-L-CapWrite` shares a
/// captured scalar in the write direction whatever its type, so a scalar the namer
/// declined would keep an un-boxed stack slot and lose the closure's write on any call
/// that is not direct.
///
/// Naming table (one cell per storage identity, deduped across all
/// captures):
///
/// | Loft type | Cell name |
/// |---|---|
/// | `integer` (canonical 8-byte) | `__cell_integer` |
/// | `long` / wide integer (8-byte) | `__cell_long` |
/// | `float` | `__cell_float` |
/// | `single` | `__cell_single` |
/// | `boolean` | `__cell_boolean` |
/// | `character` | `__cell_character` |
/// | `text` | `__cell_text` |
/// | a narrow width (`u8`, `i16`, `integer limit(0, 100)`) | `__cell_int<width>_<min>_<max>` |
/// | plain enum `E` | `__cell_enum_<E>` |
///
/// A nullable capture takes `__cell_opt_<stem>` instead (see below).
pub(crate) fn cell_struct_name(tp: &Type, data: &crate::data::Data) -> Option<String> {
    // A NULLABLE capture takes a cell of its own.  `Optional(Ï)` shares `Ï`'s storage
    // in-band (C90), so the cell reads and writes with the same ops — but the `value`
    // field has to DECLARE the nullability, because a `Ï` field may not hold the
    // sentinel (`@FR-N-Store`).  One field cannot be spelled both ways, so the answer is
    // a second cell rather than a widened one, and the dense path is left untouched.
    let (base, nullable) = tp.peel_optional();
    let stem = cell_stem(base, data)?;
    Some(if nullable {
        format!("__cell_opt_{stem}")
    } else {
        format!("__cell_{stem}")
    })
}

/// The canonical cell name for a NON-nullable scalar, without the `__cell_` prefix —
/// the naming table above, and the one place the boxable set is spelled.
///
/// `None` means "this type gets no cell": a reference, a collection, a function.  A
/// capture the namer declines keeps an un-boxed stack slot, which only carries a
/// closure's write on a DIRECT call — so declining a scalar breaks `@FR-L-CapWrite`.
fn cell_stem(tp: &Type, data: &crate::data::Data) -> Option<String> {
    // Name the STORAGE the cell takes, not the capture's spelling.  `cell_value_type`
    // is the one decision about what the `value` field holds, so deriving the name from
    // its answer keeps the two halves of a cell's identity — what it is called and what
    // it stores — from drifting apart.  They are required to agree: the name is what
    // dedupes cells, so two captures share storage exactly when they share a name.
    match cell_value_type(tp) {
        Type::Integer(spec) => Some(int_cell_stem(&spec)),
        Type::Float => Some("float".to_string()),
        Type::Single => Some("single".to_string()),
        Type::Boolean => Some("boolean".to_string()),
        Type::Character => Some("character".to_string()),
        Type::Text(_) => Some("text".to_string()),
        Type::Enum(d_nr, false, _) => {
            let enum_name = data.def(d_nr).name();
            Some(format!("enum_{enum_name}"))
        }
        _ => None,
    }
}

/// The stem naming an integer cell, carrying the spec's whole storage identity.
///
/// Two captures may share one cell only when their `value` field stores AND decodes
/// identically, so the width, the bounds and the null-flag all reach the name.  Naming a
/// narrow capture after its alias instead would give `integer limit(0, 100) size(1)` and
/// `u8` a single cell, and the first one's declared range would silently widen to the
/// second's — `@FR-L-CapBox`: boxing moves where a captured scalar lives and changes
/// nothing its type promises.
///
/// The two canonical 8-byte templates keep the plain `integer` / `long` names every cell
/// has always carried; only a narrow width needs the encoded form.
fn int_cell_stem(spec: &crate::data::IntegerSpec) -> String {
    if spec.forced_size.is_none() && spec.byte_width(true) == 8 {
        return if spec.max == u32::MAX {
            "long"
        } else {
            "integer"
        }
        .to_string();
    }
    // `m` for a negative bound: the stem is a definition NAME, so it may not carry `-`.
    let lo = if spec.min < 0 {
        format!("m{}", spec.min.unsigned_abs())
    } else {
        spec.min.to_string()
    };
    let nn = if spec.not_null { "_nn" } else { "" };
    format!("int{}_{lo}_{}{nn}", spec.byte_width(true), spec.max)
}

/// The `__cell_<T>` definition a type points at, or `None` when the type is not a
/// boxed scalar — the inverse of [`cell_struct_name`], and the ONE home for the
/// question *"is this a boxed scalar?"*.
///
/// A capture a closure MUTATES is boxed: the local's type becomes
/// `Reference(__cell_<T>)`, every read of it is rewritten into `OpGet<T>(Var, 0)`
/// (`auto_deref_boxed_scalar`) and every write into the matching `OpSet<T>`.  A
/// site that must tell such a local from an ordinary one asks here, so the sites
/// cannot drift apart on what the box looks like.
pub(crate) fn boxed_cell_def(tp: &Type, data: &crate::data::Data) -> Option<u32> {
    let Type::Reference(d_nr, _) = tp else {
        return None;
    };
    if data.def(*d_nr).name().starts_with("__cell_") {
        Some(*d_nr)
    } else {
        None
    }
}

/// Plan-22 phase 02d-ii — canonical type for the cell's `value`
/// field.
///
/// Strips bound/dep details so multiple captures of "the same
/// underlying type" with slightly different annotations
/// (e.g. `text` with different lifetime deps, or
/// `integer not null` vs `integer`) share a single cell struct.
fn cell_value_type(tp: &Type) -> Type {
    // Through `Type::optional`, so the wrap stays idempotent (`@FR-N-Idem`, its one home).
    if let (inner, true) = tp.peel_optional() {
        return Type::optional(cell_value_type(inner));
    }
    match tp {
        Type::Integer(spec) => {
            // A canonical 8-byte template drops its bounds + null-flag, so `integer` and
            // `integer not null` share one cell.  A NARROW width keeps its spec whole:
            // the width, the bounds and the reserved null sentinel are what the declared
            // type promises, and a box that widened any of them would answer a value the
            // author's type excludes (`@FR-L-CapBox`).
            if spec.forced_size.is_none() && spec.byte_width(true) == 8 {
                if spec.max == u32::MAX {
                    crate::data::I64.clone()
                } else {
                    crate::data::I32.clone()
                }
            } else {
                tp.clone()
            }
        }
        Type::Text(_) => Type::Text(Deps::none()),
        Type::Enum(d_nr, _, _) => Type::Enum(*d_nr, false, Deps::none()),
        other => other.clone(),
    }
}

/// Plan-22 phase 02d-i — accumulate the names of scalar-typed
/// captures that this lambda mutates onto the PARENT function's
/// `scalars_to_box` field.
///
/// Per-call: one lambda's mutations contribute to the parent
/// function's union.  Names are deduped; types decide
/// scalar-vs-non-scalar.
///
/// "Scalar" set (per phase 02d design):
///   - `Type::Integer(_)`
///   - `Type::Float`
///   - `Type::Single`
///   - `Type::Boolean`
///   - `Type::Character`
///   - `Type::Text(_)`
///   - `Type::Enum(_, false, _)` (plain enum)
///
/// Reference / Function / Vector / Hash / Sorted / Index /
/// Radix / Tuple captures are NOT scalars — they're handled by
/// other paths (Reference: phase 02c; Function: phase 02c via
/// existing fn-ref machinery; Vector + keyed: rejected by P257).
///
/// `parent_d_nr` is the enclosing function's def_nr; if the
/// lambda is at top-level (no enclosing function — e.g. a lambda
/// in a struct field default value), `parent_d_nr == u32::MAX`
/// and the accumulator is a no-op (top-level binds aren't
/// mutated-captured by their own scope).
/// Replaces `captured_names` entries
/// for names in the parent function's `scalars_to_box` with
/// their boxed `Reference(__cell_<T>, [])` form.
///
/// Called from the lambda parsing site BETWEEN
/// `synthesize_cell_structs` (which needs the original scalar
/// type to compute the cell name) and `synthesize_closure_record`
/// (which uses `captured_names` types directly to set the
/// closure record's attribute types).  In pass 1, this is what
/// makes the closure record's attribute carry
/// `Reference(__cell_int, _)` from creation, so phase 02c's
/// auto-Reference encoding fires (12B share-by-DbRef storage)
/// and the closure body's reads/writes route through the
/// shared cell.
///
/// In pass 2, `captured_names` is typically empty (the lambda's
/// resolve_name takes the closure-redirect arm before the
/// capture arm fires), so this helper is a no-op there.
fn box_captured_names_for_outer_scalars(
    captured_names: &mut [(String, Type)],
    data: &crate::data::Data,
    outer_context: u32,
    owner: &std::collections::HashMap<String, u32>,
) {
    if outer_context == u32::MAX || (outer_context as usize) >= data.definitions.len() {
        return;
    }
    // Per NAME, because a lambda nested in a lambda captures from further out than its
    // enclosing scope: the cell is minted in the frame that HOLDS the variable, so that is
    // also the definition whose `scalars_to_box` answers for it (loft#1236).
    let scalars_of = |name: &str| -> Vec<String> {
        let d = owner.get(name).copied().unwrap_or(outer_context);
        if d == u32::MAX || (d as usize) >= data.definitions.len() {
            return Vec::new();
        }
        data.def(d).scalars_to_box().to_vec()
    };
    // #687 — this is PROVISIONAL.  Whether the binding really takes a cell depends on
    // whether it ends up with its own indirection (a hidden `&T` out-parameter), and at
    // the lambda's epilogue that is not settled yet: a text local the function RETURNS is
    // still a plain `Text` here and only becomes `RefVar(Text)` later in the body.  The
    // parent's pass-1 body end knows, and `Parser::finalize_capture_storage` corrects the
    // attribute there — still before `fill_all` lays the record out.  Boxing is the right
    // default because it is the common case, and because the attribute has to say
    // something now: pass 1 freezes the record's storage, so leaving the un-flipped
    // scalar here would lay the field out as 8B inline instead of a 12B shared DbRef.
    for (name, tp) in captured_names {
        if !scalars_of(name).iter().any(|s| s == name) {
            continue;
        }
        if let Some(cell_name) = cell_struct_name(tp, data) {
            let cell_d_nr = data.def_nr(&cell_name);
            if cell_d_nr != u32::MAX {
                *tp = Type::Reference(cell_d_nr, Deps::none());
            }
        }
    }
}

fn accumulate_scalars_to_box(
    data: &mut crate::data::Data,
    parent_d_nr: u32,
    lambda_d_nr: u32,
    captured_names: &[(String, Type)],
    owner: &std::collections::HashMap<String, u32>,
) {
    if parent_d_nr == u32::MAX || (parent_d_nr as usize) >= data.definitions.len() {
        return;
    }
    let mutated = data.def(lambda_d_nr).mutated_captures().to_vec();
    for name in &mutated {
        // The cell belongs to the frame that HOLDS the variable.  For a lambda nested in a
        // lambda that is further out than the enclosing scope, and boxing it in the enclosing
        // closure instead leaves the owner's local an unboxed scalar while every capture of it
        // is a cell handle — the two halves of one binding disagreeing (loft#1236).
        let parent_d_nr = owner.get(name).copied().unwrap_or(parent_d_nr);
        if parent_d_nr == u32::MAX || (parent_d_nr as usize) >= data.definitions.len() {
            continue;
        }
        let Some((_, tp)) = captured_names.iter().find(|(n, _)| n == name) else {
            continue;
        };
        // `@FR-L-CapWrite` — the set of captures whose write reaches the outer variable.
        // `.base()`, because nullability does not change whether a capture is a SCALAR one.
        // This list drives the three refusals that read `scalars_to_box` — shared between two
        // closures, written through a `const` parameter, written through a `&` one — so a
        // capture that falls out of it is not merely un-boxed, it is unguarded (loft#1408).
        let is_scalar = matches!(
            tp.base(),
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
                | Type::Enum(_, false, _)
        );
        if !is_scalar {
            continue;
        }
        let parent = &mut data.definitions[parent_d_nr as usize];
        if !parent.scalars_to_box.iter().any(|s| s == name) {
            parent.scalars_to_box.push(name.clone());
        }
    }
}

/// Plan-22 phase 01 — walk the lambda body's IR and identify
/// captured bindings whose value is mutated.
///
/// Detection rules:
///   - `Value::Set(slot, _)` where slot's variable name appears
///     in the closure record → whole-binding reassignment.
///   - `Value::Call(d, args)` where `d`'s op name is in the
///     mutating-op set (OpSet* / OpAppend* / OpClear* /
///     OpInsertVector / OpRemoveVector) AND `args[0]` is a
///     `Value::Var(slot)` whose name appears in the closure
///     record → field write through the captured value.
///
/// The closure record's attribute names are the canonical list
/// of captured bindings (populated by `synthesize_closure_record`
/// from `captured_names`).  A name in `mutated_captures` means
/// some write opcode targets that name's underlying slot.
///
/// Stores result on `data.def(lambda_d_nr).mutated_captures`.
/// Phases 02-05 consume this for case classification.
///
/// Detection-only: no IR rewrite, no codegen change, no behavior
/// difference between this commit and the prior state.  The
/// stored result is consulted by later phases.
fn collect_mutated_captures(data: &mut crate::data::Data, lambda_d_nr: u32) {
    // Plan-22 phase 02c (2026-05-12): the captured-name list comes
    // from EITHER the closure record's attributes (when synthesize
    // ran first — the original phase 01 path) OR from the lambda's
    // variables that match a synthesized `__closure_<N>` struct
    // name pattern (not currently used).  The phase-02c flow runs
    // collect_mutated_captures BEFORE synthesize, so we accept the
    // closure_record == MAX case as "not yet built; no names from
    // there" and fall through to using the variables table.
    //
    // Phase 01's existing API stays compatible: when synthesize has
    // already run, this function uses the closure record's
    // attributes (the original behaviour).
    let closure_d_nr = data.def(lambda_d_nr).closure_record();
    let variables = data.def(lambda_d_nr).variables().clone();
    let captured_names: Vec<String> = if closure_d_nr == u32::MAX {
        // Pre-synthesize call site — derive captured names from the
        // lambda's variable table.  Captured names are local
        // placeholder vars created in objects.rs:187 / control.rs:3614
        // during body parsing.  Filter out `__closure` (the closure
        // param), `__work*` (text work buffers), and any argument.
        // The body walker then double-filters via the captured_names
        // membership check in `mark()`, so over-inclusion here is
        // safe (just spurious work).
        (0..variables.count())
            .filter_map(|v| {
                let name = variables.name(v);
                if name == "__closure" || name.starts_with("__work") || variables.is_argument(v) {
                    None
                } else {
                    Some(name.to_string())
                }
            })
            .collect()
    } else {
        data.def(closure_d_nr)
            .attributes
            .iter()
            .map(|a| a.name.clone())
            .collect()
    };
    if captured_names.is_empty() {
        return;
    }
    let body = data.def(lambda_d_nr).code().clone();
    // Resolve the closure-param's var slot — it's the `__closure`
    // argument added at lambda parse time (vectors.rs:417-421).
    // In the body's IR, captured-name reads are
    // `Call(OpGet*, [Var(closure_param), Int(fld_idx)])` and
    // writes are `Call(OpSet*, [Var(closure_param), Int(fld_idx),
    // value])`.  Field indices map to attribute positions in the
    // closure record (`captured_names[fld_idx]`).
    let mut closure_param: Option<u16> = None;
    for v in 0..variables.count() {
        if variables.name(v) == "__closure" {
            closure_param = Some(v);
            break;
        }
    }
    let mut mutated: Vec<String> = Vec::new();
    walk_for_mutations(
        &body,
        &captured_names,
        closure_param,
        &variables,
        data,
        &mut mutated,
    );
    data.definitions[lambda_d_nr as usize].mutated_captures = mutated;
}

/// The variable an accessor chain is ROOTED at — `v` for `Var(v)`, `GetVector(Var(v), i)`,
/// `GetField(GetVector(Var(v), i), f)` and so on; `None` when the chain does not bottom out
/// in a variable.
///
/// A write names its target through however many accessors the shape needs, and the DEPTH is
/// not part of the question *"which binding does this write reach?"*.  Asked one level down,
/// the walk saw `s.x = 7` on a captured struct and missed `p[0] = 42` on a captured
/// collection, whose target is `SetInt(GetVector(Var(p), i), …)` — so a `&` parameter written
/// only that way read as never modified (loft#1276).
fn accessor_root_var(v: &Value, data: &crate::data::Data) -> Option<u16> {
    match v.unspan() {
        Value::Var(var) => Some(*var),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).original_name().starts_with("Get") =>
        {
            accessor_root_var(args.first()?, data)
        }
        _ => None,
    }
}

/// The closure-record attribute index an accessor chain reads, when the chain is rooted at
/// the CLOSURE PARAMETER — `None` for anything else.
///
/// This is the pass-2 spelling of a capture, and the recursion generalises a check that used
/// to look exactly one level down.  ⚠ Measured 2026-09-01: at the current call ordering
/// `collect_mutated_captures` always runs with `closure_param == None` (the pre-synthesize
/// path), so nothing in the shipped suite reaches this — it was already unreached in its
/// one-level form.  It is kept because the shape is real whenever the threading order puts a
/// closure parameter here, and losing it would silently drop mutation detection rather than
/// fail; [`accessor_root_var`] is the sibling that actually fires today.
fn capture_field_of(
    v: &Value,
    closure_param: Option<u16>,
    data: &crate::data::Data,
) -> Option<i32> {
    let Value::Call(d, args) = v.unspan() else {
        return None;
    };
    if (*d as usize) >= data.definitions.len() || !data.def(*d).original_name().starts_with("Get") {
        return None;
    }
    let first = args.first()?;
    if let Value::Var(var) = first.unspan()
        && Some(*var) == closure_param
        && let Some(Value::Int(fld)) = args.get(1).map(Value::unspan)
    {
        return Some(*fld);
    }
    capture_field_of(first, closure_param, data)
}

fn walk_for_mutations(
    code: &Value,
    captured_names: &[String],
    closure_param: Option<u16>,
    variables: &crate::variables::Function,
    data: &crate::data::Data,
    out: &mut Vec<String>,
) {
    let mark = |name: &str, out: &mut Vec<String>| {
        if captured_names.iter().any(|c| c == name) && !out.iter().any(|s| s == name) {
            out.push(name.to_string());
        }
    };
    match code {
        Value::Span(boxed) => walk_for_mutations(
            &boxed.1,
            captured_names,
            closure_param,
            variables,
            data,
            out,
        ),
        Value::Set(slot, expr) => {
            if *slot < variables.count() {
                mark(variables.name(*slot), out);
            }
            walk_for_mutations(expr, captured_names, closure_param, variables, data, out);
        }
        Value::Call(d, args) => {
            if (*d as usize) < data.definitions.len() {
                // One home with `find_written_vars` — see `op_writes_first_arg`.  Asked from
                // its own list, this walker did not know `OpNewRecord`, which is how a
                // captured collection's append lowers, so the append was not recorded as a
                // mutation of the capture (loft#1276).
                if crate::parser::op_writes_first_arg(data.def(*d).name())
                    && let Some(first) = args.first()
                {
                    // Three write shapes:
                    //   (a) `OpSet*(Var(captured_local), …)` — pass-1
                    //       form, before closure-param threading.
                    //       Direct local-slot write to the captured
                    //       binding's placeholder var.
                    //   (b) `OpSet*(Var(closure_param), Int(fld_idx),
                    //       …)` — pass-2 direct write to a captured
                    //       primitive's slot in the closure record.
                    //       Map fld_idx → captured_names[fld_idx].
                    //   (c) `OpSet*(Call(GetField, [Var(closure_param),
                    //       Int(fld_idx)]), …)` — pass-2 nested write
                    //       through a captured Reference's loaded
                    //       value.  E.g. `s.x = 7` for captured
                    //       struct `s`.  The mutation targets `s`
                    //       (via its content), so flag s as mutated.
                    match first.unspan() {
                        Value::Var(v) => {
                            if Some(*v) == closure_param
                                && let Some(Value::Int(fld)) = args.get(1).map(Value::unspan)
                                && (*fld as usize) < captured_names.len()
                            {
                                mark(&captured_names[*fld as usize].clone(), out);
                            } else if *v < variables.count() {
                                mark(variables.name(*v), out);
                            }
                        }
                        Value::Call(_, _) => {
                            // Pass-1 form: the chain is rooted at the captured binding's own
                            // placeholder VAR rather than at the closure parameter (which does
                            // not exist yet).  `p[0] = 42` is `SetInt(GetVector(Var(p), 0), …)`,
                            // so the root var is the capture — one level deeper than the
                            // `Var(p)` shape an append presents, which is why an element write
                            // went unrecorded while an append did not (loft#1276).
                            if let Some(root) = accessor_root_var(first, data)
                                && root < variables.count()
                            {
                                mark(variables.name(root), out);
                            }
                            // A nested accessor chain rooted at the closure record, of ANY
                            // depth — `Call(Get*, [Var(closure_param), Int(fld)])` and every
                            // longer chain over it.  One level was enough for `s.x = 7` on a
                            // captured struct and NOT for `p[0] = 42` on a captured
                            // collection, whose target is `SetInt(GetVector(GetDbRef(closure,
                            // fld), i), …)` — two levels, so the write went unrecorded and a
                            // `&` parameter written only that way read as never modified
                            // (loft#1276).
                            if let Some(fld) = capture_field_of(first, closure_param, data)
                                && (fld as usize) < captured_names.len()
                            {
                                mark(&captured_names[fld as usize].clone(), out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            for arg in args {
                walk_for_mutations(arg, captured_names, closure_param, variables, data, out);
            }
        }
        Value::CallRef(_, args) | Value::Insert(args) | Value::Tuple(args) => {
            for arg in args {
                walk_for_mutations(arg, captured_names, closure_param, variables, data, out);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for child in &b.operators {
                walk_for_mutations(child, captured_names, closure_param, variables, data, out);
            }
        }
        Value::If(c, t, f) => {
            walk_for_mutations(c, captured_names, closure_param, variables, data, out);
            walk_for_mutations(t, captured_names, closure_param, variables, data, out);
            walk_for_mutations(f, captured_names, closure_param, variables, data, out);
        }
        Value::Iter(_, init, step, body) => {
            walk_for_mutations(init, captured_names, closure_param, variables, data, out);
            walk_for_mutations(step, captured_names, closure_param, variables, data, out);
            walk_for_mutations(body, captured_names, closure_param, variables, data, out);
        }
        Value::Return(expr)
        | Value::Drop(expr)
        | Value::Yield(expr)
        | Value::TuplePut(_, _, expr) => {
            walk_for_mutations(expr, captured_names, closure_param, variables, data, out);
        }
        _ => {}
    }
}

#[cfg(test)]
mod plan22_phase01_mutation_detection_tests {
    //! Plan-22 phase 01 — verify `collect_mutated_captures` populates
    //! `data.def(d_nr).mutated_captures` correctly across the
    //! representative shapes phase 02 will consume.

    use crate::parser::Parser;

    /// Helper: parse a snippet and find the first capturing lambda
    /// definition.  Returns its `mutated_captures` clone.
    fn first_capturing_lambda_mutations(source: &str) -> Vec<String> {
        let mut p = Parser::new();
        // Load defaults so primitive types and operators resolve.
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase01_test", false);
        for d_nr in 0..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.closure_record() != u32::MAX && !def.mutated_captures().is_empty() {
                return def.mutated_captures().to_vec();
            }
        }
        // Fall back to the first capturing lambda even when nothing
        // was detected — lets `assert_eq` show the actual empty vec.
        for d_nr in 0..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.closure_record() != u32::MAX {
                return def.mutated_captures().to_vec();
            }
        }
        Vec::new()
    }

    #[test]
    fn read_only_capture_yields_empty_set() {
        // Case A baseline — capture `n`, read it, never write.
        // mutated_captures should stay empty (the regression-net
        // signal that phases 02-05 must not flip).
        let mutated = first_capturing_lambda_mutations(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
        );
        assert!(
            mutated.is_empty(),
            "expected no mutations for read-only capture; got {mutated:?}"
        );
    }

    #[test]
    fn whole_binding_reassign_detected() {
        // Case B (basic) — `n = n + 1` rewrites the captured slot.
        // The Set arm of the walker catches this.
        let mutated = first_capturing_lambda_mutations(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
        );
        assert!(
            mutated.iter().any(|s| s == "n"),
            "expected `n` mutation detected; got {mutated:?}"
        );
    }

    #[test]
    fn struct_field_write_detected() {
        // Case B (Reference) — `s.x = ...` desugars to OpSetInt
        // on the captured Reference.  The MUTATING_OP_NAMES arm
        // catches it via `args[0] = Var(captured)`.
        let mutated = first_capturing_lambda_mutations(
            r"
            struct S { x: integer, y: integer }
            fn test() {
                s = S { x: 0, y: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
        );
        assert!(
            mutated.iter().any(|s| s == "s"),
            "expected `s` mutation detected; got {mutated:?}"
        );
    }

    #[test]
    fn text_append_detected() {
        // Case B (text capture) — `s += "x"` desugars to
        // OpAppendText on the captured text slot.
        let mutated = first_capturing_lambda_mutations(
            r#"
            fn test() {
                s = "";
                f = fn() { s += "x"; };
                f();
            }
            "#,
        );
        assert!(
            mutated.iter().any(|s| s == "s"),
            "expected `s` mutation detected; got {mutated:?}"
        );
    }

    #[test]
    fn multiple_captures_only_mutated_one_flagged() {
        // Read `r`, write `w`.  Only `w` should appear in mutated.
        let mutated = first_capturing_lambda_mutations(
            r"
            fn test() {
                r = 100;
                w = 0;
                f = fn() { w = w + r; };
                f();
            }
            ",
        );
        assert!(
            mutated.iter().any(|s| s == "w"),
            "expected `w` mutation detected; got {mutated:?}"
        );
        assert!(
            !mutated.iter().any(|s| s == "r"),
            "did not expect `r` (read-only) in mutated set; got {mutated:?}"
        );
    }
}

#[cfg(test)]
mod plan22_phase02d_i_scalars_to_box_tests {
    //! Plan-22 phase 02d-i — verify the parent-function
    //! `scalars_to_box` field is populated correctly across the
    //! representative shapes phase 02d-iii will consume.  No
    //! behavior change at this phase: the field is detection-only.

    use crate::parser::Parser;

    /// Helper: parse a snippet with a `fn test() { … }` and return
    /// `test`'s `scalars_to_box` field (sorted for stable assertion).
    fn test_fn_scalars_to_box(source: &str) -> Vec<String> {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_i_test", false);
        let test_d_nr = p.data.def_nr("n_test");
        if test_d_nr == u32::MAX {
            return Vec::new();
        }
        let mut names = p.data.def(test_d_nr).scalars_to_box().to_vec();
        names.sort();
        names
    }

    #[test]
    fn read_only_capture_yields_empty_box_set() {
        // Case A — capture `n`, read it, never write.
        // No name should be queued for boxing.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
        );
        assert!(
            names.is_empty(),
            "expected empty scalars_to_box for read-only capture; got {names:?}"
        );
    }

    #[test]
    fn integer_mutated_capture_pushed() {
        // The canonical 02d-iii target: `n = 0; f = fn() { n = n + 1; }`.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["n".to_string()],
            "expected `n` queued for boxing; got {names:?}"
        );
    }

    #[test]
    fn text_mutated_capture_pushed() {
        // `s` is text — boxable per the 02d design (Text is in
        // the scalar set even though it has internal heap storage).
        let names = test_fn_scalars_to_box(
            r#"
            fn test() {
                s = "";
                f = fn() { s += "x"; };
                f();
            }
            "#,
        );
        assert_eq!(
            names,
            vec!["s".to_string()],
            "expected `s` queued for boxing; got {names:?}"
        );
    }

    #[test]
    fn struct_capture_not_pushed() {
        // `s` is a struct (Reference type) — handled by phase 02c
        // (auto-Reference), NOT by 02d-iii's boxing.  The
        // scalars_to_box field must EXCLUDE Reference captures.
        let names = test_fn_scalars_to_box(
            r"
            struct S { x: integer }
            fn test() {
                s = S { x: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
        );
        assert!(
            names.is_empty(),
            "Reference captures must NOT be queued for boxing; got {names:?}"
        );
    }

    #[test]
    fn multiple_captures_only_mutated_scalars_pushed() {
        // Read `r`, write `w`.  Only `w` (the mutated scalar) is
        // queued.  `r` (read-only) is excluded by phase 01's
        // mutated_captures filter.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                r = 100;
                w = 0;
                f = fn() { w = w + r; };
                f();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["w".to_string()],
            "expected `w` queued (not `r`); got {names:?}"
        );
    }

    #[test]
    fn multi_scalar_capture_all_pushed() {
        // Two scalars, both mutated.  Both should be queued.
        // (Sorted by the helper for stable assertion.)
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                a = 0;
                b = 0;
                f = fn() {
                    a = a + 1;
                    b = b + 2;
                };
                f();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["a".to_string(), "b".to_string()],
            "expected both `a` and `b` queued; got {names:?}"
        );
    }

    #[test]
    fn dedup_when_two_lambdas_mutate_same_capture() {
        // Two separate lambdas in `test` both mutate `n`.  The
        // accumulator must dedup — `n` appears once, not twice.
        let names = test_fn_scalars_to_box(
            r"
            fn test() {
                n = 0;
                f1 = fn() { n = n + 1; };
                f2 = fn() { n = n + 2; };
                f1();
                f2();
            }
            ",
        );
        assert_eq!(
            names,
            vec!["n".to_string()],
            "expected `n` queued exactly once (deduped); got {names:?}"
        );
    }
}

#[cfg(test)]
mod plan22_phase02d_ii_cell_struct_synthesis_tests {
    //! Plan-22 phase 02d-ii — verify `synthesize_cell_structs`
    //! creates the canonical `__cell_<T>` structs in `Data` after
    //! parsing snippets that mutate scalar captures.  The structs
    //! are foundation infrastructure for 02d-iii's outer-binding
    //! rewrite; at this phase they exist but are never allocated
    //! or referenced (no behavior change).

    use crate::data::DefType;
    use crate::parser::Parser;

    /// Helper: parse a snippet and return the `(name, def_type)`
    /// of every `__cell_*` definition that exists in `Data`.
    fn cells_after(source: &str) -> Vec<(String, DefType)> {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_ii_test", false);
        let mut cells = Vec::new();
        for d_nr in 0..p.data.definitions() {
            let def = p.data.def(d_nr);
            if def.name().starts_with("__cell_") {
                cells.push((def.name().to_string(), def.def_type().clone()));
            }
        }
        cells.sort_by(|a, b| a.0.cmp(&b.0));
        cells
    }

    /// Helper: parse a snippet and return `value` field's type
    /// signature for the named cell struct.  Panics if the cell or
    /// its `value` attribute is missing — those failures should
    /// surface as test failures, not silent `None`s.
    fn cell_value_signature(source: &str, cell_name: &str) -> String {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_ii_test", false);
        let d_nr = p.data.def_nr(cell_name);
        assert_ne!(d_nr, u32::MAX, "cell `{cell_name}` not found");
        let def = p.data.def(d_nr);
        let attr = def
            .attributes
            .iter()
            .find(|a| a.name == "value")
            .unwrap_or_else(|| panic!("`value` attribute missing on `{cell_name}`"));
        format!("{:?}", attr.typedef)
    }

    #[test]
    fn integer_capture_creates_cell_integer() {
        let cells = cells_after(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
        );
        assert!(
            cells
                .iter()
                .any(|(n, t)| n == "__cell_integer" && *t == DefType::Struct),
            "expected `__cell_integer` Struct in Data; got {cells:?}"
        );
    }

    #[test]
    fn text_capture_creates_cell_text() {
        let cells = cells_after(
            r#"
            fn test() {
                s = "";
                f = fn() { s += "x"; };
                f();
            }
            "#,
        );
        assert!(
            cells
                .iter()
                .any(|(n, t)| n == "__cell_text" && *t == DefType::Struct),
            "expected `__cell_text` Struct in Data; got {cells:?}"
        );
    }

    #[test]
    fn read_only_capture_creates_no_cell() {
        let cells = cells_after(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
        );
        assert!(
            cells.is_empty(),
            "no cell should be synthesised for read-only capture; got {cells:?}"
        );
    }

    #[test]
    fn struct_capture_creates_no_cell() {
        // Struct (Reference) captures are handled by phase 02c's
        // auto-Reference path, NOT by 02d's boxing.  No `__cell_*`
        // should appear.
        let cells = cells_after(
            r"
            struct S { x: integer }
            fn test() {
                s = S { x: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
        );
        assert!(
            cells.is_empty(),
            "no cell should be synthesised for struct capture; got {cells:?}"
        );
    }

    #[test]
    fn dedup_two_integer_captures_share_one_cell() {
        // Two separate scalar bindings, both integer, both
        // mutated.  Exactly one `__cell_integer` should appear.
        let cells = cells_after(
            r"
            fn test() {
                a = 0;
                b = 0;
                f = fn() {
                    a = a + 1;
                    b = b + 2;
                };
                f();
            }
            ",
        );
        let count = cells.iter().filter(|(n, _)| n == "__cell_integer").count();
        assert_eq!(
            count, 1,
            "expected exactly one `__cell_integer`; got {count} (cells={cells:?})"
        );
    }

    #[test]
    fn distinct_types_create_distinct_cells() {
        // Mix of scalar types — each should get its own cell.
        let cells = cells_after(
            r#"
            fn test() {
                n = 0;
                s = "";
                b = false;
                f = fn() {
                    n = n + 1;
                    s += "x";
                    b = !b;
                };
                f();
            }
            "#,
        );
        let names: Vec<String> = cells.iter().map(|(n, _)| n.clone()).collect();
        assert!(
            names.contains(&"__cell_integer".to_string()),
            "expected __cell_integer; got {names:?}"
        );
        assert!(
            names.contains(&"__cell_text".to_string()),
            "expected __cell_text; got {names:?}"
        );
        assert!(
            names.contains(&"__cell_boolean".to_string()),
            "expected __cell_boolean; got {names:?}"
        );
    }

    #[test]
    fn cell_value_field_carries_canonical_type() {
        // Sanity check: the cell's `value` field exists and the
        // type signature renders as `Integer(...)` (the canonical
        // I32 template).  Phase 02d-iii will read/write through
        // this field.
        let sig = cell_value_signature(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
            "__cell_integer",
        );
        assert!(
            sig.starts_with("Integer("),
            "cell `value` field should be Integer-typed; got `{sig}`"
        );
    }

    #[test]
    fn dedup_across_two_lambdas() {
        // Two lambdas, each mutating its own integer capture.
        // Both reuse the single `__cell_integer` struct (the cell
        // is keyed by type, not by binding).
        let cells = cells_after(
            r"
            fn test() {
                a = 0;
                b = 0;
                f1 = fn() { a = a + 1; };
                f2 = fn() { b = b + 1; };
                f1();
                f2();
            }
            ",
        );
        let count = cells.iter().filter(|(n, _)| n == "__cell_integer").count();
        assert_eq!(
            count, 1,
            "expected single shared `__cell_integer` across two lambdas; got {count} (cells={cells:?})"
        );
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_a_type_flip_tests {
    //! Plan-22 phase 02d-iii.a — verify
    //! `flip_scalars_to_box_types` replaces each boxed scalar
    //! local's type in the variables table with its
    //! `Reference(__cell_<T>, [])` form when invoked.
    //!
    //! Foundation step: the helper is shipped but
    //! INTENTIONALLY NOT WIRED into the parse_function pass-2
    //! entry yet.  The existing void-return-closure write-back
    //! mechanism in `parse_call_ref` (control.rs lines
    //! 3729-3755) handles today's `p86_lambda_capture_*`
    //! cases by copying the closure-record's scalar attribute
    //! back to the outer slot after each call.  Activating the
    //! flip in this commit would break that path
    //! (the outer slot's shape changes from 8B Integer to 12B
    //! DbRef, segfaulting the write-back's `v_set`).
    //!
    //! 02d-iii.e activates the flip from `parse_function`
    //! AFTER 02d-iii.b-d have wired cell-based propagation
    //! (auto-Reference closure-record attribute + shared-DbRef
    //! reads/writes), at which point the write-back path can
    //! be removed without a regression.
    //!
    //! These tests invoke the helper EXPLICITLY on a parsed
    //! parser state — they verify the helper's logic in
    //! isolation, independent of the integration path.

    use crate::data::Type;
    use crate::parser::Parser;

    /// Helper: parse a snippet, restore pass-2 entry state for
    /// `n_test` (set context, restore vars), invoke the flip
    /// helper, and return the type of `var_name` after the
    /// flip.  Panics if the function or variable is missing.
    fn flipped_type_of_var(source: &str, var_name: &str) -> Type {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(source, "phase02d_iii_a_test", false);
        let test_d_nr = p.data.def_nr("n_test");
        assert_ne!(test_d_nr, u32::MAX, "function `n_test` not found");
        // Restore parse_function pass-2 entry state for the
        // test fn: set context + drain the def's saved vars
        // back into self.vars (the inverse of the save at the
        // end of parse_function).
        p.context = test_d_nr;
        // Reset p.vars to a fresh Function and drain the saved
        // pass-2 variables back into it (mirrors parse_function's
        // line-547 `Function::new` + line-853 `vars.append`).
        p.vars = crate::variables::Function::new("phase02d_iii_a_test", "test.loft");
        p.vars
            .append(&mut p.data.definitions[test_d_nr as usize].variables);
        // Invoke the flip helper under test.
        p.flip_scalars_to_box_types();
        let v_nr = p.vars.var(var_name);
        assert_ne!(v_nr, u16::MAX, "variable `{var_name}` not found");
        p.vars.tp(v_nr).clone()
    }

    #[test]
    fn integer_mutated_capture_flipped_to_reference_cell_integer() {
        // The canonical 02d-iii target.  After explicit flip,
        // `n`'s type becomes `Reference(__cell_integer, [])`.
        let tp = flipped_type_of_var(
            r"
            fn test() {
                n = 0;
                f = fn() { n = n + 1; };
                f();
            }
            ",
            "n",
        );
        match tp {
            Type::Reference(_, ref deps) => {
                assert!(
                    deps.is_empty(),
                    "boxed-scalar Reference should be heap-owned (empty deps); got {deps:?}"
                );
            }
            other => panic!("expected Reference(__cell_integer, []); got {other:?}"),
        }
    }

    #[test]
    fn read_only_capture_keeps_integer_type() {
        // Case A — capture `n` for reading only.  Type stays as
        // Integer; `flip_scalars_to_box_types` must NOT touch it.
        let tp = flipped_type_of_var(
            r"
            fn test() {
                n = 5;
                f = fn(x: integer) -> integer { x + n };
                _ = f(10);
            }
            ",
            "n",
        );
        assert!(
            matches!(tp, Type::Integer(_)),
            "read-only capture must NOT be flipped; got {tp:?}"
        );
    }

    #[test]
    fn struct_capture_keeps_struct_reference_type() {
        // Case B (Reference) — `s` is a struct; phase 02c handles
        // the auto-Reference encoding, NOT 02d-iii's boxing.
        let tp = flipped_type_of_var(
            r"
            struct S { x: integer }
            fn test() {
                s = S { x: 0 };
                f = fn() { s.x = 7; };
                f();
            }
            ",
            "s",
        );
        match tp {
            Type::Reference(_, _) => {
                // d_nr should NOT be a __cell_* struct; it's S.
                // (The flip helper doesn't touch struct
                // captures because `cell_struct_name` returns
                // `None` for `Type::Reference`.)
            }
            other => panic!("expected Reference(S, _); got {other:?}"),
        }
    }

    #[test]
    fn no_mutation_no_flip() {
        // Local `n` exists but is never captured by any closure —
        // scalars_to_box is empty for `n_test`, so the helper
        // is a no-op and `n` stays as Integer.
        let tp = flipped_type_of_var(
            r"
            fn test() {
                n = 5;
                _ = n + 1;
            }
            ",
            "n",
        );
        assert!(
            matches!(tp, Type::Integer(_)),
            "uncaptured local must NOT be flipped; got {tp:?}"
        );
    }

    #[test]
    fn multi_scalar_capture_flips_each() {
        // Two distinct scalar captures, both mutated.  Both
        // should be flipped to their respective cell References
        // when the helper is invoked.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(
            r#"
            fn test() {
                n = 0;
                s = "";
                f = fn() {
                    n = n + 1;
                    s += "x";
                };
                f();
            }
            "#,
            "phase02d_iii_a_test",
            false,
        );
        let test_d_nr = p.data.def_nr("n_test");
        let cell_int = p.data.def_nr("__cell_integer");
        let cell_text = p.data.def_nr("__cell_text");
        assert_ne!(cell_int, u32::MAX, "__cell_integer missing");
        assert_ne!(cell_text, u32::MAX, "__cell_text missing");
        // Restore pass-2 entry state and invoke the flip helper.
        p.context = test_d_nr;
        // Reset p.vars to a fresh Function and drain the saved
        // pass-2 variables back into it (mirrors parse_function's
        // line-547 `Function::new` + line-853 `vars.append`).
        p.vars = crate::variables::Function::new("phase02d_iii_a_test", "test.loft");
        p.vars
            .append(&mut p.data.definitions[test_d_nr as usize].variables);
        p.flip_scalars_to_box_types();
        let n_nr = p.vars.var("n");
        let s_nr = p.vars.var("s");
        assert_ne!(n_nr, u16::MAX, "`n` not found in vars");
        assert_ne!(s_nr, u16::MAX, "`s` not found in vars");
        match p.vars.tp(n_nr) {
            Type::Reference(d, deps) => {
                assert_eq!(*d, cell_int, "`n` should point at __cell_integer");
                assert!(deps.is_empty());
            }
            other => panic!("`n` not flipped: {other:?}"),
        }
        // Plan-22 phase 02d-vi — text is now flipped via the
        // bypass guard added in `parse_assign_op` for boxed-text
        // LHS shape.  `s` should be Reference(__cell_text, []).
        match p.vars.tp(s_nr) {
            Type::Reference(d, deps) => {
                assert_eq!(*d, cell_text, "`s` should point at __cell_text");
                assert!(deps.is_empty());
            }
            other => panic!("`s` not flipped to __cell_text: {other:?}"),
        }
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_b_read_auto_deref_tests {
    //! Plan-22 phase 02d-iii.b — verify
    //! `auto_deref_boxed_scalar` wraps reads of
    //! `Reference(__cell_<T>, _)` variables in
    //! `Call(OpGet<T>, [code, Int(0)])` and returns the
    //! cell's value-field type.
    //!
    //! Foundation step: the helper IS hooked into `parse_var`'s
    //! natural-return path, but it's a no-op in production
    //! because no variable carries the trigger type yet (phase
    //! 02d-iii.a's flip is dormant — see its test module).
    //! Phase 02d-iii.e activates the flip + this hook fires for
    //! real on every captured-scalar read in the parent body
    //! and the closure body.
    //!
    //! These tests invoke the helper directly with constructed
    //! inputs — they verify the wrapping logic in isolation,
    //! independent of the integration path.

    use crate::data::{Deps, Type, Value};
    use crate::parser::Parser;

    /// Helper: build a parser with defaults loaded so the OpGet*
    /// definitions exist in `Data`, then synthesise the named
    /// cell struct via the same machinery phase 02d-ii uses.
    fn parser_with_cell(value_tp: &Type, cell_name: &str) -> (Parser, u32) {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        // Build the cell struct directly (mirrors
        // `synthesize_cell_structs`).
        let cell_d_nr = p
            .data
            .add_def(cell_name, p.lexer.pos(), crate::data::DefType::Struct);
        p.data
            .add_attribute(&mut p.lexer, cell_d_nr, "value", value_tp.clone());
        (p, cell_d_nr)
    }

    #[test]
    fn integer_cell_wraps_with_op_get_int() {
        let (mut p, cell_d_nr) = parser_with_cell(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
        );
        let mut code = Value::Var(7);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        // Expect: Call(OpGetInt, [Var(7), Int(0)])
        let op_d_nr = p.data.def_nr("OpGetInt");
        match &code {
            Value::Call(d, args) => {
                assert_eq!(*d, op_d_nr, "expected OpGetInt; got d_nr={d}");
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0], Value::Var(7)));
                assert!(matches!(args[1], Value::Int(0)));
            }
            other => panic!("expected Call(OpGetInt, …); got {other:?}"),
        }
        assert!(
            matches!(new_t, Type::Integer(_)),
            "expected Integer return type; got {new_t:?}"
        );
    }

    #[test]
    fn text_cell_wraps_with_op_get_text() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Text(Deps::none()), "__cell_text");
        let mut code = Value::Var(3);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        let op_d_nr = p.data.def_nr("OpGetText");
        match &code {
            Value::Call(d, args) => {
                assert_eq!(*d, op_d_nr);
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Call(OpGetText, …); got {other:?}"),
        }
        assert!(matches!(new_t, Type::Text(_)));
    }

    #[test]
    fn boolean_cell_wraps_byte_then_eq() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Boolean, "__cell_boolean");
        let mut code = Value::Var(5);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        // @PLN17: boolean now reads its byte directly (0/1/255), like a plain enum —
        // Call(OpGetBoolean, [Var(5), Int(0)]) — not the old OpEqInt(OpGetByte, 1).
        let get_bool_d_nr = p.data.def_nr("OpGetBoolean");
        match &code {
            Value::Call(d, args) => {
                assert_eq!(
                    *d, get_bool_d_nr,
                    "boolean cell read should be OpGetBoolean"
                );
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Call(OpGetBoolean, …); got {other:?}"),
        }
        assert_eq!(new_t, Type::Boolean);
    }

    #[test]
    fn float_cell_wraps_with_op_get_float() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Float, "__cell_float");
        let mut code = Value::Var(2);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        let op_d_nr = p.data.def_nr("OpGetFloat");
        if let Value::Call(d, _) = &code {
            assert_eq!(*d, op_d_nr);
        } else {
            panic!("expected Call(OpGetFloat, …); got {code:?}");
        }
        assert_eq!(new_t, Type::Float);
    }

    #[test]
    fn character_cell_wraps_with_op_get_character() {
        let (mut p, cell_d_nr) = parser_with_cell(&Type::Character, "__cell_character");
        let mut code = Value::Var(4);
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(cell_d_nr, Deps::none()));
        let op_d_nr = p.data.def_nr("OpGetCharacter");
        if let Value::Call(d, _) = &code {
            assert_eq!(*d, op_d_nr);
        } else {
            panic!("expected Call(OpGetCharacter, …); got {code:?}");
        }
        assert_eq!(new_t, Type::Character);
    }

    #[test]
    fn non_cell_reference_returns_unchanged() {
        // A regular struct Reference (not a __cell_*) must NOT
        // be auto-dereffed.  This is the dormancy guarantee for
        // production code with no boxed scalars.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        // Synthesise a non-cell struct directly so the test
        // doesn't depend on which structs the defaults provide.
        let s_d_nr = p
            .data
            .add_def("MyStruct", p.lexer.pos(), crate::data::DefType::Struct);
        let mut code = Value::Var(1);
        let original_code = code.clone();
        let new_t = p.auto_deref_boxed_scalar(&mut code, Type::Reference(s_d_nr, Deps::none()));
        assert_eq!(
            code, original_code,
            "non-cell Reference must not be wrapped"
        );
        assert!(
            matches!(new_t, Type::Reference(_, _)),
            "non-cell Reference type must pass through unchanged; got {new_t:?}"
        );
    }

    #[test]
    fn non_reference_type_returns_unchanged() {
        // Passing a bare Integer through the helper must be a
        // no-op (no Reference, nothing to deref).
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let mut code = Value::Int(42);
        let original_code = code.clone();
        let new_t = p.auto_deref_boxed_scalar(
            &mut code,
            Type::Integer(crate::data::IntegerSpec::signed32()),
        );
        assert_eq!(code, original_code, "Integer input must not be wrapped");
        assert!(matches!(new_t, Type::Integer(_)));
    }

    #[test]
    fn parse_var_path_no_op_for_non_cell_locals() {
        // Smoke test through the parse_var integration path: a
        // normal local Integer goes through `parse_var` ->
        // `auto_deref_boxed_scalar` -> returns unchanged.
        // Verifies the production hook is dormant for non-boxed
        // variables.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        p.parse_str(
            r"
            fn test() {
                n = 42;
                _ = n + 1;
            }
            ",
            "phase02d_iii_b_test",
            false,
        );
        let test_d_nr = p.data.def_nr("n_test");
        let vars = p.data.def(test_d_nr).variables();
        let mut found_n = false;
        for v_nr in 0..vars.next_var() {
            if vars.name(v_nr) == "n" {
                assert!(
                    matches!(vars.tp(v_nr), Type::Integer(_)),
                    "non-boxed `n` must stay Integer; got {:?}",
                    vars.tp(v_nr)
                );
                found_n = true;
            }
        }
        assert!(found_n, "`n` not found in vars table");
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_c_assign_rewrite_tests {
    //! Plan-22 phase 02d-iii.c — verify
    //! `boxed_scalar_assign_rewrite` builds the correct IR for
    //! first vs subsequent assignments to a boxed-scalar local,
    //! plus `change_var_type` guard preserves the flipped type.
    //!
    //! Foundation step: helpers are shipped as infrastructure; a
    //! `parse_assign_op` hook activates them with 02d-iii.e
    //! (after 02d-iii.d wires the closure-body write rewrite).
    //! With the flip dormant in production, no variable carries
    //! the trigger type and the helpers return None / no-op for
    //! every assignment in real code.

    use crate::data::{Type, Value};
    use crate::parser::Parser;

    /// Helper: build a parser with defaults loaded, synthesise a
    /// `__cell_<T>` struct with the given value-field type, and
    /// add a fresh variable named `name` with type
    /// `Reference(cell_d_nr, [])`.
    fn parser_with_boxed_local(
        value_tp: &Type,
        cell_name: &str,
        var_name: &str,
    ) -> (Parser, u32, u16) {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let cell_d_nr = p
            .data
            .add_def(cell_name, p.lexer.pos(), crate::data::DefType::Struct);
        p.data
            .add_attribute(&mut p.lexer, cell_d_nr, "value", value_tp.clone());
        let v_nr = p.vars.add_variable(
            var_name,
            &Type::Reference(cell_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        (p, cell_d_nr, v_nr)
    }

    #[test]
    fn first_set_integer_emits_alloc_and_fill() {
        let (p, cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Int(42))
            .expect("expected rewrite IR");
        let op_db = p.data.def_nr("OpDatabase");
        let op_set = p.data.def_nr("OpSetInt");
        let cell_kt = i32::from(p.data.def(cell_d_nr).known_type());
        let Value::Insert(ops) = ir else {
            panic!("expected Insert");
        };
        assert_eq!(ops.len(), 3);
        match &ops[0] {
            Value::Set(set_v, val) => {
                assert_eq!(*set_v, v_nr);
                assert!(matches!(**val, Value::Null));
            }
            other => panic!("op[0] should be Set(n, Null); got {other:?}"),
        }
        match &ops[1] {
            Value::Call(d, args) => {
                assert_eq!(*d, op_db);
                assert_eq!(args.len(), 2);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(kt) if kt == cell_kt));
            }
            other => panic!("op[1] should be OpDatabase(n, kt); got {other:?}"),
        }
        match &ops[2] {
            Value::Call(d, args) => {
                assert_eq!(*d, op_set);
                assert_eq!(args.len(), 3);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(0)));
                assert!(matches!(args[2], Value::Int(42)));
            }
            other => panic!("op[2] should be OpSetInt(n, 0, 42); got {other:?}"),
        }
    }

    #[test]
    fn subsequent_set_integer_emits_field_write_only() {
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        p.vars.defined(v_nr);
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Int(7))
            .expect("expected rewrite IR");
        let op_set = p.data.def_nr("OpSetInt");
        match ir {
            Value::Call(d, args) => {
                assert_eq!(d, op_set);
                assert_eq!(args.len(), 3);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(0)));
                assert!(matches!(args[2], Value::Int(7)));
            }
            other => panic!("expected Call(OpSetInt, ...); got {other:?}"),
        }
    }

    #[test]
    fn text_first_set_uses_op_set_text() {
        let (p, _cell_d_nr, v_nr) =
            parser_with_boxed_local(&Type::Text(crate::data::Deps::none()), "__cell_text", "s");
        let op_set = p.data.def_nr("OpSetText");
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Text("hi".to_string()))
            .expect("expected rewrite IR");
        let Value::Insert(ops) = ir else {
            panic!("expected Insert");
        };
        if let Value::Call(d, _) = &ops[2] {
            assert_eq!(*d, op_set);
        } else {
            panic!("op[2] not a Call");
        }
    }

    #[test]
    fn float_first_set_uses_op_set_float() {
        let (p, _cell_d_nr, v_nr) = parser_with_boxed_local(&Type::Float, "__cell_float", "f");
        let op_set = p.data.def_nr("OpSetFloat");
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Float(2.5))
            .expect("expected rewrite IR");
        if let Value::Insert(ops) = &ir
            && let Value::Call(d, _) = &ops[2]
        {
            assert_eq!(*d, op_set);
        } else {
            panic!("expected Insert([_, _, Call(OpSetFloat, _)]); got {ir:?}");
        }
    }

    #[test]
    fn non_boxed_var_returns_none() {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let v_nr = p.vars.add_variable(
            "n",
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            &mut p.lexer,
        );
        let result = p.boxed_scalar_assign_rewrite(v_nr, "=", Value::Int(0));
        assert!(
            result.is_none(),
            "non-boxed Integer must not be rewritten; got {result:?}"
        );
    }

    #[test]
    fn non_cell_reference_returns_none() {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let s_d_nr = p
            .data
            .add_def("MyStruct", p.lexer.pos(), crate::data::DefType::Struct);
        let v_nr = p.vars.add_variable(
            "s",
            &Type::Reference(s_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        let result = p.boxed_scalar_assign_rewrite(v_nr, "=", Value::Null);
        assert!(
            result.is_none(),
            "non-cell Reference must not be rewritten; got {result:?}"
        );
    }

    #[test]
    fn compound_op_returns_none() {
        let (p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        for op in &["+=", "-=", "*=", "/=", "%="] {
            let result = p.boxed_scalar_assign_rewrite(v_nr, op, Value::Int(1));
            assert!(
                result.is_none(),
                "op `{op}` must not be rewritten by 02d-iii.c; got {result:?}"
            );
        }
    }

    #[test]
    fn boolean_first_set_uses_op_set_boolean() {
        // This case used to fall through, deferred on the premise that a boolean cell
        // needs the 4-arg `OpSetByte(ref, fld, min, val)`.  The premise was wrong: the
        // working boxed-boolean lowering emits the 3-arg `OpSetBoolean`, the same shape
        // as every other cell write.  #685 needed it for a boxed boolean PARAMETER's
        // entry seed, which has no assignment of its own to route through.
        let (p, _cell_d_nr, v_nr) = parser_with_boxed_local(&Type::Boolean, "__cell_boolean", "b");
        let op_set = p.data.def_nr("OpSetBoolean");
        let ir = p
            .boxed_scalar_assign_rewrite(v_nr, "=", Value::Boolean(true))
            .expect("expected rewrite IR");
        if let Value::Insert(ops) = &ir
            && let Value::Call(d, args) = &ops[2]
        {
            assert_eq!(*d, op_set);
            assert_eq!(args.len(), 3, "OpSetBoolean is a 3-arg write");
        } else {
            panic!("expected Insert([_, _, Call(OpSetBoolean, _)]); got {ir:?}");
        }
    }

    #[test]
    fn change_var_type_guard_preserves_flipped_type() {
        // The `change_var_type` guard added in 02d-iii.c: if the
        // variable's current type is `Reference(__cell_integer, _)`,
        // calling `change_var_type` with an Integer arg must NOT
        // revert it.  Without this guard, parse_assign_op's
        // `change_var(to, &s_type)` would undo the flip on every
        // `n = expr`.
        let (mut p, cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        // The guard only fires when !first_pass.
        p.first_pass = false;
        crate::diagnostics::set_first_pass(false);
        p.change_var_type(v_nr, &Type::Integer(crate::data::IntegerSpec::signed32()));
        match p.vars.tp(v_nr) {
            Type::Reference(d, _) => {
                assert_eq!(*d, cell_d_nr, "type was reverted; flip not preserved");
            }
            other => panic!("type was reverted to {other:?}"),
        }
    }
}

#[cfg(test)]
mod plan22_phase02d_iii_d_alloc_prepend_tests {
    //! Plan-22 phase 02d-iii.d — verify
    //! `maybe_prepend_cell_alloc` wraps a first-set
    //! `Call(OpSet<T>, [Var(n), 0, rhs])` with the cell
    //! allocation preamble, and is a no-op for subsequent sets,
    //! closure-body writes (where LHS inner is non-Var), and
    //! non-boxed locals.
    //!
    //! The closure-body write rewrite is delivered FOR FREE by
    //! 02d-iii.b's auto-deref + `call_to_set_op` in
    //! `parser/operators.rs:283` — the existing machinery
    //! already maps `OpGetInt` → `OpSetInt` when the LHS shape
    //! is `Call(OpGetInt, [<inner>, Int(pos)])`.  This helper
    //! delivers the OUTER-binding alloc that the auto-deref +
    //! `call_to_set_op` path doesn't provide.
    //!
    //! Foundation step: helper is shipped as infrastructure;
    //! a `parse_assign_op` hook activates it with 02d-iii.e.

    use crate::data::{Type, Value};
    use crate::parser::Parser;

    /// Helper: build a parser with defaults loaded, synthesise a
    /// `__cell_<T>` struct, and add a fresh boxed-scalar local.
    fn parser_with_boxed_local(
        value_tp: &Type,
        cell_name: &str,
        var_name: &str,
    ) -> (Parser, u32, u16) {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let cell_d_nr = p
            .data
            .add_def(cell_name, p.lexer.pos(), crate::data::DefType::Struct);
        p.data
            .add_attribute(&mut p.lexer, cell_d_nr, "value", value_tp.clone());
        let v_nr = p.vars.add_variable(
            var_name,
            &Type::Reference(cell_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        (p, cell_d_nr, v_nr)
    }

    #[test]
    fn first_set_wraps_with_alloc() {
        let (mut p, cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let op_db = p.data.def_nr("OpDatabase");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(
            op_set,
            vec![Value::Var(v_nr), Value::Int(0), Value::Int(42)],
        );
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        let cell_kt = i32::from(p.data.def(cell_d_nr).known_type());
        let Value::Insert(ops) = wrapped else {
            panic!("expected Insert wrap; got non-Insert");
        };
        assert_eq!(ops.len(), 3);
        match &ops[0] {
            Value::Set(set_v, val) => {
                assert_eq!(*set_v, v_nr);
                assert!(matches!(**val, Value::Null));
            }
            other => panic!("op[0] should be Set(n, Null); got {other:?}"),
        }
        match &ops[1] {
            Value::Call(d, args) => {
                assert_eq!(*d, op_db);
                assert!(matches!(args[0], Value::Var(x) if x == v_nr));
                assert!(matches!(args[1], Value::Int(kt) if kt == cell_kt));
            }
            other => panic!("op[1] should be OpDatabase(n, kt); got {other:?}"),
        }
        assert_eq!(ops[2], result, "op[2] should be the original OpSetInt call");
        assert!(
            p.vars.is_defined(v_nr),
            "variable should be marked defined after first-set alloc"
        );
    }

    #[test]
    fn subsequent_set_unchanged() {
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        p.vars.defined(v_nr);
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(op_set, vec![Value::Var(v_nr), Value::Int(0), Value::Int(7)]);
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(wrapped, result, "subsequent set should not be wrapped");
    }

    #[test]
    fn closure_body_lhs_is_no_op() {
        // When the LHS auto-deref's inner is a Call (e.g.
        // get_field(closure, n_field)) instead of Var(n), the
        // helper is a no-op — the cell was already allocated by
        // the parent.
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let fake_get_field = Value::Call(op_get, vec![Value::Var(99), Value::Int(8)]);
        let lhs = Value::Call(op_get, vec![fake_get_field.clone(), Value::Int(0)]);
        let result = Value::Call(op_set, vec![fake_get_field, Value::Int(0), Value::Int(11)]);
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(
            wrapped, result,
            "closure-body LHS (non-Var inner) should not be wrapped"
        );
    }

    #[test]
    fn non_boxed_lhs_is_no_op() {
        let mut p = Parser::new();
        let _ = p.parse_dir("default", true, false);
        let s_d_nr = p
            .data
            .add_def("MyStruct", p.lexer.pos(), crate::data::DefType::Struct);
        let v_nr = p.vars.add_variable(
            "s",
            &Type::Reference(s_d_nr, crate::data::Deps::none()),
            &mut p.lexer,
        );
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(
            op_set,
            vec![Value::Var(v_nr), Value::Int(0), Value::Int(99)],
        );
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(
            wrapped, result,
            "non-cell Reference LHS should not be wrapped"
        );
    }

    #[test]
    fn non_call_lhs_is_no_op() {
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let lhs = Value::Var(v_nr);
        let result = Value::Set(v_nr, Box::new(Value::Int(5)));
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(
            wrapped, result,
            "non-Call LHS (no auto-deref pattern) should not be wrapped"
        );
    }

    #[test]
    fn lhs_with_non_zero_offset_is_no_op() {
        // Non-zero offset means struct field read, not cell
        // value read.  Helper returns unchanged.
        let (mut p, _cell_d_nr, v_nr) = parser_with_boxed_local(
            &Type::Integer(crate::data::IntegerSpec::signed32()),
            "__cell_integer",
            "n",
        );
        let op_get = p.data.def_nr("OpGetInt");
        let op_set = p.data.def_nr("OpSetInt");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(8)]);
        let result = Value::Call(op_set, vec![Value::Var(v_nr), Value::Int(8), Value::Int(3)]);
        let wrapped = p.maybe_prepend_cell_alloc(result.clone(), &lhs);
        assert_eq!(wrapped, result, "non-zero-offset LHS should not be wrapped");
    }

    #[test]
    fn text_cell_first_set_uses_correct_kt() {
        let (mut p, cell_d_nr, v_nr) =
            parser_with_boxed_local(&Type::Text(crate::data::Deps::none()), "__cell_text", "s");
        let op_get = p.data.def_nr("OpGetText");
        let op_set = p.data.def_nr("OpSetText");
        let lhs = Value::Call(op_get, vec![Value::Var(v_nr), Value::Int(0)]);
        let result = Value::Call(
            op_set,
            vec![Value::Var(v_nr), Value::Int(0), Value::Text("hi".into())],
        );
        let wrapped = p.maybe_prepend_cell_alloc(result, &lhs);
        let cell_kt = i32::from(p.data.def(cell_d_nr).known_type());
        if let Value::Insert(ops) = wrapped
            && let Value::Call(_, args) = &ops[1]
        {
            assert!(matches!(args[1], Value::Int(kt) if kt == cell_kt));
        } else {
            panic!("expected Insert with OpDatabase op[1]");
        }
    }
}
