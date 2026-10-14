// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! What a constant collection literal writes, field by field: `[Row { flag: true, id: 5 }]`
//! read off the IR into [`ConstField`]s before anything runs.  The bytecode compiler builds
//! the interpreter's constant store from it and the native generator emits the same values,
//! so both match one exhaustive enum; the parser asks [`const_vector_blocker`] why a literal
//! cannot be pre-built.
use crate::data::{Data, Value};
use crate::data_store::ValueType;
use crate::ir_node::IrNode;

/// One literal field write inside a pre-built constant element: the value, in the terms
/// the store writes it in.
///
/// A dedicated enum rather than [`Value`]: the two writers — `build_const_vectors` here
/// and native codegen's `emit_const_vectors` — then match it EXHAUSTIVELY, so a field
/// kind added on one side cannot be silently dropped by the other.  That is how a
/// `boolean` and a `character` field came to be left out of a constant that was built
/// anyway: `[Row { flag: true, id: 5 }]` read back `flag = false` with `id` correct, and
/// nothing said so (loft#1090).
#[derive(Debug, Clone, PartialEq)]
pub enum ConstField {
    /// `integer`, written at the field's own width.
    Int(i64),
    /// A full-width 64-bit integer literal.
    Long(i64),
    Float(f64),
    Single(f32),
    /// `boolean`, stored as a 0/1 byte.
    Bool(bool),
    /// `character`, stored as a raw Unicode scalar.
    Char(u32),
    /// `text`, stored in the element's own store and referenced by record number.
    Text(String),
}

/// One ELEMENT of a vector constant: each literal field write it makes, as
/// `(byte offset within the element, value)`.
///
/// loft#702 — a scalar element makes exactly one write at offset 0, which is why a flat
/// list of values read as "one value per element" for as long as constants held only
/// scalars.  A struct element makes one per FIELD, so `[It { a: 3, b: 4 }]` looked like
/// TWO elements: the vector reported length 2, `b` reappeared as the next element's `a`,
/// and every second field read 0.  Keeping the offset is what tells the two apart.
pub type ConstElement = Vec<(u32, ConstField)>;

/// Walk the Block IR for a vector constant and extract its elements, each with the
/// literal field writes that fill it.
///
/// `Err(reason)` when the initialiser holds anything the const store cannot pre-build —
/// a value only known at run time, a field kind the store cannot be handed, or an
/// element the writes do not describe at all (a nested vector row lives in a store of
/// its own).  The reason is a noun phrase for the declaration's diagnostic to place in a
/// sentence: a constant that cannot be pre-built has no other way to exist, because the
/// use site references the constant store (`OpConstRef`) rather than re-running the
/// initialiser, so an unreported failure here IS a `null` at every use.
///
/// Both writers — the interpreter's `build_const_vectors` and native codegen's
/// `emit_const_vectors` — read THIS, so the two agree on what a constant contains by
/// construction rather than by review.
///
/// # Errors
///
/// The noun phrase naming what the constant store cannot pre-build.
pub fn extract_literal_values_public(
    code: &Value,
    data: &Data,
) -> Result<Vec<ConstElement>, String> {
    extract_literal_values(IrNode::Native(code), data)
}

/// What stops the constant store from pre-building this vector constant, or `None` when
/// nothing does.
///
/// The declaration asks this so it can refuse in the one place the reader can act on —
/// see [`extract_literal_values_public`] for why an unreported failure is a silent
/// `null` rather than a slower constant.
#[must_use]
pub fn const_vector_blocker(code: &Value, data: &Data) -> Option<String> {
    extract_literal_values_public(code, data).err()
}

/// A constant element's value, folded to the literal the store can write, or `None` when
/// it is only known at run time.
///
/// The write OPERATOR decides which literal is admissible, not the value that turns up:
/// a fold can answer a `boolean` for an arithmetic comparison, and writing that through
/// `OpSetInt` would put a byte where an integer belongs.  Asking the operator makes a
/// mismatch a refusal instead of a wrong element.
///
/// Folding matters because a constant element is written the way it is SPELLED, not the
/// way it evaluates: `-5` is `OpMinSingleInt(5)` (loft#955), `BASE + 1` is `OpAddInt`
/// under a span per operand (loft#1090), and `"a" + "b"` is a block that builds its
/// value in a work buffer.  None of them is a literal in the tree, and all three are
/// ordinary things to write in a table of rows.
fn fold_const_field(op: &str, v: &IrNode, data: &Data) -> Option<ConstField> {
    // A character literal is spelled as a conversion of its code point
    // (`OpConvCharacterFromInt(97)`), which is not arithmetic and has no literal form of
    // its own — read the code point straight out of it.
    if op == "OpSetCharacter" {
        let owned = v.to_owned_value();
        let inner = match owned.unspan() {
            Value::Call(d, args)
                if data.def(*d).name() == "OpConvCharacterFromInt" && args.len() == 1 =>
            {
                args[0].unspan().clone()
            }
            other => other.clone(),
        };
        let Value::Int(cp) = inner else { return None };
        return u32::try_from(cp).ok().map(ConstField::Char);
    }
    if op == "OpSetText" {
        return match v.to_owned_value().unspan() {
            Value::Text(t) => Some(ConstField::Text(t.clone())),
            other => crate::const_eval::fold_text_block(other, data).map(ConstField::Text),
        };
    }
    let folded = crate::const_eval::const_eval(v.to_owned_value().unspan(), data)?;
    // loft spells a null as its type's extreme value ([CODE.md](../doc/claude/CODE.md)
    // null sentinels).  A fold that lands on one would pre-build a null the author never
    // wrote — `-2147483648` is not a literal anyone can spell, but wrapping arithmetic
    // reaches it — so refuse instead, which leaves the constant reported rather than
    // quietly absent.
    match (op, folded) {
        ("OpSetInt", Value::Int(n)) if n != i32::MIN => Some(ConstField::Int(i64::from(n))),
        ("OpSetInt", Value::Long(n)) if n != i64::MIN => Some(ConstField::Long(n)),
        ("OpSetFloat", Value::Float(f)) if !f.is_nan() => Some(ConstField::Float(f)),
        ("OpSetSingle", Value::Single(f)) if !f.is_nan() => Some(ConstField::Single(f)),
        ("OpSetBoolean", Value::Boolean(b)) => Some(ConstField::Bool(b)),
        _ => None,
    }
}

