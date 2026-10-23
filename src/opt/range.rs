// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-Range` — the RANGE an integer expression provably lies in, and the plain
//! operator it therefore may emit.
//!
//! @C120 declined the closure of the non-sentinel proof over `+`, `-` and `*`: an overflow
//! mints the null sentinel on both backends, and a rewrite must leave every value as the
//! interpreter answers it, faults included.  It named the admissible successor — *"a
//! PROOF, not a policy: arithmetic whose operands carry ranges such that the result CANNOT
//! overflow may emit the plain operator, because no fault can occur and no value can
//! differ"* — and `(R-BoundedNest)` built it for one loop shape with a guard evaluated at
//! run time.  This module is the same proof for straight-line arithmetic, decided at
//! GENERATION time: every leaf's range is a fact of the language (a literal, a mask, a
//! text or vector length, a byte), the operators' ranges follow by interval arithmetic in
//! `i128`, and an operator whose RESULT fits the type cannot fault, so the processor's
//! operator answers exactly what the checked template would.
//!
//! ## What is ranged — and what deliberately is not
//!
//! Every fact is structural and conservative; a miss costs the checked template, never
//! correctness.  A range implies non-sentinel: `i64::MIN` is never inside one.
//!
//! - a literal;
//! - `a & lit` with a non-negative literal, when `a` is NON-SENTINEL (the mask of a null
//!   is null — `op_logical_and_int` propagates it — so the operand must be proven first;
//!   the corpus cell that pins this holds a null read from past a vector's end);
//! - `a & b`, `a | b`, `a ^ b` over two non-negative ranges; `a >> k` over a ranged `a`;
//! - `+`, `-`, `*`, negation over ranged operands whose result fits `(i64::MIN, i64::MAX]`;
//! - `/` and `%` by a ranged divisor that excludes zero (the sentinel is minted by a zero
//!   divisor and by `MIN / -1`, and a ranged dividend is never `MIN`);
//! - a text's size and a vector's length (`u32` words in the store layout), a text byte
//!   (`0..=255` by `text_byte_at_native`'s contract);
//! - `if` merges as the union, the parser's null-discharge shape as the union of its arms;
//! - a counted range's counters, when both ends are ranged;
//! - a LOCAL whose every assignment is ranged, never handed out by reference, and never a
//!   self-step (`x = x + 1` has no least fixpoint; the counter case above is the one shape
//!   read off the loop instead);
//! - a call of a ONE-EXPRESSION user function (`fn color_a(c) -> ((c & m) >> 24) & 255`),
//!   evaluated over the arguments' facts at the call site, three calls deep at most.
//!
//! NOT trusted: parameters and record/element reads (the @C80 escape means a non-null
//! type can hold the sentinel), anything a callee with more than one statement answers,
//! and `<<`, a shift by a variable, a remainder by a signed range.  (`^` over two
//! non-negative ranges is bounded like `|` — `@FR-R-MaskRange`.)
//!
//! `LOFT_HOIST_VERIFY=1` emits the checking form of every plain operator — the plain
//! answer beside the checked template's, compared at the operator (`ops::range_verify`).
//! `LOFT_NO_RANGE_ARITH=1` keeps every template.

use crate::data::{Data, Value};
use std::collections::HashMap;

/// An inclusive range of `i64` values, `lo > i64::MIN` (the sentinel is never inside).
pub type Range = (i64, i64);

/// The facts one function's ranges are read under: each ranged VARIABLE, and each ranged
/// record FIELD read `p.f` (by variable and byte offset) — the second only where a run-time
/// guard proved it (`@FR-R-RangedCall`): a field is never ranged by shape (@C80).
#[derive(Default, Debug, Clone)]
pub struct Ranges {
    pub vars: HashMap<u16, Range>,
    pub fields: HashMap<(u16, i64), Range>,
    /// Each ranged ELEMENT of a tuple local, by variable and element index: every
    /// assignment of the local answers that element in range ([`elem_range`]).
    pub elems: HashMap<(u16, u16), Range>,
}

/// What [`range`] reads its leaves from: a variable's range, and a guarded field's.  A plain
/// variable map answers no field (the shape every caller had before `@FR-R-RangedCall`).
pub trait RangeFacts {
    fn var(&self, v: u16) -> Option<Range>;
    fn field(&self, v: u16, off: i64) -> Option<Range>;
    /// Element `i` of the tuple local `v`; a map that holds no tuple facts answers none.
    fn elem(&self, _v: u16, _i: u16) -> Option<Range> {
        None
    }
}

impl RangeFacts for HashMap<u16, Range> {
    fn var(&self, v: u16) -> Option<Range> {
        self.get(&v).copied()
    }
    fn field(&self, _v: u16, _off: i64) -> Option<Range> {
        None
    }
}

impl RangeFacts for Ranges {
    fn var(&self, v: u16) -> Option<Range> {
        self.vars.get(&v).copied()
    }
    fn field(&self, v: u16, off: i64) -> Option<Range> {
        self.fields.get(&(v, off)).copied()
    }
    fn elem(&self, v: u16, i: u16) -> Option<Range> {
        self.elems.get(&(v, i)).copied()
    }
}

/// A callee is evaluated over its arguments' facts at most this many calls deep.
const CALL_DEPTH: u8 = 3;

/// `u32::MAX` as `i64`: the widest a text size or a vector length can be.
const U32_MAX: i64 = u32::MAX as i64;

