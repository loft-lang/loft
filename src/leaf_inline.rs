// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-InlineLeaf`, `@FR-R-MaskRange`, `@FR-R-SingleUse` and `@FR-R-ScaleFold`, decided in
//! the IR phase for both backends.
//!
//! A call of a SCALAR LEAF is replaced by the leaf's body.  A scalar leaf is a function whose
//! parameters, locals and result are integers, floats, singles, booleans or characters, and
//! whose body is a block of assignments and a result built only from operators, variables
//! and literals — with `if` and nested blocks allowed, a final `return` read as the result,
//! and nothing else: no user call (so it is a `(R-Leaf)`), no loop, no store, no text.  The
//! body is copied over fresh caller locals; an argument that is a literal — or a plain
//! variable, when no argument can write — for a parameter the body never assigns, is read in
//! place instead, and the literal operations that makes adjacent fold.  Arguments keep their order: each non-literal one is bound before the
//! body runs, as the call bound it.
//!
//! The APPENDER clause admits a void leaf whose body appends scalar elements to vector fields
//! of record parameters (`m.f += [e]`, a fused `OpPush…` over `OpGetField(m, …)`) beside
//! scalar assignments, and reads each record for nothing else.  At a call whose record
//! argument is a plain variable holding the record and whose arguments call no user code, the
//! body replaces the call with the record read in place, so the appends sit in the caller's
//! loop where its push windows reach them: one call per element otherwise re-resolved every
//! field's vector from the store on each append.
//!
//! Inside an inlined body, and only there — straight-line code over locals no one else
//! reads — three reductions follow, each answering exactly what it replaces:
//!
//! - `(R-MaskRange)`: `e & m`, `m = 2^k - 1`, is `e` when `e` provably lies in `0 ..= m`, and
//!   `{t = x / c (nullable); if t is not null then t else d}` is `x / c` when `x` converts a
//!   ranged integer and `c` is a finite non-zero literal.  The ranges are `(R-Range)`'s,
//!   carried statement by statement through the body, so a local that reads itself
//!   (`hx = (hx ^ (hx >> 13)) & m`) is ranged at each assignment from the one before.
//! - `(R-SingleUse)`: `x = e;` followed by the body's result reading `x` once, as the first
//!   thing it evaluates, puts `e` in the read.
//! - `(R-ScaleFold)`: `x / c * 2^k` and `x * 2^k / c` are `x / (c / 2^k)` when `x` converts
//!   an integer and `c / 2^k` is exact and normal.
//!
//! Not in an `open_world` program (REPL, debugger, live reload, host), where a definition can
//! change after its calls are compiled, nor in one compiled for a run that observes function
//! entries (`loft test`'s coverage, `Data::observes_entries`), whose report would otherwise
//! name an inlined function as never called.  Switches: `LOFT_NO_INLINE_LEAF=1` (the whole pass),
//! `LOFT_NO_MASK_RANGE=1`, `LOFT_NO_SINGLE_USE=1`, `LOFT_NO_SCALE_FOLD=1`,
//! `LOFT_NO_INLINE_APPENDER=1` (the appender clause);
//! `LOFT_TRACE_INLINE_LEAF=1` names every inlined call and every reduction.
use crate::data::{Block, Data, DefType, Type, Value};
use crate::generation::range::{Range, range, range_vars};
use std::collections::{HashMap, HashSet};

/// A body larger than this many nodes stays a call: it is copied into every call site.
const MAX_NODES: usize = 120;

fn off_all() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_INLINE_LEAF").is_ok_and(|v| v != "0"))
}

fn off_mask() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_MASK_RANGE").is_ok_and(|v| v != "0"))
}

fn off_single() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SINGLE_USE").is_ok_and(|v| v != "0"))
}

fn off_scale() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SCALE_FOLD").is_ok_and(|v| v != "0"))
}

fn off_appender() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_INLINE_APPENDER").is_ok_and(|v| v != "0"))
}

/// An instrument that attributes work to FUNCTIONS is armed (`LOFT_PROFILE`,
/// `LOFT_ALLOC_PATHS`, `LOFT_ALLOC_SITES`, `LOFT_OP_CENSUS`): its report names what the
/// program wrote, so the calls stay calls.
fn profiled() -> bool {
    crate::env_once!(
        [
            "LOFT_PROFILE",
            "LOFT_ALLOC_PATHS",
            "LOFT_ALLOC_SITES",
            "LOFT_OP_CENSUS"
        ]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
    )
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_INLINE_LEAF").is_ok_and(|v| v != "0"))
}

/// The operators the reductions read and write.
struct Ops {
    mul_int: u32,
    add_int: u32,
    min_int: u32,
    eor_int: u32,
    land_int: u32,
    lor_int: u32,
    div_float: u32,
    div_float_nullable: u32,
    mul_float: u32,
    conv_float_from_int: u32,
    conv_bool_from_float: u32,
}

