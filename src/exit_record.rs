// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `(R-ExitVector)`'s RECORD clause — a LOCAL RECORD returned inside the exit literal is
//! built in its field of the return buffer.
//!
//! The assembler shape: `lb = ListBox { … }; ss = Strip { … }; Panel { list: lb, status:
//! ss, … }`.  Each local is minted as a store of its own, built, deep-copied into its inline
//! field of the return buffer by the exit literal (every text and vector it holds claimed
//! again), and its store freed — a store cycle and a deep copy per local per call, for a
//! value that only ever ends up in that field.  Here the local IS the field: its mint becomes
//! the buffer's ensure, the local bound to the field's place, and a release of whatever the
//! place held (`OpClear` — a refilled buffer still holds its previous value's heap); the
//! exit's copy and the local's frees go.
//!
//! A fresh mint writes the type's defaults and the place does not, so the local's
//! construction must write EVERY leaf field of the type before it reads any: the literal
//! group right after the mint is scanned for that, and a local built any other way declines.
//! Admission, per candidate: the local is minted once, in the function body's own statement
//! list; it is only ever a RECEIVER (the first argument of a native op, or of an
//! `OpGetField` that is itself one); the function has ONE `return`, the exit literal, holding
//! the one copy of the local into a field of the buffer at a constant position; the local's
//! frees all sit in that exit; and nothing else writes into the field's bytes.  Declines keep
//! the store and the copy.  `LOFT_NO_EXIT_RECORD=1` is the switch; `LOFT_TRACE_PLACE=1` names
//! each admission and decline.  Both backends.
use crate::data::{Data, Value};
use crate::database::{Parts, Stores};

/// `LOFT_NO_EXIT_RECORD=1` — every record local returned inside the exit literal keeps its
/// own store and its copy.
fn enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_EXIT_RECORD").is_ok_and(|local| local != "0"))
}

struct Ops {
    database: u32,
    get_field: u32,
    copy: u32,
    clear: u32,
    free_ref: u32,
    free_if_distinct: u32,
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |node: &str| {
            let opd = data.def_nr(node);
            (opd != u32::MAX).then_some(opd)
        };
        Some(Self {
            database: nr("OpDatabase")?,
            get_field: nr("OpGetField")?,
            copy: nr("OpCopyRecord")?,
            clear: nr("OpClear")?,
            free_ref: nr("OpFreeRef")?,
            free_if_distinct: nr("OpFreeRefIfDistinct")?,
        })
    }
}

/// An admitted local: its mint at body statement `mint`, its field at `off` of type `ftp`.
struct Plan {
    local: u16,
    tp: u16,
    mint: usize,
    off: i32,
    ftp: i32,
}

/// Rewrite every admitted record local of function `d_nr`.
pub fn rewrite(data: &mut Data, database: &Stores, d_nr: u32) {
    if !enabled() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let def = data.def(d_nr);
    if !def.is_loft_defined() {
        return;
    }
    let Some(rb) = crate::exit_vector::own_return_buffer(data, d_nr) else {
        return;
    };
    let Value::Block(body) = def.code().unspan() else {
        return;
    };
    let Some(guard) = crate::exit_vector::exit_guard(def.code(), rb, data) else {
        return;
    };
    let function = def.variables();
    let mut plans = Vec::new();
    for (idx, stmt) in body.operators.iter().enumerate() {
        let Value::Call(opd, args) = stmt.unspan() else {
            continue;
        };
        let (true, [Value::Var(local), Value::Int(tp)]) = (*opd == ops.database, args.as_slice())
        else {
            continue;
        };
        if *local == rb || function.is_argument(*local) {
            continue;
        }
        let Ok(tp) = u16::try_from(*tp) else {
            continue;
        };
        match admit(
            data,
            database,
            &ops,
            def.code(),
            &body.operators,
            idx,
            *local,
            tp,
            rb,
        ) {
            Ok((off, ftp)) => {
                if crate::keys::trace_place() {
                    eprintln!(
                        "[exit-record] fn={} v={}: ADMITTED at +{off}",
                        def.name(),
                        function.name(*local)
                    );
                }
                plans.push(Plan {
                    local: *local,
                    tp,
                    mint: idx,
                    off,
                    ftp,
                });
            }
            Err(why) => {
                if crate::keys::trace_place() {
                    eprintln!(
                        "[exit-record] fn={} v={}: DECLINED — {why}",
                        def.name(),
                        function.name(*local)
                    );
                }
            }
        }
    }
    if plans.is_empty() {
        return;
    }
    apply(data, d_nr, &plans, rb, &guard, &ops);
}

