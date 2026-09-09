// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I116 — Library placement — in-process, a worker, or another machine
// @PLN119 arc A — routing a placed library's calls from the interpreter to its worker.

//! What makes placement *policy*: a consumer writes `use maths;` and calls
//! `add(2, 3)`, and whether that runs here or in a worker process is decided by
//! the library's manifest, not by the calling source.
//!
//! The route reuses the mechanism a native library already travels. Marking a
//! function with a `def.native` symbol makes `byte_code` emit an `OpStaticCall`
//! and registers a stub; [`install`] then replaces that stub with
//! [`placed_dispatch`], which reads the arguments off the interpreter stack,
//! hands them to the worker over the wire, and writes the answer back. The
//! interpreter never learns that anything unusual happened, which is the point.
//!
//! # What this routes
//!
//! Parameters and returns may be integer-family, boolean, `single`, text, or —
//! @PLN119 arc B — a **struct or vector**, plus a void return. A function
//! outside that is simply **not marked**, so it runs in-process —
//! byte-identically, which is the same fallback a library that cannot compile
//! native already takes. Nothing becomes a call that fails later.
//!
//! # A compound argument is passed BY REFERENCE, so it crosses twice
//!
//! `f(p)` where `p` is a struct hands the callee the caller's own record: a
//! `pub fn bump(p: Point)` that assigns `p.x` changes the CALLER's `p`. That is
//! loft's semantics, not an accident, so a crossing that only copied the
//! argument over would silently diverge the moment a library wrote to one.
//!
//! Every compound argument is therefore copied into the [arena](super::arena)
//! before the call and copied back out after it. The callee reads and writes the
//! arena record in place — it never learns that its parameter is not the
//! caller's own — and what the caller sees afterwards is what the callee left.
//!
//! # The layout gate
//!
//! The two sides are different programs. A record graph in the arena is read as
//! the RECEIVING program's own type, so the two must lay that type out
//! identically or the worker reads foreign bytes as a `Point`. At install, one
//! round trip per placed function compares [`signature_layout`] computed on each
//! side, and a function the two disagree about is not placed.
//!
//! # A text return rides the protocol it already has
//!
//! A text return does not come back on the stack. It travels the interpreter's
//! destination-buffer protocol: codegen emits `n_set_bridge_dest` immediately
//! before the call, stashing the caller's work-buffer record, and the callee
//! writes into that record and pushes nothing.
//!
//! Nothing had to be added for a placed call to use it. That routing keys on
//! [`crate::state::codegen::is_cdylib_text_call`] — a non-empty `def.native`
//! symbol plus a text return — which is exactly the shape [`mark_exports`]
//! leaves behind, so the two calls emit already. What remains is this side of
//! the contract: take the stashed destination, write the worker's answer into
//! it, and push nothing. Approximating it, which is why arc A refused text
//! returns rather than guessing, would have returned a wrong value.

use super::wire::Worker;
use crate::data::{Data, DefType, Type};
use crate::database::Stores;
use crate::host::Value;
use crate::keys::DbRef;
use crate::state::State;
use crate::use_analysis::HeapDelivery;
use std::collections::HashMap;
use std::sync::Mutex;

/// The wire shape of one parameter or return.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Void,
    Int,
    Bool,
    Single,
    Text,
    /// The hidden `&text` work buffer a text-returning loft function carries.
    ///
    /// Not a value on the wire — the worker never sees it. It is a slot the
    /// CALLER allocated and passed for the answer to be written into, and it is
    /// listed among the parameters because the emitted code pushes it like any
    /// other argument: a dispatcher that skipped it would leave it on the stack
    /// and every later frame would read one cell off.
    WorkBuf,
    /// A struct or vector: a 12-byte `DbRef` on the stack, crossing through the
    /// [arena](super::arena).
    ///
    /// `tp` is THIS program's store type id for it, which only [`install`] can
    /// resolve (it needs the live `Stores`); [`frame_kinds`] leaves it
    /// `u16::MAX`.
    ///
    /// `written` says whether the callee may write to it — false for a `const`
    /// parameter, where loft has already REJECTED every mutation through the
    /// name at compile time. That is worth knowing here because the copy-back
    /// is the expensive half of a compound crossing (a full graph copy per
    /// argument per call) and a `const` parameter cannot have changed. It also
    /// keeps the crossing off a destination that may be read-only, which is what
    /// a `const` argument's store is for the duration of the call.
    Compound {
        tp: u16,
        written: bool,
    },
    /// The hidden destination a compound-returning loft function carries — the
    /// `__retbuf` parameter. Like [`Kind::WorkBuf`], it is pushed by the emitted
    /// code and is not a value the worker is sent.
    RetBuf(u16),
}