pub(crate) fn extract_literal_values(
    code: IrNode,
    data: &Data,
) -> Result<Vec<ConstElement>, String> {
    if code.kind() != ValueType::Block {
        return Err("an initialiser that is not a vector literal".to_string());
    }
    let block = code.as_block();
    let mut elements: Vec<ConstElement> = Vec::new();
    // `OpNewRecord` opens an element and `OpFinishRecord` closes it; the field writes
    // between them belong to that element.  A vector literal emits this shape for a
    // scalar element type as well, so there is one rule, not two.
    let new_record_nr = data.def_nr("OpNewRecord");
    let finish_record_nr = data.def_nr("OpFinishRecord");
    // Whether a `OpNewRecord` is currently open.  The block also writes the VECTOR's own
    // header before the first element (its type id, through an `OpSet…` of its own), and
    // those writes belong to no element — the flag is what tells them apart from a field.
    let mut in_element = false;
    for op in block.operators().iter() {
        // A `Set(elm, OpNewRecord(…))` wraps the call, so look through the assignment.
        let op = if op.kind() == ValueType::Set {
            op.set_inner()
        } else {
            op
        };
        if op.kind() != ValueType::Call {
            continue;
        }
        let fn_nr = op.call_to();
        if fn_nr == new_record_nr {
            elements.push(Vec::new());
            in_element = true;
            continue;
        }
        if fn_nr == finish_record_nr {
            in_element = false;
            continue;
        }
        let name = data.def(fn_nr).name().to_string();
        // A fused scalar append (@PLN157 § V-m, `OpPush<Kind>(vector, val)`) is a whole
        // element in one op: the `OpSet<Kind>` at offset 0 that the three-op form wrote,
        // folded with the open and the close.  Read it as that element, through the same
        // fold and the same refusals.
        if let Some((setter, _)) = crate::parser::FUSED_PUSH_KINDS
            .iter()
            .find(|(_, push)| *push == name)
        {
            let args = op.call_args();
            let field = if args.len() == 2 {
                fold_const_field(setter, &args.get(1), data)
            } else {
                None
            };
            let Some(field) = field else {
                return Err(
                    "an element value that is only known at run time, or a field of a kind a \
                     constant cannot hold"
                        .to_string(),
                );
            };
            elements.push(vec![(0, field)]);
            in_element = false;
            continue;
        }
        if !in_element {
            continue;
        }
        // Every field write is an `OpSet…` of at least (record, offset, value).  Anything
        // else inside an element is bookkeeping and carries no field data; an `OpSet…`
        // this builder does not know is a field it cannot write, and falls to the refusal
        // below rather than being dropped.
        let args = op.call_args();
        if !name.starts_with("OpSet") || args.len() < 3 {
            continue;
        }
        let ValueType::Int = args.get(1).kind() else {
            return Err("an element field written at a computed offset".to_string());
        };
        let Value::Int(offset) = args.get(1).to_owned_value() else {
            return Err("an element field written at a computed offset".to_string());
        };
        let Some(field) = fold_const_field(&name, &args.get(2), data) else {
            // Two shapes reach here and one message covers both, because the cure is the
            // same: a value the fold cannot reduce (a call, a field read), and a field
            // whose kind the store cannot be handed through this operator (a reference,
            // an enum payload, a narrow biased integer).
            return Err(
                "an element value that is only known at run time, or a field of a kind a \
                 constant cannot hold"
                    .to_string(),
            );
        };
        // A write with no element open belongs to no element — the shape is not the one
        // this builder understands, so pre-build nothing rather than guess where the
        // bytes go.
        let Some(current) = elements.last_mut() else {
            return Err("a field write that belongs to no element".to_string());
        };
        current.push((offset as u32, field));
    }
    // An element none of the writes describe (a nested vector row, whose contents live in
    // a store of its own) would be pre-built EMPTY and read back empty.
    if elements.iter().any(Vec::is_empty) {
        return Err("an element the field writes do not describe".to_string());
    }
    if elements.is_empty() {
        return Err("no elements the constant store can build".to_string());
    }
    Ok(elements)
}
