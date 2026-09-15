// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Plan 09 phase 03 — parallel-for emitter family.
//!
//! Replaces the 95-line special case in
//! `src/generation/dispatch.rs` (the `"n_parallel_for" |
//! "n_parallel_for_light" =>` arm) with a per-Op `OpEmitter` impl.
//!
//! What the emission does (unchanged from the pre-phase-03 special
//! case):
//!
//! 1. Pull the worker fn def out of `vals[4]` (an i32 d_nr literal)
//!    and read its return type.
//! 2. Pick which native helper to call (`n_parallel_for_native`,
//!    `n_parallel_for_ref_native`, or `n_parallel_for_text_native`)
//!    based on whether the worker returns text, heap-typed, or scalar.
//! 3. Synthesise extra-arg `let _ex0 = …;` bindings so the closure
//!    can capture them.
//! 4. Emit the parallel-helper call with a closure of the correct
//!    shape (text / heap-ref / float / scalar).
//! 5. Close N braces matching the let-bindings.
//!
//! The emitter takes over because of phase 00 step 0.6's registry-
//! first guard at the top of `output_call_inner`: when this emitter
//! is registered for `n_parallel_for` / `n_parallel_for_light`,
//! dispatch routes here BEFORE the legacy match arm runs.  Phase 03
//! deletes the legacy match arm; this file becomes the sole
//! emission site for parallel-for codegen.
//!
//! Phase 06 (P202 — threading queue runtime fns) extends this
//! family with `n_parallel_queue*` emitters that reuse the helper
//! functions defined here.

use super::{EmitCtx, OpEmitter};
use crate::data::{Type, Value};
use std::io;

/// Closure shape for the worker — determines the conversion applied
/// to the worker's return value before it's stored in the result vec.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ClosureShape {
    /// Worker returns `String`; closure synthesises a write buffer.
    Text,
    /// Worker returns `DbRef` (heap struct / struct-enum / vector / etc.).
    HeapRef,
    /// Worker returns `f64` / `f32`; closure converts via `to_bits() as i64`.
    Float,
    /// Worker returns scalar integer / boolean; closure casts `as i64`.
    Scalar,
    /// Worker returns a function reference (`(u32, DbRef)` native tuple);
    /// closure returns it verbatim into the fn-ref Queue buffer (#281).
    Fn,
}

fn closure_shape(ret: &Type) -> ClosureShape {
    if matches!(ret, Type::Text(_)) {
        ClosureShape::Text
    } else if matches!(ret, Type::Function(..)) {
        ClosureShape::Fn
    } else if is_ref_return(ret) {
        ClosureShape::HeapRef
    } else if matches!(ret, Type::Float | Type::Single) {
        ClosureShape::Float
    } else {
        ClosureShape::Scalar
    }
}

/// True when the worker's return value is delivered as a `DbRef` that the ref
/// path deep-copies (`n_parallel_queue_ref_native`).  Covers named heap defs
/// (struct `Reference` / struct-`Enum`) and `Vector` — the cases `heap_ref_kt`
/// can size.  A vector-returning worker was previously mis-routed to the scalar
/// path, which cast `DbRef as i64` (E0605, the native-refreturn bug).
///
/// Keyed collections (`Sorted`/`Hash`/`Index`/`Radix`) are also DbRef returns
/// (the parser groups them with `return_size = -1`) but have no `main_<kind><T>`
/// storage wrapper for `heap_ref_kt` to size, so they stay off this path for now
/// — a worker returning one still fails to compile loudly rather than copying at
/// the wrong stride.  Extending `heap_ref_kt` to them is the follow-up.
fn is_ref_return(ret: &Type) -> bool {
    ret.heap_def_nr().is_some() || matches!(ret, Type::Vector(_, _))
}

