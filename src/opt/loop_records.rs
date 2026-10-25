// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-LoopRecord` — the record locals a loop builds afresh on every pass, which a rewrite
//! may build once and refill: asked by the IR pass `ctor_literal` and by the native generator.

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use crate::ir_shape::call_named;
use std::collections::{HashMap, HashSet};

/// The scalar in-place setters (@PLN157 P4a): each writes one fixed-width value through
/// an address it is GIVEN — the `set_*` family in `Store` — and cannot resize, insert,
/// remove or re-key anything.  No record moves and no collection's length changes, so
/// every hoisted [`crate::vector::VecHeader`] stays valid across one, aliased or not: an
/// in-place write moves nothing, so there is nothing an alias could observe stale.
/// (Hoisted scalar VALUES would be a different question — offset-keyed invalidation —
/// but this pass hoists headers only.)  Deliberately scalar-only: `OpSetRef`/`OpSetDbRef`
/// hand records to owners, `OpSetText` re-allocates, `OpSetKeyed` re-keys — excluded,
/// and the allow-list doctrine holds: an op missing here costs the hoist, never
/// correctness.
/// Enforces `@FR-R-InPlace` (formal/rewrites.md): the allow-list of fixed-width scalar sets.
pub const IN_PLACE_SET_OPS: [&str; 12] = [
    "OpSetBoolean",
    "OpSetInt",
    "OpSetInt4",
    "OpSetInt4Raw",
    "OpSetCharacter",
    "OpSetSingle",
    "OpSetFloat",
    "OpSetByte",
    "OpSetByteNullable",
    "OpSetShort",
    "OpSetShortRaw",
    "OpSetEnum",
];

/// The typed field getters whose value is a plain `Copy` scalar in the emitted Rust — the
/// reads @PLN157 P4c may hoist.  Each `#rust` template answers its kind's null sentinel at
/// `rec == 0` and a typed load otherwise, and the prelude evaluates that SAME template
/// once, so the hoisted local holds exactly what a per-iteration read would; nothing here
/// needs a per-kind table.  Reference, text and field-address getters stay out — a `DbRef`
/// or a `Str` is a view, not a value.
pub const SCALAR_GETTERS: [&str; 14] = [
    "OpGetInt",
    "OpGetInt4",
    "OpGetInt4Raw",
    "OpGetInt4Full",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetShortFull",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetSingle",
    "OpGetFloat",
    "OpGetBoolean",
    "OpGetCharacter",
    "OpGetEnum",
];

/// Is `def_nr` a record type with no heap in it — every field a fixed-width scalar (a
/// constant and a routine field hold no store either)?  The record's store then hosts
/// nothing a hoisted header could name: the fact `@FR-R-Callee`'s return-buffer half and
/// § V-ad's discharge buffer both stand on.
pub(crate) fn all_scalar_record(data: &Data, def_nr: u32) -> bool {
    data.def(def_nr).attributes().iter().all(|a| {
        a.constant || matches!(a.typedef.base(), Type::Routine(_)) || is_scalar(&a.typedef)
    })
}

/// True for the types that cannot name a store. Anything else — a reference, a collection,
/// text, a tuple, an iterator, an unresolved type — counts as one that can.
pub(crate) fn is_scalar(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Enum(_, false, _)
    )
}

