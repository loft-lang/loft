// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I73 — native function registry: the interpreter's `par` natives

//! The natives behind `par(...)` under the interpreter: each runs the worker function's
//! BYTECODE on a pool of interpreter threads (`parallel`).  A compiled program runs `par`
//! through its own generated workers, so none of this is in the runtime it links (@PLN191).
#![allow(non_snake_case)]
use crate::database::{Call, Stores};
use crate::keys::DbRef;
use crate::native_gate::is_text_work_buffer;
use crate::parallel::{WorkerProgram, run_parallel_text};
use crate::vector;
use std::sync::Arc;

/// The `par` natives, registered after `native::FUNCTIONS`.
pub const FUNCTIONS: &[(&str, Call)] = &[
    ("n_parallel_for", n_parallel_for),
    ("n_parallel_for_light", n_parallel_for_light),
    ("n_parallel_discard", n_parallel_discard),
    ("n_parallel_queue", n_parallel_queue),
    ("n_parallel_fold", n_parallel_fold),
    ("n_parallel_buf_get", n_parallel_buf_get),
    ("n_parallel_buf_drop", n_parallel_buf_drop),
    ("n_parallel_queue_text", n_parallel_queue_text),
    // @PLN10 Phase 1 — dest-passing variant (interp scratch retirement).
    ("n_parallel_buf_get_text_dest", n_parallel_buf_get_text_dest),
    ("n_parallel_buf_drop_text", n_parallel_buf_drop_text),
    ("n_parallel_queue_ref", n_parallel_queue_ref),
    ("n_parallel_buf_get_ref", n_parallel_buf_get_ref),
    ("n_parallel_buf_drop_ref", n_parallel_buf_drop_ref),
    ("n_parallel_queue_narrow", n_parallel_queue_narrow),
    ("n_parallel_buf_get_narrow", n_parallel_buf_get_narrow),
    ("n_parallel_buf_drop_narrow", n_parallel_buf_drop_narrow),
    ("n_parallel_buf_get_single", n_parallel_buf_get_single),
    ("n_parallel_buf_get_float", n_parallel_buf_get_float),
    ("n_parallel_queue_fn", n_parallel_queue_fn),
    ("n_parallel_buf_get_fn", n_parallel_buf_get_fn),
    ("n_parallel_buf_drop_fn", n_parallel_buf_drop_fn),
];

/// Plan-06 phase 4d.A — typed worker-input dispatch.  Replaces the
/// sentinel-encoded `primitive_first_arg_slot_size` channel with an
/// explicit enum so the dispatcher's pre-loop code reads as a plain
/// match, and so wide-inline first args (tuples, fn-refs, anything
/// 9..=64 bytes) are not silently misrouted as "DbRef-by-pointer"
/// when their vector storage is actually inline.
///
/// Cap at 64 bytes: anything wider falls back to `Ref` so the
/// per-row push doesn't blow the worker stack frame budget.  64 is
/// one cache line plus headroom for 4-element tuples of Long /
/// Float.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum InputKind {
    /// Worker takes a `DbRef` in slot 0 (struct-by-ref, vector,
    /// keyed collection, or any inline-typed first arg whose slot
    /// width exceeds the 64-byte inline cap).
    Ref,
    /// Worker takes a 16-byte `Str` in slot 0.  The G3 path
    /// (text-input dispatch) hooks here.
    Text,
    /// Worker takes `size` bytes inline in slot 0.  `size` is the
    /// stack-representation width from `variables::size` (which
    /// reports 20 for fn-ref, 16 for `(int, int)`, etc.) — not the
    /// 4-byte vector storage width.
    Primitive { size: u8 },
}

/// Maximum bytes accepted for `InputKind::Primitive`.  Anything
/// wider falls back to `InputKind::Ref` so the worker stack frame
/// stays bounded.
const INPUT_PRIMITIVE_MAX_BYTES: usize = 64;

/// P189d — element types of a tuple first-arg, or `None` if the
/// worker's first arg isn't a tuple.  Used by the wide-input path
/// to dispatch `read_tuple_at_wide` (per-element inflation,
/// notably 4 B heap text-pointer → 16 B stack `Str`) instead of
/// `read_primitive_at_wide` (flat memcpy).
///
/// Plan-06 phase 4d.A.2: also returns `Some(vec![first.typedef])`
/// for `Type::Function` first-args so the dispatcher routes through
/// `read_tuple_at_wide` which produces a clean failure mode (vs. the
/// hang from raw `read_primitive_at_wide` reading 20 bytes from a
/// 4-byte stride).
fn tuple_first_arg_types(def: &crate::data::Definition) -> Option<Vec<crate::data::Type>> {
    let first = def.attributes.first()?;
    match &first.typedef {
        crate::data::Type::Tuple(elems) => Some(elems.clone()),
        crate::data::Type::Function(..) => Some(vec![first.typedef.clone()]),
        _ => None,
    }
}