/// `@FR-R-Range`'s type clause — the range a value carries BY ITS STATIC TYPE, or `None`
/// where the type is not a fact about the value.  A non-nullable `Integer[lo, hi]` whose
/// range FILLS its width — `u8`, `i8`, `u16`, `i16`, every user-written `limit(lo, hi)` — is
/// one: since loft#1593 every store, call and literal into such a slot is refused unless
/// provably in range (`(I-Narrow)`, `(I-Lit)`), a `?? d` fallback lands in range, and the one
/// run-time arrival — the slot's own arithmetic stepping past the range (@C85's overflow
/// edge) — answers the type's DEFAULT, which is in range too (`(E-Uncomp-NN)`).  So no value
/// outside `[lo, hi]` can ever be held, whoever writes.
///
/// Two specs are NOT facts and answer `None`: the two full-integer templates (the range
/// the plain `integer` REPORTS is not the range it holds), and `i32`/`u32`, which keep a
/// code back for null that an overflow does write into a non-null slot — a `u32` local can
/// hold the sentinel after `+=`, and a range that trusted its type would run plain arithmetic
/// on it.  A nullable `τ?` and a `&` link answer `None` for the same reason: the slot can hold
/// what the range does not describe.
///
/// Every other non-nullable declared range IS a fact, including one that leaves codes spare
/// inside its storage width: by C127 a value that does not fit such a slot takes the type's
/// DEFAULT, which is in range, so nothing outside `[lo, hi]` can be held.  Before C127 those
/// specs answered `None` because an overflow wrote a sentinel into them.
#[must_use]
pub fn type_range(tp: &crate::data::Type) -> Option<Range> {
    match tp {
        // The nullability question, spelled (`@FR-N-Shape`): a `τ?` slot can hold what the
        // range does not describe, so it is not a fact — deliberately NOT peeled through.
        crate::data::Type::Optional(_) => None,
        crate::data::Type::Integer(spec) => {
            if spec.is_signed32_template() || spec.is_wide_template() {
                return None;
            }
            fits(i128::from(spec.min), i128::from(spec.max))
        }
        _ => None,
    }
}

/// Make a range from `i128` ends, or `None` when it does not fit the type (the sentinel
/// excluded).
fn fits(lo: i128, hi: i128) -> Option<Range> {
    if lo > i128::from(i64::MIN) && hi <= i128::from(i64::MAX) && lo <= hi {
        Some((lo as i64, hi as i64))
    } else {
        None
    }
}

fn union(a: Range, b: Range) -> Range {
    (a.0.min(b.0), a.1.max(b.1))
}

/// The range of `v` under the per-function facts (`nn` from
/// [`super::non_sentinel::non_sentinel_vars`], `rv` from [`range_vars`]), or `None`.
#[must_use]
pub fn range<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    v: &Value,
    depth: u8,
) -> Option<Range> {
    match v.unspan() {
        Value::Int(k) => Some((i64::from(*k), i64::from(*k))),
        Value::Long(l) if *l != i64::MIN => Some((*l, *l)),
        Value::Var(nr) => rv.var(*nr),
        Value::TupleGet(t, i) => rv.elem(*t, *i),
        // An `if` merge is the union of its arms — the discharge `if bool(x) x else d`
        // included, whose then-arm is x itself, so an unranged x leaves the merge unranged.
        Value::If(_, then_arm, else_arm) => {
            let then_range = range(data, nn, rv, then_arm, depth)?;
            let else_range = range(data, nn, rv, else_arm, depth)?;
            Some(union(then_range, else_range))
        }
        // A block's RESULT TYPE is a fact about its value before its tail is read: the join
        // `v[i] ?? 0` over a `u8` vector is typed `integer(0, 255)` by the compiler, while its
        // tail — an `if` whose then arm is the element temp — ranges nothing by shape.
        Value::Block(b) => type_range(&b.result).or_else(|| {
            b.operators
                .iter()
                .rev()
                .find(|op| !matches!(op, Value::Line(_)))
                .and_then(|tail| range(data, nn, rv, tail, depth))
        }),
        Value::Return(inner) => range(data, nn, rv, inner, depth),
        Value::Call(d_nr, args) => {
            if (*d_nr as usize) >= data.definitions.len() {
                return None;
            }
            let def = data.def(*d_nr);
            // A guarded field read (`@FR-R-RangedCall`): the run-time test in front of the
            // code proved the field in range, and nothing in it writes the field.
            if def.name() == "OpGetInt"
                && let [Value::Var(p), Value::Int(f)] = [args[0].unspan(), args[1].unspan()]
            {
                return rv.field(*p, i64::from(*f));
            }
            op_range(data, nn, rv, def.name(), args, depth)
                .or_else(|| call_range(data, nn, rv, def, args, None, depth))
        }
        _ => None,
    }
}

/// `x % y`: a CONSTANT divisor bounds the remainder whatever the dividend, once the dividend
/// is no sentinel (its null answers null) — `-(|c|-1) ..= |c|-1`, from 0 when the dividend
/// cannot be negative (`@FR-N-Arith`; the parser proves the same at a narrow store,
/// `Parser::int_range`).  Otherwise a non-negative dividend over a positive divisor is
/// `0 ..= min(x.hi, y.hi - 1)`.
fn rem_range<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    args: &[Value],
    depth: u8,
) -> Option<Range> {
    let x = range(data, nn, rv, &args[0], depth);
    let divisor = match args[1].unspan() {
        Value::Int(k) => Some(i64::from(*k)),
        Value::Long(k) if *k != i64::MIN => Some(*k),
        _ => None,
    };
    if let Some(c) = divisor.filter(|c| *c != 0)
        && (x.is_some() || super::non_sentinel::non_sentinel(data, nn, &args[0]))
    {
        let m = i64::try_from(c.unsigned_abs() - 1).ok()?;
        let lo = if x.is_some_and(|x| x.0 >= 0) { 0 } else { -m };
        let hi = x.map_or(m, |x| if x.0 >= 0 { x.1.min(m) } else { m });
        return Some((lo, hi));
    }
    let (x, y) = (x?, range(data, nn, rv, &args[1], depth)?);
    if x.0 < 0 || y.0 <= 0 {
        return None;
    }
    Some((0, x.1.min(y.1 - 1)))
}

