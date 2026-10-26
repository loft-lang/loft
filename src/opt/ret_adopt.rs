// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! Two per-function analyses both compiled writers read, moved here from
//! `generation::hoist` unchanged so the C backend can name them without reaching up into the
//! Rust writer: `@FR-R-RetAdopt`'s return-buffer adoption ([`ret_adopt`]) and @PLN157 § V-y's
//! literal-group coverage ([`complete_writes`]).

use crate::data::{Data, Type, Value};
use crate::database::Stores;
use crate::ir_shape::call_named;
use crate::opt::loop_records::group_covers_type;
use std::collections::{HashMap, HashSet};

/// Debug label for the move trace.
pub(crate) fn kind_of(v: &Value) -> &'static str {
    match v.unspan() {
        Value::Set(_, _) => "Set",
        Value::Call(_, _) => "Call",
        Value::Block(_) => "Block",
        Value::Loop(_) => "Loop",
        Value::Var(_) => "Var",
        Value::Null => "Null",
        _ => "other",
    }
}

/// @PLN157 § V-u (`@FR-R-RetAdopt`) — a vector-returning function whose result local ADOPTS
/// the hidden return buffer: the local builds in the caller's buffer from its declaration,
/// so every delivery copy at the exits (`OpReplaceVector`, the `OpClearVector` +
/// `OpAppendVector` pair) has nothing left to move and is emitted as nothing, and the
/// local's own witness store is never allocated.
#[derive(Clone, Copy, Debug)]
pub struct RetAdopt {
    /// The result local every delivery site copies into the buffer.
    pub v: u16,
    /// Its `__vdb` witness variable — the store the adoption leaves unallocated.
    pub vdb: u16,
    /// The hidden return-buffer variable the local aliases.
    pub buf: u16,
    /// The witness's `OpDatabase` type, for the buffer's null arm (a caller that offered
    /// no buffer gets one allocated exactly as the witness would have been).
    pub db_tp: i32,
}