fn input_kind_for_first_arg(def: &crate::data::Definition) -> InputKind {
    use crate::data::Type;
    let Some(first) = def.attributes.first() else {
        // No args — treat as Ref (the dispatcher won't push anything
        // either way; this matches the legacy fall-through to the
        // DbRef-passing path).
        return InputKind::Ref;
    };
    match &first.typedef {
        Type::Text(_) => InputKind::Text,
        Type::Boolean | Type::Enum(_, false, _) => InputKind::Primitive { size: 1 },
        Type::Single | Type::Character => InputKind::Primitive { size: 4 },
        Type::Integer(_) | Type::Float => InputKind::Primitive { size: 8 },
        Type::Function(..) | Type::Tuple(_) => {
            let sz = crate::variables::size(&first.typedef, &crate::data::Context::Argument);
            if (sz as usize) <= INPUT_PRIMITIVE_MAX_BYTES && sz > 0 {
                InputKind::Primitive { size: sz as u8 }
            } else {
                InputKind::Ref
            }
        }
        _ => InputKind::Ref,
    }
}

/// Plan-06 ARC.md A4 (closed 2026-05-07) — the legacy materialised
/// `parallel_for` path is unreachable.  After A3.6, every primitive
/// return type — including Single (4-byte f32) and Float (8-byte
/// f64) — routes through the Queue family (`n_parallel_queue` /
/// `_narrow` / `_text` / `_ref` / `_fn`).  The materialised path
/// (`Stitch::Concat`) had only one consumer in the parser (the
/// `actual_par_d_nr` resolution in `build_parallel_for_ir`) which is
/// now dead.  The function entry is retained in the FUNCTIONS table
/// for stack-trace symbol resolution but its body panics if invoked.
fn n_parallel_for(_stores: &mut Stores, _stack: &mut DbRef) {
    unreachable!(
        "n_parallel_for: ARC.md A4 retired the materialised path; \
         all par() routes through the Queue family (n_parallel_queue / \
         _narrow / _text / _ref / _fn).  If you reached this, the \
         parser failed to route the worker's return type to a Queue \
         variant — extend `narrow_route_for` / the route_*_queue \
         gates in src/parser/collections.rs."
    );
}

/// Plan-06 ARC.md A4 (closed 2026-05-07) — companion to
/// `n_parallel_for`.  Was the per-worker-pool variant; same fate
/// after A3.6 routes Single/Float through Queue.
fn n_parallel_for_light(_stores: &mut Stores, _stack: &mut DbRef) {
    unreachable!(
        "n_parallel_for_light: ARC.md A4 retired the light Concat \
         path.  See `n_parallel_for` panic message for the diagnosis \
         hint."
    );
}

