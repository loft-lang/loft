// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Call ARGUMENTS: `scan_args` gives an inline-built or call-minted argument an owner, binds a
//! borrowed argument to the store it reads, and writes a `&` argument back.

use super::Scopes;
use super::handoff::copy_hands_off;
use crate::data::{Data, Deps, Type, Value, v_set};
use crate::variables::Function;

/// What an argument LIFTED into a `__lift_N` temp borrows — the deps its type must carry.
///
/// A lift temp holds a value the caller reads out of something it does not own: an element
/// of a container, a field of a record, the surviving arm of a `??`.  Typed without deps it
/// reads as the OWNER of that store and `get_free_vars` emits a scope-exit `OpFreeRef` for
/// it — releasing a container the caller still names.  Where the container is a local of the
/// same frame the bogus free lands on a store that was dying anyway and nothing reports it;
/// where it OUTLIVES the frame (a parameter, a global) the next allocation recycles the
/// record and the container reads back as another type's bytes.
///
/// So the temp borrows what the value borrows.  The walk bottoms out at:
/// * a plain `Var` — the chain reads out of that local's store;
/// * a `TupleGet` — out of the tuple's;
/// * a BLOCK, whose `result` type already carries the deps the parser derived for it (a
///   `??` lowers to one, and its type names the container the surviving arm reads);
/// * an `If`, where either arm can be the value, so the deps are the union.
///
/// `None` where none of those is reached — a value whose source cannot be named must not be
/// bound at all.  The caller then leaves the argument as it was, which costs the leak that
/// is already there; a temp typed as an owner of somebody else's store costs a
/// use-after-free, and a leak is the better of those two.
fn lift_view_deps(arg: &Value, data: &Data) -> Option<Vec<u16>> {
    match arg.unspan() {
        Value::Var(v) => Some(vec![*v]),
        Value::TupleGet(base, _) => Some(vec![*base]),
        Value::Block(bl) => {
            let d = bl.result.depend();
            if d.is_empty() { None } else { Some(d) }
        }
        Value::If(_, then_v, else_v) => {
            let mut d = lift_view_deps(then_v, data)?;
            for x in lift_view_deps(else_v, data)? {
                if !d.contains(&x) {
                    d.push(x);
                }
            }
            Some(d)
        }
        Value::Call(d_nr, cargs) if crate::use_analysis::is_projection_op(data, *d_nr) => {
            lift_view_deps(cargs.first()?, data)
        }
        _ => None,
    }
}

/// loft#890, loft#1647 — the argument index whose STORE `outer_call` frees WHOLE for
/// itself, via the `0x8000` source-free bit on its `const u16` type parameter.
///
/// `OpReplaceKeyed` and `OpCopyRecord` answer: each releases its source's whole store
/// (`Stores::free` takes the store, not the record) once the copy is made, and a source
/// that is an inline call the scope pass lifts into a `__lift_N` is a store that temp also
/// owns.  Its scope-exit `OpFreeRef` is then a second release — silent while the slot stays
/// free, and a stolen store the moment the allocator hands that slot to anything minted
/// before the scope ends: `m.vs += [vert(a, up)]` followed by a lazily minted return buffer
/// in the same pass freed that buffer at the pass's end, and the next pass's value was read
/// from whatever took the slot after it (@FR-H-FreeAll: a store is released exactly once;
/// @FR-H-FreeTwice names the slot reuse a second release hits).  Where the
/// copy declines its release (the same store as the destination, a stack, free, read-only
/// or free-protected store) the lift's own free would release nothing it owns either.
fn moved_source_arg(outer_call: u32, args: &[Value], data: &Data) -> Option<usize> {
    if outer_call == u32::MAX
        || (outer_call != data.def_nr("OpReplaceKeyed")
            && outer_call != data.def_nr("OpCopyRecord"))
    {
        return None;
    }
    matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0).then_some(0)
}

