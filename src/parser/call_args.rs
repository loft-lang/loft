// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! What a call hands its callee and takes back: each argument converted and validated
//! against its parameter, the defaults filled in, and the dependencies a result carries —
//! which caller frame a returned view borrows from, what a fn-ref value publishes.

use super::{
    ConstHandOff, Deps, HashMap, HashSet, Level, Parser, Position, Type, Value,
    callee_param_reassigns, callee_param_rebinds_owned, diagnostic_format, v_block, v_set,
};

impl Parser {
    /// Convert and validate each positional argument for a call.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn process_call_args(
        &mut self,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        all_types: &mut [Type],
        report: bool,
        arg_pos: &[Position],
    ) -> Vec<Value> {
        let mut actual = Vec::new();
        // loft#2146 — an overload trial converts arguments too; only the call built from THIS
        // conversion takes its store-backs.
        self.pending_arg_pres.clear();
        self.pending_arg_posts.clear();
        if types.is_empty() {
            return actual;
        }
        // `@FR-F-Arity` — a compiler-inserted slot (a record's `__retbuf`, a text return's
        // buffer) is not a user parameter: an argument past the declared ones is too many,
        // never a fill of the hidden buffer.  Counted as a parameter, `txt("a", "b", "c")`
        // handed `"c"` to `txt`'s text buffer and answered `ab` in silence, and a record
        // return's extra argument was refused as a type mismatch on the buffer.
        // A compiler-built call (an enum-variant dispatcher forwarding its own buffer) does
        // fill the hidden slot, and with a buffer variable: that is the one extra argument a
        // hidden slot takes.
        let attrs = self.data.def(d_nr).attributes();
        let overfilled = list.len() > attrs.len()
            || list.iter().enumerate().any(|(i, arg)| {
                attrs[i].hidden
                    && !matches!(arg.unspan(), Value::Var(v)
                        if *v < self.vars.count() && self.vars.name(*v).starts_with("__"))
            });
        if overfilled {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Too many parameters for {}",
                    self.data.user_facing_name(d_nr)
                );
            }
            return actual;
        }
        // @PLN102 gate-2 (N-Store) at the CALL-ARG site — constant across the args: the callee
        // name (for the diagnostic) and whether the call-arg check applies. Null-transparent fns
        // (`min`/`max`/`clamp`/`abs`/… — `is_null_transparent`) PROPAGATE null via a runtime guard
        // (`wrap_null_transparent`), so a nullable arg into their non-null param is intentional, not
        // an unsound store — exempt them (operators already dodge this path via the nullable-op swap).
        // A specialisation of an overload set (`<name>__dyn_<spelling>`, `<name>__sel_…`,
        // @PLN162) stands for the call the author wrote: it is named, and exempted, as the name
        // it dispatches — never by the compiler's own spelling of it.
        let original = self.data.def(d_nr).original_name().clone();
        let callee_name = if self.data.def(d_nr).synthetic == Some("dynamic_dispatcher") {
            original
                .split_once("__dyn_")
                .or_else(|| original.split_once("__sel_"))
                .map_or_else(|| original.clone(), |(set, _)| set.to_string())
        } else {
            original
        };
        let callarg_nstore =
            crate::keys::callarg_nstore_enabled() && !Self::is_null_transparent(&callee_name);
        for (nr, a_code) in list.iter().enumerate() {
            let tp = self.data.attr_type(d_nr, nr);
            let Some(actual_type) = types.get(nr) else {
                continue;
            };
            let mut actual_code = a_code.clone();
            if let (Type::Vector(to_tp, _), Type::Vector(a_tp, _)) = (&tp, actual_type)
                && a_tp.is_unknown()
                && !to_tp.is_unknown()
            {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            // empty `[]` literal → create temp vector where parameter type is known.
            if matches!(&actual_code, Value::Insert(ops) if ops.len() <= 1)
                && let Type::Vector(elm_tp, dep) = &tp
            {
                let vec = self.create_unique("vec", &Type::Vector(elm_tp.clone(), dep.clone()));
                let mut ls = self.vector_db(elm_tp, vec);
                ls.push(Value::Var(vec));
                actual.push(v_block(ls, tp.clone(), "empty_vector_arg"));
                all_types[nr] = tp.clone();
                continue;
            }
            // … and at a KEYED parameter, the empty collection its concrete spelling builds
            // (`f([])` at a `hash<K[k]>` mints a `__kvb_N` store, loft#703).  A generic's
            // parameter learns it is keyed only here (`fold(v, [], add)` binds `U` from
            // `add`), and without this arm the `[]` reached the callee as a bare `null`: the
            // interpreter looped in the instance and `--native` did not compile (E0308), where
            // a typed empty local passed in its place was right.
            if matches!(&actual_code, Value::Insert(ops) if ops.len() <= 1)
                && crate::parser::vectors::is_keyed(&tp)
                && let Some(kt) = self.keyed_known_type(&tp)
            {
                let kvb = self.vars.work_keyed(&tp.without_deps(), &mut self.lexer);
                let ls = vec![
                    v_set(kvb, Value::Null),
                    self.cl("OpDatabase", &[Value::Var(kvb), Value::Int(i32::from(kt))]),
                    Value::Var(kvb),
                ];
                actual.push(v_block(ls, tp.clone(), "empty_vector_arg"));
                all_types[nr] = tp.clone();
                continue;
            }
            // L4: reject non-variable expressions passed to `&` parameters (except &text
            // which has its own work-text copy handling in convert()).  The `&` modifier
            // means "mutations propagate back to the caller" — passing a literal means
            // the mutations are silently discarded, which is almost certainly a bug.
            // also accept "addressable" expressions — vector element access
            // (`v[i]`), field access (`s.field`), and chains thereof — since these
            // produce a DbRef into existing mutable storage.
            // @FR-B-Ref-Lvalue — a SCALAR field or element is a place too, and a `&` parameter
            // links to it through the lowering the `&` bind uses (`scalar_place_ref`, in
            // `convert`) — except a narrow integer store place, which no link can honour yet
            // (D-bind-39), refused here in the bind's own words.
            let scalar_place = matches!(tp.base(), Type::RefVar(inner) if crate::data::is_scalar(inner))
                && !matches!(actual_code.unspan(), Value::Var(_))
                && Self::is_amp_place(&actual_code, &self.data);
            // loft#1602 — a `&text` parameter handed a text FIELD or ELEMENT.  A text place is a
            // string record in a store, and a `&text` parameter today reaches only a text
            // variable, so `convert` would hand the callee a work copy and drop its write.
            // `(B-Ref-Reshape)`: refuse a link that cannot be honoured rather than downgrade it
            // to a copy.  A temporary (a literal, a computed text) keeps its work copy: nothing
            // names it, so nothing can miss the write.
            // @PLN167 C3 — the parameter links that place: the argument is the slot's `DbRef`
            // (the place a `&` bind of it takes), and after pass 2 the call is pointed at the
            // callee's STORE instance, whose parameter reads and writes through the field ops
            // (`store_text.rs`).  The stack instance is the function as written.
            if matches!(tp.base(), Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)))
                && self.is_text_place(&actual_code)
                && let Some(place) = self.scalar_place_ref(&actual_code)
            {
                self.store_text_args = true;
                actual.push(place);
                all_types[nr] = tp.clone();
                continue;
            }
            // A tuple local's TEXT member is a place as well, and no `&text` link reaches it
            // yet: refused rather than handed a work copy that would drop the callee's write
            // (`(B-Ref-Reshape)`, as the `&` bind of the same member refuses).
            if matches!(tp.base(), Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)))
                && let Value::TupleGet(t, i) = actual_code.unspan()
                && let Err(elem) = self.linkable_tuple_member(*t, *i)
            {
                let i = *i;
                self.refuse_tuple_member_link(&elem, i);
            }
            if let Type::RefVar(inner) = &tp
                && !matches!(inner.as_ref(), Type::Text(_))
                && !matches!(&actual_code, Value::Var(_))
                && !scalar_place
                && !Self::is_addressable(&actual_code, &self.data)
                && !Self::stored_tuple_dest(&actual_code)
                    .is_some_and(|d| d.is_place_read(&self.data))
            {
                // Defer on pass 1 (#375): a field access on a struct whose
                // layout is not yet finalised — because one of its fields is a
                // forward / cross-package reference still resolving — lowers to
                // `Null` here, which is not addressable.  Erroring now would
                // abort pass 1 before the type resolves; on pass 2 the layout is
                // complete and the access lowers to `OpGetField` (addressable),
                // so the check passes.  A genuine literal-to-`&` is still an
                // error: it is non-addressable on pass 2 too, where this fires.
                let tuple_param = match inner.base() {
                    Type::Tuple(_) => true,
                    Type::Reference(t, _) => self.data.def(*t).name().starts_with("__tuple<"),
                    _ => false,
                };
                if self.first_pass {
                } else if tuple_param {
                    self.refuse_tuple_value_link();
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot pass a literal or expression to a '&' parameter — \
                         assign to a variable first"
                    );
                }
                actual.push(actual_code);
                continue;
            }
            if actual_type.is_unknown() && matches!(&tp, Type::Vector(_, _)) {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            if let (Type::Integer(_), Type::Enum(_, true, _)) = (&tp, actual_type) {
                let cd = if matches!(actual_code, Value::Enum(_, _)) {
                    actual_code
                } else {
                    self.cl("OpGetEnum", &[actual_code, Value::Int(0)])
                };
                actual.push(self.cl("OpConvIntFromEnum", &[cd]));
                continue;
            }
            // @PLN102 gate-2 (N-Store) at the CALL-ARG site — the last store site the teeth did
            // not cover (converges with the earlier routing-feedback f4 fix). `convert` below
            // leniently peels an `Optional`, so a nullable `τ?` (or a bare `null` under DN1) bound
            // silently into a non-null PARAMETER. Run the same `n_store_violation` check here
            // (identical Phase-1 warn/error split) so the param binding is held to the same rule as
            // an assignment / field / return. On a hard error (a narrow-width param) skip
            // `convert`'s generic diagnostic; otherwise fall through and let `convert` peel it.
            // The position anchors to the argument's own span (nstore-position-fix.md) so a TAIL
            // call reports at the call, not the next line.
            // loft#1287 — a plain heap PARAMETER handed to a `&` parameter the callee
            // REBINDS.  The write-back installs a store the callee minted and displaces the
            // one this binding named, so BOTH ownership questions land here: a plain heap
            // parameter aliases the caller's argument (`formal/calls.md` F-ParamHeap), so the
            // displaced store belongs to a frame above and is not the callee's to free, while
            // the fresh one has no owner at all until this binding takes it — which is what
            // `(F-ParamRebind)` means by *the rebind is LOCAL*.
            //
            // The rebind witness answers both.  It snapshots the parameter's ENTRY store, so
            // the call-site bracket (`scopes::scan_args`) can mark exactly the store this
            // frame does not own, and the function-exit `OpFreeRefIfDistinct(param, witness)`
            // releases the rebound one.  It is the same P2.1 machinery a rebind written in
            // THIS body already gets; only the spelling of the rebind is different.
            let mut amp_rebind_arg = u16::MAX;
            if !self.first_pass
                && tp.is_amp_rebindable_heap()
                && let Value::Var(v) = actual_code.unspan()
                && self.vars.is_argument(*v)
                && !matches!(self.vars.tp(*v).base(), Type::RefVar(_))
                && !self.vars.is_compiler_generated(*v)
                && !self.is_hidden_param(*v)
            {
                let v = *v;
                let mut cache: HashMap<u32, Vec<bool>> = HashMap::new();
                if callee_param_rebinds_owned(d_nr, &self.data, &mut cache)
                    .get(nr)
                    .copied()
                    .unwrap_or(false)
                {
                    amp_rebind_arg = v;
                }
            }
            if amp_rebind_arg != u16::MAX {
                self.ensure_rebind_witness(amp_rebind_arg);
            }
            // loft#1816 — the same rebind as a WRITE: the callee's write-back lands in this
            // parameter, so it is tracked like `p = …` written here, and a parameter nothing
            // reads after the call is the dead store its body-written spelling is (F-ParamRebind:
            // the caller two frames up never sees it).  Asked of a callee that reassigns that
            // parameter at all; one that only reads through its `&` writes nothing back.  A callee
            // with no body (a native operator) is not asked: it has no assignment to find.
            if !self.first_pass
                && matches!(tp.base(), Type::RefVar(_))
                && let Value::Var(v) = actual_code.unspan()
                && self.vars.is_argument(*v)
                && !matches!(self.vars.tp(*v).base(), Type::RefVar(_))
                && !self.vars.is_compiler_generated(*v)
                && !self.is_hidden_param(*v)
                && *self.data.def(d_nr).code() != Value::Null
            {
                let v = *v;
                let mut cache: HashMap<u32, Vec<bool>> = HashMap::new();
                if callee_param_reassigns(d_nr, &self.data, &mut cache)
                    .get(nr)
                    .copied()
                    .unwrap_or(false)
                {
                    self.vars.track_write(v, &mut self.lexer);
                }
            }
            // loft#1540, C124 — a value-const value reaches a `&` parameter never and a plain
            // record or collection parameter only when it is `const`; the question and its
            // wording live in `report_const_argument`, shared with a function reference's call
            // and a builtin's callback.
            // An OP is exempt: it is a primitive only the standard library's own bodies call,
            // where `const` on a parameter means an immediate operand in the bytecode
            // (`Data::add_op`), not a read-only borrow; the stdlib's `pub fn` wrapper around it
            // carries the signature a program is judged by.
            let callee_is_op = self.data.def(d_nr).is_operator();
            if report && !self.first_pass && !callee_is_op {
                let attr = self.data.def(d_nr).attributes().get(nr);
                let param_const = attr.is_some_and(|a| a.value_const);
                let param = attr.map(|a| a.name.clone());
                self.report_const_argument(
                    &actual_code,
                    &tp,
                    param_const,
                    ConstHandOff::Declared {
                        callee: &callee_name,
                        nr,
                        param: param.as_deref(),
                    },
                );
            }
            // @FR-N-Store — the parameter is a slot when this binding is REPORTED and the callee
            // is not null-transparent; an overload TRIAL (`!report`) and a null-transparent
            // callee (`abs(x)` propagates the null through a runtime guard) only test the fit.
            // @PLN204 — a 64-bit caller's argument for a 32-bit library's parameter.
            let narrowed = self.width32_call_argument(&mut actual_code, actual_type, &tp, d_nr);
            let actual_type = narrowed.as_ref().unwrap_or(actual_type);
            let arg_at = actual_code.span_pos().copied();
            // A scalar PLACE is linked, not stored, so @FR-N-Store's "a nullable value becomes
            // null there" does not describe it, and its cure (`?`) would turn the place into a
            // value.  What can go wrong is that the element is absent: a link to an absent place
            // reads null (@C80), so a callee whose `&τ` is non-null would see one.
            let linked_place = scalar_place && report && !self.first_pass;
            if linked_place
                && matches!(actual_type, Type::Optional(_))
                && matches!(tp.base(), Type::RefVar(pointee) if !matches!(pointee.as_ref(), Type::Optional(_)))
            {
                let msg = diagnostic_format(
                    Level::Warning,
                    format_args!(
                        "the element handed to parameter {} of `{callee_name}` may not exist, and \
                         the parameter is the non-null `{}` — a link to an absent element reads \
                         null; guard the index (`if i < len(v)`) or declare the parameter `{}?`",
                        nr + 1,
                        tp.source_name(&self.data),
                        tp.source_name(&self.data),
                    ),
                );
                self.nstore_diag(arg_at.as_ref(), Level::Warning, &msg);
            }
            let accepted = if report && callarg_nstore && !linked_place {
                self.convert_store(
                    &mut actual_code,
                    actual_type,
                    &tp,
                    &format!("parameter {} of `{callee_name}`", nr + 1),
                    arg_at.as_ref(),
                )
            } else {
                self.convert_admitting(&mut actual_code, actual_type, &tp)
            };
            // loft#1729, @FR-R-Const — a top-level constant's use site is a VIEW of the
            // write-locked constant store, and a callee that writes its by-value parameter
            // writes through it: `(R-Const)` makes that hand-off B-Copy's copy.
            if accepted && self.constant_arg_needs_copy(d_nr, nr, &actual_code, &tp) {
                let param = tp.base().without_deps();
                let tmp = self.materialize_collection_value(&mut actual_code, &param);
                // A VALUE, typed by the temp's own deps — the buffer the copy lives in — the
                // way a vector literal argument is a `Vector` block naming its buffer.  A bare
                // `Insert` gave the scope pass no owner for that buffer inside an `assert`'s
                // lazily built message, and it placed the buffer's free before its allocation.
                if tmp != u16::MAX
                    && let Value::Insert(ops) = &mut actual_code
                {
                    let ops = std::mem::take(ops);
                    actual_code = v_block(ops, self.vars.tp(tmp).clone(), "Vector");
                }
            }
            if !accepted {
                if report {
                    let context = format!(
                        "argument {} of call to {}",
                        nr + 1,
                        self.data.def(d_nr).original_name()
                    );
                    // `arg_pos[nr]` is the argument's start, captured in
                    // `parse_call`; the lexer cursor has drifted to `)` / `,`.
                    // Synthetic / reordered call paths pass an empty slice and
                    // fall back to the cursor.
                    let pos = arg_pos
                        .get(nr)
                        .copied()
                        .unwrap_or_else(|| *self.lexer.pos());
                    // loft#1008 — a `both`/`self` METHOD is registered only as
                    // `t_<len><Type>_<name>`, so its bare name has no value to bind. In a
                    // fn-ref ARGUMENT position it survives as an untyped placeholder and this
                    // site reported *"expected fn(P) -> integer, got null"* — a value the
                    // author wrote nowhere. Name what the argument is instead; the unknown-name
                    // sites do the same for the spellings that reach them.
                    //
                    // Reaches the `self` spelling, which arrives as a placeholder VARIABLE. A
                    // `both` one arrives as a bare `Value::Null` with no name attached, so it
                    // cannot be named HERE — it is reported at the bare-name site in
                    // `objects.rs` instead, where the name is still in hand, and both
                    // receivers now give the same message.
                    let method_arg = if matches!(tp, Type::Function(..))
                        && matches!(actual_type, Type::Null | Type::Unknown(_))
                        && let Value::Var(v) = actual_code.unspan()
                    {
                        let nm = self.vars.name(*v).to_string();
                        let recv = self.method_receivers_named(&nm);
                        if recv.is_empty() {
                            None
                        } else {
                            Some((nm, recv))
                        }
                    } else {
                        None
                    };
                    if let Some((nm, recv)) = method_arg {
                        let on = recv.join("`, `");
                        diagnostic_at!(
                            self.lexer,
                            &pos,
                            Level::Error,
                            "`{nm}` is a method on `{on}`, and a method is not a function VALUE \
                             — there is nothing to pass here. Wrap it: `|x| {{ x.{nm}(…) }}`, or \
                             declare the function with a plain first-parameter name (not \
                             `self`), which makes it a free function and a usable fn-ref"
                        );
                    } else {
                        // @C112 — a local hiding the function the author named: say so.
                        let context = match self.shadowed_function_hint(&actual_code, true) {
                            Some(hint) if matches!(tp.base(), Type::Function(..)) => {
                                format!("{context}; {hint}")
                            }
                            _ => context,
                        };
                        self.validate_convert(&context, actual_type, &tp, &pos);
                    }
                } else if !self.can_convert(actual_type, &tp) {
                    return Vec::new();
                }
            }
            actual.push(actual_code);
        }
        actual
    }

    /// loft#663 — the caller buffer a heap-vector return is actually delivered
    /// into, for a callee whose declared return deps cannot be read yet.
    ///
    /// A vector return names its hidden `__retbuf` attribute in `returned()`'s
    /// deps, and `resolve_deps` maps that onto the buffer the call site
    /// allocated.  But while a function's OWN body is being parsed its returned
    /// type has not been re-promoted this pass, so a SELF- or MUTUALLY-recursive
    /// call reads EMPTY deps.  The receiving local then looks like a vector that
    /// owns nothing, and `vector_db` hands it a fresh backing store and repoints
    /// it — discarding the value the call had just delivered into the buffer, so
    /// `r = f(n-1); r += [x]; return r` returned only `[x]`.
    ///
    /// A body-carrying vector-returning function always has the unconditional H1
    /// `__retbuf`, and `add_defaults` has already allocated the buffer for it
    /// (that attribute's entry in `types` carries `Deps::frame1(vr)`), so when
    /// the declared deps are silent that buffer is where the value is.  A
    /// fallback only: a callee whose signature IS final resolves exactly as
    /// before.
    pub(super) fn caller_vector_retbuf_dep(&self, d_nr: u32, types: &[Type]) -> Vec<u16> {
        let attrs = self.data.def(d_nr).attributes();
        for (i, a) in attrs.iter().enumerate() {
            if !a.hidden || i >= types.len() {
                continue;
            }
            if let Type::Vector(_, dep) = &types[i]
                && !dep.is_empty()
            {
                return dep.into_iter().copied().collect();
            }
        }
        Vec::new()
    }

    // Gather depended on variables from arguments of the given called routine.
    pub(super) fn call_dependencies(&mut self, d_nr: u32, types: &[Type]) -> Type {
        // loft#938 — peel `Optional` before asking the shape question, and put it back on
        // the answer.  Every arm below matches a BARE shape, so a `-> vector<T>?` return
        // matched none of them and fell through the final `else { tp }`, which hands the
        // declared type back VERBATIM — with the callee's deps still in it.  Those deps are
        // ATTRIBUTE indices in callee space; read in the caller they are frame variable
        // numbers, so the result was typed as borrowing whichever caller local happened to
        // hold that number.  With two call sites the second one landed on the first's local
        // and was freed on its schedule (`gd`/`gl` both typed `["gd"]`), which is the whole
        // of the `LOFT_NULLABLE_RETBUF` blocker: values all correct, nothing leaked, and
        // only `LOFT_STRICT_STORES=1` able to see it.
        //
        // Same peel `scopes.rs` already does for the ownership question (loft#879's
        // `peel_optional` + [`Self::reopt`]); this is the dep-translation half.
        let (declared, opt) = self.data.def(d_nr).returned().peel_optional();
        let was_optional = opt && crate::keys::optional_dep_peel();
        let tp = if was_optional {
            declared.clone()
        } else {
            self.data.def(d_nr).returned().clone()
        };
        let reopt = |t: Type| {
            if was_optional { Type::optional(t) } else { t }
        };
        // for Reference returns (structs), filter out hidden return-mechanism
        // attributes from dep resolution. The struct owns its store independently —
        // hidden return-store buffers are implementation artifacts.
        // Text/Vector returns genuinely depend on their hidden work buffers.
        let attrs = self.data.def(d_nr).attributes();
        let filter_hidden = |d: &[u16]| -> Vec<u16> {
            d.iter()
                .copied()
                .filter(|&i| (i as usize) >= attrs.len() || !attrs[i as usize].hidden)
                .collect()
        };
        if let Type::Text(d) = tp {
            reopt(Type::Text(Deps::frame(Self::resolve_deps(
                types,
                d.as_attr_indices(),
            ))))
        } else if let Type::Vector(to, d) = tp {
            // A vector return genuinely depends on its hidden retbuf (see above), so
            // an EMPTY result here means the dep could not be read — not that the
            // value owns nothing.  Fall back to the buffer this call site allocated
            // (loft#663).
            let mut dp = Self::resolve_deps(types, d.as_attr_indices());
            // Only a callee that has NOT finished parsing this pass can have lost
            // its dep — that is exactly a SELF (`d_nr == context`) or FORWARD
            // (`d_nr > context`) reference.  A backward-ref callee is fully
            // resolved, so an empty dep there is the truth and must be left alone
            // (the same def_nr ordering test `tret_bind_ok` uses).
            if dp.is_empty() && d_nr >= self.context {
                dp = self.caller_vector_retbuf_dep(d_nr, types);
            }
            reopt(Type::Vector(to, Deps::frame(dp)))
        } else if let Type::Sorted(to, key, d) = tp {
            reopt(Type::Sorted(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ))
        } else if let Type::Hash(to, key, d) = tp {
            reopt(Type::Hash(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ))
        } else if let Type::Index(to, key, d) = tp {
            reopt(Type::Index(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ))
        } else if let Type::Radix(to, key, d) = tp {
            reopt(Type::Radix(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            ))
        } else if let Type::Reference(to, d) = tp {
            reopt(Type::Reference(
                to,
                Deps::frame(Self::resolve_deps(
                    types,
                    &filter_hidden(d.as_attr_indices()),
                )),
            ))
        } else if let Type::Enum(to, true, d) = tp {
            reopt(Type::Enum(
                to,
                true,
                Deps::frame(Self::resolve_deps(
                    types,
                    &filter_hidden(d.as_attr_indices()),
                )),
            ))
        } else {
            reopt(tp)
        }
    }

    /// @PLN85 L1 — the def→frame dep conversion for FN-REF call results.
    /// A fn-ref/lambda call's declared return type carries the CALLEE's
    /// attr-space deps (e.g. a grown hidden work-buffer's index).  Read
    /// verbatim in the caller they alias arbitrary caller attrs/frame vars —
    /// the L1 leak: a lambda's hidden-buffer index 1 read as the CALLER's
    /// attr 1 (`__retbuf`) made `ref_return` believe the CallRef tail already
    /// rode the buffer, so no delivery was emitted and the store
    /// `fn_call_ref` allocates at runtime leaked on both backends.  Convert
    /// exactly as plain calls do (`call_dependencies`): map visible-param
    /// indices through the actual argument types.
    ///
    /// **An index naming no visible argument names the CLOSURE, and `fn_var` is where the
    /// caller can reach it** (loft#1180).  Dropping those indices outright — on the stated
    /// grounds that the fn-ref ABI allocates such buffers at runtime, *"so the value arrives
    /// OWNED"* — is true of a hidden work buffer and false of `__closure`, which is the
    /// CALLER's own record.  A lambda handing back what it captured then read as owned, so
    /// the bind ADOPTED the capture and scope exit released it: the captured variable
    /// answered EMPTY from the second call onward, on both backends, with nothing saying so.
    /// This is loft#1114's sentence in a third position — *a dep dropped as uninteresting is
    /// not a dep that was never there.*
    ///
    /// Only for a fn-ref that CAPTURES, which is what `fn_var` carries: a non-capturing slot
    /// has no closure record to borrow from, so its dropped indices really are the runtime
    /// buffers and dropping them is right.  The predicate is sound HERE, where the fn-ref is
    /// a caller LOCAL whose type was inferred at the bind and whose deps therefore name the
    /// closure — it is inert one frame down, where the same slot is a parameter with a
    /// DECLARED fn-type that carries no deps whatever is passed (loft#1176 measured that).
    ///
    /// ⚠ **And only for a COLLECTION return, which is where the capture actually crosses.**
    /// A struct, record-enum or text return is materialised into a fresh copy before it
    /// leaves the callee, so `fn(i: integer) -> P { cap }` hands the caller its own record
    /// and is correct as it stands; a collection return hands back the STORE.  Without that
    /// restriction the dep is a pure over-approximation and it costs: the out-of-range index
    /// is `__closure` for `{ cap }` and `{ q.xs }` AND for `{ sr_make(k) }`, whose result is
    /// a fresh store built from a captured value, so a dep-index test alone cannot separate
    /// them and reading it as a borrow leaks every struct-returning capturing lambda —
    /// eleven stores of them in `717-closure-struct-return.loft`.
    /// `v_nr` when it holds a CAPTURING fn-ref, and `None` otherwise — the closure a
    /// [`fnref_result_type`](Self::fnref_result_type) dep can name.
    ///
    /// The fn-ref TYPE's own deps are exactly the capture question, and they mean it in this
    /// position: a caller local's fn-type was INFERRED at the bind, so a capturing lambda
    /// leaves the closure record in them and a non-capturing one leaves them empty.
    pub(super) fn capturing_fnref_var(vars: &crate::variables::Function, v_nr: u16) -> Option<u16> {
        matches!(vars.tp(v_nr).base(), Type::Function(_, _, d, ..) if !d.is_empty()).then_some(v_nr)
    }

    /// The return type a fn-ref VALUE publishes to whoever holds it — the deps a CALLER can
    /// act on, and only those.
    ///
    /// A definition's return deps are ATTR-space indices into its own parameter list, and a
    /// caller can name only the visible ones.  Of the rest exactly ONE is something the
    /// caller reaches: `__closure`, the record the caller allocated and still owns.  Every
    /// other hidden attribute — `ref_return`'s `__ref_N`, a text work buffer — is an
    /// allocation the CALLEE makes and hands over, so a return tied to it arrives owned.
    ///
    /// Publishing both as *"an index the caller cannot name"* is what made `{ cap }` and
    /// `{ sr_make(k) }` read the same downstream, and the only way to read the first as a
    /// borrow was then to read the second as one too — every struct-returning capturing
    /// lambda into a leak.  [`Argument::hidden`](crate::data::Argument::hidden) is the
    /// distinction, and its own doc already states the conclusion: *"not a user-declared
    /// parameter — should be excluded from dep propagation."*  Dropping those indices here
    /// changes nothing else, because [`fnref_result_type`](Self::fnref_result_type) drops
    /// every out-of-range index anyway; what it buys is that the one index left over means
    /// what the caller reads it to mean.
    pub(super) fn published_ret_type(&self, d_nr: u32, ret: Type) -> Type {
        let Some(dep) = ret.deps_ref() else {
            return ret;
        };
        let def = self.data.def(d_nr);
        // A JOIN counts as well as a plain place, and the hand-up is what pays for it.  Read
        // as OWNED, the arm that yields the CAPTURE is a use-after-free the moment the caller
        // rebinds (loft#1186's present arm, and loft#1181's mechanism one shape over).  Read
        // as a borrow, the arm that MINTS leaks — which is why this used to answer only for a
        // tail that cannot join.  `OpFreeRefOrHandUp` gives that mint an owner at the one
        // place the callee knows it is handing back its own store, so the borrow reading is
        // now the right one for both arms.  See `formal/closures.md` D-clo-13, @PLN150.
        let place_tail = self.tail_is_a_place(def) || self.tail_joins_with_a_place(def);
        let kept: Vec<u16> = dep
            .as_attr_indices()
            .iter()
            .copied()
            .filter(|a| match def.attributes().get(*a as usize) {
                // A hidden buffer is the callee's own allocation, handed over: OWNED.
                Some(at) if at.hidden => false,
                // The caller's own record — a borrow, but only where the tail READS it.
                Some(at) if at.name == "__closure" => place_tail,
                // A visible parameter: the caller maps it through its own argument.
                Some(_) => true,
                // Past the attribute list: the same closure question in a grown spelling.
                None => place_tail,
            })
            .collect();
        if kept.len() == dep.as_attr_indices().len() {
            return ret;
        }
        ret.with_deps(&crate::data::Deps::attrs(kept))
    }

    /// Is this body's tail a PLACE — a slot, or a field / element / capture read out of one?
    ///
    /// The question [`published_ret_type`](Self::published_ret_type) asks about the closure:
    /// a tail that READS the closure hands back the captured store itself, so the caller
    /// borrows.  A tail that JOINS — a `??`, an `if`, a `match` — may hand back a fresh store
    /// on one arm and the capture on another, and the union its deps carry then names the
    /// closure for a value that is often the callee's own.  Neither static reading serves both
    /// arms: read as a borrow the minting arm leaks, read as owned the capture arm is released
    /// while its variable is still live.  Answering only for the tail that cannot join keeps
    /// the reading positive and UNDER-approximating, which is the direction every ownership
    /// proof in this file takes — a shape it cannot read keeps the behaviour it already had.
    ///
    /// Reads through the projection ops and `OpGetDbRef`, which is how a capture is read out
    /// of the closure record.
    pub(super) fn tail_is_a_place(&self, def: &crate::data::Definition) -> bool {
        self.tail_place_root(def).is_some()
    }

    /// Does this body's tail JOIN, with at least one arm a place?
    ///
    /// The `??` / `if` / `match` shape [`tail_is_a_place`](Self::tail_is_a_place) deliberately
    /// answered `false` for, back when a borrow reading leaked the minting arm.  Kept as its
    /// own question rather than folded into `tail_place_root`: that one answers *"which
    /// variable IS the tail"*, and a join has no single answer — asking it separately leaves
    /// every non-join tail reading exactly as before.
    ///
    /// Looks through the value-block wrappers a lowered `??` puts in the way, and asks only
    /// about the ARMS: a join two of whose arms mint is not this question.
    pub(super) fn tail_joins_with_a_place(&self, def: &crate::data::Definition) -> bool {
        let Value::Block(bl) = def.code() else {
            return false;
        };
        let Some(tail) = bl.operators.last() else {
            return false;
        };
        let mut node = tail.unspan();
        loop {
            match node {
                Value::Return(inner) => node = inner.unspan(),
                Value::Insert(steps) | Value::Parallel(steps) => match steps.last() {
                    Some(last) => node = last.unspan(),
                    None => return false,
                },
                Value::Block(inner) => match inner.operators.last() {
                    Some(last) => node = last.unspan(),
                    None => return false,
                },
                Value::If(_, t, e) => {
                    return self.node_place_root(t).is_some() || self.node_place_root(e).is_some();
                }
                _ => return false,
            }
        }
    }

    /// [`tail_place_root`](Self::tail_place_root) for an arbitrary node rather than a body's
    /// tail — the arm-level question [`tail_joins_with_a_place`](Self::tail_joins_with_a_place)
    /// asks.
    pub(super) fn node_place_root(&self, node: &Value) -> Option<u16> {
        let (get_field, get_vector, get_dbref) = (
            self.data.def_nr("OpGetField"),
            self.data.def_nr("OpGetVector"),
            self.data.def_nr("OpGetDbRef"),
        );
        let mut node = node.unspan();
        loop {
            match node {
                Value::Return(inner) => node = inner.unspan(),
                Value::Insert(steps) => node = steps.last()?.unspan(),
                Value::Block(bl) => node = bl.operators.last()?.unspan(),
                Value::Var(v) | Value::TupleGet(v, _) => return Some(*v),
                Value::Call(d, args) if *d == get_field || *d == get_vector || *d == get_dbref => {
                    node = args.first()?.unspan();
                }
                _ => return None,
            }
        }
    }

    /// The root variable of a PLACE tail — `None` when the tail is not a place.
    pub(super) fn tail_place_root(&self, def: &crate::data::Definition) -> Option<u16> {
        let Value::Block(bl) = def.code() else {
            return None;
        };
        let tail = bl.operators.last()?;
        let (get_field, get_vector, get_dbref) = (
            self.data.def_nr("OpGetField"),
            self.data.def_nr("OpGetVector"),
            self.data.def_nr("OpGetDbRef"),
        );
        let mut node = tail.unspan();
        loop {
            match node {
                Value::Return(inner) => node = inner.unspan(),
                // `ref_return`'s delivery wrapper: the steps that prepare a buffer, then the
                // value actually handed back.  The LAST element is the answer — a body that
                // clears a buffer and then returns a place is the shape loft#1182 names,
                // where the buffer is declared and ignored.
                Value::Insert(steps) => node = steps.last()?.unspan(),
                Value::Var(v) | Value::TupleGet(v, _) => return Some(*v),
                Value::Call(d, args) if *d == get_field || *d == get_vector || *d == get_dbref => {
                    node = args.first()?.unspan();
                }
                _ => return None,
            }
        }
    }

    pub(super) fn fnref_result_type(ret: Type, types: &[Type], fn_var: Option<u16>) -> Type {
        let visible = |d: &Deps| Deps::frame(Self::resolve_deps(types, d.as_attr_indices()));
        // The CLOSURE half.  Every index left in a PUBLISHED return type that names no
        // visible argument names `__closure` — [`published_ret_type`](Self::published_ret_type)
        // drops the callee's own hidden buffers before the type ever leaves the lambda — so
        // an out-of-range index here is the caller's own record and the value is a BORROW of
        // it.  A store handed back that way is the capture itself, whatever its type:
        // `(L-CapHeap)` names struct and vector in one breath, and the caller may read it,
        // never take it.
        let borrows_closure = |d: &Deps| {
            fn_var.is_some()
                && d.as_attr_indices()
                    .iter()
                    .any(|a| *a as usize >= types.len())
        };
        // `visible`, plus the fn-ref slot when the return borrows the closure it carries.
        let through = |d: &Deps| {
            let mut out = Self::resolve_deps(types, d.as_attr_indices());
            if borrows_closure(d)
                && let Some(v) = fn_var
                && !out.contains(&v)
            {
                out.push(v);
            }
            Deps::frame(out)
        };
        // Which shapes carry a borrow list is `Type::deps_ref` / `Type::with_deps`'s
        // question (@FR-O-Deps: one fact, read everywhere), so it is asked there rather than
        // restated as a list of variants here.  The list this replaces named four — text,
        // vector, record, record enum — and a KEYED collection or a nullable return fell
        // through it with the callee's ATTRIBUTE indices still in place, which the caller
        // then read as its own frame variables: `r = if c { h(bag) } else { d }` with
        // `h = fn(q: Bag) -> hash<K[k]> { q.m }` unioned attribute 0 (`q`) into a frame-space
        // list — the debug-assertions gate's "dep-space violation" (loft#1335), and in a
        // release build a borrow of whatever caller variable happens to be number 0.
        match ret {
            // Text is COPIED out through a work buffer rather than handed back, so the
            // caller's value is its own however the callee reached it — under `?` too.
            other if matches!(other.base(), Type::Text(_)) => match other.borrow_deps() {
                Some(d) => other.rewrap_deps(&visible(&d)),
                None => other,
            },
            // A tuple has no list of its own: each element is a return in its own right.
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::fnref_result_type(e, types, fn_var))
                    .collect(),
            ),
            // Every other kind that borrows — vector, struct, enum, the five keyed
            // collections, a fn-ref, and an `Optional`/`RefVar` over any of them — is asked
            // through `borrow_deps`, which is where "what does this type borrow?" is
            // answered and which peels a `Rewritten` wrapper on the way.
            // Enforces @FR-O-Move: *if the return borrows a parameter, the return type
            // records it* — recorded in the callee's space, and mapped into the caller's here.
            // The hand-written list this replaces named four shapes, so a keyed return
            // (`fn(q: Bag) -> hash<K[k]> { q.m }`) reached the caller with the callee's
            // ATTRIBUTE indices still in it: read in the caller's frame they name whichever
            // local holds that number, and a branch join over such an arm unioned attr-space
            // with frame-space deps (the debug-assertions gate's `dep-space violation`,
            // loft#1335).  `call_dependencies` is the named-call twin, and loft#938 was the
            // same omission there for `Optional`.
            other => match other.borrow_deps() {
                Some(d) => other.rewrap_deps(&through(&d)),
                None => other,
            },
        }
    }

    /// THE def→frame dep converter (H2 / DEPS_INVENTORY): maps the
    /// callee's ATTR-INDEX deps through the actual argument types at a
    /// call site into caller FRAME var deps.  `d` must be attr-space
    /// (callers read it via `Deps::as_attr_indices`); the result is
    /// wrapped `Deps::frame` by `call_dependencies`.
    pub(super) fn resolve_deps(types: &[Type], d: &[u16]) -> Vec<u16> {
        let mut dp = HashSet::new();
        for ar in d {
            if *ar as usize >= types.len() {
                continue;
            }
            // Asked through [`Type::deps_ref`], which is where "what does this type
            // borrow?" is answered, rather than re-listing the kinds that carry deps.  The
            // hand-written list this replaces was missing `Optional`, so a `τ?` argument
            // contributed nothing: a function delegating to one that borrows its argument
            // (`fn outer(a: S?, c) -> S? { inner(a, c) }`) declared an EMPTY return dep and
            // so read as returning an OWNED store.  The caller then lifted that return and
            // freed it — releasing its own record while the variable holding it was still
            // live, and answering whatever the next allocation reused (loft#1114's class,
            // and its dense twin was correct throughout).  `deps_ref` documents `Optional`
            // as dep-transparent; a second list could only agree with it by accident.
            if let Some(ad) = types[*ar as usize].deps_ref() {
                for a in ad {
                    dp.insert(*a);
                }
            }
        }
        Vec::from_iter(dp)
    }

    /// The pooled buffer for work-buffer attribute `a_nr` of callee `d_nr` in the current
    /// caller (`Parser::work_buffer_pool`): minted on its first use, the same variable after.
    /// `None` when no pool is open or the attribute is not a work buffer — a return buffer's
    /// contents outlive the call, so it keeps a store per site.
    pub(super) fn pooled_work_buffer(
        &mut self,
        d_nr: u32,
        a_nr: usize,
        content: &Type,
    ) -> Option<u16> {
        self.work_buffer_pool.as_ref()?;
        let attrs = self.data.def(d_nr).attributes();
        if !attrs[a_nr].work_buffer {
            return None;
        }
        let same = |a: &crate::data::Attribute| {
            a.work_buffer && matches!(a.typedef.base(), Type::Vector(c, _) if **c == *content)
        };
        let k = attrs[..a_nr].iter().filter(|a| same(a)).count();
        let key = (self.context, format!("{content:?}"), k);
        if let Some(&vr) = self.work_buffer_pool.as_ref()?.get(&key)
            && vr < self.vars.count()
        {
            return Some(vr);
        }
        let buf_tp = Type::Vector(Box::new(content.clone()), Deps::none());
        let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
        self.work_buffer_pool.as_mut()?.insert(key, vr);
        Some(vr)
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn add_defaults(
        &mut self,
        d_nr: u32,
        actual: &mut Vec<Value>,
        all_types: &mut Vec<Type>,
    ) {
        // @PLAN59 phase 2: the `__rref_N` recursive-self counter dance is
        // gone.  It existed to keep `__ref_N` numbering pass-stable so
        // `ref_return` could re-find its promoted attr BY NAME instead of
        // growing the attr count across passes.  With the signature-time
        // `__retbuf` the attr exists before any body parses and the
        // promotion re-find no longer depends on work-ref numbering.
        // Extend to full parameter count so we can fill gaps from named arguments.
        while actual.len() < self.data.attributes(d_nr) {
            actual.push(Value::Null);
            all_types.push(Type::Unknown(0));
        }
        {
            // Fill all missing (Null) parameter slots with defaults.
            for a_nr in 0..self.data.attributes(d_nr) {
                if actual[a_nr] != Value::Null {
                    continue;
                }
                let default = self.data.def(d_nr).attributes()[a_nr].value.clone();
                let tp = self.data.attr_type(d_nr, a_nr);
                // Arity: a slot with NO provided argument and NO explicit default is a
                // too-few-arguments error. loft's old "defaulted-null args" lenience (omit a trailing
                // arg → null/empty fill) was REMOVED (owner decision 2026-07-17): it was a footgun —
                // a real consumer hit it as both a stdlib SIGSEGV (a missing fn-typed arg fills a
                // broken `()`) and a silent-wrong (a missing scalar fills null). Skip the cases that
                // are NOT a user argument:
                //  · a NULLABLE param defaults to null (still optional);
                //  · a `= expr` default (value != Null) is filled below;
                //  · a COMPILER-inserted return slot — `hidden` (a `ref_return` out-buffer), a
                //    `__`-prefixed name (`__retbuf` / `__work_N` / `__tret`), OR an attr the RETURN
                //    VALUE depends on (a local promoted to a caller buffer, e.g. a returned view
                //    `return tv[0]` keeps the local's name, so `returned.depend()` names that index).
                // Pass 1 defers (a forward-ref `&` arg lowers to Null and only looks missing then);
                // the named-param feature made an internal Null slot normal, which lost this check.
                let (a_name, a_hidden) = {
                    let a = &self.data.def(d_nr).attributes()[a_nr];
                    (a.name.clone(), a.hidden)
                };
                let promoted = a_hidden
                    || a_name.starts_with("__")
                    || self
                        .data
                        .def(d_nr)
                        .returned
                        .depend()
                        .contains(&(a_nr as u16));
                // A slot the caller DID write, whose expression already errored, is not a
                // missing argument.  Such an expression is poisoned to `Never` by the site
                // that reported it (@P376's rule), and it lowers to `Value::Null` because
                // there is no value to lower — which is exactly what this check reads as
                // "nothing supplied".  Reporting it again names an internal opcode and a
                // parameter the program never wrote: `v > PEUnknown` answered
                // "missing argument for parameter 'v1' of `OpLtInt`" (loft#934).
                let already_errored = matches!(all_types.get(a_nr), Some(Type::Never));
                if !self.first_pass
                    && default == Value::Null
                    && !matches!(tp, Type::Optional(_))
                    && !promoted
                    && !already_errored
                {
                    let fname = self.data.def(d_nr).display_name().to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "missing argument for parameter '{a_name}' of `{fname}` — the call supplies \
                         too few arguments (add it, or give the parameter a default `= …`)"
                    );
                    continue;
                }
                // The three heap arms below allocate a caller-side BUFFER for a slot that
                // has no value of its own — a compiler-inserted return slot, or a
                // parameter left to fill.  A slot with a user `= …` default has a value,
                // so it takes the general path instead; before loft#699 these arms ran
                // for it too and asserted the default away.
                let no_default = default == Value::Null;
                if let Type::Vector(content, _) = &tp
                    && no_default
                {
                    // #306: the attr's dep list is callee-internal (attr
                    // indices); inherited verbatim it reads as CALLER var
                    // numbers and mislabels the fresh buffer a borrow.
                    let buf_tp = Type::Vector(content.clone(), Deps::none());
                    let vr = match self.pooled_work_buffer(d_nr, a_nr, content) {
                        Some(vr) => vr,
                        None => self.vars.work_refs(&buf_tp, &mut self.lexer),
                    };
                    // @PLAN51 Cluster IV: tag this work-ref so parse_code's
                    // preamble emits Set(vr, Null) regardless of vr's
                    // typedef dep list.  Without it, if-tail / recursion
                    // shapes leave vr without a first_def → slot allocator
                    // skips → codegen panics.
                    self.vars.mark_caller_hidden_buf(vr);
                    self.data.vector_def(&mut self.lexer, content);
                    all_types[a_nr] = Type::Vector(content.clone(), Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Reference(content, _) = tp
                    && no_default
                {
                    // #306: strip the callee-internal dep list (see Vector arm).
                    let buf_tp = Type::Reference(content, Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(vr);
                    all_types[a_nr] = Type::Reference(content, Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Enum(content, true, _) = tp
                    && no_default
                {
                    // @P301 — struct-enums are heap records like
                    // Reference/Vector, so a struct-enum return-slot
                    // promoted to a hidden caller arg by `ref_return`
                    // (parser/control.rs) needs a pre-allocated work-ref
                    // passed in.  Without this arm the hidden param
                    // stayed `Value::Null` → emitted as `()` natively
                    // (E0308: expected DbRef, found ()).  Mirrors the
                    // Reference arm above, keeping the struct-enum
                    // discriminator in the result type.
                    // #306: strip the callee-internal dep list (see Vector arm).
                    let buf_tp = Type::Enum(content, true, Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(vr);
                    all_types[a_nr] = Type::Enum(content, true, Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::RefVar(vtp) = &tp {
                    let mut ls = Vec::new();
                    let vr = if matches!(**vtp, Type::Text(_)) {
                        // Its OWN counter, like the `work_refs` retbuf arm above
                        // (loft#662): this buffer only exists once the callee's
                        // `&text` ABI does, so for a self-/forward-recursive callee
                        // it is minted on pass 2 ONLY.  Taking it from `work_text`
                        // shifted every later `__work_N` by one, and since the
                        // variable tables persist across passes by name, pass 2's
                        // format buffers then re-found pass 1's variables under the
                        // wrong roles.
                        let wv = self.vars.caller_text_buf(&mut self.lexer);
                        // clear the work buffer before each call so loop
                        // iterations start fresh (matches fn-ref path in control.rs).
                        ls.push(v_set(wv, Value::Text(String::new())));
                        if default != Value::Null
                            && if let Value::Text(t) = &default {
                                !t.is_empty()
                            } else {
                                true
                            }
                        {
                            ls.push(self.cl("OpAppendText", &[Value::Var(wv), default]));
                        }
                        wv
                    } else if self.first_pass {
                        // Defer on pass 1 (#375): this missing-slot filler runs
                        // when an arg lowered to `Null`.  A `&` field arg whose
                        // owning struct's layout is not yet finalised (a forward /
                        // cross-package field still resolving) lowers to `Null`
                        // here, so it looks like a missing parameter.  Erroring
                        // would abort pass 1; on pass 2 the access lowers to a real
                        // `OpGetField` (non-Null), this filler is skipped, and a
                        // genuinely-missing non-text `&` default still errors then.
                        0
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unexpected reference type {}",
                            vtp.source_name(&self.data)
                        );
                        0
                    };
                    // A var whose own type is already `RefVar(_)` HOLDS the borrow.
                    // `OpCreateStack` is how a var that OWNS its value hands out a
                    // reference to it; applied to a reference it builds a DbRef pointing
                    // at the reference SLOT, and the callee's single deref then reads that
                    // slot as the value — loft#806, a SIGSEGV in the interpreter.
                    // `--native` passes the reference by the Rust ABI and is immune, which
                    // is why the two backends disagreed instead of both crashing.
                    //
                    // `caller_text_buf` reaches such a var because it re-finds `__work_cN`
                    // by NAME, and that name can already be a `&text` PARAMETER of this
                    // function: `text_return` promotes a text local the return value
                    // depends on into a hidden caller buffer (loft#662), and the promotion
                    // persists in the var table across the pass boundary.  This is the same
                    // rule #266 states for non-text references at the argument-coercion
                    // site; that site compares TYPES, so a var already holding the wanted
                    // reference never reaches its conversion at all.  This one picks its
                    // variable by NAME, so it has to ask.
                    //
                    // Hand over the BARE variable, not a wrapper.  Both backends already
                    // forward a `RefVar` argument into a `RefVar` parameter without a
                    // deref (`codegen.rs` OpVarRef; `generation/calls.rs` `var_x`), and
                    // both recognise that shape only as a literal `Value::Var` — wrapped
                    // in a block or an `Insert` the generic path runs instead and re-derefs.
                    // The per-call clear is not lost: a promoted buffer is cleared by the
                    // function preamble, once per invocation, and promotion only happens
                    // when the RETURN VALUE depends on the buffer, which puts its call in
                    // tail position — reached once.
                    //
                    // Restricted to the no-default case (`ls` holding only that clear).
                    // A `&text` parameter carrying a `= "…"` default needs the default
                    // WRITTEN into the buffer, which a bare var cannot express; that path
                    // keeps its existing lowering rather than trading this crash for a
                    // dropped default.  `vr == 0` is the pass-1 defer / unexpected-type
                    // sentinel from the arms above, not a variable.
                    // `ls` is exactly the buffer-clearing `Set` when the parameter has no
                    // `= "…"` default; a default adds the `OpAppendText` that writes it.
                    let only_the_clear = ls.len() == 1;
                    let holds_borrow =
                        vr != 0 && only_the_clear && matches!(self.vars.tp(vr), Type::RefVar(_));
                    if holds_borrow {
                        actual[a_nr] = Value::Var(vr);
                    } else {
                        ls.push(self.cl("OpCreateStack", &[Value::Var(vr)]));
                        actual[a_nr] = v_block(
                            ls,
                            Type::Reference(self.data.def_nr("reference"), Deps::frame1(vr)),
                            "default ref",
                        );
                    }
                    all_types[a_nr] = tp.clone();
                } else {
                    // default expressions may reference earlier
                    // parameters by `Var(N)` slots (e.g. `b: integer = a * 2`
                    // produces a tree with `Var(0)`).  Substitute those
                    // references with the caller's actual argument values
                    // so the emitted code uses the caller's scope, not
                    // the callee's (which wouldn't resolve at the call
                    // site).  Only parameters 0..a_nr are earlier; no
                    // recursion into the current or later default.
                    let mut substituted = Self::substitute_param_refs(default, &actual[..a_nr]);
                    all_types[a_nr] = tp.clone();
                    // loft#699 — a default that CALLS something (the function a default
                    // needing a temporary is lowered into, or a plain `= mk()`) is stored
                    // with its user arguments only.  A call returning a heap value also
                    // takes a caller-allocated return buffer, and "caller" is THIS frame,
                    // not the signature the default was written in — so the hidden slots
                    // are filled here, once per call site, exactly as `object_init` fills
                    // them for a field default.  Left unfilled the call reached codegen a
                    // parameter short and tripped its arity assert.  The buffer is what
                    // gives the value a home, so its type is what this argument IS.
                    if let Value::Call(d, args) = &substituted
                        && args.len() < self.data.attributes(*d)
                    {
                        let (d, mut inner) = (*d, args.clone());
                        let mut inner_types = vec![Type::Unknown(0); inner.len()];
                        self.add_defaults(d, &mut inner, &mut inner_types);
                        if let Some(buf) = inner_types.get(args.len())
                            && !buf.is_unknown()
                        {
                            all_types[a_nr] = buf.clone();
                        }
                        substituted = Value::Call(d, inner);
                    }
                    // loft#1758, `@FR-F-Default` — the default answers what the same expression
                    // written as the argument would, and a call returning a lifetime-bearing
                    // tuple hands back the BOXED `__tuple<…>` record (`boxed_tuple_return`).
                    // An argument written by hand is unboxed on its way into a tuple parameter
                    // (`convert`'s stored-tuple arm); a default's call has to take the same
                    // conversion, or the callee reads the record's DbRef as its tuple — `null`
                    // members on the interpreter, a type error under `--native`.
                    if matches!(tp.base(), Type::Tuple(_))
                        && matches!(all_types[a_nr].base(), Type::Reference(_, _))
                    {
                        let boxed = all_types[a_nr].clone();
                        self.convert(&mut substituted, &boxed, &tp);
                        all_types[a_nr] = tp.clone();
                    }
                    actual[a_nr] = substituted;
                }
            }
        }
    }

    /// replace `Value::Var(from)` with `Value::Var(to)` throughout
    /// a default-expression tree.  Used by `parse_arguments` to rewrite
    /// internally-allocated slot numbers into stable argument indices
    /// before the default is stored on the function definition.
    ///
    /// Walks with [`visit_constant_vars`], the exhaustive walker, rather than a `match`
    /// of its own.  The hand-written one silently returned any shape it did not list
    /// UNCHANGED, and `Span` was one of them — so on the second pass, where the parser
    /// records positions, `b: integer = a + c` kept the CALLEE's raw slot numbers and
    /// the two passes disagreed about what the default names (loft#699).  A walker that
    /// reports refusal instead of guessing cannot develop that hole again: a shape it
    /// cannot rewrite leaves foreign indices in place, which
    /// `default_replayable_in_place` then reads as "not replayable" and lowers into a
    /// function of its own.
    pub(crate) fn remap_var_nr(mut val: Value, from: u16, to: u16) -> Value {
        crate::parser::definitions::visit_constant_vars(&mut val, &mut |v| {
            if *v == from {
                *v = to;
            }
        });
        val
    }

    /// replace `Value::Var(i)` for `i < args.len()` with `args[i]`
    /// in a default-expression tree.  Used at call sites to transplant a
    /// default's earlier-parameter references into the caller's scope.
    /// `@FR-F-Default` — a default's reference to an EARLIER parameter, `Var(n)` for
    /// `n < args.len()`, replaced by the caller's argument for it.  Through EVERY node shape
    /// (`Value::for_each_child_mut`, the exhaustive child list): a hand-kept list of five shapes
    /// left a TUPLE literal's members alone, so `t: (integer, integer) = (k, 9)` read the
    /// CALLER's `Var(0)` for `k` — `h(1)` answered `(0, 9)`, silently, and native did not
    /// compile.  A replacement is not descended into: it is the caller's expression, and its own
    /// variables are the caller's.
    pub(super) fn substitute_param_refs(mut val: Value, args: &[Value]) -> Value {
        if let Value::Var(n) = val {
            return if (n as usize) < args.len() {
                args[n as usize].clone()
            } else {
                val
            };
        }
        val.for_each_child_mut(&mut |c| {
            let child = std::mem::replace(c, Value::Null);
            *c = Self::substitute_param_refs(child, args);
        });
        val
    }
}