/// Plan-06 PRIORITY.md spine step 3 — `Stitch::Discard` runtime entry.
/// Pops the same arg layout as `n_parallel_for` (input, element_size,
/// return_size, threads, func, extras..., n_extra), dispatches the
/// worker fn per row via `run_parallel_discard` (step 2), drops every
/// result.  No result vector allocated; nothing pushed back onto the
/// stack — void return.
///
/// Used when a fused for-par body never references the worker result
/// (`for x in input par(_=fn(x), N) { }` or
/// `for x in input par(r=fn(x), N) { /* no use of r */ }`).  Phase
/// 10 (drop materialised vector) extends this contract: any par call
/// whose result is consumed single-pass without random access lowers
/// here.
#[expect(clippy::too_many_lines, reason = "inherited")]
fn n_parallel_discard(stores: &mut Stores, stack: &mut DbRef) {
    // Same stack layout / pop order as n_parallel_for.
    let n_extra = stores.get::<i64>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(stores.get::<i64>(stack) as u64);
    }
    extra_args.reverse();

    let v_func = stores.get::<i64>(stack) as i32;
    let v_threads = stores.get::<i64>(stack) as i32;
    let v_return_size = stores.get::<i64>(stack) as i32;
    let v_element_size = stores.get::<i64>(stack) as i32;
    let v_input = stores.get::<DbRef>(stack);

    let (fn_pos, program) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_discard called outside State::execute()");
        let data = ctx.data();
        assert!(
            v_func >= 0,
            "parallel_discard: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let fn_pos = data.def(d_nr).code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
                data_ptr: ctx.data.clone(),
                fn_positions: Arc::new(data.definitions.iter().map(|d| d.code_position).collect()),
                const_refs: Arc::clone(&ctx.const_refs),
                frame_headroom: Arc::clone(&ctx.frame_headroom),
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
        )
    };

    let element_size = v_element_size as u32;
    let n_threads = (v_threads as usize).max(1);
    // return_size is needed by execute_at_raw to drain the worker's
    // return value off the stack.  Discard accepts any return shape;
    // clamp to the same 1..=8 backstop the light path uses.
    //
    // The INPUT shape comes from the same two readers the queue family uses
    // (`input_kind_for_first_arg` / `tuple_first_arg_types`), so a worker taking a
    // primitive, a text or a tuple row gets it the way its parameter slot expects.
    let (return_size, primitive_input_size, tuple_input_types, n_hidden_text, n_hidden_dests) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_discard: missing context");
        let data = ctx.data();
        let def = data.def(v_func as u32);
        let derived = u32::from(crate::variables::size(
            &def.returned,
            &crate::data::Context::Argument,
        ));
        let return_size = if (1..=8).contains(&derived) {
            derived
        } else {
            (v_return_size.clamp(1, 8)) as u32
        };
        let primitive_input_size = match input_kind_for_first_arg(def) {
            InputKind::Ref => 0u32,
            InputKind::Text => u32::MAX,
            InputKind::Primitive { size } => u32::from(size),
        };
        // Classified by TYPE, the same two readers `parallel_queue_dispatch` uses —
        // a second spelling of "which attributes are hidden" is how the prefix
        // heuristic it replaced went wrong.
        let n_hidden_text = def
            .attributes
            .iter()
            .filter(|a| is_text_work_buffer(&a.typedef))
            .count();
        let n_hidden_dests = def
            .attributes
            .iter()
            .filter(|a| {
                a.hidden
                    && matches!(
                        a.typedef,
                        crate::data::Type::Reference(_, _)
                            | crate::data::Type::Vector(_, _)
                            | crate::data::Type::Enum(_, true, _)
                    )
            })
            .count();
        (
            return_size,
            primitive_input_size,
            tuple_first_arg_types(def),
            n_hidden_text,
            n_hidden_dests,
        )
    };

    crate::parallel::run_parallel_discard(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        n_threads,
        &extra_args,
        return_size,
        primitive_input_size,
        tuple_input_types,
        n_hidden_text,
        n_hidden_dests,
    );
    // Void return — nothing to push.  Note: post-2c integers are 8B,
    // so callers expecting a return on the stack would be wrong to
    // invoke this; the codegen path that emits the call sites this
    // fn knows it returns void and does not pop a result.
}

/// Plan-06 ARC.md A8.b — discriminator for the shared queue
/// dispatcher.  Each variant matches one of the 5 `n_parallel_queue*`
/// native fns; the per-stitch behaviour lives in
/// `parallel_queue_dispatch`'s match arms (dispatcher choice +
/// post-processing + per-type buffer-stack push).
#[derive(Copy, Clone)]
enum QueueStitch {
    Int,
    Text,
    Ref,
    Narrow,
    Fn,
}

