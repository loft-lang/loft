// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Hidden RETURN BUFFERS minted once: a call's record buffer allocated once and refilled, and a
//! vector return buffer minted lazily in front of its first use.

use crate::data::{Block, Data, Type, Value, v_if, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// @PLN157 § V (Route R, caller half) — allocate a call's hidden RECORD buffer ONCE, so a
/// callee that builds its return into the buffer it was handed reuses one record per call
/// SITE instead of minting a store per CALL.
///
/// The buffer is `__ref_N`, declared `Set(av, Null)` in the body preamble and passed to the
/// call; `OpDatabase` right after that null-init is the same pair `parse_object`'s in-place
/// arm and `gen_set_first_vector_null`'s vector twin already emit, so both backends lower it
/// from the IR and neither generator needs to know why.
///
/// Enforces `@FR-R-Reuse`; the witness it reads is `@FR-O-Buffer`.
///
/// **The gate is `witness_buffer`, and it is the whole soundness argument.**  An allocated
/// buffer outlives the call, so a call site that frees the RESULT with a plain `OpFreeRef`
/// releases the buffer's store — and the next turn of the loop writes a record that is back
/// in the pool.  `witness_buffer` names exactly the sites where @P378(a) already made the
/// result's free `OpFreeRefIfDistinct(v, av)`, which declines precisely when the callee
/// handed the buffer back.  A buffer reached any other way (a `__lift_N` temp holding the
/// result of `keep += [mk(i)]` has a plain free) is left null and keeps its mint-per-call.
///
/// Also required: the buffer is USED ONCE, and its result local is ASSIGNED ONCE.  A
/// work-ref the parser reused at a second site would have one guarded use and one that is
/// not, and this reads the guarded one alone.  A result local assigned again (`v = mk(i);
/// v = other`) releases the store it displaces as an owned one — that free is emitted by
/// each backend's set lowering, not by this scan, and under reuse the displaced store is
/// the buffer's: the next call then writes a store that is back in the pool (measured, a
/// use-after-free on every turn after the first).  Guarding that free against the buffer is
/// the widening that lifts this condition; until then the buffer stays null there.  The
/// nulls in front of a local's one real bind on every pass (`null_led_first_binds_in`) are
/// not such a second assignment: they displace nothing.
/// The calls at the VALUE positions of a branch (@PLN157 § V-af, `@FR-O-Buffer`): an `if`'s two arms, a
/// value block's last statement, recursively; a `Call` is its own tail.  Anything else — a
/// variable, a literal, a null — contributes no call.
pub(super) fn tail_calls(v: &Value) -> Vec<&Value> {
    match v.unspan() {
        Value::Call(_, _) => vec![v.unspan()],
        Value::If(_, a, b) => {
            let mut out = tail_calls(a);
            out.extend(tail_calls(b));
            out
        }
        Value::Block(bl) => bl.operators.last().map_or_else(Vec::new, tail_calls),
        _ => Vec::new(),
    }
}

#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn reuse_record_buffers(
    code: &mut Value,
    function: &mut Function,
    data: &Data,
    fn_nr: u32,
    witness_buffer: &HashMap<u16, Vec<u16>>,
    minted_pairs: &HashSet<u16>,
    reassigned: &HashSet<u16>,
) {
    if !crate::keys::retbuf_reuse_enabled() {
        return;
    }
    let Some(bl) = body_block_mut(code) else {
        return;
    };
    let ungated = crate::keys::retbuf_witness_gate_disabled();
    let mut guarded: Vec<u16> = if ungated {
        // The positive control: every hidden buffer, guarded or not.
        (0..function.count()).collect()
    } else {
        witness_buffer.values().flatten().copied().collect()
    };
    guarded.sort_unstable();
    guarded.dedup();
    // Every result local a buffer feeds — `witness_buffer` maps the other way round.
    let mut fed_locals: HashMap<u16, Vec<u16>> = HashMap::default();
    for (&v, bufs) in witness_buffer {
        for &av in bufs {
            fed_locals.entry(av).or_default().push(v);
        }
    }
    let db_nr = data.def_nr("OpDatabase");
    let clear_nr = data.def_nr("OpClear");
    // Emitted in variable order so identical source compiles to identical IR.
    let mut eager: Vec<(u16, Value, Option<Value>)> = Vec::new();
    let mut lazy: Vec<(u16, Value, Option<Value>)> = Vec::new();
    // `LOFT_TRACE_POOL=1` names the gate that keeps each witnessed buffer out of the pool.
    let trace = crate::env_once!(std::env::var_os("LOFT_TRACE_POOL").is_some());
    if trace {
        crate::loft_eprintln!("[pool] {} candidates {:?}", data.def(fn_nr).name(), guarded);
    }
    let decline = |av: u16, why: &str| {
        if trace {
            crate::loft_eprintln!(
                "[pool] {} {}: {why}",
                data.def(fn_nr).name(),
                function.name(av)
            );
        }
    };
    for av in guarded {
        // @FR-O-Proxy asks alloc — decides whether to ALLOCATE the buffer's store here; a
        // buffer carrying a dep is a view of something else and gets no store of its own.
        // The release is not this site's: the scan already placed the buffer's scope-exit
        // free and the result's guarded one.
        if !function.is_caller_hidden_buf(av) || !function.tp(av).depend().is_empty() {
            decline(av, "not an owned caller buffer");
            continue;
        }
        // @PLN164 B1 — a buffer whose callee mints the store its result adopts is paired
        // for the guarded free only; pre-minting it hands the callee a store its own rebind
        // frees on the interpreter (`Scopes::minted_pairs`).
        if !ungated && !crate::keys::adopt_buffer_reuse_enabled() && minted_pairs.contains(&av) {
            decline(av, "the result adopts the callee's mint");
            continue;
        }
        let Some(td) = function.tp(av).base().heap_def_nr() else {
            decline(av, "not a record");
            continue;
        };
        let known = data.def(td).known_type();
        if known == u16::MAX {
            decline(av, "the record type has no layout");
            continue;
        }
        if !ungated && buffer_call_uses(&bl.operators, av, data) != 1 {
            // A work-ref the parser handed to a SECOND call has one guarded use and one
            // this has not looked at; `witness_buffer` names the guarded one either way.
            decline(av, "handed to more than one call");
            continue;
        }
        if !ungated
            && fed_locals
                .get(&av)
                .is_some_and(|vs| vs.iter().any(|v| reassigned.contains(v)))
        {
            // The result local is reassigned somewhere: its set lowering frees the store
            // it displaces, which would be this buffer's.
            decline(av, "its result local is assigned more than once");
            continue;
        }
        if null_init_at(&bl.operators, av).is_none() {
            decline(av, "no top-level null-init");
            continue;
        }
        let mint = Value::Call(db_nr, vec![Value::Var(av), Value::Int(i32::from(known))]);
        // `@FR-H-ClearRelease`, the record clause — a reused buffer is REFILLED: the
        // callee's literal overwrites every handle it writes, so what the previous call left
        // in the record is released before each call after the first.
        let release = releases_what_it_held(data, function.tp(av))
            .then(|| Value::Call(clear_nr, vec![Value::Var(av), Value::Int(i32::from(known))]));
        if !ungated && crate::keys::lazy_buffer_enabled() {
            // `@FR-O-LazyBuffer` — once per activation still, but only on a path that
            // reaches the call: the null test lets a later pass through a loop reuse it,
            // and a reuse takes the release.
            lazy.push((av, mint, release));
        } else {
            eager.push((av, mint, release));
        }
    }
    let frees = free_ops(data);
    crate::rewrite_census::fired("R-Reuse", eager.len() + lazy.len());
    // A buffer minted at entry is live at every call, and the first release finds the
    // record the mint just prefilled: nothing to walk.  The releases go in before the mints
    // do, because a mint names the buffer and would itself take one.
    for (av, _, release) in &eager {
        if let Some(release) = release {
            insert_before_uses(&mut bl.operators, *av, release, &frees);
        }
    }
    for (av, mint, _) in eager {
        if let Some(at) = null_init_at(&bl.operators, av) {
            bl.operators.insert(at + 1, mint);
        }
    }
    let is_null = data.def_nr("OpRefIsNull");
    for (av, mint, release) in lazy {
        // The mark tells the native hoist gate that this `OpDatabase` only ever takes a
        // fresh store from the sentinel (`hoist::lazy_buffer_mint`); a record buffer's
        // null-init already writes the sentinel on both backends.
        function.mark_lazy_buffer(av);
        let guard = v_if(
            Value::Call(is_null, vec![Value::Var(av)]),
            Value::Insert(vec![mint]),
            release.unwrap_or(Value::Null),
        );
        insert_before_uses(&mut bl.operators, av, &guard, &frees);
    }
}