/// The `(struct_size, known_type)` a `HeapRef` worker return is copied as by
/// `n_parallel_queue_ref_native` / `n_parallel_for_ref_native`: the storage type
/// of the value the worker returns.
///
/// - Named heap defs (struct `Reference`, struct-`Enum`): the def's own
///   `known_type`, sized directly.
/// - `Vector<T>`: the `main_vector<T>` wrapper struct the worker actually
///   allocates (its `vector` field at offset 0 holds the data).  The wrapper is
///   registered during parsing — the worker built one — so this read-only
///   by-name lookup always resolves at codegen time.
///
/// Returns `(0, 0)` only for a type `is_ref_return` never admits (a defensive
/// default; a `(0, 0)` stride would make the runtime copy nothing).
fn heap_ref_kt(ctx: &EmitCtx<'_, '_>, ret: &Type) -> (i32, i32) {
    let data = ctx.output.data;
    let kt = if let Some(d_nr) = ret.heap_def_nr() {
        data.def(d_nr).known_type()
    } else if let Type::Vector(elem, _) = ret {
        // Mirror `Data::vector_def`'s wrapper naming exactly.
        let wrapper = data.def_nr(&format!("main_vector<{}>", elem.name(data)));
        if wrapper == u32::MAX {
            return (0, 0);
        }
        data.def(wrapper).known_type()
    } else {
        return (0, 0);
    };
    (i32::from(ctx.output.stores.size(kt)), i32::from(kt))
}

fn helper_name(shape: ClosureShape) -> &'static str {
    match shape {
        ClosureShape::Text => "n_parallel_for_text_native",
        ClosureShape::HeapRef => "n_parallel_for_ref_native",
        // Float and Scalar both use the primitive helper — they
        // differ only in the closure body's return-value transform.
        ClosureShape::Float | ClosureShape::Scalar => "n_parallel_for_native",
        // fn-ref returns always route through the Queue family (the parser
        // emits `n_parallel_queue_fn`), so the for-loop helper is never
        // selected for them; map it to the Queue entry for totality.
        ClosureShape::Fn => "n_parallel_queue_fn_native",
    }
}

/// Phase 06 — runtime helper-name selection for the queue family.
/// Mirrors `helper_name` but maps to the `n_parallel_queue_*_native`
/// fns introduced in plan-09 phase 06 (P202 close).
fn queue_helper_name(shape: ClosureShape) -> &'static str {
    match shape {
        ClosureShape::Text => "n_parallel_queue_text_native",
        ClosureShape::HeapRef => "n_parallel_queue_ref_native",
        ClosureShape::Fn => "n_parallel_queue_fn_native",
        ClosureShape::Float | ClosureShape::Scalar => "n_parallel_queue_native",
    }
}

/// Plan-06 ARC.md A3 — narrow-Integer queue helper name.  Narrow
/// integer returns (byte_width 1/2/4) need the byte-packing variant
/// instead of the wide `Vec<u64>` queue.
fn queue_narrow_helper_name() -> &'static str {
    "n_parallel_queue_narrow_native"
}

/// True when the worker's return type rides the narrow-Queue path
/// (byte-packed buffer, stride 1/2/4).  Mirrors the parser-side
/// `narrow_route_for` decision in `src/parser/collections.rs`:
///
/// - `Integer(spec)` with `byte_width 1/2/4` (A3 narrow Integer)
/// - `Boolean` (A3.5)
/// - `Character` (A3.5)
/// - `Enum(_, false, _)` no-payload (A3.5)
/// - `Single` (A3.6) — f32 fits stride 4 with bit-pattern preserved
///
/// Used by `ParallelQueueEmitter` to swap the runtime helper to the
/// narrow variant `n_parallel_queue_narrow_native`.  Without this,
/// the parser routes via `n_parallel_queue_narrow` (narrow buffer)
/// while the emitter would call `n_parallel_queue_native` (wide
/// buffer) — body's `parallel_buf_get_narrow` then reads from an
/// empty narrow buffer and panics.
fn is_narrow_int_return(ret: &Type) -> bool {
    match ret {
        Type::Integer(spec) => matches!(spec.byte_width(true), 1 | 2 | 4),
        Type::Boolean | Type::Character | Type::Single => true,
        Type::Enum(_, false, _) => true,
        _ => false,
    }
}