/// Plan-06 ARC.md A8.b — shared queue dispatcher.  Replaces the
/// boilerplate that was duplicated across `n_parallel_queue`,
/// `_text`, `_ref`, `_narrow`, and `_fn`.  Pops the common 7 args
/// from the runtime stack (n_extra, extras, fn, threads,
/// return_size, element_size, input), builds a `WorkerProgram` +
/// per-stitch context fetches (n_hidden_text, n_hidden_dests,
/// ret_type, data_ptr, primitive_input_size, tuple_input_types,
/// worker_return_size) in ONE parallel_ctx borrow scope, then
/// dispatches via match on `stitch`.
///
/// The 5 native fns each become a 1-line wrapper calling this with
/// the matching `QueueStitch` value.  Net ~150-200 LOC saved.
///
/// ARC.md A8 (collapse the 5 `run_parallel_*` dispatchers in
/// `src/parallel.rs` under one trait) was deferred — those
/// dispatchers diverge structurally.  A8.b targets a different
/// layer (interp-bridge native fns) where the divergence IS
/// boilerplate.
#[expect(clippy::too_many_lines, reason = "inherited")]
fn parallel_queue_dispatch(stores: &mut Stores, stack: &mut DbRef, stitch: QueueStitch) {
    let fn_label = match stitch {
        QueueStitch::Int => "parallel_queue",
        QueueStitch::Text => "parallel_queue_text",
        QueueStitch::Ref => "parallel_queue_ref",
        QueueStitch::Narrow => "parallel_queue_narrow",
        QueueStitch::Fn => "parallel_queue_fn",
    };

    // Pop common args (same layout as legacy n_parallel_for).
    let n_extra = stores.get::<i64>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(stores.get::<i64>(stack) as u64);
    }
    extra_args.reverse();

    let v_func = stores.get::<i64>(stack) as i32;
    let v_threads = stores.get::<i64>(stack) as i32;
    let v_return_size = stores.get::<i64>(stack) as i32;
    let v_element_size = stores.get::<i64>(stack) as i32;
    let v_input = stores.get::<DbRef>(stack);

    // Build (fn_pos, program) + per-stitch context fetches in one
    // parallel_ctx borrow scope.  Snapshot the `data_ptr` handle so the
    // Ref stitch's downstream call (which needs `&mut Stores`) can
    // read the definitions after we give up the borrow.  The ParallelCtx
    // outlives this fn's stack frame (set by State::execute before
    // any par fn dispatches).
    let (
        fn_pos,
        program,
        n_hidden_text,
        n_hidden_dests,
        ret_type,
        data_ptr,
        primitive_input_size,
        tuple_input_types,
        worker_return_size,
    ) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .unwrap_or_else(|| panic!("{fn_label} called outside State::execute()"));
        let data = ctx.data();
        assert!(
            v_func >= 0,
            "{fn_label}: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let def = data.def(d_nr);
        let fn_pos = def.code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        let program = WorkerProgram {
            bytecode,
            library,
            stack_trace_lib_nr: ctx.stack_trace_lib_nr,
            data_ptr: ctx.data.clone(),
            fn_positions: Arc::new(data.definitions.iter().map(|d| d.code_position).collect()),
            const_refs: Arc::clone(&ctx.const_refs),
            frame_headroom: Arc::clone(&ctx.frame_headroom),
            line_numbers: Arc::new(std::collections::BTreeMap::new()),
        };
        // Per-stitch extras (computed unconditionally; cost is a
        // few attribute reads, not heap allocations).
        // @PLAN59: classify worker attrs by TYPE, not name prefix.  The
        // old prefix heuristic ('__'-named ⇒ text buffer, user-named
        // hidden ⇒ heap dest) misclassified wrapper-promoted dests (named
        // `__ref_1`, Reference/Vector-typed) as text buffers — a live
        // frame-underflow panic for par workers calling wrapper-promoted
        // fns ('No elements left on the stack 8 < 12').
        let n_hidden_text = def
            .attributes
            .iter()
            .filter(|a| is_text_work_buffer(&a.typedef))
            .count();
        let n_hidden_dests = def
            .attributes
            .iter()
            .filter(|a| {
                a.hidden
                    && matches!(
                        a.typedef,
                        crate::data::Type::Reference(_, _)
                            | crate::data::Type::Vector(_, _)
                            | crate::data::Type::Enum(_, true, _)
                    )
            })
            .count();
        let ret_type = def.returned.clone();
        let primitive_input_size = match input_kind_for_first_arg(def) {
            InputKind::Ref => 0u32,
            InputKind::Text => u32::MAX,
            InputKind::Primitive { size } => u32::from(size),
        };
        let tuple_input_types = tuple_first_arg_types(def);
        let worker_return_size = u32::from(crate::variables::size(
            &def.returned,
            &crate::data::Context::Argument,
        ));
        (
            fn_pos,
            program,
            n_hidden_text,
            n_hidden_dests,
            ret_type,
            ctx.data.clone(),
            primitive_input_size,
            tuple_input_types,
            worker_return_size,
        )
    };

    let element_size = v_element_size as u32;
    let n_threads = (v_threads as usize).max(1);

    // Per-stitch dispatcher call + buffer-stack push.
    let n_rows: i64 = match stitch {
        QueueStitch::Int => {
            // Plan-06 spine step 8b' — derive return_size from the
            // worker fn's def so primitive / text / tuple-input
            // workers dispatch through the right input arm.
            let return_size = if (1..=8).contains(&worker_return_size) {
                worker_return_size
            } else {
                (v_return_size.clamp(1, 8)) as u32
            };
            let buf = crate::parallel::run_parallel_queue(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                return_size,
                primitive_input_size,
                tuple_input_types,
            );
            let n = buf.len() as i64;
            stores.par_buffer_stack.push(buf);
            n
        }
        QueueStitch::Text => {
            let _ = v_return_size; // text mode ignores caller-supplied size
            let n_rows = vector::length_vector(&v_input, &stores.allocations) as usize;
            let buf = run_parallel_text(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                n_rows,
                n_hidden_text,
                primitive_input_size,
                tuple_input_types,
            );
            let n = buf.len() as i64;
            stores.par_text_buffer_stack.push(buf);
            n
        }
        QueueStitch::Ref => {
            let _ = v_return_size; // ref mode uses ret_type directly
            let n_rows = crate::vector::length_vector(&v_input, &stores.allocations) as usize;
            let data: &crate::data::Data = data_ptr
                .get()
                .expect("a parallel context carries its program's definitions");
            let (refs, adopted) = crate::parallel::run_parallel_queue_ref(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                n_rows,
                &ret_type,
                data,
                n_hidden_dests,
                primitive_input_size,
                tuple_input_types,
            );
            let n = refs.len() as i64;
            stores.par_ref_buffer_stack.push((refs, adopted));
            n
        }
        QueueStitch::Narrow => {
            // Plan-06 ARC.md A3 / A3.5 — workers run with the
            // SHAPE-NATURAL return size; the pack loop below
            // truncates each u64 row to the buffer stride.
            let stride = match v_return_size {
                1 | 2 | 4 => v_return_size as u8,
                other => {
                    panic!("parallel_queue_narrow: invalid return_size {other} (expected 1/2/4)")
                }
            };
            let buf64 = crate::parallel::run_parallel_queue(
                stores,
                program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
                worker_return_size,
                primitive_input_size,
                tuple_input_types,
            );
            let mut bytes = Vec::with_capacity(buf64.len() * stride as usize);
            for u in &buf64 {
                let row_bytes = u.to_le_bytes();
                bytes.extend_from_slice(&row_bytes[..stride as usize]);
            }
            let n = buf64.len() as i64;
            stores.par_narrow_buffer_stack.push((bytes, stride));
            n
        }
        QueueStitch::Fn => {
            let _ = v_return_size; // always 20 for fn-ref
            let buf = crate::parallel::run_parallel_queue_fn(
                stores,
                &program,
                fn_pos,
                &v_input,
                element_size,
                n_threads,
                &extra_args,
            );
            let n = (buf.len() / 20) as i64;
            stores.par_fn_buffer_stack.push(buf);
            n
        }
    };

    stores.put(stack, n_rows);
}