/// Each admitted local becomes a view of its field: the mint the buffer's ensure, the bind of
/// the place and the release of what it held; the copy and the frees go.
fn apply(data: &mut Data, d_nr: u32, plans: &[Plan], rb: u16, guard: &Value, ops: &Ops) {
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    for plan in plans {
        crate::rewrite_census::fired("R-ExitVector/record", 1);
        if let Value::Block(bl) = code.unspan_mut() {
            bl.operators[plan.mint] = Value::Insert(vec![
                guard.clone(),
                Value::Set(
                    plan.local,
                    Box::new(Value::Call(
                        ops.get_field,
                        vec![Value::Var(rb), Value::Int(plan.off), Value::Int(plan.ftp)],
                    )),
                ),
                Value::Call(
                    ops.clear,
                    vec![Value::Var(plan.local), Value::Int(i32::from(plan.tp))],
                ),
            ]);
        }
        drop_sites(&mut code, plan.local, ops);
    }
    data.definitions[d_nr as usize].code = code;
    // Each local is now a VIEW of the buffer's field, never an owner: typed so, a bind of
    // the place is no copy (B-View-Base) and no exit releases it.
    for plan in plans {
        let vars = &mut data.definitions[d_nr as usize].variables;
        let tp = vars.tp(plan.local).depending(rb);
        vars.set_type(plan.local, tp);
    }
}

fn is_var(node: &Value, wv: u16) -> bool {
    matches!(node.unspan(), Value::Var(arg) if *arg == wv)
}

fn int(node: Option<&Value>) -> Option<i32> {
    match node.map(Value::unspan) {
        Some(Value::Int(idx)) => Some(*idx),
        _ => None,
    }
}

/// The copy `OpCopyRecord(v, OpGetField(rb, off, ftp), tp)`: `(off, ftp)`.
fn exit_copy(node: &Value, local: u16, rb: u16, tp: u16, ops: &Ops) -> Option<(i32, i32)> {
    let Value::Call(opd, args) = node.unspan() else {
        return None;
    };
    if *opd != ops.copy
        || args.len() != 3
        || !is_var(&args[0], local)
        || int(args.get(2)) != Some(i32::from(tp))
    {
        return None;
    }
    let Value::Call(inner, ga) = args[1].unspan() else {
        return None;
    };
    if *inner != ops.get_field || !ga.first().is_some_and(|arg| is_var(arg, rb)) {
        return None;
    }
    Some((int(ga.get(1))?, int(ga.get(2))?))
}

#[allow(clippy::too_many_arguments)]
fn admit(
    data: &Data,
    database: &Stores,
    ops: &Ops,
    code: &Value,
    stmts: &[Value],
    mint: usize,
    local: u16,
    tp: u16,
    rb: u16,
) -> Result<(i32, i32), &'static str> {
    if !matches!(
        database.types.get(tp as usize).map(|t| &t.parts),
        Some(Parts::Struct(_))
    ) {
        return Err("the local is not a plain struct");
    }
    // The one exit, and the one copy of the local into a field of the buffer inside it.
    let mut returns = 0usize;
    let mut copies = Vec::new();
    let mut exit_frees = 0usize;
    let mut frees = 0usize;
    let mut mints = 0usize;
    code.walk(&mut |node| match node.unspan() {
        Value::Return(_) => returns += 1,
        Value::Block(bl) if crate::exit_vector::is_exit_block(bl, rb) => {
            for stmt in &bl.operators {
                if let Some(at) = exit_copy(stmt, local, rb, tp, ops) {
                    copies.push(at);
                }
                if let Value::Call(opd, args) = stmt.unspan()
                    && (*opd == ops.free_ref || *opd == ops.free_if_distinct)
                    && args.first().is_some_and(|arg| is_var(arg, local))
                {
                    exit_frees += 1;
                }
            }
        }
        Value::Call(opd, args) if args.first().is_some_and(|arg| is_var(arg, local)) => {
            if *opd == ops.free_ref || *opd == ops.free_if_distinct {
                frees += 1;
            }
            if *opd == ops.database {
                mints += 1;
            }
        }
        _ => {}
    });
    if returns != 1 {
        return Err("the function has more than its one exit");
    }
    let [(off, ftp)] = copies.as_slice() else {
        return Err("the local is not copied exactly once into a field of the buffer");
    };
    if mints != 1 {
        return Err("the local is minted more than once");
    }
    if frees != exit_frees {
        return Err("the local is freed outside its exit");
    }
    if *ftp != i32::from(tp) || *off < 0 {
        return Err("the field is not the local's type");
    }
    // Every mention of the local is a receiver, and it is bound only by its null init.
    receivers_only(code, local, ops, data)?;
    let mut rebinds = 0usize;
    code.walk(&mut |node| {
        if let Value::Set(arg, val) = node.unspan()
            && *arg == local
            && !matches!(val.unspan(), Value::Null)
        {
            rebinds += 1;
        }
    });
    if rebinds != 0 {
        return Err("the local is rebound");
    }
    // Nothing else writes the field's bytes.
    let size = i32::from(database.size(tp));
    if writes_region(code, rb, *off, *off + size, local, ops, data) {
        return Err("something else writes the field's bytes");
    }
    // The construction writes every leaf field before it reads one.
    let mut leaves = Vec::new();
    leaf_offsets(database, tp, 0, &mut leaves, 0)?;
    let mut written: Vec<i32> = Vec::new();
    for stmt in &stmts[mint + 1..] {
        if leaves.iter().all(|l| written.contains(l)) {
            break;
        }
        match group_write(stmt, local, ops, data, database) {
            Some(at) => written.extend(at),
            None if on_written_field(stmt, local, ops, &written) => {}
            None => break,
        }
    }
    if !leaves.iter().all(|l| written.contains(l)) {
        return Err("the construction does not write every field before it reads one");
    }
    Ok((*off, *ftp))
}