/// The function body's own statement block, which is where a statement that must run
/// first is prepended and where a top-level null-init stands.
///
/// A body whose result is a reference the scan HOISTED to the frame arrives as
/// `Insert([Set(w, null), Block])` (the `hoisted_ref` arm of `scan_inner`), so the block is
/// found inside that wrapper too, and a `Span` (position only) is peeled; any other shape is
/// not a body this pass rewrites.  Matching a bare `Block` alone skipped every such function
/// in silence — the pool, the lazy mints and the entry-time flag and witness initialisers
/// alike.
pub(super) fn body_block_mut(code: &mut Value) -> Option<&mut Block> {
    match code {
        Value::Span(b) => body_block_mut(&mut b.1),
        Value::Block(bl) => Some(&mut **bl),
        Value::Insert(ops) => match ops.as_mut_slice() {
            [Value::Set(_, init), Value::Block(bl)] if matches!(**init, Value::Null) => {
                Some(&mut **bl)
            }
            _ => None,
        },
        _ => None,
    }
}

/// The top-level position of `av`'s null-init, where an eager mint goes right after it.
fn null_init_at(ops: &[Value], av: u16) -> Option<usize> {
    ops.iter()
        .position(|op| matches!(op.unspan(), Value::Set(s, v) if *s == av && **v == Value::Null))
}

