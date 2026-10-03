// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `(R-ForwardResult)` — a returned local bound from a call is built in the return buffer.
//!
//! The encoder shape: `buf = head(2, len(value)); buf += value; buf` as one arm of a `match`
//! whose other arms return `head(…)` directly.  Those arms already hand the call this
//! function's own return buffer; the bound arm hands it a hidden `__ref_N` buffer minted for
//! the call, appends into it, and then the delivery (`OpClearVector(rb)`,
//! `OpAppendVector(rb, buf)`) copies the whole vector into the return buffer and the scope
//! exit frees `__ref_N` — one store cycle and one full copy per call, for a value that only
//! ever ends up in the return buffer.  Handing the call the return buffer instead makes `buf`
//! that buffer: the appends land where the result belongs, the delivery pair is the identity
//! and goes, and `__ref_N` is never minted, so its frees are no-ops on a null handle.
//!
//! Admission, per block: the block binds a vector local `L = f(…, __ref_N)` from a call that
//! delivers into its hidden buffer argument (a one-buffer return, never a borrowed view),
//! later delivers `OpClearVector(rb); OpAppendVector(rb, L, _)` with nothing between the bind
//! and the delivery naming `rb`, and never names `L` after the delivery; `L` is named nowhere
//! outside that range but its null init; `__ref_N` is named only at its null inits, its null
//! guard, this call and its frees; neither is a parameter or captured, and the function has
//! no `par` or `yield`.  The callee's entry clear is the buffer's reuse contract across calls
//! (`(R-RetAdopt)`), which is why the delivery's own clear can go.  Every decline keeps the
//! copy.  `LOFT_NO_FORWARD_RESULT=1` is the switch; `LOFT_TRACE_FORWARD_RESULT=1` names each
//! admission and the reason a candidate declined.  Both backends: decided in the scope pass
//! on the settled IR, after `(R-ExitVector)`.
use crate::data::{Data, DefType, Type, Value};
use crate::variables::Function;

/// Rewrite every admitted block of function `d_nr`; a no-op under the switch.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::forward_result_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let def = data.def(d_nr);
    if !def.is_loft_defined() || !matches!(def.returned().peel_link(), Type::Vector(_, _)) {
        return;
    }
    let Some(rb) = own_return_buffer(data, d_nr) else {
        return;
    };
    if def
        .code()
        .any_node(&mut |n| matches!(n, Value::Parallel(_) | Value::Yield(_)))
    {
        return;
    }
    let trace = std::env::var("LOFT_TRACE_FORWARD_RESULT").is_ok();
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let mut sites: Vec<Site> = Vec::new();
    {
        let vars = data.def(d_nr).variables();
        find(&code, &mut |ops| match admit(ops, &code, data, vars, rb) {
            Ok(site) => sites.push(site),
            Err(Some(why)) if trace => {
                eprintln!("forward-result: {} declines — {why}", data.def(d_nr).name());
            }
            Err(_) => {}
        });
    }
    // A local is judged across ALL its sites: the arms of one `match` bind one `buf`.  Outside
    // the sites it is named only at its null inits, or every site of it keeps the copy.
    let mut locals: Vec<u16> = sites.iter().map(|s| s.local).collect();
    locals.sort_unstable();
    locals.dedup();
    let mut fired = 0usize;
    for local in locals {
        let group: Vec<&Site> = sites.iter().filter(|s| s.local == local).collect();
        let vars = data.def(d_nr).variables();
        let inside: usize = group.iter().map(|s| s.inside).sum();
        if mentions(&code, local) != inside + count_null_sets(&code, local) {
            if trace {
                eprintln!(
                    "forward-result: {} declines — `{}` is named outside its arms",
                    data.def(d_nr).name(),
                    vars.name(local)
                );
            }
            continue;
        }
        if trace {
            eprintln!(
                "forward-result: {} builds `{}` in the return buffer `{}` ({} site(s))",
                data.def(d_nr).name(),
                vars.name(local),
                vars.name(rb),
                group.len()
            );
        }
        for site in group {
            apply(&mut code, site, rb);
            fired += 1;
        }
    }
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired("R-ForwardResult", fired);
}

fn own_return_buffer(data: &Data, d_nr: u32) -> Option<u16> {
    let def = data.def(d_nr);
    let idx = def.hidden_return_buffer_attr()?;
    let name = &def.attributes().get(idx)?.name;
    let v = def.variables().var(name);
    (v != u16::MAX && def.variables().is_argument(v)).then_some(v)
}

