// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Version` — a LEAF's bounds tests, taken once per loop.
//!
//! A leaf is a function with no loop and no call but operators.  Its FAST PATH is read off
//! its body here: every `if` whose condition is integer comparisons of LINEAR expressions over
//! the parameters and the held-inputs twin's inputs (`(R-Inputs)`: record scalars and held
//! headers' lengths), and one of whose arms alone reads or writes a vector element, is
//! DECIDED toward that arm; every element read or written at a linear index takes the tests
//! `0 <= index` and `index < len`.  Those tests, in order, are the PRECONDITION; the writer
//! evaluates it with every operation checked, and calls the fast twin — the body with each
//! decided `if` its arm, each element access at its index, and each operation of the tests
//! plain — where the precondition holds over the whole range the loop's counter still takes.
//!
//! The analysis knows nothing of C: the inputs are asked of the caller through
//! [`Inputs`], and the plan names IR nodes by address (an `if`'s condition, an operator's
//! argument slice), which the twin's writer meets as it writes the same definition.

use crate::data::{Data, Type, Value};
use std::collections::{HashMap, HashSet};

#[must_use]
pub fn enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_VERSION").is_ok_and(|v| v != "0"))
}

#[must_use]
pub fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_VERSION").is_ok_and(|v| v != "0"))
}

/// A linear expression over the leaf's parameters and inputs.  Each operator carries its IR
/// argument slice's address, which the fast twin writes plain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lin {
    Const(i64),
    /// A scalar parameter, by its variable.
    Param(u16),
    /// A record parameter's scalar field: an `(R-Inputs)` scalar input `(parameter, offset)`.
    Input(u16, i64),
    /// The length of the vector at an `(R-Inputs)` header input `(parameter, offsets)`.
    Len(u16, Vec<i64>),
    Add(Box<Lin>, Box<Lin>, usize),
    Sub(Box<Lin>, Box<Lin>, usize),
    Mul(Box<Lin>, Box<Lin>, usize),
    Neg(Box<Lin>, usize),
}

impl Lin {
    fn walk(&self, f: &mut impl FnMut(&Lin)) {
        f(self);
        match self {
            Lin::Add(a, b, _) | Lin::Sub(a, b, _) | Lin::Mul(a, b, _) => {
                a.walk(f);
                b.walk(f);
            }
            Lin::Neg(a, _) => a.walk(f),
            _ => {}
        }
    }

    /// Does this expression read a parameter in `varying`?
    #[must_use]
    pub fn varies(&self, varying: &HashSet<u16>) -> bool {
        let mut v = false;
        self.walk(&mut |n| {
            if let Lin::Param(p) = n {
                v |= varying.contains(p);
            }
        });
        v
    }

    /// Linear in the counter: no product of two factors that both vary.
    #[must_use]
    pub fn linear(&self, varying: &HashSet<u16>) -> bool {
        let mut ok = true;
        self.walk(&mut |n| {
            if let Lin::Mul(a, b, _) = n {
                ok &= !(a.varies(varying) && b.varies(varying));
            }
        });
        ok
    }
}

/// One test of the precondition: `lo < hi` (`strict`) or `lo <= hi`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    pub lo: Lin,
    pub hi: Lin,
    pub strict: bool,
}

/// A leaf's fast path and its precondition.
#[derive(Default, Debug)]
pub struct Leaf {
    /// Each decided `if`, by its condition's address, with the value it is decided to.
    pub decided: HashMap<usize, bool>,
    /// The element reads and writes at a proven index, by the `OpGetInt`/`OpSetInt` argument
    /// slice's address.
    pub raw: HashSet<usize>,
    /// The operators the tests evaluate, by argument slice: plain in the fast twin.
    pub plain: HashSet<usize>,
    pub tests: Vec<Test>,
    /// The leaves this one calls, walked inline: their decided `if`s, accesses and plain
    /// operators are in the sets above (each keyed by an address in that callee's IR), and
    /// the C writes each callee's own fast twin from them.
    pub callees: Vec<u32>,
}

impl Leaf {
    /// The distinct leaves the tests read, in first-appearance order: the precondition
    /// function's parameters.
    #[must_use]
    pub fn leaves(&self) -> Vec<Lin> {
        let mut out: Vec<Lin> = Vec::new();
        for t in &self.tests {
            for side in [&t.lo, &t.hi] {
                side.walk(&mut |n| {
                    if matches!(n, Lin::Param(_) | Lin::Input(..) | Lin::Len(..))
                        && !out.contains(n)
                    {
                        out.push(n.clone());
                    }
                });
            }
        }
        out
    }
}

