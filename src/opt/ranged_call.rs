// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-RangedCall`'s facts, which every writer reads: whether a call of a function repays a
//! ranged variant (the gain test), whether a call's integer arguments are proven within
//! `[-L, L]`, and the facts a ranged variant's body runs under (its integer parameters seeded
//! to `[-L, L]`).  The writers keep only their spelling — native's `__rg` variants and guarded
//! copy (`generation::ranged_call`), the C's `__rg` twin (`c::RG_BIT`).  `LOFT_NO_RANGED_CALLS=1`
//! keeps every template on both (`range::ranged_calls_enabled`).

use crate::data::{Block, Data, Type, Value};
use crate::opt::range::{self, Range, Ranges};
use std::collections::{HashMap, HashSet};

/// The bound a ranged variant's integer parameters and scalar inputs are assumed within.
/// `2^30` leaves room for the products a grid index takes (`(2L+1)·L·3 < 2^63`).
pub const L: i64 = 1 << 30;

/// The checked operators a callee must run for its ranged variant to be worth emitting.
const MIN_CHECKED_OPS: usize = 3;

/// Is `r` within `[-b, b]`?
#[must_use]
pub fn within(r: Range, b: i64) -> bool {
    r.0 >= -b && r.1 <= b
}

fn is_integer(tp: &Type) -> bool {
    matches!(tp.base(), Type::Integer(_))
}

/// The checked integer operators a call of `d` runs — its own and its callees', three deep —
/// which a ranged variant may turn plain.
fn checked_ops(data: &Data, d: u32, depth: u8) -> usize {
    if depth > 3 || (d as usize) >= data.definitions.len() {
        return 0;
    }
    let def = data.def(d);
    if !def.is_loft_defined() || matches!(def.returned().base(), Type::Iterator(_, _)) {
        return 0;
    }
    let mut n = 0;
    def.code().walk(&mut |c| {
        let Value::Call(c, _) = c else { return };
        if (*c as usize) >= data.definitions.len() {
            return;
        }
        let cd = data.def(*c);
        if matches!(
            cd.name(),
            "OpAddInt"
                | "OpMinInt"
                | "OpMulInt"
                | "OpMinSingleInt"
                | "OpAddIntNullable"
                | "OpMinIntNullable"
                | "OpMulIntNullable"
        ) {
            n += 1;
        } else if cd.is_loft_defined() && *c != d {
            n += checked_ops(data, *c, depth + 1);
        }
    });
    n
}

/// Does a call of `d` repay a ranged variant — and, in a guarded copy, the duplicated loop?
/// The guard and the copy cost per loop and per code byte; a callee with one or two checked
/// operators saves too little to repay them (`(R-GuardedChain)` measured the same trade:
/// one-operator loops LOST 18–50 %).  Three or more reachable operators qualify.
#[must_use]
pub fn benefits(data: &Data, d: u32) -> bool {
    checked_ops(data, d, 0) >= MIN_CHECKED_OPS
}

/// Are `d`'s integer arguments `args` all proven within `[-L, L]` under `facts` — at least one
/// of them?
#[must_use]
pub fn args_ranged(
    data: &Data,
    d: u32,
    args: &[Value],
    facts: &Ranges,
    nn: &HashMap<u16, bool>,
) -> bool {
    let attrs = data.def(d).attributes();
    if attrs.len() < args.len() {
        return false;
    }
    let mut any = false;
    for (at, arg) in attrs.iter().zip(args) {
        if !is_integer(&at.typedef) {
            continue;
        }
        match range::range(data, nn, facts, arg, 0) {
            Some(r) if within(r, L) => any = true,
            _ => return false,
        }
    }
    any
}

/// The seeds of `d`'s ranged variant: each visible integer parameter within `[-L, L]`.
#[must_use]
pub fn param_seeds(data: &Data, d: u32) -> Ranges {
    let def = data.def(d);
    let vars = def.variables();
    let mut seeds = Ranges::default();
    for at in def.attributes() {
        let v = vars.var(&at.name);
        if v != u16::MAX && is_integer(vars.tp(v)) && !at.hidden {
            seeds.vars.insert(v, (-L, L));
        }
    }
    seeds
}

