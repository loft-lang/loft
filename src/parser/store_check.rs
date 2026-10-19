// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! What may be STORED into a slot (`@FR-N-Store`, `@FR-I-Narrow`): the implicit checked
//! narrowing between integer widths, the constants a narrow or nullable-narrow slot refuses,
//! and the null-into-non-null and `τ?`-into-`τ` reports with their consequence clauses.

use super::{Level, Parser, Position, Type, Value, diagnostic_format};

impl Parser {
    /// @PLAN48 P2: true when converting `src` → `dst` narrows a loft integer to a
    /// smaller explicit width (e.g. `integer` → `i32`, or `i32` → `u8`), which
    /// loses data.  Widening (`i32` → `integer`) and same-width are not narrowing.
    /// A plain `integer`/`wide`/`u32` has no `forced_size` and is treated as 8 bytes.
    /// The noun a const-modification diagnostic should use — "const parameter" for one the
    /// author declared, "const variable" for a local.  The one place that joins the two facts
    /// the answer needs, because they live apart: the variable table knows the variable
    /// OCCUPIES an argument slot, and only the DEFINITION knows whether that slot is a hidden
    /// one the return mechanism promoted it into.
    ///
    /// `text_return` / `ref_return` set `attributes[…].hidden` and call `become_argument` at
    /// the same moment, so a heap-typed local supplying the function's return value is an
    /// argument by the time any diagnostic sees it — and a const one was reported as a "const
    /// PARAMETER" in a function that has none (loft#1252).  Matching by NAME is what
    /// `scopes.rs`'s worker-output check already does with the same pair of facts.
    pub(super) fn const_noun(&self, var_nr: u16) -> &'static str {
        let promoted =
            self.context != u32::MAX && (self.context as usize) < self.data.definitions.len() && {
                let name = self.vars.name(var_nr);
                self.data
                    .def(self.context)
                    .attributes()
                    .iter()
                    .any(|a| a.hidden && a.name == name)
            };
        self.vars.const_kind(var_nr, promoted)
    }

    /// @FR-I-Narrow, completed by @PLN25 DN4 `(N-Cast?)` — the implicit CHECKED narrowing
    /// a NULLABLE narrow target takes in place of a refusal.  Answers whether it applied.
    ///
    /// `(I-Narrow)` says a narrowing store is not implicit: it needs an explicit `as` or a
    /// literal that plainly fits.  DN4 completes that for the one target which has
    /// somewhere to put a miss — an out-of-range value lands in a `u8?` as a VISIBLE
    /// `null`, never as a truncation — so the store needs no `as`, and the diagnostic that
    /// refuses the NON-nullable spelling already names `u8?` as the cure.  A non-nullable
    /// narrow target has no such value and is still refused.
    ///
    /// This is the ONE home because the two seams that store into a slot ask the question
    /// in different words.  [`Self::convert`] reaches the struct literal, the call argument
    /// and the return; the ASSIGNMENT seam never reaches `convert` for this pair at all,
    /// because `integer` and `integer(0,255)?` are `is_equal` — so it runs its own
    /// narrowing test, and that test is a REFUSAL with no checked-cast arm.  Read off
    /// separately the two disagreed: `d: u8? = p + 10` kept **260**, a value outside the
    /// range its own type declares, while `f(p + 10)` on a `u8?` parameter and
    /// `S { u: p + 10 }` on a `u8?` field both answered null (loft#1246).
    ///
    /// The `limit(lo, hi)` spelling of the same range comes here too (loft#1593): it is a
    /// narrow target under `(I-Narrow)` exactly as an alias is, so a nullable `limit` slot
    /// takes the checked cast and a non-nullable one is refused.  The runtime
    /// `OpRangeDefault` the seams apply (`guard_declared_range`) is what bounds an
    /// arithmetic OVERFLOW landing in such a slot (`(E-Uncomp-NN)`), never a substitute for
    /// this refusal — a default nothing reports is not the null `??` can recover from.
    pub(super) fn implicit_checked_narrow(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
    ) -> bool {
        if self.first_pass || !Self::takes_checked_narrow(is_type, should) {
            return false;
        }
        let Type::Optional(inner) = should else {
            return false;
        };
        // The assignment seam reaches this without `convert` (whose entry asks the same
        // question), so it asks too; the constant is then replaced by the null it would have
        // become, so the seam's own check (`narrow_store_checks`) does not report it twice.
        if let Some(msg) = self.nullable_narrow_constant_refusal(code, should) {
            self.refuse_nullable_narrow_constant(&msg);
            *code = self.cl("OpConvIntFromNull", &[]);
            return true;
        }
        let dst_base = inner.base().clone();
        let src_base = is_type.base().clone();
        self.dn4_checked_cast(code, &dst_base, &src_base);
        true
    }

    /// Does a value of `is_type` meet `should` through the implicit checked narrowing?  A
    /// question of the two TYPES alone, so pass 1 — which converts nothing — can ask it too.
    ///
    /// It asks the implicit-STORE test, not range containment: this is a store, and
    /// `integer` → `i32` is the one narrowing containment cannot see (loft#931).  Asking
    /// containment left `i32?` outside `(I-Narrow-Opt)` alone of every width — its local
    /// kept `5000000000`, its field answered a `??` fallback where every other width
    /// answers null, and its return was refused with `?? <value>`, a cure a nullable slot
    /// cannot take (loft#1812).
    pub(crate) fn takes_checked_narrow(is_type: &Type, should: &Type) -> bool {
        let Type::Optional(inner) = should else {
            return false;
        };
        Self::is_narrowing_int_store(is_type.base(), inner.base())
    }

    /// Does converting a `from` value to `to` change its representation — a numeric widening
    /// (`@FR-C-Num`), at the top or inside a tuple member (`@FR-C-Tuple`)?  An integer width
    /// change is not one: every integer member is carried full-width until it is stored.
    /// A tuple in its STORED spelling (`Reference(__tuple<…>)`, loft#822) is asked by its
    /// members like the stack spelling.
    pub(crate) fn changes_representation(&self, from: &Type, to: &Type) -> bool {
        let members = |tp: &Type| match tp.base() {
            Type::Tuple(m) => Some(m.clone()),
            Type::Reference(d, _) if self.data.def(*d).name().starts_with("__tuple<") => {
                Some(self.stored_tuple_elements(tp.base()))
            }
            _ => None,
        };
        if let (Some(a), Some(b)) = (members(from), members(to)) {
            return a.len() == b.len()
                && a.iter()
                    .zip(&b)
                    .any(|(x, y)| self.changes_representation(x, y));
        }
        Self::is_numeric_widening(from, to)
    }

    /// `@FR-I-Sub` / `@FR-C-Int` — `Integer[a,b] <: Integer[c,d]` iff `[a,b] ⊆ [c,d]`, and this
    /// is its one home: every implicit integer flow asks it.  `false` is `@FR-I-Widen` (a
    /// superset target is implicit); `true` hands the store to `@FR-I-Narrow`.
    pub(super) fn is_narrowing_int(src: &Type, dst: &Type) -> bool {
        let (Type::Integer(s), Type::Integer(d)) = (src, dst) else {
            return false;
        };
        // The integer model (formal/types.md § the integer model): width lives in the
        // value RANGE, and `(I-Narrow)` is stated over ANY `Integer[a, b]` target.  The
        // FULL integer is the one target nothing narrows to, and it has two bound encodings
        // (`IntegerSpec`'s i32/u32 bounds cannot hold the i64 range: the `signed32` template
        // ends at i32::MAX, the `wide` one at u32::MAX), so it is recognised by the two
        // template predicates `source_name` already reads — AND by carrying no
        // `forced_size`: the `i32` ALIAS has the signed-32 template's exact range and a
        // 4-byte storage, and by range alone it read as the full integer, which let
        // `10000000000 as i32` compile and print 10000000000 (the error-message golden
        // `24_runtime_narrow_cast_overflow` is the guard that caught it).  Every other spec
        // — a width alias with a `forced_size`, or a user-written `limit(lo, hi)` without
        // one — is a narrow target: `src` narrows into it iff `[s.min, s.max] ⊄ [d.min,
        // d.max]`, the range+sign test codegen's `narrow_int_cast` uses, so the two width
        // derivations agree (D2/D3/D5); containment also makes signedness visible (`i8` is
        // not contained in `u8`).
        //
        // Keying this on `forced_size` ALONE — "no forced size means the full integer" —
        // left every user-written `limit` outside all three narrowing rules at once: an
        // unranged store, call or literal into `integer limit(0, 7)` compiled, and the
        // runtime range guard then answered a legal in-range DEFAULT that nothing reports in
        // an ordinary run, where the same store into `u8` is refused with the cure named;
        // and the checked cast `as integer limit(0, 7)?` was a no-op (loft#1593).
        if d.is_full_integer() {
            return false;
        }
        s.min < d.min || s.max > d.max
    }

    /// The narrowing test for an IMPLICIT store — a local annotation, an argument, a
    /// field initialiser or a field write.  [`Self::is_narrowing_int`]'s range
    /// containment plus the one pair it cannot see: `integer` → `i32`.
    ///
    /// Every narrow alias but `i32` is declared with a `limit(…)`, so its range is
    /// genuinely tighter and containment catches it.  `i32` spans the whole 32-bit range,
    /// which is the range `integer` REPORTS — the plain integer's bounds are `i32::MIN+1
    /// ..= i32::MAX` with the 64-bit value living in an 8-byte slot the bounds do not
    /// describe.  So the two specs differ in `forced_size` alone, `[s.min,s.max] ⊆
    /// [d.min,d.max]` holds for a pair whose storage drops from 8 bytes to 4, and the one
    /// alias whose NAME says "32 bits" was the one the width discipline never checked:
    /// an `i32` field took `5000000000` and stored `705032704` with no diagnostic, on
    /// both backends (loft#931).
    ///
    /// Comparing STORAGE WIDTH is what sees it, and it is a strictly wider test than
    /// containment rather than a different one — a range that does not fit cannot have a
    /// narrower width.  It lives here, apart from `is_narrowing_int`, because DN4's
    /// explicit-cast rule must NOT adopt it: `as τ` is refused by DN4 whenever the value
    /// is not provably in range, so widening that predicate would start demanding
    /// `as i32?` at 56 sites across loft's own tests and libraries that today write
    /// `f += (i as i32)` — and, more to the point, `as i32` is the very cure this
    /// diagnostic prescribes.  It has to stay spellable.
    pub(super) fn is_narrowing_int_store(src: &Type, dst: &Type) -> bool {
        if Self::is_narrowing_int(src, dst) {
            return true;
        }
        let (Type::Integer(s), Type::Integer(d)) = (src, dst) else {
            return false;
        };
        let Some(d_width) = d.forced_size.map(std::num::NonZeroU8::get) else {
            return false;
        };
        // Only the full integer's bounds under-report its value; any other source's range
        // is its value's, and containment has already answered for it.  A contained
        // `limit` range wider than 65 536 codes has no 4-byte width bucket
        // (`range_to_width` answers 8), so asking width of it refused `x & 2147483647`
        // into an `i32` — the mask the refusal itself offers as the cure (loft#1814).
        let full = s.is_full_integer();
        full && s.byte_width(false) > d_width
    }

    /// @PLAN48 P2: render an integer type with its explicit narrow alias
    /// (`i32`/`u8`/`u16`/`i8`/`i16`) so a narrowing diagnostic doesn't print
    /// the bare `integer` for both sides (they share bounds).
    pub(super) fn int_type_name(&self, t: &Type) -> String {
        self.data.integer_name(t)
    }

    /// @PLAN48 P2: literal exemption — true when `code` is a constant integer that
    /// provably fits `dst`'s full declared range, so `x: i32 = 5` / `f(5)` /
    /// `f(65535)` to a `u16` param stay legal without `as`.  This is the TYPE-fit
    /// question (a full-width register value); the narrower nullable-narrow-FIELD
    /// sentinel reservation is a separate, store-only check
    /// (`nullable_narrow_constant_refusal`) applied at the store face.
    ///
    /// `@FR-I-Lit` — a literal checks at the width EXPECTED of it, and a value `if` or `match`
    /// passes that expectation to each arm (`@FR-T-Chk`), so one whose every arm is a fitting
    /// constant fits: `x: u8 = if c { 200 } else { 3 }` was refused while each arm alone was
    /// accepted.  A value block's answer is its last operator.
    pub(super) fn int_value_fits(&self, code: &Value, dst: &Type) -> bool {
        let Type::Integer(spec) = dst else {
            return false;
        };
        match code.unspan() {
            Value::If(_, then, other) => {
                self.int_value_fits(then, dst) && self.int_value_fits(other, dst)
            }
            Value::Block(bl) if bl.name != "ncc" => bl
                .operators
                .last()
                .is_some_and(|last| self.int_value_fits(last, dst)),
            // The integer null loses no data, so it is no narrowing — an exhaustive `match`
            // ends in one for the arm no value reaches.  Whether the slot may hold null is
            // `@FR-N-Store`'s question, asked apart.
            Value::Call(d, a)
                if a.is_empty() && self.data.def(*d).name() == "OpConvIntFromNull" =>
            {
                true
            }
            _ => self
                .int_range(code)
                .is_some_and(|(lo, hi)| lo >= i64::from(spec.min) && hi <= spec.max),
        }
    }

    /// The source a narrowing refusal names: the value's type, and the range its shape proves
    /// when there is one (`integer (here 0..=510)`), so the author sees how far it misses.
    pub(super) fn narrow_source_name(&self, code: &Value, tp: &Type) -> String {
        let name = self.int_type_name(tp);
        match self.int_range(code) {
            Some((lo, hi)) if lo != hi && Self::declared_range(tp) != Some((lo, hi)) => {
                format!("{name} (here {lo}..={hi})")
            }
            _ => name,
        }
    }

    fn declared_range(tp: &Type) -> Option<(i64, i64)> {
        match tp.base() {
            Type::Integer(sp) if sp.min > i32::MIN + 1 || sp.max < i64::from(i32::MAX) => {
                Some((i64::from(sp.min), sp.max))
            }
            _ => None,
        }
    }

    /// `@FR-N-Arith` — the values an integer EXPRESSION can take, as far as its own shape
    /// proves them: a constant is its value, a local or a call is its DECLARED range, a narrow
    /// field or element read is its width, `+ - *` the interval arithmetic of their operands,
    /// `a & c` (c ≥ 0) is `[0, c]`, `a % c` is `[-(|c|-1), |c|-1]` — `[0, |c|-1]` when `a`
    /// cannot be negative — and `x ?? d` the union of its two.  `None` is "anything an integer
    /// holds".
    ///
    /// The range is asked HERE, where a narrow destination needs it, and never written into the
    /// expression's type (owner ruling 2026-10-19): `x & 255` is an `integer`, a local bound to
    /// it is an `integer`, and only the store that meets the mask proves the fit.
    pub(super) fn int_range(&self, code: &Value) -> Option<(i64, i64)> {
        if let Some(n) = self.const_int(code) {
            return Some((n, n));
        }
        let declared = |tp: &Type| Self::declared_range(tp);
        match code.unspan() {
            Value::Var(v) => declared(self.vars.tp(*v)),
            Value::If(_, then, other) => {
                let ((a, b), (c, d)) = self.int_range(then).zip(self.int_range(other))?;
                Some((a.min(c), b.max(d)))
            }
            // `@FR-N-Coal` — `x ?? d` lowers to `{ tmp = x; if tmp { tmp } else { d } }`: it
            // answers the subject's values or the default's, so its range is their union.  Read
            // from the subject rather than from the temp, whose type is the join's.
            Value::Block(bl) if bl.name == "ncc" => {
                let [Value::Set(_, subject), last] = &bl.operators[..] else {
                    return None;
                };
                let Value::If(_, _, default) = last.unspan() else {
                    return None;
                };
                let ((a, b), (c, d)) = self.int_range(subject).zip(self.int_range(default))?;
                Some((a.min(c), b.max(d)))
            }
            Value::Block(bl) => self.int_range(bl.operators.last()?),
            Value::Call(d, args) => {
                let name = self.data.def(*d).name();
                let operand = |i: usize| args.get(i).and_then(|a| self.int_range(a));
                let min_arg = || args.get(2).and_then(|a| self.const_int(a));
                match name {
                    "OpGetByte" | "OpGetByteNullable" => min_arg().map(|m| (m, m + 255)),
                    "OpGetShort" | "OpGetShortRaw" | "OpGetShortSpare" | "OpGetShortFull" => {
                        min_arg().map(|m| (m, m + 65535))
                    }
                    "OpAddInt" | "OpMinInt" | "OpMulInt" => {
                        let ((al, ah), (bl, bh)) = operand(0).zip(operand(1))?;
                        let (lo, hi) = match name {
                            "OpAddInt" => (al.saturating_add(bl), ah.saturating_add(bh)),
                            "OpMinInt" => (al.saturating_sub(bh), ah.saturating_sub(bl)),
                            _ => {
                                let p = [
                                    al.saturating_mul(bl),
                                    al.saturating_mul(bh),
                                    ah.saturating_mul(bl),
                                    ah.saturating_mul(bh),
                                ];
                                (*p.iter().min()?, *p.iter().max()?)
                            }
                        };
                        Some((lo, hi))
                    }
                    "OpLandInt" => [operand(0), operand(1)]
                        .into_iter()
                        .flatten()
                        .filter(|(lo, _)| *lo >= 0)
                        .map(|(_, hi)| hi)
                        .min()
                        .map(|hi| (0, hi)),
                    "OpRemInt" => {
                        let c = args.get(1).and_then(|a| self.const_int(a))?;
                        if c == 0 {
                            return None;
                        }
                        let m = c.unsigned_abs().saturating_sub(1);
                        let m = i64::try_from(m).ok()?;
                        let lo = if operand(0).is_some_and(|(lo, _)| lo >= 0) {
                            0
                        } else {
                            -m
                        };
                        Some((lo, m))
                    }
                    _ => declared(self.data.def(*d).returned()),
                }
            }
            _ => None,
        }
    }

    /// `@FR-N-Reserve` — a NULLABLE narrow slot (`u8?`, `i16?`, a nullable `limit(lo, hi)`) holds
    /// its USABLE range: where the range fills a fixed width, one code at its edge is its null.
    /// A CONSTANT outside that range can only be stored as null, which is never what the author
    /// wrote, so it is refused the way its non-null twin refuses a constant that does not fit
    /// (@C80's compile-time valve) rather than becoming null in silence (loft#1796: `x: u8? =
    /// 255`, `R { b: 255 }`, `[255]` all read null).
    pub(super) fn nullable_narrow_constant_refusal(
        &self,
        code: &Value,
        dst: &Type,
    ) -> Option<String> {
        let Type::Optional(inner) = dst else {
            return None;
        };
        let Type::Integer(spec) = inner.base() else {
            return None;
        };
        // The full integer is no narrow slot; a width alias or a user range is.
        if spec.is_full_integer() {
            return None;
        }
        let (lo, hi) = (i64::from(spec.usable_min(true)), spec.usable_max(true));
        let n = self.unfitting_stored_constant(code, lo, hi)?;
        // A range that fills a fixed width gives up one code to its null; a `limit` range
        // widens instead and keeps every value, so its non-null twin holds no more than it.
        if lo > i64::from(spec.min) || hi < spec.max {
            let name = self.int_type_name(inner);
            return Some(format!(
                "{n} does not fit `{name}?` — a nullable {name} holds {lo}..={hi} (the remaining \
                 code is its null), so this constant would be stored as null.  Use a wider \
                 nullable type, or the non-null `{name}`, which holds its whole range"
            ));
        }
        // A reserving alias names a FIXED width (`u8?` holds 0..=254), so a `limit` range that
        // shares its bounds is spelled as written rather than by that alias's name.
        let mut name = self.int_type_name(inner);
        if matches!(name.as_str(), "u8" | "i8" | "u16" | "i16") {
            name = format!("integer limit({lo}, {hi})");
        }
        Some(format!(
            "{n} does not fit `{name}?` — it holds {lo}..={hi}, so this constant would be stored \
             as null.  Use a wider type"
        ))
    }

    /// The first constant a value can reach its slot as that lies outside `lo..=hi`: the value
    /// itself when it is one, and otherwise every leaf a JOIN can deliver — each arm of a value
    /// `if` or `match`, the default of a `??` (whose lowering is an `if` or a block ending in
    /// one), and a value block's answer.  Asked by the nullable narrow refusal, so a constant
    /// that cannot fit is refused however it reaches the slot (`if c { 255 } else { 3 }`,
    /// `g() ?? 300`); a leaf that is not a constant — the `??` subject, a call, a variable — is
    /// no constant to judge.  Every store into such a slot asks, so it allocates nothing.
    pub(super) fn unfitting_stored_constant(&self, code: &Value, lo: i64, hi: i64) -> Option<i64> {
        let n = match code.unspan() {
            Value::Int(n) => i64::from(*n),
            Value::Long(n) => *n,
            Value::Null => return None,
            Value::If(_, then, other) => {
                return self
                    .unfitting_stored_constant(then, lo, hi)
                    .or_else(|| self.unfitting_stored_constant(other, lo, hi));
            }
            Value::Block(bl) => {
                return bl
                    .operators
                    .last()
                    .and_then(|last| self.unfitting_stored_constant(last, lo, hi));
            }
            // Only an operator folds (`-1`, `200 + 100`, `255 as u8?`); a call to a function
            // is no constant, and asking `const_eval` about one allocates for its arguments.
            Value::Call(op, _) if self.data.def(*op).name.starts_with("Op") => {
                match crate::const_eval::const_eval(code, &self.data) {
                    Some(Value::Int(n)) => i64::from(n),
                    Some(Value::Long(n)) => n,
                    _ => return None,
                }
            }
            _ => return None,
        };
        (n < lo || n > hi).then_some(n)
    }

    /// Report [`Self::nullable_narrow_constant_refusal`]'s message once per source position.
    /// A nullable narrow DEFAULT (a field's or a parameter's) is converted where it is written
    /// and again in the function it is hoisted into, both at the default's own position, so
    /// the second report of the same message there is dropped.
    pub(super) fn refuse_nullable_narrow_constant(&mut self, msg: &str) {
        let mark = self.lexer.diagnostics().mark();
        diagnostic!(self.lexer, Level::Error, "{msg}");
        let repeated = self
            .lexer
            .diagnostics()
            .entries()
            .split_last()
            .is_some_and(|(new, old)| {
                old.iter().any(|e| {
                    e.message == new.message
                        && e.file == new.file
                        && e.line == new.line
                        && e.col == new.col
                })
            });
        if repeated {
            self.lexer.rewind_diagnostics(mark);
        }
    }

    /// @PLN25 slice (c) — the `(N-Store)` teeth at a STORE site (typed assignment, field
    /// construction, an index, a return). An un-discharged nullable `τ?` cannot be committed
    /// to a non-nullable target — discharge it first with `?? <default>` or `match` (both
    /// yield the non-null base). UNLIKE `convert`, this runs ONLY at store sites, so null-CHECK
    /// comparisons (`x == null`) stay legal. Gated on `LOFT_PLN25_DN3`; returns `true` (and
    /// emits the diagnostic) on a violation. A no-op (returns `false`) off / first pass.
    /// @PLN102 (N-Store) — emit an N-Store diagnostic at the STORED VALUE's own span
    /// (`at`) when the caller supplies one, else at the lexer cursor (the historical
    /// position).  `None` is byte-identical to the old `diagnostic!(self.lexer, …)` path
    /// (`lexer.diagnostic` == `pos_diagnostic` at the current position); a `Some` anchor
    /// is used by the block-finalization callers whose cursor has advanced to the block's
    /// `}`.  See doc/claude/plans/102-stability-contract/nstore-position-fix.md.
    pub(super) fn nstore_diag(&mut self, at: Option<&Position>, level: Level, message: &str) {
        match at {
            Some(p) => self.lexer.pos_diagnostic(level, p, message),
            None => self.lexer.diagnostic(level, message),
        }
    }

    /// Does storing `value_tp` into `target_tp` violate `(N-Store)`?  Reports it if so, and
    /// answers whether the store was REFUSED (an error) rather than merely reported.
    ///
    /// `never_error` holds the report to `Level::Warning` whatever the width.  Exactly one
    /// seam asks for it — a collection LITERAL's elements (loft#1232) — and the reason is
    /// compatibility, not doctrine: the narrow-width escalation below is right about the
    /// SLOT (a `u8` spends all 256 values on real data, so a null there has no room), but
    /// this seam was silent until now, so escalating it retro-breaks published code that
    /// compiles today.  Measured: `assets 0.2.0` writes `bp += [0 as u8?]`, whose value is
    /// never actually null, and the whole registry gate went from 42 pass to a COMPILE-BREAK.
    /// Reporting where there was silence is a strict gain; refusing what a shipped package
    /// already relies on is a break the freeze forbids, and raising the tier later is
    /// COMPATIBILITY.md's process rather than this function's call.
    /// The spelling a `τ?` cure has to be written in for this target.
    ///
    /// [`Type::name`] renders `Type::Reference(d, …)` as the bare struct name, which is what an
    /// EMBEDDED field is called (`item: Row` really is a `Row`) and not what a POINTER field is
    /// (`link: reference<Row>`).  One IR spelling carries both notions and the `u16::MAX` share
    /// marker (#328) is what tells them apart — the same bit `Data::has_value_cycle` reads to
    /// skip pointer edges.
    ///
    /// Naming the wrong one hands the reader a cure that does not do what the message says.
    /// `@FR-L-Null` gives `reference<Row>?` the pointer's own bytes with `nullref` for absence,
    /// while `@FR-L-Null-Tag` makes `Row?` an INLINE tagged record: applying `Row?` to a pointer
    /// field compiles and silently replaces sharing with a copy, and on a struct whose reference
    /// graph returns to itself it does not compile at all.
    pub(super) fn cure_spelling(&self, target_tp: &Type) -> String {
        match target_tp {
            Type::Reference(d, deps) if deps.is_pointer_marker() => {
                format!("reference<{}>", self.data.def(*d).name())
            }
            other => other.source_name(&self.data),
        }
    }

    pub(super) fn n_store_violation(
        &mut self,
        value_tp: &Type,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
    ) -> bool {
        self.n_store_violation_inner(value_tp, target_tp, what, at, false)
    }

    pub(super) fn n_store_violation_inner(
        &mut self,
        value_tp: &Type,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
        never_error: bool,
    ) -> bool {
        if self.first_pass {
            return false;
        }
        // Tuples store ELEMENT-WISE, so `(N-Store)` applies per position: an Optional (DN3)
        // or bare-null (DN1) element cannot slip into a non-null element slot just because
        // the enclosing tuple type isn't itself `Optional`. A tuple type appears either as
        // a literal `Type::Tuple` (a field / typed assign) or as the synthetic-struct
        // rewrite `Reference(__tuple<…>)` (a tuple RETURN type, rewritten by
        // `parse_function`; elements = the def's attribute typedefs, the same normalization
        // as the destructure reader in expressions.rs). Normalize both sides and recurse
        // (nested tuples included); each element rides the same DN3/DN1 checks below.
        // Arity mismatches are left to the regular type checker.
        fn tuple_elems(data: &crate::data::Data, tp: &Type) -> Option<Vec<Type>> {
            // `@FR-T-Absent` — a tuple that ARRIVES absent is read as the member-nullable one,
            // `optional((τ₁, …, τₙ)) ≡ (τ₁?, …, τₙ?)`, so the element-wise comparison below sees
            // `τᵢ?` on the value side and answers per member.  Read BARE, the whole `(τ…)?` was
            // compared against the declared record instead and reported a store into "the
            // non-null type `__tuple<integer?,text?>`" — a type whose members are exactly what
            // the value carries.  The honest half stays: against a non-null `__tuple<τ…>` each
            // `τᵢ?` still earns its own report (`D-tup-10`).
            let (base, absent) = tp.peel_optional();
            let elems: Vec<Type> = match base {
                Type::Tuple(elems) => elems.clone(),
                Type::Reference(d, _) if data.def(*d).name().starts_with("__tuple<") => data
                    .def(*d)
                    .attributes
                    .iter()
                    .map(|a| a.typedef.clone())
                    .collect(),
                _ => return None,
            };
            Some(if absent {
                elems.into_iter().map(Type::optional).collect()
            } else {
                elems
            })
        }
        if let (Some(v_elems), Some(t_elems)) = (
            tuple_elems(&self.data, value_tp),
            tuple_elems(&self.data, target_tp),
        ) && v_elems.len() == t_elems.len()
        {
            let mut hit = false;
            for (i, (ve, te)) in v_elems.iter().zip(t_elems.iter()).enumerate() {
                hit |= self.n_store_violation_inner(
                    ve,
                    te,
                    &format!("element {i} of {what}"),
                    at,
                    never_error,
                );
            }
            return hit;
        }
        // The τ? half — one body, [`Parser::nstore_unwrap_report`], shared with `convert`'s
        // `τ? ⤳ τ` arm.  This entry is for the lowerings that never reach `convert` (the
        // if-join accumulator, the append routes, a struct literal's vector-field deep copy).
        if let Type::Optional(inner) = value_tp {
            return self.nstore_unwrap_report(inner, target_tp, what, at, never_error);
        }
        if matches!(value_tp, Type::Null) {
            return self.nstore_null_report(target_tp, what, at, never_error);
        }
        false
    }

    /// @FR-N-Store's bare-`null` half (@PLN25 DN1 + loft#1313's heap gate), the ONE body:
    /// `null` written into a slot whose type does not hold it.  A full-width scalar and every
    /// heap target WARN (the slot holds null; a heap target never escalates, loft#1232), a
    /// narrow width ERRORS.  Answers whether the store was REFUSED.  Asked from
    /// [`Parser::convert`]'s entry for every store it lowers, and from
    /// [`Parser::n_store_violation`] for the lowerings that bypass it.
    pub(super) fn nstore_null_report(
        &mut self,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
        never_error: bool,
    ) -> bool {
        self.nstore_null_report_as(target_tp, what, at, never_error, None)
    }

    /// [`Parser::nstore_null_report`] with the CONSEQUENCE clause supplied by the caller.
    ///
    /// The default clause — *"the slot holds null"* — is what a scalar slot does, and what a
    /// record travelling as a HANDLE does: measured, a `null` argument arrives null and a
    /// `return null` reads back null.  A dense INLINE slot does not, because it has no
    /// discriminant to spend on absence (`synth_nullable_struct_fields`: a field with no `?`
    /// "cannot be absent and stays dense"), and an assignment into one does not happen at all.
    /// So the clause belongs to the POSITION, which only the caller knows, while the tier
    /// split and the cure stay here.  Passing `None` keeps the wording those callers already
    /// pin, to the byte.
    pub(super) fn nstore_null_report_as(
        &mut self,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
        never_error: bool,
        consequence: Option<&str>,
    ) -> bool {
        if self.first_pass {
            return false;
        }
        // DN1 (the default flip): under DN1 a plain scalar is NON-null, so a bare `null` cannot be
        // stored into a non-Optional scalar target — declare the target `τ?` to allow null.
        // (Heap types — reference/vector/enum — stay nullable; only the SCALAR default flips.)
        // The stdlib is held to the SAME rule (no STD_SOURCE exemption): F1b(b)'s `min`/`max`/`clamp`
        // non-null bodies are now clean (nullable args — including DN3-typed division results —
        // route to the `τ?` overload), so no trusted-source `return null` remains to exempt.
        //
        // The HEAP half of the same rule (loft#1313).  `(N-Opt)` is written for every `τ`, and
        // DN1 landed it for the scalars only — so a bare `null` into a non-null reference,
        // collection or struct-enum passed in silence at a field, a return, a vector element and
        // a call argument, the four positions where the scalar twin warns.  A heap LOCAL was
        // never in the gap: `change_var` refuses `x: Item = null` with its own message.
        //
        // `is_dbref` rather than a spelled list, because its own doc records how that list
        // drifts — the three obvious kinds get written and the five keyed collections are
        // forgotten, and a short list here would route a handle down the scalar path.  The
        // synthetic `__nullable<S>` is excluded for the reason the DN3 branch excludes it: it is
        // the INLINE spelling of `S?` and is exactly as nullable as the `?` it stands for.
        //
        // Gated on `nullflow_enabled` as well, so `LOFT_NO_NULLFLOW` restores the pre-@PLN102
        // model EXACTLY: the scalars back to the uniform hard error below, and the heap half
        // back to silence.  Letting it fall through to that error instead would hand the opt-out
        // a refusal this branch never had — the one outcome the freeze forbids here.
        // A PLAIN enum is the same question one kind over: a one-byte value whose `null` is a
        // reserved discriminant, declarable `E?`, and neither a scalar nor a handle — so it
        // was asked by neither half and a bare `null` into an `E` field, argument or return
        // passed in silence while the slot held null.  It takes the heap half's wording.
        let heap_target = crate::keys::heap_nstore_enabled()
            && crate::keys::nstore_softens(false)
            && (crate::data::is_dbref(target_tp) || matches!(target_tp, Type::Enum(_, false, _)))
            // Both spellings of `τ?`, asked HERE.  `is_nullable_wrapper` covers the synthetic
            // `__nullable<S>`; the `Type::Optional` marker was covered only by `is_dbref`
            // answering `false` for a wrapper — a nullability question answered by a shape
            // predicate's blindness (`@FR-N-Shape`).  Spelling it makes the outcome identical
            // and the reason local, so peeling `is_dbref` later cannot silently open this gate.
            && !matches!(target_tp, Type::Optional(_))
            && !self.data.is_nullable_wrapper(target_tp);
        if crate::keys::pln25_dn1_enabled() && (Self::is_non_null_scalar(target_tp) || heap_target)
        {
            let nm = self.cure_spelling(target_tp);
            // @PLN102 (N-Store) Phase 1 — same warn/error split as the DN3 branch: a bare `null`
            // into a NON-narrow scalar target warns (the slot reserves its null distinctly, so it
            // holds null and reads back null); a NARROW width keeps the hard error (no room).
            let narrow = Self::nstore_narrow(target_tp, never_error);
            // A heap target never escalates.  There is no narrow heap width to run out of room
            // the way a `u8` does, and loft#1232 settled the rest: reporting where there was
            // silence is a strict gain, while refusing what a shipped package already compiles is
            // the break the freeze forbids.  Raising the tier later is COMPATIBILITY.md's process.
            if heap_target || crate::keys::nstore_softens(narrow) {
                // The scalar wording is unchanged to the byte — `scalar ` is what the heap half
                // drops, not a second message.  Two spellings of one diagnostic is how the
                // fixtures that pin this text would start disagreeing with the rule behind it.
                let kind = if heap_target { "" } else { "scalar " };
                let effect = consequence.unwrap_or("the slot holds null");
                let msg = diagnostic_format(
                    Level::Warning,
                    format_args!(
                        "`null` is stored into {what} of the non-null {kind}type `{nm}` — {effect}; declare it `{nm}?` to make that explicit"
                    ),
                );
                self.nstore_diag(at, Level::Warning, &msg);
                return false;
            }
            let msg = diagnostic_format(
                Level::Error,
                format_args!(
                    "`null` cannot be stored into {what} of the non-null scalar type `{nm}` — declare it `{nm}?` to allow null"
                ),
            );
            self.nstore_diag(at, Level::Error, &msg);
            return true;
        }
        false
    }

    /// @FR-N-Store's τ? half, the ONE body: an un-discharged `τ?` reaching a non-null `τ`
    /// slot is reported with the rule's split — a WARNING where τ's non-null form reserves
    /// its null distinctly (the store proceeds and the slot holds the sentinel), an ERROR for a
    /// narrow width (`nstore_narrow`).  Answers whether the store was REFUSED.
    ///
    /// Two askers and no third: [`Parser::convert`]'s `τ? ⤳ τ` arm, which every peel passes
    /// (measured over the whole corpus, @PLN153 phase 3), and [`Parser::n_store_violation`]
    /// for the few lowerings that store without converting.
    pub(super) fn nstore_unwrap_report(
        &mut self,
        inner: &Type,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
        never_error: bool,
    ) -> bool {
        self.nstore_unwrap_report_as(inner, target_tp, what, at, never_error, None)
    }

    /// [`Parser::nstore_unwrap_report`] with the CONSEQUENCE clause supplied by the caller,
    /// for the reason [`Parser::nstore_null_report_as`] gives: *"it becomes null there"* is
    /// what a handle or a scalar slot does, and a dense record slot has no null to hold — the
    /// store does not happen.  The position knows which; `None` keeps the pinned wording.
    pub(super) fn nstore_unwrap_report_as(
        &mut self,
        inner: &Type,
        target_tp: &Type,
        what: &str,
        at: Option<&Position>,
        never_error: bool,
        consequence: Option<&str>,
    ) -> bool {
        // `τ?` has a THIRD spelling, and this gate has to ask all three of one face.
        // A `&` parameter carries its nullability INSIDE the reference: `&integer?` is
        // `RefVar(Optional(Integer))`, so asking `Type::Optional` of the OUTER type answers
        // about the reference rather than about the slot the store lands in.  Read bare, every
        // correct call handing a `τ?` to a `&τ?` parameter warned that it "becomes null there"
        // in "the non-null type `&integer?`" — naming a nullable type non-null — and `warning`
        // gates library CI, so a library taking a `&τ?` parameter failed on correct code
        // (loft#1413).  The pointee is the slot, so the pointee is what this asks.
        let null_face = match target_tp {
            Type::RefVar(pointee) => pointee.as_ref(),
            other => other,
        };
        if self.first_pass
            || !crate::keys::pln25_dn3_enabled()
            || matches!(
                null_face,
                Type::Optional(_) | Type::Void | Type::Never | Type::Null
            )
            // The second spelling: an INLINE slot holds an absent `S` as the synthetic
            // `__nullable<S>` enum, which is not a `Type::Optional` and is exactly as nullable.
            // A tuple ELEMENT is such a slot, so recursing into a promoted tuple return reaches
            // one — and reading it as non-null made the check warn that a `W2?` becomes null in
            // `__nullable<W2>` (loft#1123).
            || self.data.is_nullable_wrapper(null_face)
        {
            return false;
        }
        // The SOURCE spelling: this message names the type back to the author twice and
        // `name` is the schema key, which re-spells a keyed payload (`hash<It,["k"]>` for
        // what they wrote as `hash<It[k]>`).
        let nm = inner.source_name(&self.data);
        // @FR-N-Decl — a DECLARED `x: τ` is a commitment, so a later nullable write is
        // @FR-N-Store's refusal; this split is where a declared slot's promise is kept.
        // @PLN102 (N-Store) Phase 1 — the warn/error split (types.md § Null-flow, (N-Store)).
        // WARN (a nudge; the store PROCEEDS — `convert` peels the Optional and the slot holds
        // the null sentinel) where τ reserves its null DISTINCTLY even in the non-null form
        // (full `integer`, `float`, `single`, `boolean`, `character`, `text`, refs, aggregates).
        // Keep the hard ERROR only for a NARROW width (`byte_width < 8`), whose non-null form
        // spends the whole width on real values, so a null there would silently corrupt.
        // Gate OFF → the current uniform hard error (this branch stays byte-identical).
        let narrow = Self::nstore_narrow(target_tp, never_error);
        if crate::keys::nstore_softens(narrow) {
            let msg = diagnostic_format(
                Level::Warning,
                format_args!(
                    "a nullable `{nm}?` is stored into {what} of the non-null type `{}` — {}; discharge with `?` (the type's default), `?? <default>`, or `match` if that is not intended",
                    target_tp.source_name(&self.data),
                    consequence.unwrap_or("it becomes null there")
                ),
            );
            self.nstore_diag(at, Level::Warning, &msg);
            return false; // store proceeds — `convert` peels the Optional and stores the sentinel
        }
        let msg = diagnostic_format(
            Level::Error,
            format_args!(
                "a nullable `{nm}?` cannot be stored into {what} of the non-null type `{}` — discharge it first with `?` (the type's default), `?? <default>`, or `match`",
                target_tp.source_name(&self.data)
            ),
        );
        self.nstore_diag(at, Level::Error, &msg);
        true
    }

    /// The WIDTH half of @FR-N-Store's warn/error split: a slot whose non-null form spends
    /// the whole width on real values (`u8`…`u32`, `byte_width < 8`) has no bit pattern left
    /// for null, so a nullable stored into it is an ERROR; a full-width slot reserves its null
    /// distinctly and WARNS instead (`keys::nstore_softens` is the flag half).  `never_error`
    /// is the caller's "this site never escalates" — a null literal into a heap target.  Two
    /// branches spelled this test by hand and could only agree by accident; this is the one.
    pub(crate) fn nstore_narrow(target_tp: &Type, never_error: bool) -> bool {
        !never_error && matches!(target_tp, Type::Integer(s) if s.byte_width(false) < 8)
    }

    /// @PLN25 DN1 — the scalar types whose default flips to NON-null (a bare `null` needs `τ?`).
    /// Heap-nullable types (reference / vector / enum / keyed) are NOT here — they stay nullable.
    pub(crate) fn is_non_null_scalar(tp: &Type) -> bool {
        matches!(
            tp,
            Type::Integer(_)
                | Type::Text(_)
                | Type::Boolean
                | Type::Float
                | Type::Single
                | Type::Character
        )
    }

    /// loft#822 — is `from` the STORED spelling of the STACK tuple type `to`?
    ///
    /// A tuple is one loft type written two ways: `Type::Tuple(…)` holds its elements on
    /// contiguous stack slots, `Reference(__tuple<…>)` is a DbRef at the same elements'
    /// stored bytes.  [`Parser::convert`] answers such a pair by UNBOXING the value, so a
    /// caller that keeps classifying on `from` afterwards is reading a type the value no
    /// longer has — ask this instead of comparing the two spellings by hand.
    pub(crate) fn unboxes_stored_tuple(&self, from: &Type, to: &Type) -> bool {
        let (Type::Reference(d_nr, _), Type::Tuple(dst_elems)) = (from, to) else {
            return false;
        };
        if !self.data.def(*d_nr).name().starts_with("__tuple<") {
            return false;
        }
        let attrs = self.data.def(*d_nr).attributes();
        // A member declared `S?` has TWO spellings — `Optional(Reference(S))` as the author
        // writes it and the tagged `Enum(__nullable<S>)` the layout stores it as
        // (`@FR-L-Null-Tag`) — and `is_equal` alone read that pair as a precision loss between
        // a type and itself, so `v += [f()]` was refused for a tuple with a nullable member
        // while its dense twin was accepted (loft#1139).  `same_nullable_struct` is the home
        // for the pair; the unboxer below then reads the member through its tag.
        attrs.len() == dst_elems.len()
            && attrs.iter().zip(dst_elems).all(|(a, d)| {
                a.typedef.is_equal(d) || self.data.same_nullable_struct(&a.typedef, d).is_some()
            })
    }

    /// Element types of a stored tuple (`Reference(__tuple<…>)`), in order — the argument
    /// [`Parser::unbox_tuple_from_dbref`] needs.  Empty for any other type.
    pub(crate) fn stored_tuple_elements(&self, stored: &Type) -> Vec<Type> {
        let Type::Reference(d_nr, _) = stored else {
            return Vec::new();
        };
        self.data
            .def(*d_nr)
            .attributes()
            .iter()
            .map(|a| a.typedef.clone())
            .collect()
    }

    /// Is a value of this type carried as a heap `DbRef` — a record, a collection, or a
    /// struct-enum?  These are the types with no registered `OpConv*FromX → Boolean`, so
    /// every position that reads one as present-or-absent has to say so itself.
    ///
    /// One home, because the answer is shared by the `??` null check, the `if` / `while`
    /// / `assert` condition, and the `!x` null test — three places that must agree about
    /// which values `rec` speaks for.
    ///
    /// The list itself now lives in [`crate::data::is_dbref`], which asks the layout rather
    /// than restating the variants; this keeps the name and the `.base()` peel the null-check
    /// callers rely on.  Two homes for one predicate is what this collapses: `is_dbref` was
    /// added on 2026-08-24 without checking whether the FULL list already had a home — the
    /// search was made for the SHORT one and stopped there.
    pub(crate) fn is_heap_handle(tp: &Type) -> bool {
        crate::data::is_dbref(tp.base())
    }
}