/// Classify a loft type for the wire, or refuse it.
fn kind_of(ty: &Type, returning: bool) -> Option<Kind> {
    match ty {
        Type::Void | Type::Null if returning => Some(Kind::Void),
        Type::Integer(_) => Some(Kind::Int),
        Type::Boolean => Some(Kind::Bool),
        Type::Single => Some(Kind::Single),
        Type::Text(_) => Some(Kind::Text),
        _ if crate::host::is_compound(ty) => Some(Kind::Compound {
            tp: u16::MAX,
            written: true,
        }),
        _ => None,
    }
}

/// This program's store type id for `ty` — the id `copy_claims` dispatches on.
fn type_id(stores: &mut Stores, data: &Data, ty: &Type) -> u16 {
    stores.db_type(ty, data)
}

/// How this program lays out `func`'s compound parameters and return.
///
/// Both sides compute it from their OWN type table and the strings must match,
/// or the value one side writes into the arena is not the value the other reads
/// out. Scalars contribute only their arity: their ABI is already pinned by
/// [`Kind`], and a mismatch there is refused at marking rather than here.
///
/// Reuses @PLN97's `layout_algo_hash`, which is the single home of "do these two
/// programs lay this type out the same way" — including everything the type
/// references and the host's endianness.
#[must_use]
pub fn signature_layout(program: &mut crate::host::Program, func: &str) -> Option<String> {
    use std::fmt::Write as _;
    let (params, ret) = program.signature(func)?;
    let mut out = String::new();
    for (i, ty) in params.iter().chain(std::iter::once(&ret)).enumerate() {
        if i > 0 {
            out.push(',');
        }
        if crate::host::is_compound(ty) {
            let (_, hash) = program.layout_of(ty);
            let _ = write!(out, "{hash:016x}");
        } else {
            out.push('.');
        }
    }
    Some(out)
}

/// [`signature_layout`], computed against a `Stores` + `Data` directly — the
/// caller has those rather than a `host::Program`.
///
/// The two must produce the same string for the same types or the gate would
/// refuse every placement, so they are deliberately one shape written twice
/// against two different holders of the same tables rather than two rules.
fn signature_layout_here(
    stores: &mut Stores,
    data: &Data,
    def: &crate::data::Definition,
) -> String {
    use std::fmt::Write as _;
    let params: Vec<Type> = def
        .attributes()
        .iter()
        .filter(|a| !a.hidden)
        .map(|a| a.typedef.clone())
        .collect();
    let mut out = String::new();
    for (i, ty) in params
        .iter()
        .chain(std::iter::once(&def.returned))
        .enumerate()
    {
        if i > 0 {
            out.push(',');
        }
        if crate::host::is_compound(ty) {
            let tp = type_id(stores, data, ty);
            let _ = write!(out, "{:016x}", stores.layout_algo_hash(&[tp]));
        } else {
            out.push('.');
        }
    }
    out
}

/// The frame shape of one call: every attribute the emitted code pushes, in
/// declaration order, hidden work buffers included.
///
/// The two lists a signature produces are different questions and must not be
/// conflated: what the WORKER is sent (user parameters) and what the emitted
/// code PUSHES (those plus the compiler's hidden ones).
fn frame_kinds(def: &crate::data::Definition) -> Option<Vec<Kind>> {
    def.attributes()
        .iter()
        .map(|a| {
            if crate::native_lib::is_text_work_buffer(&a.typedef) {
                Some(Kind::WorkBuf)
            } else if a.hidden && crate::host::is_compound(&a.typedef) {
                Some(Kind::RetBuf(u16::MAX))
            } else if a.hidden {
                // Some other compiler-inserted parameter. Its layout is not
                // known here, so refuse the function rather than pop a frame
                // shape that is a guess.
                None
            } else {
                kind_of(&a.typedef, false)
            }
        })
        .collect()
}