impl Ops {
    fn new(data: &Data) -> Ops {
        Ops {
            mul_int: data.def_nr("OpMulInt"),
            add_int: data.def_nr("OpAddInt"),
            min_int: data.def_nr("OpMinInt"),
            eor_int: data.def_nr("OpEorInt"),
            land_int: data.def_nr("OpLandInt"),
            lor_int: data.def_nr("OpLorInt"),
            div_float: data.def_nr("OpDivFloat"),
            div_float_nullable: data.def_nr("OpDivFloatNullable"),
            mul_float: data.def_nr("OpMulFloat"),
            conv_float_from_int: data.def_nr("OpConvFloatFromInt"),
            conv_bool_from_float: data.def_nr("OpConvBoolFromFloat"),
        }
    }
}

/// An admitted leaf: what its call is replaced by.
struct Leaf {
    name: String,
    /// The parameters in call order, as variable numbers.
    params: Vec<u16>,
    /// Every variable's name and type, by number.
    vars: Vec<(String, Type)>,
    /// The variables the body assigns: never substituted by a literal argument.
    assigned: HashSet<u16>,
    /// The body's assignments, in order, without line markers.
    stmts: Vec<Value>,
    /// The body's result, a final `return` unwrapped.
    tail: Value,
    result: Type,
    /// The appender clause: the record parameters the body appends to, read in place at the
    /// call; empty for a scalar leaf.
    records: Vec<u16>,
    /// The appender clause: a void body of statements with no result to copy.
    appender: bool,
    /// The appender clause: variables the body never reads (a push's spent element
    /// temporary), which get no local in the caller.
    unused: Vec<u16>,
}

fn scalar(tp: &Type) -> bool {
    match tp {
        Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character => true,
        // Deliberately not peeled: a `τ?` admits the null its type allows, which the
        // reductions inside the body would have to prove away; declining keeps the call.
        Type::Optional(_) => false,
        _ => false,
    }
}

fn literal(v: &Value) -> bool {
    matches!(
        v.unspan(),
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_) | Value::Boolean(_)
    )
}

/// Is `v` built only from operators, variables, literals, `if` and blocks of assignments —
/// the shapes a scalar leaf's body may hold?  Counts the nodes into `nodes`.
///
/// Every other shape answers no, and that is the safe direction: a shape this walk does not
/// name might call user code, touch a store or leave the body, and declining only keeps the
/// call.
fn pure(data: &Data, v: &Value, assigned: &mut HashSet<u16>, nodes: &mut usize) -> bool {
    *nodes += 1;
    match v {
        Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Var(_) => true,
        Value::Span(s) => pure(data, &s.1, assigned, nodes),
        Value::Call(op, args) => {
            (*op as usize) < data.definitions.len()
                && data.def(*op).name().starts_with("Op")
                && args.iter().all(|a| pure(data, a, assigned, nodes))
        }
        Value::If(c, t, e) => {
            pure(data, c, assigned, nodes)
                && pure(data, t, assigned, nodes)
                && pure(data, e, assigned, nodes)
        }
        // A tuple literal of pure elements: the tuple clause's result.
        Value::Tuple(items) => items.iter().all(|i| pure(data, i, assigned, nodes)),
        Value::Block(b) => {
            let body: Vec<&Value> = b
                .operators
                .iter()
                .filter(|o| !matches!(o, Value::Line(_)))
                .collect();
            let Some((last, init)) = body.split_last() else {
                return false;
            };
            init.iter().all(|s| statement(data, s, assigned, nodes))
                && pure(data, last, assigned, nodes)
        }
        _ => false,
    }
}

/// An assignment of a pure expression — the only statement a leaf's blocks may hold before
/// their result.
fn statement(data: &Data, v: &Value, assigned: &mut HashSet<u16>, nodes: &mut usize) -> bool {
    let Value::Set(x, e) = v else {
        return false;
    };
    assigned.insert(*x);
    pure(data, e, assigned, nodes)
}

/// The scalar appends the appender clause admits: `OpPush…(OpGetField(r, offset, type), …)`.
const PUSH_OPS: [&str; 7] = [
    "OpPushInt",
    "OpPushFloat",
    "OpPushSingle",
    "OpPushBoolean",
    "OpPushEnum",
    "OpPushCharacter",
    "OpPushByte",
];

/// `@FR-R-InlineLeaf`'s appender clause: one element appended to a vector field of a record
/// parameter, `r.f += [e]` — the push's vector is the field read straight off one of
/// `records`, and every other operand is pure.  The count of `r`'s uses is checked apart
/// ([`record_uses`]), so a record read anywhere else in the body still declines.
fn push(
    data: &Data,
    v: &Value,
    records: &[u16],
    assigned: &mut HashSet<u16>,
    nodes: &mut usize,
) -> bool {
    *nodes += 1;
    let Value::Call(op, args) = v.unspan() else {
        return false;
    };
    if !PUSH_OPS.contains(&data.def(*op).name()) {
        return false;
    }
    let Some((target, rest)) = args.split_first() else {
        return false;
    };
    let Value::Call(get, field) = target.unspan() else {
        return false;
    };
    data.def(*get).name() == "OpGetField"
        && field.len() == 3
        && matches!(field[0].unspan(), Value::Var(r) if records.contains(r))
        && literal(&field[1])
        && literal(&field[2])
        && rest.iter().all(|a| pure(data, a, assigned, nodes))
}

