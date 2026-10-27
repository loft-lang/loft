// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN204 — the 32-bit regime's boundary with 64-bit code.
//!
//! A file under the regime (`#width 32`, or every file but the stdlib under `LOFT_WIDTH=32`)
//! reads a plain `integer` as an `i32` and a plain `float` as a `single` (`parse_type_inner`),
//! types its literals so (`parse_primary`), and keeps 32-bit arithmetic 32-bit
//! (`handle_operator`).  What it still meets is a value typed by a 64-bit file: the result of a
//! stdlib function, or of a library that opted out with `#width 64`.  Such a value is narrowed
//! where the call answers it, so a 64-bit `integer` does not carry the whole expression it
//! lands in back to 64 bits.  A value that does not fit reads null, as an overflow does.  A
//! callee that NEEDS 64 bits says so with `i64` / `f64`, and those stay wide.

use super::{Parser, Type, Value};

impl Parser {
    /// The type a call answers in the file being parsed: under the 32-bit regime, a plain
    /// `integer` or `float` from a callee declared in a 64-bit file is narrowed to `i32` /
    /// `single` here, and `code` wrapped in the conversion.  Anything else is `tp` unchanged.
    pub(crate) fn width32_call_result(&mut self, code: &mut Value, tp: Type) -> Type {
        if !self.data.width32() {
            return tp;
        }
        let Value::Call(d, _) = code.unspan() else {
            return tp;
        };
        let def = self.data.def(*d);
        if def.name.starts_with("Op") || self.data.width32_source(def.source) {
            return tp;
        }
        let optional = matches!(tp, Type::Optional(_));
        let (op, narrow) = match tp.base() {
            Type::Integer(spec)
                if !spec.is_width32() && spec.forced_size.map(std::num::NonZeroU8::get) != Some(8) =>
            {
                (
                    "OpNarrow32",
                    Type::Integer(crate::narrow::IntegerSpec {
                        not_null: spec.not_null,
                        ..crate::narrow::IntegerSpec::i32()
                    }),
                )
            }
            Type::Float => ("OpCastSingleFromFloat", Type::Single),
            _ => return tp,
        };
        let inner = std::mem::replace(code, Value::Null);
        *code = self.cl(op, &[inner]);
        if optional { Type::optional(narrow) } else { narrow }
    }
}