/// Fill in this program's store type ids, which `frame_kinds` cannot know.
fn resolve_ids(
    kinds: &mut [Kind],
    ret: &mut Kind,
    def: &crate::data::Definition,
    stores: &mut Stores,
    data: &Data,
) {
    for (i, (k, a)) in kinds.iter_mut().zip(def.attributes().iter()).enumerate() {
        match k {
            Kind::Compound { tp, written } => {
                *tp = type_id(stores, data, &a.typedef);
                // `const` on a parameter is a compile-time promise, not a hint:
                // every mutation THROUGH the name is rejected, so there is
                // nothing for the copy-back to bring home.
                *written = !def.variables.is_value_const(i as u16);
            }
            Kind::RetBuf(id) => *id = type_id(stores, data, &a.typedef),
            _ => {}
        }
    }
    if let Kind::Compound { tp, .. } = ret {
        *tp = type_id(stores, data, &def.returned);
    }
}

/// One placed function: which worker serves it, what it is called there, and
/// the shape of its frame.
struct Placed {
    worker: usize,
    func: String,
    params: Vec<Kind>,
    ret: Kind,
}

/// Every worker this process started, and the call table the dispatcher reads.
///
/// One lock covers both because a call must hold the worker for its whole
/// round trip: the wire has a single request slot, so two threads calling the
/// same placed library concurrently would interleave two frames in one buffer.
/// Serialising them is therefore correctness, not caution — and it is the
/// reason a placed library is not yet a good fit for a hot `par` arm.
static PLACEMENT: Mutex<Option<Registry>> = Mutex::new(None);

struct Registry {
    workers: Vec<Worker>,
    calls: HashMap<u16, Placed>,
}

/// The functions of the library at `pkg_dir` that arc A can route, marked with
/// their dispatch symbol.
///
/// Mirrors `native_lib::library_export_set` — a top-level, user-named, `pub`
/// function whose source file sits under the package — and applies the wire's
/// own type gate instead of the shared-store one. **Call before `byte_code`.**
///
/// Returns the (definition, symbol, name) of each marked function.
pub fn mark_exports(data: &mut Data, pkg_dir: &str) -> Vec<(u32, String, String)> {
    let dups = crate::generation::duplicate_fn_names(data);
    let mut marked = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !matches!(def.def_type(), DefType::Function)
            || !def.pub_visible
            || !crate::portable_path::is_under(&def.position().file, pkg_dir)
            || !def.native().is_empty()
        {
            continue;
        }
        let name = def.original_name().clone();
        if name.starts_with('_') {
            continue;
        }
        // Both questions must be answerable: what the worker is sent, and what
        // the emitted code pushes. A signature that fails either runs in-process.
        let Some(frame) = frame_kinds(def) else {
            continue;
        };
        let Some(ret) = kind_of(&def.returned, true) else {
            continue;
        };
        // A text answer needs somewhere in the CALLER to live, and the caller
        // only ever offers one of two: a destination record (where the result is
        // assigned to a text variable) or the hidden work buffer a promoted text
        // return carries. A function whose text return was never promoted — the
        // usual reason being that it returns a constant, `fn version() -> text {
        // "1.0" }` — offers neither, and a `Str` over the worker's own answer
        // would point at a String freed the moment this call returns.
        //
        // So it is not placed, and runs in-process instead. That is the same
        // fallback every other signature the wire cannot carry takes, and by the
        // invariant it is the same program. Making it uniform means promoting
        // such a return to a retbuf (@PLN104's transform, which today only
        // considers the main program's own definitions).
        if ret == Kind::Text && !frame.contains(&Kind::WorkBuf) {
            continue;
        }
        // The same question for a compound answer, and the same reason: it is
        // built in the worker's arena, and the caller has to have offered
        // somewhere in ITS OWN memory for the copy to land. That offer is the
        // hidden `__retbuf` the emitted code pushes; without one there is
        // nowhere, and the function runs in-process instead.
        if matches!(ret, Kind::Compound { .. })
            && !frame.iter().any(|k| matches!(k, Kind::RetBuf(_)))
        {
            continue;
        }
        // @PLN119 arc C — and the caller has to be going to FREE it.
        //
        // A heap return is delivered one of three ways, and only two of them
        // leave the caller owning what it gets. A `View` return hands back a
        // borrow of something the callee did not create — an argument, or its
        // own long-lived state — so the caller's emitted code frees nothing.
        // In-process that is right: there is nothing new to free. Placed, the
        // answer has to be COPIED into the caller's address space, and a copy
        // nobody frees is a leak per call — which is exactly what
        // `fn head(v: vector<P>) -> P { v[0] }` did.
        //
        // A view across a process boundary is not a view anyway: the thing it
        // views is in the other process. So the function is not placed, and
        // runs in-process, where the borrow means what it says.
        if matches!(ret, Kind::Compound { .. })
            && crate::use_analysis::heap_return_delivery(data, d) == HeapDelivery::View
        {
            continue;
        }
        let sym = format!(
            "loft_placed_{}",
            crate::generation::disambiguated_fn_ident(&dups, data.def(d))
        );
        data.def_mut(d).native.clone_from(&sym);
        marked.push((d, sym, name));
    }
    marked
}