/// How often `r` is read in `v`, and how often as a push's record operand: equal when the
/// body uses the record parameter for nothing but its appends.
fn record_uses(data: &Data, v: &Value, r: u16) -> (usize, usize) {
    let (mut all, mut pushed) = (0, 0);
    v.any_node(&mut |n| {
        if matches!(n, Value::Var(x) if *x == r) {
            all += 1;
        }
        if let Value::Call(op, args) = n
            && PUSH_OPS.contains(&data.def(*op).name())
            && let Some(Value::Call(_, field)) = args.first().map(Value::unspan)
            && matches!(field.first().map(Value::unspan), Some(Value::Var(x)) if *x == r)
        {
            pushed += 1;
        }
        false
    });
    (all, pushed)
}

/// `@FR-R-InlineLeaf`'s appender clause: `admit` for a void body — every statement a scalar
/// assignment or a push onto a record parameter's field, and each record read for nothing else.
fn admit_appender(
    data: &Data,
    def: &crate::data::Definition,
    params: Vec<u16>,
    vars: Vec<(String, Type)>,
    records: Vec<u16>,
    unused: Vec<u16>,
    body: &[&Value],
) -> Result<Leaf, &'static str> {
    let mut assigned = HashSet::new();
    let mut nodes = 0usize;
    for s in body {
        if !statement(data, s, &mut assigned, &mut nodes)
            && !push(data, s, &records, &mut assigned, &mut nodes)
        {
            return Err("an appender statement other than an assignment or a push");
        }
    }
    for &r in &records {
        if assigned.contains(&r) {
            return Err("an appender that rebinds its record");
        }
        let (all, pushed) = record_uses(data, def.code(), r);
        if all != pushed {
            return Err("an appender that reads its record other than to push");
        }
    }
    if nodes > MAX_NODES {
        return Err("a body too large to copy into every call site");
    }
    Ok(Leaf {
        name: def.name().to_string(),
        params,
        vars,
        assigned,
        stmts: body.iter().map(|s| (*s).clone()).collect(),
        tail: Value::Null,
        result: Type::Void,
        records,
        appender: true,
        unused,
    })
}

/// The leaf a call of `d` may be replaced by, or `None` — and why, for the trace.
fn admit(data: &Data, d: u32) -> Result<Leaf, &'static str> {
    let def = data.def(d);
    if def.def_type != DefType::Function {
        return Err("not a function");
    }
    let Value::Block(b) = def.code() else {
        return Err("no loft body");
    };
    // `@FR-R-InlineLeaf`'s tuple clause: a result that is a tuple of scalars, built by
    // tuple literals, travels as the inlined block's value like a scalar does.
    let tuple_of_scalars = match &def.returned {
        Type::Tuple(es) => es.iter().all(scalar),
        // Not peeled: a `τ?` result admits the null the reductions would have to prove away.
        Type::Optional(_) => false,
        _ => false,
    };
    // `@FR-R-InlineLeaf`'s appender clause: a void body appending to record parameters.
    let appender = matches!(def.returned, Type::Void) && !off_appender();
    if !scalar(&def.returned) && !tuple_of_scalars && !appender {
        return Err("a non-scalar result");
    }
    let fv = &def.variables;
    let mut vars = Vec::new();
    let mut records = Vec::new();
    let mut unused = Vec::new();
    for v in 0..fv.count() {
        if fv.is_captured(v) {
            return Err("a captured variable");
        }
        if !scalar(fv.tp(v)) {
            if appender && fv.is_argument(v) && matches!(fv.tp(v), Type::Reference(..)) {
                records.push(v);
            } else if appender
                && !fv.is_argument(v)
                && !def
                    .code()
                    .any_node(&mut |n| matches!(n, Value::Var(x) if *x == v))
            {
                // A temporary the parser declared and the fused push no longer reads
                // (`_elm_1` of `r.f += [e]`): not copied into the caller.
                unused.push(v);
            } else {
                return Err("a non-scalar variable");
            }
        }
        vars.push((fv.name(v).to_string(), fv.tp(v).clone()));
    }
    if appender && records.is_empty() {
        return Err("a void body with no record to append to");
    }
    let mut params = Vec::new();
    for a in &def.attributes {
        let v = fv.var(&a.name);
        if v == u16::MAX || !fv.is_argument(v) {
            return Err("a parameter without its variable");
        }
        params.push(v);
    }
    let body: Vec<&Value> = b
        .operators
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let Some((last, init)) = body.split_last() else {
        return Err("an empty body");
    };
    let mut assigned = HashSet::new();
    let mut nodes = 0usize;
    if appender {
        return admit_appender(data, def, params, vars, records, unused, &body);
    }
    for s in init {
        if !statement(data, s, &mut assigned, &mut nodes) {
            return Err("a statement other than an assignment");
        }
    }
    let tail = match last.unspan() {
        Value::Return(e) => (**e).clone(),
        e => e.clone(),
    };
    if !pure(data, &tail, &mut assigned, &mut nodes) {
        return Err("a result other than operators over variables and literals");
    }
    if nodes > MAX_NODES {
        return Err("a body too large to copy into every call site");
    }
    Ok(Leaf {
        name: def.name().to_string(),
        params,
        vars,
        assigned,
        stmts: init.iter().map(|s| (*s).clone()).collect(),
        tail,
        result: def.returned.clone(),
        records: Vec::new(),
        appender: false,
        unused: Vec::new(),
    })
}

