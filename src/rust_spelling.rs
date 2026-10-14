// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

// @I68 — Native Rust generator: how a loft type is spelled in emitted Rust

//! How a loft type is spelled in the Rust the native generator emits.  The generator asks it
//! at every declaration; the parser asks it too — a fn-ref call's dispatch arms are the
//! definitions whose Rust signature matches (`fnref`) — so it sits in the core layer with
//! the types it spells (@PLN191).

use crate::data::{Context, Type};

/// Use this to map a loft type to the Rust type used in generated code.
/// The context controls whether the type appears as an owned value, argument, variable, or reference.
///
/// # Panics
/// When the rust type cannot be determined.
#[must_use]
pub fn rust_type(tp: &Type, context: &Context) -> String {
    if context == &Context::Reference {
        let mut result = String::new();
        result += "&";
        result += &rust_type(tp, &Context::Argument);
        return result;
    }
    if let Type::RefVar(in_tp) = tp {
        if is_raw_scalar_ref(tp) {
            return format!("*mut {}", link_base_type(in_tp));
        }
        return format!("&mut {}", rust_type(in_tp, &Context::Variable));
    }
    match tp {
        // Narrow integer subtypes use their precise Rust type only in the function-return
        // context.  In variable and argument contexts `i32` is used instead to avoid
        // cascading type-mismatch errors when the variable is passed to a template
        // operation (e.g. `set_short`) that expects `i32`.  The `return` site adds an
        // explicit `as u16` / `as u8` cast (see `narrow_int_cast`).
        //
        // WHICH narrow type is `native_narrow_int`'s answer, shared with `narrow_int_cast` so
        // the signature and the cast at the return site cannot disagree (loft#1255).
        Type::Integer(s)
            if context == &Context::Result && native_narrow_int(s).is_some() =>
        {
            native_narrow_int(s).unwrap_or("i64")
        }
        Type::Enum(_, false, _) => "u8",
        Type::Character | Type::Null => "i32",
        Type::Integer(_) => "i64",
        // null is represented as the null sentinel of the target type
        Type::Text(_) if context == &Context::Variable => "String",
        Type::Text(_) if context == &Context::Argument => "&str",
        Type::Text(_) => "Str",
        // @PLN17: boolean tri-state.  Null-capable positions (local/field storage,
        // params, returns) hold the raw byte `u8` (0/1/255); transient expression
        // results are 2-state `bool`.  Mirrors the narrow-int Result→u8 split above
        // and the text String/Str split; coercion to/from `bool` happens at the
        // op-operand (calls.rs `as u8` wrap), store, return, and arg seams.
        Type::Boolean
            if matches!(
                context,
                Context::Variable | Context::Argument | Context::Result | Context::Reference
            ) =>
        {
            "u8"
        }
        Type::Boolean => "bool",
        Type::Float => "f64",
        Type::Single => "f32",
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _) | Type::Trie(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Index(_, _, _)
        // N8b.1: generator variables are stored as DbRef (index into native coroutine table).
        // `@FR-O-One-Kind` — every heap value is one `DbRef` in native code too (@C125).
        | Type::Iterator(_, _) => "DbRef",
        Type::Routine(_) => "u32",
        // C39/A5.6: fn-ref carries d_nr + closure DbRef as a tuple.
        Type::Function(..) => "(u32, DbRef)",
        Type::Unknown(_) => "??",
        Type::Keys => "&[Key]",
        Type::Void => "()",
        // N8a.1: emit the correct Rust tuple type, e.g. (i32, i64) for (integer, long).
        // T1.8a: in Result context, recurse with Variable context for element
        // types so a tuple-of-text return signature is `(String, String)` —
        // matching the caller's owned-tuple slot.  Without this the signature
        // becomes `(Str, Str)` (Result-context Text → "Str") while the
        // caller's local declares `(String, String)` (Variable-context),
        // producing a type mismatch.  Argument/Variable contexts already
        // recurse with their own context (giving `(&str, &str)` or
        // `(String, String)` respectively), which matches the call site
        // and the variable slot.
        Type::Tuple(elems) => {
            let elem_context = if matches!(context, Context::Result) {
                &Context::Variable
            } else {
                context
            };
            let parts: Vec<String> = elems.iter().map(|e| rust_type(e, elem_context)).collect();
            return format!("({})", parts.join(", "));
        }
        // @PLN25: a nullable narrow integer is carried FULL-WIDTH (i64) as a Rust value.
        // The narrow packing is store-only; a narrow Rust type (u8/u16) can't hold the
        // null sentinel alongside the full value range (`u8?` must keep 255-as-value
        // distinct from null). Non-Result contexts already map a narrow Integer to i64,
        // so this only corrects the Result (function-return) signature to match the i64
        // the body yields — else a `-> u8` header over a `return <i64>` body is rustc E0308.
        Type::Optional(inner) if matches!(inner.base(), Type::Integer(_)) => {
            return "i64".to_string();
        }
        // @PLN25 slice (b): `Optional(τ)` shares its base's Rust type (sentinel storage).
        Type::Rewritten(inner) | Type::Optional(inner) => return rust_type(inner, context),
        _ => panic!("Incorrect type {tp:?}"),
    }
    .to_string()
}

