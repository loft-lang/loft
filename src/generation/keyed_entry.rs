// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-KeyedEntry` — `h += [T{k: key, …}]` written into the entry the key names.
//!
//! The plain form claims a fresh entry (`OpNewRecord`), writes the literal's fields into it,
//! and files it (`OpFinishRecord`), which on a key already present replaces the entry there and
//! releases it: a slot claimed and a slot released per repeated key.  Here the literal's values
//! are evaluated first (STAGED, in program order — the plain form evaluates them while its
//! entry is claimed but not yet filed, and nothing a value can read sees an unfiled entry),
//! then `Stores::keyed_entry_long` answers the entry under the key — the existing one, or a
//! fresh one filed where the probe for it ended — and the staged values are written into it.
//! Native only; the interpreter keeps the plain form.
//!
//! Admitted when overwriting the entry IS replacing it: a `hash` with one `integer` or `long`
//! key, whose element owns no heap (an overwrite releases nothing, so a replaced text or
//! vector cannot be stranded) and is no member of a linked group (a group's dedup decides per
//! member), every field of the element written by the literal (so an overwrite leaves nothing
//! of the old entry), and the literal's buffer named nowhere else.

use crate::data::{Data, Value};
use crate::database::{Parts, Stores};

/// `LOFT_NO_KEYED_ENTRY=1` — every keyed insert claims, writes and files its entry again.
#[must_use]
pub fn enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_KEYED_ENTRY").is_ok_and(|v| v != "0"))
}

/// The setters a literal of a heap-free element writes its fields with.
const SETTERS: &[&str] = &[
    "OpSetBoolean",
    "OpSetByte",
    "OpSetByteNullable",
    "OpSetCharacter",
    "OpSetEnum",
    "OpSetFloat",
    "OpSetInt",
    "OpSetInt4",
    "OpSetInt4Raw",
    "OpSetShort",
    "OpSetShortRaw",
    "OpSetSingle",
];

/// An admitted insert: statements `first ..= last` of a block are `B = OpNewRecord(coll, tp,
/// fld)`, the literal's field writes through `B`, and `OpFinishRecord(coll, B, tp, fld)`.
pub struct Site<'a> {
    pub coll: &'a Value,
    pub parent_tp: u16,
    pub field: u16,
    /// The value written to the key field.
    pub key: &'a Value,
    /// The field writes, in program order, the key's included.
    pub writes: Vec<&'a Value>,
    pub last: usize,
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |m| {
        if is_var(m, x) {
            n += 1;
        }
    });
    n
}

