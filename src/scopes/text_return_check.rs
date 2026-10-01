// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Debug check: a text-returning function must not free the text it returns on any path.

#[cfg(debug_assertions)]
use super::returns::returned_var_null_unified;
#[cfg(debug_assertions)]
use crate::data::{Data, Type, Value};
#[cfg(debug_assertions)]
use crate::fxhash::FxHashSet as HashSet;
#[cfg(debug_assertions)]
use crate::variables::Function;

/// Debug-only check: refuse to compile a text-returning function that frees a
/// local text on a path that REACHES a `Return` handing that same local back.
/// The returned Str would dangle into freed `String` memory — the interpreter
/// occasionally gets away with it (if the underlying allocator hasn't reused
/// the slot), but native codegen materialises this as
/// `let _v = String::new(); … free(_v); return &_v;` and trips Rust's UB check.
///
/// The judgement is per-`Return` and path-sensitive, and it has to be both.
/// A function with more than one `return` legitimately frees the locals it is
/// NOT handing back, and that free sits inside the branch returning something
/// else: `if n > 0 { free(ta); return tb; } return ta;` is correct code, and
/// the free of `ta` never runs on the path that returns `ta`.  Asking instead
/// "is this var freed anywhere in the body?" answers that shape as a dangling
/// return.  A branch that always returns cannot fall through, so its frees are
/// carried on its own path only and are dropped at the join.
///
/// Every `Return` is judged, not only the body's tail — an early return is
/// exactly where a second text buffer puts its free.
///
/// Companion to `check_ref_leaks` above — that check catches owned-ref leaks
/// at compile time; this one catches use-after-free on return.
#[cfg(debug_assertions)]
pub(super) fn check_text_return(
    ir: &Value,
    function: &Function,
    fn_name: &str,
    ret_type: &Type,
    data: &Data,
) {
    if !matches!(ret_type, Type::Text(_)) {
        return;
    }
    let free_text_nr = data.def_nr("OpFreeText");
    if free_text_nr == u32::MAX {
        return;
    }
    let mut freed: HashSet<u16> = HashSet::default();
    // A `break` at function level has no loop to leave, so this set stays empty; it exists
    // so the walker can hand a break arm's frees to the loop that encloses it.
    let mut breaks: HashSet<u16> = HashSet::default();
    check_text_return_path(
        ir,
        &mut freed,
        &mut breaks,
        free_text_nr,
        data.def_nr("OpNullRefSentinel"),
        function,
        fn_name,
    );
}

/// True when control cannot fall through `node` to the statement AFTER it.
///
/// Two ways out qualify, and they differ in where the frees go rather than in
/// whether they fall through:
///
/// - a `Return` leaves the function, so a free on that path reaches nothing;
/// - a `Break` leaves the LOOP, so a free on that path reaches what follows the
///   loop and NOT the rest of the loop body.  The `Loop` arm of the walker is
///   what carries those frees past the loop, so counting `Break` here loses no
///   strictness — it only stops a break arm's frees being charged to the body
///   statements the break jumps over.
///
/// Missing the `Break` case is a FALSE POSITIVE, not a blind spot, and it fired:
/// `for v in it { if done { free(v); break; } return v; }` — the shape every
/// early-returning loop over a text generator has — read as freeing `v` on the
/// path reaching its own `Return`, and hard-failed the debug-assertions gate on
/// a program with nothing wrong with it.
///
/// Answers `false` for anything it cannot prove, including `Loop` itself.  That
/// stays the safe direction: an unproven terminator keeps its frees in the
/// continuation set, which makes the check stricter rather than blinder.
#[cfg(debug_assertions)]
fn always_returns(node: &Value) -> bool {
    match node.unspan() {
        Value::Return(_) => true,
        Value::Block(bl) => bl.operators.iter().any(always_returns),
        Value::Insert(ops) => ops.iter().any(always_returns),
        Value::If(_, t, f) => always_returns(t) && always_returns(f),
        _ => false,
    }
}

/// True when control cannot fall through `node` to the statement after it — by
/// RETURNING or by BREAKING.  Paired with [`always_returns`], which separates the
/// two: a return's frees reach nothing, a break's reach what follows the loop.
#[cfg(debug_assertions)]
fn never_falls_through(node: &Value) -> bool {
    match node.unspan() {
        Value::Return(_) | Value::Break(_) => true,
        // Anything after a `Return` or `Break` in the same block is dead, so one
        // top-level terminator terminates the whole block.
        Value::Block(bl) => bl.operators.iter().any(never_falls_through),
        Value::Insert(ops) => ops.iter().any(never_falls_through),
        Value::If(_, t, f) => never_falls_through(t) && never_falls_through(f),
        _ => false,
    }
}