/// What one admitted block rewrites, by statement index.
struct Site {
    local: u16,
    /// The hidden buffer the bind's call was handed: unique to this site.
    buffer: u16,
    /// How often the local is named inside [bind, delivery].
    inside: usize,
    /// The null guard that mints the hidden buffer, when the block has one.
    guard: Option<usize>,
    bind: usize,
    /// `OpClearVector(rb)`; the append follows it.
    clear: usize,
}

/// Visit every statement list in `v` — a block's or a loop's.
fn find<'a>(v: &'a Value, f: &mut impl FnMut(&'a [Value])) {
    v.walk(&mut |c| {
        if let Value::Block(bl) | Value::Loop(bl) = c {
            f(&bl.operators);
        }
    });
}

/// How often `x` is named in `v` — as a variable or as a `Set` target.
fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |c| {
        if matches!(c, Value::Var(y) if *y == x) || matches!(c, Value::Set(y, _) if *y == x) {
            n += 1;
        }
    });
    n
}

fn call_named<'a>(v: &'a Value, data: &Data, name: &str) -> Option<&'a [Value]> {
    match v.unspan() {
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len() && data.def(*d).name() == name =>
        {
            Some(args)
        }
        _ => None,
    }
}

fn is_var(v: Option<&Value>, x: u16) -> bool {
    matches!(v.map(Value::unspan), Some(Value::Var(y)) if *y == x)
}

/// `Err(None)` for a block that holds no candidate at all, `Err(Some(why))` for one that does
/// and declines.
fn admit(
    ops: &[Value],
    code: &Value,
    data: &Data,
    vars: &Function,
    rb: u16,
) -> Result<Site, Option<String>> {
    // The delivery pair, then the bind it delivers.
    let clear = (0..ops.len().saturating_sub(1))
        .find(|&k| {
            call_named(&ops[k], data, "OpClearVector").is_some_and(|a| is_var(a.first(), rb))
                && call_named(&ops[k + 1], data, "OpAppendVector")
                    .is_some_and(|a| is_var(a.first(), rb) && matches!(a.get(1).map(Value::unspan), Some(Value::Var(_))))
        })
        .ok_or(None)?;
    let Some(Value::Var(local)) = call_named(&ops[clear + 1], data, "OpAppendVector")
        .and_then(|a| a.get(1))
        .map(Value::unspan)
    else {
        return Err(None);
    };
    let local = *local;
    let bind = (0..clear)
        .rev()
        .find(|&j| matches!(ops[j].unspan(), Value::Set(l, _) if *l == local))
        .ok_or(None)?;
    let Value::Set(_, rhs) = ops[bind].unspan() else {
        return Err(None);
    };
    let Value::Call(f, args) = rhs.unspan() else {
        return Err(Some(format!("`{}` is not bound from a call", vars.name(local))));
    };
    let name = |x: u16| vars.name(x).to_string();
    let Some(Value::Var(h)) = args.last().map(Value::unspan) else {
        return Err(Some(format!("`{}`'s call takes no buffer", name(local))));
    };
    let h = *h;
    let callee = data.def(*f);
    if !callee.is_loft_defined()
        || callee.hidden_return_buffer_attr() != Some(args.len() - 1)
        || callee.returns_borrowed_view()
        || !matches!(callee.returned().peel_link(), Type::Vector(_, _))
    {
        return Err(Some(format!(
            "`{}` does not deliver into its buffer argument",
            callee.name()
        )));
    }
    if !vars.name(h).starts_with("__ref_") {
        return Err(Some(format!("the buffer `{}` is not a hidden one", name(h))));
    }
    for x in [local, h] {
        if vars.is_argument(x) || vars.is_captured(x) {
            return Err(Some(format!("`{}` is a parameter or captured", name(x))));
        }
    }
    if !matches!(vars.tp(local).peel_link(), Type::Vector(_, _)) {
        return Err(Some(format!("`{}` is not a vector", name(local))));
    }
    if args[..args.len() - 1].iter().any(|a| mentions(a, rb) > 0)
        || ops[bind + 1..clear].iter().any(|o| mentions(o, rb) > 0)
    {
        return Err(Some("the return buffer is named before its delivery".into()));
    }
    if ops[bind + 1..clear]
        .iter()
        .any(|o| o.any_node(&mut |c| matches!(c, Value::Set(y, _) if *y == local)))
    {
        return Err(Some(format!("`{}` is rebound before its delivery", name(local))));
    }
    if ops[clear + 2..].iter().any(|o| mentions(o, local) > 0) {
        return Err(Some(format!("`{}` is used after its delivery", name(local))));
    }
    let inside: usize = ops[bind..=clear + 1].iter().map(|o| mentions(o, local)).sum();
    // `__ref_N` serves this call alone: its null inits, its guard's test, the call, its frees.
    let guard = (0..bind).rev().find(|&g| is_null_guard(&ops[g], data, h));
    let mut frees = 0usize;
    code.walk(&mut |c| {
        if call_named(c, data, "OpFreeRef").is_some_and(|a| a.len() == 1 && is_var(a.first(), h)) {
            frees += 1;
        }
    });
    let guard_mentions = guard.map_or(0, |g| mentions(&ops[g], h));
    let allowed = count_null_sets(code, h) + frees + 1 + guard_mentions
        - guard.map_or(0, |g| count_null_sets(&ops[g], h));
    if mentions(code, h) != allowed {
        return Err(Some(format!("the buffer `{}` serves more than this call", name(h))));
    }
    Ok(Site {
        local,
        buffer: h,
        inside,
        guard,
        bind,
        clear,
    })
}

