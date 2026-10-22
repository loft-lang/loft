// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-PureReuse`, THE SHARED-CALL CLAUSE — a TOTAL call that two wrappers make on the same
//! arguments is made once, by their caller, and handed to each wrapper's VARIANT.
//!
//! - [`total`]: the effect-free functions that always return and report nothing: no loop, no
//!   recursion, only operators that read, compare or hand a buffer on, and calls of total
//!   functions or of natives declared `#pure`.  Such a call may be evaluated where the program
//!   would have skipped it (before a short circuit) — it adds work, never an effect or a report.
//! - [`sharing_of`]: a wrapper whose body makes ONE call of a total function returning a vector,
//!   on parameters it never assigns.
//! - [`variant`]: that wrapper with the vector as one more parameter, standing where the call was.
//! - [`hand_in`]: what a call of the total function becomes when its value is handed in — the
//!   vector itself where the call's value is only read and its buffer is a local nothing else
//!   names, otherwise a copy of it into the buffer the call was handed, which is what the call
//!   would have left there.

use crate::data::{AliasFact, Attribute, Block, Data, Deps, Purity, Type, Value};
use crate::database::Stores;
use crate::fxhash::FxHashSet as HashSet;

/// The origin tag of a wrapper's variant: effect-free as the wrapper it copies.
pub const VARIANT: &str = "shared-call variant";

/// `OpEq…`, `OpNe…`, `OpLt…`, `OpLe…` over a value kind — never `OpNewRecord`.
pub(crate) fn comparison(name: &str) -> bool {
    const KINDS: [&str; 7] = ["Bool", "Content", "Float", "Int", "Ref", "Single", "Text"];
    ["OpEq", "OpNe", "OpLt", "OpLe"]
        .iter()
        .any(|p| name.strip_prefix(p).is_some_and(|k| KINDS.contains(&k)))
}

/// Operators a TOTAL body may use: each reads, compares or hands a buffer on, and none can
/// report a fault (checked arithmetic reports an overflow, so it is not here).
fn total_op(name: &str) -> bool {
    const ENVIRONMENT: [&str; 3] = ["OpGetDir", "OpGetFile", "OpGetFileText"];
    (name.starts_with("OpGet") && !ENVIRONMENT.contains(&name))
        || name.starts_with("OpLength")
        || comparison(name)
        || matches!(
            name,
            "OpNot"
                | "OpRefIsNull"
                | "OpClearVector"
                | "OpAdoptVector"
                | "t_6vector_len"
                | "t_4text_len"
        )
}

/// A native whose declaration says its value is its only effect.
pub(crate) fn pure_native(data: &Data, d: u32) -> bool {
    let def = data.def(d);
    matches!(def.code(), Value::Null) && def.purity == Purity::Pure && !def.name().starts_with("Op")
}

/// The effect-free functions of `free` that are TOTAL (the least fixed point: a function joins
/// once everything it calls is total, so a recursive one never does).
pub fn total(data: &Data, free: &HashSet<u32>) -> HashSet<u32> {
    let mut total = HashSet::default();
    let mut order: Vec<u32> = free.iter().copied().collect();
    order.sort_unstable();
    loop {
        let add: Vec<u32> = order
            .iter()
            .copied()
            .filter(|f| !total.contains(f) && body_total(data, *f, &total))
            .collect();
        if add.is_empty() {
            return total;
        }
        total.extend(add);
    }
}

fn body_total(data: &Data, f: u32, total: &HashSet<u32>) -> bool {
    !data.def(f).code().any_node(&mut |n| match n {
        Value::Loop(_) | Value::CallRef(..) | Value::Yield(_) => true,
        Value::Call(d, _) => {
            (*d as usize) >= data.definitions.len()
                || !(total.contains(d) || pure_native(data, *d) || total_op(data.def(*d).name()))
        }
        _ => false,
    })
}

/// The attribute `f` takes its vector result's buffer in.
pub fn vector_buffer(data: &Data, f: u32) -> Option<usize> {
    let def = data.def(f);
    let k = def.hidden_return_buffer_attr()?;
    matches!(def.attributes()[k].typedef.base(), Type::Vector(..)).then_some(k)
}

/// A wrapper's one call of a total function: the function, and for each of its arguments
/// before the buffer, the wrapper parameter it is.
#[derive(Clone)]
pub struct Sharing {
    pub f: u32,
    pub args: Vec<usize>,
}