/// The range of the native op `name` over `args` — the operator rules alone, which is
/// what the emitter asks at the op it is about to write.  `None` for a user call: that is
/// [`range`]'s business through the callee's one expression.
#[must_use]
pub fn op_range<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    name: &str,
    args: &[Value],
    depth: u8,
) -> Option<Range> {
    let a = |i: usize| range(data, nn, rv, &args[i], depth);
    {
        {
            match (name, args.len()) {
                // The `*Nullable` twins are the same arithmetic with a silent null on a fault
                // (`(E-Report)`): a result that provably fits has no fault to be silent about.
                ("OpAddInt" | "OpAddIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    fits(
                        i128::from(x.0) + i128::from(y.0),
                        i128::from(x.1) + i128::from(y.1),
                    )
                }
                ("OpMinInt" | "OpMinIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    fits(
                        i128::from(x.0) - i128::from(y.1),
                        i128::from(x.1) - i128::from(y.0),
                    )
                }
                ("OpMulInt" | "OpMulIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    let c = [
                        i128::from(x.0) * i128::from(y.0),
                        i128::from(x.0) * i128::from(y.1),
                        i128::from(x.1) * i128::from(y.0),
                        i128::from(x.1) * i128::from(y.1),
                    ];
                    fits(*c.iter().min()?, *c.iter().max()?)
                }
                ("OpMinSingleInt", 1) => {
                    let x = a(0)?;
                    fits(-i128::from(x.1), -i128::from(x.0))
                }
                ("OpDivInt" | "OpDivIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    if y.0 <= 0 && y.1 >= 0 {
                        return None;
                    }
                    // Truncating division is monotone in each operand while the divisor
                    // keeps its sign, so the extremes are at the corners.
                    let c = [x.0 / y.0, x.0 / y.1, x.1 / y.0, x.1 / y.1];
                    fits(i128::from(*c.iter().min()?), i128::from(*c.iter().max()?))
                }
                ("OpRemInt" | "OpRemIntNullable", 2) => rem_range(data, nn, rv, args, depth),
                ("OpLandInt", 2) => {
                    // A non-negative literal masks ANY non-sentinel value into `0..=lit`.
                    let masked = |m: &Value, other: &Value| match m.unspan() {
                        Value::Int(k) if *k >= 0 => {
                            (super::non_sentinel::non_sentinel(data, nn, other)
                                || range(data, nn, rv, other, depth).is_some())
                            .then_some((0, i64::from(*k)))
                        }
                        Value::Long(k) if *k >= 0 => {
                            (super::non_sentinel::non_sentinel(data, nn, other)
                                || range(data, nn, rv, other, depth).is_some())
                            .then_some((0, *k))
                        }
                        _ => None,
                    };
                    if let Some(r) =
                        masked(&args[1], &args[0]).or_else(|| masked(&args[0], &args[1]))
                    {
                        return Some(r);
                    }
                    let (x, y) = (a(0)?, a(1)?);
                    (x.0 >= 0 && y.0 >= 0).then(|| (0, x.1.min(y.1)))
                }
                // `@FR-R-MaskRange` adds `^` beside `|`: over two non-negative ranges no bit
                // is set above the higher end's top bit, and no result is negative, so none
                // is the sentinel.
                ("OpLorInt" | "OpEorInt", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    if x.0 < 0 || y.0 < 0 {
                        return None;
                    }
                    // Every bit set in either lies below the higher end's top bit.
                    let top = x.1.max(y.1);
                    let hi = if top == 0 {
                        0
                    } else {
                        (1i64 << (64 - top.leading_zeros())).checked_sub(1)?
                    };
                    Some((0, hi))
                }
                ("OpSRightInt", 2) => {
                    let Value::Int(k) = args[1].unspan() else {
                        return None;
                    };
                    if !(0..64).contains(k) {
                        return None;
                    }
                    let x = a(0)?;
                    Some((x.0 >> k, x.1 >> k))
                }
                // The store layout's facts: a length word is `u32`, a byte is a byte.
                ("OpSizeText" | "OpLengthVector", 1) => Some((0, U32_MAX)),
                ("t_4text_size" | "t_4text_len" | "t_6vector_len", 1) => Some((0, U32_MAX)),
                ("t_4text_byte_at", 2) => Some((0, 255)),
                _ => None,
            }
        }
    }
}

/// The range a user function answers for THESE arguments, its parameters carrying the
/// arguments' facts.  A ONE-expression body is evaluated directly.  A longer body — when the
/// arguments are ranged (`@FR-R-RangedCall`'s callee clause) — is analysed whole under those
/// seeds ([`range_vars`]), and its answer is the union of every `return`'s value and the
/// tail's; one unranged exit and it answers `None`.  A native or a fn-ref answers `None`: a
/// range cannot be read off a body this walk does not see whole.  Memoised per callee and
/// argument ranges, since an emitter asks at every operator.
fn call_range<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    def: &crate::data::Definition,
    args: &[Value],
    elem: Option<u16>,
    depth: u8,
) -> Option<Range> {
    // The value an exit answers, or its element `elem` when the call is read as a tuple.
    let answer = |nn: &HashMap<u16, bool>, rv: &Ranges, e: &Value, depth: u8| match elem {
        None => range(data, nn, rv, e, depth),
        Some(i) => elem_range(data, nn, rv, e, i, depth),
    };
    if depth >= CALL_DEPTH {
        return None;
    }
    let Value::Block(body) = def.code() else {
        return None;
    };
    let attrs = def.attributes();
    if attrs.len() != args.len() {
        return None;
    }
    let vars = def.variables();
    let mut nn2: HashMap<u16, bool> = HashMap::new();
    let mut rv2 = Ranges::default();
    let mut key: Vec<Option<Range>> = Vec::with_capacity(args.len());
    for (at, arg) in attrs.iter().zip(args) {
        let p = vars.var(&at.name);
        if p == u16::MAX {
            return None;
        }
        let r = range(data, nn, rv, arg, depth);
        key.push(r);
        if let Some(r) = r {
            rv2.vars.insert(p, r);
            nn2.insert(p, true);
        } else if super::non_sentinel::non_sentinel(data, nn, arg) {
            nn2.insert(p, true);
        }
    }
    let mut stmts = body
        .operators
        .iter()
        .filter(|op| !matches!(op, Value::Line(_)));
    let tail = stmts.next()?;
    if stmts.next().is_none() {
        return answer(&nn2, &rv2, tail, depth + 1);
    }
    if !ranged_calls_enabled() || rv2.vars.is_empty() {
        return None;
    }
    let memo_key = (std::ptr::from_ref(def) as usize, elem, key);
    if let Some(hit) = CALL_MEMO.with(|m| m.borrow().get(&memo_key).copied()) {
        return hit;
    }
    // `range_vars` re-enters `range` at depth 0, so `depth` cannot bound this recursion:
    // an explicit nesting count does, and the memo entry is marked IN PROGRESS (unranged)
    // before the analysis, so a recursive callee meets itself as unranged instead of
    // analysing itself again (cbor's `encode` crashed the compiler on the stack).
    if LONG_DEPTH.with(std::cell::Cell::get) >= CALL_DEPTH {
        return None;
    }
    CALL_MEMO.with(|m| m.borrow_mut().insert(memo_key.clone(), None));
    LONG_DEPTH.with(|d| d.set(d.get() + 1));
    // The longer body: its own facts under the parameters' seeds.
    let code = def.code();
    let callee_nn = super::non_sentinel::non_sentinel_vars(data, def.variables(), code);
    let seeded = Ranges {
        vars: rv2.vars,
        ..Ranges::default()
    };
    let callee_rv = range_vars(
        data,
        vars,
        code,
        &callee_nn,
        &std::collections::BTreeMap::new(),
        &seeded,
    );
    let mut out: Option<Range> = None;
    let mut ok = true;
    code.walk(&mut |n| {
        if ok && let Value::Return(e) = n {
            match answer(&callee_nn, &callee_rv, e, depth + 1) {
                Some(r) => out = Some(out.map_or(r, |o| union(o, r))),
                None => ok = false,
            }
        }
    });
    if ok {
        let tail = body
            .operators
            .iter()
            .rev()
            .find(|op| !matches!(op, Value::Line(_)));
        match tail.map(Value::unspan) {
            // A body ending in a `return` has no fall-through value.
            Some(Value::Return(_)) | None => {}
            Some(t) => match answer(&callee_nn, &callee_rv, t, depth + 1) {
                Some(r) => out = Some(out.map_or(r, |o| union(o, r))),
                None => ok = false,
            },
        }
    }
    LONG_DEPTH.with(|d| d.set(d.get() - 1));
    let answer = if ok { out } else { None };
    CALL_MEMO.with(|m| m.borrow_mut().insert(memo_key, answer));
    answer
}