/// A copy of a leaf's node over the caller's names: a variable becomes its fresh local or its
/// literal argument, a nested block takes the call site's scope, a line marker goes (a fault
/// inside the inlined body is reported at the call's line).
fn remap(v: &Value, map: &HashMap<u16, Value>, scope: u16) -> Value {
    match v {
        Value::Var(n) => map[n].clone(),
        Value::Set(n, e) => {
            let Value::Var(t) = map[n] else {
                unreachable!("an assigned variable is never substituted by a literal");
            };
            Value::Set(t, Box::new(remap(e, map, scope)))
        }
        Value::Span(s) => Value::Span(Box::new((s.0, remap(&s.1, map, scope)))),
        Value::Call(op, args) => {
            Value::Call(*op, args.iter().map(|a| remap(a, map, scope)).collect())
        }
        Value::If(c, t, e) => Value::If(
            Box::new(remap(c, map, scope)),
            Box::new(remap(t, map, scope)),
            Box::new(remap(e, map, scope)),
        ),
        Value::Tuple(items) => Value::Tuple(items.iter().map(|i| remap(i, map, scope)).collect()),
        Value::Block(b) => Value::Block(Box::new(Block {
            name: b.name,
            operators: b
                .operators
                .iter()
                .filter(|o| !matches!(o, Value::Line(_)))
                .map(|o| remap(o, map, scope))
                .collect(),
            result: b.result.clone(),
            scope,
            var_size: 0,
        })),
        other => other.clone(),
    }
}

/// The body that replaces a call of `leaf` with `args` at a site in `scope`, and the fresh
/// locals it introduced.
fn expand(
    leaf: &Leaf,
    args: Vec<Value>,
    scope: u16,
    vars: &mut crate::variables::Function,
    fresh: &mut HashSet<u16>,
) -> Value {
    let mut map: HashMap<u16, Value> = HashMap::new();
    let mut ops = Vec::new();
    // A plain variable argument is read in place as well, when no argument can write: the
    // body writes only its own fresh locals, so nothing changes the variable between the
    // call and the read.  An argument with a call in it might write it, and then the call's
    // order — read first, then the argument that writes — is kept by binding.
    let simple = args
        .iter()
        .all(|a| literal(a) || matches!(a.unspan(), Value::Var(_)));
    for (&p, arg) in leaf.params.iter().zip(args) {
        // A record the appender pushes to is the caller's own variable, read in place: the
        // call site admitted only a plain variable, and only pure arguments beside it.
        if leaf.records.contains(&p) {
            map.insert(p, arg.unspan().clone());
            continue;
        }
        let in_place = literal(&arg) || (simple && matches!(arg.unspan(), Value::Var(_)));
        if !leaf.assigned.contains(&p) && in_place {
            map.insert(p, arg.unspan().clone());
        } else {
            let (name, tp) = &leaf.vars[p as usize];
            let t = vars.add_unique(&format!("il_{name}"), tp, scope);
            fresh.insert(t);
            ops.push(Value::Set(t, Box::new(arg)));
            map.insert(p, Value::Var(t));
        }
    }
    for (v, (name, tp)) in leaf.vars.iter().enumerate() {
        let v = v as u16;
        if leaf.unused.contains(&v) {
            continue;
        }
        map.entry(v).or_insert_with(|| {
            let t = vars.add_unique(&format!("il_{name}"), tp, scope);
            fresh.insert(t);
            Value::Var(t)
        });
    }
    for s in &leaf.stmts {
        ops.push(remap(s, &map, scope));
    }
    if !leaf.appender {
        ops.push(remap(&leaf.tail, &map, scope));
    }
    Value::Block(Box::new(Block {
        name: "Inline",
        operators: ops,
        result: leaf.result.clone(),
        scope,
        var_size: 0,
    }))
}