/// The facts `d`'s body runs under from `seeds` (`range_vars` drops the seed of a parameter
/// the body assigns).
#[must_use]
pub fn ranges_under(data: &Data, d: u32, seeds: &Ranges) -> Ranges {
    let def = data.def(d);
    let nn = crate::opt::non_sentinel::non_sentinel_vars(data, def.variables(), def.code());
    range::range_vars(
        data,
        def.variables(),
        def.code(),
        &nn,
        &std::collections::BTreeMap::new(),
        seeds,
    )
}

/// Does `d`'s ranged variant repay its copy of the body: does it turn at least
/// `MIN_CHECKED_OPS` operators plain that its plain body keeps checked ([`gained_ops`])?  The
/// threshold is [`benefits`]' trade read on what the variant SAVES, not on what the callee
/// runs: a variant that plains one subtraction of a long body still doubles that body, and the
/// second copy costs its callees their called-once inlining (text2d's `atlas_cell` twin, one
/// operator, ran the hot base 17 % slower with `blit_glyph` left out of line).  A variant equal
/// to its plain body only duplicates code: a function whose arithmetic is all in the apart
/// variants it calls (`__apv`) has nothing for the ranges to change.  Memoised in `memo`.
pub fn variant_gains(data: &Data, d: u32, depth: u8, memo: &mut HashMap<u32, bool>) -> bool {
    if let Some(g) = memo.get(&d) {
        return *g;
    }
    let gain = gained_ops(data, d, depth, &mut HashMap::new()) >= MIN_CHECKED_OPS;
    memo.insert(d, gain);
    gain
}

/// The operators `d`'s ranged variant turns plain that its plain body keeps checked: its own,
/// under its parameters seeded `[-L, L]`, and for each call it newly ranges, that callee's
/// variant's in turn (three deep).  Memoised in `memo`; a recursive function meets itself as
/// no gain.
fn gained_ops(data: &Data, d: u32, depth: u8, memo: &mut HashMap<u32, usize>) -> usize {
    if depth > 3 || !benefits(data, d) {
        return 0;
    }
    if let Some(g) = memo.get(&d) {
        return *g;
    }
    memo.insert(d, 0);
    let def = data.def(d);
    let code = def.code();
    let seeded = ranges_under(data, d, &param_seeds(data, d));
    let nn = crate::opt::non_sentinel::non_sentinel_vars(data, def.variables(), code);
    let plain = range::range_vars(
        data,
        def.variables(),
        code,
        &nn,
        &std::collections::BTreeMap::new(),
        &Ranges::default(),
    );
    let mut gained = 0;
    let mut calls: Vec<u32> = Vec::new();
    code.walk(&mut |n| {
        let Value::Call(c, args) = n else { return };
        if (*c as usize) >= data.definitions.len() {
            return;
        }
        let name = data.def(*c).name();
        if range::plain_form(name, args.len()).is_some() {
            if range::op_range(data, &nn, &seeded, name, args, 0).is_some()
                && range::op_range(data, &nn, &plain, name, args, 0).is_none()
            {
                gained += 1;
            }
        } else if *c != d
            && data.def(*c).is_loft_defined()
            && args_ranged(data, *c, args, &seeded, &nn)
            && !args_ranged(data, *c, args, &plain, &nn)
        {
            calls.push(*c);
        }
    });
    for c in calls {
        gained += gained_ops(data, c, depth + 1, memo);
    }
    memo.insert(d, gained);
    gained
}

/// The bound a guarded copy tests each leaf against: `G = 2^20` leaves `L` room for the sums a
/// guarded copy forms before it calls.
pub const G: i64 = 1 << 20;

/// `LOFT_NO_RANGED_CALL_VARS=1` — the variable clause off: a guard takes hoisted record fields
/// only, as before it.  The bisect step for a wrong answer in a window walk.
fn variable_clause_enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_RANGED_CALL_VARS").is_ok_and(|v| v != "0"))
}

/// A hoisted integer field the guard may test: its key (variable, offset) and its read.
pub type GuardField = ((u16, i64), Value);

/// The facts a writer hands [`guard_plan`] about the function it writes.
pub struct GuardFacts<'a> {
    /// The function being written.
    pub f: u32,
    /// The facts the block runs under unguarded (a writer's current ranges).
    pub known: &'a Ranges,
    /// The function's non-sentinel facts.
    pub nn: &'a HashMap<u16, bool>,
    /// The function's character walks (`range_vars`' walks).
    pub walks: &'a std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
}

