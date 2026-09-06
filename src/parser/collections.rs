// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Context, DefType, I32, Level, LexItem, OutputState, Parser, Parts, Type, Value,
    diagnostic_format, v_block, v_if, v_loop, v_set, var_size,
};
use crate::variables::Function;

/// True when `v` (unspanned) is a CAPTURING fn-ref — `Value::FnRef` whose
/// closure-var slot is set (`!= u16::MAX`).  A bare `return add5;` carries
/// `u16::MAX` (non-capturing) and is fine for par.
fn is_capturing_fnref(v: &Value) -> bool {
    match v.unspan() {
        Value::FnRef(_, clos_var, _) => *clos_var != u16::MAX,
        // A capturing lambda lowers to a `fn_ref_with_closure` block that builds
        // the closure record and yields the `FnRef` as its tail expression.
        Value::Block(bl) | Value::Loop(bl) => bl.operators.last().is_some_and(is_capturing_fnref),
        Value::Insert(ops) => ops.last().is_some_and(is_capturing_fnref),
        _ => false,
    }
}

/// The text a text-iteration driver actually reads its characters from — the
/// first argument of the `OpTextCharacterNullable` call [`Parser::iter_text`]
/// buried in `iter_next`.
///
/// The loop's bound must be read from THIS expression, not from the collection
/// expression the source was written as: the two differ once a substitution has
/// rewritten the source, and a bound taken from the other one would answer for
/// a different text than the character read walks (loft#755).
pub(crate) fn find_text_coll(v: &Value, target: u32) -> Option<Value> {
    match v {
        Value::Call(op, args) if *op == target => args.first().cloned(),
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            args.iter().find_map(|a| find_text_coll(a, target))
        }
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().find_map(|o| find_text_coll(o, target))
        }
        Value::If(c, t, e) => find_text_coll(c, target)
            .or_else(|| find_text_coll(t, target))
            .or_else(|| find_text_coll(e, target)),
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) | Value::Yield(x) => {
            find_text_coll(x, target)
        }
        Value::Span(b) => find_text_coll(&b.1, target),
        _ => None,
    }
}

/// True when a par worker's body returns a capturing closure in a `return` or
/// tail position.  Conservative — only flags closures found directly in return
/// position, so a non-capturing fn-ref return and any closure used only
/// internally (never returned) are never rejected; an indirect return (via a
/// local variable) falls through to the runtime path rather than a false
/// positive.  Used to give a clear diagnostic instead of the dangling-ref panic.
fn worker_returns_capturing_closure(body: &Value) -> bool {
    match body.unspan() {
        Value::Return(inner) => is_capturing_fnref(inner),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.last().is_some_and(is_capturing_fnref)
                || bl.operators.iter().any(worker_returns_capturing_closure)
        }
        Value::Insert(ops) => {
            ops.last().is_some_and(is_capturing_fnref)
                || ops.iter().any(worker_returns_capturing_closure)
        }
        Value::If(_, t, f) => {
            is_capturing_fnref(t)
                || is_capturing_fnref(f)
                || worker_returns_capturing_closure(t)
                || worker_returns_capturing_closure(f)
        }
        _ => false,
    }
}

/// Plan-06 ARC.md A3 / A3.5 — how to convert the i64 returned by
/// `parallel_buf_get_narrow` back to the worker's actual return type.
enum NarrowWrap {
    /// No conversion needed — raw i64 result is what the body
    /// expects (narrow Integer; body's `r as integer` accepts i64
    /// natively).
    None,
    /// Single-arg Op call: `OpFoo(buf_get_call) -> T`.  Used for
    /// `OpConvCharacterFromInt` (i64 → char via low 4 bytes) and
    /// `OpCastEnumFromInt` (i64 → u8 enum variant; treats i64::MIN
    /// as 255-sentinel).  Op functions are looked up by their bare
    /// name (no `n_` prefix — that's reserved for user-fn `n_<name>`).
    OpCall(&'static str),
    /// `OpNeInt(buf_get_call, 0) -> boolean`.  Boolean's existing
    /// `OpConvBoolFromInt` has null-check semantics (v != i64::MIN),
    /// not value semantics (v != 0) — the buf_get always returns
    /// 0 or 1, never i64::MIN, so the conv would yield true for
    /// both.  `OpNeInt` gives the right 0 → false / 1 → true mapping.
    NeZero,
    /// ARC.md A3.6 — use a different buf_get fn-name instead of
    /// `n_parallel_buf_get_narrow`, and skip the wrap (the named
    /// fn already returns the typed value).  Used for `single`
    /// (routes through `n_parallel_buf_get_single` returning
    /// `single` directly via `f32::from_bits`).
    TypedBufGet(&'static str),
}

/// Plan-06 ARC.md A3 / A3.5 — descriptor for routing a parallel
/// worker's narrow-primitive return through `n_parallel_queue_narrow`
/// instead of the legacy materialised-vector path.
///
/// Returned by [`narrow_route_for`] when the worker's return type is
/// representable in 1/2/4 bytes per row.  `None` otherwise (8-byte
/// integers stay on the regular `n_parallel_queue` path; reference /
/// text / fn-ref returns have their own queue variants).
/// The parts of the `any` / `all` / `count_if` loop that are the same for all three:
/// what runs before the loop, the element the predicate is handed, how the next element
/// is fetched, and how the loop ends.  Each builtin adds only its own accumulator step.
///
/// Returned by [`Parser::predicate_loop_scaffold`].
struct PredicateLoop {
    /// Statements that run once, before the loop: bind the vector, park the fn-ref value
    /// (when there is one), create the iterator.
    preamble: Vec<Value>,
    /// The VALUE handed to the predicate for one element, from
    /// [`Parser::callback_element_arg`] — `Var(for_var)` for every element type except a
    /// TUPLE, which is unboxed from its DbRef into the stack tuple the callee declares
    /// (loft#1074).  Held here rather than rebuilt at the call, so the scaffold and the
    /// call cannot disagree about the element's representation.
    elem_arg: Value,
    /// Fetch of the next element into `for_var`, first statement of the loop body.
    for_next: Value,
    /// `if <past the end> { break }`, second statement of the loop body.
    break_if_done: Value,
    /// Local holding the predicate when it is a fn-ref VALUE (a capturing lambda, or a
    /// fn-ref variable) rather than a static definition; `None` for a static fn-ref.
    fn_ref_var: Option<u16>,
}

/// The call to the predicate for one element — `Call` for a static fn-ref, `CallRef` for a
/// fn-ref value [`Parser::predicate_loop_scaffold`] parked in a local.
fn predicate_call(fn_d_nr: Option<u32>, lp: &PredicateLoop) -> Value {
    match fn_d_nr {
        Some(d) => Value::Call(d, vec![lp.elem_arg.clone()]),
        None => Value::CallRef(
            lp.fn_ref_var
                .expect("a non-static predicate always parks its fn-ref in a local"),
            vec![lp.elem_arg.clone()],
        ),
    }
}

struct NarrowRoute {
    /// Per-row stride in bytes (1, 2, or 4).
    width: u8,
    /// Sign-extension flag passed to `parallel_buf_get_narrow`.
    /// `true` for signed narrow Integer (i8/i16/i32) — buf_get
    /// reads via `i8`/`i16`/`i32` cast.  `false` for unsigned int,
    /// Boolean, Character, and Enum-no-payload.
    signed: bool,
    /// How to wrap the i64 buf_get result back to the worker's
    /// declared return type (see [`NarrowWrap`]).
    wrap: NarrowWrap,
}

/// Decide whether the worker's return type fits the narrow-Queue
/// route.  Mirrors the original A3 plan's `route_narrow_queue` design:
///
/// | Type                              | width | signed     | wrap                          |
/// |-----------------------------------|-------|------------|-------------------------------|
/// | `Integer(spec)` w/ byte_width 1/2/4 | spec  | spec.min<0 | `None`                        |
/// | `Boolean`                         | 1     | false      | `NeZero` (OpNeInt(_, 0))      |
/// | `Character`                       | 4     | false      | `OpCall(OpConvCharacterFromInt)` |
/// | `Enum(_, false, _)` (no payload)  | 1     | false      | `OpCall(OpCastEnumFromInt)`   |
/// | anything else                     | None — falls through to other routes                     |
///
/// 8-byte Integer + Float go through the regular `n_parallel_queue`
/// (wide u64 rows; Float reads via `parallel_buf_get_float`).
/// Single routes here with `TypedBufGet` — its f32 bit pattern fits
/// stride 4 and `parallel_buf_get_single` recovers the typed value.
fn narrow_route_for(ret_type: &Type) -> Option<NarrowRoute> {
    match ret_type {
        Type::Integer(spec) => match spec.byte_width(true) {
            w @ (1 | 2 | 4) => Some(NarrowRoute {
                width: w,
                signed: spec.min < 0,
                wrap: NarrowWrap::None,
            }),
            _ => None,
        },
        Type::Boolean => Some(NarrowRoute {
            width: 1,
            signed: false,
            wrap: NarrowWrap::NeZero,
        }),
        Type::Character => Some(NarrowRoute {
            width: 4,
            signed: false,
            wrap: NarrowWrap::OpCall("OpConvCharacterFromInt"),
        }),
        Type::Enum(_, false, _) => Some(NarrowRoute {
            width: 1,
            signed: false,
            wrap: NarrowWrap::OpCall("OpCastEnumFromInt"),
        }),
        // ARC.md A3.6 — `single` (f32) routes through the narrow
        // path with stride 4.  No wrap: the typed buf_get fn
        // returns `single` directly via `f32::from_bits` over the
        // same per-row bytes.  Symmetric with Store::set_single.
        Type::Single => Some(NarrowRoute {
            width: 4,
            signed: false,
            wrap: NarrowWrap::TypedBufGet("n_parallel_buf_get_single"),
        }),
        _ => None,
    }
}

/// The synthetic definition a deferred `par` marker calls, in its MANGLED spelling —
/// the one `Data::def_nr` answers to (loft#1040).
pub(crate) const PAR_MARKER_FN: &str = "n___par_template";

/// Every variable the par body binds DIRECTLY from the loop element (`_ = e`, `q = e`),
/// read BEFORE the accessor substitution rewrites those reads.  Such a binding holds a
/// BORROWED view of the input vector, and the binding itself drops the deps that said so —
/// which is why the body then freed the caller's record once per row.
fn elem_borrow_bindings(block: &Value, elem_var: u16) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    block.walk(&mut |v| {
        if let Value::Set(target, from) = v
            && matches!(from.unspan(), Value::Var(src) if *src == elem_var)
            && !out.contains(target)
        {
            out.push(*target);
        }
    });
    out
}

/// What the LEFT-hand side of an assignment is, beyond the expression itself.
///
/// Both fields answer a question the lvalue EXPRESSION cannot: which container the place
/// lives in, and — for a `fn(…)` field — which attribute of it. Both must be captured
/// before the right-hand side is parsed, because parsing an RHS overwrites the parser
/// state they come from (an `a.f = b.g` would otherwise leave `b`'s answers behind).
/// Carried as one value so the two cannot be threaded apart and disagree.
#[derive(Clone, Copy)]
pub(crate) struct AssignPlace<'a> {
    /// The type of what the place is read OUT of — `Reference(S)` for `s.f`, `&S` for the
    /// same write inside a `&`-parameter, `vector<τ>` for `v[i]`.
    pub parent_tp: &'a Type,
    /// `(struct def_nr, attribute index)` when the place is a `fn(…)` FIELD read. The
    /// authoritative answer is the field's byte offset, but that is `u16::MAX` on pass 1
    /// (the struct has no layout yet), and pass 1 is when a capturing source must be
    /// recorded for the attribute to get its split layout at all (loft#1072).
    pub fn_attr: Option<(u32, usize)>,
}

impl Parser {
    pub(crate) fn iter_text(
        &mut self,
        code: &mut Value,
        iter_var: u16,
        pre_var: Option<u16>,
    ) -> Value {
        // iter_var is {id}#next — the post-advance byte position (loop driver).
        // pre_var  is {id}#index — saved to the start position of the current char.
        let index_var = pre_var.unwrap();
        let res_var = self
            .vars
            .unique("for_result", &Type::Character, &mut self.lexer);
        let l = self.cl("OpLengthCharacter", &[Value::Var(res_var)]);
        let read = self.cl(
            // Plan-07 phase 4 step 4.8 — for-loop iteration over text uses
            // the *Nullable* peer of OpTextCharacter.  User-facing `text[i]`
            // (parser/fields.rs:750) keeps the raising OpTextCharacter.
            "OpTextCharacterNullable",
            &[code.clone(), Value::Var(iter_var)],
        );
        let advance = self.cl("OpAddInt", &[Value::Var(iter_var), l]);
        // loft#755 — a character read at an in-bounds position always spans at
        // least one byte, but `OpLengthCharacter` answers 0 for code point 0:
        // `character`'s null IS 0, so a NUL and "no character" share one value.
        // Termination is decided by the POSITION alone (`text_loop_break`), so
        // the only 0 reaching here is a real NUL — whose UTF-8 encoding is
        // exactly one byte.  Stepping over it keeps the walk moving; without
        // this the position stands still on a NUL and the loop never ends.
        let stalled = self.cl("OpLeInt", &[Value::Var(iter_var), Value::Var(index_var)]);
        let step_one = self.cl("OpAddInt", &[Value::Var(index_var), Value::Int(1)]);
        let next = vec![
            // Save current position as #index before advancing.
            v_set(index_var, Value::Var(iter_var)),
            v_set(res_var, read),
            v_set(iter_var, advance),
            v_if(stalled, v_set(iter_var, step_one), Value::Null),
            Value::Var(res_var),
        ];
        // Initialise the loop driver at the outer scope.
        // The caller must separately initialise index_var at the same scope level.
        *code = v_set(iter_var, Value::Int(0));
        v_block(next, Type::Character, "for text next")
    }

    /// The ONE home for "has text iteration passed the last character?" — the
    /// break steps every loop driven by [`iter_text`] pushes into its body.
    ///
    /// Text iteration cannot terminate on the yielded CHARACTER, the way it did
    /// before loft#755: `character`'s null is code point 0, so a NUL that the
    /// text really holds is the same value as the out-of-bounds read that ends
    /// the walk.  `text_from_bytes` builds such a text from valid UTF-8, and
    /// `len`, `size`, `byte_at`, `find` and slicing all read straight past the
    /// NUL — only the loop stopped there, silently dropping the rest.  So
    /// terminate on the POSITION instead, exactly as the vector arm yields
    /// `len(coll)` elements whatever the elements are.
    ///
    /// Two facts end the walk, and they are genuinely different:
    ///
    /// * the text is null — loft's null text IS the one-byte NUL string
    ///   (`STRING_NULL`), for which `size` still answers 1.  A null text holds
    ///   no characters, so it must yield none.
    /// * the saved position (`index_var`, where the character just read
    ///   started) has reached the byte length.
    ///
    /// `coll` is re-read each iteration rather than hoisted, matching the
    /// vector arm; pass the SAME expression the character read uses, so reader
    /// and bound cannot drift.
    pub(crate) fn text_loop_break(&mut self, coll: &Value, index_var: u16) -> Vec<Value> {
        let live = self.cl("OpConvBoolFromText", std::slice::from_ref(coll));
        let is_null = self.cl("OpNot", &[live]);
        let size = self.cl("OpSizeText", std::slice::from_ref(coll));
        let past_end = self.cl("OpLeInt", &[size, Value::Var(index_var)]);
        vec![is_null, past_end]
            .into_iter()
            .map(|test| {
                v_if(
                    test,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                )
            })
            .collect()
    }

    /// The loop-end test for iterating a VECTOR: its LENGTH, never the element's value.
    ///
    /// The sibling of [`text_loop_break`](Self::text_loop_break), and it exists for the
    /// same reason one construct over. Ending on a null ELEMENT works only while no
    /// element can legitimately be null-shaped, and two things break that: a null the
    /// vector really holds (it shares the out-of-bounds sentinel), and a `value struct`
    /// element, which @PLN101 deep-copies into a freshly minted record on bind — so the
    /// bound local is never null and the test can never fire (loft#1000). `map`, `filter`,
    /// `reduce`, `all`, `count_if` and the comprehension all hung forever on one, and
    /// `any` answered from a phantom element read one past the end.
    ///
    /// The `for` STATEMENT was cured this way first; these are the six lowerings that did
    /// not get the same treatment, and routing them through one function is what stops
    /// "how the loop ends" being seven independent decisions.
    ///
    /// The index is pre-incremented from `-1` by the `{#iter next}` block, so at the test
    /// it is the 0-based index just READ, and `len <= idx` is exactly "past the end".
    /// Re-read each iteration rather than hoisted, so an in-loop `#remove` — which shrinks
    /// the vector and steps the index back — still terminates. Forward-only: these
    /// builtins have no reverse form, so the `for` statement's `idx < 0` companion test
    /// has nothing to answer here.
    pub(crate) fn vector_loop_break(&mut self, coll: &Value, index_var: u16) -> Vec<Value> {
        let len = self.cl("OpLengthVector", std::slice::from_ref(coll));
        let past_end = self.cl("OpLeInt", &[len, Value::Var(index_var)]);
        vec![v_if(
            past_end,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        )]
    }

    #[allow(clippy::too_many_lines)] // sorted/index/spatial iterator setup — splitting would lose context
    /// The ONE home for a vector's per-element ITERATION stride — the byte
    /// step `vector::get_vector(size, idx)` walks per element.  Both the
    /// direct for-loop emission (the `Type::Vector` arm below) and the
    /// generic-instantiation elm-size fixup (`substitute_type_in_value`)
    /// read this; a second derivation is how the generic path silently
    /// drifted (it hand-summed struct field widths → 20 where the schema
    /// strides 4 for a linked struct, so iterating a bounded-generic
    /// method's `vector<Self>` result read garbage past element 0).
    ///
    /// The decision chain, in precedence order:
    /// - linked db type (structs holding vectors/text etc.) → 4-byte rec-id
    /// - narrow-int element → its forced 1/2/4-byte storage width
    /// - anything the shared element-type resolver knows
    ///   (`Data::vector_element_type`) → that schema type's own size: a fn-ref
    ///   is a 4-byte d_nr (@P343), a NESTED vector is a 4-byte handle, a plain
    ///   leaf is its scalar width
    /// - else → the schema's `database.size(db_tp)`
    ///
    /// The nested case used to read `element_stack_size(inner).max(4)` — the
    /// INNER scalar's width, so a `vector<vector<integer>>` strode 8 over rows
    /// that are 4-byte handles.  It agreed with the storage side only while the
    /// storage side made the same mistake; reading both from one derivation is
    /// what keeps reader and writer in step (loft#624 nested).
    pub(crate) fn vector_elem_iter_stride(&mut self, vtp: &Type) -> u16 {
        let vec_tp = self.data.type_def_nr(vtp);
        let db_tp = self.data.def(vec_tp).known_type();
        if self.database.is_linked(db_tp) {
            4
        } else if let Type::Integer(spec) = vtp
            && let Some(n) = spec.vector_narrow_width(false)
        {
            u16::from(n)
        } else if let Some(elem) = self.data.vector_element_type(vtp, &mut self.database) {
            self.database.size(elem)
        } else {
            self.database.size(db_tp)
        }
    }