/// `@FR-R-InlineLeaf`'s appender clause at one call site: each record argument a plain
/// variable holding the record itself (not a `&` link, whose variable is not the record), and
/// no argument calling user code — so nothing between the call and its body can rebind the
/// variable the appends now read in place.  Any other site keeps its call.  A scalar leaf
/// passes untouched.
fn appender_site(
    leaf: &Leaf,
    args: &[Value],
    is_op: &[bool],
    vars: &crate::variables::Function,
) -> bool {
    if leaf.records.is_empty() {
        return true;
    }
    let records_plain = leaf.params.iter().zip(args).all(|(p, a)| {
        !leaf.records.contains(p)
            || matches!(a.unspan(), Value::Var(v) if matches!(vars.tp(*v), Type::Reference(..)))
    });
    let call_free = args.iter().all(|a| {
        !a.any_node(&mut |n| {
            matches!(n, Value::Call(d, _) if !is_op.get(*d as usize).copied().unwrap_or(false))
                || !matches!(
                    n,
                    Value::Call(..)
                        | Value::Var(_)
                        | Value::Int(_)
                        | Value::Long(_)
                        | Value::Float(_)
                        | Value::Single(_)
                        | Value::Boolean(_)
                        | Value::Span(_)
                )
        })
    });
    records_plain && call_free
}

/// Replace every call of an admitted leaf in `v`, innermost first (a leaf call in another's
/// argument is expanded before the outer call is).
#[allow(clippy::too_many_arguments)]
fn inline_in(
    v: &mut Value,
    scope: u16,
    caller: u32,
    leaves: &HashMap<u32, Leaf>,
    is_op: &[bool],
    vars: &mut crate::variables::Function,
    fresh: &mut HashSet<u16>,
    n: &mut usize,
) {
    let inner = match v {
        Value::Block(b) | Value::Loop(b) => b.scope,
        // A `Span` is transparent: the block it wraps is visited as its own node next and
        // takes its own scope there.
        Value::Span(_) => scope,
        _ => scope,
    };
    v.for_each_child_mut(&mut |c| inline_in(c, inner, caller, leaves, is_op, vars, fresh, n));
    if let Value::Call(d, args) = v
        && *d != caller
        && let Some(leaf) = leaves.get(d)
        && args.len() == leaf.params.len()
        && appender_site(leaf, args, is_op, vars)
    {
        if trace() {
            eprintln!("inline-leaf: {} into fn {caller}", leaf.name);
        }
        let args = std::mem::take(args);
        *v = expand(leaf, args, scope, vars, fresh);
        *n += 1;
    }
}

/// The literal an integer operator over two literals answers, when it answers one — never on
/// an overflow or a result that is the null sentinel, which the operator itself reports.
fn fold_int(ops: &Ops, op: u32, first: &Value, second: &Value) -> Option<Value> {
    let lit = |node: &Value| match node.unspan() {
        Value::Int(k) => Some(i64::from(*k)),
        Value::Long(l) if *l != i64::MIN => Some(*l),
        _ => None,
    };
    let (lhs, rhs) = (lit(first)?, lit(second)?);
    let folded = if op == ops.mul_int {
        lhs.checked_mul(rhs)?
    } else if op == ops.add_int {
        lhs.checked_add(rhs)?
    } else if op == ops.min_int {
        lhs.checked_sub(rhs)?
    } else if op == ops.eor_int {
        lhs ^ rhs
    } else if op == ops.land_int {
        lhs & rhs
    } else if op == ops.lor_int {
        lhs | rhs
    } else {
        return None;
    };
    if folded == i64::MIN {
        return None;
    }
    Some(i32::try_from(folded).map_or(Value::Long(folded), Value::Int))
}

/// Fold every literal-over-literal integer operation in `v`, innermost first.  Operands are
/// never regrouped: a sentinel test sits on every result, so `(a ^ x) ^ b` and `(a ^ b) ^ x`
/// can disagree when an intermediate is `i64::MIN`.
fn fold(ops: &Ops, v: &mut Value) {
    v.for_each_child_mut(&mut |c| fold(ops, c));
    if let Value::Call(op, args) = v
        && args.len() == 2
        && let Some(r) = fold_int(ops, *op, &args[0], &args[1])
    {
        *v = r;
    }
}

/// What the reductions read and count while they walk one inlined body.
struct Reduce<'a> {
    data: &'a Data,
    ops: &'a Ops,
    nn: &'a std::collections::HashMap<u16, bool>,
    masks: usize,
    singles: usize,
    scales: usize,
}