/// Start a worker for each placed library and point its marked functions at it.
///
/// Call after `byte_code`, which is what registers the stubs this replaces.
///
/// `cwd` is where the CALLER will run — see [`Worker::spawn`]. Pass an empty
/// path to inherit this process's current directory.
///
/// # Errors
/// The first library whose worker will not start, named. A placed library that
/// cannot run is not degraded to in-process silently: its whole reason for the
/// declaration is isolation, and quietly withdrawing that would leave no trace
/// in any output.
pub fn install(
    state: &mut State,
    data: &Data,
    libs: &[(String, String, crate::lib_placement::Placement)],
    stdlib_dir: &std::path::Path,
    cwd: &std::path::Path,
) -> Result<usize, String> {
    let mut reg = Registry {
        workers: Vec::new(),
        calls: HashMap::new(),
    };
    let dups = crate::generation::duplicate_fn_names(data);
    let mut wired = 0usize;
    for (name, pkg_dir, placement) in libs {
        // The one place the two transports are chosen between. A local worker is
        // this process's to START; a remote server is already running and is
        // only CONNECTED to — the asymmetry is real and is not smoothed over.
        let worker = match placement {
            crate::lib_placement::Placement::Remote => {
                let address = crate::lib_placement::Placement::address_for(name)?;
                Worker::connect(name, &address).map_err(|e| e.to_string())?
            }
            _ => Worker::spawn(name, std::path::Path::new(pkg_dir), stdlib_dir, cwd)
                .map_err(|e| e.to_string())?,
        };
        let w_idx = reg.workers.len();
        reg.workers.push(worker);

        for d in 0..data.definitions() {
            let def = data.def(d);
            if !matches!(def.def_type(), DefType::Function)
                || !crate::portable_path::is_under(&def.position().file, pkg_dir.as_str())
                || !def.native().starts_with("loft_placed_")
            {
                continue;
            }
            let sym = format!(
                "loft_placed_{}",
                crate::generation::disambiguated_fn_ident(&dups, def)
            );
            let Some(&lib_idx) = state.library_names.get(&sym) else {
                continue;
            };
            let (Some(mut params), Some(mut ret)) =
                (frame_kinds(def), kind_of(&def.returned, true))
            else {
                continue;
            };
            resolve_ids(&mut params, &mut ret, def, &mut state.database, data);
            let func = def.original_name().clone();
            // The layout gate. Only a signature with something compound in it
            // has anything to disagree about, so the round trip is skipped
            // where it could only ever agree.
            let carries_compound = matches!(ret, Kind::Compound { .. })
                || params
                    .iter()
                    .any(|k| matches!(k, Kind::Compound { .. } | Kind::RetBuf(_)));
            if carries_compound {
                let here = signature_layout_here(&mut state.database, data, def);
                match reg.workers[w_idx].layout(&func) {
                    Ok(there) if there == here => {}
                    // Not an error, a refusal: the function runs in-process,
                    // which by the invariant is the same program. Saying so is
                    // the point — a silent skip would look like the library
                    // simply not being placed.
                    Ok(there) => {
                        eprintln!(
                            "Warning: '{name}::{func}' is not placed — this program and its \
                             worker lay its struct/vector types out differently ({here} vs \
                             {there}); the call runs in-process"
                        );
                        continue;
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: '{name}::{func}' is not placed — its worker could not \
                             report a layout ({e}); the call runs in-process"
                        );
                        continue;
                    }
                }
            }
            reg.calls.insert(
                lib_idx,
                Placed {
                    worker: w_idx,
                    func,
                    params,
                    ret,
                },
            );
            state.replace_static_fn(&sym, placed_dispatch);
            wired += 1;
        }
    }
    *PLACEMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reg);
    Ok(wired)
}