/// Do the statements FOLLOWING index `at` in `ops` cover every field position of `tp`
/// with contiguous `OpSet*`s on `target` (@PLN157 § V-y)?  Coverage is by BYTE OFFSET
/// against the schema's field list, the variant tag included (its field sits at
/// position 0 and the literal's `OpSetEnum` writes it).  A field the run never reaches
/// — a nested struct arriving by `OpCopyRecord`, a vector field bound by an append —
/// leaves the group incomplete, and the prefill stays: the check can only DECLINE the
/// elision, never miscompile it.
pub(crate) fn group_covers_type(
    ops: &[Value],
    at: usize,
    target: u16,
    tp: u16,
    data: &Data,
    stores: &Stores,
    fresh: &HashMap<usize, u16>,
) -> bool {
    if (tp as usize) >= stores.types.len() {
        return false;
    }
    let (crate::database::Parts::Struct(fields) | crate::database::Parts::EnumValue(_, fields)) =
        &stores.types[tp as usize].parts
    else {
        return false;
    };
    if fields.is_empty() {
        return false;
    }
    let mut written: HashSet<u16> = HashSet::new();
    // An inline record field written member by member: field position → member positions.
    let mut members: HashMap<u16, HashSet<u16>> = HashMap::new();
    for stmt in ops.iter().skip(at) {
        match stmt.unspan() {
            Value::Line(_) => {}
            // A bind of another variable whose value never names the record — the value a
            // later set stores, `__lift = f(…)` — reads nothing the prefill would write.
            Value::Set(x, rhs)
                if *x != target
                    && !rhs.any_node(&mut |n| matches!(n, Value::Var(u) if *u == target)) => {}
            // The declaration's own bind (`v = OpGetField(target, 0, tp)`) interleaves
            // a VECTOR store's group between the OpDatabase and the length reset — a
            // member that covers nothing.  (The collection-field prefill is ONE u32
            // zero at the field position, exactly the group's `OpSetInt4`'s width, so
            // admitting the shape is width-equal, not width-blind.)
            Value::Set(_, x)
                if matches!(x.unspan(), Value::Call(d, cargs)
                    if (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpGetField"
                        && matches!(cargs.first().map(Value::unspan),
                            Some(Value::Var(u)) if *u == target)) => {}
            // A WHOLE-record copy into the element (`self.items += [p]`, `p` a value of the
            // element type delivered field by field) writes every field: it covers the type
            // when the element owns no heap.  A heap-owning element is covered too when the
            // copy is flagged FRESH (`keys::COPY_FRESH_DEST`) for this very type: such a copy
            // never reads its destination — no `remove_claims`, then `copy_block` over every
            // byte and fresh handles from `copy_claims`, or every byte zeroed for a null
            // source (`Stores::copy_null_into`) — so the prefill's zeroed handles are read by
            // nothing.  Unflagged, the copy releases what the destination holds, and only the
            // prefill makes that nothing.
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpCopyRecord"
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Var(w)) if *w == target) =>
            {
                let fresh_same_type = matches!(args.get(2).map(Value::unspan),
                    Some(Value::Int(n)) if u16::try_from(*n).is_ok_and(|n|
                        n & crate::keys::COPY_FRESH_DEST != 0
                            && n & crate::keys::COPY_TP_MASK == tp));
                return fresh_same_type || !stores.owns_heap(tp);
            }
            // A record copy into one of the element's FIELDS writes that field whole: its
            // null arm writes the field's default (`CompleteWrites::fresh_fields`, the same
            // scan's members), a present source every byte.  A field type that owns heap is
            // covered only by a FRESH copy, for the reason the whole-record arm gives.
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpCopyRecord"
                    && let Some(f) =
                        args.get(1).and_then(|a| call_named(a, data, "OpGetField"))
                    && matches!(f.first().map(Value::unspan), Some(Value::Var(w)) if *w == target) =>
            {
                let (Some(Value::Int(off)), Some(Value::Int(n))) =
                    (f.get(1).map(Value::unspan), args.get(2).map(Value::unspan))
                else {
                    break;
                };
                let (Ok(off), Ok(n)) = (u16::try_from(*off), u16::try_from(*n)) else {
                    break;
                };
                let sub = n & crate::keys::COPY_TP_MASK;
                let field = fields.iter().find(|fd| fd.position == off);
                if field.is_none_or(|fd| fd.content != sub)
                    || (n & crate::keys::COPY_FRESH_DEST == 0
                        && !fresh.contains_key(&(args.as_ptr() as usize)))
                    || (stores.owns_heap(sub) && n & crate::keys::COPY_FRESH_DEST == 0)
                {
                    break;
                }
                written.insert(off);
            }
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                if name.starts_with("OpSet")
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == target)
                    && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
                    && let Ok(off) = u16::try_from(*off)
                {
                    written.insert(off);
                } else if name.starts_with("OpSet")
                    && let Some(f) = args.first().and_then(|a| call_named(a, data, "OpGetField"))
                    && matches!(f.first().map(Value::unspan), Some(Value::Var(w)) if *w == target)
                    && let Some(Value::Int(off)) = f.get(1).map(Value::unspan)
                    && let Ok(off) = u16::try_from(*off)
                    && let Some(Value::Int(m)) = args.get(1).map(Value::unspan)
                    && let Ok(m) = u16::try_from(*m)
                {
                    members.entry(off).or_default().insert(m);
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    fields.iter().all(|f| {
        written.contains(&f.position)
            || members.get(&f.position).is_some_and(|m| {
                let sub = f.content as usize;
                sub < stores.types.len()
                    && matches!(&stores.types[sub].parts,
                        crate::database::Parts::Struct(sf)
                            if !sf.is_empty() && sf.iter().all(|x| m.contains(&x.position)))
            })
    })
}

pub(crate) fn refillable_plain(stores: &Stores, tp: u16) -> bool {
    let Some(t) = stores.types.get(tp as usize) else {
        return false;
    };
    let crate::database::Parts::Struct(fields) = &t.parts else {
        return false;
    };
    !fields.is_empty()
        && fields.iter().all(|f| {
            !stores.owns_heap(f.content)
                || matches!(stores.types.get(f.content as usize).map(|c| &c.parts),
                    Some(crate::database::Parts::Vector(e)) if !stores.owns_heap(*e))
        })
}

/// The type `buf`'s mint names in `def_nr` — the first `OpDatabase(buf, tp)` found.
pub fn mint_type_of(data: &Data, def_nr: u32, buf: u16) -> Option<u16> {
    let mut tp = None;
    data.def(def_nr).code().any_node(&mut |n| {
        if let Some([b, Value::Int(t)]) = call_named(n, data, "OpDatabase")
            && is_var(b, buf)
        {
            tp = u16::try_from(*t).ok();
        }
        tp.is_some()
    });
    tp
}

pub(crate) fn is_var(v: &Value, var: u16) -> bool {
    matches!(v.unspan(), Value::Var(x) if *x == var)
}

/// `@FR-R-LoopRecord` — one admitted loop record: the loop that owns the reuse and the body
/// block that declares the local (whose `Set(v, null)` and end-of-body `OpFreeRef(v)` the
/// emitter drops).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRecord {
    /// The `Loop` block's scope: the local is declared at its prelude and freed after it.
    pub loop_scope: u16,
    /// The block whose direct `Set(v, null)` declares the local per iteration.
    pub decl_block: u16,
    /// The REFILL clause: a record whose vector fields hold no-heap elements.  Its per-pass
    /// mint keeps the store as it stands (`OpDatabaseRefill`) and each literal zero of a
    /// vector field empties that vector in place, so the vectors keep their records.
    pub refill: bool,
}

