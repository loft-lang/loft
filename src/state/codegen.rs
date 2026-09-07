// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I65 — Bytecode code generator

//! Bytecode generation: lowers the `Value` IR tree into flat bytecode.
//!
//! Each function's IR (produced by the parser) is walked by
//! [`State::def_code`] which emits operator words into `State.bytecode`.
//! The emitted bytecode is a flat `Vec<u32>` indexed by code position;
//! each operator is one or more words (opcode + operands).
//!
//! Key helpers: `gen_set` (assignment), `gen_call` (function call),
//! `gen_format` (format strings), `gen_block` / `gen_if` / `gen_for`
//! (control flow).

use super::State;
use crate::data::{Context, Data, Deps, I32, IntegerSpec, Type, Value};
use crate::data_store::ValueType;
use crate::database::Stores;
use crate::ir_node::{IrBlock, IrNode, IrNodeList};
use crate::stack::Stack;
#[cfg(debug_assertions)]
use crate::variables::Function;
use crate::variables::size;
use std::collections::HashSet;

/// The callee's argument SLOTS and the definition's ATTRIBUTES are one list, in one order.
///
/// A call site lowers its argument list from the definition's attributes; the callee places its
/// slots in [`crate::variables::Function::arguments`] order, which is variable-NUMBER order.
/// Nothing made the two agree. When a return-buffer promotion retires one argument variable and
/// makes a later-numbered one the argument in its place, they stop agreeing — and a `CallRef`
/// then writes the closure into the return buffer's slot while the body reads its `__closure`
/// from the buffer's. The result is a wrong answer with no diagnostic: a zeroed record, or a
/// captured integer reading 0 (loft#1188, and the collection leg of loft#1178 beside it).
///
/// **The condition is a name disagreement AND a type disagreement at the same position**, and
/// each half rules out a benign case the other admits:
///
/// - names agreeing settles it, whatever the types say. A `__vdb_N` backing-store work-ref is
///   spelled `main_vector<T>` as a VARIABLE and `vector<T>` as an ATTRIBUTE — two spellings of
///   one slot, not two arguments.
/// - types agreeing settles it too. Two hidden scratch buffers of the same type are
///   interchangeable — the caller pushes two empty `&text`s and the callee fills whichever it
///   was handed — and the stdlib's `text::resolve` carries exactly that pair in swapped order
///   and answers correctly on both backends.
///
/// What is left is a slot holding a value of another NAME and another SHAPE, which no spelling
/// difference explains and every read of which reads someone else's bytes.
///
/// Only a definition whose argument count matches its attribute count is compared. A `#rust` /
/// `#native` body has attributes and no variable table at all, and a promoted `text` parameter is
/// deliberately redirected to a shadow local, so a LENGTH mismatch is one of those rather than
/// this. Deps are stripped before comparing: they record where a value came from, not what
/// storage the slot is.
fn check_argument_geometry(
    function: &crate::variables::Function,
    data: &Data,
    def_nr: u32,
    args: &[u16],
) {
    let attrs = data.def(def_nr).attributes();
    if attrs.len() != args.len() {
        return;
    }
    let mut bad: Option<usize> = None;
    for (i, (v, a)) in args.iter().zip(attrs.iter()).enumerate() {
        if function.name(*v) != a.name && function.tp(*v).without_deps() != a.typedef.without_deps()
        {
            bad = Some(i);
            break;
        }
    }
    let Some(i) = bad else { return };
    let by_slot: Vec<&str> = args.iter().map(|v| function.name(*v)).collect();
    let by_attr: Vec<&str> = attrs.iter().map(|a| a.name.as_str()).collect();
    panic!(
        "argument geometry: `{}` places its argument slots in variable order ({}) while a call \
         site lowers the attribute order ({}) — position {i} holds a `{}` where the call site \
         writes a `{}`, so every read of that slot reads another argument's bytes",
        data.def(def_nr).name(),
        by_slot.join(", "),
        by_attr.join(", "),
        function.tp(args[i]).name(data),
        attrs[i].typedef.name(data),
    );
}

/// Stored-tuple element offset — the byte offset of element `idx`
/// inside the synthetic `__tuple<…>` struct as laid out by the
/// storage routine.  Looks up the struct's post-finish field
/// position so stored-tuple reads use the SAME field offset that
/// `OpSetInt` / `OpGetInt` would use for an ordinary struct field.
///
/// Falls back to the alignment-aware `data::element_stack_offsets`
/// calculation when the synthetic struct hasn't been registered
/// yet (e.g. very early parse stages before `tuple_def` runs).
/// In practice the caller path always reaches this AFTER
/// `tuple_def` has fired, so the fallback is defensive only.
fn stored_tuple_field_offset(data: &Data, database: &Stores, elems: &[Type], idx: usize) -> u16 {
    if let Some(offsets) = crate::data::stored_tuple_offsets(data, database, elems) {
        return offsets[idx];
    }
    // Defensive fallback: stack-width offsets (atomic-aware after the
    // 2026-04-28 element_stack_offsets update).  Reaches this branch only
    // when the synthetic struct hasn't been registered or finish()
    // hasn't run — rare paths during early parse / partial state.
    crate::data::element_stack_offsets(elems)[idx] as u16
}

/// The byte offset of element `idx` in a `&(…)` REFERENCE tuple.
///
/// A reference tuple points into the caller's STACK FRAME — the caller passes
/// `OpCreateStack(t)`, whose DbRef targets the frame rather than a data store (see
/// `OpCreateStack` in `default/01_code.loft`). So the element sits at its STACK offset, and
/// this is deliberately NOT [`stored_tuple_field_offset`], which answers the synthetic
/// `__tuple<…>` struct's RECORD field positions.
///
/// The two agree for scalars and disagree for anything wider on the stack than in a record,
/// which is what hid the bug: `(integer, integer)` is `[0, 8]` under both, while
/// `(text, text)` is `[0, 4]` as a record and `[0, 16]` on the stack. Reading element 1 of a
/// text pair at the record offset lands 12 bytes early, inside element 0 — an out-of-bounds
/// dereference, not a wrong value (loft#1006).
///
/// One datum must have one derivation; the plain (non-reference) `TupleGet`/`TuplePut`
/// branches already read `element_stack_offsets`, and these now agree with them.
fn ref_tuple_field_offset(elems: &[Type], idx: usize) -> u16 {
    u16::try_from(crate::data::element_stack_offsets(elems)[idx]).unwrap_or(u16::MAX)
}

/// Text-returning natives that accept a destination buffer instead of allocating one.
pub(crate) fn is_text_dest_native(name: &str) -> bool {
    matches!(
        name,
        "t_4text_replace"
            | "t_4text_to_lowercase"
            | "t_4text_to_uppercase"
            // @PLN10 — text producers with `_dest` variants (registered in
            // `native.rs FUNCTIONS`).  `as_text` is INCLUDED (Phase 2): text-null
            // is content-based ("\0"), so a dest record carries it — the premise
            // that "a dest can't represent null" was probe-falsified on both
            // backends.
            | "n_source_dir"
            // #635 — os_temp_dir / os_cache_dir (the private natives temp_dir /
            // cache_dir wrap); non-null text, "" only on a filesystem-less target.
            | "n_os_temp_dir"
            | "n_os_cache_dir"
            | "n_kernel_event_payload"
            | "n_kernel_sync_payload"
            | "n_kernel_client_event_payload"
            | "n_kernel_client_sync_payload"
            | "n_kernel_default_host"
            | "n_kernel_rebuild_artifact"
            | "n_json_errors"
            | "t_9JsonValue_kind"
            | "t_9JsonValue_to_json"
            | "t_9JsonValue_to_json_pretty"
            // @PLN10 Phase 1 — always-non-null `#rust`-template producers.
            | "n_ymd_days_ago"
            | "n_store_memory"
            // @PLN129 arc C — always-non-null: "" IS the healthy answer.
            | "n_store_lazy_error"
            // @PLN10 Phase 1 batch 2 — always-non-null codegen_runtime producers.
            | "i_parse_errors"
            | "n_struct_to_json"
            | "n_struct_to_json_pretty"
            // @PLN10 Phase 1 — par-text buffer reader (interp; native is owned
            // String already, native par-text loops are blocked by #273).
            | "n_parallel_buf_get_text"
            // @PLN10 Phase 2 — as_text (null carried as the "\0" sentinel content).
            | "t_9JsonValue_as_text"
            // @PLN10 Phase 2 — env_variable (non-null; os_variable owns its String).
            | "n_env_variable"
            // host_input — non-null program-input reader (stdin / --html host).
            | "n_host_input"
            // text_from_bytes — owned text decoded from a vector<u8>.
            | "n_text_from_bytes"
    )
}

/// @PLN10 N2b — a **cdylib FFI** text call: an EXTERNAL `#native "symbol"`
/// function returning text.  Built-in text natives are `#pure` / `#rust` (empty
/// `def.native`); only loaded cdylib functions carry a non-empty native symbol,
/// so this is disjoint from `is_text_dest_native` (no `default/` decl uses
/// `#native`).  Such calls dispatch through the runtime bridge, which is
/// dest-passed via `n_set_bridge_dest` + `gen_cdylib_text_dest_call` instead of
/// `stores.scratch`.
pub(crate) fn is_cdylib_text_call(def: &crate::data::Definition) -> bool {
    !def.native().is_empty() && matches!(def.returned(), Type::Text(_))
}

/// @PLN24 arc D — a **`#c` binding** returning text: a C `char *` brought back
/// as a loft `text`.
///
/// Routed exactly like [`is_cdylib_text_call`] (`n_set_bridge_dest` +
/// `gen_cdylib_text_dest_call`), and for the same reason: a body-less external
/// declaration has no `_dest` sibling to call and no promoted work buffer of its
/// own, so the destination has to be handed over as a separate step. Disjoint
/// from both neighbours — a `#c` definition carries no `native` symbol (arc A
/// leaves it empty so the Rust dispatch path cannot claim it) and no `default/`
/// declaration carries a `c_sig`.
pub(crate) fn is_c_text_call(def: &crate::data::Definition) -> bool {
    // `.base()` peels `Optional`: a `text?` return uses the same owned-text ABI,
    // and for a `char *` it is the spelling that lets the null-flow analysis see
    // the NULL the crossing carries either way.
    !def.c_sig.is_empty() && matches!(def.returned().base(), Type::Text(_))
}