/// What the caller knows of the held-inputs twin: which record fields cross as scalars and
/// which vector paths as headers.
pub struct Inputs<'a> {
    pub scalars: &'a [(u16, i64)],
    pub headers: &'a [(u16, Vec<i64>)],
}

/// What a local holds on the fast path.
#[derive(Clone)]
enum Local {
    Lin(Lin),
    /// The vector at header input `(parameter, offsets)`.
    Header(u16, Vec<i64>),
}

struct Walk<'a> {
    data: &'a Data,
    params: HashSet<u16>,
    inputs: &'a Inputs<'a>,
    env: HashMap<u16, Local>,
    leaf: Leaf,
    /// Does an element access follow the statement being walked, on the path that does not
    /// leave the leaf?  An early exit is decided toward that path.
    follows: bool,
    /// Inside a called leaf: its record parameters, by the caller's record they name.  Its
    /// variables are its own, so a field input is found only through this map there.
    alias: HashMap<u16, u16>,
    callee: bool,
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

/// Plain `integer`: the 64-bit value the writers carry, null its sentinel.
fn plain_integer(tp: &Type) -> bool {
    // `integer?` is refused by name (`@FR-N-Shape`): the guard compares bound VALUES, and a
    // null has none to compare.
    !matches!(tp, Type::Optional(_))
        && matches!(tp.base(), Type::Integer(s) if s.is_signed32_template() || s.is_wide_template())
}

/// The variables an expression assigns anywhere inside it.
fn assigned(v: &Value, out: &mut HashSet<u16>) {
    v.walk(&mut |n| {
        if let Value::Set(x, _) = n {
            out.insert(*x);
        }
    });
}

/// Does `v` read or write a vector element?
/// Does `v` only leave the leaf: a `return`, or a block whose last statement is one?
fn exits(v: &Value) -> bool {
    match v.unspan() {
        Value::Return(_) => true,
        Value::Block(b) => b
            .operators
            .iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
            .is_some_and(exits),
        _ => false,
    }
}

/// Is `v` an arm that does nothing: no `else`, or an empty block?
fn empty_arm(v: &Value) -> bool {
    match v.unspan() {
        Value::Null => true,
        Value::Block(b) => b
            .operators
            .iter()
            .all(|o| matches!(o.unspan(), Value::Line(_))),
        _ => false,
    }
}

fn has_access(data: &Data, v: &Value) -> bool {
    let mut found = false;
    v.walk(&mut |n| {
        if let Value::Call(d, a) = n
            && a.len() == 3
            && matches!(op_name(data, *d), "OpGetVector" | "OpGetVectorNullable")
        {
            found = true;
        }
    });
    found
}

