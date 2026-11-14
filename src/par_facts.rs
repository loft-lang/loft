// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! What a `par` worker's definition says about how it takes its row and what it hides — read
//! from the definitions, so the front end's run image (`ir_store`, @PLN91) and the
//! interpreter's dispatch (`parallel_natives`, `parallel`) read one answer.

/// Typed worker-input dispatch: an explicit enum rather than a
/// sentinel-encoded slot size, so the dispatcher's pre-loop code reads as a plain
/// match, and so wide-inline first args (tuples, fn-refs, anything
/// 9..=64 bytes) are not silently misrouted as "DbRef-by-pointer"
/// when their vector storage is actually inline.
///
/// Cap at 64 bytes: anything wider falls back to `Ref` so the
/// per-row push doesn't blow the worker stack frame budget.  64 is
/// one cache line plus headroom for 4-element tuples of Long /
/// Float.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum InputKind {
    /// Worker takes a `DbRef` in slot 0 (struct-by-ref, vector,
    /// keyed collection, or any inline-typed first arg whose slot
    /// width exceeds the 64-byte inline cap).
    Ref,
    /// Worker takes a 16-byte `Str` in slot 0.  The G3 path
    /// (text-input dispatch) hooks here.
    Text,
    /// Worker takes `size` bytes inline in slot 0.  `size` is the
    /// stack-representation width from `variables::size` (which
    /// reports 20 for fn-ref, 16 for `(int, int)`, etc.) — not the
    /// 4-byte vector storage width.
    Primitive { size: u8 },
}

/// Maximum bytes accepted for `InputKind::Primitive`.  Anything
/// wider falls back to `InputKind::Ref` so the worker stack frame
/// stays bounded.
pub(crate) const INPUT_PRIMITIVE_MAX_BYTES: usize = 64;

/// P189d — element types of a tuple first-arg, or `None` if the
/// worker's first arg isn't a tuple.  Used by the wide-input path
/// to dispatch `read_tuple_at_wide` (per-element inflation,
/// notably 4 B heap text-pointer → 16 B stack `Str`) instead of
/// `read_primitive_at_wide` (flat memcpy).
///
/// Also returns `Some(vec![first.typedef])`
/// for `Type::Function` first-args so the dispatcher routes through
/// `read_tuple_at_wide` which produces a clean failure mode (vs. the
/// hang from raw `read_primitive_at_wide` reading 20 bytes from a
/// 4-byte stride).
pub(crate) fn tuple_first_arg_types(
    def: &crate::data::Definition,
) -> Option<Vec<crate::data::Type>> {
    let first = def.attributes.first()?;
    match &first.typedef {
        crate::data::Type::Tuple(elems) => Some(elems.clone()),
        crate::data::Type::Function(..) => Some(vec![first.typedef.clone()]),
        _ => None,
    }
}

pub(crate) fn input_kind_for_first_arg(def: &crate::data::Definition) -> InputKind {
    use crate::data::Type;
    let Some(first) = def.attributes.first() else {
        // No args — treat as Ref (the dispatcher won't push anything
        // either way; this matches the legacy fall-through to the
        // DbRef-passing path).
        return InputKind::Ref;
    };
    match &first.typedef {
        Type::Text(_) => InputKind::Text,
        Type::Boolean | Type::Enum(_, false, _) => InputKind::Primitive { size: 1 },
        Type::Single | Type::Character => InputKind::Primitive { size: 4 },
        Type::Integer(_) | Type::Float => InputKind::Primitive { size: 8 },
        Type::Function(..) | Type::Tuple(_) => {
            let sz = crate::variables::size(&first.typedef, &crate::data::Context::Argument);
            if (sz as usize) <= INPUT_PRIMITIVE_MAX_BYTES && sz > 0 {
                InputKind::Primitive { size: sz as u8 }
            } else {
                InputKind::Ref
            }
        }
        _ => InputKind::Ref,
    }
}