/// Emit the Rust expression that reads one by-value tuple element out of the
/// store record `elm` at byte offset `off`.  Mirrors the scalar read helpers
/// the interpreter uses for tuple-typed worker arguments; `_ts` is the
/// `&Store` bound by [`tuple_arg_prep`].
fn tuple_elem_read(t: &Type, off: usize) -> String {
    let off = off as u32;
    match t {
        // @PLN114 — read the element at its STORED width.  `get_int` reads 8 bytes,
        // which is right only for a full-width integer; a `u8` element occupies one
        // byte and a `u16` two, so a blanket `get_int` reads its neighbours too.
        // Width and encoding come from the same home the field ops use.
        Type::Integer(spec) => {
            let width = spec.byte_width(false);
            let min = spec.min;
            match width {
                1 => format!("(_ts.get_byte(elm.rec, elm.pos + {off}, {min}) as i64)"),
                2 => format!("(_ts.get_short_full(elm.rec, elm.pos + {off}, {min}) as i64)"),
                4 => format!("(_ts.get_i32_raw(elm.rec, elm.pos + {off}) as i64)"),
                _ => format!("_ts.get_int(elm.rec, elm.pos + {off})"),
            }
        }
        Type::Character | Type::Null => format!("_ts.get_i32_raw(elm.rec, elm.pos + {off})"),
        // @PLN17: boolean tuple slot is u8 (storage form); get_boolean returns bool.
        Type::Boolean => format!("(_ts.get_boolean(elm.rec, elm.pos + {off}, 1) as u8)"),
        Type::Enum(_, false, _) => format!(
            "{{ let r = _ts.get_byte(elm.rec, elm.pos + {off}, 0); if r < 0 {{ 255u8 }} else {{ r as u8 }} }}"
        ),
        Type::Single => format!("_ts.get_single(elm.rec, elm.pos + {off})"),
        Type::Float => format!("_ts.get_float(elm.rec, elm.pos + {off})"),
        // A text element is stored as a 4-byte heap pointer and the worker's slot is a
        // `&str`, so the pointer has to be resolved through the same store the rest of
        // the row is read from — the native twin of `parallel::read_text_at`.
        //
        // This arm's absence became a BACKEND DIVERGENCE the moment loft#1055 taught the
        // interpreter to marshal `vector<(text, integer)>` correctly: the same program
        // then ran on `--interpret` and was refused here.  A divergence introduced by a
        // fix is worse than one that always existed, because it reads as the fix being
        // complete.
        Type::Text(_) => {
            format!("_ts.get_str(_ts.get_u32_raw(elm.rec, elm.pos + {off}))")
        }
        other => format!(
            "compile_error!(\"par tuple worker: unsupported by-value element type {other:?}\")"
        ),
    }
}