impl Reduce<'_> {
    fn range(&self, facts: &std::collections::HashMap<u16, Range>, v: &Value) -> Option<Range> {
        range(self.data, self.nn, facts, v, 0)
    }

    /// `(R-MaskRange)` over `v` in evaluation order, `facts` holding each fresh local's range
    /// at this point of the body.
    fn masks(&mut self, v: &mut Value, facts: &mut std::collections::HashMap<u16, Range>) {
        match v {
            Value::Set(x, e) => {
                self.masks(e, facts);
                match self.range(facts, e) {
                    Some(r) => facts.insert(*x, r),
                    None => facts.remove(x),
                };
            }
            Value::Block(_) => {
                if let Some(div) = self.coalesce(v, facts) {
                    *v = div;
                    self.masks += 1;
                    return;
                }
                let Value::Block(b) = v else { unreachable!() };
                for o in &mut b.operators {
                    self.masks(o, facts);
                }
            }
            Value::If(c, t, e) => {
                self.masks(c, facts);
                let mut then_facts = facts.clone();
                self.masks(t, &mut then_facts);
                let mut else_facts = facts.clone();
                self.masks(e, &mut else_facts);
                facts.retain(|k, r| then_facts.get(k) == Some(r) && else_facts.get(k) == Some(r));
            }
            Value::Span(s) => self.masks(&mut s.1, facts),
            Value::Call(op, args) => {
                for a in args.iter_mut() {
                    self.masks(a, facts);
                }
                if *op == self.ops.land_int
                    && args.len() == 2
                    && let Some(keep) = self.redundant_mask(facts, args)
                {
                    *v = std::mem::replace(&mut args[keep], Value::Null);
                    self.masks += 1;
                }
            }
            _ => {}
        }
    }

    /// The operand of `a & m` that the mask leaves unchanged: `m = 2^k - 1` and the operand
    /// lies in `0 ..= m`.  A range inside `0 ..= m` is not enough for any other `m`
    /// (`5 & 6` is `4`).
    fn redundant_mask(
        &self,
        facts: &std::collections::HashMap<u16, Range>,
        args: &[Value],
    ) -> Option<usize> {
        for (m, o) in [(1usize, 0usize), (0, 1)] {
            let mask = match args[m].unspan() {
                Value::Int(k) => i64::from(*k),
                Value::Long(l) => *l,
                _ => continue,
            };
            if mask < 0 || !(mask as u64).wrapping_add(1).is_power_of_two() {
                continue;
            }
            if let Some((lo, hi)) = self.range(facts, &args[o])
                && lo >= 0
                && hi <= mask
            {
                return Some(o);
            }
        }
        None
    }

    /// `{t = x / c (nullable); if t is not null then t else d}` as the plain `x / c`, when `x`
    /// converts a ranged — hence non-sentinel — integer and `c` is a finite non-zero literal:
    /// then the quotient is finite and the fallback is never taken.
    fn coalesce(
        &self,
        node: &Value,
        facts: &std::collections::HashMap<u16, Range>,
    ) -> Option<Value> {
        let Value::Block(block) = node else {
            return None;
        };
        let body: Vec<&Value> = block
            .operators
            .iter()
            .filter(|o| !matches!(o, Value::Line(_)))
            .collect();
        let [Value::Set(tmp, div), Value::If(test, then_arm, _)] = body.as_slice() else {
            return None;
        };
        let Value::Call(div_op, dargs) = div.unspan() else {
            return None;
        };
        if *div_op != self.ops.div_float_nullable || dargs.len() != 2 {
            return None;
        }
        let Value::Call(conv, cargs) = dargs[0].unspan() else {
            return None;
        };
        if *conv != self.ops.conv_float_from_int || cargs.len() != 1 {
            return None;
        }
        let Value::Float(divisor) = dargs[1].unspan() else {
            return None;
        };
        if !divisor.is_finite() || *divisor == 0.0 || self.range(facts, &cargs[0]).is_none() {
            return None;
        }
        let Value::Call(cb, targs) = test.unspan() else {
            return None;
        };
        if *cb != self.ops.conv_bool_from_float
            || targs.len() != 1
            || !matches!(targs[0].unspan(), Value::Var(read) if read == tmp)
            || !matches!(then_arm.unspan(), Value::Var(read) if read == tmp)
        {
            return None;
        }
        Some(Value::Call(
            self.ops.div_float,
            vec![dargs[0].clone(), dargs[1].clone()],
        ))
    }

    /// `(R-SingleUse)` on an inlined body: its last assignment, to a fresh local the result
    /// reads once and FIRST, goes into that read.
    fn single_use(&mut self, b: &mut Block, fresh: &HashSet<u16>) {
        let n = b.operators.len();
        if n < 2 {
            return;
        }
        let Value::Set(x, _) = &b.operators[n - 2] else {
            return;
        };
        let x = *x;
        if !fresh.contains(&x) {
            return;
        }
        let mut events = Vec::new();
        if !evaluation(&b.operators[n - 1], x, &mut events)
            || events.first() != Some(&Event::Read)
            || events.iter().filter(|e| **e == Event::Read).count() != 1
        {
            return;
        }
        let Value::Set(_, e) = b.operators.remove(n - 2) else {
            unreachable!()
        };
        let mut e = Some(*e);
        substitute(&mut b.operators[n - 2], x, &mut e);
        self.singles += 1;
    }

    /// `(R-ScaleFold)` over `v`, innermost first.
    fn scales(&mut self, v: &mut Value) {
        v.for_each_child_mut(&mut |c| self.scales(c));
        if let Some(r) = self.scale(v) {
            *v = r;
            self.scales += 1;
        }
    }

    fn scale(&self, v: &Value) -> Option<Value> {
        let Value::Call(op, args) = v.unspan() else {
            return None;
        };
        if args.len() != 2 {
            return None;
        }
        let float = |v: &Value| match v.unspan() {
            Value::Float(f) => Some(*f),
            _ => None,
        };
        // `x / c * m` (either operand order of the product) or `x * m / c`.
        let (x, c, m) = if *op == self.ops.mul_float {
            let (q, m) = match (float(&args[0]), float(&args[1])) {
                (None, Some(m)) => (&args[0], m),
                (Some(m), None) => (&args[1], m),
                _ => return None,
            };
            let Value::Call(d, dargs) = q.unspan() else {
                return None;
            };
            if *d != self.ops.div_float || dargs.len() != 2 {
                return None;
            }
            (&dargs[0], float(&dargs[1])?, m)
        } else if *op == self.ops.div_float {
            let c = float(&args[1])?;
            let Value::Call(p, pargs) = args[0].unspan() else {
                return None;
            };
            if *p != self.ops.mul_float || pargs.len() != 2 {
                return None;
            }
            match (float(&pargs[0]), float(&pargs[1])) {
                (None, Some(m)) => (&pargs[0], c, m),
                (Some(m), None) => (&pargs[1], c, m),
                _ => return None,
            }
        } else {
            return None;
        };
        let Value::Call(conv, _) = x.unspan() else {
            return None;
        };
        if *conv != self.ops.conv_float_from_int || !scale_folds(c, m) {
            return None;
        }
        Some(Value::Call(
            self.ops.div_float,
            vec![x.clone(), Value::Float(c / m)],
        ))
    }
}