/// The § V-u adoption of `def_nr`, or `None`.  Every gate is an under-approximation on
/// purpose — a declined function keeps the delivery copies, which are always correct:
///
/// - the function VALUE-returns a plain vector through a hidden buffer (a borrow return
///   publishes a dep and delivers nothing);
/// - every delivery into the buffer sources the SAME local `v`, and the buffer serves
///   nothing else — a site delivering another value, or a call handed the buffer (the
///   `one_buffer_chain` shape), declines;
/// - `v` is bound exactly once, from its own witness (`Set(v, OpGetField(__vdb, 0, _))`),
///   never reassigned (a rebind's `OpDatabase` reuse would clear the CALLER's store) and
///   never captured;
/// - the witness serves only its init and its frees;
/// - no `Parallel` or `Yield` in the body (a resumable frame's buffer discipline is its
///   own question).
#[must_use]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn ret_adopt(data: &Data, def_nr: u32) -> Option<RetAdopt> {
    let def = data.def(def_nr);
    if !def.is_loft_defined() {
        return None;
    }
    // The RETURN-TYPE gate comes FIRST, and the order is load-bearing.
    // `returns_borrowed_view` is a heap-return ownership read: it walks the return's dep list
    // as ATTRIBUTE indices, and a `Type::Function` return does not carry those — a closure's
    // deps are callee-frame notes tagged `0x8000`, which are not attr indices at all.
    // `data.rs` states that invariant as an assert ("closure-internal note reached a heap-return
    // ownership read") and it fired here on every closure factory under `-C debug-assertions=on`,
    // because this gate asked the ownership question before knowing the return was a vector.
    if !matches!(def.returned().peel_link(), Type::Vector(_, _)) {
        return None;
    }
    // A dep naming only HIDDEN attrs is the one-buffer return marker, not a borrow —
    // `returns_borrowed_view` reads exactly that distinction (a visible attr borrows).
    if def.returns_borrowed_view() {
        return None;
    }
    let attr = def.hidden_return_buffer_attr()?;
    let vars = def.variables();
    let buf = vars.var(&def.attributes()[attr].name);
    if buf == u16::MAX {
        return None;
    }
    let body = def.code();
    // One pass collects every fact the gates need.
    let mut delivery_src: Option<u16> = None;
    let mut bad = false;
    let mut init: Option<(u16, u16)> = None; // (v, vdb)
    let mut v_sets: u32 = 0;
    // The Clear+Append delivery pair is only skippable as its `one_buffer_vec_copy`
    // BLOCK; an append reaching the buffer outside one has no emission that elides it,
    // so it declines the adoption.
    let mut appends_total: u32 = 0;
    let mut appends_in_copy_block: u32 = 0;
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && bl.name == "one_buffer_vec_copy"
        {
            for op in &bl.operators {
                if let Value::Call(d, args) = op.unspan()
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpAppendVector"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if *b == buf)
                {
                    appends_in_copy_block += 1;
                }
            }
        }
        false
    });
    body.any_node(&mut |n| {
        match n {
            Value::Parallel(_) | Value::Yield(_) => bad = true,
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                match name {
                    "OpReplaceVector" | "OpAppendVector"
                        if matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if *b == buf) =>
                    {
                        if name == "OpAppendVector" {
                            appends_total += 1;
                        }
                        match args.get(1).map(Value::unspan) {
                            Some(Value::Var(s)) => match delivery_src {
                                None => delivery_src = Some(*s),
                                Some(prev) if prev == *s => {}
                                Some(_) => bad = true,
                            },
                            _ => bad = true,
                        }
                    }
                    "OpClearVector" => {}
                    _ => {
                        // Any other call handed the buffer is a use this analysis
                        // does not own (the chain shape delivers THROUGH a callee).
                        if args
                            .iter()
                            .any(|a| matches!(a.unspan(), Value::Var(b) if *b == buf))
                        {
                            bad = true;
                        }
                    }
                }
            }
            Value::Set(sv, to) => {
                if let Value::Call(d, gargs) = to.unspan()
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpGetField"
                    && let Some(Value::Var(w)) = gargs.first().map(Value::unspan)
                    && (*w as usize) < vars.count() as usize
                    && vars.name(*w).starts_with("__vdb")
                    // A PARAMETER bound to a backing is a work buffer taking its null
                    // road (`@FR-R-WorkBuffer`), never the result local: skipped, so the
                    // first bind found is the first a local could be adopted through.
                    && !vars.is_argument(*sv)
                    && init.is_none()
                {
                    init = Some((*sv, *w));
                }
            }
            _ => {}
        }
        false
    });
    if std::env::var("LOFT_TRACE_ADOPT").is_ok() {
        eprintln!(
            "[adopt] {}: bad={bad} src={delivery_src:?} init={init:?} appends={appends_total}/{appends_in_copy_block}",
            def.name()
        );
    }
    if bad || appends_total != appends_in_copy_block {
        return None;
    }
    let v = delivery_src?;
    let (iv, vdb) = init?;
    if iv != v || v >= vars.count() || vars.is_argument(v) || vars.is_captured(v) {
        return None;
    }
    // Exactly one Set of v (its init), and the witness's OpDatabase for the null arm.
    let mut db_tp: Option<i32> = None;
    body.any_node(&mut |n| {
        match n {
            Value::Set(sv, to) if *sv == v && !matches!(to.unspan(), Value::Null) => {
                v_sets += 1;
            }
            Value::Set(sv, to) if *sv == vdb && !matches!(to.unspan(), Value::Null) => {
                bad = true;
            }
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpDatabase"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == vdb) =>
            {
                if let Some(Value::Int(tp)) = args.get(1).map(Value::unspan) {
                    if db_tp.is_some() {
                        bad = true; // two OpDatabase on the witness: a rebind
                    }
                    db_tp = Some(*tp);
                }
            }
            // The witness reached by any call BUT its own init family declines.
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                if !matches!(
                    name,
                    "OpDatabase" | "OpGetField" | "OpSetInt4" | "OpFreeRef" | "OpFreeRefTag"
                ) && args
                    .iter()
                    .any(|a| matches!(a.unspan(), Value::Var(w) if *w == vdb))
                {
                    if std::env::var("LOFT_TRACE_ADOPT").is_ok() {
                        eprintln!("[adopt] {}: witness touched by {name}", def.name());
                    }
                    bad = true;
                }
            }
            _ => {}
        }
        false
    });
    if std::env::var("LOFT_TRACE_ADOPT").is_ok() {
        eprintln!(
            "[adopt] {}: second pass bad={bad} v_sets={v_sets} db_tp={db_tp:?}",
            def.name()
        );
    }
    if bad || v_sets != 1 {
        return None;
    }
    Some(RetAdopt {
        v,
        vdb,
        buf,
        db_tp: db_tp?,
    })
}

