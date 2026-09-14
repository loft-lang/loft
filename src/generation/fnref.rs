// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The dispatch ARMS of a fn-ref call: which definitions a call through a `fn(…) -> τ`
//! value can reach.  ONE home, read by two askers — the emitter, to build the `match` a
//! `CallRef` becomes (every arm shares that one argument list and one return type), and
//! the value-record gate (`hoist::value_records`), to decline every arm, because an arm
//! whose ABI changes breaks that join.  Asked a second way the two drift: the gate once
//! tested "returns the same record as a `FnRef`'d function" and missed a lambda that
//! reached a dispatch through a typed variable alone (@PLN157 § V-ah stage 1).
use super::{Context, rust_type};
use crate::data::{Data, DefType, Type};
use std::collections::HashSet;

/// One arm of a fn-ref dispatch.
pub struct Arm {
    /// The definition the arm calls.
    pub d_nr: u32,
    /// Its name, as the definition spells it.
    pub name: String,
    /// The last attribute is the synthetic `__closure`, injected at the call site.
    pub has_closure: bool,
}

/// The arms a call through a value of `fn_type` with `n_args` call-site arguments can
/// dispatch to: every definition whose USER-VISIBLE signature matches — the same count of
/// visible parameters, the same Rust type per parameter, the same Rust result type.
/// `reachable` empty means every definition; otherwise only those in it.  `None` when
/// `fn_type` is not a function type.
///
/// The count compared is the fn-ref TYPE's, not the call's: a text-returning fn-ref
/// call site appends one OR MORE `&text` work buffers to its arguments
/// (`Data::fnref_text_buffers`, loft#1116), so subtracting a fixed one from `n_args`
/// matched no candidate once a call appended two — the arm collapsed to
/// `_ => unreachable!()` and rustc answered E0282 rather than naming anything about
/// loft.  Through `base()`, because the `?` does not change how a text return is
/// DELIVERED: `Parser::text_return` peels `Optional` before it converts the body, so a
/// `-> text?` site appends exactly the same hidden buffers, and reading the raw type
/// here asked for an arity that counted them (P227).
#[must_use]
pub fn dispatch_arms(
    data: &Data,
    reachable: &HashSet<u32>,
    fn_type: &Type,
    n_args: usize,
) -> Option<Vec<Arm>> {
    let Type::Function(param_types, ret_type, _) = fn_type else {
        return None;
    };
    let user_arg_match = if matches!(ret_type.base(), Type::Text(_)) && n_args > param_types.len()
    {
        param_types.len()
    } else {
        n_args
    };
    // Only native-callable functions (`n_` / `t_` prefix) are arms; bytecode ops (`Op*`)
    // are never callable via fn-refs in native mode.
    let n_defs = data.definitions();
    let mut candidates: Vec<Arm> = Vec::new();
    for d in 0..n_defs {
        if !reachable.is_empty() && !reachable.contains(&d) {
            continue;
        }
        let def = data.def(d);
        if !matches!(def.def_type(), DefType::Function) {
            continue;
        }
        if def.name().starts_with("Op") {
            continue;
        }
        // A closure-capturing lambda has a hidden `__closure` param as its last
        // attribute.  The closure is injected explicitly at the call site, so the total
        // arg count must equal the full attribute count.
        let has_closure = def
            .attributes()
            .last()
            .is_some_and(|a| a.name == "__closure");
        // P227: hidden-attribute detection is TYPE-based, not name-based.  Text-return
        // work buffers ride as `Type::RefVar(Type::Text(_))` attributes that the parser
        // names after the user-visible variable they shadow (e.g. `a` for
        // `a = "first: {n}"; a`) — a name-prefix check would miss these and reject
        // otherwise matching candidates.  Closure records stay detected by the exact
        // `__closure` name (its typedef is plain `DbRef`).
        let visible_attrs: Vec<&crate::data::Attribute> = def
            .attributes
            .iter()
            .filter(|a| {
                // PLAN51 V-c: `ref_return` appends a hidden Reference/Vector/struct-enum
                // buffer arg to heap-returning user fns.  Excluding that synthetic attr
                // keeps arity matching against the call site's user-visible arg count —
                // without it every ref_return-promoted lambda fails the count and the
                // dispatch emits only `_ => unreachable!` (probes 30, 59, 62 panicked
                // with `invalid fn-ref`).
                !a.hidden
                    && !matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Text(_)))
                    && a.name != "__closure"
            })
            .collect();
        if visible_attrs.len() != user_arg_match {
            continue;
        }
        let params_match = visible_attrs
            .iter()
            .zip(param_types.iter())
            .all(|(a, expected)| {
                rust_type(&a.typedef, &Context::Argument)
                    == rust_type(expected, &Context::Argument)
            });
        if !params_match {
            continue;
        }
        if rust_type(def.returned(), &Context::Result) != rust_type(ret_type, &Context::Result) {
            continue;
        }
        candidates.push(Arm {
            d_nr: d,
            name: def.name().to_string(),
            has_closure,
        });
    }
    Some(candidates)
}