/// Walk `node` in execution order carrying `freed` — the text vars already
/// released on the path reaching it — and assert each `Return` hands back a
/// var this path has not freed.  See `check_text_return` for the rule.
#[cfg(debug_assertions)]
fn check_text_return_path(
    node: &Value,
    freed: &mut HashSet<u16>,
    breaks: &mut HashSet<u16>,
    free_text_nr: u32,
    null_nr: u32,
    function: &Function,
    fn_name: &str,
) {
    let walk = |n: &Value, set: &mut HashSet<u16>, brk: &mut HashSet<u16>| {
        check_text_return_path(n, set, brk, free_text_nr, null_nr, function, fn_name);
    };
    match node.unspan() {
        Value::Call(d_nr, args) if *d_nr == free_text_nr => {
            if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                freed.insert(*v);
            }
        }
        Value::Return(inner) => {
            // The return expression runs BEFORE the return itself, so a free
            // inside it counts against this very return.
            walk(inner, freed, breaks);
            let ret_var = returned_var_null_unified(inner, null_nr);
            if ret_var != u16::MAX && matches!(function.tp(ret_var), Type::Text(_)) {
                assert!(
                    !freed.contains(&ret_var),
                    "[check_text_return] fn '{}' frees local text '{}' (var_nr={ret_var}) \
                     on the path reaching its Return — the returned Str would dangle into \
                     freed String memory.  scopes.rs must leave '{}' for the caller to free.",
                    fn_name,
                    function.name(ret_var),
                    function.name(ret_var),
                );
            }
        }
        Value::If(cond, t, f) => {
            // The condition runs on both paths; each arm gets its own.
            walk(cond, freed, breaks);
            let mut then_freed = freed.clone();
            walk(t, &mut then_freed, breaks);
            let mut else_freed = freed.clone();
            walk(f, &mut else_freed, breaks);
            // Where an arm's frees go is decided by HOW it leaves.  Falling through hands
            // them to the next statement; RETURNING hands them nowhere, since the function
            // is over; BREAKING hands them to what follows the LOOP, which is what `breaks`
            // carries out to the enclosing `Loop` arm.  Collapsing the last two — treating a
            // break like a return — would lose a real free, and treating it like a
            // fall-through charges it to body statements the break jumps over, which is the
            // false positive this split exists to remove.
            let mut arm = |a: &Value, arm_freed: HashSet<u16>, freed: &mut HashSet<u16>| {
                if always_returns(a) {
                } else if never_falls_through(a) {
                    breaks.extend(arm_freed);
                } else {
                    freed.extend(arm_freed);
                }
            };
            arm(t, then_freed, freed);
            arm(f, else_freed, freed);
        }
        Value::Block(bl) => {
            for op in &bl.operators {
                walk(op, freed, breaks);
            }
        }
        Value::Loop(bl) => {
            // A loop is left either by falling out of its body or by a BREAK, and both
            // continuations resume AFTER the loop — so both sets are unioned here.  The
            // break set is fresh per loop, which is what keeps an inner loop's breaks from
            // reaching the outer loop's continuation.
            let mut body_freed = freed.clone();
            let mut body_breaks = HashSet::default();
            for op in &bl.operators {
                walk(op, &mut body_freed, &mut body_breaks);
            }
            freed.extend(body_breaks);
            freed.extend(body_freed);
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                walk(op, freed, breaks);
            }
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                walk(a, freed, breaks);
            }
        }
        Value::Iter(_, create, next, extra_init) => {
            walk(create, freed, breaks);
            walk(next, freed, breaks);
            walk(extra_init, freed, breaks);
        }
        Value::Set(_, inner)
        | Value::TuplePut(_, _, inner)
        | Value::Drop(inner)
        | Value::Yield(inner) => walk(inner, freed, breaks),
        _ => {}
    }
}

/// The `check_text_return` walker's own gate.  Every cell is one step from
/// another, and the pair that matters is `free_then_return_same_var_in_arm`
/// (must fire) against `free_in_arm_that_returns_another_var` (must not) —
/// they differ only in WHICH var the arm hands back, which is exactly the
/// distinction the path rule exists to draw.
///
/// Compiled only where the check itself is: `[profile.dev.package.loft]`
/// strips debug assertions from ordinary builds, so these run in the
/// `-C debug-assertions=on` CI gate that runs the check.
#[cfg(all(test, debug_assertions))]
mod text_return_path_tests {
    use super::check_text_return_path;
    use crate::data::{Deps, Type, Value, v_block, v_if, v_loop};
    use crate::variables::Function;
    use std::collections::HashSet;