/// Does `callee` run `OpDatabase` on its own hidden return buffer (@PLN157 § V-j)?  That
/// is the witness-promoted delivery shape: the reuse arm CLEARS the buffer's whole store,
/// so a buffer such a callee receives must own its store — it cannot be placed.
pub(crate) fn callee_reinits_buffer(data: &Data, callee: &crate::data::Definition) -> bool {
    let Some(attr) = callee.hidden_return_buffer_attr() else {
        return true; // no buffer attr: not a delivery this analysis understands
    };
    let bv = callee.variables().var(&callee.attributes()[attr].name);
    if bv == u16::MAX {
        return true;
    }
    let mut reinits = false;
    callee.code().any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpDatabase"
            && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == bv)
        {
            reinits = true;
        }
        false
    });
    reinits
}

/// @PLN157 § V-y — what [`complete_writes`] admitted: `db_vars` are locals whose EVERY
/// `OpDatabase` site heads a covering literal group (the emitter calls `OpDatabaseNP`);
/// `mint_tps` are element types whose every `OpNewRecord` site in this function does
/// (the emitter calls `OpNewRecordNP`).
#[derive(Default)]
pub struct CompleteWrites {
    pub db_vars: HashSet<u16>,
    pub mint_tps: HashSet<u16>,
    /// `@C137`'s fresh-field arm: each record copy into a FIELD of a record a literal group
    /// is building (`OpCopyRecord(src, OpGetField(…e…, off, Sub), Sub)`), by argument-slice
    /// address, with the type it writes.  The destination was just created, so a null source
    /// leaves it what the prefill writes — the emitter writes `Sub`'s default in the copy's
    /// null arm, which a prefilled element already holds and an element minted without the
    /// prefill does not (loft#2044).  Computed whatever the switches say: it is not an
    /// optimisation.
    pub fresh_fields: HashMap<usize, u16>,
}

/// The variable a field chain `OpGetField(…OpGetField(Var(t), a, _)…, b, _)` is rooted at,
/// and whether it IS that variable (no field step).
fn field_chain_root(v: &Value, data: &Data) -> Option<(u16, bool)> {
    if let Value::Var(t) = v.unspan() {
        return Some((*t, true));
    }
    let args = call_named(v, data, "OpGetField")?;
    field_chain_root(args.first()?, data).map(|(t, _)| (t, false))
}

/// A record copy into a field reached from `target`: its argument slice's address and the
/// type it writes.  A copy flagged FRESH already writes a null source absent
/// (`Stores::copy_null_into`) and needs no arm of its own.
fn fresh_field_copy(stmt: &Value, target: u16, data: &Data) -> Option<(usize, u16)> {
    let args = call_named(stmt, data, "OpCopyRecord")?;
    let (root, whole) = field_chain_root(args.get(1)?, data)?;
    let Value::Int(raw) = args.get(2)?.unspan() else {
        return None;
    };
    let raw = u16::try_from(*raw).ok()?;
    (root == target && !whole && raw & crate::keys::COPY_FRESH_DEST == 0)
        .then_some((args.as_ptr() as usize, raw & crate::keys::COPY_TP_MASK))
}

/// The record copies into fields of the element `target` that the MINT at index `at - 1`
/// builds ([`CompleteWrites::fresh_fields`]): every statement up to the `OpFinishRecord`
/// that closes the element builds that one record, and nothing outside the literal names
/// its compiler temporary.  A declared local's literal is not read here: a later
/// `x.prev = p` writes a record that exists, and the IR does not tell the two apart.
fn collect_fresh_fields(
    ops: &[Value],
    at: usize,
    target: u16,
    data: &Data,
    out: &mut HashMap<usize, u16>,
) {
    for stmt in ops.iter().skip(at) {
        if let Some((addr, sub)) = fresh_field_copy(stmt, target, data) {
            out.insert(addr, sub);
        } else if call_named(stmt, data, "OpFinishRecord")
            .and_then(|a| a.get(1))
            .is_some_and(|e| matches!(e.unspan(), Value::Var(v) if *v == target))
        {
            return;
        }
    }
}