/// The [`Sharing`] wrapper `w` is, if it is one: effect-free, compiled in this load, and its
/// body one call of a total vector-returning function on its own unassigned parameters, the
/// buffer a local of its own or its own return buffer.
pub fn sharing_of(
    data: &Data,
    w: u32,
    free: &HashSet<u32>,
    total: &HashSet<u32>,
) -> Option<Sharing> {
    let def = data.def(w);
    if !free.contains(&w) || def.variables.done {
        return None;
    }
    let mut calls: Vec<(u32, &[Value])> = Vec::new();
    def.code().walk(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d != w
            && total.contains(d)
            && vector_buffer(data, *d).is_some()
        {
            calls.push((*d, args));
        }
    });
    let (f, args) = calls
        .iter()
        .find(|(f, _)| calls.iter().filter(|(g, _)| g == f).count() == 1)?;
    let kbuf = vector_buffer(data, *f)?;
    let attrs = def.attributes();
    // Attribute `i` is the `i`-th argument variable (a promoted return buffer keeps the
    // placeholder's attribute and name, so a lookup by name finds the retired placeholder).
    let arg_vars = def.variables.arguments();
    if arg_vars.len() != attrs.len() {
        return None;
    }
    let mut pos = Vec::new();
    let mut params = Vec::new();
    for (i, a) in args.iter().enumerate() {
        let Value::Var(x) = a.unspan() else {
            return None;
        };
        if i == kbuf {
            // A local of its own, or its own return buffer: what it fills there is its result.
            let own_result = def
                .hidden_return_buffer_attr()
                .is_some_and(|k| arg_vars[k] == *x);
            if def.variables.is_argument(*x) && !own_result {
                return None;
            }
            continue;
        }
        let p = arg_vars.iter().position(|a| a == x)?;
        if attrs[p].hidden {
            return None;
        }
        pos.push(p);
        params.push(*x);
    }
    if def
        .code()
        .any_node(&mut |n| matches!(n, Value::Set(x, _) if params.contains(x)))
    {
        return None;
    }
    Some(Sharing { f: *f, args: pos })
}

/// The variant of `wrapper`: a copy taking `sharing.f`'s vector as one more parameter, inserted
/// before the hidden buffers, read where the body called `sharing.f`.  Answers the variant and the
/// position of the new parameter.
pub fn variant(
    data: &mut Data,
    db: &mut Stores,
    wrapper: u32,
    sharing: &Sharing,
) -> Option<(u32, usize)> {
    let given_tp = data.def(sharing.f).returned.without_deps();
    let Type::Vector(content, _) = given_tp.base() else {
        return None;
    };
    let tp = data.vector_element_type(content, db)?;
    let kbuf = vector_buffer(data, sharing.f)?;
    // The new parameter goes before the hidden buffers, its variable at that index: every
    // parameter before it must be the variable of its own index (`a == v`).  A hidden buffer
    // after it may be a promoted local anywhere; it keeps its place in `arguments()` order.
    let def = data.def(wrapper);
    let at = def
        .attributes()
        .iter()
        .position(|a| a.hidden)
        .unwrap_or(def.attributes().len());
    let args = def.variables.arguments();
    if args.len() != def.attributes().len() || (0..at).any(|i| args[i] as usize != i) {
        return None;
    }
    let name = format!("{}__given_{}", def.name(), sharing.f);
    let var_fn = data.add_variant_fn(wrapper, &name, VARIANT);
    let given = insert_param(data, var_fn, at, &given_tp);
    let mut code = std::mem::replace(&mut data.definitions[var_fn as usize].code, Value::Null);
    let inner = sharing.f;
    let uses = buffer_uses(std::iter::once(&code));
    hand_in(
        &mut code,
        data,
        &|c: &Value| matches!(c.unspan(), Value::Call(d, _) if *d == inner),
        &Value::Var(given),
        kbuf,
        tp,
        &|b| uses.throwaway(b),
        &given_tp,
    );
    data.definitions[var_fn as usize].code = code;
    Some((var_fn, at))
}

/// Insert parameter `tp` at attribute `k` of `d`, its variable at index `k`: the new variable
/// is bubbled down past every variable from `k` on, the body and the type deps renumbered with
/// it, and attribute-space deps past `k` moved up one.
fn insert_param(data: &mut Data, def_nr: u32, at: usize, tp: &Type) -> u16 {
    let def = &mut data.definitions[def_nr as usize];
    let mut code = std::mem::replace(&mut def.code, Value::Null);
    // The body's own result type is the function's return type: its deps are ATTRIBUTE
    // indices, which the new parameter moves as it moves the attributes' (`shift` below) — not
    // frame variables for the renumber, which read `[1]` (a hidden buffer's attribute) as the
    // variable that happens to hold that number.
    let returned = def.returned.clone();
    let body_result = match &mut code {
        Value::Block(bl) if bl.result == returned && !returned.has_no_deps() => {
            Some(std::mem::replace(&mut bl.result, Type::Void))
        }
        _ => None,
    };
    let vars = &mut def.variables;
    let mut given = vars.add_unique("given", tp, u16::MAX);
    vars.become_argument(given);
    while given as usize > at {
        let (lower, upper) = (given - 1, given);
        let tmp = vars.count();
        code.renumber_frame_var(lower, tmp);
        code.renumber_frame_var(upper, lower);
        code.renumber_frame_var(tmp, upper);
        vars.renumber_frame_in_types(lower, tmp);
        vars.renumber_frame_in_types(upper, lower);
        vars.renumber_frame_in_types(tmp, upper);
        vars.swap_variables(lower, upper);
        given -= 1;
    }
    let shift = |t: &Type| -> Type {
        let deps = t.depend();
        if deps.is_empty() {
            return t.clone();
        }
        let moved: Vec<u16> = deps
            .into_iter()
            .map(|x| if x as usize >= at { x + 1 } else { x })
            .collect();
        t.with_deps(&Deps::attrs(moved))
    };
    if let (Some(r), Value::Block(bl)) = (body_result, &mut code) {
        bl.result = shift(&r);
    }
    def.returned = shift(&def.returned);
    for a in &mut def.attributes {
        a.typedef = shift(&a.typedef);
    }
    // A parameter as the parser declares one: an attribute that is not `mutable` is an
    // operator's CONSTANT operand, which a call does not push — the frame would be short.
    let attr = Attribute {
        name: def.variables.name(given).to_string(),
        typedef: tp.clone(),
        mutable: true,
        constant: false,
        const_field: false,
        value_const: false,
        init: false,
        nullable: true,
        primary: false,
        hidden: false,
        work_buffer: false,
        pub_field: false,
        value: Value::Null,
        check: Value::Null,
        check_message: Value::Null,
        alias_d_nr: u32::MAX,
        fact: AliasFact::Plain,
        assigned_lambda_d_nr: u32::MAX,
        links: Vec::new(),
        lexeme: false,
    };
    def.attributes.insert(at, attr);
    def.attr_names = def
        .attributes
        .iter()
        .enumerate()
        .map(|(i, a)| (a.name.clone(), i))
        .collect();
    def.code = code;
    given
}