thread_local! {
    /// How many longer-body analyses `call_range` is nested in right now.
    static LONG_DEPTH: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    /// `call_range`'s memo for longer bodies: (callee definition address, argument ranges).
    /// A fact about ONE program: once its `Data` is freed, another program's definition can
    /// take the same address, and kept, the memo handed it the first one's range — a proof
    /// for the wrong body, which can drop an overflow guard.  [`begin_generation`] clears it.
    #[allow(clippy::type_complexity)]
    static CALL_MEMO: std::cell::RefCell<
        HashMap<(usize, Option<u16>, Vec<Option<Range>>), Option<Range>>,
    > =
        std::cell::RefCell::new(HashMap::new());
}

/// The range of element `i` of the tuple `v` answers: a tuple literal's own element, an `if`
/// merge's union, a block's tail, or a user call's — its exits read the same way, in context
/// ([`call_range`]).  Every other shape is unranged, which costs only the checked template.
#[must_use]
pub fn elem_range<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    v: &Value,
    i: u16,
    depth: u8,
) -> Option<Range> {
    match v.unspan() {
        Value::Tuple(items) => range(data, nn, rv, items.get(usize::from(i))?, depth),
        Value::If(_, then_arm, else_arm) => Some(union(
            elem_range(data, nn, rv, then_arm, i, depth)?,
            elem_range(data, nn, rv, else_arm, i, depth)?,
        )),
        Value::Block(b) => {
            let tail = b
                .operators
                .iter()
                .rev()
                .find(|op| !matches!(op, Value::Line(_)))?;
            elem_range(data, nn, rv, tail, i, depth)
        }
        Value::Return(inner) => elem_range(data, nn, rv, inner, i, depth),
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let def = data.def(*d);
            if !def.is_loft_defined() {
                return None;
            }
            call_range(data, nn, rv, def, args, Some(i), depth)
        }
        _ => None,
    }
}

/// One round of the tuple facts: a tuple local whose every assignment answers element `i`
/// in range and that is never handed out by reference carries that element's union.  A local
/// written element by element (`t.0 = x`, a `TuplePut`) is such a hand-out
/// ([`super::non_sentinel::collect_escapes`]), so its elements carry no fact.  Entries are only added, never replaced,
/// so the facts grow monotonically with the variables' as `range_vars` iterates.
fn seed_elems(
    data: &Data,
    vars: &crate::variables::Function,
    code: &Value,
    nn: &HashMap<u16, bool>,
    escaped: &std::collections::HashSet<u16>,
    rv: &mut Ranges,
) {
    let mut sets: HashMap<u16, Vec<&Value>> = HashMap::new();
    code.walk(&mut |n| {
        if let Value::Set(t, e) = n.unspan()
            && matches!(vars.tp(*t).base(), crate::data::Type::Tuple(_))
        {
            sets.entry(*t).or_default().push(e);
        }
    });
    for (t, exprs) in sets {
        if escaped.contains(&t) {
            continue;
        }
        let crate::data::Type::Tuple(items) = vars.tp(t).base() else {
            continue;
        };
        for i in 0..items.len() {
            let Ok(i) = u16::try_from(i) else { break };
            if rv.elems.contains_key(&(t, i)) {
                continue;
            }
            let mut acc: Option<Range> = None;
            let mut ok = true;
            for e in &exprs {
                let Some(r) = elem_range(data, nn, &*rv, e, i, 0) else {
                    ok = false;
                    break;
                };
                acc = Some(acc.map_or(r, |a| union(a, r)));
            }
            if ok && let Some(r) = acc {
                rv.elems.insert((t, i), r);
            }
        }
    }
}

/// `LOFT_NO_WHILE_COUNTER=1` — `@FR-R-Range`'s while-counter clause off.
#[must_use]
pub fn while_counters_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_NO_WHILE_COUNTER").map_or(true, |v| v == "0"))
}

fn is_op(data: &Data, v: &Value, name: &str) -> Option<Vec<Value>> {
    match v.unspan() {
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len() && data.def(*d).name() == name =>
        {
            Some(args.clone())
        }
        _ => None,
    }
}

fn mentions_var(v: &Value, i: u16) -> bool {
    v.any_node(&mut |n| matches!(n, Value::Var(x) if *x == i))
}