/// `x / c * m` rounds as `x / (c / m)` for every integer conversion `x`: `m` a power of two
/// of at least 2, `c` finite and non-zero with `|c| < 2^1021` (so `x / c` is zero or normal),
/// and `c / m` exact and normal.
fn scale_folds(c: f64, m: f64) -> bool {
    let power_of_two = m >= 2.0 && m.is_normal() && m.to_bits() & ((1u64 << 52) - 1) == 0;
    let q = c / m;
    power_of_two
        && c.is_finite()
        && c != 0.0
        && c.abs() < 2f64.powi(1021)
        && q.is_normal()
        // Exactness, so the BITS must agree — not a tolerance.
        && (q * m).to_bits() == c.to_bits()
}

#[derive(PartialEq, Eq, Debug)]
enum Event {
    Read,
    Op,
}

/// The evaluation of `v` as reads of `x` and completed operators, in order; `false` for a
/// shape the single-use check does not follow (it then declines).
fn evaluation(v: &Value, x: u16, out: &mut Vec<Event>) -> bool {
    match v.unspan() {
        Value::Var(y) => {
            if *y == x {
                out.push(Event::Read);
            }
            true
        }
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_) | Value::Boolean(_) => {
            true
        }
        Value::Call(_, args) => {
            for a in args {
                if !evaluation(a, x, out) {
                    return false;
                }
            }
            out.push(Event::Op);
            true
        }
        _ => false,
    }
}

/// Replace the one read of `x` in `v` by `e`.
fn substitute(v: &mut Value, x: u16, e: &mut Option<Value>) {
    if matches!(v, Value::Var(y) if *y == x) {
        if let Some(e) = e.take() {
            *v = e;
        }
        return;
    }
    v.for_each_child_mut(&mut |c| substitute(c, x, e));
}

/// Run the reductions on every inlined body in `v`.
fn reduce_in(
    v: &mut Value,
    r: &mut Reduce,
    base: &std::collections::HashMap<u16, Range>,
    fresh: &HashSet<u16>,
) {
    v.for_each_child_mut(&mut |c| reduce_in(c, r, base, fresh));
    let Value::Block(b) = v else {
        return;
    };
    if b.name != "Inline" {
        return;
    }
    for o in &mut b.operators {
        fold(r.ops, o);
    }
    if !off_mask() {
        let mut facts = base.clone();
        for o in &mut b.operators {
            r.masks(o, &mut facts);
        }
    }
    if !off_single() {
        r.single_use(b, fresh);
    }
    if !off_scale() {
        for o in &mut b.operators {
            r.scales(o);
        }
    }
}