/// The positions of every leaf field of `tp`, an embedded record's fields flattened in.
fn leaf_offsets(
    database: &Stores,
    tp: u16,
    base: i32,
    out: &mut Vec<i32>,
    depth: usize,
) -> Result<(), &'static str> {
    if depth > 8 {
        return Err("the record nests too deep");
    }
    let Some(Parts::Struct(fields)) = database.types.get(tp as usize).map(|t| &t.parts) else {
        return Err("a field's type is no plain struct");
    };
    for f in fields {
        let at = base + i32::from(f.position);
        match database.types.get(f.content as usize).map(|t| &t.parts) {
            Some(Parts::Struct(_)) => leaf_offsets(database, f.content, at, out, depth + 1)?,
            Some(Parts::EnumValue(..)) => return Err("a field holds a struct-enum payload"),
            Some(Parts::Enum(values))
                if values.iter().any(|(t, _)| {
                    matches!(
                        database.types.get(*t as usize).map(|arg| &arg.parts),
                        Some(Parts::EnumValue(..))
                    )
                }) =>
            {
                return Err("a field holds a struct-enum");
            }
            _ => out.push(at),
        }
    }
    Ok(())
}

/// One statement of the construction: `Some(at)` the leaf offsets a write of the local
/// covers (empty for a statement that neither reads nor writes it), `None` anything else —
/// the end of the scan.
fn group_write(
    stmt: &Value,
    local: u16,
    ops: &Ops,
    data: &Data,
    database: &Stores,
) -> Option<Vec<i32>> {
    let names = |arg: &Value| {
        let mut hit = false;
        arg.walk(&mut |node| {
            if is_var(node, local) {
                hit = true;
            }
        });
        hit
    };
    if !names(stmt) {
        return Some(Vec::new());
    }
    let Value::Call(opd, args) = stmt.unspan() else {
        return None;
    };
    // A sub-record written whole by a copy into it.
    if *opd == ops.copy {
        let Some(Value::Call(inner, ga)) = args.get(1).map(Value::unspan) else {
            return None;
        };
        if *inner != ops.get_field
            || !ga.first().is_some_and(|arg| is_var(arg, local))
            || names(&args[0])
        {
            return None;
        }
        let base = int(ga.get(1))?;
        let sub = u16::try_from(int(ga.get(2))?).ok()?;
        let mut out = Vec::new();
        leaf_offsets(database, sub, base, &mut out, 1).ok()?;
        return Some(out);
    }
    if !data.def(*opd).name().starts_with("OpSet") || args.iter().skip(1).any(names) {
        return None;
    }
    let pos = int(args.get(1))?;
    match args.first().map(Value::unspan)? {
        Value::Var(arg) if *arg == local => Some(vec![pos]),
        Value::Call(inner, ga)
            if *inner == ops.get_field && ga.first().is_some_and(|arg| is_var(arg, local)) =>
        {
            Some(vec![int(ga.get(1))? + pos])
        }
        _ => None,
    }
}

/// A statement whose only mention of the local is the receiver `OpGetField(v, at, _)` of a
/// native op, `at` a field the construction already wrote: an append into its vector.
fn on_written_field(stmt: &Value, local: u16, ops: &Ops, written: &[i32]) -> bool {
    let Value::Call(_, args) = stmt.unspan() else {
        return false;
    };
    let Some(Value::Call(inner, ga)) = args.first().map(Value::unspan) else {
        return false;
    };
    if *inner != ops.get_field || !ga.first().is_some_and(|arg| is_var(arg, local)) {
        return false;
    }
    let Some(at) = int(ga.get(1)) else {
        return false;
    };
    let mut rest = 0usize;
    for arg in &args[1..] {
        arg.walk(&mut |node| {
            if is_var(node, local) {
                rest += 1;
            }
        });
    }
    rest == 0 && written.contains(&at)
}