/// Shut every worker down. Called at the end of a run so a worker never
/// outlives the process that started it.
pub fn shutdown() {
    if let Ok(mut g) = PLACEMENT.lock() {
        g.take();
    }
}

/// One compound argument of a call in flight: where the CALLER keeps it, what
/// type it is, and whether the callee may have written to it.
///
/// Recorded while the frame is popped and read twice — to marshal the value into
/// the arena, and to bring the callee's writes back out.
#[derive(Clone, Copy)]
struct CompoundArg {
    /// Index into the argument list the worker is sent.
    slot: usize,
    /// The record in THIS process the value came from and goes back to.
    caller: DbRef,
    /// This program's store type id for it.
    tp: u16,
    /// False for a `const` parameter — see [`Kind::Compound`].
    written: bool,
}

/// Do two references name the same record?
///
/// Aliasing is not a corner case here: `f(p, p)` hands the callee ONE record
/// in-process, so the crossing has to hand it one arena record too. Two copies
/// would give the callee two independent values, and copying both back would
/// then be a race between them decided by argument order.
fn same_record(a: &DbRef, b: &DbRef) -> bool {
    a.store_nr == b.store_nr && a.rec == b.rec && a.pos == b.pos
}

/// The interpreter's entry into a placed call: read the frame off the stack,
/// cross, write the answer back.
fn placed_dispatch(stores: &mut Stores, stack: &mut DbRef) {
    let lib_idx = crate::extensions::current_lib_idx();
    let mut guard = PLACEMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(reg) = guard.as_mut() else {
        panic!("placed call with no placement registry — install() did not run");
    };
    let Registry { workers, calls } = reg;
    let Some(placed) = calls.get(&lib_idx) else {
        panic!("no placed signature for library index {lib_idx}");
    };
    let worker = &mut workers[placed.worker];

    // Pop in reverse (the stack is LIFO), then restore declaration order. Every
    // cell the emitted code pushed is popped here, hidden ones included — a cell
    // left behind does not fail here, it shifts every later frame by one.
    let mut args: Vec<Value> = Vec::with_capacity(placed.params.len());
    let mut work_buf: Option<DbRef> = None;
    let mut ret_buf: Option<DbRef> = None;
    // Which argument slots hold a compound, and where the caller keeps each —
    // needed twice: to marshal it in, and to copy the callee's writes back out.
    let mut compound: Vec<CompoundArg> = Vec::new();
    for k in placed.params.iter().rev() {
        match k {
            // The whole cell, unnarrowed: the callee's declared width is what
            // decides the value, exactly as the native bridge treats it.
            Kind::Int => args.push(Value::Int(stores.get::<i64>(stack))),
            Kind::Bool => args.push(Value::Bool(stores.get::<bool>(stack))),
            // `single` is a 4-byte cell, so it pops as `f32` — reading it as an
            // `f64` would take the next argument's bytes with it.
            Kind::Single => args.push(Value::Float(f64::from(stores.get::<f32>(stack)))),
            Kind::Text => {
                args.push(Value::Text(
                    stores.get::<crate::keys::Str>(stack).str().to_string(),
                ));
            }
            Kind::Void => args.push(Value::Void),
            // Popped, never sent: it is the caller's answer slot, not an input.
            Kind::WorkBuf => work_buf = Some(stores.get::<DbRef>(stack)),
            Kind::RetBuf(_) => ret_buf = Some(stores.get::<DbRef>(stack)),
            // A placeholder now; the real value is a record in the arena, and
            // the arena is not bound until the frame has been popped clean.
            Kind::Compound { tp, written } => {
                compound.push(CompoundArg {
                    slot: args.len(),
                    caller: stores.get::<DbRef>(stack),
                    tp: *tp,
                    written: *written,
                });
                args.push(Value::Ref(DbRef::NULL));
            }
        }
    }
    args.reverse();
    // `args` was built back-to-front, so every recorded slot is counted from the
    // wrong end. Turn them round together rather than searching for them again.
    let last = args.len().saturating_sub(1);
    for c in &mut compound {
        c.slot = last - c.slot;
    }

    let ret = placed.ret;
    // Claimed before the crossing, so it is claimed on EVERY exit below —
    // including a fault, where a destination left set would redirect the next
    // text call in the program into this call's buffer.
    let text_dest = if ret == Kind::Text {
        stores.bridge_text_dest.take()
    } else {
        None
    };
    // Checked before the crossing, while the signature is still in hand: a text
    // answer with nowhere to go is a routing bug, and refusing it here is what
    // arc A chose over approximating the protocol and returning a wrong value.
    assert!(
        ret != Kind::Text || text_dest.is_some() || work_buf.is_some(),
        "placed text call '{}' has nowhere to put its answer — neither a stashed \
         destination nor a work buffer reached the dispatcher",
        placed.func
    );
    // Where a compound answer will land, decided BEFORE either arena is bound.
    //
    // Not merely tidy: `stores.null()` takes the next store slot, so minting
    // this while the arenas are registered puts it ABOVE them, and the slot
    // watermark can then never come back down past a live store. Under
    // `LOFT_STRICT_STORES` (where a released slot is deliberately never
    // recycled) that grew the table by two slots per call and exhausted all
    // 65535 of them after ~32k iterations — a loop that ran flat at four slots
    // in-process. Minting first puts the answer BELOW the arenas, so releasing
    // them lowers the watermark again.
    //
    // The caller offers one of exactly two destinations and neither is a null
    // reference: a materialised record (which the result then borrows), or an
    // empty placeholder store — in which case an in-process callee would mint
    // its own and hand ownership over, so that is what happens here.
    let compound_dest = match ret {
        Kind::Compound { tp, .. } => Some(match ret_buf {
            Some(d) if super::arena::names_a_record(&d) => d,
            _ => super::arena::mint_value(stores, tp),
        }),
        _ => None,
    };
    // Emptied on EVERY call, not only one carrying a compound. Locally that is
    // a couple of word writes either way; remotely it is what the argument
    // arena's live prefix costs to SEND, and a scalar call has no business
    // shipping the graph the last compound call left behind.
    worker.arg_arena().reset();
    // Build every compound argument in the arena. The `seen` map is not an
    // optimisation: `f(p, p)` passes ONE record twice in-process, so two arena
    // copies would give the callee two independent values and the copy-back
    // would then have to pick which one won.
    if !compound.is_empty() {
        let arena_nr = worker.arg_arena().bind(stores);
        // A list rather than a map: a signature has a handful of parameters, and
        // `DbRef` is a plain triple with no hash to spend.
        let mut seen: Vec<(DbRef, DbRef)> = Vec::new();
        for &CompoundArg {
            slot,
            caller: src,
            tp,
            ..
        } in &compound
        {
            if src.is_null() {
                continue;
            }
            let dst = if let Some(&(_, d)) = seen.iter().find(|(s, _)| same_record(s, &src)) {
                d
            } else {
                {
                    // A fresh arena record, default-initialised. An argument
                    // that names no record — an untouched placeholder, or a
                    // vector that is validly empty — is exactly that default,
                    // so the copy is skipped rather than reading the source
                    // store's header as if it were the value.
                    let d = super::arena::alloc_value(stores, arena_nr, tp);
                    if super::arena::names_a_record(&src) {
                        super::arena::trace(stores, "arg-src", &src, tp);
                        super::arena::copy_value(stores, &d, &src, tp);
                        super::arena::trace(stores, "arg-dst", &d, tp);
                    }
                    seen.push((src, d));
                    d
                }
            };
            args[slot] = Value::Ref(dst);
        }
        worker.arg_arena().unbind(stores, arena_nr);
    }

    let out = worker.call(&placed.func, &args);

    // A FAILED crossing does not map or read the shared arenas at all — @PLN119
    // arc C makes the fault isolation structural rather than incidental.
    //
    // A worker that died mid-call left both arenas in whatever state it got to,
    // and may have resized one (which moves the file out from under this side's
    // mapping). Nothing here has to look: on this path the answer does not exist
    // and the callee's writes to the arguments are not ones the caller may see,
    // because in-process the same fault would have halted before them. Skipping
    // the bind is what makes "a crashed worker cannot corrupt the caller's
    // stores" a property of the code rather than an argument about it.
    let out = match out {
        Err(e) => {
            let intact = compound_arguments_intact(stores, &compound);
            drop(guard);
            let detail = if intact {
                e
            } else {
                e.with_detail(" — AND this program's own stores did not survive it")
            };
            fault(stores, detail);
            return;
        }
        Ok(v) => v,
    };

    // Bind both arenas for the way back: the answer lives in one and the
    // callee's writes to the arguments in the other.
    let arg_nr = worker.arg_arena().bind(stores);
    let ret_nr = worker.ret_arena().bind(stores);
    // Re-read the answer now that the return arena has a store number on this
    // side — a compound reference could not be completed before it had one.
    let out = match out {
        Value::Ref(_) => worker
            .reread_answer(ret_nr)
            .ok_or_else(|| super::wire::Fault::from("malformed compound answer")),
        other => Ok(other),
    };

    // loft passes a compound BY REFERENCE, so whatever the callee wrote into its
    // parameter is the caller's to see. Copying back is what makes that true
    // across the boundary; skipping it would make `bump(p)` a no-op under
    // `placement = "process"` and not in-process.
    if out.is_ok() {
        let mut done: Vec<DbRef> = Vec::new();
        for &CompoundArg {
            slot,
            caller: dst,
            tp,
            written,
        } in &compound
        {
            // A `const` parameter cannot have been written, so there is nothing
            // to bring home — and the copy-back is the expensive half.
            if !written
                || !super::arena::names_a_record(&dst)
                || done.iter().any(|d| same_record(d, &dst))
            {
                continue;
            }
            if let Some(Value::Ref(src)) = args.get(slot)
                && !src.is_null()
            {
                let src = DbRef {
                    store_nr: arg_nr,
                    ..*src
                };
                super::arena::copy_value(stores, &dst, &src, tp);
                done.push(dst);
            }
        }
    }

    finish_call(stores, stack, ret, out, text_dest, work_buf, compound_dest);
    worker.arg_arena().unbind(stores, arg_nr);
    worker.ret_arena().unbind(stores, ret_nr);
    // The lock is held across the crossing on purpose (see `PLACEMENT`), but a
    // fault must not poison it while a guard is live.
    drop(guard);
}