    const FREE_TEXT: u32 = 7;
    const NULL_SENTINEL: u32 = 8;
    const TA: u16 = 0;
    const TB: u16 = 1;

    fn vars() -> Function {
        let mut f = Function::new("f", "t.loft");
        f.add_temp_var("ta", &Type::Text(Deps::none()));
        f.add_temp_var("tb", &Type::Text(Deps::none()));
        f
    }

    fn free(v: u16) -> Value {
        Value::Call(FREE_TEXT, vec![Value::Var(v)])
    }
    fn ret(v: u16) -> Value {
        Value::Return(Box::new(Value::Var(v)))
    }

    /// Run the walker over a function body; panics exactly as the check does.
    fn check(body: Vec<Value>) {
        let ir = v_block(body, Type::Text(Deps::none()), "body");
        let mut freed = HashSet::default();
        let mut breaks = HashSet::default();
        check_text_return_path(
            &ir,
            &mut freed,
            &mut breaks,
            FREE_TEXT,
            NULL_SENTINEL,
            &vars(),
            "probe",
        );
    }

    /// Straight-line use-after-free: the plainest shape the check exists for.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_then_return_same_var() {
        check(vec![free(TA), ret(TA)]);
    }

    /// The free and the `return` of the same var sit in ONE arm.  Skipping a
    /// returning arm wholesale would blind the check here, so this is the cell
    /// that keeps the fall-through rule honest.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_then_return_same_var_in_arm() {
        check(vec![
            v_if(
                Value::Var(TB),
                v_block(vec![free(TA), ret(TA)], Type::Never, "then"),
                Value::Null,
            ),
            ret(TB),
        ]);
    }

    /// A free on an arm that FALLS THROUGH still reaches the tail return —
    /// the arm not returning is the whole difference from the cell below.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_in_arm_that_falls_through() {
        check(vec![
            v_if(
                Value::Var(TB),
                v_block(vec![free(TA)], Type::Void, "then"),
                Value::Null,
            ),
            ret(TA),
        ]);
    }

    /// The shape the path rule was written for: the arm frees `ta` and returns
    /// `tb`, so the free never runs on the path that returns `ta`.  Correct
    /// code — the check must stay silent (loft#1113's two-text-local lambda).
    #[test]
    fn free_in_arm_that_returns_another_var() {
        check(vec![
            v_if(
                Value::Var(TB),
                v_block(vec![free(TA), ret(TB)], Type::Never, "then"),
                Value::Null,
            ),
            ret(TA),
        ]);
    }

    /// Both arms return, each freeing the local it does not hand back.
    #[test]
    fn each_arm_frees_what_it_does_not_return() {
        check(vec![v_if(
            Value::Var(TB),
            v_block(vec![free(TA), ret(TB)], Type::Never, "then"),
            v_block(vec![free(TB), ret(TA)], Type::Never, "else"),
        )]);
    }

    /// A BREAK arm's free does not reach the loop body that follows it — the shape every
    /// early-returning loop over a text source has, and the one that read as a
    /// use-after-free before `never_falls_through` learned about `Break`:
    ///
    /// ```text
    /// loop { ta = next(); if done { free(ta); break; } return ta; }
    /// ```
    ///
    /// The arm that frees is the arm that LEAVES, so it can never reach the `return`.
    /// Correct code — the check must stay silent.
    #[test]
    fn free_in_a_break_arm_does_not_reach_the_bodys_return() {
        check(vec![v_loop(
            vec![
                v_if(
                    Value::Var(TB),
                    v_block(vec![free(TA), Value::Break(0)], Type::Never, "then"),
                    Value::Null,
                ),
                ret(TA),
            ],
            "loop",
        )]);
    }

    /// The control for the cell above, and the reason it cannot be written as "skip a loop".
    /// Here the free FALLS THROUGH inside the same loop body and the `return` follows it, so
    /// the free really is on the path that returns `ta`.  The two differ only by the `break`.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_without_a_break_still_reaches_the_bodys_return() {
        check(vec![v_loop(
            vec![
                v_if(
                    Value::Var(TB),
                    v_block(vec![free(TA)], Type::Void, "then"),
                    Value::Null,
                ),
                ret(TA),
            ],
            "loop",
        )]);
    }

    /// A break arm's free is still charged to what follows the LOOP, which is where that
    /// path actually resumes.  Counting `Break` as a terminator must not lose this.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn a_break_arms_free_still_reaches_after_the_loop() {
        check(vec![
            v_loop(
                vec![v_if(
                    Value::Var(TB),
                    v_block(vec![free(TA), Value::Break(0)], Type::Never, "then"),
                    Value::Null,
                )],
                "loop",
            ),
            ret(TA),
        ]);
    }
}