impl Walk<'_> {
    /// `x` as a linear expression on the fast path, or `None`.
    fn lin(&self, x: &Value) -> Option<Lin> {
        match x.unspan() {
            Value::Int(k) => Some(Lin::Const(i64::from(*k))),
            Value::Long(k) if *k != i64::MIN => Some(Lin::Const(*k)),
            Value::Var(v) => {
                if self.params.contains(v) {
                    return Some(Lin::Param(*v));
                }
                match self.env.get(v)? {
                    Local::Lin(l) => Some(l.clone()),
                    Local::Header(..) => None,
                }
            }
            Value::Call(d, args) => {
                let at = args.as_ptr() as usize;
                let two = |w: &Self| -> Option<(Box<Lin>, Box<Lin>)> {
                    Some((Box::new(w.lin(&args[0])?), Box::new(w.lin(&args[1])?)))
                };
                match (op_name(self.data, *d), args.len()) {
                    ("OpAddInt" | "OpAddIntNullable", 2) => {
                        two(self).map(|(a, b)| Lin::Add(a, b, at))
                    }
                    ("OpMinInt" | "OpMinIntNullable", 2) => {
                        two(self).map(|(a, b)| Lin::Sub(a, b, at))
                    }
                    ("OpMulInt" | "OpMulIntNullable", 2) => {
                        two(self).map(|(a, b)| Lin::Mul(a, b, at))
                    }
                    ("OpMinSingleInt", 1) => Some(Lin::Neg(Box::new(self.lin(&args[0])?), at)),
                    ("OpGetInt", 2) => {
                        let Value::Var(p) = args[0].unspan() else {
                            return None;
                        };
                        let Value::Int(fld) = args[1].unspan() else {
                            return None;
                        };
                        let key = (self.root(*p)?, i64::from(*fld));
                        self.inputs
                            .scalars
                            .contains(&key)
                            .then_some(Lin::Input(key.0, key.1))
                    }
                    ("OpLengthVector" | "t_6vector_len", 1) => {
                        let (p, offs) = self.header(&args[0])?;
                        Some(Lin::Len(p, offs))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The header input a vector expression is, on the fast path.
    fn header(&self, x: &Value) -> Option<(u16, Vec<i64>)> {
        match x.unspan() {
            Value::Var(v) => match self.env.get(v)? {
                Local::Header(p, o) => Some((*p, o.clone())),
                Local::Lin(_) => None,
            },
            Value::Call(d, a) if a.len() == 3 && op_name(self.data, *d) == "OpGetField" => {
                let Value::Var(p) = a[0].unspan() else {
                    return None;
                };
                let Value::Int(off) = a[1].unspan() else {
                    return None;
                };
                let key = (self.root(*p)?, vec![i64::from(*off)]);
                self.inputs.headers.contains(&key).then_some(key)
            }
            _ => None,
        }
    }

    /// The tests that make condition `cond` answer `want`, or `None` where it cannot be decided.
    fn tests(&self, cond: &Value, want: bool) -> Option<Vec<Test>> {
        match cond.unspan() {
            Value::Call(d, a) if a.len() == 2 => {
                let (lo, hi) = (self.lin(&a[0])?, self.lin(&a[1])?);
                let t = match (op_name(self.data, *d), want) {
                    ("OpLtInt", true) => Test {
                        lo,
                        hi,
                        strict: true,
                    },
                    ("OpLtInt", false) => Test {
                        lo: hi,
                        hi: lo,
                        strict: false,
                    },
                    ("OpLeInt", true) => Test {
                        lo,
                        hi,
                        strict: false,
                    },
                    ("OpLeInt", false) => Test {
                        lo: hi,
                        hi: lo,
                        strict: true,
                    },
                    ("OpGtInt", true) => Test {
                        lo: hi,
                        hi: lo,
                        strict: true,
                    },
                    ("OpGtInt", false) => Test {
                        lo,
                        hi,
                        strict: false,
                    },
                    ("OpGeInt", true) => Test {
                        lo: hi,
                        hi: lo,
                        strict: false,
                    },
                    ("OpGeInt", false) => Test {
                        lo,
                        hi,
                        strict: true,
                    },
                    _ => return None,
                };
                Some(vec![t])
            }
            Value::Call(d, a) if a.len() == 1 && op_name(self.data, *d) == "OpNot" => {
                self.tests(&a[0], !want)
            }
            // `a or b` decided false: both false.
            Value::If(first, yes, second)
                if matches!(yes.unspan(), Value::Boolean(true)) && !want =>
            {
                let mut both = self.tests(first, false)?;
                both.extend(self.tests(second, false)?);
                Some(both)
            }
            // `a and b` decided true: both true.
            Value::If(first, second, no)
                if matches!(no.unspan(), Value::Boolean(false)) && want =>
            {
                let mut both = self.tests(first, true)?;
                both.extend(self.tests(second, true)?);
                Some(both)
            }
            _ => None,
        }
    }

    /// An element read or write at a linear index of a header input: its tests and its slice.
    fn access(&mut self, name: &str, args: &[Value]) {
        // An integer, float or boolean element field: each has a plain form in the fast twin.
        let elem = match (name, args.len()) {
            ("OpGetInt" | "OpGetFloat" | "OpGetBoolean", 2)
            | ("OpSetInt" | "OpSetFloat" | "OpSetBoolean", 3) => &args[0],
            _ => return,
        };
        let Value::Call(g, ga) = elem.unspan() else {
            return;
        };
        if ga.len() != 3
            || !matches!(
                op_name(self.data, *g),
                "OpGetVector" | "OpGetVectorNullable"
            )
        {
            return;
        }
        if !matches!(ga[1].unspan(), Value::Int(_)) || !matches!(args[1].unspan(), Value::Int(_)) {
            return;
        }
        let (Some((p, offs)), Some(idx)) = (self.header(&ga[0]), self.lin(&ga[2])) else {
            return;
        };
        self.leaf.tests.push(Test {
            lo: Lin::Const(0),
            hi: idx.clone(),
            strict: false,
        });
        self.leaf.tests.push(Test {
            lo: idx,
            hi: Lin::Len(p, offs),
            strict: true,
        });
        self.leaf.raw.insert(args.as_ptr() as usize);
    }

    /// The leaf's own record variable `p` names: itself, or inside a called leaf the caller's
    /// record its parameter was bound to.
    fn root(&self, p: u16) -> Option<u16> {
        if self.callee {
            self.alias.get(&p).copied()
        } else {
            Some(p)
        }
    }

    /// Walk a call of leaf `d` inline — `@FR-R-Version`'s callee clause: its parameters bound
    /// to the caller's arguments (a linear value, a header, or the caller's record), its
    /// decisions and accesses joining this leaf's plan.  The value it answers, as a linear
    /// expression over the caller's terms, or `None`.  One level: a callee calls nothing.
    fn call_leaf(&mut self, d: u32, args: &[Value]) -> Result<Option<Lin>, String> {
        let def = self.data.def(d);
        let name = def.original_name();
        if self.callee {
            return Err(format!("calls {name} from a called leaf"));
        }
        if def.def_type != crate::data::DefType::Function || matches!(def.code(), Value::Null) {
            return Err(format!("calls {name}"));
        }
        let vars = def.variables();
        let mut rebound = HashSet::new();
        assigned(def.code(), &mut rebound);
        let mut env = HashMap::new();
        let mut alias = HashMap::new();
        for (p, arg) in vars.arguments().into_iter().zip(args) {
            if rebound.contains(&p) {
                continue;
            }
            if let Some(l) = self.lin(arg) {
                env.insert(p, Local::Lin(l));
            } else if let Some((r, o)) = self.header(arg) {
                env.insert(p, Local::Header(r, o));
            } else if let Value::Var(v) = arg.unspan()
                && let Some(r) = self.root(*v)
            {
                alias.insert(p, r);
            }
        }
        let saved = (
            std::mem::replace(&mut self.env, env),
            std::mem::take(&mut self.params),
            std::mem::replace(&mut self.alias, alias),
        );
        self.callee = true;
        let r = self.walk(def.code());
        // The value the callee answers: its last statement, or what that returns.
        let value = r.is_ok().then(|| match def.code().unspan() {
            Value::Block(b) => b
                .operators
                .iter()
                .rev()
                .find(|o| !matches!(o.unspan(), Value::Line(_)))
                .and_then(|last| match last.unspan() {
                    Value::Return(e) => self.lin(e),
                    other => self.lin(other),
                }),
            other => self.lin(other),
        });
        self.callee = false;
        (self.env, self.params, self.alias) = saved;
        r?;
        if !self.leaf.callees.contains(&d) {
            self.leaf.callees.push(d);
        }
        Ok(value.flatten())
    }

    /// A statement list in order, each statement knowing whether an access follows it.
    fn walk_list(&mut self, ops: &[Value]) -> Result<(), String> {
        let outer = self.follows;
        for (k, op) in ops.iter().enumerate() {
            self.follows = outer || ops[k + 1..].iter().any(|o| has_access(self.data, o));
            self.walk(op)?;
        }
        self.follows = outer;
        Ok(())
    }

    /// Walk `v` in evaluation order; `Err` where the function is no leaf.
    fn walk(&mut self, v: &Value) -> Result<(), String> {
        match v.unspan() {
            Value::Block(b) => self.walk_list(&b.operators),
            Value::Insert(ops) => self.walk_list(ops),
            Value::Set(x, e) => {
                if let Value::Call(d, args) = e.unspan()
                    && !op_name(self.data, *d).starts_with("Op")
                    && op_name(self.data, *d) != "t_6vector_len"
                {
                    for a in args {
                        self.walk(a)?;
                    }
                    match self.call_leaf(*d, args)? {
                        Some(l) => {
                            self.env.insert(*x, Local::Lin(l));
                        }
                        None => {
                            self.env.remove(x);
                        }
                    }
                    return Ok(());
                }
                self.walk(e)?;
                let local = self
                    .lin(e)
                    .map(Local::Lin)
                    .or_else(|| self.header(e).map(|(p, o)| Local::Header(p, o)));
                match local {
                    Some(l) => {
                        self.env.insert(*x, l);
                    }
                    None => {
                        self.env.remove(x);
                    }
                }
                Ok(())
            }
            Value::Return(e) | Value::Drop(e) => self.walk(e),
            Value::If(c, t, e) => {
                let (at, ae) = (has_access(self.data, t), has_access(self.data, e));
                let want = match (at, ae) {
                    (true, false) => Some(true),
                    (false, true) => Some(false),
                    // An early exit — `if c { return x }` — with an access after it: decided
                    // toward going on, so `!c` joins the precondition and the fast twin skips it.
                    (false, false) if self.follows && exits(t) && empty_arm(e) => Some(false),
                    (false, false) if self.follows && exits(e) && empty_arm(t) => Some(true),
                    _ => None,
                };
                if let Some(want) = want
                    && let Some(tests) = self.tests(c, want)
                {
                    self.leaf.tests.extend(tests);
                    self.leaf
                        .decided
                        .insert(std::ptr::from_ref::<Value>(c) as usize, want);
                    return self.walk(if want { t } else { e });
                }
                self.walk(c)?;
                // Undecided: each arm from the same facts, and what either assigns unknown after.
                let saved = self.env.clone();
                self.walk(t)?;
                self.env = saved.clone();
                self.walk(e)?;
                self.env = saved;
                let mut set = HashSet::new();
                assigned(t, &mut set);
                assigned(e, &mut set);
                for x in set {
                    self.env.remove(&x);
                }
                Ok(())
            }
            Value::Call(d, args) => {
                let name = op_name(self.data, *d);
                if !name.starts_with("Op") && name != "t_6vector_len" {
                    for a in args {
                        self.walk(a)?;
                    }
                    self.call_leaf(*d, args)?;
                    return Ok(());
                }
                for a in args {
                    self.walk(a)?;
                }
                self.access(name, args);
                Ok(())
            }
            Value::Loop(_) | Value::Iter(..) | Value::Yield(_) | Value::Parallel(_) => {
                Err("a loop".into())
            }
            Value::CallRef(..) | Value::Break(_) | Value::Continue(_) => {
                Err("a call through a reference".into())
            }
            _ => Ok(()),
        }
    }
}

/// Leaf `d`'s fast path and precondition, given the twin's `inputs`; `None` where `d` is no
/// leaf, or decides nothing and proves no access.
#[must_use]
pub fn leaf(data: &Data, d: u32, inputs: &Inputs) -> Option<Leaf> {
    if !enabled() || (d as usize) >= data.definitions.len() {
        return None;
    }
    let def = data.def(d);
    let vars = def.variables();
    // A parameter the body assigns does not hold its argument after that assignment.
    let mut rebound = HashSet::new();
    assigned(def.code(), &mut rebound);
    let params: HashSet<u16> = vars
        .arguments()
        .into_iter()
        .filter(|&a| plain_integer(vars.tp(a)) && !rebound.contains(&a))
        .collect();
    let mut w = Walk {
        data,
        params,
        inputs,
        env: HashMap::new(),
        leaf: Leaf::default(),
        follows: false,
        alias: HashMap::new(),
        callee: false,
    };
    let r = w.walk(def.code());
    let name = def.original_name();
    match r {
        Err(why) => {
            if trace() {
                eprintln!("[version] {name}: no leaf: {why}");
            }
            None
        }
        Ok(()) if w.leaf.raw.is_empty() => {
            if trace() {
                eprintln!("[version] {name}: proves no element access");
            }
            None
        }
        Ok(()) => {
            let mut leaf = w.leaf;
            // A test met twice (the decided `idx < len` and the access's own) is asked once.
            let mut seen: Vec<Test> = Vec::new();
            leaf.tests.retain(|t| {
                if seen.contains(t) {
                    false
                } else {
                    seen.push(t.clone());
                    true
                }
            });
            let mut plain = HashSet::new();
            for t in &leaf.tests {
                for side in [&t.lo, &t.hi] {
                    side.walk(&mut |n| {
                        if let Lin::Add(_, _, at)
                        | Lin::Sub(_, _, at)
                        | Lin::Mul(_, _, at)
                        | Lin::Neg(_, at) = n
                        {
                            plain.insert(*at);
                        }
                    });
                }
            }
            leaf.plain = plain;
            if trace() {
                eprintln!(
                    "[version] {name}: leaf, {} decided, {} accesses, {} tests",
                    leaf.decided.len(),
                    leaf.raw.len(),
                    leaf.tests.len()
                );
            }
            Some(leaf)
        }
    }
}