/// When the par worker's element parameter is a tuple passed by value, the
/// native closure receives a `DbRef` (`elm`) but the worker fn expects the
/// unpacked tuple.  Returns `(prep, arg)`: `prep` is the closure-body prelude
/// that materialises a Rust tuple `_p` from the record's fields, and `arg` is
/// the expression passed to the worker.  Non-tuple workers return
/// `("", "elm")` — byte-identical to the original single-`DbRef` path.
fn tuple_arg_prep(ctx: &EmitCtx<'_, '_>, fn_d_nr: u32, elem_size: i32) -> (String, &'static str) {
    let worker_def = ctx.output.data.def(fn_d_nr);
    let Some(elem_attr) = worker_def.attributes().first() else {
        return (String::new(), "elm");
    };
    if let Type::Tuple(elems) = &elem_attr.typedef {
        // @PLN114 — the element rows are STORAGE, so offsets come from the storage
        // view.  Using the stack view made a `par` over `vector<(u8,u16)>` read the
        // row at the wrong stride (the interpreter twin had the same defect).
        let offsets = crate::data::element_storage_offsets(elems);
        let reads: Vec<String> = elems
            .iter()
            .zip(offsets.iter())
            .map(|(t, off)| tuple_elem_read(t, *off))
            .collect();
        let prep = format!(
            "let _ts = unsafe {{ &*cell.get() }}.store(&elm); let _p = ({},); ",
            reads.join(", ")
        );
        return (prep, "_p");
    }
    // A fn-ref worker parameter (`fn(f: fn(integer) -> integer)` over a
    // `vector<fn-ref>`): the native fn-ref value is a `(u32, DbRef)` tuple
    // (fn-index, closure), but the queue hands the closure the element `DbRef`.
    // Vector-stored fn-refs are non-capturing — only the `i32` fn-index is
    // stored (offset 0) — so read it, widen to `u32`, and pair with a NULL
    // closure.  Mirrors the working for-loop unpack
    // (`tests/generated/issues_p4d_a2_vector_fn_ref_for_loop.rs`).
    if matches!(elem_attr.typedef, Type::Function(..)) {
        let prep = "let _ts = unsafe { &*cell.get() }.store(&elm); \
                    let _p = (_ts.get_i32_raw(elm.rec, elm.pos) as u32, DbRef::NULL); "
            .to_string();
        return (prep, "_p");
    }
    // A by-value scalar worker parameter (e.g. `fn(x: integer)` over a
    // `vector<integer>` / range): the queue hands the closure a `DbRef` into
    // the element record, but the worker wants the value.  Read it out — the
    // 1-element version of the tuple path.  Reference / struct / heap-enum /
    // text workers take the `DbRef` directly, so they keep the bare `elm`.
    if is_by_value_scalar(&elem_attr.typedef) {
        // An `Integer` element is read at the vector's STRIDE width (`elem_size`),
        // not the worker param's width — they differ when a narrow element
        // (`vector<u8>`/`u16`/`i32`) widens to an `integer` param.  Read raw +
        // zero-extend to `i64` (the worker's arg-context type), matching the
        // interpreter's `read_primitive_at`.  Other scalar kinds (bool / char /
        // enum / single / float) have a fixed, type-determined width.
        let read = if matches!(&elem_attr.typedef, Type::Integer(_)) {
            match elem_size {
                1 => "i64::from(_ts.get_byte(elm.rec, elm.pos, 0))".to_string(),
                4 => "(_ts.get_i32_raw(elm.rec, elm.pos) as u32) as i64".to_string(),
                _ => "_ts.get_int(elm.rec, elm.pos)".to_string(),
            }
        } else {
            tuple_elem_read(&elem_attr.typedef, 0)
        };
        let prep = format!("let _ts = unsafe {{ &*cell.get() }}.store(&elm); let _p = {read}; ");
        return (prep, "_p");
    }
    // A text worker parameter (`fn(s: text)` over a `vector<text>` / text input):
    // the closure receives the element `DbRef`, but the worker wants `&str`.
    // Read the row's text into an owned String and pass it by reference — the
    // expression is constant (uses only the closure's `cell`/`elm`), so it rides
    // as the `arg` directly with no `prep`.
    if matches!(elem_attr.typedef, Type::Text(_)) {
        return (
            String::new(),
            "&loft::codegen_runtime::par_read_text_input(cell, elm)",
        );
    }
    (String::new(), "elm")
}

/// True for worker-parameter types the par closure must read out of the
/// element record by value (the scalar kinds `tuple_elem_read` handles).
/// Reference / heap-enum / struct / text parameters instead receive the
/// element `DbRef` directly, so they are excluded here.
fn is_by_value_scalar(t: &Type) -> bool {
    matches!(
        t,
        Type::Integer(_)
            | Type::Character
            | Type::Null
            | Type::Boolean
            | Type::Enum(_, false, _)
            | Type::Single
            | Type::Float
    )
}

/// `n_parallel_for` / `n_parallel_for_light` emitter.
///
/// Lifts the legacy match-arm body verbatim into a custom emitter
/// while producing byte-identical output.
pub struct ParallelForEmitter;

