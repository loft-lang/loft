// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F38 — the test ledger's fault sites, named and instrumented on the IR

//! The fault sites of a program, named for the TEST LEDGER (@PLN201 step 2,
//! [`crate::ledger`]), and the instrumentation a ledger run gives them.
//!
//! A site is named on the IR, so the interpreter and every code generator agree: the
//! function's definition name, [`body_hash`] of its body, and the operator's ordinal among
//! the function's sites in a pre-order walk.  The walk runs on the closed program's bodies
//! before any whole-program rewrite (`compile::close_program` calls [`rewrite_program`]
//! first): a body `leaf_inline` copies into a caller then carries its callee's site numbers,
//! and a shipped build that names its sites at the same point reads the same names.
//!
//! Under `LOFT_LEDGER` each site `OpAddInt(a, b)` is instrumented as `__ledger(site, kind, a,
//! b)` (`__ledger_div` for a division, which reports a zero divisor), builtins both backends
//! call ([`crate::ledger::op`]): each answers what the operator answers and counts the site.  The rewritten operators no longer match the patterns later rewrites
//! look for, so those decline — a ledger run is a CHECKED run, and only its speed differs.

use crate::data::{Data, DefType, Value};
use crate::ledger::{self, Site};

/// The operators that are fault sites, by definition number — the set the release-pass
/// probe drops (`generation::ops::int_arith::probe_form`), so the ledger licenses exactly
/// what a shipped build may change.
pub struct SiteOps {
    ops: [(u32, i64); 10],
}

impl SiteOps {
    /// `None` when the stdlib declares none of them (a program parsed without it).
    #[must_use]
    pub fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| data.def_nr(n);
        let ops = [
            (nr("OpAddInt"), ledger::ADD),
            (nr("OpMinInt"), ledger::SUB),
            (nr("OpMulInt"), ledger::MUL),
            (nr("OpDivInt"), ledger::DIV),
            (nr("OpRemInt"), ledger::REM),
            (nr("OpMinSingleInt"), ledger::NEG),
            (nr("OpLandInt"), ledger::AND),
            (nr("OpLorInt"), ledger::OR),
            (nr("OpEorInt"), ledger::XOR),
            (nr("OpSRightInt"), ledger::SHR),
        ];
        ops.iter()
            .any(|(d, _)| *d != u32::MAX)
            .then_some(Self { ops })
    }

    /// The ledger kind of `Call(op, args)`, or `None` when it is no fault site.  A right shift
    /// is a site only by a constant in 0..64, as the probe has it: any other shift amount
    /// raises in the checked build and has no processor form to compare with.
    #[must_use]
    pub fn kind(&self, op: u32, args: &[Value]) -> Option<i64> {
        let (_, kind) = *self.ops.iter().find(|(d, _)| *d == op && *d != u32::MAX)?;
        let arity_ok = match kind {
            ledger::NEG => args.len() == 1,
            ledger::SHR => {
                args.len() == 2 && matches!(args[1].unspan(), Value::Int(k) if (0..64).contains(k))
            }
            _ => args.len() == 2,
        };
        arity_ok.then_some(kind)
    }
}

/// A hash of a function body that a rebuild of the same source reproduces and an edit to the
/// body changes: the tree's shape and every value in it, with call targets named by their
/// definition NAME (a definition number moves when another function is added) and source
/// positions left out (an edit above the function moves its lines, not its meaning).
/// FNV-1a, so it is the same on every host and every Rust release.
#[must_use]
pub fn body_hash(data: &Data, code: &Value) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    hash_value(data, code, &mut h);
    h.0
}

struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, b: &[u8]) {
        for byte in b {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn tag(&mut self, t: u8) {
        self.bytes(&[t]);
    }
    fn num(&mut self, n: u64) {
        self.bytes(&n.to_le_bytes());
    }
    fn text(&mut self, s: &str) {
        self.num(s.len() as u64);
        self.bytes(s.as_bytes());
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per IR node kind, each its own few bytes"
)]
fn hash_value(data: &Data, v: &Value, h: &mut Fnv) {
    match v {
        // Positions: transparent.
        Value::Span(b) => return hash_value(data, &b.1, h),
        Value::Line(_) => {
            h.tag(1);
            return;
        }
        Value::Null => h.tag(2),
        Value::Int(i) => {
            h.tag(3);
            h.num(u64::from(i.cast_unsigned()));
        }
        Value::Enum(a, b) => {
            h.tag(4);
            h.num(u64::from(*a));
            h.num(u64::from(*b));
        }
        Value::Boolean(b) => {
            h.tag(5);
            h.num(u64::from(*b));
        }
        Value::Float(f) => {
            h.tag(6);
            h.num(f.to_bits());
        }
        Value::Long(l) => {
            h.tag(7);
            h.num(l.cast_unsigned());
        }
        Value::Single(f) => {
            h.tag(8);
            h.num(u64::from(f.to_bits()));
        }
        Value::Text(s) => {
            h.tag(9);
            h.text(s);
        }
        Value::Call(d, _) => {
            h.tag(10);
            h.text(data.def(*d).name());
        }
        Value::CallRef(v, _) => {
            h.tag(11);
            h.num(u64::from(*v));
        }
        Value::Block(b) => {
            h.tag(12);
            h.text(b.name);
        }
        Value::Insert(_) => h.tag(13),
        Value::Var(n) => {
            h.tag(14);
            h.num(u64::from(*n));
        }
        Value::Set(n, _) => {
            h.tag(15);
            h.num(u64::from(*n));
        }
        Value::Return(_) => h.tag(16),
        Value::Break(n) => {
            h.tag(17);
            h.num(u64::from(*n));
        }
        Value::Continue(n) => {
            h.tag(18);
            h.num(u64::from(*n));
        }
        Value::If(..) => h.tag(19),
        Value::Loop(b) => {
            h.tag(20);
            h.text(b.name);
        }
        Value::Drop(_) => h.tag(21),
        Value::Iter(n, ..) => {
            h.tag(22);
            h.num(u64::from(*n));
        }
        Value::Keys(k) => {
            h.tag(23);
            h.text(&format!("{k:?}"));
        }
        Value::Tuple(_) => h.tag(24),
        Value::TupleGet(a, b) => {
            h.tag(25);
            h.num(u64::from(*a));
            h.num(u64::from(*b));
        }
        Value::TuplePut(a, b, _) => {
            h.tag(26);
            h.num(u64::from(*a));
            h.num(u64::from(*b));
        }
        Value::Yield(_) => h.tag(27),
        Value::FnRef(d, n, _) => {
            h.tag(28);
            match u32::try_from(*d) {
                Ok(d) if d < data.definitions() => h.text(data.def(d).name()),
                _ => h.num(u64::from(d.cast_unsigned())),
            }
            h.num(u64::from(*n));
        }
        Value::FnRefDnr(n) => {
            h.tag(29);
            h.num(u64::from(*n));
        }
        Value::Parallel(_) => h.tag(30),
        Value::RawExpr(s) => {
            h.tag(31);
            h.text(s);
        }
    }
    // The children, counted, so two shapes with the same pre-order of nodes differ.
    let mut n = 0u64;
    v.for_each_child(&mut |c| {
        n += 1;
        hash_value(data, c, h);
    });
    h.num(n);
}

/// Name every fault site of `code` in pre-order and hand each to `f` with its kind and
/// ordinal; `f` may replace the node (the replacement's children are visited next, so a site
/// nested in an operand is numbered after the site that holds it).
pub fn for_each_site(code: &mut Value, ops: &SiteOps, f: &mut impl FnMut(&mut Value, i64, u32)) {
    let mut ordinal = 0u32;
    code.map_nodes(&mut |n| {
        let Value::Call(op, args) = n else {
            return;
        };
        let Some(kind) = ops.kind(*op, args) else {
            return;
        };
        f(n, kind, ordinal);
        ordinal += 1;
    });
}

/// Under `LOFT_LEDGER`: instrument every fault site of every loft-defined function, record
/// the site table in [`Data::ledger_sites`], and arm it for the run this process makes.
/// Nothing at all otherwise.
pub fn rewrite_program(data: &mut Data) {
    if !ledger::armed_for_build() {
        return;
    }
    let Some(ops) = SiteOps::lookup(data) else {
        return;
    };
    let ledger_fn = data.def_nr("n___ledger");
    let ledger_div = data.def_nr("n___ledger_div");
    if ledger_fn == u32::MAX || ledger_div == u32::MAX {
        return;
    }
    let mut sites: Vec<Site> = Vec::new();
    for d_nr in 0..data.definitions.len() {
        let d = d_nr as u32;
        if data.def_type(d) != DefType::Function || !data.def(d).is_loft_defined() {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d_nr].code, Value::Null);
        let body = body_hash(data, &code);
        let function = data.def(d).name().to_string();
        for_each_site(&mut code, &ops, &mut |n, kind, ordinal| {
            let Value::Call(_, args) = n else {
                return;
            };
            let site = i32::try_from(sites.len()).unwrap_or(i32::MAX);
            let mut operands = std::mem::take(args).into_iter();
            let a = operands.next().unwrap_or(Value::Null);
            let b = operands.next().unwrap_or(Value::Int(0));
            let call = if matches!(kind, ledger::DIV | ledger::REM) {
                ledger_div
            } else {
                ledger_fn
            };
            *n = Value::Call(call, vec![Value::Int(site), Value::Int(kind as i32), a, b]);
            sites.push(Site {
                function: function.clone(),
                body,
                ordinal,
                kind,
            });
        });
        data.definitions[d_nr].code = code;
    }
    ledger::arm(&sites);
    data.ledger_sites = sites;
}