fn count_null_sets(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |c| {
        if matches!(c, Value::Set(y, rhs) if *y == x && matches!(rhs.unspan(), Value::Null)) {
            n += 1;
        }
    });
    n
}

/// `if OpRefIsNull(h) { h = null } else null` — the mint of a hidden buffer on first use.
fn is_null_guard(v: &Value, data: &Data, h: u16) -> bool {
    let Value::If(test, then, _) = v.unspan() else {
        return false;
    };
    call_named(test, data, "OpRefIsNull").is_some_and(|a| is_var(a.first(), h))
        && then.any_node(&mut |c| matches!(c, Value::Set(y, _) if *y == h))
}

/// Is `ops` the statement list `site` was admitted on?  Its bind hands the call the site's
/// own hidden buffer, which `admit` proved serves that call alone.
fn is_site(ops: &[Value], site: &Site, rb: u16) -> bool {
    matches!(ops.get(site.bind).map(Value::unspan), Some(Value::Set(l, rhs))
        if *l == site.local
            && matches!(rhs.unspan(), Value::Call(_, a) if is_var(a.last(), site.buffer)))
        && matches!(ops.get(site.clear).map(Value::unspan), Some(Value::Call(_, a)) if is_var(a.first(), rb))
}

fn apply(code: &mut Value, site: &Site, rb: u16) {
    let mut done = false;
    code.map_nodes(&mut |n| {
        if done {
            return;
        }
        let (Value::Block(bl) | Value::Loop(bl)) = n else {
            return;
        };
        if !is_site(&bl.operators, site, rb) {
            return;
        }
        done = true;
        let ops = &mut bl.operators;
        // The bind becomes the call into rb, bound back onto rb, and every read of L until the
        // delivery reads rb.
        let call = match ops[site.bind].unspan_mut() {
            Value::Set(_, rhs) => std::mem::replace(rhs.as_mut(), Value::Null),
            _ => unreachable!("is_site checked the bind"),
        };
        let mut call = call;
        if let Value::Call(_, args) = call.unspan_mut()
            && let Some(last) = args.last_mut()
        {
            *last = Value::Var(rb);
        }
        // Bound back onto rb: a caller that offered no buffer hands in the null sentinel, and
        // the callee then answers a store it minted — the value a bare call would drop.  The
        // shape is the hidden-buffer self-rebind the backends already take (`S1`).
        ops[site.bind] = Value::Set(rb, Box::new(call));
        for op in &mut ops[site.bind + 1..site.clear] {
            op.map_nodes(&mut |n| {
                if matches!(n, Value::Var(y) if *y == site.local) {
                    *n = Value::Var(rb);
                }
            });
        }
        // From the back, so the earlier indices stay valid.
        ops.remove(site.clear + 1);
        ops.remove(site.clear);
        if let Some(g) = site.guard {
            ops.remove(g);
        }
    });
    assert!(done, "(R-ForwardResult): the admitted block was not found again");
}