/// An element handed to a call AS ITS RETURN BUFFER, and copied from the call's answer when
/// that is another record: `r = f(…, e)` then `if OpDistinctStore(r, e) OpCopyRecord(r, e,
/// FRESH | tp) else null`.  The element is written whole on both arms when `f` writes every
/// field of the buffer it is handed ([`buffer_complete`]) — the answer is then `e` itself —
/// and the copy, flagged FRESH for the element's type, writes every byte of `e` otherwise.
fn call_pair_covers(
    ops: &[Value],
    at: usize,
    target: u16,
    tp: u16,
    data: &Data,
    stores: &Stores,
) -> bool {
    let mut rest = ops
        .iter()
        .skip(at)
        .filter(|o| !matches!(o.unspan(), Value::Line(_)));
    let Some(Value::Set(r, call)) = rest.next().map(Value::unspan) else {
        return false;
    };
    let Value::Call(f, args) = call.unspan() else {
        return false;
    };
    if (*f as usize) >= data.definitions.len() {
        return false;
    }
    let Some(rb) = data.def(*f).hidden_return_buffer_attr() else {
        return false;
    };
    let names_target = |v: &Value| v.any_node(&mut |n| matches!(n, Value::Var(u) if *u == target));
    let handed = args.iter().enumerate().all(|(i, a)| {
        if i == rb {
            matches!(a.unspan(), Value::Var(u) if *u == target)
        } else {
            !names_target(a)
        }
    });
    if !handed {
        return false;
    }
    let Some(Value::If(test, copy, other)) = rest.next().map(Value::unspan) else {
        return false;
    };
    let pair = |v: &Value, name: &str| {
        call_named(v, data, name).is_some_and(|a| {
            matches!(a.first().map(Value::unspan), Some(Value::Var(u)) if u == r)
                && matches!(a.get(1).map(Value::unspan), Some(Value::Var(u)) if *u == target)
        })
    };
    let fresh = call_named(copy, data, "OpCopyRecord").is_some_and(|a| {
        matches!(a.get(2).map(Value::unspan), Some(Value::Int(n)) if u16::try_from(*n)
            .is_ok_and(|n| n & crate::keys::COPY_FRESH_DEST != 0 && n & crate::keys::COPY_TP_MASK == tp))
    });
    pair(test, "OpDistinctStore")
        && pair(copy, "OpCopyRecord")
        && fresh
        && matches!(other.unspan(), Value::Null)
        && buffer_complete(data, stores, *f)
}

