// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! What a function's attributes say about its calling convention.

use crate::data::Data;

use crate::data::Type;

/// The hidden RETURN-BUFFER attribute of a record-returning function — the parameter
/// `ref_return` appends, a `Reference` or struct-enum marked hidden — by position, or
/// `None` for a function that has none.  ONE predicate for the four sites that drop it:
/// its NAME is `__retbuf` when the parser minted the buffer and the promoted LOCAL's own
/// name (`__ref_3`, `p`) when the buffer IS that local, so a name test sees half of them
/// (measured: `half_chord`'s buffer is `__ref_3`, and the call site kept passing it to a
/// signature that had dropped it).
#[must_use]
pub fn ret_buffer_attr(def: &crate::data::Definition) -> Option<usize> {
    def.attributes().iter().rposition(|a| {
        a.hidden
            && matches!(
                a.typedef.base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
    })
}

/// The variable of `def_nr`'s hidden return buffer, when it has one.
pub fn retbuf_var(data: &Data, def_nr: u32) -> Option<u16> {
    let def = data.def(def_nr);
    let attr = def.hidden_return_buffer_attr()?;
    let v = def.variables().var(&def.attributes()[attr].name);
    (v != u16::MAX).then_some(v)
}