/// @PLN119 arcs C+D — is every compound value this call handed over still
/// structurally sound in THIS process?
///
/// The plan claims a crashed worker cannot corrupt the caller's stores, and
/// until now that rested on a structural argument: the worker has its own
/// address space and only values are copied in and out. This turns the argument
/// into a reading, taken at the one moment it could be false — a crossing that
/// failed — using loft's own guard-before-dereference walk, which NAMES a broken
/// edge rather than faulting on it.
///
/// Only on the failure path, so it costs a healthy call nothing, and a failed
/// call is fatal anyway.
fn compound_arguments_intact(stores: &Stores, compound: &[CompoundArg]) -> bool {
    compound
        .iter()
        .filter(|c| super::arena::names_a_record(&c.caller))
        .all(|c| stores.verify_graph_ok(&c.caller))
}

/// Surface a failed crossing as the CALLER's runtime error.
///
/// What makes a placed library's error behaviour match an in-process one — the
/// fourth thing the module's invariant names, beside type, effect and lifetime.
/// The fault is rebuilt from the parts that crossed rather than re-derived, so
/// the position is the LIBRARY's own file and line and the detail is the one the
/// library rendered.  Re-describing it here is what used to print
/// `panic: runtime error: panic: …`, with the position, the caret and the frames
/// all gone.
///
/// The call chain arriving here is only the library's half; `main()` is in THIS
/// process. `State::note_runtime_error_halt` adds the caller's frames, which is
/// what `crossed_placement` is for.
fn fault(stores: &mut Stores, e: super::wire::Fault) {
    stores.runtime_error = Some(Box::new(crate::runtime_error::RuntimeError::relayed(
        e.label,
        e.message,
        e.position,
        e.call_chain,
    )));
    stores.had_fatal = true;
}