/// `@FR-R-RangedCall`'s guarded copy, decided once for every writer: for a counted `For` block
/// `bl` whose loop hoists the integer record scalars `fields` (the writer's own loop hoist),
/// the leaves its guard tests (`|x| <= G` each) and the facts its guarded copy runs under —
/// or `None` when the copy would make no call newly ranged, or would leave a call doing
/// checked work checked (the copy is then not worth its duplicated loop).  The writer spells
/// the test and writes the block twice.
pub fn guard_plan(
    data: &Data,
    bl: &Block,
    mut fields: Vec<GuardField>,
    facts: &GuardFacts,
    memo: &mut HashMap<u32, bool>,
) -> Option<(Vec<GuardField>, Vec<u16>, Ranges)> {
    let lp = bl.operators.iter().find_map(|op| match op.unspan() {
        Value::Loop(lp) => Some(lp),
        _ => None,
    })?;
    prelude_fields(data, bl, lp, &mut fields);
    let block_code = Value::Block(Box::new(bl.clone()));
    let all_leaves = variable_leaves(data, facts, &block_code);
    let trace = std::env::var("LOFT_TRACE_RANGED_CALL").is_ok();
    let fname = data.def(facts.f).name();
    if fields.is_empty() && all_leaves.is_empty() {
        if trace {
            eprintln!(
                "ranged-call: {fname} For block {} declines: no hoisted integer field or invariant integer variable",
                bl.scope
            );
        }
        return None;
    }
    // The fewest leaves that make a call NEWLY ranged.  Each leaf is a test that can fail at
    // run time and send every pass to the checked copy, and a copy whose calls already take
    // their ranged variants unguarded only duplicates the loop: fields alone first, then the
    // variables with no range, and only then the ones a wider fact bounds.
    let unranged: Vec<u16> = all_leaves
        .iter()
        .copied()
        .filter(|v| !facts.known.vars.contains_key(v))
        .collect();
    let tiers: [&[u16]; 3] = [&[], &unranged, &all_leaves];
    let mut chosen: Option<(Vec<u16>, Ranges)> = None;
    for (t, leaves) in tiers.iter().enumerate() {
        if (t == 0 && fields.is_empty()) || (t > 0 && leaves.len() == tiers[t - 1].len()) {
            continue;
        }
        let mut seeds = Ranges::default();
        for (key, _) in &fields {
            seeds.fields.insert(*key, (-G, G));
        }
        for v in *leaves {
            seeds.vars.insert(*v, (-G, G));
        }
        let merged = guarded_facts(data, facts, bl, &block_code, &seeds, leaves);
        if copy_gains(data, facts, &block_code, &merged, memo) {
            chosen = Some((leaves.to_vec(), merged));
            break;
        }
    }
    let vars = data.def(facts.f).variables();
    let Some((leaves, merged)) = chosen else {
        if trace {
            eprintln!(
                "ranged-call: {fname} For block {} declines: no call gains (fields {:?}; variables {:?})",
                bl.scope,
                fields.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
                all_leaves.iter().map(|v| vars.name(*v)).collect::<Vec<_>>()
            );
        }
        return None;
    };
    if trace && !leaves.is_empty() {
        eprintln!(
            "ranged-call: {fname} For block {} guards variables {:?}",
            bl.scope,
            leaves.iter().map(|v| vars.name(*v)).collect::<Vec<_>>()
        );
    }
    Some((fields, leaves, merged))
}

/// Does the guarded copy gain: some call in `block` takes a ranged variant under the copy's
/// facts `copy` and would not under the facts the block runs under unguarded, and no call
/// doing checked integer work is left checked in the copy?  A call left checked is duplicated
/// for nothing, and the doubled body is what costs a hot function its inlining
/// (`slope_path_with_undo` lost 7 % to a copy that ranged three small `lerp_int` calls beside
/// an unranged `set_height_with_undo`).
fn copy_gains(
    data: &Data,
    facts: &GuardFacts,
    block: &Value,
    copy: &Ranges,
    memo: &mut HashMap<u32, bool>,
) -> bool {
    let mut candidates: Vec<u32> = Vec::new();
    let mut left_checked: Option<u32> = None;
    block.walk(&mut |n| {
        if let Value::Call(d, args) = n
            && benefits(data, *d)
        {
            if !args_ranged(data, *d, args, copy, facts.nn) {
                left_checked.get_or_insert(*d);
            } else if !args_ranged(data, *d, args, facts.known, facts.nn) {
                candidates.push(*d);
            }
        }
    });
    if let Some(d) = left_checked
        && std::env::var("LOFT_TRACE_RANGED_CALL").is_ok()
    {
        eprintln!(
            "ranged-call: {} copy leaves {} checked",
            data.def(facts.f).name(),
            data.def(d).name()
        );
    }
    left_checked.is_none()
        && candidates
            .into_iter()
            .any(|d| variant_gains(data, d, 0, memo))
}