/// Can a record of this buffer type own heap — so a refill of it owes a release of what it
/// held?  A struct with a field that is not a scalar can, and so can a struct-enum, whose
/// variants this does not read.
///
/// Conservative on purpose, and the fallback says why: a record this answers `true` for
/// that owns nothing pays one walk that returns at once, while a `false` for one that
/// owns heap strands the previous occupant's heap on every call.  The release walks the
/// buffer's own type — for a struct-enum the PARENT, whose walk follows the variant the
/// buffer holds rather than the one the next call writes.
fn releases_what_it_held(data: &Data, tp: &Type) -> bool {
    // `.base()`: a buffer's record shape is the same behind a nullability marker
    // (`@FR-N-Shape`).
    match tp.base() {
        Type::Reference(td, _) => !data
            .def(*td)
            .attributes()
            .iter()
            .all(|a| crate::data::is_scalar(&a.typedef)),
        Type::Enum(_, true, _) => true,
        _ => false,
    }
}

/// The ops that name a store only to release it or to compare its identity: a buffer
/// mentioned in nothing else is never read, so it needs no store.
fn free_ops(data: &Data) -> Vec<u32> {
    [
        "OpFreeRef",
        "OpFreeRefIfDistinct",
        "OpFreeRefOrHandUp",
        "OpFreeRefTag",
        "OpStoreTag",
        "OpDistinctStore",
    ]
    .iter()
    .map(|n| data.def_nr(n))
    .filter(|&d| d != u32::MAX)
    .collect()
}

/// Does `v` name `av` anywhere but inside a free (`free_ops`)?  A `Set(av, Null)` — the
/// buffer's own null-init or an already inserted mint — is not a use either.  Every other
/// shape is walked, so a buffer reached through an expression the walker does not name is
/// still found: the fallback over-reports, and an extra mint on a path that runs the call
/// anyway is only the eager behaviour.
fn names_outside_free(v: &Value, av: u16, frees: &[u32]) -> bool {
    match v.unspan() {
        Value::Var(x) => *x == av,
        Value::Call(d, _) if frees.contains(d) => false,
        Value::Set(x, val) if *x == av && matches!(val.unspan(), Value::Null) => false,
        other => {
            let mut hit = false;
            other.for_each_child(&mut |c| {
                if !hit && names_outside_free(c, av, frees) {
                    hit = true;
                }
            });
            hit
        }
    }
}