/// Plan-06 PRIORITY.md spine step 8a — `Stitch::Queue` runtime entry.
/// Pops the same arg layout as `n_parallel_for` (input, element_size,
/// return_size, threads, func, extras..., n_extra), dispatches the
/// worker fn per row via `run_parallel_queue` (step 4), and pushes
/// the resulting `Vec<u64>` onto `stores.par_buffer_stack`.  The
/// row count is pushed onto the operand stack so the caller can use
/// it as a loop bound.
///
/// Step 8b is the first parser-side consumer; until then the only
/// caller is the Rust unit test in `tests/threading.rs`.
///
/// Plan-06 ARC.md A8.b: body now delegates to
/// `parallel_queue_dispatch` — the shared scaffolding that was
/// duplicated across all 5 `n_parallel_queue*` fns.
fn n_parallel_queue(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Int);
}

/// Plan-06 ARC.md A5 — `Stitch::Reduce` runtime entry.
///
/// V1 stack layout (LIFO pop): `n_extra` (i64), N extras (i64), threads
/// (i32), fold_fn_d_nr (i32), init (i64), input (DbRef).
///
/// `fold_fn` must be declared as `fn fold(acc: integer, row: integer) ->
/// integer`; the runtime invokes it once per row to update each worker's
/// accumulator, then combines per-worker partials with the same fn.
/// Restricts to `vector<integer>` input — see ARC.md A5 for the future
/// extension story.
fn n_parallel_fold(stores: &mut Stores, stack: &mut DbRef) {
    let n_extra = stores.get::<i64>(stack) as usize;
    let mut extra_args: Vec<u64> = Vec::with_capacity(n_extra);
    for _ in 0..n_extra {
        extra_args.push(stores.get::<i64>(stack) as u64);
    }
    extra_args.reverse();

    let v_threads = stores.get::<i64>(stack) as i32;
    let v_func = stores.get::<i64>(stack) as i32;
    let v_init = stores.get::<i64>(stack);
    let v_input = stores.get::<DbRef>(stack);

    let (fn_pos, program, element_size) = {
        let ctx = stores
            .parallel_ctx
            .as_ref()
            .expect("parallel_fold called outside State::execute()");
        let data = ctx.data();
        assert!(
            v_func >= 0,
            "parallel_fold: invalid function reference {v_func}"
        );
        let d_nr = v_func as u32;
        let fn_pos = data.def(d_nr).code_position;
        let bytecode = unsafe { Arc::clone(&*ctx.bytecode) };
        let library = unsafe { Arc::clone(&*ctx.library) };
        // V1: input is vector<integer>; element_size = 8.  Future
        // versions might derive this from the worker's row arg type.
        let elem_size = 8u32;
        (
            fn_pos,
            WorkerProgram {
                bytecode,
                library,
                stack_trace_lib_nr: ctx.stack_trace_lib_nr,
                data_ptr: ctx.data.clone(),
                fn_positions: Arc::new(data.definitions.iter().map(|d| d.code_position).collect()),
                const_refs: Arc::clone(&ctx.const_refs),
                frame_headroom: Arc::clone(&ctx.frame_headroom),
                line_numbers: Arc::new(std::collections::BTreeMap::new()),
            },
            elem_size,
        )
    };

    let n_threads = (v_threads as usize).max(1);
    let result = crate::parallel::run_parallel_fold(
        stores,
        program,
        fn_pos,
        &v_input,
        element_size,
        v_init,
        n_threads,
        &extra_args,
    );
    stores.put(stack, result);
}