/// The variables written anywhere in `code` outside the block `skip` (by address).
fn sets_outside(code: &Value, skip: *const Block, out: &mut HashSet<u16>) {
    let code = code.unspan();
    if let Value::Block(b) | Value::Loop(b) = code
        && std::ptr::eq(std::ptr::from_ref::<Block>(b), skip)
    {
        return;
    }
    if let Value::Set(v, _) | Value::TuplePut(v, _, _) = code {
        out.insert(*v);
    }
    code.for_each_child(&mut |c| sets_outside(c, skip, out));
}

/// The facts the guarded copy of `bl` runs under: the function's own, the copy's facts for the
/// locals it alone assigns (never handed out by reference), the seeded fields, and each
/// variable leaf at the guard's bound — which the block never changes, and which is the
/// tighter fact where a wider one was known.
fn guarded_facts(
    data: &Data,
    facts: &GuardFacts,
    bl: &Block,
    block_code: &Value,
    seeds: &Ranges,
    leaves: &[u16],
) -> Ranges {
    let vars = data.def(facts.f).variables();
    let copy = range::range_vars(data, vars, block_code, facts.nn, facts.walks, seeds);
    let code = data.def(facts.f).code();
    let mut outside: HashSet<u16> = HashSet::new();
    sets_outside(code, std::ptr::from_ref(bl), &mut outside);
    let mut escaped: HashSet<u16> = HashSet::new();
    crate::opt::non_sentinel::collect_escapes(data, code, &mut escaped);
    let mut merged = facts.known.clone();
    // Both facts hold in the copy — the function's and the copy's own, derived under the guard
    // — so a variable both range keeps their INTERSECTION: inside a ranged variant the
    // function's counters are only `[-L, 2L]` wide, and the copy's are what it is for.
    let meet = |a: (i64, i64), b: (i64, i64)| (a.0.max(b.0), a.1.min(b.1));
    for (v, r) in copy.vars {
        if !outside.contains(&v) && !escaped.contains(&v) {
            merged
                .vars
                .entry(v)
                .and_modify(|k| *k = meet(*k, r))
                .or_insert(r);
        }
    }
    for (key, r) in copy.elems {
        if !outside.contains(&key.0) && !escaped.contains(&key.0) {
            merged
                .elems
                .entry(key)
                .and_modify(|k| *k = meet(*k, r))
                .or_insert(r);
        }
    }
    merged.fields.extend(seeds.fields.clone());
    for v in leaves {
        merged.vars.insert(*v, (-G, G));
    }
    merged
}

/// `@FR-R-RangedCall`'s variable clause — the integer variables `block` reads but never assigns
/// and that the function never hands out by reference, and that the facts do not already bound
/// within `G`: a ranged variant's parameter is seeded only to `[-L, L]`, whose sums are too
/// wide to call a variant with, and the guard tightens it.  Each holds one value for the whole
/// block, so a guard at its entry bounds it everywhere inside.  A null one fails that guard
/// (`i64::MIN`'s magnitude is 2^63), so its nullability costs nothing.  Only a variable a
/// counted range's start or end is built from: a window walk's bounds, which is what the guard
/// pays for itself on (`c_slope_path` lost 7 % to a guard on a salt).
fn variable_leaves(data: &Data, facts: &GuardFacts, block: &Value) -> Vec<u16> {
    if !variable_clause_enabled() {
        return Vec::new();
    }
    let def = data.def(facts.f);
    let vars = def.variables();
    let mut written: HashSet<u16> = HashSet::new();
    block.walk(&mut |n| {
        if let Value::Set(v, _) | Value::TuplePut(v, _, _) = n {
            written.insert(*v);
        }
    });
    let mut escaped: HashSet<u16> = HashSet::new();
    crate::opt::non_sentinel::collect_escapes(data, def.code(), &mut escaped);
    let mut bounds: Vec<&Value> = Vec::new();
    block.walk(&mut |n| {
        if let Value::Set(t, e) = n
            && (vars.name(*t).starts_with("_range_start")
                || vars.name(*t).starts_with("_range_end"))
        {
            bounds.push(e);
        }
    });
    let mut leaves: Vec<u16> = Vec::new();
    for e in bounds {
        e.walk(&mut |n| {
            if let Value::Var(v) = n
                && is_integer(vars.tp(*v))
                && !written.contains(v)
                && !escaped.contains(v)
                && facts.known.vars.get(v).is_none_or(|r| !within(*r, G))
                && !leaves.contains(v)
            {
                leaves.push(*v);
            }
        });
    }
    leaves
}