/// Put a text answer where the CALL SITE asked for it.
///
/// A text return has two call shapes and the call site chose which one, so this
/// reads the choice rather than assuming it — the same two branches the cdylib
/// bridge (`bridge_text_result`) has.
///
///   * Destination-passing, emitted where the result is assigned to a text
///     variable: the answer goes straight into that variable's record and
///     NOTHING is pushed.
///   * Otherwise the ordinary loft convention, which every other position uses:
///     the answer goes into the hidden work buffer the caller passed, and a
///     `Str` over it is pushed as the result.
///
/// Both of the wire's two text shapes — inline in the frame, or parked in the
/// return arena for loft#1061 — end here, so the caller cannot tell them apart
/// and neither shape can drift away from the other.
fn deliver_text(
    stores: &mut Stores,
    stack: &mut DbRef,
    text_dest: Option<DbRef>,
    work_buf: Option<DbRef>,
    answer: &str,
) {
    let into = text_dest.or(work_buf).expect("checked before the crossing");
    let buf = stores
        .store_mut(&into)
        .addr_mut::<String>(into.rec, into.pos);
    // Append: the call site cleared the buffer beforehand, exactly as it does
    // for an in-process text return.
    buf.push_str(answer);
    if text_dest.is_none() {
        // `Str` borrows the buffer's bytes; the buffer is the caller's own
        // variable and outlives this result.
        let result = crate::keys::Str::new(buf.as_str());
        stores.put(stack, result);
    }
}