/// Plan-06 spine step 8a — read one element from the active par buffer.
///
/// Pops `idx` (i64), reads `stores.par_buffer_stack.last()[idx]`, and
/// pushes the i64 value onto the operand stack.  The caller is
/// responsible for masking to the worker's actual return width when
/// less than 8 bytes — for narrow primitives the high bits are zero
/// (workers store via `to_le_bytes` into a u64 slot).
///
/// Panics if `par_buffer_stack` is empty (no active queue) or if
/// `idx` is out of range — both indicate a parser-side bug.
fn n_parallel_buf_get(stores: &mut Stores, stack: &mut DbRef) {
    let idx = stores.get::<i64>(stack);
    let buf = stores
        .par_buffer_stack
        .last()
        .expect("parallel_buf_get: par_buffer_stack is empty");
    let val = buf[idx as usize] as i64;
    stores.put(stack, val);
}

/// Plan-06 spine step 8a — pop the active par buffer.  Called after
/// the body loop completes.  Panics if the stack is already empty
/// (parser-side bug).  Void return.
fn n_parallel_buf_drop(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_buffer_stack
        .pop()
        .expect("parallel_buf_drop: par_buffer_stack is already empty");
}

/// Plan-06 spine step 8c — text-return Queue runtime entry.  Same
/// arg layout as `n_parallel_queue`, but workers return owned
/// `String`s collected via `run_parallel_text` (which routes through
/// the per-worker output Store slot machinery — same path text
/// returns always use inside loft).  The resulting `Vec<String>` is
/// pushed onto `stores.par_text_buffer_stack`; the row count is
/// pushed onto the operand stack so the caller can use it as a
/// loop bound.
///
/// Step 8c's parser rewrite extends the gate to text returns so
/// fused for-par over text-returning workers no longer allocates a
/// heap text-vector.  Reads use `n_parallel_buf_get_text` (clones
/// into scratch following the standard text-return convention).
fn n_parallel_queue_text(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Text);
}

/// @PLN10 Phase 1 — destination-passing variant of `n_parallel_buf_get_text`.
/// Always-non-null (clones an owned `String` from the par text buffer).
/// Routed by `is_text_dest_native`.
fn n_parallel_buf_get_text_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let idx = stores.get::<i64>(stack);
    let s_owned = {
        let buf = stores
            .par_text_buffer_stack
            .last()
            .expect("parallel_buf_get_text: par_text_buffer_stack is empty");
        buf[idx as usize].clone()
    };
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&s_owned);
}

/// Plan-06 spine step 8c — pop the active par text buffer.  Called
/// after the body loop completes.  Panics if the stack is already
/// empty (parser-side bug).  Void return.
fn n_parallel_buf_drop_text(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_text_buffer_stack
        .pop()
        .expect("parallel_buf_drop_text: par_text_buffer_stack is already empty");
}

/// Plan-06 spine step 8d.1 — reference-return Queue runtime entry.
/// Same arg layout as `n_parallel_queue` but workers return a
/// `DbRef` into their own output stores.  `run_parallel_queue_ref`
/// (8d.0) adopts each worker's output stores into the parent's
/// allocations table and rebases the per-row `DbRef`s into the
/// parent's namespace via `Stores::adopt_worker_excess` +
/// `rebase_walk_record`.  The resulting `(refs, adopted_stores)`
/// pair is pushed onto `stores.par_ref_buffer_stack`; the row count
/// is pushed onto the operand stack so the caller can use it as a
/// loop bound.
///
/// Step 8d.2 wires this through the parser; until then this is
/// exercised only by Rust unit tests in `tests/threading.rs`.
fn n_parallel_queue_ref(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Ref);
}

/// Plan-06 spine step 8d.1 — read one rebased `DbRef` from the
/// active par-ref buffer.  Pops `idx` (i64), reads
/// `par_ref_buffer_stack.last().0[idx]`, pushes the `DbRef` (12
/// bytes) onto the operand stack.
///
/// Panics if `par_ref_buffer_stack` is empty (no active queue) or
/// `idx` is out of range — both indicate a parser-side bug.
fn n_parallel_buf_get_ref(stores: &mut Stores, stack: &mut DbRef) {
    let idx = stores.get::<i64>(stack);
    let r = {
        let buf = stores
            .par_ref_buffer_stack
            .last()
            .expect("parallel_buf_get_ref: par_ref_buffer_stack is empty");
        buf.0[idx as usize]
    };
    stores.put(stack, r);
}