/// [`loop_records`]' answer: the admitted locals, and the literal zeros of the refill-clause
/// records' vector fields (by node address) — the empties `emit.rs` writes for
/// `@FR-R-RefillBuffer` too.
#[derive(Default)]
pub struct LoopRecords {
    pub recs: HashMap<u16, LoopRecord>,
    pub field_zeros: HashSet<usize>,
}

/// The byte offsets of `tp`'s vector fields.
pub(crate) fn vector_field_offsets(stores: &Stores, tp: u16) -> Vec<i32> {
    let Some(crate::database::Parts::Struct(fields)) =
        stores.types.get(tp as usize).map(|t| &t.parts)
    else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|f| {
            matches!(
                stores.types.get(f.content as usize).map(|c| &c.parts),
                Some(crate::database::Parts::Vector(_))
            )
        })
        .map(|f| i32::from(f.position))
        .collect()
}

/// The field NUMBERS of `tp`'s vector fields — what `OpNewRecord` / `OpFinishRecord` name
/// an append's target field by.
pub(crate) fn vector_field_numbers(stores: &Stores, tp: u16) -> Vec<i32> {
    let Some(crate::database::Parts::Struct(fields)) =
        stores.types.get(tp as usize).map(|t| &t.parts)
    else {
        return Vec::new();
    };
    (0i32..)
        .zip(fields.iter())
        .filter(|(_, f)| {
            matches!(
                stores.types.get(f.content as usize).map(|c| &c.parts),
                Some(crate::database::Parts::Vector(_))
            )
        })
        .map(|(nr, _)| nr)
        .collect()
}