/// The block's PRELUDE reads too — a non-literal range end is taken into a local before the
/// loop (`@FR-I-Range`).  Seeding the field is sound only where the seed stays true: the read
/// runs right after the guard (the prelude calls no user function that could write it first),
/// and inside the loop the field is either hoisted (invariant, already in `fields`) or not read
/// at all.
fn prelude_fields(data: &Data, bl: &Block, lp: &Block, fields: &mut Vec<GuardField>) {
    let Some(at_loop) = bl
        .operators
        .iter()
        .position(|op| matches!(op.unspan(), Value::Loop(_)))
    else {
        return;
    };
    let prelude = &bl.operators[..at_loop];
    let calls_user = prelude.iter().any(|op| {
        op.any_node(&mut |n| {
            matches!(n, Value::Call(c, _) if (*c as usize) < data.definitions.len()
                && data.def(*c).is_loft_defined())
        })
    });
    if calls_user {
        return;
    }
    let read_of = |n: &Value| -> Option<(u16, i64)> {
        if let Value::Call(g, args) = n
            && data.def(*g).name() == "OpGetInt"
            && let (Some(Value::Var(v)), Some(Value::Int(off))) = (
                args.first().map(Value::unspan),
                args.get(1).map(Value::unspan),
            )
        {
            Some((*v, i64::from(*off)))
        } else {
            None
        }
    };
    let mut extra: Vec<GuardField> = Vec::new();
    for op in prelude {
        op.walk(&mut |n| {
            if let Some(key) = read_of(n)
                && !fields.iter().any(|(k, _)| *k == key)
                && !extra.iter().any(|(k, _)| *k == key)
            {
                extra.push((key, n.clone()));
            }
        });
    }
    for (key, getter) in extra {
        let read_in_loop = lp
            .operators
            .iter()
            .any(|op| op.any_node(&mut |n| read_of(n) == Some(key)));
        if !read_in_loop {
            fields.push((key, getter));
        }
    }
}

/// For a call of an `__inv` twin: is every scalar input the twin takes (`hoist::CalleeInputs`'
/// `(parameter, field, getter)`) proven within `[-L, L]` under the caller's `facts`?  An
/// argument that is a variable keys its field directly; any other must be a pure path
/// (`hoist::path_scalar`).
///
/// # Errors
/// The first input that is not proven, as its `(parameter or variable, field)` key, for a
/// trace.
pub fn scalar_inputs_ranged(
    data: &Data,
    vars: &crate::variables::Function,
    args: &[Value],
    scalars: &[(u16, i64, Value)],
    facts: &Ranges,
) -> Result<(), (u16, i64)> {
    for (p, fld, _) in scalars {
        let Some(arg) = args.get(*p as usize) else {
            return Err((*p, *fld));
        };
        let key = match arg.unspan() {
            Value::Var(c) => (*c, *fld),
            _ => match crate::opt::hoist::path_scalar(data, vars, arg, *fld) {
                Some((k, _)) => k,
                None => return Err((*p, *fld)),
            },
        };
        match facts.fields.get(&key) {
            Some(r) if within(*r, L) => {}
            _ => return Err(key),
        }
    }
    Ok(())
}

/// The seeds of an `__inv` twin's ranged variant: its parameters' ([`param_seeds`]) and each
/// scalar input field within `[-L, L]` — what [`scalar_inputs_ranged`] proved at the call.
#[must_use]
pub fn twin_seeds(data: &Data, d: u32, scalars: &[(u16, i64, Value)]) -> Ranges {
    let mut seeds = param_seeds(data, d);
    for (p, fld, _) in scalars {
        seeds.fields.insert((*p, *fld), (-L, L));
    }
    seeds
}