impl Scopes<'_> {
    /// Record what the `__lift_N` temp `tmp` — holding argument `arg_idx` of the call
    /// `scan_args` is lowering — no longer owns, because the call takes it over.
    ///
    /// Two different hand-offs, and they cost different things to get wrong:
    ///
    /// - the **drop** (@PLN139 stage C): a copy into a container field or a collection
    ///   element makes the container the releaser, so running the source's own scope-end
    ///   hook releases one resource twice.
    /// - the **store** (loft#890): a `0x8000` move FREES the source store inside the op.
    ///   The lift still names it, so its scope-exit `OpFreeRef` is a second free — silent
    ///   while the slot stays free, and a stolen store the moment the allocator hands
    ///   that slot to somebody else.  `br = mk_hash(n); br[7, 0]` in a record-returning
    ///   function is exactly that: the return buffer is allocated between the two frees
    ///   and lands on the recycled slot, so the function returns freed bytes.
    fn mark_lift_handoff(
        &mut self,
        tmp: u16,
        arg_idx: usize,
        transfer_copy: bool,
        moved_arg: Option<usize>,
    ) {
        if transfer_copy && arg_idx == 0 || moved_arg == Some(arg_idx) {
            self.drop_transferred.insert(tmp);
        }
        if moved_arg == Some(arg_idx) {
            self.free_transferred.insert(tmp);
        }
    }

    /// loft#1512 — give every call-minted member of a tuple-literal ARGUMENT an owner, at
    /// any nesting depth: each member `inline_struct_return` can type is moved into a
    /// `__lift_N` (declared in `preamble`, so it lives at the statement's scope and its
    /// scope-end cascade releases the record once), and the member slot reads the temp.
    /// Returns whether anything was lifted, so an untouched tuple keeps its original
    /// (span-carrying) value.  Members the lift cannot type — a local, a projection, a
    /// scalar — are left as they were: those either own nothing or are owned elsewhere,
    /// and lifting them would move a view into an owner.
    #[allow(clippy::too_many_arguments)]
    fn lift_tuple_call_members(
        &mut self,
        members: &mut [Value],
        arg_idx: usize,
        transfer_copy: bool,
        moved_arg: Option<usize>,
        preamble: &mut Vec<Value>,
        function: &mut Function,
        data: &Data,
        outer_call: u32,
    ) -> bool {
        let mut lifted = false;
        for m in members.iter_mut() {
            if let Some(tp) = self.inline_struct_return(m, data, outer_call, function) {
                let tmp = self.new_lift_var(function, &tp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                let call = std::mem::replace(m, Value::Var(tmp));
                preamble.push(self.lift_set(tmp, call, function, data));
                lifted = true;
            } else if matches!(m.unspan(), Value::Tuple(_)) {
                let mut inner = match m.unspan() {
                    Value::Tuple(inner) => inner.clone(),
                    _ => unreachable!("matched Value::Tuple above"),
                };
                if self.lift_tuple_call_members(
                    &mut inner,
                    arg_idx,
                    transfer_copy,
                    moved_arg,
                    preamble,
                    function,
                    data,
                    outer_call,
                ) {
                    *m = Value::Tuple(inner);
                    lifted = true;
                }
            }
        }
        lifted
    }

    /// Scan a list of call arguments.
    ///
    /// If any scanned arg comes back as `Insert([Set(w, Null), body])` where `w` is an
    /// owned Reference (dep empty) — a hoisted closure-record allocation — the `Set(w,
    /// Null)` is lifted out into `preamble` and the arg is replaced with `body` alone.
    ///
    /// This prevents `ConvRefFromNull` (12 B) from landing on the eval stack *between*
    /// other call arguments, which would corrupt the `CallRef` argument layout and cause
    /// the lambda to receive garbage for `y` (A5.6 "Incorrect store" bug).
    ///
    /// Returns `(preamble, scanned_args)`.  The caller wraps the result as
    /// `Insert([preamble..., Call/CallRef(...)])` when the preamble is non-empty;
    /// `convert` flattens this so the preamble executes before any args are pushed.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn scan_args(
        &mut self,
        args: &[Value],
        function: &mut Function,
        data: &Data,
        outer_call: u32,
    ) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
        let mut preamble: Vec<Value> = Vec::new();
        let mut ls: Vec<Value> = Vec::new();
        // @PLN90 / loft#506 — POST-call store-backs for computed-lvalue `&`-write-back args.
        let mut postamble: Vec<Value> = Vec::new();
        // loft#1287 — the rebind witnesses to mark protected-from-free for THIS call.
        let mut amp_foreign: Vec<u16> = Vec::new();
        // #248 (interpreter arg-layout) — when the call's first argument is a
        // borrowed receiver pushed via `OpCreateStack(Var(_))` (a `&self` / `&T`
        // method or free-function call), a LATER argument that is an inline
        // heap-returning call which grows the eval frame (e.g. `tick(s, mk(), …)`
        // where `mk()` returns a vector via a hidden `__ref_N` work buffer)
        // shifts the receiver's stack slot relative to where the callee reads
        // `self`.  The interpreter then derefs the receiver at the wrong offset
        // and lands on a CONST_STORE record → "Write to read-only store".  The
        // native backend passes args by the Rust ABI and is immune.  Force such a
        // trailing call argument into a `__lift_N` temp (preamble), so it is
        // evaluated and its store materialised BEFORE the receiver `OpCreateStack`
        // is pushed — exactly the shape `x = mk(); tick(s, x, …)` that already
        // works on both backends.  Gated on a CreateStack-receiver first arg so
        // ordinary calls keep their existing argument lowering untouched.
        let create_stack_nr = data.def_nr("OpCreateStack");
        // @PLN139 stage C — the lift-site half of [`collect_drop_transferred`].  A copy that
        // hands its source off — the `0x8000` move into a collection element, or a copy
        // whose destination is a container FIELD — must not leave the source dropping what
        // it no longer owns.  When that source is an inline call it is lifted into a
        // `__lift_N` temp right here, so this is the only point that knows which temp the
        // copy took: before the lift the IR still names the CALL, and after it nothing
        // records the pairing.  The copy's source is arg 0.
        let moved_arg = moved_source_arg(outer_call, args, data);
        let copy_record_nr = data.def_nr("OpCopyRecord");
        let transfer_copy = outer_call == copy_record_nr && {
            let moved =
                matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0);
            moved
                || args
                    .get(1)
                    .is_some_and(|d| copy_hands_off(d, function, data))
        };
        let has_create_stack_receiver = args.first().is_some_and(|a| {
            matches!(a.unspan(), Value::Call(d, cargs)
                if *d == create_stack_nr
                    && matches!(cargs.first().map(Value::unspan), Some(Value::Var(_))))
        });
        for (arg_idx, a) in args.iter().enumerate() {
            // loft#1320 — a branch VALUE consumed by this call (`total(if c { g(a) } else { g(n) })`)
            // never reaches `scan_set`, so its arms get their owners here, homed in this statement's
            // scope: the call reads the temp and the scope's exit frees it by identity.
            let rewritten_branch_arg;
            let a: &Value = if Self::is_value_branch(a)
                && self.arm_tails_need_binding(a, u16::MAX, data, function)
            {
                let mut rw = a.clone();
                let home = self.scope;
                let _ = self.lift_join_arm_tails(&mut rw, home, u16::MAX, function, data);
                rewritten_branch_arg = rw;
                &rewritten_branch_arg
            } else {
                a
            };
            let scanned = self.scan(a, function, data);
            // #248 — force-lift a trailing inline heap-returning call argument
            // (one NOT already lifted by the `inline_struct_return` arms below
            // because it returns via a hidden work-ref / non-empty dep) when the
            // receiver is a borrowed CreateStack ref.  Must run before the
            // Insert/`inline_struct_return` handling so it is not skipped for the
            // exact shape that triggers the bug.
            if has_create_stack_receiver
                && arg_idx > 0
                && self
                    .inline_struct_return(&scanned, data, outer_call, function)
                    .is_none()
                && let Some(tp) = Self::heap_call_return(&scanned, data)
            {
                let tmp = self.new_lift_var(function, &tp);
                // loft#735 — this lift is an ORDERING device, never an ownership
                // transfer.  It fires exactly where `inline_struct_return` said "do
                // NOT lift into an owned temp": the callee delivers through the
                // caller's hidden work-ref (`__ref_N`), which the caller already
                // frees at function exit.  `heap_call_return` hands back the type
                // with `Deps::none()` (a caller temp cannot carry the callee's
                // DEF-space dep), so without this mark `get_free_vars` reads
                // `owns == true` and frees a store the caller still owns — the slot
                // is recycled under the live value and the next write lands in it.
                // `new_lift_var` already sets the allocate-time half of the fact
                // (`inline_ref` — borrow, don't allocate); this is the free-time
                // half.  The hand-correct source shape proves the target: binding
                // the same call to a named local yields `flat:vector<integer>
                // ["__ref_1"]` and NO `OpFreeRef`.
                function.set_skip_free(tmp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
                continue;
            }
            // A `Span` is source position, not structure. `parse_call` wraps a call
            // argument in one, so an argument this pass rewrote into a preamble-plus-value
            // sequence arrives as `Span(Insert(…))` and the bare-variant match below sees
            // nothing to split — which is how loft#1029's hoisted argument reached the
            // emitters still wrapped, with the lift that owns its result never firing.
            // Peel it for the Insert case only, so every other argument keeps its position.
            let scanned = match scanned {
                Value::Span(b) if matches!(b.1, Value::Insert(_)) => b.1,
                other => other,
            };
            // loft#1287 — a `&` argument whose binding this frame does NOT own.  The
            // callee's write-back releases the store the binding stopped naming, and it
            // cannot see whose that store is: for a plain heap PARAMETER it is the store the
            // CALLER handed down (`formal/calls.md` F-ParamHeap), owned a frame further up.
            // Freeing it there is a use-after-free plus a double free against the real
            // owner's own release.  The rebind witness names that store — the parameter's
            // ENTRY store, which is the only one this frame never owns, so a REPEATED call
            // still lets the callee release the fresh store the previous one installed.
            // `free_displaced` honours the mark; `(F-ParamRebind)`'s function-exit
            // `OpFreeRefIfDistinct(param, witness)` releases what the binding ends up naming.
            if outer_call != u32::MAX
                && let Value::Call(cs, cargs) = scanned.unspan()
                && *cs == create_stack_nr
                && let Some(Value::Var(v)) = cargs.first().map(Value::unspan)
                && let Some(orig) = function.rebind_orig(*v)
                && data
                    .def(outer_call)
                    .attributes()
                    .get(arg_idx)
                    .is_some_and(|at| at.typedef.is_amp_rebindable_heap())
            {
                amp_foreign.push(orig);
            }
            if let Value::Insert(ops) = scanned {
                // Existing A5.6 hoisting: lift Set(w, Null) for owned Reference.
                let is_a56_hoisted = Self::is_null_init_preamble(&ops, function);
                // hoist Set(__lift_N, ...) preamble from nested scan_args.
                // These are produced when an inner call's arguments contained
                // inline struct-returning calls that were already lifted.
                let n = ops.len();
                let is_p135_hoisted = n >= 2
                    && ops[..n - 1].iter().all(|v| {
                        matches!(v, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__lift_"))
                    });
                // hoist Set(__ref_N, expr) preamble produced by the
                // parser's `&T`-conversion path for non-Var sources.  The
                // final op is always OpCreateStack(Var(__ref_N)); after
                // hoisting it stays as the arg value, while the Set moves
                // into the enclosing statement list so the work-ref lives
                // at function scope (its slot must survive the call).
                //
                // loft#745 — a work-ref that ALREADY holds a value carries its
                // overwrite-free, so the parser's `Set` arrives wrapped as
                // `Insert([OpFreeRef(__ref_N), Set(__ref_N, …)])`.  Matching only the
                // bare `Set` left that shape unhoisted, so the materialisation stayed
                // INSIDE the argument: native then had no `OpCreateStack(Var(_))` to
                // recognise, hoisted the whole argument into a `let _pre_N = …`
                // binding whose value is the assignment's `()`, and rustc rejected the
                // call with E0308 (expected `&mut DbRef`, found `()`).
                let is_p179_hoisted = n >= 2
                    && ops[..n - 1]
                        .iter()
                        .all(|v| Self::is_ref_materialisation(v, function, data))
                    && matches!(&ops[n - 1], Value::Call(d_nr, _)
                        if data.def(*d_nr).name == "OpCreateStack");
                // @C118 — the slot a null collection local is given before a `&` links to
                // it (`Parser::null_local_slot`): guarded mints of the variable the final
                // `OpCreateStack` names.  They hoist like the materialisations above, so the
                // argument stays the bare `OpCreateStack(Var)` native passes as `&mut`.
                let is_slot_hoisted = n >= 2
                    && matches!(&ops[n - 1], Value::Call(d_nr, a)
                        if data.def(*d_nr).name == "OpCreateStack"
                            && matches!(a.as_slice(), [Value::Var(v)]
                                if ops[..n - 1].iter().all(|op| Self::is_null_slot_mint(op, *v, data))));
                if is_a56_hoisted || is_p135_hoisted || is_p179_hoisted || is_slot_hoisted {
                    // @PLN90 / loft#506 — a computed-lvalue `&`-WRITE-BACK arg.  Capture
                    // `items[i]` into a FRESH OWNED temp (so the callee's write-back frees the
                    // COPY, never the caller's element), pass the temp, then copy the result
                    // back into the element after the call.  The element's record is never
                    // freed — it is the stable backing.  A field-mutation callee is untouched.
                    if is_p179_hoisted
                        && let Some((pre, arg, post)) =
                            self.amp_writeback_owned_copy(&ops, arg_idx, outer_call, function, data)
                    {
                        preamble.extend(pre);
                        ls.push(arg);
                        postamble.push(post);
                        continue;
                    }
                    // loft#899 — hoisting the null-init out of the value block moves
                    // the temp's OWNER to the enclosing scope: its declaration now
                    // stands in that statement list, and an argument is only READ, so
                    // no binding adopts the store the way `v = <block>` does.  Nothing
                    // freed it, so every unbound `f#read(n) as vector<T>` leaked one
                    // store.  Re-register it at the current scope for `get_free_vars`,
                    // and run the same hand-off marking a lifted call-result gets so an
                    // argument the callee MOVES from does not drop twice.
                    let a56_owned = if is_a56_hoisted {
                        match &ops[0] {
                            Value::Set(v, _) => Some(*v),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let mut it = ops.into_iter();
                    for _ in 0..n - 1 {
                        preamble.push(it.next().unwrap());
                    }
                    if let Some(v) = a56_owned {
                        self.var_scope.insert(v, self.scope);
                        self.mark_lift_handoff(v, arg_idx, transfer_copy, moved_arg);
                    }
                    let final_val = it.next().unwrap();
                    // the remaining Call may also be struct-returning
                    // (e.g. normalize3(__lift_1) inside add_dir).  Lift it too.
                    if let Some(tp) =
                        self.inline_struct_return(&final_val, data, outer_call, function)
                    {
                        let tmp = self.new_lift_var(function, &tp);
                        self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                        preamble.push(self.lift_set(tmp, final_val, function, data));
                        ls.push(Value::Var(tmp));
                    } else {
                        ls.push(final_val);
                    }
                } else if let Some(tp) = ops
                    .last()
                    .and_then(|last| self.inline_struct_return(last, data, outer_call, function))
                {
                    // loft#1029 — an `Insert` whose TAIL is a heap-returning call, which is
                    // what the argument hoist below produces one level down: the ops that
                    // build the inner call's argument, then the call.  The lift that gives
                    // such a result an OWNER matches a `Call`, so the wrapper hid it and the
                    // callee's store was orphaned — `print("{pick(S { a: 7 }, false).a}")`
                    // leaked one record per evaluation on both backends while the same call
                    // BOUND to a local was clean.
                    //
                    // That is @P297's pitfall exactly one wrapper later ("the argument
                    // reaching here is `Span(Call(…))`; unspan before matching or the lift
                    // never fires"), so the cure is the same shape: read through to the
                    // value.  The preamble ops move into the enclosing statement list, where
                    // they already ran, and only the call is lifted — the three recognisers
                    // above split the same way for their own shapes.
                    let mut it = ops.into_iter();
                    let call = it.next_back().expect("checked non-empty by `last`");
                    preamble.extend(it);
                    let tmp = self.new_lift_var(function, &tp);
                    self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                    preamble.push(self.lift_set(tmp, call, function, data));
                    ls.push(Value::Var(tmp));
                } else {
                    ls.push(Value::Insert(ops));
                }
            } else if let Some(tp) = self.inline_struct_return(&scanned, data, outer_call, function)
            {
                // inline struct-returning or vector-returning call as argument
                // — lift to a temporary variable so get_free_vars emits
                // OpFreeRef at scope exit.  Without this, the callee's store
                // leaks every call.
                //
                // The argument becomes Set(tmp, call(...)) which the codegen
                // handles via gen_set_first_at_tos on first encounter and
                // generate_set (reassignment) on subsequent loop iterations.
                // get_free_vars emits OpFreeRef(tmp) at scope exit because
                // the dep is empty (owned).
                let tmp = self.new_lift_var(function, &tp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                preamble.push(self.lift_set(tmp, scanned, function, data));
                ls.push(Value::Var(tmp));
            } else if let Value::Call(g_nr, _) = scanned.unspan()
                // `@FR-N-Shape`: the shape is read through `?`; the lift keeps the whole type.
                && matches!(data.def(*g_nr).returned.base(), Type::Iterator(_, _))
                && data.def(*g_nr).is_loft_defined()
                && crate::use_analysis::inline_handle_needs_holder(data, outer_call)
            {
                // `@FR-G-Hold` — an inline generator handle is held by nothing but the
                // argument, so a `__lift_N` temp holds it and the scope's sweep releases its
                // hold, as the bound `g = gen(); first(g)` does; a callee that keeps it takes a
                // hold of its own (loft#1708).  Unlifted, an abandoned frame and every heap
                // local it allocated stayed to program exit, one per call (loft#1705).
                let gen_tp = data.def(*g_nr).returned.clone();
                let tmp = self.new_lift_var(function, &gen_tp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else if matches!(scanned.unspan(), Value::Tuple(_)) {
                // loft#1512 — a tuple-literal ARGUMENT binds no variable, so a member record
                // minted by the member's own call has no element-free site: the bound
                // spelling's release machinery (`Scopes::tuple_call_mint`) is keyed on a
                // `Set` this shape never makes, and the record leaked whole, hook included.
                // Lift each such member into a `__lift_N` exactly as the bare call argument
                // above is lifted — the temp owns the store and its scope-end cascade
                // releases it once — reducing the tuple to the local-member spelling that
                // already releases once.  Recursive, because a nested literal
                // (`deep(((mk(1), 2), 3))`) carries the same unowned mint one level down.
                // A member no lift can type (a local, a projection, a literal) is passed
                // as it was.
                let mut new_members = match scanned.unspan() {
                    Value::Tuple(members) => members.clone(),
                    _ => unreachable!("matched Value::Tuple above"),
                };
                let lifted = self.lift_tuple_call_members(
                    &mut new_members,
                    arg_idx,
                    transfer_copy,
                    moved_arg,
                    &mut preamble,
                    function,
                    data,
                    outer_call,
                );
                if lifted {
                    ls.push(Value::Tuple(new_members));
                } else {
                    ls.push(scanned);
                }
            } else if let Some((w, absorb)) =
                self.inline_built_borrow_source(&scanned, outer_call, data)
            {
                // loft#1029 — an argument BUILT INLINE (`pick(S { a: 7 }, …)`) that the
                // callee's return may BORROW.  The @P290 bracket that decides borrow-vs-
                // owned at runtime can only name a bare `Var` slot, so an argument still
                // wrapped in its construction block leaves the witness set incomplete
                // (`use_analysis::protectable_ref_args`) and the caller keeps the
                // conservative answer: it COPIES the returned store and orphans the one
                // the callee minted — one leaked record per call, both backends.
                //
                // The slot already exists and this frame already frees it: the parser
                // builds the literal into a function-scope work-ref and the block's tail
                // IS that work-ref.  So nothing needs a new owner — only the CALL SITE
                // needs to be able to say its name.  Hoisting the construction into the
                // preamble and passing `Var(w)` makes the argument nameable, which is
                // exactly the hand-written spelling that was always clean
                // (`q = S { a: 7 }; pick(q, …)`), and the emitted code becomes identical
                // to it.
                //
                // Deliberately NOT done by widening `protectable_ref_args` to see through
                // the block: `protect_store_frees` reads the DbRef VALUE at call time and
                // the bracket is emitted BEFORE the arguments are evaluated, so a work-ref
                // still holding its null would be "protected" while empty — the witness
                // set would read complete while protecting nothing, and the source-free it
                // then licenses would release a store the caller still reaches.  That
                // trades this leak for a use-after-free.
                let Value::Block(bl) = scanned else {
                    unreachable!("inline_built_borrow_source matched a non-Block")
                };
                // The block's own scope disappears with the block, so every var it
                // declared is now declared in the statement list we hoisted into. Move
                // their `var_scope` entries with them: an entry left pointing at a scope
                // no emitted code opens would have slot assignment place it against a
                // sibling scope's zone. Only the VECTOR-literal shape reaches this — a
                // struct literal's tail is the function-scope work-ref itself.
                if let Some(block_scope) = absorb {
                    for sc in self.var_scope.values_mut() {
                        if *sc == block_scope {
                            *sc = self.scope;
                        }
                    }
                }
                let mut ops = bl.operators;
                ops.pop();
                preamble.extend(ops);
                ls.push(Value::Var(w));
            } else if let Some(tp) =
                Self::unnameable_borrow_source(&scanned, outer_call, arg_idx, data)
            {
                // loft#1105 — an argument the @P290 bracket cannot NAME, at a call whose return
                // may borrow it.  The bracket protects a store through a variable holding a
                // `DbRef`, and `view_root_slots` walks a bare `Var`, a projection chain and a
                // JOIN to find one.  A `??` in argument position lowers to an `ncc` BLOCK whose
                // tail is a join with a CALL arm, and neither the multi-statement block nor the
                // call is nameable — so the witness set read incomplete and the caller copied
                // the returned store, orphaning the one the callee minted.
                //
                // Binding it to a temp is the same cure the cases above take, generalised to the
                // question itself: if the bracket cannot name the value, give it a name.  The
                // preamble runs BEFORE the bracket is emitted, so the temp holds the real
                // `DbRef` by then — which is why the hand-written `e = v[0] ?? mk(); pick(e, …)`
                // was always clean and this now emits the same thing.  And because the name is
                // taken at RUNTIME, the bracket protects whichever store the value turned out to
                // be, which is what makes one temp serve a join whose arms disagree about it.
                //
                // ⚠ LAST in the chain, and that is load-bearing rather than tidy.  The temp
                // takes the CALLEE'S PARAMETER type — the one type available for a value with no
                // variable behind it — and a parameter declaration carries NO DEPS, so the temp
                // reads as an OWNER of whatever it holds.  For every shape the arms above claim
                // that is wrong: a tuple element and a projection chain are VIEWS of a store the
                // caller owns, and an owner's scope-exit free would release a record the caller
                // still reaches.  Ordered after them, this arm only ever sees values no earlier
                // arm could type.
                //
                // …and the temp BORROWS what the value borrows, which is the type
                // `unnameable_borrow_source` answers: the callee's parameter SHAPE carrying
                // `lift_view_deps`'s answer for the argument.  A `skip_free` temp would also
                // stop the over-free, and it says less — "do not free me" rather than "whose
                // store is this", which is the question `Type::depend`'s other readers ask.
                // Where the walk can name no source the argument is NOT bound at all, so a
                // value with no provenance costs the leak it already had rather than a name
                // that cannot say why it is safe.
                let tmp = self.new_lift_var(function, &tp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else {
                ls.push(scanned);
            }
        }
        for (i, orig) in amp_foreign.iter().enumerate() {
            preamble.insert(
                i,
                Value::Call(
                    data.def_nr("n_protect_store_frees"),
                    vec![Value::Var(*orig)],
                ),
            );
            postamble.push(Value::Call(
                data.def_nr("n_unprotect_store_frees"),
                vec![Value::Var(*orig)],
            ));
        }
        (preamble, ls, postamble)
    }

    /// loft#1105 — the TYPE to bind an argument to when the @P290 bracket cannot NAME the store
    /// its value will lie in, at a call whose return may borrow it.
    ///
    /// `Some(tp)` is the callee's PARAMETER shape — the type the argument is converted to
    /// regardless — carrying the DEPS the value itself borrows ([`lift_view_deps`]).
    ///
    /// The deps are the load-bearing half.  The parameter's declared type has none, and a
    /// temp typed that way reads as the OWNER of a store it only VIEWS: `get_free_vars`
    /// emits a scope-exit free that releases the caller's container.  It is silent while the
    /// container is a local of the same frame — the store was dying at that scope exit
    /// anyway — and a use-after-free the moment the container OUTLIVES the call, which is
    /// why `pick(h[k], …)` corrupted a `hash` passed in as a parameter.
    ///
    /// So a value whose source cannot be named is NOT bound (`lift_view_deps` answers
    /// `None`), and the argument stays exactly as it was — the leak that is already there,
    /// which is the better of the two.
    ///
    /// Gated as its two siblings are, plus one exclusion of its own: a bare `Var` is already
    /// nameable and must not be re-bound, and an argument the bracket CAN name needs nothing.
    /// The inline-construction and tuple-element cases are tried first and handle their shapes
    /// more precisely — a construction is HOISTED rather than bound, because binding a work-ref
    /// that still holds null at bracket-emit time would read as covered while protecting
    /// nothing (loft#981).
    fn unnameable_borrow_source(
        arg: &Value,
        outer_call: u32,
        arg_idx: usize,
        data: &Data,
    ) -> Option<Type> {
        if outer_call == u32::MAX {
            return None;
        }
        let callee = data.def(outer_call);
        if !callee.is_loft_defined() {
            return None;
        }
        if matches!(callee.returned().base(), Type::Function(..)) {
            return None;
        }
        if !callee.returns_borrowed_view() {
            return None;
        }
        if matches!(arg.unspan(), Value::Var(_)) {
            return None;
        }
        if crate::use_analysis::bracket_can_name(data, arg) {
            return None;
        }
        let tp = callee.attributes().get(arg_idx)?.typedef.clone();
        // Only an argument that CARRIES a store needs a witness at all — asked through
        // `base`, because a NULLABLE parameter (`s: S?`) is `Optional(Reference(S))` and
        // carries exactly the store its non-null twin does.  Asked on the raw type this
        // declined every nullable parameter, so a `??` argument at one was never lifted and
        // kept leaking the callee's minted store while the dense twin was cured.
        if !crate::data::is_dbref(tp.base()) {
            return None;
        }
        Some(tp.with_deps(&Deps::frame(lift_view_deps(arg, data)?)))
    }

    /// loft#1029 — the work-ref an INLINE-built argument yields, when the callee's return
    /// may borrow it.
    ///
    /// `Some(w)` for a value block that fills a work-ref and ends in it — the shape the
    /// parser gives `S { … }` / a collection literal in argument position — at a call whose
    /// return names a visible parameter (`returns_borrowed_view`). `w` is a function-scope
    /// slot this frame already allocates and frees, so hoisting the block moves nothing's
    /// ownership; it only lets the call site NAME the borrow source.
    ///
    /// Gated on `returns_borrowed_view` on purpose. Every other call is already correct as
    /// it stands, and hoisting an argument reorders it relative to its left-hand siblings —
    /// a cost worth paying only where the alternative is a leak.
    fn inline_built_borrow_source(
        &self,
        arg: &Value,
        outer_call: u32,
        data: &Data,
    ) -> Option<(u16, Option<u16>)> {
        // `scan_args` runs for argument lists with no enclosing DEF as well (the
        // `u32::MAX` no-call sentinel), and `Data::def` asserts on it. Nothing to decide
        // there: with no callee there is no return that could borrow the argument.
        if outer_call == u32::MAX {
            return None;
        }
        let callee = data.def(outer_call);
        // Only a call into a LOFT-DEFINED body, which is what the @P290 copy-or-adopt
        // bracket serves. A native accessor answers a borrowed view too (`OpGetText` is
        // `text[v1]`), but it never goes through that machinery, so hoisting its argument
        // buys nothing — and it is reached in EXPRESSION position, where the preamble is
        // not a statement list: the hoisted ops were rendered into the argument parens and
        // `--native` rejected the call with E0277 (`((), (), (), (), &str): AsRef<str>` in
        // 875-json-absent-text-field).
        if !callee.is_loft_defined() {
            return None;
        }
        // A callee returning a CLOSURE is not a heap return, and `returns_borrowed_view`
        // is documented as a heap-return ownership read: a `Type::Function` return carries
        // `CALLEE_FRAME`-tagged deps (a closure-internal frame var, never an attr index)
        // and its own debug assert says such a dep must not reach it.  This bracket serves
        // heap returns alone — the shape test below names them — so the ownership question
        // is asked only of a callee that has one.  Without the gate, `fn make_adder(b) ->
        // fn(integer) -> integer` tripped that assert before any of its own work ran.
        if matches!(callee.returned().base(), Type::Function(..)) {
            return None;
        }
        if !callee.returns_borrowed_view() {
            return None;
        }
        let Value::Block(bl) = arg else {
            return None;
        };
        // At least one construction op plus the trailing `Var` — a bare `{ v }` has
        // nothing to hoist and is already a nameable value.
        if bl.operators.len() < 2
            || !matches!(
                bl.result.base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
        {
            return None;
        }
        let Value::Var(w) = bl.operators.last()?.unspan() else {
            return None;
        };
        // The block must MINT ITS STORE INTO A SLOT THAT OUTLIVES THE HOIST. Its result dep
        // names that slot — the owner — and the dep the block already carries is the fact
        // itself, so nothing here keeps a second list of it. (Not `is_work_ref`: that set
        // is the return-delivery materialiser's own register and does not contain the
        // parser's object work-ref — measured, it answers false for the `__ref_1` this very
        // shape builds into.)
        let [owner] = bl.result.depend()[..] else {
            return None;
        };
        // The owner's declaration must ENCLOSE the statement list we are hoisting into, so
        // the moved ops land inside its lifetime and nothing changes about who frees the
        // store. `self.stack` is the chain of scopes currently open, so this asks the real
        // question — a numeric `<=` would not, because scope numbers are allocated in
        // encounter order and an earlier SIBLING also compares less while enclosing nothing.
        if !self.scope_encloses(owner) {
            return None;
        }
        // The tail names either that same owner — `S { … }`, whose block fills `__ref_N`
        // and yields it — or a VIEW the block opened at its own scope: a vector literal
        // fills `__vdb_N` one level up and yields `_vec_N`, a view of it. The second is
        // still ownership-neutral, because the store's owner is `__vdb_N` and that is not
        // moving; what moves is the view's DECLARATION, out of a block that ceases to
        // exist. So the block's scope has to be absorbed into the one we hoist into, or
        // `var_scope` would keep pointing those vars at a scope no emitted code opens and
        // slot assignment would place them against a sibling's zone.
        if self.scope_encloses(*w) {
            Some((*w, None))
        } else if self.var_scope.get(w) == Some(&bl.scope) {
            Some((*w, Some(bl.scope)))
        } else {
            None
        }
    }

    /// Is `v` declared in a scope that is still OPEN — the one being scanned or one
    /// enclosing it?  That is the condition for moving code into the current statement
    /// list and still being inside `v`'s lifetime.
    fn scope_encloses(&self, v: u16) -> bool {
        self.var_scope
            .get(&v)
            .is_some_and(|sc| *sc == self.scope || self.stack.contains(sc))
    }

    /// The A5.6 hoistable preamble: `Insert([Set(v, Null), value])` whose `v` OWNS a
    /// heap store.  It is the shape the `Value::Block` arm returns for a value block
    /// that yields an owned temp — a `#reading file` read, a `??` join — and
    /// [`Self::scan_args`] lifts that `Set` into the enclosing statement list so the
    /// slot's `first_def` lives OUTSIDE the argument expression.
    ///
    /// One home for the question, because two places ask it: `scan_args`, which does
    /// the hoisting, and the `Value::Span` arm of [`Self::scan`], which must drop a
    /// span that would otherwise hide this Insert from `scan_args`' `if let
    /// Value::Insert`.  While only `scan_args` knew the shape, a span-wrapped one
    /// stayed inside the argument: `println("{len(f#read(8) as vector<single>)}")`
    /// left the temp's declaration in an expression slot, which native emitted
    /// literally as a `let` statement inside an argument list — rustc "expected
    /// expression, found `let` statement" — and which nothing then freed (loft#899).
    pub(super) fn is_null_init_preamble(ops: &[Value], function: &Function) -> bool {
        ops.len() == 2
            && matches!(&ops[0], Value::Set(v, val)
                if matches!(val.as_ref(), Value::Null) && function.tp(*v).is_heap_owned())
    }

    /// True when `v` writes the work-ref that a `&`-argument's `OpCreateStack` then
    /// borrows — the `Set(__ref_N, …)` the parser emits for a non-`Var` `&`-source,
    /// either bare or wrapped with the overwrite-`OpFreeRef` a re-assigned work-ref
    /// carries (loft#745).  `scan_args` hoists these out of the argument so the
    /// work-ref lives at function scope and its slot survives the call.
    /// The guarded mint `Parser::null_local_slot` emits for `var`:
    /// `if OpRefIsNull(var) { … } else null`.
    fn is_null_slot_mint(op: &Value, var: u16, data: &Data) -> bool {
        matches!(op.unspan(), Value::If(test, _, _)
            if matches!(test.unspan(), Value::Call(d_nr, a)
                if data.def(*d_nr).name == "OpRefIsNull"
                    && matches!(a.as_slice(), [Value::Var(t)] if *t == var)))
    }

    fn is_ref_materialisation(v: &Value, function: &Function, data: &Data) -> bool {
        let writes_work_ref = |op: &Value| matches!(op, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__ref_"));
        match v {
            Value::Set(_, _) => writes_work_ref(v),
            // The free targets the work-ref's PREVIOUS value and belongs with the write,
            // so the pair hoists as one unit.  A wrapper holding anything else is not a
            // materialisation and stays inside the argument.
            Value::Insert(inner) => {
                inner.iter().any(writes_work_ref)
                    && inner.iter().all(|op| {
                        writes_work_ref(op)
                            || matches!(op, Value::Call(d_nr, _)
                                if data.def(*d_nr).name == "OpFreeRef")
                    })
            }
            _ => false,
        }
    }

    /// @PLN90 / loft#506 — a computed-lvalue `&`-WRITE-BACK argument.  The arg is
    /// `Insert([Set(wv, orig), OpCreateStack(wv)])` where `orig` reads a heap lvalue
    /// (`OpGetVector`/`OpGetField`).  When the callee WHOLE-REASSIGNS this `&`-param (a
    /// `Set(param, non-Null)` through the ref — a write-back), the reassignment FREES the
    /// displaced record; if the arg aliased the caller's element that would free the
    /// element (corruption).  So capture `orig` into a FRESH OWNED temp: the callee frees
    /// the COPY, the element's record survives, and after the call the temp's new record is
    /// copied back into the element.  Returns `(preamble, arg, postamble)` — the owned-copy
    /// setup, the `OpCreateStack(tmp)` argument, and the copy-back — or `None` (field
    /// mutation / not a computed lvalue → keep the default aliasing lowering).
    fn amp_writeback_owned_copy(
        &mut self,
        ops: &[Value],
        arg_idx: usize,
        outer_call: u32,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Value, Value)> {
        if outer_call == u32::MAX || ops.len() != 2 {
            return None;
        }
        let Value::Set(wv, orig) = &ops[0] else {
            return None;
        };
        let wv = *wv;
        let orig = orig.unspan().clone();
        if !matches!(&orig, Value::Call(g, _)
            if matches!(data.def(*g).name(), "OpGetVector" | "OpVectorRef" | "OpGetField"))
        {
            return None;
        }
        // Does the callee whole-reassign the `&`-param at this position? (`Set(param, non-Null)`
        // through the ref — NOT `rebind_orig`, which is non-`&` reassignment-locality.)
        let attrs = data.def(outer_call).attributes();
        if arg_idx >= attrs.len() {
            return None;
        }
        let callee_vars = &data.definitions[outer_call as usize].variables;
        let param_var = callee_vars.var(&attrs[arg_idx].name);
        if param_var == u16::MAX || !matches!(callee_vars.tp(param_var), Type::RefVar(_)) {
            return None;
        }
        let mut writes_back = false;
        data.definitions[outer_call as usize]
            .code
            .walk(&mut |node| {
                if let Value::Set(v, rhs) = node
                    && *v == param_var
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    writes_back = true;
                }
            });
        if !writes_back {
            return None;
        }
        let struct_d = match function.tp(wv) {
            Type::RefVar(inner) => match &**inner {
                Type::Reference(d, _) => *d,
                _ => return None,
            },
            Type::Reference(d, _) => *d,
            _ => return None,
        };
        let type_val = Value::Int(i32::from(data.def(struct_d).known_type()));
        let inner_tp = Type::Reference(struct_d, Deps::none());
        // A fresh OWNED temp via `new_lift_var` — registers the slot + scope-exit `OpFreeRef`
        // (var_scope / var_order / lift_vars) and an inline-ref entry init (no alloc); the
        // `OpDatabase` below allocates its store, freed at scope exit.
        let tmp = self.new_lift_var(function, &inner_tp);
        let db = data.def_nr("OpDatabase");
        let cp = data.def_nr("OpCopyRecord");
        let cs = data.def_nr("OpCreateStack");
        let preamble = vec![
            Value::Call(db, vec![Value::Var(tmp), type_val.clone()]),
            Value::Call(cp, vec![orig.clone(), Value::Var(tmp), type_val.clone()]),
        ];
        let arg = Value::Call(cs, vec![Value::Var(tmp)]);
        let postamble = Value::Call(cp, vec![Value::Var(tmp), orig, type_val]);
        Some((preamble, arg, postamble))
    }

    /// #248 — does this scanned argument lower to an inline call (or an
    /// `Insert`/`Span` whose final op is one) that PRODUCES a heap value
    /// (vector / reference / struct-enum)?  Used by `scan_args` to force-lift a
    /// trailing heap-returning call argument when the call's receiver is a
    /// borrowed `OpCreateStack` ref — the interpreter arg-layout hazard #248.
    ///
    /// Unlike [`inline_struct_return`], this DOES match calls that return via a
    /// hidden caller work-ref (non-empty dep like `["??"]`): those are exactly
    /// the frame-growing inline calls that `inline_struct_return` skips but that
    /// still shift the receiver's stack slot.  Returns the owned element/struct
    /// type (empty dep) for the `__lift_N` temp, or `None`.
    fn heap_call_return(val: &Value, data: &Data) -> Option<Type> {
        // Peel `Span` / trailing-op-of-`Insert` to reach the producing call.
        let inner = match val.unspan() {
            Value::Insert(ops) => ops.last()?.unspan(),
            other => other,
        };
        let Value::Call(fn_nr, _) = inner else {
            return None;
        };
        let def = data.def(*fn_nr);
        // Only user/method bodies (n_* / t_*) — native helpers and the
        // OpCreateStack/OpVar* lowering ops never own a fresh return store here.
        if !def.is_loft_defined() {
            return None;
        }
        match &def.returned {
            Type::Vector(elem, _) => Some(Type::Vector(elem.clone(), Deps::none())),
            Type::Reference(d_nr, _) => Some(Type::Reference(*d_nr, Deps::none())),
            Type::Enum(d_nr, true, _) => Some(Type::Enum(*d_nr, true, Deps::none())),
            _ => None,
        }
    }
}