/// The REFILL clause's group check: every mint of `v` heads a literal group that writes
/// every field of `tp` ([`group_covers_type`]), and every zero of a vector field of `v` is
/// one of those groups' — answered as the set of those zeros, or `None`.  A zero anywhere
/// else would drop a kept vector's record into the store, where the clear arm it replaces
/// reclaimed it.
pub(crate) fn loop_record_zeros(
    body: &Value,
    v: u16,
    tp: u16,
    data: &Data,
    stores: &Stores,
) -> Option<HashSet<usize>> {
    let offs = vector_field_offsets(stores, tp);
    let mut ok = true;
    let mut mints = 0usize;
    let mut zeros: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n {
            let ops = &bl.operators;
            for (i, stmt) in ops.iter().enumerate() {
                let Some(Value::Call(d, args)) = Some(stmt.unspan()) else {
                    continue;
                };
                if (*d as usize) >= data.definitions.len()
                    || !matches!(data.def(*d).name(), "OpDatabase" | "OpDatabaseNP")
                    || !matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == v)
                {
                    continue;
                }
                mints += 1;
                if !matches!(args.get(1).map(Value::unspan), Some(Value::Int(t)) if *t == i32::from(tp))
                    || !group_covers_type(ops, i + 1, v, tp, data, stores, &HashMap::new())
                {
                    ok = false;
                    continue;
                }
                for g in &ops[i + 1..] {
                    let Value::Call(d, args) = g.unspan() else {
                        if matches!(g.unspan(), Value::Line(_)) {
                            continue;
                        }
                        break;
                    };
                    if (*d as usize) >= data.definitions.len()
                        || !data.def(*d).name().starts_with("OpSet")
                    {
                        break;
                    }
                    if data.def(*d).name() == "OpSetInt4"
                        && let [a0, Value::Int(off), zero] = &args[..]
                        && matches!(a0.unspan(), Value::Var(w) if *w == v)
                        && matches!(zero.unspan(), Value::Int(0))
                        && offs.contains(off)
                    {
                        zeros.insert(std::ptr::from_ref(g.unspan()) as usize);
                    }
                }
            }
        }
        false
    });
    // Every vector-field zero of `v` must be one the groups own.
    body.any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name().starts_with("OpSet")
            && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == v)
            && matches!(args.get(1).map(Value::unspan), Some(Value::Int(off)) if offs.contains(off))
            && !zeros.contains(&(std::ptr::from_ref(n) as usize))
        {
            ok = false;
        }
        false
    });
    (ok && mints > 0 && zeros.len() == mints * offs.len()).then_some(zeros)
}