    pub(crate) fn iterator(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        iter_var: u16,
        pre_var: Option<u16>,
    ) -> Value {
        // unwrap &vector<T> / &sorted<T> so the iterator setup
        // matches the underlying collection type.
        if let Type::RefVar(inner) = is_type {
            return self.iterator(code, inner, should, iter_var, pre_var);
        }
        if let Value::Iter(_, start, next, _) = code.clone() {
            if matches!(*next, Value::Block(_)) {
                *code = *start;
                return *next.clone();
            }
            diagnostic!(self.lexer, Level::Error, "Malformed iterator expression");
            return Value::Null;
        }
        if matches!(*is_type, Type::Text(_)) {
            return self.iter_text(code, iter_var, pre_var);
        }
        // CO1.5a: a coroutine handle needs a next()-based advance.
        //
        // The SUBJECT's own type decides this, not the shape it arrives in.  A collection
        // loop always arrives with its container type (`Type::Vector`, `Type::Sorted`, a
        // keyed type, …) — only `should`, the type the loop is being driven TOWARD, is the
        // generic iterator marker — so a subject that already IS `Type::Iterator` can only
        // be a generator handle.  Gating on a CALL instead left `g = steps(); for x in g`
        // falling through to the collection-cursor advance: nothing ran the generator, the
        // read never terminated, and the loop yielded `u16::MAX` forever (loft#841).
        if let Type::Iterator(inner, _) = is_type
            && !self.first_pass
        {
            // A handle already in a variable is used in place.  Copying it into a `__gen`
            // temp would hand a second owner to one coroutine's state store, and the loop's
            // scope would free it out from under the variable the caller still holds.
            let (gen_var, setup) = if let Value::Var(v) = code.unspan() {
                (*v, Value::Null)
            } else {
                let g = self.create_unique("__gen", is_type);
                self.vars.defined(g);
                (g, v_set(g, code.clone()))
            };
            *code = setup;
            let op = self.data.def_nr("OpCoroutineNext");
            let yield_tp = (**inner).clone();
            // @P327 / @P328 native — yield-channel dispatch via packed
            // `value_size` (low byte = byte size, high byte = channel
            // tag).  Tag 0 = legacy per-type channel (next_i64 /
            // next_text / next_dbref dispatched by byte size).  Tag 1 =
            // unified `next_into` with tuple-of-(integer|float) rebuild.
            // Tag 2 = unified `next_into` with fn-ref rebuild
            // (`(u32, DbRef)` from `[i64; 2]`).  Interp masks the high
            // byte off in `fill.rs::coroutine_next` and `state/codegen.rs`'s
            // `OpCoroutineNext` arm; only native inspects it.  See
            // `plans/16-coroutine-validation/01-unified-channel.md`.
            // @PLAN16 phase 02 — a tuple whose every element classifies into a
            // transport slot rides channel 1 (the layout-driven flatten-walk);
            // the per-slot kind codes ride as extra args so the native consumer
            // reconstructs the tuple.  `tuple_kinds` is the SAME decision the
            // producer's `is_tuple_into` makes, so the two ends agree.
            // #401 — one shared home for the channel decision (float/single/enum
            // need their own tags), and loft#1032 made it the home a monomorph
            // re-asks: see `coroutine_layout::next_operands`.
            let (value_size, kinds) = crate::coroutine_layout::next_operands(&yield_tp);
            let mut call_args = vec![Value::Var(gen_var), Value::Int(value_size)];
            call_args.extend(kinds.into_iter().map(Value::Int));
            return Value::Call(op, call_args);
        }
        if is_type == should {
            // Reached only on the FIRST pass, where the coroutine branch above is skipped:
            // a subject that IS an iterator is a coroutine handle, and pass 1 emits no code.
            let orig = code.clone();
            *code = Value::Null;
            return orig;
        }
        if self.first_pass {
            self.reverse_iterator = false;
            return Value::Null;
        }
        if let Type::Iterator(_, _) = should {
            match is_type {
                Type::Vector(vtp, dep) => {
                    let i = Value::Var(iter_var);
                    let vec_tp = self.data.type_def_nr(vtp);
                    let db_tp = self.data.def(vec_tp).known_type();
                    let size = self.vector_elem_iter_stride(vtp);
                    // Plan-07 phase 4 step 4.6 — for-loop iteration uses
                    // the *Nullable* peers; OOB returns a null DbRef which
                    // the loop's pre-body null-check
                    // (parser/collections.rs:1492-1499) converts to false
                    // and breaks.  User-facing `v[i]` (parser/fields.rs:629-640)
                    // keeps the raising OpGetVector / OpVectorRef.
                    let mut ref_expr = self.cl(
                        "OpGetVectorNullable",
                        &[code.clone(), Value::Int(i32::from(size)), i.clone()],
                    );
                    // @PLN25 E2 — a `__nullable<S>` element is `Enum(synth,true)`,
                    // not `Reference`, but in a LINKED collection (an array of
                    // ref slots — e.g. a struct-field vector that shares records
                    // with a sibling hash) it is stored as a 4-byte rec-ref, so
                    // the element read must DEREF via `OpVectorRefNullable` just
                    // like a `Reference` element does.  Without this it keeps the
                    // inline `OpGetVectorNullable(size=4)` and reads the rec-id
                    // slot AS the record → every field offset is junk.  An INLINE
                    // `vector<__nullable<S>>` (not linked) keeps the inline read.
                    let elem_ref_like = matches!(&**vtp, Type::Reference(_, _))
                        || matches!(&**vtp, Type::Enum(d, true, _)
                            if self.data.def(*d).name.starts_with("__nullable<"));
                    if elem_ref_like {
                        if self.database.is_linked(db_tp) {
                            ref_expr = self.cl("OpVectorRefNullable", &[code.clone(), i.clone()]);
                        }
                    } else if matches!(*vtp.clone(), Type::Tuple(_)) {
                        // P189b: vector-of-tuple — keep ref_expr as the
                        // raw `OpGetVector` DbRef.  The loop var is typed
                        // `Reference(__tuple<…>)` (see for_type) so codegen
                        // reads elements via `OpVarRef` + `OpGet*(offset)`
                        // per element type.  Without this skip,
                        // `get_val(Tuple, …)` falls through the field-
                        // type dispatch and errors with "Field access
                        // not supported on type tuple([…])".
                    } else if matches!(*vtp.clone(), Type::Function(_, _, _)) {
                        // @P343: vector-of-fn-ref element read.  Mirror the
                        // working index-apply path (parser/fields.rs:730-752)
                        // — vector elements store only the 4-byte d_nr, so
                        // assemble the stack fn-ref tuple from `OpGetInt4`
                        // (the d_nr) + `OpNullRefSentinel` (the closure half;
                        // vector fn-refs are non-capturing).  Routing through
                        // `get_val(Function, …)` instead (the struct-field
                        // path) reads a non-existent `__closure_rec` child via
                        // `OpRefFromChildRec(OpGetField(elem, 4, 0))` →
                        // garbage closure.  The block name `fn_ref_field_read`
                        // is reused so native codegen's tuple-emit shortcut
                        // (`((d_nr) as u32, closure_DbRef)`) fires here too.
                        let read_dnr = self.cl("OpGetInt4", &[ref_expr, Value::Int(0)]);
                        let read_clos = self.cl("OpNullRefSentinel", &[]);
                        ref_expr = crate::data::v_block(
                            vec![read_dnr, read_clos],
                            *vtp.clone(),
                            "fn_ref_field_read",
                        );
                    } else {
                        // route through `get_val` with the full
                        // element Type — preserves `IntegerSpec.forced_size`
                        // so narrow vectors dispatch to `OpGetShortRaw` /
                        // `OpGetByte` / `OpGetInt4` via the narrow_vec
                        // split.  Previously via `get_field(vec_tp, MAX)`
                        // which looked up `def(integer).returned` and lost
                        // the forced_size → emitted `OpGetInt` (8 bytes)
                        // into a 2-byte slot, producing off-bytes reads.
                        ref_expr = self.get_val(vtp, false, 0, ref_expr, u32::MAX);
                    }
                    let mut tp = *vtp.clone();
                    if matches!(tp, Type::Tuple(_)) {
                        // keep block type aligned with for_type's RefVar(Tuple)
                        // — the next-expression yields a 12-byte DbRef.
                        tp = Type::RefVar(Box::new(tp));
                    }
                    for d in dep {
                        tp = tp.depending(*d);
                    }
                    let reverse = self.reverse_iterator;
                    let step = if reverse {
                        // Decrement, but clamp at i32::MIN to prevent negative-index wrap.
                        // When iter reaches -1, set it to i32::MIN so GetVector returns null.
                        let decremented = self.op("Min", i.clone(), Value::Int(1), I32.clone());
                        let cond = self.op("Le", Value::Int(1), i.clone(), I32.clone());
                        v_block(
                            vec![Value::If(
                                Box::new(cond),
                                Box::new(decremented),
                                Box::new(Value::Int(i32::MIN)),
                            )],
                            I32.clone(),
                            "rev step",
                        )
                    } else {
                        self.op("Add", i.clone(), Value::Int(1), I32.clone())
                    };
                    // P189b: keep block type aligned with for_type's
                    // Reference(__tuple<…>) — the next-expression yields
                    // a 12-byte DbRef into vector storage.
                    let block_tp = if let Type::Tuple(ref elems) = *vtp.clone() {
                        let elems_clone = elems.clone();
                        let tuple_d = self.data.tuple_def(&mut self.lexer, &elems_clone);
                        Type::Reference(tuple_d, crate::data::Deps::none())
                    } else {
                        *vtp.clone()
                    };
                    let next =
                        v_block(vec![v_set(iter_var, step), ref_expr], block_tp, "iter next");
                    // The reverse bit belongs in `on` even though this loop steps its
                    // own counter: `e#remove` reads it to decide which way to rewind
                    // the cursor, and without it a `rev()` loop rewound FORWARD and
                    // skipped the next element (loft#903).
                    self.vars.set_loop(
                        if reverse { 64 } else { 0 },
                        self.data.def(vec_tp).known_type(),
                        code,
                    );
                    if reverse {
                        // Start at length; the first step gives len-1 (last element).
                        *code = v_set(
                            iter_var,
                            self.cl("OpLengthVector", std::slice::from_ref(code)),
                        );
                    } else {
                        *code = v_set(iter_var, Value::Int(-1));
                    }
                    self.reverse_iterator = false;
                    return next;
                }
                Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _) => {
                    // Derive element type for the block result annotation.
                    let elem_type = match is_type {
                        Type::Sorted(dnr, _, dep)
                        | Type::Index(dnr, _, dep)
                        | Type::Hash(dnr, _, dep)
                        | Type::Radix(dnr, _, dep)
                        | Type::Trie(dnr, _, dep) => Type::Reference(*dnr, dep.clone()),
                        _ => Type::Null,
                    };
                    // Create a separate Long variable to hold the packed i64 iterator
                    // state (cur << 32 | finish).  iter_var ({id}#index) remains I32
                    // as the user-visible sequential loop counter.
                    // The state var is named "{loop_name}#iter_state" so that iter_op()
                    // can find it by name when generating #remove.
                    let iter_base = self.vars.name(iter_var);
                    let iter_state_name = format!(
                        "{}#iter_state",
                        iter_base.strip_suffix("#index").unwrap_or(iter_base)
                    );
                    let state_var = self.create_var(&iter_state_name, &crate::data::I64);
                    self.vars.defined(state_var);
                    // Tell the loop which local its cursor is, so `#remove` reads it instead
                    // of rebuilding the name (loft#1272).
                    self.vars.set_loop_state_var(state_var);
                    let mut ls = Vec::new();
                    self.fill_iter(&mut ls, code, is_type, true, true);
                    ls.push(Value::Int(0));
                    ls.push(Value::Int(0));
                    let iter_expr = self.cl("OpIterate", &ls);
                    let mut ls = vec![Value::Var(state_var)];
                    self.fill_iter(&mut ls, code, is_type, false, true);
                    // Reset the reverse flag after both fill_iter calls so the second call
                    // also picks up the bit (fill_iter does not reset it itself).
                    self.reverse_iterator = false;
                    let next_expr = self.cl("OpStep", &ls);
                    let incr = self.op("Add", Value::Var(iter_var), Value::Int(1), I32.clone());
                    let iter_next = v_block(
                        vec![v_set(iter_var, incr), next_expr],
                        elem_type,
                        "sorted iter next",
                    );
                    // Use Insert (not v_block+Void) so that state_var and iter_var are
                    // claimed at the outer For-block scope and their stack slots persist
                    // for the duration of the loop.  A Void block would free them on exit.
                    *code = Value::Insert(vec![
                        v_set(state_var, iter_expr),
                        v_set(iter_var, Value::Int(-1)),
                    ]);
                    return iter_next;
                }
                _ => {
                    // @F32 — custom iterators via fn next(self) -> T?
                    // I13: custom iterator protocol — check for fn next(&T) -> Item?
                    let next_d_nr = self.data.find_fn(u16::MAX, "next", is_type);
                    if next_d_nr != u32::MAX {
                        // Store the iterable in a variable so .next() has a stable target.
                        let iter_obj_var = self.create_unique("__iter_obj", is_type);
                        self.vars.defined(iter_obj_var);
                        let obj_expr = code.clone();
                        *code = v_set(iter_obj_var, obj_expr);
                        // The "next" expression is a method call: iter_obj.next().
                        //
                        // The declared `self` is not the whole call.  A `next` answering a
                        // HEAP item — a struct, `text`, a collection, a struct-enum — also
                        // takes a hidden buffer the CALLER allocates, and `callback_call`
                        // fills those slots the way every ordinary call site does.  Built
                        // by hand the buffer was missing, so six of the ten item types a
                        // `next` can declare aborted the compiler outright: *"Too few
                        // parameters on t_7Counter_next (got 1, need 2)"* (loft#1310).
                        // Same class as loft#945 at the combinator callbacks and loft#1114
                        // at a lambda; routing through the one filler is what keeps the
                        // `for` spelling agreeing with the `while` + `.next()` one.
                        return self.callback_call(
                            next_d_nr,
                            vec![Value::Var(iter_obj_var)],
                            vec![is_type.clone()],
                        );
                    }
                    if self.first_pass {
                        return Value::Null;
                    }
                    // Plan-07 phase 6 (partial) — name the offending type
                    // and the iterables loft accepts, so the user knows
                    // what to substitute.  Old wording "Unknown iterator
                    // type T" left users guessing whether T was the issue
                    // or the syntax.
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot iterate over {}; expected vector, sorted, index, hash, text, or range",
                        is_type.name(&self.data)
                    );
                }
            }
        }
        Value::Null
    }

    /// Convert a type to another type when possible
    /// Returns false when impossible. However, the other way round might still be possible.
    pub(crate) fn towards_set_hash_remove(
        &mut self,
        to: &Value,
        val: &Value,
        op: &str,
        f_type: &Type,
    ) -> Option<Value> {
        if !self.first_pass && *val == Value::Null && op == "=" {
            // Partial-key lookup produces an iteration (Value::Iter), not a single record.
            // Assigning null to an iteration has no defined semantics — require all key fields.
            if matches!(to, Value::Iter(..)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot assign null to a partial-key lookup — \
                     provide all key fields to remove a single entry"
                );
                return Some(Value::Null);
            }
            if let Value::Call(get_nr, get_args) = to.unspan()
                && self.data.def(*get_nr).name() == "OpGetRecord"
                && let Some(Value::Int(db_tp_val)) = get_args.get(1)
                && (*db_tp_val as usize) < self.database.types.len()
                // `Ordered` belongs here as much as `Sorted` does, and leaving it
                // out was loft#719.  A `sorted<T[k]>` becomes an `ORDERED<T[k]>`
                // — the by-reference twin — as soon as anything else in the
                // program declares an `index<T[..]>` over the same element type,
                // so the same source line lowers differently depending on a
                // declaration somewhere else entirely.  With `Ordered` missing,
                // `coll[key] = null` fell through to a plain assignment and
                // generated `OpCopyRecord(cell, (), …)`: the interpreter dropped
                // the removal silently (the element was still there afterwards)
                // and `--native` failed to compile the void argument.
                //
                // `Radix` (a `spatial<T[x,y]>`) belongs here too, and its absence was
                // loft#720: `sp[x, y] = null` fell through to the same plain
                // `OpCopyRecord(cell, (), …)` that #719 produced — the interpreter
                // corrupted the store (it freed a record derived from a leftover key
                // value) and `--native` failed to compile the void argument.
                // `Stores::remove_owned` already unlinks a `Radix` element, so the
                // removal only ever needed routing to it.
                && matches!(
                    self.database.types[*db_tp_val as usize].parts,
                    Parts::Hash(_, _)
                        | Parts::Index(_, _, _)
                        | Parts::Sorted(_, _)
                        | Parts::Ordered(_, _)
                        | Parts::Radix(_, _)
                        | Parts::Trie(_, _)
                )
            {
                let db_tp = *db_tp_val;
                let get_args = get_args.clone();
                let get_rec = self.cl("OpGetRecord", &get_args);
                if let Some(group) = self.keyed_group_remove(&get_args[0], db_tp, &get_rec, f_type)
                {
                    return Some(group);
                }
                return Some(self.cl(
                    "OpHashRemove",
                    &[get_args[0].clone(), get_rec, Value::Int(db_tp)],
                ));
            }
        }
        None
    }

    /// `coll[key] = null` where `coll` is one member of a LINKED COLLECTION GROUP:
    /// the record leaves every member, and is freed exactly once (loft#900).
    /// `None` when `coll` is not a group member, which yields the single removal.
    ///
    /// Two or more collections over one element type in one struct are auto-linked
    /// into several routes to a SINGLE record set (loft#843). Removal had no owner
    /// for those records and was wrong in both directions: spelled through a VIEW it
    /// freed the record the primary still held (the vector kept the entry, its text
    /// read back null), spelled through the PRIMARY it never reached the views, which
    /// went on reporting a length over a record that was gone.
    ///
    /// **A removal spelled through any member removes it from the group.** Like
    /// loft#898's clear, that is not a choice made here — `h.view += [e]` has
    /// appended to every member since loft#843, so an operation spelled through a
    /// view acts on the group. The alternative, dropping one index entry and leaving
    /// the record in the primary, has no coherent successor: `h.by_k[1] = null`
    /// followed by `h.by_k[1] = E{k:1,…}` would remove one entry and then add to the
    /// whole group, leaving the primary holding two records under one key and nothing
    /// able to repair it.
    ///
    /// The ORDER is what makes it safe. Every unlink reads the record's key out of
    /// the record, so the free must come last and the record must stay reachable
    /// until then: the lookup runs ONCE into a temporary, every other member unlinks
    /// from it, and the member the removal was spelled through goes last and frees.
    /// The temporary also keeps the key expression evaluated once, which repeating
    /// the lookup per member would not (@PLN102 F2).
    fn keyed_group_remove(
        &mut self,
        coll: &Value,
        db_tp: i32,
        get_rec: &Value,
        f_type: &Type,
    ) -> Option<Value> {
        let (struct_tp, byte_off) = self.keyed_field_site(coll)?;
        let members = self.database.keyed_group_members(struct_tp, byte_off);
        if members.len() < 2 {
            return None;
        }
        // The record is a BORROW of the collection — `f_type` is the type the
        // element place already resolved to, deps included — so the temporary must
        // not be treated as owning a store and freed again at scope exit.
        let found = self.vars.work_refs(f_type, &mut self.lexer);
        self.change_var_type(found, f_type);
        self.vars.mark_inline_ref(found);
        // …and `inline_ref` is not what stops the free. A work-ref is freed at scope
        // exit unless it is marked `skip_free`, so the temp emitted an `OpFreeRef` on
        // the record the removal below had ALREADY freed: a double free, silent in a
        // release build and `Unknown record N` from `Store::valid` under debug
        // assertions.  The doc above says the record belongs to the collection and is
        // freed once; this is the flag that makes that true.
        self.vars.set_skip_free(found);
        let mut ops = vec![Value::Set(found, Box::new(get_rec.clone()))];
        ops.extend(self.group_sibling_unlinks(coll, byte_off, &members, &Value::Var(found)));
        ops.push(self.cl(
            "OpHashRemove",
            &[coll.clone(), Value::Var(found), Value::Int(db_tp)],
        ));
        Some(Value::Insert(ops))
    }

    /// `e#remove` where the iterated collection is one member of a LINKED
    /// COLLECTION GROUP: the element leaves every member, and is freed exactly
    /// once (loft#903).  Yields `remove` unchanged when it is not a group member.
    ///
    /// Same rule and same order as [`Self::keyed_group_remove`], which is the
    /// `coll[key] = null` spelling of the identical operation — every unlink reads
    /// the key out of the record, so each OTHER member unlinks first and the
    /// spelled member goes last and frees.
    ///
    /// What differs is where the RECORD comes from. A key lookup can be hoisted
    /// into a temporary; a loop cursor cannot, because `OpRemove` derives the
    /// element's reference internally from the iterator state. It does not have to
    /// be: the LOOP VARIABLE already is that reference, resolved once per
    /// iteration and at the record's payload start for every collection kind a
    /// group can contain (an `index` yields `new_ref(.., 8)`, an `ordered` and a
    /// linked `array` yield the record a slot names). An inline `vector`/`sorted`
    /// element is not a record and never joins a group, so it never reaches here.
    fn loop_group_remove(&mut self, coll: &Value, elem_var: u16, remove: Value) -> Value {
        let Some((struct_tp, byte_off)) = self.keyed_field_site(coll) else {
            return remove;
        };
        let members = self.database.keyed_group_members(struct_tp, byte_off);
        if members.len() < 2 {
            return remove;
        }
        let mut ops = self.group_sibling_unlinks(coll, byte_off, &members, &Value::Var(elem_var));
        ops.push(remove);
        Value::Insert(ops)
    }

    /// The unlinks that let a record LEAVE every other member of its linked group — one
    /// `OpHashRemove` with the [`crate::database::CLEAR_KEYED_VIEW`] bit per sibling, which
    /// unlinks and never frees, because the member the operation is spelled through (or the
    /// write that follows) owns what happens to the record.
    ///
    /// The one home for that loop (`@FR-Col-Group`: a record leaving through any member
    /// leaves every member).  Every removal spelling and every element-level write through
    /// the vector member emits it: `coll[key] = null`, `e#remove`, `v.remove(i)`, `v[i] = e`
    /// and `v[i] = null`.  Each unlink reads the record's KEY out of the record, so the caller
    /// runs these before anything changes or frees it.
    fn group_sibling_unlinks(
        &mut self,
        coll: &Value,
        byte_off: u16,
        members: &[(u16, u16, bool)],
        rec: &Value,
    ) -> Vec<Value> {
        let mut ops = Vec::with_capacity(members.len());
        for (off, coll_tp, _) in members {
            if *off == byte_off {
                continue;
            }
            let field = Self::keyed_field_at(coll, *off);
            let tp = i32::from(coll_tp | crate::database::CLEAR_KEYED_VIEW);
            ops.push(self.cl("OpHashRemove", &[field, rec.clone(), Value::Int(tp)]));
        }
        ops
    }

    /// An element place of a linked group's VECTOR member — `w.es[i]` as the
    /// `OpVectorRef(OpGetField(base, off), i)` it resolved to — with the facts every
    /// element-level write through it needs.  `None` when the vector is not a group member.
    fn vector_group_elem_site(&self, to: &Value) -> Option<GroupElemSite> {
        let Value::Call(nr, args) = to.unspan() else {
            return None;
        };
        if !matches!(
            self.data.def(*nr).name(),
            "OpVectorRef" | "OpVectorRefNullable"
        ) {
            return None;
        }
        let coll = args.first()?.unspan().clone();
        let (struct_tp, byte_off) = self.keyed_field_site(&coll)?;
        let members = self.database.keyed_group_members(struct_tp, byte_off);
        if members.len() < 2 {
            return None;
        }
        let Value::Call(_, gf_args) = &coll else {
            return None;
        };
        let base = gf_args.first()?.clone();
        Some(GroupElemSite {
            coll,
            base,
            struct_tp,
            byte_off,
            members,
        })
    }

    /// An element-level write through the vector member of a linked group, made to act on
    /// the group: the element is bound ONCE (its index evaluated once), unlinked from every
    /// keyed sibling, written by `build`, and — when `relink` — handed back to the siblings
    /// through `OpLinkRecord`, which indexes it under the key it now carries.
    ///
    /// `@FR-Col-Group` says a record entering through any member is in every member and a
    /// record leaving through any member leaves every member, by any write route.  The
    /// per-record chokepoint `Stores::record_finish` covers every route that ADDS a record;
    /// a write that replaces, nulls or removes an element the vector already holds reached
    /// no such point, so the views kept an entry under the old key: `w.es[0] = E{id:11}`
    /// left `by_id[11]` null and `by_id[7]` null with `len(by_id)` still 2, and
    /// `w.es.remove(0)` left the removed key findable and a re-add counted twice — silent,
    /// both backends, the same for a `vector<E?>` holder and one nesting level down.
    ///
    /// `build` receives the bound element (a `Var`) and the element place with its index
    /// hoisted, for a write that needs the index rather than the element (`remove`).
    /// `None` when `to` is not a group member's element, so the caller keeps its own form.
    pub(crate) fn group_elem_write(
        &mut self,
        to: &Value,
        elem_tp: &Type,
        relink: bool,
        build: impl FnOnce(&mut Self, Value, &Value) -> Value,
    ) -> Option<Vec<Value>> {
        let site = self.vector_group_elem_site(to)?;
        let (to, mut ops) = self.hoist_index_arg(to.clone());
        // The element is a BORROW of the vector, exactly as `keyed_group_remove`'s temporary
        // is a borrow of the collection: it must not be freed at scope exit.
        let found = self.vars.work_refs(elem_tp, &mut self.lexer);
        self.change_var_type(found, elem_tp);
        self.vars.mark_inline_ref(found);
        self.vars.set_skip_free(found);
        ops.push(Value::Set(found, Box::new(to.clone())));
        ops.extend(self.group_sibling_unlinks(
            &site.coll,
            site.byte_off,
            &site.members,
            &Value::Var(found),
        ));
        ops.push(build(self, Value::Var(found), &to));
        if relink && let Some(fld) = self.database.field_index_at(site.struct_tp, site.byte_off) {
            ops.push(self.cl(
                "OpLinkRecord",
                &[
                    site.base.clone(),
                    Value::Var(found),
                    Value::Int(i32::from(site.struct_tp)),
                    Value::Int(i32::from(fld)),
                ],
            ));
        }
        Some(ops)
    }

    /// The `(struct type, byte offset)` of the struct FIELD a collection expression
    /// names, or `None` when it names something else (a local, a parameter).
    ///
    /// Walks a nested `OpGetField` chain rather than reading the base VARIABLE's
    /// type, so a group one level down (`outer.inner.by_k`) resolves as well as a
    /// bare `x.by_k` — stopping at the bare form is what left loft#898's nested case
    /// on the unsafe path until its guard row a7 caught it.
    fn keyed_field_site(&self, coll: &Value) -> Option<(u16, u16)> {
        let Value::Call(gf_nr, gf_args) =
            crate::use_analysis::through_null_arm(&self.data, coll).unspan()
        else {
            return None;
        };
        if self.data.def(*gf_nr).name() != "OpGetField" {
            return None;
        }
        let Value::Int(byte_off) = gf_args.get(1)?.unspan() else {
            return None;
        };
        Some((self.holder_type(gf_args.first()?)?, *byte_off as u16))
    }

    /// The database type of the struct a field-access BASE evaluates to.
    fn holder_type(&self, base: &Value) -> Option<u16> {
        // An element of a `vector<S?>` is READ as `if <present> { payload } else { nullref }`
        // (loft#1367), so the walk meets an `If` where the schema names a record.  The read
        // answers as its present arm — `use_analysis::through_null_arm` is that question's one
        // home — and without the peel `rooms[0].items.remove(0)` resolved no holder, so the
        // group's sibling unlinks were never emitted and the removed record stayed findable
        // under its key.
        match crate::use_analysis::through_null_arm(&self.data, base).unspan() {
            Value::Var(v) => {
                let d_nr = match self.vars.tp(*v).base() {
                    Type::Reference(d, _) => *d,
                    Type::RefVar(inner) => match inner.base() {
                        Type::Reference(d, _) => *d,
                        _ => return None,
                    },
                    _ => return None,
                };
                let tp = self.data.def(d_nr).known_type();
                (tp != u16::MAX).then_some(tp)
            }
            Value::Call(nr, args) if self.data.def(*nr).name() == "OpGetField" => {
                // The read carries the field's own type id as its third operand, which names
                // what a NESTED base evaluates to without re-walking the schema — and without
                // mis-walking it: a field inside a `vector<S?>` element is reached through the
                // `Some` payload, two reads the schema does not list as fields of `S`.
                if let Some(Value::Int(tp)) = args.get(2).map(Value::unspan)
                    && *tp >= 0
                    && (*tp as u16) != u16::MAX
                {
                    return Some(*tp as u16);
                }
                let outer = self.holder_type(args.first()?)?;
                let Value::Int(off) = args.get(1)?.unspan() else {
                    return None;
                };
                self.database.field_content_at(outer, *off as u16)
            }
            // An ELEMENT of a vector — `rooms[0].items` — is a record of the vector's element
            // type; a `vector<S?>` element is the dense `S` its payload holds.
            Value::Call(nr, args)
                if matches!(
                    self.data.def(*nr).name(),
                    "OpVectorRef" | "OpVectorRefNullable" | "OpGetVector" | "OpGetVectorNullable"
                ) =>
            {
                let elem = self.vector_element_type(args.first()?)?;
                Some(self.database.key_owner(elem))
            }
            _ => None,
        }
    }

    /// The database type of the ELEMENT of a vector expression — a local, or a struct field
    /// reached through [`Self::holder_type`].
    fn vector_element_type(&self, vec: &Value) -> Option<u16> {
        match crate::use_analysis::through_null_arm(&self.data, vec).unspan() {
            Value::Var(v) => {
                let Type::Vector(inner, _) = self.vars.tp(*v).base() else {
                    return None;
                };
                let d = self.data.type_def_nr(inner.base());
                if d == u32::MAX {
                    return None;
                }
                let tp = self.data.def(d).known_type();
                (tp != u16::MAX).then_some(tp)
            }
            Value::Call(nr, args) if self.data.def(*nr).name() == "OpGetField" => {
                let vec_tp = self.holder_type(vec)?;
                let _ = (nr, args);
                let elem = self.database.content(vec_tp);
                (elem != u16::MAX).then_some(elem)
            }
            _ => None,
        }
    }

    /// `coll` — an `OpGetField(base, off)` read — re-aimed at the sibling field at
    /// byte offset `off`. Rebuilt by swapping the offset in the SAME call so the base
    /// expression and its variable stay whatever the original site resolved them to.
    fn keyed_field_at(coll: &Value, off: u16) -> Value {
        let mut out = coll.unspan().clone();
        if let Value::Call(_, args) = &mut out
            && let Some(slot) = args.get_mut(1)
        {
            *slot = Value::Int(i32::from(off));
        }
        out
    }

    pub(crate) fn towards_set(
        &mut self,
        to: &Value,
        val: &Value,
        f_type: &Type,
        src_tp: &Type,
        op: &str,
        lhs: &AssignPlace<'_>,
    ) -> Value {
        let AssignPlace {
            parent_tp,
            fn_attr: lhs_fn_attr,
        } = *lhs;
        if std::env::var("LOFT_PROBE_TS").is_ok() && !self.first_pass {
            eprintln!(
                "TS {}:{} op={op} f_type={f_type:?} src_tp={src_tp:?}\n   to={to:?}\n   val={val:?}",
                self.lexer.pos().file,
                self.lexer.pos().line
            );
        }
        // Intercept `h[key] = null` → remove the key from hash/index/sorted
        if let Some(result) = self.towards_set_hash_remove(to, val, op, f_type) {
            return result;
        }
        // @P305 — `coll[key] = value` insert-or-replace for a KEYED
        // collection.  The LHS parsed to `OpGetRecord(coll, db_tp, key…)`,
        // which returns the existing slot or null; the default reference
        // copy below (`OpCopyRecord(value, OpGetRecord(…))`) UPDATES an
        // existing key but silently NO-OPs when the key is absent (copy into
        // a null lookup), so it could never INSERT.  Route to `OpSetKeyed`,
        // which finds-or-inserts at runtime (dedup by `value`'s key),
        // uniformly for local / field / `&`-param collections.  (The
        // `coll[key] = null` removal is intercepted earlier in this fn.)
        //
        // @PLN25 E2 — gate-on, a keyed collection over a nullable element
        // (`hash<S[k]>` rewritten to `hash<__nullable<S>[k]>`) has element
        // type `Enum(__nullable<S>, true)` (the inline-nullable form, see
        // `index_type`), NOT `Reference(S)`.  Accept it too so the keyed-set
        // still routes to `OpSetKeyed`: `set_keyed` reads `value`'s key via
        // the SAME `key_owner`-resolved key descriptors the lookup uses, so
        // insert and lookup agree.  Without this the set falls through to the
        // update-only `OpCopyRecord`, which no-ops on the insert-miss and
        // leaves every lookup returning null.  Inert gate-off (no element
        // type is ever a `__nullable<` enum).
        let nullable_elem = matches!(
            f_type,
            Type::Enum(e, true, _) if self.data.def(*e).name.starts_with("__nullable<")
        );
        if op == "="
            && (matches!(f_type, Type::Reference(_, _)) || nullable_elem)
            && let Value::Call(get_nr, get_args) = to.unspan()
            && self.data.def(*get_nr).name() == "OpGetRecord"
            && let Some(Value::Int(db_tp)) = get_args.get(1)
            && (*db_tp as usize) < self.database.types.len()
            && matches!(
                self.database.types[*db_tp as usize].parts,
                Parts::Hash(_, _)
                    | Parts::Sorted(_, _)
                    // `Ordered` is the by-reference twin a `sorted<T[k]>` BECOMES as soon
                    // as anything else in the program declares an `index<T[..]>` over the
                    // same element type — so the same source line lowers differently
                    // because of a declaration somewhere else entirely.  Its absence here
                    // is loft#719's omission one function over: the removal arm
                    // (`towards_set_hash_remove`) lists it, the INSERT arm did not, so
                    // `s[k] = v` fell through to the update-only `OpCopyRecord` on a
                    // lookup that misses.  Every insert was silently dropped and the
                    // collection stayed empty — in a program that never used the struct
                    // whose second field caused the promotion.
                    | Parts::Ordered(_, _)
                    | Parts::Index(_, _, _)
                    | Parts::Radix(_, _)
                    | Parts::Trie(_, _)
            )
        {
            let db_tp = *db_tp;
            let coll = get_args[0].clone();
            // Multi-index guard: a keyed STRUCT FIELD cross-linked with a
            // sibling index (two+ keyed fields sharing an element type,
            // auto-linked in types.rs) can't be maintained by `OpSetKeyed`
            // (it lacks the struct + field context to update the siblings).
            // Detect it and fall through to the update-only `copy_ref` below
            // (which keeps the shared record consistent — no insert, no
            // corruption — matching the pre-@P305 behaviour for that case).
            let mut multi_index = false;
            if let Value::Call(gf_nr, gf_args) = coll.unspan()
                && self.data.def(*gf_nr).name() == "OpGetField"
                && let Some(Value::Int(byte_off)) = gf_args.get(1)
                && let Value::Var(sv) = gf_args[0].unspan()
            {
                let d_nr = match self.vars.tp(*sv) {
                    Type::Reference(d, _) => *d,
                    Type::RefVar(inner) => match &**inner {
                        Type::Reference(d, _) => *d,
                        _ => u32::MAX,
                    },
                    _ => u32::MAX,
                };
                if d_nr != u32::MAX {
                    let struct_tp = self.data.def(d_nr).known_type();
                    multi_index = self
                        .database
                        .keyed_field_is_linked(struct_tp, *byte_off as u16);
                }
            }
            if multi_index {
                // `coll[key] = value` on a multi-indexed field would desync
                // the sibling indexes (OpSetKeyed) or `copy_block` into a
                // null lookup on an insert-miss (copy_ref) — both corrupt.
                // Direct the user to `+= [value]`, which maintains every
                // sibling index via record_finish's `other_indexes` path.
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`coll[key] = value` is not supported on a multi-indexed \
                         field (a struct with two-or-more keyed fields sharing an \
                         element type); use `coll += [value]` instead"
                    );
                }
                return Value::Null;
            }
            // @P311: do NOT set the 0x8000 "free source" bit here.  Unlike the
            // field-assignment path (copy_ref → OpCopyRecord), `set_keyed`
            // already takes a deep copy of the value into the freshly-claimed
            // collection record, and the inline-literal/struct-call work-ref
            // that produced `val` still carries its own scope `OpFreeRef`.
            // Freeing it again from set_keyed is a double free: the work-ref's
            // store is released while still owned, then reused by the next
            // iteration's OpDatabase, corrupting the nested-vector backings of
            // every entry inserted after the first (silent data loss on the
            // interpreter, use-after-free SIGSEGV once the store is recycled).
            let tp_val = db_tp;
            // A keyed LOCAL declared `= null` owns no store, so the insert would follow the
            // null sentinel as a record number.  Build the empty collection first, which is
            // what `(N-Default)` says a write to a null collection does.
            let materialise = match coll.unspan() {
                Value::Var(v) => self.keyed_local_materialise(*v),
                // …and a TUPLE ELEMENT, which owns no store either and cannot be repointed by
                // `OpDatabase`: it is a slot inside the tuple, so the build goes through a
                // `__kvb_N` accumulator and a `TuplePut` (loft#1225).  The element type comes
                // off the tuple's own type rather than from `f_type`, which here names the
                // collection's ELEMENT and not the collection.
                Value::TupleGet(tuple_var, idx) => {
                    let (tuple_var, idx) = (*tuple_var, *idx);
                    match self.vars.tp(tuple_var).clone() {
                        Type::Tuple(elems) if (idx as usize) < elems.len() => {
                            let elem_tp = elems[idx as usize].clone();
                            if elem_tp.peel_optional().1 {
                                self.keyed_place_materialise(&coll, db_tp as u16, &elem_tp)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            let set = self.cl("OpSetKeyed", &[coll, val.clone(), Value::Int(tp_val)]);
            return match materialise {
                Some(guard) => Value::Insert(vec![guard, set]),
                None => set,
            };
        }
        // #328: a `reference<T>` field is a POINTER — assignment repoints
        // the field; it must never deep-copy record bytes into the current
        // referent (which both clobbered the referent and left `= null` a
        // no-op).  The discriminator is the lvalue SHAPE: `get_val` emits
        // `OpGetDbRef(host, pos)` only for pointer fields (the parse-time
        // share marker is rewritten to liveness deps by the expression
        // layer, so the TYPE cannot be tested here).  Closure-capture reads
        // also produce OpGetDbRef but on the hidden `__closure` param —
        // excluded so captured-Reference reassignment keeps its existing
        // copy semantics.
        //
        // `base()`, because a nullable pointer field arrives as
        // `Optional(Reference(…))` — the same peel the sibling arms below take.
        // `@FR-L-Null` gives `reference<T>?` the same 12-byte pointer bytes as
        // `reference<T>`, so a repoint is the same OpSetDbRef; unpeeled, the arm
        // fell through to `copy_ref` and deep-copied the source through the
        // field's CURRENT value, which for a terminator is the null sentinel —
        // an out-of-bounds store index on both backends (loft#1316).
        if matches!(f_type.base(), Type::Reference(_, _))
            && op == "="
            && let Value::Call(d, args) = to.unspan()
            && self.data.def(*d).name() == "OpGetDbRef"
            && args.len() == 2
            && !matches!(args[0].unspan(), Value::Var(v) if *v == self.closure_param)
        {
            let (r, p) = (args[0].clone(), args[1].clone());
            // `= null` clears the pointer: write the canonical null
            // sentinel DbRef (a bare `Value::Null` here makes codegen
            // allocate a typeless temp record that then leaks).
            let v = if matches!(val.unspan(), Value::Null) {
                self.cl("OpNullRefSentinel", &[])
            } else {
                val.clone()
            };
            return self.cl("OpSetDbRef", &[r, p, v]);
        }
        // @PLN25 E2a.5 — `lvalue = null` for a nullable inline struct field / vector element.
        // `build_nullable_set_null` carries the rationale and is shared with the CONSTRUCTION
        // path (`H { maybe: null }`), so the two spellings of "absent" cannot drift.
        if op == "="
            && matches!(val.unspan(), Value::Null)
            && let Type::Enum(syn, true, _) = f_type
            && self.data.def(*syn).name.starts_with("__nullable<")
            && !matches!(to, Value::Var(_))
        {
            let syn = *syn;
            if let Some(ops) = self.group_elem_write(to, f_type, false, |p, t, _| {
                p.build_nullable_set_null(syn, t)
            }) {
                return Value::Insert(ops);
            }
            return self.build_nullable_set_null(syn, to.clone());
        }
        // loft#1071 — the same for a USER struct-enum inline slot (`b.s = null` where
        // `s: Shape?`).  Its absence is not a discriminant but the record pointer `0`, so
        // write that word.  Shared with the CONSTRUCTION spelling (`Box { s: null }`) in
        // `handle_field` for the reason the arm above states: the two spellings of
        // "absent" must not drift, and here they are the same four-byte store.
        //
        // Without it the assignment lowered to a record COPY of the null — which, on a
        // slot whose test now reads the pointer, emitted nothing observable at all: the
        // field stayed present and `b.s = null` was silently a no-op.
        // `base()`, because a nullable field arrives as `Optional(Enum(…))` — the same
        // peel this whole family needs (loft#1065).
        if op == "="
            && self.is_null_source(val)
            && let Type::Enum(syn, true, _) = f_type.base()
            && !self.data.def(*syn).name.starts_with("__nullable<")
            && let Some((base, fld)) = self.inline_slot_word(to)
        {
            return self.cl("OpSetInt4", &[base, fld, Value::Int(0)]);
        }
        // @PLN25 E2 — `inline_nullable = <expression source>`: a nullable `__nullable<S>`
        // field / vector element assigned from an EXPRESSION rather than a literal.
        // `copy_ref` → `OpCopyRecord` would copy S's flat layout (`id@0,tag@8`) into
        // the packed `Some` element (`disc@0,tag@4,id@8`) — garbage on a present
        // source, and `allocation.rs:560` OOB on a null source (no store to read).
        // `emit_nullable_slot_write` is the shared home; the struct field and the tuple
        // member reach the same slot through it.  A literal `S{…}` already built `Some` in
        // `parse_var`, so its `src_tp` is the enum itself and `needs_nullable_wrap` — which
        // reads BOTH spellings of the source, `S` and `S?` — answers no for it.
        if op == "="
            && !self.first_pass
            && !matches!(to, Value::Var(_))
            && let Type::Enum(syn, true, _) = f_type.base()
            && self.needs_nullable_wrap(*syn, src_tp)
        {
            let syn = *syn;
            if let Some(ops) = self.group_elem_write(to, f_type.base(), true, |p, t, _| {
                let write = p.emit_nullable_slot_write(syn, &t, val.clone());
                v_block(write, Type::Void, "nullable_elem_convert")
            }) {
                return Value::Insert(ops);
            }
            let write = self.emit_nullable_slot_write(syn, to, val.clone());
            return v_block(write, Type::Void, "nullable_elem_convert");
        }
        // @PLN25 index flip — an element WRITE `v[i] = h` is an lvalue slot, not a nullable
        // read: under the flip `v[i]` types `Optional(Reference/Enum)`, but the slot itself
        // holds the base record, so a whole-element assign is still a `copy_ref` (OpCopyRecord).
        // Peel `.base()` so the Optional read-nullability marker doesn't drop it to the generic
        // op-name path (which errors "Cannot assign to attribute on OpGetVector"). Gate-OFF
        // inert (no Optional exists → `.base()` is identity → byte-identical).
        if matches!(
            f_type.base(),
            Type::Enum(_, true, _) | Type::Reference(_, _)
        ) && op == "="
            && (!matches!(to, Value::Var(_))
                // loft#1376 — a `&`-linked local NAMES a place.  `pi = &o.i` / `pe = &v[0]`
                // reach no `&` lowering (a struct projection is already a VIEW under
                // `@FR-B-View`), so the variable holds the field's or element's own `DbRef`
                // and `is_amp_link` is what records that the `&` was written.  A whole-value
                // write to it is therefore the same copy-INTO-the-place that writing `o.i`
                // directly is — `@FR-B-Ref-Write` at a heap τ REPLACES the source's contents.
                // Left to the bare-`Var` path it emitted a plain `Set`, which RE-POINTED the
                // variable while the place kept its value, on both backends and in silence;
                // an interior write through the same link (`pi.n = 7`) landed, which is what
                // made the shape look like it worked.  Only an `&` link qualifies: a plain
                // view is not marked, so `c = o.i; c = S{…}` is unchanged.
                //
                // The discriminator is the VALUE — the same thing native's own link arm
                // dispatches on, and it survives an IR snapshot where a per-statement flag
                // would not.  It has to be, because `pi = &o.j` and `pi = o.j` emit
                // IDENTICAL ops (@PLN130 F9: a `&` at a struct projection is invisible in
                // the IR), so a place READ cannot be told apart from a re-point and keeps
                // the binding meaning it has today.  What is unambiguous is a value that
                // PRODUCES a record — a literal or a call — since there is no place there to
                // link to; that is the one this rule claims.  Routed through the copy, the
                // BIND defined nothing and every later read reached codegen at slot 65535.
                || (matches!(to.unspan(), Value::Var(v) if self.vars.is_amp_link(*v))
                    && self.produces_whole_record(val)))
        {
            if std::env::var("LOFT_PROBE_TS").is_ok() {
                eprintln!("TS   -> copy_ref branch TAKEN");
            }
            if let Some(ops) = self.group_elem_write(to, f_type.base(), true, |p, t, _| {
                p.copy_ref(&t, val, f_type.base())
            }) {
                return Value::Insert(ops);
            }
            return self.copy_ref(to, val, f_type.base());
        }
        // loft#821 — `v[i] = t` on a `vector<(…)>`.  A tuple element is stored INLINE, so
        // the write is per-element at the element's own offsets, the same way a
        // tuple-typed struct field is written.  No arm matched it before: the LHS parsed
        // to the `tuple_unbox` read block (neither a `Call` nor a `Var`), so the dispatch
        // at the bottom of this function answered "Not implemented operation = for type
        // (float, float, float)" — for the one element type a vector could hold but not
        // have written into it.
        if op == "="
            && !self.first_pass
            && let Type::Tuple(elems) = f_type.base()
            && let Some(dest) = Self::stored_tuple_dest(to)
        {
            let elems = elems.clone();
            // Each element writes through its own copy of the address expression, so an
            // INDEX that does work would do it once per element — `v[bump(c)] = (1.0, 2.0,
            // 3.0)` called `bump` three times where `v[bump(c)] = 5` calls it once.  Hoist
            // it to a local so the three writes address one evaluation.  The container half
            // is left alone: it is a place read (a var or a field chain), pure address
            // arithmetic that costs nothing to repeat.
            let (dest, mut ops) = self.hoist_index_arg(dest);
            ops.extend(self.emit_tuple_set_ops(&dest, 0, &elems, val.clone()));
            return v_block(ops, Type::Void, "tuple_elem_index_set");
        }
        // loft#1072 — `h.f = inc` on a fn-typed struct FIELD, and `v[0] = inc` on a
        // fn-typed vector ELEMENT.
        //
        // Neither was recognised as writing anywhere.  A fn-ref READ is a Block (@PLN114's
        // split layout takes two reads to assemble the 20-byte pair), not the `Call`
        // getter or `Var` the dispatch at the bottom of this function knows, so both fell
        // through to *"Not implemented operation = for type function(…)"* — a message
        // about the `=` operator, contradicted by the same field accepting the same value
        // in a literal one line earlier.  `formal/closures.md`'s `L-Escape` says a closure
        // may be STORED in a struct field, and says nothing about the slot being fresh, so
        // this is the rule's storage half rather than a message to reword (D-clo-3).
        //
        // Each destination is handed to the writer the LITERAL already uses, so a literal
        // and an assignment into the same place cannot come to different conclusions about
        // what a fn-ref source may be:
        //
        //   * a struct field → `set_field`, which carries the whole contract — the
        //     capturing-lambda closure record, the `assigned_lambda_d_nr` bookkeeping that
        //     gives the attribute its split layout, the heterogeneous-capture diagnostic,
        //     the #318 frame-lifetime refusal, and the P215 deferral for a source that is
        //     not an inline literal (which is the diagnostic this case never reached);
        //   * a vector element → `fn_ref_slot_dnr`, the same four-byte projection the
        //     vector literal writes, behind the same #247 capture refusal.
        if op == "="
            && matches!(f_type.base(), Type::Function(_, _, _))
            && let Some((host, pos, split)) = self.fn_ref_place(to)
        {
            // The host may be named directly (`Reference`) or through a `&` parameter
            // (`RefVar(Reference)`) — `h.f = inc` inside `fn f(h: &Holder)` is the same
            // write, and reading only the first shape sent it to the four-byte path
            // below, which is wrong the moment the field is split.
            let host_def = match parent_tp.base() {
                Type::Reference(d, _) => Some(*d),
                Type::RefVar(inner) => match inner.base() {
                    Type::Reference(d, _) => Some(*d),
                    _ => None,
                },
                _ => None,
            };
            if let Some(d_nr) = host_def {
                let Value::Int(offset) = pos.unspan() else {
                    return Value::Null;
                };
                let offset = *offset;
                // The layout answers on pass 2 and is authoritative there; on pass 1 the
                // struct has no layout, every field offset reads `u16::MAX`, and the read
                // site's own record of the attribute is the only answer there is.  Pass 1
                // is not optional here: a capturing source must be RECORDED against the
                // attribute in that pass for `fill_database` to give it the split layout,
                // and a pass-2-only recording emits a write into a `__closure_rec` half
                // that was never registered.
                let f_nr = self
                    .fn_ref_attr_at(d_nr, offset)
                    .or_else(|| lhs_fn_attr.filter(|(md, _)| *md == d_nr).map(|(_, f)| f));
                if let Some(f_nr) = f_nr {
                    // A SPLIT field already owns a closure record, and the write is about
                    // to replace the d_nr that record belongs to.  Release it first,
                    // whatever the new source is:
                    //
                    //   * a capturing source claims a FRESH child record and overwrites
                    //     the pointer, so without this the old record is orphaned in the
                    //     host's store — an unbounded leak when the assignment runs in a
                    //     loop;
                    //   * a NON-capturing source writes only the d_nr, so without this the
                    //     field reads back as that function paired with the PREVIOUS
                    //     closure — and `fn_call_ref` pushes a non-null closure as the
                    //     hidden argument, so the callee is entered with an argument it
                    //     does not declare. That is not a stale value but a corrupt frame:
                    //     measured, the call returned with the stack misaligned and the
                    //     NEXT read of an unrelated field (`h.tag`) faulted in `get_int`.
                    //
                    // `OpClearKeyed` against the `child_rec<…>` field frees the record and
                    // zeroes the pointer in one step — the same `remove_claims` cascade
                    // that frees it when the host dies. Emitted only here, on the
                    // REASSIGNMENT path: an initialising write (a struct literal) has a
                    // freshly claimed record whose pointer is already zero, so the
                    // literal's emit stays exactly what it was.
                    let mut ops = Vec::new();
                    if split
                        && let Ok(crec_off) = u16::try_from(offset + 4)
                        && let Some(crec_tp) = self
                            .database
                            .field_content_at(self.data.def(d_nr).known_type(), crec_off)
                    {
                        let tp_val = Value::Int(i32::from(crec_tp));
                        let field = self.cl(
                            "OpGetField",
                            &[host.clone(), Value::Int(offset + 4), tp_val.clone()],
                        );
                        ops.push(self.cl("OpClearKeyed", &[field, tp_val]));
                    }
                    let write = self.set_field(d_nr, f_nr, 0, host, val.clone());
                    if ops.is_empty() {
                        return write;
                    }
                    ops.push(write);
                    return v_block(ops, Type::Void, "fn_ref_field_reset");
                }
            }
            // Everything below writes FOUR BYTES and nothing else, so a destination that
            // has a closure half must never reach it: the field would read back as the
            // new function paired with the PREVIOUS closure, and `fn_call_ref` enters a
            // callee that declares no closure with one pushed as its hidden argument.
            // Reaching here with `split` means the attribute could not be resolved, which
            // is a gap in this dispatch rather than a program to run.
            if split {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot assign into this fn-ref field — the attribute it belongs to \
                         could not be resolved from the assignment target, and its closure \
                         half cannot be released without it; assign through a named holder, \
                         or rebuild the value (`h = Holder {{ f: …, … }}`)"
                    );
                }
                return Value::Null;
            }
            // A vector element (and any other four-byte fn-ref slot) has no
            // `__closure_rec` half to receive a captured environment: the same
            // refusal the collection LITERAL gives, from its one home.
            if !self.first_pass && self.fn_ref_source_captures(val) {
                self.refuse_capturing_closure_in_collection();
                return Value::Null;
            }
            let (dnr_val, mut ops) = self.fn_ref_slot_dnr(val, f_type);
            let write = self.cl("OpSetInt4", &[host, pos, dnr_val]);
            ops.push(write);
            return v_block(ops, Type::Void, "fn_ref_slot_set");
        }
        if crate::parser::vectors::is_collection(f_type) {
            if let Value::Var(nr) = to.unspan() {
                // The const guard is `parse_assign_op_inner`'s, asked before it routed here.
                return v_set(*nr, val.clone());
            }
            // @P308 — a KEYED-collection FIELD whole-assignment `s.h = expr`
            // (hash/sorted/index, RHS an expression) must be DEEP-COPIED into
            // the field; before this it fell to the bare `return val.clone()`
            // below → silent no-op (empty field).  `OpReplaceKeyed(val, to,
            // kt)` against the field ref = remove_claims(field) +
            // copy_claims(val, field); `0x8000` frees a fresh-storage call
            // source.  (Empty `s.h = []` → OpClearKeyed and `s.h[k]=v` →
            // OpSetKeyed are handled earlier; this is the remaining
            // whole-value case.)  Vector/Radix keep the bare return —
            // vector field-replace lives in parse_assign_op, and Radix's
            // copy_claims is unimplemented (per @P295), so `keyed_field_kt`
            // returns None for both.
            if op == "="
                && !matches!(val, Value::Insert(_) | Value::Null)
                && let Some(kt) = self.keyed_field_kt(f_type)
            {
                #[cfg(not(feature = "wasm"))]
                let tp_val = if self.is_struct_returning_call(val) {
                    i32::from(kt) | 0x8000
                } else {
                    i32::from(kt)
                };
                #[cfg(feature = "wasm")]
                let tp_val = i32::from(kt);
                // loft#1159 — ask what the SOURCE is, which the keyed-LOCAL site beside this
                // one has always asked (`is_keyed(&s_type)` in `parse_assign_op`) and this
                // one did not.  `OpReplaceKeyed` hands the source to `copy_claims` under the
                // DESTINATION's type, so a plain `vector<E>` was walked as if it were a hash
                // / index / trie: `hash` found nothing (0), `index`, `trie` and `spatial`
                // found one node, and only `sorted` survived — because a sorted's own
                // storage IS a sequential vector.  A length that disagrees with its own
                // lookups is the state that leaves behind, and nothing said so.
                //
                // The records are the same records either way, so the answer is not a
                // refusal: `h.a = [E{…}, E{…}]` is the documented spelling and it inserts
                // each record by key.  A vector VALUE now reaches those same inserts.  The
                // clear comes first for the reason `=` always clears — it replaces the
                // collection rather than adding to it — and it is the group-aware clear, so
                // a member's siblings are reset with it (loft#898).
                if !crate::parser::vectors::is_keyed(src_tp) {
                    let mut ops = self.keyed_group_clear(to, kt, parent_tp);
                    let (parent, parent_tp_id, field_nr) = self.fill_keyed_site(to, parent_tp, kt);
                    ops.push(self.cl(
                        "OpFillKeyed",
                        &[
                            parent,
                            val.clone(),
                            Value::Int(tp_val),
                            Value::Int(i32::from(parent_tp_id)),
                            Value::Int(i32::from(field_nr)),
                        ],
                    ));
                    return Value::Insert(ops);
                }
                return self.cl(
                    "OpReplaceKeyed",
                    &[val.clone(), to.clone(), Value::Int(tp_val)],
                );
            }
            // LHS is a field access (e.g. `s.v = fresh`).  Pre-fix this
            // returned bare `val` and the assignment was silently discarded.
            // The full clear-then-append pair lives in parse_assign_op where
            // the RHS type is in scope (so we can avoid emitting OpAppendVector
            // when the RHS is not actually a vector — e.g. `b.data = f#read(...)`
            // where f#read returns text — which would mismatch types in
            // codegen).  Empty literal `[]` is also handled there.
            return val.clone();
        }
        // A right-hand side that has ALREADY written the target leaves nothing to assign, and
        // wrapping it in a `Set` would build a second, empty collection over the top of it.
        // That is what a `&`-vector `=` looks like by the time it arrives: `assign_refvar_vector`
        // lowers the write into ops that fill the target in place, and the shapes it declines —
        // a bracket literal, a comprehension — carry their own appends in an `Insert` / `Block`.
        //
        // loft#1292 — the condition used to name the FORMER (`Vector | Sorted`) instead of the
        // fact.  A `sorted` has no such lowering, so its right-hand side is a bare VALUE and
        // returning it alone DROPPED the write: `fn f(x: &sorted<E[k]>) { x = mks(); }` left the
        // caller's collection untouched and leaked the one the callee minted.  The refusal
        // beside it (*"has & but is never modified"*) was the only thing stopping that, since
        // the write it could not see is the write that never happened.  `&hash` and `&index`
        // were correct all along, which is what said the difference had to be a site naming one
        // kind rather than anything about keyed collections.
        //
        // loft#1303 — and `Sorted` had to leave the list for the fact to be named ONCE rather
        // than twice.  `assign_refvar_vector` is the only lowering that fills the target in
        // place, and it fires for `Vector` alone, so a `Vector` target is what "the right-hand
        // side has already written the target" means.  A keyed target's block is
        // `assign_refvar_keyed`'s materialisation, which fills a fresh WORK-REF that the `Set`
        // still has to install; matching it here dropped the write and re-raised the refusal
        // the line above describes.
        if let Type::RefVar(tp) = f_type
            && matches!(**tp, Type::Vector(_, _))
            && matches!(val.unspan(), Value::Insert(_) | Value::Block(_))
        {
            if let Value::Var(nr) = to.unspan() {
                if self.vars.uses(*nr) > 0 {
                    return val.clone();
                }
            } else {
                return val.clone();
            }
        }
        // @PLN17: boolean element/field writes now go through the generic
        // call_to_set_op path (OpGetBoolean -> OpSetBoolean, with try_swap on the
        // inner OpGetVector) — identical to how plain enums are handled.  The old
        // special-case here destructured the two-level OpEqInt(OpGetByte(…)) read
        // shape and is obsolete (it mis-read the single-level OpGetBoolean shape).
        let mut code = self.compute_op_code(op, to, val, f_type);
        // loft#1009 — a COMPOUND assignment into a bounded integer slot had no range check
        // of any kind, so `l: u8 = 250; l += 10;` answered 260 and `b: u8 = 5; b -= 10;`
        // answered -5.  The written-out form (`l = l + 10`) is refused at compile time, so
        // the compile-time check cannot close it either: at the store site `code` is the
        // OPERAND (`10`), which fits `u8` — only the COMPOSED value, which is what `code`
        // holds right here, can be judged, and only at run time.
        //
        // This is the one seam every compound assignment passes through, whatever its
        // target: `to` is still a `Var`, a field read or an element read at this point, and
        // the store-op dispatch below has not happened yet.  So the guard goes on the
        // composed value ONCE, and the local, the field and the element cannot disagree.
        //
        // It used to be Var-only, on the reasoning that a field reaches "the store's own
        // guard" — true, but only for the 1- and 2-byte widths: `set_byte` / `set_short`
        // carry a `min` operand and substitute the range's low end, while the 4-byte
        // setters take no range at all and simply truncate.  So a `u32` field wrapped to
        // 2^32-5 where the `u32` LOCAL beside it clamped to 0 (loft#1031), and an `i32`
        // field wrapped likewise.  Guarding here instead of teaching four more opcodes a
        // range keeps one rule in one place; for the 1- and 2-byte widths the store guard
        // becomes a backstop that this path can no longer trip, since the value reaching
        // it is already in range — clamping is idempotent, and `set_byte`'s out-of-range
        // return is discarded, so nothing is judged or reported twice.
        if op != "=" && !self.first_pass {
            let holds_null = crate::parser::expressions::target_holds_null(f_type, parent_tp);
            self.guard_compound_range(&mut code, f_type, holds_null);
        }
        if let Value::Call(d_nr, args) = to.unspan() {
            let name = self.data.def(*d_nr).name().to_string();
            let args = args.clone();
            self.call_to_set_op(&name, &args, code, op)
        } else if let Some(tuple_lhs) = crate::parser::expressions::extract_nested_tuple_lhs(to) {
            // loft#1228 — a TUPLE ELEMENT is a place, and this is the seam where a place
            // becomes a write.  A `Call` gets its `OpGetX` -> `OpSetX` twin above and a `Var`
            // gets a `Set`; a tuple slot had neither, so every compound assignment to one fell
            // to the diagnostic below — *"Not implemented operation + for type integer"*, a
            // message about the OPERATOR when `+` on an integer is plainly implemented and the
            // target is what had no route.
            //
            // `code` is already the COMPOSED value here (the comment on the range guard above
            // says so), so the write is the only missing half.  Both IR spellings of a tuple
            // place are handled, through the one home that knows them — a bare `TupleGet` at
            // depth 1 and a `Block[Set(w, …), TupleGet(w, idx)]` chain deeper — because
            // matching only the first is the half-fix QUALITY.md's `spellings` screen exists
            // to catch.
            crate::parser::expressions::build_nested_tuple_assign(to, &tuple_lhs, code)
        } else if let Value::Var(nr) = to.unspan() {
            // The const guard is `parse_assign_op_inner`'s, asked before it routed here.
            // This variable was created here and thus not yet used.
            self.var_usages(*nr, false);
            v_set(*nr, code)
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Not implemented operation {op} for type {}",
                    f_type.show(&self.data, &self.vars)
                );
            }
            Value::Null
        }
    }

    /// Compute the RHS value after applying `op` to `to` and `val`.
    /// `x#break` / `x#continue` named something that is not an enclosing loop's variable
    /// — report it, and answer a poisoned value (loft#998).
    ///
    /// Reported HERE, at the parse, because this is where the source position and the
    /// author's spelling are. `Scopes::scan` sees only a level, has no position to point
    /// at, and until this existed simply indexed with it — `k#break` on a declared local
    /// was *"internal compiler error … index is 18446744073709551615"*, which names
    /// neither the mistake nor a cure.
    ///
    /// Names what CAN be written: the enclosing loops' variables. That list is the whole
    /// answer to "then what may I say", and the author already has one of them on screen.
    fn not_a_loop_variable(&mut self, name: &str, verb: &str) -> Value {
        // Outside a loop entirely, "cannot break outside a loop" is the right and only
        // message — the caller has already said it, and adding "…and `k` is not a loop
        // variable" says the same thing twice from further away.
        if !self.first_pass && self.in_loop {
            let open = self.vars.enclosing_loop_names();
            let cure = if open.is_empty() {
                format!("a plain `{verb}`")
            } else {
                let names = open
                    .iter()
                    .map(|n| format!("`{n}#{verb}`"))
                    .collect::<Vec<_>>()
                    .join(" or ");
                format!("a plain `{verb}` for the innermost loop, or {names}")
            };
            diagnostic!(
                self.lexer,
                Level::Error,
                "`{name}` is not a loop variable — `{name}#{verb}` names the loop to \
                 {verb} by the variable that loop binds, and no enclosing loop binds \
                 `{name}`; write {cure}"
            );
        }
        Value::Null
    }

    pub(crate) fn iter_op_count_or_first(
        &mut self,
        code: &mut Value,
        name: &str,
        t: &mut Type,
        is_first: bool,
        loop_var: u16,
    ) {
        // Named after the BINDING `name` denotes, not after the spelling — see `iter_op`
        // (loft#915).  `loop_var` is that binding: loft#794 already routes the counter to
        // the loop `name` iterates rather than the loop being parsed.
        let base = if loop_var == u16::MAX {
            name.to_string()
        } else {
            self.vars.name(loop_var).to_string()
        };
        let count_var = format!("{base}#count");
        let count = if self.vars.name_exists(&count_var) {
            self.vars.var(&count_var)
        } else {
            self.create_var(&count_var, &I32)
        };
        // loft#794 — register the counter on the loop `name` iterates, which is
        // not necessarily the loop being parsed: `for p { for q { p#count } }`
        // reads the OUTER attribute from inside the INNER body.
        self.vars.loop_count_of(loop_var, count);
        self.vars.defined(count);
        if is_first {
            *code = self.cl("OpEqInt", &[Value::Var(count), Value::Int(0)]);
            *t = Type::Boolean;
        } else {
            *code = Value::Var(count);
            *t = I32.clone();
        }
    }

    #[allow(clippy::too_many_lines)] // iterator operation dispatch — splitting would lose context
    pub(crate) fn iter_op(&mut self, code: &mut Value, name: &str, t: &mut Type, index_var: u16) {
        // File variables handle their own # operations before iterator operations.
        if self.is_file_var(index_var) {
            self.file_op(code, t, index_var);
            return;
        }
        // A loop's companion locals (`#index`, `#next`, `#iter_state`, `#count`) are named
        // after the BINDING, not after the spelling that reached them: two `for i` loops in
        // one function are two bindings (`i`, `i#1`) and each carries its own set
        // (loft#915).  `index_var` is what `name` resolves to right here, so its own name
        // is the base — the same base the loop used when it created them.  `name` stays the
        // source spelling, which is what a diagnostic must quote.
        let base = if index_var == u16::MAX {
            name.to_string()
        } else {
            self.vars.name(index_var).to_string()
        };
        let base = base.as_str();
        // detect #fields for compile-time field iteration.
        if self.lexer.has_keyword("fields") {
            let var = self.vars.var(name);
            let var_type = if var == u16::MAX {
                Type::Unknown(0)
            } else {
                self.vars.tp(var).clone()
            };
            if let Type::Reference(d, _) = &var_type {
                self.fields_of = *d;
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#fields requires a struct variable, got {}",
                    var_type.name(&self.data)
                );
            }
            // Set code to the source variable so parse_field_iteration receives it.
            if var != u16::MAX {
                *code = Value::Var(var);
            }
            *t = Type::Void;
            return;
        }
        if self.lexer.has_keyword("index") {
            // For index<T> collections, {name}#index holds an internal B-tree record number,
            // not a sequential 0-based counter.  Reject it at compile time.
            if self.vars.loop_on(index_var) & 63 == 1 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#index is not supported on index<T> collections \
(it holds an internal record number, not a sequential counter); \
use #count instead"
                );
                *t = Type::Unknown(0);
            } else {
                let i_name = &format!("{base}#index");
                if self.vars.name_exists(i_name) {
                    let v = self.vars.var(i_name);
                    *t = self.vars.tp(v).clone();
                    *code = Value::Var(v);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect #index variable on {}",
                        name
                    );
                    *t = Type::Unknown(0);
                }
            }
        } else if self.lexer.has_keyword("next") {
            let n_name = format!("{base}#next");
            if self.vars.name_exists(&n_name) {
                let v = self.vars.var(&n_name);
                *t = self.vars.tp(v).clone();
                *code = Value::Var(v);
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Incorrect #next variable on {} (only valid in text loops)",
                    name
                );
                *t = Type::Unknown(0);
            }
        } else if self.lexer.has_token("break") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            // loft#998 — `x#break` names the loop to leave by its VARIABLE, so a name that
            // is not one has no level to jump. `Value::Null` rather than a level: every
            // level is a real jump, and the one that used to be handed over (the chain
            // length, from a walk with no not-found answer) underflowed `Scopes::scan`'s
            // `loops.len() - lv - 1` into an internal compiler error.
            *code = match self.vars.loop_nr(name) {
                Some(lv) => Value::Break(lv),
                None => self.not_a_loop_variable(name, "break"),
            };
            *t = Type::Void;
        } else if self.lexer.has_token("continue") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *code = match self.vars.loop_nr(name) {
                Some(lv) => Value::Continue(lv),
                None => self.not_a_loop_variable(name, "continue"),
            };
            *t = Type::Void;
        } else if self.lexer.has_keyword("count") {
            self.iter_op_count_or_first(code, name, t, false, index_var);
        } else if self.lexer.has_keyword("first") {
            self.iter_op_count_or_first(code, name, t, true, index_var);
        } else if self.lexer.has_keyword("remove") {
            // CO1.5c: #remove on generator iterators is already rejected by the
            // loop_value == Null check below — coroutine for-loops never call set_loop.
            if !self.first_pass && *self.vars.loop_value(index_var) == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}#remove' is only valid on a loop iteration variable (e.g. 'for {} in collection {{ {}#remove }}')",
                    name,
                    name,
                    name
                );
                *t = Type::Void;
                return;
            }
            // C60 Step 9: reject #remove on a SNAPSHOT walk.  The parser substitutes such
            // iteration with a scratch rec-nr vector (see parse_for, the
            // `{id}#hash_scratch` variable), so #remove would remove from the snapshot and
            // not from the collection — silently diverging from what was written.
            //
            // Three kinds take that substitution — `hash`, `trie` and `spatial`
            // (`Type::Hash | Type::Trie | Type::Radix` at the scratch's creation) — and they
            // share the one scratch NAME, so a message spelled for the hash told a `trie`
            // author their loop was "hash iteration" and prescribed `hash[key] = null` for a
            // collection they never wrote.  The kind is recovered from the scratch's own
            // deps, which name the source collection; where it cannot be
            // (a field, a call result — nothing to name), the wording stays kind-neutral
            // rather than guessing, and the cure is right either way.
            if !self.first_pass {
                let coll = self.vars.loop_coll_var(index_var);
                if coll != u16::MAX && self.vars.name(coll).contains("hash_scratch") {
                    // `.base()` because a snapshot-walked collection can be declared
                    // NULLABLE (`h: hash<E[k]>?`), and `Optional` is a marker over the same
                    // storage — the kind a message names is the kind that was written, with
                    // or without the `?`.
                    fn snapshot_kind(tp: &Type) -> Option<&'static str> {
                        match tp.base() {
                            Type::Hash(_, _, _) => Some("hash"),
                            Type::Trie(_, _, _) => Some("trie"),
                            Type::Radix(_, _, _) => Some("spatial"),
                            _ => None,
                        }
                    }
                    let source = self
                        .vars
                        .tp(coll)
                        .depend()
                        .first()
                        .map(|d| self.vars.tp(*d).clone());
                    let kind = match &source {
                        // A local: the dep names it and its type IS the collection.
                        Some(tp) if snapshot_kind(tp).is_some() => snapshot_kind(tp),
                        // A FIELD (`for e in b.data`): the dep names the STRUCT, so the kind
                        // is the one snapshot-walked field it declares.  Named only when
                        // there is exactly one — with two the loop's own field is not
                        // decidable from here, and a guess in a refusal is worse than the
                        // kind-neutral wording below.
                        Some(Type::Reference(d, _)) => {
                            let mut found = self
                                .data
                                .def(*d)
                                .attributes
                                .iter()
                                .filter_map(|a| snapshot_kind(&a.typedef));
                            match (found.next(), found.next()) {
                                (Some(k), None) => Some(k),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    let what =
                        kind.map_or_else(|| "this collection".to_string(), |k| format!("a `{k}`"));
                    // A `spatial` is keyed by its 1-3 coordinate AXES, so `[key]` is not a
                    // spelling its author can copy; the other two are keyed by one value.
                    let cure = match kind {
                        Some("spatial") => "spatial[x, y]",
                        Some(k) => {
                            if k == "trie" {
                                "trie[key]"
                            } else {
                                "hash[key]"
                            }
                        }
                        None => "collection[key]",
                    };
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "#remove is not supported when iterating {what} — the loop walks a \
                         snapshot of the records, so the removal would not reach the \
                         collection; remove by key instead (`{} = null`)",
                        cure
                    );
                    *t = Type::Void;
                    return;
                }
            }
            let on = self.vars.loop_on(index_var);
            // The loop records its own cursor, because the two keyed lowerings name it
            // differently — `{base}#iter_state` for an unbounded walk, `_iter_N` for a
            // bounded range — and reconstructing the first spelling here silently missed the
            // second. The fallback then named `{base}#index`, which a range ELIDES, so the
            // operand was measured against a slot that does not exist (loft#1272).
            let recorded = self.vars.loop_state_var(index_var);
            let state_var = if recorded == u16::MAX {
                // No cursor was recorded: a vector walk, a range, a custom iterator. Fall
                // back to the historical name-based lookup, which those shapes still satisfy.
                let state_name = if on & 63 >= 1 && on & 63 <= 3 {
                    let state_key = format!("{base}#iter_state");
                    if self.vars.name_exists(&state_key) {
                        state_key
                    } else {
                        format!("{base}#index")
                    }
                } else {
                    format!("{base}#index")
                };
                self.vars.var(&state_name)
            } else {
                recorded
            };
            let coll = self.vars.loop_value(index_var).clone();
            let remove = self.cl(
                "OpRemove",
                &[
                    Value::Var(state_var),
                    coll.clone(),
                    Value::Int(i32::from(on)),
                    Value::Int(i32::from(self.vars.loop_db_tp(index_var))),
                ],
            );
            *code = self.loop_group_remove(&coll, index_var, remove);
            *t = Type::Void;
        } else if self.lexer.has_keyword("lock") {
            // d#lock — read the lock state of the store containing a reference or vector variable.
            // Assignment d#lock = true/false is resolved in towards_set.
            if !self.first_pass
                && !matches!(
                    self.vars.tp(index_var),
                    Type::Reference(_, _) | Type::Vector(_, _)
                )
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "#lock is only valid on reference or vector variables, not on '{}'",
                    name
                );
                *t = Type::Unknown(0);
            } else {
                *code = self.cl("n_get_store_lock", &[Value::Var(index_var)]);
                *t = Type::Boolean;
            }
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown loop attribute '#{name}'; use #index, #count, #first, #last, or #break"
            );
            *t = Type::Unknown(0);
        }
    }

    /// The letter a reader wrote for this radix, for a diagnostic that names the part.
    ///
    /// The inverse of [`crate::parser::radix_for`]; `10` is the decimal default, which no
    /// refusal reports, so it has no letter here.
    fn radix_letter(radix: i32) -> &'static str {
        match radix {
            -1 => "j",
            1 => "e",
            2 => "b",
            8 => "o",
            16 => "x",
            _ => "X",
        }
    }

    pub(crate) fn append_data_fp(state: OutputState, fmt: Value) -> (Value, Value, Value) {
        let mut a_width = state.width;
        let mut p_rec = Value::Int(-1); // -1 = no precision specified; 0 = :.0
        if let Value::Float(w) = a_width {
            let s = format!("{w}");
            let mut split = s.split('.');
            a_width = Value::Int(split.next().unwrap().parse::<i32>().unwrap());
            // `{m:1.0}` — precision ZERO: the dotted spec was parsed into a
            // FLOAT whose Display drops the trailing `.0`, so the fraction
            // part is absent here.  Absent means a zero fraction (a dotted
            // spec is the only way the width arrives as Float), not "no
            // precision" — unwrapping it was a parser ICE on `:N.0`.
            p_rec = Value::Int(split.next().map_or(0, |p| p.parse::<i32>().unwrap_or(0)));
        }
        if state.float {
            p_rec = a_width;
            a_width = Value::Int(0);
        }
        (fmt, a_width, p_rec)
    }

    pub(crate) fn append_data_long(
        &mut self,
        list: &mut Vec<Value>,
        start: &str,
        var: Value,
        fmt: Value,
        state: OutputState,
    ) {
        list.push(self.cl(
            &(start.to_owned() + "Int"),
            &[
                var,
                fmt,
                Value::Int(state.radix),
                state.width,
                Value::Int(i32::from(state.token.as_bytes()[0])),
                Value::Boolean(state.plus),
                Value::Boolean(state.note),
                Value::Int(state.dir),
            ],
        ));
    }

    pub(crate) fn append_data_text(
        &mut self,
        list: &mut Vec<Value>,
        start: &str,
        var: Value,
        fmt: Value,
        state: OutputState,
    ) {
        list.push(self.cl(
            &(start.to_owned() + "Text"),
            &[
                var,
                fmt,
                state.width,
                Value::Int(state.dir),
                Value::Int(i32::from(state.token.as_bytes()[0])),
            ],
        ));
    }

    #[allow(clippy::too_many_lines)]
    /// P242: when `d_nr` is the type variable of the current
    /// generic-function context AND that variable's bound
    /// supplies a `to_text(self: Self) -> text` method, return a
    /// `Value::Call(t_<len><tvname>_to_text, [format])` IR node.
    /// Otherwise return `None`.
    ///
    /// Used by `append_data` to route generic-T format-string
    /// interpolation (`println("{x}")` where `x: T`) through the
    /// bound's stub method.  At monomorphisation time
    /// `re_resolve_call` substitutes the stub with the concrete
    /// type's implementation, just like an explicit
    /// `x.to_text()` call site.
    fn try_bound_to_text_call(&mut self, d_nr: u32, format: &Value, spec: &str) -> Option<Value> {
        // @PLN99 Arc B — route `{x}` / `{x:spec}` through the type's OWN
        // `to_text(self, …)` for ANY struct, not only the current bounded-
        // generic's type variable (the old gate required `context == Generic`).
        // A struct with no `t_<len><Type>_to_text` method (`stub_nr == u32::MAX`
        // below) falls back to the generic `OpFormatDatabase` dump — nothing
        // regresses.
        if self.data.def_type(d_nr) != DefType::Struct {
            return None;
        }
        // loft#1147 — a TYPE VARIABLE's stub existing is not evidence that THIS function may
        // call it.  Type-variable bounds are keyed by NAME, so every generic in the program
        // spelling its variable `T` shares one `T` definition, and one bounded generic
        // anywhere mints the stub for all of them.  `call_op` asks `has_bound_for_method` for
        // exactly this reason — its comment names the leak — while this site, whose own doc
        // says *"AND that variable's bound supplies a `to_text` method"*, only checked that
        // the stub existed.  Adding an `Equatable + Printable` generic to the stdlib is what
        // made that observable: `fn show<T>(v: T) -> text { "{v}" }` went from refused to
        // accepted with nothing in the user's file changed.
        //
        // A CONCRETE struct keeps @PLN99 Arc B's widened behaviour untouched — its `to_text`
        // is its own and there is no bound to consult.
        if self.data.is_type_var_placeholder(d_nr)
            && !self.has_bound_for_method("to_text", d_nr, None)
        {
            return None;
        }
        let tv_name = self.data.def(d_nr).name().to_string();
        if tv_name.is_empty() {
            return None;
        }
        // loft#1153 — a HOLDER's stub and a concrete type's method have different spellings;
        // which to look up follows from which this is.  They used to collide, and a struct named
        // like a type variable then resolved to the stub and rendered EMPTY.
        let stub_name = self.data.method_key(d_nr, "to_text", 1);
        let stub_nr = self.data.def_nr(&stub_name);
        if stub_nr == u32::MAX {
            return None;
        }
        // loft#1147 — the mangled key spells a NAME, not a type.  `t_1T_to_text` is minted
        // for the stdlib's type VARIABLE `T` and is the same string a user `struct T` mangles
        // to, so a name-only lookup hands one the other's method.  Adding the stdlib's first
        // `Printable`-bounded generic is what made that reachable: every user struct named `T`
        // began formatting as EMPTY, because `{t}` routed through a bound stub that
        // monomorphisation could not resolve for it.
        //
        // The stub's own `self` parameter carries the answer — `set_bound_stub_signature`
        // substitutes `Self` with the holder — so require it to name THIS definition.  A
        // struct's genuine `t_<LEN><Type>_to_text` names itself and still routes; a stub
        // belonging to a same-named type variable does not.
        if !matches!(self.data.attr_type(stub_nr, 0), Type::Reference(r, _) if r == d_nr) {
            return None;
        }
        // Classify the stub's params by TYPE, not arity.  A `to_text` may or may
        // not carry a user `spec: text` param (@PLN99 Arc B — the value owns its
        // `{x:spec}` DSL), and INDEPENDENTLY may or may not carry the hidden
        // text-return work buffer (`RefVar(Text)`, added by `parse_function`'s
        // I9-text path so the call mirrors an auto-generated `convert(text →
        // &text)`).  Counting attributes conflates `(self, spec)` with
        // `(self, __work)` — both are 2 — so a `to_text(self, spec)` whose body
        // returns text directly (a tail `if`, a bare literal: no work buffer)
        // silently lost its spec and received the empty work buffer in its place
        // (#533).  The buffer is also renamed to a promoted local (`r`) by
        // text_return, so its `__`-name is not reliable — only the type is.  self
        // is attr 0 (the struct being formatted, never text/RefVar).
        // Fill ONE argument per attribute, walking the definition's own order.
        // The count is the fact, not the presence: a body with more than one
        // formatted `return` promotes one text work buffer PER return
        // (`__work_1`, `__work_2`, …), and a boolean cannot carry "two". It read
        // as "has a work buffer", the call went out one argument short, and
        // `generate_call` asserted — an internal compiler error on BOTH
        // backends, from a `to_text` whose only sin was an early `return`. The
        // tail-`if` form that #533 fixed happens to promote exactly one, which
        // is why a boolean survived that round: the working member hid the
        // omission.
        let mut args = vec![format.clone()];
        let mut spec_given = false;
        for a in 1..self.data.attributes(stub_nr) {
            match self.data.attr_type(stub_nr, a) {
                // The user's `spec: text`. At most one — a second `text`
                // parameter is not a shape this hook defines, so it falls
                // through to the arity guard rather than silently receiving the
                // spec twice.
                Type::Text(_) if !spec_given => {
                    spec_given = true;
                    args.push(Value::str(spec));
                }
                Type::RefVar(_) => {
                    let wv = self.vars.work_text(&mut self.lexer);
                    args.push(v_block(
                        vec![
                            v_set(wv, Value::Text(String::new())),
                            self.cl("OpCreateStack", &[Value::Var(wv)]),
                        ],
                        Type::Reference(
                            self.data.def_nr("reference"),
                            crate::data::Deps::frame1(wv),
                        ),
                        "p242_to_text_work",
                    ));
                }
                _ => {}
            }
        }
        // The chokepoint: emit the call only when the argument list FITS the
        // definition. Any other signature is one this hook cannot spell, and
        // falling back to the generic field dump is a defined answer where a
        // short call is an internal compiler error.
        if args.len() != self.data.attributes(stub_nr) {
            return None;
        }
        Some(Value::Call(stub_nr, args))
    }

    /// The storage row `"{v}"` dumps a `vector<cont>` through — `0` when there is none.
    ///
    /// One home, because the answer is needed TWICE: here, where the format op is emitted,
    /// and again per monomorph, where a template's `vector<T>` finally learns what `T` is
    /// (`Parser::retarget_parametric_vector_format`, loft#845). Two derivations of one row
    /// is how a monomorph came to dump a `vector<integer>` through the type variable's row
    /// and print `{}`.
    ///
    /// #250's recursive resolution, the FORMAT twin of `db_type`'s Vector arm, for NESTED
    /// content only: `vector<vector<X>>` content must resolve recursively — the def-level
    /// `known_type()` fallback returns whichever vector row registered first, a
    /// layout-coincident id that broke (#483 SIGSEGV in `ShowDb::write_list`) as soon as new
    /// stdlib content shifted the type table. Non-vector content keeps `known_type()`: an
    /// enum's row carries the VARIANT NAMES the format needs, where `db_type` returns the
    /// generic byte storage row.
    ///
    /// #624 — a NARROW element (`u8` / `u16` / a 4-byte `integer` subtype) is stored packed
    /// at 1/2/4 bytes, but the def-level `known_type()` resolves the WIDE integer row.
    /// Dumping through that row strides 8 bytes over packed data: `{v}` on a `vector<u8>`
    /// printed the first eight elements packed into one i64 followed by zeros. Resolve the
    /// narrow storage row the way the element READ and the `+=` append already do, and skip
    /// `check_vector` — that registers `vector<integer>`'s own row, which a narrow element
    /// does not own and must not overwrite.
    pub(crate) fn format_vector_row(&mut self, cont: &Type) -> u16 {
        if let Some(narrow) = self.data.narrow_vector_content(cont, &mut self.database) {
            return self.database.vector(narrow);
        }
        let db_tp = if matches!(cont, Type::Vector(_, _)) {
            self.database.db_type(cont, &self.data)
        } else {
            let d_nr = self.data.type_def_nr(cont);
            self.data.def(d_nr).known_type()
        };
        if db_tp == u16::MAX {
            return 0;
        }
        let v = self.database.vector(db_tp);
        self.data
            .check_vector(self.data.type_def_nr(cont), v, self.lexer.pos());
        v
    }

    pub(crate) fn append_data(
        &mut self,
        tp: Type,
        list: &mut Vec<Value>,
        append: u16,
        append_value: u16,
        format: &Value,
        state: OutputState,
    ) {
        // @PLN25 — format dispatch peels an `Optional` value type (`"{s}"` where
        // `s: text?`) to its base, matching index/method dispatch which already
        // peel; inert gate-OFF (no `Optional` is ever constructed). A null-holding
        // value formats via its base-text/scalar sentinel exactly as gate-OFF.
        let tp = match tp {
            Type::Optional(inner) => *inner,
            other => other,
        };
        let var = Value::Var(append);
        let start = if matches!(self.vars.tp(append), Type::RefVar(_)) {
            "OpFormatStack"
        } else {
            "OpFormat"
        };
        // L9: escalate format-specifier mismatches to compile errors.
        // A specifier that can never have any effect on the value type is always a bug.
        if !self.first_pass {
            let is_text = matches!(tp, Type::Text(_));
            // @FR-F-Spec — a precision reaches here in two spellings: a bare `.P` sets
            // `float` and leaves `P` in the width slot, and the dotted `W.P` — the only
            // spelling that gives both at once — arrives as one `Value::Float`.
            let has_precision = state.float || matches!(state.width, Value::Float(_));
            // @FR-F-Spec — an integer renders through `ops::format_long`, which implements
            // the radixes the rule lists (`b` 2, `o` 8, decimal 10, `x` 16, `X` upper) and
            // ends in `panic!("Unknown radix")` for anything else.  `get_radix` answers two
            // more: `e` (scientific, 1) and `j` (JSON, -1).  Neither means anything for an
            // integer and both reached that panic, so `println("{n:e}")` — a plain source
            // program — aborted the interpreter.  Refuse them here, where the value's type
            // is known, instead of at a renderer that has only the radix number left.
            let hex_upper = i32::from(crate::ops::HEX_UPPER);
            // @FR-F-Spec-Radix — which radixes the renderer for this type has an arm for.  An
            // integer has the four bases plus upper-case hex; a heap value renders
            // through the store walker, whose one switch is JSON; every other type has a
            // single rendering, so only the decimal default reaches it.
            let radix_ok = match tp {
                Type::Integer(_) => {
                    matches!(state.radix, 2 | 8 | 10 | 16) || state.radix == hex_upper
                }
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, _, _) => {
                    matches!(state.radix, 10 | -1)
                }
                _ => state.radix == 10,
            };
            if matches!(tp, Type::Integer(_)) && !radix_ok {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{}` is not an integer format — use `x`, `X`, `b`, `o` or `d`",
                    Self::radix_letter(state.radix)
                );
            } else if !radix_ok {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{}` has no effect on {}",
                    Self::radix_letter(state.radix),
                    tp.name(&self.data)
                );
            } else if is_text && state.token == "0" && state.width != Value::Int(0) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Zero-padding has no effect on text"
                );
            } else if has_precision && !matches!(tp, Type::Float | Type::Single) {
                // @FR-F-Spec — `.P` asks for fractional digits, and only `float` and
                // `single` have them.  Their two arms are also the only ones that call
                // `append_data_fp`, which is what splits a dotted `W.P` into a width and a
                // precision; every other renderer takes `state.width` as written, so the
                // `f64` of a dotted spec lands in a slot the opcode reads as an i64 WIDTH.
                // Reinterpreted, `8.2` is a pad count of ~4.6e18: `--interpret` asks for
                // the whole field in one allocation and is OOM-killed, and `--native`
                // hands rustc `E0308 expected i64, found f64` about loft's internals.  The
                // bare `.P` spelling is quieter and worse — it leaves the precision in the
                // width slot, so `{n:.4}` renders a four-wide field in silence.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a precision has no effect on {} — `.N` sets fractional digits, \
                     which only `float` and `single` have",
                    tp.name(&self.data)
                );
            }
        }
        // @FR-F-Spec composes with @FR-F-Render: a spec TUNES ⟦v⟧, so the width, the
        // alignment and the pad token apply to whatever the value's type renders as.  Most
        // arms below carry them into their own renderer.  Two families cannot:
        // `OpAppendCharacter` takes only the accumulator and the value, and
        // `OpFormatDatabase` has room for the two `db_format` bits and nothing else — so a
        // `{c:>5}` or a `{v:>12}` was rendered unpadded and nothing said the spec had been
        // dropped (loft#1165, loft#1166).
        //
        // Render into a scratch text with the padding REMOVED, then format that text with
        // it.  The composition is the rule, and it reaches every such type at once rather
        // than widening one op signature per family.  The flags that belong to the RENDER
        // — `#` and the `:j` radix, which `db_format` carries — stay on the inner call;
        // only the field-shaping ones move out.
        if !self.first_pass
            && state.width != Value::Int(0)
            && matches!(
                tp,
                Type::Character | Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, _, _)
            )
        {
            let wv = self.vars.work_text(&mut self.lexer);
            let inner = OutputState {
                width: Value::Int(0),
                dir: crate::parser::OUTPUT_DEFAULT.dir,
                token: crate::parser::OUTPUT_DEFAULT.token,
                ..state
            };
            let mut rendered = vec![v_set(wv, Value::Text(String::new()))];
            self.append_data(tp, &mut rendered, wv, append_value, format, inner);
            list.extend(rendered);
            self.append_data_text(list, start, var, Value::Var(wv), state);
            return;
        }
        match tp {
            Type::Integer(_) => {
                self.append_data_long(list, start, var, format.clone(), state);
            }
            Type::Boolean => {
                let value = self.cl("OpCastTextFromBool", std::slice::from_ref(format));
                self.append_data_text(list, start, var, value, state);
            }
            Type::Text(_) => {
                self.append_data_text(list, start, var, format.clone(), state);
            }
            Type::Character => {
                list.push(self.cl("OpAppendCharacter", &[var, format.clone()]));
            }
            Type::Float => {
                let dir = Value::Int(state.dir);
                let plus = Value::Boolean(state.plus);
                // @FR-F-Spec — the pad TOKEN travels with the rest of the spec.  The four
                // float opcodes had no slot for it, so `ops::format_float` filled with a
                // hard-coded space: `{f:06}` padded with spaces and `{f:*^11}` ignored its
                // fill, both silently, while the same specs worked on an integer.
                let token = Value::Int(i32::from(state.token.as_bytes()[0]));
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(
                    &(start.to_owned() + "Float"),
                    &[var, fmt, a_width, p_rec, token, plus, dir],
                ));
            }
            Type::Single => {
                let dir = Value::Int(state.dir);
                let plus = Value::Boolean(state.plus);
                // @FR-F-Spec — the pad TOKEN travels with the rest of the spec.  The four
                // float opcodes had no slot for it, so `ops::format_float` filled with a
                // hard-coded space: `{f:06}` padded with spaces and `{f:*^11}` ignored its
                // fill, both silently, while the same specs worked on an integer.
                let token = Value::Int(i32::from(state.token.as_bytes()[0]));
                let (fmt, a_width, p_rec) = Self::append_data_fp(state, format.clone());
                list.push(self.cl(
                    &(start.to_owned() + "Single"),
                    &[var, fmt, a_width, p_rec, token, plus, dir],
                ));
            }
            Type::Vector(cont, _) => {
                let fmt = format.clone();
                let vec_tp = self.format_vector_row(&cont);
                list.push(self.cl(
                    &(start.to_owned() + "Database"),
                    &[
                        var,
                        fmt,
                        Value::Int(i32::from(vec_tp)),
                        Value::Int(state.db_format()),
                    ],
                ));
            }
            Type::Iterator(vtp, _) => {
                self.append_iter(list, append, append_value, vtp.as_ref(), format, state);
            }
            Type::Reference(d_nr, _) => {
                // P242 fix: when `d_nr` is the current generic
                // function's type variable AND the bound supplies
                // a `to_text` method, route the format through it
                // before dispatching as text.  Without this, the
                // codegen falls back to OpFormatDatabase with a
                // non-DbRef arg at monomorphisation time (interp
                // prints "null" then panics; native rejects with
                // rustc E0308 "expected DbRef").  The bound's
                // `to_text` stub (`t_<len><tvname>_to_text`) is
                // already created by `parse_function`'s I7/I8.1
                // path; `re_resolve_call` substitutes it with the
                // concrete type's impl at instantiation time.
                if let Some(text_call) = self.try_bound_to_text_call(d_nr, format, state.spec) {
                    self.append_data_text(list, start, var, text_call, state);
                } else if self.data.is_type_var_placeholder(d_nr) {
                    // loft#845 — the same fault P242 fixed for a BOUND type variable, for
                    // an unbounded one, where there is no `to_text` to route through.
                    //
                    // Falling through to the record formatter below picked the op from the
                    // TEMPLATE's view of the type variable — an attribute-less struct, so a
                    // reference — and the monomorph replaces the TYPE without re-choosing
                    // the OP.  Measured over every argument kind, NOT ONE produced a right
                    // answer: `OpFormatDatabase` on an `i64`/`f64`/`boolean`/`character`
                    // SIGSEGVs on `--interpret` and is `E0308` on `--native`, and a `text`
                    // or struct argument rendered the literal `{}` on both.  So this
                    // refuses nothing that worked.
                    //
                    // Refusing is also the rule the rest of the language already applies:
                    // inside a generic ONLY the bounds may be relied on — a method call, a
                    // subscript (@PLN125 arc C) and an operator all say so — and formatting
                    // is not an exception to it.  `Printable` is the bound, it is satisfied
                    // by every built-in, and the bounded path renders correctly on both
                    // backends for every kind.
                    if !self.first_pass {
                        let name = crate::data::Data::type_var_spelling(self.data.def(d_nr).name())
                            .to_string();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "generic type {name} cannot be formatted — `\"{{…}}\"` needs a \
                             bound that renders it; write `<{name}: Printable>` (every \
                             built-in satisfies it, and a user type does by defining \
                             `fn to_text(self: {name}) -> text`)"
                        );
                    }
                } else {
                    let fmt = format.clone();
                    let db_tp = self.data.def(d_nr).known_type();
                    list.push(self.cl(
                        &(start.to_owned() + "Database"),
                        &[
                            var,
                            fmt,
                            Value::Int(i32::from(db_tp)),
                            Value::Int(state.db_format()),
                        ],
                    ));
                }
            }
            Type::Enum(d_nr, is_ref, _) => {
                let fmt = format.clone();
                let e_tp = self.data.def(d_nr).known_type();
                if e_tp == u16::MAX || !is_ref {
                    // A scalar enum is a byte, not a record, so it never reaches
                    // the record walker below: it is cast to text HERE, before
                    // the format spec is applied, which is why `:j` on a bare
                    // scalar enum still renders the unquoted name (loft#768's
                    // residual).  Carrying the JSON-ness into the cast is not
                    // enough — `Str` is a borrowed view, and the quoted form has
                    // nowhere to live now that @PLN10 retired the Str-lifetime
                    // scratch buffer.  The fix is a destination-passing op that
                    // appends into the work buffer, as `OpFormatDatabase` does.
                    let e_val = self.cl("OpCastTextFromEnum", &[fmt, Value::Int(i32::from(e_tp))]);
                    self.append_data_text(list, start, var, e_val, state);
                } else {
                    list.push(self.cl(
                        &(start.to_owned() + "Database"),
                        &[
                            var,
                            fmt,
                            Value::Int(i32::from(e_tp)),
                            Value::Int(state.db_format()),
                        ],
                    ));
                }
            }
            _ => {
                // @P376 — `Type::Never` is the poison an errored struct
                // construction assigns; the real `unknown type '…'` was already
                // reported, so silently skip formatting it instead of adding a
                // cascade "Cannot format type never".
                if !self.first_pass && !matches!(tp, Type::Never) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot format type {}",
                        tp.name(&self.data)
                    );
                }
            }
        }
    }

    pub(crate) fn append_iter(
        &mut self,
        list: &mut Vec<Value>,
        append: u16,
        append_value: u16,
        var_type: &Type,
        value: &Value,
        state: OutputState,
    ) {
        if let Value::Iter(var, init, next, extra_init) = value
            && matches!(**next, Value::Block(_))
        {
            let count = if *var == u16::MAX {
                self.create_unique("count", &I32)
            } else {
                let count_name = format!("{}#count", self.vars.name(*var));
                let c = self.vars.var(&count_name);
                if c == u16::MAX {
                    self.create_var(&count_name, &I32)
                } else {
                    c
                }
            };
            list.push(self.cl("OpAppendText", &[Value::Var(append), Value::str("[")]));
            list.push(*init.clone());
            if !matches!(**extra_init, Value::Null) {
                list.push(*extra_init.clone());
            }
            list.push(v_set(count, Value::Int(0)));
            let mut append_var = append_value;
            if append_value == u16::MAX {
                append_var = self.create_unique("val", var_type);
            }
            let mut steps = Vec::new();
            steps.push(v_set(append_var, *next.clone()));
            steps.push(v_if(
                self.cl("OpLtInt", &[Value::Int(0), Value::Var(count)]),
                self.cl("OpAppendText", &[Value::Var(append), Value::str(",")]),
                Value::Null,
            ));
            steps.push(v_set(
                count,
                self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
            ));
            self.append_data(
                var_type.clone(),
                &mut steps,
                append,
                append_var,
                &Value::Var(append_var),
                state,
            );
            list.push(v_loop(steps, "Append Iter"));
            list.push(self.cl("OpAppendText", &[Value::Var(append), Value::str("]")]));
        }
    }

    // <object> ::= [ <identifier> ':' <expression> { ',' <identifier> ':' <expression> } ] '}'
    /// Parse a single `field: value` entry in an object literal.
    /// Returns `None` if parsing should abort (lexer reverted), `Some(false)` on unknown field,
    /// `Some(true)` on success.
    /// Parse a single `field: value` entry in an object literal.
    /// Returns false if no identifier found or `:` missing (caller handles revert).
    /// `id` is the name this loop BINDS (`Function::loop_binding`), `src_id` the name the
    /// program wrote.  They differ only from the second loop over a name onward, and the
    /// two are read for different questions: companions and the binding itself are keyed
    /// off `id`, while the shadow guards ask what `src_id` denotes right now.
    pub(crate) fn parse_for_iter_setup(
        &mut self,
        id: &str,
        src_id: &str,
        in_type: &Type,
        expr: Value,
    ) -> (u16, Option<u16>, u16, Value, Value, Value) {
        let var_tp = self.for_type(in_type);
        // For text loops: {id}#next drives the loop; {id}#index is saved per-iteration.
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
        // A loop variable that lands on a name the function already uses for something
        // else is rejected, in two shapes that both silently produced wrong values.
        //
        //   * Outer-local shadow (`x = 5; for x in …`) — the loop clobbers `x`, whatever
        //     the two types are.  Caught on PASS 1: the prior binding is unambiguously a
        //     plain local, because a preceding loop's binding carries `was_loop_var`.
        //   * Nested same-name loops (`for i { for i { … } }`) — the inner loop re-points
        //     `i` at its own variable for the body and never restores it, so the outer
        //     loop's remaining body would read the inner one.  Caught on PASS 2, via the
        //     active-loop chain.
        //
        // Both ask what the SOURCE spelling denotes right now.  `_` is exempt: it is the
        // universal discard and must work across element types in one function.
        //
        // SEQUENTIAL same-name loops are legal, and loft#915 is why they need no check:
        // each loop binds its own variable, so the second inherits no type, dep or storage
        // from the first.  That replaces loft#690's diagnostic — *"loop variable 'i' has
        // type text but was previously used as integer"* — which existed because
        // `add_variable` handed the second loop the FIRST binding and its body then read
        // B's records through A's layout (`m=8589934636` for a sum of 3).  The corruption
        // is now unreachable by construction rather than by report, so the diagnostic is
        // gone and the shape it rejected compiles.  The local collision it ALSO covered is
        // the case above, which no longer needs a type comparison to state: any non-loop
        // binding of the name is the shadow, whatever its type.
        let existing_var = self.vars.var(src_id);
        if id != "_"
            && existing_var != u16::MAX
            && self.first_pass
            && !self.vars.was_loop_var(existing_var)
            && !self.vars.is_active_loop_var(existing_var)
            // `text_return` converts text variables to `RefVar(Text)` work buffers for the
            // return path.  A loop variable converted this way IS the work buffer, so the
            // iterator writing into it is correct, not a shadow.
            && !matches!(self.vars.var_type(existing_var), Type::RefVar(_))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "loop variable '{src_id}' shadows a local named '{src_id}' — \
                 rename the loop variable (e.g. loop_{src_id}) or drop the \
                 outer `{src_id}` if it was a dead placeholder; loft does \
                 not block-scope loop variables"
            );
        }
        if id != "_"
            && existing_var != u16::MAX
            && !self.first_pass
            && self.vars.is_active_loop_var(existing_var)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "loop variable '{src_id}' shadows the enclosing loop's '{src_id}' — \
                 rename the inner loop variable (e.g. inner_{src_id}); loft does \
                 not support nested same-name loops"
            );
        }
        // loft#762 — `_` gets its OWN slot per loop.
        //
        // `_` is the universal discard, so `change_var_type` retypes it silently and
        // the C61 shadow guards exempt it — both deliberate, because one function may
        // discard several types. But there was still only ONE `_` slot, so a loop
        // binding SHARED it with any earlier `_ = call()`. The interpreter retypes and
        // copes; native declares `let mut var__: <whichever came first>` once and then
        // assigns the other, which is E0308 — a program that runs interpreted and does
        // not compile. The ASSIGNMENT form has no such hole: `_ = f(); _ = mk()` with
        // two types is rejected at parse time. It is the loop path, exempt from that
        // check, that retyped without complaint.
        //
        // The name stays bound to this slot for the body (the caller rebinds it around
        // `parse_block` and restores the outer one after), because `_` IS readable —
        // `for _ in 0..4 { r = r + _ }` is pinned in `anon-loop-counters.loft`.
        let for_var = if id == "_" {
            self.create_unique("_", &var_tp)
        } else {
            self.create_loop_var(id, &var_tp)
        };
        // Point the source spelling at THIS loop's binding, and leave it there: the body
        // reads `i` as this loop's variable, and so does the code after the loop, which is
        // the value `i` held before loop variables were split per loop (loft#915).
        if id != src_id {
            self.vars.set_name(src_id, for_var);
        }
        self.vars.defined(for_var);
        let if_step = if self.lexer.has_token("if") {
            let mut if_expr = Value::Null;
            self.expression(&mut if_expr);
            if_expr
        } else {
            Value::Null
        };
        let mut create_iter = expr;
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, in_type, &it, iter_var, pre_var);
        (iter_var, pre_var, for_var, if_step, create_iter, iter_next)
    }

    // @F28 — for-in loops (ranges, loop attributes, filtered, rev())
    pub(crate) fn parse_for(&mut self, code: &mut Value) {
        // P235: tuple destructure — `for (a, b, ...) in items { ... }`.
        // Parse the parenthesised name list now; later (after the iter
        // type is known) we synthesize a temp loop var and prepend
        // `Set(name_i, loop_var.<i>)` to the body so `a` / `b` resolve
        // as if the user had written them.  Closes both par and
        // non-par destructure with one rewrite (the par variant
        // dispatches into `parse_parallel_for_loop` which inherits
        // `id` from us).
        let destructure_names: Option<Vec<String>> = if self.lexer.peek_token("(") {
            self.lexer.token("(");
            let mut names = Vec::new();
            loop {
                if let Some(n) = self.lexer.has_identifier() {
                    names.push(n);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect identifier in for-destructure pattern"
                    );
                    let _ = self.lexer.has_token(")");
                    return;
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            if !self.lexer.has_token(")") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect ')' to close for-destructure pattern"
                );
                return;
            }
            if names.len() < 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "for-destructure pattern requires at least 2 names; got {}",
                    names.len()
                );
                return;
            }
            Some(names)
        } else {
            None
        };

        // @PLN115 tail — capture the simple `for id` binder's position before it is
        // consumed, to record its DECLARATION once the loop var exists (only when
        // recording; destructure binders are synthesized, so they are excluded).
        let binder_pos = (destructure_names.is_none() && self.record_resolutions)
            .then(|| self.lexer.peek_pos().clone());
        // P235: when destructuring, synthesize a loop var name from
        // the source line/column; the user-named binders are defined
        // later as proper variables and prepended to the body.
        let id_opt: Option<String> = if destructure_names.is_some() {
            let pos = self.lexer.peek().position.clone();
            Some(format!("__destructure_t_{}_{}", pos.line, pos.pos))
        } else {
            self.lexer.has_identifier()
        };
        if let Some(src_id) = id_opt {
            // @P345: a loop variable's type is fully determined by the
            // iterable's element type, so an annotation is always redundant
            // (and unsupported).  Catch `for i: T in …` here and emit one
            // clear message instead of the misleading `Expect token in` →
            // `{` → `;` cascade the bare `token("in")` would produce.
            if self.lexer.peek_token(":") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "loop variable '{src_id}' is type-inferred from the iterable — remove the ': <type>' annotation (write `for {src_id} in …`)"
                );
                // Recover: skip the (possibly compound) annotation up to `in`
                // so the loop body still parses and no spurious token cascade
                // follows the clear message above.
                self.lexer.has_token(":");
                while !self.lexer.peek_token("in")
                    && !self.lexer.peek_token("{")
                    && !self.lexer.peek_token("")
                {
                    self.lexer.cont();
                }
            }
            // loft#915 — the name this loop BINDS.  Every variable this loop mints (the
            // loop variable and its `#index` / `#next` companions) is keyed off it, so a
            // second `for i` in the same function shares nothing with the first; the body
            // still reads `i`, which `parse_for_iter_setup` re-points.
            let id = self.vars.loop_binding(&src_id);
            self.lexer.token("in");
            let loop_nr = self.vars.start_loop();
            let mut expr = Value::Null;
            // loft#986 — see `in_control_head`: the `{` after the iterable opens the body.
            let outer_head = self.in_control_head;
            self.in_control_head = true;
            let mut in_type = self.parse_in_range(&mut expr, &Value::Null, &id);
            self.in_control_head = outer_head;
            // if #fields was detected, take the compile-time unrolling path.
            if self.fields_of != u32::MAX {
                let struct_def_nr = self.fields_of;
                self.fields_of = u32::MAX;
                self.vars.finish_loop(loop_nr);
                self.parse_field_iteration(&id, &src_id, struct_def_nr, &expr, code);
                return;
            }
            let mut fill = Value::Null;
            // For vector loops, the iterator runs on a unique temp copy so that the loop
            // variable does not alias the user-visible collection.  Record the original
            // variable number so that mutation of the original can be detected later.
            let orig_coll_var = if let Value::Var(v) = &expr {
                *v
            } else {
                u16::MAX
            };
            // Save the original collection expression before the vector temp-copy substitution
            // so that is_iterated_value() can match field-access patterns like `db.items`.
            let orig_coll_expr = expr.clone();
            // C60 piece 3 edit B (re-attempt with typed scratch): when
            // iterating a hash, substitute the collection expression
            // with a call to `hash_sorted(h, tp_id)` that builds a
            // u32-stride rec-nr scratch in the hash's own store
            // (edit A).  `in_type` stays Type::Hash so fill_iter hits
            // the Hash arm and emits on=3 (edit C); the empty-bounds
            // guard in iterate on=3 (edit E) handles unbounded.
            //
            // Key fix from prior segfault attempt: type the scratch
            // variable with the hash's actual content def-nr
            // (`Type::Reference(content, dep)`), not `Reference(0)`.
            // Downstream type-size + free-cleanup machinery reads
            // `self.data.def(content)`; passing 0 gave whatever
            // definition happens to sit at index 0 and corrupted
            // stack layout.
            // Set to the `hash_scratch` var for on=4 (Hash/Radix) iteration, so the
            // loop epilogue can free a read-only source's DEDICATED scratch store
            // (`OpFreeScratch`, a no-op when co-located).  u16::MAX = not on=4.
            let mut hash_scratch_var: u16 = u16::MAX;
            if let Type::Hash(content, _, dep)
            | Type::Radix(content, _, dep)
            | Type::Trie(content, _, dep) = in_type.clone()
            {
                // @FR-Col-Order — this snapshot is WHY a sequential `for x in h` is in KEY
                // order: the hash builder sorts, and the walk reads the sorted copy.  Only
                // the `par` walk skips the sort (@FR-C-Order), which is the one place the
                // two orders differ.
                //
                // A trie is a radix TREE too, so its in-order walk is already key
                // order: it takes the tree builder, not the hash one (whose bucket
                // walk would read a trie's records as a hash table).
                let is_radix = matches!(in_type, Type::Radix(_, _, _) | Type::Trie(_, _, _));
                let scratch_tp = Type::Reference(content, dep.clone());
                let scratch_var = self.create_unique("hash_scratch", &scratch_tp);
                hash_scratch_var = scratch_var;
                let hash_tp_id = self.get_type(&in_type);
                let tp_arg = if hash_tp_id == u16::MAX {
                    0
                } else {
                    i32::from(hash_tp_id)
                };
                // Enforces @FR-C-Order's keyed exception.  A sequential `for x in h` is
                // KEY-ordered; a `par` one is the hash's UNSORTED bucket walk, because the
                // parallel queue has no use for key order — so it skips the O(n log n) key
                // sort.  The two orders differing is stated by the rule, not a divergence.
                //
                // A radix has a natural order and its walk is already ordered (no sort), so
                // it uses the same builder in every case (@PLN48).
                let is_par =
                    matches!(&self.lexer.peek().has, LexItem::Identifier(kw) if kw == "par");
                let scratch_fn_name = if is_radix {
                    "n_radix_sorted"
                } else if is_par {
                    "n_hash_unsorted"
                } else {
                    "n_hash_sorted"
                };
                // @PLN48 S3 — a spatial range slice (`xs[(x,y)..]`, `xs[(x,y)..:n]`,
                // `xs[(x1,y1)..(x2,y2)]`) and a trie prefix slice have ALREADY been
                // rewritten to a call that BUILDS the ordered scratch.  Use it directly
                // — do not wrap it in n_radix_sorted, which would walk the scratch as
                // if it were a tree.  (There is no `.within` / `.near` / `.nearest`
                // method: proximity is ordinary range slicing.)
                let already_scratch = matches!(
                    expr.unspan(),
                    Value::Call(d, _) if matches!(self.data.def(*d).name(), "n_spatial_range" | "n_trie_prefix")
                );
                if already_scratch {
                    fill = v_set(scratch_var, expr.clone());
                    expr = Value::Var(scratch_var);
                    if !self.first_pass {
                        self.vars.set_type(scratch_var, scratch_tp);
                    }
                } else {
                    let hash_sorted_fn = self.data.def_nr(scratch_fn_name);
                    if hash_sorted_fn != u32::MAX {
                        let call =
                            Value::Call(hash_sorted_fn, vec![expr.clone(), Value::Int(tp_arg)]);
                        fill = v_set(scratch_var, call);
                        expr = Value::Var(scratch_var);
                        if !self.first_pass {
                            self.vars.set_type(scratch_var, scratch_tp);
                        }
                    }
                }
            }
            if matches!(in_type, Type::Vector(_, _)) {
                let vec_var = self.create_unique("vector", &in_type);
                // The loop iterates THIS temp — see `Function::iteration_source`.
                self.vars.set_iteration_source(vec_var);
                // On the second pass in_type may carry __vdb_N dependencies that
                // were not present on the first pass (vector_db only runs on pass 2).
                // Update the temp variable's type so that get_free_vars sees the
                // deps and does NOT emit OpFreeRef for the temp — the __vdb_N
                // variable at the outer scope owns the store and will free it.
                if !self.first_pass {
                    self.vars.set_type(vec_var, in_type.clone());
                }
                in_type = in_type.depending(vec_var);
                fill = v_set(vec_var, expr);
                expr = Value::Var(vec_var);
            }
            // Optional parallel clause: par(result=worker(elem), threads)
            if let LexItem::Identifier(kw) = &self.lexer.peek().has
                && kw == "par"
            {
                self.lexer.has_identifier(); // consume "par"
                // Plan-06 phase 4d.B — par-over-keyed-collection
                // desugar.  When the input is sorted/hash/index/
                // spatial, the par dispatcher's flat-vector iteration
                // doesn't know how to walk the tree/hashmap layout.
                // Pre-materialise into a `vector<reference<T>>` and
                // re-route par() to use the materialised vector.
                if crate::parser::vectors::is_keyed(&in_type)
                    && let Some((mat_fill_ir, mat_var, mat_in_type)) =
                        self.materialise_keyed_for_par(&in_type, &expr)
                {
                    let combined_fill = if fill == Value::Null {
                        mat_fill_ir
                    } else {
                        // Inline (Insert), not a scoped Block — see the
                        // materialise_keyed_for_par note: native codegen must see
                        // `__par_mat`'s `let` in the enclosing function scope.
                        Value::Insert(vec![fill, mat_fill_ir])
                    };
                    self.parse_parallel_for_loop(
                        code,
                        &id,
                        &src_id,
                        &mat_in_type,
                        &Value::Var(mat_var),
                        combined_fill,
                        loop_nr,
                        destructure_names.as_deref(),
                    );
                    return;
                }
                // A range / `iterator<T>` / text input has no flat vector for
                // the par dispatcher to partition; materialise it into one via
                // the same iterate-and-append the comprehension uses, then
                // re-route par() over the materialised vector.
                if let Some((mat_fill_ir, mat_var, mat_in_type)) =
                    self.materialise_iter_for_par(&in_type, &expr, loop_nr)
                {
                    let combined_fill = if fill == Value::Null {
                        mat_fill_ir
                    } else {
                        Value::Insert(vec![fill, mat_fill_ir])
                    };
                    self.parse_parallel_for_loop(
                        code,
                        &id,
                        &src_id,
                        &mat_in_type,
                        &Value::Var(mat_var),
                        combined_fill,
                        loop_nr,
                        destructure_names.as_deref(),
                    );
                    return;
                }
                self.parse_parallel_for_loop(
                    code,
                    &id,
                    &src_id,
                    &in_type,
                    &expr,
                    fill,
                    loop_nr,
                    destructure_names.as_deref(),
                );
                return;
            }
            let (iter_var, pre_var, for_var, if_step, create_iter, iter_next) =
                self.parse_for_iter_setup(&id, &src_id, &in_type, expr);
            // loft#762 — `_` names THIS loop's binding while its body is parsed, and
            // the outer one again afterwards, so a later `_ = call()` keeps its own
            // slot instead of retyping the loop's.
            let outer_discard: Option<u16> = if id == "_" && for_var != u16::MAX {
                self.vars.set_name("_", for_var)
            } else {
                None
            };
            // @PLN115 tail — record the loop binder's DECLARATION (pass 2, recording
            // on): `Local{fn_def, for_var}` at the binder name, so a `for i` binder's
            // references/rename take S4's precise path instead of the F-v1 fallback.
            if let Some(pos) = &binder_pos
                && !self.first_pass
                && for_var != u16::MAX
            {
                self.record_decl(
                    pos,
                    src_id.chars().count() as u16,
                    crate::resolution::Resolution::Local {
                        fn_def: self.context,
                        var_nr: for_var,
                    },
                );
            }
            let var_tp = self.for_type(&in_type);
            // For vector loops: set_loop stores the temp-copy var; override with the
            // original so that `orig += elem` is correctly identified as a mutation.
            if matches!(in_type, Type::Vector(_, _)) {
                if orig_coll_var != u16::MAX {
                    self.vars.set_coll_var(orig_coll_var);
                }
                // Always restore the original collection expression so that
                // is_iterated_value() can match field-access forms like `db.items`.
                self.vars.set_coll_value(orig_coll_expr.clone());
            }
            if !self.first_pass && iter_next == Value::Null {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Need an iterable expression in a for statement"
                );
                // Balance the loop stack before bailing: `start_loop` (above)
                // has already pushed this loop's scope, so a bare `return`
                // leaves `current_loop` pointing at it and the ENCLOSING loop's
                // `finish_loop` then trips the "Incorrect loop finish" assert
                // (a hard panic masking this diagnostic).  Mirrors the `#fields`
                // early-return at the top of this function.  Reached e.g. when a
                // loop variable is reused across two sequential loops of
                // different element types (`for t in a {…} for t in b {…}`): the
                // reused name keeps the first type, so a field access on the
                // second loop var fails to resolve and yields no iterator.
                self.vars.finish_loop(loop_nr);
                return;
            }
            // P235 step 2: with for_var resolved, define each destructured
            // binder as a proper variable typed as the matching tuple
            // element.  The Set(name_i, TupleGet(for_var, i)) ops will be
            // prepended to the body block once `parse_block` returns.
            // Defining the variables BEFORE `parse_block` runs is essential
            // — the body's references to `a` / `b` / etc. resolve through
            // the parser's scope at parse time.
            let destructure_setup: Vec<Value> = if let Some(names) = &destructure_names {
                // Tuple element types come from one of two shapes:
                //   - `Type::Tuple([T1, T2, ...])` — direct tuple type
                //     (uncommon for for-loops: would require iterating
                //     over a "tuple of …" rather than a vector<tuple>).
                //   - `Type::Reference(d_nr, _)` where `def(d_nr).name`
                //     starts with `__tuple<` — synthetic struct created
                //     by `tuple_def`; element types live as attributes.
                //     This is the common shape for `vector<(T1, T2)>`
                //     iteration, mirroring P189b's element-access path
                //     in `src/parser/operators.rs:608-658`.
                let (elem_types_opt, ref_def_nr): (Option<Vec<Type>>, u32) = match &var_tp {
                    Type::Tuple(elems) => (Some(elems.clone()), u32::MAX),
                    Type::Reference(d_nr, _)
                        if self.data.def(*d_nr).name().starts_with("__tuple<") =>
                    {
                        let elems: Vec<Type> = self
                            .data
                            .def(*d_nr)
                            .attributes
                            .iter()
                            .map(|a| a.typedef.clone())
                            .collect();
                        (Some(elems), *d_nr)
                    }
                    _ => (None, u32::MAX),
                };
                if let Some(elem_types) = elem_types_opt {
                    if elem_types.len() == names.len() {
                        // Build per-element read.  Two shapes:
                        //   - Direct Tuple: `Value::TupleGet(for_var, i)`
                        //     — reads from the var's stack-resident
                        //     tuple slot.
                        //   - Reference(__tuple<…>): use `get_val` with
                        //     the synthetic struct's per-attribute byte
                        //     offset — same path P189b's `.0` / `.1`
                        //     element access takes.
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, name)| {
                                let elem_tp = elem_types[i].clone();
                                let var = self.create_var(name, &elem_tp);
                                self.vars.defined(var);
                                self.vars.in_use(var, true);
                                let read = if ref_def_nr == u32::MAX {
                                    Value::TupleGet(for_var, i as u16)
                                } else {
                                    let elem_offset = if let Some(offs) =
                                        crate::data::stored_tuple_offsets_for_def(
                                            &self.data,
                                            &self.database,
                                            ref_def_nr,
                                            elem_types.len(),
                                        ) {
                                        u32::from(offs[i])
                                    } else {
                                        crate::data::element_stack_offsets(&elem_types)[i] as u32
                                    };
                                    self.get_val(
                                        &elem_tp,
                                        false,
                                        elem_offset,
                                        Value::Var(for_var),
                                        u32::MAX,
                                    )
                                };
                                v_set(var, read)
                            })
                            .collect()
                    } else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "for-destructure: pattern has {} names but iterated tuple has {} elements",
                                names.len(),
                                elem_types.len()
                            );
                        }
                        Vec::new()
                    }
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "for-destructure requires a tuple element type, got {}",
                            var_tp.name(&self.data)
                        );
                    }
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            // Extract the generator var (first arg of OpCoroutineNext) before
            // `iter_next` is consumed by `for_next` — @P327 needs it for the
            // tuple-yield exhaustion check below.
            //
            // This ALSO decides whether the loop is driving a coroutine at all, because the
            // advance `iterator()` emitted is the only thing that knows.  The subject's type
            // cannot answer it: a range `1..5` is typed `Type::Iterator` too, but arrives as
            // an inline `Value::Iter` that carries its own bound test and never reaches the
            // coroutine path — reading coroutine-ness off the type gave every range loop a
            // second, redundant exhaustion break.  Matching the op by name rather than any
            // call-with-a-var keeps that answer exact.
            let coroutine_next_op = self.data.def_nr("OpCoroutineNext");
            let gen_var = if let Value::Call(d_nr, args) = &iter_next
                && *d_nr == coroutine_next_op
                && let Some(Value::Var(v)) = args.first()
            {
                *v
            } else {
                u16::MAX
            };
            let is_coroutine_loop = gen_var != u16::MAX;
            // #481 (the #306-message half): a heap-ref coroutine yield is a
            // DbRef INTO the generator's state store — both for yielded views
            // of generator locals AND for records constructed in the
            // generator (they allocate in the state store, which is freed
            // with the generator).  Bind the loop var as a BORROW of the
            // generator (dep on gen_var): the consumer's scope machinery
            // must never emit a per-iteration OpFreeRef for it.  Such a free
            // releases the generator's whole STATE STORE, not one record — the
            // store is then recycled under allocation pressure with the
            // generator still live, and the exhausted-null read trips the #306
            // stack-store guard well after the corruption.
            //
            // The test is EVERY type that REACHES a store, not the three obvious ones: a
            // keyed collection is handed over as a handle exactly as a `Reference` or
            // `Vector` is, and a TUPLE reaches one through its elements — `(integer, S)`
            // carries `S`'s handle in its second slot and is freed element by element
            // (`scopes::tuple_owned_elem_frees`).  `data::holds_dbref` is the one home for
            // that set (@FR-Col-Store), and it is the tuple-transparent question because
            // this arm's is the borrow question, not the layout one.
            //
            // ⚠ A short list here does not skip a nicety — it inverts this arm.  The loop
            // var binds WITHOUT the dep, the scope machinery reads it as an owner, and the
            // per-iteration free this arm exists to PREVENT is exactly what gets emitted.
            // Measured with the tuple spelling outside the set: `for t in g()` over an
            // `iterator<(integer, S)>` freed the generator's whole extensible frame store
            // once per iteration, four frees of one live store across four iterations, the
            // values surviving only on the allocator handing the slot straight back.
            //
            // The dep is attached with `Type::with_deps`, which is the declared home for
            // "this type carrying this borrow" and already states how each variant holds
            // one — including a tuple, which has no list of its own and spreads the dep to
            // its elements for `Type::depend` to union back.  A `match` restating the
            // variants here was a THIRD copy of the set inside one `if`, and its
            // `other => other` fall-through is silent: the type it cannot spell binds
            // unchanged and the arm reads as taken (@FR-O-Proxy).
            if gen_var != u16::MAX
                && matches!(in_type, Type::Iterator(_, _))
                && crate::data::holds_dbref(&var_tp)
            {
                let dep_tp = var_tp.with_deps(&crate::data::Deps::frame1(gen_var));
                self.change_var_type(for_var, &dep_tp);
            }
            // @PLN93 (#511): iterating a CAPTURED collection (`for e in h`, `h` captured
            // into this lambda).  The element is a DbRef INTO the captured (shared) store,
            // reached via the hidden `__closure` param — exactly the #481 coroutine shape.
            // Bind the loop var as a BORROW of the closure so the scope machinery never
            // emits a per-iteration OpFreeRef for it: that free calls `free_named` on the
            // element's store_nr, which whole-store-frees the shared collection — a later
            // closure capturing the same collection then reads an empty store (native
            // only; interp already treats the element as a borrow).
            if self.closure_param != u16::MAX
                && !self.first_pass
                && Self::is_collection_type(&in_type)
                && in_type.depend().contains(&self.closure_param)
                && matches!(
                    var_tp,
                    Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                )
            {
                let dep = crate::data::Deps::frame1(self.closure_param);
                let dep_tp = match var_tp.clone() {
                    Type::Reference(d, _) => Type::Reference(d, dep),
                    Type::Enum(d, m, _) => Type::Enum(d, m, dep),
                    Type::Vector(e, _) => Type::Vector(e, dep),
                    other => other,
                };
                self.change_var_type(for_var, &dep_tp);
            }
            // For length-based vector termination below: pull the collection the
            // element fetch actually reads from (the materialised iteration temp),
            // not the original collection expression.  Re-reading the original each
            // iteration would re-run a side-effecting source (`for x in make()`); the
            // fetch temp is already materialised once, so its length is cheap + stable.
            let vec_fetch_coll = if matches!(in_type, Type::Vector(_, _)) {
                fn find_vec_coll(v: &Value, gvn: u32, vrn: u32) -> Option<Value> {
                    match v {
                        Value::Call(op, args) if *op == gvn || *op == vrn => args.first().cloned(),
                        Value::Call(_, args)
                        | Value::Insert(args)
                        | Value::Tuple(args)
                        | Value::Parallel(args) => {
                            args.iter().find_map(|a| find_vec_coll(a, gvn, vrn))
                        }
                        Value::Block(bl) | Value::Loop(bl) => {
                            bl.operators.iter().find_map(|o| find_vec_coll(o, gvn, vrn))
                        }
                        Value::If(c, t, e) => find_vec_coll(c, gvn, vrn)
                            .or_else(|| find_vec_coll(t, gvn, vrn))
                            .or_else(|| find_vec_coll(e, gvn, vrn)),
                        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) | Value::Yield(x) => {
                            find_vec_coll(x, gvn, vrn)
                        }
                        Value::Span(b) => find_vec_coll(&b.1, gvn, vrn),
                        _ => None,
                    }
                }
                let gvn = self.data.def_nr("OpGetVectorNullable");
                let vrn = self.data.def_nr("OpVectorRefNullable");
                find_vec_coll(&iter_next, gvn, vrn)
            } else {
                None
            };
            // loft#755 — the text the character read actually walks, captured
            // before `iter_next` is consumed, so the loop's bound and its
            // reader answer for the same text.
            let text_fetch_coll = if matches!(in_type, Type::Text(_)) {
                let tcn = self.data.def_nr("OpTextCharacterNullable");
                find_text_coll(&iter_next, tcn)
            } else {
                None
            };
            let for_next = v_set(for_var, iter_next);
            self.vars.loop_var(for_var);
            let in_loop = self.in_loop;
            self.in_loop = true;
            let mut block = Value::Null;
            let loop_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            self.parse_block("for", &mut block, &Type::Void);
            if id == "_"
                && let Some(prev) = outer_discard
            {
                self.vars.set_name("_", prev);
            }
            // P235 step 3: prepend the destructure Set ops so each
            // iteration unpacks the loop var into the user-named binders
            // before the user's body runs.
            if !destructure_setup.is_empty() {
                if let Value::Block(ref mut bl) = block {
                    for s in destructure_setup.into_iter().rev() {
                        bl.operators.insert(0, s);
                    }
                } else {
                    let inner = std::mem::replace(&mut block, Value::Null);
                    let mut ops = destructure_setup;
                    ops.push(inner);
                    block = v_block(ops, Type::Void, "destructure_for_body");
                }
            }
            self.vars.restore_write_state(&loop_write_state);
            let count = self.vars.loop_counter();
            self.in_loop = in_loop;
            self.vars.finish_loop(loop_nr);
            let mut for_steps = Vec::new();
            if fill != Value::Null {
                for_steps.push(fill);
            }
            // For text loops, initialise {id}#index at the FOR block scope so its live
            // interval covers the entire loop (not just the inner "for text next" block).
            if let Some(idx_var) = pre_var {
                for_steps.push(v_set(idx_var, Value::Int(0)));
            }
            for_steps.push(create_iter);
            let mut lp = vec![for_next];
            // CO1.5b: coroutine iterators also need a termination check.
            //
            // @P327 — for-yield types without a single-value null sentinel
            // (Tuple today; the same shape would catch any future composite
            // yielded type) MUST check the iterator's exhausted state, NOT
            // the yielded value.  Without this branch, `convert(tuple,
            // Boolean)` finds no OpConv* match → `test_for` stays as
            // `Var(for_var)` → `OpNot(tuple)` reads one byte of the tuple's
            // storage as boolean and inverts → silent wrong answer (loop
            // iterates 0 or N times depending on which byte aligns to 0).
            // OpCoroutineExhausted(gen) reads the coroutine status, which
            // CoroutineNext sets to Exhausted on the post-last-yield call.
            // `gen_var` is captured above from `iter_next`'s first arg
            // (the coroutine path never assigns a slot to the index var).
            //
            // @PLAN16 phase 05 — closures (`Type::Function`) join tuples on
            // this path: a yielded fn-ref has no null sentinel that
            // `OpConv*FromX → OpNot` can drive — `OpNot(fnref)` reads bytes
            // of the 20-byte fn-ref slot as boolean (SIGBUS on interp,
            // E0600 on native).  Same fix: terminate via the coroutine's
            // own exhausted state.
            // #401 — every coroutine loop terminates via the iterator's own
            // exhausted state, NOT a value-sentinel.  The value-sentinel path
            // below (`convert(value, Boolean)` → `OpNot`) only terminates when
            // the yielded type's null sentinel matches the transport channel's
            // exhaustion sentinel (`i64::MIN`): true for int/text/ref, but NOT
            // for `float`/`single` (null = NaN) or `enum` — `coroutine_next`
            // returns `i64::MIN`, whose f64 bit-pattern is not NaN, so the break
            // never fires and the loop spins forever (and the interp codegen of
            // that doomed check hangs).  Originally this used the state check
            // only for composite yields (Tuple/Function, which have no
            // single-value sentinel at all); it is correct for every element type.
            if is_coroutine_loop && gen_var != u16::MAX {
                let test_exhausted = self.cl("OpCoroutineExhausted", &[Value::Var(gen_var)]);
                lp.push(v_if(
                    test_exhausted,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            } else if matches!(in_type, Type::Vector(_, _))
                && matches!(&var_tp, Type::Function(_, _, _))
            {
                // @P343: a `vector<fn(...)>` loop terminates by testing the
                // d_nr half of the loop's fn-ref (see the
                // `fn_ref_field_read` branch in `iterator()`): break once it
                // is no longer a valid (callable, > 0) function reference.
                // At out-of-bounds the element read sets the d_nr to the
                // backend's invalid sentinel — i64::MIN on the interpreter
                // (`OpGetInt4` on the `OpGetVectorNullable` null sentinel)
                // and 0 on `--native` (that i64::MIN truncates to `0u32` in
                // the `(u32, DbRef)` fn-ref tuple, which is also native's
                // "invalid fn-ref" marker).  Both are `<= 0`, and every
                // real fn d_nr is `> 0`, so `d_nr > 0` (encoded as
                // `OpLtInt(0, d_nr)`) is the one test that terminates
                // correctly on both backends.  `convert(Function, Boolean)`
                // has no rule, so the generic branch below would leave the
                // raw 20-byte fn-ref for `OpNot` to misread — testing the
                // always-null closure half, which broke the loop on
                // iteration 0.
                let dnr = Value::FnRefDnr(for_var);
                let in_bounds = self.cl("OpLtInt", &[Value::Int(0), dnr]);
                let test_for = self.cl("OpNot", &[in_bounds]);
                lp.push(v_if(
                    test_for,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            } else if matches!(in_type, Type::Vector(_, _)) {
                // Length-based termination: yield exactly len(coll) elements,
                // independent of the element value, so a null ELEMENT (which shares
                // the OOB null sentinel) no longer ends the loop early.  The length is
                // re-read EACH iteration (not hoisted) so in-loop `x#remove` — which
                // shrinks the vector and decrements the index (state/io.rs remove) —
                // still terminates: when the vector drains, len falls to the index.
                // Direction-agnostic: forward ends at index == len, reverse ends at
                // index < 0 (the i32::MIN sentinel the reverse step sets).
                let coll = vec_fetch_coll
                    .clone()
                    .unwrap_or_else(|| orig_coll_expr.clone());
                let len = self.cl("OpLengthVector", std::slice::from_ref(&coll));
                let past_end = self.cl("OpLeInt", &[len, Value::Var(iter_var)]);
                lp.push(v_if(
                    past_end,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
                let before_start = self.cl("OpLtInt", &[Value::Var(iter_var), Value::Int(0)]);
                lp.push(v_if(
                    before_start,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            } else if matches!(in_type, Type::Text(_))
                && let Some(idx) = pre_var
            {
                // loft#755 — position-based termination: yield exactly the
                // text's characters, whatever they are, so an embedded NUL no
                // longer reads as end-of-text.  See `text_loop_break`.
                let coll = text_fetch_coll
                    .clone()
                    .unwrap_or_else(|| orig_coll_expr.clone());
                for step in self.text_loop_break(&coll, idx) {
                    lp.push(step);
                }
            } else if !matches!(in_type, Type::Iterator(_, _)) || is_coroutine_loop {
                // "Has the iterator run out?" is the question `x == null` asks, so it is
                // asked where every other spelling of it is asked — `null_test`, which
                // documents itself as the ONE place that answers *what is `τ`'s null*.
                //
                // Reaching for `convert(τ, Boolean)` instead was a second spelling, and it
                // silently disagreed for every item type whose truthiness rule is written
                // against the NULLABLE form.  A custom iterator's loop variable is
                // deliberately typed as the non-null item (@PLN102 D1 — the body only ever
                // binds a present value), so the test saw the bare type: `vector` and
                // struct-enum items got no conversion at all and `OpNot` inverted the raw
                // handle, ending the loop before its first iteration.  Four of the ten item
                // types a `next` can declare returned zero elements and exit 0 — no
                // diagnostic, no crash (loft#1310).
                //
                // `None` is the answer for the types that have no dedicated test — an
                // `integer`, a `text`, a bare `Reference` — and those take `null_test`'s
                // own documented fallthrough, the comparison against the TYPED null.
                let test_for =
                    if let Some(is_null) = self.null_test(Value::Var(for_var), &var_tp, false) {
                        is_null
                    } else {
                        // `null_test`'s documented fallthrough: the types with no dedicated
                        // test answer it by comparing against the TYPED null.  Reaching for
                        // `convert(τ, Boolean)` here instead asked whether the item was FALSY,
                        // which is a different question that merely coincides for most types —
                        // an `integer`'s conversion is `!= i64::MIN` and a `text`'s is
                        // out-of-band, so a `0` and an `""` element correctly kept iterating.
                        // A `boolean`'s conversion is the IDENTITY, and its null is the
                        // three-state `255` (C73), so the loop broke on the first `false`
                        // element it was handed and yielded nothing at all.
                        let mut t = Value::Var(for_var);
                        self.call_op(
                            &mut t,
                            "==",
                            &[Value::Var(for_var), Value::Null],
                            &[var_tp.clone(), Type::Null],
                        );
                        t
                    };
                lp.push(v_if(
                    test_for,
                    v_block(vec![Value::Break(0)], Type::Void, "break"),
                    Value::Null,
                ));
            }
            if if_step != Value::Null {
                lp.push(v_if(if_step, Value::Null, Value::Continue(0)));
            }
            lp.push(block);
            if count != u16::MAX {
                for_steps.insert(0, v_set(count, Value::Int(0)));
                lp.push(v_set(
                    count,
                    self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
                ));
            }
            for_steps.push(v_loop(lp, "For loop"));
            // on=4 epilogue — free a read-only source's DEDICATED scratch store on loop
            // exit.  Runs on completion AND break (break leaves the v_loop to here), and
            // once per (re-)entry so a loop-in-loop frees each build.  Conditional:
            // OpFreeScratch frees only when the scratch store differs from the source
            // recorded in the header, so a co-located scratch (writable source) is a
            // no-op.  A `return` out of the loop still bypasses this — a bounded residual
            // (expose-iteration-scratch.md Open question A).
            if hash_scratch_var != u16::MAX {
                for_steps.push(self.cl("OpFreeScratch", &[Value::Var(hash_scratch_var)]));
                // Null the scratch var so the scope-exit OpFreeScratch (emitted by
                // get_free_vars, which catches the `return`-out-of-loop path) is a no-op
                // here — its rec==0 guard skips a nulled ref, so the two never double-free.
                for_steps.push(v_set(hash_scratch_var, Value::Null));
            }
            *code = v_block(for_steps, Type::Void, "For block");
        } else {
            diagnostic!(self.lexer, Level::Error, "Expect variable after for");
        }
    }

    /// Plan-06 phase 4d.B — materialise a keyed-collection input
    /// (`sorted/hash/index/spatial<T[key]>`) into a temporary
    /// `vector<reference<T>>` so the par dispatcher's flat-vector
    /// path can iterate it.  Returns `(fill_ir, mat_var, mat_in_type)`
    /// or None if the source can't be unwrapped.  Mirrors the IR
    /// shape that the parser emits for the manual workaround
    /// `refs += [s]`: OpPreAllocVector + OpNewRecord + OpCopyRecord
    /// + OpFinishRecord per loop iteration.
    pub(crate) fn materialise_keyed_for_par(
        &mut self,
        in_type: &Type,
        source_expr: &Value,
    ) -> Option<(Value, u16, Type)> {
        let (content_d, dep) = match in_type {
            Type::Sorted(c, _, dep)
            | Type::Hash(c, _, dep)
            | Type::Index(c, _, dep)
            | Type::Radix(c, _, dep)
            | Type::Trie(c, _, dep) => (*c, dep.clone()),
            _ => return None,
        };
        let elem_ref_tp = Type::Reference(content_d, dep);
        let vec_ref_tp = Type::Vector(Box::new(elem_ref_tp.clone()), crate::data::Deps::none());
        let mat_var = self.create_unique("__par_mat", &vec_ref_tp);
        self.vars.defined(mat_var);
        // Register the wrapper struct EARLY (both passes) so the
        // typedef pass between pass 1 and pass 2 runs fill_database
        // and assigns a real `known_type`.
        let _ = self.data.vector_def(&mut self.lexer, &elem_ref_tp);
        if self.first_pass {
            return Some((Value::Null, mat_var, vec_ref_tp));
        }
        // Allocate the backing store for __par_mat.
        let db_setup = self.vector_db(&elem_ref_tp, mat_var);
        let iter_idx = self.create_unique("__par_mat_idx", &I32);
        self.vars.defined(iter_idx);
        let elm_var = self.create_unique("__par_mat_e", &elem_ref_tp);
        self.vars.defined(elm_var);
        // Drive the keyed-collection iterator via iterator() — same
        // helper that powers `for x in sorted_items`.
        let mut create_iter = source_expr.clone();
        let it_marker = Type::Iterator(Box::new(elem_ref_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, in_type, &it_marker, iter_idx, None);
        if iter_next == Value::Null {
            return None;
        }
        // Build the per-element append IR.  Mirrors the manual case
        // `refs += [s]` which lowers to:
        //   OpPreAllocVector(refs, 1, elem_size)
        //   tmp = OpNewRecord(refs, vec_tp, u16::MAX)
        //   OpCopyRecord(elm_var, tmp, content_tp)
        //   OpFinishRecord(refs, tmp, vec_tp, u16::MAX)
        let content_known = self.data.def(content_d).known_type();
        let elem_size = if content_known == u16::MAX {
            8
        } else {
            i32::from(self.database.size(content_known))
        };
        let vec_known = i32::from(self.vector_of(&elem_ref_tp));
        let prealloc = self.cl(
            "OpPreAllocVector",
            &[Value::Var(mat_var), Value::Int(1), Value::Int(elem_size)],
        );
        let tmp_var = self.create_unique("__par_mat_t", &elem_ref_tp);
        self.vars.defined(tmp_var);
        let new_rec = self.cl(
            "OpNewRecord",
            &[
                Value::Var(mat_var),
                Value::Int(vec_known),
                Value::Int(i32::from(u16::MAX)),
            ],
        );
        let copy_rec = self.cl(
            "OpCopyRecord",
            &[
                Value::Var(elm_var),
                Value::Var(tmp_var),
                Value::Int(i32::from(content_known)),
            ],
        );
        let finish_rec = self.cl(
            "OpFinishRecord",
            &[
                Value::Var(mat_var),
                Value::Var(tmp_var),
                Value::Int(vec_known),
                Value::Int(i32::from(u16::MAX)),
            ],
        );
        let body = Value::Insert(vec![
            prealloc,
            v_set(tmp_var, new_rec),
            copy_rec,
            finish_rec,
        ]);
        // Loop body: read next via iter_next into elm_var, break on
        // null, otherwise append (body).
        let for_next = v_set(elm_var, iter_next);
        let mut lp = vec![for_next];
        let mut test_for = Value::Var(elm_var);
        self.convert(&mut test_for, &elem_ref_tp, &Type::Boolean);
        test_for = self.cl("OpNot", &[test_for]);
        lp.push(v_if(
            test_for,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        ));
        lp.push(body);
        // Assemble the materialisation Block.
        let mut for_steps: Vec<Value> = Vec::new();
        for s in &db_setup {
            for_steps.push(s.clone());
        }
        for_steps.push(create_iter);
        for_steps.push(v_loop(lp, "Materialise par input"));
        // Splice the steps inline (Insert), NOT a v_block: native codegen emits
        // a Block as a Rust `{ }` scope, which would confine the `__par_mat`
        // `let` to that scope so the following par dispatch can't see it
        // (E0425).  The vector-input path likewise feeds a bare statement, not a
        // scoped block.
        let fill_ir = Value::Insert(for_steps);
        let mat_in_type = vec_ref_tp.depending(mat_var);
        Some((fill_ir, mat_var, mat_in_type))
    }

    /// Materialise a non-keyed iterable — a range / `iterator<T>` / text — into
    /// a flat `vector<T>` so the par dispatcher's index-partitioned walk can run
    /// over it.  This is the same "iterate, append" that the comprehension
    /// `[for e in src { e }]` performs; we reuse `build_comprehension_code` so
    /// element append is correct per kind (scalar / text / ref) on both
    /// backends.  Returns `(fill_ir, mat_var, mat_in_type)`, or None when the
    /// source isn't one of these iterables (a flat `vector` is dispatched
    /// directly; keyed collections go through `materialise_keyed_for_par`).
    pub(crate) fn materialise_iter_for_par(
        &mut self,
        in_type: &Type,
        source_expr: &Value,
        loop_nr: u16,
    ) -> Option<(Value, u16, Type)> {
        if !matches!(in_type, Type::Iterator(_, _) | Type::Text(_)) {
            return None;
        }
        let elem_tp = self.for_type(in_type);
        let vec_tp = Type::Vector(Box::new(elem_tp.clone()), crate::data::Deps::none());
        // Register the element vector type EARLY (both passes) so the typedef
        // pass between pass 1 and pass 2 assigns it a real `known_type`.
        let _ = self.data.vector_def(&mut self.lexer, &elem_tp);
        // Name these vars by the stable `loop_nr` (NOT the global create_unique
        // counter): the materialise body is built pass-2-only, so a counter-named
        // var advances the counter only on pass 2, which desyncs numbering for
        // sibling materialise loops — two loops' `__par_mat` then collide on one
        // name (#282).  When their element types differ (e.g. integer range vs
        // character text-source) the merged var takes one type, so the other loop
        // reads its store at the wrong stride → silent garbage.  A loop-keyed name
        // is unique per loop and identical across both passes, so no collision.
        let mk = |p: &mut Self, name: &str, tp: &Type| -> u16 {
            let v = p
                .vars
                .add_variable(&format!("_par_{name}_l{loop_nr}"), tp, &mut p.lexer);
            p.vars.defined(v);
            v
        };
        let mat_var = mk(self, "mat", &vec_tp);
        if self.first_pass {
            return Some((Value::Null, mat_var, vec_tp));
        }
        // Iterator state vars — text drives a (pos, index) pair, every other
        // iterator a single index — mirroring parse_vector_for.
        let (iter_var, pre_var) = if matches!(in_type, Type::Text(_)) {
            let pos = mk(self, "mat_next", &I32);
            let idx = mk(self, "mat_index", &I32);
            (pos, Some(idx))
        } else {
            let iv = mk(self, "mat_index", &I32);
            (iv, None)
        };
        let for_var = mk(self, "mat_e", &elem_tp);
        let mut create_iter = source_expr.clone();
        let it = Type::Iterator(Box::new(elem_tp.clone()), Box::new(Type::Null));
        let iter_next = self.iterator(&mut create_iter, in_type, &it, iter_var, pre_var);
        if iter_next == Value::Null {
            return None;
        }
        let for_next = v_set(for_var, iter_next);
        let elm = self.unique_elm_var(&vec_tp, &elem_tp, mat_var);
        // Identity comprehension `[for e in src { e }]` filling mat_var.  is_var
        // mode emits a Value::Insert (no Rust `{ }` scope), so the `let mat_var`
        // lands in the enclosing function scope where par dispatch reads it —
        // the same scoping the keyed materialiser relies on.
        let mut fill_expr = Value::Var(mat_var);
        self.build_comprehension_code(
            mat_var,
            &Value::Var(mat_var),
            elm,
            &elem_tp,
            in_type,
            &elem_tp,
            for_var,
            for_next,
            pre_var,
            if matches!(in_type, Type::Vector(_, _)) {
                // `source_expr`, not `mat_var` — `mat_var` is the destination this
                // loop FILLS, and its length grows every iteration.
                Some((source_expr.clone(), iter_var))
            } else {
                None
            },
            Value::Null,
            create_iter,
            Value::Null,
            Value::Var(for_var),
            &mut fill_expr,
            true,
            false,
            false,
            vec_tp.clone(),
        );
        let mat_in_type = self.vars.tp(mat_var).clone();
        Some((fill_expr, mat_var, mat_in_type))
    }

    /// P235 par half — parse the worker call inside a destructured
    /// par expression and synthesize a wrapper fn that bridges the
    /// gap between par dispatch's "one per-iteration arg" model and
    /// the user's multi-arg-from-tuple call shape.
    ///
    /// Given `for (a, b) in pairs par(r = work(a, b), N) { ... }`
    /// (a, b already defined in scope as tuple-element vars by
    /// `parse_parallel_for_loop`), this method parses `work(a, b)`
    /// and synthesizes:
    ///
    /// ```text
    /// fn __par_destructure_w_<N>(t: tuple_type) -> ret_type {
    ///     work(t.<a_idx>, t.<b_idx>, ...)
    /// }
    /// ```
    ///
    /// Each user arg that is `Var(destructure_var_nrs[i])` becomes
    /// a tuple element read at the matching tuple position; other
    /// args (e.g. context constants) pass through verbatim.  The
    /// par dispatch then calls the wrapper with the tuple loop
    /// element as its single per-iteration arg.
    ///
    /// Returns the wrapper's d_nr (or u32::MAX on error), the
    /// return type, and empty extras (no context args — they're
    /// baked into the wrapper body).
    pub(crate) fn parse_destructure_par_worker(
        &mut self,
        tuple_tp: &Type,
        destructure_var_nrs: &[u16],
    ) -> (u32, Type, Vec<Value>, Vec<Type>) {
        // Parse worker fn name + ( arg, arg, ... )
        let Some(work_id) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect function name in par(...) destructure worker"
                );
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        };
        let work_d_nr = {
            let prefixed = format!("n_{work_id}");
            let nr = self.data.def_nr(&prefixed);
            if nr == u32::MAX {
                self.data.def_nr(&work_id)
            } else {
                nr
            }
        };
        if !self.lexer.has_token("(") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect '(' after worker name '{work_id}'"
                );
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        }
        let mut user_args: Vec<Value> = Vec::new();
        if !self.lexer.peek_token(")") {
            loop {
                let mut arg = Value::Null;
                self.expression(&mut arg);
                user_args.push(arg);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
        }
        self.lexer.token(")");

        if work_d_nr == u32::MAX {
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Unknown function '{work_id}'");
            }
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        }
        if !self.first_pass && !matches!(self.data.def_type(work_d_nr), DefType::Function) {
            diagnostic!(self.lexer, Level::Error, "'{work_id}' is not a function");
            return (u32::MAX, Type::Unknown(0), Vec::new(), Vec::new());
        }
        let ret_type = self.data.def(work_d_nr).returned().clone();
        if self.first_pass {
            // First pass: return the user worker so the parser's
            // downstream type-shape decisions (return_size, ladder
            // routing) see realistic values.  Argcount checks in
            // build_parallel_for_ir are gated on !first_pass, so
            // mismatched extras don't fire here.  Second pass
            // synthesizes the real wrapper.
            return (work_d_nr, ret_type, Vec::new(), Vec::new());
        }
        self.data.def_used(work_d_nr);

        // Resolve tuple element types + def_nr for offsets
        let elem_types: Vec<Type> = match tuple_tp {
            Type::Tuple(elems) => elems.clone(),
            Type::Reference(d, _) => self
                .data
                .def(*d)
                .attributes
                .iter()
                .map(|a| a.typedef.clone())
                .collect(),
            _ => Vec::new(),
        };
        let tuple_d_nr = match tuple_tp {
            Type::Reference(d, _) => *d,
            _ => u32::MAX,
        };

        // Allocate wrapper def
        let wrapper_pos = self.lexer.pos().clone();
        let wrapper_file = wrapper_pos.file.clone();
        // Use lexer line:col + work fn name for a stable, unique
        // synthetic name (avoids needing a Parser-level counter).
        let wrapper_name = format!(
            "__par_destructure_w_{}_{}_{work_id}",
            wrapper_pos.line, wrapper_pos.pos
        );
        let wrapper_d_nr = self
            .data
            .add_def(&wrapper_name, &wrapper_pos, DefType::Function);
        let _ = self
            .data
            .add_attribute(&mut self.lexer, wrapper_d_nr, "t", tuple_tp.clone());
        self.data.set_returned(wrapper_d_nr, ret_type.clone());

        // Build wrapper variable table
        let mut wrapper_vars = Function::new(&wrapper_name, &wrapper_file);
        let t_var = wrapper_vars.add_variable("t", tuple_tp, &mut self.lexer);
        wrapper_vars.become_argument(t_var);
        wrapper_vars.defined(t_var);

        // Translate user_args: Var(destructure_var_nrs[i]) → tuple
        // element read; other shapes pass through verbatim.
        let mut wrapper_call_args: Vec<Value> = Vec::with_capacity(user_args.len());
        for arg in &user_args {
            let arg_var_nr = if let Value::Var(v) = arg.unspan() {
                Some(*v)
            } else {
                None
            };
            if let Some(v) = arg_var_nr
                && let Some(idx) = destructure_var_nrs.iter().position(|&dv| dv == v)
            {
                let elem_offset = if tuple_d_nr == u32::MAX {
                    crate::data::element_stack_offsets(&elem_types)[idx] as u32
                } else if let Some(offs) = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    tuple_d_nr,
                    elem_types.len(),
                ) {
                    u32::from(offs[idx])
                } else {
                    crate::data::element_stack_offsets(&elem_types)[idx] as u32
                };
                let read = self.get_val(
                    &elem_types[idx],
                    false,
                    elem_offset,
                    Value::Var(t_var),
                    u32::MAX,
                );
                wrapper_call_args.push(read);
            } else {
                wrapper_call_args.push(arg.clone());
            }
        }

        let body_call = Value::Call(work_d_nr, wrapper_call_args);
        let body = v_block(
            vec![Value::Return(Box::new(body_call))],
            ret_type.clone(),
            "destructure_wrapper",
        );
        self.data.definitions[wrapper_d_nr as usize].code = body;
        self.data.definitions[wrapper_d_nr as usize].variables = wrapper_vars;

        (wrapper_d_nr, ret_type, Vec::new(), Vec::new())
    }

    // Desugar `for a in vec par(b = worker(a), N) { body }` into an
    // index-based loop over the `parallel_for` result vector.
    #[allow(clippy::too_many_arguments)]
    /// `elem_var` is the name this loop BINDS, `src_elem_var` the name the program wrote
    /// — they differ from the second loop over a name onward (loft#915).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_parallel_for_loop(
        &mut self,
        code: &mut Value,
        elem_var: &str,
        src_elem_var: &str,
        in_type: &Type,
        vec_expr: &Value,
        fill: Value,
        loop_nr: u16,
        destructure_names: Option<&[String]>,
    ) {
        // Consume opening '('.
        self.lexer.token("(");

        // Validate: parallel syntax requires a vector input.
        let elem_tp = if let Type::Vector(_, _) = in_type {
            self.for_type(in_type)
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par(...) requires a vector<T> input, not {}",
                    in_type.name(&self.data)
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            // A `for … par(…) { … }` is a STATEMENT whichever way its parse ends; see
            // the note on the unresolved-worker exit in `build_parallel_for_ir`.
            *code = v_block(Vec::new(), Type::Void, "par (clause not parsed)");
            return;
        };

        // Parse: result_name = worker_call , threads )
        let Some(result_name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect result variable name after 'par('"
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            // A `for … par(…) { … }` is a STATEMENT whichever way its parse ends; see
            // the note on the unresolved-worker exit in `build_parallel_for_ir`.
            *code = v_block(Vec::new(), Type::Void, "par (clause not parsed)");
            return;
        };
        if !self.lexer.has_token("=") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect '=' after result name '{}' in par(...)",
                    result_name
                );
            }
            self.skip_to_parallel_body();
            self.vars.finish_loop(loop_nr);
            // A `for … par(…) { … }` is a STATEMENT whichever way its parse ends; see
            // the note on the unresolved-worker exit in `build_parallel_for_ir`.
            *code = v_block(Vec::new(), Type::Void, "par (clause not parsed)");
            return;
        }

        // Create the element variable so the worker call expression can resolve it.
        // (e.g. `calc(a)` needs `a` in scope during parsing even though the body
        // never runs `a` directly — the parallel map handles that.)
        //
        // Plan-04 B.3 follow-up: the body of
        // `for a in items par(b = worker(a), N) { ... a.iv ... }`
        // is parsed against this `elem_var_nr`, but the desugared
        // loop iterates over an `idx` counter and never writes `a` —
        // so the slot allocator would never place it.  `a` is treated
        // as an inline alias for `OpGetVector(items, idx)`, same as
        // `b` → `OpGetVector(results, idx)`.  build_parallel_for_ir
        // performs the actual Var→accessor rewrite after body parse,
        // once `idx_var` exists.
        let elem_var_nr = self.create_var(elem_var, &elem_tp);
        // The body reads the name the program wrote; point it at this loop's binding
        // and leave it there, the same as the sequential form (loft#915).
        if elem_var != src_elem_var {
            self.vars.set_name(src_elem_var, elem_var_nr);
        }
        self.vars.defined(elem_var_nr);
        if matches!(elem_tp, Type::Integer(_)) {
            self.vars.in_use(elem_var_nr, true);
        }

        // P235 par half: when the for-loop binds a tuple destructure
        // (`for (a, b) in pairs par(r = work(a, b), N) { ... }`),
        // the destructured names need to be in scope BEFORE
        // parse_parallel_worker parses the worker call.  Define them
        // here as proper variables typed from the tuple's element
        // types — same pattern as the non-par destructure setup at
        // parse_for:1289-1346.  The variables persist through the
        // body too (matching non-par destructure semantics).
        //
        // The worker call itself is parsed via a destructure-aware
        // path (`parse_destructure_par_worker`) that captures ALL
        // user args (parse_parallel_worker dummies the first arg)
        // and synthesizes a wrapper fn `__par_destructure_w_<N>(t)
        // -> ret { work(t.0, t.1, ...) }`.  Par dispatch then calls
        // the wrapper with the tuple loop element as its single arg.
        let destructure_var_nrs: Option<Vec<u16>> = if let Some(names) = destructure_names {
            let elem_types_opt: Option<Vec<Type>> = match &elem_tp {
                Type::Tuple(elems) => Some(elems.clone()),
                Type::Reference(d_nr, _) if self.data.def(*d_nr).name().starts_with("__tuple<") => {
                    Some(
                        self.data
                            .def(*d_nr)
                            .attributes
                            .iter()
                            .map(|a| a.typedef.clone())
                            .collect(),
                    )
                }
                _ => None,
            };
            if let Some(elem_types) = elem_types_opt {
                if elem_types.len() == names.len() {
                    Some(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, name)| {
                                let var = self.create_var(name, &elem_types[i]);
                                self.vars.defined(var);
                                self.vars.in_use(var, true);
                                var
                            })
                            .collect(),
                    )
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "for-destructure: pattern has {} names but iterated tuple has {} elements",
                            names.len(),
                            elem_types.len()
                        );
                    }
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Resolve worker function: consumes the worker call tokens up to the ','.
        let (fn_d_nr, ret_type, extra_vals, _extra_types) =
            if let Some(d_var_nrs) = destructure_var_nrs.as_ref() {
                self.parse_destructure_par_worker(&elem_tp, d_var_nrs)
            } else {
                self.parse_parallel_worker(src_elem_var, elem_var_nr, &elem_tp)
            };
        // loft#808 — the one place BOTH worker-resolution shapes converge, so it is
        // where "this def is a par worker" is recorded.  A pure-value tuple return
        // keeps Rust's tuple ABI everywhere else; only a worker's still needs the
        // synthetic-`__tuple<…>` boxing, because the routes below carry a result
        // through buffers sized for ≤8-byte primitives / text / fn-refs / refs.
        // Recorded on pass 1 too (the destructure path answers with the USER worker
        // there, and with its synthesized wrapper on pass 2 — the wrapper inherits
        // the already-promoted return type, so both answers are the right one to
        // record).  See `Parser::par_worker_defs`.
        if fn_d_nr != u32::MAX {
            self.par_worker_defs.insert(fn_d_nr);
        }

        // Plan-06 phase 5b' — par-safety DEEP check at ERROR level.
        // Recurses through user-fn callees until it hits a direct
        // call to a `Purity::Impure(ParentWrite)` stdlib fn, or
        // until every path bottoms out in pure/host_io/prng/io/
        // par_call/native primitives.  Unannotated declared-only
        // natives (Op*, n_*) are treated as safe — they're C-level
        // primitives, and the ones that DO write to parent state
        // are explicitly tagged in `default/01_code.loft`.
        //
        // Emits Level::Error: writes from a worker to parent state
        // silently vanish at thread join (D2.0).  The error gives
        // the full reachability chain so the user can see exactly
        // which helper introduces the offending call.
        if !self.first_pass
            && fn_d_nr != u32::MAX
            && let Some(chain) = crate::scopes::worker_calls_parent_write_deep(&self.data, fn_d_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "par() worker {} — a par worker's captured state is READ-ONLY (@PLN102 C93): a \
                     write to shared parent state is a data race, which loft disallows rather \
                     than run.  Return the value from the worker and accumulate it in the fold \
                     body instead.",
                chain
            );
        }

        // Comma separating worker from thread count.
        self.lexer.token(",");
        let mut threads_expr = Value::Null;
        self.expression(&mut threads_expr);
        // Closing ')'.
        self.lexer.token(")");

        // Compute element size from the return type.
        // return_size =  0 signals text mode to n_parallel_for.
        // return_size = -1 signals reference (struct) mode.
        let return_size: i32 = self.par_return_size(&ret_type, fn_d_nr);
        let elem_size = self.par_elem_size(&elem_tp);

        self.build_parallel_for_ir(
            code,
            &result_name,
            fn_d_nr,
            &ret_type,
            elem_size,
            return_size,
            vec_expr,
            threads_expr,
            fill,
            loop_nr,
            extra_vals,
            elem_var_nr,
            &elem_tp,
        );
    }

    // parallel_for IR builder; threads unrelated IR params alongside &mut self — no sensible grouping
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    /// The `return_size` a par route is built from: the worker's return width, with two
    /// sentinels — `0` for text (workers collect Strings and the main thread stores refs)
    /// and `-1` for a heap value (the worker builds it in its own store and the main
    /// thread deep-copies).  One home, because a monomorph re-derives it when it lowers a
    /// clause its template could not (loft#1040).
    pub(crate) fn par_return_size(&mut self, ret_type: &Type, fn_d_nr: u32) -> i32 {
        if matches!(ret_type, Type::Text(_)) {
            0 // sentinel: text mode — workers collect Strings, main thread stores refs
        } else if crate::data::is_dbref(ret_type) {
            // Reference mode — workers return a DbRef into their own
            // store; main deep-copies via copy_from_worker.  Plan-06
            // phase 1 G1: struct-enum returns (Enum variants with
            // payload, e.g. `Verdict::Pass{score}`) are heap-typed
            // (`heap_def_nr().is_some()`) so they share the ref path
            // verbatim.  This closes the size-8 gate for variant payloads.
            // Plan-06 phase 1 G6: vector<T> returns also route here —
            // the worker constructs the vector in its own output
            // store and the main thread deep-copies it via the same
            // copy_from_worker mechanism.
            // Plan-06 ARC.md A6.d: keyed collections (Sorted / Hash /
            // Index / Radix) are stored as DbRefs to their backing
            // records and route through the same ref path; the rebase
            // walk in `data::owned_elements` already enumerates their
            // internal owned-DbRef fields.
            -1
        } else {
            let sz = i32::from(var_size(ret_type, &Context::Argument));
            // Plan-06 phase 1 G4 — accept Type::Function returns
            // (size 20 = 8B d_nr + 12B closure DbRef).  Workers
            // write the 20-byte fn-ref into per-worker output
            // slots; main thread copies bytes back via the
            // execute_at_raw_to path in run_parallel_direct.
            let is_fn_ref = matches!(ret_type, Type::Function(_, _, _));
            if !self.first_pass && fn_d_nr != u32::MAX && (sz == 0 || (sz > 8 && !is_fn_ref)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Parallel worker return type '{}' (size {sz}) is not supported",
                    ret_type.name(&self.data)
                );
            }
            // A non-capturing fn-ref return (e.g. `return add5;`) is fine, but a
            // CAPTURING closure can't be returned from a par worker: its captured
            // environment lives in the worker's per-thread store and is dropped at
            // join, so the fn-ref would dangle.  Reject it with a clear message
            // instead of the raw out-of-bounds panic the dangling ref triggers.
            if is_fn_ref
                && !self.first_pass
                && fn_d_nr != u32::MAX
                && worker_returns_capturing_closure(self.data.def(fn_d_nr).code())
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a parallel worker cannot return a capturing closure — its \
                     captured state lives in the worker thread's store and cannot \
                     cross back.  Return a non-capturing function reference, or \
                     compute the result directly in the worker."
                );
            }
            sz.max(1) // fallback to 1 if unknown
        }
    }

    /// The `elem_size` a par route steps the INPUT vector by — the element's own storage
    /// stride, which is what the dispatcher partitions on.  One home, for the same reason
    /// as [`Self::par_return_size`].
    pub(crate) fn par_elem_size(&mut self, elem_tp: &Type) -> i32 {
        {
            let elm_td = self.data.type_elm(elem_tp);
            // Plan-06 phase 1 G2.1 — narrow-integer vector inputs
            // (vector<u8>, vector<i32>) store one element per
            // forced_size byte slot.  IntegerSpec::vector_narrow_width
            // returns 1/2/4 for u8/i16/i32 and matches the iterator
            // dispatch in collections.rs:105-113 for non-par for loops.
            // Without this, var_size() returned 8 for any Integer
            // and par read garbage at row_idx*8 instead of row_idx*4.
            if let Type::Integer(spec) = elem_tp
                && let Some(n) = spec.vector_narrow_width(false)
            {
                i32::from(n)
            } else if matches!(elem_tp, Type::Function(_, _, _)) {
                // Plan-06 phase 4d.A.2 — fn-ref vector storage is
                // 4-byte i32 d_nr (matches `data::element_stack_size(Type::Function)`).
                // The known_type / db_size lookup below would return
                // var_size(.., Argument) = 20 (the wide stack-slot
                // width for fn-refs), which is wrong for vector
                // stride.  Hard-code 4 here so par steps through the
                // vector in 4-byte increments matching `OpSetInt4`'s
                // narrow writes.
                4
            } else if matches!(elem_tp, Type::Vector(_, _)) {
                // A nested VECTOR element is stored as a 4-byte record index, not as the
                // inner element type it holds.  `type_elm` resolves `vector<integer>` to
                // `integer`, whose db size is 8, so par strode TWICE per row: over four rows
                // a worker saw rows 0 and 2 and then read past the end, answering the
                // element's default — `null` for a value, `0` for a length — with no
                // diagnostic, on both backends.  `vector<text>` was the one inner type that
                // worked, and only because `text`'s db size is 4 by coincidence (loft#1033).
                //
                // VECTOR and not `is_collection`: a `vector<hash<T[k]>>` cannot be built at
                // all today — the construction panics the interpreter before any stride
                // question arises (loft#1298) — so a keyed element's stride is unmeasured,
                // and a number nothing can check is not a fact to write down.
                4
            } else {
                let known = self.data.def(elm_td).known_type();
                let db_size = i32::from(self.database.size(known));
                if db_size > 0 {
                    db_size
                } else {
                    i32::from(var_size(elem_tp, &Context::Argument))
                }
            }
        }
    }

    /// True when this `par` clause must wait for a monomorph to be lowered — the
    /// enclosing function is a TEMPLATE and the clause's element or return type is its
    /// type VARIABLE, so every route decision here would be made for a 12-byte reference
    /// and left behind by substitution (loft#1040).
    ///
    /// Both halves are load-bearing.  A par inside a template over a CONCRETE vector
    /// (`for x in [1, 2] par(…)` in a generic function) has real types and lowers here as
    /// it always did; and outside a template there is no variable to wait for.
    fn defer_parametric_par(&mut self, ret_type: &Type, elem_tp: &Type) -> bool {
        if self.data.def_type(self.context) != DefType::Generic {
            return false;
        }
        let attrs = self.data.def(self.context).attributes();
        let tv = attrs
            .iter()
            .map(|a| Self::type_var_of(&self.data, &a.typedef))
            .find(|t| *t != u32::MAX)
            .unwrap_or(u32::MAX);
        if tv == u32::MAX {
            return false;
        }
        ret_type.contains_def(tv) || elem_tp.contains_def(tv)
    }

    /// The definition a deferred `par` marker calls.  It is a placeholder, never emitted:
    /// a template produces no code, and every monomorph replaces the marker with the real
    /// lowering before anything reads the body again.
    fn par_marker_def(&mut self) -> u32 {
        // `add_fn` mangles to `n_<name>`, so the lookup has to spell the MANGLED name —
        // asking for the source name found nothing on the second pass and minted a
        // `#dup`, which the H5 cross-pass guard reports as a real divergence.
        let existing = self.data.def_nr(PAR_MARKER_FN);
        if existing != u32::MAX {
            return existing;
        }
        let d_nr = self.data.add_fn(&mut self.lexer, "__par_template", &[]);
        if d_nr != u32::MAX {
            // VOID, so the emitted stub is valid Rust.  It is never called: every
            // monomorph replaces the marker before anything reads the body again, and a
            // TEMPLATE emits no code — but a definition with no declared return renders
            // as `-> ??` and refuses to compile, which would make a deferral break the
            // build of a program that merely CONTAINS one.
            self.data.definitions[d_nr as usize].returned = Type::Void;
        }
        d_nr
    }

    // parallel_for IR builder; threads unrelated IR params alongside &mut self — no sensible grouping
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_lines)]
    pub(crate) fn build_parallel_for_ir(
        &mut self,
        code: &mut Value,
        result_name: &str,
        fn_d_nr: u32,
        ret_type: &Type,
        elem_size: i32,
        return_size: i32,
        vec_expr: &Value,
        threads_expr: Value,
        fill: Value,
        loop_nr: u16,
        extra_args: Vec<Value>,
        elem_var: u16,
        elem_tp: &Type,
    ) {
        let ref_d_nr = self.data.def_nr("reference");
        let results_ref_type = Type::Reference(ref_d_nr, crate::data::Deps::none());
        let par_for_d_nr = self.data.def_nr("n_parallel_for");

        // Plan-06 PRIORITY.md spine step 8c — compute the queue gate
        // up front so we can avoid declaring a `par_results` slot for
        // the streaming queue path.  In 8b/c that slot was created
        // and `defined()` unconditionally; on the queue path nothing
        // ever wrote to it, so scope-analysis + `generate_block`'s
        // eval-stack residue check emitted a phantom `OpFreeStack(8)`
        // at the par-block tail.  The discard underflowed below the
        // function frame, corrupted `Return discard`'s wraparound,
        // and SIGSEGV'd on the resulting bogus `code_pos`.  Skipping
        // the slot keeps the codegen tracker honest.
        //
        // The buf_get / buf_drop d_nrs are looked up once here; the
        // dispatch site below clones from these.
        let queue_d_nr = self.data.def_nr("n_parallel_queue");
        let buf_get_d_nr = self.data.def_nr("n_parallel_buf_get");
        let buf_drop_d_nr = self.data.def_nr("n_parallel_buf_drop");
        let queue_narrow_d_nr = self.data.def_nr("n_parallel_queue_narrow");
        let buf_get_narrow_d_nr = self.data.def_nr("n_parallel_buf_get_narrow");
        let buf_drop_narrow_d_nr = self.data.def_nr("n_parallel_buf_drop_narrow");
        let queue_text_d_nr = self.data.def_nr("n_parallel_queue_text");
        let buf_get_text_d_nr = self.data.def_nr("n_parallel_buf_get_text");
        let buf_drop_text_d_nr = self.data.def_nr("n_parallel_buf_drop_text");
        let queue_ref_d_nr = self.data.def_nr("n_parallel_queue_ref");
        let buf_get_ref_d_nr = self.data.def_nr("n_parallel_buf_get_ref");
        let buf_drop_ref_d_nr = self.data.def_nr("n_parallel_buf_drop_ref");
        let queue_fn_d_nr = self.data.def_nr("n_parallel_queue_fn");
        let buf_get_fn_d_nr = self.data.def_nr("n_parallel_buf_get_fn");
        let buf_drop_fn_d_nr = self.data.def_nr("n_parallel_buf_drop_fn");
        let early_is_primitive_return = !matches!(
            ret_type,
            Type::Text(_)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Function(_, _, _)
                | Type::Vector(_, _)
                | Type::Unknown(_)
        );
        // ARC.md A3.6 — `float` (8B f64) joins 8B Integer on the
        // wide-Queue path; the body's read goes through
        // `parallel_buf_get_float` for typed `f64::from_bits` recovery.
        let early_ret_size_8 = matches!(ret_type, Type::Integer(spec) if u32::from(crate::variables::size(
            &Type::Integer(*spec),
            &Context::Argument,
        )) == 8)
            || matches!(ret_type, Type::Float);
        // Plan-06 ARC.md A3 / A3.5 — narrow primitive returns route
        // through n_parallel_queue_narrow.  The buf_get_narrow result
        // is i64; for non-Integer shapes we wrap with a conversion Op
        // that maps i64 → bool / character / enumerate.  Single /
        // Float (A3.6) still need new IR bit-cast Ops.
        //
        // Shape coverage:
        //   Integer narrow (forced_size 1/2/4) — A3, no wrap needed.
        //   Boolean — A3.5, wrap with OpConvBoolFromInt.
        //   Character — A3.5, wrap with OpConvCharacterFromInt.
        //   Enum (no payload) — A3.5, wrap with OpCastEnumFromInt.
        //   Single / Float — A3.6, deferred (no bit-cast IR Op).
        let early_narrow_route = narrow_route_for(ret_type);
        let early_route_int_queue = early_is_primitive_return
            && fn_d_nr != u32::MAX
            && early_ret_size_8
            && queue_d_nr != u32::MAX
            && buf_get_d_nr != u32::MAX
            && buf_drop_d_nr != u32::MAX;
        let early_route_narrow_queue = early_is_primitive_return
            && fn_d_nr != u32::MAX
            && early_narrow_route.is_some()
            && queue_narrow_d_nr != u32::MAX
            && buf_get_narrow_d_nr != u32::MAX
            && buf_drop_narrow_d_nr != u32::MAX;
        let early_route_text_queue = matches!(ret_type, Type::Text(_))
            && fn_d_nr != u32::MAX
            && queue_text_d_nr != u32::MAX
            && buf_get_text_d_nr != u32::MAX
            && buf_drop_text_d_nr != u32::MAX;
        // 8d.3: route Reference + struct-enum-payload + Vector
        // returns through Queue.  All three share the adopt-and-
        // rebase contract; the per-thread reserved-slot-range
        // allocator (8d.3 in `run_parallel_queue_ref`) ensures
        // worker-written DbRefs already live in parent namespace
        // so cross-worker collision is eliminated.
        let early_route_ref_queue = crate::data::is_dbref(ret_type)
            && fn_d_nr != u32::MAX
            && queue_ref_d_nr != u32::MAX
            && buf_get_ref_d_nr != u32::MAX
            && buf_drop_ref_d_nr != u32::MAX;
        // ARC.md A6.b — fn-ref returns route through a packed-buffer
        // queue (par_fn_buffer_stack) instead of the heap result
        // vector.  See run_parallel_queue_fn for the runtime; the
        // body substitution diverges below to emit
        // `Set(b_var, Call(buf_get_fn, [idx]))` per iteration
        // instead of inline-expanding b_var via replace_var_in_ir.
        let early_route_fn_queue = matches!(ret_type, Type::Function(_, _, _))
            && fn_d_nr != u32::MAX
            && queue_fn_d_nr != u32::MAX
            && buf_get_fn_d_nr != u32::MAX
            && buf_drop_fn_d_nr != u32::MAX;
        let early_route_through_queue = early_route_int_queue
            || early_route_narrow_queue
            || early_route_text_queue
            || early_route_ref_queue
            || early_route_fn_queue;

        // Create result-reference variable — only on the materialised
        // path.  The queue path stores results in
        // `Stores::par_buffer_stack` / `_text_buffer_stack` and never
        // touches a heap result vector.
        let results_var = if early_route_through_queue {
            None
        } else {
            let v = self.create_unique("par_results", &results_ref_type);
            self.vars.defined(v);
            Some(v)
        };

        // Create index variable (b#index).
        let idx_var = self.create_var(&format!("{result_name}#index"), &I32);
        self.vars.defined(idx_var);
        self.vars.in_use(idx_var, true);

        // Create length variable (par_len#N).
        let len_var = self.create_unique("par_len", &I32);
        self.vars.defined(len_var);
        self.vars.in_use(len_var, true);

        // Create the result element variable (b) with the worker's return type.
        //
        // An unresolved worker answers `(u32::MAX, Unknown)` for two OPPOSITE reasons,
        // and the PASS is what tells them apart:
        //
        //   pass 1 — the worker is declared below the loop, so it is not resolved YET.
        //     `Unknown` is the honest placeholder and pass 2 refines it.  Answering
        //     `integer` here pinned the slot, and pass 2 refines only a slot that is
        //     still unknown, so it stayed: `t += b` retyped a float accumulator to
        //     integer and the pass-1 error aborted before pass 2 could correct it.
        //     Only the compound assignment showed it — `t = t + b` coerces (loft#988).
        //
        //   pass 2 — the worker will NEVER resolve, and the error saying so is already
        //     reported (an unknown name, or a generator worker, which
        //     `parse_parallel_worker` refuses with this same sentinel).  Here `integer`
        //     is what the body needs: `b` has to carry SOME usable type or every use of
        //     it in the body earns a second diagnostic under the first one.
        //
        // A RESOLVED worker whose return type is still unknown keeps `integer` too,
        // which is the width the downstream route decisions already assume.
        let b_type = if fn_d_nr == u32::MAX && self.first_pass {
            Type::Unknown(u32::MAX)
        } else if matches!(ret_type, Type::Unknown(_)) {
            I32.clone()
        } else if let Type::Text(_) = ret_type {
            // Strip worker-internal deps — they reference variables in the worker scope.
            Type::Text(crate::data::Deps::none())
        } else {
            ret_type.clone()
        };
        // Plan-04 B.3 follow-up v2 (b3-par-inline.md): each par block gets
        // its OWN uniquely-named `b_var`, so two par blocks sharing the
        // user's loop-variable name can no longer collide on a single
        // `Function::variables` entry.  During body parsing the user's
        // name is aliased to this `b_var` via `set_name` (same mechanism
        // as match-arm field aliases in `control.rs:867`).  After the
        // body parses, every `Value::Var(b_var)` in the body is rewritten
        // to the element-accessor expression (see post-parse rewrite
        // below) — `b` becomes an inline alias rather than a runtime
        // slot, so there is no `Set(b_var, …)`, no `OpPut*`, and no
        // type-width mismatch to drift the stack.
        //
        // Key it on the stable `loop_nr` (identical across both parser
        // passes), NOT the global `create_unique` counter: that counter can
        // accumulate a different number of increments between pass 1 and
        // pass 2 across many par loops, so a counter-named `b_var` failed to
        // reuse its pass-1 entry — the user name then aliased to a wrong-typed
        // var (`r.len()` on a `text` result seen as `integer`).  A loop-keyed
        // name reuses the same entry on both passes.
        let b_var_name = format!("_{result_name}_par{loop_nr}");
        let b_var = self
            .vars
            .add_variable(&b_var_name, &b_type, &mut self.lexer);
        self.vars.defined(b_var);
        let prior_name_target = self.vars.set_name(result_name, b_var);
        if matches!(b_type, Type::Integer(_) | Type::Unknown(_)) {
            self.vars.in_use(b_var, true);
        }

        // Parse the body block.
        //
        // loft#1040 — a REPLAY (a monomorph re-lowering a clause its TEMPLATE deferred)
        // has no tokens left to read: the template parsed this body once, and the
        // monomorph inherited it through the marker's arguments, substituted like any
        // other IR.  Everything else in this function is type-driven, so injecting the
        // body is the whole of what a replay needs — and the loop BOOKKEEPING around the
        // parse is skipped with it, because that loop was opened, counted and finished
        // while the template was read.
        let replayed = self.par_replay_body.take();
        let is_replay = replayed.is_some();
        let mut block = Value::Null;
        let count;
        if let Some((body, replay_count)) = replayed {
            block = body;
            count = replay_count;
        } else {
            self.vars.loop_var(b_var);
            let in_loop = self.in_loop;
            self.in_loop = true;
            // M11-a: flag that we are inside a par() body so that any `yield`
            // encountered during parsing can emit a compile-time error.
            let outer_par = self.in_par_body;
            self.in_par_body = true;
            self.parse_block("parallel for", &mut block, &Type::Void);
            count = self.vars.loop_counter();
            self.in_par_body = outer_par;
            self.in_loop = in_loop;
            self.vars.finish_loop(loop_nr);
        }
        // Restore prior `result_name` alias (or remove ours if none).
        match prior_name_target {
            Some(nr) => {
                self.vars.set_name(result_name, nr);
            }
            None => self.vars.remove_name(result_name),
        }

        // loft#1040 — the route cannot be picked here when the types it is picked FROM are
        // this function's type variable.  The body is parsed (that is what types it, and
        // it is what a monomorph inherits); the LOWERING waits for a monomorph, which has
        // the element and return types this clause is really about.  Everything the
        // re-lowering must see substituted rides in the marker's arguments.
        if !is_replay && self.defer_parametric_par(ret_type, elem_tp) {
            let id = self.par_deferred.len();
            self.par_deferred.push(crate::parser::DeferredPar {
                result_name: result_name.to_string(),
                worker: fn_d_nr,
                loop_nr,
                elem_var,
                count,
            });
            let marker = self.par_marker_def();
            let mut args = vec![
                Value::Int(i32::try_from(id).unwrap_or(i32::MAX)),
                block,
                vec_expr.clone(),
                threads_expr,
                fill,
            ];
            args.extend(extra_args);
            // A `for … par(…) { … }` is a STATEMENT whichever way its parse ends, so the
            // marker is wrapped in a VOID block — a bare call carries its callee's type,
            // and the statement parser then read the loop as a value and demanded a `;`
            // after its closing brace.  The same reason the unresolved-worker exit below
            // answers with a block.
            *code = v_block(
                vec![Value::Call(marker, args)],
                Type::Void,
                "par (deferred to the monomorph)",
            );
            return;
        }

        // Build IR only when we have a valid function reference.
        if fn_d_nr == u32::MAX || par_for_d_nr == u32::MAX {
            // No IR to emit — but the loop is still a STATEMENT, and the placeholder has
            // to say so.  `Value::Null` does not: the statement parser read the loop as a
            // value and demanded a `;` after its closing brace, so
            // `for a in v par(b = w(a), 4) { … }` compiled with `w` declared ABOVE it and
            // failed with `Expect token ;` on the next line with `w` declared below —
            // the same loop, legal or not by where its worker sits.
            //
            // "Errors already reported" holds on the second pass.  On the FIRST it does
            // not: an unresolved worker is the ordinary state of a forward reference, and
            // this recovery path is on the normal route to compiling one.  Both terminal
            // branches below answer `Type::Void`; so does this one.
            *code = v_block(Vec::new(), Type::Void, "par (worker not resolved)");
            return;
        }

        // Plan-06 PRIORITY.md spine step 3 (Discard detection) — when the
        // body never references the worker's result name (`b_var`) and
        // never references the loop variable (`elem_var`), the user
        // doesn't need a materialised result vector.  Lower to a direct
        // call into `n_parallel_discard` (spine step 2 / 3b) which runs
        // workers, drops results, allocates no result vector.
        //
        // Tighter conditions surface in steps 4 (Queue) and 9 (Reduce);
        // until then, only the empty-body case routes here.  Body shapes
        // like `{ log("done"); }` (uses neither r nor x) also qualify
        // but are gated behind a per-Var walk in a later step.
        let body_is_empty = matches!(&block, Value::Null)
            || matches!(&block, Value::Block(bl) if bl.operators.is_empty())
            || matches!(&block, Value::Insert(ops) if ops.is_empty());
        let discard_d_nr = self.data.def_nr("n_parallel_discard");
        if !self.first_pass && body_is_empty && discard_d_nr != u32::MAX && extra_args.is_empty() {
            // Build a Discard call with the same arg layout as
            // n_parallel_for (so the native fn pops in the same order).
            // The body and per-element accessor (b/a inline aliases)
            // are not needed — workers run, results dropped.
            let n_extra_v = Value::Int(0);
            let pf_args = vec![
                vec_expr.clone(),
                Value::Int(elem_size),
                Value::Int(return_size),
                threads_expr,
                Value::Int(fn_d_nr as i32),
                n_extra_v,
            ];
            *code = v_block(
                vec![fill, Value::Call(discard_d_nr, pf_args)],
                Type::Void,
                "par_discard",
            );
            // Loop-counter accounting from the empty body.
            let _ = count;
            return;
        }

        // A14.5/A14.6: auto-select light path for eligible workers.
        // Heap-typed returns (Reference, struct-enum, Text, Unknown) need the
        // heavy path's deep-copy machinery — the light path's `execute_at_raw`
        // memcpy only handles inline returns ≤ 8 bytes.
        // Plan-06 phase 1 G4 — fn-ref returns (Type::Function, 20 bytes)
        // also need the heavy path's per-worker output slot mechanism;
        // the light path writes via 8-byte execute_at_raw and would
        // truncate the closure DbRef.
        let is_primitive_return = !matches!(
            ret_type,
            Type::Text(_)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Function(_, _, _)
                | Type::Vector(_, _)
                | Type::Unknown(_)
        );
        // ARC.md A4 (closed 2026-05-07) — `n_parallel_for_light` was
        // retired; every primitive return type now routes through the
        // Queue family (route_int_queue / route_narrow_queue / etc.).
        // `actual_par_d_nr` falls back to `n_parallel_for` only for
        // shapes the Queue family doesn't cover (Tuple returns,
        // pending A7).  The native body of `n_parallel_for` panics
        // with a clear diagnostic if it ever runs.
        let _ = is_primitive_return; // light_m elimination retained for future scope-analysis hooks
        let actual_par_d_nr = par_for_d_nr;

        // parallel_for(input, elem_size, return_size, threads, fn_d_nr, [pool_m], extra1, ..., n_extra)
        // n_extra is pushed LAST so it's on top of the stack for popping first.
        let n_extra = extra_args.len();
        let mut pf_args = vec![
            vec_expr.clone(),
            Value::Int(elem_size),
            Value::Int(return_size),
            threads_expr,
            Value::Int(fn_d_nr as i32),
        ];
        // pool_m is hardcoded in the native function (avoids stack-ordering complexity)
        // ARC.md A4: light_m elimination — `n_parallel_for_light` was retired
        pf_args.extend(extra_args);
        pf_args.push(Value::Int(n_extra as i32));

        // Plan-06 PRIORITY.md spine step 8 — primitive 8-byte integer
        // returns dispatch through the streaming Queue path (8a's
        // `n_parallel_queue` + `n_parallel_buf_get`/`_drop`) instead
        // of allocating a heap result vector.
        //
        // After 8b' (this commit) `run_parallel_queue` mirrors
        // `run_parallel_direct`'s input-kind dispatch (DbRef, text,
        // primitive, wide-inline / tuple / fn-ref), so the parser
        // gate no longer needs to filter by input kind — every
        // `is_primitive_return` worker now routes correctly.
        //
        // Gating today:
        //   - `is_primitive_return` (no Text/Reference/Enum-payload/
        //     Function/Vector/Unknown — handled by 8c/8d).
        //   - `Type::Integer(_)` with full 8-byte width.  Narrow
        //     integer widths (u8, i32, etc.) and other primitive
        //     return types (bool, single, character, enum-no-
        //     payload, tuple) need per-size buf_get variants to
        //     mask the high bits and land cleanly into the body's
        //     b_var slot.  Deferred to a future per-size sub-step.
        //   - `fn_d_nr` resolved (partial-parse failures fall
        //     through to the legacy path).
        //   - `n_parallel_queue` / `_buf_get` / `_buf_drop`
        //     registered (defensively, so a stripped stdlib still
        //     parses).
        //
        // Trade-off: workers eligible for the light path
        // (`light_m.is_some()`) skip its per-thread pool optimisation
        // when routed through Queue.  A later sub-step combines
        // light + queue once the architecture stabilises.
        let queue_d_nr = self.data.def_nr("n_parallel_queue");
        let buf_get_d_nr = self.data.def_nr("n_parallel_buf_get");
        let buf_drop_d_nr = self.data.def_nr("n_parallel_buf_drop");
        let queue_narrow_d_nr = self.data.def_nr("n_parallel_queue_narrow");
        let buf_get_narrow_d_nr = self.data.def_nr("n_parallel_buf_get_narrow");
        let buf_drop_narrow_d_nr = self.data.def_nr("n_parallel_buf_drop_narrow");
        let queue_text_d_nr = self.data.def_nr("n_parallel_queue_text");
        let buf_get_text_d_nr = self.data.def_nr("n_parallel_buf_get_text");
        let buf_drop_text_d_nr = self.data.def_nr("n_parallel_buf_drop_text");
        let queue_ref_d_nr = self.data.def_nr("n_parallel_queue_ref");
        let buf_get_ref_d_nr = self.data.def_nr("n_parallel_buf_get_ref");
        let buf_drop_ref_d_nr = self.data.def_nr("n_parallel_buf_drop_ref");
        let queue_fn_d_nr = self.data.def_nr("n_parallel_queue_fn");
        let buf_get_fn_d_nr = self.data.def_nr("n_parallel_buf_get_fn");
        let buf_drop_fn_d_nr = self.data.def_nr("n_parallel_buf_drop_fn");
        // ARC.md A3.6 — `float` (8B f64) joins 8B Integer on the
        // wide-Queue path (mirrors `early_ret_size_8`).
        let ret_size_8 = matches!(ret_type, Type::Integer(spec) if u32::from(crate::variables::size(
            &Type::Integer(*spec),
            &Context::Argument,
        )) == 8)
            || matches!(ret_type, Type::Float);
        // Plan-06 ARC.md A3 / A3.5 — narrow primitive return routing.
        // Mirrors the early-gate logic; see comment above for shape
        // coverage.
        let narrow_route = narrow_route_for(ret_type);
        // 8b: integer-i64 returns route through `n_parallel_queue` +
        // `par_buffer_stack`.
        // 8c: text returns route through `n_parallel_queue_text` +
        // `par_text_buffer_stack` (sibling stack — keeps the per-row
        // read path tight by avoiding an enum match per element).
        // 8d.2: reference / struct-enum-payload returns route through
        // `n_parallel_queue_ref` + `par_ref_buffer_stack` — workers
        // return DbRefs into their own output stores, the dispatcher
        // adopts those stores into the parent and rebases the DbRefs
        // via `Stores::adopt_worker_excess` + `rebase_walk_record`.
        // Vector returns stay on the legacy materialised path for
        // now — they're treated as a Reference but the
        // `parallel_execute_and_collect` branch picks the heap-vector
        // ownership invariants and matches its element-stride
        // accounting differently; 8d.3 generalises Queue dispatch
        // to vector returns.
        // ARC.md A6.b: fn-ref returns route through `n_parallel_queue_fn`
        // + `par_fn_buffer_stack` (packed Vec<u8>, 20 bytes per row).
        // The body substitution diverges: instead of inline-expanding
        // b_var via `replace_var_in_ir`, b_var is kept as a real
        // variable and `Set(b_var, Call(buf_get_fn, [idx]))` is emitted
        // at the top of each iteration body.  This works around the
        // `replace_var_in_ir` limitation that doesn't substitute the
        // u16 fn-ref var index inside `Value::CallRef`.
        let route_int_queue = is_primitive_return
            && fn_d_nr != u32::MAX
            && ret_size_8
            && queue_d_nr != u32::MAX
            && buf_get_d_nr != u32::MAX
            && buf_drop_d_nr != u32::MAX;
        // Plan-06 ARC.md A3 / A3.5 — narrow primitive Queue route.
        // Same gate as route_int_queue but for narrow Integer + bool /
        // character / enum-no-payload (each shape fits 1 or 4 bytes).
        let route_narrow_queue = is_primitive_return
            && fn_d_nr != u32::MAX
            && narrow_route.is_some()
            && queue_narrow_d_nr != u32::MAX
            && buf_get_narrow_d_nr != u32::MAX
            && buf_drop_narrow_d_nr != u32::MAX;
        let route_text_queue = matches!(ret_type, Type::Text(_))
            && fn_d_nr != u32::MAX
            && queue_text_d_nr != u32::MAX
            && buf_get_text_d_nr != u32::MAX
            && buf_drop_text_d_nr != u32::MAX;
        // Late-gate matches early-gate (see comment above).
        // ARC.md A6.d: keyed-collection returns share the ref path
        // (DbRef to backing record + type-driven rebase walk).
        let route_ref_queue = crate::data::is_dbref(ret_type)
            && fn_d_nr != u32::MAX
            && queue_ref_d_nr != u32::MAX
            && buf_get_ref_d_nr != u32::MAX
            && buf_drop_ref_d_nr != u32::MAX;
        let route_fn_queue = matches!(ret_type, Type::Function(_, _, _))
            && fn_d_nr != u32::MAX
            && queue_fn_d_nr != u32::MAX
            && buf_get_fn_d_nr != u32::MAX
            && buf_drop_fn_d_nr != u32::MAX;
        let route_through_queue = route_int_queue
            || route_narrow_queue
            || route_text_queue
            || route_ref_queue
            || route_fn_queue;

        let stop_cond = self.cl("OpLeInt", &[Value::Var(len_var), Value::Var(idx_var)]);
        let stop = v_if(
            stop_cond,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );

        // Build the body's `b` accessor.  For the queue paths it's
        // `n_parallel_buf_get[_text/_ref/_fn](idx)`; for the materialised
        // path it's `OpGetVector + get_field` indexing the heap
        // result vector.
        // Plan-06 ARC.md A3 / A3.5 — narrow takes priority over wide
        // int.  var_size() returns 8 for all Integer stack slots
        // (post-2c), so route_int_queue would also fire for narrow
        // returns; the narrow path is what we want when storage
        // width is < 8 OR the return type is Boolean / Character /
        // Enum-no-payload.
        let get_call = if route_narrow_queue {
            // Narrow reader takes (idx, return_size, signed).  Returns
            // i64; for non-Integer shapes we wrap with the matching
            // conversion (Op call or OpNeInt-vs-zero) so the body's
            // `r` substitution lands correctly typed.
            let route = narrow_route
                .as_ref()
                .expect("narrow_route set when route_narrow_queue is true");
            // ARC.md A3.6 — TypedBufGet bypasses the i64 reader
            // entirely.  The named buf_get fn is signature-typed
            // (returns `single` etc.) and reads the same per-row
            // bytes via a typed memcpy.  No wrap needed.
            if let NarrowWrap::TypedBufGet(typed_fn_name) = &route.wrap {
                let typed_d_nr = self.data.def_nr(typed_fn_name);
                if typed_d_nr == u32::MAX {
                    // Defensive: stripped stdlib without the typed
                    // reader.  Fall back to the raw narrow buf_get
                    // (will type-mismatch downstream — louder than
                    // a silent miscompile).
                    Value::Call(
                        buf_get_narrow_d_nr,
                        vec![
                            Value::Var(idx_var),
                            Value::Int(i32::from(route.width)),
                            Value::Int(i32::from(route.signed)),
                        ],
                    )
                } else {
                    Value::Call(typed_d_nr, vec![Value::Var(idx_var)])
                }
            } else {
                let raw_call = Value::Call(
                    buf_get_narrow_d_nr,
                    vec![
                        Value::Var(idx_var),
                        Value::Int(i32::from(route.width)),
                        Value::Int(i32::from(route.signed)),
                    ],
                );
                match &route.wrap {
                    NarrowWrap::None => raw_call,
                    NarrowWrap::OpCall(conv_name) => {
                        let conv_d_nr = self.data.def_nr(conv_name);
                        if conv_d_nr == u32::MAX {
                            // Defensive: stripped stdlib without the conv
                            // Op.  Skip the wrap and let downstream type-
                            // check catch the mismatch — much louder than
                            // a silent miscompile.
                            raw_call
                        } else {
                            Value::Call(conv_d_nr, vec![raw_call])
                        }
                    }
                    NarrowWrap::NeZero => {
                        // Boolean wrap: `OpNeInt(buf_get, 0) -> boolean`.
                        let ne_d_nr = self.data.def_nr("OpNeInt");
                        if ne_d_nr == u32::MAX {
                            raw_call
                        } else {
                            Value::Call(ne_d_nr, vec![raw_call, Value::Int(0)])
                        }
                    }
                    NarrowWrap::TypedBufGet(_) => unreachable!("handled above"),
                }
            }
        } else if route_int_queue {
            // ARC.md A3.6 — Float returns reuse the wide u64-row
            // queue but the body needs `parallel_buf_get_float` for
            // typed `f64::from_bits` recovery.
            if matches!(ret_type, Type::Float) {
                let buf_get_float_d_nr = self.data.def_nr("n_parallel_buf_get_float");
                if buf_get_float_d_nr == u32::MAX {
                    Value::Call(buf_get_d_nr, vec![Value::Var(idx_var)])
                } else {
                    Value::Call(buf_get_float_d_nr, vec![Value::Var(idx_var)])
                }
            } else {
                Value::Call(buf_get_d_nr, vec![Value::Var(idx_var)])
            }
        } else if route_text_queue {
            Value::Call(buf_get_text_d_nr, vec![Value::Var(idx_var)])
        } else if route_ref_queue {
            // 8d.2: returns a rebased DbRef (12 bytes) — body field
            // accesses route through the standard OpGetField path,
            // same as Var(struct_var).foo.
            Value::Call(buf_get_ref_d_nr, vec![Value::Var(idx_var)])
        } else if route_fn_queue {
            // ARC.md A6.b: returns a 20-byte fn-ref blob.  Used as
            // the RHS of `Set(b_var, ...)` below — NOT inline-substituted
            // via `replace_var_in_ir` because the body's `f(10)`
            // parses as `CallRef(b_var, [Int(10)])` and `replace_var_in_ir`
            // doesn't substitute the u16 fn-ref var index inside CallRef.
            Value::Call(buf_get_fn_d_nr, vec![Value::Var(idx_var)])
        } else {
            // Use OpGetVector + get_field to extract the element from the result
            // vector. This works for all return types (int, long, float, bool, text)
            // without per-type getter functions.
            let result_elem_size = match return_size {
                0 => 4, // text: 4-byte string pointer per element
                -1 => {
                    // reference: inline struct size from the database
                    let ret_td = self.data.type_def_nr(ret_type);
                    let known = self.data.def(ret_td).known_type();
                    i32::from(self.database.size(known))
                }
                other => other,
            };
            // Plan-07 phase 4 step 4.6 — par-worker iteration over the
            // result vector uses the Nullable peer (iteration site).
            // `idx_var` is bounded by the parallel-loop driver so OOB
            // shouldn't fire here, but keeping the Nullable shape
            // makes the design rule "every iteration site emits
            // Nullable" hold uniformly.
            let get_vec = self.cl(
                "OpGetVectorNullable",
                &[
                    Value::Var(results_var.expect(
                        "materialised path requires results_var; queue gate let it slip through",
                    )),
                    Value::Int(result_elem_size),
                    Value::Var(idx_var),
                ],
            );
            if matches!(ret_type, Type::Reference(_, _)) || fn_d_nr == u32::MAX {
                // fn_d_nr == u32::MAX: worker was rejected (e.g. S23 generator check);
                // skip the type-based field access to avoid crashing on Unknown type.
                get_vec
            } else {
                let vec_tp = self.data.type_def_nr(ret_type);
                if vec_tp == u32::MAX {
                    // Unsupported return type (e.g. iterator<T> in first pass before S23
                    // diagnostic fires): fall back to raw vector access to prevent crash.
                    get_vec
                } else {
                    self.get_field(vec_tp, usize::MAX, get_vec)
                }
            }
        };
        // Plan-04 B.3 follow-up v2 (b3-par-inline.md): rewrite every
        // `Value::Var(b_var)` in the body with a clone of `get_call`.
        // `b` is no longer a runtime variable — each reference expands
        // inline to the accessor expression.  No `Set(b_var, get_call)`
        // is emitted; the body references ARE the reads.  Under the B.3
        // atomic bundle's slot-aware `OpPut*` dispatch this eliminates
        // the type-width mismatch and the stack drift.
        //
        // ARC.md A6.b: fn-ref returns diverge — `f(10)` parses as
        // `CallRef(b_var, [Int(10)])` and `replace_var_in_ir`
        // (src/parser/collections.rs:replace_var_in_ir) walks
        // `CallRef(_, args)` into `args` but doesn't substitute the
        // first u16 (the fn-ref var index).  Inline substitution
        // would leave b_var as a dangling reference.  Instead, keep
        // b_var as a real variable with a 20-byte slot, and prepend
        // `Set(b_var, Call(buf_get_fn, [idx]))` to the body so each
        // iteration's CallRef reads the fresh fn-ref blob.
        if route_fn_queue {
            // Mark b_var as in_use so the slot allocator reserves
            // a 20-byte slot.  The body's `f(10)` (CallRef) uses
            // b_var implicitly through the u16 var index, which
            // doesn't increment the standard use-count tracker.
            self.vars.in_use(b_var, true);
            let init = v_set(b_var, get_call.clone());
            // Prepend init to the body block.  If the body is not
            // already a Block, wrap it in one.
            if let Value::Block(ref mut bl) = block {
                bl.operators.insert(0, init);
            } else {
                let inner = std::mem::replace(&mut block, Value::Null);
                block = v_block(vec![init, inner], Type::Void, "fn-queue body");
            }
        } else {
            replace_var_in_ir(&mut block, b_var, &get_call);
        }

        // apply the same inline-alias treatment to the outer
        // iterator variable `a`.  The desugared loop increments `idx`;
        // `a` is logically `items[idx]` on every iteration.  Rewriting
        // `Var(a)` → `OpGetVectorNullable(items, elem_size, idx)` (plus
        // `get_field` for non-Reference element types, mirroring the
        // `b` path) means `a` never needs a slot.  Without this the
        // allocator leaves `a` at `stack_pos == u16::MAX` and codegen
        // panics `Incorrect var a[65535] versus N`.
        //
        // Plan-07 phase 4 step 4.6 — fused-for-par iteration site →
        // Nullable peer (consistent with all other iteration sites).
        let a_get_vec = self.cl(
            "OpGetVectorNullable",
            &[vec_expr.clone(), Value::Int(elem_size), Value::Var(idx_var)],
        );
        let a_accessor = if matches!(elem_tp, Type::Reference(_, _)) {
            a_get_vec
        } else {
            let elm_td = self.data.type_def_nr(elem_tp);
            if elm_td == u32::MAX {
                a_get_vec
            } else {
                self.get_field(elm_td, usize::MAX, a_get_vec)
            }
        };
        // Any body variable BOUND to the element accessor holds a borrowed view of the
        // input vector — the same thing the loop variable held before this substitution
        // inlined it.  A binding drops the RHS's deps (`_ = e` gives its discard a bare
        // `ref(T)`), and the body then freed that view once per ROW: with a struct
        // element, each iteration freed one of the caller's records, and the vector read
        // back correct bytes until something reused the arena (`LOFT_POISON=1` is what
        // shows it).  The sequential form escapes it because its binding copies a VAR
        // whose type still carries the dep.
        //
        // Marking the bound var skip-free says exactly what is true — the loop does not
        // own what it is looking at — and leaves every other free in the body alone.
        //
        // A TEXT binding is the exception: a text `Set` copies bytes into the binding's own
        // `String` (`OpAppendText`), so `_ = e` over a `vector<text>` owns what it holds and
        // marking it never-free orphaned one buffer per row (loft#1357).  The sequential
        // form frees the same binding; so does this one now.
        let borrowed_views = elem_borrow_bindings(&block, elem_var);
        replace_var_in_ir(&mut block, elem_var, &a_accessor);
        for v in borrowed_views {
            if matches!(self.vars.tp(v).base(), Type::Text(_)) {
                continue;
            }
            self.vars.set_skip_free(v);
        }
        let idx_inc = v_set(
            idx_var,
            self.cl("OpAddInt", &[Value::Var(idx_var), Value::Int(1)]),
        );

        let mut lp = vec![stop, block, idx_inc];
        if count != u16::MAX {
            lp.insert(
                3,
                v_set(
                    count,
                    self.cl("OpAddInt", &[Value::Var(count), Value::Int(1)]),
                ),
            );
        }

        let mut for_steps = Vec::new();
        if count != u16::MAX {
            for_steps.push(v_set(count, Value::Int(0)));
        }
        if fill != Value::Null {
            for_steps.push(fill);
        }
        if route_through_queue {
            // Queue path: `n_parallel_queue[_text]` returns the row
            // count directly, doubling as `par_len` and saving the
            // `OpLengthVector(input)` call.  The result heap vector
            // (`results_var`) is never allocated; per-iteration reads
            // come from `stores.par_buffer_stack` (int),
            // `par_text_buffer_stack` (text), or
            // `par_ref_buffer_stack` (ref).  After the loop, pop
            // the buffer with `n_parallel_buf_drop[_text/_ref]()` so
            // the next par-call doesn't see a stale buffer
            // underneath; the ref drop also frees adopted worker
            // stores.
            let (call_d_nr, drop_d_nr, label_loop, label_block) = if route_text_queue {
                (
                    queue_text_d_nr,
                    buf_drop_text_d_nr,
                    "Parallel for loop (queue text)",
                    "Parallel for block (queue text)",
                )
            } else if route_narrow_queue {
                // Plan-06 ARC.md A3 / A3.5 — for the narrow path,
                // pf_args[2] (return_size) was set by var_size which
                // returns the wide stack-slot width (8 bytes); the
                // narrow runtime expects the actual stride (1/2/4).
                // Patch in place before the queue_call below grabs
                // `pf_args`.
                let route = narrow_route
                    .as_ref()
                    .expect("narrow_route set when route_narrow_queue is true");
                pf_args[2] = Value::Int(i32::from(route.width));
                (
                    queue_narrow_d_nr,
                    buf_drop_narrow_d_nr,
                    "Parallel for loop (queue narrow)",
                    "Parallel for block (queue narrow)",
                )
            } else if route_ref_queue {
                (
                    queue_ref_d_nr,
                    buf_drop_ref_d_nr,
                    "Parallel for loop (queue ref)",
                    "Parallel for block (queue ref)",
                )
            } else if route_fn_queue {
                (
                    queue_fn_d_nr,
                    buf_drop_fn_d_nr,
                    "Parallel for loop (queue fn)",
                    "Parallel for block (queue fn)",
                )
            } else {
                (
                    queue_d_nr,
                    buf_drop_d_nr,
                    "Parallel for loop (queue)",
                    "Parallel for block (queue)",
                )
            };
            let queue_call = Value::Call(call_d_nr, pf_args);
            for_steps.push(v_set(len_var, queue_call));
            for_steps.push(v_set(idx_var, Value::Int(0)));
            for_steps.push(v_loop(lp, label_loop));
            for_steps.push(Value::Call(drop_d_nr, Vec::new()));
            *code = v_block(for_steps, Type::Void, label_block);
        } else {
            // Materialised path (text / ref / fn-ref / vector / non-
            // integer primitives).  Allocates a heap result vector
            // and indexes it via `OpGetVector`.  Step 8c/8d/8b' will
            // route the remaining cases through Queue; step 8e then
            // retires `parallel_execute_and_collect` and the
            // `Stitch::Concat` arm.
            let pf_call = Value::Call(actual_par_d_nr, pf_args);
            // len(input_vec) — compute once before the loop.
            let len_call = self.cl("OpLengthVector", std::slice::from_ref(vec_expr));
            for_steps.push(v_set(len_var, len_call));
            for_steps.push(v_set(
                results_var.expect(
                    "materialised path requires results_var; queue gate let it slip through",
                ),
                pf_call,
            ));
            for_steps.push(v_set(idx_var, Value::Int(0)));
            for_steps.push(v_loop(lp, "Parallel for loop"));
            *code = v_block(for_steps, Type::Void, "Parallel for block");
        }
    }

    // Consume the remaining `par(...)` tokens and then the body block so the
    // parser can recover after an error in the parallel clause.
    // Called after '(' has already been consumed, so this drains to ')'.
    /// Build the call to a user CALLBACK that a builtin lowers by hand — `map`'s and
    /// `filter`'s per-element call, `reduce`'s per-step fold (loft#945).
    ///
    /// The declared arguments are not the whole call.  A callee that answers a HEAP value
    /// (text, a collection, a struct) also takes a hidden buffer parameter the CALLER
    /// allocates, and an ordinary call site fills those slots in `add_defaults`.  A
    /// hand-built `Value::Call` skipped that step, so any callback returning text crashed
    /// the compiler outright — *"Too few parameters on n_shout (got 1, need 2)"* — while
    /// the equivalent comprehension `[for s in xs { shout(s) }]` was fine.  Routing
    /// through the same filler is what makes the two spellings agree.
    /// The element value to hand a combinator's CALLBACK, and the type to declare it as.
    ///
    /// [`Parser::for_type`]'s P189b block types a TUPLE loop variable as
    /// `Reference(__tuple<…>)`, because iteration yields a DbRef pointing at the element's
    /// inline bytes in the vector record.  That is right for a `for` BODY, where `t.0`
    /// reads through the reference at the stored field offsets.  A callback is a CALL, and
    /// a `fn(t: (…))` parameter is by VALUE everywhere else in the language — a direct
    /// call, a comprehension and an index all agree — so the DbRef has to be unboxed into
    /// the stack tuple first.
    ///
    /// Without it the callee read the DbRef's twelve bytes as the tuple's own members:
    /// `map` answered a packed DbRef as `343597383710`, `filter` SIGSEGV'd once a member
    /// was `text`, and `--native` refused to compile the call at all — `expected
    /// (i64, i64), found DbRef` (loft#1074).  A STRUCT element is unaffected and must stay
    /// on the reference path, because a struct IS a DbRef and the two representations
    /// already agree; the tuple is the one element type where they do not.
    ///
    /// Returns the value and its type together so the three combinators that call this
    /// cannot drift apart on them — the hazard loft#1006 was, in this same area.
    pub(crate) fn callback_element_arg(
        &mut self,
        in_type: &Type,
        for_var: u16,
        var_tp: &Type,
    ) -> (Value, Type) {
        if let Type::Vector(elem, _) = in_type.base()
            && let Type::Tuple(elems) = elem.base()
        {
            let elems = elems.clone();
            let arg = self.unbox_tuple_from_dbref(Value::Var(for_var), &elems);
            return (arg, Type::Tuple(elems));
        }
        (Value::Var(for_var), var_tp.clone())
    }

    /// A `CallRef` to a callback, carrying the one hidden text work buffer that a
    /// text-returning target expects (loft#1115).
    ///
    /// The fn-ref call ABI hands a text-returning callee exactly one `RefVar(Text)` work
    /// buffer, allocated by the CALLER, because the call site cannot know which lambda a
    /// fn-typed slot holds (`State::fn_call_ref`, and P227 on the callee side).  The
    /// ordinary `f(args)` spelling appends it in `parse_operators`; a combinator that
    /// lowers its own call had to append it too, and did not — so a capturing lambda
    /// written inline in `map` and returning `text` was entered with its frame one DbRef
    /// span short and read its `__closure` from the wrong offset.
    ///
    /// Only the buffer is appended here.  The closure argument is NOT: `fn_call_ref` reads
    /// it back from the 20-byte fn-ref slot at run time, which is why the same shape
    /// returning an integer, a struct, or a boolean was always correct.
    ///
    /// The buffer is drawn from `caller_text_buf`'s `__work_c<N>` sequence rather than
    /// `work_text`'s `__work_<N>`, because this mint is **pass-2 only**: the map/filter
    /// family early-returns on pass 1, where an unresolved callback type makes the
    /// desugar impossible.  A pass-2-only mint drawing from the shared counter shifts
    /// every later `__work_N`, and the variable tables persist across passes BY NAME —
    /// so pass 2 would re-find pass 1's variables under the wrong roles (loft#662).
    /// `caller_text_buf` is the sequence for exactly this: a buffer the CALLER allocates
    /// for a callee's hidden `&text` out-param.
    fn callback_call_ref(&mut self, fn_ref_var: u16, mut args: Vec<Value>, ret: &Type) -> Value {
        if !self.first_pass {
            // The COUNT is the one every text-returning `CallRef` site reads
            // (`Data::fnref_text_buffers`), because `fn_call_ref` trims the frame against
            // it: a combinator's callback can be a NAMED function declaring more buffers
            // than the one a lambda ever takes (loft#1116).
            //
            // The VARIABLES come from `caller_text_buf` rather than
            // `Parser::fnref_text_buffer_vars`' `work_text`, and that difference is
            // load-bearing: the map family early-returns on pass 1, so this mint is
            // pass-2-only, and taking it from the shared counter would shift every later
            // `__work_N` (loft#662's class).
            let n = self.data.fnref_text_buffers(args.len(), ret);
            let work_vars: Vec<u16> = (0..n)
                .map(|_| self.vars.caller_text_buf(&mut self.lexer))
                .collect();
            self.push_fnref_text_buffers(&mut args, &work_vars);
        }
        Value::CallRef(fn_ref_var, args)
    }

    pub(crate) fn callback_call(
        &mut self,
        d_nr: u32,
        mut args: Vec<Value>,
        mut types: Vec<Type>,
    ) -> Value {
        self.add_defaults(d_nr, &mut args, &mut types);
        Value::Call(d_nr, args)
    }

    /// Compiler special-case for `map(v: vector<T>, f: fn(T) -> U) -> vector<U>`.
    /// Generates inline bytecode equivalent to `[for elm in v { f(elm) }]`.
    #[allow(clippy::too_many_lines)]
    // @F24 — higher-order functions (map / filter / reduce)
    pub(crate) fn parse_map(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), crate::data::Deps::none());
        // On first pass, return the concrete output vector type derived from the function's
        // return type so that downstream variables (e.g. `r = map(...)`) get the right type
        // and subsequent `for x in r` iterations resolve correctly.
        // We must NOT create unique variables here — only determine the type.
        if self.first_pass {
            // loft#945 — the output element is the CALLBACK's return type: `map` is
            // `fn(T) -> U` answering `vector<U>`.  Both passes must agree on it or the
            // BINDING reports "Variable 'v' cannot change type from vector<T> to
            // vector<U>" — which is how a perfectly good `map(xs, label)` was refused.
            if let Some(Type::Function(_, ret, _)) = types.get(1)
                && !ret.is_unknown()
                && !matches!(**ret, Type::Void)
            {
                return Type::Vector(ret.clone(), crate::data::Deps::none());
            }
            // The callback's return is not resolvable yet — a function declared BELOW
            // this call has nothing to resolve against on pass 1 (loft#918's shape).
            // Fall back to the input element type, which is right whenever `U == T`
            // and lets downstream code like `r[0]` type-check either way.
            if let Type::Vector(elm, _) = &types[0] {
                return Type::Vector(elm.clone(), crate::data::Deps::none());
            }
            return placeholder;
        }
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map requires 2 arguments: map(vector, fn f)"
            );
            return placeholder;
        }
        let _in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: first argument must be a vector"
            );
            return placeholder;
        };
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret, _) = &types[1] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: second argument must be a function reference (use fn <name>)"
            );
            return placeholder;
        };
        if fn_param_types.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "map: function must take exactly one argument"
            );
            return placeholder;
        }
        // D-clo-2 — a stored short `|x|` lambda whose types could not be inferred (it was
        // assigned without a type context — `g = |y| { y*2 }`) arrives here as
        // `Function([Unknown], Unknown)`.  Building a result vector of an Unknown element type
        // panics in `build_comprehension_code` (`def(u32::MAX)`).  Emit the SAME guiding
        // diagnostic the lambda already gets when used standalone / called directly, instead of
        // crashing — the inline `.map(|y| …)` form (which has the element-type hint) is unaffected.
        // D-clo-2 — a stored short `|x|` lambda assigned without a type context (`g = |y| {…}`)
        // is un-inferrable: it arrives here with a GARBAGE signature (a `text`/`void` default,
        // or `Unknown`).  `map`'s result element is the lambda's return type, so a `void`/unknown
        // return builds a `vector<void>` and panics in `build_comprehension_code`
        // (`def(u32::MAX)`).  Emit the SAME guiding diagnostic the lambda already gets standalone,
        // instead of crashing.  The inline `.map(|y| …)` form has the element-type hint and a
        // real return type, so it is unaffected.
        if !self.first_pass
            && (fn_ret_type.is_unknown()
                || matches!(fn_ret_type, Type::Void)
                || fn_param_types.iter().any(Type::is_unknown))
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot infer the type of the function passed to `map` — a short `|x|` lambda \
                 stored in a variable has no type context. Pass it inline to `.map(…)`, or use \
                 the long form `fn(x: <type>) -> <ret> {{ … }}` which declares its types"
            );
            return placeholder;
        }
        // accept both static fn-refs (Value::Int) and fn-ref variables/lambdas.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None // fn-ref variable or lambda — will use CallRef
        };
        // For CallRef path, store the fn-ref value in a local variable.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique("map_fn", &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("map_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("map_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("map_elm", &var_tp);
        self.vars.defined(for_var);

        let out_elem = fn_ret_type.clone();
        let result_type = Type::Vector(Box::new(out_elem.clone()), crate::data::Deps::none());
        let result_vec = self.create_unique("map_result", &result_type);
        let elm = self.unique_elm_var(&result_type, &out_elem, result_vec);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut fill = v_set(vec_copy_var, list[0].clone());
        // for CallRef path, assign the fn-ref value before the loop.
        if let Some(fv) = fn_ref_var {
            fill = Value::Insert(vec![fill, v_set(fv, list[1].clone())]);
        }

        let (elem_arg, elem_arg_tp) = self.callback_element_arg(&in_type, for_var, &var_tp);
        let body = if let Some(d) = fn_d_nr {
            self.callback_call(d, vec![elem_arg], vec![elem_arg_tp])
        } else {
            let rt = fn_ret_type.clone();
            self.callback_call_ref(fn_ref_var.unwrap(), vec![elem_arg], &rt)
        };

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector rather than
        // pre-seeding it with the LHS variable (which would cause a self-reference panic).
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
            &Value::Var(result_vec),
            elm,
            &out_elem,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            None,
            // The SOURCE, not `result_vec` — that is what this loop appends to.
            if matches!(in_type, Type::Vector(_, _)) {
                Some((Value::Var(vec_copy_var), iter_var))
            } else {
                None
            },
            fill,
            create_iter_code,
            Value::Null,
            body,
            val,
            false,
            false,
            true,
            tp,
        )
    }

    /// Validate `filter` arguments and extract `(in_elem_type, fn_d_nr)`.
    /// Returns `Err(placeholder)` on validation failure.
    pub(crate) fn parse_filter_validate(
        &mut self,
        list: &[Value],
        types: &[Type],
    ) -> Result<(Type, Option<u32>), Type> {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), crate::data::Deps::none());
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter requires 2 arguments: filter(vector, fn pred)"
            );
            return Err(placeholder);
        }
        let in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: first argument must be a vector"
            );
            return Err(placeholder);
        };
        let (fn_param_types, fn_ret_type) = if let Type::Function(params, ret, _) = &types[1] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: second argument must be a function reference (use fn <name>)"
            );
            return Err(placeholder);
        };
        if fn_param_types.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must take exactly one argument"
            );
            return Err(placeholder);
        }
        if fn_ret_type != Type::Boolean {
            diagnostic!(
                self.lexer,
                Level::Error,
                "filter: predicate must return boolean"
            );
            return Err(placeholder);
        }
        // accept both static fn-refs and fn-ref variables/lambdas.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None
        };
        Ok((in_elem_type, fn_d_nr))
    }

    pub(crate) fn parse_filter(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        let placeholder = Type::Vector(Box::new(Type::Unknown(0)), crate::data::Deps::none());
        // On first pass, return the concrete output type from the input vector's element type.
        if self.first_pass {
            if !types.is_empty()
                && let Type::Vector(elm, _) = &types[0]
            {
                return Type::Vector(elm.clone(), crate::data::Deps::none());
            }
            return placeholder;
        }
        let (in_elem_type, fn_d_nr) = match self.parse_filter_validate(list, types) {
            Ok(v) => v,
            Err(t) => return t,
        };
        // for CallRef path, store the fn-ref value in a local variable.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique("filter_fn", &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("filter_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("filter_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("filter_elm", &var_tp);
        self.vars.defined(for_var);

        let out_elem = in_elem_type.clone();
        let result_type = Type::Vector(Box::new(out_elem.clone()), crate::data::Deps::none());
        let result_vec = self.create_unique("filter_result", &result_type);
        let elm = self.unique_elm_var(&result_type, &out_elem, result_vec);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        let mut fill = v_set(vec_copy_var, list[0].clone());
        if let Some(fv) = fn_ref_var {
            fill = Value::Insert(vec![fill, v_set(fv, list[1].clone())]);
        }

        let (elem_arg, elem_arg_tp) = self.callback_element_arg(&in_type, for_var, &var_tp);
        let if_step = if let Some(d) = fn_d_nr {
            self.callback_call(d, vec![elem_arg], vec![elem_arg_tp])
        } else {
            Value::CallRef(fn_ref_var.unwrap(), vec![elem_arg])
        };

        // What the result vector COLLECTS needs the same unboxing as the predicate's
        // argument, and for the same reason: `out_elem` is the TUPLE, so appending the
        // loop variable's `Reference(__tuple<…>)` assigns a DbRef into a tuple slot.  A
        // second unbox rather than reusing the one above, because the two read at
        // different points of the loop — the collect runs only when the predicate passed.
        let (body, _) = self.callback_element_arg(&in_type, for_var, &var_tp);

        self.data.vector_def(&mut self.lexer, &out_elem);

        let tp = result_type.clone();
        // Reset val so build_comprehension_code creates a fresh result vector.
        *val = Value::Null;
        self.build_comprehension_code(
            result_vec,
            &Value::Var(result_vec),
            elm,
            &out_elem,
            &in_type,
            &var_tp,
            for_var,
            for_next,
            None,
            // The SOURCE, not `result_vec` — that is what this loop appends to.
            if matches!(in_type, Type::Vector(_, _)) {
                Some((Value::Var(vec_copy_var), iter_var))
            } else {
                None
            },
            fill,
            create_iter_code,
            if_step,
            body,
            val,
            false,
            false,
            true,
            tp,
        )
    }

    /// Build ops to construct a struct/struct-enum instance, replicating the IR that
    /// `parse_object` produces. Returns the ops list and the work variable holding the result.
    fn build_object_ops(&mut self, td_nr: u32, fields: &[(usize, Value)]) -> (Vec<Value>, u16) {
        let ret = self.data.def(td_nr).returned().clone();
        let w = self.vars.work_refs(&ret, &mut self.lexer);
        self.data.set_referenced(td_nr, self.context, Value::Null);
        let tp = i32::from(self.data.def(td_nr).known_type());
        let mut list: Vec<Value> = vec![
            v_set(w, Value::Null),
            self.cl("OpDatabase", &[Value::Var(w), Value::Int(tp)]),
        ];
        for &(f_nr, ref val) in fields {
            list.push(self.set_field_no_check(td_nr, f_nr, 0, Value::Var(w), val.clone()));
        }
        (list, w)
    }

    /// Compile-time unroll `for f in s#fields` into one block per field.
    ///
    /// `loop_var_name` is the name this loop BINDS, `src_var_name` the name the program
    /// wrote — they differ from the second loop over a name onward (loft#915).
    fn parse_field_iteration(
        &mut self,
        loop_var_name: &str,
        src_var_name: &str,
        struct_def_nr: u32,
        source_expr: &Value,
        code: &mut Value,
    ) {
        let field_def_nr = self.data.def_nr("StructField");
        let field_type = Type::Reference(field_def_nr, crate::data::Deps::none());
        let loop_var = self.create_var(loop_var_name, &field_type);
        if loop_var_name != src_var_name {
            self.vars.set_name(src_var_name, loop_var);
        }
        self.vars.defined(loop_var);

        let mut body = Value::Null;
        self.parse_block("fields", &mut body, &Type::Void);

        // @PLN85 45-field-iter — OWNED-text locals bound in the body (a text
        // match-payload binding `is FvText { v }`, or a `"{v}"` interpolation
        // `__work`) are PER-ITERATION temporaries.  The body is parsed ONCE and
        // cloned per field below, so all clones would share ONE var; the scope
        // machinery then frees it only at its LAST textual use — which sits inside
        // a CONDITIONAL match arm — and when the last field doesn't take that arm
        // the owned copy from an earlier text field orphans on the interpreter
        // (native RAII-drops it).  Give each field-block its OWN copy so each is
        // freed at its own block's use, exactly like a real loop's per-iteration
        // scope.  Identify them as owned-text vars ASSIGNED (`Set`) inside the body
        // — an outer accumulator (`r += v`) is an `OpAppendText`, not a `Set`, so it
        // is left shared; a borrow/skip_free binding owns no allocation.
        let mut set_targets: Vec<u16> = Vec::new();
        Self::collect_set_targets(&body, &mut set_targets);
        let owned_text_locals: Vec<u16> = set_targets
            .into_iter()
            .filter(|&v| {
                (v as usize) < self.vars.count() as usize
                    && matches!(self.vars.tp(v).base(), Type::Text(_))
                    // @FR-O-Proxy asks copy — which bindings get a FRESH per-field copy
                    // (`copy_variable` + `remap_var_deep` below).  The frees that follow are
                    // of those new bindings, each allocating its own text, and never of the
                    // binding tested here — so @FR-O-Override is not this site's question.
                    //
                    // ⚠ The prose above overstates the filter: it says "a borrow/skip_free
                    // binding owns no allocation", but only the BORROW half is asked.  A
                    // `skip_free` binding does reach here — measured on 8 of the 1119 corpus
                    // files — so adding the veto is a live behaviour change, not a guard, and
                    // it belongs with a leak measurement rather than with a rule citation.
                    // OWNED text (empty deps) — a payload-copy binding, not a borrow view.
                    && self.vars.tp(v).depend().is_empty()
                    && !self.vars.is_argument(v)
                    // ONLY the match-payload binding (`_mv_<field>`): its free
                    // lands at the OUTER last-use (a conditional arm) so it orphans
                    // across the unroll.  An interpolation `__work` buffer is freed
                    // per-statement WITHIN its arm already — remapping it splits the
                    // var away from that free and re-introduces a leak.
                    && self.vars.name(v).starts_with("_mv_")
            })
            .collect();

        let num_attrs = self.data.attributes(struct_def_nr);
        let mut blocks: Vec<Value> = Vec::new();

        // work_checkpoint + clean_work_refs removed — see comment at the
        // end of this loop explaining why skip_free must NOT be set here.
        for a in 0..num_attrs {
            let attr_name = self.data.attr_name(struct_def_nr, a);
            let attr_type = self.data.attr_type(struct_def_nr, a);

            // @PLN25 — peel the nullable wrapper FIRST. `text?` is
            // `Optional(Text)`, which no arm below names, so without this it
            // fell into the `_ => continue` meant for records and vectors and
            // the field was SILENTLY SKIPPED: the loop simply ran fewer times,
            // and `b: text? = "y"` — a real value — never appeared.  That also
            // made `#fields` and `type_of(x).fields` disagree about what a
            // struct's fields are, which is two accounts of one layout.
            //
            // `Optional(τ)` and `τ` share the runtime layout (no wrapper, a
            // sentinel for absent), so the READ is the same one either way;
            // only the declaration differs, and `StructField.nullable` below is
            // where that difference is reported instead of swallowed.
            let (base_type, nullable) = attr_type.peel_optional();
            let variant_name = match base_type {
                Type::Boolean => "FvBool",
                // Post-2c round 10c: wide Type::Integer (former Type::Long)
                // maps to FvLong; narrow range maps to FvInt.
                Type::Integer(s) if s.is_wide() => "FvLong",
                Type::Integer(_) => "FvInt",
                Type::Float => "FvFloat",
                Type::Single => "FvSingle",
                Type::Character => "FvChar",
                Type::Text(_) => "FvText",
                // A record, a vector or a keyed collection genuinely has no
                // scalar payload to carry. This arm is for those, and for
                // nothing else — a type that merely LOOKS unfamiliar here is
                // how the nullable drop happened.
                _ => continue,
            };

            let field_read = self.get_field(struct_def_nr, a, source_expr.clone());
            // @PLN22 Phase 1 — the FieldValue reflection variants (FvBool, FvInt,
            // …) resolve within the FieldValue enum via the variant_of chokepoint,
            // not the bare global def_nr (the enum itself stays globally keyed).
            let fv_enum = self.data.def_nr("FieldValue");
            let variant_def_nr = self.data.variant_of(fv_enum, variant_name);
            let disc_val = self.data.def(variant_def_nr).attributes()[0].value.clone();

            // Construct FieldValue variant as Value::Insert (flat ops list).
            let (fv_ops, fv_work) =
                self.build_object_ops(variant_def_nr, &[(0, disc_val), (1, field_read)]);
            let fv_insert = Value::Insert(fv_ops);

            // Construct StructField: the FieldValue is passed as Value::Var(fv_work)
            // after the Insert has executed.  `nullable` rides alongside because
            // the payload variants are typed non-null: a nullable field's value
            // arrives as loft's sentinel, and this is what tells a reader that
            // is possible rather than leaving it to be discovered.
            let (sf_ops, sf_work) = self.build_object_ops(
                field_def_nr,
                &[
                    (0, Value::Text(attr_name)),
                    (1, Value::Var(fv_work)),
                    (2, Value::Boolean(nullable)),
                ],
            );
            let sf_insert = Value::Insert(sf_ops);

            blocks.push(fv_insert);
            blocks.push(sf_insert);
            blocks.push(v_set(loop_var, Value::Var(sf_work)));
            // Fresh per-field copies of the owned-text bindings so each field-block
            // frees its own (see `owned_text_locals` above).
            let mut this_body = body.clone();
            for &v in &owned_text_locals {
                let fresh = self.vars.copy_variable(v);
                Self::remap_var_deep(&mut this_body, v, fresh);
            }
            blocks.push(this_body);
        }
        // do NOT call clean_work_refs here.  The unrolled loop
        // creates 2 work-refs per iteration (FvFloat/etc + StructField)
        // and assigns the latter to loop_var via v_set.  Only the LAST
        // iteration's work-refs feed loop_var; earlier ones are orphaned.
        // Marking them all skip_free prevented get_free_vars from
        // emitting OpFreeRef at scope exit, leaking 1 store per
        // orphaned work-ref (8 stores for a 3-field + 4-field struct).
        // The scan_set var-copy companion (Set(v, Var(src)) path) already
        // strips loop_var's deps so it gets its own OpFreeRef; the
        // work-refs themselves pass get_free_vars's is_work_ref check.

        if blocks.is_empty() {
            *code = Value::Null;
        } else {
            *code = v_block(blocks, Type::Void, "field_iter");
        }
    }

    /// Deep-remap every occurrence of var `from` → `to` in `val`, recursing ALL
    /// container variants (unlike `remap_var_nr`, which only walks
    /// `Call`/`Set`/`Insert` for flat default-expression trees).  Used by
    /// `parse_field_iteration` to give each unrolled field-block its own copy of a
    /// body-local text binding.
    /// @PLN104 — renumber a FRAME variable `from` → `to` through a `Value` IR tree,
    /// moving BOTH the `Value::Var`/`Set`/… references AND the frame deps inside every
    /// embedded type (`Block.result`, `FnRef`).  This is the type-dep-aware companion
    /// `remap_var_deep` lacks: a variable swap that renumbers only the IR references
    /// desyncs the cref-buffer / block-result type deps (loft-lang/loft#568).  The
    /// CALLER must also renumber the variable-table typedefs (`Type::renumber_frame_deps`
    /// on each `vars.tp`) and swap the `Variable` structs — see the retbuf swap in
    /// `block_result`.  Do a 3-way `from→TMP→to` to swap two live indices atomically.
    pub(crate) fn renumber_frame_var(val: &mut Value, from: u16, to: u16) {
        match val {
            Value::Var(n) if *n == from => *n = to,
            Value::Set(v, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::renumber_frame_var(inner, from, to);
            }
            // `CallRef`'s first field is the fn-ref VARIABLE (remap it); `Call`'s is a
            // def_nr (a definition, NOT a variable — leave it). Missing the CallRef var
            // desyncs it from the swapped variable table → codegen's `generate_call_ref`
            // reads a non-`Function` slot and panics (loft-lang/loft#568 default-on).
            Value::CallRef(v, xs) => {
                if *v == from {
                    *v = to;
                }
                for x in xs {
                    Self::renumber_frame_var(x, from, to);
                }
            }
            Value::Call(_, xs) | Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
                for x in xs {
                    Self::renumber_frame_var(x, from, to);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                bl.result.renumber_frame_deps(from, to);
                for op in &mut bl.operators {
                    Self::renumber_frame_var(op, from, to);
                }
            }
            Value::If(c, t, e) => {
                Self::renumber_frame_var(c, from, to);
                Self::renumber_frame_var(t, from, to);
                Self::renumber_frame_var(e, from, to);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::renumber_frame_var(inner, from, to);
            }
            Value::Span(b) => Self::renumber_frame_var(&mut b.1, from, to),
            Value::Iter(v, a, b, c) => {
                if *v == from {
                    *v = to;
                }
                Self::renumber_frame_var(a, from, to);
                Self::renumber_frame_var(b, from, to);
                Self::renumber_frame_var(c, from, to);
            }
            Value::TupleGet(v, _) => {
                if *v == from {
                    *v = to;
                }
            }
            Value::TuplePut(v, _, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::renumber_frame_var(inner, from, to);
            }
            Value::FnRef(_, v, ty) => {
                if *v == from {
                    *v = to;
                }
                ty.renumber_frame_deps(from, to);
            }
            _ => {}
        }
    }

    fn remap_var_deep(val: &mut Value, from: u16, to: u16) {
        match val {
            Value::Var(n) if *n == from => *n = to,
            Value::Set(v, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::remap_var_deep(inner, from, to);
            }
            Value::Call(_, xs)
            | Value::CallRef(_, xs)
            | Value::Insert(xs)
            | Value::Tuple(xs)
            | Value::Parallel(xs) => {
                for x in xs {
                    Self::remap_var_deep(x, from, to);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &mut bl.operators {
                    Self::remap_var_deep(op, from, to);
                }
            }
            Value::If(c, t, e) => {
                Self::remap_var_deep(c, from, to);
                Self::remap_var_deep(t, from, to);
                Self::remap_var_deep(e, from, to);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::remap_var_deep(inner, from, to);
            }
            Value::Span(b) => Self::remap_var_deep(&mut b.1, from, to),
            Value::Iter(v, a, b, c) => {
                if *v == from {
                    *v = to;
                }
                Self::remap_var_deep(a, from, to);
                Self::remap_var_deep(b, from, to);
                Self::remap_var_deep(c, from, to);
            }
            Value::TupleGet(v, _) => {
                if *v == from {
                    *v = to;
                }
            }
            Value::TuplePut(v, _, inner) => {
                if *v == from {
                    *v = to;
                }
                Self::remap_var_deep(inner, from, to);
            }
            _ => {}
        }
    }

    /// Collect the var numbers that are the TARGET of a `Set` anywhere in `val`
    /// (recursing every child).  Used by `parse_field_iteration` to find the
    /// body-local temporaries an unrolled field-block assigns (and so must own a
    /// private copy of).  Duplicates are harmless — the caller filters + copies
    /// each once.
    fn collect_set_targets(val: &Value, out: &mut Vec<u16>) {
        match val.unspan() {
            Value::Set(v, inner) => {
                if !out.contains(v) {
                    out.push(*v);
                }
                Self::collect_set_targets(inner, out);
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    Self::collect_set_targets(op, out);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::collect_set_targets(op, out);
                }
            }
            Value::If(c, t, e) => {
                Self::collect_set_targets(c, out);
                Self::collect_set_targets(t, out);
                Self::collect_set_targets(e, out);
            }
            Value::Return(inner) | Value::Drop(inner) => Self::collect_set_targets(inner, out),
            Value::Call(_, args) => {
                for a in args {
                    Self::collect_set_targets(a, out);
                }
            }
            _ => {}
        }
    }

    /// Compute the in-store byte size of a vector element type.
    pub(crate) fn element_store_size(&self, elm: &Type) -> i32 {
        let elm_td = self.data.type_elm(elm);
        // Post-2c: honor size(N) on integer aliases.  Must run before the
        // generic `known_type → database.size(...)` path below, because
        // database.size for the 8-byte integer base returns 8 regardless.
        if matches!(elm, Type::Integer(_))
            && let Some(n) = self.data.forced_size(elm_td)
        {
            return i32::from(n);
        }
        // B5 (2026-04-13): for a mixed struct-enum element type
        // (`Type::Enum(_, true, _)`), the parent enum's `known_type` is
        // a byte-sized enumerate (size 1) — wrong for vector storage,
        // since instances are records.  Use the size of the largest
        // variant's structure type instead.  Without this, recursive
        // struct-enums (`vector<Tree>` inside Tree's own variant) trip
        // `OpDatabase(db_tp=u16::MAX)` panics in `Store::claim`.
        if let Type::Enum(parent_d_nr, true, _) = elm
            && elm_td != u32::MAX
        {
            let mut max_size = 0i32;
            for a_nr in 0..self.data.attributes(*parent_d_nr) {
                let variant_name = self.data.attr_name(*parent_d_nr, a_nr);
                let variant_d_nr = self.data.def_nr(&variant_name);
                if variant_d_nr != u32::MAX {
                    let variant_known = self.data.def(variant_d_nr).known_type();
                    let s = i32::from(self.database.size(variant_known));
                    if s > max_size {
                        max_size = s;
                    }
                }
            }
            if max_size > 0 {
                return max_size;
            }
        }
        if elm_td != u32::MAX {
            let known = self.data.def(elm_td).known_type();
            let db_size = i32::from(self.database.size(known));
            if db_size > 0 {
                return db_size;
            }
        }
        // Fallback for primitive types
        match elm {
            Type::Single | Type::Boolean | Type::Character | Type::Text(_) => 4,
            Type::Integer(_) | Type::Float => 8,
            _ => 12, // DbRef size for reference types
        }
    }

    /// Compiler special-case for `sort(v: vector<T>)`.
    /// Emits `OpSortVector(v, db_tp)` which sorts in-place at runtime, dispatching
    /// on the database element type.
    pub(crate) fn parse_sort(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "sort requires 1 argument: sort(vector)"
            );
            return Type::Void;
        }
        if let Type::Vector(elm, _) = &types[0] {
            if !matches!(
                elm.as_ref(),
                Type::Integer(_) | Type::Float | Type::Single | Type::Text(_)
            ) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "sort is not supported for vector<{}>; use integer, long, float, single, or text",
                    elm.name(&self.data)
                );
                return Type::Void;
            }
            let info = self.type_info(elm);
            *val = self.cl("OpSortVector", &[list[0].clone(), info]);
        } else {
            diagnostic!(self.lexer, Level::Error, "sort requires a vector argument");
        }
        Type::Void
    }

    /// Compiler special-case for `insert(v: vector<T>, idx: integer, elem: T)`.
    /// Emits `OpInsertVector` to create space, then the appropriate `OpSet*` to write the value.
    pub(crate) fn parse_insert(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "insert requires 3 arguments: insert(vector, index, element)"
            );
            return Type::Void;
        }
        let elm_tp = if let Type::Vector(elm, _) = &types[0] {
            (**elm).clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "insert requires a vector as first argument"
            );
            return Type::Void;
        };
        let elm_size = Value::Int(self.element_store_size(&elm_tp));
        let db_tp = self.type_info(&elm_tp);
        let ed_nr = self.data.type_def_nr(&elm_tp);
        // Create a temp var with dependency on the vector to prevent premature free
        let ref_tp = Type::Reference(ed_nr, crate::data::Deps::frame(types[0].depend()));
        let tmp = self.create_unique("ins", &ref_tp);
        if let Value::Var(vec_var) = &list[0] {
            self.vars.depend(tmp, *vec_var);
        }
        // tmp = OpInsertVector(v, elem_size, idx, db_tp)
        let insert_call = self.cl(
            "OpInsertVector",
            &[list[0].clone(), elm_size, list[1].clone(), db_tp],
        );
        let set_val = self.set_field(ed_nr, usize::MAX, 0, Value::Var(tmp), list[2].clone());
        *val = v_block(vec![v_set(tmp, insert_call), set_val], Type::Void, "insert");
        Type::Void
    }

    /// Compiler special-case for `reserve(v: vector<T>, n: integer)` (loft#710).
    ///
    /// A hint, never a promise about contents: it gives `v` room for `n`
    /// elements so filling it does not run the doubling ladder, and changes
    /// neither `len(v)` nor what is in it.  A compiler special-case for the same
    /// reason `reverse` is one — the element's stored width is known here and
    /// nowhere else, and the op needs it to size the claim.
    pub(crate) fn parse_reserve(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reserve requires 2 arguments: reserve(vector, count)"
            );
            return Type::Void;
        }
        if !matches!(types[1].base(), Type::Integer(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reserve's count must be an integer — reserve(collection, count)"
            );
            return Type::Void;
        }
        // @PLN135 arc C — a `hash` reserves its BUCKET TABLE, which is a different
        // allocation from a vector's element block, so it needs its own op.  The
        // contract is the same one `reserve(v, n)` states: capacity only, never the
        // contents or the length, and a count the collection already covers does
        // nothing.  Filling a 1M-entry hash otherwise rebuilds the table 17 times.
        if matches!(&types[0], Type::Hash(_, _, _)) {
            let Some(kt) = self.keyed_known_type(&types[0]) else {
                // The collection type never resolved; the cause is already reported.
                return Type::Void;
            };
            *val = self.cl(
                "OpReserveHash",
                &[list[0].clone(), list[1].clone(), Value::Int(i32::from(kt))],
            );
            return Type::Void;
        }
        let Type::Vector(elm, _) = &types[0] else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reserve takes a vector or a hash as its first argument — a sorted, \
                 index, spatial or trie collection has no capacity to set"
            );
            return Type::Void;
        };
        let elm_size = self.element_store_size(elm);
        *val = self.cl(
            "OpReserveVector",
            &[list[0].clone(), list[1].clone(), Value::Int(elm_size)],
        );
        Type::Void
    }

    /// Compiler special-case for `reverse(v: vector<T>)`.
    /// Dispatches to `OpReverseVector` which works for any element type.
    pub(crate) fn parse_reverse(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return Type::Void;
        }
        if list.len() != 1 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reverse requires 1 argument: reverse(vector)"
            );
            return Type::Void;
        }
        let elm_size = if let Type::Vector(elm, _) = &types[0] {
            self.element_store_size(elm)
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reverse requires a vector argument"
            );
            return Type::Void;
        };
        *val = self.cl("OpReverseVector", &[list[0].clone(), Value::Int(elm_size)]);
        Type::Void
    }

    /// Validate arguments for `any`/`all`/`count_if`: (vector, fn-pred→boolean).
    ///
    /// The second half of the answer is the predicate's STATIC definition number, and it is
    /// `None` for a fn-ref VALUE — a lambda that captures, or a fn-ref held in a variable.
    /// Those reach the callee through `CallRef` rather than `Call`, exactly as `filter` does
    /// (`parse_filter_validate`).  Answering `None` for the whole call there instead is what
    /// made `count_if(v, |x| { x > k })` compile to nothing at all (loft#1001).
    fn validate_predicate_args(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> Option<(Type, Option<u32>)> {
        if list.len() != 2 {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name} requires 2 arguments: {name}(vector, fn pred)"
                );
            }
            return None;
        }
        let elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: first argument must be a vector"
                );
            }
            return None;
        };
        if let Type::Function(params, ret, _) = &types[1] {
            if params.len() != 1 && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: predicate must take exactly one argument"
                );
            }
            if **ret != Type::Boolean && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "{name}: predicate must return boolean"
                );
            }
        } else if !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "{name}: second argument must be a function reference (use fn <name>)"
            );
            return None;
        }
        // Accept both a static fn-ref (`Value::Int`) and a fn-ref VALUE (a capturing lambda
        // or a fn-ref variable); the caller picks `Call` or `CallRef` off this.
        let fn_d_nr = if let Value::Int(d) = &list[1] {
            Some(*d as u32)
        } else {
            None
        };
        Some((elem_type, fn_d_nr))
    }

    /// Build the iteration preamble shared by `any`/`all`/`count_if`: copies the
    /// vector, creates an iterator, and returns the loop scaffolding.
    ///
    /// `fn_d_nr` is the predicate's static definition number, or `None` for a fn-ref VALUE —
    /// which the preamble then binds to a local so the loop body can reach it with `CallRef`.
    fn predicate_loop_scaffold(
        &mut self,
        name: &str,
        list: &[Value],
        types: &[Type],
        fn_d_nr: Option<u32>,
    ) -> PredicateLoop {
        // A fn-ref VALUE has to live in a local: the loop body is built before the preamble
        // runs, so it needs a slot to name rather than the expression that produced it.
        let fn_ref_var = if fn_d_nr.is_none() {
            let v = self.create_unique(&format!("{name}_fn"), &types[1]);
            self.vars.defined(v);
            Some(v)
        } else {
            None
        };

        let mut in_type = types[0].clone();
        let vec_var = self.create_unique(&format!("{name}_vec"), &in_type);
        in_type = in_type.depending(vec_var);

        let iter_var = self.create_unique(&format!("{name}_idx"), &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique(&format!("{name}_elm"), &var_tp);
        self.vars.defined(for_var);

        let mut create_iter = Value::Var(vec_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        // loft#1000 — a VECTOR ends on its length, not on the element's value.
        let break_if_done = if matches!(in_type, Type::Vector(_, _)) {
            let brk = self.vector_loop_break(&Value::Var(vec_var), iter_var);
            Value::Insert(brk)
        } else {
            let mut test_for = Value::Var(for_var);
            self.convert(&mut test_for, &var_tp, &Type::Boolean);
            let not_test = self.cl("OpNot", &[test_for]);
            v_if(
                not_test,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            )
        };

        let mut preamble = vec![v_set(vec_var, list[0].clone())];
        if let Some(fv) = fn_ref_var {
            preamble.push(v_set(fv, list[1].clone()));
        }
        preamble.push(create_iter);
        // N8a.4: return for_next and break_if_done as separate values so callers
        // inline them directly in the loop body.  A v_block wrapper would declare
        // `for_var` inside a nested Rust `{ }` block, making it invisible to the
        // short_circuit/count_step expression that follows in native code.
        let (elem_arg, _) = self.callback_element_arg(&in_type, for_var, &var_tp);
        PredicateLoop {
            preamble,
            elem_arg,
            for_next,
            break_if_done,
            fn_ref_var,
        }
    }

    /// `any(vec, pred)` — true if pred returns true for any element.
    pub(crate) fn parse_any(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Boolean;
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("any", list, types) else {
            return Type::Boolean;
        };

        let acc = self.create_unique("any_acc", &Type::Boolean);
        self.vars.defined(acc);

        let lp = self.predicate_loop_scaffold("any", list, types, fn_d_nr);

        // if pred(elem) { acc = true; break }
        let pred_call = predicate_call(fn_d_nr, &lp);
        let short_circuit = v_if(
            pred_call,
            v_block(
                vec![v_set(acc, Value::Boolean(true)), Value::Break(0)],
                Type::Void,
                "any_hit",
            ),
            Value::Null,
        );

        let PredicateLoop {
            preamble,
            for_next,
            break_if_done,
            ..
        } = lp;
        let loop_body = vec![for_next, break_if_done, short_circuit];
        let mut stmts = vec![v_set(acc, Value::Boolean(false))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "any"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, Type::Boolean, "any");
        Type::Boolean
    }

    /// `all(vec, pred)` — true if pred returns true for every element.
    pub(crate) fn parse_all(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            return Type::Boolean;
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("all", list, types) else {
            return Type::Boolean;
        };

        let acc = self.create_unique("all_acc", &Type::Boolean);
        self.vars.defined(acc);

        let lp = self.predicate_loop_scaffold("all", list, types, fn_d_nr);

        // if !pred(elem) { acc = false; break }
        let pred_call = predicate_call(fn_d_nr, &lp);
        let not_pred = self.cl("OpNot", &[pred_call]);
        let short_circuit = v_if(
            not_pred,
            v_block(
                vec![v_set(acc, Value::Boolean(false)), Value::Break(0)],
                Type::Void,
                "all_miss",
            ),
            Value::Null,
        );

        let PredicateLoop {
            preamble,
            for_next,
            break_if_done,
            ..
        } = lp;
        let loop_body = vec![for_next, break_if_done, short_circuit];
        let mut stmts = vec![v_set(acc, Value::Boolean(true))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "all"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, Type::Boolean, "all");
        Type::Boolean
    }

    /// `count_if(vec, pred)` — count of elements where pred returns true.
    pub(crate) fn parse_count_if(
        &mut self,
        val: &mut Value,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        if self.first_pass {
            return I32.clone();
        }
        let Some((_, fn_d_nr)) = self.validate_predicate_args("count_if", list, types) else {
            return I32.clone();
        };

        let acc = self.create_unique("cntif_acc", &I32);
        self.vars.defined(acc);

        let lp = self.predicate_loop_scaffold("count_if", list, types, fn_d_nr);

        // if pred(elem) { acc += 1 }
        let pred_call = predicate_call(fn_d_nr, &lp);
        let inc = v_set(acc, self.cl("OpAddInt", &[Value::Var(acc), Value::Int(1)]));
        let count_step = v_if(pred_call, inc, Value::Null);

        let PredicateLoop {
            preamble,
            for_next,
            break_if_done,
            ..
        } = lp;
        let loop_body = vec![for_next, break_if_done, count_step];
        let mut stmts = vec![v_set(acc, Value::Int(0))];
        stmts.extend(preamble);
        stmts.push(v_loop(loop_body, "count_if"));
        stmts.push(Value::Var(acc));

        *val = v_block(stmts, I32.clone(), "count_if");
        I32.clone()
    }
}

/// Recursively rename variable `from` to `to` everywhere in `val` — as a READ
/// (`Value::Var` and the other var-carrying reads) AND as a BINDING TARGET (the
/// slot of `Set`/`TuplePut`/`Iter` and a block's `scope`).  Unlike
/// `replace_var_in_ir` (which rewrites reads only, leaving binding targets), this
/// is a total substitution: after it, `from` no longer appears anywhere.
///
/// Used by the vector-literal receiver fix (loft#501): when a literal reused the
/// outer assignment LHS as its build accumulator but a `.`/`[` chain follows, the
/// accumulator is renamed to a fresh synthetic local so the LHS is free to receive
/// the chain's result.
pub(crate) fn rename_var(val: &mut Value, from: u16, to: u16) {
    match val {
        Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) | Value::FnRef(_, v, _) => {
            if *v == from {
                *v = to;
            }
        }
        Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Text(_)
        | Value::Enum(_, _)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::Null
        | Value::RawExpr(_) => {}
        Value::CallRef(v, args) => {
            if *v == from {
                *v = to;
            }
            for a in args.iter_mut() {
                rename_var(a, from, to);
            }
        }
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            for a in args.iter_mut() {
                rename_var(a, from, to);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            if bl.scope == from {
                bl.scope = to;
            }
            for op in &mut bl.operators {
                rename_var(op, from, to);
            }
        }
        Value::Set(t, body) | Value::TuplePut(t, _, body) => {
            if *t == from {
                *t = to;
            }
            rename_var(body, from, to);
        }
        Value::Return(body) | Value::Drop(body) | Value::Yield(body) => {
            rename_var(body, from, to);
        }
        Value::If(cond, t, f) => {
            rename_var(cond, from, to);
            rename_var(t, from, to);
            rename_var(f, from, to);
        }
        Value::Iter(t, a, b, c) => {
            if *t == from {
                *t = to;
            }
            rename_var(a, from, to);
            rename_var(b, from, to);
            rename_var(c, from, to);
        }
        Value::Span(b) => rename_var(&mut b.1, from, to),
    }
}

