// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Core IR-to-Rust emission: translates `Value` IR nodes into Rust source.

use crate::data::{Block, Context, Deps, IntegerSpec, Type, Value};
use crate::data_store::ValueType;
use crate::ir_node::{IrBlock, IrNode};
use std::io::Write;

use super::text::count_format_ops;
use super::{
    Output, block_needs_i64_widen, block_tail_cast, default_native_value, default_native_value_in,
    narrow_int_cast, rust_type, sanitize,
};

impl Output<'_> {
    /// `&Value` entry — wraps the native node in an [`IrNode`] and delegates to
    /// [`Self::output_code_node`].  @PLN11 G2/M4 (cf. interpreter `generate`):
    /// every existing caller keeps this signature; M5's store-backed entry calls
    /// `output_code_node(IrNode::Store(…))`.
    pub(super) fn output_code_inner(
        &mut self,
        w: &mut dyn Write,
        code: &Value,
    ) -> std::io::Result<()> {
        // One-walk pre-eval substitution: if THIS node was hoisted for the
        // statement being emitted, emit its `_pre_N` name.  Keyed on the node's
        // address (intrinsic identity) — no counter, no regenerated-text match.
        // Every operand emission funnels through here (incl. `#rust` template
        // operands via `generate_expr_buf`), so this single check covers them
        // all.  See COMPILER.md § Synthesised-identity stability.
        if !self.active_pre_eval.is_empty()
            && let Some(name) = self
                .active_pre_eval
                .get(&(std::ptr::from_ref(code) as usize))
        {
            return write!(w, "{name}");
        }
        self.output_code_node(w, IrNode::Native(code))
    }

    /// Central recursive dispatch from an IR node to its Rust representation
    /// (@PLN11 G2/M4).  Dispatches on `node.kind()` and reads payloads through
    /// the `IrNode` handle; arms not yet converted fall to the `match
    /// node.as_native()` below (native-backed bridge, lifted as they convert).
    #[allow(clippy::too_many_lines)]
    pub(super) fn output_code_node(
        &mut self,
        w: &mut dyn Write,
        node: IrNode,
    ) -> std::io::Result<()> {
        match node.kind() {
            // Phase 09 step 0.7 — synthetic raw-expression passthrough (fn-ref
            // dispatch); emit the string verbatim.
            ValueType::RawExpr => return write!(w, "{}", node.text()),
            ValueType::Text => {
                // Debug format → a properly escaped Rust string literal, always a
                // `&str`.  Converting it to an owned `String` is NOT decided here: a
                // literal does not know whether it is the tuple element itself or some
                // sub-expression of one, and it used to convert in both cases — so a
                // literal nested inside an element (a generic's deferred `a?` discharge)
                // emitted `&*("".to_string())`, a borrow of a temporary.  The position
                // that knows the slot does the conversion; see the `Tuple` and `TuplePut`
                // arms below.
                write!(w, "{:?}", node.text())?;
                return Ok(());
            }
            ValueType::Long => return write!(w, "{}_i64", node.int_value()),
            ValueType::Int => {
                if self.fn_ref_context {
                    // in fn-ref context (if-else branch), emit tuple.
                    return write!(
                        w,
                        "({}_i32 as u32, loft::keys::DbRef::NULL)",
                        node.int_value()
                    );
                } else if self.i32_literal_context {
                    // tp-number / field-index / flag-enum slot: runtime wants i32.
                    return write!(w, "{}_i32", node.int_value());
                }
                return write!(w, "{}_i64", node.int_value());
            }
            ValueType::Enum => return write!(w, "{}_u8", node.enum_pair().0),
            ValueType::Boolean => return write!(w, "{}", node.bool_value()),
            ValueType::Float => return write!(w, "{}_f64", node.float_value()),
            ValueType::Single => return write!(w, "{}_f32", node.single_value()),
            ValueType::Null => return write!(w, "()"),
            // @PLN11 G2/M4.2 — scalar arms (no Value child).
            ValueType::Line => {
                // P198 / DX-source-map: a `// loft:<file>:<line>` comment so
                // rustc errors trace back to the loft source line.
                let file = self.data.def(self.def_nr).position().file.replace('\n', "");
                // @PLN157 — remember where we are, so an operator checkpoint can name the
                // loft line it came from.  This stream is the emitter's only notion of
                // position.
                self.ckpt_cur_line = node.line_nr();
                return writeln!(w, "// loft:{file}:{}", node.line_nr());
            }
            ValueType::Break => {
                let n = node.break_nr();
                if n == 0 || self.loop_stack.is_empty() {
                    return write!(w, "break");
                }
                let idx = self.loop_stack.len().saturating_sub(n as usize + 1);
                return write!(w, "break 'l{}", self.loop_stack[idx]);
            }
            ValueType::Continue => {
                let n = node.continue_nr();
                if n == 0 || self.loop_stack.is_empty() {
                    return write!(w, "continue");
                }
                let idx = self.loop_stack.len().saturating_sub(n as usize + 1);
                return write!(w, "continue 'l{}", self.loop_stack[idx]);
            }
            // @PLN11 G2/M4.4 — single/list-child arms recurse via output_code_node.
            ValueType::Drop => return self.output_code_node(w, node.drop_inner()),
            ValueType::Insert => {
                let ops = node.insert_items();
                let n = ops.len();
                for (vnr, v) in ops.iter().enumerate() {
                    self.indent(w)?;
                    self.indent += 1;
                    self.output_code_node(w, v)?;
                    self.indent -= 1;
                    if vnr < n - 1 {
                        writeln!(w, ";")?;
                    } else {
                        writeln!(w)?;
                    }
                }
                return Ok(());
            }
            // @PLN11 G2/M4.5 — self-contained arms (no &Value-helper delegation).
            ValueType::Yield => {
                if let Some((open, close)) = self.yield_lazy_wrap.clone() {
                    // CL-9 — the SUSPEND of a lazily-lowered loop: hand the value to the
                    // enclosing single-iteration wrapper and leave it, so control reaches the
                    // consumer before the loop's next iteration runs (loft#836).
                    write!(w, "{{ __y = {open}")?;
                    self.output_code_node(w, node.yield_inner())?;
                    write!(w, "{close}; __exhausted = false; break 'iter; }}")?;
                } else if let Some(msg) = self.yield_collect_refuse.clone() {
                    // The eager collector cannot carry this yield type.  Emit the value
                    // bound to `_` so nothing is cast and no rustc cascade drowns the
                    // message, then stop the build with the refusal itself (loft#1132).
                    write!(w, "{{ let _ = (")?;
                    self.output_code_node(w, node.yield_inner())?;
                    write!(w, "); compile_error!(\"{msg}\"); }}")?;
                } else if let Some(kinds) = self.yield_collect_kinds.clone() {
                    // A by-value tuple yield: push each element's `i64` image FLAT, in the
                    // slot order `yield_slot_i64` defines, so the `next_into` reader pops
                    // one stride per yield.  Every kind here is single-slot by construction
                    // (`eager_tuple_kinds` rejects a `Ref`), so slot index == element index.
                    let inner = node.yield_inner().unspan();
                    write!(w, "{{ ")?;
                    if inner.kind() == ValueType::Tuple {
                        for (elem, &kind) in inner.tuple_items().iter().zip(kinds.iter()) {
                            let mut buf: Vec<u8> = Vec::new();
                            self.output_code_node(&mut buf, elem)?;
                            let code = String::from_utf8_lossy(&buf).into_owned();
                            let img = super::coroutine::yield_slot_i64(kind, &code)
                                .expect("eager_tuple_kinds admits only single-slot kinds");
                            write!(w, "__values.push({img}); ")?;
                        }
                    }
                    write!(w, "}}")?;
                } else if self.yield_collect {
                    // Inside an eager factory: push to the collector.  The whole loop
                    // runs before the consumer reads anything, so a HANDLE to a
                    // per-iteration record would alias that record's final state
                    // (three yields of {7,17,27} summed to 81 instead of 51).  A
                    // handle yield therefore pushes a SNAPSHOT — a copy in the
                    // generator's own store, released at exhaustion or abandonment —
                    // which is the value at the yield, as the interpreter's lazy
                    // suspension hands out.  COROUTINE.md § Design: lazy loop yields.
                    if let Some(tp) = self.yield_collect_snapshot_tp {
                        write!(
                            w,
                            "__values.push(loft::codegen_runtime::coroutine_snapshot(cell, &mut __snap, ("
                        )?;
                        self.output_code_node(w, node.yield_inner())?;
                        write!(w, "), {tp}))")?;
                    } else {
                        write!(w, "__values.push((")?;
                        self.output_code_node(w, node.yield_inner())?;
                        if self.yield_collect_text {
                            write!(w, ").to_string())")?;
                        } else if self.yield_collect_dbref {
                            // A handle kind with no standalone record to copy into (a
                            // keyed collection lives inline in its parent) has no
                            // snapshot form: refuse loudly rather than alias.
                            write!(w, "))")?;
                            write!(
                                w,
                                "; compile_error!(\"loft --native: a keyed collection yielded \
                                 from a generator's LOOP body has no snapshot form — yield its \
                                 element records instead, or run under --interpret\")"
                            )?;
                        } else {
                            write!(w, ") as i64)")?;
                        }
                    }
                } else {
                    write!(w, "yield ")?;
                    self.output_code_node(w, node.yield_inner())?;
                }
                return Ok(());
            }
            // C39/C47: FnRef emits a (u32, DbRef) tuple — closure var's DbRef, or
            // a null sentinel when non-capturing.
            ValueType::FnRef => {
                let clos_var = node.fnref_clos_var();
                let clos_name = if clos_var == u16::MAX {
                    None
                } else {
                    let variables = self.data.def(self.def_nr).variables();
                    Some(sanitize(variables.name(clos_var)))
                };
                let d_nr = node.fnref_dnr();
                if let Some(name) = clos_name {
                    return write!(w, "({d_nr}_u32, var_{name})");
                }
                return write!(w, "({d_nr}_u32, loft::keys::DbRef::NULL)");
            }
            ValueType::Parallel => {
                // One Rust closure per arm, handed to `n_parallel_block_native`, which
                // runs each against its own read-only clone of the parent's stores —
                // the same isolation the interpreter's `run_parallel_block` gives them.
                //
                // The closures capture enclosing locals by SHARED reference, which is
                // exactly the surface the language allows: writing an enclosing local,
                // mutating one through a reference, and capturing a parameter are all
                // compile errors already (`170-parallel-capture-soundness.loft`), so
                // `Fn + Send + Sync` asks for nothing the parser has not enforced.
                //
                // An arm's OWN locals are declared where they are first set, so they land
                // inside the closure and stay private to their arm, as they must.
                //
                // Before loft#1054 this arm emitted a comment: the block compiled to
                // NOTHING on the default backend, the arms never ran, and the program
                // exited 0 having done none of the work it asked for.
                let arms = node.parallel_arms();
                // EVERY arm is a top-level expression of the block, so `sum = 0;` and the
                // loop that reads `sum` are two DIFFERENT arms — and the second one has to
                // compile.  The interpreter gets that for free: each arm runs against a
                // SNAPSHOT of the parent's stack, so every arm-local has a slot in the
                // parent's frame and each arm sees its own copy of it, at the value the
                // frame held on entry.
                //
                // Reproduced here by declaring every variable the block's arms assign at
                // the top of every closure, at its type's default.  An arm that assigns one
                // shadows the declaration with its own; an arm that only READS one gets the
                // entry value, which is what the snapshot would have given it.  Private per
                // closure, so no arm can observe another's writes — the isolation the
                // construct promises.
                let mut arm_vars: Vec<u16> = Vec::new();
                for arm in arms.iter() {
                    Self::collect_assigned_vars(arm, &mut arm_vars);
                }
                let variables = self.data.def(self.def_nr).variables();
                let mut preamble = String::new();
                for &v in &arm_vars {
                    let tp = variables.tp(v);
                    // A `&`-link local holds a raw pointer with no meaningful default, so
                    // there is nothing honest to pre-declare it as.  Left out: an arm that
                    // reads a SIBLING arm's link then fails to compile, loudly, instead of
                    // dereferencing a made-up address.
                    if matches!(tp, Type::RefVar(_)) {
                        continue;
                    }
                    let name = sanitize(variables.name(v));
                    let ty = rust_type(tp, &Context::Variable);
                    let init = default_native_value_in(tp, &Context::Variable);
                    use std::fmt::Write as _;
                    let _ = write!(preamble, "let mut var_{name}: {ty} = {init}; ");
                }
                write!(w, "n_parallel_block_native(cell, &[")?;
                for arm in arms.iter() {
                    write!(
                        w,
                        "&|cell: &std::cell::UnsafeCell<Stores>| {{ \
                         let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; \
                         let _ = &stores; {preamble}"
                    )?;
                    self.output_code_node(w, arm)?;
                    write!(w, "; }},")?;
                }
                return write!(w, "])");
            }
            ValueType::FnRefDnr => {
                // P215: project the d_nr from a fn-ref var's (u32, DbRef) tuple.
                let var_name = sanitize(
                    self.data
                        .def(self.def_nr)
                        .variables
                        .name(node.fnref_dnr_var()),
                );
                return write!(w, "(var_{var_name}.0 as i64)");
            }
            // Plan-07 — Span is transparent in native emit.
            ValueType::Span => return self.output_code_node(w, node.span_inner()),
            ValueType::Iter => return write!(w, "{:?}", node.as_native()),
            // @PLN11 G2/M4.6 — Var + tuple arms (self-contained; `infer_type`
            // keeps a native bridge as it is a &Value-only predicate).
            ValueType::Var => {
                let var = node.var_nr();
                let variables = self.data.def(self.def_nr).variables();
                let var_name = sanitize(variables.name(var));
                // loft#1354 — this read HANDS the local to an `if` that binds it, and a
                // tuple carrying text is `(…, String, …)`: not `Copy`, so a bare read moves
                // it and any later use of the local is E0382 on a program the interpreter
                // runs.  Only the variable the arm was entered for is cloned.
                if self.clone_handed_tuple_local == Some(var) {
                    self.clone_handed_tuple_local = None;
                    return write!(w, "var_{var_name}.clone()");
                }
                if let Some(field) = self.coroutine_persistent_fields.get(&var) {
                    // P224: read from the coroutine struct field, under the name the struct
                    // definition gave it — which is the variable's own only where no other
                    // field already claimed that spelling (loft#928).
                    if matches!(variables.tp(var), Type::Text(_)) {
                        return write!(w, "&self.var_{field}");
                    }
                    return write!(w, "self.var_{field}");
                } else if variables.is_argument(var) {
                    if let Type::RefVar(inner) = variables.tp(var) {
                        // By-ref argument: holds &mut T — dereference to read.
                        if matches!(**inner, Type::Text(_)) {
                            return write!(w, "&*var_{var_name}");
                        }
                        // A `&boolean` slot is the tri-state STORAGE byte (0/1/255),
                        // because locals are `u8`; every loft-level USE of it is a
                        // two-state `bool`, exactly as a plain `boolean` parameter
                        // already is.  Convert on read so consumers see the type they
                        // expect (`if b`, `b && …`) rather than a `u8` (loft#655).
                        // `== 1` deliberately: a null (255) reads as false.
                        if matches!(**inner, Type::Boolean) {
                            return write!(w, "(*var_{var_name} == 1)");
                        }
                        return write!(w, "*var_{var_name}");
                    }
                    return write!(w, "var_{var_name}");
                } else if let Type::RefVar(inner) = variables.tp(var)
                    && matches!(
                        inner.base(),
                        Type::Integer(..)
                            | Type::Float
                            | Type::Single
                            | Type::Boolean
                            | Type::Character
                            | Type::Text(_)
                            // loft#1455 — a local `&fn(…)` link holds `*mut (u32, DbRef)` and
                            // reads as the PAIR behind it.  Left out, the pointer itself was
                            // handed to a `fn(…)` parameter — rustc E0308, `expected
                            // (u32, DbRef), found *mut (u32, DbRef)`.
                            | Type::Function(_, _, _)
                    )
                {
                    // @PLN87 L1 — a local scalar `&`-link holds `*mut T` (raw); deref
                    // to read the linked source's current value.  loft#1371 — a local
                    // `&text` link holds `*mut String` and reads as a borrow of it, the
                    // same shape the `&text` PARAMETER reads through.
                    // loft#1372 — the read side asks the SLOT too.
                    if matches!(inner.base(), Type::Boolean) {
                        return write!(w, "unsafe {{ *var_{var_name} == 1 }}");
                    }
                    if matches!(inner.base(), Type::Text(_)) {
                        return write!(w, "unsafe {{ &*var_{var_name} }}");
                    }
                    return write!(w, "unsafe {{ *var_{var_name} }}");
                } else if let Type::RefVar(inner) = variables.tp(var)
                    && matches!(inner.base(), Type::Reference(..))
                    && self.local_record_link.contains(&var)
                {
                    // loft#1371 — a local `&struct` link built from `OpCreateStack` holds
                    // `*mut DbRef`; deref to read the source slot's CURRENT record.  Only
                    // the links this function actually built that way: a `RefVar` local
                    // that aliases another `RefVar` (the #257 shape) still holds the
                    // pointer it copied.
                    return write!(w, "unsafe {{ *var_{var_name} }}");
                } else if matches!(variables.tp(var).base(), Type::Text(_))
                    && !self.tuple_text_to_string
                {
                    // Text locals are `String` — `&` coerces to `&str`.  Inside a
                    // (String, …) tuple-return literal (@P330) emit the bare name.
                    // @PLN25 — `.base()` peels a `text?` local so an indexed/sliced
                    // nullable text receiver (`raw: text?; raw[i]`, `raw[a..b]`) is
                    // borrowed to `&str` too; inert gate-OFF (no `Optional` exists).
                    return write!(w, "&var_{var_name}");
                }
                return write!(w, "var_{var_name}");
            }
            ValueType::Tuple => {
                write!(w, "(")?;
                let elems = node.tuple_items();
                // loft#1069 — TAKEN, so a nested tuple element does not read its parent's
                // positions; the destination re-sets it per assignment.
                let slots = std::mem::take(&mut self.tuple_slot_types);
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    // loft#1069 — a fn-ref member whose slot is `(u32, DbRef)` but whose
                    // VALUE is the bare d_nr (a plain `dbl`, or a non-capturing inline
                    // lambda) has to be built into the pair here.  Only the destination
                    // knows: the d_nr infers as an `integer`, which is why the element
                    // could not be asked.  A CAPTURING source already emits the pair (its
                    // closure block's type IS the function), so it is left alone — that
                    // case worked, and it is the shape being matched.
                    let needs_fn_pair =
                        matches!(slots.get(i).map(Type::base), Some(Type::Function(_, _, _)))
                            && !matches!(self.infer_type(e), Some(Type::Function(_, _, _)));
                    if needs_fn_pair {
                        write!(w, "((")?;
                    }
                    // A NESTED tuple member gets its own declared element types handed
                    // down, so the inner arm can ask for its own fn-ref pairs.  Without
                    // this the `take` above left the inner arm with an empty slot list and
                    // `needs_fn_pair` false, so `((dbl, 1), "z")` emitted
                    // `((710_i64, 1_i64), …)` against `(((u32, DbRef), i64), String)` and
                    // rustc refused the program — depth being the axis loft#1069's fix
                    // held fixed.  Only a member that CARRIES a fn-ref seeds anything, so
                    // an ordinary nested tuple keeps taking the path it always did.
                    if let Some(Type::Tuple(inner)) = slots.get(i).map(Type::base)
                        && inner.iter().any(crate::data::tuple_carries_fn_ref)
                    {
                        self.tuple_slot_types = inner.clone();
                    }
                    let elem_is_text = matches!(self.infer_type(e), Some(Type::Text(_)));
                    // @PLN17: a boolean tuple element is stored u8 (slot type);
                    // wrap `((elem) as u8)` so a `bool` sub-expression fits the slot.
                    let elem_is_bool = matches!(self.infer_type(e), Some(Type::Boolean));
                    if elem_is_bool {
                        write!(w, "((")?;
                    }
                    // A member the scope pass rewrote as `Insert([statements…, value])` — a
                    // heap local's declaration hoisted to the enclosing statement ahead of the
                    // block that binds it — is a statement list with a value tail, which
                    // inside a tuple is legal Rust only as a block expression.
                    let as_block = e.unspan().kind() == ValueType::Insert;
                    if as_block {
                        write!(w, "{{ ")?;
                    }
                    self.output_code_node(w, e)?;
                    if as_block {
                        write!(w, " }}")?;
                    }
                    if elem_is_bool {
                        write!(w, ") as u8)")?;
                    }
                    // Wrap a text element with `.to_string()` so it fits a
                    // `String`-typed tuple slot.  This is the ONE place an element is
                    // converted — a Text literal included, which is what lets the `Text`
                    // arm stay a plain `&str` and stop converting sub-expressions that
                    // merely pass through it.
                    if elem_is_text && self.tuple_text_to_string {
                        write!(w, ".to_string()")?;
                    }
                    if needs_fn_pair {
                        write!(w, ") as u32, DbRef::NULL)")?;
                    }
                }
                write!(w, ")")?;
                self.tuple_slot_types = slots;
                return Ok(());
            }
            ValueType::TupleGet => {
                // N8a.2: use the variable's declared name (not its number).
                let var = node.tupleget_var();
                let idx = node.tupleget_idx();
                let variables = self.data.def(self.def_nr).variables();
                let (name, deref) = crate::generation::tuple_base(variables, var);
                if deref {
                    // A `&`-bound tuple LOCAL: the element sits behind a raw pointer.
                    // `T-Ref-El` admits only scalars here, so none of the text/borrow
                    // spellings below can apply to this base.
                    return write!(w, "unsafe {{ {name}.{idx} }}");
                }
                // loft#1038 — one derivation for what the slot holds, shared with the
                // WRITE arm and with `set_var`'s clone rule (`tuple_elem_is_text`).
                let elem_is_text =
                    crate::generation::tuple_elem_is_text(variables, var, u32::from(idx));
                let is_arg = variables.is_argument(var);
                // P247 — a work-ref (`__ref_…`) tuple-text read must `.clone()`
                // (returns owned String) instead of borrowing, else the borrow
                // escapes the enclosing block (E0597).
                let is_work_ref = variables.name(var).starts_with("__ref_");
                if elem_is_text && !is_arg {
                    if is_work_ref {
                        return write!(w, "{name}.{idx}.clone()");
                    }
                    if self.tuple_text_to_string {
                        // Feeding a `String`-typed tuple slot, and the caller appends
                        // `.to_string()`.  Borrowing first makes that `&(x.to_string())`,
                        // which rustc reads as `&String` where a `String` is wanted —
                        // `a = ("X", a.1)` refused the whole program (loft#1004).  Same
                        // guard the plain-variable arm above already carries.
                        return write!(w, "{name}.{idx}");
                    }
                    return write!(w, "&{name}.{idx}");
                }
                return write!(w, "{name}.{idx}");
            }
            ValueType::TuplePut => {
                // N8a.2: emit the element assignment (TuplePut is a void stmt).
                let var = node.tupleput_var();
                let idx = node.tupleput_idx();
                let variables = self.data.def(self.def_nr).variables();
                let (name, deref) = crate::generation::tuple_base(variables, var);
                // A text element is a `String` slot, so the value has to arrive owned —
                // the same rule the tuple LITERAL arm below obeys through this flag.
                // Without it `t.0 = "X"` emitted `var_t.0 = "X";` and rustc refused the
                // whole program with *"expected `String`, found `&str`"* (loft#1004).
                // Both tuple spellings reach here — a by-value local and a `&(…)`
                // reference-tuple parameter — which is why the element type comes from
                // the shared `tuple_elem_type` (reading only the by-value spelling is
                // why the boolean cast below silently did not fire on the reference
                // form, loft#1006).
                let elem_tp = crate::generation::tuple_elem_type(variables, var, u32::from(idx));
                let elem_is_text = elem_tp
                    .as_ref()
                    .is_some_and(|e| matches!(e.base(), Type::Text(_)));
                // A boolean element's SLOT is `u8` (0/1/255) while a transient expression
                // is `bool`, so a tuple-element write is one of the store positions
                // `boolean_u8_cast` exists for — and was the one missing from that list.
                // `var_p.1 = true;` on a `&(integer, boolean)` was rustc E0308 *"expected
                // `u8`, found `bool`"*, while `&(boolean, boolean)` compiled because both
                // sides of its swap were already `u8` (loft#1006).  The wrap is idempotent
                // for `u8`, so it is right for both.
                let bool_cast = elem_tp
                    .as_ref()
                    .and_then(|e| crate::generation::boolean_u8_cast(e));
                // The fn-ref half of the same rule: the slot is the `(u32, DbRef)` PAIR,
                // while a bare `inc` carries only the d_nr and emits as an `i64`.  Only
                // the destination names the element a `fn(…)`, so the pair is asked for
                // here — through `fn_ref_context`, the same channel the `Set` arm in
                // dispatch.rs opens for a fn-ref VARIABLE, whose `Int` arm builds the
                // whole pair.  Going through the shared flag rather than wrapping the
                // emitted text is what makes an if-VALUED source come out right: the pair
                // is then built inside EACH BRANCH, where a wrap placed around the whole
                // `if` would leave both arms bare `i64` and rustc refusing the program.
                // Without it `t.0 = inc` emitted `var_t.0 = 711_i64;` — E0308 *"expected
                // `(u32, DbRef)`, found `i64`"*.  A CAPTURING source emits a block that is
                // already the pair, and a `Var` source is already one, so neither is a
                // bare `Int` and neither is touched.
                let elem_is_fn = elem_tp
                    .as_ref()
                    .is_some_and(|e| matches!(e.base(), Type::Function(_, _, _)));
                // The write through a `&`-bound tuple local derefs a raw pointer, so the
                // whole assignment — not just the base — has to sit inside the block.
                if deref {
                    write!(w, "unsafe {{ ")?;
                }
                write!(w, "{name}.{idx} = ")?;
                if bool_cast.is_some() {
                    write!(w, "(")?;
                }
                let prev = self.tuple_text_to_string;
                let prev_fn_ref_ctx = self.fn_ref_context;
                self.tuple_text_to_string = elem_is_text;
                if elem_is_fn {
                    self.fn_ref_context = true;
                }
                let r = self.output_code_node(w, node.tupleput_inner());
                self.fn_ref_context = prev_fn_ref_ctx;
                self.tuple_text_to_string = prev;
                // The same conversion the `Tuple` element loop applies, for the same
                // reason: this position knows the slot is an owned `String` and the
                // value emitted into it does not.  A `TupleGet`/`Var` source naming an
                // owned text LOCAL already emitted un-borrowed (the flag above), so for
                // that one this would only add a copy.
                //
                // A text ARGUMENT is the exception, and the carve-out did not know it: a
                // text parameter arrives BORROWED (`&str`), so the same bare spelling is a
                // type error at an owned slot — `p.1 = s` on
                // `fn f(p: (integer, text), s: text)` was rustc E0308 *"expected `String`,
                // found `&str`"* while the literal `p.1 = "x"` beside it compiled
                // (loft#1278).  The tuple LITERAL arm has always converted a `Var` here;
                // only this arm carved it out, so the two spellings of the same write
                // disagreed about the same source.
                let borrowed_var_src = matches!(node.tupleput_inner().kind(), ValueType::Var) && {
                    let v = node.tupleput_inner().var_nr();
                    let vars = self.data.def(self.def_nr).variables();
                    vars.is_argument(v) && matches!(vars.tp(v).base(), Type::Text(_))
                };
                if elem_is_text
                    && (!matches!(node.tupleput_inner().kind(), ValueType::Var) || borrowed_var_src)
                {
                    write!(w, ".to_string()")?;
                }
                if let Some(cast) = bool_cast {
                    write!(w, ") as {cast}")?;
                }
                if deref {
                    write!(w, " }}")?;
                }
                return r;
            }
            _ => {}
        }
        // @PLN11 G2/M4 — materialise-at-boundary for the residual arms
        // (Block/Loop/Set/If/Call/Return/Keys/CallRef), which delegate to large
        // `&Value`/`&Block` helpers (output_block/output_if/output_set/…).
        // Native backing is zero-cost (`code = v`); a store-backed node
        // materialises once here so output_code_node is fully store-capable
        // without threading the handle through that whole helper cluster.
        let owned_code;
        let code = match node {
            IrNode::Native(v) => v,
            IrNode::Store(..) => {
                owned_code = node.to_owned_value();
                &owned_code
            }
        };
        match code {
            Value::Block(bl) => self.output_block(w, IrBlock::Native(bl), false, false)?,
            Value::Loop(lp) => {
                let hoisted = self.begin_vector_hoist(w, lp)?;
                self.loop_stack.push(lp.scope);
                writeln!(w, "'l{}: loop {{ //{}_{}", lp.scope, lp.name, lp.scope)?;
                for v in &lp.operators {
                    self.indent(w)?;
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    writeln!(w, ";")?;
                }
                self.indent(w)?;
                write!(w, "}} /*{}_{}*/", lp.name, lp.scope)?;
                self.loop_stack.pop();
                self.end_vector_hoist(w, hoisted)?;
            }
            Value::Set(var, to) => self.output_set(w, *var, to)?,
            Value::If(test, true_v, false_v) => self.output_if(w, test, true_v, false_v)?,
            Value::Call(def_nr, vals) => {
                self.output_call(w, *def_nr, vals)?;
            }
            Value::Return(val) => {
                let returned = self.data.def(self.def_nr).returned();
                // @PLN10 Phase A — a bufferless ("nwb") user text fn has a
                // `-> String` wrapper, so every text return emits an owned
                // `String`, never a buffer-backed `Str`.  Computed once here and
                // checked first at each return shape below.
                let outer_owned = super::def_returns_owned_text(self.data.def(self.def_nr));
                if matches!(**val, Value::Null) && *returned != Type::Void {
                    if outer_owned {
                        // String null: the content sentinel as an owned String
                        // (matches the `-> String` wrapper, not `Str::new(...)`).
                        write!(w, "return loft::state::STRING_NULL.to_string()")?;
                    } else {
                        write!(w, "return {}", super::default_native_value(returned))?;
                    }
                } else if let Value::If(test, true_v, false_v) = &**val {
                    self.pre_declare_branch_vars(w, true_v, false_v)?;
                    // @PLN25 slice (c): `.base()` — a `-> text?` return is `Str`-typed just
                    // like `-> text`, so its buffer-backed return still needs the `Str::new(…)`
                    // wrap (without the peel it emitted a raw `&String` → E0308).
                    let returns_text = matches!(returned.base(), Type::Text(_));
                    let narrow = narrow_int_cast(returned);
                    let wrap_text = returns_text;
                    // @P386: when an if-branch is a Text-result Block whose
                    // body contains the `__ncc_*` skip_free pattern (the `??`
                    // value-block lowering), the block's tail produces an
                    // OWNED `String` via `_ret.to_string()`.  The outer
                    // `Str::new(<if>)` wrap then sees `Str::new(String)` which
                    // fails E0308 ("expected `&str`, found `String`").  Route
                    // through `stores.scratch` like the non-If path at
                    // lines ~234-310: convert the if-expression to a String
                    // via `.to_string()` (accepts `&str` / `String` / `Str`),
                    // push to scratch (program-lifetime), and emit `Str::new`
                    // pointing into the scratch entry.
                    let if_needs_scratch = wrap_text && {
                        let true_has_ncc = matches!(&**true_v, Value::Block(b)
                            if self.block_contains_ncc_skip_free(b));
                        let false_has_ncc = matches!(&**false_v, Value::Block(b)
                            if self.block_contains_ncc_skip_free(b));
                        // #534 — a `String`-vs-`&str` arm mismatch is unified to
                        // owned `String` by `output_if_inner`, so this buffered
                        // (`Str::new`) return must route it through the work
                        // buffer too, exactly like the ncc case.
                        true_has_ncc
                            || false_has_ncc
                            || self.text_if_mismatched_reps(true_v, false_v)
                    };
                    write!(w, "return ")?;
                    if outer_owned {
                        write!(w, "(")?;
                    } else if if_needs_scratch {
                        write!(w, "{{ let _tmp = (")?;
                    } else if wrap_text {
                        write!(w, "Str::new(")?;
                    } else if narrow.is_some() {
                        write!(w, "(")?;
                    }
                    // #263: see the non-If path below — a fn-ref return
                    // whose branches are bare d_nrs must emit the
                    // `(u32, DbRef)` tuple per branch.
                    let prev_fn_ref_ctx = self.fn_ref_context;
                    if matches!(returned, Type::Function(_, _, _)) {
                        self.fn_ref_context = true;
                    }
                    self.output_if_inner(w, test, true_v, false_v, true)?;
                    self.fn_ref_context = prev_fn_ref_ctx;
                    if outer_owned {
                        write!(w, ").to_string()")?;
                    } else if if_needs_scratch {
                        // @PLN10 Phase B — write the materialised String into the
                        // caller-owned work buffer (`!nwb` here ⇒ a buffer exists),
                        // not the never-cleared `stores.scratch`.
                        if let Some(buf) = self.return_buffer_name() {
                            write!(
                                w,
                                ").to_string(); *var_{buf} = _tmp; Str::new(&*var_{buf}) }}"
                            )?;
                        } else {
                            // @PLN10 D/G1 — dead branch: `needs_p205_scratch` here
                            // means a non-nwb text return (nwb is handled by the
                            // `outer_owned` arm above), and a non-nwb text fn ALWAYS
                            // has a `RefVar(Text)` work buffer, so `return_buffer_name`
                            // is never `None`.  Panic loudly rather than emit
                            // `stores.scratch` into generated code (the field is being
                            // retired).  Whole-suite `=panic` = zero proves this is
                            // unreached.
                            unreachable!(
                                "non-nwb text return without a work buffer \
                                 (return_buffer_name() == None) — @PLN10 invariant violated"
                            );
                        }
                    } else if wrap_text {
                        write!(w, ")")?;
                    } else if let Some(cast) = narrow {
                        write!(w, ") as {cast}")?;
                    }
                } else {
                    // @PLN25 slice (c): `.base()` — a `-> text?` return is `Str`-typed just
                    // like `-> text`, so its buffer-backed return still needs the `Str::new(…)`
                    // wrap (without the peel it emitted a raw `&String` → E0308).
                    let returns_text = matches!(returned.base(), Type::Text(_));
                    let narrow = narrow_int_cast(returned);
                    // #433 — widen a narrow-int value-block returned from a plain
                    // `integer` (i64) function back to i64 (see block_needs_i64_widen).
                    let widen_block = block_needs_i64_widen(val, returned);
                    // A direct `return helper()` where `helper` is itself a
                    // BUFFERED user text fn already yields a `Str` — no re-wrap.
                    // @PLN10 Phase A: EXCLUDE nwb inner fns (they now return an
                    // owned `String`, not `Str`); a `return nwb_helper()` must be
                    // re-wrapped to this fn's return type instead (owned→owned for
                    // an nwb outer, owned→buffer/scratch for a buffered outer).
                    // `.unspan()` on both call-shape probes: a TAIL-expression return
                    // (scopes' free_vars wraps the block tail, Span and all) reaches
                    // here as `Span(Call(..))`, which a bare `Value::Call` match
                    // misses — the nwb inner then skipped the scratch/buffer route
                    // and emitted `Str::new(<String>)` (E0308; the routing `jtext`
                    // tail-call bug, feedback 2026-07-02).  parse_return-created
                    // Returns carry the call bare, which is why an explicit
                    // `return helper();` never reproduced it.
                    let inner_already_str = matches!(
                        (**val).unspan(),
                        Value::Call(d, _) if (*d as usize) < self.data.definitions.len()
                            && matches!(self.data.def(*d).returned(), Type::Text(_))
                            && self.data.def(*d).rust().is_empty()
                            && self.data.def(*d).native().is_empty()
                            && !self.data.def(*d).name().starts_with("Op")
                            && !super::def_returns_owned_text(self.data.def(*d))
                    );
                    // @PLN10 Phase A — `return nwb_helper()` in a BUFFERED outer:
                    // the inner produces an owned `String` but this fn's wrapper is
                    // `-> Str`, so route the String through the program-lifetime
                    // backing (scratch here; buffer-write is the follow-up) and
                    // hand back a `Str` pointing into it.  `Str::new(String)` would
                    // fail E0308.
                    let inner_is_nwb_call = matches!(
                        (**val).unspan(),
                        Value::Call(d, _) if (*d as usize) < self.data.definitions.len()
                            && super::def_returns_owned_text(self.data.def(*d))
                    );
                    let wrap_text = returns_text && !inner_already_str;
                    // T1.8a's tuple-of-text return path was retired by
                    // Plan-14 phase 07 (P234 runtime closure).  Function
                    // returns of `Type::Tuple(elems)` with any
                    // lifetime-bearing element (Text, Reference, etc.)
                    // are now rewritten in
                    // `src/parser/definitions.rs::parse_function` to
                    // `Type::Reference(__tuple<…>)` and the body's tail
                    // tuple literal becomes a synthetic-struct
                    // construction sequence.  So `Value::Return(Tuple)`
                    // with text elements is unreachable from any
                    // user-written tuple-of-text return — the
                    // `tuple_text_to_string` save/set/restore that lived
                    // here became dead code and was removed.
                    // (`output_set`'s analogous handling at
                    // dispatch.rs:295-359 stays — it serves LOCAL
                    // tuple-with-text variables, which the rewrite does
                    // not touch.)
                    // P205 (plan-09 phase 07): if the function returns
                    // Type::Text but has no `Type::RefVar(Type::Text(_))`
                    // attribute (no proper work buffer set up by
                    // `text_return`), `Str::new(<local_String>)` would
                    // dangle.  Route through `stores.scratch` instead so
                    // the value's backing String lives as long as `stores`.
                    // A work buffer is a HIDDEN `RefVar(Text)` attribute — every one
                    // `text_return` promotes is marked so; a user `&text` parameter is
                    // not a buffer (`return_buffer_name`, loft#1338).
                    let needs_p205_scratch = wrap_text && {
                        let def = self.data.def(self.def_nr);
                        let no_work_buffer = def.text_work_buffers() == 0;
                        // @P321e — also route through scratch when the return
                        // value is a text LOCAL var (an owned `String`, not the
                        // RefVar work-buffer arg).  `Str::new(&var_local)`
                        // borrows a fn-local that drops at return → dangling ptr.
                        // Happens when a text fn's body is a match whose result
                        // is `.to_string()`'d into a `__ret_N` local and returned
                        // (`edit_kind_label`): a work-buffer arg exists but the
                        // fn returns a DIFFERENT local, so the `no_work_buffer`
                        // guard above doesn't catch it.
                        //
                        // `.base()` peels nullability for the same reason
                        // `returns_text` above does: a `text?` local is just as
                        // much a fn-local `String` as a `text` one, and matching
                        // the un-peeled `Type::Optional(Text)` missed it — a
                        // `-> text?` fn with two `return <call>` statements gave
                        // every non-final return a `__ret_N` local and handed
                        // back `Str::new(&var___ret_N)` into a dropped String
                        // (loft#740).  Non-null `text` was unaffected, which is
                        // why it stayed hidden.
                        let returns_local_text = matches!((**val).unspan(), Value::Var(v)
                            if matches!(def.variables().tp(*v).base(), Type::Text(_))
                                && !def.variables().is_argument(*v));
                        // @PLAN52 cluster VI (2026-05-30): closures returning text
                        // have a `__work_ret: &mut String` parameter but the
                        // closure body's `??` value-block doesn't write into it —
                        // instead emits `return Str::new(<value-block>)`.  Inner
                        // block tail materialises a String via cluster I's
                        // `_ret.to_string()` machinery.  Plain `Str::new(String)`
                        // fails E0308.  Detect the inner `__ncc_*` skip_free
                        // pattern and route through scratch so the materialised
                        // String lives in `stores.scratch` and `Str::new(&str)`
                        // reads from there.
                        let returns_ncc_block = matches!((**val).unspan(), Value::Block(b)
                            if self.block_contains_ncc_skip_free(b));
                        // #557 — a text Block whose tail `output_block` materialises as an owned
                        // `String` (a value result followed by a trailing `OpFreeText`, e.g. a
                        // `vector<text>` match freeing its bound element).  `Str::new(String)`
                        // would fail E0308, so route it through the buffer like the ncc case.
                        let returns_materialised_block = matches!((**val).unspan(), Value::Block(b)
                            if self.block_tail_materialises_string(b));
                        no_work_buffer
                            || returns_local_text
                            || returns_ncc_block
                            || returns_materialised_block
                            || inner_is_nwb_call
                    };
                    write!(w, "return ")?;
                    if outer_owned {
                        // @PLN10 Phase A — nwb fn: emit an owned `String`
                        // (`(val).to_string()` coerces &str / String / Str / a
                        // buffered-inner `Str` / an nwb-inner `String` alike).
                        write!(w, "(")?;
                    } else if needs_p205_scratch {
                        // Plan-07 phase 4 — pre-bind the body's text
                        // result into a local before calling
                        // `stores.scratch.push(...)` so the inner
                        // expression's mutable borrow of `stores`
                        // (e.g. `stores.vec_ref_or_raise_runtime(...)`,
                        // `stores.text_char_or_raise_runtime(...)`,
                        // and any other `stores.X` call introduced
                        // by the C66 production-mode helpers) doesn't
                        // overlap with the `stores.scratch.push(...)`
                        // borrow.  Without the pre-bind, rustc
                        // rejects with E0499 (`cannot borrow *stores
                        // as mutable more than once at a time`).
                        write!(w, "{{ let _tmp = (")?;
                    } else if wrap_text {
                        write!(w, "Str::new(")?;
                    } else if narrow.is_some() || widen_block {
                        write!(w, "(")?;
                    }
                    // P238: when the function's return type is a tuple
                    // with text element(s) and the body's return value is
                    // a `Value::Tuple` literal, set `tuple_text_to_string`
                    // so each text element gets a `.to_string()` wrap to
                    // fit the `(String, …)` slot.  The parser's
                    // tuple-of-text → synthetic-struct rewrite (Plan-14
                    // phase 07 / P234) does NOT fire for generic
                    // monomorphisations: the source fn was parsed with T
                    // as a generic struct, so the return type at parse
                    // time was `(T, T)` with no Text elements; the
                    // rewrite trigger missed.  At monomorphisation time
                    // the type becomes `(String, String)` but the body
                    // is still a plain `Value::Tuple`.
                    // The shared predicate, not a fourth spelling of it: this site had
                    // its own inline `any(Text)`, which saw neither a nested tuple nor a
                    // `text?` element.
                    let returns_text_tuple = matches!(returned, Type::Tuple(elems)
                        if crate::generation::dispatch::tuple_has_text_leaf(elems));
                    let prev_tuple_text = self.tuple_text_to_string;
                    if returns_text_tuple && matches!(&**val, Value::Tuple(_)) {
                        self.tuple_text_to_string = true;
                    }
                    // #263: a fn-ref returned as a bare d_nr (e.g.
                    // `return dbl` → `Value::Int(d_nr)`) must emit the
                    // full `(u32, DbRef)` fn-ref tuple, not a bare
                    // `i64`, or rustc rejects with E0308 ("expected
                    // `(u32, DbRef)`, found `i64`").  Set fn_ref_context
                    // so the Int arm (emit.rs ~53) emits the tuple with a
                    // null-sentinel closure half — the native mirror of
                    // the interpreter's gen_return sentinel padding.
                    let prev_fn_ref_ctx = self.fn_ref_context;
                    if matches!(returned, Type::Function(_, _, _)) {
                        self.fn_ref_context = true;
                    }
                    // @PLN85 — a `Block`/`Insert` return value (e.g. a `match` tail
                    // whose arms declare their own buffers) emits `let …; let …;
                    // <expr>` statements; `return let …` is invalid Rust and the
                    // block-local vars are out of scope at the tail (E0425). Wrap
                    // it as `return { … }` so the block is a value expression with
                    // its locals correctly scoped.
                    let block_braces = matches!(val.unspan(), Value::Block(_) | Value::Insert(_));
                    if block_braces {
                        write!(w, "{{ ")?;
                    }
                    self.output_code_inner(w, val)?;
                    if block_braces {
                        write!(w, " }}")?;
                    }
                    self.fn_ref_context = prev_fn_ref_ctx;
                    self.tuple_text_to_string = prev_tuple_text;
                    if outer_owned {
                        write!(w, ").to_string()")?;
                    } else if needs_p205_scratch {
                        // @PLN10 Phase B — buffer-write (see If-Return path).
                        if let Some(buf) = self.return_buffer_name() {
                            write!(
                                w,
                                ").to_string(); *var_{buf} = _tmp; Str::new(&*var_{buf}) }}"
                            )?;
                        } else {
                            // @PLN10 D/G1 — dead branch: `needs_p205_scratch` here
                            // means a non-nwb text return (nwb is handled by the
                            // `outer_owned` arm above), and a non-nwb text fn ALWAYS
                            // has a `RefVar(Text)` work buffer, so `return_buffer_name`
                            // is never `None`.  Panic loudly rather than emit
                            // `stores.scratch` into generated code (the field is being
                            // retired).  Whole-suite `=panic` = zero proves this is
                            // unreached.
                            unreachable!(
                                "non-nwb text return without a work buffer \
                                 (return_buffer_name() == None) — @PLN10 invariant violated"
                            );
                        }
                    } else if wrap_text {
                        write!(w, ")")?;
                    } else if let Some(cast) = narrow {
                        write!(w, ") as {cast}")?;
                    } else if widen_block {
                        write!(w, ") as i64")?;
                    }
                }
            }
            Value::Keys(keys) => {
                write!(w, "&[")?;
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    write!(
                        w,
                        "Key {{ type_nr: {}, position: {}, start: {} }}",
                        k.type_nr, k.position, k.start
                    )?;
                }
                write!(w, "]")?;
            }
            Value::CallRef(v_nr, args) => {
                self.output_call_ref(w, *v_nr, args)?;
            }
            // @PLN11 G2/M4.1–M4.6 — these kinds are handled by the
            // `match node.kind()` above, which `return`s before reaching here.
            Value::Var(_)
            | Value::Tuple(_)
            | Value::TupleGet(_, _)
            | Value::TuplePut(_, _, _)
            | Value::RawExpr(_)
            | Value::Text(_)
            | Value::Long(_)
            | Value::Int(_)
            | Value::Enum(_, _)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Null
            | Value::Line(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Drop(_)
            | Value::Insert(_)
            | Value::Yield(_)
            | Value::FnRef(_, _, _)
            | Value::Parallel(_)
            | Value::FnRefDnr(_)
            | Value::Span(_)
            | Value::Iter(_, _, _, _) => {
                unreachable!("M4.1–M4.5-converted kind reached legacy match: {code:?}")
            }
        }
        Ok(())
    }

    /// Emit a call through a fn-ref variable (`Value::CallRef`).
    /// The variable `v_nr` holds a `u32` definition number at runtime.
    /// We enumerate all reachable definitions with a matching signature and
    /// generate a `match` dispatch.
    fn output_call_ref(
        &mut self,
        w: &mut dyn Write,
        v_nr: u16,
        args: &[Value],
    ) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        let var_name = sanitize(variables.name(v_nr));
        let fn_type = variables.tp(v_nr).clone();
        let (param_types, ret_type) = if let Type::Function(p, r, _) = &fn_type {
            (p.clone(), *r.clone())
        } else {
            // Not a function type — fall back to debug print.
            write!(w, "{:?}", crate::data::Value::CallRef(v_nr, args.to_vec()))?;
            return Ok(());
        };
        // P227: a text-returning fn-ref call site appends the `&text` work buffers the
        // widest candidate of this signature could want, which is one OR MORE
        // (`Data::fnref_text_buffers`, loft#1116).  So the candidate filter reads the
        // user-visible count off the fn-ref TYPE rather than subtracting a fixed one from
        // `args.len()` — a count that was right only while every call appended exactly
        // one, and that silently matched NO candidate once a call appended two (the arm
        // collapsed to `_ => unreachable!()` and rustc answered E0282 rather than naming
        // anything about loft).
        //
        // Through `base()`, because the `?` does not change how a text return is DELIVERED:
        // `Parser::text_return` peels `Optional` before it converts the body, so a `-> text?`
        // call site appends exactly the same hidden buffers.  Reading the raw type here asked
        // for a user-visible arity that counted them, so a `-> text?` lambda matched NO
        // candidate and the dispatch collapsed to `_ => unreachable!()` — which is the very
        // failure the paragraph above records, one wrapper out.  One question, one spelling:
        // `is_text_return` below is the same read and both must move together.
        let user_arg_match =
            if matches!(ret_type.base(), Type::Text(_)) && args.len() > param_types.len() {
                param_types.len()
            } else {
                args.len()
            };
        // Collect all definitions with a matching signature.
        // Only include native-callable functions (n_ / t_ prefix) in the reachable set;
        // bytecode ops (Op* prefix) are never callable via fn-refs in native mode.
        let n_defs = self.data.definitions();
        // (d_nr, fn_name, has_closure): has_closure=true when the last attribute is __closure.
        let mut candidates: Vec<(u32, String, bool)> = Vec::new();
        for d in 0..n_defs {
            if !self.reachable.is_empty() && !self.reachable.contains(&d) {
                continue;
            }
            let def = self.data.def(d);
            if !matches!(def.def_type(), crate::data::DefType::Function) {
                continue;
            }
            // Exclude bytecode ops (Op* prefix) — they are not callable in native mode.
            if def.name().starts_with("Op") {
                continue;
            }
            // closure-capturing lambdas have a hidden __closure param as the last
            // attribute. The closure is injected explicitly at the call site (in arg_exprs),
            // so total arg count must equal the full attribute count.
            let has_closure = def
                .attributes()
                .last()
                .is_some_and(|a| a.name == "__closure");
            // P227: hidden-attribute detection is TYPE-based, not name-based.
            // Text-return work-buffers ride as `Type::RefVar(Type::Text(_))`
            // attributes that the parser names after the user-visible variable
            // they shadow (e.g. `a` for `a = "first: {n}"; a`) — a name-prefix
            // check (`starts_with("__")`) would miss these and reject otherwise
            // matching candidates.  Closure records remain detected by the
            // exact `__closure` name (its typedef is plain `DbRef`).
            let visible_attrs: Vec<&crate::data::Attribute> = def
                .attributes
                .iter()
                .filter(|a| {
                    // PLAN51 V-c: `ref_return` (src/parser/control.rs:3203)
                    // appends a hidden Reference/Vector/struct-enum buffer
                    // arg to heap-returning user fns.  This filter must
                    // exclude that synthetic attr to keep arity matching
                    // against the call site's user-visible arg count —
                    // without it, every ref_return-promoted lambda fails
                    // the visible_attrs.len() == user_arg_match check below
                    // and the fn-ref match arm emits only `_ => unreachable!`
                    // (probes 30, 59, 62 panicked with `invalid fn-ref`).
                    !a.hidden
                        && !matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
                        && a.name != "__closure"
                })
                .collect();
            if visible_attrs.len() != user_arg_match {
                continue;
            }
            let params_match = visible_attrs
                .iter()
                .zip(param_types.iter())
                .all(|(a, expected)| {
                    rust_type(&a.typedef, &Context::Argument)
                        == rust_type(expected, &Context::Argument)
                });
            if !params_match {
                continue;
            }
            if rust_type(def.returned(), &Context::Result) != rust_type(&ret_type, &Context::Result)
            {
                continue;
            }
            candidates.push((d, def.name().to_string(), has_closure));
        }
        // Phase 09 phase 00 step 0.7 — fn-ref dispatch routes each
        // candidate arm through `output_call_user_fn` (which dispatches
        // via `emit_op`), so a custom emitter registered for any
        // candidate target is honoured even when the call comes via
        // fn-ref dispatch.
        //
        // Args evaluate exactly once across all arms (correctness for
        // side-effecting expressions, plus the original "avoid
        // double-borrow of `stores`" concern).  Hoist them into Rust
        // `let _farg_N` bindings inside a wrapping block, then pass
        // synthetic `Value::RawExpr("_farg_N")` args to each per-arm
        // call.  `output_code_inner`'s `RawExpr` arm emits the binding
        // name verbatim, so the per-arm code reads
        // `fn_name(cell, _farg_0, _farg_1, …, closure_expr)` —
        // semantically the same shape as the pre-step-0.7 direct
        // emission, just routed through emit_op.
        //
        // `output_call_user_fn` iterates over the candidate's
        // `def_fn.attributes` and emits one arg per attribute.  When
        // `has_closure`, the candidate's last attribute is the
        // synthetic `__closure` (a `DbRef`); we append a RawExpr arg
        // for the closure expression so attribute count and arg count
        // line up.
        // P227: for text-returning fn-refs, the parser appends ONE
        // work-buffer arg to `args` (a `Value::Block` evaluating to
        // `&mut String` referencing a caller-function-scope work-text
        // variable).  Split it off so candidate matching uses the
        // user-visible args only; pass the work-buffer via `_farg_<n>`
        // to any candidate whose attribute list has a `RefVar(Text)`
        // hidden attr.  Function-scope lifetime means the lambda's
        // returned `Str` borrows a buffer that lives long enough for
        // the outer assignment / format consumer to read it — no
        // block-scope buffer, no per-arm `.to_string()` clone.
        // The same read as `user_arg_match` above, and it peels for the same reason — a
        // `-> text?` return is delivered through the caller's work buffer exactly as `-> text`
        // is.  Splitting the two spellings is what let the filter reject a candidate the
        // argument list had already been built for.
        let is_text_return = matches!(ret_type.base(), Type::Text(_));
        let user_arg_count = if is_text_return && args.len() > param_types.len() {
            args.len() - 1
        } else {
            args.len()
        };
        let work_buf_idx = if is_text_return && args.len() > user_arg_count {
            Some(args.len() - 1)
        } else {
            None
        };
        // loft#1185 — a fn-ref callee that MINTS its return hands back a store nobody owns once
        // the value crosses a forwarding frame.  Snapshot the allocation counter here and ask
        // `cr_fnref_minted` afterwards; a CAPTURE's stamp predates this and is left alone.
        // Heap returns only — a text return crosses as an owned `String`.
        let heap_return = matches!(
            ret_type.base(),
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
        );
        write!(w, "{{ ")?;
        if heap_return {
            write!(w, "let __vc_seq = codegen_runtime::cr_alloc_serial(cell); ")?;
        }
        for (i, arg) in args.iter().enumerate() {
            // A `fn(…)` PARAMETER takes the whole `(u32, DbRef)` fn-ref pair, and a bare
            // fn-ref name emits the lone d_nr as an `i64`.  Bound raw, every match arm below
            // then passed that `i64` where the callee wants the pair — rustc E0308, so the
            // crate did not build at all (loft#1285).  `fn_ref_context` is the same channel
            // the tuple-element write opens for exactly this reason, and going through the
            // flag rather than wrapping the emitted text is what keeps an if-VALUED source
            // right: the pair is then built inside EACH branch.
            let is_fn_arg =
                i < param_types.len() && matches!(param_types[i].base(), Type::Function(_, _, _));
            let prev_fn_ref_ctx = self.fn_ref_context;
            if is_fn_arg {
                self.fn_ref_context = true;
            }
            let expr = self.generate_expr_buf(arg)?;
            self.fn_ref_context = prev_fn_ref_ctx;
            // P265: when the fn-ref's parameter at this index is text,
            // coerce the binding to `&str` at the bind site so every
            // match arm can pass `_farg_{i}` to a `&str` parameter
            // uniformly.  Without this, text-returning user fn calls
            // produce `Str` (the codegen-runtime wrapper struct), and
            // each match arm trips rustc E0308 on `n_println(cell,
            // _farg_0)` etc.  Sibling to the direct-call fix shipped
            // for P262 in `src/generation/calls.rs` — same Str→&str
            // mismatch, different emit site.  The work-buffer arg
            // (when `is_text_return` adds args.len() == param_types.len()+1)
            // sits at index `args.len() - 1` which is `>= param_types.len()`,
            // so the condition correctly excludes it (its type is
            // `Type::RefVar(Type::Text(_))`, emitted as `&mut String`).
            let is_text_arg = i < param_types.len() && matches!(param_types[i], Type::Text(_));
            if is_text_arg {
                write!(
                    w,
                    "let _farg_{i}_h = {expr}; let _farg_{i}: &str = &*_farg_{i}_h; "
                )?;
            } else {
                write!(w, "let _farg_{i} = {expr}; ")?;
            }
        }
        // Look up the closure work-var for this fn-ref variable (if any).
        let closure_var_nr = self.data.def(self.def_nr).variables().closure_var_of(v_nr);
        // `@FR-B-Ref-Uniform` — the environment comes from the SLOT, always.  `var_<slot>.1` is
        // where the mint put it (`var_g = (dnr, var____clos_1)` writes the local straight into
        // it) and it is also where a later rebind puts a different one, so reading the slot is
        // never staler than reading the local and is sometimes fresher.
        //
        // The same-scope form used to prefer the caller's own `___clos_N`, which re-derives
        // "which environment does this slot hold" from the MINT SITE — a non-local fact — and
        // is right only while nothing else can write the slot.  A `&fn(…) -> τ` link is exactly
        // the thing that can: `inner(g)` wrote a new closure through `&mut var_g`, the dispatch
        // kept calling the caller's original environment, and `g()` answered the old value with
        // no diagnostic.  It reproduced only when the caller's slot started out CAPTURING,
        // because that is the only shape where a `___clos_N` exists to prefer — which is why
        // loft#1443's ten cells, all pinned to a non-capturing initial value, never moved it
        // (`D-bind-29`).
        let _ = closure_var_nr;
        let closure_expr: String = format!("var_{var_name}.1");
        let work_buf_expr: String = if let Some(idx) = work_buf_idx {
            format!("_farg_{idx}")
        } else {
            // No parser-allocated work-buffer (e.g. fn-ref type isn't
            // text-return, or call site predates Step 2).  Fall back to
            // a block-scope buffer; arms that never write to it are
            // unaffected.  This branch should be unreachable when ret
            // is text after Step 2, but kept as a defensive default.
            String::new()
        };
        // PLAN51 V-c — vector-returning fn-refs need a PRE-ALLOCATED
        // hidden buffer because the lambda body's `v = []` init compiles
        // to `if var_v.rec != 0 { clear }; pre_alloc_vector(&var_v, …)`
        // WITHOUT an OpDatabase on var_v.  Passing the u16::MAX sentinel
        // (as Reference-returning lambdas accept) crashes at
        // `pre_alloc_vector → mut_store(stores[u16::MAX])`.  Mirrors the
        // pattern direct callers use at `src/generation/dispatch.rs:586-591`
        // for vector-returning user fns.  Reference/struct-enum returns
        // chain-allocate via the body's struct literal / nested call so
        // they still get the sentinel.
        //
        // Read through `ret_promo_base`, because that is what decides whether the CALLEE
        // has a buffer to be handed.  A nullable collection return (`-> vector<τ>?`) is
        // reserved a hidden `__retbuf` exactly like the bare spelling — the `?` belongs to
        // the value the caller reads, not to the storage — so asking the RAW type here
        // handed such a candidate `DbRef::NULL` and its delivery wrote through it.  The
        // `.rec != 0` guard in front of the clear is not the whole delivery: `vector_add`
        // has no such test, so the crash was a NULL `DbRef` reaching a store accessor.
        let vec_hbuf_tp: Option<u16> = if let Type::Vector(elm_tp, _) = ret_type.ret_promo_base() {
            let elm_name = elm_tp.name(self.data);
            let tp = self.data.name_type(&format!("main_vector<{elm_name}>"), 0);
            (tp != u16::MAX).then_some(tp)
        } else {
            None
        };
        // @PLN85 L2 — the buffer is allocated INSIDE each arm that needs it
        // (lazily), never before the match: a pre-match allocation leaked one
        // store per call through every candidate arm that takes no buffer
        // (a plain lambda body returns its own fresh store by value).
        let heap_hbuf_expr: String = if vec_hbuf_tp.is_some() {
            "__vc_hbuf".to_string()
        } else {
            "loft::keys::DbRef::NULL".to_string()
        };
        // match on .0 (d_nr) of the (u32, DbRef) fn-ref tuple.
        if heap_return {
            write!(w, "let __vc_out = ")?;
        }
        write!(w, "match var_{var_name}.0 {{")?;
        for (d_nr, _fn_name, has_closure) in &candidates {
            write!(w, " {d_nr}_u32 => ")?;
            // P227: text-return arms wrap each call result with
            // `.to_string()` so heterogeneous candidate Rust signatures
            // (some return `Str`, some return `String`) collapse to a
            // uniform `String` for the match expression.  Both `Str`
            // and `String` implement `ToString` (via `Display` / blanket
            // impl), so this works for any text-typed candidate.  No
            // `stores.scratch` usage — the produced `String` is owned
            // by the match arm and naturally drops when the outer
            // expression has consumed it.
            if is_text_return {
                write!(w, "(")?;
            }
            // @PLN85 L2 — a candidate with a hidden Vector buffer gets an
            // in-arm allocation.  Whether the arm must also FREE it is
            // statically known from the candidate's returned deps: a
            // buffer-DELIVERING candidate returns the buffer (the result IS
            // the store — the caller frees it as the owned result), while a
            // candidate whose returned deps do not name the buffer (a
            // CallRef-tail body returns its own store by value) leaves the
            // buffer behind — free it right after the call.
            let cand_def_pre = self.data.def(*d_nr);
            // ALL hidden heap attrs — the same predicate the synthetic-args
            // loop below uses to emit `heap_hbuf_expr`, so every arm that
            // references `__vc_hbuf` also binds it (a Reference-returning
            // candidate cross-matches a vector dispatch — same DbRef ABI —
            // and its hidden attr is Reference-typed, not Vector).
            let hidden_heap_attrs: Vec<usize> = cand_def_pre
                .attributes
                .iter()
                .enumerate()
                .filter(|(_, a)| {
                    a.hidden
                        && matches!(
                            a.typedef,
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                })
                .map(|(i, _)| i)
                .collect();
            let arm_allocs_buf = vec_hbuf_tp.is_some() && !hidden_heap_attrs.is_empty();
            let _ = &cand_def_pre;
            if arm_allocs_buf {
                let tp = vec_hbuf_tp.unwrap_or_default();
                write!(
                    w,
                    "{{ let mut __vc_hbuf: DbRef = stores.null_named(\"__vc_hbuf\"); \
                     __vc_hbuf = OpDatabase(cell, __vc_hbuf, {tp}_i32); let __vc_r = "
                )?;
            }
            // Build synthetic args matching this candidate's attribute list.
            // The candidate's attrs are interleaved: user params, then
            // hidden `RefVar(Text)` work-buffers, then `__closure` (if
            // has_closure).  Detection is type-based for work-buffers,
            // name-based only for `__closure`.
            let candidate_def = self.data.def(*d_nr);
            let mut synthetic: Vec<Value> = Vec::with_capacity(candidate_def.attributes.len());
            let mut user_idx = 0_usize;
            let mut text_buf_used = false;
            for a in &candidate_def.attributes {
                if a.hidden
                    && matches!(
                        a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    // PLAN51 V-c: ref_return-promoted hidden buffer.
                    // For Reference / struct-enum returns the sentinel
                    // DbRef is fine — the callee's struct-literal /
                    // nested call body chain-allocates via OpDatabase.
                    // For Vector returns the pre-allocated __vc_hbuf is
                    // required because the body's `v = []` init does
                    // NOT emit OpDatabase, and `pre_alloc_vector`
                    // dereferences var_v.store_nr.  `heap_hbuf_expr`
                    // was computed above to be either "__vc_hbuf"
                    // (Vector ret) or the sentinel literal.
                    synthetic.push(Value::RawExpr(heap_hbuf_expr.clone()));
                } else if matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
                {
                    // loft#1116 — the call site supplies exactly ONE text work buffer,
                    // because it cannot know which function the fn-typed slot holds.  A
                    // candidate declaring more than one used to receive that same
                    // expression once per attribute, which borrows it mutably twice and
                    // does not compile (`E0499`) — and candidates are chosen by SIGNATURE,
                    // so a function nobody ever takes a reference to reddened the build
                    // merely because some lambda shared its shape.
                    //
                    // Giving the extras their own temporaries is safe HERE and only here:
                    // native returns text OWNED and never threads a value back through
                    // this buffer (`State::fn_call_ref` says so — the threading is
                    // interpreter-only).  The buffers are parameters that must type-check,
                    // not the delivery, so a candidate whose two paths deliver through two
                    // different buffers still answers correctly through a fn-ref.  Do NOT
                    // carry this reasoning to the interpreter, where the buffer IS the
                    // delivery and a scratch one would swallow the result.
                    if text_buf_used || work_buf_expr.is_empty() {
                        synthetic.push(Value::RawExpr("&mut String::new()".to_string()));
                    } else {
                        synthetic.push(Value::RawExpr(work_buf_expr.clone()));
                        text_buf_used = true;
                    }
                } else if a.name == "__closure" {
                    synthetic.push(Value::RawExpr(closure_expr.clone()));
                } else {
                    synthetic.push(Value::RawExpr(format!("_farg_{user_idx}")));
                    user_idx += 1;
                }
            }
            let _ = has_closure;
            // Route through output_call_user_fn → emit_op → custom emitter
            // (or DefaultEmitter::user_fn_call_body when no emitter is
            // registered for this candidate).
            self.output_call_user_fn(w, candidate_def, &synthetic)?;
            if arm_allocs_buf {
                // The call site allocated this buffer, so the call site owns whatever it did
                // not hand over — and WHICH of the two came back is a run-time fact, not a
                // static one.  The callee's return deps naming the buffer says it MEANT to
                // deliver through it, and a body whose delivery slot pass 2 rebound to a view
                // (`r = xs.map(…); r`) declares that and hands back its own store instead.
                // So ask the value: same store, the caller's result IS the buffer and freeing
                // it would dangle; different store, nothing owns the buffer and it leaks.
                //
                // This is `State::fn_return`'s rule verbatim (loft#1179 — *"keep the one the
                // callee handed back, identified by STORE"*), and reading it the same way on
                // both backends is what that fix set out to do.  The static test it replaces
                // was an over-approximation in one direction only: it kept the buffer for
                // every callee that DECLARED a delivery, filled or not.
                //
                // The KEPT one then needs an owner, and the destination's static type may
                // say it owns nothing — a local that borrows on one assignment and receives
                // this store on another (loft#1183).  `cr_fnref_buf` records it against the
                // running frame and `FnRefBufGuard` releases it there, which is the
                // interpreter's `release_fnref_bufs` on this side.
                write!(
                    w,
                    "; codegen_runtime::cr_fnref_buf(cell, __vc_r, __vc_hbuf); __vc_r }}"
                )?;
            }
            if is_text_return {
                write!(w, ").to_string()")?;
            }
            write!(w, ",")?;
        }
        write!(
            w,
            " _ => unreachable!(\"invalid fn-ref: {{}} in {var_name}\", var_{var_name}.0) }}"
        )?;
        if heap_return {
            // The match is the block's value; bind it so the store can be asked about, then
            // yield it unchanged.
            write!(
                w,
                "; codegen_runtime::cr_fnref_minted(cell, __vc_out, __vc_seq, {closure_expr}); \
                 __vc_out }}"
            )?;
        } else {
            write!(w, " }}")?;
        }
        Ok(())
    }

    /// Use this to emit an `if/else` expression. Handles whether branches are bare
    /// blocks (no extra braces needed) or single expressions (braces required).
    /// Infer the result type of an expression for generating typed null defaults.
    pub(super) fn infer_type(&self, node: IrNode) -> Option<Type> {
        match node.kind() {
            // P243 — see through `Span` so callers querying a wrapped
            // expression's type get the inner value's type, not `None`.
            ValueType::Span => self.infer_type(node.span_inner()),
            ValueType::Int => Some(Type::Integer(IntegerSpec::signed32())),
            ValueType::Long => Some(crate::data::I64.clone()),
            ValueType::Float => Some(Type::Float),
            ValueType::Single => Some(Type::Single),
            ValueType::Boolean => Some(Type::Boolean),
            ValueType::Text => Some(Type::Text(Deps::none())),
            ValueType::Enum => Some(Type::Enum(
                u32::from(node.enum_pair().1),
                false,
                Deps::none(),
            )),
            // @PLN25: native codegen treats `Optional(τ)` as its base (shared ABI/layout; the
            // null sentinel is a VALUE, not a separate codegen type) — peel so every
            // infer_type-based decision (text/bool branch unification, typed-null, predicate
            // coercion, …) sees through nullability. Gate-OFF inert (Optional never built).
            ValueType::Var => Some(
                self.data
                    .def(self.def_nr)
                    .variables
                    .tp(node.var_nr())
                    .base()
                    .clone(),
            ),
            ValueType::Call => {
                let ret = self.data.def(node.call_to()).returned();
                (*ret != Type::Void).then(|| ret.base().clone())
            }
            ValueType::Block => {
                let r = node.as_block().result();
                (r != Type::Void).then_some(r)
            }
            // An Insert's result type is its LAST item's type (the tail value;
            // earlier items are side-effecting ops like clear/append).  A
            // `match` arm such as `{ clear(__retbuf); append(__retbuf, p);
            // __retbuf }` lowers to an Insert, so a value-position
            // `if … else null` whose value-branch is such an Insert must report
            // this type — otherwise the sibling-Null typed-null handler below
            // (`false_v == Null`) falls through and the `null` emits `()`,
            // which rustc rejects against the `DbRef` result (the
            // match-return-over-borrowed-params E0308; interp was already
            // correct, so this closes a native-only divergence).
            ValueType::Insert => node
                .insert_items()
                .iter()
                .last()
                .and_then(|last| self.infer_type(last)),
            ValueType::If => self.infer_type(node.if_then()),
            // @PLN17: resolve a tuple element's type so a boolean element used as
            // a predicate (`if t.1`) gets the `output_test_predicate` u8->bool
            // coercion (and typed-null defaults are correct for tuple elements).
            ValueType::TupleGet => {
                let var = node.tupleget_var();
                let idx = node.tupleget_idx() as usize;
                match self.data.def(self.def_nr).variables.tp(var) {
                    Type::Tuple(elems) => elems.get(idx).cloned(),
                    _ => None,
                }
            }
            // @PLN17: a fn-ref call's type is the fn-ref var's Function return type,
            // so a boolean-returning `flip()` used as a predicate (`if flip()`) gets
            // the output_test_predicate u8->bool coercion (the dispatch `match` arms
            // return the u8 storage form).
            ValueType::CallRef => {
                let var = node.callref_var();
                if let Type::Function(_, r, _) = self.data.def(self.def_nr).variables.tp(var) {
                    (**r != Type::Void).then(|| (**r).clone())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Emit a value in an i32-literal context — any `Value::Int`
    /// descendant emits as `_i32` instead of the post-2c `_i64`
    /// default.  Use at tp-number, field-index, and flag-enum
    /// argument slots where the runtime signature is still i32.
    pub(super) fn emit_i32_slot(&mut self, w: &mut dyn Write, val: &Value) -> std::io::Result<()> {
        // A bare integer literal renders directly as `N_i32` (the i32-literal
        // context).  Anything else — a variable or a computed expression, e.g. a
        // runtime `par(…, threads)` thread count — is an `i64`/typed value in
        // normal context, so emit it there and cast the whole expression to
        // `i32`.  Turning on the i32-literal context around a non-literal instead
        // would leave the value `i64` (the flag only rewrites bare literals) — the
        // E0308 a variable thread count used to hit — and would also mint `i32`
        // literals inside any `i64` arithmetic the expression contains.
        if matches!(val.unspan(), Value::Int(_)) {
            let saved = self.i32_literal_context;
            self.i32_literal_context = true;
            let r = self.output_code_inner(w, val);
            self.i32_literal_context = saved;
            r
        } else {
            write!(w, "(")?;
            self.output_code_inner(w, val)?;
            write!(w, " as i32)")
        }
    }

    /// Emit a typed null sentinel for the given type.
    pub(super) fn write_typed_null(w: &mut dyn Write, tp: &Type) -> std::io::Result<()> {
        Self::write_typed_null_in(w, tp, false)
    }

    /// The typed null, in the representation the destination actually holds.
    ///
    /// `storage` picks the same split [`rust_type`](crate::generation::rust_type) makes for
    /// `boolean`: a null-capable POSITION (a parameter, a local, a return) holds the @PLN17
    /// tri-state as a raw `u8` (0/1/255), while a transient expression result is a 2-state
    /// `bool`. Everything else spells the same either way, so this is the only arm that reads
    /// the flag.
    ///
    /// The two `if`-branch callers pass `false`: they emit a null to unify with the OTHER
    /// branch, which is an expression. The call-argument site passes `true` — its destination
    /// is a parameter slot, and emitting `false` there put a `bool` where the generated
    /// signature declares `u8`, so an omitted `boolean? = null` argument failed to compile at
    /// all (rustc E0308, loft#1015).
    pub(super) fn write_typed_null_in(
        w: &mut dyn Write,
        tp: &Type,
        storage: bool,
    ) -> std::io::Result<()> {
        match tp {
            // @PLN25 slice (b): `Optional(τ)`'s null is its base's sentinel (same storage).
            Type::Optional(inner) => Self::write_typed_null_in(w, inner, storage),
            // types.md pins `Char`'s in-band sentinel at CODEPOINT 0 — the same
            // `'\0'` that `op_conv_bool_from_character` tests for and that the
            // interpreter's `OpConvCharacterFromNull` pushes.  `i32::MIN` was a
            // fourth spelling nothing else recognised, so `a == null` on an
            // omitted `character? = null` answered FALSE here and `true` on the
            // interpreter; worse, `ops::to_char` reaches it via
            // `from_u32_unchecked`, for which `0x8000_0000` is undefined
            // behaviour (loft#1014).
            Type::Character => write!(w, "0"),
            Type::Integer(_) => write!(w, "i64::MIN"),
            Type::Float => write!(w, "f64::NAN"),
            Type::Single => write!(w, "f32::NAN"),
            Type::Boolean if storage => write!(w, "255_u8"),
            Type::Boolean => write!(w, "false"),
            Type::Text(_) => write!(w, "loft::state::STRING_NULL"),
            Type::Enum(_, false, _) => write!(w, "255_u8"),
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _) => {
                // The canonical heap-ref null is `DbRef::NULL` (`keys.rs`), the
                // ONE source every backend reads — `is_null()` keys off
                // `store_nr == u16::MAX` and ignores `pos`, so the historical
                // `pos: 8` literal here was drift, not a distinct sentinel
                // (Cluster D / H6).
                write!(w, "DbRef::NULL")
            }
            _ => write!(w, "()"),
        }
    }

    pub(super) fn output_if(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
    ) -> std::io::Result<()> {
        self.output_if_inner(w, test, true_v, false_v, false)
    }

    /// Emit an `if` test value as a Rust bool predicate.
    ///
    /// When the test is a heap-DbRef-typed variable (Vector / Hash /
    /// Sorted / Index / Reference / struct-Enum), the value-block `??`
    /// lowering at `parser/operators.rs` synthesises `if _ncc_N { _ncc_N }
    /// else { fallback }` — but `_ncc_N` is a `DbRef`, not a `bool`,
    /// and rustc rejects it with E0308.  The null sentinel for heap
    /// DbRefs is `DbRef::NULL` (`keys.rs`, `store_nr == u16::MAX`; see
    /// `write_typed_null`), so `.rec != 0` is the canonical present-check
    /// (mirrors the existing checks throughout `codegen_runtime.rs`).
    ///
    /// PLAN52 cluster IV (heap-typed value-block `??`): probes 21 / 22 /
    /// 23 / 36 / 40 / 41 / 50.
    fn output_test_predicate(&mut self, w: &mut dyn Write, test: &Value) -> std::io::Result<()> {
        let heap_dbref = matches!(
            self.infer_type(IrNode::Native(test)),
            Some(
                Type::Reference(_, _)
                    | Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Enum(_, true, _)
            ),
        );
        // @PLN17: a boolean test may emit as `u8` (var / call / field — storage
        // form) or `bool` (comparison / `!` / `&&` result — expression form).
        // `((test) as u8) == 1` is the uniform truthiness coercion: idempotent for
        // a u8 (255 -> false, 1 -> true, 0 -> false), and 0/1 for a bool.
        let is_boolean = matches!(self.infer_type(IrNode::Native(test)), Some(Type::Boolean));
        if heap_dbref {
            self.output_code_inner(w, test)?;
            write!(w, ".rec != 0")
        } else if is_boolean {
            write!(w, "((")?;
            self.output_code_inner(w, test)?;
            write!(w, ") as u8) == 1")
        } else {
            self.output_code_inner(w, test)
        }
    }

    fn output_if_inner(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
        pre_declared: bool,
    ) -> std::io::Result<()> {
        // When the test carries pre-statements (an `Insert` — e.g. a boolean operand that lifted a
        // value-struct-returning call), the if-expression emits as `<lift>; if <pred> {…} else {…}`,
        // a STATEMENT sequence. Every consumer that needs a VALUE (a pre-eval `(…) as i64`, a
        // bool_unify arm `((…) as u8)`, a nested test predicate `((…) as u8) == 1`) wraps it in
        // parens, giving the invalid `( stmt; expr )`. Wrap the whole if-expression in a block
        // `{ … }` here so it is one valid expression for ALL of them.
        // Every shape test below peels: this decides how a branch is EMITTED, and a `Span`
        // that hides an `Insert` or a `Block` does not change what the branch is, only what
        // this function can see of it.  Measured over the 863-program native corpus: 4 193 of
        // 110 157 arrivals carry one.
        let wrap_block = matches!(test.unspan(), Value::Insert(ops) if ops.len() >= 2);
        if wrap_block {
            write!(w, "{{")?;
        }
        if !pre_declared {
            self.pre_declare_branch_vars(w, true_v, false_v)?;
        }
        if let Value::Insert(ops) = test.unspan()
            && ops.len() >= 2
        {
            for op in &ops[..ops.len() - 1] {
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
                self.indent(w)?;
            }
            write!(w, "if ")?;
            self.output_test_predicate(w, &ops[ops.len() - 1])?;
        } else {
            write!(w, "if ")?;
            self.output_test_predicate(w, test)?;
        }
        let b_true = matches!(true_v.unspan(), Value::Block(_));
        let b_false = matches!(false_v.unspan(), Value::Block(_));
        // @PLAN52 cluster VII: when the if-result is `Text`, branches can
        // produce `&String` (text local via Var emit's `&var_x`), `Str`
        // (text-returning native call), or `&'static str` (literal).  These
        // do NOT unify at the if-branch level — rustc reports E0308
        // "expected `&String`, found `Str`".  Wrapping each non-Block
        // branch with `&*(...)` forces a common `&str` (idempotent on
        // `&String` and `&str`, valid on `Str` via its `Deref<Target=str>`).
        // #534 — when the two text arms deliver mismatched Rust reps (a
        // `String` call arm vs a `&str` literal/interpolation arm), `&*(…)`
        // cannot unify them: borrowing an owned-`String` temporary would
        // dangle.  Defer to `text_string_unify` below (owned `String` for
        // both) instead.
        let text_mismatch = self.text_if_mismatched_reps(true_v, false_v);
        // @PLN102 `?? null`: a coalesce arm now reports `text?` (`Optional(Text)`) when its
        // fallback can be null (`v[i] ?? null`), so peel `Optional` before the text-rep check — a
        // `text?` if-arm has the SAME `String`/`&str` unify hazard as a plain `text` one (its
        // present-path still materialises an owned `String` while a `null` sibling arm is `&str`).
        let true_arm_is_text = self
            .infer_type(IrNode::Native(true_v))
            .is_some_and(|t| matches!(t.base(), Type::Text(_)));
        let text_unify = !b_true
            && !b_false
            && !text_mismatch
            && !matches!(false_v, Value::Null)
            && true_arm_is_text;
        // @P386: a text-result if-expression where any branch is a Block
        // containing the `__ncc_*` skip_free pattern produces an OWNED
        // `String` for that branch (via the `_ret.to_string()` block-tail
        // materialisation in `output_block`'s trailing-void path).  The
        // SIBLING branch typically emits a `&str` (a `STRING_NULL` typed
        // null, an OpConvTextFromNull → `loft::state::STRING_NULL`, or a
        // bare literal).  The two arms then fail to unify (rustc E0308
        // "expected `&str`, found `String`").  Force each branch through
        // `.to_string()` so both produce `String`.  This is idempotent on
        // an already-`String` value and converts `&str` / `Str` / `&String`
        // uniformly.  The outer text-return wrap (Return(If(…))'s scratch
        // routing) then takes `String` via `.to_string()` push.
        // #534 extends the trigger: also unify to `String` when the arms'
        // reps mismatch (owned call vs borrowed literal/interp), not only for
        // the `??` ncc pattern.
        let text_string_unify = !text_unify
            && (text_mismatch
                || matches!(true_v, Value::Block(b) if self.block_contains_ncc_skip_free(b))
                || matches!(false_v, Value::Block(b) if self.block_contains_ncc_skip_free(b)))
            && true_arm_is_text;
        // @PLN17: a boolean if-expression (e.g. the `&&` / `||` lowering
        // `if a {b} else {false}`) has arms that may be `u8` (a var/call — storage
        // form) or `bool` (a literal/comparison — expression form), which don't
        // unify.  Wrap both arms `((arm) as u8)` so the if-expression is uniformly
        // `u8`; the consumer (predicate / store / operand) coerces from there.
        // #354: Block arms join the wrap (`{(({…block…}) as u8)}` is valid
        // Rust) — a boolean-typed ncc Block then-arm casts itself to u8 while
        // a literal `false` else-arm stayed `bool` (E0308).  A statement-if is
        // unaffected: its arm blocks are void-typed, so `infer_type` is not
        // Boolean and the gate stays closed.
        let bool_unify = !matches!(false_v, Value::Null)
            && matches!(self.infer_type(IrNode::Native(true_v)), Some(Type::Boolean));
        // `(F-Block)` in `formal/calls.md` — an `if` in STATEMENT position discards what its
        // arms yield, and the work still runs.  The interpreter has always done that; the
        // native emitter rendered each arm as a Rust expression, so `if c { println("x") }
        // else { 5 }` handed rustc a `()` arm beside an `i64` one and the user got a raw
        // E0308 for a program loft accepts (loft#1381).
        //
        // A VOID arm beside a non-void one is exactly that position and nothing else: an `if`
        // used as a VALUE has arms the parser already made agree, and a void one could not be
        // read.  So the two arms are made to yield `()` by discarding each with a `;`, which
        // is what a statement means.
        // Both arms must be POSITIVELY typed: an arm the emitter cannot type is not evidence
        // of a statement, and reading "unknown" as void fired this on six tuple files whose
        // arms `infer_type` answers `None` for — each then lost the value the `if` was there
        // to produce (E0308 on `--native`, measured over the corpus).
        // loft#1495 — and the void arm must be void because the `if` is a STATEMENT, not
        // because control LEFT it.  An arm that diverges (`=> { b = …; return b.items; }`)
        // yields nothing and so types `Void`, while its sibling carries the value the `if`
        // is there to produce: discarding both then handed rustc a `()` where a `DbRef` was
        // wanted, and the program loft runs would not compile at all.  Rust types a diverging
        // arm `!`, which coerces to whatever the sibling yields, so the ordinary value
        // emission is already right for it.
        let stmt_discard = match (self.arm_result(true_v), self.arm_result(false_v)) {
            (Some(t), Some(f)) => {
                let t_void = matches!(t, Type::Void);
                let f_void = matches!(f, Type::Void);
                t_void != f_void
                    && !matches!(false_v, Value::Null)
                    && !Self::arm_diverges(if t_void { true_v } else { false_v })
            }
            _ => false,
        };
        // For `text_string_unify` we emit `{ (<branch>).to_string() }` around
        // each arm so the if-expression unifies on `String`.  Rust requires
        // braces for if-arms regardless of inner expression form, so even if
        // the branch is itself a `Block` (which emits its own `{…}`), we wrap
        // the block in `({…}).to_string()` inside an outer `{ … }`.
        if text_string_unify {
            write!(w, " {{(")?;
        } else if bool_unify {
            // Block, not parens, around the arm: the arm can be a STATEMENT sequence (a boolean
            // operand that lifted a value-struct-returning call → `<lift>; <predicate>`), so
            // `(( stmt; expr ) as u8)` is invalid Rust. `({ … } as u8)` is valid either way.
            write!(w, " {{({{")?;
        } else if stmt_discard {
            // An outer block whose single statement is the arm: `{ <arm>; }` yields `()`
            // whatever the arm yields, and a `Block` arm brings its own braces inside it.
            write!(w, " {{")?;
        } else if b_true {
            write!(w, " ")?;
        } else if text_unify {
            write!(w, " {{&*(")?;
        } else {
            write!(w, " {{")?;
        }
        self.indent += u32::from(!b_true || text_string_unify || bool_unify);
        // save/restore fn_ref_context — Call arguments inside the branch
        // must NOT inherit it (OpDatabase int args would be misinterpreted).
        let saved_ctx = self.fn_ref_context;
        // Symmetric to the else-Null handling below: a Null in the THEN branch
        // must emit the typed null sentinel, not `()`.  This is the match-arm
        // lowering `if subj==X { null } else { <value> }` — a struct-returning
        // `match … { … => null }` put the `null` in the then-branch, so it
        // emitted `()` and the arm failed to unify with the value-producing else
        // (rustc E0308 `()` vs `DbRef`, breaking the whole --native compile).
        if matches!(true_v, Value::Null)
            && let Some(tp) = self.infer_type(IrNode::Native(false_v))
        {
            Self::write_typed_null(w, &tp)?;
        } else {
            self.clone_handed_tuple_local = self.arm_moves_a_live_tuple_local(true_v);
            self.output_code_inner(w, true_v)?;
            self.clone_handed_tuple_local = None;
        }
        self.fn_ref_context = saved_ctx;
        self.indent -= u32::from(!b_true || text_string_unify || bool_unify);
        if text_string_unify {
            write!(w, ").to_string()}} else ")?;
        } else if text_unify {
            write!(w, ")}} else ")?;
        } else if bool_unify {
            write!(w, "}} as u8)}} else ")?;
        } else if stmt_discard {
            write!(w, ";}} else ")?;
        } else if b_true {
            // The SAME test that decided not to open a brace for this arm: a `Block` emits its
            // own `{ … }`, and `b_true` peels a `Span` to see it.  Asked here on the bare value,
            // a span-wrapped arm — the `join-arm-owner` block a `match` arm's minting call is
            // lowered into — opened no brace of its own and then closed one, and every
            // `s = match k { 9 => mk(7), _ => s }` was an unbalanced `}` rustc refused.
            write!(w, " else ")?;
        } else {
            write!(w, "}} else ")?;
        }
        if text_string_unify {
            write!(w, "{{(")?;
        } else if text_unify {
            write!(w, "{{&*(")?;
        } else if bool_unify {
            write!(w, "{{({{")?;
        } else if stmt_discard || !b_false {
            write!(w, "{{")?;
        }
        self.indent += u32::from(!b_false || text_string_unify || bool_unify);
        // When the else branch is Null and the true branch returns a value,
        // emit a typed null sentinel instead of () to match the true branch type.
        if matches!(false_v, Value::Null)
            && let Some(tp) = self.infer_type(IrNode::Native(true_v))
        {
            Self::write_typed_null(w, &tp)?;
        } else {
            self.clone_handed_tuple_local = self.arm_moves_a_live_tuple_local(false_v);
            self.output_code_inner(w, false_v)?;
            self.clone_handed_tuple_local = None;
        }
        if text_string_unify {
            write!(w, ").to_string()}}")?;
        } else if text_unify {
            write!(w, ")}}")?;
        } else if bool_unify {
            write!(w, "}} as u8)}}")?;
        } else if stmt_discard {
            write!(w, ";}}")?;
        } else if !b_false {
            write!(w, "}}")?;
        }
        self.indent -= u32::from(!b_false || text_string_unify || bool_unify);
        if wrap_block {
            write!(w, " }}")?;
        }
        Ok(())
    }

    /// What an `if` ARM yields, as the emitter can see it: a `Block`'s declared result, or
    /// the inferred type of anything else.
    ///
    /// A block's own `result` rather than `infer_type` because the two disagree on exactly the
    /// case that matters — an arm block holding a discarded value is typed by its declaration,
    /// and inference reads the last operator.
    fn arm_result(&mut self, v: &Value) -> Option<Type> {
        match v.unspan() {
            Value::Block(bl) => Some(bl.result.clone()),
            // loft#1382 — an `else if` CHAIN: the arm is a nested `if`, which `infer_type`
            // does not answer for, so the outer gate saw "cannot type" and declined while the
            // inner one had already discarded its own arms.  The outer `if` then had a
            // value arm beside a `()` one and rustc rejected it.  Answer by the same rule the
            // gate applies: a nested `if` with exactly one void arm is discarded and yields
            // `()`, so it is VOID to whoever encloses it; otherwise it answers what its arms
            // agreed on.
            Value::If(_, then_arm, else_arm) if !matches!(else_arm.unspan(), Value::Null) => {
                let then_tp = self.arm_result(then_arm)?;
                let else_tp = self.arm_result(else_arm)?;
                // By the same rule the gate applies, INCLUDING loft#1495's exclusion: a
                // nested `if` whose void arm merely DIVERGES is not discarded, so it still
                // answers the value its other arm carries.  Reading it as `Void` here made
                // the enclosing `if` discard a chain that does yield one — which is how a
                // three-armed `if … return … else if … else …` failed to compile.
                let then_void = matches!(then_tp, Type::Void);
                let else_void = matches!(else_tp, Type::Void);
                let discarded = then_void != else_void
                    && !Self::arm_diverges(if then_void { then_arm } else { else_arm });
                Some(if discarded {
                    Type::Void
                } else if then_void {
                    else_tp
                } else {
                    then_tp
                })
            }
            other => self.infer_type(IrNode::Native(other)),
        }
    }

    /// Does this arm never YIELD a value, because control LEAVES it rather than falling out
    /// of the bottom?
    ///
    /// Such an arm types `Void` for a reason that has nothing to do with the `if` being a
    /// statement — there is no value because control left.  Rust types it `!`, which coerces
    /// to whatever the sibling arm yields, so the ordinary value emission serves it and the
    /// statement discard does not.
    ///
    /// [`Value::tail`] is the one home for *descend to where control leaves*, through a
    /// `Span`, a value block and an argument-lift `Insert` alike — the same helper the
    /// diverging-return capture below asks.
    fn arm_diverges(v: &Value) -> bool {
        matches!(
            v.tail(),
            Value::Return(_) | Value::Break(_) | Value::Continue(_)
        )
    }

    fn pre_declare_branch_vars(
        &mut self,
        w: &mut dyn Write,
        true_v: &Value,
        false_v: &Value,
    ) -> std::io::Result<()> {
        let mut t_vars: Vec<u16> = Vec::new();
        let mut f_vars: Vec<u16> = Vec::new();
        Self::collect_set_vars(IrNode::Native(true_v), &mut t_vars);
        Self::collect_set_vars(IrNode::Native(false_v), &mut f_vars);
        let variables = self.data.def(self.def_nr).variables();
        for &v in &t_vars {
            if f_vars.contains(&v) && !self.declared.contains(&v) {
                let name = sanitize(variables.name(v));
                let tp_str = rust_type(variables.tp(v), &Context::Variable);
                let default = default_native_value_in(variables.tp(v), &Context::Variable);
                writeln!(w, "let mut var_{name}: {tp_str} = {default};")?;
                self.indent(w)?;
                self.declared.insert(v);
            }
        }
        Ok(())
    }

    /// Every variable this subtree ASSIGNS, anywhere within it.
    ///
    /// The total-walk twin of [`Self::collect_set_vars`], which stops at the kinds its own
    /// caller cares about and so misses a `Set` inside a loop.  Built on `for_each_child`,
    /// whose match is exhaustive on purpose: a new [`ValueType`] forces a decision there
    /// rather than silently dropping out of this answer (loft#815).
    /// Does this if-arm hand over a LOCAL whose Rust value cannot be copied?
    ///
    /// A loft tuple lowers to a Rust tuple, and a tuple carrying a text element is
    /// `(…, String, …)` — not `Copy`.  Yielding the bare local from an arm therefore MOVES
    /// it, and any later read of the local is `error[E0382]: borrow of moved value`, on a
    /// program the interpreter runs (loft#1354).
    ///
    /// Only the TUPLE shape needs this.  A text local is already re-materialised by the
    /// text-unify paths above, and a struct or vector local is a `DbRef`, which is `Copy` —
    /// which is why `x = if c { s } else { … }` compiles for every other heap type and
    /// only the tuple refused.
    ///
    /// The clone matches what the arm means: the element that is a store handle stays a
    /// handle (the same `DbRef`, so the two names see one vector, which is what the IR's
    /// `["dp"]` dep already says), and the owned `String` is duplicated so both names can
    /// read it.
    fn arm_moves_a_live_tuple_local(&self, v: &Value) -> Option<u16> {
        // An arm is usually a BLOCK whose tail is the variable, so the bare-`Var` spelling
        // is only the simplest case.  Cloning the block's RESULT would not help: the tail
        // has already moved the local by then, so the clone has to happen at the tail.
        let inner = match v.unspan() {
            Value::Var(nr) => *nr,
            Value::Block(b) => match b.operators.last().map(Value::unspan) {
                Some(Value::Var(nr)) => *nr,
                _ => return None,
            },
            _ => return None,
        };
        let vars = self.data.def(self.def_nr).variables();
        if vars.is_argument(inner) {
            return None;
        }
        let Type::Tuple(elems) = vars.tp(inner).base() else {
            return None;
        };
        elems
            .iter()
            .any(|t| matches!(t.base(), Type::Text(_)))
            .then_some(inner)
    }

    fn collect_assigned_vars(node: IrNode, result: &mut Vec<u16>) {
        if node.kind() == ValueType::Set {
            let v = node.set_var();
            if !result.contains(&v) {
                result.push(v);
            }
        }
        node.for_each_child(&mut |child| Self::collect_assigned_vars(child, result));
    }

    fn collect_set_vars(node: IrNode, result: &mut Vec<u16>) {
        match node.kind() {
            ValueType::Set => {
                let v = node.set_var();
                if !result.contains(&v) {
                    result.push(v);
                }
                Self::collect_set_vars(node.set_inner(), result);
            }
            ValueType::Block => {
                for op in node.as_block().operators().iter() {
                    Self::collect_set_vars(op, result);
                }
            }
            ValueType::If => {
                Self::collect_set_vars(node.if_cond(), result);
                Self::collect_set_vars(node.if_then(), result);
                Self::collect_set_vars(node.if_else(), result);
            }
            ValueType::Insert => {
                for op in node.insert_items().iter() {
                    Self::collect_set_vars(op, result);
                }
            }
            ValueType::Call => {
                for a in node.call_args().iter() {
                    Self::collect_set_vars(a, result);
                }
            }
            ValueType::CallRef => {
                for a in node.callref_args().iter() {
                    Self::collect_set_vars(a, result);
                }
            }
            ValueType::Drop => Self::collect_set_vars(node.drop_inner(), result),
            ValueType::Return => Self::collect_set_vars(node.return_inner(), result),
            _ => {}
        }
    }

    /// Use this to emit a scoped sequence of operators with an optional return value.
    /// This is the most involved emitter because blocks must handle three interacting concerns:
    /// 1. **Pre-evaluation hoisting** — sub-expressions that would double-borrow `stores`
    ///    are lifted into `let _preN` bindings before the enclosing expression.
    /// 2. **Return-value tracking** — when void operators trail the last non-void expression,
    ///    that expression is captured into `let _ret` first, then yielded at the end.
    /// 3. **String conversion** — a text-typed block may receive a `Str` from a field read;
    ///    `.to_string()` converts it to an owned `String`.
    // @PLN10 Phase B — the sanitized `var_…` name of this function's first HIDDEN
    // `RefVar(Text)` work buffer (a `&mut String` arg the caller owns), if any.
    // A buffered (`!nwb`) text fn returning a LOCAL / `??`-block / nwb-inner value
    // writes that owned `String` into this buffer and hands back a `Str` pointing
    // into it — caller-lifetime backing, no `stores.scratch`.  `None` only for an
    // nwb fn (handled by the owned-`String` path), so the scratch fallback below
    // is dead-but-safe.
    //
    // HIDDEN, because a user-written `&text` parameter is the caller's variable: read
    // as the buffer, an early `return mk()` in `fn f(s: &text, …)` was written into `s`
    // and the caller's text silently changed (loft#1338).  Every buffer `text_return`
    // promotes is marked hidden, so the hidden ones are exactly the deliverable ones —
    // `Definition::text_work_buffers` is the one home for that count.
    pub(super) fn return_buffer_name(&self) -> Option<String> {
        self.data
            .def(self.def_nr)
            .attributes
            .iter()
            .find(|a| {
                a.hidden && matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
            })
            .map(|a| sanitize(&a.name))
    }

    /// Whether a text-typed `if`/`else` arm delivers an OWNED `String` (as
    /// opposed to a borrowed `&str`).  A bare text-returning user fn call
    /// returns an owned `String` (`def_returns_owned_text`); the `??`
    /// value-block materialises one via the `__ncc_*` skip-free pattern.  A
    /// literal, an interpolation (a work-buffer borrow `&*var___work_N`), or a
    /// text variable all deliver `&str`.  For a block arm the delivered value
    /// is its tail (last) operator.
    ///
    /// Two arms that disagree here do NOT unify to one Rust type — rustc
    /// rejects with E0308 (#534: `text` `if`/`else` mixing a `String` arm and a
    /// `&str` arm) — so `output_if_inner` routes both through `.to_string()`
    /// and the `Str::new` return path materialises via the work buffer.  Keyed
    /// on `def_returns_owned_text`, so it never reports `String` for an arm
    /// that actually emits `&str`: a false match could only arise between arms
    /// whose reps genuinely differ, and those never compiled in the first place.
    fn text_arm_yields_owned_string(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Block(b) => {
                self.block_contains_ncc_skip_free(b)
                    || b.operators
                        .last()
                        .is_some_and(|t| self.text_arm_yields_owned_string(t))
            }
            Value::Insert(items) => items
                .last()
                .is_some_and(|t| self.text_arm_yields_owned_string(t)),
            Value::Call(d, _) => {
                (*d as usize) < self.data.definitions.len()
                    && matches!(self.data.def(*d).returned().base(), Type::Text(_))
                    && super::def_returns_owned_text(self.data.def(*d))
            }
            _ => false,
        }
    }

    /// The two text `if`/`else` arms deliver mismatched Rust reps (one owned
    /// `String`, one borrowed `&str`) and so must be unified via `.to_string()`
    /// (#534).  See [`Self::text_arm_yields_owned_string`].
    fn text_if_mismatched_reps(&self, true_v: &Value, false_v: &Value) -> bool {
        if self.text_arm_yields_owned_string(true_v) != self.text_arm_yields_owned_string(false_v) {
            return true;
        }
        // #552 — a text-returning CALL yields `Str` (buffered fn) or `String`, a bare text
        // LITERAL yields `&str`; these never unify at the if-arm level (rustc E0308 "expected
        // `Str`, found `&str`" — the vector-match desugar's `if <g(x)> else {"lit"}`).  A
        // buffered `-> Str` fn is NOT `def_returns_owned_text`, so the owned-String check above
        // misses it; catch the call-vs-literal pairing and unify both arms to owned `String`.
        (self.text_arm_ends_in_text_call(true_v) && Self::text_arm_is_bare_literal(false_v))
            || (Self::text_arm_is_bare_literal(true_v) && self.text_arm_ends_in_text_call(false_v))
    }

    /// True when the arm's tail is a text-returning function call (yields `Str`/`String`),
    /// walking through Block/Insert to the final value.  #552.
    fn text_arm_ends_in_text_call(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Block(b) => b
                .operators
                .last()
                .is_some_and(|t| self.text_arm_ends_in_text_call(t)),
            Value::Insert(items) => items
                .last()
                .is_some_and(|t| self.text_arm_ends_in_text_call(t)),
            Value::Call(d, _) => {
                (*d as usize) < self.data.definitions.len()
                    && matches!(self.data.def(*d).returned().base(), Type::Text(_))
                    // Only a USER text fn yields an owned `Str`/`String`; an `Op*` text op
                    // (OpConvTextFromNull → STRING_NULL, OpConstText, …) yields `&str`, which
                    // unifies with a bare literal — excluding them avoids a false mismatch that
                    // would `.to_string()` an arm inside a `&*(…)` borrow context (E0716).
                    && !self.data.def(*d).name().starts_with("Op")
            }
            _ => false,
        }
    }

    /// True when the arm's tail is a BARE text literal (yields `&str`), walking through
    /// Block/Insert to the final value.  #552.
    fn text_arm_is_bare_literal(v: &Value) -> bool {
        match v.unspan() {
            Value::Block(b) => b
                .operators
                .last()
                .is_some_and(Self::text_arm_is_bare_literal),
            Value::Insert(items) => items.last().is_some_and(Self::text_arm_is_bare_literal),
            Value::Text(_) => true,
            _ => false,
        }
    }

    // @PLAN52 cluster I/VI helper: walk a Block's operators (recursively
    // into nested Block / If / Match values) and report whether any
    // `Set(v, _)` exists where `v`'s name starts with `__ncc_` and the
    // variable is marked `skip_free`.  Used to gate scratch-buffer
    // materialisation for value-block `??` patterns.
    pub(super) fn block_contains_ncc_skip_free(&self, bl: &Block) -> bool {
        let variables = self.data.def(self.def_nr).variables();
        bl.operators.iter().any(|op| {
            op.any_node(&mut |n| {
                matches!(n, Value::Set(var, _)
                    if variables.name(*var).starts_with("__ncc_")
                        && variables.is_skip_free(*var))
            })
        })
    }

    /// #557 — will `output_block` emit this TEXT block's tail as an owned `String`
    /// (`let _ret = <value>; <trailing void op>; _ret.to_string()`)?  That happens when the
    /// block's value result is followed by a trailing VOID op — e.g. the `OpFreeText(a)` a
    /// vector-match on a `vector<text>` subject appends to free the bound element.  The
    /// `Str::new(<block>)` return wrapper then sees `Str::new(String)` (E0308), so the return
    /// must route the block through the work buffer instead (see `needs_p205_scratch`).  Mirrors
    /// the `has_trailing_void && !return_value_is_return` gate in `output_block`.
    fn block_tail_materialises_string(&self, bl: &Block) -> bool {
        if !matches!(bl.result.base(), Type::Text(_)) {
            return false;
        }
        let Some(ri) = bl.operators.iter().rposition(|v| !self.is_void_value(v)) else {
            return false;
        };
        // A tail that DIVERGES (ends in `Return`) is emitted directly — no `_ret` materialise.
        if matches!(bl.operators[ri].unspan(), Value::Return(_))
            || matches!(bl.operators[ri].tail(), Value::Return(_))
        {
            return false;
        }
        // A trailing VOID op after the value, or the `__ncc_*` skip-free pattern.
        ri < bl.operators.len().saturating_sub(1) || self.block_contains_ncc_skip_free(bl)
    }

    #[allow(clippy::too_many_lines)]
    /// `is_fn_body` marks the one block whose Rust type is the function's
    /// return signature (`Context::Result`).  Only there may the tail expression
    /// carry a narrow-integer cast — see [`block_tail_cast`].
    pub(super) fn output_block(
        &mut self,
        w: &mut dyn Write,
        block: IrBlock,
        wrap_text: bool,
        is_fn_body: bool,
    ) -> std::io::Result<()> {
        // @PLN11 G2/M4 — materialise-at-boundary: native is zero-cost; a
        // store-backed block materialises once, then the intricate `&Block` body
        // (which threads `bl` through patch_hoisted_returns /
        // detect_ref_tail_capture / is_void_value) runs unchanged.
        let owned_block;
        let bl: &Block = match block {
            IrBlock::Native(b) => b,
            IrBlock::Store(..) => {
                owned_block = block.to_owned_block();
                &owned_block
            }
        };
        // Plan-06 phase 4d: fn-ref field read emits the (u32, DbRef)
        // native tuple form directly.  The block carries two ops —
        // `OpGetInt4(ref, fld)` (returns i64) and `OpNullRefSentinel()`
        // (returns DbRef) — which the interpreter pushes sequentially
        // onto the eval stack to form the 20-byte fn-ref slot.  Native
        // can't use a generic Rust block here because the last
        // expression's type would be `DbRef`, not `(u32, DbRef)`; and
        // the `i64` from OpGetInt4 needs an explicit `as u32` cast to
        // match the fn-ref tuple's first slot.
        // @PLN157 § V-u (`@FR-R-RetAdopt`) — inside the delivery block of an ADOPTED
        // result local, the `OpClearVector(buf)` + `OpAppendVector(buf, v, tp)` pair has
        // nothing to move (the local IS the buffer; the clear would wipe the result, the
        // append would self-copy).  The block's OTHER statements — the scope-exit frees,
        // the returned value — emit unchanged, and the ENTRY clear (a bare statement
        // outside any such block) stays: it is the buffer's reuse contract.
        let adopt_delivery = bl.name == "one_buffer_vec_copy" && self.ret_adopt.is_some();
        if adopt_delivery {
            self.in_adopt_delivery += 1;
        }
        if bl.name == "fn_ref_field_read" && bl.operators.len() == 2 {
            write!(w, "((")?;
            self.output_code_inner(w, &bl.operators[0])?;
            write!(w, ") as u32, ")?;
            self.output_code_inner(w, &bl.operators[1])?;
            write!(w, ")")?;
            return Ok(());
        }
        writeln!(
            w,
            "{{ //{}_{}: {}",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, self.data.def(self.def_nr).variables())
        )?;
        // Inject shadow call stack instrumentation if set by output_function().
        if let Some(prefix) = self.call_stack_prefix.take() {
            writeln!(w, "{prefix}")?;
        }
        // @PLN157 § V-j (`@FR-R-MoveAppend`) — a paired `for f in call(…)`: place the
        // call's buffer as a record in the destination's own store, and arm the loop
        // variable so the append's OpCopyRecord emits the move.  The destination's
        // `__vdb` is live here by scoping: the loop appends to it, so its declaration
        // already ran.  The guard places ONCE per host lifetime: an enclosing loop
        // re-enters with the record still placed and reuses it (the callee's entry
        // clear resets its length); the var is nulled only where the HOST store is
        // freed, which re-arms the guard for a host recreated per iteration (c21).
        let move_pair = self.move_pair_for_block(bl).cloned();
        if let Some(pair) = &move_pair {
            let vars = self.data.def(self.def_nr).variables();
            let buf = super::sanitize(vars.name(pair.buf));
            // @PLN157 § V-u — an adopted result local has no witness store; its elements
            // live in the return buffer, which is where the paired call's buffer belongs.
            let vdb = match &self.ret_adopt {
                Some(a) if a.vdb == pair.host_vdb => super::sanitize(vars.name(a.buf)),
                _ => super::sanitize(vars.name(pair.host_vdb)),
            };
            self.indent(w)?;
            writeln!(
                w,
                "if var_{buf}.store_nr == u16::MAX {{ var_{buf} = stores.place_record_in(&var_{vdb}, {}u16); }} //@PLN157 § V-j placed buffer",
                pair.buf_tp
            )?;
            self.active_move_vars.push(pair.loop_var);
        }
        // @PLN157 § V-aa (`@FR-R-ValueRecord`) — an admitted function's `Object` tail
        // builds its record's fields in REGISTERS: the block's `OpSet*` calls carry one
        // value per field at a constant offset, so the whole block — the buffer's
        // allocate-or-reuse, the writes, the yield — is exactly the tuple of those
        // values, in field order.  The buffer it wrote into no longer exists.
        if bl.name == "Object"
            && let Some(&tp) = self.value_records.fns.get(&self.def_nr)
            && let Some(parts) = self.value_record_parts(bl, tp)
        {
            write!(w, "(")?;
            for (i, val) in parts.iter().enumerate() {
                if i > 0 {
                    write!(w, ", ")?;
                }
                self.output_code_inner(w, val)?;
            }
            write!(w, ")")?;
            // The block's OPENING brace is already out; close it exactly as the ordinary
            // path does, or the arm eats a delimiter (measured: `unclosed delimiter` on
            // the conditional-construction cell, where the Object sits inside an `if` arm).
            self.indent(w)?;
            return write!(
                w,
                "}} /*{}_{}: {}*/",
                bl.name,
                bl.scope,
                bl.result
                    .show(self.data, self.data.def(self.def_nr).variables())
            );
        }
        let is_void_block = matches!(bl.result, Type::Void);
        let is_text_result = wrap_text && matches!(bl.result, Type::Text(_));
        // Fix "hoisted return value" pattern from scopes::free_vars before iterating.
        // This replaces [expr, OpFreeText…, Return(Null)] with [OpFreeText…, Return(expr)]
        // so native code emits `return expr` rather than a dropped `expr` + `return ()`.
        // also patch Type::Never blocks (unconditional return with cleanup).
        // also patch `Type::Text` blocks when the enclosing function is
        // a bounded-generic T-stub (name like `t_<len><Type>_<method>`).
        // Their IR is produced by template specialisation and shows the same
        // `[Call, OpFreeText(work), Return(Null)]` pattern at the top of the
        // body block — without the patch, native codegen emits the Call as a
        // discarded statement and returns STRING_NULL.
        let fn_name = self.data.def(self.def_nr).name();
        let is_t_stub_text_body = matches!(bl.result, Type::Text(_)) && fn_name.starts_with("t_");
        // P240 fix (2026-05-11): bounded-generic T-stubs that return a
        // stack-passed tuple — `t_<len><Type>_<method>` returning
        // `Type::Tuple(...)` — go through the same hoisted-return
        // pattern as text-returning T-stubs (the parser appends a
        // synthetic `(lt, gt); OpFreeText(work); return null;` shape
        // to keep the interpreter's stack-cleanup happy).  Without
        // running `patch_hoisted_returns`, native codegen emits the
        // tuple as a discarded statement and falls through to a
        // hardcoded `return (0, 0)` — function always returned the
        // type's null sentinel regardless of actual inputs.  Mirror
        // the text branch above; same hoist logic applies because
        // both shapes have a `Return(Null)` tail with the actual
        // value as a preceding statement.
        let is_t_stub_tuple_body = matches!(bl.result, Type::Tuple(_)) && fn_name.starts_with("t_");
        // Any text-returning block whose body contains the B5-L3
        // `Set(__ret_N, call); ...; Return(Var(__ret_N))` temp-transfer
        // pattern must also go through `patch_hoisted_returns` so the
        // collapse pass can rewrite it to `return call(...)` — otherwise
        // the local `String` ret-temp drops at function exit and the
        // returned `Str` raw ptr dangles
        // (`tests/scripts/86-interfaces.loft::if_label`).
        let has_ret_temp = matches!(bl.result, Type::Text(_))
            && bl.operators.iter().any(|op| {
                matches!(op.unspan(), Value::Set(v, _) if
                    self.data.def(self.def_nr).variables().name(*v).starts_with("__ret_")
                    && self.data.def(self.def_nr).variables().is_skip_free(*v))
            });
        let patched_ops;
        let operators: &[Value] = if is_void_block
            || matches!(bl.result, Type::Never)
            || is_t_stub_text_body
            || is_t_stub_tuple_body
            || has_ret_temp
        {
            patched_ops = self.patch_hoisted_returns(&bl.operators);
            &patched_ops
        } else {
            &bl.operators
        };
        // Native-only ref-return tail-call capture (87-store-leaks).
        // See pre_eval.rs::detect_ref_tail_capture for the pattern.  At the
        // call index we emit `let __native_tail_ret: DbRef = <call>;`; at
        // the Return(Null) index we emit `return __native_tail_ret;`
        // instead of the null-sentinel `return DbRef { store_nr: u16::MAX, … }`.
        let tail_capture = self.detect_ref_tail_capture(bl, operators);
        // When the block expects a non-void result but trailing operator(s) are
        // void (drops, if-without-else, etc.), find the last non-void operator
        // and capture its value before the trailing void ops run.
        let last_op_idx = operators.len().saturating_sub(1);
        let return_idx = if is_void_block || operators.is_empty() {
            None
        } else {
            operators.iter().rposition(|v| !self.is_void_value(v))
        };
        // @PLAN52 cluster I iteration 2 (2026-05-30): force the
        // `let _ret = ...; ...; _ret.to_string()` block-tail pattern (the
        // @P323 materialisation at line ~1382) when the block holds a
        // `__ncc_*` skip_free text temp.  Without the skip_free flag (the
        // pre-fix shape), an OpFreeText was emitted after the value-op,
        // making `has_trailing_void = true` via the trailing-void check.
        // With skip_free suppressing the OpFreeText (interpret-side fix
        // for the dangling-Str), there's no trailing void op anymore —
        // but the materialisation is STILL required on native (otherwise
        // the block returns a borrow that dies at `}`).  Detect the
        // `__ncc_*` skip_free pattern explicitly and gate has_trailing_void
        // on for those blocks.
        // Only force when there's a value-yielding return_idx — otherwise
        // the close emits `_ret.to_string()` referencing an undeclared
        // `_ret` (the per-op emit at line ~1189 only fires when
        // `return_idx == Some(vnr)`; an all-void block has return_idx=None
        // and skips the declaration).
        let has_ncc_skip_free_temp = matches!(bl.result, Type::Text(_))
            && return_idx.is_some()
            && operators.iter().any(|op| {
                matches!(op.unspan(), Value::Set(v, _) if
                    self.data.def(self.def_nr).variables().name(*v).starts_with("__ncc_")
                    && self.data.def(self.def_nr).variables().is_skip_free(*v))
            });
        let has_trailing_void =
            return_idx.is_some_and(|i| i < last_op_idx) || has_ncc_skip_free_temp;
        // If the captured "return value" DIVERGES — it is a `Return(…)` OR a
        // Block/Insert whose TAIL is a `Return` — we emit it directly and skip the
        // `_ret` tail.  loft#500: a `??`-value-block that allocates (`(v[i] ?? E{…})
        // .f ?? d`) gets `scopes::free_vars`' Set+free+Return dance INSIDE the ncc
        // block, so the block ends in `return …` but is NOT itself a `Return` node;
        // the `.unspan()`-only check missed it, so `let _ret = { … return … }; then
        // `Str::new(_ret)` emitted a dead, wrong-typed (`Str` vs `-> String`) tail
        // (E0308).  `.tail()` descends Block/Insert/Span to the divergence.
        let return_value_is_return = has_trailing_void
            && return_idx.is_some_and(|i| {
                matches!(operators[i].unspan(), Value::Return(_))
                    || matches!(operators[i].tail(), Value::Return(_))
            });
        // @PLN157 § V-n — the header frames this block's view bindings pushed, popped
        // before the block closes.
        let mut view_frames = 0usize;
        // @PLN157 § V-x — the open FLAT literal group, if any: `(local, witness)`.
        let mut flat_lit_open: Option<(u16, u16)> = None;
        for (vnr, v) in operators.iter().enumerate() {
            // DX-source-map: surface line comments at the
            // statement-list level so rustc errors map back to .loft
            // source.  Without this, only Value::Line nodes inside an
            // expression context get rendered (rare in practice).
            if let Value::Line(line) = v {
                let file = self.data.def(self.def_nr).position().file.replace('\n', "");
                // @PLN157 — see the sibling in `output_code_node`: the statement-level
                // half of the same position stream.
                self.ckpt_cur_line = *line;
                self.indent(w)?;
                writeln!(w, "// loft:{file}:{line}")?;
                continue;
            }
            // @PLN157 § V-x (`@FR-R-LitHoist`) — an invariant loop-body literal builds
            // ONCE: the declaration statement (its pre-evals included — an OpDatabase
            // hoisted out of the guard would re-clear the store per iteration) runs only
            // while the local is unbound.  The local was pre-declared at function top, so
            // the binding survives iterations and re-entries.
            // A FLAT group closes at its first non-member statement.
            if let Some((fv, fvdb)) = flat_lit_open
                && !super::hoist::flat_lit_member(
                    v,
                    fv,
                    fvdb,
                    self.data,
                    self.data.def(self.def_nr).variables(),
                )
            {
                flat_lit_open = None;
                self.indent -= 1;
                self.indent(w)?;
                writeln!(w, "}}")?;
            }
            // …and opens at its witness's OpDatabase.
            if flat_lit_open.is_none()
                && let Value::Call(d, cargs) = v.unspan()
                && self.data.def(*d).name() == "OpDatabase"
                && let Some(Value::Var(vdb)) = cargs.first().map(Value::unspan)
                && let Some(fv) = self.invariant_lits.flat.get(vdb).copied()
            {
                flat_lit_open = Some((fv, *vdb));
                let name = sanitize(self.data.def(self.def_nr).variables().name(fv));
                self.indent(w)?;
                writeln!(
                    w,
                    "if var_{name}.store_nr == u16::MAX || var_{name}.rec == 0 {{ //@PLN157 § V-x invariant literal built once"
                )?;
                self.indent += 1;
            }
            // @PLN157 § V-z (`@FR-R-ElemFirst`) — the element-first overrides: a paired
            // temp's declaration becomes the element mint (first temp) plus a bind to
            // the element's own field slot, and the append site loses its reservation,
            // mint, paired handle-zeros and paired copies (scalar sets and the finish
            // stay — the finish is the length bump that keeps the element invisible
            // until the append).
            if !self.elem_first.pairs.is_empty() {
                let dvars = self.data.def(self.def_nr).variables();
                let named = |d: &u32, n: &str| {
                    (*d as usize) < self.data.definitions.len() && self.data.def(*d).name() == n
                };
                let uv = |x: Option<&Value>| match x.map(Value::unspan) {
                    Some(Value::Var(w)) => Some(*w),
                    _ => None,
                };
                let ui = |x: Option<&Value>| match x.map(Value::unspan) {
                    Some(Value::Int(n)) => Some(*n),
                    _ => None,
                };
                let mut handled = false;
                match v.unspan() {
                    // The temp's declaration site.
                    Value::Call(d, args) if named(d, "OpDatabase") => {
                        if let Some(vdb) = uv(args.first())
                            && let Some(&pi) = self.elem_first.by_vdb.get(&vdb)
                        {
                            let pair = &self.elem_first.pairs[pi];
                            let b = pair
                                .binds
                                .iter()
                                .find(|b| b.vdb == vdb)
                                .expect("by_vdb names a bind");
                            let outn = sanitize(dvars.name(pair.out));
                            let elmn = sanitize(dvars.name(pair.elm));
                            let tmpn = sanitize(dvars.name(b.tmp));
                            self.indent(w)?;
                            if b.first {
                                writeln!(
                                    w,
                                    "{{vector::pre_alloc_vector(&(var_{outn}), (1_i64) as u32, ({}_i64) as u32, &mut stores.allocations);}}; var_{elmn} = OpNewRecord(cell, var_{outn}, {}_i32, 65535_i32); //@PLN157 § V-z element minted at the declaration",
                                    pair.prealloc_size, pair.out_tp
                                )?;
                                self.indent(w)?;
                            }
                            writeln!(
                                w,
                                "var_{tmpn} = DbRef {{ store_nr: var_{elmn}.store_nr, rec: var_{elmn}.rec, pos: var_{elmn}.pos + {} }}; //@PLN157 § V-z field-slot bind",
                                b.field_off
                            )?;
                            handled = true;
                        }
                    }
                    // The declaration's bind and length reset are replaced above.
                    Value::Set(_, x)
                        if matches!(x.unspan(), Value::Call(d, cargs)
                            if named(d, "OpGetField")
                                && uv(cargs.first())
                                    .is_some_and(|u| self.elem_first.by_vdb.contains_key(&u))) =>
                    {
                        handled = true;
                    }
                    Value::Call(d, args)
                        if named(d, "OpSetInt4")
                            && uv(args.first())
                                .is_some_and(|u| self.elem_first.by_vdb.contains_key(&u)) =>
                    {
                        handled = true;
                    }
                    // The append site's reservation (only when the next statement is
                    // the suppressed mint of an admitted element).
                    Value::Call(d, args) if named(d, "OpPreAllocVector") => {
                        if ui(args.get(1)) == Some(1)
                            && let Some(next) = operators[vnr + 1..]
                                .iter()
                                .find(|o| !matches!(o.unspan(), Value::Line(_)))
                            && let Value::Set(e2, m2) = next.unspan()
                            && self.elem_first.by_elm.contains_key(e2)
                            && matches!(m2.unspan(), Value::Call(md, margs)
                                if named(md, "OpNewRecord")
                                    && uv(margs.first()) == uv(args.first()))
                        {
                            handled = true;
                        }
                    }
                    // The append site's mint.
                    Value::Set(e2, m2)
                        if self.elem_first.by_elm.contains_key(e2)
                            && matches!(m2.unspan(), Value::Call(md, _) if named(md, "OpNewRecord")) =>
                    {
                        handled = true;
                    }
                    // Paired handle-zeros and paired copies on the element.
                    Value::Call(d, args)
                        if named(d, "OpSetInt4")
                            && uv(args.first())
                                .and_then(|e| self.elem_first.by_elm.get(&e))
                                .is_some_and(|&pi| {
                                    ui(args.get(1)).is_some_and(|off| {
                                        self.elem_first.pairs[pi]
                                            .binds
                                            .iter()
                                            .any(|b| b.field_off == off)
                                    })
                                }) =>
                    {
                        handled = true;
                    }
                    Value::Call(d, args)
                        if named(d, "OpAppendVector")
                            && matches!(args.first().map(Value::unspan), Some(Value::Call(gd, gargs))
                            if named(gd, "OpGetField")
                                && uv(gargs.first())
                                    .and_then(|e| self.elem_first.by_elm.get(&e))
                                    .is_some_and(|&pi| {
                                        ui(gargs.get(1)).is_some_and(|off| {
                                            self.elem_first.pairs[pi]
                                                .binds
                                                .iter()
                                                .any(|b| b.field_off == off)
                                        })
                                    })) =>
                    {
                        handled = true;
                    }
                    _ => {}
                }
                if handled {
                    continue;
                }
            }
            let lit_guard = match v.unspan() {
                Value::Set(var, _)
                    if self.invariant_lits.wrapped.contains(var)
                        && tail_capture.is_none_or(|(_, ri)| ri != vnr) =>
                {
                    Some(*var)
                }
                _ => None,
            };
            if let Some(gv) = lit_guard {
                let name = sanitize(self.data.def(self.def_nr).variables().name(gv));
                self.indent(w)?;
                writeln!(
                    w,
                    "if var_{name}.store_nr == u16::MAX || var_{name}.rec == 0 {{ //@PLN157 § V-x invariant literal built once"
                )?;
                self.indent += 1;
            }
            // Ref-return tail-call capture: `return __native_tail_ret;` in
            // place of the Return(Null)'s null-sentinel emission.  No pre_evals
            // — the Return(Null) itself references no vars.
            if let Some((_, ret_idx)) = tail_capture
                && vnr == ret_idx
            {
                // When the placeholder return wraps a scope-exit free
                // (`Return(OpFreeText(j))` — a struct-or-null arm holding a heap
                // local; see `detect_ref_tail_capture`), run that free here, after
                // the value was captured into __native_tail_ret and before the
                // return.  Emitting `return <free>` would return the free's `()`
                // (rustc E0069); emitting it as a statement keeps the cleanup.
                if let Value::Return(inner) = v.unspan()
                    && Self::free_op_var(inner.unspan(), self.data).is_some()
                {
                    self.indent(w)?;
                    self.output_code_inner(w, inner.unspan())?;
                    writeln!(w, ";")?;
                }
                self.indent(w)?;
                writeln!(w, "return __native_tail_ret;")?;
                continue;
            }
            // O7: pre-compute format-segment count so that text assignments at the
            // start of a format-string block (Set(var, Text)) and OpClearStackText/
            // OpClearText can emit a with_capacity hint when ≥ 2 format/append ops follow.
            self.next_format_count = match v {
                Value::Set(var, boxed)
                    if matches!(**boxed, Value::Text(_))
                        && matches!(
                            self.data.def(self.def_nr).variables().tp(*var),
                            crate::data::Type::Text(_)
                        ) =>
                {
                    count_format_ops(operators, vnr + 1, self.data)
                }
                Value::Call(d, _) => {
                    let name = self.data.def(*d).name();
                    if name == "OpClearStackText" || name == "OpClearText" {
                        count_format_ops(operators, vnr + 1, self.data)
                    } else {
                        0
                    }
                }
                _ => 0,
            };
            // Collect pre-evaluations needed for this operator (to avoid double
            // mutable borrow of stores when user-defined functions are nested).
            // NOTE: indent is incremented here to match the level used in
            // output_code_with_subst below, so multi-line block pre_codes match.
            let counter_before = self.counter;
            self.indent += 1;
            let pre_evals = self.collect_pre_evals(v)?;
            self.indent -= 1;
            let counter_after_collect = self.counter;
            for (name, _, bind_code, _, _) in &pre_evals.entries {
                self.indent(w)?;
                writeln!(w, "let {name} = {bind_code};")?;
            }
            // Make the hoisted bindings the active source of truth for this
            // statement's emission: output_code_inner now substitutes each
            // hoisted node by address.  Saved/restored so nested blocks (emitted
            // during this statement) can install their own without clobbering ours.
            let saved_pre_eval = std::mem::replace(&mut self.active_pre_eval, pre_evals.name_map());
            // Restore counter to the value it had when the pre-eval code was generated
            // so that output_code_with_subst regenerates the same inner _pre_N names
            // as those stored in the pre-eval strings (counter desync fix).
            let restore_counter = pre_evals
                .entries
                .iter()
                .map(|(_, _, _, c, _)| *c)
                .max()
                .unwrap_or(self.counter);
            self.counter = restore_counter;
            self.indent(w)?;
            // Restore counter so the buffer-check pass in output_code_with_subst
            // produces the same counter values as collect_pre_evals did above.
            self.counter = counter_before;
            if has_trailing_void && return_idx == Some(vnr) {
                // If the captured "return value" DIVERGES (a `Return(…)`, or a
                // Block/Insert whose tail is a `Return` — loft#500), emitting
                // `let _ret = { … return … };` binds `_ret: !` and the trailing
                // `Str::new(_ret)` is a dead, wrong-typed tail (E0308).  Emit the
                // value directly instead; the function exits via its own return.
                // Uses the block-level `return_value_is_return` so this direct-emit
                // and the tail-skip at the block close stay in lock-step.
                if return_value_is_return {
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    writeln!(w, ";")?;
                    // All remaining operators are unreachable — skip trailing void tail.
                    // (We break here; the loop over subsequent ops continues but they
                    //  are free-ops which emit nothing harmful under allow(unreachable_code).)
                } else {
                    // loft#1493 — a multi-statement `Insert` in VALUE position needs braces,
                    // for the same reason the `return` path one screen up gives: it emits
                    // `stmt; stmt; <expr>`, so `let _ret = stmt; stmt; <expr>;` binds `_ret`
                    // to the FIRST statement and leaves the rest dangling.  Where that first
                    // statement is a void `if` the binding is `()` and the block fails to
                    // compile (`expected DbRef, found ()`); where it is not, it would bind the
                    // wrong value.  A delivery into the return buffer is exactly this shape —
                    // clear, append, yield the buffer — so it arrives here whenever a block's
                    // scope frees follow its value tail.
                    let block_braces = matches!(v.unspan(), Value::Insert(ops) if ops.len() >= 2);
                    write!(w, "let _ret = ")?;
                    if block_braces {
                        write!(w, "{{ ")?;
                    }
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    if block_braces {
                        write!(w, " }}")?;
                    }
                    writeln!(w, ";")?;
                }
            } else {
                let is_return_expr =
                    !is_void_block && !has_trailing_void && return_idx == Some(vnr);
                let is_tail_capture_call =
                    tail_capture.is_some_and(|(call_idx, _)| vnr == call_idx);
                // When OpCreateStack is the tail expression of a non-void block, the
                // op itself emits nothing at runtime (it's a stack-slot no-op), but
                // the block must return the mutable reference.  Emit `&mut var_<name>`
                // directly rather than delegating to output_call which writes nothing.
                if is_return_expr
                    && let Value::Call(d_nr, args) = v.unspan()
                    && self.data.def(*d_nr).name() == "OpCreateStack"
                    && let [Value::Var(nr)] = args.as_slice()
                {
                    let vname = sanitize(self.data.def(self.def_nr).variables().name(*nr));
                    writeln!(w, "&mut var_{vname}")?;
                } else {
                    // A `Value::Return(...)` already emits its own `return …`
                    // (typed for the function signature), so wrapping it in
                    // `Str::new(...)` would produce `Str::new(return Str::new(X))`
                    // which fails Rust type-check.  Same reasoning for narrow
                    // int casts: the return statement carries the right type.
                    //
                    // P208 (plan-17 phase 01 follow-up): the same redundancy
                    // applies when the value is a `Value::Block` whose tail
                    // expression is a `Value::Return` (recursively).  The
                    // inner Return handles its own scratch.push wrap; the
                    // outer wrap_result wrap then surrounds an unreachable
                    // expression (the Block's tail has type `!`), which
                    // rustc rejects with E0282 because `to_string()` can't
                    // be inferred on the never type.  Walk through Blocks
                    // and Spans to detect tail-Return.
                    // Pass-3: `Value::tail` also descends Insert — scopes
                    // wraps a tail return in `Insert([frees…, Return])`,
                    // which the old hand-rolled walker missed.
                    let value_is_return = matches!(v.tail(), Value::Return(_));
                    let wrap_result = is_return_expr && is_text_result && !value_is_return;
                    // Iterator-next blocks (name "iter next" / "sorted iter next")
                    // return their element value OR `i64::MIN` as the
                    // end-of-iteration sentinel.  Wrapping the result in
                    // `as u16` / `as u8` for narrow element types truncates
                    // `i64::MIN` to `0`, destroying the sentinel — the
                    // subsequent `!op_conv_bool_from_int(var_x)` break check
                    // compares `0 != i64::MIN` → true, inverted to false,
                    // never breaking.  `for x in vector<u16>` then loops
                    // forever printing `x=0`.  Skip the narrow cast for
                    // iterator-next blocks so `i64::MIN` survives intact;
                    // the consuming variable assignment applies its own
                    // `as i64` widening which is a no-op for i64 values.
                    let is_iter_next = bl.name.contains("iter next");
                    let narrow_cast = if is_return_expr && !value_is_return && !is_iter_next {
                        block_tail_cast(&bl.result, is_fn_body)
                    } else {
                        None
                    };
                    // P205 (plan-09 phase 07): when this function returns
                    // Type::Text but has NO `Type::RefVar(Type::Text(_))`
                    // attribute (i.e. text_return didn't add a proper
                    // work buffer — happens for bounded-generic
                    // specialisations and a few other text-return paths),
                    // a plain `Str::new(<value>)` wrap captures a borrow
                    // into a local String that drops at function return,
                    // dangling the returned `Str`'s raw pointer.  Route
                    // through `stores.scratch` instead so the value's
                    // backing String lives as long as `stores` does.
                    // A work buffer is a HIDDEN `RefVar(Text)` attribute (see
                    // `return_buffer_name`, loft#1338).
                    let needs_p205_scratch = wrap_result && {
                        let def = self.data.def(self.def_nr);
                        matches!(def.returned(), Type::Text(_))
                            && (def.text_work_buffers() == 0
                                // @PLAN52 cluster VI (2026-05-30): closures (and
                                // other functions with a `__work_ret: &mut String`
                                // attribute) declare the buffer but the closure
                                // body's `??` value-block doesn't write into it —
                                // the body emits `return Str::new(<value-block>)`
                                // where the inner block tail materialises a String
                                // via `_ret.to_string()` (cluster I iteration 2).
                                // Plain `Str::new(String)` fails E0308 ("expected
                                // &str, found String").  Route through scratch so
                                // the materialised String lives in `stores.scratch`
                                // (program-lifetime) and `Str::new` reads from
                                // there.  Detect by the `__ncc_*` skip_free temp
                                // signature.
                                || self.block_contains_ncc_skip_free(bl))
                    };
                    // @PLN10 Phase A — a bufferless ("nwb") user text fn returns
                    // an owned `String` (its wrapper is `-> String`), so its
                    // body-tail emits `(tail).to_string()`, not a `Str` wrap.
                    // Checked before `needs_p205_scratch` (which is also true for
                    // nwb fns via the no-work-buffer arm).
                    let tail_outer_owned =
                        wrap_result && super::def_returns_owned_text(self.data.def(self.def_nr));
                    if is_tail_capture_call {
                        // Wrap the captured value in a block.  A tail call whose
                        // argument carries a store-lifetime "lift" pre-eval emits that
                        // lift as a LEADING `{ … };` statement (it reassigns the lifted
                        // temp), and its own `;` would otherwise terminate this `let`
                        // early — binding `__native_tail_ret` to the lift's `()` and
                        // detaching the real call (E0308, the ztserve blocker).  The
                        // block makes the lift a statement and the call the tail expr,
                        // so `__native_tail_ret` binds the call's result.  Harmless for
                        // lift-free tails: `{ n_error_frame(…) }` is just a tail expr.
                        write!(w, "let __native_tail_ret: DbRef = {{ ")?;
                    } else if tail_outer_owned {
                        write!(w, "(")?;
                    } else if needs_p205_scratch {
                        // Plan-07 phase 4 — pre-bind the body's text
                        // result into a local before calling
                        // `stores.scratch.push(...)` so the inner
                        // expression's mutable borrow of `stores`
                        // (e.g. `stores.vec_ref_or_raise_runtime(...)`,
                        // and any other `stores.X` call introduced
                        // by the C66 production-mode helpers) doesn't
                        // overlap with the `stores.scratch.push(...)`
                        // borrow.  Without the pre-bind, rustc rejects
                        // with E0499.  Move the resulting String into
                        // `stores.scratch`, then return a Str pointing
                        // into the scratch entry.  The
                        // `(value).to_string()` coerces &str / String
                        // / Str all into an owned String.
                        write!(w, "{{ let _tmp = (")?;
                    } else if wrap_result {
                        write!(w, "Str::new(")?;
                    } else if narrow_cast.is_some() {
                        write!(w, "(")?;
                    }
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    if is_tail_capture_call {
                        // Close the block opened above; the call is its tail expr.
                        write!(w, " }}")?;
                    } else if tail_outer_owned {
                        write!(w, ").to_string()")?;
                    } else if needs_p205_scratch {
                        // @PLN10 Phase B — buffer-write (see If-Return path).
                        if let Some(buf) = self.return_buffer_name() {
                            write!(
                                w,
                                ").to_string(); *var_{buf} = _tmp; Str::new(&*var_{buf}) }}"
                            )?;
                        } else {
                            // @PLN10 D/G1 — dead branch: `needs_p205_scratch` here
                            // means a non-nwb text return (nwb is handled by the
                            // `outer_owned` arm above), and a non-nwb text fn ALWAYS
                            // has a `RefVar(Text)` work buffer, so `return_buffer_name`
                            // is never `None`.  Panic loudly rather than emit
                            // `stores.scratch` into generated code (the field is being
                            // retired).  Whole-suite `=panic` = zero proves this is
                            // unreached.
                            unreachable!(
                                "non-nwb text return without a work buffer \
                                 (return_buffer_name() == None) — @PLN10 invariant violated"
                            );
                        }
                    } else if wrap_result {
                        write!(w, ")")?;
                    } else if let Some(cast) = narrow_cast {
                        write!(w, ") as {cast}")?;
                    }
                    if is_return_expr {
                        writeln!(w)?;
                    } else {
                        writeln!(w, ";")?;
                    }
                }
            }
            // Restore counter to the state after collect_pre_evals so the next
            // operator gets fresh, non-conflicting pre-eval names.
            self.counter = counter_after_collect;
            // Restore the enclosing statement's pre-eval map (empty at top level).
            self.active_pre_eval = saved_pre_eval;
            if lit_guard.is_some() {
                self.indent -= 1;
                self.indent(w)?;
                writeln!(w, "}}")?;
            }
            if self.bind_view_header(w, operators, vnr)? {
                view_frames += 1;
            }
        }
        if flat_lit_open.is_some() {
            self.indent -= 1;
            self.indent(w)?;
            writeln!(w, "}}")?;
        }
        for _ in 0..view_frames {
            self.vec_headers.pop();
        }
        if has_trailing_void && !return_value_is_return {
            self.indent(w)?;
            if is_text_result {
                writeln!(w, "Str::new(_ret)")?;
            } else if matches!(bl.result, Type::Text(_)) {
                // @P321e / @P323 — a TEXT value-block's `_ret` is typically a
                // `&str` borrowing a block-local (the `??`/#ncc block's inner
                // `_ncc` String; a format-string work buffer; etc.).  Yielding
                // the borrow lets the consumer's `.to_string()` run AFTER the
                // local drops at the block's `}` — rustc E0597 ("does not live
                // long enough"), or a dangling raw ptr at runtime.  Materialise
                // to an OWNED String inside the block (where the local is still
                // alive); `.to_string()` accepts &str / String / Str alike.
                writeln!(w, "_ret.to_string()")?;
            } else if let Some(cast) = block_tail_cast(&bl.result, is_fn_body) {
                writeln!(w, "_ret as {cast}")?;
            } else {
                writeln!(w, "_ret")?;
            }
        } else if !is_void_block && return_idx.is_none() {
            // Non-void block with all-void operators (e.g. dynamic dispatch where all code
            // paths use explicit `return`).  Emit a typed default so Rust accepts the
            // function signature; this line is unreachable at runtime.
            self.indent(w)?;
            if is_text_result {
                writeln!(w, "Str::new(loft::state::STRING_NULL)")?;
            } else if let Some(cast) = block_tail_cast(&bl.result, is_fn_body) {
                writeln!(w, "0 as {cast}")?;
            } else {
                writeln!(w, "{}", default_native_value(&bl.result))?;
            }
        }
        if move_pair.is_some() {
            self.active_move_vars.pop();
            // @PLN157 § V-j (`@FR-R-MoveAppend`) — no free here: the placed record stays
            // for an enclosing loop to REUSE (the callee's entry clear resets its length)
            // and dies WITH ITS HOST STORE — `OpFreeRefEmitter` injects the record-level
            // release before every free of the host `__vdb` (early dead-after-last-read
            // sites included, the soundness line the 2026-09-11 gate corruption drew) and
            // the buffer attr's own scope-end free is the null-guarded backstop that
            // covers an adopted `__retbuf` host, which no in-function site frees.
        }
        if adopt_delivery {
            self.in_adopt_delivery -= 1;
        }
        self.indent(w)?;
        write!(
            w,
            "}} /*{}_{}: {}*/",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, self.data.def(self.def_nr).variables())
        )?;
        Ok(())
    }
}