/// Plan-06 spine step 8d.1 — pop the active par-ref buffer and free
/// every adopted store.  Called after the body loop completes.
/// Panics if the stack is already empty (parser-side bug).
///
/// Each adopted store_nr is freed via `Stores::free_named` with a
/// synthetic `DbRef` (`store_nr, rec=0, pos=0`) so the standard
/// alloc-free instrumentation logs the release.
fn n_parallel_buf_drop_ref(stores: &mut Stores, _stack: &mut DbRef) {
    let (_refs, adopted) = stores
        .par_ref_buffer_stack
        .pop()
        .expect("parallel_buf_drop_ref: par_ref_buffer_stack is already empty");
    for store_nr in adopted {
        let synthetic = DbRef {
            store_nr,
            rec: 0,
            pos: 0,
        };
        stores.free_named(&synthetic, "par_buf_drop_ref");
    }
}

/// Plan-06 ARC.md A3 — narrow-primitive Queue runtime entry.  Same
/// arg layout as `n_parallel_queue`; workers return a 1, 2, or 4-byte
/// value.  Internally reuses `run_parallel_queue`'s `Vec<u64>`
/// scratch buffer, then packs each row to its declared `return_size`
/// stride and pushes `(bytes, stride)` onto
/// `par_narrow_buffer_stack`.  The packed buffer saves memory on
/// large narrow-return workloads (~7× for 1-byte returns).
///
/// Reads use `n_parallel_buf_get_narrow(idx, return_size, signed)`.
fn n_parallel_queue_narrow(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Narrow);
}

/// Plan-06 ARC.md A3 — read one narrow row from the active narrow
/// buffer.  Pops `(idx, return_size, signed)` (i64s; `signed` is
/// 0/1).  Reads `return_size` little-endian bytes at offset
/// `idx * return_size` from `par_narrow_buffer_stack.last().0`,
/// sign-extending if `signed != 0`, and pushes the result as i64.
///
/// `return_size` is checked against the stored stride — a mismatch
/// is a parser-side bug.
fn n_parallel_buf_get_narrow(stores: &mut Stores, stack: &mut DbRef) {
    let signed = stores.get::<i64>(stack) != 0;
    let return_size = stores.get::<i64>(stack) as usize;
    let idx = stores.get::<i64>(stack) as usize;

    let val: i64 = {
        let entry = stores
            .par_narrow_buffer_stack
            .last()
            .expect("parallel_buf_get_narrow: par_narrow_buffer_stack is empty");
        let stride = entry.1 as usize;
        debug_assert_eq!(
            stride, return_size,
            "parallel_buf_get_narrow: stride/return_size mismatch (stride={stride}, arg={return_size})"
        );
        let off = idx * stride;
        let bytes = &entry.0;
        match (stride, signed) {
            (1, false) => i64::from(bytes[off]),
            (1, true) => i64::from(bytes[off] as i8),
            (2, false) => i64::from(u16::from_le_bytes([bytes[off], bytes[off + 1]])),
            (2, true) => i64::from(i16::from_le_bytes([bytes[off], bytes[off + 1]])),
            (4, false) => i64::from(u32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ])),
            (4, true) => i64::from(i32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ])),
            _ => panic!("parallel_buf_get_narrow: invalid stride {stride}"),
        }
    };
    stores.put(stack, val);
}

/// Plan-06 ARC.md A3 — pop the active narrow-queue buffer.  Called
/// after the body loop completes.  Panics if the stack is already
/// empty (parser-side bug).  Void return.
fn n_parallel_buf_drop_narrow(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_narrow_buffer_stack
        .pop()
        .expect("parallel_buf_drop_narrow: par_narrow_buffer_stack is already empty");
}

/// Plan-06 ARC.md A3.6 — typed reader for `single` (f32) returns
/// from `n_parallel_queue_narrow` (stride 4).  Reads the same
/// per-row bytes as `n_parallel_buf_get_narrow` but interprets them
/// as a 4-byte f32 bit pattern via `f32::from_bits` instead of
/// returning an i64 that the caller would then have to bit-cast.
///
/// Justified by symmetry with `Store::set_single` /
/// `Store::get_single` in `src/store.rs:1406-1421`: those write /
/// read f32 as a typed-pointer memcpy at a slot.  Worker functions
/// returning `single` populate their return slot via the same
/// typed pointer write, so the slot bytes ARE the f32 bit pattern.
/// `execute_at_raw` reads those bytes as u64 (low 4 = f32 bits);
/// `n_parallel_queue_narrow` truncates to stride 4 — preserving the
/// f32 bit pattern.  This getter recovers the typed value with no
/// intermediate IR Op required.
///
/// Pushes the value as f32 onto the operand stack — `Type::Single`
/// has slot size 4 (per `variables::size`), matching `f32`'s width.
/// Mirrors how `OpConvSingleFromInt` returns f32 (`src/ops.rs:502`).
fn n_parallel_buf_get_single(stores: &mut Stores, stack: &mut DbRef) {
    let idx = stores.get::<i64>(stack) as usize;
    let val: f32 = {
        let entry = stores
            .par_narrow_buffer_stack
            .last()
            .expect("parallel_buf_get_single: par_narrow_buffer_stack is empty");
        let stride = entry.1 as usize;
        debug_assert_eq!(
            stride, 4,
            "parallel_buf_get_single: stride must be 4 (got {stride})"
        );
        let off = idx * stride;
        let bytes = &entry.0;
        let raw = u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        f32::from_bits(raw)
    };
    stores.put(stack, val);
}