/// Does `f` write every field of the return buffer it is handed before it answers it?  Every
/// mint of the buffer heads a covering literal ([`complete_writes`]'s `db_vars`), and every
/// `return` of the buffer follows such a mint in its own block.  A recursive chain answers no
/// past two levels.
fn buffer_complete(data: &Data, stores: &Stores, f: u32) -> bool {
    thread_local! {
        static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    let def = data.def(f);
    let Some(rb) = def
        .hidden_return_buffer_attr()
        .map(|i| def.variables().var(&def.attributes()[i].name))
    else {
        return false;
    };
    if DEPTH.with(std::cell::Cell::get) >= 2 {
        return false;
    }
    DEPTH.with(|d| d.set(d.get() + 1));
    let complete = complete_writes(data, stores, f).db_vars.contains(&rb);
    DEPTH.with(|d| d.set(d.get() - 1));
    if !complete {
        return false;
    }
    let mut ok = true;
    def.code().any_node(&mut |n| {
        if let Value::Block(bl) = n {
            let mut minted = false;
            for op in &bl.operators {
                if guarded_mint(op, data).is_some_and(|(v, _)| v == rb)
                    || call_named(op, data, "OpDatabase").is_some_and(
                        |a| matches!(a.first().map(Value::unspan), Some(Value::Var(u)) if *u == rb),
                    )
                {
                    minted = true;
                }
                if let Value::Return(x) = op.unspan()
                    && matches!(x.unspan(), Value::Var(u) if *u == rb)
                    && !minted
                {
                    ok = false;
                }
            }
        }
        !ok
    });
    ok
}

/// `if <test of v> null else OpDatabase(v, tp)` — the mint a struct literal built into a
/// return buffer opens with: the record the caller handed, or a fresh one.  The test names
/// only `v`.
fn guarded_mint(stmt: &Value, data: &Data) -> Option<(u16, u16)> {
    let Value::If(test, then, els) = stmt.unspan() else {
        return None;
    };
    if !matches!(then.unspan(), Value::Null) {
        return None;
    }
    let args = call_named(els, data, "OpDatabase")?;
    let (Some(Value::Var(v)), Some(Value::Int(tp))) = (
        args.first().map(Value::unspan),
        args.get(1).map(Value::unspan),
    ) else {
        return None;
    };
    if test.any_node(&mut |n| matches!(n, Value::Var(u) if u != v)) {
        return None;
    }
    Some((*v, u16::try_from(*tp).ok()?))
}

/// @PLN157 § V-y (`@FR-R-CompleteWrite`) — the literal groups whose write set is
/// COMPLETE, so the default prefill writes nothing that survives: the parser's lowering
/// writes every field explicitly (a named value, the declared default, the interned
/// empty text, the null sentinel, `false`, the variant tag), and the emitter's proof is
/// coverage of every schema field position by the group's contiguous `OpSet*`s.
/// `db_vars` is keyed by the LOCAL (every `OpDatabase(v, …)` site must head a covering
/// group — a var built completely in one branch and partially in another declines);
/// `mint_tps` by the ELEMENT TYPE (every `Set(_, OpNewRecord(…, tp, …))` group in the
/// function must cover it).  The interpreter keeps the prefill and is the oracle.
#[expect(
    clippy::too_many_lines,
    reason = "one scan classifying both group heads (locals and mint types) beside its site counts"
)]
pub fn complete_writes(data: &Data, stores: &Stores, def_nr: u32) -> CompleteWrites {
    let def = data.def(def_nr);
    let body = def.code();
    let mut out = CompleteWrites::default();
    let mut db_declined: HashSet<u16> = HashSet::new();
    let mut mint_declined: HashSet<u16> = HashSet::new();
    // loft#2193 — complete only when EVERY site heads a covering group: a site the scan below
    // never reaches is neither covered nor declined, so all sites are counted against it.
    let (db_sites, mint_sites) = mint_site_counts(body, data);
    let mut db_covered: HashMap<u16, u32> = HashMap::new();
    let mut mint_covered: HashMap<u16, u32> = HashMap::new();
    body.any_node(&mut |n| {
        // A loop's body is a statement list as a block's is: its groups are classified too.
        if let Value::Block(bl) | Value::Loop(bl) = n {
            let ops = &bl.operators;
            for (i, stmt) in ops.iter().enumerate() {
                match stmt.unspan() {
                    // `OpDatabase(v, tp)` heading a struct-literal group.
                    Value::Call(d, args)
                        if (*d as usize) < data.definitions.len()
                            && data.def(*d).name() == "OpDatabase" =>
                    {
                        let (Some(Value::Var(v)), Some(Value::Int(tp))) = (
                            args.first().map(Value::unspan),
                            args.get(1).map(Value::unspan),
                        ) else {
                            continue;
                        };
                        let Ok(tp) = u16::try_from(*tp) else {
                            db_declined.insert(*v);
                            continue;
                        };
                        if group_covers_type(ops, i + 1, *v, tp, data, stores, &out.fresh_fields) {
                            out.db_vars.insert(*v);
                            *db_covered.entry(*v).or_default() += 1;
                        } else {
                            db_declined.insert(*v);
                        }
                    }
                    // A return buffer's guarded mint, `if <v holds a record> null else
                    // OpDatabase(v, tp)`, heading the literal group: on either arm the group
                    // writes the record the statements after it name.
                    Value::If(_, _, _) if let Some((v, tp)) = guarded_mint(stmt, data) => {
                        if group_covers_type(ops, i + 1, v, tp, data, stores, &out.fresh_fields) {
                            out.db_vars.insert(v);
                            *db_covered.entry(v).or_default() += 1;
                        } else {
                            db_declined.insert(v);
                        }
                    }
                    // `Set(e, OpNewRecord(P, tp, fld))` heading a mint group: the sets
                    // that follow write through `e`.
                    Value::Set(e, inner) => {
                        let Value::Call(d, cargs) = inner.unspan() else {
                            continue;
                        };
                        if (*d as usize) >= data.definitions.len()
                            || data.def(*d).name() != "OpNewRecord"
                        {
                            continue;
                        }
                        collect_fresh_fields(ops, i + 1, *e, data, &mut out.fresh_fields);
                        let Some(Value::Int(ptp)) = cargs.get(1).map(Value::unspan) else {
                            continue;
                        };
                        let Ok(ptp) = u16::try_from(*ptp) else {
                            continue;
                        };
                        // The element's CONTENT type is what the prefill would walk.
                        let fld = match cargs.get(2).map(Value::unspan) {
                            Some(Value::Int(f)) => u16::try_from(*f).unwrap_or(u16::MAX),
                            _ => u16::MAX,
                        };
                        let etp = if fld == u16::MAX {
                            stores.content(ptp)
                        } else {
                            stores.content(stores.field_type(ptp, fld))
                        };
                        if etp != u16::MAX
                            && (group_covers_type(
                                ops,
                                i + 1,
                                *e,
                                etp,
                                data,
                                stores,
                                &out.fresh_fields,
                            ) || call_pair_covers(ops, i + 1, *e, etp, data, stores))
                        {
                            out.mint_tps.insert(ptp);
                            *mint_covered.entry(ptp).or_default() += 1;
                        } else {
                            mint_declined.insert(ptp);
                        }
                    }
                    _ => {}
                }
            }
        }
        false
    });
    if std::env::var("LOFT_TRACE_COMPLETE_WRITE").is_ok_and(|v| v != "0") {
        trace_complete_declines(
            def,
            &out,
            (&db_covered, &db_sites),
            (&mint_covered, &mint_sites),
        );
    }
    out.db_vars
        .retain(|v| !db_declined.contains(v) && db_covered.get(v) == db_sites.get(v));
    out.mint_tps
        .retain(|t| !mint_declined.contains(t) && mint_covered.get(t) == mint_sites.get(t));
    out
}