/// `@FR-R-Range`'s while-counter clause — the counter `i` of a loop shaped
/// `while i < E { …; i += 1 }`: the loop's first statement is the test's break
/// (`if !(i < E) break`, `E` naming no `i`), its last is the step `i = i + 1`, and the step is
/// the loop's one write of `i`.  Every statement of the loop then runs after the test passed
/// and before the step, where `i < E <= i64::MAX` — and a null `E` (`i64::MIN`) ends the loop,
/// since native compares the raw words.  `None` for every other loop.
#[must_use]
pub fn while_counter_shape(data: &Data, lp: &crate::data::Block) -> Option<u16> {
    let mut ops = lp
        .operators
        .iter()
        .filter(|op| !matches!(op, Value::Line(_)));
    let Value::If(cond, _, _) = ops.next()?.unspan() else {
        return None;
    };
    let not = is_op(data, cond, "OpNot")?;
    let lt = is_op(data, not.first()?, "OpLtInt")?;
    let (Value::Var(i), bound) = (lt.first()?.unspan(), lt.get(1)?) else {
        return None;
    };
    if mentions_var(bound, *i) {
        return None;
    }
    let Value::Block(body) = lp
        .operators
        .iter()
        .rev()
        .find(|op| !matches!(op, Value::Line(_)))?
        .unspan()
    else {
        return None;
    };
    let Value::Set(x, step) = body
        .operators
        .iter()
        .rev()
        .find(|op| !matches!(op, Value::Line(_)))?
        .unspan()
    else {
        return None;
    };
    let add = is_op(data, step, "OpAddInt")?;
    let one = matches!(
        add.get(1).map(Value::unspan),
        Some(Value::Int(1) | Value::Long(1))
    );
    if *x != *i || !matches!(add.first().map(Value::unspan), Some(Value::Var(v)) if v == i) || !one
    {
        return None;
    }
    let mut writes = 0;
    Value::Loop(Box::new(lp.clone())).walk(&mut |n| {
        if let Value::Set(v, _) | Value::TuplePut(v, _, _) = n
            && *v == *i
        {
            writes += 1;
        }
    });
    (writes == 1).then_some(*i)
}

/// One round of the while counters: a local whose every self-step is the step of a loop
/// [`while_counter_shape`] admits for it, and whose every other assignment is ranged and
/// names it not, lies in `[lowest seed, i64::MAX]` — it only grows, by one, while below a
/// bound no larger than `i64::MAX`.
fn seed_while_counters(
    data: &Data,
    code: &Value,
    nn: &HashMap<u16, bool>,
    escaped: &std::collections::HashSet<u16>,
    rv: &mut Ranges,
    counters: &mut std::collections::HashSet<u16>,
) {
    if !while_counters_enabled() {
        return;
    }
    let mut shaped: HashMap<u16, usize> = HashMap::new();
    code.walk(&mut |n| {
        if let Value::Loop(lp) = n
            && let Some(i) = while_counter_shape(data, lp)
        {
            *shaped.entry(i).or_default() += 1;
        }
    });
    for (i, loops) in shaped {
        if counters.contains(&i) || escaped.contains(&i) || rv.vars.contains_key(&i) {
            continue;
        }
        let mut steps = 0;
        let mut lo: Option<i64> = None;
        let mut ok = true;
        code.walk(&mut |n| {
            if !ok {
                return;
            }
            match n.unspan() {
                Value::Set(v, e) if *v == i => {
                    if mentions_var(e, i) {
                        steps += 1;
                    } else if let Some(r) = range(data, nn, &*rv, e, 0) {
                        lo = Some(lo.map_or(r.0, |l| l.min(r.0)));
                    } else {
                        ok = false;
                    }
                }
                Value::TuplePut(v, _, _) if *v == i => ok = false,
                _ => {}
            }
        });
        // Each self-step is one shaped loop's step: a step anywhere else is a second write.
        if ok
            && steps == loops
            && let Some(lo) = lo
        {
            rv.vars.insert(i, (lo, i64::MAX));
            counters.insert(i);
        }
    }
}

/// Forget every memoised call range: a new run is about a new program.  Called by
/// [`super::begin_program`], beside `non_sentinel::begin_generation`.
pub(crate) fn begin_generation() {
    CALL_MEMO.with(|m| m.borrow_mut().clear());
}

/// `LOFT_NO_RANGED_CALLS=1` — `@FR-R-RangedCall` off: no callee body longer than one
/// expression is analysed, no guarded copy and no ranged variant is emitted.
#[must_use]
pub fn ranged_calls_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_NO_RANGED_CALLS").map_or(true, |v| v == "0"))
}

/// Per-function var facts: a local is ranged iff every `Set` to it assigns a ranged
/// expression, none of them names the local itself (no self-step induction: `x = x + 1`
/// has no least fixpoint, and a var that can step itself can overflow itself), and it is
/// never writable behind the map's back ([`super::non_sentinel::collect_escapes`]).  A
/// counted range's counters are seeded from the loop's own ends when both are ranged.
///
/// Pessimistic start, re-walk to fixpoint: a var enters the map only once every input of
/// every assignment is in it, so its range never changes afterwards and the map grows
/// monotonically; it settles in at most `vars` rounds.
#[must_use]
pub fn range_vars(
    data: &Data,
    vars: &crate::variables::Function,
    code: &Value,
    nn: &HashMap<u16, bool>,
    walks: &std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
    seeds: &Ranges,
) -> Ranges {
    let mut escaped: std::collections::HashSet<u16> = std::collections::HashSet::new();
    super::non_sentinel::collect_escapes(data, code, &mut escaped);
    let mut rv = Ranges {
        vars: HashMap::new(),
        fields: seeds.fields.clone(),
        elems: HashMap::new(),
    };
    // `@FR-R-RangedCall`'s seeds — a variable the caller vouches for (a ranged variant's
    // parameter) holds its seed only while nothing in the code can change it: the fixpoint
    // never replaces an entry, so a seeded variable that is assigned, or handed out by
    // reference, would keep a range its new value need not have.
    for (&v, &r) in &seeds.vars {
        let written = code
            .any_node(&mut |n| matches!(n, Value::Set(x, _) | Value::TuplePut(x, _, _) if *x == v));
        if !written && !escaped.contains(&v) {
            rv.vars.insert(v, r);
        }
    }
    // `@FR-R-Range`'s type clause — every variable whose STATIC TYPE is a fact carries that
    // range from the start: a `u8` or `limit(lo, hi)` parameter (which no `Set` ever ranges),
    // and a local the compiler typed narrow.  Seeded before the fixpoint, so a `Set` into such
    // a variable is judged against the type's range rather than widening it: the fixpoint
    // below never inserts over an existing entry, and the type is the narrower fact.
    for v in 0..vars.count() {
        if let Some(r) = type_range(vars.tp(v)) {
            rv.vars.entry(v).or_insert(r);
        }
    }
    // Counters first: their step names themselves, which the fixpoint below refuses.
    let mut counters: std::collections::HashSet<u16> = std::collections::HashSet::new();
    seed_counters(data, code, code, nn, &mut rv, &mut counters);
    // Then the walks' accumulators — the second self-stepping shape read off a loop.
    seed_accumulators(data, code, nn, &escaped, walks, &mut rv, &mut counters);
    seed_while_counters(data, code, nn, &escaped, &mut rv, &mut counters);
    loop {
        let mut round: HashMap<u16, Option<Range>> = HashMap::new();
        let elems_before = rv.elems.len();
        seed_elems(data, vars, code, nn, &escaped, &mut rv);
        scan_sets(code, data, nn, &rv, &counters, &mut round);
        let mut changed = false;
        for (nr, r) in round {
            // A parameter is never ranged by its assignments (`@FR-R-Range`, loft#2143): the
            // caller's argument is one the body does not show.  Its stated seeds — the type
            // clause, a ranged variant's — were entered above and are not this fixpoint's.
            if let Some(r) = r
                && !escaped.contains(&nr)
                && !counters.contains(&nr)
                && !vars.is_argument(nr)
                && !rv.vars.contains_key(&nr)
            {
                rv.vars.insert(nr, r);
                changed = true;
            }
        }
        if !changed && rv.elems.len() != elems_before {
            changed = true;
        }
        if !changed {
            // A counter whose END is a local the fixpoint has only now ranged — the hidden
            // `_range_end` a non-literal bound is taken into (`@FR-I-Range`: once, before the
            // first round) — is seeded here, and the fixpoint resumes with it.  Seeding is
            // monotone: a counter enters the map once both its seed and its end are ranged,
            // and neither leaves it.
            // The accumulators likewise: a step under a counted loop is bounded by that
            // loop's trips, which are known once its counter is — an inner loop whose end is
            // an outer counter (`for _ in 0..i`) is ranged only in this resumed round.
            let before = rv.vars.len();
            seed_counters(data, code, code, nn, &mut rv, &mut counters);
            seed_accumulators(data, code, nn, &escaped, walks, &mut rv, &mut counters);
            seed_while_counters(data, code, nn, &escaped, &mut rv, &mut counters);
            if rv.vars.len() == before {
                return rv;
            }
        }
    }
}