/// Put `guard` in front of every statement of `ops` that uses `av` (`names_outside_free`),
/// descending into the statement lists of blocks, loops, inserts and `if` arms so the
/// guard lands on the innermost list that holds the use.  An `if` whose CONDITION uses
/// `av`, or whose arm is a bare expression that does, takes the guard in front of the
/// whole `if`.
fn insert_before_uses(ops: &mut Vec<Value>, av: u16, guard: &Value, frees: &[u32]) {
    let mut i = 0;
    while i < ops.len() {
        let before = match ops[i].unspan_mut() {
            Value::Block(bl) | Value::Loop(bl) => {
                insert_before_uses(&mut bl.operators, av, guard, frees);
                false
            }
            Value::Insert(ls) => {
                insert_before_uses(ls, av, guard, frees);
                false
            }
            Value::If(cond, a, b) => place_in_if(cond, a, b, av, guard, frees),
            // A value-position `if` — a `match` lowered as the value of a bind or a return
            // — descends the same way: the guard lands in the arm that makes the call, so a
            // buffer for one arm's callee is not minted on every path through the function
            // (measured: cbor's `encode` minted its map arm's four buffers on every call).
            Value::Set(x, inner) if *x != av => match inner.unspan_mut() {
                Value::If(cond, a, b) => place_in_if(cond, a, b, av, guard, frees),
                Value::Block(bl) => {
                    insert_before_uses(&mut bl.operators, av, guard, frees);
                    false
                }
                other => names_outside_free(other, av, frees),
            },
            Value::Return(inner) => match inner.unspan_mut() {
                Value::If(cond, a, b) => place_in_if(cond, a, b, av, guard, frees),
                Value::Block(bl) => {
                    insert_before_uses(&mut bl.operators, av, guard, frees);
                    false
                }
                other => names_outside_free(other, av, frees),
            },
            other => names_outside_free(other, av, frees),
        };
        if before {
            ops.insert(i, guard.clone());
            i += 1;
        }
        i += 1;
    }
}

/// The `if` half of [`insert_before_uses`]: a condition that uses `av` takes the guard in
/// front of the whole `if`; otherwise each arm that is a statement list takes it inside,
/// and a bare-expression arm that uses `av` reports the `if` as a use.
fn place_in_if(
    cond: &mut Value,
    then_arm: &mut Value,
    else_arm: &mut Value,
    av: u16,
    guard: &Value,
    frees: &[u32],
) -> bool {
    if names_outside_free(cond, av, frees) {
        return true;
    }
    let mut bare = false;
    for arm in [then_arm, else_arm] {
        match arm.unspan_mut() {
            Value::Block(bl) => insert_before_uses(&mut bl.operators, av, guard, frees),
            Value::Insert(ls) => insert_before_uses(ls, av, guard, frees),
            Value::If(inner_cond, inner_then, inner_else) => {
                bare |= place_in_if(inner_cond, inner_then, inner_else, av, guard, frees);
            }
            other => bare |= names_outside_free(other, av, frees),
        }
    }
    bare
}

/// Is `av` the buffer of a call a `for` loop ITERATES (`for f in make(…) { … }`)?  Such a
/// buffer is the native emitter's to place (@PLN157 § V-j, `hoist::move_appends`): it starts
/// as the sentinel and is claimed in the destination's store at the loop, so a guarded mint
/// in front of the loop would be a second owner of the same slot.
fn iterates_a_call_into(ops: &[Value], av: u16) -> bool {
    ops.iter().any(|op| {
        op.any_node(&mut |n| {
            let Value::Block(bl) = n else { return false };
            bl.name == "For block"
                && matches!(bl.operators.first().map(Value::unspan),
                    Some(Value::Set(_, call)) if matches!(call.unspan(),
                        Value::Call(_, args) if matches!(args.last().map(Value::unspan),
                            Some(Value::Var(b)) if *b == av)))
        })
    })
}