/// Every `Var(v)` is the first argument of a native op, or of an `OpGetField` that is itself
/// the first argument of a native op.
fn receivers_only(code: &Value, local: u16, ops: &Ops, data: &Data) -> Result<(), &'static str> {
    // Each `Var(v)` must be the direct first argument of a native op; each such op that is an
    // `OpGetField` must in turn be the first argument of a native op other than a copy.
    let mut total = 0usize;
    let mut direct = 0usize;
    let mut fields = 0usize;
    let mut fields_received = 0usize;
    code.walk(&mut |node| {
        if is_var(node, local) {
            total += 1;
        }
        let Value::Call(opd, args) = node.unspan() else {
            return;
        };
        if !native(data, *opd) {
            return;
        }
        if args.first().is_some_and(|arg| is_var(arg, local)) {
            direct += 1;
            if *opd == ops.get_field {
                fields += 1;
            }
        }
        if *opd != ops.copy
            && let Some(Value::Call(inner, ga)) = args.first().map(Value::unspan)
            && *inner == ops.get_field
            && ga.first().is_some_and(|arg| is_var(arg, local))
        {
            fields_received += 1;
        }
        // A sub-record of the local written whole: `OpCopyRecord(src, OpGetField(v, …), tp)`.
        if *opd == ops.copy
            && let Some(Value::Call(inner, ga)) = args.get(1).map(Value::unspan)
            && *inner == ops.get_field
            && ga.first().is_some_and(|arg| is_var(arg, local))
        {
            fields_received += 1;
        }
    });
    if direct == total && fields == fields_received {
        Ok(())
    } else {
        Err("the local is named outside a receiver position")
    }
}

fn native(data: &Data, opd: u32) -> bool {
    (opd as usize) < data.definitions.len() && data.def(opd).name().starts_with("Op")
}

/// Does any statement other than the local's own copy write into `rb`'s bytes `lo..hi`?
fn writes_region(
    code: &Value,
    rb: u16,
    lo: i32,
    hi: i32,
    local: u16,
    ops: &Ops,
    data: &Data,
) -> bool {
    let mut hit = false;
    code.walk(&mut |node| {
        let Value::Call(opd, args) = node.unspan() else {
            return;
        };
        if !native(data, *opd) || *opd == ops.get_field || *opd == ops.database {
            return;
        }
        if *opd == ops.copy && args.first().is_some_and(|arg| is_var(arg, local)) {
            return;
        }
        // Ops that only read, compare or release a handle write no field's bytes.
        let name = data.def(*opd).name();
        // `OpPlaceRecord(rb, tp)` claims a NEW record in the buffer's store.
        if [
            "OpGet",
            "OpFree",
            "OpDistinct",
            "OpRefIsNull",
            "OpConv",
            "OpEq",
            "OpNe",
            "OpLength",
            "OpSize",
            "OpPlaceRecord",
        ]
        .iter()
        .any(|p| name.starts_with(p))
        {
            return;
        }
        for (idx, arg) in args.iter().enumerate() {
            match arg.unspan() {
                // The buffer itself: as the receiver, its write position is the next argument.
                Value::Var(wv) if *wv == rb => {
                    if idx == 0
                        && name.starts_with("OpSet")
                        && let Some(pos) = int(args.get(1))
                        && (pos < lo || pos >= hi)
                    {
                        continue;
                    }
                    hit = true;
                }
                // A field of the buffer: outside the region it is another field's.
                Value::Call(inner, ga)
                    if *inner == ops.get_field
                        && ga.first().is_some_and(|arg2| is_var(arg2, rb)) =>
                {
                    match int(ga.get(1)) {
                        Some(off) if off < lo || off >= hi => {}
                        _ => hit = true,
                    }
                }
                _ => {}
            }
        }
    });
    hit
}

/// The exit's copy of the local and its frees become nothing.
fn drop_sites(code: &mut Value, local: u16, ops: &Ops) {
    match code {
        Value::Call(opd, args)
            if (*opd == ops.copy || *opd == ops.free_ref || *opd == ops.free_if_distinct)
                && args.first().is_some_and(|arg| is_var(arg, local)) =>
        {
            *code = Value::Null;
        }
        Value::Span(b) => drop_sites(&mut b.1, local, ops),
        Value::Block(bl) | Value::Loop(bl) => {
            for stmt in &mut bl.operators {
                drop_sites(stmt, local, ops);
            }
        }
        Value::Insert(list) => {
            for stmt in list {
                drop_sites(stmt, local, ops);
            }
        }
        Value::If(c, t, e) => {
            drop_sites(c, local, ops);
            drop_sites(t, local, ops);
            drop_sites(e, local, ops);
        }
        Value::Set(_, arg) | Value::Return(arg) | Value::Drop(arg) => drop_sites(arg, local, ops),
        Value::Call(_, args) => {
            for arg in args {
                drop_sites(arg, local, ops);
            }
        }
        _ => {}
    }
}