/// How often each variable is assigned and read in a body: a call's buffer that is assigned
/// once, to `null`, and read once — by that call — is a THROWAWAY local.
pub struct BufferUses {
    sets: crate::fxhash::FxHashMap<u16, (usize, bool)>,
    reads: crate::fxhash::FxHashMap<u16, usize>,
}

impl BufferUses {
    pub fn throwaway(&self, b: u16) -> bool {
        self.sets.get(&b) == Some(&(1, true)) && self.reads.get(&b) == Some(&1)
    }
}

pub fn buffer_uses<'a>(roots: impl IntoIterator<Item = &'a Value>) -> BufferUses {
    let mut u = BufferUses {
        sets: crate::fxhash::FxHashMap::default(),
        reads: crate::fxhash::FxHashMap::default(),
    };
    for code in roots {
        code.walk(&mut |n| match n {
            Value::Set(x, v) => {
                let e = u.sets.entry(*x).or_insert((0, true));
                e.0 += 1;
                e.1 &= matches!(v.unspan(), Value::Null);
            }
            Value::Var(x) => *u.reads.entry(*x).or_insert(0) += 1,
            _ => {}
        });
    }
    u
}

/// Replace every call `is_site` accepts in `v` by the vector `given` (of type `tp_given`,
/// element database type `tp`): `given` itself where the call is an operand of a read and its
/// buffer (argument `kbuf`) a throwaway local, otherwise `{clear buf; append given; buf}`.
#[allow(clippy::too_many_arguments)]
pub fn hand_in(
    v: &mut Value,
    data: &Data,
    is_site: &dyn Fn(&Value) -> bool,
    given: &Value,
    kbuf: usize,
    tp: u16,
    throwaway: &dyn Fn(u16) -> bool,
    tp_given: &Type,
) {
    hand_in_at(
        v, data, is_site, given, kbuf, tp, throwaway, tp_given, false,
    );
}

#[allow(clippy::too_many_arguments)]
fn hand_in_at(
    v: &mut Value,
    data: &Data,
    is_site: &dyn Fn(&Value) -> bool,
    given: &Value,
    kbuf: usize,
    tp: u16,
    throwaway: &dyn Fn(u16) -> bool,
    tp_given: &Type,
    read: bool,
) {
    if is_site(v) {
        let Value::Call(_, args) = v.unspan() else {
            return;
        };
        let Some(Value::Var(b)) = args.get(kbuf).map(Value::unspan) else {
            return;
        };
        let b = *b;
        *v = if read && throwaway(b) {
            given.clone()
        } else {
            let op = |n: &str| data.def_nr(n);
            Value::Block(Box::new(Block {
                name: "shared_call",
                operators: vec![
                    Value::Call(op("OpClearVector"), vec![Value::Var(b)]),
                    Value::Call(
                        op("OpAppendVector"),
                        vec![Value::Var(b), given.clone(), Value::Int(i32::from(tp))],
                    ),
                    Value::Var(b),
                ],
                result: tp_given.with_deps(&Deps::frame1(b)),
                scope: u16::MAX,
                var_size: 0,
            }))
        };
        return;
    }
    let reads = match v.unspan() {
        Value::Call(d, _) => crate::pure_reuse::reads_only(data.def(*d).name()),
        _ => false,
    };
    v.for_each_child_mut(&mut |c| {
        hand_in_at(
            c, data, is_site, given, kbuf, tp, throwaway, tp_given, reads,
        );
    });
}