/// Inline every admitted leaf call and reduce the bodies; answers how many calls were
/// inlined.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn rewrite_program(data: &mut Data) -> usize {
    if off_all() || data.open_world || data.observes_entries || profiled() {
        return 0;
    }
    let mut leaves: HashMap<u32, Leaf> = HashMap::new();
    for d in 0..data.definitions() {
        if data.def(d).def_type != DefType::Function {
            continue;
        }
        match admit(data, d) {
            Ok(l) => {
                leaves.insert(d, l);
            }
            Err(why) if trace() && data.def(d).name().starts_with("n_") => {
                eprintln!("inline-leaf: {} declined: {why}", data.def(d).name());
            }
            Err(_) => {}
        }
    }
    if leaves.is_empty() {
        return 0;
    }
    let ops = Ops::new(data);
    let is_op: Vec<bool> = (0..data.definitions())
        .map(|d| data.def(d).name().starts_with("Op"))
        .collect();
    let (mut calls, mut masks, mut singles, mut scales) = (0, 0, 0, 0);
    for caller in 0..data.definitions() {
        let def = data.def(caller);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        let blocked = def
            .code()
            .any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_)));
        let calls_a_leaf = def.code().any_node(
            &mut |n| matches!(n, Value::Call(d, _) if *d != caller && leaves.contains_key(d)),
        );
        if blocked || !calls_a_leaf {
            continue;
        }
        let _census_body = crate::rewrite_census::InBody::enter("ir", data.def(caller).name());
        let mut code = std::mem::replace(&mut data.definitions[caller as usize].code, Value::Null);
        let mut fresh = HashSet::new();
        let mut n = 0usize;
        let top = match &code {
            Value::Block(b) => b.scope,
            _ => 0,
        };
        inline_in(
            &mut code,
            top,
            caller,
            &leaves,
            &is_op,
            &mut data.definitions[caller as usize].variables,
            &mut fresh,
            &mut n,
        );
        if n > 0 {
            let nn = crate::generation::non_sentinel::non_sentinel_vars(data, &code);
            let base = range_vars(
                data,
                &data.definitions[caller as usize].variables,
                &code,
                &nn,
                &std::collections::BTreeMap::new(),
                &crate::generation::range::Ranges::default(),
            )
            .vars;
            let mut r = Reduce {
                data,
                ops: &ops,
                nn: &nn,
                masks: 0,
                singles: 0,
                scales: 0,
            };
            reduce_in(&mut code, &mut r, &base, &fresh);
            if trace() {
                eprintln!(
                    "inline-leaf: fn {} — {n} call(s) inlined, {} mask/fallback(s), {} single use(s), {} scale fold(s)",
                    data.def(caller).name(),
                    r.masks,
                    r.singles,
                    r.scales
                );
            }
            masks += r.masks;
            singles += r.singles;
            scales += r.scales;
            calls += n;
        }
        data.definitions[caller as usize].code = code;
        if n > 0 {
            data.definitions[caller as usize]
                .variables
                .reset_intervals();
            crate::scopes::compute_function_intervals(data, caller);
            crate::scopes::assign_function_slots(data, caller);
        }
    }
    crate::rewrite_census::fired("R-InlineLeaf", calls);
    crate::rewrite_census::fired("R-MaskRange", masks);
    crate::rewrite_census::fired("R-SingleUse", singles);
    crate::rewrite_census::fired("R-ScaleFold", scales);
    calls
}

#[cfg(test)]
mod tests {
    use super::{Ops, Value, fold_int, scale_folds};

    fn ops() -> Ops {
        Ops {
            mul_int: 1,
            add_int: 2,
            min_int: 3,
            eor_int: 4,
            land_int: 5,
            lor_int: 6,
            div_float: 7,
            div_float_nullable: 8,
            mul_float: 9,
            conv_float_from_int: 10,
            conv_bool_from_float: 11,
        }
    }

    #[test]
    fn a_literal_fold_never_answers_an_overflow_or_the_sentinel() {
        let o = ops();
        assert_eq!(
            fold_int(&o, 1, &Value::Int(7), &Value::Int(83_492_791)),
            Some(Value::Int(584_449_537))
        );
        assert_eq!(
            fold_int(&o, 1, &Value::Long(1 << 62), &Value::Int(4)),
            None,
            "an overflow stays the operator's"
        );
        assert_eq!(
            fold_int(&o, 2, &Value::Long(i64::MAX), &Value::Int(1)),
            None
        );
        assert_eq!(
            fold_int(&o, 3, &Value::Long(i64::MIN + 1), &Value::Int(1)),
            None,
            "a result that is the sentinel"
        );
        assert_eq!(
            fold_int(&o, 4, &Value::Long(i64::MIN), &Value::Int(0)),
            None,
            "a null operand"
        );
        assert_eq!(
            fold_int(&o, 5, &Value::Int(5), &Value::Int(6)),
            Some(Value::Int(4))
        );
        assert_eq!(
            fold_int(&o, 1, &Value::Long(1 << 40), &Value::Int(2)),
            Some(Value::Long(1 << 41))
        );
    }

    #[test]
    fn a_power_of_two_scale_folds_only_where_both_forms_round_alike() {
        assert!(scale_folds(4_294_967_295.0, 2.0));
        assert!(scale_folds(7.0, 4.0));
        assert!(!scale_folds(7.0, 3.0), "3 is no power of two");
        assert!(!scale_folds(7.0, 1.0), "1 scales nothing");
        assert!(!scale_folds(1e-308, 2.0), "a subnormal divisor's half");
        assert!(!scale_folds(f64::MAX, 2.0), "a divisor past 2^1021");
        assert!(!scale_folds(0.0, 2.0) && !scale_folds(f64::NAN, 2.0));
    }
}
