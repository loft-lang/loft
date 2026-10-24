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
//! of the old entry), and the literal's buffer named nowhere else.  The TEXT clause admits one
//! `text` key over an element whose every other field is a plain value: the key is the only
//! heap the element owns, and an overwrite never writes it — the entry found carries the same
//! text, and a fresh one has it written by `Stores::keyed_entry_text` — so nothing is stranded.

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

/// `@FR-R-KeyedEntry`'s SUBSCRIPT clause — `coll[k₁, …] = T{…}`, the `(R-InPlaceLiteral)`
/// keyed site whose literal writes its fields with `writes` (each `(setter, byte offset)`):
/// the offsets of the hash's key parts when the entry under the key may be OVERWRITTEN in
/// place of removed and claimed again.
///
/// Admitted when that overwrite IS the replace: `tp` a `hash` keyed by one to four `integer`
/// / `long` parts, whose element owns no heap (nothing an overwrite would strand) and is no
/// member of a linked group (a group's dedup decides per member), and whose every field that
/// is not a key part is a plain value written by the literal through a scalar setter — so an
/// overwrite leaves nothing of the old entry.  A key part needs no write: the entry carries
/// the subscript's key whether it was found or filed, which is what the plain form's
/// `set_key` leaves.
#[must_use]
pub fn subscript_keys(stores: &Stores, tp: u16, writes: &[(&str, u16)]) -> Option<Vec<u16>> {
    if !enabled() || !writes.iter().all(|(op, _)| SETTERS.contains(op)) {
        return None;
    }
    let t = stores.types.get(tp as usize)?;
    let Parts::Hash(c, _) = t.parts else {
        return None;
    };
    if t.keys.is_empty()
        || t.keys.len() > 4
        || !t
            .keys
            .iter()
            .all(|k| matches!(k.type_nr.unsigned_abs(), 1 | 2))
        || stores.is_linked(c)
        || stores.owns_heap(c)
    {
        return None;
    }
    let key_pos: Vec<u16> = t.keys.iter().map(|k| k.position).collect();
    let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
        &stores.types.get(c as usize)?.parts
    else {
        return None;
    };
    let plain = |f: &crate::database::Field| {
        key_pos.contains(&f.position)
            || (stores.types.get(f.content as usize).is_some_and(|t| {
                matches!(
                    t.parts,
                    Parts::Base
                        | Parts::Enum(_)
                        | Parts::Byte(..)
                        | Parts::Short(..)
                        | Parts::Int(..)
                        | Parts::ShortRaw(..)
                        | Parts::IntRaw(..)
                )
            }) && writes.iter().any(|(_, o)| *o == f.position))
    };
    if fields.is_empty() || !fields.iter().all(plain) {
        return None;
    }
    Some(key_pos)
}

/// An admitted insert: statements `first ..= last` of a block are `B = OpNewRecord(coll, tp,
/// fld)`, the literal's field writes through `B`, and `OpFinishRecord(coll, B, tp, fld)`.
pub struct Site<'a> {
    pub coll: &'a Value,
    pub parent_tp: u16,
    pub field: u16,
    /// The values written to the key parts, in the key's part order.
    pub keys: Vec<&'a Value>,
    /// The key is a `text` (the text clause): the entry is answered by `keyed_entry_text`.
    pub text_key: bool,
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
/// with its one key's position and whether that key is a `text`, when an overwrite of its entry
/// is a replace.
fn heap_free_hash(stores: &Stores, parent_tp: u16, field: u16) -> Option<(u16, Vec<u16>, bool)> {
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
    if stores.is_linked(c) {
        return None;
    }
    // The compound clause: two to four `integer` / `long` parts of a heap-free element.
    if (2..=4).contains(&t.keys.len()) {
        return (t
            .keys
            .iter()
            .all(|k| matches!(k.type_nr.unsigned_abs(), 1 | 2))
            && !stores.owns_heap(c))
        .then(|| (c, t.keys.iter().map(|k| k.position).collect(), false));
    }
    let [k] = &t.keys[..] else {
        return None;
    };
    match k.type_nr.unsigned_abs() {
        1 | 2 if !stores.owns_heap(c) => Some((c, vec![k.position], false)),
        6 if only_heap_is_key(stores, c, k.position) => Some((c, vec![k.position], true)),
        _ => None,
    }
}