/// A `par` worker's hidden parameters, classified by TYPE (a name-prefix rule misclassified
/// wrapper-promoted destinations as text buffers): its text work buffers, which a text queue makes
/// for each row, and its hidden return destinations (a record, a vector or a struct enum), which a
/// reference queue makes.  The one home for every reader of the two counts.
#[must_use]
pub(crate) fn worker_hidden_counts(def: &crate::data::Definition) -> (usize, usize) {
    let texts = def
        .attributes
        .iter()
        .filter(|a| is_text_work_buffer(&a.typedef))
        .count();
    let dests = def
        .attributes
        .iter()
        .filter(|a| {
            a.hidden
                && matches!(
                    a.typedef,
                    crate::data::Type::Reference(_, _)
                        | crate::data::Type::Vector(_, _)
                        | crate::data::Type::Enum(_, true, _)
                )
        })
        .count();
    (texts, dests)
}

/// How a `par` worker takes its row, as one number for the run image (@PLN91): 0 a reference to
/// the row, -1 a text, `n` a primitive of `n` bytes read from the row (`input_kind_for_first_arg`),
/// `1000 + n` a tuple of `n` bytes, which is re-laid out from the row (`read_tuple_at_wide`).
#[must_use]
pub(crate) fn worker_input_code(def: &crate::data::Definition) -> i64 {
    match input_kind_for_first_arg(def) {
        InputKind::Ref => 0,
        InputKind::Text => -1,
        InputKind::Primitive { size } if tuple_first_arg_types(def).is_some() => {
            1000 + i64::from(size)
        }
        InputKind::Primitive { size } => i64::from(size),
    }
}

/// How one element of a tuple row moves into a worker's argument slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TupleMove {
    /// Its bytes copied (`stored.min(arg)` of them).
    Copy,
    /// A 4-byte text record inflated to a 16-byte `Str`.
    Text,
    /// A 4-byte function number widened to a fn-ref whose closure is the null sentinel.
    FnRef,
}

/// One element of a tuple row: where it sits in the row, its stored width, its width as an
/// argument, and how it moves.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TupleElement {
    pub at: usize,
    pub stored: usize,
    pub arg: usize,
    pub kind: TupleMove,
}

/// How a `vector<(T1, T2, …)>` row is laid out for a worker taking the tuple
/// (`read_tuple_at_wide`, and the run image's `RunWorker.row`, @PLN91): one entry per element.
///
/// @PLN114 — the row is STORAGE, so its offsets and widths are the storage view; the stack view
/// walks a row with a narrow element at the wrong stride.  `@FR-L-Tuple` — the `__tuple<…>`
/// record's own finished positions where the program's definitions are at hand (a stored tuple
/// packs largest alignment first, and an inline struct member has a width the hand-computed
/// view cannot know); that view otherwise.
pub(crate) fn tuple_row_plan(
    data: Option<&crate::data::Data>,
    stores: &crate::database::Stores,
    elem_types: &[crate::data::Type],
) -> Vec<TupleElement> {
    let in_vec_offsets: Vec<usize> = data
        .and_then(|d| crate::data::stored_tuple_offsets(d, stores, elem_types))
        .map_or_else(
            || crate::data::element_storage_offsets(elem_types),
            |o| o.into_iter().map(usize::from).collect(),
        );
    elem_types
        .iter()
        .enumerate()
        .map(|(i, t)| TupleElement {
            at: in_vec_offsets[i],
            stored: crate::data::element_storage_size(t),
            arg: crate::variables::size(t, &crate::data::Context::Argument) as usize,
            kind: match t {
                crate::data::Type::Text(_) => TupleMove::Text,
                crate::data::Type::Function(..) => TupleMove::FnRef,
                _ => TupleMove::Copy,
            },
        })
        .collect()
}

/// Is `t` a `text_return` work buffer — a `&mut String` the inner fn appends the
/// result into (loft IR type `RefVar(Text)`)?  The bridge owns a local `String`
/// for each and passes `&mut`.  Shared with `native_lib` (the gate admits exactly
/// these as the one allowed `__`-prefixed param, and only for a text-returning fn).
pub(crate) fn is_text_work_buffer(t: &crate::data::Type) -> bool {
    matches!(t, crate::data::Type::RefVar(inner) if matches!(**inner, crate::data::Type::Text(_)))
}