impl State {
    /**
    Define byte code for a function.
    # Panics
    when code cannot be output.
    */
    pub fn def_code(
        &mut self,
        def_nr: u32,
        data: &mut Data,
        program_store: Option<&(Stores, crate::keys::DbRef)>,
    ) {
        let logging = !crate::portable_path::is_stdlib_source(&data.def(def_nr).position().file);
        let console = false; //logging;
        let mut stack = Stack::new(data.def(def_nr).variables().clone(), data, def_nr, logging);
        // @PLN11 G2/M6 — read the body's SHAPE (null / empty-block) from the
        // persistent store when present, so these last native body reads are
        // also store-backed; else from the native graph.
        let (body_is_null, body_is_empty_block) = if let Some((stores, root)) = program_store {
            let b = IrNode::Store(stores, crate::ir_read::def_body_node(stores, *root, def_nr));
            (
                b.kind() == ValueType::Null,
                b.kind() == ValueType::Block && b.as_block().operators().is_empty(),
            )
        } else {
            let code = stack.data.def(def_nr).code();
            (
                *code == Value::Null,
                matches!(code, Value::Block(bl) if bl.operators.is_empty()),
            )
        };
        if body_is_null {
            // B7 (2026-04-13): set up the same frame layout the regular
            // path uses, so `add_return` emits a correct OpReturn.  For
            // native fns declared as `pub fn name(...) -> T;` (no body),
            // the variables list is empty so `function.arguments()`
            // returns nothing — the regular path's `self.arguments` /
            // `stack.position` accumulator is never run, and we used to
            // emit `Return(ret=stale, discard=0)` with a leftover
            // `self.arguments` from the previous function.  This caused
            // native methods with struct-enum self (e.g.
            // `fn len(self: JsonValue) -> integer;`) to loop forever in
            // `Return(ret=12, value=4, discard=0)` because the runtime's
            // unwind landed past the saved return-PC slot.  Compute the
            // arg-frame size from the def's attributes (which exist for
            // native fns even when variables don't) and account for the
            // 4-byte return-PC slot.
            // @PLAN53 cluster 2 / S4: each arg + the return-address slot
            // occupies a STEPPED span in aligned mode (step is identity
            // when LOFT_ALIGN is off → V1 unchanged).
            let mut args_size: u16 = 0;
            for attr in stack.data.def(def_nr).attributes() {
                args_size += stack.step(size(&attr.typedef, &Context::Argument));
            }
            self.arguments = args_size;
            stack.position = args_size + stack.step(4); // args + return-address slot
            let start = self.code_pos;
            self.add_return(&mut stack, start);
            data.definitions[def_nr as usize].code_position = start;
            data.definitions[def_nr as usize].code_length = self.code_pos - start;
            return;
        }
        let is_empty_stub = body_is_empty_block;
        // use arguments() instead of names map lookup — the names map may
        // redirect promoted text parameters to shadow locals.
        let args = stack.function.arguments();
        check_argument_geometry(&stack.function, stack.data, def_nr, &args);
        for v in &args {
            stack.function.set_stack_pos(*v, stack.position);
            // @PLAN53 cluster 2 / S4: stepped arg span (identity when off).
            stack.position += stack.step(size(stack.function.tp(*v), &Context::Argument));
        }
        let start = self.code_pos;
        self.arguments = stack.position;
        stack.position += stack.step(4); // keep space for the code return address
        if is_empty_stub {
            // loft#1254 — a stub with an empty body still has to RETURN something, and
            // `add_return` only copies `size(return_type)` bytes off the frame: with
            // nothing pushed it copied whatever the stack happened to hold.  That is the
            // one answer `formal/operational.md` forbids outright — "never … whatever the
            // hardware happened to leave in the register" — and it was not even stable
            // (three runs of one program gave three integers, and a struct return indexed
            // a record by garbage and panicked).
            //
            // The value a type takes when nothing chooses one is `Data::to_default`, the
            // one home for `construct_default` (@FR-D-Scalar / D-Text / D-Coll / D-Enum /
            // D-Rec / D-Opt) — the same answer an omitted struct field and a
            // default-initialised local already take.  A type that HAS no default
            // (@FR-D-NoRef) keeps the old path rather than inventing one.
            let stub_ret = stack.data.def(def_nr).returned().clone();
            if stub_ret != Type::Void && stack.data.has_default(&stub_ret).is_ok() {
                // A HANDLE-carried return needs a well-formed `DbRef`, and `to_default`
                // answers `Value::Null` for one — which pushes none of the twelve bytes
                // `add_return` then copies, so the caller read a garbage handle and
                // indexed a record by it ("index out of bounds: the len is 2 but the
                // index is 8658").  `OpNullRefSentinel` is the no-allocation spelling of
                // that null (`DbRef::NULL`), which is what `--native` already returns
                // here, so the two backends agree.
                //
                // Whether a NON-nullable heap return should answer null or a
                // default-constructed record is loft#1254 cell 2's question, not this
                // one: that cell decides the policy for every non-nullable return, and
                // this site follows it.  What is settled either way is that reading an
                // unwritten frame is not an answer.
                let heap_ret = matches!(
                    stub_ret.base(),
                    Type::Reference(_, _)
                        | Type::Vector(_, _)
                        | Type::Sorted(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Radix(_, _, _)
                        | Type::Trie(_, _, _)
                        | Type::Enum(_, true, _)
                );
                let dflt = if heap_ret {
                    Value::Call(stack.data.def_nr("OpNullRefSentinel"), Vec::new())
                } else {
                    crate::data::to_default(&stub_ret, stack.data)
                };
                self.generate(&dflt, &mut stack, false);
            }
            self.add_return(&mut stack, start);
            data.definitions[def_nr as usize].code_position = start;
            data.definitions[def_nr as usize].code_length = self.code_pos - start;
            return;
        }
        // Plan-04 Phase B.3 atomic bundle: single function-entry
        // `OpReserveFrame(frame_hwm)` covers every local's slot.
        // `frame_hwm` is the maximum slot-end across all non-argument
        // placed locals (computed from V1's slot assignments).
        // Per-block `OpReserveFrame(block.var_size)` is deleted in
        // `generate_block`; first-Set helpers now write positional-init
        // or push-then-`OpPut*` into the pre-reserved slots.
        // @PLAN53 cluster 2 / S4: round the frame high-water mark to 8 so the
        // first eval-TOS push above the reserved frame is 8-aligned (identity
        // when off).
        let frame_hwm = stack.step(stack.function.frame_hwm(&Context::Variable));
        if frame_hwm > stack.position {
            let reserve = frame_hwm - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(reserve);
            stack.position += reserve;
        }
        // #260 Fix B (interpreter twin of the native `__vdb` prologue):
        // sentinel-init every `__vdb` slot at function entry, so an
        // early-return scope-exit free that `lastuse_reclaim` left ABOVE the
        // store's relocated null-init frees a null sentinel (`OpFreeRef`
        // no-ops on it) instead of reading uninitialized slot bytes (a
        // garbage `DbRef` → allocation-table index panic).  The relocated
        // null-init + `OpDatabase` still run at their IR position, so
        // allocation timing (the watermark benefit) is unchanged.
        for v in 0..stack.function.count() {
            // Slotless (`stack == u16::MAX`) vars are skipped: the allocator
            // drops unused vars — e.g. a stale table entry from a #339 extra
            // parse pass whose final lowering no longer touches it — and a
            // var with no frame slot has nothing to sentinel-init (no free
            // can reference it either).
            if !stack.function.is_argument(v)
                && stack.function.name(v).starts_with("__vdb")
                && stack.function.stack(v) != u16::MAX
            {
                let slot_offset = stack.var_pos(v);
                stack.add_op("OpInitRefSentinel", self);
                self.code_add(slot_offset);
            }
        }
        if console {
            println!("{} ", stack.data.def(def_nr).header(stack.data, def_nr));
            stack.data.dump(def_nr);
        }
        let mut started = HashSet::new();
        for a in stack.data.def(def_nr).variables().arguments() {
            started.insert(a);
        }
        // Optional IR dump: set LOFT_IR=<name-filter> (or LOFT_IR=* for all user fns).
        if let Ok(filter) = std::env::var("LOFT_IR") {
            let fn_name = stack.data.def(def_nr).name();
            let want_all = filter.is_empty() || filter == "*";
            let matches = want_all || filter == fn_name || fn_name.contains(&*filter);
            if matches && logging {
                eprintln!("=== IR: {fn_name} ===");
                #[cfg(debug_assertions)]
                print_ir(
                    stack.data.def(def_nr).code(),
                    stack.data,
                    &stack.function,
                    0,
                );
                #[cfg(not(debug_assertions))]
                {
                    let mut w = Vec::new();
                    let mut vars = stack.function.clone();
                    let _ = stack.data.show_code(
                        &mut w,
                        &mut vars,
                        stack.data.def(def_nr).code(),
                        0,
                        true,
                    );
                    eprint!("{}", String::from_utf8_lossy(&w));
                }
                eprintln!();
                eprintln!("===");
            }
        }
        self.source = stack.data.def(def_nr).source();
        // @PLN11 G2/M2/M5 — store-backed lowering.  When `byte_code_from`
        // supplied a persistent program store, read this function's body node
        // directly from it (`def_body_node`) and lower it through
        // `IrNode::Store`; `generate_inner` reads the body via the handle and the
        // Definition table via `stack.data` (native), so the emitted bytecode is
        // identical to the native path.  No store → the native lowering runs.
        if let Some((stores, root)) = program_store {
            let body = IrNode::Store(stores, crate::ir_read::def_body_node(stores, *root, def_nr));
            self.generate_node(body, &mut stack, true);
        } else {
            self.generate(stack.data.def(def_nr).code(), &mut stack, true);
        }
        let mut stack_pos = Vec::new();
        let mut skip_free_vars = Vec::new();
        for v_nr in 0..stack.function.next_var() {
            stack_pos.push(stack.function.stack(v_nr));
            if stack.function.is_skip_free(v_nr) {
                skip_free_vars.push(v_nr);
            }
        }
        data.definitions[def_nr as usize].code_position = start;
        data.definitions[def_nr as usize].code_length = self.code_pos - start;
        if let Some(v) = self.calls.get(&def_nr) {
            let old = self.code_pos;
            for pos in v.clone() {
                // `pos` IS the address of the i64 target operand — recorded at
                // emission rather than re-derived from the opcode, which is not a
                // fixed width (loft#1032).
                self.code_pos = pos;
                self.code_add(i64::from(start));
            }
            self.code_pos = old;
        }
        // @PLN120 A — every slotted USER local must have produced a store span.
        // `assign_slots_v2` gives a slot only to a variable with a live interval, and
        // `compute_intervals` derives one only from a `Set` or a `TuplePut` — the two
        // arms that record.  A miss means a new slot-writing lowering appeared without
        // a recording site, which would render that local `<unset>` in every paused
        // frame forever.  Loud here beats silent there.
        //
        // **Compiler work-refs (`__`-prefixed) are exempt, and that is measured, not
        // assumed.** The hidden return buffer `__ref_1` holds a slot with no recorded
        // span in 488 of the `tests/scripts` corpus — its slot is claimed and
        // sentinel-inited by the function preamble rather than by an IR `Set`, so the
        // interval-derived premise above does not cover it.  The consequence is benign:
        // no span means `<unset>`, which is conservative, and `__`-prefixed names are
        // filtered out of the displayed frame anyway.  Asserting over them would be
        // asserting something false.
        //
        // Anything a user can name is still in scope, `#`-infixed loop temps included
        // (`i#index` is a real `Set` in the IR) — those are what the check is for.
        #[cfg(debug_assertions)]
        {
            let vars = &data.definitions[def_nr as usize].variables;
            for (v_nr, &pos) in stack_pos.iter().enumerate() {
                let v = v_nr as u16;
                if pos == u16::MAX || vars.is_argument(v) || vars.name(v).starts_with("__") {
                    continue;
                }
                assert!(
                    self.store_spans
                        .iter()
                        .any(|&(s, _, w)| w == v && s >= start && s < self.code_pos),
                    "@PLN120 A: local '{}' of {} holds slot {pos} but codegen recorded \
                     no store span for it — a slot-writing lowering is missing a \
                     `record_store_span` call",
                    vars.name(v),
                    data.definitions[def_nr as usize].name,
                );
            }
        }
        for (v_nr, pos) in stack_pos.into_iter().enumerate() {
            data.definitions[def_nr as usize]
                .variables
                .set_stack(v_nr as u16, pos);
        }
        // Propagate skip_free flags set during codegen (e.g. S34 Option A) so that
        // validate_slots can recognise intentional slot aliases and not report them
        // as conflicts.
        for v_nr in skip_free_vars {
            data.definitions[def_nr as usize]
                .variables
                .set_skip_free(v_nr);
        }
        #[cfg(any(debug_assertions, test))]
        crate::variables::validate_slots(
            &data.definitions[def_nr as usize].variables,
            data,
            def_nr,
            // The V2 allocator is the ONLY allocator (@PLAN53) — every
            // slot here is scope-blind-placed, so I7 (the V1 zone-frame
            // invariant) must be skipped, exactly as the scopes.rs call
            // site already does.  This site kept enforcing it after V1's
            // deletion and false-positived on legitimately zone-2-placed
            // small vars (e.g. the stdlib's `_reduce_acc_N`) in any
            // armed build.
            true,
        );
    }

    /**
    Generate the byte code equivalent of a function definition
    # Panics
    On not implemented Value constructions
    */
    /// The `&Value` recursion entry — wraps the native node in an [`IrNode`]
    /// and delegates to [`Self::generate_node`].  Every existing caller keeps
    /// this signature; @PLN11 G2/M5's store-backed entry will call
    /// `generate_node(IrNode::Store(…), …)` directly instead.
    pub(super) fn generate(&mut self, val: &Value, stack: &mut Stack, top: bool) -> Type {
        self.generate_node(IrNode::Native(val), stack, top)
    }

    /// Handle-based recursion entry (@PLN11 G2): depth-guard + dispatch on the
    /// backing-agnostic [`IrNode`].
    pub(super) fn generate_node(&mut self, node: IrNode, stack: &mut Stack, top: bool) -> Type {
        self.generate_depth += 1;
        assert!(
            self.generate_depth <= 1000,
            "expression nesting limit exceeded at depth {}",
            self.generate_depth
        );
        let result = self.generate_inner(node, stack, top);
        self.generate_depth -= 1;
        result
    }

    /// @PLN11 G2 — lower one IR node.  Dispatches entirely on `node.kind()`
    /// and reads payloads through `IrNode` accessors, so the SAME codegen runs
    /// once the backing flips from native to store-read (M5).  The remaining
    /// `as_native()` calls (Block/Loop/TupleGet/TuplePut children and the
    /// `&Value`-taking `gen_*` helpers) are the native-backed bridges M5 lifts.
    #[allow(clippy::too_many_lines)]
    fn generate_inner(&mut self, node: IrNode, stack: &mut Stack, top: bool) -> Type {
        match node.kind() {
            ValueType::Int => {
                stack.add_op("OpConstInt", self);
                self.code_add(node.int_value());
                I32.clone()
            }
            ValueType::Long => {
                stack.add_op("OpConstInt", self);
                self.code_add(node.int_value());
                crate::data::I64.clone()
            }
            ValueType::Single => {
                stack.add_op("OpConstSingle", self);
                self.code_add(node.single_value());
                Type::Single
            }
            ValueType::Float => {
                stack.add_op("OpConstFloat", self);
                self.code_add(node.float_value());
                Type::Float
            }
            ValueType::Boolean => {
                stack.add_op(
                    if node.bool_value() {
                        "OpConstTrue"
                    } else {
                        "OpConstFalse"
                    },
                    self,
                );
                Type::Boolean
            }
            ValueType::Enum => {
                let (ord, tp) = node.enum_pair();
                self.types.insert(self.code_pos, tp);
                stack.add_op("OpConstEnum", self);
                self.code_add(ord);
                Type::Enum(0, false, Deps::none())
            }
            ValueType::Text => self.gen_text(node.text(), stack),
            ValueType::Var => self.generate_var(stack, node.var_nr()),
            ValueType::Break => self.gen_break(node.break_nr(), stack),
            ValueType::Continue => self.gen_continue(node.continue_nr(), stack),
            // Keys: already part of the search request — nothing to emit.
            ValueType::Keys => Type::Null,
            // Null: e.g. an `else` clause without code.
            ValueType::Null => Type::Void,
            ValueType::Line => {
                self.line_numbers.insert(self.code_pos, node.line_nr());
                // @PLN10 D/G4 — the former `OpClearScratch` emission here never
                // fired (`library_names` never resolved the opcode name), which is
                // why `scratch` was never cleared.  The field is retired (G5), so
                // the dead emit is removed.
                Type::Void
            }
            // @PLN11 G2 — single-child / compound delegating arms: dispatch +
            // children flow through the handle to IrNode-taking helpers
            // (generate_set materialises at its boundary).
            ValueType::Set => {
                let v = node.set_var();
                let from = self.code_pos;
                self.generate_set(stack, v, node.set_inner());
                self.record_store_span(from, v);
                Type::Void
            }
            ValueType::Return => self.gen_return(node.return_inner(), stack),
            ValueType::Drop => self.gen_drop(node.drop_inner(), stack),
            ValueType::If => self.gen_if(node.if_cond(), node.if_then(), node.if_else(), stack),
            ValueType::Yield => {
                // CO1.3c: emit the yielded expression, then OpCoroutineYield.
                let t = self.generate_node(node.yield_inner(), stack, false);
                let value_size = crate::variables::size(&t, &crate::data::Context::Argument);
                stack.add_op("OpCoroutineYield", self);
                self.code_add(value_size);
                // CO1.3d: the yield suspends and transfers the value to the
                // caller; the eval stack is empty again on resume, so undo the
                // push.  @PLAN53 S4: the yielded value occupied a stepped span.
                stack.position -= stack.step(value_size);
                Type::Void
            }
            // @PLN11 G2 — list-child + scalar arms.  Insert/Tuple iterate the
            // handle directly; Call/CallRef/Parallel pass an IrNodeList to their
            // helpers (generate_call materialises its param list at the boundary).
            ValueType::Insert => {
                for op in node.insert_items().iter() {
                    self.generate_node(op, stack, false);
                }
                Type::Void
            }
            ValueType::Call => self.generate_call(stack, node.call_to(), node.call_args()),
            ValueType::CallRef => {
                self.generate_call_ref(stack, node.callref_var(), node.callref_args())
            }
            ValueType::Tuple => {
                // T1.4: generate each element onto contiguous stack slots.
                let mut types = Vec::new();
                // @PLN114 — "contiguous" here means the PACKED layout every reader
                // uses (`element_stack_offsets`: a `DbRef` element occupies 12B), but each
                // element is pushed by an op whose own stack step is 8-rounded (16B
                // for a `DbRef`).  Nothing reconciles the two, so a tuple built here
                // and read by a callee silently disagrees about where element i is:
                // `(P,P)` as a call argument segfaults, `(P,P,P,P)` returns
                // `2,null,0,360287970323857408`.  Record where each element actually
                // lands and check it against the layout the readers assume.
                #[cfg(debug_assertions)]
                let push_base = stack.position;
                #[cfg(debug_assertions)]
                let mut landed: Vec<u16> = Vec::new();
                for e in node.tuple_items().iter() {
                    #[cfg(debug_assertions)]
                    landed.push(stack.position.saturating_sub(push_base));
                    types.push(self.generate_node(e, stack, false));
                }
                // Only a tuple consumed DIRECTLY as a callee frame block has to be
                // packed.  One bound to a local is relocated element-by-element by
                // `emit_tuple_put_ops`, so its eval-stack placement is scratch.
                #[cfg(debug_assertions)]
                if self.in_call_arg {
                    let want = crate::data::element_stack_offsets(&types);
                    for (i, got) in landed.iter().enumerate() {
                        debug_assert_eq!(
                            usize::from(*got),
                            want[i],
                            "@PLN114 [{caller}]: tuple element {i} lands at +{got}B but \
                             every reader expects +{expect}B (element_stack_offsets for \
                             {types:?}) — the element push advances by the 8-rounded \
                             stack step while the layout is packed",
                            caller = stack.data.def(stack.def_nr).name(),
                            expect = want[i],
                        );
                    }
                    // The frame reserves the STEPPED size, so compare against that —
                    // an over-reserve of the trailing pad is not a defect.
                    let total = usize::from(stack.position.saturating_sub(push_base));
                    let reserved =
                        usize::from(stack.step(crate::data::element_stack_size(&Type::Tuple(
                            types.clone(),
                        )) as u16));
                    debug_assert_eq!(
                        total,
                        reserved,
                        "@PLN114 [{caller}]: tuple {types:?} occupies {total}B on the \
                         stack but its readers reserve {reserved}B",
                        caller = stack.data.def(stack.def_nr).name(),
                    );
                }
                Type::Tuple(types)
            }
            ValueType::Parallel => {
                self.gen_parallel(node.parallel_arms(), stack);
                Type::Void
            }
            ValueType::FnRefDnr => {
                // P215: project the d_nr (first 8B of the fn-ref slot) via
                // OpVarInt — its dispatcher reads 8B regardless of declared type.
                let v_pos = stack.var_pos(node.fnref_dnr_var());
                stack.add_op("OpVarInt", self);
                self.code_add(v_pos);
                crate::data::I64.clone()
            }
            // @PLN11 G2/M3.4 — Span passthrough + the two never-reachable
            // rewrite placeholders.
            ValueType::Span => {
                // Plan-07 phase 1 step 1.20 — record the pc RANGE this construct
                // occupies against its source position, so phase 3's runtime-error
                // printer can surface `at file:line:col` for a fault inside it.
                //
                // The end is only knowable here, on the far side of lowering the
                // inner node, and it is what lets the lookup tell a pc inside this
                // construct from one past it.  Without it every pc inherited the
                // nearest preceding span, so a fault in an unwrapped statement was
                // reported at an unrelated earlier one (loft#1262).
                let start = self.code_pos;
                // The published snapshot is now behind the table it snapshots.
                self.published_spans = None;
                let inner = self.generate_inner(node.span_inner(), stack, top);
                self.source_spans
                    .insert(start, (node.span_pos(), self.code_pos));
                inner
            }
            ValueType::Iter => {
                panic!("Iter node should have been rewritten before codegen")
            }
            // @PLN11 G2/M3.5 — FnRef on the stack: [d_nr i64][closure DbRef],
            // 8 + 12 B (P249 widened d_nr from 4).  add_op already advances
            // stack.position.
            ValueType::FnRef => {
                stack.add_op("OpConstInt", self);
                self.code_add(i64::from(node.fnref_dnr()));
                let clos_var = node.fnref_clos_var();
                // @P328 — a non-capturing closure has no closure variable; push
                // a null DbRef sentinel instead (else `stack(u16::MAX)` panics).
                if clos_var == u16::MAX {
                    stack.add_op("OpNullRefSentinel", self);
                } else {
                    let clos_pos = stack.var_pos(clos_var);
                    stack.add_op("OpVarRef", self);
                    self.code_add(clos_pos);
                }
                node.fnref_type()
            }
            // @PLN11 G2 — Block/Loop lower through the IrBlock handle; TupleGet/
            // TuplePut read their fields via scalar/child accessors.  Every arm
            // of generate_inner now reads only through the handle — it is fully
            // store-capable (M5 just constructs IrNode::Store at the entry).
            // @PLN120 A — a block's children are emitted sequentially, so the pc
            // window either side of the walk IS the scope's extent, and a nested
            // block lands inside it.  Recorded here rather than inside the two
            // helpers so both kinds go through one shape.
            ValueType::Loop => {
                let b = node.as_block();
                let (scope, from) = (b.scope(), self.code_pos);
                let tp = self.gen_loop(b, stack);
                self.record_scope_span(from, scope);
                tp
            }
            ValueType::Block => {
                let b = node.as_block();
                let (scope, from) = (b.scope(), self.code_pos);
                let tp = self.generate_block(stack, b, top);
                self.record_scope_span(from, scope);
                tp
            }
            ValueType::TupleGet => {
                let var_nr = node.tupleget_var();
                let elem_idx = node.tupleget_idx();
                let tuple_tp = stack.function.tp(var_nr).clone();
                // T1.5: RefVar(Tuple) — read element through the DbRef
                // using the SAME OpGetInt / OpGetFloat / OpGet… opcodes
                // that ordinary struct-field access uses.  The field
                // offset comes from the synthetic `__tuple<…>` struct's
                // post-finish field position (computed by
                // `calculate_positions_with_groups` for the registered
                // LinkedFieldGroup) — NOT from a tuple-specific path.
                // This guarantees stored-tuple element access is
                // byte-coherent with the layout the storage routine
                // reserved.
                if let Type::RefVar(ref inner) = tuple_tp
                    && let Type::Tuple(ref elems) = **inner
                {
                    let idx = elem_idx as usize;
                    let elem_tp = elems[idx].clone();
                    // Look up the synthetic struct's field position via
                    // the LinkedFieldGroup, falling back to the legacy
                    // `element_stack_offsets` calculation for shapes whose
                    // synthetic struct hasn't been registered (defensive —
                    // tuple_def is normally called eagerly during parse).
                    let elem_offset = ref_tuple_field_offset(elems, idx);
                    let var_pos = stack.var_pos(var_nr);
                    let code_pos = self.code_pos;
                    stack.add_op("OpVarRef", self);
                    self.code_add(var_pos);
                    match elem_tp.base() {
                        Type::Integer(_) | Type::Function(_, _, _) => {
                            stack.add_op("OpGetInt", self);
                        }
                        Type::Float => stack.add_op("OpGetFloat", self),
                        Type::Single => stack.add_op("OpGetSingle", self),
                        Type::Character => stack.add_op("OpGetCharacter", self),
                        Type::Boolean => stack.add_op("OpGetBoolean", self),
                        Type::Enum(_, false, _) => stack.add_op("OpGetEnum", self),
                        // Unreachable: `ref_tuple_element_ok` refuses anything this
                        // match cannot emit, at the signature, with a message naming the
                        // element type.  The two lists are one list on purpose —
                        // loft#1006 was them disagreeing.
                        _ => panic!("RefTupleGet: unsupported element type {elem_tp:?}"),
                    }
                    self.code_add(elem_offset);
                    return self.insert_types(elem_tp, code_pos, stack);
                }
                // T1.4: read element elem_idx from tuple variable var_nr.
                let Type::Tuple(ref elems) = tuple_tp else {
                    panic!("TupleGet on non-tuple variable");
                };
                let idx = elem_idx as usize;
                let elem_tp = elems[idx].clone();
                let offsets = crate::data::element_stack_offsets(elems);
                let elem_offset = offsets[idx] as u16;
                // The element is at tuple_var_stack_pos + elem_offset.
                // Compute distance from current stack top to that position.
                let tuple_var_pos = stack.function.stack(var_nr);
                let elem_abs_pos = tuple_var_pos + elem_offset;
                let var_pos = stack.position - elem_abs_pos;
                let code_pos = self.code_pos;
                if let Type::Tuple(inner_elems) = &elem_tp {
                    // Plan-06 phase 4d: nested tuple element — push
                    // each inner leaf primitive recursively from the
                    // outer tuple variable's slot.  `elem_abs_pos` is
                    // the inner tuple's base stack offset (computed
                    // from the outer var's slot + outer offset).  The
                    // helper walks inner offsets and emits one OpVar*
                    // per leaf.
                    self.emit_tuple_var_push_recursive(stack, inner_elems, elem_abs_pos);
                    return self.insert_types(elem_tp.clone(), code_pos, stack);
                }
                match elem_tp.base() {
                    Type::Integer(_) => {
                        stack.add_op("OpVarInt", self);
                    }
                    // P249 — fn-ref slots are 20 B (8 B d_nr + 12 B
                    // closure DbRef); OpVarInt would only push 8 B and
                    // truncate the closure half.  OpVarFnRef pushes
                    // the full 20 B (with the same +4 stack tracker
                    // bump generate_var uses for plain Function vars).
                    Type::Function(_, _, _) => {
                        stack.add_op("OpVarFnRef", self);
                        // @PLAN53 S4: 20B fn-ref push − 16B `text` account.
                        stack.position += stack.fnref_signature_gap();
                    }
                    Type::Boolean => stack.add_op("OpVarBool", self),
                    Type::Float => stack.add_op("OpVarFloat", self),
                    Type::Single => stack.add_op("OpVarSingle", self),
                    Type::Character => stack.add_op("OpVarCharacter", self),
                    Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
                    Type::Text(_) => stack.add_op("OpArgText", self),
                    Type::Reference(c, _) | Type::Enum(c, true, _) => {
                        self.types
                            .insert(self.code_pos, stack.data.def(*c).known_type());
                        stack.add_op("OpVarRef", self);
                    }
                    Type::Vector(_, _)
                    | Type::Hash(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Radix(_, _, _)
                    | Type::Trie(_, _, _)
                    | Type::Sorted(_, _, _)
                    | Type::Iterator(_, _) => {
                        // Collection-typed stack element: 12-byte DbRef
                        // pointing at the collection record in the
                        // host's store.  Same opcode as the Reference
                        // arm above.
                        stack.add_op("OpVarRef", self);
                    }
                    _ => panic!("TupleGet: unsupported element type {elem_tp:?}"),
                }
                self.code_add(var_pos);
                self.insert_types(elem_tp.clone(), code_pos, stack)
            }
            ValueType::TuplePut => {
                // @PLN120 A — a tuple-element write is a store to `var_nr`'s slot,
                // so it records a store span like `Set` does; the arm has three
                // exits and each records before leaving (the debug-build audit in
                // `def_code` is what keeps that honest).
                let from = self.code_pos;
                let var_nr = node.tupleput_var();
                let elem_idx = node.tupleput_idx();
                let value = node.tupleput_inner();
                let tuple_tp = stack.function.tp(var_nr).clone();
                // T1.5: RefVar(Tuple) — write element through the DbRef using
                // the SAME OpSetInt / OpSetFloat / OpSet… opcodes that
                // ordinary struct-field assignment uses.  Field offset comes
                // from the synthetic `__tuple<…>` struct's post-finish field
                // position (computed by `calculate_positions_with_groups`
                // for the registered LinkedFieldGroup) — NOT from a tuple-
                // specific path.  Mirrors the TupleGet RefVar branch above.
                if let Type::RefVar(ref inner) = tuple_tp
                    && let Type::Tuple(ref elems) = **inner
                {
                    let idx = elem_idx as usize;
                    let elem_tp = elems[idx].clone();
                    let elem_offset = ref_tuple_field_offset(elems, idx);
                    let var_pos = stack.var_pos(var_nr);
                    stack.add_op("OpVarRef", self);
                    self.code_add(var_pos);
                    self.generate_node(value, stack, false);
                    match elem_tp.base() {
                        Type::Integer(_) | Type::Function(_, _, _) => {
                            stack.add_op("OpSetInt", self);
                        }
                        Type::Float => stack.add_op("OpSetFloat", self),
                        Type::Single => stack.add_op("OpSetSingle", self),
                        Type::Character => stack.add_op("OpSetCharacter", self),
                        Type::Boolean => stack.add_op("OpSetBoolean", self),
                        Type::Enum(_, false, _) => stack.add_op("OpSetEnum", self),
                        // Unreachable — see the RefTupleGet arm above.
                        _ => panic!("RefTuplePut: unsupported element type {elem_tp:?}"),
                    }
                    self.code_add(elem_offset);
                    self.record_store_span(from, var_nr);
                    return Type::Void;
                }
                // T1.4: write to element elem_idx of tuple variable var_nr.
                let Type::Tuple(ref elems) = tuple_tp else {
                    panic!("TuplePut on non-tuple variable");
                };
                let idx = elem_idx as usize;
                let elem_tp = elems[idx].clone();
                let offsets = crate::data::element_stack_offsets(elems);
                let elem_offset = offsets[idx] as u16;
                // Generate the value to write.  A `Function` element's put-op below is
                // `OpPutFnRef`, which pops the full 20-byte pair, so the push has to be
                // the pair too — the same decision the FIRST-Set path makes in
                // `gen_set_first_at_tos` and the tuple LITERAL makes for loft#1069.  A
                // bare `inc` lowers to the lone d_nr and pushed only eight, so the pop ran
                // twelve bytes into whatever sat beneath: `--interpret` panicked
                // `fn_call_ref: fn_var=16 < 20` and `--native` emitted `var_t.0 = 711_i64`
                // against a `(u32, DbRef)` slot, failing with a raw rustc E0308.  A
                // CAPTURING lambda already pushes the pair and is unchanged.
                if matches!(elem_tp.base(), Type::Function(_, _, _)) {
                    self.gen_fn_ref_value_node(value, stack);
                } else {
                    self.generate_node(value, stack, false);
                }
                // Compute distance from stack top to the element's position.
                let tuple_var_base = stack.function.stack(var_nr);
                let elem_abs_pos = tuple_var_base + elem_offset;
                let var_pos = stack.position - elem_abs_pos;
                // P248 — when the destination element is itself a tuple
                // (e.g. nested-LHS writeback `t.0 = w0` where t.0 is
                // `(int, int)`), the value has been pushed by
                // `generate(value, …)` above as a sequence of leaves
                // on the eval stack.  Pop them in reverse and
                // OpPut* each leaf at its offset within
                // `elem_abs_pos`.  Mirrors the read-side
                // `emit_tuple_var_push_recursive` at line 394.
                if let Type::Tuple(inner_elems) = &elem_tp {
                    self.emit_tuple_var_pop_put(stack, inner_elems, elem_abs_pos);
                    self.record_store_span(from, var_nr);
                    return Type::Void;
                }
                match elem_tp.base() {
                    Type::Integer(_) => {
                        stack.add_op("OpPutInt", self);
                    }
                    // P249 — fn-ref slots are 20 B; OpPutInt would only
                    // pop 8 B and leave the closure half (12 B) garbage
                    // in the destination.  OpPutFnRef pops the full
                    // 20 B in matching layout.  The opcode's stdlib
                    // signature pops 16 B (`text`) but the runtime
                    // actually pops 20 B; mirror gen_set_first_at_tos's
                    // `fnref_signature_gap` correction.
                    Type::Function(_, _, _) => {
                        stack.add_op("OpPutFnRef", self);
                        stack.position -= stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
                    }
                    Type::Boolean => stack.add_op("OpPutBool", self),
                    Type::Float => stack.add_op("OpPutFloat", self),
                    Type::Single => stack.add_op("OpPutSingle", self),
                    Type::Character => stack.add_op("OpPutCharacter", self),
                    Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                    // `OpPutText`, like every sibling here and like the three other
                    // element-write matches in this file — NOT `OpAppendText`, which pops a
                    // DIFFERENT amount, so every text-element write lands one index high.
                    //
                    // ⚠ That misalignment is mostly silent: on a `(text, text)`, `t.0 = "X"`
                    // writes `.1`, a write to the LAST element falls off the end with nothing
                    // reported, and only a write onto a non-text neighbour faults. (loft#1004)
                    Type::Text(_) => stack.add_op("OpPutText", self),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
                        stack.add_op("OpPutRef", self);
                    }
                    // …and a KEYED collection is the same 12-byte handle its vector twin is, so
                    // it writes the same way.  Named through `is_keyed` rather than by listing
                    // five variants beside `Vector`, because a site that names some of the
                    // kinds reads as a complete rule and is not one: `hash`, `sorted`, `index`,
                    // `radix` and `trie` all reached the panic below, so assigning any of them
                    // into a tuple element was an internal compiler error on ordinary source
                    // (loft#1225).
                    t if crate::parser::vectors::is_keyed(t) => {
                        stack.add_op("OpPutRef", self);
                    }
                    _ => panic!("TuplePut: unsupported element type {elem_tp:?}"),
                }
                self.code_add(var_pos);
                self.record_store_span(from, var_nr);
                Type::Void
            }
            // Phase 09 phase 00 step 0.7 — RawExpr is created only by native
            // codegen (`src/generation/emit.rs` fn-ref dispatch); bytecode
            // codegen never produces it.
            ValueType::RawExpr => panic!(
                "Value::RawExpr is native-codegen-internal; not reachable from bytecode codegen"
            ),
            // The store-record discriminant is always one of the 34 known Node
            // variants (the materializer writes only those); `Other` marks a
            // corrupt/foreign record.
            ValueType::Other(d) => unreachable!("unknown IR node discriminant {d}"),
        }
    }

    pub(super) fn gen_text(&mut self, value: &str, stack: &mut Stack) -> Type {
        if value.len() < 256 {
            stack.add_op("OpConstText", self);
            self.code_add_str(value);
        } else {
            // Store long strings in CONST_STORE via Store::set_str().
            let const_store = &mut self.database.allocations[crate::database::CONST_STORE as usize];
            let rec = const_store.set_str(value);
            stack.add_op("OpConstStoreText", self);
            self.code_add(i64::from(rec));
            // set_str stores length at (rec, 4); text bytes start at (rec, 8).
            // We encode the record position; the opcode reads length from the store.
            self.code_add(0i64); // pos offset within the record (length is at rec+4)
        }
        Type::Text(Deps::none())
    }

    /// @PLN120 A — record the scope span of the block that has just been emitted:
    /// `from` (the pc before its first child) → the current pc.
    ///
    /// Dropped when it carries no information — a block that emitted nothing, or one
    /// from an uninstantiated generic template, whose blocks never received scope
    /// numbers (`u16::MAX`).  A local whose scope cannot be placed falls back to the
    /// bytecode-reference filter in [`frame_view`](State::frame_view), so a dropped
    /// span costs information, never correctness.
    fn record_scope_span(&mut self, from: u32, scope: u16) {
        if scope != u16::MAX && self.code_pos > from {
            self.scope_spans.push((from, self.code_pos, scope));
        }
    }

    /// @PLN120 A — record the store span of the assignment that has just been
    /// emitted.  The recorded end is the pc *after* the write, so a pause at or past
    /// it may read the slot and a pause before it must not: that is the difference
    /// between showing a local's value and showing `<unset>`.
    fn record_store_span(&mut self, from: u32, var_nr: u16) {
        if self.code_pos > from {
            self.store_spans.push((from, self.code_pos, var_nr));
        }
    }

    pub(super) fn gen_loop(&mut self, lp: IrBlock, stack: &mut Stack) -> Type {
        stack.add_loop(self.code_pos);
        let pos = self.code_pos;
        for v in lp.operators().iter() {
            self.generate_node(v, stack, false);
        }
        self.clear_stack(stack, 0);
        stack.add_op("OpGotoWord", self);
        self.code_add((i64::from(pos) - i64::from(self.code_pos) - 4) as i32);
        stack.end_loop(self);
        Type::Void
    }

    pub(super) fn gen_break(&mut self, loop_nr: u16, stack: &mut Stack) -> Type {
        let old_pos = stack.position;
        self.clear_stack(stack, loop_nr);
        stack.add_op("OpGotoWord", self);
        stack.add_break(self.code_pos, loop_nr);
        self.code_add(0i32); // temporary value to the end of the loop
        stack.position = old_pos;
        Type::Void
    }

    pub(super) fn gen_continue(&mut self, loop_nr: u16, stack: &mut Stack) -> Type {
        let old_pos = stack.position;
        self.clear_stack(stack, loop_nr);
        stack.add_op("OpGotoWord", self);
        self.code_add((i64::from(stack.get_loop(loop_nr)) - i64::from(self.code_pos) - 4) as i32);
        stack.position = old_pos;
        Type::Void
    }

    pub(super) fn gen_if(
        &mut self,
        test: IrNode,
        t_val: IrNode,
        f_val: IrNode,
        stack: &mut Stack,
    ) -> Type {
        self.generate_node(test, stack, false);
        stack.add_op("OpGotoFalseWord", self);
        let code_step = self.code_pos;
        self.code_add(0i32); // temp step
        let true_pos = self.code_pos;
        let stack_pos = stack.position;
        let tp = self.generate_node(t_val, stack, false);
        let true_stack = stack.position;
        // @P356: an EXPRESSION if whose true branch produces a value but whose
        // else is a bare `Value::Null` (e.g. the `expr ?? null` ncc lowering
        // `if bool(tmp) { tmp } else null`) must push a TYPED NULL on the false
        // path — otherwise the goto-false skips straight to the join leaving the
        // value-sized slot unwritten, so when the null branch runs at runtime the
        // stack is short by `size(tp)` and every later slot offset (e.g. the ncc
        // temp's `OpFreeText`) is wrong → frees an uninitialised slot → SIGSEGV.
        // The statement-style fast-path below (no value, or divergent true arm)
        // keeps the original skip-to-join behaviour.
        //
        // #405 / @PLN85: use the actual stack delta (`true_stack != stack_pos`)
        // rather than `tp != Void` alone.  A void block whose last expression
        // returns a non-Void type (e.g. a block ending in `OpAppendVector`, which
        // consumes its two DbRef operands and pushes nothing) leaves `tp` non-Void
        // even though the net stack change is zero.  Using `tp` alone triggers a
        // spurious `ConstInt(i64::MIN)` on the else path, growing the runtime
        // stack by 8 bytes per outer-loop iteration → stale DbRef in ki →
        // SIGSEGV at `OpFreeRef`.  The stack-delta check is the authoritative
        // signal: if no value reached the eval stack, no null sentinel is needed.
        let null_else_value = matches!(f_val.kind(), ValueType::Null)
            && !is_divergent(t_val)
            && !matches!(tp, Type::Void | Type::Never)
            && true_stack != stack_pos;
        if matches!(f_val.kind(), ValueType::Null) && !null_else_value {
            self.code_put(code_step, (self.code_pos - true_pos) as i32); // actual step
            // when the true branch diverges (return/break/continue, possibly
            // wrapped by scopes.rs in Insert/Block), execution only reaches the
            // join point via the goto-false path — where runtime stack_pos equals
            // the pre-if `stack_pos`, not `true_stack`. Without this reset, every
            // subsequent Var/Put encodes a wrong offset and writes corrupt the
            // return-address slot, eventually overflowing the stack store.
            if is_divergent(t_val) {
                stack.position = stack_pos;
            }
        } else if null_else_value {
            // Emit a real false arm that pushes the type's null sentinel, so
            // both arms reach the join with the same stack delta (= true_stack).
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i32); // temp end
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i32); // goto-false → false arm
            stack.position = stack_pos;
            self.emit_typed_null(stack, &tp);
            self.code_put(end, (self.code_pos - false_pos) as i32);
            // emit_typed_null pushes exactly size(tp); both arms now balanced.
            stack.position = true_stack;
        } else {
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i32); // temp end
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i32); // actual step
            stack.position = stack_pos;
            let fp = self.generate_node(f_val, stack, false);
            // @PLN90 P4 — a value-producing `if` whose FALSE arm pushes no result (an empty
            // `_ => { [] }` arm that elides to nothing, the borrowed-yield's else) leaves
            // `false_stack` at the eval base. The B5 join below would then take `target` at
            // the eval base and slide the TRUE arm's result DOWN onto the saved return
            // address (below the eval base) — the P4 derail. Deliver a typed result on the
            // false path so both arms exit ABOVE the frame at the same level; B5 then sees
            // equal levels and never shrinks. (`else ;` reaches here as a non-`Null` arm, so
            // gen_if's null-else pad does not cover it.)
            let false_stack = if !is_divergent(f_val)
                && stack.position == stack_pos
                && true_stack > stack_pos
                && !matches!(tp, Type::Void | Type::Never)
            {
                self.emit_typed_null(stack, &tp);
                stack.position = true_stack;
                true_stack
            } else {
                stack.position
            };
            if !is_divergent(t_val) && !is_divergent(f_val) && true_stack != false_stack {
                // B5: both arms produce a value but exit at different stack levels
                // (e.g. match arms with different local allocations).  The result
                // sits on top of each arm's stack; join at the SHORTER arm's level
                // and make the TALLER arm — whichever it is — free its extra bytes
                // (keeping the result on top) before reaching the join.
                //
                // `free_stack(value, discard)` lands `stack_pos` at
                // `arm_stack - discard + step(value)`, so to reach `target` the
                // discard must be `(arm_stack - target) + step(ret_size)` — the
                // STEP-ROUNDED result size, not the raw size.  A 12-byte vector
                // occupies a 16-byte slot; using the raw size strands the arm 4
                // bytes high, and never shrinking a taller TRUE arm strands it a
                // whole result slot.  Either way the function-tail `OpReturn`'s
                // `discard` (= `stack.position`) ends up short, so `fn_return`
                // misreads the saved return address → the interpreter derails
                // (deterministic on Filled, clean on Empty).  See @PLN85 P2.
                let target = true_stack.min(false_stack);
                let ret_size = size(stack.data.def(stack.def_nr).returned(), &Context::Argument);
                let keep = stack.step(ret_size);
                if false_stack > target {
                    // The false arm is taller; the cursor is at its end, so shrink
                    // it inline, then aim the true arm's goto at the join below.
                    stack.add_op("OpFreeStack", self);
                    self.code_add(ret_size as u8);
                    self.code_add((false_stack - target) + keep);
                    stack.position = target;
                    self.code_put(end, (self.code_pos - false_pos) as i32);
                } else {
                    // The true arm is taller, but its code and join-goto were
                    // already emitted, so it cannot be shrunk in place.  Route it
                    // through a shrink trampoline placed after the false arm: the
                    // false path jumps over the trampoline straight to the join,
                    // the true path lands in it, frees its excess, falls through.
                    stack.add_op("OpGotoWord", self);
                    let skip = self.code_pos;
                    self.code_add(0i32);
                    let skip_base = self.code_pos;
                    self.code_put(end, (skip_base - false_pos) as i32); // true arm → trampoline
                    stack.add_op("OpFreeStack", self);
                    self.code_add(ret_size as u8);
                    self.code_add((true_stack - target) + keep);
                    self.code_put(skip, (self.code_pos - skip_base) as i32); // false arm → join
                }
                stack.position = target;
            } else {
                self.code_put(end, (self.code_pos - false_pos) as i32);
                // when one branch diverges (return/break/continue), use the
                // other branch's stack position. The divergent branch exits the
                // scope so its stack delta is irrelevant at the join point.
                if is_divergent(t_val) {
                    stack.position = false_stack;
                } else if is_divergent(f_val) {
                    stack.position = true_stack;
                }
            }
            if matches!(tp, Type::Never) {
                return fp;
            }
        }
        tp
    }

    /// Generate a fn-ref assignment value, ensuring every branch in an if-else
    /// expression produces a full 16-byte fn-ref slot ([d_nr 4B][closure DbRef 12B]).
    /// A plain branch only pushes 4 bytes (d_nr via OpConstInt); OpNullRefSentinel pads
    /// to 16 bytes.  For if-else, the sentinel must be emitted *inside* each branch so
    /// both paths reach the join point with the same stack delta.
    fn gen_fn_ref_value(&mut self, value: &Value, stack: &mut Stack) {
        if let Value::If(test, t_val, f_val) = value {
            self.generate(test, stack, false);
            stack.add_op("OpGotoFalseWord", self);
            let code_step = self.code_pos;
            self.code_add(0i32);
            let true_pos = self.code_pos;
            let stack_pos = stack.position;
            self.gen_fn_ref_value(t_val, stack);
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i32);
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i32);
            stack.position = stack_pos;
            self.gen_fn_ref_value(f_val, stack);
            self.code_put(end, (self.code_pos - false_pos) as i32);
        } else {
            let before = stack.position;
            self.generate(value, stack, false);
            if stack.position - before < 16 {
                self.emit_push_sentinel(stack);
            }
        }
    }

    /// `IrNode` sibling of [`Self::gen_fn_ref_value`] — same contract, for the arms that
    /// walk handles rather than `Value`s (the `TuplePut` element write).
    ///
    /// A fn-ref value has to arrive as the whole 20-byte pair (8 B d_nr + 12 B closure
    /// DbRef) because the put-op that consumes it pops all twenty.  A non-capturing
    /// source pushes only the d_nr, so it is topped up with the null-closure sentinel.
    /// For an `if`, the sentinel goes INSIDE each branch, so both reach the join with the
    /// same stack delta — a branch pushing eight against a branch pushing twenty is the
    /// shape a delta check taken after the join cannot repair.
    fn gen_fn_ref_value_node(&mut self, node: IrNode, stack: &mut Stack) {
        let node = node.unspan();
        if matches!(node.kind(), ValueType::If) {
            self.generate_node(node.if_cond(), stack, false);
            stack.add_op("OpGotoFalseWord", self);
            let code_step = self.code_pos;
            self.code_add(0i32);
            let true_pos = self.code_pos;
            let stack_pos = stack.position;
            self.gen_fn_ref_value_node(node.if_then(), stack);
            stack.add_op("OpGotoWord", self);
            let end = self.code_pos;
            self.code_add(0i32);
            let false_pos = self.code_pos;
            self.code_put(code_step, (self.code_pos - true_pos) as i32);
            stack.position = stack_pos;
            self.gen_fn_ref_value_node(node.if_else(), stack);
            self.code_put(end, (self.code_pos - false_pos) as i32);
        } else {
            let before = stack.position;
            self.generate_node(node, stack, false);
            if stack.position - before < 16 {
                self.emit_push_sentinel(stack);
            }
        }
    }

    /// Push a tuple literal's members with each `fn(…)` member topped up to the whole
    /// 20-byte pair, recursing into NESTED tuple members.
    ///
    /// The flat version of this was loft#1069.  It read the members of the destination
    /// type against the members of the literal and topped up the ones declared `fn(…)` —
    /// correct, and blind one level in, because a nested tuple member is a `Type::Tuple`
    /// and not a `Type::Function`, so the loop handed the whole inner literal to plain
    /// `generate` and the inner fn member went back to being eight bytes.  Depth was the
    /// axis that fix held fixed: `((dbl, 1), "z")` panicked `fn_var=16 < 20` with no
    /// assignment anywhere in the program.
    ///
    /// Arity is re-checked per level, so a mismatch degrades to the plain push for that
    /// member instead of zipping two different shapes together.
    fn gen_tuple_members_with_fn_refs(
        &mut self,
        elems: &[Type],
        items: &[Value],
        stack: &mut Stack,
    ) {
        for (e, item) in elems.iter().zip(items.iter()) {
            match e.base() {
                Type::Function(_, _, _) => self.gen_fn_ref_value(item, stack),
                Type::Tuple(inner) if inner.iter().any(crate::data::tuple_carries_fn_ref) => {
                    if let Value::Tuple(inner_items) = item.unspan()
                        && inner_items.len() == inner.len()
                    {
                        let inner = inner.clone();
                        self.gen_tuple_members_with_fn_refs(&inner, inner_items, stack);
                    } else {
                        self.generate(item, stack, false);
                    }
                }
                _ => {
                    self.generate(item, stack, false);
                }
            }
        }
    }

    /// #263: pad a fn-ref **return value** to the full 20-byte ABI slot.
    ///
    /// A fn-ref returned as a bare d_nr (e.g. `return dbl`, which lowers to
    /// `Value::Int(d_nr)` via parser/objects.rs) pushes only the 4-byte d_nr,
    /// but the `Function` return ABI is a 20-byte slot ([d_nr 8B][closure DbRef
    /// 12B]).  Without padding, `OpReturn` copies 20 bytes — 12 of them stale
    /// eval-stack garbage — into the caller's fn-ref slot, so the closure half
    /// is wild and a later `OpFreeRef`/`store_mut` dereferences it as a
    /// `store_nr` (OOB / SIGSEGV).  Push a null-ref sentinel so a non-capturing
    /// call-returned fn-ref has a well-formed (null) closure half, mirroring the
    /// block-result padding in `generate_block` and the `f = dbl` assignment
    /// path in `gen_fn_ref_value`.  `before` is `stack.position` captured before
    /// the return value was generated.
    fn pad_fn_ref_return(&mut self, stack: &mut Stack, before: u16) {
        let return_is_fn = matches!(
            stack.data.def(stack.def_nr).returned(),
            Type::Function(_, _, _)
        );
        if return_is_fn && stack.position.saturating_sub(before) < 16 {
            self.emit_push_sentinel(stack);
        }
    }

    pub(super) fn gen_return(&mut self, v: IrNode, stack: &mut Stack) -> Type {
        let before = stack.position;
        self.generate_node(v, stack, false);
        self.pad_fn_ref_return(stack, before);
        let return_type = stack.data.def(stack.def_nr).returned();
        // CO1.3c: generator functions use OpCoroutineReturn instead of OpReturn.
        if matches!(return_type, Type::Iterator(_, _)) {
            // For generators, `return` means exhaust — push null of the yield type.
            let yield_size = if let Type::Iterator(inner, _) = return_type {
                size(inner, &Context::Argument)
            } else {
                0
            };
            stack.add_op("OpCoroutineReturn", self);
            self.code_add(yield_size);
        } else {
            if return_type != &Type::Void {
                let ret_nr = stack.data.type_def_nr(return_type);
                let known = stack.data.def(ret_nr).known_type();
                self.types.insert(self.code_pos, known);
            }
            stack.add_op("OpReturn", self);
            self.code_add(self.arguments);
            self.code_add(size(return_type, &Context::Argument) as u8);
            self.code_add(stack.position);
        }
        Type::Void
    }

    pub(super) fn gen_drop(&mut self, node: IrNode, stack: &mut Stack) -> Type {
        self.generate_node(node, stack, false);
        // get all variables of the current scope.
        // @PLAN53 cluster 2 / S4: the value occupies a stepped eval span;
        // the OpFreeStack discard + the position pop must both be stepped.
        let size = stack.step(stack.size_code(node));
        if size > 0 {
            stack.add_op("OpFreeStack", self);
            self.code_add(0u8);
            self.code_add(size);
        }
        stack.position -= size;
        Type::Void
    }

    /// Generate bytecode for `parallel { arm1; arm2; ... }`.
    ///
    /// Current implementation: sequential execution inline.
    /// The opcodes are emitted for future threading support but act as noops.
    /// Arms run inline in sequence, each dropping its result value.
    /// Generate bytecode for `parallel { arm1; arm2; ... }`.
    ///
    /// Layout:
    ///   `OpParallelBegin(n)`
    ///   `OpParallelArm(off0)` `OpParallelArm(off1)` ...
    ///   `OpParallelJoin` — main thread blocks here
    ///   `OpGotoWord(skip_arms)` — main thread skips arm code
    ///   [arm0 code] `OpReturn`
    ///   [arm1 code] `OpReturn`
    ///   [continue]
    pub(super) fn gen_parallel(&mut self, arms: IrNodeList, stack: &mut Stack) {
        let n = arms.len();
        stack.add_op("OpParallelBegin", self);
        self.code_add(n as u8);
        // OpParallelArm(offset) placeholders
        let mut arm_offset_positions = Vec::with_capacity(n);
        for _ in 0..n {
            stack.add_op("OpParallelArm", self);
            arm_offset_positions.push(self.code_pos);
            self.code_add(0u16);
        }
        // OpParallelJoin — join_pos is code_pos after this opcode
        stack.add_op("OpParallelJoin", self);
        let join_pos = self.code_pos;
        // OpGotoWord past all arm code — main thread skips
        stack.add_op("OpGotoWord", self);
        let goto_skip_pos = self.code_pos;
        self.code_add(0i32);
        let goto_skip_base = self.code_pos;
        // Emit each arm as a separate region ending with OpReturn
        for (i, arm) in arms.iter().enumerate() {
            let arm_start = self.code_pos;
            let offset = (arm_start - join_pos) as u16;
            self.code_put(arm_offset_positions[i], offset);
            self.gen_drop(arm, stack);
            // OpReturn — arm is a void "function"
            stack.add_op("OpReturn", self);
            self.code_add(0u16); // arguments = 0
            self.code_add(0u8); // return_size = 0
            self.code_add(stack.position);
        }
        // Patch skip goto
        let end_pos = self.code_pos;
        self.code_put(goto_skip_pos, (end_pos - goto_skip_base) as i32);
    }

    /// Plan-04 Phase B.2: push a `DbRef{store_nr: u16::MAX, …}` sentinel
    /// onto the eval stack.  Runtime-equivalent to `OpNullRefSentinel`,
    /// decomposed into `OpReserveFrame(12) + OpInitRefSentinel(12)` so
    /// that "advance the stack pointer" and "write the sentinel value"
    /// are distinct primitives.
    fn emit_push_sentinel(&mut self, stack: &mut Stack) {
        // @PLAN53 cluster 2 / S4: a DbRef push occupies a stepped span; the
        // reserve operand, the position advance, and the init slot-offset all
        // use step(ref_size) (identity when LOFT_ALIGN off).
        let step = stack.step(size_of::<crate::keys::DbRef>() as u16);
        stack.add_op("OpReserveFrame", self);
        self.code_add(step);
        stack.position += step;
        stack.add_op("OpInitRefSentinel", self);
        self.code_add(step);
    }

    /// Plan-04 Phase B.2: push a null-state `DbRef` (allocates a store
    /// via `database.null()`) onto the eval stack.  Runtime-equivalent
    /// to `OpConvRefFromNull`, decomposed into `OpReserveFrame(12) +
    /// OpInitRef(12)`.
    fn emit_push_null_ref(&mut self, stack: &mut Stack) {
        // @PLAN53 cluster 2 / S4: stepped DbRef span (see emit_push_sentinel).
        let step = stack.step(size_of::<crate::keys::DbRef>() as u16);
        stack.add_op("OpReserveFrame", self);
        self.code_add(step);
        stack.position += step;
        stack.add_op("OpInitRef", self);
        self.code_add(step);
    }

    /// Plan-04 Phase B.2: push a stack-frame `DbRef` pointing at
    /// `dep_offset` below the pre-op TOS.  Runtime-equivalent to
    /// `OpCreateStack(dep_offset)`, decomposed into `OpReserveFrame(12) +
    /// OpInitCreateStack(12, dep_offset + 12)`.  Caller computes
    /// `dep_offset = stack.position_before - dep.stack_pos`.
    fn emit_push_create_stack(&mut self, stack: &mut Stack, dep_offset: u16) {
        // @PLAN53 cluster 2 / S4: stepped DbRef span; dep_pos shifts by the
        // same stepped advance (dep is now `step` further below the new TOS).
        let step = stack.step(size_of::<crate::keys::DbRef>() as u16);
        stack.add_op("OpReserveFrame", self);
        self.code_add(step);
        stack.position += step;
        stack.add_op("OpInitCreateStack", self);
        self.code_add(step);
        self.code_add(dep_offset + step);
    }

    pub(super) fn gen_set_first_text(&mut self, stack: &mut Stack, v: u16, value: &Value) {
        // Plan-04 Phase B.2: positional init.  Emit OpReserveFrame(n) +
        // OpInitText(slot_offset) instead of the compound OpText.  This
        // decouples "advance the stack pointer" from "write the empty
        // String at a slot".  Net runtime effect is identical to OpText
        // (TOS += size_str, empty String written at (new_TOS - slot_offset)).
        let pos = stack.function.stack(v);
        let size = super::size_str() as u16;
        // Advance TOS so that stack.position >= pos + size (slot fits).
        let slot_end = pos + size;
        if stack.position < slot_end {
            let advance = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(advance);
            stack.position += advance;
        }
        let slot_offset = stack.position - pos;
        stack.add_op("OpInitText", self);
        self.code_add(slot_offset);
        if let Value::Text(s) = value {
            if !s.is_empty() {
                self.set_var(stack, v, value);
            }
        } else {
            self.set_var(stack, v, value);
        }
    }

    pub(super) fn gen_set_first_ref_null(&mut self, stack: &mut Stack, v: u16) {
        // Plan-04 Phase B.2: positional init.  Each branch emits
        // `OpReserveFrame(12) + OpInit*(slot_offset)` instead of the
        // compound push-and-init ops.  Net runtime effect is identical.
        let pos = stack.function.stack(v);
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = pos + ref_size;
        if stack.position < slot_end {
            let advance = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(advance);
            stack.position += advance;
        }
        let slot_offset = stack.position - pos;
        let dep = match stack.function.tp(v).clone() {
            Type::Reference(_, d) | Type::Enum(_, _, d) => d,
            _ => Deps::none(),
        };
        // A `__vdb_N` vector backing allocates at its BUILD site, never here.  That site
        // can be conditional — an `if c { v = [..] }` rebind, or a literal that is one ARM
        // of a branch — and an eager store on the path not taken is never named again and
        // never typed, so nothing can free it and the leak report cannot even say what it
        // held (`kt=65535 ?`).  Enforces @FR-O-Derived: a free is derived from the
        // ownership fact, so a store no binding's fact ever names has no derivable free.
        //
        // The eager store buys nothing on the taken path either — `alloc_record_at`
        // allocates fresh from a null sentinel — and the function prologue already
        // sentinel-inits every `__vdb` slot (`def_code` above).  (loft#1081.)
        let vdb_backing = stack.function.name(v).starts_with("__vdb");
        if dep.is_empty() {
            if vdb_backing || stack.function.is_inline_ref(v) || stack.function.is_skip_free(v) {
                // Inline-ref temporaries must not allocate a database store at null-init
                // time.  A real store is assigned later via OpPutRef when the method
                // returns.  Writes DbRef{store_nr:u16::MAX} at slot; Stores::free
                // treats it as a no-op if the var is never assigned.
                //
                // @PLN85 t2 — the same holds for a `skip_free` var: its store is
                // owned/freed ELSEWHERE (a `&`-param write-back's transfer buffer
                // `__ref_N` — objects.rs:2200 — or a borrowed alias), so an eager
                // owned placeholder here would ORPHAN on the path that never
                // materialises the real store (the `if !r { r = … }` write-back not
                // taken → the interp-only kt=65535 leak; native already lowers null
                // to DbRef::NULL).  Sentinel-init: OpDatabase allocates fresh when
                // the real store IS assigned, and the free is a no-op if it is not.
                stack.add_op("OpInitRefSentinel", self);
                self.code_add(slot_offset);
            } else {
                stack.add_op("OpInitRef", self);
                self.code_add(slot_offset);
            }
        } else {
            // Pre-init a borrowed Reference with a null-state DbRef pointing
            // into dep's slot.  The DbRef uses stack_cur.store_nr (the
            // stack-frame store) — it is NOT a valid data-store pointer and
            // must be overwritten by OpPutRef before any field access.
            let dep_pos = stack.function.stack(dep[0]);
            if dep_pos > stack.position {
                // Dependency not yet on the stack — use a sentinel instead of
                // CreateStack.  The variable will be overwritten by OpPutRef
                // before any field access (e.g. loop variable assigned from
                // iterator next).
                stack.add_op("OpInitRefSentinel", self);
                self.code_add(slot_offset);
            } else {
                stack.add_op("OpInitCreateStack", self);
                self.code_add(slot_offset);
                let dep_offset = stack.position - dep_pos;
                self.code_add(dep_offset);
            }
        }
    }

    pub(super) fn gen_set_first_tuple_null(&mut self, stack: &mut Stack, v: u16) {
        // Plan-04 Phase B.3 atomic bundle: slot-aware.  Push each null
        // element value then OpPut it at the element's absolute slot
        // (mirrors set_var's tuple dispatch around codegen.rs:2339).
        // The function-entry frame reserve ensures every element slot
        // is addressable.
        let Type::Tuple(elems) = stack.function.tp(v).clone() else {
            return;
        };
        let tuple_var_base = stack.function.stack(v);
        self.emit_tuple_null_init(stack, &elems, tuple_var_base);
    }

    /// Recursive helper for `TupleGet` on a `Type::Tuple` element.
    /// The outer tuple variable is already at a known stack base; this
    /// pushes the inner tuple's leaves from the outer variable's slot
    /// at the inner offsets.  Recurses for nested-nested tuples.
    fn emit_tuple_var_push_recursive(&mut self, stack: &mut Stack, elems: &[Type], base: u16) {
        let offsets = crate::data::element_stack_offsets(elems);
        for (i, elem) in elems.iter().enumerate() {
            let elem_abs = base + offsets[i] as u16;
            if let Type::Tuple(inner_elems) = elem {
                self.emit_tuple_var_push_recursive(stack, inner_elems, elem_abs);
                continue;
            }
            let var_pos = stack.position - elem_abs;
            match elem.base() {
                Type::Integer(_) => {
                    stack.add_op("OpVarInt", self);
                }
                // P249 — fn-ref slot is 20 B; OpVarFnRef pushes the
                // full layout (8 B d_nr + 12 B closure DbRef) with a
                // +4 stack tracker bump for the signature mismatch.
                Type::Function(_, _, _) => {
                    stack.add_op("OpVarFnRef", self);
                    stack.position += stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
                }
                Type::Boolean => stack.add_op("OpVarBool", self),
                Type::Float => stack.add_op("OpVarFloat", self),
                Type::Single => stack.add_op("OpVarSingle", self),
                Type::Character => stack.add_op("OpVarCharacter", self),
                Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
                Type::Text(_) => stack.add_op("OpArgText", self),
                Type::Reference(c, _) | Type::Enum(c, true, _) => {
                    self.types
                        .insert(self.code_pos, stack.data.def(*c).known_type());
                    stack.add_op("OpVarRef", self);
                }
                Type::Vector(_, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
                | Type::Sorted(_, _, _)
                | Type::Iterator(_, _) => {
                    stack.add_op("OpVarRef", self);
                }
                other => panic!("Tuple push: unsupported element type {other:?}"),
            }
            self.code_add(var_pos);
        }
    }

    /// Recursive helper for `set_var` on a `Type::Tuple` destination.
    /// The tuple's leaves are already on the eval stack in declaration
    /// order; this walks them in REVERSE so each `OpPut*` pops the
    /// most-recently-pushed leaf and writes it to the corresponding
    /// slot offset within the variable.  For nested `Type::Tuple`
    /// elements, recurses with the inner offsets added to `base`.
    fn emit_tuple_var_pop_put(&mut self, stack: &mut Stack, elems: &[Type], base: u16) {
        let offsets = crate::data::element_stack_offsets(elems);
        for i in (0..elems.len()).rev() {
            let elem_abs = base + offsets[i] as u16;
            if let Type::Tuple(inner_elems) = &elems[i] {
                self.emit_tuple_var_pop_put(stack, inner_elems, elem_abs);
                continue;
            }
            let pos = stack.position - elem_abs;
            match elems[i].base() {
                Type::Integer(_) => {
                    stack.add_op("OpPutInt", self);
                }
                // P249 — fn-ref slot is 20 B; matches push above.
                // OpPutFnRef's signature pops 16 B (`text`) but the
                // runtime pops 20; mirror gen_set_first_at_tos's
                // `fnref_signature_gap` correction.
                Type::Function(_, _, _) => {
                    stack.add_op("OpPutFnRef", self);
                    stack.position -= stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
                }
                Type::Boolean => stack.add_op("OpPutBool", self),
                Type::Float => stack.add_op("OpPutFloat", self),
                Type::Single => stack.add_op("OpPutSingle", self),
                Type::Character => stack.add_op("OpPutCharacter", self),
                Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                Type::Text(_) => stack.add_op("OpPutText", self),
                // Every DbRef-shaped element travels as one handle, so the membership
                // question is [`is_dbref`](crate::data::is_dbref)'s and is asked there —
                // spelled inline it drifts short by exactly the five keyed collections,
                // which is what a `hash<S[k]>` tuple element hit here.
                t if crate::data::is_dbref(t) => stack.add_op("OpPutRef", self),
                other => panic!("Tuple set: unsupported element type {other:?}"),
            }
            self.code_add(pos);
        }
    }

    /// Recursive helper for tuple null-init.  Walks the element list
    /// and pushes/OpPut's a zero value at each leaf primitive's
    /// absolute stack slot.  For nested `Type::Tuple` elements,
    /// recurses with the inner offsets added to `base`.
    fn emit_tuple_null_init(&mut self, stack: &mut Stack, elems: &[Type], base: u16) {
        let offsets = crate::data::element_stack_offsets(elems);
        for (i, elem) in elems.iter().enumerate() {
            let elem_abs = base + offsets[i] as u16;
            if let Type::Tuple(inner_elems) = elem {
                self.emit_tuple_null_init(stack, inner_elems, elem_abs);
                continue;
            }
            match elem.base() {
                Type::Integer(_) | Type::Function(_, _, _) => {
                    stack.add_op("OpConstInt", self);
                    self.code_add(0i64);
                }
                Type::Boolean => {
                    stack.add_op("OpConstFalse", self);
                }
                Type::Single => {
                    stack.add_op("OpConstSingle", self);
                    self.code_add(0.0f32);
                }
                Type::Float => {
                    stack.add_op("OpConstFloat", self);
                    self.code_add(0.0f64);
                }
                t if crate::data::is_dbref(t) => {
                    // T1.8c: use NullRefSentinel (no store allocation) for tuple
                    // reference elements.  The element will be overwritten by PutRef
                    // or CopyRecord during destructuring; a real store is not needed
                    // at null-init time.  Membership is `is_dbref`'s question — see the
                    // sibling note in `emit_tuple_var_pop_put`.
                    self.emit_push_sentinel(stack);
                }
                Type::Text(_) => {
                    stack.add_op("OpConvTextFromNull", self);
                }
                // loft#808 — Character and payload-less Enum complete the table.  The
                // three sibling walkers (`emit_tuple_put_ops`, `emit_tuple_var_pop_put`,
                // `emit_tuple_var_push_recursive`) already carry both; only null-init
                // was short, so `fn f() -> (character, character)` panicked here the
                // moment its return stopped being boxed into a `__tuple<…>` record and
                // became a real tuple local that needs initialising.
                Type::Character => {
                    stack.add_op("OpConvCharacterFromNull", self);
                }
                Type::Enum(_, false, _) => {
                    stack.add_op("OpConvEnumFromNull", self);
                }
                other => panic!("emit_tuple_null_init: unsupported element type {other:?}"),
            }
            let pos = stack.position - elem_abs;
            match elem.base() {
                Type::Integer(_) | Type::Function(_, _, _) => stack.add_op("OpPutInt", self),
                Type::Boolean => stack.add_op("OpPutBool", self),
                Type::Single => stack.add_op("OpPutSingle", self),
                Type::Float => stack.add_op("OpPutFloat", self),
                Type::Text(_) => stack.add_op("OpPutText", self),
                Type::Character => stack.add_op("OpPutCharacter", self),
                Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                t if crate::data::is_dbref(t) => stack.add_op("OpPutRef", self),
                _ => unreachable!(),
            }
            self.code_add(pos);
        }
    }

    pub(super) fn gen_set_first_vector_null(&mut self, stack: &mut Stack, v: u16) {
        // Plan-04 Phase B.3 atomic bundle: slot-aware.  Writes the DbRef
        // directly at v's slot rather than pushing onto TOS.  The
        // function-entry `OpReserveFrame(frame_hwm)` in `def_code`
        // guarantees `stack.position >= v.slot + 12`, so the init is
        // always addressable as `slot_offset = stack.position - v.slot`.
        if let Type::Vector(elm_tp, dep) = stack.function.tp(v).clone() {
            let ref_size = size_of::<crate::keys::DbRef>() as u16;
            let slot_end = stack.function.stack(v).saturating_add(ref_size);
            if stack.position < slot_end {
                let bump = stack.step(slot_end) - stack.position;
                stack.add_op("OpReserveFrame", self);
                self.code_add(bump);
                stack.position += bump;
            }
            let slot_offset = stack.var_pos(v);
            if stack.function.is_skip_free(v) || stack.function.is_inline_ref(v) {
                // skip_free bindings borrow; inline_ref lift temporaries are
                // overwritten before read.  Both want a null sentinel in the
                // slot with no store alloc.
                stack.add_op("OpInitRefSentinel", self);
                self.code_add(slot_offset);
            } else if dep.is_empty() {
                // Owned vector: allocate a fresh store at v's slot, then
                // init the header's length field (4B) and dep field (1B).
                stack.add_op("OpInitRef", self);
                self.code_add(slot_offset);
                stack.add_op("OpDatabase", self);
                self.code_add(slot_offset);
                let name = format!("main_vector<{}>", elm_tp.name(stack.data));
                let known = stack.data.name_type(&name, self.source);
                debug_assert_ne!(
                    known,
                    u16::MAX,
                    "Incomplete type {name} in {}",
                    stack.function.name
                );
                self.code_add(known);
                // Push a DbRef copy of v's slot, set length=0 (4B), set dep byte.
                stack.add_op("OpVarRef", self);
                self.code_add(slot_offset);
                stack.add_op("OpConstInt", self);
                self.code_add(0i64);
                // Vector header length field is 4 bytes (u32).
                stack.add_op("OpSetInt4", self);
                self.code_add(4u16);
                // OpCreateStack pointing at v's slot (now the real DbRef).
                let dep_offset = stack.var_pos(v);
                self.emit_push_create_stack(stack, dep_offset);
                stack.add_op("OpConstInt", self);
                self.code_add(12i64);
                stack.add_op("OpSetByte", self);
                self.code_add(4u16);
                self.code_add(0u16);
            } else {
                // Borrowed view: a stack-frame DbRef pointing into dep's
                // slot.  Must be overwritten by OpPutRef before any field
                // access.
                let dep_pos = stack.function.stack(dep[0]);
                let dep_offset = stack.position - dep_pos;
                stack.add_op("OpInitCreateStack", self);
                self.code_add(slot_offset);
                self.code_add(dep_offset);
            }
        }
    }

    /// The null-init of a NULLABLE vector local (`vector<T>?`): the slot takes the null
    /// sentinel — ABSENT — never the empty store its dense twin allocates.  What reaches
    /// here is the pre-init a branch or a loop emits for a local first assigned inside it
    /// (`scopes::needs_pre_init`): on the path that never assigns, `x != null` must answer
    /// false, and the assignment that does run installs its own value over the sentinel.
    /// Routed by `Type`, not by the dense arm's peel: that arm allocates a store and, for a
    /// borrowed vector, a stack placeholder into the DEP's slot — with `x` itself left
    /// unwritten, the read on the untaken path was a poisoned frame word (`DbRef store_nr
    /// 48879 is out of range`).
    ///
    /// A KEYED nullable local is NOT routed here: its assignment is `OpReplaceKeyed` INTO
    /// the local's own store, so the null-init has to allocate that store (`gen_keyed_null`,
    /// the same answer `--native` gives), and on the untaken path the local reads as present
    /// and empty on both backends — what absence means for a keyed local is the null model's
    /// question (@PLN153), not this lowering's.
    pub(super) fn gen_set_first_nullable_collection_null(&mut self, stack: &mut Stack, v: u16) {
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRefSentinel", self);
        self.code_add(slot_offset);
    }

    /// P188: first-assignment init for a keyed-collection local
    /// (`sorted<T[key]>`, `hash<T[key]>`, `index<T[key]>`,
    /// `spatial<T[key]>`).  Allocates a fresh store and claims a
    /// record sized for the collection type at the local's slot.
    /// `set_default_value` zero-initialises the root-pointer field;
    /// subsequent `OpNewRecord` / `OpFinishRecord` operations will
    /// dispatch through `record_new`'s `Parts::Sorted/Hash/Index/Radix`
    /// arms and grow the collection in-place.
    pub(super) fn gen_set_first_keyed_null(&mut self, stack: &mut Stack, v: u16) {
        self.gen_keyed_null(stack, v, true);
    }

    /// Lower a `Set(v, Null)` whose `v` is a keyed-collection local.
    ///
    /// `first == true` (first assignment): allocate a fresh empty store at
    /// `v`'s slot (`OpInitRef` + `OpDatabase`).  `first == false`
    /// (reassignment — the @P302 `s = []` clear): the slot already holds a
    /// live `DbRef`, so emit `OpDatabase` ALONE — `clear()` empties the store
    /// in place (reuses `store_nr`, reclaims all space, no leak).  Emitting
    /// `OpInitRef` on a reassignment would null the slot and leak the store.
    pub(super) fn gen_keyed_null(&mut self, stack: &mut Stack, v: u16, first: bool) {
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if first && stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        // De-conflated no-alloc gate (@PLN85 A.1 companion): gate on
        // `is_inline_ref` ALONE — the genuine "borrow, don't allocate" signal —
        // NOT `is_skip_free`, whose only contract is "don't emit `OpFreeRef`"
        // (a free-time fact).  A keyed return-local that OWNS its store but
        // transfers it on return is marked `skip_free`; the old `skip_free`
        // sentinel suppressed `OpDatabase` for it, leaving `store_nr = u16::MAX`
        // and OOB-panicking the next record op (`allocation.rs`).  With the gate
        // on `inline_ref` such a local falls through and ALLOCATES below.
        //
        // The vector twin (`gen_set_first_vector_null`) keeps the broader
        // `skip_free` gate on purpose: there `skip_free` also marks match-bind /
        // `??`-coalesce borrowed views (`_mv_*`, `__ncc_*`) that are overwritten
        // before read and must NOT allocate (allocating leaks them — they carry
        // no `OpFreeRef`).  No owned-vector case needs the vector site changed,
        // so de-conflating there only over-reaches.  See
        // `tests/scripts/122-keyed-return-local-allocates.loft`.
        //
        // ⚠ loft#1155 — the de-conflation DROPPED that second half here rather than renaming
        // it, and a keyed match binding then allocated a store and orphaned it the moment the
        // arm overwrote the slot with its `OpGetField` projection: one store per call,
        // unbounded in a loop, and `vector` clean beside it — which is precisely the boundary
        // the issue measured.  It comes back under its own name, `is_overwritten_view`, so the
        // owned return-local the de-conflation was FOR still allocates.
        if stack.function.is_inline_ref(v) || stack.function.is_overwritten_view(v) {
            // inline_ref keyed locals borrow an outer-owned store.
            // On first assignment, point the slot at it via the sentinel; on
            // reassignment there is nothing to re-init (no current shape
            // produces a keyed inline-ref reassignment).
            if first {
                stack.add_op("OpInitRefSentinel", self);
                self.code_add(slot_offset);
            }
            return;
        }
        // Resolve the database type id for the specific keyed-collection
        // instantiation.  `database.sorted/hash/index/spatial` is idempotent —
        // it returns the existing registered type id when the (content, key)
        // pair has been registered (every struct field of the same shape
        // does so via `fill_database`), otherwise registers a new one.
        // @FR-L-Null — `base()`, because a nullable keyed local holds the SAME store as its
        // dense twin (layout(τ) = layout(τ?)).  Asked bare, a `hash<S[k]>?` fell to the
        // catch-all below and ICE'd.
        let tp = stack.function.tp(v).base().clone();
        let tp_nr = match &tp {
            Type::Sorted(td, key, _) => {
                let c = stack.data.def(*td).known_type();
                self.database.sorted(c, key)
            }
            Type::Hash(td, key, _) => {
                let c = stack.data.def(*td).known_type();
                self.database.hash(c, key)
            }
            Type::Index(td, key, _) => {
                let c = stack.data.def(*td).known_type();
                self.database.index(c, key)
            }
            Type::Radix(td, key, _) => {
                let c = stack.data.def(*td).known_type();
                self.database.spatial(c, key)
            }
            Type::Trie(td, key, _) => {
                let c = stack.data.def(*td).known_type();
                self.database.trie(c, key)
            }
            _ => unreachable!("gen_keyed_null on non-keyed type"),
        };
        debug_assert_ne!(
            tp_nr,
            u16::MAX,
            "Unregistered keyed-collection type for {} in {}",
            stack.function.name(v),
            stack.function.name,
        );
        // First assignment: null the slot before allocating.  Reassignment /
        // clear: the slot already holds a live DbRef → OpDatabase clears that
        // store in place; OpInitRef would null the slot and leak the store.
        if first {
            stack.add_op("OpInitRef", self);
            self.code_add(slot_offset);
        }
        stack.add_op("OpDatabase", self);
        self.code_add(slot_offset);
        self.code_add(tp_nr);
    }

    /// Emit a null sentinel for the given type onto the stack.
    /// Used when Value::Null appears in a typed context (e.g. function argument).
    fn emit_typed_null(&mut self, stack: &mut Stack, tp: &Type) {
        // @PLN25 slice (b) — an `Optional(τ)`'s null IS `τ`'s typed null (the same sentinel),
        // so peel before dispatching. The parser's own `null()` (`definitions.rs`) already
        // peels for this reason; matching the unpeeled type here sent every `τ?` to the
        // catch-all below and pushed a zero-filled DbRef where a scalar was expected.
        //
        // Reached whenever a caller OMITS a parameter declared `= null`: the parser fills a
        // bare `Value::Null`, and this is what gives it the parameter's width and sentinel.
        // An `integer? = null` then arrived as a REF sentinel and `a?` discharged it to
        // 65535 instead of 0 — a silent wrong number, against types.md's
        // `(D-Scalar) construct_default(Integer[r]) = 0` (loft#1015).
        match tp.base() {
            Type::Text(_) => {
                stack.add_op("OpConvTextFromNull", self);
            }
            // loft#725 — a SENTINEL, not an allocated null.  `emit_push_null_ref`
            // calls `database.null()`, so every typed Reference null claimed a store
            // that only a variable could later free.  Pushed onto the eval stack and
            // discarded, nothing ever did: `for i in 0..8 { mk(i) }` leaked one store
            // per round, because the lift consumes the call's value and leaves the
            // block with no tail, so codegen synthesises this null purely to satisfy
            // the block's declared type — then `gen_drop` throws it away.
            //
            // The sentinel is what a typed null means here (this function's own name),
            // and it is not weaker: `store_nr == u16::MAX` reads as null, `free_named`
            // treats it as a no-op, and an `OpDatabase` landing on such a slot
            // allocates a fresh store (`state/io.rs::database`), so a null that is
            // later written through still works.  Interpreter-only — native already
            // lowers this to `DbRef::NULL` with no allocation.
            Type::Reference(_, _) | Type::Enum(_, true, _) => {
                self.emit_push_sentinel(stack);
            }
            Type::Integer(_) => {
                stack.add_op("OpConstInt", self);
                self.code_add(i64::MIN);
            }
            // types.md — `Char`'s in-band sentinel is CODEPOINT 0, and it is a
            // 4-byte slot, so the integer arm was wrong on both counts: it pushed
            // eight bytes of `i64::MIN` where four are read.  It happened to read
            // as null only because the low word of `i64::MIN` is zero on a
            // little-endian box.  `OpConvCharacterFromNull` is the same `'\0'` the
            // parser's own `null()` emits for a `character` — one fact, one
            // spelling, and the width the slot actually holds (loft#1014).
            Type::Character => {
                stack.add_op("OpConvCharacterFromNull", self);
            }
            Type::Float => {
                stack.add_op("OpConstFloat", self);
                self.code_add(f64::NAN);
            }
            Type::Single => {
                stack.add_op("OpConstSingle", self);
                self.code_add(f32::NAN.to_bits());
            }
            // @PLN17 — a null-capable `boolean` is the TRI-STATE byte (0/1/255), so its null
            // is 255, not the integer sentinel. This arm was unreachable for `boolean?` until
            // the `.base()` peel above (every `τ?` fell to the catch-all), and it was wrong:
            // `i64::MIN` is not a value the tri-state can hold, so `a == null` on an omitted
            // `boolean? = null` answered FALSE. 255 matches what `--native` writes for the
            // same slot (`write_typed_null_in`'s `255_u8`), which is what makes the two
            // backends agree (loft#1015).
            Type::Boolean => {
                stack.add_op("OpConstInt", self);
                self.code_add(255i64);
            }
            _ => {
                // For other types, push a zero-filled DbRef as a generic null.
                self.emit_push_sentinel(stack);
            }
        }
    }

    /// Adjust the slot position for a first-assignment variable.
    /// Case 1: pre-assigned above TOS → move down. Case 2: large type below TOS →
    /// override only if no child-scope overlap (A13 guard).
    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_set(&mut self, stack: &mut Stack, v: u16, value: IrNode) {
        // @PLN11 G2/M3.15 — materialise-at-boundary.  generate_set's body is an
        // intricate store-ownership tracker (its comments flag use-after-free /
        // S1-substitution-corruption hazards); rather than thread the handle
        // through that hazardous logic and its gen_set_first_* / set_var family,
        // materialise the assigned sub-value to a native `Value` once here (a
        // compile-time clone for native; a `read_value` for store) and run the
        // existing logic unchanged.  This makes the Set path store-capable; a
        // zero-copy body rewrite is a later refinement.
        let value_owned = value.to_owned_value();
        let value = &value_owned;
        self.vars.insert(self.code_pos, v);
        // Zero-sized variables (null-typed) have no stack storage.
        if size(stack.function.tp(v), &Context::Variable) == 0 {
            stack.function.set_stack_allocated(v);
            return;
        }
        // Plan-04 Phase 2h.3: codegen is the allocator.  For a
        // first-assignment, `stack_pos` is still `u16::MAX` here —
        // `gen_set_first_at_tos` claims the slot at current TOS
        // below.  Only on the reassignment branch is `pos` read.
        let pos = stack.function.stack(v);
        if stack.function.is_stack_allocated(v) {
            debug_assert!(
                pos != u16::MAX,
                "variable '{}' flagged stack_allocated but has no slot",
                stack.function.name(v)
            );
            // Reassignment — variable already on the stack.
            // @P302 — `s = []` on a populated keyed local clears it IN PLACE
            // (OpDatabase reuses the store, no leak).  Reached now that
            // `scan_set` no longer elides keyed `Set(v, Null)`.  Without this
            // arm it would fall to `set_var` → `gen_put_var` → panic (no keyed
            // OpPut* arm).
            if crate::parser::vectors::owns_keyed_store(stack.function.tp(v))
                && *value == Value::Null
            {
                self.gen_keyed_null(stack, v, false);
                return;
            }
            // @PLN25: a `text?` reassignment must CLEAR before the append, exactly like plain
            // `text` (set_var's OpAppendText appends in place) — peel the marker. Without this
            // `a: text? = "hi"; a = "x"` appended → "hix".
            if matches!(stack.function.tp(v).base(), Type::Text(_)) {
                let var_pos = stack.position - pos;
                stack.add_op("OpClearText", self);
                self.code_add(var_pos);
            }
            // Free the old store before reassigning an owned Reference.
            // Only safe when the variable truly owns its store (dep empty).
            // The dep was cleared on first assignment only when codegen will
            // deep-copy (gen_set_first_ref_call_copy). O-B2 adopted stores
            // keep their dep, so this won't fire for them.
            // @P377-CORRUPTION: skip when this is an S1-substituted call
            // (`Set(v, Call(fn, [.., Var(v), ..]))` — the inner Call's
            // hidden-buffer arg has been substituted to be v itself by
            // `nrvo_collapse_tail_set`).  In that shape, the Call writes
            // its result INTO v's existing store, so freeing v's old
            // content here destroys what the Call is about to write into.
            // Detection (#360): an arg of the RHS Call is `Var(v)` AT A
            // HIDDEN parameter position — only the hidden buffer arg is
            // what `nrvo_collapse_tail_set` substitutes.  A VISIBLE
            // `Var(v)` argument (`s = grow(s)` passing s as plain data) is
            // NOT the S1 shape: classifying it as S1 skipped both the
            // pre-Set free and the deep-copy, so `s` adopted the call-site
            // buffer store and the next iteration's callee `OpDatabase`
            // wiped it while `x` still pointed there — every later
            // iteration read null.  Pairs with the evaluate-call-first
            // re-init order in the deep-copy emission below (the interp
            // half of @P290).  Bails for every other shape
            // (struct-literal RHS, non-S1 calls, non-tail patterns) —
            // those keep the load-bearing pre-Set FreeRef intact, so
            // `tests/scripts/130-gridmesh-crystal-equiv.loft` and the
            // broader moros_* suite stay green.
            let s1_substituted = if let Value::Call(fn_nr, args) = value.unspan() {
                let attrs = stack.data.def(*fn_nr).attributes();
                args.iter().enumerate().any(|(i, a)| {
                    matches!(a.unspan(), Value::Var(av) if *av == v)
                        && attrs.get(i).is_some_and(|at| at.hidden)
                })
            } else {
                false
            };
            // @PLAN51 Cluster II/III — also skip the pre-Set OpFreeRef
            // when `v` is a hidden-buffer parameter AND the call IS S1-
            // substituted (the call's args contain `v` itself).  In that
            // case the callee's OpDatabase routes through
            // `state/io.rs::database` which does `clear(cv); claim(cv,
            // size)` — an in-place reuse of the caller's buffer store.
            // No leak, no corruption: store_nr is preserved.
            //
            // When the call is NOT S1-substituted (probes 04 + 28: the
            // call's hidden buf is a fresh `__ref_N` distinct from `v`),
            // the in-place-reuse assumption breaks — `v`'s current store
            // (e.g. from a preceding struct-literal init) becomes
            // orphan when the reassignment writes the deep-copied
            // result.  We must fall through to the reassignment path so
            // the pre-Set free runs on `v`.  `s1_substituted` is the
            // already-computed check we need.
            let is_hidden_buf_arg = s1_substituted && stack.function.is_argument(v) && {
                let attrs = &stack.data.def(stack.def_nr).attributes();
                attrs
                    .iter()
                    .any(|a| a.hidden && stack.function.var(&a.name) == v)
            };
            // Enforces @FR-O-Detach — this is the site that gets the order right, and the
            // one the other three are measured against: the free is DEFERRED past the
            // assignment when the RHS reads `v`, rather than the slot being prepared first.
            //
            // #330 — ONE recursive "does the RHS read v?" predicate decides
            // the free strategy (shared with scopes' #316 transition logic).
            // The old top-level-arg S1 scan missed struct-literal field
            // initialisers (`x = S { v: x.v + 1 }`) and the bare self-assign
            // (`x = x`): the pre-Set free released the store, the literal's
            // OpDatabase reused the slot, and the RHS read the recycled
            // record — silent null/corruption on both backends.
            //
            // When the RHS reads v, the old DbRef is STASHED on the eval
            // stack before anything runs and freed AFTER the assignment via
            // OpFreeRefIfDistinct — which also degrades to a no-op for the
            // S1 in-place shapes (the new value IS the old store).
            let rhs_reads_v = value.reads_var(v);
            // loft#615 — an OWNED heap variable that is re-assigned must free the
            // store it is dropping, and `Vector` was missing from this list while
            // `Reference` / `Enum` had it.  A `??` materialises its subject into a
            // function-scope owner (`__ncc_N`, see
            // `parser/operators.rs::handle_null_coalesce`); re-assigning it once per
            // loop iteration overwrote the previous store with nothing freeing it, so
            // `for … { ns = list_dir(d) ?? [] }` leaked exactly (iterations - 1)
            // stores.  Empty deps is loft's owned-vs-borrowed convention, so a view
            // (`vv[i]`, a `??` result naming its owner) still takes no free here.
            // loft#723 — empty deps is only a PROXY for ownership, and it reads
            // "owned" for a borrow whose dep list was never populated.  A `??`
            // subject of Reference type is exactly that: the parser materialises
            // it into `__ncc_N` and marks it `skip_free` — the carried fact that
            // says its store belongs to someone else — while its type keeps empty
            // deps.  `skip_free`'s whole contract is "never emit `OpFreeRef` for
            // this var", so it must veto the proxy here rather than only at the
            // scope-exit sweep that `get_free_vars` runs.
            //
            // Consulting it only there left the unconditional pre-Set free below
            // reachable, and in a LOOP that free lands on the NEXT iteration's
            // store: `for … { e = f().items[0] ?? P {} }` freed the subject at the
            // body exit, `f()` reallocated the same slot, and the pre-Set free then
            // released that live store before the element read — silent stale bytes
            // without `LOFT_POISON`, SIGSEGV with it.  The @PLN118 sentinel reset
            // cannot cover it: that reset hangs off a block-exit free, which a
            // `skip_free` var by definition does not have.
            //
            // An OWNED Vector `??` subject is unaffected — the parser marks that one
            // `inline_ref`, not `skip_free`, precisely so it keeps the loft#615 free.
            //
            // The five KEYED kinds are here for the same reason `Vector` is, and were missing:
            // a keyed local's handle is the same store-backed slot, so `c = null` (which lowers
            // to `OpNullRefSentinel`, NOT to the @P302 in-place clear above) displaced its store
            // with nothing naming it — one orphan per evaluation on BOTH backends, values right
            // throughout.  Read through `base()`, because the shape that reaches this is the
            // NULLABLE one: a dense keyed local cannot be assigned the sentinel at all.
            //
            // `Vector` is read through `base()` for the same reason, and once did not be:
            // the bare spelling carried the claim that a nullable vector *"already releases
            // through its own path"*, so widening it *"would free twice"*.  Re-measured, that
            // is false — `x: vector<τ>? = null; for i in 0..N { x = m(i) }` over a fn-ref held
            // one store per iteration on BOTH backends, and the peak grew 1:1 with N while the
            // dense twin beside it stayed flat.  What makes the peel safe is `nullable_local`
            // below: an `Optional` destination routes to the runtime-GUARDED post-free, which
            // `free_displaced` no-ops on a same-store, free-protected or stack-record ref —
            // so the double free the bare spelling was avoiding cannot be reached from here.
            // Enforces @FR-O-Proxy, and is a SECOND home for the question
            // `Scopes::owns_freeable_store` already answers — *may this function free the
            // store this binding names?*  The last three conjuncts are that predicate
            // restated: `depend().is_empty() && !is_skip_free(v)` verbatim, and
            // `!is_hidden_buf_arg` where it spells the argument carve-out
            // `(!is_argument(v) || is_promoted_ret_buffer(..))`.  Two spellings of one rule
            // is what lets them disagree, and they do: the shape test in front of them does
            // not peel `Optional`, so a NULLABLE heap-record local (`c: S?` is
            // `Optional(Reference(S))`) matches no arm and skips this whole free path —
            // pre-free, stash and post-free alike — while its dense twin gets all three.
            // That is `formal/ownership.md` D-own-16's open half, and folding this onto
            // `owns_freeable_store` is where the peel belongs, so it lands once rather than
            // at whichever site a leak was noticed
            // (doc/claude/plans/own16-displaced-store-free/, step 2).
            //
            // ⚠ The peel is NOT sound on its own, which is why this comment is not a TODO to
            // widen the `matches!`: the empty dep list the rule above calls a PROXY reads
            // `OWNS` for a local a lambda has CAPTURED, and freeing there is a use-after-free
            // against the closure's capture-time DbRef.  The licence has to come from a
            // per-run witness.
            // @FR-O-Proxy asks free — the pre-Set free of the store `v` is displacing.  The
            // fact-reading half is `Function::owns_displaced_store`, the ONE spelling both
            // backends read (@FR-O-NoDiverge; its native twin is `generation/dispatch.rs`'s
            // `owned_ref_reassign`).  What this side adds is the interpreter's own: the hidden
            // buffer ARGUMENT is excluded, because the caller owns that store.  The one-argument
            // borrow the predicate admits (`d: S? = p`, D-own-16's residual) licenses only the
            // GUARDED free — `nullable_local` below routes it there — and `free_displaced`
            // declines a free-protected store, so the caller's argument is never released here;
            // measured: `LOFT_POISON=1` answers identically to `LOFT_POISON=0`.
            let owned_ref =
                stack.function.owns_displaced_store(v, value, stack.data) && !is_hidden_buf_arg;
            // An `OpNewRecord` RHS returns an INTERIOR ref into an existing
            // container's backing store (a vector element / nested field), so
            // the new value can land in the SAME store as v's old value —
            // consecutive elements of one vector share its backing.  The
            // unconditional pre-Set `OpFreeRef` below whole-store-frees that
            // backing, killing the live data: `xs[i].field = V{a: [literal]}`
            // (field-of-element set built in place) silently freed
            // `xs[i].field.a`'s store (correct bytes until churn overwrote the
            // freed slot; SIGSEGV/UAF under the armed build — the native
            // generator's `if _old.store_nr != new.store_nr` guard already
            // protects it).  Route it through the runtime-guarded post-free
            // (`OpFreeRefIfDistinct`) so a same-store reassignment is a no-op.
            let rhs_is_new_record = matches!(
                value.unspan(),
                Value::Call(fn_nr, _) if stack.data.def(*fn_nr).name() == "OpNewRecord"
            );
            // A NULLABLE heap local's slot does not always hold an owned heap store: on the
            // paths where it never took one it holds the null sentinel, and where its record
            // was stack-allocated it holds a stack-record ref.  The pre-Set free below is
            // UNCONDITIONAL and whole-store, which is sound only for a local that always
            // holds a store of its own — a dense `Reference` does, an `Optional` one does
            // not.  Route it to the runtime-GUARDED post-free instead, which is the same
            // answer `rhs_is_new_record` above takes for the same reason: `free_displaced`
            // no-ops on a same-store, free-protected or stack-record displaced ref, where
            // `OpFreeRef` can only refuse it loudly (`#306`).  @FR-H-Free's `store(r) ≠ 0`
            // side condition is what separates the two frees; see `Stores::free_displaced`.
            let nullable_local = matches!(stack.function.tp(v), Type::Optional(_));
            // loft#1336 / @FR-O-Witness — a local whose OWNER WITNESS releases its stores.
            // It is never-free, so `owned_ref` above is false and none of the frees below
            // are emitted for it; but the COPY arms further down must still run, because
            // @FR-B-Copy does not depend on who frees: a whole-value bind is independent.
            // Every copy into such a local lands in a FRESH store (the slot is reset before
            // the allocation), since the local may be holding a VIEW at that moment and an
            // in-place `OpDatabase` would write the copy into the viewed record.
            let witnessed = stack.function.owner_witness(v).is_some();
            let mut stash_old_for_post_free = false;
            if owned_ref && (rhs_reads_v || rhs_is_new_record || nullable_local) {
                let free_pos = stack.var_pos(v);
                stack.add_op("OpVarRef", self);
                self.code_add(free_pos);
                stash_old_for_post_free = true;
            } else if owned_ref && !s1_substituted {
                let free_pos = stack.var_pos(v);
                stack.add_op("OpVarRef", self);
                self.code_add(free_pos);
                stack.add_op("OpFreeRef", self);
                // @PLN118 — this var takes an unconditional pre-build free; its
                // block-exit free must reset it to the sentinel so a loop re-entry's
                // pre-build free cannot re-free a reused slot (moros glb H4 UAF).
                stack.owned_reassigned.insert(v);
            }
            // @PLN85 unification (first chokepoint collapse, LOFT_JOIN_OWN): read the
            // ONE carried fact instead of the type-shape proxy below. A `??`-JOIN call
            // return bound into an owned slot (`__lift_1 = pick(t,i)`, `pick` returns
            // `t[i] ?? …`) needs the runtime arg-aliasing guard, witnessed by the
            // oracle's interprocedurally-resolved base: `OpBindOrCopy` adopts the owned
            // arm (the source-free then frees it) and materialises the borrow arm (the
            // source-free hits the copy; the borrowed arg is intact). A static gate
            // cannot — the source-free is load-bearing for the owned branch.
            if owned_ref
                && !s1_substituted
                && !stash_old_for_post_free
                // The runtime adopt-vs-copy guard is for a FRESH runtime-Join value
                // — a call return (`__lift = pick(t,i)`) or an ncc/`??` reassign
                // (`r = v[i] ?? x`, #495/test 85): `OpBindOrCopy` adopts the owned
                // arm and materialises the borrow arm.  A plain `Value::Var` RHS is
                // a REUSED named local (`md = sb`); loft value semantics require
                // COPYING it — adopting would move `sb`'s store into `md`, then
                // `md`'s owned `OpFreeRef` would kill `sb` (#496 interp clobber:
                // `md = sa; if c { md = sb }`).  The `Var` case falls through to the
                // #306 whole-struct copy below.  (Native's join-own in
                // `generation/dispatch.rs` never reaches a bare-Var RHS either.)
                && !matches!(value.unspan(), Value::Var(_))
                && crate::keys::join_own_enabled()
                && let crate::use_analysis::Own::Join { base } =
                    crate::use_analysis::ownership_of(stack.data, stack.def_nr, value)
                && base != u16::MAX
                && let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            {
                let tp_nr = stack.data.def(d_nr).known_type();
                // Free the DISPLACED old store first — this branch REPLACES the
                // plain owned-reassign path (which frees it just above), and
                // `OpBindOrCopy` overwrites `v` without releasing its previous
                // store: each conditional reassign otherwise orphans one store
                // (the p462_cond_reassign_retbuf gate-ON leak, M×N per call
                // site).  Safe here: `\!stash_old_for_post_free` means the RHS
                // does not read `v`, and a first-Set's null ref free is a no-op.
                let old_pos = stack.var_pos(v);
                stack.add_op("OpVarRef", self);
                self.code_add(old_pos);
                stack.add_op("OpFreeRef", self);
                // The call result (`src`) FIRST, then the witness on top — so the
                // witness's frame-relative `var_pos` accounts for `src` already on the
                // eval stack. `OpBindOrCopy` pops witness (top) then src.
                self.generate(value, stack, false);
                // Push the witness (the borrowed-arg base) on top of the call result.
                // A PUSH op reads at the PRE-push position, so `var_pos(base)` is taken
                // BEFORE `add_op` (matching the pre-Set free above); the POP op's
                // `var_pos(v)` is taken AFTER `add_op` (post-pop position).
                let witness_pos = stack.var_pos(base);
                stack.add_op("OpVarRef", self);
                self.code_add(witness_pos);
                stack.add_op("OpBindOrCopy", self);
                self.code_add(stack.var_pos(v));
                self.code_add(tp_nr);
                return;
            }
            if (owned_ref || witnessed) && !s1_substituted {
                // when the value is a call whose return is tied to a passed
                // buffer/param, the callee returns via a hidden __ref_N that is
                // reused across calls — or a BORROW of a param's store (e.g.
                // `return cs[i]` from a vector param).  OpPutRef would alias v
                // with that store; the next FreeRef (v's owned pre-Set free)
                // would whole-store-free it → use-after-free of the lender
                // (#497: build_walls' cs died through a reassigned corner_at
                // result borrowing a VECTOR param element; #496 is the same
                // class through `md = sa; md = sb`).  Deep-copy into a fresh
                // store instead; do NOT free the source.
                //
                // Cluster A.3 (OWNERSHIP_MODEL row 102/270): read the ONE
                // carried adopt-vs-copy fact, `return_adopts_fresh_store()`,
                // exactly like the first-Set path (gen_set_first_at_tos).
                // The old visible-Reference/Enum param scan was the coarse
                // `has_ref_params` proxy this fact replaced — it MISSED a
                // callee borrowing from a visible VECTOR param, taking the
                // plain-adopt path for a borrowed return.
                // loft#1245 — BOTH spellings, and this is the site the LOOP case needs.
                // A lifted inline call is declared at function scope and assigned inside
                // the loop body, so its `Set` is a REASSIGNMENT, not a first bind, and
                // reads `Value::Call` alone sent every fn-ref one to the plain-adopt
                // fallthrough: `CallRef` then `PutRef`, with no `OpDatabase`, no
                // `OpCopyRecord` and no @P290 bracket.  `--native` had it right through
                // `output_set_witnessed`, so the two backends disagreed about the same IR.
                // Through `base()` — a nullable destination is the same storage behind a
                // nullability marker (@FR-L-Null), and asked bare this arm left `c: S? =
                // keep(other)` on `set_var`'s plain `OpPutRef`: an ALIAS of the argument,
                // where @FR-B-Copy says the bound variable is independent (loft#1336).
                if let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) =
                    stack.function.tp(v).base().clone()
                    && !stack.function.is_argument(v)
                    && matches!(value.unspan(), Value::Call(_, _) | Value::CallRef(_, _))
                    && let Some(fn_nr) =
                        crate::use_analysis::callee_of(stack.data, stack.def_nr, value)
                    // The shared gate on the carried ownership facts — a METHOD
                    // (`t_`) takes a caller-allocated buffer exactly like a global
                    // (`n_`) does, and reading the fact for only one of them is what
                    // made a method's return adopt the buffer and then free it
                    // (loft#810).  See `Def::is_loft_defined`.
                    && stack.data.def(fn_nr).is_loft_defined()
                    && (if crate::keys::reassign_copy_enabled() {
                        // The carried A.3 fact (see the comment above).
                        !stack.data.def(fn_nr).return_adopts_fresh_store()
                    } else {
                        // Preserved raw path (LOFT_NO_REASSIGN_COPY) — the old
                        // visible-Reference/Enum proxy, kept ONLY so the fuzz
                        // gate's crash control can still reproduce the class.
                        stack.data.def(fn_nr).attributes().iter().any(|a| {
                            !a.hidden
                                && matches!(
                                    a.typedef,
                                    Type::Reference(_, _) | Type::Enum(_, true, _)
                                )
                        })
                    })
                    // #360 follow-up: when the RHS call READS v AND the
                    // callee returns a BORROWED view (its returned dep names
                    // a VISIBLE param, e.g. `fn id_g(g: G) -> G { return g }`),
                    // the call's result may BE v's own store — any dst
                    // re-init/copy here would wipe the source in place
                    // (tests/scripts/291's hash round-trip).  Keep the plain
                    // pointer pass-through for that composition: PutRef binds
                    // the returned DbRef and the stash's FreeRefIfDistinct
                    // no-ops on the same store, exactly the (correct)
                    // behaviour the over-broad S1 match used to produce.
                    // @PLN85 D-own-1 — the canonical borrowed-view fact (its
                    // returned dep names a VISIBLE param), not the inline visible-dep
                    // scan; identical verdict, one fewer per-site re-derivation
                    // (siblings at 1845/2582 already read it).
                    //
                    // The question is whether the CALL reads `v` — `rhs_reads_v` — and not
                    // whether the post-free is routed through the guarded form.  That
                    // routing (`stash_old_for_post_free`) is also taken for a NULLABLE
                    // local and for an `OpNewRecord` right-hand side, neither of which
                    // means the result may be `v`'s own store; asked through it, every
                    // nullable record local reassigned from a borrowed-view call kept the
                    // raw pointer, and a `-> S?` result chosen by an `if` (its lifted arm
                    // temp is declared nullable and assigned in the arm) aliased the
                    // callee's argument field on this backend alone — `--native` copies
                    // through its own arm (loft#1346, @FR-O-NoDiverge).
                    && !(rhs_reads_v && stack.data.def(fn_nr).returns_borrowed_view())
                {
                    let tp_nr = stack.data.def(d_nr).known_type();
                    // Plan-04 Phase B.3.f: allocate fresh store directly
                    // at v's slot.  Old pattern emitted
                    // `emit_push_null_ref + OpDatabase(12, tp) + OpPutRef`
                    // which pushed a DbRef at TOS, allocated there, then
                    // moved the DbRef to v's slot.  Slot-aware pattern
                    // uses positional `OpInitRef(slot_offset) +
                    // OpDatabase(slot_offset, tp)` — the allocator
                    // operates directly on v's slot; the trailing
                    // `OpPutRef` is redundant and dropped.
                    let ref_size = size_of::<crate::keys::DbRef>() as u16;
                    let slot_end = stack.function.stack(v).saturating_add(ref_size);
                    if stack.position < slot_end {
                        let bump = stack.step(slot_end) - stack.position;
                        stack.add_op("OpReserveFrame", self);
                        self.code_add(bump);
                        stack.position += bump;
                    }
                    // #360: when the RHS call READS `v` (stash_old_for_post_free),
                    // the slot re-init is emitted AFTER the call evaluates —
                    // see the copy emission below — so the call's arguments
                    // still see v's old store.  For every other shape the
                    // re-init stays here, before the copy sequence.
                    // A witnessed local whose value READS it keeps its slot until the
                    // call has run (@FR-O-Detach) and takes the fresh store afterwards.
                    let alloc_after = stash_old_for_post_free || (witnessed && rhs_reads_v);
                    if !alloc_after {
                        let slot_offset = stack.var_pos(v);
                        stack.add_op("OpInitRef", self);
                        self.code_add(slot_offset);
                        stack.add_op("OpDatabase", self);
                        self.code_add(slot_offset);
                        self.code_add(tp_nr);
                    }
                    // Cluster-A A.4: ONE return-ownership query, shared with
                    // the native backend (`generation/dispatch.rs`).  Hidden-only
                    // deps (ref_return-promoted buffer attrs) read as NOT
                    // borrowed — either canonical (same-store no-op gate in
                    // OpCopyRecord) or fresh S1, paired with the post-call
                    // sentinel reset below to clear the caller's slot.
                    //
                    // loft#981/#982 — a borrowed-view return is not always a borrow.
                    // The callee may hand back either the parameter's store or one it
                    // minted (a `??` whose arms split, or a `return o` the return hoist
                    // materialises into a fresh `__ret_N`), and clearing the bit for
                    // both leaked the minted one — once per call, unbounded in a loop.
                    // When this site brackets EVERY ref argument with
                    // `protect_store_frees` (below), the bit is safe to set: the
                    // bracket refuses the free on a protected argument's store and
                    // allows it on a callee-minted one, so the split is decided per
                    // execution instead of by a static bit that cannot express it.
                    let tp_val = if crate::use_analysis::call_return_frees_source(
                        stack.data,
                        stack.def_nr,
                        value,
                    ) {
                        i32::from(tp_nr) | 0x8000
                    } else {
                        i32::from(tp_nr)
                    };
                    // #360: the DESTINATION argument carries the slot re-init.
                    // `Call` lowers its arguments left-to-right, so the RHS
                    // call (the SOURCE) is fully evaluated — reading v's OLD
                    // store — before this Insert runs `OpDatabase(v, tp)`,
                    // which reuses-or-allocates v's record in place
                    // (`state/io.rs::database`: null → fresh store, live →
                    // clear + reclaim).  The old raw `OpInitRef + OpDatabase`
                    // prologue ran BEFORE the source evaluated, so a
                    // self-referencing call (`s = grow(s)`) read the freshly
                    // wiped store and every assignment yielded null.
                    // bracket the call + deep-copy with n_set_store_lock
                    // on every Ref/Vector/Enum arg (like gen_set_first_ref_call_copy
                    // already does for first-assignments).  Without this, when the
                    // callee returns a DbRef aliased with a caller arg, OpCopyRecord's
                    // 0x8000 free-source flag frees the caller's arg store — later
                    // uses of that arg SIGSEGV in OpGetVector.  The first-assignment
                    // path brackets the call with locks on ref-typed args;
                    // the reassignment path needs the same treatment.
                    // loft#981/#982 — ONE derivation, shared with the source-free gate
                    // above (which asks whether these cover every ref argument) and with
                    // the native backend.  Two lists of the same arguments drift.
                    let ref_args: Vec<u16> =
                        crate::use_analysis::protectable_ref_args(stack.data, stack.def_nr, value)
                            .0;
                    // @PLAN51 Cluster II Step 2 — collect caller-hidden-buf
                    // work-ref args.  After the OpCopyRecord wrap frees
                    // the source store (via 0x8000), the caller's slot
                    // for that work-ref still holds the now-stale DbRef
                    // — across-iter the allocator can recycle the
                    // store_nr for a different var, and the next call's
                    // OpDatabase on this slot would clobber that other
                    // var's record (probe 02's `data[0] = w_value`
                    // corruption pinned in commit `a957a365`).  Emit
                    // OpInitRef after the wrap to reset each caller-
                    // hidden-buf arg's slot to the u16::MAX sentinel.
                    let caller_hidden_args: Vec<u16> = if let Value::Call(_, args) = value.unspan()
                    {
                        args.iter()
                            .filter_map(|a| {
                                if let Value::Var(av) = a.unspan()
                                    && *av != v
                                    && stack.function.is_caller_hidden_buf(*av)
                                {
                                    Some(*av)
                                } else {
                                    None
                                }
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    // @P290 — protect caller args from being freed by the
                    // callee's source-free in OpCopyRecord, without blocking
                    // writes / claims (which would crash a callee that
                    // iterates a passed-in hash field — the original
                    // P290 panic).
                    let protect_fn = stack.data.def_nr("n_protect_store_frees");
                    let unprotect_fn = stack.data.def_nr("n_unprotect_store_frees");
                    for av in &ref_args {
                        let prot = Value::Call(protect_fn, vec![Value::Var(*av)]);
                        self.generate(&prot, stack, false);
                    }
                    // #360: evaluate the source call FIRST (reads v's OLD store, so
                    // a self-ref `s = grow(s)` works), THEN OpDatabase re-inits v's
                    // record in place, THEN OpCopyRefOrNull binds the result — a
                    // null return frees the re-init'd record and sets the slot null
                    // (so `v == null` holds), a value deep-copies into it (ABI-B
                    // nullable-struct-return fix).  OpDatabase runs with src still
                    // on the eval stack (its slot offset is taken there);
                    // OpCopyRefOrNull's slot offset is taken after it pops src.
                    self.generate(value, stack, false);
                    if witnessed && rhs_reads_v {
                        // The call is done with the old store; a fresh one, never the
                        // record the local may be VIEWING.
                        stack.add_op("OpInitRef", self);
                        self.code_add(stack.var_pos(v));
                    }
                    stack.add_op("OpDatabase", self);
                    self.code_add(stack.var_pos(v));
                    self.code_add(tp_nr);
                    stack.add_op("OpCopyRefOrNull", self);
                    self.code_add(stack.var_pos(v));
                    self.code_add(tp_val as u16);
                    for av in &ref_args {
                        let unprot = Value::Call(unprotect_fn, vec![Value::Var(*av)]);
                        self.generate(&unprot, stack, false);
                    }
                    // @PLAN51 Cluster II Step 2 — post-wrap sentinel reset.
                    // OpInitRefSentinel writes DbRef{store_nr: u16::MAX, ...}
                    // at the slot — no allocation, no leak.  The next
                    // call's OpDatabase on this slot (paired with the
                    // sentinel-handling at `state/io.rs::database` line
                    // 723+) allocates a fresh store.
                    // @PLAN51 Cluster II Step 2 — post-wrap free + sentinel
                    // reset for caller-hidden-buf args.  Sequence:
                    //   OpVarRef(slot) → OpFreeRef → OpInitRefSentinel(slot)
                    // OpFreeRef is idempotent — `free_named` handles both
                    // the sentinel (u16::MAX) and already-freed (`store.free`)
                    // inputs as no-ops.  This makes the sequence safe whether
                    // OpCopyRecord 0x8000 already freed the source or @P290
                    // `protect_store_frees` blocked the free (placeholder
                    // store still live).  Resetting the slot to sentinel
                    // afterward stops the next iter's OpDatabase from
                    // reclaiming a recycled store_nr that the allocator
                    // may have handed to a different var (probe 02-style
                    // corruption pinned in commit `a957a365`).
                    for av in &caller_hidden_args {
                        let slot_offset = stack.var_pos(*av);
                        stack.add_op("OpVarRef", self);
                        self.code_add(slot_offset);
                        stack.add_op("OpFreeRef", self);
                        stack.add_op("OpInitRefSentinel", self);
                        self.code_add(slot_offset);
                    }
                    if stash_old_for_post_free {
                        // #330 epilogue: free the stashed old store unless
                        // the assignment kept it (witness = v's NEW DbRef;
                        // same store → no-op).
                        let free_pos = stack.var_pos(v);
                        stack.add_op("OpVarRef", self);
                        self.code_add(free_pos);
                        stack.add_op("OpFreeRefIfDistinct", self);
                    }
                    // @PLN130 — REASSIGNMENT from a call.  A lift temp inside an expression
                    // (`__lift_N = file(path)`) is compiled here, not through any first-bind
                    // path, so instrumenting only the first-bind emitters left every
                    // reassignment copy off the manifest.
                    crate::copy_manifest::record(
                        stack.def_nr,
                        v,
                        tp_nr,
                        crate::copy_manifest::Origin::InterpReassignCall,
                    );
                    return;
                }
                // #306 — reassignment `v = src` of same-struct References must
                // DEEP-COPY, matching first-assignment
                // (`gen_set_first_ref_var_copy`) and the parser's dep-strip
                // (objects.rs strips v's deps assuming codegen copies).  The
                // old fall-through aliased src's DbRef into v via OpPutRef, so
                // v's next FreeRef released a store it never owned — when src
                // held a view of a collection element, that freed the whole
                // collection's store mid-program.  No 0x8000 free-source: src
                // stays owned by its own scope.
                // Read through `base()` on BOTH sides — `S?` is `Optional(Reference(S))`, the
                // same storage behind a nullability marker (@FR-L-Null), and the bare spelling
                // sent a nullable destination to `set_var`'s plain `OpPutRef`: an ALIAS,
                // where @FR-B-Copy says the bound variable is independent (`cur: Node? = a;
                // cur = b; cur.value = 9` reached `b`, loft#1336).  A nullable SOURCE takes
                // `OpBindOrCopy` with itself as witness, exactly as the first bind does: a
                // present source is copied into a fresh store, an absent one leaves the
                // destination null rather than holding a record that reads PRESENT.
                // Through `heap_def_nr()` on BOTH sides too: a struct-enum is the same heap
                // record shape as a struct (`Type::heap_def_nr` is the one home for that
                // question), and the bare `Reference` spelling left its rebind on the same
                // alias its first bind took.
                if let Some(d_nr) = stack.function.tp(v).base().heap_def_nr()
                    && let Value::Var(src) = value.unspan()
                    && *src != v
                    && let Some(src_d) = stack.function.tp(*src).base().heap_def_nr()
                    && stack.data.copies_as(d_nr, src_d)
                {
                    let tp_nr = stack.data.def(d_nr).known_type();
                    let ref_size = size_of::<crate::keys::DbRef>() as u16;
                    let slot_end = stack.function.stack(v).saturating_add(ref_size);
                    if stack.position < slot_end {
                        let bump = stack.step(slot_end) - stack.position;
                        stack.add_op("OpReserveFrame", self);
                        self.code_add(bump);
                        stack.position += bump;
                    }
                    let slot_offset = stack.var_pos(v);
                    stack.add_op("OpInitRef", self);
                    self.code_add(slot_offset);
                    if matches!(stack.function.tp(*src), Type::Optional(_)) {
                        self.generate(&Value::Var(*src), stack, false);
                        let witness_pos = stack.var_pos(*src);
                        stack.add_op("OpVarRef", self);
                        self.code_add(witness_pos);
                        stack.add_op("OpBindOrCopy", self);
                        self.code_add(stack.var_pos(v));
                        self.code_add(tp_nr);
                    } else {
                        stack.add_op("OpDatabase", self);
                        self.code_add(slot_offset);
                        self.code_add(tp_nr);
                        let copy_nr = stack.data.def_nr("OpCopyRecord");
                        let copy_val = Value::Call(
                            copy_nr,
                            vec![
                                Value::Var(*src),
                                Value::Var(v),
                                Value::Int(i32::from(tp_nr)),
                            ],
                        );
                        self.generate(&copy_val, stack, false);
                    }
                    // @PLN130 — reassignment `v = src`, both same-struct References (#306).
                    crate::copy_manifest::record(
                        stack.def_nr,
                        v,
                        tp_nr,
                        crate::copy_manifest::Origin::InterpReassignVar,
                    );
                    if stash_old_for_post_free {
                        // #330 epilogue: free the stashed old store unless
                        // the assignment kept it (witness = v's NEW DbRef;
                        // same store → no-op).
                        let free_pos = stack.var_pos(v);
                        stack.add_op("OpVarRef", self);
                        self.code_add(free_pos);
                        stack.add_op("OpFreeRefIfDistinct", self);
                    }
                    return;
                }
                // @PLN130 F1, the REASSIGNMENT twin — materialise an element/field read into
                // a store `v` owns.  `gen_set_first_ref_elem_copy` does this for a first
                // bind; a REBIND fell through to `set_var`, which binds the interior pointer
                // and leaves `v` an owner of the CONTAINER's store.  The pre-Set `OpFreeRef`
                // above then releases the container on the next turn, so `a = w.inner` inside
                // a loop read the record `w = Outer{inner: a}` had just torn down and every
                // heap field of it answered empty (loft#1184).
                //
                // Reaching here means the deps are EMPTY, which for a projection means
                // `scopes.rs` stripped them: @FR-B-View's materialise clause fired because the
                // container is disturbed while this view is live.  A view that kept its deps
                // is not `owned_ref` and still aliases.
                // Not for a witnessed local (loft#1336): the materialise clause is decided
                // from the FINAL dep list, which its per-`Set` witness cannot follow, so such
                // a local keeps every projection an alias on both backends.
                if owned_ref
                    && let Type::Reference(d_nr, _) = stack.function.tp(v).base().clone()
                    && !stash_old_for_post_free
                    && crate::generation::container_element_base(stack.data, value).is_some()
                {
                    self.gen_set_first_ref_elem_copy(stack, v, value, d_nr);
                    return;
                }
            }
            self.set_var(stack, v, value);
            if stash_old_for_post_free {
                // #330 epilogue (fall-through path): see above.
                let free_pos = stack.var_pos(v);
                stack.add_op("OpVarRef", self);
                self.code_add(free_pos);
                stack.add_op("OpFreeRefIfDistinct", self);
            }
        } else {
            // First allocation — slot pre-assigned by assign_slots.
            #[cfg(debug_assertions)]
            assert!(
                !ir_contains_var(value, v),
                "[generate_set] first-assignment of '{}' (var_nr={v}) in '{}' contains \
                 a Var({v}) self-reference — storage not yet allocated, will produce a \
                 garbage DbRef at runtime. This is a parser bug. value={value:?}",
                stack.function.name(v),
                stack.data.def(stack.def_nr).name(),
            );
            stack.function.set_stack_allocated(v);
            // Plan-04 Phase B.3 atomic bundle: under the single
            // function-entry `OpReserveFrame(frame_hwm)`, every slot
            // lives below TOS at every first-Set.  Route all
            // first-Sets through `gen_set_first_at_tos`, which now
            // handles positional init (Text / Ref-null / Vector-null /
            // Tuple-null) or push-then-`OpPut*` (Fn-ref, Ref call
            // adopt, and every simple type via the fall-through).
            // `set_var`'s first-Set path was unsafe for Text anyway
            // (`OpAppendText` on uninitialised bytes) — routing Text
            // here is a safety improvement.
            self.gen_set_first_at_tos(stack, v, value);
        }
    }

    /// First-assignment dispatch for large-type locals.
    ///
    /// **Plan-04 Phase B.3 atomic bundle:** slot-move and gap-fill
    /// are gone.  Every local's slot is reserved by the single
    /// function-entry `OpReserveFrame(frame_hwm)` in `def_code`, so
    /// `v.stack_pos` already names a live slot in the pre-reserved
    /// frame and `stack.position >= v.stack_pos + size(v)` is an
    /// invariant maintained by codegen.  First-Set helpers write at
    /// `slot_offset = stack.position - v.stack_pos` (positional init
    /// ops) or push-then-`OpPut*(slot_offset)` (for paths whose only
    /// way to produce the value is via the eval stack).
    /// Emit per-leaf `OpPut*` ops that pop a tuple value off TOS into
    /// the slot at absolute offset `tuple_base`.  Recurses through
    /// nested tuples so an outer element of type `Tuple(...)` is
    /// flattened to its leaf scalars/refs.
    ///
    /// Iteration is reverse-order (last leaf first) because TOS holds
    /// the value pushed last — exactly the layout `generate` produces
    /// for a tuple literal evaluated left-to-right depth-first.  Each
    /// `OpPut*` consumes its slot's bytes from TOS.
    ///
    /// P212 — nested tuple literals (`((1,2),(3,4))`, triply-nested,
    /// or any tuple containing a tuple) previously panicked here
    /// because the inline match in `gen_set_first_at_tos` had no arm
    /// for `Type::Tuple(_)` elements.  Recursion handles arbitrary
    /// nesting depth using the same offset arithmetic.
    // @PLN25: every per-element tuple op in this file matches `<elem>.base()` — a nullable
    // `τ?` tuple element stores/reads through its base's OpPut*/OpGet* (same sentinel storage),
    // mirroring `gen_set_first_at_tos`. Read and write peel in lockstep so the round-trip
    // (incl. a null element) stays consistent; without it an Optional element panicked
    // "unsupported elem". The full set: emit_tuple_put_ops, emit_tuple_var_pop_put,
    // emit_tuple_var_push_recursive, emit_tuple_null_init, and the generate_node/generate_var
    // TupleGet/TuplePut element matches.
    fn emit_tuple_put_ops(&mut self, stack: &mut Stack, elems: &[Type], tuple_base: u16) {
        let offsets = crate::data::element_stack_offsets(elems);
        for i in (0..elems.len()).rev() {
            let elem_abs = tuple_base + offsets[i] as u16;
            if let Type::Tuple(inner) = &elems[i] {
                self.emit_tuple_put_ops(stack, inner, elem_abs);
                continue;
            }
            // Compute pos BEFORE add_op — `stack.add_op` calls
            // `operator(...)` which decrements `stack.position` by the
            // op's pop size, but the offset argument we encode in the
            // bytecode stream is relative to the stack position
            // *before* the pop (it's the offset from TOS at op-fire
            // time, which is then `stack_pos + size - pos` inside
            // `put_var`).  Mirrors the original flat-tuple loop.
            let pos = stack.position - elem_abs;
            match elems[i].base() {
                Type::Integer(_) => stack.add_op("OpPutInt", self),
                // P249 — fn-ref slot is 20 B (8 d_nr + 12 closure
                // DbRef).  OpPutInt would only pop 8 B and leave the
                // closure half garbage in the destination.  OpPutFnRef
                // pops 20 B; mirror gen_set_first_at_tos's
                // `fnref_signature_gap` correction (op signature
                // declares 16 B / `text` but the runtime pops 20).
                Type::Function(_, _, _) => {
                    stack.add_op("OpPutFnRef", self);
                    stack.position -= stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
                }
                Type::Boolean => stack.add_op("OpPutBool", self),
                Type::Float => stack.add_op("OpPutFloat", self),
                Type::Single => stack.add_op("OpPutSingle", self),
                Type::Character => stack.add_op("OpPutCharacter", self),
                Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                Type::Text(_) => stack.add_op("OpPutText", self),
                // See the sibling note in `emit_tuple_var_pop_put`: the DbRef-shaped set
                // is [`is_dbref`](crate::data::is_dbref)'s to answer.
                t if crate::data::is_dbref(t) => stack.add_op("OpPutRef", self),
                Type::Tuple(_) => unreachable!("handled above"),
                other => panic!("emit_tuple_put_ops: unsupported elem {other:?}"),
            }
            self.code_add(pos);
        }
    }

    fn gen_set_first_at_tos(&mut self, stack: &mut Stack, v: u16, value: &Value) {
        // @PLN25: a `text?` local's first-Set routes to the heap-aware text path (same sentinel
        // storage as plain `text`) — peel the marker. Else an `Optional(Text)` var fell through
        // to the scalar match below (no Text arm) and panicked "unsupported var type".
        if matches!(stack.function.tp(v).base(), Type::Text(_)) {
            self.gen_set_first_text(stack, v, value);
        } else if matches!(
            stack.function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && *value == Value::Null
        {
            // `.base()`: a `τ?` local's null-init is the same sentinel write as `τ`'s.  Asked
            // bare, an `Optional(Reference)` temp fell to the generic fallthrough, which lands
            // the shared null store (`Stores::null()`, a real slot with `rec == 0`) in the
            // slot instead of the sentinel — and the scope-exit `OpFreeRef` of that temp then
            // trips the stack-store net (BUG #306).  User code never reaches this arm with a
            // `τ?` (a `P? = null` is parsed to `OpNullRefSentinel`); a compiler temp's
            // preamble does.
            self.gen_set_first_ref_null(stack, v);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            && let Value::Call(op_nr, _) = value.unspan()
            && stack.data.def(*op_nr).name() == "OpCopyRecord"
        {
            // The first assignment of a Reference variable being copied from another:
            // allocate a fresh store, initialize the struct record, then copy the data.
            self.gen_set_first_ref_copy(stack, v, d_nr, value);
        } else if let Some(d_nr) = stack.function.tp(v).base().heap_def_nr()
            && let Value::Var(src) = value
            && let Some(src_d_nr) = stack.function.tp(*src).base().heap_def_nr()
            && stack.data.copies_as(d_nr, src_d_nr)
        {
            // First assignment `d = c` where both hold the same heap RECORD type — a struct
            // or a struct-enum, the two shapes `Type::heap_def_nr` names: give d its own
            // independent record by allocating storage and copying c's data.  Asked as a
            // bare `Reference`, a struct-enum bind fell through to the plain adopt below and
            // ALIASED (`e: Sh = Ci {…}; c = e; e.n = 9` read 9 through `c`), while `--native`
            // reads the same question through `heap_def_nr` and copies.
            //
            // Read through `base()`, the peel the `Text` arm at the top of this function
            // already takes and for the same reason: `S?` is `Optional(Reference(S))`, the
            // same storage behind a nullability marker, and matching the bare spelling left
            // a nullable bind on the plain-adopt fallthrough — an ALIAS, where `@FR-B-Copy`
            // says a whole-value bind is INDEPENDENT (`c2 = ns; ns.v = 99` then read 99
            // through `c2`, loft#1319).
            self.gen_set_first_ref_var_copy(stack, v, *src, d_nr);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            // @FR-O-Proxy asks copy — whether to MATERIALISE an element read into a store `v`
            // owns rather than bind the interior pointer.  The materialise is what PREVENTS
            // the container-wide free described below; it emits no free of its own.
            && stack.function.tp(v).depend().is_empty()
            // Not for a WITNESSED local (loft#1336, @FR-O-Witness): its deps are empty
            // here only because a later whole-value copy stripped them, and the
            // container-wide free this arm guards against is one such a local never
            // emits.  Its projections stay views on both backends, so the witness — which
            // classifies a projection as a view — cannot lose the store this would mint.
            && stack.function.owner_witness(v).is_none()
            && crate::generation::container_element_base(stack.data, value).is_some()
        {
            // @PLN130 F1 — MATERIALISE an element read into a store `v` owns.
            //
            // `c = v[i]` normally binds a VIEW and keeps its container dep, which is the
            // documented alias (loft#774) and stays a borrow.  Reaching here means the deps
            // are EMPTY — `scopes.rs` stripped them because `v` is reassigned later, so from
            // this point on every consumer treats `v` as an owner.  Binding the raw interior
            // pointer anyway left an owner whose "own" store is the CONTAINER's: the
            // reassignment's pre-Set `OpFreeRef` then released the whole container, and
            // `k = a[0]; for x in a { k = x }` ended with `len(a) == 0` before any removal
            // (probe 30 / loft#778).
            //
            // Native already does the right thing here — the same empty deps make its
            // generator allocate `_own_store_k` at the bind — so this makes the interpreter
            // act on the fact both backends already read, rather than changing the fact.
            // A binding that kept its deps is untouched and still aliases.
            self.gen_set_first_ref_elem_copy(stack, v, value, d_nr);
        } else if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
            && let Value::TupleGet(_, _) = value
            // @FR-O-Proxy asks copy — whether a tuple-element bind deep-copies the record
            // instead of aliasing it.  A binding whose deps say BORROW is skip-free, so
            // copying for one would leave an owner nobody frees; the guard is against a
            // stranded allocation, and the arm releases nothing.
            && stack.function.tp(v).depend().is_empty()
            // Not for a witnessed local, for the element arm's reason above.
            && stack.function.owner_witness(v).is_none()
        {
            // T1.8c: tuple destructuring `(q1, q2) = expr` — when an element
            // is Type::Reference, deep-copy the record to avoid aliasing.
            //
            // ONLY when the binding OWNS, which is what empty deps mean.  The copy
            // allocates a store and puts the binding in charge of it, and a binding that
            // deps say is a BORROW is skip-free — so the copy had an owner nobody frees,
            // and `t = (a, b); (ca, cb) = t` leaked one record per struct element on
            // every execution, unbounded in a loop (loft#1051).
            //
            // The guard is the one the element-read arm above already uses, for the same
            // reason and out of the same rule: ownership.md O-Deps — the free placement
            // DERIVES from `deps`, and a codegen condition that re-derives it is the bug.
            // This condition re-derived "copy" without consulting the fact.
            //
            // It also closes a backend divergence, which is the other half of O-Deps:
            // `--native` reads the same deps and aliases a borrow, so it was always
            // clean here while the interpreter leaked.  A binding that keeps its deps
            // now aliases on BOTH backends, exactly as the undestructured read `ca = t.0`
            // already did on both.
            self.gen_set_first_ref_tuple_copy(stack, v, value, d_nr);
        } else if let Some((join_d_nr, base)) = crate::use_analysis::callref_join_first_bind(
            stack.data,
            stack.def_nr,
            stack.function.tp(v),
            value,
        ) {
            // loft#1248 — a first bind from a CLOSURE call whose return may borrow.  The arm
            // below that asks this for a direct call is keyed on `Value::Call`, so a
            // `CallRef` — which names a runtime value rather than a definition — reached
            // neither it nor the plain-adopt paths' deps strip.  The bind stayed an alias
            // carrying the argument's dep, and the arm where the closure MINTED its store
            // left it with no owner: one per call, released only when the CALLER's frame
            // exits, so a loop reaches the store-table ceiling.
            //
            // Same guard as its direct-call twin, for the same reason: `OpBindOrCopy` copies
            // when the returned store is the witness's and adopts when it is not, so the
            // scope-exit free the deps strip enables is right on both arms.
            self.gen_set_first_ref_join(stack, v, value, join_d_nr, base);
        } else if let Some((join_d_nr, base)) = crate::use_analysis::nullable_join_first_bind(
            stack.data,
            stack.def_nr,
            stack.function.tp(v),
            value,
        ) {
            // loft#1106 — a NULLABLE heap local (`r: S?`) first-bound from a call whose
            // return may borrow one of its arguments.  `S?` is `Optional(Reference(S))`,
            // and every arm of this dispatch asks its shape question against the bare
            // type, so the nullable spelling of the same storage reached none of them:
            // the bind stayed a plain `OpPutRef` alias.  A write through the result then
            // reached the caller's own variable — which `(B-Copy)` says a bind cannot do —
            // and the arm where the callee MINTED its store left it with no owner.
            //
            // The `-> S` twin of the same call already resolves this per execution, and
            // this is that bind: `OpBindOrCopy` copies when the returned store is the
            // witness's and adopts when it is not, so the scope-exit free is right on
            // both arms.  It is also what makes a `null` answer safe — a null `src` has
            // no store to alias, so the guard adopts it and the local stays null.
            self.gen_set_first_ref_join(stack, v, value, join_d_nr, base);
        } else if let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) =
            stack.function.tp(v).clone()
            // loft#1245 — BOTH spellings of a call, because a `CallRef` reaching a
            // definition is the same question as a `Call` reaching one and only the
            // second was asked.  Reading `Value::Call` alone sent every fn-ref bind to
            // the plain-adopt fallthrough at the bottom of this dispatch: a borrowed
            // return was then ALIASED (a write through the bind reached the caller's own
            // variable, which B-Copy forbids) and a minted one was left with no owner.
            // An unresolved fn-ref answers `None` and keeps that fallthrough, which is
            // the behaviour every fn-ref had before.
            && let Some(fn_nr) = crate::use_analysis::callee_of(stack.data, stack.def_nr, value)
            // The shared gate on the carried ownership facts (`Def::is_loft_defined`) —
            // methods and generic monomorphs (`t_`) reach this arm too.  While it read
            // `n_` alone, a method returning through the caller's `__ref_N` fell to the
            // plain-adopt fallthrough at the bottom of this dispatch and was then freed
            // as an owner, taking the caller's buffer with it (loft#810).
            && stack.data.def(fn_nr).is_loft_defined()
        {
            // Cluster A.3 (OWNERSHIP_MODEL row 102): read the carried
            // adopt-vs-copy fact.  When the callee returns a genuinely FRESH
            // store (empty dep, or the `["??"]` one-buffer marker), the caller
            // adopts it directly — no deep copy needed (OpAppendVector in
            // handle_field already deep-copies vector field data into the
            // struct's store during construction).  Otherwise the return is
            // tied to a passed buffer/param — a visible param it aliases, or a
            // hidden `ref_return` work-ref the caller REUSES across iterations —
            // and must be deep-copied into a fresh store `v` owns.  This reads
            // `return_adopts_fresh_store()` rather than re-deriving the coarse
            // "any visible ref param" proxy (A.3).
            //
            // #429: a struct-enum (`Type::Enum(_, true, _)`) binding is a heap
            // record exactly like `Type::Reference`, so it needs the SAME
            // copy-or-adopt split.  Before, only `Reference` had this arm — a
            // `acl = get_field(m, k)` whose callee returns a BORROWED view of
            // `m` (`["m"]`) fell through to a plain adopt, then `OpFreeRef(acl)`
            // at scope exit whole-store-freed `m`'s record (interp-only;
            // native's runtime adopt-or-copy guard already handled it).  The
            // record copy (`OpDatabase` + `OpCopyRecord`, keyed on the enum's
            // `known_type`) is identical for an enum d_nr, mirroring
            // `materialize_return_into`'s `Type::Enum(td, true, _)` leg.
            // @PLN85 join delivery, first-Set half: when the callee's return is a
            // runtime `??` JOIN — owned on one arm, a borrow of `base` on the other —
            // neither static answer below is right for both arms, so bind through the
            // same `OpBindOrCopy` store-identity guard the REASSIGNMENT path uses.
            if crate::keys::join_own_enabled()
                && let Type::Reference(join_d_nr, _) = stack.function.tp(v).clone()
                && let crate::use_analysis::Own::Join { base } =
                    crate::use_analysis::ownership_of(stack.data, stack.def_nr, value)
                && base != u16::MAX
            {
                self.gen_set_first_ref_join(stack, v, value, join_d_nr, base);
            } else if stack.data.def(fn_nr).return_adopts_fresh_store() {
                // runtime tolerates double-free as a no-op so leaving
                // __ref_N to be freed by scopes.rs's is_work_ref gate at
                // scope exit is safe in both adoption and orphan cases.
                // Plan-04 Phase B.3 atomic bundle: the callee's returned
                // DbRef is pushed onto TOS by `generate`; move it to v's
                // slot via `OpPutRef(slot_offset)`.
                self.generate(value, stack, false);
                let var_pos = stack.var_pos(v);
                stack.add_op("OpPutRef", self);
                self.code_add(var_pos);
            } else {
                self.gen_set_first_ref_call_copy(stack, v, value, d_nr);
            }
        } else if *value == Value::Null
            && matches!(stack.function.tp(v), Type::Optional(_))
            && matches!(stack.function.tp(v).base(), Type::Vector(_, _))
        {
            self.gen_set_first_nullable_collection_null(stack, v);
        } else if matches!(stack.function.tp(v), Type::Vector(_, _)) && *value == Value::Null {
            self.gen_set_first_vector_null(stack, v);
        } else if crate::parser::vectors::owns_keyed_store(stack.function.tp(v))
            && *value == Value::Null
        {
            self.gen_set_first_keyed_null(stack, v);
        } else if matches!(stack.function.tp(v), Type::Tuple(_)) && *value == Value::Null {
            self.gen_set_first_tuple_null(stack, v);
        } else if matches!(stack.function.tp(v), Type::Function(_, _, _)) {
            // Plan-04 Phase B.3 atomic bundle: push the 20-byte fn-ref
            // value (8B d_nr + 12B closure DbRef) then OpPutFnRef at v's
            // slot.  `OpPutFnRef`'s stdlib signature pops a 16-byte
            // `text`, so the runtime actually pops 20 bytes but the
            // compile-time tracker only accounts for 16 — mirror
            // `set_var`'s `fnref_signature_gap` correction.
            if *value == Value::Null {
                stack.add_op("OpConstInt", self);
                self.code_add(i64::MIN);
                self.emit_push_sentinel(stack);
            } else {
                self.gen_fn_ref_value(value, stack);
            }
            let var_pos = stack.var_pos(v);
            stack.add_op("OpPutFnRef", self);
            self.code_add(var_pos);
            stack.position -= stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
        } else if matches!(stack.function.tp(v), Type::RefVar(_))
            && let Value::Var(src) = value
            && matches!(stack.function.tp(*src), Type::RefVar(_))
        {
            // #257: aliasing a `&ref` param into a fresh local (`snap = s`).
            // Both sides are RefVars — a RefVar slot is double-indirect (it
            // holds a stack-cell DbRef that itself points at the record).  Copy
            // the RAW cell: emit OpVarRef WITHOUT generate_var's trailing
            // OpGetStackRef deref.  Going through `generate` would store the
            // already-dereferenced record ref, so every later `snap.field`
            // access (which derefs once more) would read one level too far →
            // garbage / null write.
            let src_pos = stack.var_pos(*src);
            stack.add_op("OpVarRef", self);
            self.code_add(src_pos);
            let var_pos = stack.var_pos(v);
            stack.add_op("OpPutRef", self);
            self.code_add(var_pos);
        } else {
            // Plan-04 Phase B.3 atomic bundle: push then OpPut* at v's
            // slot.  Mirrors `set_var`'s non-RefVar dispatch at
            // codegen.rs:2313.  `Value::Null` on a type where `generate`
            // produces no bytes leaves the slot uninitialised (old
            // fall-through behaviour) — detected via a stack-position
            // delta check.
            let before = stack.position;
            // loft#1069 — a fn-ref TUPLE MEMBER has to arrive as the whole 20-byte slot
            // (8B d_nr + 12B closure DbRef), because `emit_tuple_put_ops` POPS twenty for
            // it.  A bare `dbl` lowers to `Value::Int(d_nr)` and pushes only eight, so the
            // pop ran twelve bytes into whatever sat below it — every other member's offset
            // shifted, and the tuple's own base moved under the reader.  That is why
            // `t.1`, a plain integer, faulted exactly as `t.0(3)` did: the member that
            // broke the tuple was not the member being read.
            //
            // The top-up is `gen_fn_ref_value`, which every OTHER position for the same
            // value already routes through (a plain local's first Set, a return, a call
            // argument).  It has to happen HERE rather than in the tuple walker because
            // only the destination names the element as a `fn(…)`: a bare `d_nr` generates
            // as an `integer`, so the value cannot say what it is.
            if let Type::Tuple(elems) = stack.function.tp(v).base().clone()
                && let Value::Tuple(items) = value.unspan()
                && items.len() == elems.len()
                && elems.iter().any(crate::data::tuple_carries_fn_ref)
            {
                self.gen_tuple_members_with_fn_refs(&elems, items, stack);
            } else {
                self.generate(value, stack, false);
            }
            if stack.position == before {
                return;
            }
            let var_pos = stack.var_pos(v);
            let tp = stack.function.tp(v).clone();
            // @PLN25 slice (b): an `Optional(τ)` var's first-Set uses the base put-op (same
            // sentinel storage) — peel the marker.
            let tp = tp.base().clone();
            match tp {
                Type::Integer(_) => stack.add_op("OpPutInt", self),
                Type::Character => stack.add_op("OpPutCharacter", self),
                Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
                Type::Boolean => stack.add_op("OpPutBool", self),
                Type::Single => stack.add_op("OpPutSingle", self),
                Type::Float => stack.add_op("OpPutFloat", self),
                Type::Vector(_, _)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::Iterator(_, _)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _) => stack.add_op("OpPutRef", self),
                // @PLN87 L1 — first-Set of a local `&`-link (`b = &a` →
                // `b: &T = OpCreateStack(a)`): the value is the 12-byte stack-cell
                // ref; store it raw into `b`'s RefVar slot.  Reads/writes of `b`
                // then deref the cell to the source's slot.
                Type::RefVar(_) => stack.add_op("OpPutRef", self),
                Type::Tuple(elems) => {
                    let tuple_var_base = stack.function.stack(v);
                    self.emit_tuple_put_ops(stack, &elems, tuple_var_base);
                    return;
                }
                other => panic!(
                    "gen_set_first_at_tos fallthrough: unsupported var type {other:?} for \
                     first-Set of {}",
                    stack.function.name(v)
                ),
            }
            self.code_add(var_pos);
        }
    }

    /// First-assignment reference copy from a Call(OpCopyRecord, ...).
    ///
    /// **Plan-04 Phase B.3.b:** slot-aware.  `v` is the first-assign
    /// target; the fallback path now uses
    /// `slot_offset = stack.position - v.stack_pos` for `OpInitRef` +
    /// `OpDatabase` so the DbRef allocation lands in `v`'s slot
    /// regardless of where V1 placed it.  Under the current
    /// slot-move invariant (slot-move in `gen_set_first_at_tos` is
    /// still active through B.3.g), `v.stack_pos == stack.position`
    /// at entry, so `slot_offset` equals `ref_size = 12` after the
    /// intervening `OpReserveFrame(12)` — bytecode-identical to B.2.
    /// Slot-move is removed in B.3.h, at which point `slot_offset`
    /// reflects V1's actual placement.
    fn gen_set_first_ref_copy(&mut self, stack: &mut Stack, v: u16, d_nr: u32, value: &Value) {
        // O-B2: if the inner call returns a genuinely FRESH store (empty dep or
        // the `["??"]` one-buffer marker), adopt it directly instead of deep
        // copying.  This eliminates both the copy overhead and the store leak.
        // Reads the carried `return_adopts_fresh_store()` fact (OWNERSHIP_MODEL
        // row 102), matching the `gen_set_first_at_tos` adopt-vs-copy decision
        // (A.3).  NOTE: this `Call(OpCopyRecord, [Call(inner), …])` shape does
        // not arise in the current corpus (the whole-suite usage sentinel found
        // zero fires) — it is kept consistent with its sibling sites so a future
        // revival reads the same precise fact, not the old coarse proxy.
        if let Value::Call(_, args) = value.unspan()
            && !args.is_empty()
            && let Value::Call(inner_nr, _) = &args[0]
            && matches!(
                stack.data.def(*inner_nr).returned(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && stack.data.def(*inner_nr).return_adopts_fresh_store()
        {
            // Safe: the inner call's return is owned/fresh, never an aliased store.
            // Generate just the inner call — its result DbRef is on the eval stack.
            // Plan-04 Phase B.3.h.3: slot-aware.  Move the pushed DbRef to
            // v's slot via OpPutRef(slot_offset).  Under slot-move (still
            // active through B.3.h), slot_offset equals 12 after the push,
            // so OpPutRef copies the DbRef bytes onto themselves — harmless
            // bytecode-level overhead.  Once slot-move is removed, the
            // OpPutRef becomes the mechanism that lands the result in v's
            // actual slot.
            self.generate(&args[0], stack, false);
            let var_pos = stack.var_pos(v);
            stack.add_op("OpPutRef", self);
            self.code_add(var_pos);
            return;
        }
        // Fallback: allocate fresh store + deep copy (source store may alias a parameter).
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        // Bump TOS so v's slot is fully below stack.position, then emit
        // positional init + allocator at slot_offset.
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRef", self);
        self.code_add(slot_offset);
        stack.add_op("OpDatabase", self);
        self.code_add(slot_offset);
        let tp_nr = stack.data.def(d_nr).known_type();
        self.code_add(tp_nr);
        self.generate(value, stack, false);
        // @PLN130 — past the adopt-the-fresh-store return above: this arm deep-copies.
        crate::copy_manifest::record(
            stack.def_nr,
            v,
            tp_nr,
            crate::copy_manifest::Origin::InterpCallReturn,
        );
    }

    /// First-assignment reference copy from another variable of the same type.
    ///
    /// **Plan-04 Phase B.3.c:** deep-copy path is slot-aware; the
    /// O-B1 last-use-move shortcut at the top is untouched because it
    /// already addresses v via the push-and-stack-position-tracking
    /// pattern that works the same whether slot-move is active or not
    /// (v.stack_pos keeps V1's value when OpVarRef just pushes a copy
    /// of src's DbRef).
    fn gen_set_first_ref_var_copy(&mut self, stack: &mut Stack, v: u16, src: u16, d_nr: u32) {
        // O-B1: last-use move — if source is only read once (this assignment),
        // transfer the DbRef instead of deep copying. Skip the source's OpFreeRef.
        //
        // A move hands `v` the source's store and suppresses the source's free, so it is
        // sound only where the source HAS a store to give.  `uses(src) == 1` answers
        // LIVENESS — nobody reads it after this — which says nothing about ownership; a
        // non-empty type dep says positively that `src` is a VIEW of another variable
        // (loft#823).  Moving from a view handed `v` an interior pointer into the
        // container, and `v`'s scope-exit `OpFreeRef` then released the CONTAINER's
        // record: `for p in v { g = p; … }` SIGSEGV'd for every heap element type — a
        // struct as readily as a tuple — whenever an earlier loop had pushed the use
        // count to exactly 1.  The cells that survived did so on that accident of
        // counting, not on any ownership fact.
        //
        // @PLN90's `use_analysis::move_elidable_source` states the same rule for the
        // move plans it builds — "never a view/projection, which owns no store to move".
        // This is the shortcut that predates it.
        if stack.function.uses(src) == 1
            && stack.function.tp(src).depend().is_empty()
            && !stack.function.is_argument(src)
            && !stack.function.is_captured(src)
            // @FR-O-Proxy asks free, so @FR-O-Override applies.  A move is a free
            // decision made one binding away: `v` takes `src`'s store and `v`'s scope-exit
            // `OpFreeRef` releases it.  If the proxy was wrong about `src` that release lands
            // on a store someone else owns — the same shape as the view this arm already
            // refuses, reached through the flag instead of through the deps.
            && !stack.function.is_skip_free(src)
        {
            let src_pos = stack.var_pos(src);
            stack.add_op("OpVarRef", self);
            self.code_add(src_pos);
            stack.position += stack.step(size_of::<crate::keys::DbRef>() as u16);
            stack.function.set_skip_free(src);
            // Plan-04 Phase B.3.h.4: slot-aware — move the pushed DbRef
            // from TOS to v's slot via OpPutRef.  Under slot-move (still
            // active through B.3.h), var_pos equals 12 and this is a
            // behavior-preserving bytecode-level overhead (copy onto
            // self).  Once slot-move is removed, this becomes the
            // mechanism that lands the transferred DbRef in v's slot.
            let var_pos = stack.var_pos(v);
            stack.add_op("OpPutRef", self);
            self.code_add(var_pos);
            return;
        }
        let tp_nr = stack.data.def(d_nr).known_type();
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        // Bump TOS so v's slot is fully below stack.position, then emit
        // positional init + allocator at slot_offset.
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRef", self);
        self.code_add(slot_offset);
        // A source that may be ABSENT cannot take the plain allocate-then-copy: the copy
        // dereferences the source store, so a `null` one indexes `allocations[u16::MAX]`,
        // and copying NOTHING instead would be worse than the panic — the destination would
        // keep the record allocated for it and read PRESENT where its source was absent,
        // which is emptiness standing in for absence (the trade `Stores::vector_replace`
        // records for the collections).
        //
        // `OpBindOrCopy` decides that at runtime, and with the source as its OWN witness it
        // says exactly what `@FR-B-Copy` wants here: a present source aliases the witness,
        // so the borrow arm materialises a fresh store and deep-copies into it; an absent
        // one fails the `store_nr != u16::MAX` half and is ADOPTED, which lands the true
        // sentinel and leaves the destination null.  It allocates on the arm that needs it,
        // so no `OpDatabase` is emitted ahead of it.
        //
        // Asked of both sides through the one predicate the native record bind asks
        // (`Variables::bind_admits_absence`): a source typed non-null can still hold
        // `nullref`, and a nullable destination has to receive it as absence.
        if stack.function.bind_admits_absence(v, src) {
            self.generate(&Value::Var(src), stack, false);
            // A PUSH op reads at the PRE-push position, so the witness offset is taken
            // BEFORE `add_op`; the slot the opcode POPS into is taken after, at the
            // post-pop position.  Same order as the `Join` emission in `generate_set`.
            let witness_pos = stack.var_pos(src);
            stack.add_op("OpVarRef", self);
            self.code_add(witness_pos);
            stack.add_op("OpBindOrCopy", self);
            self.code_add(stack.var_pos(v));
            self.code_add(tp_nr);
        } else {
            stack.add_op("OpDatabase", self);
            self.code_add(slot_offset);
            self.code_add(tp_nr);
            let copy_nr = stack.data.def_nr("OpCopyRecord");
            let copy_val = Value::Call(
                copy_nr,
                vec![Value::Var(src), Value::Var(v), Value::Int(i32::from(tp_nr))],
            );
            self.generate(&copy_val, stack, false);
        }
        // @PLN130 — recorded HERE, past the last-use-move return above, so the manifest
        // claims a copy only where one is actually written.
        crate::copy_manifest::record(
            stack.def_nr,
            v,
            tp_nr,
            crate::copy_manifest::Origin::InterpRecordBind,
        );
    }

    /// First-assignment materialisation of a container element read into a store `v` owns.
    ///
    /// Same emission shape as [`Self::gen_set_first_ref_tuple_copy`] — allocate at `v`'s
    /// slot, then deep-copy the interior pointer's record into it — but reached from an
    /// element/field read whose deps were stripped, so `v` is an owner (@PLN130 F1).
    fn gen_set_first_ref_elem_copy(&mut self, stack: &mut Stack, v: u16, value: &Value, d_nr: u32) {
        let tp_nr = stack.data.def(d_nr).known_type();
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRef", self);
        self.code_add(slot_offset);
        stack.add_op("OpDatabase", self);
        self.code_add(slot_offset);
        self.code_add(tp_nr);
        // `OpCopyRefOrNull`, not `OpCopyRecord` — the element being read may not BE there.
        // `k = v[oob]` allocated the destination record above and then deep-copied from an
        // absent source, leaving `k` bound to that live-but-uninitialised record: `k == null`
        // read false and `k.x` answered its bytes (loft#823).  This opcode is the null-aware
        // sibling built for exactly that shape — it reclaims the pre-allocated record and
        // binds the sentinel when the source is absent, and deep-copies otherwise.
        // `generate(value)` pushes the source and the opcode pops it, so the slot offset
        // taken above stays valid.
        self.generate(value, stack, false);
        stack.add_op("OpCopyRefOrNull", self);
        self.code_add(slot_offset);
        self.code_add(tp_nr);
        crate::copy_manifest::record(
            stack.def_nr,
            v,
            tp_nr,
            crate::copy_manifest::Origin::InterpRecordBind,
        );
    }

    /// First-assignment reference from tuple destructuring — deep copy.
    ///
    /// **Plan-04 Phase B.3.d:** slot-aware — uses
    /// `slot_offset = stack.position - v.stack_pos` (after a bump
    /// to make room for the DbRef) for `OpInitRef` + `OpDatabase`.
    fn gen_set_first_ref_tuple_copy(
        &mut self,
        stack: &mut Stack,
        v: u16,
        value: &Value,
        d_nr: u32,
    ) {
        let tp_nr = stack.data.def(d_nr).known_type();
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRef", self);
        self.code_add(slot_offset);
        stack.add_op("OpDatabase", self);
        self.code_add(slot_offset);
        self.code_add(tp_nr);
        let copy_nr = stack.data.def_nr("OpCopyRecord");
        let copy_val = Value::Call(
            copy_nr,
            vec![value.clone(), Value::Var(v), Value::Int(i32::from(tp_nr))],
        );
        self.generate(&copy_val, stack, false);
        // @PLN130 — tuple destructuring deep-copies unconditionally.
        crate::copy_manifest::record(
            stack.def_nr,
            v,
            tp_nr,
            crate::copy_manifest::Origin::InterpTupleBind,
        );
    }

    /// First-assignment reference from a call whose return is a runtime `??` JOIN —
    /// owned on one arm, a borrow of `base` on the other.  Binds it through @PLN85's
    /// `OpBindOrCopy` store-identity guard, which adopts the fresh arm and deep-copies
    /// the borrow arm, so `v`'s ordinary scope-exit `OpFreeRef` is correct either way.
    ///
    /// The reassignment path (`generate_set`) has resolved this per execution since
    /// @PLN85; a first Set had only the STATIC verdict, and a static verdict is wrong on
    /// one of the two arms by construction.  `returns_borrowed_view()` answered "borrow",
    /// so [`gen_set_first_ref_call_copy`] suppressed the source-free — and every
    /// execution taking the FRESH arm orphaned the store the callee had just minted.  A
    /// loop body re-declares its local every iteration, which made that a first Set every
    /// time and scaled the leak with the trip count (the `j1`/`j3` probe cells).
    ///
    /// The guard needs a live local to witness `base` against.  When none exists — the
    /// base reaches the callee as a field expression, `f(b.items, i)` — `ownership_of`
    /// answers `base == u16::MAX` and the caller falls through to the static split, where
    /// the fresh arm still leaks (`j6`, which leaks on the native backend too).
    fn gen_set_first_ref_join(
        &mut self,
        stack: &mut Stack,
        v: u16,
        value: &Value,
        d_nr: u32,
        base: u16,
    ) {
        let tp_nr = stack.data.def(d_nr).known_type();
        // The slot must exist and hold a sentinel before `OpBindOrCopy` writes it: the
        // borrow arm allocates INTO the slot and the owned arm overwrites the DbRef
        // there.  Same preamble as `gen_set_first_ref_call_copy`, minus its `OpDatabase`
        // — the record is the guard's to allocate, and only on the borrow arm.
        //
        // `OpInitRefSentinel`, NOT `OpInitRef`: the latter writes `Stores::null()`,
        // which ALLOCATES an empty store.  `gen_set_first_ref_call_copy` gets away with
        // that because its `OpDatabase` immediately reuses the store in place, but the
        // owned arm here overwrites the slot with the callee's DbRef, so that store
        // would be orphaned once per binding.  The sentinel costs nothing, and
        // `alloc_record_at` already turns a `u16::MAX` slot into a real store for the
        // borrow arm.
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        // The call result (`src`) FIRST, then the witness on top — so the witness's
        // frame-relative `var_pos` accounts for `src` already sitting on the eval stack.
        // `OpBindOrCopy` pops witness, then src.  A PUSH op reads at the PRE-push
        // position, so `var_pos(base)` is taken BEFORE `add_op`; the POP op's
        // `var_pos(v)` is taken AFTER it.  (Mirrors the reassignment site exactly.)
        self.generate(value, stack, false);
        // The sentinel goes in AFTER the call is evaluated, which is the @P290 rule the
        // reassignment path states: touch the destination only once the call no longer
        // needs what is there.  `v` is a FIRST bind, so the call cannot name it — but it
        // can name a variable the slot allocator gave `v`'s slot to, and that is not the
        // same question.  A lifted argument is exactly that neighbour: it dies at the call
        // and its slot is reused for the very local the call is bound to, so a sentinel
        // written first replaced the argument with `null` between its write and its read.
        // Writing it here still satisfies `OpBindOrCopy`'s precondition — the slot holds a
        // sentinel before the guard writes it, which is all the borrow arm's
        // `alloc_record_at` needs.
        let witness_pos = stack.var_pos(base);
        stack.add_op("OpVarRef", self);
        self.code_add(witness_pos);
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRefSentinel", self);
        self.code_add(slot_offset);
        stack.add_op("OpBindOrCopy", self);
        self.code_add(stack.var_pos(v));
        self.code_add(tp_nr);
    }

    /// First-assignment reference from a function call — deep copy to prevent aliasing.
    ///
    /// Sets the `0x8000` "free source" bit on `OpCopyRecord` so the
    /// callee store is released, but FIRST locks every ref-typed
    /// argument of the call, then unlocks them after the copy.  The
    /// `OpCopyRecord` code at `src/state/io.rs:1001` skips the
    /// source-free when the source store is `locked`, so an
    /// early-return that aliases one of the args (e.g.
    /// `return arg.field[i]` inside `for ... in arg.collection { ... }`)
    /// does not free part of the caller's argument.  Args that were
    /// already locked at function entry (const params) get a no-op lock
    /// + a redundant-but-harmless unlock — `n_set_store_lock` doesn't
    ///   touch program-lifetime locked stores (rc >= u32::MAX/2).
    fn gen_set_first_ref_call_copy(&mut self, stack: &mut Stack, v: u16, value: &Value, d_nr: u32) {
        let tp_nr = stack.data.def(d_nr).known_type();
        // Plan-04 Phase B.3.e: slot-aware init + allocate.  The
        // subsequent `n_set_store_lock(…)` calls between the
        // allocator and the `OpCopyRecord` are void
        // and leave `stack.position` unchanged, so `slot_offset`
        // computed here stays valid across them.
        let ref_size = size_of::<crate::keys::DbRef>() as u16;
        let slot_end = stack.function.stack(v).saturating_add(ref_size);
        if stack.position < slot_end {
            let bump = stack.step(slot_end) - stack.position;
            stack.add_op("OpReserveFrame", self);
            self.code_add(bump);
            stack.position += bump;
        }
        let slot_offset = stack.var_pos(v);
        stack.add_op("OpInitRef", self);
        self.code_add(slot_offset);
        stack.add_op("OpDatabase", self);
        self.code_add(slot_offset);
        self.code_add(tp_nr);

        // Collect ref-typed args of the call to bracket with lock/unlock
        // so OpCopyRecord's `0x8000` source-free skips them.  loft#981/#982 — ONE
        // derivation, shared with the source-free gate below (which asks whether these
        // cover every ref argument) and with the native backend.
        let ref_args: Vec<u16> =
            crate::use_analysis::protectable_ref_args(stack.data, stack.def_nr, value).0;
        // @PLAN51 Cluster II Step 2 — collect caller-hidden-buf work-ref
        // args for the post-wrap sentinel reset (see the reassignment
        // path's matching comment for rationale).
        let caller_hidden_args: Vec<u16> = if let Value::Call(_, args) = value.unspan() {
            args.iter()
                .filter_map(|a| {
                    if let Value::Var(av) = a.unspan()
                        && *av != v
                        && stack.function.is_caller_hidden_buf(*av)
                    {
                        Some(*av)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        // @P290 — soft free-protect over the deep-copy; writes / claims
        // from the callee remain legal.  Switched from `n_set_store_lock`
        // (the hard user-facing `d#lock` tripwire) to dedicated
        // `n_protect_store_frees` so the two semantics no longer collide.
        let protect_fn = stack.data.def_nr("n_protect_store_frees");
        let unprotect_fn = stack.data.def_nr("n_unprotect_store_frees");
        for av in &ref_args {
            let prot = Value::Call(protect_fn, vec![Value::Var(*av)]);
            self.generate(&prot, stack, false);
        }
        // Cluster-A A.4: ONE return-ownership query, shared with the native
        // backend (`generation/dispatch.rs`).  Hidden-only deps are either
        // canonical (same-store no-op gate handles) or fresh S1 (0x8000 safely
        // frees, paired with caller_hidden_args reset below).
        // loft#981/#982 — a borrowed-view return is not always a borrow; the bracket
        // above decides per execution.  Same fact, same three emitters (the sibling
        // reassignment path here, and native `generation/dispatch.rs`).
        #[cfg(not(feature = "wasm"))]
        let tp_with_free =
            if crate::use_analysis::call_return_frees_source(stack.data, stack.def_nr, value) {
                i32::from(tp_nr) | 0x8000
            } else {
                i32::from(tp_nr)
            };
        #[cfg(feature = "wasm")]
        let tp_with_free = i32::from(tp_nr);
        // Push the call result, then OpCopyRefOrNull binds it into v's slot: a
        // null return (sentinel) frees the pre-allocated record and sets the slot
        // null (so `v == null` holds); otherwise it deep-copies into that record
        // exactly as OpCopyRecord would (ABI-B nullable-struct-return fix).
        // `slot_offset` is v's slot (same one OpInitRef/OpDatabase used above);
        // generate(value) pushes src and the opcode pops it, so the stack returns
        // to that level and the slot offset stays valid.
        self.generate(value, stack, false);
        stack.add_op("OpCopyRefOrNull", self);
        self.code_add(slot_offset);
        self.code_add(tp_with_free as u16);
        for av in &ref_args {
            let unprot = Value::Call(unprotect_fn, vec![Value::Var(*av)]);
            self.generate(&unprot, stack, false);
        }
        // @PLAN51 Cluster II Step 2 — post-wrap free + sentinel reset
        // (see the reassignment-path comment for rationale).
        for av in &caller_hidden_args {
            let slot_offset = stack.var_pos(*av);
            stack.add_op("OpVarRef", self);
            self.code_add(slot_offset);
            stack.add_op("OpFreeRef", self);
            stack.add_op("OpInitRefSentinel", self);
            self.code_add(slot_offset);
        }
        // @PLN130 — the call-return deep copy that actually fires.  Its sibling
        // `gen_set_first_ref_copy` handles a `Call(OpCopyRecord, [Call(inner), …])` shape the
        // corpus never produces (its own doc records zero fires), so instrumenting that one
        // alone left every call-return copy off the manifest.
        crate::copy_manifest::record(
            stack.def_nr,
            v,
            tp_nr,
            crate::copy_manifest::Origin::InterpCallReturn,
        );
    }

    pub(super) fn clear_stack(&mut self, stack: &mut Stack, loop_nr: u16) {
        let loop_pos = stack.loop_position(loop_nr);
        if stack.position > loop_pos {
            stack.add_op("OpFreeStack", self);
            self.code_add(0u8);
            self.code_add(stack.position - loop_pos);
            stack.position = loop_pos;
        }
    }

    /// Destination-passing for text-producing natives inside `OpAppendText`.
    /// Returns true if the optimisation was applied (caller should return Void).
    fn try_text_dest_pass(&mut self, stack: &mut Stack, op: u32, parameters: &[Value]) -> bool {
        if stack.data.def(op).name() != "OpAppendText" || parameters.len() < 2 {
            return false;
        }
        // P217-class footgun: the parser wraps the appended RHS in a
        // `Value::Span` for source-position tracking, so matching `&parameters[1]`
        // as a bare `Value::Call` silently misses every `out += native()` and
        // routed it through scratch instead of `_dest`.  Unspan both operands.
        let (Value::Var(dest_var), Value::Call(inner_op, inner_args)) =
            (parameters[0].unspan(), parameters[1].unspan())
        else {
            return false;
        };
        let inner_name = stack.data.def(*inner_op).name().to_owned();
        // @PLN24 arc D — the `#c` twin. This is the THIRD place a text producer
        // has to be recognised (`set_var`'s two branches are the others), and the
        // one that is easy to miss: it fires for the `w += producer()` IR the
        // assignment lowering builds, which is what a call inside a struct
        // literal, a vector literal, a `match` subject or a field write becomes.
        // Missing it here is not a missed optimisation for a `#c` binding the way
        // it is for a `_dest` native — a `_dest` native has a working non-dest
        // sibling to fall back to, and a `#c` binding has none.
        if is_c_text_call(stack.data.def(*inner_op))
            && let Some(&c_lib) = self.library_names.get(&inner_name)
        {
            let (dest_var, inner_op) = (*dest_var, *inner_op);
            let inner_args = inner_args.clone();
            self.gen_cdylib_text_dest_call(stack, dest_var, inner_op, &inner_args, c_lib);
            return true;
        }
        if !is_text_dest_native(&inner_name) {
            return false;
        }
        let dest_name = inner_name.clone() + "_dest";
        let Some(&lib_nr) = self.library_names.get(&dest_name) else {
            return false;
        };
        let dest_var = *dest_var;
        let inner_op = *inner_op;
        let inner_args = inner_args.clone();
        let inner_attrs: Vec<Type> = stack
            .data
            .def(inner_op)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect();
        self.gen_dest_call_args(stack, &inner_args, &inner_attrs);
        let dep_offset = stack.var_pos(dest_var);
        self.emit_push_create_stack(stack, dep_offset);
        stack.add_op("OpStaticCall", self);
        self.code_add(lib_nr);
        for attr_type in &inner_attrs {
            stack.position -= stack.step(size(attr_type, &Context::Argument));
        }
        stack.position -= stack.step(size_of::<crate::keys::DbRef>() as u16);
        true
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_call(
        &mut self,
        stack: &mut Stack,
        op: u32,
        parameters: IrNodeList,
    ) -> Type {
        // @PLN11 G2/M3.15 — materialise-at-boundary (see generate_set).
        // generate_call threads `parameters` through gather_key /
        // try_text_dest_pass and inspects `parameters[0]` as a `Var`; rather
        // than convert that whole &[Value] cluster, materialise the param list
        // to native once (compile-time clones / store read_value) and run the
        // existing body unchanged.  Makes the Call path store-capable.
        let params_owned: Vec<Value> = parameters.iter().map(|p| p.to_owned_value()).collect();
        let parameters = &params_owned[..];
        let mut tps = Vec::new();
        let mut last = 0;
        let mut was_stack = u16::MAX;
        assert!(
            parameters.len() >= stack.data.def(op).attributes().len(),
            "Too few parameters on {} (got {}, need {})",
            stack.data.def(op).name(),
            parameters.len(),
            stack.data.def(op).attributes().len(),
        );
        // S34: suppress OpFreeRef for variables moved to a shared slot by Option A.
        // The outer variable at the same slot emits its own OpFreeRef; emitting a
        // second one would produce a double-free of the same database record.
        if stack.data.def(op).name() == "OpFreeRef"
            && let Some(Value::Var(v)) = parameters.first()
            && stack.function.is_skip_free(*v)
        {
            return Type::Void;
        }
        // free the closure DbRef embedded at offset+8 in a 20-byte fn-ref
        // slot.  OpFreeRef normally reads from offset+0, but the fn-ref layout
        // is [d_nr 8B][closure DbRef 12B], so the closure is at var_pos - 8.
        // OpNullRefSentinel produces store_nr=u16::MAX; database.free() is a no-op
        // for that sentinel, so non-capturing lambdas are safe.
        if stack.data.def(op).name() == "OpFreeRef"
            && let Some(Value::Var(v)) = parameters.first()
            && matches!(stack.function.tp(*v), Type::Function(_, _, _))
        {
            let var_pos = stack.var_pos(*v);
            stack.add_op("OpVarRef", self);
            self.code_add(var_pos - 8);
            stack.add_op("OpFreeRef", self);
            return Type::Void;
        }
        // @PLN118 — free a heap variable that took an unconditional pre-build free
        // (`generate_set`'s `owned_ref` path), then RESET its slot to the null
        // sentinel.  The native backend resets after EVERY var free
        // (`generation/ops/ref_ops.rs`: `OpFreeRef(...); var_x.store_nr = u16::MAX`);
        // the interpreter emitted `OpVarRef + OpFreeRef` only, leaving the variable
        // pointing at the freed slot.  In a loop the block-exit sweep frees the
        // per-iteration temp and the next iteration's pre-build `OpFreeRef` re-frees
        // it — and if the allocator reclaimed that slot for a live value meanwhile,
        // the re-free destroys it (the moros glb H4 store-slot-reuse UAF: `pos`
        // reads null on 4-of-7 hex verts).  `OpInitRefSentinel` after the free makes
        // that re-free a no-op.  Scoped to `owned_reassigned` (not every var free):
        // a retbuf/Database-reused local like `off` keeps its DbRef for in-place
        // `OpDatabase` reuse, and nulling it destabilises the `protect_store_frees`
        // machinery (leaks the returned value).
        if stack.data.def(op).name() == "OpFreeRef"
            && let Some(Value::Var(v)) = parameters.first()
            && stack.owned_reassigned.contains(v)
        {
            let var_pos = stack.var_pos(*v);
            self.generate(&parameters[0], stack, false);
            stack.add_op("OpFreeRef", self);
            stack.add_op("OpInitRefSentinel", self);
            self.code_add(var_pos);
            return Type::Void;
        }
        // try destination-passing optimisation for text-producing natives.
        if self.try_text_dest_pass(stack, op, parameters) {
            return Type::Void;
        }
        // @PLN24 arc D — a `#c` text binding reaching the ORDINARY call emission
        // is a routing gap, and it has to stop here.
        //
        // For a `_dest` native the dest route is an optimisation: miss it and the
        // plain sibling still answers correctly. A `#c` binding has no plain
        // sibling — the definition is body-less, so the ordinary emission calls
        // into a function that does not exist and the interpreter walks off into
        // whatever follows. That is a SIGSEGV in a position nobody tested, which
        // is exactly how this crossing would come to differ between the backends
        // (`--native` has no such hole: rustc calls a typed extern whatever the
        // position). Failing loudly HERE turns a future uncovered position into
        // one message naming the shape, instead of a corrupted stack.
        assert!(
            !is_c_text_call(stack.data.def(op)),
            "loft: `{}` is a `#c` binding returning text, called in a position the \
             compiler cannot route a destination into. This is a compiler gap, not a \
             mistake in the program — please report it with the calling expression \
             (@PLN24 arc D)",
            stack.data.def(op).name().strip_prefix("n_").unwrap_or("?"),
        );
        for (a_nr, a) in stack.data.def(op).attributes().iter().enumerate() {
            if a.mutable {
                let stack_before = stack.position;
                // When a RefVar argument is passed directly to a matching RefVar parameter
                // (e.g. a dispatcher forwarding its text-buffer arg to a variant), emit only
                // OpVarRef to push the raw DbRef — do NOT emit the trailing OpGetStackText /
                // OpGetStackRef that generate_var would normally add.
                if matches!(a.typedef, Type::RefVar(_))
                    && let Value::Var(v) = &parameters[a_nr]
                    && matches!(stack.function.tp(*v), Type::RefVar(_))
                {
                    let var_pos = stack.var_pos(*v);
                    stack.add_op("OpVarRef", self);
                    self.code_add(var_pos);
                    tps.push(a.typedef.clone());
                } else if stack.data.def(op).name() == "OpSetInt4"
                    && let Value::TupleGet(tvar, tidx) = parameters[a_nr].unspan()
                    && let Type::Tuple(elems) = stack.function.tp(*tvar).clone()
                    && (*tidx as usize) < elems.len()
                    && matches!(elems[*tidx as usize].base(), Type::Function(_, _, _))
                {
                    // #493 — a fn-ref TUPLE ELEMENT stored into a 4-byte d_nr
                    // struct field (OpSetInt4): project only the 8-byte d_nr, not
                    // the full 20-byte fn-ref.  The default `generate()` would emit
                    // OpVarFnRef (20B) which overruns the 8-byte value slot and
                    // imbalances the eval stack — a garbage d_nr plus a corrupted
                    // FOLLOWING store (silent under a normal build; a generate_call
                    // width assert under debug-assertions).  Mirrors native's
                    // fn-ref-element projection (`generation/calls.rs`) and the
                    // TupleGet Integer arm; the d_nr is the first 8 bytes of the
                    // element slot, so `OpVarInt` at its offset reads exactly it.
                    let offsets = crate::data::element_stack_offsets(&elems);
                    let elem_abs = stack.function.stack(*tvar) + offsets[*tidx as usize] as u16;
                    let var_pos = stack.position - elem_abs;
                    stack.add_op("OpVarInt", self);
                    self.code_add(var_pos);
                    tps.push(crate::data::I64.clone());
                } else {
                    // @PLN114 — this argument's value is pushed straight into the
                    // callee's frame block, so a tuple built here must land packed.
                    #[cfg(debug_assertions)]
                    let outer_in_call_arg = std::mem::replace(&mut self.in_call_arg, true);
                    tps.push(self.generate(&parameters[a_nr], stack, false));
                    #[cfg(debug_assertions)]
                    {
                        self.in_call_arg = outer_in_call_arg;
                    }
                    // When a Value::Null is passed as a typed argument, generate()
                    // pushes 0 bytes.  Emit the correct null sentinel for the
                    // expected type so the stack size matches.
                    if parameters[a_nr] == Value::Null && stack.position == stack_before {
                        self.emit_typed_null(stack, &a.typedef);
                    }
                    // A fn-ref arg is [d_nr i64][closure DbRef] — 8 + 12 B, which
                    // the eval stack steps to 8 + 16 = 24.  A plain fn-ref constant
                    // pushes only the d_nr (8), so pad the closure half.  Both halves
                    // are target-independent, unlike the `text` slot the fn-ref OPS
                    // are declared with (see `Stack::fnref_signature_gap`).
                    if matches!(a.typedef, Type::Function(_, _, _))
                        && stack.position - stack_before < 16
                    {
                        self.emit_push_sentinel(stack);
                    }
                }
                #[cfg(debug_assertions)]
                {
                    // @PLAN53 S4: the arg occupies a stepped span on the stack.
                    let expected = stack.step(size(&a.typedef, &Context::Argument));
                    let actual = stack.position - stack_before;
                    debug_assert_eq!(
                        actual,
                        expected,
                        "generate_call [{caller}]: mutable arg {a_nr} ({arg_name}: {arg_tp:?}) \
                         expected {expected}B on stack but generate({val:?}) pushed {actual}B — \
                         Value::Null in a typed slot? Missing convert() call in the parser?",
                        caller = stack.data.def(stack.def_nr).name(),
                        arg_name = a.name,
                        arg_tp = a.typedef,
                        val = &parameters[a_nr],
                    );
                }
            }
        }
        // push extra Call args beyond the declared parameter count.
        // Only for n_parallel_for / n_parallel_for_light / n_parallel_discard —
        // each forwards extra context args + n_extra count.
        if stack.data.def(op).name() == "n_parallel_for"
            || stack.data.def(op).name() == "n_parallel_for_light"
            || stack.data.def(op).name() == "n_parallel_discard"
            || stack.data.def(op).name() == "n_parallel_queue"
            || stack.data.def(op).name() == "n_parallel_queue_text"
            || stack.data.def(op).name() == "n_parallel_queue_ref"
            || stack.data.def(op).name() == "n_parallel_queue_narrow"
            || stack.data.def(op).name() == "n_parallel_queue_fn"
            || stack.data.def(op).name() == "n_parallel_fold"
        {
            let n_declared = stack.data.def(op).attributes().len();
            for extra in parameters.iter().skip(n_declared) {
                self.generate(extra, stack, false);
            }
        }
        match stack.data.def(op).name() {
            "OpGetRecord" => {
                was_stack = stack.position;
                self.gather_key(stack, &parameters, 2, &mut tps);
            }
            "OpStart" => {
                // @PLAN53 cluster 2 / S4: the iterator-pushed value (+4) and the
                // consumed iterable DbRef (size_ref) each occupy a stepped span.
                was_stack = stack.position + stack.step(4) - stack.step(super::size_ref() as u16);
                self.gather_key(stack, &parameters, 2, &mut tps);
            }
            "OpNext" => {
                was_stack = stack.position;
                self.gather_key(stack, &parameters, 3, &mut tps);
            }
            "OpIterate" => {
                // @PLAN53 cluster 2 / S4 (2b+2c): OpIterate pushes the start+finish
                // pair as ONE contiguous 8-byte i64 (see io.rs `iterate`) and
                // consumes the iterable DbRef.  Account the result as a single
                // step(8) span, not 2*step(4): under LOFT_ALIGN two separate u32
                // slots would be 16 bytes (8+8) and leave a 4-byte gap the i64
                // read-back can't see.  Identity flag-OFF (step(8) == 2*step(4) == 8).
                was_stack = stack.position + stack.step(8) - stack.step(super::size_ref() as u16);
                if let Value::Int(parameter_length) = parameters[4] {
                    self.gather_key(stack, &parameters, 4, &mut tps);
                    self.gather_key(stack, &parameters, 5 + parameter_length, &mut tps);
                }
            }
            _ => (),
        }
        if !parameters.is_empty()
            && let Value::Int(n) = parameters[parameters.len() - 1]
        {
            last = n as u16;
        }
        let name = stack.data.def(op).name().to_owned();
        // CO1.6a: OpCoroutineNext/OpCoroutineExhausted take a gen DbRef from the
        // stack at runtime, but their declarations have only const params.
        // Bypass the operator path and manually handle the stack adjustment.
        if name == "OpCoroutineNext" && parameters.len() >= 2 {
            // CO1.6a: parameters[0]=gen expr, parameters[1]=Int(value_size).
            // @P327 — `value_size` packs two fields (encoding from
            // `parser/collections.rs::iterator`):
            //   * low byte  = byte size of the yielded value
            //   * high byte = native channel tag (0 = legacy per-type,
            //                                     1 = unified `next_into`)
            // The interp dispatch only cares about the byte size; the
            // native dispatch (`OpCoroutineNextEmitter`) reads both
            // bytes.  Stack accounting uses ONLY the low byte — the high
            // byte is a tag, not extra payload, so using the raw value
            // here would push 0x0110 = 272 bytes of stack space for a
            // 16-byte tuple yield and corrupt every subsequent var
            // offset in the consumer frame.
            self.generate(&parameters[0], stack, false); // push DbRef (+12)
            let value_size = if let Value::Int(n) = &parameters[1] {
                *n as u16
            } else {
                4 // fallback: integer
            };
            let byte_size = value_size & 0xFF;
            self.remember_stack(stack.position);
            super::emit_op(stack.data.def(op).op_code(), self);
            self.code_add(value_size);
            // Stack: -12 (DbRef consumed) + byte_size (yielded value pushed).
            // @PLAN53 S4: both occupy stepped spans.
            stack.position -= stack.step(super::size_ref() as u16);
            stack.position += stack.step(byte_size);
            // Return type is the yield type — inferred from value_size for now.
            return match byte_size {
                1 => Type::Boolean,
                8 => crate::data::I64.clone(),
                _ => I32.clone(),
            };
        }
        if name == "OpCoroutineExhausted" && !parameters.is_empty() {
            // parameters[0] — the gen expression — is ALREADY on the stack: `gen` is a real
            // parameter (`fn OpCoroutineExhausted(gen: reference) -> boolean`), so the
            // mutable-argument loop above generated it.  Generating it here as well pushed
            // the DbRef TWICE and only one was consumed, stranding 16 bytes on the eval stack
            // at every call (loft#1309).
            //
            // The header above says these ops "have only const params", and that is why the
            // second push looked necessary.  It is true of `OpCoroutineNext`, whose
            // `value_size: const u16` the loop skips — which is exactly why `next` was never
            // affected and `exhausted` always was.
            //
            // Silent in release: the surplus is only NOTICED where a caller measures an
            // argument's width, so `assert(exhausted(g), …)` reported it as a 24B-vs-8B
            // mismatch under debug assertions while `d = exhausted(g)` leaked the same 16
            // bytes with nothing to object.
            self.remember_stack(stack.position);
            super::emit_op(stack.data.def(op).op_code(), self);
            // Stack: -12 (DbRef consumed) + 1 (bool pushed).
            // @PLAN53 S4: both occupy stepped spans.
            stack.position -= stack.step(super::size_ref() as u16);
            stack.position += stack.step(1);
            return Type::Boolean;
        }
        // resolve library index — prefer #native symbol, fall back to def name.
        // only use the library_names fallback for functions without a user-
        // defined body.  A user function whose `n_<name>` collides with a native
        // stdlib name (e.g. user `to_json` vs native `n_to_json` for JsonValue)
        // must go through OpCall, not OpStaticCall.
        let native_sym = stack.data.def(op).native().to_owned();
        let has_user_body = *stack.data.def(op).code() != Value::Null;
        let lib_lookup: &str = if !native_sym.is_empty() {
            &native_sym
        } else if has_user_body {
            // User function with a body — never route through native dispatch,
            // even if the name happens to match a library_names entry.
            ""
        } else {
            &name
        };
        let lib_nr = if lib_lookup.is_empty() {
            None
        } else {
            self.library_names.get(lib_lookup).copied()
        };
        // Plan-04 Phase B.2.c: compound push-and-init ops emitted by the
        // parser (via `self.cl("OpName", ...)` in parser/mod.rs etc.) are
        // decomposed here into the positional `OpReserveFrame(n) +
        // OpInit*(n)` form.  This lets the parser keep emitting the
        // familiar compound names while the runtime path uses the clean
        // primitives.
        if name == "OpNullRefSentinel" {
            self.emit_push_sentinel(stack);
            return stack.data.def(op).returned().clone();
        }
        if name == "OpConvRefFromNull" {
            self.emit_push_null_ref(stack);
            return stack.data.def(op).returned().clone();
        }
        if name == "OpCreateStack" && !parameters.is_empty() {
            if let Value::Var(wv) = &parameters[0] {
                // Dep is the named variable at wv.stack_pos.
                let dep_offset = stack.var_pos(*wv);
                self.emit_push_create_stack(stack, dep_offset);
            } else {
                // OpCreateStack with a non-Var expression (e.g.
                // OpGetVector result).  Generate the expression to push
                // a 12-byte DbRef, then point OpInitCreateStack at it.
                self.generate(&parameters[0], stack, false);
                self.emit_push_create_stack(stack, size_of::<crate::keys::DbRef>() as u16);
            }
            return stack.data.def(op).returned().clone();
        }
        if stack.data.def(op).is_operator() {
            // B7: OpAppendCharacter on a RefVar(Text) target must use
            // OpAppendStackCharacter — same pattern as OpAppendText →
            // OpAppendStackText in set_var.  Without this, the append
            // writes to the raw RefVar slot instead of dereferencing it.
            //
            // @P283 — the same dispatch is needed for `OpAppendText`
            // and `OpClearText` when they enter via the operator-call
            // path (as opposed to `set_var`'s direct emit).  The parser's
            // `assign_text` work-text path constructs `Value::Call(OpAppendText, …)`
            // / `Value::Call(OpClearText, …)` IR; if the work-text was
            // promoted to a `RefVar(Text)` hidden arg by `text_return`,
            // these ops must dispatch to the stack variants so the
            // refvar slot is dereferenced.  Without this, the interpreter
            // treats the refvar's DbRef bytes as a `String` and SIGSEGVs;
            // the native backend's `OpAppendStackText` arm (dispatch.rs:582)
            // already emits the required `*` deref.
            let stack_variant = if let Some(Value::Var(v)) = parameters.first()
                && matches!(stack.function.tp(*v), Type::RefVar(inner) if matches!(**inner, Type::Text(_)))
            {
                crate::generation::ops::refvar_text_stack_variant(name.as_str())
            } else {
                None
            };
            let actual_op = stack_variant.map_or(op, |n| stack.data.def_nr(n));
            let before_stack = stack.position;
            self.remember_stack(stack.position);
            let code = self.code_pos;
            super::emit_op(stack.data.def(actual_op).op_code(), self);
            stack.operator(actual_op);
            if was_stack != u16::MAX {
                stack.position = was_stack;
            }
            for (a_nr, a) in stack.data.def(op).attributes().iter().enumerate() {
                if a.mutable {
                    continue;
                }
                // OpIterate: from_key is at parameters[4], but till_key is at
                // parameters[5 + from_count] because the from-key values occupy
                // parameters[5..5+from_count], pushing till_key_count further out.
                let param_idx = if name == "OpIterate"
                    && a_nr == 5
                    && let Value::Int(from_count) = parameters[4]
                {
                    (5 + from_count) as usize
                } else {
                    a_nr
                };
                self.add_const(&a.typedef, &parameters[param_idx], stack, before_stack);
            }
            self.op_type(op, &tps, last, code, stack)
        } else if let Some(lib_idx) = lib_nr {
            stack.add_op("OpStaticCall", self);
            self.code_add(lib_idx);
            // @PLAN53 cluster 2 / S4: args were pushed at stepped spans.
            for a in stack.data.def(op).attributes() {
                stack.position -= stack.step(size(&a.typedef, &Context::Argument));
            }
            // also subtract the extra args pushed beyond declared params.
            if stack.data.def(op).name() == "n_parallel_for"
                || stack.data.def(op).name() == "n_parallel_for_light"
                || stack.data.def(op).name() == "n_parallel_discard"
                || stack.data.def(op).name() == "n_parallel_queue"
                || stack.data.def(op).name() == "n_parallel_queue_text"
                || stack.data.def(op).name() == "n_parallel_queue_ref"
                || stack.data.def(op).name() == "n_parallel_queue_narrow"
                || stack.data.def(op).name() == "n_parallel_queue_fn"
                || stack.data.def(op).name() == "n_parallel_fold"
            {
                let n_declared = stack.data.def(op).attributes().len();
                for extra in parameters.iter().skip(n_declared) {
                    // Extra args are always integer (8 bytes post-2c).
                    let _ = extra;
                    stack.position -= stack.step(8);
                }
            }
            // add the result to the stack
            stack.position += stack.step(size(stack.data.def(op).returned(), &Context::Argument));
            stack.data.def(op).returned().clone()
        } else {
            // CO1.3c: emit OpCoroutineCreate for generator function calls.
            let is_generator = matches!(stack.data.def(op).returned(), Type::Iterator(_, _));
            if is_generator {
                stack.add_op("OpCoroutineCreate", self);
            } else {
                stack.add_op("OpCall", self);
            }
            self.code_add(i64::from(op)); // d_nr: i64 (stdlib `const i32` widens post-2c)
            // @PLAN53 cluster 2 / S4: args_size is the runtime frame-base
            // delta (args_base = stack_pos - args_size); it MUST equal the
            // per-arg stepped pushes, so sum Σ step(size) — never step(Σ).
            let args_size: u16 = stack
                .data
                .def(op)
                .attributes
                .iter()
                .map(|a| stack.step(size(&a.typedef, &Context::Argument)))
                .sum();
            self.code_add(args_size);
            // loft#1032 — remember the address of the i64 target operand ITSELF, so
            // a forward call's back-patch (`byte_code_for`) cannot drift from what
            // was emitted here.  It used to remember the address of the OPCODE and
            // re-derive the operand as `+ opcode(1) + d_nr(8) + args_size(2)`, but an
            // opcode is one byte only below 255 and TWO at or above it (`emit_op`).
            // `OpCoroutineCreate` sits above 255, so every call to a generator whose
            // body had not been generated yet was patched one byte early — over the
            // high byte of `args_size`.  The interpreter then read a multi-kilobyte
            // frame size and `coroutine_create`'s `stack_pos - args_size` underflowed.
            self.calls.entry(op).or_default().push(self.code_pos);
            self.code_add(i64::from(stack.data.def(op).code_position));
            // remove the arguments that are already on the stack
            for a in stack.data.def(op).attributes() {
                stack.position -= stack.step(size(&a.typedef, &Context::Argument));
            }
            // add the result to the stack
            stack.position += stack.step(size(stack.data.def(op).returned(), &Context::Argument));
            stack.data.def(op).returned().clone()
        }
    }

    pub(super) fn gather_key(
        &mut self,
        stack: &mut Stack,
        parameters: &&[Value],
        from: i32,
        tps: &mut Vec<Type>,
    ) {
        let no_keys = if let Value::Int(v) = &parameters[from as usize] {
            *v
        } else {
            0
        };
        for k in 0..no_keys {
            tps.push(self.generate(&parameters[(no_keys + from - k) as usize], stack, false));
        }
    }

    pub(super) fn op_type(
        &mut self,
        op: u32,
        tps: &[Type],
        last: u16,
        code: u32,
        stack: &mut Stack,
    ) -> Type {
        match stack.data.def(op).name() {
            "OpDatabase" | "OpAppend" | "OpConvEnumFromNull" | "OpCastEnumFromInt"
            | "OpCastEnumFromText" | "OpGetField" => {
                self.types.insert(code, last);
            }
            // Plan-07 phase 4 step 4.6 — `OpGetVectorNullable` /
            // `OpVectorRefNullable` are runtime-equivalent to their
            // raising peers (return null DbRef on OOB instead of
            // raising), so they need identical type-inference: the
            // returned DbRef points at an element of the source
            // vector's inner type.
            "OpGetVector"
            | "OpGetVectorNullable"
            | "OpVectorRef"
            | "OpVectorRefNullable"
            | "OpInsertVector"
            | "OpAppendVector" => {
                if let Type::Vector(v, _) = &tps[0] {
                    self.types
                        .insert(code, stack.data.def(stack.data.type_def_nr(v)).known_type);
                    return *v.clone();
                }
            }
            "OpGetHash" => {
                if let Type::Hash(v, _, link) = &tps[0] {
                    return Type::Reference(*v, link.clone());
                }
            }
            "OpGetIndex" => {
                if let Type::Index(v, _, link) = &tps[0] {
                    return Type::Reference(*v, link.clone());
                }
            }
            "OpGetRadix" => {
                if let Type::Radix(v, _, link) | Type::Trie(v, _, link) = &tps[0] {
                    return Type::Reference(*v, link.clone());
                }
            }
            "OpVarEnum" => {
                self.insert_types(tps[0].clone(), code, stack);
            }
            _ => (),
        }
        stack.data.def(op).returned().clone()
    }

    pub(super) fn insert_types(&mut self, tp: Type, code: u32, stack: &Stack) -> Type {
        match tp {
            Type::Enum(t, _, _) => {
                self.types.insert(code, stack.data.def(t).known_type());
            }
            Type::Reference(t, _) if t < u32::from(u16::MAX) => {
                self.types.insert(code, stack.data.def(t).known_type());
            }
            _ => (),
        }
        tp
    }

    /// Emit bytecode for a call through a fn-ref variable.
    ///
    /// Use when the callee is `Value::CallRef(v_nr, args)` — the fn-ref is stored as an
    /// i32 `d_nr` in a local variable; arguments are already type-checked by the parser.
    pub(super) fn generate_call_ref(
        &mut self,
        stack: &mut Stack,
        v_nr: u16,
        args: IrNodeList,
    ) -> Type {
        let Type::Function(param_types, ret_type, _) = stack.function.tp(v_nr).clone() else {
            panic!("generate_call_ref: variable is not Type::Function");
        };
        let ret_type = *ret_type;

        // Generate all args: visible params, then work-buffer blocks (12B DbRefs each),
        // then closure arg (12B DbRef).  Blocks produce the correct type/size automatically.
        //
        // A `fn(…)` PARAMETER takes the whole 20-byte pair, and a bare fn-ref name lowers to
        // the lone d_nr — eight bytes.  `declared_size` below counts the pair either way, so
        // the short push left the frame twelve bytes out and the call ran off it: on
        // `--interpret` the program produced NO OUTPUT AT ALL and exited 0 (not even the
        // print before the call), and `--native` passed an `i64` where the callee's parameter
        // is a `(u32, DbRef)` (loft#1285).  A fn-ref VARIABLE already holds the pair, which
        // is why `lf(v)` worked and `lf(dbl)` did not.
        //
        // `gen_fn_ref_value_node` is the same top-up the tuple-element write and the
        // first-Set path use, so the three spellings of "put a fn-ref in a 20-byte slot"
        // cannot drift (loft#1069's family).
        for (i, arg) in args.iter().enumerate() {
            if i < param_types.len() && matches!(param_types[i].base(), Type::Function(_, _, _)) {
                self.gen_fn_ref_value_node(arg, stack);
            } else {
                self.generate_node(arg, stack, false);
            }
        }

        // fn-ref variable is below all pushed arguments.
        let fn_var_dist = stack.var_pos(v_nr);
        // declared: visible param sizes; extra: work-buf + closure (all 12-byte DbRefs).
        // @PLAN53 cluster 2 / S4: each arg occupies a stepped span (Σ step).
        let declared_size: u16 = param_types
            .iter()
            .map(|t| stack.step(size(t, &Context::Argument)))
            .sum();
        let extra = if args.len() > param_types.len() {
            (args.len() - param_types.len()) as u16 * stack.step(super::size_ref() as u16)
        } else {
            0
        };
        let total_arg_size = declared_size + extra;
        stack.add_op("OpCallRef", self);
        self.code_add(fn_var_dist);
        self.code_add(total_arg_size);
        stack.position -= total_arg_size;
        stack.position += stack.step(size(&ret_type, &Context::Argument));
        ret_type
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn generate_var(&mut self, stack: &mut Stack, variable: u16) -> Type {
        if stack.function.stack(variable) > stack.position
            && std::env::var("LOFT_VAR_PANIC_IR").is_ok()
        {
            eprintln!(
                "[generate_var] IR of {}:\n{:?}",
                stack.data.def(stack.def_nr).name(),
                stack.data.def(stack.def_nr).code()
            );
        }
        assert!(
            stack.function.stack(variable) <= stack.position,
            "Incorrect var {}[{}] versus {} on {}",
            stack.function.name(variable),
            stack.function.stack(variable),
            stack.position,
            stack.data.def(stack.def_nr).name()
        );
        let var_pos = stack.var_pos(variable);
        let argument = stack.function.is_argument(variable);
        let code = self.code_pos;
        self.vars.insert(code, variable);
        // @PLN25 slice (b): an `Optional(τ)` var loads exactly like `τ` (same sentinel
        // storage) — peel the marker so each op-emission arm sees the base type.
        match stack.function.tp(variable).base() {
            Type::Integer(_) => stack.add_op("OpVarInt", self),
            Type::Function(_, _, _) => {
                stack.add_op("OpVarFnRef", self);
                // Post-2c fn-ref slot is 20 bytes, but OpVarFnRef's stdlib
                // signature returns `text` (16 B Str).  Add the 4-byte
                // discrepancy to the compile-time stack tracker.
                stack.position += stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
            }
            Type::Character => stack.add_op("OpVarCharacter", self),
            Type::RefVar(_) => stack.add_op("OpVarRef", self),
            Type::Enum(_, false, _) => stack.add_op("OpVarEnum", self),
            Type::Boolean => stack.add_op("OpVarBool", self),
            Type::Single => stack.add_op("OpVarSingle", self),
            Type::Float => stack.add_op("OpVarFloat", self),
            Type::Text(_) => {
                stack.add_op(if argument { "OpArgText" } else { "OpVarText" }, self);
            }
            Type::Vector(tp, _) => {
                let typedef: &Type = tp;
                let known = if matches!(typedef, Type::Unknown(_)) {
                    u16::MAX
                } else if matches!(typedef, Type::Text(_)) {
                    self.database.vector(5)
                } else {
                    let name = typedef.name(stack.data);
                    let mut tp_nr = self.database.name(&name);
                    if tp_nr == u16::MAX {
                        tp_nr = self.database.db_type(typedef, stack.data);
                    }
                    self.database.vector(tp_nr)
                };
                if known != u16::MAX {
                    self.types.insert(self.code_pos, known);
                }
                stack.add_op("OpVarVector", self);
            }
            // P188 — keyed collections as local vars share the
            // 4-byte u32 in-stack representation with vectors (a
            // record pointer).  Emit OpVarVector so the load reads
            // the same 4 bytes; field access / `+=` codegen handles
            // the per-collection-kind dispatch via record_finish.
            Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _) => {
                let tp = stack.function.tp(variable);
                let name = tp.name(stack.data);
                let mut tp_nr = self.database.name(&name);
                if tp_nr == u16::MAX {
                    tp_nr = self.database.db_type(tp, stack.data);
                }
                let known = self.database.vector(tp_nr);
                if known != u16::MAX {
                    self.types.insert(self.code_pos, known);
                }
                stack.add_op("OpVarVector", self);
            }
            Type::Reference(c, _) | Type::Enum(c, true, _) => {
                self.types
                    .insert(self.code_pos, stack.data.def(*c).known_type());
                stack.add_op("OpVarRef", self);
            }
            // CO1.3c: iterator variables are DbRef-sized (coroutine frame reference).
            Type::Iterator(_, _) => {
                stack.add_op("OpVarRef", self);
            }
            Type::Tuple(elems) => {
                // T1.4: read the whole tuple by reading each element.  Delegated to
                // the same helper `TupleGet` uses, which is where the element set
                // and the nested-tuple recursion live.  This arm used to hand-roll
                // its own copy of that loop, which is how it came to be narrower
                // than its sibling: no `Type::Tuple` recursion, so reading a whole
                // NESTED tuple variable was an ICE (`r = ((1,4),5); r`), and no
                // collection arms either.  That ICE is what bounded loft#816's
                // return hoist and left loft#817 — a nested tuple return with a
                // store local answering the zero initialiser on native.
                let elems = elems.clone();
                let tuple_base = stack.function.stack(variable);
                self.emit_tuple_var_push_recursive(stack, &elems, tuple_base);
                return self.insert_types(stack.function.tp(variable).clone(), code, stack);
            }
            _ => panic!(
                "Unknown var '{}' type {} at {}",
                stack.function.name(variable),
                stack.function.tp(variable).name(stack.data),
                stack.data.def(stack.def_nr).position()
            ),
        }
        self.code_add(var_pos);
        if let Type::RefVar(tp) = stack.function.tp(variable) {
            // loft#1372 — through `base()`: `Optional(τ)` shares `τ`'s storage exactly, so a
            // `&τ?` link reads at the same op as its `&τ` twin and the null travels in the
            // slot's own sentinel.  Asked bare, `Optional` matched no arm and the read fell
            // through to the panic below, which is why @FR-B-Ref-Intro's `&τ` for every τ
            // had to be declined (D-bind-17).
            let tp = tp.base();
            let txt = matches!(tp, Type::Text(_));
            match tp {
                Type::Integer(_) => stack.add_op("OpGetInt", self),
                Type::Character => stack.add_op("OpGetCharacter", self),
                Type::Single => stack.add_op("OpGetSingle", self),
                Type::Float => stack.add_op("OpGetFloat", self),
                // `&boolean` reads through the ref as a BOOLEAN, not as the raw
                // byte an enum uses: storage is tri-state (0/1/255, null-capable)
                // while the expression form is two-state, and `OpGetBoolean` is
                // the op that already makes that conversion.  Its absence panicked
                // codegen — the natural shape for a two-state out-parameter, next
                // to an `&integer` that worked (loft#655).
                Type::Boolean => stack.add_op("OpGetBoolean", self),
                Type::Enum(_, false, _) => stack.add_op("OpGetByte", self),
                Type::Text(_) => stack.add_op("OpGetStackText", self),
                // @P305 — a keyed collection passed by `&` is DbRef-backed just like a
                // vector or a reference; reaching one (e.g. as the `coll` argument of
                // `OpSetKeyed` for `h[k] = v` on a `&hash` parameter) needs the same
                // stack-ref deref.  @FR-Col-Store defines the store-backed set as
                // `Vector` plus the five keyed kinds, so `vectors::is_collection` is the
                // one home for that half and this arm derives from it rather than
                // listing the kinds again — the list spelled out is exactly how `trie`
                // and `spatial` came to be missing while their three siblings worked
                // (loft#1445, the fourth instance of the class after loft#1291,
                // loft#1292 and loft#1433).
                //
                // The `_ =>` below is a `panic!`, so a kind missing here is an ICE and
                // not a lost optimisation: the cost of an omission is the whole program.
                Type::Reference(_, _) | Type::Enum(_, true, _) => {
                    stack.add_op("OpGetStackRef", self);
                }
                other if crate::parser::vectors::is_collection(other) => {
                    stack.add_op("OpGetStackRef", self);
                }
                _ => panic!("Unknown referenced variable type: {tp}"),
            }
            if !txt {
                self.code_add(0u16);
            }
        }
        self.insert_types(stack.function.tp(variable).clone(), code, stack)
    }

    pub(super) fn generate_block(&mut self, stack: &mut Stack, block: IrBlock, top: bool) -> Type {
        let ops = block.operators();
        if ops.is_empty() {
            return Type::Void;
        }
        let to = stack.position;

        // Plan-04 Phase B.3 atomic bundle: per-block
        // `OpReserveFrame(block.var_size)` deleted.  Every local is
        // covered by the single function-entry `OpReserveFrame(frame_hwm)`
        // emitted in `def_code`.  The per-block `OpFreeStack` below
        // stays — it still discards eval-stack residue above the
        // block's result value.

        let mut tp = Type::Void;
        let mut return_expr = 0;
        let mut has_return = false;
        let free_ops = stack.data.op_sets();
        for v in ops.iter() {
            let s_pos = self.stack_pos;
            if v.kind() == ValueType::Return {
                has_return = true;
                if return_expr == 0 {
                    return_expr = s_pos;
                    let before = stack.position;
                    self.generate_node(v.return_inner(), stack, false);
                    // #263: pad a bare-d_nr fn-ref return to the 20-byte ABI.
                    self.pad_fn_ref_return(stack, before);
                }
                self.add_return(stack, return_expr);
                return_expr = 0;
                tp = Type::Void;
            } else {
                // Preserve return_expr across cleanup ops — a free in ANY of its spellings,
                // read off the one free-op home — that don't produce a return value.  These
                // are inserted by scope analysis between the tail expression and Return(Null).
                let is_cleanup =
                    v.kind() == ValueType::Call && free_ops.frees.contains(&v.call_to());
                if !is_cleanup {
                    has_return = false;
                    return_expr = 0;
                }
                tp = self.generate_node(v, stack, false);
            }
            if self.stack_pos > s_pos && v.kind() != ValueType::Set {
                // Normal expressions do not claim stack space (because of Value::Drop).
                // So, if there is data left, it should be a return expression.
                return_expr = s_pos;
            }
        }
        if top {
            if !has_return {
                self.add_return(
                    stack,
                    if return_expr > 0 {
                        return_expr
                    } else {
                        self.code_pos
                    },
                );
            }
        } else {
            let result = block.result();
            // @PLAN53 S4: the block result occupies a stepped span.
            let size = stack.step(size(&result, &Context::Argument));
            let after = to + size;
            // Plan-04 Phase B.3: under the per-block `OpReserveFrame`
            // that this atomic bundle deletes, a `drop <var>` tail
            // coincidentally aliased the block's result slot with the
            // block-local's storage.  With function-entry reserve the
            // local lives elsewhere in the frame and the push+pop of
            // `drop` leaves no explicit result on the eval stack.
            // Re-read the inner `Var` to produce the missing push
            // (safe: `Var` reads are idempotent).
            if stack.position < after && !ops.is_empty() {
                let last = ops.get(ops.len() - 1);
                if last.kind() == ValueType::Drop {
                    let inner = last.drop_inner();
                    if inner.kind() == ValueType::Var {
                        self.generate_node(inner, stack, false);
                    }
                }
            }
            if stack.position > after {
                stack.add_op("OpFreeStack", self);
                self.code_add(size as u8);
                self.code_add(stack.position - to);
            } else if matches!(&result, Type::Function(_, _, _)) && stack.position < after {
                // a fn-ref block result is 16 bytes ([d_nr 4B][closure DbRef 12B]).
                // If the block only pushed 4 bytes (d_nr via OpConstInt), pad to 16 with
                // a null-ref sentinel so both branches of an if-else reach the join point
                // with the same stack delta.
                self.emit_push_sentinel(stack);
            } else if stack.position == to && to < after {
                // @PLN85 P3: the block is typed to yield a value but its operators
                // produced NO result on the eval stack — e.g. a `{ [] }` match arm
                // that reduced to a bare `Line` marker (the multi-line spelling of an
                // empty-vector arm; the single-line spelling becomes a Null else that
                // gen_if already pads).  Without a pushed result the runtime stack is
                // short by one result slot, so the enclosing join / `OpReturn` discard
                // over-counts and `fn_return` misreads the saved return address →
                // interpreter underflow.  Push a typed null of the result, mirroring
                // gen_if's null-else arm (`emit_typed_null`).
                self.emit_typed_null(stack, &result);
            }
            stack.position = after;
        }
        tp
    }

    pub(super) fn add_return(&mut self, stack: &mut Stack, code: u32) {
        let return_type = stack.data.def(stack.def_nr).returned();
        // CO1.3c: generator functions use OpCoroutineReturn.
        if matches!(return_type, Type::Iterator(_, _)) {
            let yield_size = if let Type::Iterator(inner, _) = return_type {
                size(inner, &Context::Argument)
            } else {
                0
            };
            stack.add_op("OpCoroutineReturn", self);
            self.code_add(yield_size);
        } else {
            stack.add_op("OpReturn", self);
            self.code_add(self.arguments);
            self.code_add(size(return_type, &Context::Argument) as u8);
            self.code_add(stack.position);
            if return_type != &Type::Void {
                self.types.insert(code, self.known_type(return_type, stack));
            }
        }
    }

    /// The schema type id an `OpReturn` records for a returned value.
    ///
    /// Through `base`, because a nullable heap return holds the SAME record as its non-null
    /// twin (@FR-L-Null: layout(τ) = layout(τ?)).  Asked bare, an `S?` fell to the name
    /// lookup, which has no `"S?"` to find and answers `u16::MAX` — so the one consumer, the
    /// execution-trace renderer, had no type to decode the returned value with on exactly the
    /// shapes nullable-return bugs live on.
    pub(super) fn known_type(&self, tp: &Type, stack: &Stack) -> u16 {
        if let Some(c) = tp.base().heap_def_nr() {
            stack.data.def(c).known_type()
        } else {
            self.database.name(&tp.name(stack.data))
        }
    }

    pub(super) fn add_const(&mut self, tp: &Type, p: &Value, stack: &Stack, before_stack: u16) {
        match tp {
            Type::Integer(IntegerSpec {
                min: 0, max: 255, ..
            }) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as u8);
                }
            }
            Type::Integer(IntegerSpec {
                min: -128,
                max: 127,
                ..
            }) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as i8);
                }
            }
            Type::Integer(IntegerSpec {
                min: 0, max: 65535, ..
            }) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as u16);
                } else if let Value::Var(v) = p {
                    let r = stack.function.stack(*v);
                    self.code_add(before_stack - r);
                }
            }
            Type::Integer(IntegerSpec {
                min: -32768,
                max: 32767,
                ..
            }) => {
                if let Value::Int(nr) = p {
                    self.code_add(*nr as i16);
                }
            }
            Type::Integer(_) => {
                if let Value::Int(nr) = p {
                    self.code_add(i64::from(*nr));
                } else if let Value::Long(val) = p {
                    self.code_add(*val);
                }
            }
            Type::Enum(_, _, _) => {
                if let Value::Enum(nr, _) = p {
                    self.code_add(*nr);
                }
            }
            Type::Boolean => {
                if let Value::Boolean(v) = p {
                    self.code_add(u8::from(*v));
                }
            }
            Type::Text(_) => {
                if let Value::Text(s) = p {
                    self.code_add_str(s);
                }
            }
            Type::Float => {
                if let Value::Float(val) = p {
                    self.code_add(*val);
                }
            }
            Type::Single => {
                if let Value::Single(val) = p {
                    self.code_add(*val);
                }
            }
            Type::Keys => {
                if let Value::Keys(keys) = p {
                    self.code_add(keys.len() as u8);
                    for k in keys {
                        self.code_add(k.type_nr);
                        self.code_add(k.position);
                        // loft#812 — the shift travels with the descriptor, so a ranged
                        // scan compares against the same decode the record side uses.
                        self.code_add(k.start);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn set_var(&mut self, stack: &mut Stack, var: u16, value: &Value) {
        if let Type::RefVar(tp) = stack.function.tp(var).clone() {
            // loft#1372 — the write half of the same peel: `Optional(τ)` stores as `τ`, so
            // the write op is the slot's, and whether the slot may hold null is
            // @FR-N-Store's question rather than the op's.
            let tp = Box::new(tp.base().clone());
            if matches!(*tp, Type::Text(_)) {
                if value == &Value::Text(String::new()) {
                    // @P346: assigning "" to a RefVar(Text) is NOT a no-op — it
                    // must CLEAR the dereferenced buffer.  When this RefVar is a
                    // reused work-buffer (e.g. a format-string interpolation
                    // promoted to the function's `&text` return slot, reset each
                    // loop iteration), the buffer holds stale content from the
                    // previous iteration; skipping the clear let the following
                    // OpFormatStack* APPEND to it → text accumulated across
                    // iterations (`[2.5][2.58][2.581]`).  Emit OpClearStackText
                    // (deref) so the buffer is emptied, matching the native path.
                    let var_pos = stack.var_pos(var);
                    stack.add_op("OpClearStackText", self);
                    self.code_add(var_pos);
                    return;
                }
                // @PLN10 — destination-passing into a `RefVar(Text)` work buffer.
                // When the value is a text-dest native (`is_text_dest_native`), the
                // native writes its result DIRECTLY into the deref'd buffer (clear
                // it first, then the `_dest` native `push_str`s into the empty
                // record), avoiding `stores.scratch`.  This is the return-dep-local
                // case: `text_return` promotes `k = json.kind()` 's `k` to this
                // `&text` return slot, so the `Type::Text` dest-pass below never
                // fires and the call fell back to the scratch producer (`n_kind`).
                if let Value::Call(op, args) = value.unspan() {
                    let name = stack.data.def(*op).name().to_owned();
                    if is_text_dest_native(&name)
                        && let Some(&lib_nr) = self.library_names.get(&(name + "_dest"))
                    {
                        let var_pos = stack.var_pos(var);
                        stack.add_op("OpClearStackText", self);
                        self.code_add(var_pos);
                        self.gen_text_dest_call(stack, var, *op, args, lib_nr);
                        return;
                    }
                    // @PLN10 N2b — a cdylib FFI text call assigned to a return-dep
                    // `RefVar(Text)` buffer (e.g. `p = req.path(); … return p`).
                    // `gen_cdylib_text_dest_call` pushes the RefVar's raw DbRef as
                    // the dest, so clear it first (the bridge `push_str`s in).
                    if is_cdylib_text_call(stack.data.def(*op)) {
                        let native_sym = stack.data.def(*op).native().to_owned();
                        if let Some(&cdylib_lib) = self.library_names.get(&native_sym) {
                            let var_pos = stack.var_pos(var);
                            stack.add_op("OpClearStackText", self);
                            self.code_add(var_pos);
                            self.gen_cdylib_text_dest_call(stack, var, *op, args, cdylib_lib);
                            return;
                        }
                    }
                    // @PLN24 arc D — a `#c` binding returning text. Registered
                    // under its OWN name (`c_call::register`), so the library
                    // index comes from the definition rather than a `#native`
                    // symbol; the dest hand-over is the same two-call shape.
                    if is_c_text_call(stack.data.def(*op))
                        && let Some(&c_lib) = self.library_names.get(stack.data.def(*op).name())
                    {
                        let var_pos = stack.var_pos(var);
                        stack.add_op("OpClearStackText", self);
                        self.code_add(var_pos);
                        self.gen_cdylib_text_dest_call(stack, var, *op, args, c_lib);
                        return;
                    }
                }
                // always clear RefVar(Text) before appending — prevents
                // text accumulation across reassignments in text-returning functions.
                {
                    let var_pos = stack.var_pos(var);
                    stack.add_op("OpClearStackText", self);
                    self.code_add(var_pos);
                }
                self.generate(value, stack, false);
                let var_pos = stack.var_pos(var);
                stack.add_op("OpAppendStackText", self);
                self.code_add(var_pos);
                return;
            }
            // @PLN87 P2.2 / @PLN85 t4 — a `&`-param whole-record write-back that
            // installs a fresh OWNED store (`o = Obj{..}` literal OR `o = mk()`
            // owned-returning call) must FREE the DISPLACED caller store, else it
            // orphans.  P2.2 covered only the literal (via `is_skip_free`); t4 is
            // the call twin, whose NRVO buffer `__ref_N` is NOT skip_free — the
            // ownership oracle's `Own::Owned` names both.  Aliasing-safe: stash
            // `*o`'s OLD store BY VALUE (`OpGetStackRef` reads it, and the value
            // survives the set), install the new store, then `OpFreeRefIfDistinct`
            // — a self-reading RHS (`o = mk_from(o)`) keeps the old store live
            // across the eval (no free-before UAF, unlike the old P2.2 emission)
            // and a same-store install degrades to a no-op.  Path-sensitive (the
            // ops sit on the write-back branch only).  Heap inner type only; scalar
            // `&` has no store to free.
            // The question is about the LINK, not about how the link was INTRODUCED:
            // `@FR-B-Ref-Uniform` says a `&τ` variable is used exactly like a τ variable,
            // and the native twin (`generation/dispatch.rs`) already asks only the inner
            // type and the ownership.  Gated on `is_argument`, a `&` LOCAL bind
            // (`pd = &d; pd = S { n: 2 }`) installed the fresh store through the link and
            // orphaned the one it displaced — a leak on the interpreter, and one more
            // reason the two backends answered this shape differently (loft#1371).
            let amp_owned_writeback = (matches!(
                *tp,
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
            ) || crate::parser::vectors::is_keyed(&tp))
                && matches!(
                    crate::use_analysis::ownership_of(stack.data, stack.def_nr, value),
                    crate::use_analysis::Own::Owned
                );
            if amp_owned_writeback {
                // stash OLD = *o (by value — survives the SetStackRef below)
                let var_pos = stack.var_pos(var);
                stack.add_op("OpVarRef", self);
                self.code_add(var_pos);
                stack.add_op("OpGetStackRef", self);
                self.code_add(0u16);
            }
            let var_pos = stack.var_pos(var);
            stack.add_op("OpVarRef", self);
            self.code_add(var_pos);
            self.generate(value, stack, false);
            match *tp {
                Type::Integer(_) => stack.add_op("OpSetInt", self),
                Type::Character => stack.add_op("OpSetCharacter", self),
                Type::Single => stack.add_op("OpSetSingle", self),
                Type::Float => stack.add_op("OpSetFloat", self),
                // The write half of `&boolean` (loft#655).  Paired with
                // `OpGetBoolean` on the read path: both carry the tri-state
                // storage ↔ two-state expression conversion, which `OpSetByte`
                // would skip.
                Type::Boolean => stack.add_op("OpSetBoolean", self),
                Type::Enum(_, false, _) => stack.add_op("OpSetByte", self),
                // A KEYED collection joins the store-backed kinds: its slot holds a DbRef
                // exactly as a vector's does, so the write-back repoints it the same way.
                // The list was Vector/Reference/Enum and a `&hash<T[k]>` fell into the
                // `panic!` — an allow-list whose omission costs an ICE rather than an
                // optimisation, which is the trade the other way round (loft#1292).
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _) => {
                    stack.add_op("OpSetStackRef", self);
                }
                ref other if crate::parser::vectors::is_keyed(other) => {
                    stack.add_op("OpSetStackRef", self);
                }
                _ => panic!("Unknown reference variable type"),
            }
            self.code_add(0u16);
            if amp_owned_writeback {
                // free OLD unless the install kept it (witness = *o's NEW store)
                let var_pos = stack.var_pos(var);
                stack.add_op("OpVarRef", self);
                self.code_add(var_pos);
                stack.add_op("OpGetStackRef", self);
                self.code_add(0u16);
                stack.add_op("OpFreeRefIfDistinct", self);
            }
            return;
        }
        // destination-passing — avoid scratch buffer for text-returning natives.
        if matches!(stack.function.tp(var), Type::Text(_))
            && let Value::Call(op, args) = value.unspan()
        {
            let name = stack.data.def(*op).name().to_owned();
            if is_text_dest_native(&name) {
                let dest_name = name.clone() + "_dest";
                if let Some(&lib_nr) = self.library_names.get(&dest_name) {
                    self.gen_text_dest_call(stack, var, *op, args, lib_nr);
                    return;
                }
            }
            // @PLN10 N2b — cdylib FFI text call (an external `#native` text fn;
            // built-ins are `#pure`/`#rust` with empty `def.native`).  Dest-pass
            // through the bridge: write into `var` instead of `stores.scratch`.
            if is_cdylib_text_call(stack.data.def(*op)) {
                let native_sym = stack.data.def(*op).native().to_owned();
                if let Some(&cdylib_lib) = self.library_names.get(&native_sym) {
                    self.gen_cdylib_text_dest_call(stack, var, *op, args, cdylib_lib);
                    return;
                }
            }
            // @PLN24 arc D — the `#c` twin of the line above, keyed on the
            // definition's own name (a `#c` binding carries no `#native` symbol).
            if is_c_text_call(stack.data.def(*op))
                && let Some(&c_lib) = self.library_names.get(stack.data.def(*op).name())
            {
                self.gen_cdylib_text_dest_call(stack, var, *op, args, c_lib);
                return;
            }
        }
        // Plan-06 spine 8c hardening: catch IR shapes where `value`'s
        // stack-push width disagrees with `var`'s slot width before we
        // emit a put-op that silently corrupts adjacent slots.  User-
        // level code is screened by the parser's type checker; this
        // catches hand-built IR (par desugar, lambdas, tuple internals)
        // that bypass the front-end checks.  Surfaced after a
        // `parallel_queue_text` returning `integer` (8 B) was
        // assigned to an `i32` slot, overflowed into the next
        // variable, and produced a SIGSEGV on function return.
        //
        // Skip Tuple / Function — multi-element tuples push elements
        // separately and the put dispatch handles per-leaf widths
        // (`emit_tuple_var_pop_put`); fn-ref slot is 20 B but stdlib
        // OpPutFnRef pops 16 B Str then the tracker compensates by
        // -4 below.
        // loft#908 — appending the EMPTY literal to a text var is a no-op (the var
        // has just been cleared), and the put dispatch below skips `OpAppendText`
        // for it.  Skip the PUSH as well: the two decisions are one decision, and
        // separating them left a 16-byte const on the eval stack with nothing to
        // consume it, so `stack.position` ran high for the rest of the statement.
        // Where the statement was one arm of an `if`/`else` — which `?? ""` over a
        // CALL always is — the arms then disagreed in height and `gen_if`'s
        // equaliser "corrected" it with an `OpFreeStack` that discarded past the
        // frame's eval base into the LOCALS, overwriting a live text descriptor;
        // freeing that at scope exit aborted the interpreter (`free(): invalid
        // pointer`).  `gen_set_first_text` already skips the whole `set_var` for
        // this value; this is the reassignment path agreeing with it.
        if matches!(stack.function.tp(var).base(), Type::Text(_))
            && value == &Value::Text(String::new())
        {
            return;
        }
        let stack_before = stack.position;
        // A fn-ref slot is the PAIR (8 B d_nr + 12 B closure DbRef), and the put-op chosen
        // below pops all twenty for a `Function` slot.  A non-capturing source — a bare
        // `dbl`, or a lambda that captures nothing — lowers to the lone d_nr and pushes
        // only eight, so the pop ran twelve bytes into whatever sat beneath it.  Route the
        // push through `gen_fn_ref_value`, which tops a short one up with the null-closure
        // sentinel, exactly as the FIRST-Set path does (`gen_set_first_at_tos`'s
        // `Type::Function` arm).  Same predicate as the put-op selection below, so the push
        // and the pop that consumes it stay one decision and cannot drift apart.
        //
        // The first Set was the only fn-ref write anyone had exercised, so this
        // REASSIGNMENT half kept the eight-byte form: `g = dbl` after `g` was already
        // live panicked `fn_call_ref: fn_var=16 < 20` on `--interpret` while `--native`
        // ran it, which is the backend split rather than a shared refusal.  A CAPTURING
        // lambda already pushes the pair (its closure block's type IS the function), so it
        // is unchanged — that case worked, and it is the shape being matched.
        if matches!(stack.function.tp(var).base(), Type::Function(_, _, _))
            && !matches!(value.unspan(), Value::Null)
        {
            self.gen_fn_ref_value(value, stack);
        } else {
            self.generate(value, stack, false);
        }
        // loft#1026 — a bare `Value::Null` has no `generate` arm, so it pushes NOTHING while
        // the put-op below pops the slot's full width: `OpAppendText` then read whatever the
        // eval stack happened to hold under it.  Give it the slot's typed null instead, the
        // same repair `gen_dest_call_args` makes for an omitted argument and the same value
        // `--native` already writes for this IR (`STRING_NULL.to_string()`), so the two
        // backends store one value rather than disagreeing.  `gen_set_first_at_tos` guards
        // the same hazard by returning early; this is the reassignment/append half.
        //
        // Reached by a generic monomorph: a template's zero-value work-ref is `Set(v, Null)`
        // on a `Reference`, and substituting `T = text` retypes the slot without retyping
        // that null.  Silent without `LOFT_POISON` (a plausible stale `Str`), a SIGSEGV with.
        if stack.position == stack_before && matches!(value.unspan(), Value::Null) {
            let tp = stack.function.tp(var).clone();
            self.emit_typed_null(stack, &tp);
        }
        #[cfg(debug_assertions)]
        {
            let var_tp = stack.function.tp(var).clone();
            if !matches!(var_tp, Type::Tuple(_) | Type::Function(_, _, _)) {
                let pushed = stack.position.saturating_sub(stack_before);
                let slot = size(&var_tp, &Context::Variable);
                // #493: the `pushed == slot` model holds ONLY for the RAW
                // fixed-width stores — `OpPutInt` / `OpPutFloat` — which copy the
                // pushed bytes verbatim and so can overrun a narrower neighbour
                // (the SIGSEGV that motivated this check: an over-wide value in an
                // integer slot).  EVERY other put NORMALISES the value to the slot
                // width on store, so a wider (8-stepped) eval-stack push is by
                // design and cannot overrun the neighbour:
                //   * `OpPutRef` (Reference/Vector/keyed/Enum-payload/Iterator):
                //     stores a 12 B DbRef — an `OpNewRecord` interior ref or a
                //     record/File call result (16 B) is normalised.
                //   * `OpPutBool` (1 B), `OpPutEnum` (plain enum, 1 B),
                //     `OpPutSingle` (4 B), `OpPutCharacter` (4 B): store a fixed
                //     narrow width from the 8 B stack value.
                //   * `OpAppendText` (Text, incl. `text?`): deep-copies the 16 B
                //     `Str` view into the 24 B `String` slot.
                //   * fn-ref / tuple are excluded above.
                // Restricting the check to the raw-put family clears the DA
                // warning flood (t_4File_lines `_elm`, plain enums, `single`,
                // `text?`, …) WITHOUT weakening the integer-overflow detection: an
                // integer slot is 8 B, so a correct store matches and only a
                // genuinely over-wide value (a 16 B text into an int slot) trips.
                let raw_put = matches!(var_tp.base(), Type::Integer(_) | Type::Float);
                if raw_put && pushed != slot && pushed != 0 {
                    // The model (pushed == Variable-slot width) is
                    // INCOMPLETE for several put-op families that pop a
                    // different width than the slot and compensate (the
                    // fn-ref and text conversions above are the two
                    // taught so far; the armed-corpus sweep 2026-06-12
                    // found more, e.g. a 16 B call result put into a
                    // 12 B DbRef slot in `t_4File_lines`).  Until the
                    // per-op width table exists, report instead of
                    // killing the armed channel; LOFT_WIDTH_ASSERT=1
                    // escalates for focused slot work.
                    let msg = format!(
                        "[set_var] width mismatch assigning to '{}' ({}) in fn '{}': \
                         value pushed {pushed} bytes, slot is {slot} bytes.  IR: {value:?}",
                        stack.function.name(var),
                        var_tp.name(stack.data),
                        stack.data.def(stack.def_nr).name(),
                    );
                    assert!(std::env::var_os("LOFT_WIDTH_ASSERT").is_none(), "{msg}");
                    eprintln!("{msg} (warning; LOFT_WIDTH_ASSERT=1 to make fatal)");
                }
            }
        }
        let var_pos = stack.var_pos(var);
        // @PLN25: an `Optional(τ)` reassignment uses the base put-op (same sentinel storage)
        // — peel the marker (mirrors the first-Set `gen_set_first_at_tos`). Without this a
        // nullable local reassignment (`x: integer? = 5; x = 9`) panicked "Unknown var type".
        match stack.function.tp(var).base() {
            Type::Integer(_) => stack.add_op("OpPutInt", self),
            Type::Function(_, _, _) => {
                stack.add_op("OpPutFnRef", self);
                // Post-2c fn-ref slot is 20 bytes, but OpPutFnRef's stdlib
                // signature pops `text` (16 B Str).  Subtract the 4-byte
                // discrepancy from the compile-time stack tracker.
                stack.position -= stack.fnref_signature_gap(); // @PLAN53 S4 fn-ref
            }
            Type::Character => stack.add_op("OpPutCharacter", self),
            Type::Enum(_, false, _) => stack.add_op("OpPutEnum", self),
            Type::Boolean => stack.add_op("OpPutBool", self),
            Type::Single => stack.add_op("OpPutSingle", self),
            Type::Float => stack.add_op("OpPutFloat", self),
            // The empty literal never reaches here — it returned above, before the
            // push, so that the push and the op that consumes it stay one decision
            // (loft#908).
            Type::Text(_) => stack.add_op("OpAppendText", self),
            Type::Vector(_, _)
            | Type::Reference(_, _)
            | Type::Enum(_, true, _)
            | Type::Iterator(_, _)
            // @PLN85 p188 — the keyed heap collections are DbRef-backed stores
            // like Vector, so a plain DbRef put binds them; needed for the
            // discarded-owned `sorted<>`-return `__lift_N` temp (get_free_vars
            // then frees the store).
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _) | Type::Trie(_, _, _) => {
                stack.add_op("OpPutRef", self);
            }
            Type::Tuple(elems) => {
                // T1.4: store each element from the stack into the variable.
                // Elements are on the stack in order; emit OpPut* for each in
                // reverse order (last element is at top of stack).  For
                // nested `Type::Tuple` elements, recurses to emit per-leaf
                // OpPut* calls at the correct sub-offsets.
                let elems = elems.clone();
                let tuple_var_base = stack.function.stack(var);
                self.emit_tuple_var_pop_put(stack, &elems, tuple_var_base);
                return;
            }
            _ => panic!(
                "Unknown var {} type {} at {}",
                stack.function.name(var),
                stack.function.tp(var).name(stack.data),
                stack.data.def(stack.def_nr).position()
            ),
        }
        self.code_add(var_pos);
    }

    /// Generate the argument pushes for a destination-passing native call
    /// (`gen_text_dest_call` / `gen_cdylib_text_dest_call`).
    ///
    /// A `Value::Null` argument (explicit, or a parser-filled default for an
    /// omitted parameter) generates 0 bytes on its own, while the callers' pop
    /// accounting — and the runtime bridge dispatcher — consume the full
    /// parameter size, desyncing the frame (#307: the `render("a", "b")`
    /// two-defaulted-args slot panic).  Emit the typed null sentinel so every
    /// argument occupies exactly `size(attr_type)` (mirrors the same repair in
    /// `generate_call`'s mutable-arg loop).
    fn gen_dest_call_args(&mut self, stack: &mut Stack, args: &[Value], attr_types: &[Type]) {
        for (a_nr, arg_val) in args.iter().enumerate() {
            let stack_before = stack.position;
            self.generate(arg_val, stack, false);
            if matches!(arg_val.unspan(), Value::Null)
                && stack.position == stack_before
                && let Some(tp) = attr_types.get(a_nr)
            {
                self.emit_typed_null(stack, tp);
            }
        }
    }

    /// Emit a destination-passing call for a text-returning native function.
    ///
    /// Instead of: evaluate call → Str on stack → OpAppendText(var)
    /// Emits:      args → OpCreateStack(var) → `OpStaticCall`  (native writes to var directly)
    fn gen_text_dest_call(
        &mut self,
        stack: &mut Stack,
        var: u16,
        op: u32,
        args: &[Value],
        lib_nr: u16,
    ) {
        let attr_types: Vec<Type> = stack
            .data
            .def(op)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect();
        self.gen_dest_call_args(stack, args, &attr_types);
        // The destination DbRef the `_dest` native writes into.  For a plain
        // `Text` var that is the create-stack address of the var's String slot.
        // @PLN10 — for a `RefVar(Text)` var (a `&mut String` work buffer, e.g. a
        // return-dep local promoted by `text_return`) the dest is the RAW DbRef
        // the RefVar holds — pushed via `OpVarRef` (returns `reference`, so
        // `add_op` self-accounts the stepped DbRef, matching `emit_push_create_stack`).
        // The caller (`set_var`) has already cleared the buffer, so the native's
        // `push_str` appends into an empty record.
        if matches!(stack.function.tp(var), Type::RefVar(_)) {
            let var_pos = stack.var_pos(var);
            stack.add_op("OpVarRef", self);
            self.code_add(var_pos);
        } else {
            let dep_offset = stack.var_pos(var);
            self.emit_push_create_stack(stack, dep_offset);
        }
        stack.add_op("OpStaticCall", self);
        self.code_add(lib_nr);
        // @PLAN53 cluster 2 / S4: the args + the create-stack dest DbRef each
        // occupy a STEPPED span (DbRef 12 -> 16 in aligned mode), matching the
        // native get<T> pops; identity when off.  (Twin of try_text_dest_pass —
        // this is the set_var `r = text_dest_native(...)` path; missing the
        // step here drifted the frame -4 and corrupted the var read — a8.)
        for attr_type in &attr_types {
            stack.position -= stack.step(size(attr_type, &Context::Argument));
        }
        stack.position -= stack.step(size_of::<crate::keys::DbRef>() as u16);
    }

    /// @PLN10 N2b — destination-passing for a **cdylib FFI** text call.  Unlike a
    /// built-in `_dest` native (one `OpStaticCall`), the foreign function has no
    /// `_dest` variant, so this emits TWO calls: `n_set_bridge_dest` (pops the
    /// work-buffer DbRef and stashes it on `stores`), then the cdylib itself —
    /// whose bridge (`bridge_text_result`) writes the `LoftStr` into that record
    /// and pushes NOTHING (dest-mode), bypassing `stores.scratch`.  Stack order
    /// mirrors `gen_text_dest_call`: args, then the dest on top.
    fn gen_cdylib_text_dest_call(
        &mut self,
        stack: &mut Stack,
        var: u16,
        op: u32,
        args: &[Value],
        cdylib_lib: u16,
    ) {
        let set_dest_lib = *self
            .library_names
            .get("n_set_bridge_dest")
            .expect("n_set_bridge_dest registered in native FUNCTIONS");
        let attr_types: Vec<Type> = stack
            .data
            .def(op)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect();
        self.gen_dest_call_args(stack, args, &attr_types);
        // Push the destination DbRef (top) — the raw RefVar DbRef for a promoted
        // return buffer, else the create-stack address of the plain text slot
        // (same dest forms as `gen_text_dest_call`).
        if matches!(stack.function.tp(var), Type::RefVar(_)) {
            let var_pos = stack.var_pos(var);
            stack.add_op("OpVarRef", self);
            self.code_add(var_pos);
        } else {
            let dep_offset = stack.var_pos(var);
            self.emit_push_create_stack(stack, dep_offset);
        }
        // n_set_bridge_dest pops the dest DbRef off the top and stashes it.
        stack.add_op("OpStaticCall", self);
        self.code_add(set_dest_lib);
        stack.position -= stack.step(size_of::<crate::keys::DbRef>() as u16);
        // The cdylib dispatch pops its args; the bridge writes into the stashed
        // dest and pushes nothing — so subtract the args but DON'T add a result.
        stack.add_op("OpStaticCall", self);
        self.code_add(cdylib_lib);
        for attr_type in &attr_types {
            stack.position -= stack.step(size(attr_type, &Context::Argument));
        }
    }
}

/// Check if a Value is a divergent expression (return/break/continue)
/// that never produces a value at the join point.
fn is_divergent(node: IrNode) -> bool {
    match node.kind() {
        ValueType::Return | ValueType::Break | ValueType::Continue => true,
        // scopes.rs wraps `return` in `Insert([free_ops..., Return(...)])` so the
        // raw-Return check misses it. Walk the last op of Insert/Block to recover
        // divergence for these wrappers.
        ValueType::Insert => {
            let ops = node.insert_items();
            !ops.is_empty() && is_divergent(ops.get(ops.len() - 1))
        }
        ValueType::Block => {
            let ops = node.as_block().operators();
            !ops.is_empty() && is_divergent(ops.get(ops.len() - 1))
        }
        _ => false,
    }
}

/// Recursively checks whether `value` contains a direct `Var(v)` reference.
/// Used in debug builds to detect first-assignment self-reference bugs.
#[cfg(debug_assertions)]
fn ir_contains_var(value: &Value, v: u16) -> bool {
    // Pass-2 wave 2: descent now comes from `Value::any_node`.  The
    // hand-rolled predecessor had no `Span` arm, so a Span-wrapped
    // self-reference escaped this assertion entirely.
    value.any_node(&mut |n| matches!(n, Value::Var(x) if *x == v))
}

/// Recursively prints a `Value` IR tree to stderr in a loft-like syntax.
///
/// Gated by the `LOFT_IR` environment variable (set to a function-name filter
/// or `*` for all); only compiled in debug builds to keep release binaries
/// clean.  Produces a lot of output for large functions, so the filter is
/// important.
#[cfg(debug_assertions)]
#[allow(clippy::too_many_lines)]
fn print_ir(value: &Value, data: &crate::data::Data, vars: &Function, depth: usize) {
    let pad = "  ".repeat(depth);
    match value {
        Value::Null => eprint!("null"),
        Value::Int(n) => eprint!("{n}"),
        Value::Long(n) => eprint!("{n}L"),
        Value::Float(f) => eprint!("{f}"),
        Value::Single(f) => eprint!("{f}f"),
        Value::Boolean(b) => eprint!("{b}"),
        Value::Enum(v, tp) => eprint!("enum({v},tp={tp})"),
        Value::Text(s) => eprint!("{s:?}"),
        Value::Line(_) => {} // source-line markers: skip
        Value::Var(n) => eprint!("{}", vars.name(*n)),
        Value::Break(n) => eprint!("break({n})"),
        Value::Continue(n) => eprint!("continue({n})"),
        Value::Keys(keys) => eprint!("keys({keys:?})"),
        Value::Set(v, inner) => {
            eprint!("{} = ", vars.name(*v));
            print_ir(inner, data, vars, depth);
        }
        Value::Return(inner) => {
            eprint!("return ");
            print_ir(inner, data, vars, depth);
        }
        Value::Drop(inner) => {
            eprint!("drop(");
            print_ir(inner, data, vars, depth);
            eprint!(")");
        }
        Value::Call(d, args) => {
            eprint!("{}(", data.def(*d).name());
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    eprint!(", ");
                }
                print_ir(a, data, vars, depth);
            }
            eprint!(")");
        }
        Value::CallRef(v, args) => {
            eprint!("fn_ref[{}](", vars.name(*v));
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    eprint!(", ");
                }
                print_ir(a, data, vars, depth);
            }
            eprint!(")");
        }
        Value::Block(b) => {
            eprintln!("{{  // {}", b.name);
            for op in &b.operators {
                eprint!("{pad}  ");
                print_ir(op, data, vars, depth + 1);
                eprintln!();
            }
            eprint!("{pad}}}");
        }
        Value::Loop(b) => {
            eprint!("loop ");
            print_ir(&Value::Block(b.clone()), data, vars, depth);
        }
        Value::Insert(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    eprint!("{pad}  ");
                }
                print_ir(item, data, vars, depth);
                if i + 1 < items.len() {
                    eprintln!();
                }
            }
        }
        Value::If(cond, then, els) => {
            eprint!("if ");
            print_ir(cond, data, vars, depth);
            eprint!(" ");
            print_ir(then, data, vars, depth);
            if **els != Value::Null {
                eprint!(" else ");
                print_ir(els, data, vars, depth);
            }
        }
        Value::Iter(v, create, next, extra) => {
            eprint!("for {} in ", vars.name(*v));
            print_ir(create, data, vars, depth);
            if **extra != Value::Null {
                eprint!(", extra=");
                print_ir(extra, data, vars, depth);
            }
            // `next` is the advance expression: omit for brevity
            let _ = next;
        }
        Value::Tuple(elems) => {
            eprint!("(");
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    eprint!(", ");
                }
                print_ir(e, data, vars, depth);
            }
            eprint!(")");
        }
        Value::TupleGet(var, idx) => {
            eprint!("{}.{idx}", vars.name(*var));
        }
        Value::TuplePut(var, idx, val) => {
            eprint!("{}.{idx} = ", vars.name(*var));
            print_ir(val, data, vars, depth);
        }
        Value::Yield(inner) => {
            eprint!("yield ");
            print_ir(inner, data, vars, depth);
        }
        Value::FnRef(d_nr, clos_var, _) => {
            eprint!("FnRef({d_nr}, clos={})", vars.name(*clos_var));
        }
        Value::Parallel(arms) => {
            eprintln!("parallel {{");
            for arm in arms {
                eprint!("{pad}  ");
                print_ir(arm, data, vars, depth + 1);
                eprintln!(";");
            }
            eprint!("{pad}}}");
        }
        // `Span` wraps every expression with a source position — unwrap it so
        // the dump reads cleanly.
        Value::Span(b) => print_ir(&b.1, data, vars, depth),
        // Rare IR nodes (FnRefDnr, …) printed opaquely; this is a
        // debug-only dumper.  The catch-all also keeps `print_ir` exhaustive in
        // debug-assertions builds (e.g. under cargo-fuzz) as new variants land.
        _ => eprint!("<ir>"),
    }
}