/// One round: fold every `Set(var, expr)`'s range under the current map into `acc` —
/// the union of the assignments, or `None` once any assignment is unranged or names the
/// var itself.
fn scan_sets<F: RangeFacts + ?Sized>(
    v: &Value,
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    counters: &std::collections::HashSet<u16>,
    acc: &mut HashMap<u16, Option<Range>>,
) {
    if let Value::Set(nr, expr) = v.unspan()
        && !counters.contains(nr)
    {
        let self_step = expr.any_node(&mut |n| matches!(n, Value::Var(x) if x == nr));
        let r = if self_step {
            None
        } else {
            range(data, nn, rv, expr, 0)
        };
        acc.entry(*nr)
            .and_modify(|e| {
                *e = match (*e, r) {
                    (Some(a), Some(b)) => Some(union(a, b)),
                    _ => None,
                }
            })
            .or_insert(r);
    }
    v.for_each_child(&mut |c| scan_sets(c, data, nn, rv, counters, acc));
}

/// Seed every counted loop's counters from the loop's own ends: with the seed `s` of the
/// stepped counter and the end `hi` both ranged, the index runs over `s ..= hi`, the `next`
/// counter over `s ..= hi`, and the loop variable over `s(+1) ..= hi(-1)` — the single-counter
/// form seeds its index one BELOW the start, the two-counter form seeds `next` AT it.  A
/// counter whose seed or end is unranged is left out, and named in `counters` either way
/// so the fixpoint never reads its self-step.
///
/// `root` is the whole function body and `v` the node being walked.  The seed is looked for
/// in `root`, NOT in the loop: the parser emits `index = <start>` as the statement BEFORE the
/// loop, so searching the loop found nothing and every counted loop went unranged — the
/// clause read as implemented and was inert (loft#1558).  Searching the function is what
/// makes it sound rather than merely wider: `v_seed` counts EVERY non-step `Set` to that
/// counter anywhere in the function, so a counter written from a second place declines on
/// `seeds != 1` instead of taking the first value it meets.
fn seed_counters(
    data: &Data,
    root: &Value,
    v: &Value,
    nn: &HashMap<u16, bool>,
    rv: &mut Ranges,
    counters: &mut std::collections::HashSet<u16>,
) {
    if let Value::Loop(lp) = v.unspan()
        && let Ok(rc) = crate::loop_shape::range_counters(lp, data)
    {
        counters.insert(rc.index);
        counters.insert(rc.loop_var);
        if let Some(n) = rc.next {
            counters.insert(n);
        }
        let stepped = rc.next.unwrap_or(rc.index);
        // The seed: the one `Set` to the stepped counter that is not its own step.
        let mut seed: Option<Range> = None;
        let mut seeds = 0usize;
        v_seed(data, nn, rv, root, stepped, &mut seed, &mut seeds);
        if seeds == 1
            && let Some(s) = seed
            && let Some(h) = range(data, nn, rv, rc.hi, 0)
        {
            // The stepped counter reaches at most `hi` (exclusive end: the test breaks
            // at `hi <= counter`; inclusive: the stop guard breaks at `hi == index`).
            let top = h.1;
            let lo_var = if rc.next.is_some() {
                s.0
            } else {
                s.0.saturating_add(1)
            };
            let hi_var = if rc.inclusive {
                top
            } else {
                top.saturating_sub(1)
            };
            rv.vars.insert(stepped, (s.0, top.max(s.0)));
            if rc.next.is_some() {
                rv.vars.insert(rc.index, (lo_var, hi_var.max(lo_var)));
            }
            rv.vars.insert(rc.loop_var, (lo_var, hi_var.max(lo_var)));
        }
    }
    v.for_each_child(&mut |c| seed_counters(data, root, c, nn, rv, counters));
}