/// Is the field at `key_pos` the only heap element type `c` owns — every other field a plain
/// value an overwrite replaces whole?
fn only_heap_is_key(stores: &Stores, c: u16, key_pos: u16) -> bool {
    let Some(Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
        stores.types.get(c as usize).map(|t| &t.parts)
    else {
        return false;
    };
    !fields.is_empty()
        && fields.iter().all(|f| {
            f.position == key_pos
                || stores.types.get(f.content as usize).is_some_and(|t| {
                    matches!(
                        t.parts,
                        Parts::Base
                            | Parts::Enum(_)
                            | Parts::Byte(..)
                            | Parts::Short(..)
                            | Parts::Int(..)
                            | Parts::ShortRaw(..)
                            | Parts::IntRaw(..)
                    )
                })
        })
}

/// Does `op` write a field of the literal: a scalar setter, or the key's text write under the
/// text clause?
fn field_setter(op: &str, text_key: bool) -> bool {
    SETTERS.contains(&op) || (text_key && op == "OpSetText")
}

/// What a literal's field write is to the clause.
enum KeyWrite {
    /// The write of key part `n`.
    Part(usize),
    /// The write of a field that is not a key part.
    Field,
    /// A write the clause declines: a key part written by anything but its setter, or a text
    /// write to a field that is not the key — every other field is a plain value.
    Decline,
}

fn writes_key(op: &str, off: u16, key_pos: &[u16], text_key: bool) -> KeyWrite {
    let key_setter = if text_key { "OpSetText" } else { "OpSetInt" };
    match (key_pos.iter().position(|p| *p == off), op == key_setter) {
        (Some(part), true) => KeyWrite::Part(part),
        (None, _) if op != "OpSetText" => KeyWrite::Field,
        _ => KeyWrite::Decline,
    }
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
    let (content, key_pos, text_key) = heap_free_hash(stores, parent_tp, field)?;
    let mut key: Vec<Option<&Value>> = vec![None; key_pos.len()];
    // The collection is evaluated once for the entry: a pure path, the place the plain form's
    // claim and finish both name.
    crate::opt::writes::vector_path(data, coll)?;
    let mut roots: Vec<u16> = Vec::new();
    coll.walk(&mut |m| {
        if let Value::Var(x) = m.unspan() {
            roots.push(*x);
        }
    });
    let mut writes: Vec<&Value> = Vec::new();
    let mut written: Vec<u16> = Vec::new();
    let mut last = None;
    for (j, op) in ops.iter().enumerate().skip(at + 1) {
        match op.unspan() {
            Value::Line(_) => {}
            Value::Call(c, a)
                if name(data, *c) == "OpFinishRecord"
                    && a.get(1).is_some_and(|x| is_var(x, buf)) =>
            {
                let [fin_coll, _, fin_tp, fin_fld] = &a[..] else {
                    return None;
                };
                if fin_coll.unspan() != coll.unspan() || fin_tp != ptp || fin_fld != fld {
                    return None;
                }
                last = Some(j);
                break;
            }
            Value::Call(c, a)
                if field_setter(name(data, *c), text_key)
                    && a.first().is_some_and(|x| is_var(x, buf))
                    && a.iter()
                        .skip(1)
                        .all(|x| mentions(x, buf) == 0 && !rebinds(x, &roots)) =>
            {
                let off = u16::try_from(int_of(a.get(1)?)?).ok()?;
                if written.contains(&off) {
                    return None;
                }
                written.push(off);
                match writes_key(name(data, *c), off, &key_pos, text_key) {
                    KeyWrite::Part(part) => key[part] = Some(a.get(2)?),
                    KeyWrite::Field => {}
                    KeyWrite::Decline => return None,
                }
                writes.push(op);
            }
            _ => return None,
        }
    }
    let last = last?;
    let keys: Vec<&Value> = key.into_iter().collect::<Option<_>>()?;
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
        keys,
        text_key,
        writes,
        last,
    })
}