/// `@FR-R-LoopRecord` (the `@FR-R-LoopBuffer` shape for a RECORD) — the loop records of
/// `def_nr`: a plain no-heap record local declared and minted by a literal INSIDE a loop
/// (`sub = Spec {…}` per iteration), keyed by variable.  Such a local's store dies with the
/// iteration today — `OpFreeRef` at the body's end, a fresh store minted next pass — where a
/// local declared OUTSIDE the loop already keeps its store (`OpDatabase` on a var that still
/// holds one clears it and claims the record again).  The emitter gives the body-declared
/// local the same life: declared at the loop's prelude, freed once after the loop, its
/// per-pass mint taking the clear arm.  Observably the same record as a fresh one — every
/// field is written by the literal or by the prefill before it is read.
///
/// Admitted where the local's every mention is its init family — the declaration
/// `Set(v, null)`, the mint `OpDatabase*(v, tp)`, a fusable scalar field read or in-place
/// write, a free — or a hand-off as an argument to a loft-bodied callee whose return borrows
/// nothing (`(O-Borrow)`: a callee can keep a caller's record only by copying it or through
/// its return's deps).  Declined: a heap-owning or nullable type, a second `Set` (a copy to
/// another local, a rebind), a `return` of it, a capture, a native op taking it otherwise, a
/// declaration outside any loop, and — as for the loop buffers — a body with `yield` or `par`.
/// The fallback is "not a loop record", which costs the reuse and never a value.
#[must_use]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn loop_records(data: &Data, stores: &Stores, def_nr: u32) -> LoopRecords {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let mut out = LoopRecords::default();
    if matches!(body, Value::Null)
        || body.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_)))
    {
        return out;
    }
    let trace = std::env::var("LOFT_TRACE_LOOP_RECORD").is_ok();
    let refill_on = !std::env::var("LOFT_NO_LOOP_RECORD_REFILL").is_ok_and(|v| v != "0");
    // THE NEST CLAUSE: the record is kept across the OUTERMOST loop around its declaration,
    // not the innermost — a nest re-entering its inner loop per outer pass otherwise frees and
    // mints the record once per entry (`forms_upto`: once per `ea`, a thousand times a call).
    // Every pass of any loop in the nest reaches the declaration's mint, which takes the clear
    // arm, so the argument is the innermost one's.  `LOFT_NO_LOOP_RECORD_NEST` keeps the
    // innermost.
    let nest = !std::env::var("LOFT_NO_LOOP_RECORD_NEST").is_ok_and(|v| v != "0");
    // Where each candidate is declared: the loop that owns it and the block holding its
    // `Set(v, null)`.
    let mut decls: HashMap<u16, LoopRecord> = HashMap::new();
    fn walk(
        v: &Value,
        loop_scope: Option<u16>,
        block: Option<u16>,
        nest: bool,
        f: &mut impl FnMut(&Value, Option<u16>, Option<u16>),
    ) {
        let n = v.unspan();
        f(n, loop_scope, block);
        let (ls, bs) = match n {
            // `@FR-R-TailSelf`'s loop is the function body made a loop: it never falls through,
            // so "the one free after the loop" has no place to run, and a record declared in
            // it is declared per pass as it was — the loop is transparent here.
            Value::Loop(lp) if lp.name == "tail_self" => (loop_scope, block),
            Value::Loop(lp) if nest && loop_scope.is_some() => (loop_scope, Some(lp.scope)),
            Value::Loop(lp) => (Some(lp.scope), Some(lp.scope)),
            Value::Block(b) => (loop_scope, Some(b.scope)),
            _ => (loop_scope, block),
        };
        n.for_each_child(&mut |c| walk(c, ls, bs, nest, f));
    }
    walk(body, None, None, nest, &mut |n, ls, bs| {
        if let Value::Set(v, rhs) = n
            && matches!(rhs.unspan(), Value::Null)
            && let (Some(loop_scope), Some(decl_block)) = (ls, bs)
            && !vars.name(*v).starts_with("__")
            && !vars.is_captured(*v)
            && !matches!(vars.tp(*v), Type::Optional(_))
            && let Type::Reference(d, _) = vars.tp(*v).peel_link()
            && data.def_type(*d) == DefType::Struct
        {
            decls.entry(*v).or_insert(LoopRecord {
                loop_scope,
                decl_block,
                refill: !all_scalar_record(data, *d),
            });
        }
    });
    for (v, mut rec) in decls {
        let mut minted = false;
        let mut declined: Option<&'static str> = None;
        // The REFILL clause: the store type the mints name, refillable, every mint heading a
        // complete literal group.  The vector fields' own ops are then part of the init family.
        let mut vec_offs: Vec<i32> = Vec::new();
        let mut vec_nrs: Vec<i32> = Vec::new();
        let mut rec_tp: i32 = -1;
        let mut zeros: HashSet<usize> = HashSet::new();
        if rec.refill {
            let tp = mint_type_of(data, def_nr, v);
            match tp {
                Some(tp) if refill_on && refillable_plain(stores, tp) => {
                    match loop_record_zeros(body, v, tp, data, stores) {
                        Some(z) => {
                            zeros = z;
                            vec_offs = vector_field_offsets(stores, tp);
                            vec_nrs = vector_field_numbers(stores, tp);
                            rec_tp = i32::from(tp);
                        }
                        None => declined = Some("a mint whose literal group the refill cannot see"),
                    }
                }
                _ => declined = Some("a heap-owning type"),
            }
        }
        // Every mention of `v`, judged by its parent.
        fn check(
            n: &Value,
            v: u16,
            vec: (&[i32], &[i32], i32),
            data: &Data,
            minted: &mut bool,
            declined: &mut Option<&'static str>,
        ) {
            let (vec_offs, vec_nrs, rec_tp) = vec;
            let n = n.unspan();
            match n {
                Value::Var(w) if *w == v => {
                    // A bare mention reaching here was not consumed by an accounted parent.
                    declined.get_or_insert("a mention outside its init family");
                    return;
                }
                Value::Set(w, rhs) if *w == v => {
                    if !matches!(rhs.unspan(), Value::Null) {
                        declined.get_or_insert("a second binding");
                    }
                    return;
                }
                Value::Set(_, rhs) if matches!(rhs.unspan(), Value::Var(w) if *w == v) => {
                    declined.get_or_insert("copied to another local");
                    return;
                }
                Value::Return(val) if matches!(val.unspan(), Value::Var(w) if *w == v) => {
                    declined.get_or_insert("returned");
                    return;
                }
                Value::Call(g, args) if (*g as usize) < data.definitions.len() => {
                    let name = data.def(*g).name();
                    let first =
                        matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == v);
                    let native = matches!(data.def(*g).code(), Value::Null)
                        || !data.def(*g).rust().is_empty();
                    // The REFILL clause: a vector field read in place as a native op's
                    // operated-on vector (`OpPushInt(OpGetField(v, off, tp), x)`, a length, an
                    // element read) names the record only for that op.
                    let on_vec_field = native
                        && name != "OpGetField"
                        && matches!(args.first().map(Value::unspan), Some(Value::Call(gf, ga))
                            if (*gf as usize) < data.definitions.len()
                                && data.def(*gf).name() == "OpGetField"
                                && matches!(ga.first().map(Value::unspan), Some(Value::Var(w)) if *w == v)
                                && matches!(ga.get(1).map(Value::unspan), Some(Value::Int(off)) if vec_offs.contains(off)));
                    if on_vec_field {
                        for a in args.iter().skip(1) {
                            check(a, v, vec, data, minted, declined);
                        }
                        return;
                    }
                    if first && native {
                        let fld_lit = matches!(args.get(1).map(Value::unspan), Some(Value::Int(_)));
                        let ok = match name {
                            "OpDatabase" | "OpDatabaseNP" => {
                                *minted = true;
                                true
                            }
                            "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag" => true,
                            // The refill clause: an append to one of the record's own vector
                            // fields, named by field number (`e.v += [R {…}]`).
                            "OpNewRecord" | "OpNewRecordNP" | "OpFinishRecord" => {
                                let at = if name == "OpFinishRecord" { 2 } else { 1 };
                                matches!(args.get(at).map(Value::unspan), Some(Value::Int(t)) if *t == rec_tp)
                                    && matches!(args.get(at + 1).map(Value::unspan), Some(Value::Int(f)) if vec_nrs.contains(f))
                            }
                            _ => {
                                fld_lit
                                    && (SCALAR_GETTERS.contains(&name)
                                        || IN_PLACE_SET_OPS.contains(&name))
                            }
                        };
                        if !ok {
                            declined.get_or_insert("a native op takes it otherwise");
                            return;
                        }
                        // The other operands still walk (a value may mention `v` again).
                        for a in args.iter().skip(1) {
                            check(a, v, vec, data, minted, declined);
                        }
                        return;
                    }
                    if !native {
                        // A loft callee: `v` may be handed as any argument whose PARAMETER the
                        // return does not borrow (`(O-Borrow)`: the return type names the
                        // parameters it aliases by index — a record-returning callee names its
                        // hidden buffer there too, which is not a borrow of `v`).
                        let deps = data.def(*g).returned().depend();
                        let borrowed = args.iter().enumerate().any(|(i, a)| {
                            matches!(a.unspan(), Value::Var(w) if *w == v)
                                && u16::try_from(i).is_ok_and(|p| deps.contains(&p))
                        });
                        if borrowed {
                            declined.get_or_insert("handed to a callee whose return borrows it");
                            return;
                        }
                        for a in args {
                            if !matches!(a.unspan(), Value::Var(w) if *w == v) {
                                check(a, v, vec, data, minted, declined);
                            }
                        }
                        return;
                    }
                }
                _ => {}
            }
            n.for_each_child(&mut |c| check(c, v, vec, data, minted, declined));
        }
        check(
            body,
            v,
            (&vec_offs, &vec_nrs, rec_tp),
            data,
            &mut minted,
            &mut declined,
        );
        if !minted {
            declined.get_or_insert("never minted by a literal");
        }
        match declined {
            None => {
                if trace {
                    eprintln!(
                        "[loop-record] {}: {} keeps its store{} across iterations of loop {}",
                        def.name(),
                        vars.name(v),
                        if rec.refill { " and its vectors" } else { "" },
                        rec.loop_scope
                    );
                }
                rec.refill = rec.refill && !zeros.is_empty();
                out.field_zeros.extend(zeros);
                out.recs.insert(v, rec);
            }
            Some(why) => {
                if trace {
                    eprintln!(
                        "[loop-record] {}: {} declines: {why}",
                        def.name(),
                        vars.name(v)
                    );
                }
            }
        }
    }
    out
}