/// loft#2193 — every `OpDatabase(v, …)` site per local and every `OpNewRecord(…, tp, …)` site
/// per type in `body`, wherever it stands: what [`complete_writes`] compares its covered
/// sites against.
fn mint_site_counts(body: &Value, data: &Data) -> (HashMap<u16, u32>, HashMap<u16, u32>) {
    let mut db_sites: HashMap<u16, u32> = HashMap::new();
    let mut mint_sites: HashMap<u16, u32> = HashMap::new();
    body.any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
        {
            match data.def(*d).name() {
                "OpDatabase" => {
                    if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                        *db_sites.entry(*v).or_default() += 1;
                    }
                }
                "OpNewRecord" => {
                    if let Some(Value::Int(ptp)) = args.get(1).map(Value::unspan)
                        && let Ok(ptp) = u16::try_from(*ptp)
                    {
                        *mint_sites.entry(ptp).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
        false
    });
    (db_sites, mint_sites)
}

/// `LOFT_TRACE_COMPLETE_WRITE=1` — each local and type [`complete_writes`] declines because
/// not every one of its sites heads a covering group, with the counts.
#[allow(clippy::type_complexity)]
fn trace_complete_declines(
    def: &crate::data::Definition,
    out: &CompleteWrites,
    db: (&HashMap<u16, u32>, &HashMap<u16, u32>),
    mint: (&HashMap<u16, u32>, &HashMap<u16, u32>),
) {
    for v in out.db_vars.iter().filter(|v| db.0.get(v) != db.1.get(v)) {
        eprintln!(
            "complete-write: {} local {} declined: {:?} of {:?} sites covered",
            def.name(),
            def.variables().name(*v),
            db.0.get(v),
            db.1.get(v)
        );
    }
    for t in out
        .mint_tps
        .iter()
        .filter(|t| mint.0.get(t) != mint.1.get(t))
    {
        eprintln!(
            "complete-write: {} mint type {t} declined: {:?} of {:?} sites covered",
            def.name(),
            mint.0.get(t),
            mint.1.get(t)
        );
    }
}