/// Write the answer back where the emitted code expects it — the half of a
/// placed call that is about the CALLER's frame rather than the crossing.
#[allow(clippy::too_many_arguments)]
fn finish_call(
    stores: &mut Stores,
    stack: &mut DbRef,
    ret: Kind,
    out: Result<Value, super::wire::Fault>,
    text_dest: Option<DbRef>,
    work_buf: Option<DbRef>,
    compound_dest: Option<DbRef>,
) {
    match out {
        Ok(v) => match (ret, v) {
            (Kind::Void, _) => {}
            (Kind::Int, Value::Int(i)) => stores.put::<i64>(stack, i),
            (Kind::Bool, Value::Bool(b)) => stores.put(stack, b),
            (Kind::Single, Value::Float(f)) => stores.put::<f32>(stack, f as f32),
            (Kind::Text, Value::Text(s)) => {
                deliver_text(stores, stack, text_dest, work_buf, &s);
            }
            // loft#1061 — a text answer too large for the frame travelled in the
            // return arena instead, and arrives as the handle to it.  The return KIND
            // is what distinguishes this from a compound answer, which is why it needs
            // no wire tag of its own; the arena is still bound here, so the bytes are
            // readable, and copying them out first keeps the borrow honest.  Where it
            // lands from there is the same question as for any other text answer.
            (Kind::Text, Value::Ref(handle)) if handle.pos == super::wire::TEXT_IN_ARENA => {
                let parked = stores.store(&handle).get_str(handle.rec).to_string();
                deliver_text(stores, stack, text_dest, work_buf, &parked);
            }
            // A compound answer was built in the return arena. Where it lands is
            // whatever the caller offered — and the caller offers one of exactly
            // two things, the same pair an in-process call sees:
            //
            //   * a live destination (`v = range_vec(4)` materialises `__ref_2`
            //     at function entry, and `v` BORROWS it) — fill it, hand it back;
            //   * nothing, a null placeholder (`p = make_point(3,4)`) — in-process
            //     the callee mints its own store and hands ownership over, so mint
            //     one here and hand that over. The caller's `OpFreeRef` frees it.
            (Kind::Compound { tp, .. }, Value::Ref(src)) => {
                let dest = compound_dest.expect("a compound return always has a destination");
                if !src.is_null() {
                    super::arena::copy_value(stores, &dest, &src, tp);
                }
                super::arena::trace(stores, "ret-src", &src, tp);
                super::arena::trace(stores, "ret-dst", &dest, tp);
                stores.put(stack, dest);
            }
            (k, v) => panic!("placed call returned {v:?} where the signature declares {k:?}"),
        },
        // The only failure that reaches here is a malformed answer: a worker that
        // faulted or died is handled before either arena is bound.
        Err(e) => fault(stores, e),
    }
}