fn int_of(v: &Value) -> Option<i64> {
    match v.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

/// Does `v` assign a variable the collection's path is rooted in?  The plain form evaluates
/// the path at the claim and again at the finish, around the values; staged, it is evaluated
/// once after them, so a value that rebinds it would move the insert.
fn rebinds(v: &Value, roots: &[u16]) -> bool {
    let mut hit = false;
    v.walk(&mut |m| {
        if let Value::Set(x, _) = m
            && roots.contains(x)
        {
            hit = true;
        }
    });
    hit
}

/// The element type a `hash` field (or a `hash` itself, `field` the whole-parent marker) holds,
/// with its one key's position, when an overwrite of its entry is a replace.
fn heap_free_hash(stores: &Stores, parent_tp: u16, field: u16) -> Option<(u16, u16)> {
    if (parent_tp as usize) >= stores.types.len() || stores.key_owner(parent_tp) != parent_tp {
        return None;
    }
    let htp = if field == u16::MAX {
        parent_tp
    } else {
        let Parts::Struct(fields) = &stores.types[parent_tp as usize].parts else {
            return None;
        };
        let f = fields.get(field as usize)?;
        if stores.field_is_group_member(parent_tp, f.position) {
            return None;
        }
        stores.field_type(parent_tp, field)
    };
    let t = stores.types.get(htp as usize)?;
    let Parts::Hash(c, _) = t.parts else {
        return None;
    };
    let [k] = &t.keys[..] else {
        return None;
    };
    if !matches!(k.type_nr.unsigned_abs(), 1 | 2)
        || stores.is_linked(c)
        || stores.owns_heap(c)
    {
        return None;
    }
    Some((c, k.position))
}

/// The insert headed by statement `at` of `ops`, in function `d_nr`, when the clause admits it.
pub fn site<'a>(
    ops: &'a [Value],
    at: usize,
    data: &Data,
    stores: &Stores,
    d_nr: u32,
) -> Option<Site<'a>> {
    let Value::Set(buf, mint) = ops[at].unspan() else {
        return None;
    };
    let buf = *buf;
    let Value::Call(d, margs) = mint.unspan() else {
        return None;
    };
    if name(data, *d) != "OpNewRecord" {
        return None;
    }
    let [coll, ptp, fld] = &margs[..] else {
        return None;
    };
    let parent_tp = u16::try_from(int_of(ptp)?).ok()?;
    let field = u16::try_from(int_of(fld)?).unwrap_or(u16::MAX);
    let (content, key_pos) = heap_free_hash(stores, parent_tp, field)?;
    // The collection is evaluated once for the entry: a pure path, the place the plain form's
    // claim and finish both name.
    super::hoist::vector_path(data, coll)?;
    let mut roots: Vec<u16> = Vec::new();
    coll.walk(&mut |m| {
        if let Value::Var(x) = m.unspan() {
            roots.push(*x);
        }
    });
    let mut writes: Vec<&Value> = Vec::new();
    let mut written: Vec<u16> = Vec::new();
    let mut key: Option<&Value> = None;
    let mut last = None;
    for (j, op) in ops.iter().enumerate().skip(at + 1) {
        match op.unspan() {
            Value::Line(_) => {}
            Value::Call(c, a)
                if name(data, *c) == "OpFinishRecord"
                    && a.get(1).is_some_and(|x| is_var(x, buf)) =>
            {
                let [fc, _, ftp, ffld] = &a[..] else {
                    return None;
                };
                if fc.unspan() != coll.unspan() || ftp != ptp || ffld != fld {
                    return None;
                }
                last = Some(j);
                break;
            }
            Value::Call(c, a)
                if SETTERS.contains(&name(data, *c))
                    && a.first().is_some_and(|x| is_var(x, buf))
                    && a.iter().skip(1).all(|x| mentions(x, buf) == 0 && !rebinds(x, &roots)) =>
            {
                let off = u16::try_from(int_of(a.get(1)?)?).ok()?;
                if written.contains(&off) {
                    return None;
                }
                written.push(off);
                if off == key_pos {
                    if name(data, *c) != "OpSetInt" {
                        return None;
                    }
                    key = Some(a.get(2)?);
                }
                writes.push(op);
            }
            _ => return None,
        }
    }
    let (last, key) = (last?, key?);
    // Every field written, so an overwrite leaves nothing of the entry it lands on.
    let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
        &stores.types.get(content as usize)?.parts
    else {
        return None;
    };
    if fields.is_empty() || !fields.iter().all(|f| written.contains(&f.position)) {
        return None;
    }
    // The buffer is the literal's alone: outside these statements nothing reads it, and
    // nothing binds it but a null (a `Set` names its variable by number, not as a `Var`).
    let inside: usize = ops[at..=last].iter().map(|o| mentions(o, buf)).sum();
    let mint_node = std::ptr::from_ref(mint.unspan());
    let mut foreign_bind = false;
    data.def(d_nr).code().walk(&mut |n| {
        if let Value::Set(x, v) = n
            && *x == buf
            && !matches!(v.unspan(), Value::Null)
            && !std::ptr::eq(v.unspan(), mint_node)
        {
            foreign_bind = true;
        }
    });
    if foreign_bind || mentions(data.def(d_nr).code(), buf) != inside {
        return None;
    }
    Some(Site {
        coll,
        parent_tp,
        field,
        key,
        writes,
        last,
    })
}
