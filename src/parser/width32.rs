// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

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

use super::{Level, Parser, Type, Value, diagnostic_format};

impl Parser {
    /// The regime of the file the lexer has just begun: under `LOFT_WIDTH=32` every source
    /// but the stdlib, unless the file's first line is `#width 64`; elsewhere only a file
    /// whose first line is `#width 32`.  Decided once per source, so a file resumed after a
    /// `use` keeps it.  The `#width` line is consumed; any other `#` line is left in place.
    pub(crate) fn width_header(&mut self) {
        let src = self.data.source;
        if src != crate::data::STD_SOURCE && !self.width_decided.contains(&src) {
            self.width_decided.insert(src);
            self.data.set_width32(src, crate::keys::width32_all());
        }
        if !self.lexer.peek_token("#") {
            return;
        }
        let back = self.lexer.link();
        self.lexer.has_token("#");
        if self.lexer.has_identifier().as_deref() != Some("width") {
            self.lexer.revert(back);
            return;
        }
        self.width_directive(src);
    }

    /// The rest of a `#width` line, its `#width` read: `32` puts source `src` under the
    /// regime, `64` keeps it out.
    pub(crate) fn width_directive(&mut self, src: u16) {
        let bits = self.lexer.has_integer();
        match bits {
            Some(32) => self.data.set_width32(src, src != crate::data::STD_SOURCE),
            Some(64) => self.data.set_width32(src, false),
            _ => diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown width '#width {}' (expected '#width 32' or '#width 64')",
                bits.map_or_else(String::new, |b| b.to_string())
            ),
        }
        let _ = self.lexer.has_token(";");
    }

    /// Under the 32-bit regime, a plain `integer` — a width nobody chose: an element's
    /// `#index`, a 64-bit file's field — stored into a slot that holds the whole `i32` range
    /// is narrowed there (`OpNarrow32`, null when it does not fit) rather than refused.  An
    /// `i64` or `u32` source, and a target narrower than `i32`, keep the refusal.  Answers
    /// whether `code` was wrapped.
    pub(crate) fn width32_narrow_store(
        &mut self,
        code: &mut Value,
        source: &Type,
        target: &Type,
    ) -> bool {
        if !self.data.width32() {
            return false;
        }
        let (Type::Integer(src), Type::Integer(dst)) = (source.base(), target.base()) else {
            return false;
        };
        if src.max > i64::from(i32::MAX)
            || !dst.is_width32()
            || i64::from(dst.min) > i64::from(i32::MIN) + 1
            || dst.max < i64::from(i32::MAX)
        {
            return false;
        }
        if !matches!(code.unspan(), Value::Call(d, _) if self.data.def(*d).name() == "OpNarrow32") {
            let inner = std::mem::replace(code, Value::Null);
            *code = self.cl("OpNarrow32", &[inner]);
        }
        true
    }

    /// The type a call answers in the file being parsed: under the 32-bit regime, an integer
    /// inside the `i32` range by its type (not `i64` or `u32`) or a plain `float` from a callee
    /// whose body is 64-bit ([`crate::data::Data::body_width32`]) is
    /// narrowed to 32 bits here, and `code` wrapped in the conversion.  Anything else is `tp`
    /// unchanged.
    pub(crate) fn width32_call_result(&mut self, code: &mut Value, tp: Type) -> Type {
        if !self.data.width32() {
            return tp;
        }
        let Value::Call(d, _) = code.unspan() else {
            return tp;
        };
        let d = *d;
        if self.data.def(d).name.starts_with("Op") || self.data.body_width32(d) {
            return tp;
        }
        let optional = matches!(tp, Type::Optional(_));
        let (op, narrow) = match tp.base() {
            // Every integer but an `i64` (asked for: the wide template) and a `u32` (whose
            // upper half no `i32` holds) — neither has its upper bound inside `i32`.  A 32-bit
            // type is narrowed too: a 64-bit file computes it in 64 bits — a generic `sum<T>`
            // over a `vector<i32>` answers a `T` past the `i32` range — and keeps that type.
            Type::Integer(spec) if spec.max <= i64::from(i32::MAX) => {
                let narrow = if spec.is_width32() {
                    tp.base().clone()
                } else {
                    Type::Integer(crate::narrow::IntegerSpec {
                        not_null: spec.not_null,
                        ..crate::narrow::IntegerSpec::i32()
                    })
                };
                ("OpNarrow32", narrow)
            }
            Type::Float => ("OpCastSingleFromFloat", Type::Single),
            _ => return tp,
        };
        let narrow = if optional {
            Type::optional(narrow)
        } else {
            narrow
        };
        // A COUNT — a collection's `len`, a text's `size` — is typed 32-bit without the
        // conversion: no collection past `i32::MAX` elements fits a 32-bit address space, and
        // the shape `len(v)` is what the index proofs read (`if i < len(v) { v[i] }`).
        if op == "OpNarrow32" && Self::is_count_fn(self.data.def(d).name()) {
            return narrow;
        }
        let inner = std::mem::replace(code, Value::Null);
        *code = self.cl(op, &[inner]);
        narrow
    }

    /// The other side of [`Self::width32_call_result`]: a 64-bit file calling a function whose
    /// body is under the regime hands its arguments across the same boundary.  A plain
    /// `integer` for a parameter the regime made 32-bit (a whole-range `i32`) is narrowed at
    /// the call (`OpNarrow32`, null when it does not fit), and a `float` for a `single` one is
    /// rounded (`OpCastSingleFromFloat`), so a library can go under the regime without its
    /// 64-bit callers changing a line.  An `i64` / `f64` parameter is wide and takes the value
    /// as it is; a 32-bit caller needs nothing here.  Answers the argument's new type.
    pub(crate) fn width32_call_argument(
        &mut self,
        code: &mut Value,
        actual: &Type,
        param: &Type,
        callee: u32,
    ) -> Option<Type> {
        if self.data.width32()
            || self.data.def(callee).name.starts_with("Op")
            || !self.data.body_width32(callee)
        {
            return None;
        }
        let (op, to) = match (actual.base(), param.base()) {
            (Type::Integer(src), Type::Integer(dst))
                if src.max > i64::from(i32::MAX)
                    && dst.is_width32()
                    && i64::from(dst.min) <= i64::from(i32::MIN) + 1
                    && dst.max >= i64::from(i32::MAX) =>
            {
                ("OpNarrow32", param.base().clone())
            }
            (Type::Float, Type::Single) => ("OpCastSingleFromFloat", Type::Single),
            _ => return None,
        };
        let inner = std::mem::replace(code, Value::Null);
        *code = self.cl(op, &[inner]);
        Some(if matches!(actual, Type::Optional(_)) {
            Type::optional(to)
        } else {
            to
        })
    }

    /// A function answering how many elements, characters or bytes something holds.
    fn is_count_fn(name: &str) -> bool {
        matches!(name, "n_len" | "n_size")
            || (name.starts_with("t_") && (name.ends_with("_len") || name.ends_with("_size")))
    }
}