/// Plan-06 ARC.md A3.6 — typed reader for `float` (f64) returns
/// from `n_parallel_queue` (stride 8 / Vec<u64> rows).  Reads the
/// same per-row u64 as `n_parallel_buf_get` but interprets the bits
/// as f64 via `f64::from_bits` instead of returning a raw i64.
///
/// Same justification as `n_parallel_buf_get_single`: worker
/// functions returning `float` populate their return slot via a
/// typed memcpy (`Store::set_float`); `execute_at_raw` reads the
/// slot as u64 — the bytes ARE the f64 bit pattern.  This getter
/// recovers the typed value with no intermediate IR Op.
fn n_parallel_buf_get_float(stores: &mut Stores, stack: &mut DbRef) {
    let idx = stores.get::<i64>(stack) as usize;
    let row = stores
        .par_buffer_stack
        .last()
        .expect("parallel_buf_get_float: par_buffer_stack is empty")[idx];
    let val = f64::from_bits(row);
    stores.put(stack, val);
}

/// Plan-06 ARC.md A6.b — fn-ref-return Queue runtime entry.  Same
/// arg layout as `n_parallel_queue` but workers return 20-byte
/// fn-ref blobs (8B i64 d_nr + 12B closure DbRef per Rust's
/// reordered layout).  `run_parallel_queue_fn` writes each row's
/// 20 bytes directly into a packed `Vec<u8>` via
/// `State::execute_at_raw_to`, bypassing both the truncating light
/// path (L1) and the body's `get_field`-as-i32 misread (L2).
///
/// The packed buffer is pushed onto `par_fn_buffer_stack`; readers
/// pull 20 bytes per row via `n_parallel_buf_get_fn`.  Reads are
/// inserted at the top of each fused-for-par iteration as
/// `Set(b_var, Call(buf_get_fn, [idx]))` so the body's
/// `CallRef(b_var, ...)` reads `b_var`'s 20-byte slot directly.
fn n_parallel_queue_fn(stores: &mut Stores, stack: &mut DbRef) {
    parallel_queue_dispatch(stores, stack, QueueStitch::Fn);
}

/// Plan-06 ARC.md A6.b — read one 20-byte fn-ref blob from the
/// active fn-buffer.  Pops `idx` (i64), reads 20 bytes at offset
/// `idx * 20` from `par_fn_buffer_stack.last()`, and pushes them
/// onto the operand stack as a fn-ref blob (matches the on-stack
/// representation `OpVarFnRef` and `OpPutFnRef` operate on:
/// `[u8; 20]`).
///
/// Declared as `-> integer` in stdlib (since loft's type system
/// can't express a generic-fn-typed return), but the parser
/// substitutes the call site's actual fn-ref type so codegen
/// allocates a 20-byte slot for `b_var`.  The 20-byte stack push
/// happens here at runtime regardless of the declared return type.
///
/// Panics if `par_fn_buffer_stack` is empty (no active queue) or
/// if `idx` is out of range — both indicate a parser-side bug.
fn n_parallel_buf_get_fn(stores: &mut Stores, stack: &mut DbRef) {
    let idx = stores.get::<i64>(stack);
    let bytes_20: [u8; 20] = {
        let buf = stores
            .par_fn_buffer_stack
            .last()
            .expect("parallel_buf_get_fn: par_fn_buffer_stack is empty");
        let off = (idx as usize) * 20;
        let mut arr = [0u8; 20];
        arr.copy_from_slice(&buf[off..off + 20]);
        arr
    };
    stores.put(stack, bytes_20);
}

/// Plan-06 ARC.md A6.b — pop the active fn-queue buffer.  Called
/// after the body loop completes.  Panics if the stack is already
/// empty (parser-side bug).  Void return.
fn n_parallel_buf_drop_fn(stores: &mut Stores, _stack: &mut DbRef) {
    stores
        .par_fn_buffer_stack
        .pop()
        .expect("parallel_buf_drop_fn: par_fn_buffer_stack is already empty");
}