/// @PLN164 A0 (`@FR-O-LazyBuffer`) — a hidden VECTOR return buffer is minted in front of
/// the statements that hand it to a callee, behind `OpRefIsNull`, instead of at function
/// entry: its null-init writes the sentinel (`Function::mark_lazy_buffer` tells both
/// emitters), and the guarded `Set(av, Null)` is the mint.  A path that never makes the
/// call never mints the store, and every exit's free already tolerates the sentinel
/// (`@FR-H-FreeNull`).  Declined for a body that suspends or forks (a generator, `par`),
/// and for a buffer with a second assignment, which this cannot order.  The type test is
/// on the bare `vector<T>` on purpose: a `vector<T>?` buffer's entry init already writes
/// the ABSENT sentinel and is minted by another route, so it is declined and keeps its
/// entry-time behaviour, as does every buffer this does not name.
pub(super) fn lazy_buffer_mints(code: &mut Value, function: &mut Function, data: &Data) {
    if !crate::keys::lazy_buffer_enabled() {
        return;
    }
    if code.any_node(&mut |v| matches!(v, Value::Yield(..) | Value::Parallel(..))) {
        return;
    }
    let Some(bl) = body_block_mut(code) else {
        return;
    };
    let frees = free_ops(data);
    let is_null = data.def_nr("OpRefIsNull");
    for av in 0..function.count() {
        if !function.is_caller_hidden_buf(av)
            || function.is_argument(av)
            || function.is_inline_ref(av)
            || function.is_skip_free(av)
            || !function.name(av).starts_with("__ref_")
        {
            continue;
        }
        // A `vector<T>?` buffer is minted by another route (see the doc above).
        if matches!(function.tp(av), Type::Optional(_)) {
            continue;
        }
        let Type::Vector(_, dep) = function.tp(av).base() else {
            continue;
        };
        if !dep.is_empty() {
            continue;
        }
        let top_inits = bl
            .operators
            .iter()
            .filter(|op| matches!(op.unspan(), Value::Set(x, v) if *x == av && matches!(v.unspan(), Value::Null)))
            .count();
        let mut sets = 0;
        for op in &bl.operators {
            op.walk(&mut |v| {
                if let Value::Set(x, _) = v
                    && *x == av
                {
                    sets += 1;
                }
            });
        }
        if top_inits != 1 || sets != 1 || iterates_a_call_into(&bl.operators, av) {
            continue;
        }
        function.mark_lazy_buffer(av);
        // `@FR-R-WorkBuffer`'s positive control: a work buffer left at the sentinel makes
        // the callee take its null road, on both backends.
        if crate::keys::work_buffer_null_control() && function.is_work_buffer_ref(av) {
            continue;
        }
        let guard = v_if(
            Value::Call(is_null, vec![Value::Var(av)]),
            Value::Insert(vec![v_set(av, Value::Null)]),
            Value::Null,
        );
        insert_before_uses(&mut bl.operators, av, &guard, &frees);
    }
}

/// How many USER calls in `ops` are handed `av` as an argument.
///
/// The buffer's other mentions are its null-init and the frees the scan just emitted
/// (`OpFreeRefIfDistinct(v, av)` is itself one of them), so a raw occurrence count answers a
/// different question.  Only a loft-defined callee takes a hidden buffer at all, which is
/// what `is_loft_defined` is the one home for.
fn buffer_call_uses(ops: &[Value], av: u16, data: &Data) -> usize {
    let mut n = 0;
    for op in ops {
        op.walk(&mut |v| {
            if let Value::Call(d_nr, args) = v
                && data.def(*d_nr).is_loft_defined()
                && args
                    .iter()
                    .any(|a| matches!(a.unspan(), Value::Var(x) if *x == av))
            {
                n += 1;
            }
        });
    }
    n
}
