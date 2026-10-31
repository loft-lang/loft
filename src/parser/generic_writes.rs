// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Element writes a generic parsed against a type variable, rewritten for the concrete
//! element type once it is known: the primitive setter calls and the matched write triplets.

use super::{Data, DefType, Deps, Parser, Stores, Type, Value, v_block};
use crate::type_schema::StoreTypeOf;

impl Parser {
    /// P241 fix (2026-05-11) — slice 2: integer-only.  POST-PASS that
    /// walks the substituted IR + patches the variable table to
    /// rewrite the parametric vector-element-write triplet to its
    /// primitive shape.  Runs AFTER `substitute_type_in_value` AND
    /// AFTER `vars.substitute_type` so it sees the substituted-types
    /// IR and can patch the elm variable's type back to `Reference`
    /// (it was originally `Reference(T_d_nr, deps)` and
    /// `vars.substitute_type` turned it into the wrong primitive
    /// type because it holds a DbRef, not a primitive value).
    ///
    /// Detects the post-substitution triplet:
    ///
    /// ```ignore
    /// // Triplet at adjacent positions i, i+1, i+2:
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_T), Int(MAX)]))
    /// Call(OpCopyRecord, [src_value, Var(elm_var), Int(t_T)])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_T), Int(MAX)])
    /// ```
    ///
    /// When `concrete` is a primitive (slice 2: only `Type::Integer`),
    /// rewrites the triplet to the primitive shape:
    ///
    /// ```ignore
    /// // 4-op sequence:
    /// Call(OpPreAllocVector, [Var(out_var), Int(1), Int(elem_size)])
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_concrete_vec), Int(MAX)]))
    /// Call(OpSetInt, [Var(elm_var), Int(0), src_value])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_concrete_vec), Int(MAX)])
    /// ```
    ///
    /// AND patches `vars.set_type(elm_var, Type::Reference(...))`.
    ///
    /// Recurses into nested Blocks / Loops / Ifs / etc.  Slice 2 ships
    /// only `Type::Integer`; struct-T is a no-op (the existing
    /// OpCopyRecord path is correct because the source IS a DbRef).
    /// P241 fix slice 3 — true when `concrete` is a primitive type
    /// whose vector-element write uses a primitive setter
    /// (`OpSetInt` / `OpSetText` / `OpSetFloat` / etc.) at the
    /// parse-time concrete-T path instead of `OpCopyRecord`.
    /// `Type::Reference` / `Type::Vector` / etc. are NOT primitive
    /// targets — their vector-element shape uses `OpCopyRecord`,
    /// which the parametric IR already encodes correctly (the source
    /// IS a DbRef regardless of substitution).
    pub(crate) fn is_primitive_vector_element_target(tp: &Type) -> bool {
        // `@FR-N-Shape` — `.base()`: a `τ?` element is `τ`'s slot, its null the sentinel.
        matches!(
            tp.base(),
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
                | Type::Function(..)
                | Type::Enum(_, false, _) // plain enum (struct-enums use OpCopyRecord)
        )
    }

    /// Does the rewrite apply to this concrete type?  Both primitive and
    /// struct (Reference) targets need the type-id substitution; the only
    /// difference is whether OpCopyRecord is replaced (primitive) or kept
    /// with patched type-id args (struct).
    pub(crate) fn is_rewritable_vector_element_target(tp: &Type) -> bool {
        Self::is_primitive_vector_element_target(tp)
            || matches!(tp.base(), Type::Reference(_, _) | Type::Tuple(_))
    }

    /// The targets an APPEND's triplet is rewritten for: every element-write target, and a
    /// nested vector — an appended element that is itself a vector takes a zeroed handle and
    /// a deep copy, as the concrete append does.  An INDEXED write of a vector element is the
    /// clear-and-append lowering, not a record copy, so it stays out of
    /// [`Self::is_rewritable_vector_element_target`].
    pub(super) fn is_rewritable_append_target(tp: &Type) -> bool {
        Self::is_rewritable_vector_element_target(tp) || matches!(tp.base(), Type::Vector(_, _))
    }

    /// P241 fix slice 3 — build the per-type primitive setter Call
    /// for the rewritten triplet's middle op.  Mirrors the parse-time
    /// concrete-T dispatch in `parser/vectors.rs:1560-1599`.
    /// Returns `None` only when the type is not a primitive target
    /// (the caller should pre-check via `is_primitive_vector_element_target`).
    pub(super) fn primitive_setter_call(
        concrete: &Type,
        elm: Value,
        src_value: Value,
        data: &Data,
    ) -> Option<Value> {
        let pos = Value::Int(0);
        // A `τ?` writes as `τ` does (`@FR-N-Shape`) — except a `boolean?`, whose null is a byte
        // of its own: it is written as the concrete `v[i] = x` writes it, the value's byte
        // unchanged, where the dense boolean's 0/1 wrap would turn the null into `false`.
        if matches!(concrete, Type::Optional(inner) if **inner == Type::Boolean) {
            let d = data.def_nr("OpSetByte");
            return (d != u32::MAX)
                .then(|| Value::Call(d, vec![elm, pos, Value::Int(0), src_value]));
        }
        let concrete = concrete.base();
        // Resolve op def_nrs.  Each branch resolves only the ones it needs.
        let op = match concrete {
            Type::Integer(_) => {
                // The narrow widths through the one home every vector build shares —
                // `vectors::narrow_elm_write`, the twin of the index read; only the wide
                // 8-byte element stays on `OpSetInt`.  This arm used to key on the ALIAS
                // def's `forced_size` (`type_elm(concrete)` resolves every integer to the
                // one `integer` def, which has none), so it wrote every narrow element of a
                // generic `vector<T>` eight bytes wide into a one- or two-byte slot
                // (loft#1378: `200 0` for two `u8`s, `29768 32767` for `-3000, 3000` as i16).
                if let Some(call) = crate::parser::vectors::narrow_elm_write(
                    concrete,
                    elm.clone(),
                    &src_value,
                    data,
                ) {
                    call
                } else {
                    let d = data.def_nr("OpSetInt");
                    Value::Call(d, vec![elm, pos, src_value])
                }
            }
            Type::Float => {
                let d = data.def_nr("OpSetFloat");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Single => {
                let d = data.def_nr("OpSetSingle");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Boolean => {
                // Booleans store as a 0/1 byte; same shape as `set_field`'s
                // Boolean arm at `parser/mod.rs:2799`.  Must wrap the
                // raw boolean value in `if val { 1 } else { 0 }` so the
                // OpSetByte writes the correct integer encoding —
                // without the wrap, OpSetByte sees a DbRef-shaped
                // boolean that codegen can't decode.
                let d = data.def_nr("OpSetByte");
                let wrapped = crate::data::v_if(src_value, Value::Int(1), Value::Int(0));
                Value::Call(d, vec![elm, pos, Value::Int(0), wrapped])
            }
            Type::Character => {
                let d = data.def_nr("OpSetCharacter");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Text(_) => {
                let d = data.def_nr("OpSetText");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Function(..) => {
                // Plan-06 phase 4d.A.2 — fn-ref vector elements store the
                // 4-byte i32 d_nr.  Same shape as `vectors.rs:1597`.
                let d = data.def_nr("OpSetInt4");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Enum(_, false, _) => {
                // Plain enum — variants encode as a small integer index.
                let d = data.def_nr("OpSetEnum");
                Value::Call(d, vec![elm, pos, src_value])
            }
            _ => return None,
        };
        // Defensive: if any def_nr lookup returned u32::MAX, return None
        // so the caller falls through (rather than emitting a malformed Call).
        if let Value::Call(d, _) = &op
            && *d == u32::MAX
        {
            return None;
        }
        Some(op)
    }

    pub(crate) fn rewrite_generic_vector_writes(
        mut val: Value,
        bindings: &[(u32, Type)],
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
    ) -> Value {
        // Slice 2: Integer.  Slice 3: extended to all primitive
        // types whose vector-element shape uses a primitive setter
        // instead of OpCopyRecord.  P255 (2026-05-12): also handle
        // struct T (Type::Reference) — the OpCopyRecord shape is
        // kept but its tp arg AND the surrounding OpNewRecord /
        // OpFinishRecord parent_tp args must be patched from the
        // parametric T type-id to the concrete struct's type-ids,
        // otherwise the runtime reads the wrong record size.
        //
        // Each write is rewritten against the binding of the type variable IT was lowered
        // for ([`Self::element_write_binding`]), so a write of an ordinary record inside the
        // template keeps its own row, and with several variables each write takes its own.  The walk is total (`for_each_child_mut`), and the
        // operator lists of a block or loop are searched for triplets once their children
        // are done.
        val.for_each_child_mut(&mut |child| {
            let taken = std::mem::replace(child, Value::Null);
            *child = Self::rewrite_generic_vector_writes(taken, bindings, vars, data, database);
        });
        match val {
            // A `Span` wraps one statement, and the walk above has already rewritten it.
            span @ Value::Span(_) => span,
            Value::Block(mut bl) => {
                let ops = std::mem::take(&mut bl.operators);
                bl.operators =
                    Self::rewrite_vector_write_triplets(ops, bindings, vars, data, database);
                Value::Block(bl)
            }
            Value::Loop(mut lp) => {
                let ops = std::mem::take(&mut lp.operators);
                lp.operators =
                    Self::rewrite_vector_write_triplets(ops, bindings, vars, data, database);
                Value::Loop(lp)
            }
            Value::Call(d, args) => {
                Self::rewrite_indexed_element_write(d, args, bindings, data, database)
            }
            other => other,
        }
    }

    /// The binding a parametric element write stands for: the type variable whose record
    /// row the write's `OpCopyRecord` was lowered against (`tp`, its flag bits masked off —
    /// [`crate::keys::COPY_TP_MASK`]).  `None` for a write of anything else, so a record
    /// appended or assigned inside the template keeps its own row, a `vector<T>` element of
    /// a `vector<vector<T>>` (a row of its own) is not taken for `T`, and each of several
    /// variables finds its own binding.  A variable with no row yet (`u16::MAX`) matches
    /// nothing: the sentinel is shared by every unresolved row.
    pub(super) fn element_write_binding<'b>(
        tp: i32,
        bindings: &'b [(u32, Type)],
        data: &Data,
    ) -> Option<&'b Type> {
        let mask = i32::from(crate::keys::COPY_TP_MASK);
        bindings
            .iter()
            .find(|(holder, _)| {
                let row = data.def(*holder).known_type();
                row != u16::MAX && i32::from(row) & mask == tp & mask
            })
            .map(|(_, bound)| bound)
    }

    /// P241 fix (2026-05-11) — slice 2: integer-only.  Walks a Block's
    /// post-substitution operator list looking for the parametric
    /// vector-element-write triplet emitted by
    /// `parser/vectors.rs::new_record` (for parametric T):
    ///
    /// ```ignore
    /// // Triplet at adjacent positions i, i+1, i+2:
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_T), Int(MAX)]))
    /// Call(OpCopyRecord, [src_value, Var(elm_var), Int(t_T_known)])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_T), Int(MAX)])
    /// ```
    ///
    /// When `concrete` is a primitive (slice 2: only `Type::Integer`),
    /// rewrites the triplet to the primitive shape:
    ///
    /// ```ignore
    /// // 4-op sequence:
    /// Call(OpPreAllocVector, [Var(out_var), Int(1), Int(elem_size)])
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_concrete_vec), Int(MAX)]))
    /// Call(OpSetInt, [Var(elm_var), Int(0), src_value])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_concrete_vec), Int(MAX)])
    /// ```
    ///
    /// Where:
    /// - `t_concrete_vec = database.vector(e)` and `elem_size = database.size(e)`, where `e`
    ///   is `Data::vector_element_type(concrete)` — the one home the concrete `+=` append
    ///   asks (loft#1378).
    ///
    /// Slice 2 ships only the `Type::Integer` arm.  Slice 3 extends to
    /// Text / Float / Single / Boolean / Character / Enum / Function +
    /// narrow-int variants.  For struct T (the `Type::Reference` shape)
    /// the rewrite is a no-op — the existing OpCopyRecord path is
    /// correct because the source IS a DbRef.
    ///
    /// Tolerates `Value::Span` and `Value::Line` markers between or
    /// inside the triplet operators by using `unspan` for shape
    /// matching.  Multi-element pushes (`out += [a, b, c]`) produce
    /// N adjacent triplets; this function processes them
    /// independently — each triplet rewrites once.
    pub(super) fn rewrite_vector_write_triplets(
        ops: Vec<Value>,
        bindings: &[(u32, Type)],
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
    ) -> Vec<Value> {
        // Register the element rows of each of the template's own variables up front, before
        // any triplet is looked for — the order type rows are minted in is observable
        // (native's `init()` replays it), and this is where they have always been minted.  An
        // interface's associated type (a placeholder whose parent is the interface) mints its
        // rows where a write of it is rewritten.
        for (holder, bound) in bindings {
            let parent = data.def(*holder).parent;
            let associated = (parent as usize) < data.definitions.len()
                && data.def_type(parent) == DefType::Interface;
            if !associated && Self::is_rewritable_vector_element_target(bound) {
                let content = data
                    .vector_element_type(bound, database)
                    .unwrap_or_else(|| database.db_type(bound, data));
                database.vector(content);
            }
        }
        // Resolve op def_nrs once — re-resolution per-triplet would
        // cost N lookups for a long Block.
        let new_record_d = data.def_nr("OpNewRecord");
        let copy_record_d = data.def_nr("OpCopyRecord");
        let finish_record_d = data.def_nr("OpFinishRecord");
        let pre_alloc_d = data.def_nr("OpPreAllocVector");
        if new_record_d == u32::MAX
            || copy_record_d == u32::MAX
            || finish_record_d == u32::MAX
            || pre_alloc_d == u32::MAX
        {
            return ops; // missing op definitions — bail safely
        }
        // Walk operators looking for the triplet.  Build a new vec
        // with rewrites applied; copy unchanged ops verbatim.
        // Drain `ops` into a deque-like cursor so we can take owned
        // values without index juggling — rewriting consumes 3 ops
        // and produces 4, so we can't use simple in-place mutation.
        let mut iter = ops.into_iter();
        let mut out: Vec<Value> = Vec::new();
        let mut buf: Vec<Value> = Vec::new();
        loop {
            // Refill buffer to at least 3 entries (the triplet length).
            while buf.len() < 3 {
                match iter.next() {
                    Some(v) => buf.push(v),
                    None => break,
                }
            }
            if buf.len() < 3 {
                // Tail — nothing left that could be a full triplet.
                out.extend(buf);
                break;
            }
            let matched = Self::match_vector_write_triplet(
                &buf[0],
                &buf[1],
                &buf[2],
                new_record_d,
                copy_record_d,
                finish_record_d,
            );
            // Slice 3 covers all primitive vector-element targets
            // (Integer/Text/Float/Single/Boolean/Character/Enum/Function +
            // narrow-int variants).  P255 extends to struct T (Reference)
            // by keeping OpCopyRecord and patching its tp arg.  The element type is the
            // binding of the variable this triplet was lowered for — never the template's
            // first variable by default.
            // A `τ?` element is `τ`'s shape with a nullability bit (@FR-N-Shape): written as
            // `τ` — at `integer?` the triplet stayed a record copy and the write crashed.
            let site = matched.as_ref().and_then(|(_, _, _, copy_tp)| {
                Self::element_write_binding(*copy_tp, bindings, data)
                    .map(|bound| bound.base().clone())
                    .filter(Self::is_rewritable_append_target)
            });
            if let (Some((elm_var, out_var, src_value, _)), Some(concrete)) = (matched, site) {
                buf.drain(0..3);
                Self::emit_rewritten_triplet(
                    &mut out,
                    (elm_var, out_var, src_value),
                    &concrete,
                    vars,
                    data,
                    database,
                    (new_record_d, copy_record_d, finish_record_d, pre_alloc_d),
                );
            } else {
                // No triplet at this position — emit one op and slide.
                out.push(buf.remove(0));
            }
        }
        out
    }

    /// Emit the concrete shape of one matched parametric element-write triplet at the type
    /// `concrete` — the four-op sequence documented on [`Self::rewrite_vector_write_triplets`].
    pub(super) fn emit_rewritten_triplet(
        out: &mut Vec<Value>,
        (elm_var, out_var, src_value): (u16, u16, Value),
        concrete: &Type,
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
        (new_record_d, copy_record_d, finish_record_d, pre_alloc_d): (u32, u32, u32, u32),
    ) {
        let is_struct_target = matches!(concrete, Type::Reference(_, _));
        let is_vector_target = matches!(concrete.base(), Type::Vector(_, _));
        // Look up the concrete vector-element record type-id.
        // Mirrors `vectors.rs:1532-1535` — `database.vector(content_db_type)`
        // returns the synthetic vector<concrete> type id (registers
        // it on first use; idempotent on subsequent calls).
        // The element's STORAGE type from the one home every writer and reader of a vector
        // element routes through — `Data::vector_element_type`, which the concrete `+=`
        // append asks too (a narrow integer keeps its width, a struct is its record) — and
        // the stride from the store that owns the layout, exactly as the concrete path sizes
        // its `OpPreAllocVector` (`vectors.rs`: `database.size(known)`).  This used to
        // re-derive the stride from the Type alone (`type_element_size`), summing a struct's
        // fields and descending into a field of the struct's own type without end, and it
        // was computed before any triplet was looked for — so EVERY generic instantiated at
        // a self-referential struct died in the parser with a bare SIGSEGV, `fn id<T>(v: T)
        // -> T? { v }` at `struct Node { …, next: reference<Node>? }` included (loft#1378).
        let content_db_type = data
            .vector_element_type(concrete, database)
            .unwrap_or_else(|| database.db_type(concrete, data));
        let concrete_vec_tp = i32::from(database.vector(content_db_type));
        let elem_size = i32::from(database.size(content_db_type));
        {
            {
                // Patch the elm var's type back to `Reference(content_def_nr, [out_var])`.
                // After `vars.substitute_type`, `Reference(T_d_nr, deps)` became
                // `Type::<concrete>` (deps lost — primitive types don't carry deps).
                // But the elm var is the destination of `OpNewRecord`, so it
                // holds a DbRef regardless of T's concrete type.  Without this
                // patch, codegen emits the wrong opcodes for elm reads/writes
                // (e.g. `VarInt` instead of `VarRef`) and the runtime reads
                // garbage.  The dep on `out_var` mirrors `unique_elm_var`'s
                // `self.vars.depend(elm, vec)` so elm doesn't outlive the
                // backing store.
                let content_def_nr = data.type_def_nr(concrete);
                // A nested vector's element is typed as the vector it is, as `unique_elm_var`
                // types it for the concrete append.
                vars.set_type(
                    elm_var,
                    if let Type::Vector(inner, _) = concrete.base() {
                        Type::Vector(inner.clone(), Deps::frame1(out_var))
                    } else {
                        Type::Reference(content_def_nr, Deps::frame1(out_var))
                    },
                );
                // 1. OpPreAllocVector(Var(out_var), Int(1), Int(elem_size))
                //    Mirrors `vectors.rs:1161-1178` for perf parity with
                //    concrete-T vector pushes.
                out.push(Value::Call(
                    pre_alloc_d,
                    vec![Value::Var(out_var), Value::Int(1), Value::Int(elem_size)],
                ));
                // 2. Set(elm_var, OpNewRecord(Var(out_var), Int(concrete_vec_tp), Int(MAX)))
                out.push(Value::Set(
                    elm_var,
                    Box::new(Value::Call(
                        new_record_d,
                        vec![
                            Value::Var(out_var),
                            Value::Int(concrete_vec_tp),
                            Value::Int(i32::from(u16::MAX)),
                        ],
                    )),
                ));
                // 3. Middle op:
                //    - Primitive T: per-type setter (OpSetInt / OpSetText / …)
                //      replaces OpCopyRecord (no DbRef to copy).
                //    - Struct T: keep OpCopyRecord but patch its tp arg
                //      from the parametric T's known_type to the concrete
                //      struct's known_type so `state::copy_record` reads
                //      the correct record size.
                if let Type::Tuple(_) = concrete.base() {
                    // A tuple element is written member by member, as the concrete append
                    // writes it (`emit_tuple_set_ops`) — which mints a tuple temporary, so it
                    // is lowered in the monomorph's own frame (`TV_TUPLE_ELEM`).  As a record
                    // copy at the template's row it read a stack tuple as a `DbRef` and the
                    // store panicked.
                    out.push(v_block(
                        vec![Value::Var(elm_var), src_value],
                        concrete.clone(),
                        Self::TV_TUPLE_ELEM,
                    ));
                } else if is_vector_target {
                    // @FR-G-Mono — a vector element is a 4-byte handle to a record of its own:
                    // zeroed, then the source vector deep-copied in at the element's row, as
                    // the concrete append writes it (`build_comprehension_code`).  Left a record
                    // copy at the template's row, `filter(vv, f)` inside a generic panicked at
                    // `vector<vector<integer>>` ("the vector handle … points at record 3,
                    // whose size word is -1").
                    let set_int4_d = data.def_nr("OpSetInt4");
                    out.push(Value::Call(
                        set_int4_d,
                        vec![Value::Var(elm_var), Value::Int(0), Value::Int(0)],
                    ));
                    out.push(Value::Call(
                        copy_record_d,
                        vec![
                            src_value,
                            Value::Var(elm_var),
                            Value::Int(i32::from(content_db_type)),
                        ],
                    ));
                } else if is_struct_target {
                    let known_tp = if (content_def_nr as usize) < data.definitions.len() {
                        i32::from(data.def(content_def_nr).known_type())
                    } else {
                        i32::from(u16::MAX)
                    };
                    out.push(Value::Call(
                        copy_record_d,
                        vec![src_value, Value::Var(elm_var), Value::Int(known_tp)],
                    ));
                } else {
                    let setter =
                        Self::primitive_setter_call(concrete, Value::Var(elm_var), src_value, data);
                    if let Some(call) = setter {
                        out.push(call);
                    } else {
                        // Concrete type matched `is_primitive_vector_element_target`
                        // but no setter mapping exists — should not happen.
                        // Bail by emitting a no-op (the OpFinishRecord still
                        // fires; the value just isn't written).  Defensive
                        // fall-through to keep the IR well-formed.
                    }
                }
                // 4. OpFinishRecord(Var(out_var), Var(elm_var), Int(concrete_vec_tp), Int(MAX))
                out.push(Value::Call(
                    finish_record_d,
                    vec![
                        Value::Var(out_var),
                        Value::Var(elm_var),
                        Value::Int(concrete_vec_tp),
                        Value::Int(i32::from(u16::MAX)),
                    ],
                ));
            }
        }
    }

    /// P241 fix slice 2 — pattern-matches the parametric vector-element
    /// write triplet.  Returns `Some((elm_var, out_var, src_value))`
    /// when all three statements match the expected shape AND the
    /// per-statement vars cross-reference correctly:
    /// - The Set's target var equals the OpCopyRecord's 2nd arg's
    ///   var equals the OpFinishRecord's 2nd arg's var (`elm_var`).
    /// - The OpNewRecord's 1st arg's var equals the OpFinishRecord's
    ///   1st arg's var (`out_var`).
    /// - All Int args match expected sentinel shape (Int(MAX) for
    ///   `fld`; matching parametric type-id between OpNewRecord and
    ///   OpFinishRecord).
    ///
    /// `src_value` is the OpCopyRecord's 1st arg (the value being
    /// copied into the new vector slot).  Returned by-value (cloned
    /// from the matched IR) so the caller can construct the new
    /// `OpSetInt` Call without borrowing back into `ops`.
    /// `v[i] = x` inside a template — the write spelling the triplet rewriter does not
    /// see, because an INDEXED assignment emits a lone `OpCopyRecord` instead of the
    /// `OpNewRecord` / `OpCopyRecord` / `OpFinishRecord` sequence an APPEND emits.
    ///
    /// Left parametric, that `OpCopyRecord` reached the monomorph with the type
    /// VARIABLE's record id and a source that is no longer a record: at every scalar type
    /// the run PANICKED in the allocator, at `float`/`text` it walked a corrupt edge, and
    /// for a struct parameter it silently wrote nowhere and read the old element back.
    /// The concrete twin emits `OpSetInt(OpGetVector(v, 8, 0), 0, x)`, so that is what the
    /// monomorph must emit too — `(G-Mono)`, and the same one-home setter builder the
    /// append path uses.
    ///
    /// The two spellings are told apart by WHERE the copy writes: an append's destination
    /// is the freshly-minted `Var(elm)`, an indexed write's is an element READ.  So only a
    /// destination rooted in `OpGetVector` is rewritten here, which leaves the triplet for
    /// its own matcher and leaves an ordinary record copy inside a generic body alone.
    pub(super) fn rewrite_indexed_element_write(
        d: u32,
        args: Vec<Value>,
        bindings: &[(u32, Type)],
        data: &Data,
        database: &mut Stores,
    ) -> Value {
        let copy_record_d = data.def_nr("OpCopyRecord");
        if d != copy_record_d || copy_record_d == u32::MAX || args.len() != 3 {
            return Value::Call(d, args);
        }
        let Some(elem) = Self::element_write_destination(&args[1], data) else {
            return Value::Call(d, args);
        };
        let Value::Int(copy_tp) = args[2].unspan() else {
            return Value::Call(d, args);
        };
        let Some(concrete) = Self::element_write_binding(*copy_tp, bindings, data)
            .filter(|bound| Self::is_rewritable_vector_element_target(bound))
            .cloned()
        else {
            return Value::Call(d, args);
        };
        let concrete = &concrete;
        if Self::is_primitive_vector_element_target(concrete) {
            // The element's VALUE wrapper is stripped by `element_write_destination`: the
            // substitution wraps every rewritten element read for value use, and a write
            // destination is the one place that wrapper is wrong.
            if let Some(setter) = Self::primitive_setter_call(concrete, elem, args[0].clone(), data)
            {
                return setter;
            }
            return Value::Call(d, args);
        }
        // A struct element keeps the record copy and takes the CONCRETE record id — the
        // parametric one names an attribute-less placeholder, so the runtime copied the
        // wrong number of words.
        let tp = i32::from(database.db_type(concrete, data));
        Value::Call(d, vec![args[0].clone(), elem, Value::Int(tp)])
    }

    /// The element reference an `OpCopyRecord` destination names, with the value-getter
    /// wrapper stripped — or `None` when the destination is not an element read at all.
    ///
    /// `substitute_type_in_value` wraps every element read it retargets so the value can
    /// be USED (`OpGetInt(OpGetVector(…), 0)`); a write destination wants the reference
    /// itself, and the wrapper is what made the copy read an `i64` where it needed an
    /// address.  Recognising the wrapper by its ARGUMENT SHAPE — one element read plus a
    /// zero offset — keeps this from having to carry a second copy of the op list that
    /// `wrap_vector_get_val` already owns.
    pub(super) fn element_write_destination(dest: &Value, data: &Data) -> Option<Value> {
        // A tuple element's read is stamped for unboxing (`TV_TUPLE_READ`); as a write
        // destination it is the element reference inside.
        if let Value::Block(bl) = dest.unspan()
            && bl.name == Self::TV_TUPLE_READ
            && let [inner] = bl.operators.as_slice()
        {
            return Self::element_write_destination(inner, data);
        }
        let is_element_read = |v: &Value| {
            matches!(v.unspan(), Value::Call(d, _)
                if *d != u32::MAX
                    && (*d as usize) < data.definitions.len()
                    && matches!(data.def(*d).name(), "OpGetVector" | "OpGetVectorNullable"))
        };
        if is_element_read(dest) {
            return Some(dest.clone());
        }
        if let Value::Call(_, wrapped) = dest.unspan()
            && wrapped.len() == 2
            && matches!(wrapped[1].unspan(), Value::Int(0))
            && is_element_read(&wrapped[0])
        {
            return Some(wrapped[0].clone());
        }
        None
    }

    pub(super) fn match_vector_write_triplet(
        op0: &Value,
        op1: &Value,
        op2: &Value,
        new_record_d: u32,
        copy_record_d: u32,
        finish_record_d: u32,
    ) -> Option<(u16, u16, Value, i32)> {
        // op0: Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(_), Int(MAX)]))
        let (elm_var_set, out_var, _new_record_tp) = match op0.unspan() {
            Value::Set(elm, set_val) => {
                let inner = set_val.unspan();
                if let Value::Call(d, args) = inner
                    && *d == new_record_d
                    && args.len() == 3
                    && let Value::Var(out) = args[0].unspan()
                    && let Value::Int(tp) = args[1].unspan()
                    && let Value::Int(fld) = args[2].unspan()
                    && *fld == i32::from(u16::MAX)
                {
                    (*elm, *out, *tp)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // op1: Call(OpCopyRecord, [src_value, Var(elm_var), Int(copy_tp)])
        let (src_value, copy_tp) = match op1.unspan() {
            Value::Call(d, args) => {
                if *d == copy_record_d
                    && args.len() == 3
                    && let Value::Var(elm_in_copy) = args[1].unspan()
                    && *elm_in_copy == elm_var_set
                    && let Value::Int(copy_tp) = args[2].unspan()
                {
                    (args[0].clone(), *copy_tp)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // op2: Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(_), Int(MAX)])
        match op2.unspan() {
            Value::Call(d, args) => {
                if *d == finish_record_d
                    && args.len() == 4
                    && let Value::Var(out_in_finish) = args[0].unspan()
                    && let Value::Var(elm_in_finish) = args[1].unspan()
                    && let Value::Int(_finish_tp) = args[2].unspan()
                    && let Value::Int(fld) = args[3].unspan()
                    && *out_in_finish == out_var
                    && *elm_in_finish == elm_var_set
                    && *fld == i32::from(u16::MAX)
                {
                    Some((elm_var_set, out_var, src_value, copy_tp))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// One `OpConvBoolFrom<X>` call over `args`, or `None` when the op is not registered.
    ///
    /// A helper only so the exhaustive map above reads as a table of decisions rather
    /// than of `def_nr` bookkeeping repeated per arm.
    pub(super) fn conv_bool_call(name: &str, args: &[Value], data: &Data) -> Option<Value> {
        let d = data.def_nr(name);
        if d == u32::MAX {
            None
        } else {
            Some(Value::Call(d, args.to_vec()))
        }
    }

    /// I9-vec: wrap an `OpGetVector` result with the value-extraction op the ELEMENT
    /// type needs (`OpGetInt`, `OpGetCharacter`, …), matching what the hand-written
    /// concrete function emits for the same read.  A reference-shaped element needs no
    /// wrapper — the `DbRef` IS the value.
    ///
    /// The match is EXHAUSTIVE on purpose.  It used to end `_ => return code`,
    /// which reads as *"everything else is reference-shaped"* and was not: `character` and
    /// a VALUE enum both store a raw slot that needs unpacking, and both fell through it.
    /// A template's `v[i]` then handed back the address as if it were the value —
    /// `v[1]` on `['a','b']` answered a garbage codepoint and on `[Col::Blue, Col::Green]`
    /// answered `null`, on both backends, while the concrete twin was right. Nothing said
    /// so, because a missing arm and a deliberate one looked identical. Now adding a
    /// `Type` variant fails the build here until someone decides which it is.
    pub(super) fn wrap_vector_get_val(code: Value, tp: &Type, data: &Data) -> Value {
        let p = Value::Int(0);
        // @PLN25: peel `Optional(τ)` — a nullable scalar element needs the SAME value-
        // extraction op as its base; without this it fell through with no OpGet
        // and the raw slot was read as a DbRef.
        // A NARROW integer element is unpacked at the width and encoding its type stores,
        // through the read twin of the op every vector build writes it with
        // (`vectors::narrow_elm_read`); only the wide 8-byte element takes `OpGetInt` below.
        // Reading every integer through `OpGetInt` took eight bytes out of a one- or two-byte
        // slot, so a generic body's `v[i]` on a `vector<u8>` or `vector<i16>` answered its
        // neighbours' bytes (loft#1378's in-body read cells).
        if let Some(read) = crate::parser::vectors::narrow_elm_read(tp, code.clone(), data) {
            return read;
        }
        let op_name = match tp.base() {
            Type::Integer(_) => "OpGetInt",
            Type::Float => "OpGetFloat",
            Type::Single => "OpGetSingle",
            Type::Text(_) => "OpGetText",
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => "OpGetBoolean",
            Type::Character => "OpGetCharacter",
            // A VALUE enum is a discriminant byte in the slot; a struct-enum below is a
            // reference, which is why the two spellings part here.
            Type::Enum(_, false, _) => "OpGetEnum",
            // Reference-shaped: the slot holds a `DbRef` and the read IS the value.
            Type::Reference(_, _)
            | Type::Enum(_, true, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Iterator(_, _)
            | Type::Function(..)
            | Type::Routine(_)
            | Type::RefVar(_) => return code,
            // @FR-G-Mono — a tuple element is the stack tuple its twin unboxes (`v[i]`, `for x
            // in v`): stamped here, lowered in the monomorph's frame (`TV_TUPLE_READ`).  Left a
            // reference to the stored element, a callback over `vector<T>` at `(integer,
            // integer)` read the reference's bytes as the tuple's (`137438953478`) and native
            // did not compile.
            Type::Tuple(_) => {
                return crate::data::v_block(vec![code], tp.base().clone(), Self::TV_TUPLE_READ);
            }
            // Not element types: `Unknown`/`Null`/`Void`/`Never` carry no storage,
            // `Keys` describes a key list, and `Rewritten` is an append form that has
            // already been lowered.  `Optional` cannot appear — `base()` peeled it.
            Type::Unknown(_)
            | Type::Null
            | Type::Void
            | Type::Never
            | Type::Keys
            | Type::Rewritten(_)
            | Type::Optional(_) => return code,
        };
        let d = data.def_nr(op_name);
        if d == u32::MAX {
            return code;
        }
        Value::Call(d, vec![code, p])
    }
}