/// `@FR-R-Range`'s accumulator clause — the second self-stepping shape read off a loop,
/// after the counters: a local `n` seeded ONCE by a ranged value (`n = 0`) and stepped
/// only by literals (`n += 1`, `n -= 2`), every step straight-line after the seed in the
/// seed's own block or under loops there whose trips are bounded — a character walk over a
/// text the body never writes (`CharWalk::hoist_null`, at most `size(T)`, a `u32` word) or
/// a counted loop (at most `hi - lo + 1` over its stepped counter's range).  A step under
/// nested bounded loops moves `n` by `|c|` times the product of their bounds, so the seed
/// plus the sum over every step is `n`'s range whenever it fits the type: the checked
/// `n + c` cannot fault, and the processor's add answers what the template would.  The
/// seed's block may itself sit in a loop — each pass re-seeds — but a step under a loop
/// with no bound, a step by anything but a literal, a second seed, a write to `n` anywhere
/// else, or `n` handed out by reference declines, and `n` stays unranged.  Measured: the
/// stdlib text bench's `char_walk` 12.5 → 10.5 µs (−17 %); the insertion sort's `j -= 1`
/// under `for _ in 0..i` (`bench/10_sort`) −12 %.
#[expect(clippy::too_many_lines, reason = "inherited")]
fn seed_accumulators(
    data: &Data,
    code: &Value,
    nn: &HashMap<u16, bool>,
    escaped: &std::collections::HashSet<u16>,
    walks: &std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
    rv: &mut Ranges,
    counters: &mut std::collections::HashSet<u16>,
) {
    /// The literal step `n = n + c` / `n = n - c` (`OpAddInt` / `OpMinInt`, `|c| < 2^31`)
    /// answers `c` signed.
    fn literal_step(data: &Data, acc: u16, expr: &Value) -> Option<i64> {
        let Value::Call(def_nr, args) = expr.unspan() else {
            return None;
        };
        let sign = match data.def(*def_nr).name() {
            "OpAddInt" => 1,
            "OpMinInt" => -1,
            _ => return None,
        };
        if args.len() != 2 || !matches!(args[0].unspan(), Value::Var(x) if *x == acc) {
            return None;
        }
        let step = match args[1].unspan() {
            Value::Int(k) => i64::from(*k),
            Value::Long(k) => *k,
            _ => return None,
        };
        (step.unsigned_abs() < (1u64 << 31)).then_some(sign * step)
    }
    /// How many times one entry of `lp` can run its body, or `None` when nothing bounds it.
    /// A character walk over a text the body never writes makes at most `size(T)` trips, a
    /// `u32` word.  A counted loop steps its counter by one from its single seed until the
    /// end test breaks, so it makes at most `hi - lo + 1` trips over the counter's range —
    /// the range `seed_counters` recorded from the seed and the end.  Any other loop (a
    /// `while`, a counted loop whose seed or end is unranged) is unbounded here.
    fn trips(
        data: &Data,
        walks: &std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
        rv: &Ranges,
        lp: &crate::data::Block,
    ) -> Option<i128> {
        if walks.get(&lp.scope).is_some_and(|w| w.hoist_null) {
            return Some(i128::from(U32_MAX));
        }
        let rc = crate::loop_shape::range_counters(lp, data).ok()?;
        let (lo, hi) = *rv.vars.get(&rc.next.unwrap_or(rc.index))?;
        Some((i128::from(hi) - i128::from(lo) + 1).max(0))
    }
    /// The total movement the steps under `node` can make in one run of the seed's block —
    /// `None` where a step is not one this clause reads.  `mult` is the product of the trip
    /// bounds of the loops between the seed's block and `node` (1 outside any loop), `None`
    /// once one of them is unbounded: a step under such a loop declines.
    fn movement(
        data: &Data,
        walks: &std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
        rv: &Ranges,
        node: &Value,
        acc: u16,
        mult: Option<i128>,
        steps: &mut usize,
    ) -> Option<i128> {
        // Past this the bound cannot fit an `i64` range anyway; stopping here keeps the
        // products inside `i128`.
        const CAP: i128 = 1 << 96;
        match node.unspan() {
            Value::Set(target, expr) if *target == acc => {
                let step = i128::from(literal_step(data, acc, expr)?.unsigned_abs());
                *steps += 1;
                Some(step * mult?)
            }
            Value::TuplePut(target, _, _) if *target == acc => None,
            Value::Loop(lp) => {
                let inner = mult
                    .zip(trips(data, walks, rv, lp))
                    .map(|(m, t)| m * t)
                    .filter(|m| *m <= CAP);
                let mut total: i128 = 0;
                for op in &lp.operators {
                    total += movement(data, walks, rv, op, acc, inner, steps)?;
                }
                Some(total)
            }
            _ => {
                let mut total: i128 = 0;
                let mut ok = true;
                node.for_each_child(&mut |child| {
                    if ok {
                        match movement(data, walks, rv, child, acc, mult, steps) {
                            Some(m) => total += m,
                            None => ok = false,
                        }
                    }
                });
                ok.then_some(total)
            }
        }
    }
    code.any_node(&mut |node| {
        let Value::Block(bl) = node else { return false };
        for (k, op) in bl.operators.iter().enumerate() {
            let Value::Set(n, e) = op.unspan() else {
                continue;
            };
            let n = *n;
            if counters.contains(&n) || rv.vars.contains_key(&n) || escaped.contains(&n) {
                continue;
            }
            // The seed: ranged, and not itself a step.
            if literal_step(data, n, e).is_some() {
                continue;
            }
            let Some(seed) = range(data, nn, rv, e, 0) else {
                continue;
            };
            // Every write to `n` in the function: this seed, and the steps after it here.
            let mut writes = 0usize;
            code.any_node(&mut |m| {
                if matches!(m, Value::Set(x, _) | Value::TuplePut(x, _, _) if *x == n) {
                    writes += 1;
                }
                false
            });
            let mut steps = 0usize;
            let bound: Option<i128> =
                bl.operators[k + 1..]
                    .iter()
                    .try_fold(0i128, |total, later| {
                        Some(total + movement(data, walks, rv, later, n, Some(1), &mut steps)?)
                    });
            let Some(bound) = bound else { continue };
            if steps == 0 || writes != steps + 1 {
                continue;
            }
            if let Some(r) = fits(i128::from(seed.0) - bound, i128::from(seed.1) + bound) {
                rv.vars.insert(n, r);
                counters.insert(n);
            }
        }
        false
    });
}

/// Find the seed `Set` of a counter: every `Set(counter, e)` whose `e` is not the step
/// `counter + 1`.  Counts them, so a counter set from two places declines.
fn v_seed<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    v: &Value,
    counter: u16,
    seed: &mut Option<Range>,
    seeds: &mut usize,
) {
    if let Value::Set(nr, e) = v.unspan()
        && *nr == counter
    {
        let is_step = matches!(e.unspan(), Value::Call(d, a)
            if data.def(*d).name() == "OpAddInt"
                && a.len() == 2
                && matches!(a[0].unspan(), Value::Var(c) if *c == counter));
        if !is_step {
            *seeds += 1;
            *seed = range(data, nn, rv, e, 0);
        }
    }
    v.for_each_child(&mut |c| v_seed(data, nn, rv, c, counter, seed, seeds));
}