/// Plan-04 B.3 follow-up v2: recursively walk `val` and replace every
/// `Value::Var(target)` with a clone of `replacement`.  Used by
/// `build_parallel_for_ir` to inline-expand the par loop variable `b`
/// to its element-accessor expression, so that `b` is a parse-time
/// alias rather than a runtime slot.  See
/// `doc/claude/plans/finished/04-slot-assignment-redesign/b3-par-inline.md`.
fn replace_var_in_ir(val: &mut Value, target: u16, replacement: &Value) {
    match val {
        Value::Var(v) if *v == target => {
            *val = replacement.clone();
        }
        Value::Var(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Text(_)
        | Value::Enum(_, _)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_)
        | Value::Null => {}
        Value::Call(_, args)
        | Value::CallRef(_, args)
        | Value::Insert(args)
        | Value::Tuple(args)
        | Value::Parallel(args) => {
            for a in args.iter_mut() {
                replace_var_in_ir(a, target, replacement);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &mut bl.operators {
                replace_var_in_ir(op, target, replacement);
            }
        }
        Value::Set(_, body)
        | Value::Return(body)
        | Value::Drop(body)
        | Value::TuplePut(_, _, body)
        | Value::Yield(body) => {
            replace_var_in_ir(body, target, replacement);
        }
        Value::If(cond, t, f) => {
            replace_var_in_ir(cond, target, replacement);
            replace_var_in_ir(t, target, replacement);
            replace_var_in_ir(f, target, replacement);
        }
        Value::Iter(_, a, b, c) => {
            replace_var_in_ir(a, target, replacement);
            replace_var_in_ir(b, target, replacement);
            replace_var_in_ir(c, target, replacement);
        }
        // Plan-07 phase 1 — Span is transparent; recurse into the
        // wrapped node.
        Value::Span(b) => replace_var_in_ir(&mut b.1, target, replacement),
        // Plan-06 spine step 3 — recurse into all child Values.
        // Phase 09 phase 00 step 0.7 — RawExpr is a codegen-internal
        // synthetic value; the parser walker never produces or sees it.
        Value::RawExpr(_) => {}
    }
}

/// One element place of a linked group's VECTOR member, as [`Parser::group_elem_write`]
/// needs it: the member (`coll`, an `OpGetField` read), the struct it is read from (`base`),
/// that struct's type and the member's byte offset, and the whole group.
struct GroupElemSite {
    coll: Value,
    base: Value,
    struct_tp: u16,
    byte_off: u16,
    members: Vec<(u16, u16, bool)>,
}