impl OpEmitter for ParallelForEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Guard: the special case requires at least 5 args with vals[4]
        // a non-negative i32 (the worker fn's def_nr).  When violated,
        // fall through to the default emitter — preserves pre-phase-03
        // behaviour where the special case `if` clause didn't match.
        let fn_d_nr = match args.get(4) {
            Some(Value::Int(d)) if *d >= 0 => (*d).cast_unsigned(),
            _ => {
                return super::default::DefaultEmitter.emit(ctx, args);
            }
        };
        if args.len() < 5 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        let worker_def = ctx.output.data.def(fn_d_nr);
        let worker_name = worker_def.name().to_string();
        let worker_ret = worker_def.returned().clone();
        let shape = closure_shape(&worker_ret);
        // A literal / bufferless text worker (@P205 nwb) returns an owned `String`
        // and takes NO `&mut String` work-buffer param — so its closure must NOT
        // pass one (else E0061).  Computed as a bool now to avoid holding the
        // `worker_def` borrow across the `ctx.emit` calls below.
        let owned_text = crate::generation::returns_owned_string(worker_def);

        // Extra context args: vals[5..len-1].  The trailing element
        // is `n_extra` (the count); we don't read it directly because
        // arg arithmetic gives us the same number.
        let n_extra = if args.len() > 6 { args.len() - 6 } else { 0 };

        // Emit let-bindings for extra args so they're captured by name
        // (and not re-evaluated) inside the closure.  Each binding
        // wraps the rest of the emission in a `{ … }` block; the
        // matching `}` characters are emitted at the end.
        for i in 0..n_extra {
            write!(ctx.w, "{{ let _ex{i} = ")?;
            ctx.emit(&args[5 + i])?;
            write!(ctx.w, "; ")?;
        }

        // Helper call: `name(cell, input, elem_size, …, threads`
        let par_fn = helper_name(shape);
        write!(ctx.w, "{par_fn}(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ", ")?;
        if shape == ClosureShape::HeapRef {
            // Ref mode: emit struct_size and known_type instead of
            // return_size — the storage type the runtime deep-copies each
            // worker result as (struct/enum def, or a vector's wrapper).
            let (struct_size, known_type) = heap_ref_kt(ctx, &worker_ret);
            write!(ctx.w, "{struct_size}, {known_type}, ")?;
        } else {
            ctx.emit_i32_slot(&args[2])?;
            write!(ctx.w, ", ")?;
        }
        ctx.emit_i32_slot(&args[3])?;

        // Closure args: `, _ex0, _ex1, …` appended after `elm`.
        let extras = {
            use std::fmt::Write as _;
            let mut s = String::new();
            for i in 0..n_extra {
                write!(s, ", _ex{i}").unwrap();
            }
            s
        };

        // Closure body — return-shape-specific.  P199 — worker
        // closures receive `&UnsafeCell<Stores>` from the parallel-
        // runner helpers; user-fn calls take `cell`, so the closure
        // parameter is named `cell` and threaded through verbatim.
        // `prep`/`arg` unpack a by-value tuple element from the record
        // (empty/`elm` for the common single-`DbRef` worker).
        // args[1] is the vector element stride (a literal Int), needed to read a
        // narrow scalar element at its true width.
        let elem_sz = match args.get(1).map(Value::unspan) {
            Some(Value::Int(s)) => *s,
            _ => 8,
        };
        let (prep, arg) = tuple_arg_prep(ctx, fn_d_nr, elem_sz);
        // @PLAN59: hidden heap DESTINATION attrs (ref_return promotion /
        // the signature-time `__retbuf`) — allocate one backing store per
        // dest inside the worker closure and pass it in attr order (dests
        // are the trailing attrs).  Classified by TYPE, mirroring
        // `native_gate::classify_bridge_attr`; workers without dests emit
        // nothing (today's literal-returning corpus is byte-identical).
        let (prep, dests) = {
            let mut prep = prep;
            let mut dests = String::new();
            use std::fmt::Write as _;
            for (i, a) in ctx.output.data.def(fn_d_nr).attributes().iter().enumerate() {
                if a.hidden
                    && matches!(
                        a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    write!(
                        prep,
                        "let _pd{i}: DbRef = unsafe {{ &mut *cell.get() }}.database(100); "
                    )
                    .unwrap();
                    write!(dests, ", _pd{i}").unwrap();
                }
            }
            (prep, dests)
        };
        match shape {
            ClosureShape::Text if owned_text => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) }})"
            )?,
            ClosureShape::Text => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}let mut _w = String::new(); {worker_name}(cell, {arg}{extras}{dests}, &mut _w); _w }})"
            )?,
            ClosureShape::HeapRef => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) }})"
            )?,
            // #281 — fn-ref return: the worker yields the native fn-ref tuple
            // `(u32, DbRef)` directly; the closure returns it verbatim.
            ClosureShape::Fn => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) }})"
            )?,
            ClosureShape::Float => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}).to_bits() as i64 }})"
            )?,
            ClosureShape::Scalar => {
                // Plan-06 ARC.md A3.5 — Boolean returns map to Rust
                // `bool`, which cannot cast directly to i64; insert
                // an `as u8` bridge.  Other Scalar shapes (Integer
                // narrow / wide, Character → i32, Enum-no-payload →
                // u8) all support `as i64` natively.
                if matches!(worker_ret.base(), Type::Boolean) {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) as u8 as i64 }})"
                    )?;
                } else {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) as i64 }})"
                    )?;
                }
            }
        }

        // Close the `{ let _ex0 = … ;` blocks opened above.
        for _ in 0..n_extra {
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// Phase 06 — `n_parallel_queue` / `_text` / `_ref` emitter family.
///
/// Closes P202 (native missing `n_parallel_queue` family).  Mirrors
/// `ParallelForEmitter`'s closure-shape logic but routes calls to
/// `n_parallel_queue_*_native` runtime fns (defined in
/// `src/codegen_runtime.rs`).  Queue variants return `i64` (row count)
/// instead of `DbRef`; the result feeds a fused for-loop that reads
/// rows via `n_parallel_buf_get_*_native`.
///
/// The arg layout is identical to `n_parallel_for` (input, elem_size,
/// return_size, threads, func, extras..., n_extra) so the emitter
/// reuses the same parsing + closure-emission scaffolding.
pub struct ParallelQueueEmitter;

impl OpEmitter for ParallelQueueEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Guard: same as for-par — at least 5 args with vals[4] a
        // non-negative i32 (worker fn's def_nr).  Otherwise fall through.
        let fn_d_nr = match args.get(4) {
            Some(Value::Int(d)) if *d >= 0 => (*d).cast_unsigned(),
            _ => {
                return super::default::DefaultEmitter.emit(ctx, args);
            }
        };
        if args.len() < 5 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        let worker_def = ctx.output.data.def(fn_d_nr);
        let worker_name = worker_def.name().to_string();
        let worker_ret = worker_def.returned().clone();
        let shape = closure_shape(&worker_ret);
        // A literal / bufferless text worker (@P205 nwb) returns an owned `String`
        // and takes NO `&mut String` work-buffer param — so its closure must NOT
        // pass one (else E0061).  Computed as a bool now to avoid holding the
        // `worker_def` borrow across the `ctx.emit` calls below.
        let owned_text = crate::generation::returns_owned_string(worker_def);

        // Extras: args[5..len-1]; trailing args[len-1] is the n_extra count.
        let n_extra = if args.len() > 6 { args.len() - 6 } else { 0 };

        for i in 0..n_extra {
            write!(ctx.w, "{{ let _ex{i} = ")?;
            ctx.emit(&args[5 + i])?;
            write!(ctx.w, "; ")?;
        }

        // Plan-06 ARC.md A3 — narrow-Integer returns route through
        // `n_parallel_queue_narrow_native` (byte-packed buffer);
        // wide / non-Integer scalars stay on `n_parallel_queue_native`.
        let par_fn = if is_narrow_int_return(&worker_ret) {
            queue_narrow_helper_name()
        } else {
            queue_helper_name(shape)
        };
        write!(ctx.w, "{par_fn}(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ", ")?;
        if shape == ClosureShape::HeapRef {
            let (struct_size, known_type) = heap_ref_kt(ctx, &worker_ret);
            write!(ctx.w, "{struct_size}, {known_type}, ")?;
        } else {
            ctx.emit_i32_slot(&args[2])?;
            write!(ctx.w, ", ")?;
        }
        ctx.emit_i32_slot(&args[3])?;

        let extras = {
            use std::fmt::Write as _;
            let mut s = String::new();
            for i in 0..n_extra {
                write!(s, ", _ex{i}").unwrap();
            }
            s
        };

        // args[1] is the vector element stride (a literal Int), needed to read a
        // narrow scalar element at its true width.
        let elem_sz = match args.get(1).map(Value::unspan) {
            Some(Value::Int(s)) => *s,
            _ => 8,
        };
        let (prep, arg) = tuple_arg_prep(ctx, fn_d_nr, elem_sz);
        // @PLAN59: hidden heap DESTINATION attrs (ref_return promotion /
        // the signature-time `__retbuf`) — allocate one backing store per
        // dest inside the worker closure and pass it in attr order (dests
        // are the trailing attrs).  Classified by TYPE, mirroring
        // `native_gate::classify_bridge_attr`; workers without dests emit
        // nothing (today's literal-returning corpus is byte-identical).
        let (prep, dests) = {
            let mut prep = prep;
            let mut dests = String::new();
            use std::fmt::Write as _;
            for (i, a) in ctx.output.data.def(fn_d_nr).attributes().iter().enumerate() {
                if a.hidden
                    && matches!(
                        a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    write!(
                        prep,
                        "let _pd{i}: DbRef = unsafe {{ &mut *cell.get() }}.database(100); "
                    )
                    .unwrap();
                    write!(dests, ", _pd{i}").unwrap();
                }
            }
            (prep, dests)
        };
        match shape {
            ClosureShape::Text if owned_text => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) }})"
            )?,
            ClosureShape::Text => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}let mut _w = String::new(); {worker_name}(cell, {arg}{extras}{dests}, &mut _w); _w }})"
            )?,
            ClosureShape::HeapRef => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) }})"
            )?,
            // #281 — fn-ref return: the worker yields the native fn-ref tuple
            // `(u32, DbRef)` directly; the closure returns it verbatim.
            ClosureShape::Fn => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) }})"
            )?,
            ClosureShape::Float => write!(
                ctx.w,
                ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}).to_bits() as i64 }})"
            )?,
            ClosureShape::Scalar => {
                // Plan-06 ARC.md A3.5 — Boolean returns map to Rust
                // `bool`, which cannot cast directly to i64; insert
                // an `as u8` bridge.  Other Scalar shapes (Integer
                // narrow / wide, Character → i32, Enum-no-payload →
                // u8) all support `as i64` natively.
                if matches!(worker_ret.base(), Type::Boolean) {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) as u8 as i64 }})"
                    )?;
                } else {
                    write!(
                        ctx.w,
                        ", |cell, elm| {{ {prep}{worker_name}(cell, {arg}{extras}{dests}) as i64 }})"
                    )?;
                }
            }
        }

        for _ in 0..n_extra {
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// `n_parallel_discard` emitter — the route a `par` loop with a body that never
/// names the worker's result takes (`for x in v par(r = f(x), N) { }`).
///
/// Same arg layout as `ParallelForEmitter` (input, elem_size, return_size, threads,
/// func, extras…, n_extra), and the same closure scaffolding, with one difference
/// that removes most of it: the result is dropped, so the closure returns `()` and
/// the emitter needs neither a per-shape return bridge nor `return_size` nor the
/// heap-ref storage type.  What is left is the ONE shape that still matters — a text
/// worker taking a `&mut String` work buffer must still be handed one, or the call
/// does not compile.
///
/// Without this emitter the route fell through to the declaration-driven default,
/// which emitted `n_parallel_discard`'s five-parameter loft declaration with an empty
/// body: workers that never ran.  A six-argument call site turned that into a rustc
/// arity error, which is the only reason it was ever seen (loft#987).
pub struct ParallelDiscardEmitter;

impl OpEmitter for ParallelDiscardEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Guard: same as the for/queue family — at least 5 args with args[4] a
        // non-negative i32 (the worker fn's def_nr).  Otherwise fall through.
        let fn_d_nr = match args.get(4) {
            Some(Value::Int(d)) if *d >= 0 => (*d).cast_unsigned(),
            _ => return super::default::DefaultEmitter.emit(ctx, args),
        };
        if args.len() < 5 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        let worker_def = ctx.output.data.def(fn_d_nr);
        let worker_name = worker_def.name().to_string();
        let worker_ret = worker_def.returned().clone();
        let wants_work_buffer = matches!(closure_shape(&worker_ret), ClosureShape::Text)
            && !crate::generation::returns_owned_string(worker_def);

        // Extras: args[5..len-1]; the trailing args[len-1] is the n_extra count.
        let n_extra = if args.len() > 6 { args.len() - 6 } else { 0 };
        for i in 0..n_extra {
            write!(ctx.w, "{{ let _ex{i} = ")?;
            ctx.emit(&args[5 + i])?;
            write!(ctx.w, "; ")?;
        }

        write!(ctx.w, "n_parallel_discard_native(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[3])?;

        let extras = {
            use std::fmt::Write as _;
            let mut s = String::new();
            for i in 0..n_extra {
                write!(s, ", _ex{i}").unwrap();
            }
            s
        };

        // args[1] is the vector element stride (a literal Int), needed to read a
        // narrow scalar element at its true width.
        let elem_sz = match args.get(1).map(Value::unspan) {
            Some(Value::Int(s)) => *s,
            _ => 8,
        };
        let (prep, arg) = tuple_arg_prep(ctx, fn_d_nr, elem_sz);
        // @PLAN59: hidden heap DESTINATION attrs — one backing store per dest inside
        // the closure, passed in attr order.  Same classification as the for/queue
        // emitters; a worker without dests emits nothing.
        let (prep, dests) = {
            let mut prep = prep;
            let mut dests = String::new();
            use std::fmt::Write as _;
            for (i, a) in ctx.output.data.def(fn_d_nr).attributes().iter().enumerate() {
                if a.hidden
                    && matches!(
                        a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    write!(
                        prep,
                        "let _pd{i}: DbRef = unsafe {{ &mut *cell.get() }}.database(100); "
                    )
                    .unwrap();
                    write!(dests, ", _pd{i}").unwrap();
                }
            }
            (prep, dests)
        };
        if wants_work_buffer {
            write!(
                ctx.w,
                ", |cell, elm| {{ {prep}let mut _w = String::new(); {worker_name}(cell, {arg}{extras}{dests}, &mut _w); }})"
            )?;
        } else {
            write!(
                ctx.w,
                ", |cell, elm| {{ {prep}let _ = {worker_name}(cell, {arg}{extras}{dests}); }})"
            )?;
        }

        for _ in 0..n_extra {
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// Plan-06 ARC.md A5b — `n_parallel_fold` emitter.  Closure-based
/// bridge to `n_parallel_fold_native` (defined in
/// `src/codegen_runtime.rs`).  Closes the native gap left by A5
/// (interp-only `par_fold` builtin) so the same loft surface runs
/// on `--native`.
///
/// The fold Call's arg layout (from `parse_par_fold` in
/// `src/parser/builtins.rs`) differs from the for/queue family:
///   - args[0] — input (vector<integer>) — DbRef
///   - args[1] — init — i64
///   - args[2] — fold fn — Value::Int(d_nr)
///   - args[3] — threads — i32
///   - args[4] — n_extra (V1: Value::Int(0))
///
/// V1 ignores args[4]: extras pass-through is a future ARC step
/// (per ARC.md A5).  The worker closure synthesised here has
/// signature `Fn(cell, acc: i64, row: i64) -> i64` matching the
/// runtime helper's bound.
pub struct ParallelFoldEmitter;

impl OpEmitter for ParallelFoldEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Guard: V1 fold has exactly 5 args with args[2] a non-negative
        // i32 d_nr (the worker fn).  Otherwise fall through.
        let fn_d_nr = match args.get(2) {
            Some(Value::Int(d)) if *d >= 0 => (*d).cast_unsigned(),
            _ => return super::default::DefaultEmitter.emit(ctx, args),
        };
        if args.len() < 5 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        let worker_def = ctx.output.data.def(fn_d_nr);
        let worker_name = worker_def.name().to_string();

        // Helper call: `n_parallel_fold_native(cell, input, init, threads, |cell, acc, row| worker(cell, acc, row))`.
        write!(ctx.w, "n_parallel_fold_native(cell, ")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit(&args[1])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[3])?;
        write!(
            ctx.w,
            ", |cell, acc, row| {{ {worker_name}(cell, acc, row) }})"
        )
    }
}

/// Phase 06 — `n_parallel_buf_get` / `_text` / `_ref` and
/// `n_parallel_buf_drop` / `_text` / `_ref` emitter.  Renames the
/// call site to the corresponding `_native` runtime fn; args pass
/// through unchanged.
///
/// These fns have no closure transformation — they're plain reads
/// from / pops of the active par-buffer.  The emitter exists only
/// to disambiguate the runtime fn name from the loft-side stub
/// (which has a `todo!()` body in generated Rust).
pub struct ParallelBufRenameEmitter;

impl OpEmitter for ParallelBufRenameEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let native_name = format!("{}_native", ctx.def_fn.name());
        write!(ctx.w, "{native_name}(cell")?;
        for arg in args {
            write!(ctx.w, ", ")?;
            ctx.emit(arg)?;
        }
        write!(ctx.w, ")")
    }
}