/// The plain form of an op the range proof may emit: `wrapping_*` for the four arithmetic
/// operators (the exact value, since the range says no step can overflow), `/` and `%`
/// for a division whose range excludes a zero divisor.  `None` for every other op.
#[must_use]
pub fn plain_form(op_name: &str, args: usize) -> Option<&'static str> {
    match (op_name, args) {
        ("OpAddInt" | "OpAddIntNullable", 2) => Some("wrapping_add"),
        ("OpMinInt" | "OpMinIntNullable", 2) => Some("wrapping_sub"),
        ("OpMulInt" | "OpMulIntNullable", 2) => Some("wrapping_mul"),
        ("OpMinSingleInt", 1) => Some("wrapping_neg"),
        ("OpDivInt" | "OpDivIntNullable", 2) => Some("/"),
        ("OpRemInt" | "OpRemIntNullable", 2) => Some("%"),
        _ => None,
    }
}

/// Why [`range`] answered `None` for `v`: the first leaf the proof could not range, named by
/// KIND, or the operator whose ranged operands may still overflow.  `@FR-R-Range`'s decline
/// reason for the decline census (`LOFT_DECLINE_CENSUS`), so the widening with the most
/// checked operators behind it is a count, not a guess.  A report only: nothing is emitted
/// from it.  The fallback names the node's shape rather than a cause, which a reader of the
/// census takes as "not classified" — it can only under-report a kind, never admit a site.
#[must_use]
pub fn blocker<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    vars: &crate::variables::Function,
    code: &Value,
    v: &Value,
    depth: u8,
) -> &'static str {
    let unranged = |x: &Value| range(data, nn, rv, x, 0).is_none();
    let next = |x: &Value| blocker(data, nn, rv, vars, code, x, depth + 1);
    if depth > 6 {
        return "a chain of locals";
    }
    match v.unspan() {
        Value::Long(l) if *l == i64::MIN => "a null literal",
        Value::Var(nr) if vars.is_argument(*nr) => "a parameter",
        // A local is charged to the ROOT of its first unranged assignment, so the census
        // counts the leaf a widening would have to range, whatever locals carry it.
        Value::Var(nr) => local_blocker(data, nn, rv, vars, code, *nr, depth),
        Value::If(_, then_arm, else_arm) => {
            if unranged(then_arm) {
                next(then_arm)
            } else {
                next(else_arm)
            }
        }
        Value::Block(b) => match b
            .operators
            .iter()
            .rev()
            .find(|op| !matches!(op, Value::Line(_)))
        {
            Some(tail) => next(tail),
            None => "an empty block",
        },
        Value::Return(inner) => next(inner),
        Value::Call(d_nr, args) => {
            if (*d_nr as usize) >= data.definitions.len() {
                return "a call of another shape";
            }
            let def = data.def(*d_nr);
            let name = def.name();
            if plain_form(name, args.len()).is_some()
                || matches!(name, "OpLandInt" | "OpLorInt" | "OpEorInt" | "OpSRightInt")
            {
                return match args.iter().find(|a| unranged(a)) {
                    Some(a) => next(a),
                    None => "ranged operands whose result may overflow",
                };
            }
            if name.starts_with("OpGet") {
                let element = args.first().is_some_and(|b| {
                    matches!(b.unspan(), Value::Call(e, _)
                        if (*e as usize) < data.definitions.len()
                            && data.def(*e).name().starts_with("OpGetVector"))
                });
                return if element {
                    "a vector element"
                } else {
                    "a record field"
                };
            }
            if name.starts_with("OpConv") || name.starts_with("OpCast") {
                return "a conversion";
            }
            if def.is_loft_defined() {
                return "a user call's result";
            }
            "a native call's result"
        }
        _ => "an expression of another shape",
    }
}

/// [`blocker`] for a LOCAL: charged to the root of its first unranged assignment, so the
/// census counts the leaf a widening would have to range, whatever locals carry it; a
/// self-step, a reference hand-out and a tuple bind are kinds of their own.
fn local_blocker<F: RangeFacts + ?Sized>(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &F,
    vars: &crate::variables::Function,
    code: &Value,
    nr: u16,
    depth: u8,
) -> &'static str {
    let unranged = |x: &Value| range(data, nn, rv, x, 0).is_none();
    let next = |x: &Value| blocker(data, nn, rv, vars, code, x, depth + 1);
    let mut escaped = std::collections::HashSet::new();
    super::non_sentinel::collect_escapes(data, code, &mut escaped);
    if escaped.contains(&nr) {
        return "a local handed out by reference";
    }
    let mut tuple = false;
    let mut sets: Vec<&Value> = Vec::new();
    code.walk(&mut |n| match n.unspan() {
        Value::Set(x, e) if *x == nr => sets.push(e),
        Value::TuplePut(x, _, _) if *x == nr => tuple = true,
        _ => {}
    });
    if tuple {
        return "a local bound from a tuple";
    }
    let steps: Vec<&&Value> = sets
        .iter()
        .filter(|e| e.any_node(&mut |n| matches!(n, Value::Var(x) if *x == nr)))
        .collect();
    if !steps.is_empty() {
        let name = vars.name(nr);
        if name.contains("#index") || name.starts_with("_index") {
            return "a counter whose range has an unranged end";
        }
        // A step by literals only (`n += 1`) is bounded by its loop's trips; a step by
        // anything else is the accumulator's own unranged value.
        let literal = steps.iter().all(|e| {
            !e.any_node(&mut |n| match n.unspan() {
                Value::Var(x) => *x != nr,
                Value::Call(..) => !matches!(n.unspan(), Value::Call(d, _)
                        if (*d as usize) < data.definitions.len()
                            && plain_form(data.def(*d).name(), 2).is_some()),
                _ => false,
            })
        });
        return if literal {
            "an accumulator stepped by a literal under an unbounded loop"
        } else {
            "an accumulator stepped by a value"
        };
    }
    match sets.into_iter().find(|e| unranged(e)) {
        Some(e) => next(e),
        None => "a local ranged only through another unranged local",
    }
}