/// The Rust type behind a scalar `&τ` pointer — `link_base_type(τ)` — which for a NARROW
/// integer is the storage width the field encoding uses (`data::NarrowSlot`, @PLN167
/// decision 1), so one `*mut u8` names a linked local and a field alike; every other τ is
/// its variable type.
#[must_use]
pub(crate) fn link_base_type(inner: &Type) -> String {
    match crate::data::NarrowSlot::of_type(inner) {
        Some(slot) => slot.rust_type().to_string(),
        None => rust_type(inner.base(), &Context::Variable),
    }
}

#[must_use]
pub(crate) fn is_raw_scalar_ref(tp: &Type) -> bool {
    matches!(tp.base(), Type::RefVar(inner) if crate::data::is_scalar(inner))
}

/// The narrow Rust type a value of this integer range fits UNENCODED, or `None` when it needs
/// the full width.
///
/// The ONE home for that question, asked by `rust_type`'s Result arms (which pick the
/// signature) and by [`narrow_int_cast`] (which casts the value at the return site).  They
/// have to agree or the generated code does not compile — and they were two copies of the
/// same four-line predicate, which is how they came to share a defect.
///
/// The test is which VALUES the range holds, not how MANY.  Read off the SPAN
/// (`range() - 1 <= 255`) both answered `u8` for `integer limit(300, 400)` — 101 values,
/// which fit a byte only OFFSET-encoded, which is what the store layer does and a register
/// does not.  A body of `350` came back as `94` on `--native` and `350` on `--interpret`,
/// silently, on every build back to the last release; `limit(70000, 70100)` is the cell that
/// shows it is not a u8-vs-u16 boundary question at all, since a 101-wide range picked `u8`
/// for a five-digit number (loft#1255).
///
/// `formal/layout.md` is why the answer is the values: the packed encoding is a STORAGE fact,
/// so the span belongs to `bytes_for_range` and never to a register.  A value in flight is
/// its plain number, which is what the interpreter's i64 slot already does.
pub(crate) fn native_narrow_int(s: &crate::data::IntegerSpec) -> Option<&'static str> {
    // ⚠ A NULLABLE spec never reaches here — `rust_type`'s Result arms peel the `Optional`
    // first — which is what makes the declared range the whole answer.  Between loft#1634 and
    // C127 a non-nullable spec that left a code spare inside its width carried one more value
    // in flight than its range held (the null its overflow answered), so this returned `None`
    // for those; C127 retired that null, so the range is again exactly what a value of the
    // type can be.  loft#1634's guard keeps the cell either way: what changes is the value
    // every frame boundary must agree on, from null to the type's default.
    let (min, max) = (i64::from(s.min), s.max);
    if min >= 0 && max <= 255 {
        Some("u8")
    } else if min >= 0 && max <= 65535 {
        Some("u16")
    } else if min >= -128 && max <= 127 {
        Some("i8")
    } else if min >= -32768 && max <= 32767 {
        Some("i16")
    } else {
        None
    }
}
