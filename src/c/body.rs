// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! One function's body as C: statements, expressions and their C shapes.
//!
//! Every temporary is declared at the top of the function, so one created inside a branch is
//! still in scope where it is freed.  An owned text temporary is freed after the statement
//! that made it; every `return` first copies its value out, then frees the function's texts.

mod apart;
mod apart_value;
mod chain;
mod deferred;
mod files;
mod generator;
mod keyed;
mod misc;
mod par;
mod value;

use super::{CType, Cx, c_name, held, is_operator, type_word};
use crate::data::{Block, Value};
use crate::variables::Function;
use std::borrow::Borrow;
use std::fmt::Write as _;

/// A C-safe name for variable `v`: its number first, so two loft names that sanitise alike
/// stay apart.
pub(crate) fn var_name(name: &str, v: u16) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("v{v}_{clean}")
}

/// The shape of a C expression's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    /// `int64_t`.
    Int,
    /// @PLN204 — `int32_t`, null `LOFT_NULL32` (`CType::Int32`).
    Int32,
    /// A C condition, 0 or 1.
    Cond,
    /// A stored boolean, `uint8_t` 0, 1 or `LOFT_BOOL_NULL`.
    Bool,
    /// A `loft_str` view.
    View,
    /// A `loft_ref`.
    Ref,
    /// A plain enum value, `uint8_t`.
    Enum,
    /// `double`.
    Float,
    /// `float`.
    Single,
    /// A character's code point, `uint32_t`; 0 is null.
    Char,
    /// A function value, `loft_fn`.
    Fn,
    /// A tuple struct, by its number.
    Tup(u32),
    /// No value.
    Void,
}

/// The collection a new record is added to (`FnWriter::record_collection`).
pub(super) enum Coll {
    /// A plain vector.
    Vector,
    /// A record collection of this type (`Cx::keyed`).
    Keyed(u16),
    /// A member of a linked group: the parent read once, and each member's byte position and
    /// type, the one written first.
    Group(String, Vec<(u16, u16)>),
}

/// A C expression and the shape of its value.
struct E {
    code: String,
    kind: K,
}

/// What a loop's verdict hands its push headers: the minted record paths and the scalar pushed
/// paths, each with the vector operand the prelude evaluates, and whether the loop's one
/// growth is its one pushed path's pushes (`hoist::LoopHoist::sole_push`).
type Pushed = (
    Vec<(crate::opt::writes::PathKey, Value)>,
    Vec<(crate::opt::writes::PathKey, Value)>,
    bool,
);

impl E {
    fn new(code: impl Into<String>, kind: K) -> E {
        E {
            code: code.into(),
            kind,
        }
    }
}

/// `@FR-R-RecPtr` — is `n` a record copy into an inline sub-record of view `r`
/// (`OpCopyRecord(_, OpGetField(r, off, Sub), _)`)?  A value-record copy writes it through
/// `r`'s address (`vr_copy`), so it is a write through the address and a use of it.
fn copy_into_sub(data: &crate::data::Data, n: &Value, r: u16) -> bool {
    matches!(n, Value::Call(d, a) if (*d as usize) < data.definitions.len()
        && data.def(*d).name() == "OpCopyRecord"
        && a.len() == 3
        && crate::opt::body_writes::sub_record(data, &a[1]).is_some_and(|(root, _, _)| root == r))
}

/// `LOFT_NO_FIELD_VIEW=1`: every text field operand is copied (`text_reads`).
fn no_field_view() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_FIELD_VIEW").is_ok_and(|v| v != "0"))
}

/// The expression kind that reads a held value of shape `c`.
fn kind_of(c: CType) -> K {
    match c {
        CType::Int => K::Int,
        CType::Int32 => K::Int32,
        CType::Bool => K::Bool,
        CType::Text => K::View,
        CType::Ref => K::Ref,
        CType::Enum => K::Enum,
        CType::Float => K::Float,
        CType::Single => K::Single,
        CType::Char => K::Char,
        CType::Fn => K::Fn,
        CType::Tuple(id) => K::Tup(id),
        CType::Void => K::Void,
    }
}

/// One function's body under construction.
// Its flags are independent per-rewrite facts about one function, not one state to enumerate.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct FnWriter<'a> {
    cx: &'a Cx<'a>,
    /// The function's definition number.
    d: u32,
    vars: &'a Function,
    code: &'a Value,
    /// The C shape of every variable, parameters first.
    shapes: Vec<CType>,
    /// Which variables are the function's parameters (`Function::arguments`).
    params: Vec<bool>,
    /// Which parameters are `&` links: a pointer to the caller's variable, read and written
    /// through (`(*v)`), as the bytecode reads and writes through the stack reference.
    links: Vec<bool>,
    /// The locals whose copy is deferred to the first statement that may write its source
    /// (`@FR-R-DeferredCopy`, `deferred::deferred_plan`).
    dc: deferred::DeferredCopies,
    /// Which value-record parameters arrive as `const loft_tupN *` (`Cx::vr_ptr_params`,
    /// `@FR-R-ValueRecord`'s pointer clause): read through (`(*v)`), never written.
    vr_ptr: Vec<bool>,
    /// Which `&` locals link a STORE place (`c = &v[1]`, `f = &o.x`): every bind names an
    /// element or a field, never a variable.  Held as the place's `loft_ref`; a read or a
    /// write goes through the place's own operator, as the bytecode's `OpGetInt`/`OpSetInt`
    /// through the link's `DbRef`.  A link bound to a variable on one path and a place on
    /// another is not carried.
    store_links: Vec<bool>,
    ret: CType,
    body: String,
    /// Every temporary: its name and shape, declared at the top.
    temps: Vec<(String, CType)>,
    /// The owned text temporaries the current statement made.
    pending: Vec<String>,
    /// The label number of each enclosing loop, innermost last.
    loops: Vec<u32>,
    labels: u32,
    /// The vectors each enclosing loop holds (`loft_vh`), innermost last (@FR-R-Header), each
    /// with whether its element base stays valid for the loop's extent (`@FR-R-Base`: the
    /// loop grows no store).  An `__inv` twin starts with its inputs as one frame.
    headers: Vec<Vec<(crate::opt::writes::PathKey, String, bool)>>,
    /// `@FR-R-Header`'s growth half: the held headers derived again after every statement
    /// that grows their vector (`refresh_grown_header`), and the paths the verdict just
    /// admitted under it, which `hold_vectors` binds.
    grown_headers: std::collections::HashSet<String>,
    pending_grown: Vec<crate::opt::writes::PathKey>,
    /// The grown headers the statement being emitted handed to a `__kh` twin, which kept them
    /// current: the refresh after it skips them.
    kh_passed: std::collections::HashSet<String>,
    /// A `__nn` twin: the parameters its caller's arguments prove (`super::NN_MASK`).
    nn_seed: Vec<u16>,
    /// The record scalars each enclosing loop reads once before it (`@FR-R-Scalar`): the
    /// `(variable, field offset)` key, the local holding it and its shape, innermost last.
    scalars: Vec<Vec<(crate::opt::hoist::ScalarKey, String, CType)>>,
    /// `@FR-R-Invariant` — the invariant integer chains the enclosing loops memoise
    /// (`hoist::invariant_chains`), by the address of each node that spells one: the memo's
    /// local, and the chain its first use evaluates.
    invariants: Vec<std::collections::HashMap<usize, std::rc::Rc<(String, Value)>>>,
    /// `@FR-R-ValueRecord`'s facts for this function (`value::VrFn`), when the plan has any.
    vr: Option<Box<value::VrFn>>,
    /// The leaf being written as its dense self inside its widening (`value::vr_assign`).
    widening: Option<usize>,
    /// A value staged into a C temporary, by the temporary's name, with its kind: what a
    /// `Value::RawExpr` the writer built for a rewrite reads (`keyed::keyed_in_place`).
    raw: std::collections::HashMap<String, K>,
    /// Set by a text VALUE op (`@FR-R-TextBorrow`) for the one operand it reads at once: the
    /// generic `OpGetText` answers the field's bytes where the store holds them instead of a
    /// copy (`text_reads`).  Taken by the first `OpGetText` that sees it.
    field_view: bool,
    /// `@FR-R-HandOver`'s rotate clause: in a `__nk` twin, the keep statement the body skips
    /// (by node address); at a rotate site, the callee whose next call goes to that twin.
    nk_skip: Option<usize>,
    /// `@FR-R-AppendTwin` — in an `__ap` twin, the return buffer (the caller's vector) and every
    /// local that views it; the statement-level call site's rewritten call, matched by callee
    /// and argument slice so its emission asks for the callee's twin; the eligibility memo.
    ap_buf: Option<u16>,
    ap_views: std::collections::HashSet<u16>,
    ap_site: Option<(u32, usize)>,
    ap_memo: std::collections::HashMap<u32, bool>,
    /// `@FR-R-Destination` — in a `__d` twin, the shape (its buffer is the caller's element,
    /// its scalar fields the locals `loft_ds<k>`); the call site's rewritten call, matched by
    /// callee and argument slice so its emission asks for the callee's twin.
    ds: Option<crate::lower::destination::Shape>,
    ds_site: Option<(u32, usize)>,
    nk_call: Option<u32>,
    /// `@FR-R-GuardedChain` — per guarded loop, the argument-slice addresses of its admitted
    /// chain operators, and the guard they branch on (`None` in the plain copy of an innermost
    /// loop, where every one is plain).
    plain_chains: Vec<(std::collections::HashSet<usize>, Option<String>)>,
    /// The guards written so far, which numbers each one's flag.
    chain_guards: u32,
    /// `@FR-R-RecPtr` — the record views whose address the enclosing blocks hold: the view,
    /// the address local and its store's lock flag, innermost last.
    rec_ptrs: Vec<(u16, String, String)>,
    /// A formatted value's work text written straight into the text it is appended to
    /// (`format_into_dest`): the work variable and the destination's C name.
    work_alias: Option<(u16, String)>,
    /// The record push headers each enclosing loop binds (`loft_ph`, `@FR-R-PushRec`): the
    /// path, the header, the element size and whether its pushes write through the record
    /// address it holds (`@FR-R-Push`'s sole-growth clause), innermost last.
    push_heads: Vec<Vec<(crate::opt::writes::PathKey, String, u32, bool)>>,
    /// The header gate's memo of which callees write a store.
    hoist_cache: std::collections::HashMap<u32, bool>,
    /// The locals `(R-Place)` put in another record's store (`hoist::placed_locals`), for
    /// the store facts the loop verdict proves bases apart with.
    placed: std::collections::HashSet<u16>,
    /// `@FR-R-Alias`'s versioned clause: the variable pairs the loop copy being written may
    /// take as holding apart stores, tested at its entry.  Empty outside such a copy.
    assumed: Vec<(u16, u16)>,
    /// `@FR-R-Range` (`opt::range`, native's proof): this function's non-null and range facts,
    /// computed on first use.
    range_facts: Option<
        std::rc::Rc<(
            std::collections::HashMap<u16, bool>,
            crate::opt::range::Ranges,
        )>,
    >,
    /// Declarations beside the temporaries: the held vectors.
    decls: Vec<String>,
    /// `set_stmt` is assigning a tuple local this function owns: a call's tuple moving into it
    /// frees the old value's texts first (`assign`).
    frees_old_tuple: bool,
    /// Which variables a statement written so far has bound: a later `Set` is a rebind.
    bound: Vec<bool>,
    /// A generator's element shape (`body::generator`): its `yield`s and its `return` differ.
    gen_elem: Option<CType>,
    /// The `yield`s written so far, each a resume label.
    yields: u32,
    /// The source line the statements now being written come from (`Value::Line`), which a
    /// call records as its frame's site when the unit keeps frames (`Cx::frames`).
    cur_line: u32,
    /// `LOFT_C_WHERE` reported this function's refusal already.
    where_told: bool,
    /// `@FR-R-RetAdopt` — the result local that IS the hidden return buffer
    /// (`opt::ret_adopt::ret_adopt`, the analysis the Rust generator reads): it builds in
    /// the caller's buffer, its witness store is never minted, and the delivery copy at the
    /// exit has nothing to move.  `LOFT_NO_RETBUF_ADOPT=1` keeps the copies.
    adopt: Option<crate::opt::ret_adopt::RetAdopt>,
    /// Inside an adopted function's `one_buffer_vec_copy` delivery block.
    in_delivery: u32,
    /// `@FR-R-PushFill`'s window clause — the push windows the enclosing counted loops hold:
    /// each path, its `loft_win`, the `loft_ref` the vector operand was read into, and the
    /// element width.
    windows: Vec<(crate::opt::writes::PathKey, String, String, u32)>,
    /// `@FR-R-TextBorrow` (`opt::text_borrow::borrowed_text_walks`, the analysis native reads): the
    /// text locals a walk binds to an element it only READS, under a loop that writes no store —
    /// each a `loft_str` view of the element, as a text parameter is, never a copy.
    borrowed: Vec<bool>,
    /// `@FR-R-TextBorrow`'s tested clause: the borrowed locals that view a parameter's text only
    /// while each root parameter's store is not the return buffer's, with those roots.
    tested: std::collections::HashMap<u16, Vec<u16>>,
    /// `@FR-R-CharWalk` (`opt::char_walk::char_walks`, which the interpreter and native read):
    /// this function's `for c in t` walks by loop scope — their step takes an ASCII byte without
    /// decoding it, and their null test is asked once where the loop never writes `t`.
    /// `LOFT_NO_CHAR_WALK=1` writes every step as the lowering gives it.
    char_walks: std::collections::BTreeMap<u16, crate::opt::char_walk::CharWalk>,
    /// `@FR-R-LazySplit` (`hoist::lazy_splits`, native's analysis): the hidden vectors of
    /// `for piece in t.split(c)` loops, by variable — each walked as a `loft_ls` over the text,
    /// the vector never built.  `LOFT_NO_LAZY_SPLIT=1` builds them, as on native.
    lazy: std::collections::BTreeMap<u16, crate::opt::lazy_split::LazySplit>,
    /// `@FR-R-SplitTable` (`hoist::split_tables`): each bound split's local, as a `loft_stab`
    /// `st<var>` declared up front; the walks' hidden vectors map to their table.
    split_tabs: std::collections::BTreeMap<u16, crate::opt::text_borrow::SplitTable>,
    tab_alias: std::collections::BTreeMap<u16, u16>,
    /// `@FR-R-CompleteWrite` (`hoist::complete_writes`): the element types every mint of this
    /// function writes completely, so the default image copied into a fresh element writes
    /// nothing that survives; computed on first use.
    complete_mints: Option<std::collections::HashSet<u16>>,
    /// The record copies into a field of an element a literal group builds, by argument slice,
    /// with the field's type (`CompleteWrites::fresh_fields`): their null arm writes the
    /// field's default (`fresh_null_arm`); computed with `complete_mints`.
    fresh_fields: std::collections::HashMap<usize, u16>,
    /// `@FR-R-Apart` instance 4: this body's apart records of record vectors and their push
    /// groups' elements, or its `__apr` twin's parameter (`apart.rs`), and the program's plan.
    apr: crate::lower::apart_records::RecRoles,
    apr_plan: Option<std::rc::Rc<crate::lower::apart_records::RecPlan>>,
    /// `@FR-R-Apart` instance 3: this body's apart `vector<float>` locals and their companions.
    apv: crate::lower::apart_vector::VecRoles,
    /// `@FR-R-Apart` instance 2: this body's apart and dead records of fixed-length float
    /// vectors (its twin's, or its region's), whether it is an `__apv` twin, the program's plan
    /// and the function's return buffer (`apart_value.rs`).
    av: crate::lower::apart_value::Roles,
    av_twin: bool,
    av_plan: Option<std::rc::Rc<crate::lower::apart_value::ApartPlan>>,
    av_rb: Option<u16>,
    /// `@FR-R-FileHold` (`lower::file_hold::held_file`, native's gate): the files the enclosing
    /// loops read through a held cursor, each with its `loft_fcur` and the index in `loops` of
    /// the loop that holds it.
    file_holds: Vec<(u16, String, usize)>,
}

/// A counted loop a push window is opened for (`FnWriter::window_loop`): the pushed path,
/// its vector operand, the range that counts its passes and the pushes one pass runs at most.
struct WindowLoop<'b> {
    path: crate::opt::writes::PathKey,
    vector: std::borrow::Cow<'b, Value>,
    hi: &'b Value,
    index_var: u16,
    next_var: Option<u16>,
    inclusive: bool,
    pushes: u32,
    size: u32,
}

/// `@FR-R-TextBorrow` — the text locals of function `d` the C holds as a `loft_str` view rather
/// than an owned copy: the walk variables `opt::text_borrow::borrowed_text_walks` admits and the
/// plain borrows of `hoist::borrowed_store_texts`, its `?` temps included (its run-time-tested and work-text clauses need
/// emitter support the C does not have, so they keep their copies).  A local is kept only when
/// EVERY bind of it is a shape the view form writes without a temporary — another borrowed
/// local or a text parameter, a literal, a text field read, a block ending in one — so
/// admitting it never turns a carried program into a refused one.
/// `LOFT_NO_TEXT_BORROW=1` keeps every copy, as on native.
fn borrowed_texts(cx: &Cx, d: u32) -> (Vec<bool>, std::collections::HashMap<u16, Vec<u16>>) {
    let def = cx.data.def(d);
    let vars = def.variables();
    let n = vars.count() as usize;
    let mut b = vec![false; n];
    if std::env::var("LOFT_NO_TEXT_BORROW").is_ok_and(|v| v != "0") {
        return (b, std::collections::HashMap::new());
    }
    // As native's setup: the lazy splits' and the split tables' walks need no store
    // condition — their pieces are slices of a text — and a `?` discharge of a table's
    // element borrows the piece too.
    let tables = if cx.switches.split_table_disabled {
        std::collections::BTreeMap::new()
    } else {
        crate::opt::hoist::split_tables(cx.data, d)
    };
    let lazy = if std::env::var("LOFT_NO_LAZY_SPLIT").is_ok_and(|v| v != "0") {
        std::collections::BTreeMap::new()
    } else {
        crate::opt::lazy_split::lazy_splits(cx.data, d)
    };
    let sliced: std::collections::BTreeSet<u16> = lazy
        .keys()
        .copied()
        .chain(tables.values().flat_map(|t| t.aliases.iter().copied()))
        .collect();
    let mut walks = crate::opt::text_borrow::borrowed_text_walks(cx.data, d, &sliced, &tables);
    walks.extend(crate::opt::hoist::borrowed_discharge_temps(
        cx.data, d, &tables,
    ));
    // `@FR-R-SliceBorrow` rides on the same analysis: a value slice of a text parameter.
    let store = crate::opt::store_text_borrow::borrowed_store_texts(cx.data, d, &walks);
    // Every variable the clause admits — the `?` / `??` temps it borrows a parameter's text
    // for as well as its statement-bound locals, as native's `walks.extend(store.borrows)`.
    let plain = store
        .borrows
        .keys()
        .filter(|v| !store.tested.contains_key(v) && !store.work.contains(v))
        .copied();
    // The format clause: a local bound only to literals and to format strings whose work text
    // is the format's own (`store.work`) views that work text, which only another run of the
    // same format rewrites — and that run binds the local again.  The call clause keeps its
    // copy: its view would be of the callee's answer.
    let work: std::collections::HashSet<u16> =
        if crate::env_once!(std::env::var("LOFT_NO_C_WORK_BORROW").is_ok_and(|v| v != "0")) {
            std::collections::HashSet::new()
        } else {
            store.work.clone()
        };
    // The tested clause: a local whose scope also writes the return buffer borrows while each
    // root parameter's store is not the buffer's, tested at its bind, and copies into a text
    // of its own (`loft_own<v>`) otherwise.  `LOFT_NO_C_TESTED_BORROW=1` keeps the copy.
    let tested: std::collections::HashMap<u16, Vec<u16>> =
        if crate::env_once!(std::env::var("LOFT_NO_C_TESTED_BORROW").is_ok_and(|v| v != "0"))
            || crate::opt::hoist::retbuf_var(cx.data, d).is_none()
        {
            std::collections::HashMap::new()
        } else {
            store
                .tested
                .iter()
                .filter(|(v, _)| !store.work.contains(v))
                .map(|(v, r)| (*v, r.clone()))
                .collect()
        };
    for v in walks
        .keys()
        .copied()
        .chain(plain)
        .chain(work.iter().copied())
        .chain(tested.keys().copied())
    {
        let i = v as usize;
        if i < n
            && !vars.is_argument(v)
            && matches!(vars.tp(v).base(), crate::data::Type::Text(_))
            && !matches!(vars.tp(v).base(), crate::data::Type::RefVar(_))
        {
            b[i] = true;
        }
    }
    // Every bind of a candidate, gathered once.
    let mut binds: Vec<(u16, &Value)> = Vec::new();
    def.code().any_node(&mut |node| {
        if let Value::Set(v, x) = node
            && b.get(*v as usize).copied().unwrap_or(false)
        {
            binds.push((*v, x));
        }
        false
    });
    loop {
        let mut dropped = false;
        for (v, x) in &binds {
            if b[*v as usize]
                && !view_bind(cx, vars, &b, x)
                && !(work.contains(v) && work_view_bind(vars, &b, x))
            {
                b[*v as usize] = false;
                dropped = true;
            }
        }
        if !dropped {
            let tested = tested
                .into_iter()
                .filter(|(v, _)| b.get(*v as usize).copied().unwrap_or(false))
                .collect();
            return (b, tested);
        }
    }
}

/// `@FR-R-TextBorrow`'s format clause (C): a bind of a work-text borrower that is a format
/// block ending in its work text, a frame-local text the view reads in place.
fn work_view_bind(vars: &Function, borrowed: &[bool], x: &Value) -> bool {
    let Value::Block(bl) = x.unspan() else {
        return false;
    };
    let last = bl
        .operators
        .iter()
        .rev()
        .find(|o| !matches!(o.unspan(), Value::Line(_)));
    let Some(Value::Var(w)) = last.map(Value::unspan) else {
        return false;
    };
    (*w as usize) < borrowed.len()
        && !borrowed[*w as usize]
        && !vars.is_argument(*w)
        && matches!(vars.tp(*w).base(), crate::data::Type::Text(_))
}

/// Is `x` a text the view form of a borrowed bind writes with no temporary (`borrowed_texts`)?
/// `@FR-R-RecPtr`'s apart-call clause (C): the statements after a view's binding at `at`, each
/// call to a loft function that cannot reach the view's store replaced by its arguments — a
/// callee reaches only the stores its arguments name, so where every store-naming variable
/// those mention is proven apart from `r`'s (`StoreFacts::distinct`) and `r` is not one of
/// them, nothing the call runs grows or frees `r`'s store, and the arguments stay to be judged
/// themselves.  A scalar field read in an argument passes a number, not its record's store.  A
/// release (`OpFreeRef`, `OpFreeRefIfDistinct`) of a store apart from `r`'s is dropped the same
/// way: it frees nothing `r` names.  A helper appending to another collection
/// (`put(stack, sp, op.a)`) then no longer costs the loop's record views their address.
/// `LOFT_NO_APART_CALL=1` keeps every call.
fn apart_calls(
    data: &crate::data::Data,
    facts: Option<&crate::opt::push_window::StoreFacts>,
    r: u16,
    ops: &[Value],
    at: usize,
) -> Vec<Value> {
    let Some(facts) = facts
        .filter(|_| !crate::env_once!(std::env::var("LOFT_NO_APART_CALL").is_ok_and(|v| v != "0")))
    else {
        return ops.to_vec();
    };
    let vars = facts.vars;
    let names_store = |v: u16| {
        (v as usize) < vars.var_count()
            && (crate::data::is_dbref(vars.tp(v).base())
                || matches!(vars.tp(v).base(), crate::data::Type::RefVar(_)))
    };
    let var_apart = |v: u16| v != r && (!names_store(v) || facts.distinct(r, v));
    let apart = |args: &[Value]| args.iter().all(|a| mentions_apart(a, data, &var_apart));
    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            let mut o = op.clone();
            if i > at {
                o.map_nodes(&mut |n| {
                    if let Value::Call(g, args) = n
                        && (*g as usize) < data.definitions.len()
                        && apart(args)
                    {
                        let def = data.def(*g);
                        if *def.code() != Value::Null && !def.name().starts_with("Op") {
                            *n = Value::Insert(std::mem::take(args));
                        } else if matches!(def.name(), "OpFreeRef" | "OpFreeRefIfDistinct") {
                            // A release in a store apart from `r`'s frees nothing `r` names.
                            *n = Value::Null;
                        }
                    }
                });
            }
            o
        })
        .collect()
}

/// Does every store-naming variable `x` mentions pass `ok`?  A scalar field read of any record —
/// `op.a` handed to a helper — passes a number, not its store, so its record is not judged.
pub(super) fn mentions_apart(
    x: &Value,
    data: &crate::data::Data,
    ok: &dyn Fn(u16) -> bool,
) -> bool {
    match x.unspan() {
        Value::Var(v) => ok(*v),
        Value::Call(g, a)
            if (*g as usize) < data.definitions.len()
                && crate::opt::loop_records::SCALAR_GETTERS.contains(&data.def(*g).name())
                && matches!(a.first().map(Value::unspan), Some(Value::Var(_))) =>
        {
            a[1..].iter().all(|y| mentions_apart(y, data, ok))
        }
        other => {
            let mut all = true;
            other.for_each_child(&mut |c| all &= mentions_apart(c, data, ok));
            all
        }
    }
}

/// Per variable of `d`: whether it is a value-record parameter passed by pointer
/// (`Cx::vr_ptr_params`).
fn vr_ptr_flags(cx: &Cx, d: u32) -> Vec<bool> {
    let vars = cx.data.def(d).variables();
    let args = vars.arguments();
    let mut p = vec![false; vars.count() as usize];
    for &i in cx.vr_ptr_params(d).iter() {
        if let Some(&a) = args.get(i) {
            p[a as usize] = true;
        }
    }
    p
}

fn view_bind(cx: &Cx, vars: &Function, borrowed: &[bool], x: &Value) -> bool {
    match x.unspan() {
        Value::Block(bl) => bl
            .operators
            .iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
            .is_some_and(|last| view_bind(cx, vars, borrowed, last)),
        Value::Var(w) => {
            borrowed.get(*w as usize).copied().unwrap_or(false)
                || (vars.is_argument(*w)
                    && matches!(vars.tp(*w).base(), crate::data::Type::Text(_))
                    && !matches!(vars.tp(*w).base(), crate::data::Type::RefVar(_)))
        }
        Value::Text(_) => true,
        // A substring is left out: the C may copy its source into a statement temporary first,
        // which a view would outlive (three corpus programs did).
        Value::Call(d, _) => cx.data.def(*d).name() == "OpGetText",
        _ => false,
    }
}

impl<'a> FnWriter<'a> {
    #[expect(clippy::too_many_lines)]
    pub(crate) fn new(cx: &'a Cx<'a>, d: u32, shapes: Vec<CType>, ret: CType) -> FnWriter<'a> {
        let def = cx.data.def(d);
        let (borrowed, tested) = borrowed_texts(cx, d);
        FnWriter {
            cx,
            d,
            vars: def.variables(),
            code: def.code(),
            shapes,
            params: {
                let vars = def.variables();
                let mut p = vec![false; vars.count() as usize];
                for a in vars.arguments() {
                    p[a as usize] = true;
                }
                p
            },
            links: (0..def.variables().count())
                .map(|v| matches!(def.variables().tp(v).base(), crate::data::Type::RefVar(_)))
                .collect(),
            vr_ptr: vr_ptr_flags(cx, d),
            dc: deferred::DeferredCopies::default(),
            // A store instance's text parameter (`n_app@st1`, `parser/store_text.rs`) is a
            // store link: the IR reads and writes it as the field it names.
            store_links: {
                let vars = def.variables();
                let seed: Vec<u16> = vars
                    .arguments()
                    .into_iter()
                    .filter(|&a| vars.is_store_text_link(a))
                    .collect();
                store_links(cx, def, &seed)
            },
            ret,
            body: String::new(),
            temps: Vec::new(),
            pending: Vec::new(),
            loops: Vec::new(),
            labels: 0,
            headers: Vec::new(),
            grown_headers: std::collections::HashSet::new(),
            pending_grown: Vec::new(),
            kh_passed: std::collections::HashSet::new(),
            nn_seed: Vec::new(),
            push_heads: Vec::new(),
            vr: value::VrFn::of(cx, d).map(Box::new),
            widening: None,
            raw: std::collections::HashMap::new(),
            field_view: false,
            nk_skip: None,
            ap_buf: None,
            ap_views: std::collections::HashSet::new(),
            ap_site: None,
            ap_memo: std::collections::HashMap::new(),
            ds: None,
            ds_site: None,
            nk_call: None,
            plain_chains: Vec::new(),
            chain_guards: 0,
            rec_ptrs: Vec::new(),
            work_alias: None,
            scalars: Vec::new(),
            invariants: Vec::new(),
            hoist_cache: std::collections::HashMap::new(),
            placed: crate::opt::hoist::placed_locals(def.code(), cx.data),
            assumed: Vec::new(),
            range_facts: None,
            cur_line: 0,
            where_told: false,
            adopt: if std::env::var("LOFT_NO_RETBUF_ADOPT").is_ok_and(|v| v != "0") {
                None
            } else {
                crate::opt::ret_adopt::ret_adopt(cx.data, d)
            },
            in_delivery: 0,
            windows: Vec::new(),
            file_holds: Vec::new(),
            borrowed,
            tested,
            lazy: if std::env::var("LOFT_NO_LAZY_SPLIT").is_ok_and(|v| v != "0") {
                std::collections::BTreeMap::new()
            } else {
                crate::opt::lazy_split::lazy_splits(cx.data, d)
            },
            split_tabs: if cx.switches.split_table_disabled {
                std::collections::BTreeMap::new()
            } else {
                crate::opt::hoist::split_tables(cx.data, d)
            },
            tab_alias: std::collections::BTreeMap::new(),
            char_walks: if std::env::var("LOFT_NO_CHAR_WALK").is_ok_and(|v| v != "0") {
                std::collections::BTreeMap::new()
            } else {
                crate::opt::char_walk::char_walks(cx.data, d)
            },
            complete_mints: None,
            fresh_fields: std::collections::HashMap::new(),
            apr: crate::lower::apart_records::RecRoles::default(),
            apr_plan: None,
            apv: crate::lower::apart_vector::VecRoles::default(),
            av: crate::lower::apart_value::Roles::default(),
            av_twin: false,
            av_plan: None,
            av_rb: None,
            decls: Vec::new(),
            frees_old_tuple: false,
            // A parameter arrives bound: its first `Set` is a rebind.
            bound: {
                let vars = def.variables();
                let mut b = vec![false; vars.count() as usize];
                for a in vars.arguments() {
                    b[a as usize] = true;
                }
                b
            },
            gen_elem: None,
            yields: 0,
        }
    }

    /// loft#1254 — a stub with an empty body answers its type's default (`Data::to_default`),
    /// a handle-carried result the null reference, as the bytecode writer's stub path does;
    /// `None` for any other body, or a type with no default.
    fn empty_stub_default(&self) -> Option<Value> {
        use crate::data::Type;
        if !matches!(self.code, Value::Block(b) if b.operators.is_empty()) {
            return None;
        }
        let data = self.cx.data;
        let ret = data.def(self.d).returned();
        if data.has_default(ret).is_err() {
            return None;
        }
        Some(
            if matches!(
                ret.base(),
                Type::Reference(..)
                    | Type::Vector(..)
                    | Type::Sorted(..)
                    | Type::Index(..)
                    | Type::Hash(..)
                    | Type::Radix(..)
                    | Type::Trie(..)
                    | Type::Enum(_, true, _)
            ) {
                Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new())
            } else {
                crate::data::to_default(ret, data)
            },
        )
    }

    /// This is the place twin for `mask` (`twin_name`): each `&` parameter whose bit is set is
    /// a store link — a `loft_ref` to the place — as a `&` local bound to a field is.
    pub(crate) fn places(mut self, mask: u64) -> Self {
        if mask != 0 {
            let def = self.cx.data.def(self.d);
            let seed: Vec<u16> = self
                .vars
                .arguments()
                .iter()
                .enumerate()
                .filter(|&(i, &a)| mask & (1 << i) != 0 || self.vars.is_store_text_link(a))
                .map(|(_, &a)| a)
                .collect();
            self.store_links = store_links(self.cx, def, &seed);
        }
        self
    }

    /// This is an `__inv` twin (`@FR-R-Inputs`): `frame` is the headers its caller hands it,
    /// valid for the call, since an admitted callee grows no store.
    /// An `__ap` twin (`super::AP_BIT`, `@FR-R-AppendTwin`): the return buffer is the caller's
    /// vector, so none of its clears run and it is minted only where the caller handed none;
    /// the result never adopts the buffer (`@FR-R-RetAdopt` binds the result to it and clears
    /// it — the caller's elements).
    pub(crate) fn append_twin(mut self, on: bool) -> Self {
        if on && let Some(b) = crate::opt::append_twin::buffer(self.cx.data, self.d) {
            self.ap_buf = Some(b);
            self.ap_views = crate::opt::append_twin::views(self.vars, b);
            self.adopt = None;
        }
        self
    }

    /// A `__d` twin (`super::DS_BIT`, `@FR-R-Destination`): the return buffer is the caller's
    /// fresh element, a write to one of its scalar fields is the local the twin answers, and
    /// `return buf` answers those locals as a tuple.  The result never adopts the buffer.
    pub(crate) fn destination(mut self, on: bool) -> Self {
        if on {
            self.ds = self.cx.dest_shape(self.d);
            self.adopt = None;
        }
        self
    }

    /// A `__nk` twin (`super::NK_BIT`): the body without its keep statement.
    pub(crate) fn nokeep(mut self, on: bool) -> Result<Self, String> {
        if on {
            self.nk_skip = Some(
                crate::lower::hand_over::keep_statement(self.cx.data, self.d)
                    .ok_or("a __nk twin of a function with no keep statement")?,
            );
        }
        Ok(self)
    }

    /// A `__kh` twin (`super::KH_BIT`): its caller's header `(*khp)`, in the input frame, is a
    /// grown header — derived again through the pointer after every growth statement.
    pub(crate) fn kept_growth(mut self, on: bool) -> Self {
        if on {
            self.grown_headers.insert("(*khp)".to_string());
        }
        self
    }

    /// A `__nn` twin (`super::NN_MASK`): its proof seeds the parameters the mask names.
    pub(crate) fn nn_seed(mut self, mask: u64) -> Self {
        let bits = (mask & super::NN_MASK) >> super::NN_SHIFT;
        self.nn_seed = (0..26u16).filter(|i| bits & (1 << i) != 0).collect();
        self
    }

    pub(crate) fn inputs(
        mut self,
        frame: Vec<(crate::opt::writes::PathKey, String, bool)>,
    ) -> Self {
        if !frame.is_empty() {
            self.headers.push(frame);
        }
        self
    }

    /// The record scalars an `__inv` twin takes as parameters (`@FR-R-Inputs`' scalar clause),
    /// served as a loop's held scalars are: a read `p.f` of one names its parameter.
    pub(crate) fn scalar_inputs(
        mut self,
        frame: Vec<(crate::opt::hoist::ScalarKey, String, CType)>,
    ) -> Self {
        if !frame.is_empty() {
            self.scalars.push(frame);
        }
        self
    }

    /// This function is a generator of `elem`s: written by `write_gen`.
    pub(crate) fn yielding(mut self, elem: CType) -> Self {
        self.gen_elem = Some(elem);
        self
    }

    /// `@FR-R-Inline` counts STATEMENTS: the body's lines without its frame sites and the lines
    /// that only close a block or open its `else` (`LOFT_INLINE_COUNT_BRACES=1` counts those).
    fn statement_count(&self) -> usize {
        let braces =
            crate::env_once!(std::env::var("LOFT_INLINE_COUNT_BRACES").is_ok_and(|v| v != "0"));
        self.body
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !l.contains("loft_frame_site(")
                    && (braces || !matches!(t, "}" | "{" | "};" | "} else {"))
            })
            .count()
    }

    /// The body: the locals, the temporaries and the statements.
    /// The function's C body, and how many statements it holds — its own lines, without the
    /// declarations, the blank lines and the frame-site bookkeeping (`small_function`).
    pub(crate) fn write(mut self) -> Result<(String, usize), String> {
        // The rewrite census (`LOFT_REWRITE_CENSUS`), per function as the native writer counts:
        // the analyses this body reads were settled when it was set up.
        {
            use crate::rewrite_census::fired;
            fired("R-TextBorrow", self.borrowed.iter().filter(|b| **b).count());
            fired("R-CharWalk", self.char_walks.len());
            fired("R-RetAdopt", usize::from(self.adopt.is_some()));
        }
        self.complete_writes_once();
        let code = self.code;
        self.dc = self.deferred_plan();
        let mut flags: Vec<u16> = self.dc.vars.keys().copied().collect();
        flags.sort_unstable();
        for x in flags {
            self.decls.push(format!("uint8_t loft_dcv{x} = 0;"));
        }
        // The return buffer an admitted function's signature dropped is a local nothing binds.
        if let Some(p) = self.vr.as_ref().and_then(|f| f.phantom) {
            self.params[p as usize] = false;
        }
        for (v, tab) in &self.split_tabs {
            self.decls
                .push(format!("loft_stab st{v} = {{ {{ 0, 0, 0 }}, 0, 0, 0 }};"));
            for a in &tab.aliases {
                self.tab_alias.insert(*a, *v);
            }
        }
        self.ds_decls();
        let mut owns: Vec<u16> = self.tested.keys().copied().collect();
        owns.sort_unstable();
        for v in owns {
            self.decls
                .push(format!("loft_text loft_own{v} = {{ 0, 0, 0 }};"));
        }
        self.apr_decls();
        self.apv_decls();
        self.av_decls();
        self.bind_param_ptrs(code);
        if self.ret == CType::Void {
            self.stmt(code, 1)?;
            self.epilogue(1);
        } else if let Some(dflt) = self.empty_stub_default() {
            self.ret_value(Some(&dflt), 1)?;
        } else {
            self.ret_value(Some(code), 1)?;
        }
        let mut out = String::new();
        // An apart record and a push group's element are no records (`apart.rs`).
        for v in (0..self.shapes.len()).filter(|v| !self.params[*v] && !self.apr_holds(*v as u16)) {
            let c = self.shapes[v];
            if self.store_links[v] {
                let name = var_name(self.vars.name(v as u16), v as u16);
                let _ = writeln!(out, "    loft_ref {name} = {{ 0xFFFF, 0, 0 }};");
                continue;
            }
            if self.links[v] {
                let name = var_name(self.vars.name(v as u16), v as u16);
                let _ = writeln!(out, "    {} *{name} = 0;", held(c));
                continue;
            }
            if self.borrowed[v] && c == CType::Text {
                let _ = writeln!(out, "    loft_str {} = {{ 0, 0 }};", self.var(v as u16));
                continue;
            }
            let _ = writeln!(
                out,
                "    {} {} = {};",
                held(c),
                self.var(v as u16),
                self.cx.initial(c)
            );
        }
        // A temporary holds one expression's value between the statement that computes it and
        // the one that reads it, so it is written whole before it is read on every path: one of
        // a plain type takes no initial value, which the compiler could not drop in a large
        // function (cbor's `read_value`: 116 of them, 15 % of its calls).  A temporary that owns
        // or may contain something keeps it — a text (the exit and the statement flush free it),
        // a function value (its closure record), a tuple (a text field is SET in place, which
        // reads and frees the old one).  Checked by clang's flow analysis and MemorySanitizer
        // over the corpus.
        for (t, c) in &self.temps {
            let plain = matches!(
                c,
                CType::Int
                    | CType::Int32
                    | CType::Bool
                    | CType::Ref
                    | CType::Enum
                    | CType::Float
                    | CType::Single
                    | CType::Char
            );
            if plain {
                let _ = writeln!(out, "    {} {t};", held(*c));
            } else {
                let _ = writeln!(out, "    {} {t} = {};", held(*c), self.cx.initial(*c));
            }
        }
        for d in &self.decls {
            let _ = writeln!(out, "    {d}");
        }
        let slots: Vec<String> = self.held_texts().iter().map(|t| format!("&{t}")).collect();
        if !slots.is_empty() {
            let _ = writeln!(out, "    LOFT_HOLD({}, {});", slots.len(), slots.join(", "));
        }
        let statements = self.statement_count();
        out.push_str(&self.body);
        Ok((out, statements))
    }

    /// `v`'s C spelling.  A link holds its source's ADDRESS, so every use dereferences it and
    /// reads the source as it is at that use (`@FR-B-Ref-Read`); a read-only record parameter
    /// passed by pointer (`Cx::vr_ptr_params`) is spelled the same way.
    fn var(&self, v: u16) -> String {
        let name = var_name(self.vars.name(v), v);
        if self.links[v as usize] || self.vr_ptr[v as usize] {
            format!("(*{name})")
        } else {
            name
        }
    }

    /// A text variable this function holds as a `loft_text`: a local, or the caller's
    /// through a link; a plain text parameter is a borrowed `loft_str`.
    fn text_held(&self, v: u16) -> bool {
        self.shapes[v as usize] == CType::Text
            && (!self.params[v as usize] || self.links[v as usize])
            && !self.borrowed[v as usize]
    }

    fn line(&mut self, ind: usize, s: &str) {
        let _ = writeln!(self.body, "{}{s}", "    ".repeat(ind));
    }

    fn temp(&mut self, c: CType) -> String {
        let t = format!("t{}", self.temps.len() + 1);
        self.temps.push((t.clone(), c));
        t
    }

    /// Free the owned text temporaries the statement just finished made.
    fn flush(&mut self, ind: usize) {
        for t in std::mem::take(&mut self.pending) {
            self.line(ind, &format!("loft_text_free(&{t});"));
        }
    }

    /// `@FR-R-FileHold` — give back the cursor of every hold of a loop at `loops` index `from`
    /// or deeper, innermost first: what Rust's guard does by its drop.  Loft has no exception,
    /// so these are every exit there is: the loop's own end (`loop_stmt`), a `break` or a
    /// `continue` to an outer loop, a `return`.
    fn release_holds(&mut self, from: usize, ind: usize) {
        let gone: Vec<String> = self
            .file_holds
            .iter()
            .rev()
            .filter(|(_, _, at)| *at >= from)
            .map(|(_, c, _)| c.clone())
            .collect();
        for c in gone {
            self.line(ind, &format!("loft_fcur_release(&{c}, {c}p);"));
        }
    }

    /// Free every text the function holds: its locals and its temporaries.
    fn epilogue(&mut self, ind: usize) {
        // A `return` from inside a held loop: the cursors go back before the frame does.
        self.release_holds(0, ind);
        let tabs: Vec<u16> = self.split_tabs.keys().copied().collect();
        for v in tabs {
            self.line(ind, &format!("loft_stab_free(&st{v});"));
        }
        for t in self.held_texts() {
            self.line(ind, &format!("loft_text_free(&{t});"));
        }
    }

    /// The texts the function holds, each freed by its epilogue — and released by a halt in a
    /// unit that runs tests (`LOFT_HOLD`, loft#2198): its text locals, the texts its tuple
    /// locals own, its text temporaries and the texts its tuple temporaries hold.
    fn held_texts(&self) -> Vec<String> {
        let mut texts: Vec<String> = (0..self.shapes.len())
            .filter(|&v| {
                !self.params[v]
                    && !self.links[v]
                    && !self.borrowed[v]
                    && self.shapes[v] == CType::Text
            })
            .map(|v| self.var(v as u16))
            .collect();
        // The texts a tuple local owns.
        for v in (0..self.shapes.len()).filter(|&v| !self.params[v] && !self.links[v]) {
            if let CType::Tuple(id) = self.shapes[v] {
                self.tuple_text_leaves(&self.var(v as u16), id, &mut texts);
            }
        }
        texts.extend(
            self.temps
                .iter()
                .filter(|(_, c)| *c == CType::Text)
                .map(|(t, _)| t.clone()),
        );
        // A tested borrow's own text, which its copying path fills.
        let mut owns: Vec<u16> = self.tested.keys().copied().collect();
        owns.sort_unstable();
        texts.extend(owns.into_iter().map(|v| format!("loft_own{v}")));
        // And the texts a tuple temporary holds.
        let tuple_temps: Vec<(String, u32)> = self
            .temps
            .iter()
            .filter_map(|(t, c)| match c {
                CType::Tuple(id) => Some((t.clone(), *id)),
                _ => None,
            })
            .collect();
        for (t, id) in tuple_temps {
            self.tuple_text_leaves(&t, id, &mut texts);
        }
        // And the field buffers of the apart records it owns (`@FR-R-Apart` instance 4).
        texts.extend(self.apr_texts());
        texts.extend(self.apv_texts());
        texts
    }

    /// Before loop `b`: each vector the loop verdict holds (`hoist::hoistable`, native's own —
    /// the vectors the body indexes and the ones its admitted callees index, `@FR-R-Inputs`)
    /// in a `loft_vh`, the frame the loop's element accesses read.  A loop that grows a store
    /// (a push, a mint, a text set) keeps the record and the length, which no growth moves; it
    /// keeps the base only of a vector whose store is proven apart from every store it grows
    /// (`@FR-R-Base`'s growth clause, `StoreFacts`), and derives the others' base again at
    /// each access (`loft_vh_fresh`).  `LOFT_NO_VECTOR_HOIST=1` holds none; the tier switches
    /// are native's (`lower::switches`).
    /// Answers the minted paths the verdict gives a record push header (`mint_pushes`) and the
    /// scalar pushed paths it gives a push header (`pushes`, `@FR-R-Push`), for
    /// [`Self::hold_pushes`] once the loop's window is chosen.
    fn hold_vectors(&mut self, b: &Block, ind: usize) -> Result<Pushed, String> {
        let mut frame = Vec::new();
        let mut mints = Vec::new();
        let mut pushes = Vec::new();
        let mut held_scalars = Vec::new();
        let mut sole_push = false;
        if let Some(verdict) = self.verdict(b) {
            mints.clone_from(&verdict.mint_pushes);
            pushes.clone_from(&verdict.pushes);
            sole_push = verdict.sole_push;
            for (key, getter) in &verdict.scalars {
                // A direct read the loop spells `getter(Var(v), fld)` is what this serves; a
                // path scalar of a callee's input shares its key and its bytes.
                if self.held_scalar(key).is_some() {
                    continue;
                }
                let c = self.shape_of(getter)?;
                if !matches!(
                    c,
                    CType::Int
                        | CType::Float
                        | CType::Single
                        | CType::Bool
                        | CType::Enum
                        | CType::Char
                ) {
                    continue;
                }
                let e = self.nested(|w| w.expr(getter, ind))?;
                let name = format!("s{}", self.decls.len() + 1);
                self.decls
                    .push(format!("{} {name} = {};", held(c), self.cx.initial(c)));
                self.line(ind, &format!("{name} = {};", as_shape(e, c)?));
                held_scalars.push((*key, name, c));
            }
            let bases = self.kept_bases(&verdict);
            for ((path, expr), base) in verdict.vectors.into_iter().zip(bases) {
                // A split table's local, or a walk's alias of one, is never built.
                if self.split_tabs.contains_key(&path.0) || self.tab_alias.contains_key(&path.0) {
                    continue;
                }
                // A path an enclosing frame holds as well — with a base, or without one where
                // this loop would keep none either — is served by that frame (`@FR-R-State`:
                // one holder per path).
                let outer = self
                    .headers
                    .iter()
                    .rev()
                    .flat_map(|f| f.iter())
                    .find(|(p, _, _)| *p == path)
                    .map(|(_, _, b)| *b);
                if outer.is_some_and(|b| b || !base) {
                    continue;
                }
                let e = self.nested(|w| w.expr(&expr, ind))?;
                let h = format!("h{}", self.decls.len() + 1);
                self.decls
                    .push(format!("loft_vh {h} = {{ 0xFFFF, 0, 0, 0 }};"));
                self.line(ind, &format!("{h} = loft_vh_of({});", as_ref(e)?));
                if self.pending_grown.contains(&path) {
                    self.grown_headers.insert(h.clone());
                }
                frame.push((path, h, base));
            }
        }
        {
            use crate::rewrite_census::fired;
            fired("R-Header", frame.len());
            fired("R-Base", frame.iter().filter(|(_, _, b)| *b).count());
            fired(
                "R-Kept",
                frame
                    .iter()
                    .filter(|(_, h, _)| self.grown_headers.contains(h))
                    .count(),
            );
            fired("R-Scalar", held_scalars.len());
        }
        self.headers.push(frame);
        self.scalars.push(held_scalars);
        Ok((mints, pushes, sole_push))
    }

    /// `@FR-R-Invariant` (@PLN157 § V-ao, rustc's `begin_vector_hoist`): each invariant integer
    /// chain loop `b` itself spells (a nested loop's are its own) takes a memo pair — the value,
    /// and whether its first use has evaluated it — whose flag is cleared HERE, at the loop's
    /// prelude, so it is clear on every entry and the compiler peels the first-use test out of
    /// the loop.  The first use evaluates the chain where it stands — the same value, the same
    /// overflow answer — and every later use answers the memo.  A chain an enclosing frame
    /// already memoises is its.  `LOFT_NO_INVARIANT_HOIST=1` memoises none.  Measured on the C
    /// before porting (a checked `a * b` and `a ^ b` in a 1 000-element loop): gcc's own code
    /// motion already hoisted them (the same 401M instructions with the memo), clang's did not
    /// (621M -> 541M, -13 %).
    fn hold_invariants(&mut self, b: &Block, ind: usize) {
        let mut frame = std::collections::HashMap::new();
        if !self.cx.switches.invariant_hoist_disabled && self.gen_elem.is_none() {
            for ch in crate::opt::hoist::invariant_chains(b, self.cx.data, self.vars) {
                if ch
                    .nodes
                    .iter()
                    .any(|a| self.invariants.iter().any(|f| f.contains_key(a)))
                {
                    continue;
                }
                let name = format!("ia{}", self.decls.len() + 1);
                self.decls
                    .push(format!("int64_t {name} = LOFT_NULL; int {name}_set = 0;"));
                self.line(ind, &format!("{name}_set = 0;"));
                let memo = std::rc::Rc::new((name, ch.chain));
                for a in ch.nodes {
                    frame.insert(a, memo.clone());
                }
            }
        }
        crate::rewrite_census::fired("R-Invariant", {
            let mut names: Vec<&String> = frame.values().map(|m| &m.0).collect();
            names.sort();
            names.dedup();
            names.len()
        });
        self.invariants.push(frame);
    }

    /// The memo an enclosing loop keeps for the chain node `v`, if any: the node's address
    /// (and its `Span` wrapper's) is what `hoist::invariant_chains` registered.
    fn invariant_of(&self, v: &Value) -> Option<std::rc::Rc<(String, Value)>> {
        let a = std::ptr::from_ref(v) as usize;
        let b = std::ptr::from_ref(v.unspan()) as usize;
        self.invariants
            .iter()
            .rev()
            .find_map(|f| f.get(&a).or_else(|| f.get(&b)).cloned())
    }

    /// A memoised chain's use: the first use evaluates the chain where it stands — its own
    /// temporaries inside the block, so a later use runs none of them — and records it; every
    /// use answers the memo.  The chain is the memo's own clone, whose nodes no frame holds, so
    /// this cannot re-enter itself.  Under `LOFT_VERIFY` every use evaluates the chain again
    /// and the proof asserts the memo agrees — rustc's `LOFT_HOIST_VERIFY` falsifier.
    fn invariant_use(&mut self, memo: &(String, Value), ind: usize) -> Result<E, String> {
        let (name, chain) = memo;
        self.line(ind, &format!("if (!{name}_set) {{"));
        let e = self.expr(chain, ind + 1)?;
        self.line(ind + 1, &format!("{name} = {};", as_int(e)?));
        self.line(ind + 1, &format!("{name}_set = 1;"));
        self.line(ind, "}");
        self.line(ind, "#ifdef LOFT_VERIFY");
        let fresh = self.expr(chain, ind)?;
        self.line(
            ind,
            &format!(
                "LOFT_PROVED({} == {name}, \"R-Invariant:{name}\");",
                as_int(fresh)?
            ),
        );
        self.line(ind, "#endif");
        Ok(E::new(name.clone(), K::Int))
    }

    /// The local an enclosing loop holds record scalar `key` in, with its shape.
    fn held_scalar(&self, key: &crate::opt::hoist::ScalarKey) -> Option<(String, CType)> {
        self.scalars
            .iter()
            .rev()
            .flat_map(|f| f.iter())
            .find(|(k, _, _)| k == key)
            .map(|(_, n, c)| (n.clone(), *c))
    }

    /// `@FR-R-PushRec`, as native's record push header (`@FR-R-Mint`'s group, the path's one
    /// holder under `@FR-R-State`, re-derived after a growth under `@FR-R-Refresh`): before
    /// loop `b`, a `loft_ph` for each
    /// minted path of `mints` that no window and no enclosing loop serves; the path's mints and
    /// finishes go through it (`Self::pushed_mint`).  `LOFT_NO_RECORD_PUSH=1` admits none, in
    /// the verdict, as on native.
    fn hold_pushes(
        &mut self,
        b: &Block,
        (mints, pushes, sole_push): Pushed,
        ind: usize,
    ) -> Result<(), String> {
        let mut frame = Vec::new();
        // `@FR-R-Push`'s sole-growth clause: the loop's one growth is this path's pushes, so
        // its header writes through the record address it holds.  `LOFT_NO_PUSH_BASE=1`
        // resolves the store per push again.
        let based = sole_push
            && pushes.len() == 1
            && !std::env::var("LOFT_NO_PUSH_BASE").is_ok_and(|v| v != "0");
        // `@FR-R-Push`: a scalar pushed path the verdict admits (never also one of its held
        // vectors) keeps a push header the push itself keeps current; its size is its pushes'.
        for (path, expr) in pushes {
            if self.windows.iter().any(|(p, _, _, _)| *p == path) || self.push_head(&path).is_some()
            {
                continue;
            }
            let Some(size) = self.push_size(b, &path) else {
                continue;
            };
            let e = self.nested(|w| w.expr(&expr, ind))?;
            let h = format!("ph{}", self.decls.len() + 1);
            self.decls
                .push(format!("loft_ph {h} = {{ {{ 0xFFFF, 0, 0 }}, 0, 0, 0, 0 }};"));
            self.line(ind, &format!("{h} = loft_ph_of({}, {size});", as_ref(e)?));
            crate::rewrite_census::fired("R-Push", 1);
            if based {
                crate::rewrite_census::fired("R-Push/sole", 1);
            }
            frame.push((path, h, size, based));
        }
        for (path, expr) in mints {
            if self.windows.iter().any(|(p, _, _, _)| *p == path) || self.push_head(&path).is_some()
            {
                continue;
            }
            let Some(size) = self.mint_size(b, &path) else {
                continue;
            };
            let e = self.nested(|w| w.expr(&expr, ind))?;
            let h = format!("ph{}", self.decls.len() + 1);
            self.decls
                .push(format!("loft_ph {h} = {{ {{ 0xFFFF, 0, 0 }}, 0, 0, 0, 0 }};"));
            self.line(ind, &format!("{h} = loft_ph_of({}, {size});", as_ref(e)?));
            crate::rewrite_census::fired("R-PushRec", 1);
            frame.push((path, h, size, false));
        }
        self.push_heads.push(frame);
        Ok(())
    }

    /// The element size of the scalars loop `b` pushes onto `path`, read off its pushes — `None`
    /// where two pushes disagree, which no admitted path does.
    fn push_size(&self, b: &Block, path: &crate::opt::writes::PathKey) -> Option<u32> {
        let data = self.cx.data;
        let mut size: Option<u32> = None;
        let mut agree = true;
        for op in &b.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(fp) =
                        crate::opt::writes::fused_push(data, data.def(*d).name(), args)
                    && fp.path == *path
                {
                    agree &= size.is_none_or(|s| s == fp.size);
                    size = Some(fp.size);
                }
                false
            });
        }
        size.filter(|_| agree)
    }

    /// The element size of the records loop `b` mints into `path`, read off its first mint.
    fn mint_size(&self, b: &Block, path: &crate::opt::writes::PathKey) -> Option<u32> {
        let data = self.cx.data;
        let mut size = None;
        for op in &b.operators {
            op.any_node(&mut |n| {
                if size.is_none()
                    && let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && let Some(t) = crate::opt::writes::mint_target(
                        data,
                        self.cx.stores,
                        "OpNewRecord",
                        args,
                        self.vars,
                    )
                    && t.path == *path
                {
                    size = Some(u32::from(
                        self.cx.stores.size(self.cx.stores.content(t.vector_tp)),
                    ));
                }
                false
            });
        }
        size
    }

    /// The record push header an enclosing loop binds for `path`, with its element size.
    fn push_head(&self, path: &crate::opt::writes::PathKey) -> Option<(String, u32)> {
        self.push_head_based(path).map(|(h, size, _)| (h, size))
    }

    /// [`Self::push_head`], with whether its pushes write through its record address.
    fn push_head_based(&self, path: &crate::opt::writes::PathKey) -> Option<(String, u32, bool)> {
        self.push_heads
            .iter()
            .rev()
            .flat_map(|f| f.iter())
            .find(|(p, _, _, _)| p == path)
            .map(|(_, h, size, based)| (h.clone(), *size, *based))
    }

    /// A mint, a finish or a length of a path a record push header serves.
    fn pushed_mint(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let data = self.cx.data;
        if name == "OpLengthVector" {
            return Ok(args
                .first()
                .and_then(|v| crate::opt::writes::vector_path(data, v))
                .and_then(|p| self.push_head(&p))
                .map(|(h, _)| E::new(format!("((int64_t) {h}.len)"), K::Int)));
        }
        // A group's reservation is capacity, observably nothing; the header's mint grows the
        // record itself, and a reservation made beside it would claim a record it does not hold.
        if name == "OpPreAllocVector" {
            return Ok(args
                .first()
                .and_then(|v| crate::opt::writes::vector_path(data, v))
                .and_then(|p| self.push_head(&p))
                .map(|_| E::new("((void) 0)", K::Void)));
        }
        if let Some(fp) = crate::opt::writes::fused_push(data, name, args)
            && let Some((h, size, based)) = self.push_head_based(&fp.path)
            && size == fp.size
        {
            let val = self.expr(fp.val, ind)?;
            let (put, bits) = match name {
                "OpPushInt" => ("loft_ph_put8", format!("(uint64_t) {}", as_int(val)?)),
                "OpPushFloat" => ("loft_ph_put8", format!("loft_f64_bits({})", as_float(val)?)),
                "OpPushSingle" => (
                    "loft_ph_put4",
                    format!("loft_f32_bits({})", as_single(val)?),
                ),
                "OpPushInt4" => ("loft_ph_put4", format!("loft_i4_bits({})", as_int(val)?)),
                "OpPushCharacter" => ("loft_ph_put4", format!("(uint32_t) {}", as_char(val)?)),
                "OpPushBoolean" => ("loft_ph_put1", format!("(uint8_t) {}", as_bool(val)?)),
                "OpPushEnum" => ("loft_ph_put1", format!("(uint8_t) {}", as_enum(val)?)),
                "OpPushByte" => {
                    let min = Self::literal(
                        fp.bias.ok_or("a byte push without its minimum")?,
                        "byte minimum",
                    )?;
                    (
                        "loft_ph_put1",
                        format!("loft_byte_raw({min}, {})", as_int(val)?),
                    )
                }
                _ => return Ok(None),
            };
            let proof_at = format!("{}:{h}", data.def(self.d).original_name());
            let (same, put) = if based {
                ("loft_phb_same", put.replacen("loft_ph_put", "loft_ph_putb", 1))
            } else {
                ("loft_ph_same", put.to_string())
            };
            return Ok(Some(E::new(
                format!(
                    "(LOFT_PROVED({same}({h}, {size}), {}), {put}(&{h}, {bits}))",
                    c_string(&proof_at)
                ),
                K::Void,
            )));
        }
        if !matches!(name, "OpNewRecord" | "OpFinishRecord") {
            return Ok(None);
        }
        let Some(t) = crate::opt::writes::mint_target(data, self.cx.stores, name, args, self.vars)
        else {
            return Ok(None);
        };
        let Some((h, size)) = self.push_head(&t.path) else {
            return Ok(None);
        };
        if name == "OpFinishRecord" {
            return Ok(Some(E::new(format!("loft_ph_finish(&{h})"), K::Void)));
        }
        let elem = self.cx.stores.content(t.vector_tp);
        let ptp = u16::try_from(Self::literal(&args[1], "type")?)
            .map_err(|_| "a type number out of range")?;
        let proof_at = format!("{}:{h}", data.def(self.d).original_name());
        let e = self.temp(CType::Ref);
        self.line(
            ind,
            &format!(
                "{e} = (LOFT_PROVED(loft_ph_same({h}, {size}), {}), loft_ph_new(&{h}, {size}));",
                c_string(&proof_at)
            ),
        );
        if !self.mint_complete(ptp) {
            let (image, n) = self.cx.default_image(elem);
            self.line(ind, &format!("loft_fill({e}, {image}, {n});"));
        }
        Ok(Some(E::new(e, K::Ref)))
    }

    /// The store facts the verdict proves stores apart with (`hoist::StoreFacts`), under
    /// the versioned copy's `assumed` pairs; `None` under `LOFT_NO_DISTINCT_GROWTH=1`.
    fn store_facts(&self) -> Option<crate::opt::hoist::StoreFacts<'_>> {
        (!self.cx.switches.distinct_growth_disabled).then(|| crate::opt::hoist::StoreFacts {
            vars: self.vars,
            placed: &self.placed,
            adopted: self.adopt.as_ref().map(|a| (a.v, a.vdb)),
            owned: None,
            assumed: &self.assumed,
        })
    }

    /// Loop `b`'s header verdict (`hoist::hoistable`), or `None` where none is asked: in a
    /// generator, whose locals live in its frame between resumes, or under
    /// `LOFT_NO_VECTOR_HOIST=1`.  The C keeps a header only where the verdict says so, so it
    /// keeps what native's gate enforces: `@FR-R-InPlace` (only in-place sets run in a held
    /// loop) and `@FR-R-Callee` (a callee is admitted as its own writes are).
    fn verdict(&mut self, b: &Block) -> Option<crate::opt::hoist::LoopHoist> {
        // A twin and a region body (`@FR-R-Apart` instance 2) are emitted with the hoists off.
        if self.gen_elem.is_some()
            || std::env::var("LOFT_NO_VECTOR_HOIST").is_ok_and(|v| v != "0")
            || self.av_any()
        {
            return None;
        }
        let sw = self.cx.switches;
        let tiers = crate::opt::writes::HoistTiers {
            in_place: !sw.write_hoist_disabled,
            scalars: !sw.scalar_hoist_disabled,
            push: !sw.push_hoist_disabled,
            mint: !sw.mint_hoist_disabled,
            record_push: !sw.record_push_disabled,
            heap_push: !sw.heap_record_push_disabled,
            rebound_movers: !sw.rebound_mover_disabled,
            text_set: false,
        };
        let mut cache = std::mem::take(&mut self.hoist_cache);
        let facts = self.store_facts();
        let memo = &mut *self.cx.hoist_memo.borrow_mut();
        let v = crate::opt::hoist::hoistable(
            b,
            self.cx.data,
            self.cx.stores,
            self.d,
            &mut cache,
            &mut memo.writes,
            tiers,
            (!sw.callee_inputs_disabled).then_some(&mut memo.inputs),
            None,
            facts.as_ref(),
        );
        // `@FR-R-Header`'s rebind clause: a bare vector the loop rebinds by navigating, held
        // apart and derived again after each rebind (`refresh_rebound_header`).
        let mut v = v;
        let rebound = crate::opt::hoist::rebound_apart_headers(
            b,
            self.cx.data,
            self.d,
            facts.as_ref(),
            &mut cache,
            None,
        );
        // Held alone — as the apart clause holds its own — the stores they live in grow
        // nowhere in the loop, so each carries its base; beside a verdict that holds other
        // paths the base is that verdict's to judge (`kept_bases`).
        let alone = v.vectors.is_empty()
            && v.pushes.is_empty()
            && v.mint_pushes.is_empty()
            && v.scalars.is_empty();
        for (path, expr) in rebound {
            if !v.vectors.iter().any(|(p, _)| *p == path) {
                v.vectors.push((path, expr));
            }
        }
        if alone && !v.vectors.is_empty() {
            v.growth_free = true;
        }
        // `@FR-R-Header`'s growth half: in a loop that holds nothing but apart headers, a bare
        // vector this activation owns alone that the loop grows through statements of its own
        // level, held with its base and derived again after each (`refresh_grown_header`).
        let mut pending = Vec::new();
        if v.pushes.is_empty()
            && v.mint_pushes.is_empty()
            && v.scalars.is_empty()
            && (v.vectors.is_empty() || v.growth_free)
        {
            let grown = crate::opt::hoist::grown_apart_headers(
                b,
                self.cx.data,
                self.d,
                facts.as_ref(),
                &mut cache,
                None,
            );
            for (path, expr) in grown {
                if !v.vectors.iter().any(|(p, _)| *p == path) {
                    pending.push(path.clone());
                    v.vectors.push((path, expr));
                    v.growth_free = true;
                }
            }
        }
        self.hoist_cache = cache;
        // `@FR-R-Apart`: a path rooted at a value held apart names no record, so it takes no
        // header, base, push header or held scalar; the loop's other paths keep theirs.
        // `@FR-R-DeferredCopy`: a deferred local, and a view derived from one, moves to its copy
        // at a copy point, so no header is held for a path rooted at either.
        if !self.dc.vars.is_empty() {
            v.vectors.retain(|(p, _)| !self.dc_root(p.0));
            v.pushes.retain(|(p, _)| !self.dc_root(p.0));
            v.mint_pushes.retain(|(p, _)| !self.dc_root(p.0));
        }
        if self.apr_any() {
            v.vectors.retain(|(p, _)| !self.apr_holds(p.0));
            v.pushes.retain(|(p, _)| !self.apr_holds(p.0));
            v.mint_pushes.retain(|(p, _)| !self.apr_holds(p.0));
            v.scalars.retain(|(k, _)| !self.apr_holds(k.0));
        }
        self.pending_grown = pending;
        Some(v)
    }

    /// Per held vector of `v`, whether its base stays valid for the loop's extent: every one
    /// in a loop that grows no store, else each whose store is proven apart from every store
    /// the loop grows (`StoreFacts::distinct`), as native's `begin_vector_hoist` decides.
    fn kept_bases(&self, v: &crate::opt::hoist::LoopHoist) -> Vec<bool> {
        if v.growth_free {
            return vec![true; v.vectors.len()];
        }
        let facts = self.store_facts();
        v.vectors
            .iter()
            .map(|(p, _)| {
                !v.movers.is_empty()
                    && facts
                        .as_ref()
                        .is_some_and(|f| v.movers.iter().all(|m| f.distinct(p.0, *m)))
            })
            .collect()
    }

    /// `@FR-R-Alias`'s versioned clause, as native's `distinct_version`: when loop `lp` grows
    /// this function's return buffer and would hold more headers if each parameter's store were
    /// apart from the buffer's, answer the pairs to test at run time and the pairs the tested
    /// copy may assume apart.  `None` for a loop that gains nothing, cannot be copied (a
    /// `yield`, a `par`, a function-value call), or under `LOFT_NO_DISTINCT_VERSION=1`.
    #[expect(clippy::type_complexity, reason = "two pair lists, named in the doc")]
    fn distinct_version(&mut self, lp: &Block) -> Option<(Vec<(u16, u16)>, Vec<(u16, u16)>)> {
        let sw = self.cx.switches;
        if sw.distinct_version_disabled
            || sw.distinct_growth_disabled
            || !self.assumed.is_empty()
            || lp.operators.iter().any(|op| {
                op.any_node(&mut |n| {
                    matches!(
                        n,
                        Value::Yield(_) | Value::Parallel(_) | Value::CallRef(_, _)
                    )
                })
            })
        {
            return None;
        }
        let data = self.cx.data;
        let rb = crate::opt::attrs::retbuf_var(data, self.d)?;
        let adopted = self
            .adopt
            .as_ref()
            .filter(|a| a.buf == rb)
            .map(|a| (a.v, a.vdb));
        let bufs: Vec<u16> = std::iter::once(rb)
            .chain(adopted.into_iter().flat_map(|(v, vdb)| [v, vdb]))
            .collect();
        let tested = adopted.map_or(rb, |(v, _)| v);
        let vars = self.vars;
        let params: Vec<u16> = (0..vars.count())
            .filter(|&v| vars.is_argument(v) && v != rb && crate::data::is_dbref(vars.tp(v).base()))
            .collect();
        if params.is_empty() {
            return None;
        }
        let plain = self.verdict(lp)?;
        let grows = crate::opt::hoist::mover_roots(&lp.operators, data, self.cx.stores, vars)
            .iter()
            .any(|&m| chain_reaches(vars, m, &bufs));
        if plain
            .pushes
            .iter()
            .chain(plain.mint_pushes.iter())
            .all(|(p, _)| !bufs.contains(&p.0))
            && !plain.movers.iter().any(|m| bufs.contains(m))
            && !grows
        {
            return None;
        }
        let assumed: Vec<(u16, u16)> = params
            .iter()
            .flat_map(|&p| bufs.iter().map(move |&b| (p, b)))
            .collect();
        self.assumed.clone_from(&assumed);
        let versioned = self.verdict(lp);
        self.assumed.clear();
        let versioned = versioned?;
        // A path counts only when no enclosing frame holds it already.
        let mut gained: Vec<u16> = Vec::new();
        for (p, _) in &versioned.vectors {
            if !plain.vectors.iter().any(|(q, _)| q == p)
                && self.held_path(p).is_none()
                && let Some(r) = param_root(vars, p.0, &params)
                && !gained.contains(&r)
            {
                gained.push(r);
            }
        }
        if gained.is_empty() {
            return None;
        }
        let mut tests: Vec<(u16, u16)> = Vec::new();
        let mut pairs: Vec<(u16, u16)> = Vec::new();
        for r in gained {
            if !tests.contains(&(r, tested)) {
                tests.push((r, tested));
            }
            for &b in &bufs {
                if !pairs.contains(&(r, b)) {
                    pairs.push((r, b));
                }
            }
        }
        Some((tests, pairs))
    }

    /// `@FR-R-PushFill`'s window clause, as the Rust writer's `push_reserve`: before a COUNTED
    /// push loop (`hoist::push_loop`) whose pushes nothing else in the body can observe
    /// (`hoist::push_window_ok`: the root exclusive, the other statements naming no store it
    /// may view and writing none), reserve the loop's pushes once and open a `loft_win` on the
    /// vector; every push of the path then goes through it, its per-push pre-allocation is
    /// the reservation's, and the length is written once at the loop's exit.  A reservation
    /// is capacity, observably nothing.  `LOFT_NO_PUSH_FILL=1` / `LOFT_NO_PUSH_WINDOW=1` open
    /// none, as on native.  Answers whether a window was opened.
    fn open_window(&mut self, lp: &Block, ind: usize) -> Result<bool, String> {
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        if self.gen_elem.is_some()
            || env_on("LOFT_NO_PUSH_FILL")
            || env_on("LOFT_NO_PUSH_WINDOW")
            || self.av_any()
        {
            return Ok(false);
        }
        let Some(p) = self.window_loop(lp) else {
            return self.deep_window(lp, ind);
        };
        // A push window on a value held apart (`@FR-R-Apart`) has no record to open.
        if self.apr_holds(p.path.0) {
            return Ok(false);
        }
        let n = self.decls.len() + 1;
        let (w, pv) = (format!("pw{n}"), format!("pv{n}"));
        self.decls
            .push(format!("loft_win {w} = {{ 0, 0, 0, 0, 0xFFFF }};"));
        self.decls
            .push(format!("loft_ref {pv} = {{ 0xFFFF, 0, 0 }};"));
        let vector = self.expr(&p.vector, ind)?;
        self.line(ind, &format!("{pv} = {};", as_ref(vector)?));
        // The trip count: the range's end less its start (the `next` counter, or the index
        // the loop steps from plus one), plus one for an inclusive range — checked, so a
        // count that does not fit reserves nothing.
        let hi = self.expr(p.hi, ind)?;
        let hi = as_int(hi)?;
        let lo = match p.next_var {
            Some(nx) => self.var(nx),
            None => format!("loft_add({}, INT64_C(1))", self.var(p.index_var)),
        };
        let incl = u8::from(p.inclusive);
        self.flush(ind);
        self.line(
            ind,
            &format!(
                "loft_vec_reserve_more({pv}, loft_mul(loft_add(loft_sub({hi}, {lo}), INT64_C({incl})), INT64_C({})), {});",
                p.pushes, p.size
            ),
        );
        self.line(ind, &format!("{w} = loft_win_of({pv}, {});", p.size));
        self.windows.push((p.path, w, pv, p.size));
        Ok(true)
    }

    /// `@FR-R-PushFill`'s HASH CLAUSE, as native's `hash_reserve`: a counted loop inserting
    /// records into one hash local (`hoist::hash_fill_loop`) reserves its table for
    /// `len(h) + inserts × trip count` before it runs, so the fill never rehashes on the way.
    /// A reservation is capacity only: length, entries and order are the plain fill's.  An
    /// absent hash, a count that does not fit and an empty range reserve nothing.
    /// `LOFT_NO_PUSH_FILL=1` / `LOFT_NO_HASH_RESERVE=1` reserve nothing, as on native.
    fn hash_reserve(&mut self, lp: &Block, ind: usize) -> Result<(), String> {
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        if self.gen_elem.is_some() || env_on("LOFT_NO_PUSH_FILL") || env_on("LOFT_NO_HASH_RESERVE")
        {
            return Ok(());
        }
        let Ok((hf, hi)) = crate::opt::hoist::hash_fill_loop(lp, self.cx.data, self.cx.stores)
        else {
            return Ok(());
        };
        let hi = as_int(self.expr(hi, ind)?)?;
        let lo = match hf.next {
            Some(nx) => self.var(nx),
            None => format!("loft_add({}, INT64_C(1))", self.var(hf.index)),
        };
        let incl = u8::from(hf.inclusive);
        let h = self.var(hf.hash);
        // The table row the reservation reads (its keys and element stride).
        self.cx.name_types();
        self.cx.describe_type(hf.tp);
        self.flush(ind);
        self.line(
            ind,
            &format!(
                "{{ int64_t pn = loft_add(loft_sub({hi}, {lo}), INT64_C({incl})); if (pn > 0 && pn != LOFT_NULL && {h}.store_nr != 0xFFFF && {h}.rec != 0) loft_hash_reserve(loft_types, {}, {h}, loft_add(loft_hash_len({h}), loft_mul(pn, INT64_C({})))); }}",
                hf.tp, hf.per_pass
            ),
        );
        crate::rewrite_census::fired("R-PushFill/hash", 1);
        Ok(())
    }

    /// `@FR-R-PushFill`'s DEEP window clause, as native's `deep_window`: a loop whose scalar
    /// pushes, at any depth, reach ONE path no enclosing window holds, where
    /// `hoist::deep_window_ok` admits it — the root exclusive, nothing else in the body naming
    /// it or writing a store, no exit but the loop's own end — pushes through a window opened
    /// with no reservation: a push that does not fit takes the window's growth arm.
    fn deep_window(&mut self, lp: &Block, ind: usize) -> Result<bool, String> {
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        if env_on("LOFT_NO_DEEP_WINDOW") {
            return Ok(false);
        }
        let data = self.cx.data;
        let mut paths: Vec<(crate::opt::writes::PathKey, Value, u32)> = Vec::new();
        for op in &lp.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(fp) =
                        crate::opt::writes::fused_push(data, data.def(*d).name(), args)
                    && !paths.iter().any(|(p, _, _)| *p == fp.path)
                    && !self.apr_holds(fp.path.0)
                {
                    paths.push((fp.path, fp.vector.clone(), fp.size));
                }
                false
            });
        }
        let [(path, vector, size)] = paths.as_slice() else {
            return Ok(false);
        };
        if self.windows.iter().any(|(p, _, _, _)| p == path) {
            return Ok(false);
        }
        let mut cache = std::mem::take(&mut self.hoist_cache);
        let facts = self.store_facts();
        let ok = crate::opt::hoist::deep_window_ok(
            lp,
            path,
            data,
            self.d,
            Some(self.cx.stores),
            &mut cache,
            facts.as_ref(),
        );
        self.hoist_cache = cache;
        if ok.is_err() {
            return Ok(false);
        }
        let (path, vector, size) = (path.clone(), vector.clone(), *size);
        let n = self.decls.len() + 1;
        let (w, pv) = (format!("pw{n}"), format!("pv{n}"));
        self.decls
            .push(format!("loft_win {w} = {{ 0, 0, 0, 0, 0xFFFF }};"));
        self.decls
            .push(format!("loft_ref {pv} = {{ 0xFFFF, 0, 0 }};"));
        let v = self.expr(&vector, ind)?;
        self.line(ind, &format!("{pv} = {};", as_ref(v)?));
        self.flush(ind);
        self.line(ind, &format!("{w} = loft_win_of({pv}, {size});"));
        crate::rewrite_census::fired("R-PushFill", 1);
        self.windows.push((path, w, pv, size));
        Ok(true)
    }

    /// The counted loop `lp` a window is admitted for: scalar pushes (`hoist::push_loop`,
    /// `push_window_ok`), or record mint groups (`hoist::mint_loop`, `mint_window_ok`, under
    /// `LOFT_NO_RECORD_PUSH` / `LOFT_NO_HEAP_RECORD_PUSH` as on native).  A path an enclosing
    /// loop holds a window on is served by that window: a reservation would move its record.
    fn window_loop<'b>(&mut self, lp: &'b Block) -> Option<WindowLoop<'b>> {
        use crate::opt::push_window as hoist;
        let env_on = |k: &str| std::env::var(k).is_ok_and(|v| v != "0");
        let data = self.cx.data;
        let open = |w: &Self, path: &crate::opt::writes::PathKey| {
            w.windows.iter().any(|(p, _, _, _)| p == path)
        };
        if let Some(p) = hoist::push_loop(lp, data) {
            if p.fill.is_some() || open(self, &p.path) {
                return None;
            }
            hoist::push_window_ok(
                lp,
                &p,
                data,
                self.d,
                Some(self.cx.stores),
                &mut self.hoist_cache,
            )
            .ok()?;
            return Some(WindowLoop {
                path: p.path,
                vector: std::borrow::Cow::Borrowed(p.vector),
                hi: p.hi,
                index_var: p.index_var,
                next_var: p.next_var,
                inclusive: p.inclusive,
                pushes: p.pushes,
                size: p.size,
            });
        }
        if env_on("LOFT_NO_RECORD_PUSH") {
            return None;
        }
        let (m, hi) = hoist::mint_loop(
            lp,
            data,
            self.cx.stores,
            self.vars,
            !env_on("LOFT_NO_HEAP_RECORD_PUSH"),
        )
        .ok()?;
        if open(self, &m.path) {
            return None;
        }
        hoist::mint_window_ok(
            lp,
            &m,
            data,
            self.d,
            Some(self.cx.stores),
            &mut self.hoist_cache,
        )
        .ok()?;
        Some(WindowLoop {
            path: m.path,
            vector: std::borrow::Cow::Owned(m.vector),
            hi,
            index_var: m.index_var,
            next_var: m.next_var,
            inclusive: m.inclusive,
            pushes: m.mints_per_pass,
            size: m.size,
        })
    }

    /// `@FR-R-CompleteWrite` — does every mint of element type `tp` in this function write it
    /// completely?  Then the default image a fresh element is filled with writes nothing that
    /// survives, and the copy is left out, as native calls `OpNewRecordNP`.  A record copy into
    /// a field of such an element writes that field's default for a null source
    /// (`fresh_null_arm`, loft#2044), so the group covers the field either way.
    fn mint_complete(&mut self, tp: u16) -> bool {
        if std::env::var("LOFT_NO_COMPLETE_WRITE").is_ok_and(|v| v != "0") {
            return false;
        }
        self.complete_writes_once();
        self.complete_mints
            .as_ref()
            .is_some_and(|m| m.contains(&tp))
    }

    /// Is the copy with these arguments one into a field of an element being built?
    fn is_fresh_field_copy(&mut self, args: &[Value]) -> bool {
        self.complete_writes_once();
        self.fresh_fields.contains_key(&(args.as_ptr() as usize))
    }

    fn complete_writes_once(&mut self) {
        if self.complete_mints.is_none() {
            let c = crate::opt::ret_adopt::complete_writes(self.cx.data, self.cx.stores, self.d);
            crate::rewrite_census::fired("R-CompleteWrite", c.db_vars.len() + c.mint_tps.len());
            self.fresh_fields = c.fresh_fields;
            self.complete_mints = Some(c.mint_tps);
        }
    }

    /// `@FR-R-CompleteWrite`'s null arm — `@C137`'s fresh-field arm (loft#2044), as native's
    /// `OpCopyRecordEmitter::fresh_field`: a
    /// record copy into a field of an element a literal group is building writes, for a null
    /// source, the field type's default — what the prefill would have left there — so the
    /// element needs no prefill.  The C statement for a null `src` into `dst`, or `None` for a
    /// copy that is no such field's.
    fn fresh_null_arm(&mut self, args: &[Value], dst: &str) -> Option<String> {
        self.complete_writes_once();
        let sub = *self.fresh_fields.get(&(args.as_ptr() as usize))?;
        let (image, n) = self.cx.default_image(sub);
        Some(format!("loft_fill({dst}, {image}, {n});"))
    }

    /// A push, or its pre-allocation, onto a path an enclosing loop holds a window on: the
    /// element's bytes as the ordinary push writes them, through the window.  `None` for any
    /// other operator.
    fn windowed(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let data = self.cx.data;
        let by_path = |w: &Self, path: &crate::opt::writes::PathKey| {
            w.windows
                .iter()
                .find(|(p, _, _, _)| p == path)
                .map(|(_, w, pv, size)| (w.clone(), pv.clone(), *size))
        };
        let window_of = |w: &Self, v: &Value| {
            let path = crate::opt::writes::vector_path(data, v)?;
            by_path(w, &path)
        };
        if name == "OpPreAllocVector" {
            return Ok(args
                .first()
                .and_then(|v| window_of(self, v))
                .map(|_| E::new("((void) 0)", K::Void)));
        }
        if matches!(name, "OpNewRecord" | "OpFinishRecord")
            && let Some(t) =
                crate::opt::writes::mint_target(data, self.cx.stores, name, args, self.vars)
            && let Some((w, pv, size)) = by_path(self, &t.path)
        {
            if name == "OpFinishRecord" {
                return Ok(Some(E::new(
                    format!("loft_win_finish(&{w}, {pv}, {size})"),
                    K::Void,
                )));
            }
            let elem = self.cx.stores.content(t.vector_tp);
            let ptp = u16::try_from(Self::literal(&args[1], "type")?)
                .map_err(|_| "a type number out of range")?;
            let site = format!("{}:{w}", data.def(self.d).original_name());
            let e = self.temp(CType::Ref);
            self.line(
                ind,
                &format!(
                    "{e} = (LOFT_PROVED(loft_win_same({w}, {pv}, {size}), {}), loft_win_slot(&{w}, {pv}, {size}));",
                    c_string(&site)
                ),
            );
            if !self.mint_complete(ptp) {
                let (image, n) = self.cx.default_image(elem);
                self.line(ind, &format!("loft_fill({e}, {image}, {n});"));
            }
            return Ok(Some(E::new(e, K::Ref)));
        }
        let Some(fp) = crate::opt::writes::fused_push(data, name, args) else {
            return Ok(None);
        };
        let Some((w, pv, _)) = window_of(self, fp.vector) else {
            return Ok(None);
        };
        let val = self.expr(fp.val, ind)?;
        let (put, bits) = match name {
            "OpPushInt" => ("loft_win_put8", format!("(uint64_t) {}", as_int(val)?)),
            "OpPushFloat" => (
                "loft_win_put8",
                format!("loft_f64_bits({})", as_float(val)?),
            ),
            "OpPushSingle" => (
                "loft_win_put4",
                format!("loft_f32_bits({})", as_single(val)?),
            ),
            "OpPushInt4" => ("loft_win_put4", format!("loft_i4_bits({})", as_int(val)?)),
            "OpPushCharacter" => ("loft_win_put4", as_char(val)?),
            "OpPushBoolean" => ("loft_win_put1", as_bool(val)?),
            "OpPushEnum" => ("loft_win_put1", as_enum(val)?),
            "OpPushByte" => {
                let min = Self::literal(
                    fp.bias.ok_or("a byte push without its minimum")?,
                    "byte minimum",
                )?;
                (
                    "loft_win_put1",
                    format!("loft_byte_raw({min}, {})", as_int(val)?),
                )
            }
            _ => return Ok(None),
        };
        let site = format!("{}:{w}", data.def(self.d).original_name());
        Ok(Some(E::new(
            format!(
                "(LOFT_PROVED(loft_win_same({w}, {pv}, {}), {}), {put}(&{w}, {pv}, {bits}))",
                fp.size,
                c_string(&site)
            ),
            K::Void,
        )))
    }

    /// `(R-Header)`'s proof site for a held access: the vector still is what `held` holds.
    /// Checked only under `LOFT_VERIFY`; `((void) 0)` when the vector is not a plain place.
    fn held_proof(&self, v: &Value, held: &str) -> String {
        match self.place_of(v) {
            Some(place) => {
                let site = format!("{}:{held}", self.cx.data.def(self.d).original_name());
                format!(
                    "LOFT_PROVED(loft_vh_same({held}, {place}), {})",
                    c_string(&site)
                )
            }
            None => "((void) 0)".into(),
        }
    }

    /// A vector path as a C expression with no statement before it: a variable, or fields of one.
    fn place_of(&self, v: &Value) -> Option<String> {
        match v.unspan() {
            Value::Var(var) if self.shapes[*var as usize] == CType::Ref => Some(self.var(*var)),
            Value::Call(d, a) if a.len() == 3 && self.cx.data.def(*d).name() == "OpGetField" => {
                let Value::Int(fld) = a[1].unspan() else {
                    return None;
                };
                Some(format!("loft_field({}, {fld})", self.place_of(&a[0])?))
            }
            _ => None,
        }
    }

    /// The `loft_vh` an enclosing loop holds for vector expression `v`, if any, as the
    /// expression an access reads: the header itself, or under growth the header with its
    /// base derived again (`loft_vh_fresh`).
    fn held_vector(&self, v: &Value) -> Option<String> {
        let path = crate::opt::writes::vector_path(self.cx.data, v)?;
        self.held_path(&path)
    }

    fn held_path(&self, path: &crate::opt::writes::PathKey) -> Option<String> {
        self.headers
            .iter()
            .rev()
            .flat_map(|f| f.iter())
            .find(|(p, _, _)| p == path)
            .map(|(_, h, base)| {
                if *base {
                    h.clone()
                } else {
                    format!("loft_vh_fresh({h})")
                }
            })
    }

    /// An element access of a held vector: `OpGetVector(v, size, i)` itself, or a scalar read
    /// or in-place write of a field of one — through the held base.  `None` for any other.
    fn held_access(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let elem = |v: &Value| -> Option<(Value, Value, Value)> {
            match v.unspan() {
                Value::Call(d, a) if a.len() == 3 => {
                    let n = self.cx.data.def(*d).name();
                    (n == "OpGetVector" || n == "OpGetVectorNullable")
                        .then(|| (a[0].clone(), a[1].clone(), a[2].clone()))
                }
                _ => None,
            }
        };
        match (name, args) {
            ("OpGetVector" | "OpGetVectorNullable", [v, size, idx]) => {
                let Some(h) = self.held_vector(v) else {
                    return Ok(None);
                };
                let size = Self::literal(size, "element size")?;
                let i = self.expr(idx, ind)?;
                let proof = self.held_proof(v, &h);
                Ok(Some(E::new(
                    format!("({proof}, loft_vh_get({h}, {size}, {}))", as_int(i)?),
                    K::Ref,
                )))
            }
            ("OpLengthVector", [v]) => Ok(self
                .held_vector(v)
                .map(|h| E::new(format!("{h}.len"), K::Int))),
            ("OpGetInt" | "OpGetFloat", [inner, fld]) => {
                let Some((vec, size, idx)) = elem(inner) else {
                    return Ok(None);
                };
                let Some(held) = self.held_vector(&vec) else {
                    return Ok(None);
                };
                let size = Self::literal(&size, "element size")?;
                let fld = Self::literal(fld, "field")?;
                let index = self.expr(&idx, ind)?;
                let (f, k) = if name == "OpGetInt" {
                    ("loft_vh_get_int", K::Int)
                } else {
                    ("loft_vh_get_float", K::Float)
                };
                let proof = self.held_proof(&vec, &held);
                Ok(Some(E::new(
                    format!("({proof}, {f}({held}, {size}, {}, {fld}))", as_int(index)?),
                    k,
                )))
            }
            ("OpSetInt" | "OpSetFloat", [inner, fld, val]) => {
                let Some((vec, size, idx)) = elem(inner) else {
                    return Ok(None);
                };
                let Some(held) = self.held_vector(&vec) else {
                    return Ok(None);
                };
                let size = Self::literal(&size, "element size")?;
                let fld = Self::literal(fld, "field")?;
                let (index, value) = self.two(name, &[&idx, val], ind)?;
                let (f, value) = if name == "OpSetInt" {
                    ("loft_vh_set_int", as_int(value)?)
                } else {
                    ("loft_vh_set_float", as_float(value)?)
                };
                let proof = self.held_proof(&vec, &held);
                Ok(Some(E::new(
                    format!(
                        "({proof}, {f}({held}, {size}, {}, {fld}, {value}))",
                        as_int(index)?
                    ),
                    K::Void,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The loop label `depth` loops out from the innermost.
    fn label(&self, depth: u16) -> Result<u32, String> {
        let n = self.loops.len();
        if (depth as usize) < n {
            Ok(self.loops[n - 1 - depth as usize])
        } else {
            Err(format!("a break or continue {depth} loops out of {n}"))
        }
    }

    /// Return `v`'s value (or nothing): copy it out, free the function's texts, return.
    fn ret_value(&mut self, v: Option<&Value>, ind: usize) -> Result<(), String> {
        let v = v.filter(|v| !matches!(v.unspan(), Value::Null));
        match (self.ret, v) {
            (CType::Void, Some(v)) => {
                self.stmt(v, ind)?;
                self.epilogue(ind);
                self.line(ind, "return;");
            }
            (CType::Void, None) => {
                self.epilogue(ind);
                self.line(ind, "return;");
            }
            (c, v) => {
                let r = self.temp(c);
                match v {
                    Some(v) => self.assign(v, &r, c, ind)?,
                    None => self.null_into(&r, c, ind),
                }
                self.ret_held(&r, ind);
            }
        }
        Ok(())
    }

    /// Return the value temporary `r` already holds, of the function's result shape.
    fn ret_held(&mut self, r: &str, ind: usize) {
        // The result leaves the function: its text buffers — its own, or the ones a tuple
        // holds — move out before the epilogue frees every text the function holds.
        let mut moved = Vec::new();
        match self.ret {
            CType::Text => moved.push(r.to_string()),
            CType::Tuple(id) => self.tuple_text_leaves(r, id, &mut moved),
            _ => {}
        }
        if moved.is_empty() {
            self.epilogue(ind);
            self.line(ind, &format!("return {r};"));
        } else {
            self.line(ind, &format!("{} {r}_out = {r};", held(self.ret)));
            for m in moved {
                self.line(ind, &format!("{m}.p = 0;"));
            }
            self.epilogue(ind);
            self.line(ind, &format!("return {r}_out;"));
        }
    }

    /// A value the scopes pass left for the bytecode's evaluation stack: `[value, frees…,
    /// return null]`, where `OpReturn` answers the value still on the stack top.  The
    /// positions of the value and of the return, when `ops` ends that way; the native
    /// writer's `detect_ref_tail_capture` is the same reading.
    fn tail_capture(&self, ops: &[Value]) -> Option<(usize, usize)> {
        if self.ret == CType::Void {
            return None;
        }
        let ret = ops
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)))?;
        let Value::Return(x) = ops[ret].unspan() else {
            return None;
        };
        if !matches!(x.unspan(), Value::Null) && !self.is_free(x) {
            return None;
        }
        let at = ops[..ret]
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)) && !self.is_free(o))?;
        (self.shape_of(&ops[at]).ok() == Some(self.ret)).then_some((at, ret))
    }

    /// A scope-exit free, or the store lock the cleanup window may hold.
    fn is_free(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Call(d, _) => {
                let name = self.cx.data.def(*d).name();
                // A drop cascade releases as a free does: a closure's (`OpDropFnRef`) and a
                // record type's (`t_<n><Type>_OpDrop` / `_OpDropAll`), each ahead of its
                // store's free.
                name.starts_with("OpFree")
                    || name == "OpDropFnRef"
                    || (name.starts_with("t_")
                        && (name.ends_with("_OpDrop") || name.ends_with("_OpDropAll")))
                    || name == "n_set_store_lock"
            }
            // The guarded release `if <holds a record> <release> else null`.
            Value::If(_, t, e) => matches!(e.unspan(), Value::Null) && self.is_free(t),
            _ => false,
        }
    }

    /// `ops` for their effect, returning the value a `tail_capture` names.
    fn ops_stmt(&mut self, ops: &[Value], ind: usize) -> Result<(), String> {
        let Some((at, ret)) = self.tail_capture(ops) else {
            return self.stmts(ops, ind);
        };
        self.stmts(&ops[..at], ind)?;
        let r = self.temp(self.ret);
        self.assign(&ops[at], &r, self.ret, ind)?;
        self.flush(ind);
        ops[at + 1..ret]
            .iter()
            .try_for_each(|o| self.stmt(o, ind))?;
        if let Value::Return(x) = ops[ret].unspan() {
            self.stmt(x, ind)?;
        }
        self.ret_held(&r, ind);
        Ok(())
    }

    /// A block's statements in order; a run of them a rewrite emits whole (`keyed_entry_site`)
    /// is taken together.
    fn stmts(&mut self, ops: &[Value], ind: usize) -> Result<(), String> {
        let held = self.rec_ptrs.len();
        let frames = self.headers.len();
        let r = self.stmts_in(ops, ind);
        self.rec_ptrs.truncate(held);
        // The view headers this block bound (`@FR-R-View`) close with it.
        self.headers.truncate(frames.min(self.headers.len()));
        r
    }

    fn stmts_in(&mut self, ops: &[Value], ind: usize) -> Result<(), String> {
        let mut i = 0;
        while i < ops.len() {
            // `@FR-R-Apart` instance 2: a construction run builds its value as a whole.
            if let Some(next) = self.av_construction(ops, i, ind)? {
                ops[i..next]
                    .iter()
                    .for_each(|o| self.refresh_grown_header(o, ind));
                i = next;
                continue;
            }
            if let Some(last) = self.rotate_site(ops, i, ind)? {
                ops[i..=last]
                    .iter()
                    .for_each(|o| self.refresh_grown_header(o, ind));
                i = last + 1;
                continue;
            }
            if let Some(last) = self.destination_site(ops, i, ind)? {
                i = last + 1;
                continue;
            }
            if let Some(last) = self.keyed_entry_site(ops, i, ind)? {
                ops[i..=last]
                    .iter()
                    .for_each(|o| self.refresh_grown_header(o, ind));
                i = last + 1;
                continue;
            }
            self.stmt(&ops[i], ind)?;
            self.bind_record_ptr(ops, i, ind);
            self.bind_view_header(ops, i, ind);
            i += 1;
        }
        Ok(())
    }

    /// `@FR-R-View` (@PLN157 § V-n, rustc's `bind_view_header`): statement `at` bound a vector
    /// VIEW `d = &P` whose header the rest of the block may share (`hoist::view_def_header`: the
    /// remainder writes only in place, never rebinds `d`, and indexes it) — `d` takes a header
    /// frame for the rest of the block, so `d[i]` and `len(d)` read through it.  Where `P` is held
    /// already (a twin's input, a loop's prelude) the header is a COPY of that holder, base and all
    /// (`@FR-R-State`); else it is derived once here, its base valid because the remainder grows no
    /// store.  The frame closes with the block (`stmts`).  `LOFT_NO_VIEW_HOIST=1` binds none.
    fn bind_view_header(&mut self, ops: &[Value], at: usize, ind: usize) {
        if self.cx.switches.view_hoist_disabled
            || self.gen_elem.is_some()
            || std::env::var("LOFT_NO_VECTOR_HOIST").is_ok_and(|v| v != "0")
        {
            return;
        }
        let mut cache = std::mem::take(&mut self.hoist_cache);
        let d = crate::opt::hoist::view_def_header(
            ops,
            at,
            self.cx.data,
            self.d,
            &mut cache,
            !self.cx.switches.write_hoist_disabled,
        );
        self.hoist_cache = cache;
        let Some(d) = d else { return };
        let path: crate::opt::writes::PathKey = (d, Vec::new());
        if self.held_path(&path).is_some()
            || self.split_tabs.contains_key(&d)
            || self.tab_alias.contains_key(&d)
        {
            return;
        }
        let Value::Set(_, rhs) = ops[at].unspan() else {
            return;
        };
        let held = crate::opt::writes::vector_path(self.cx.data, rhs).and_then(|p| {
            self.headers
                .iter()
                .rev()
                .flat_map(|f| f.iter())
                .find(|(q, _, _)| *q == p)
                .map(|(_, h, base)| (h.clone(), *base))
        });
        let h = format!("h{}", self.decls.len() + 1);
        self.decls
            .push(format!("loft_vh {h} = {{ 0xFFFF, 0, 0, 0 }};"));
        let base = if let Some((src, base)) = held {
            self.line(ind, &format!("{h} = {src};"));
            base
        } else {
            let var = self.var(d);
            self.line(ind, &format!("{h} = loft_vh_of({var});"));
            true
        };
        self.headers.push(vec![(path, h, base)]);
        crate::rewrite_census::fired("R-View", 1);
    }

    /// `@FR-R-RecPtr`'s PARAMETER CLAUSE: a plain-record parameter is a view the caller fixed
    /// before the first statement, so the body is its remainder (`hoist::param_view_ptr`,
    /// native's verdict); one read or written at least twice takes its address at entry, as
    /// native does.  A value record, a `&` link and a generator's parameter are never bound.
    fn bind_param_ptrs(&mut self, code: &Value) {
        if self.gen_elem.is_some()
            || crate::env_once!(std::env::var("LOFT_NO_RECORD_PTR").is_ok_and(|v| v != "0"))
        {
            return;
        }
        let Value::Block(b) = code.unspan() else {
            return;
        };
        let stmts = &b.operators;
        let data = self.cx.data;
        let trace = crate::env_once!(std::env::var("LOFT_TRACE_RECPTR").is_ok_and(|v| v != "0"));
        for p in self.vars.arguments() {
            if self.shapes.get(p as usize) != Some(&CType::Ref)
                || self.apr_holds(p)
                || self.links[p as usize]
                || self.vr_local(p).is_some()
                || crate::opt::hoist::scalar_field_uses(stmts, p, data, &[]) < 2
            {
                continue;
            }
            let mut cache = std::mem::take(&mut self.hoist_cache);
            let verdict = {
                let facts = self.store_facts();
                crate::opt::hoist::param_view_ptr(
                    stmts,
                    p,
                    data,
                    self.cx.stores,
                    self.d,
                    &mut cache,
                    true,
                    &std::collections::HashSet::new(),
                    facts.as_ref(),
                    None,
                )
            };
            self.hoist_cache = cache;
            if let Err(why) = verdict {
                if trace && why != "not a plain record" {
                    eprintln!(
                        "recptr: {} declines parameter {}: {why}",
                        data.def(self.d).name(),
                        self.vars.name(p)
                    );
                }
                continue;
            }
            let slot = self.decls.len();
            let (pa, pl) = (format!("loft_pa{p}_{slot}"), format!("loft_pl{p}_{slot}"));
            self.decls.push(format!("uint8_t *{pa} = 0;"));
            self.decls.push(format!("int {pl} = 0;"));
            let rec = self.var(p);
            self.line(1, &format!("{pa} = loft_rec_ptr({rec});"));
            let writes = stmts.iter().any(|op| {
                op.any_node(&mut |n| {
                    matches!(n, Value::Call(d, a) if (*d as usize) < data.definitions.len()
                        && matches!(data.def(*d).name(), "OpSetInt" | "OpSetFloat" | "OpSetSingle" | "OpSetEnum")
                        && a.len() == 3
                        && crate::opt::body_writes::view_field(data, &a[0], &a[1]).is_some_and(|(root, _)| root == p))
                        || copy_into_sub(data, n, p)
                })
            });
            if writes {
                self.line(1, &format!("{pl} = loft_rec_locked({rec});"));
            }
            if trace {
                eprintln!(
                    "recptr: {} parameter {} bound",
                    data.def(self.d).name(),
                    self.vars.name(p)
                );
            }
            crate::rewrite_census::fired("R-RecPtr/param", 1);
            self.rec_ptrs.push((p, pa, pl));
        }
    }

    /// `@FR-R-RecPtr` — after statement `at` binds a plain record VIEW whose address the rest of
    /// the block may share (`hoist::record_view_ptr`, native's verdict: the remainder grows no
    /// store, frees no record before a use and never rebinds the view), take the address once;
    /// the view's scalar field reads and in-place writes then go through it (`rec_ptr_access`).
    /// A value-record local is a struct, not a place, and is never bound.
    /// `LOFT_NO_RECORD_PTR=1` keeps every access a store resolution.
    fn bind_record_ptr(&mut self, ops: &[Value], at: usize, ind: usize) {
        if self.gen_elem.is_some()
            || self.nk_skip.is_some()
            || crate::env_once!(std::env::var("LOFT_NO_RECORD_PTR").is_ok_and(|v| v != "0"))
        {
            return;
        }
        let Some(Value::Set(r, _)) = ops.get(at).map(Value::unspan) else {
            return;
        };
        let r = *r;
        if self.vr_local(r).is_some()
            || self.apr_holds(r)
            || self.rec_ptrs.iter().any(|(v, _, _)| *v == r)
            || self.shapes.get(r as usize) != Some(&CType::Ref)
        {
            return;
        }
        // The address serves the view's field reads and writes alone (`rec_ptr_access`), so
        // only the statements up to the last of them are judged: what follows — an append's
        // finish, which may grow the store the element lives in — takes the reference.
        let data = self.cx.data;
        let field_use = |op: &Value| {
            op.any_node(&mut |n| {
                matches!(n, Value::Call(d, a) if (*d as usize) < data.definitions.len()
                    && a.len() >= 2
                    && { let nm = data.def(*d).name(); nm.starts_with("OpGet") || nm.starts_with("OpSet") }
                    && crate::opt::body_writes::view_field(data, &a[0], &a[1]).is_some_and(|(root, _)| root == r))
                    || copy_into_sub(data, n, r)
            })
        };
        let Some(last) = ops.iter().rposition(field_use).filter(|&l| l > at) else {
            return;
        };
        let mut cache = std::mem::take(&mut self.hoist_cache);
        let verdict = {
            let facts = self.store_facts();
            let remainder = apart_calls(data, facts.as_ref(), r, &ops[..=last], at);
            crate::opt::hoist::record_view_ptr(
                &remainder,
                at,
                self.cx.data,
                self.cx.stores,
                self.d,
                &mut cache,
                true,
                &std::collections::HashSet::new(),
                facts.as_ref(),
                None,
                false,
            )
        };
        self.hoist_cache = cache;
        let trace = crate::env_once!(std::env::var("LOFT_TRACE_RECPTR").is_ok_and(|v| v != "0"));
        if let Err(why) = verdict {
            if trace {
                eprintln!(
                    "recptr: {} {} declined: {why}",
                    self.cx.data.def(self.d).name(),
                    self.vars.name(r)
                );
            }
            return;
        }
        if trace {
            eprintln!(
                "recptr: {} {} bound",
                self.cx.data.def(self.d).name(),
                self.vars.name(r)
            );
        }
        let slot = self.decls.len();
        let (pa, pl) = (format!("loft_pa{r}_{slot}"), format!("loft_pl{r}_{slot}"));
        self.decls.push(format!("uint8_t *{pa} = 0;"));
        self.decls.push(format!("int {pl} = 0;"));
        let rec = self.var(r);
        self.line(ind, &format!("{pa} = loft_rec_ptr({rec});"));
        // The lock flag only matters to a write through the address.
        let data = self.cx.data;
        let writes = ops[at + 1..].iter().any(|op| {
            op.any_node(&mut |n| {
                matches!(n, Value::Call(d, a) if (*d as usize) < data.definitions.len()
                    && matches!(data.def(*d).name(), "OpSetInt" | "OpSetFloat" | "OpSetSingle" | "OpSetEnum")
                    && a.len() == 3
                    && crate::opt::body_writes::view_field(data, &a[0], &a[1]).is_some_and(|(root, _)| root == r))
                    || copy_into_sub(data, n, r)
            })
        });
        if writes {
            self.line(ind, &format!("{pl} = loft_rec_locked({rec});"));
        }
        crate::rewrite_census::fired("R-RecPtr", 1);
        self.rec_ptrs.push((r, pa, pl));
    }

    /// A scalar field read or in-place write of a record view whose address an enclosing block
    /// holds (`bind_record_ptr`), through that address; `None` for any other access.
    fn rec_ptr_access(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let (getter, base, fld) = match (name, args) {
            (
                "OpGetInt" | "OpGetFloat" | "OpGetSingle" | "OpGetEnum" | "OpGetInt4"
                | "OpGetInt4Raw" | "OpGetInt4Full",
                [base, fld],
            )
            | (
                "OpGetByte" | "OpGetByteNullable" | "OpGetShort" | "OpGetShortRaw"
                | "OpGetShortFull",
                [base, fld, _],
            ) => (true, base, fld),
            ("OpSetInt" | "OpSetFloat" | "OpSetSingle" | "OpSetEnum", [base, fld, _]) => {
                (false, base, fld)
            }
            _ => return Ok(None),
        };
        let Some((root, off)) = crate::opt::body_writes::view_field(self.cx.data, base, fld) else {
            return Ok(None);
        };
        let Some((_, pa, pl)) = self
            .rec_ptrs
            .iter()
            .rev()
            .find(|(v, _, _)| *v == root)
            .cloned()
        else {
            return Ok(None);
        };
        let Ok(off) = u32::try_from(off) else {
            return Ok(None);
        };
        if getter {
            // A narrow read's encoding is decoded by the reader its store getter uses.
            let (f, k) = match name {
                "OpGetInt" => ("loft_pa_i64", K::Int),
                "OpGetFloat" => ("loft_pa_f64", K::Float),
                "OpGetSingle" => ("loft_pa_f32", K::Single),
                "OpGetEnum" => ("loft_pa_enum", K::Enum),
                "OpGetInt4" => ("loft_pa_int4", K::Int),
                "OpGetInt4Raw" => ("loft_pa_int4_raw", K::Int),
                "OpGetInt4Full" => ("loft_pa_int4_full", K::Int),
                "OpGetByte" => ("loft_pa_byte", K::Int),
                "OpGetByteNullable" => ("loft_pa_byte_nullable", K::Int),
                "OpGetShort" => ("loft_pa_short", K::Int),
                _ => ("loft_pa_short_raw", K::Int),
            };
            if args.len() == 3 {
                let min = c_long(Self::literal(&args[2], "minimum")?);
                return Ok(Some(E::new(format!("{f}({pa}, {off}, {min})"), k)));
            }
            return Ok(Some(E::new(format!("{f}({pa}, {off})"), k)));
        }
        let val = self.expr(&args[2], ind)?;
        let (ty, val) = match name {
            "OpSetInt" => ("int64_t", as_int(val)?),
            "OpSetFloat" => ("double", as_float(val)?),
            "OpSetSingle" => ("float", as_single(val)?),
            _ => ("uint8_t", as_enum(val)?),
        };
        let t = self.temp(match name {
            "OpSetInt" => CType::Int,
            "OpSetFloat" => CType::Float,
            "OpSetSingle" => CType::Single,
            _ => CType::Enum,
        });
        self.line(ind, &format!("{t} = ({ty}) ({val});"));
        Ok(Some(E::new(
            format!("loft_pa_set({pa}, {pl}, {off}, &{t}, sizeof({ty}))"),
            K::Void,
        )))
    }

    /// `@FR-R-FormatAppend` at emission: `dest += { w = ""; <writes into w>; w }`, where every
    /// statement after the clear is a format-family write into `w` (an `OpFormat…` or
    /// `OpAppend…` with `w` as its target) that names neither `w` elsewhere nor the
    /// destination, is the same writes made to `dest` directly — what appending the work text
    /// afterwards leaves.  The parser applies the rule to `x += "…"`; this takes the shapes it
    /// leaves, `big = big + "{ch}"` among them.  `None` for any other value.
    /// `LOFT_NO_FORMAT_INTO=1` keeps the work text.
    fn format_into_dest(
        &mut self,
        dest: &Value,
        val: &Value,
        ind: usize,
    ) -> Result<Option<E>, String> {
        if crate::env_once!(std::env::var("LOFT_NO_FORMAT_INTO").is_ok_and(|v| v != "0")) {
            return Ok(None);
        }
        let Value::Block(b) = val.unspan() else {
            return Ok(None);
        };
        let ops: Vec<&Value> = b
            .operators
            .iter()
            .filter(|o| !matches!(o.unspan(), Value::Line(_)))
            .collect();
        let [clear, writes @ .., last] = &ops[..] else {
            return Ok(None);
        };
        let Value::Var(w) = last.unspan() else {
            return Ok(None);
        };
        let w = *w;
        if !matches!(clear.unspan(), Value::Set(x, t) if *x == w && matches!(t.unspan(), Value::Text(s) if s.is_empty()))
            || writes.is_empty()
            || self.shapes.get(w as usize) != Some(&CType::Text)
        {
            return Ok(None);
        }
        let data = self.cx.data;
        let mut dest_vars: Vec<u16> = Vec::new();
        dest.walk(&mut |n| {
            if let Value::Var(x) = n.unspan() {
                dest_vars.push(*x);
            }
        });
        if dest_vars.contains(&w) {
            return Ok(None);
        }
        let ok = writes.iter().all(|op| {
            let Value::Call(d, args) = op.unspan() else {
                return false;
            };
            let name = if (*d as usize) < data.definitions.len() {
                data.def(*d).name()
            } else {
                return false;
            };
            if !(name.starts_with("OpFormat") || name.starts_with("OpAppend"))
                || !matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == w)
            {
                return false;
            }
            let mut untouched = true;
            for a in &args[1..] {
                a.walk(&mut |n| {
                    if let Value::Var(x) = n.unspan()
                        && (*x == w || dest_vars.contains(x))
                    {
                        untouched = false;
                    }
                });
            }
            untouched
        });
        if !ok {
            return Ok(None);
        }
        let target = self.text_dest(dest)?;
        self.work_alias = Some((w, target));
        let r = writes.iter().try_for_each(|op| self.stmt(op, ind));
        self.work_alias = None;
        r?;
        crate::rewrite_census::fired("R-FormatAppend/c", 1);
        Ok(Some(E::new("((void) 0)", K::Void)))
    }

    /// `@FR-R-HandOver`'s rotate clause (native's `Output::output_rotate_site`): when the vector
    /// is full (`loft_rotate_into` admits it) the copy goes into the oldest element and the
    /// slots rotate, S runs, and the suffix-keep callee's `__nk` twin runs the rest of its body;
    /// otherwise every statement of the range as written.  The index of the range's last
    /// statement, or `None` where `ops[at]` opens no site.  `LOFT_NO_HAND_OVER=1` keeps the
    /// statements as written.
    fn rotate_site(
        &mut self,
        ops: &[Value],
        at: usize,
        ind: usize,
    ) -> Result<Option<usize>, String> {
        if self.gen_elem.is_some() || self.nk_skip.is_some() {
            return Ok(None);
        }
        let Some(site) = crate::lower::hand_over::site(ops, at, self.cx.data, self.cx.stores)
        else {
            return Ok(None);
        };
        let Some((plan, count)) = self.cx.stores.plain_vectors_of(site.elem_tp) else {
            return Ok(None);
        };
        if crate::lower::hand_over::keep_statement(self.cx.data, site.g).is_none() {
            return Ok(None);
        }
        let elem_size = u32::from(self.cx.stores.size(site.elem_tp));
        let pairs: Vec<String> = plan[..count]
            .iter()
            .flat_map(|(o, s)| [format!("{o}u"), format!("{s}u")])
            .collect();
        let plan_c = if pairs.is_empty() {
            "(const uint32_t *) 0".to_string()
        } else {
            format!("(const uint32_t[]){{ {} }}", pairs.join(", "))
        };
        let vec = self.expr(&site.vector, ind)?;
        let vec = as_ref(vec)?;
        // An apart source (`@FR-R-Apart` instance 4) rotates in from its field buffers.
        let rotate = if let Some(apart) =
            self.apr_rotate(site.src, &vec, elem_size, site.k, &plan_c, count)
        {
            apart
        } else {
            let src = self.expr(site.src, ind)?;
            let src = as_ref(src)?;
            format!(
                "loft_rotate_into({vec}, {src}, {elem_size}u, INT64_C({}), {plan_c}, {count})",
                site.k
            )
        };
        self.line(ind, &format!("if ({rotate}) {{"));
        for s in &site.between {
            self.stmt(s, ind + 1)?;
        }
        let call = Value::Call(site.g, vec![site.owner.clone(), Value::Int(1)]);
        self.nk_call = Some(site.g);
        self.stmt(&call, ind + 1)?;
        if self.nk_call.take().is_some() {
            return Err("the rotate's call of its suffix keep took no __nk twin".into());
        }
        self.line(ind, "} else {");
        for s in &site.range {
            self.stmt(s, ind + 1)?;
        }
        self.line(ind, "}");
        crate::rewrite_census::fired("R-HandOver/rotate", 1);
        Ok(Some(site.last))
    }

    fn null_into(&mut self, target: &str, c: CType, ind: usize) {
        let s = match c {
            CType::Int => format!("{target} = LOFT_NULL;"),
            CType::Int32 => format!("{target} = LOFT_NULL32;"),
            CType::Bool => format!("{target} = LOFT_BOOL_NULL;"),
            CType::Text => format!("loft_text_set(&{target}, loft_null_text());"),
            CType::Ref => format!("{target} = loft_ref_null();"),
            CType::Enum => format!("{target} = 255;"),
            CType::Float => format!("{target} = (double) NAN;"),
            CType::Single => format!("{target} = (float) NAN;"),
            CType::Char => format!("{target} = 0;"),
            CType::Fn => format!("{target} = loft_fn_of(LOFT_NULL);"),
            CType::Tuple(id) => {
                for (i, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.null_into(&format!("{target}.f{i}"), e, ind);
                }
                return;
            }
            CType::Void => return,
        };
        self.line(ind, &s);
    }

    /// loft#1753 — a statement that calls into a frame first records the line it calls from,
    /// which `stack_trace()` reports as the callee frame's `line` (`cr_call_site`).
    fn note_site(&mut self, v: &Value, ind: usize) {
        if let Value::Line(n) = v.unspan() {
            self.cur_line = *n;
        } else if self.cur_line != 0
            && !matches!(
                v.unspan(),
                Value::Block(_) | Value::Insert(_) | Value::Span(_)
            )
            && crate::opt::frame_calls::calls_a_frame(self.cx.data, v)
        {
            self.line(ind, &format!("loft_frame_site({});", self.cur_line));
        }
    }

    /// Emit `v` for its effect.
    /// `@FR-R-Destination`'s callee half: the scalar locals a `__d` twin answers.
    fn ds_decls(&mut self) {
        if let Some(ds) = &self.ds {
            for (k, sc) in ds.scalars.iter().enumerate() {
                let c = super::dest_scalar_ctype(sc);
                self.decls.push(format!("{} loft_ds{k} = 0;", held(c)));
            }
        }
    }

    /// `@FR-R-Destination`'s callee half: a write to a scalar field of the buffer, by that
    /// field's own setter, is the twin's local — the field lies past the caller's element the
    /// buffer stands for.
    fn dest_scalar_set(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Some(ds) = &self.ds else { return Ok(None) };
        let [base, off, val] = args else {
            return Ok(None);
        };
        if !matches!(base.unspan(), Value::Var(b) if *b == ds.rb) {
            return Ok(None);
        }
        let Value::Int(off) = off.unspan() else {
            return Ok(None);
        };
        let Some(k) = ds
            .scalars
            .iter()
            .position(|sc| sc.off == i64::from(*off) && sc.setter == name)
        else {
            return Ok(None);
        };
        let c = super::dest_scalar_ctype(&ds.scalars[k]);
        let e = self.expr(val, ind)?;
        let v = match c {
            CType::Float => as_float(e)?,
            CType::Single => as_single(e)?,
            CType::Bool => as_bool(e)?,
            _ => as_int(e)?,
        };
        Ok(Some(E::new(format!("loft_ds{k} = {v}"), K::Void)))
    }

    /// `@FR-R-Destination`'s callee half: `return buf` answers the scalar locals.
    fn dest_return(&mut self, ind: usize) {
        let n = self.ds.as_ref().map_or(0, |ds| ds.scalars.len());
        let CType::Tuple(id) = self.ret else { return };
        let r = self.temp(self.ret);
        let parts: Vec<String> = (0..n).map(|k| format!("loft_ds{k}")).collect();
        self.line(
            ind,
            &format!("{r} = (loft_tup{id}){{ {} }};", parts.join(", ")),
        );
        self.ret_held(&r, ind);
    }

    /// `@FR-R-Destination`'s caller half (`lower::destination::site`): the element's mint,
    /// hoisted above the chain, then each call into its destination — the callee's `__d` twin
    /// writing the moved field into the element and answering its scalars — and its `ok` arm
    /// with the moves gone; the element finished only where the plain form moved every result,
    /// and each destination filled so far released on a failing path.  `LOFT_NO_DESTINATION=1`
    /// keeps the buffer and the move.
    fn destination_site(
        &mut self,
        ops: &[Value],
        i: usize,
        ind: usize,
    ) -> Result<Option<usize>, String> {
        if !crate::lower::destination::enabled() || self.gen_elem.is_some() {
            return Ok(None);
        }
        let site = {
            let mut memo = self.cx.dest_memo.borrow_mut();
            crate::lower::destination::site(ops, i, self.cx.data, self.cx.stores, self.d, &mut memo)
        };
        let Some(site) = site else { return Ok(None) };
        for m in &site.mint {
            self.stmt(m, ind)?;
        }
        let mut names: Vec<String> = Vec::new();
        let mut filled: Vec<(Value, u16)> = Vec::new();
        self.destination_level(&site.top, &mut names, &mut filled, ind)?;
        Ok(Some(site.last))
    }

    /// Resolve the chain's `__ds#m#.k` placeholders to the C tuples already named.
    fn destination_names(&mut self, v: &Value, names: &[String]) -> Value {
        let mut v = v.clone();
        let mut made: Vec<(String, K)> = Vec::new();
        v.map_nodes(&mut |n| {
            if let Value::RawExpr(e) = n
                && e.starts_with("__ds#")
            {
                for (m, t) in names.iter().enumerate() {
                    let pre = format!("__ds#{m}#.");
                    if let Some(k) = e.strip_prefix(&pre) {
                        *e = format!("{t}.f{k}");
                        made.push((e.clone(), K::Int));
                        break;
                    }
                }
            }
        });
        for (code, _) in made {
            let (t, k) = code.split_once(".f").unwrap_or(("", "0"));
            let kind = self
                .temps
                .iter()
                .find(|(n, _)| n == t)
                .and_then(|(_, c)| match c {
                    CType::Tuple(id) => k
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| self.cx.tuple_shapes(*id).get(i).copied()),
                    _ => None,
                })
                .map_or(K::Int, kind_of);
            self.raw.insert(code, kind);
        }
        v
    }

    fn destination_level(
        &mut self,
        level: &crate::lower::destination::Level<'_>,
        names: &mut Vec<String>,
        filled: &mut Vec<(Value, u16)>,
        ind: usize,
    ) -> Result<(), String> {
        let mut args: Vec<Value> = level
            .call_args
            .iter()
            .map(|a| self.destination_names(a, names))
            .collect();
        args[level.at] = level.dest.clone();
        let call = Value::Call(level.callee, args);
        let Value::Call(_, a) = &call else {
            unreachable!()
        };
        self.ds_site = Some((level.callee, a.as_ptr() as usize));
        let e = self.expr(&call, ind)?;
        if self.ds_site.take().is_some() {
            return Err(format!(
                "@FR-R-Destination: the rewritten call of {} was written without its twin",
                self.cx.data.def(level.callee).name()
            ));
        }
        let shape = &level.shape;
        let id = self
            .cx
            .tuple_id(shape.scalars.iter().map(super::dest_scalar_ctype).collect());
        let t = self.temp(CType::Tuple(id));
        self.line(ind, &format!("{t} = {};", e.code));
        names.push(t.clone());
        filled.push((level.dest.clone(), shape.vt));
        self.line(ind, &format!("if ({t}.f{} == 1) {{", level.ok));
        for op in &level.pre {
            if matches!(op, Value::Line(_)) {
                continue;
            }
            let op = self.destination_names(op, names);
            self.stmt(&op, ind + 1)?;
        }
        if let Some(inner) = &level.inner {
            self.destination_level(inner, names, filled, ind + 1)?;
        }
        for op in &level.post {
            if matches!(op, Value::Line(_)) {
                continue;
            }
            let op = self.destination_names(op, names);
            self.stmt(&op, ind + 1)?;
        }
        self.line(ind, "} else {");
        for (d, vt) in filled.clone().iter().rev() {
            self.cx.copy_type(*vt)?;
            let r = self.expr(d, ind + 1)?;
            self.line(
                ind + 1,
                &format!("loft_remove_claims(loft_types, {vt}, {});", as_ref(r)?),
            );
        }
        self.stmt(level.else_v, ind + 1)?;
        self.line(ind, "}");
        filled.pop();
        names.pop();
        Ok(())
    }

    /// `@FR-R-AppendTwin` — `X += f(args)` where f has an append twin (`opt::append_twin`):
    /// written as the call of `f__ap` with X as its buffer, so f's appends land after X's
    /// elements.  `LOFT_NO_APPEND_TWIN=1` keeps the copy.
    fn append_site(&mut self, v: &Value, ind: usize) -> Result<bool, String> {
        if !crate::keys::append_twin_enabled() || self.gen_elem.is_some() {
            return Ok(false);
        }
        let mut memo = std::mem::take(&mut self.ap_memo);
        let site = crate::opt::append_twin::call_site(v, self.cx.data, self.d, &mut memo);
        self.ap_memo = memo;
        let Some(site) = site else { return Ok(false) };
        let mut args = site.args.to_vec();
        args[site.at] = site.dest.clone();
        let call = Value::Call(site.callee, args);
        let Value::Call(_, a) = &call else {
            unreachable!()
        };
        self.ap_site = Some((site.callee, a.as_ptr() as usize));
        let e = self.expr(&call, ind)?;
        if self.ap_site.take().is_some() {
            return Err(format!(
                "@FR-R-AppendTwin: the rewritten call of {} was written without its twin",
                self.cx.data.def(site.callee).name()
            ));
        }
        self.line(ind, &format!("{};", e.code));
        Ok(true)
    }

    /// Does this call go to the callee's `__ap` twin?  The statement-level site's own call
    /// (matched by callee and argument slice), or — inside a twin — a call handed the twin's
    /// buffer, or a view of it, as the callee's buffer: eligibility admitted the twin only
    /// where every such callee has a twin of its own.
    fn append_call(&mut self, d: u32, args: &[Value]) -> bool {
        let site = self
            .ap_site
            .is_some_and(|(c, at)| c == d && at == args.as_ptr() as usize);
        if site {
            self.ap_site = None;
        }
        let handed = !site
            && self.ap_buf.is_some()
            && self
                .cx
                .data
                .def(d)
                .hidden_return_buffer_attr()
                .and_then(|at| args.get(at))
                .is_some_and(|a| matches!(a.unspan(), Value::Var(x) if self.ap_views.contains(x)));
        if site || handed {
            crate::rewrite_census::fired("R-AppendTwin", 1);
        }
        site || handed
    }

    fn stmt(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        if self.nk_skip == Some(std::ptr::from_ref(v.unspan()) as usize) {
            return Ok(());
        }
        // `@FR-R-DeferredCopy`: a deferred bind takes the source's reference; any statement that
        // may write a source's store takes the copies it would otherwise disturb first.
        if let Value::Set(var, rhs) = v.unspan()
            && self.dc_set(v, *var, rhs, ind).is_some()
        {
            return Ok(());
        }
        self.dc_before(v, ind);
        if self.append_site(v, ind)? {
            return Ok(());
        }
        let r = self.stmt_kind(v, ind);
        if r.is_ok()
            && let Value::Set(var, _) = v.unspan()
        {
            self.refresh_rebound_header(*var, ind);
            self.dc_after(v, ind);
        }
        // A span is emitted as the statement it wraps, which refreshes for itself.
        if r.is_ok() && !matches!(v, Value::Span(_)) {
            self.refresh_grown_header(v, ind);
        }
        // `LOFT_C_WHERE=1`: a refusal names the innermost statement it came from, on stderr.
        if let Err(why) = &r
            && !self.where_told
            && std::env::var("LOFT_C_WHERE").is_ok_and(|w| w != "0")
            && !matches!(
                v.unspan(),
                Value::Block(_) | Value::Loop(_) | Value::If(..) | Value::Insert(_)
            )
        {
            self.where_told = true;
            let mut at = format!("{v:?}");
            at.truncate(600);
            eprintln!(
                "loft: --emit-c: {}: {why}\n  at: {at}",
                self.cx.data.def(self.d).original_name()
            );
        }
        r
    }

    /// `@FR-R-Header`'s rebind clause: after `var` is rebound, every header an enclosing loop
    /// holds for it as a bare variable is derived again — element base included — so it
    /// describes the vector `var` names now (`hoist::rebound_apart_headers`).
    fn refresh_rebound_header(&mut self, var: u16, ind: usize) {
        let held: Vec<String> = self
            .headers
            .iter()
            .flat_map(|f| f.iter())
            .filter(|(p, _, _)| p.0 == var && p.1.is_empty())
            .map(|(_, h, _)| h.clone())
            .collect();
        for h in held {
            let v = self.var(var);
            self.line(ind, &format!("{h} = loft_vh_of({v});"));
        }
    }

    /// `@FR-R-Header`'s growth half: after statement `op` — when it is one `apart::growth_args`
    /// recognises — every grown header held for a variable it names is derived again, element
    /// base included, so the reads after it see the vector's record, length and memory now.
    fn refresh_grown_header(&mut self, op: &Value, ind: usize) {
        if self.grown_headers.is_empty() {
            return;
        }
        let passed = std::mem::take(&mut self.kh_passed);
        let Some(xs) = crate::opt::apart::growth_args(op, self.cx.data) else {
            return;
        };
        for x in xs {
            let held: Vec<String> = self
                .headers
                .iter()
                .flat_map(|f| f.iter())
                .filter(|(p, h, _)| {
                    p.0 == x
                        && p.1.is_empty()
                        && self.grown_headers.contains(h)
                        && !passed.contains(h)
                })
                .map(|(_, h, _)| h.clone())
                .collect();
            for h in held {
                let v = self.var(x);
                self.line(ind, &format!("{h} = loft_vh_of({v});"));
            }
        }
    }

    /// A loop: its held vectors and its push window before it, the window closed after —
    /// written twice under `@FR-R-Alias`'s versioned clause (`distinct_version`): the copy
    /// that assumes the parameters apart from the return buffer runs when a test of their
    /// store numbers says so at its entry, the plain one otherwise.
    fn loop_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        if let Some((tests, pairs)) = self.distinct_version(b) {
            let test = tests
                .iter()
                .map(|&(p, rb)| format!("{}.store_nr != {}.store_nr", self.var(rb), self.var(p)))
                .collect::<Vec<_>>()
                .join(" && ");
            self.flush(ind);
            self.line(ind, &format!("if ({test}) {{"));
            self.assumed = pairs;
            let r = self.loop_copy(b, ind + 1);
            self.assumed.clear();
            r?;
            self.line(ind, "} else {");
            self.loop_copy(b, ind + 1)?;
            self.line(ind, "}");
            crate::rewrite_census::fired("R-AliasVersion", 1);
            return Ok(());
        }
        self.loop_copy(b, ind)
    }

    fn loop_copy(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        let held_file = self.open_file_hold(b, ind)?;
        self.hold_invariants(b, ind);
        let mints = self.hold_vectors(b, ind)?;
        let window = self.open_window(b, ind)?;
        if !window {
            self.hash_reserve(b, ind)?;
        }
        self.hold_pushes(b, mints, ind)?;
        match self.chain_guard(b, ind)? {
            Some((guard, admitted, true)) => {
                // An innermost loop: the guarded copy plain, the checked loop as the `else`.
                self.line(ind, &format!("if ({guard}) {{"));
                self.plain_chains.push((admitted, None));
                let r = self.loop_core(b, ind + 1);
                self.plain_chains.pop();
                r?;
                self.line(ind, "} else {");
                self.loop_core(b, ind + 1)?;
                self.line(ind, "}");
            }
            Some((guard, admitted, false)) => {
                // Loops inside: written once, each admitted operator branching on the guard.
                self.plain_chains.push((admitted, Some(guard)));
                let r = self.loop_core(b, ind);
                self.plain_chains.pop();
                r?;
            }
            None => self.loop_core(b, ind)?,
        }
        self.headers.pop();
        self.push_heads.pop();
        self.scalars.pop();
        self.invariants.pop();
        if window {
            // The loop's one exit: the counted push loop has no break, return or
            // continue of its own (`hoist::push_loop`).
            let (path, w, _, _) = self.windows.pop().expect("the window this loop opened");
            self.line(ind, &format!("loft_win_sync({w});"));
            // `@FR-R-Refresh`: an enclosing loop's push header on the same path did not see the
            // window's reservation or its pushes — the record may have moved and the length
            // grew — so it is derived again here, where the window gave the path back.
            if let Some((h, size)) = self.push_head(&path) {
                self.line(ind, &format!("{h} = loft_ph_of({h}.v, {size});"));
            }
        }
        if held_file {
            // The loop's own end; its other exits gave the cursor back where they leave.
            let (_, c, _) = self.file_holds.pop().expect("the cursor this loop holds");
            self.line(ind, &format!("loft_fcur_release(&{c}, {c}p);"));
        }
        Ok(())
    }

    /// The loop itself: its labels, its `for (;;)` (or a character walk's) and its body.
    fn loop_core(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        self.labels += 1;
        let l = self.labels;
        self.loops.push(l);
        if let Some(walk) = self.char_walks.get(&b.scope).cloned() {
            self.char_walk_loop(b, &walk, ind)?;
        } else {
            self.line(ind, "for (;;) {");
            self.block_stmt(b, ind + 1)?;
        }
        self.line(ind, &format!("cont_{l}:;"));
        self.line(ind, "}");
        self.line(ind, &format!("brk_{l}:;"));
        self.loops.pop();
        Ok(())
    }

    /// `@FR-R-CharWalk` — a `for c in t` walk: its null test once before the loop where the loop
    /// never writes `t`, and its step's ASCII arm — a byte below 0x80 at `next` is the character,
    /// one byte wide — ahead of the step as the lowering writes it, which every other byte takes.
    /// Opens the loop's `for (;;) {`; the caller closes it.
    fn char_walk_loop(
        &mut self,
        b: &Block,
        walk: &crate::opt::char_walk::CharWalk,
        ind: usize,
    ) -> Result<(), String> {
        if walk.hoist_null
            && let Some(test) = b.operators.get(walk.null_test)
        {
            self.stmt(test, ind)?;
        }
        self.line(ind, "for (;;) {");
        for (i, op) in b.operators.iter().enumerate() {
            if walk.hoist_null && i == walk.null_test {
                continue;
            }
            if i == 0 {
                let src = self.expr(&Value::Var(walk.src), ind + 1)?;
                let src = as_view(src)?;
                let (next, index, c) = (
                    self.var(walk.next),
                    self.var(walk.index),
                    self.var(walk.loop_var),
                );
                self.line(
                    ind + 1,
                    &format!(
                        "if ({next} >= 0 && {next} < (int64_t) ({src}).len && (unsigned) (unsigned char) ({src}).p[{next}] - 1u < 0x7Fu) {{"
                    ),
                );
                self.line(ind + 2, &format!("{index} = {next};"));
                self.line(
                    ind + 2,
                    &format!("{c} = (uint32_t) (unsigned char) ({src}).p[{next}];"),
                );
                self.line(ind + 2, &format!("{next} = {next} + 1;"));
                self.line(ind + 1, "} else {");
                self.stmt(op, ind + 2)?;
                self.line(ind + 1, "}");
                continue;
            }
            self.stmt(op, ind + 1)?;
        }
        Ok(())
    }

    #[expect(clippy::too_many_lines)]
    fn stmt_kind(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        self.note_site(v, ind);
        match v {
            // A value block's result where nothing takes it: a leaf has no effect.
            Value::Null
            | Value::Line(_)
            | Value::Var(_)
            | Value::Int(_)
            | Value::Long(_)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Enum(..)
            | Value::Text(_) => Ok(()),
            Value::Span(b) => self.stmt(&b.1, ind),
            // `@FR-R-ValueRecord` — a materialised view return, or a build that returns what
            // it builds, standing as a statement returns its struct.
            Value::Block(b) if self.vr_stmt_leaf(b) => self.vr_stmt(v, b, ind),
            Value::Block(b) => self.block_stmt(b, ind),
            Value::Insert(ops) => ops.iter().try_for_each(|o| self.stmt(o, ind)),
            Value::Set(var, x) => {
                if self.apr_set(*var, x, ind)?
                    || self.apv_stmt(v, ind)
                    || self.av_set(*var, x, ind)?
                {
                    return Ok(());
                }
                self.set_stmt(*var, x, ind)
            }
            Value::TuplePut(var, i, x) => {
                let CType::Tuple(id) = self.shapes[*var as usize] else {
                    return Err("a tuple write into a non-tuple".into());
                };
                let shapes = self.cx.tuple_shapes(id);
                let c = *shapes
                    .get(*i as usize)
                    .ok_or("a tuple index out of range")?;
                let target = format!("{}.f{i}", self.var(*var));
                self.assign(x, &target, c, ind)?;
                self.flush(ind);
                Ok(())
            }
            Value::If(..) if self.apv_stmt(v, ind) => Ok(()),
            Value::If(c, t, e) => {
                let null_test = self.null_test(c);
                // The test first; each arm inside its braces (`@FR-E-And`, `@FR-E-Or`, as in
                // `assign_if`).
                let c = self.expr(c, ind)?;
                let c = as_cond(c)?;
                self.line(ind, &format!("if ({}) {{", bare(&c)));
                self.stmt(t, ind + 1)?;
                if !matches!(e.unspan(), Value::Null) {
                    self.line(ind, "} else {");
                    if null_test {
                        self.line(ind + 1, "LOFT_COLD_PATH();");
                    }
                    self.stmt(e, ind + 1)?;
                }
                self.line(ind, "}");
                self.flush(ind);
                Ok(())
            }
            Value::Loop(b) => self.loop_stmt(b, ind),
            Value::Break(n) => {
                let l = self.label(*n)?;
                self.flush(ind);
                // The holds of the loops it leaves; the target's own goes back after its end.
                self.release_holds(self.loops.len() - *n as usize, ind);
                self.line(ind, &format!("goto brk_{l};"));
                Ok(())
            }
            Value::Continue(n) => {
                let l = self.label(*n)?;
                self.flush(ind);
                // The holds of the loops it leaves; the target's own stays held.
                self.release_holds(self.loops.len() - *n as usize, ind);
                self.line(ind, &format!("goto cont_{l};"));
                Ok(())
            }
            // A generator's `return` ends it: what it returns is done for its effect.
            Value::Return(x) if self.av_any() && self.av_return(x, ind)? => Ok(()),
            Value::Return(x) if self.gen_elem.is_some() => {
                self.stmt(x, ind)?;
                self.line(ind, "goto loft_gen_end;");
                Ok(())
            }
            Value::Return(x)
                if self
                    .ds
                    .as_ref()
                    .is_some_and(|ds| matches!(x.unspan(), Value::Var(b) if *b == ds.rb)) =>
            {
                self.dest_return(ind);
                Ok(())
            }
            Value::Return(x) => self.ret_value(Some(x), ind),
            Value::Yield(x) => self.gen_yield(x, ind),
            // `parallel { … }`: the arms run one after another — each arm's effect is the one
            // its thread has (State's `OpParallelBegin`).
            Value::Parallel(arms) => arms.iter().try_for_each(|a| self.stmt(a, ind)),
            // A dropped branch or block runs as a statement: each arm for its effect, a value
            // arm's value discarded (`@FR-F-Drop`) — where an arm assigns, it has no value to read.
            Value::Drop(x)
                if matches!(
                    x.unspan(),
                    Value::If(..) | Value::Block(_) | Value::Insert(_)
                ) =>
            {
                self.stmt(x, ind)
            }
            Value::Drop(_) | Value::Call(..) | Value::CallRef(..) => {
                let x = if let Value::Drop(x) = v { &**x } else { v };
                let e = self.expr(x, ind)?;
                if e.kind == K::Void {
                    self.line(ind, &format!("{};", e.code));
                } else {
                    self.line(ind, &format!("(void) {};", e.code));
                }
                self.flush(ind);
                Ok(())
            }
            other => Err(unsupported(other)),
        }
    }

    /// `v = src` binding one record variable to another of a type that copies
    /// (`@FR-B-Copy`, @C86): the source and the store type — the bytecode writer's
    /// `gen_set_first_ref_var_copy` test, `record_copy_source` and `copies_as`.
    fn whole_value_bind(&self, v: u16, x: &Value) -> Option<(u16, u16)> {
        let Value::Var(src) = x.unspan() else {
            return None;
        };
        let data = self.cx.data;
        let d = self.vars.tp(v).base().heap_def_nr()?;
        let sd = self.vars.record_copy_source(data, v, *src)?;
        data.copies_as(d, sd)
            .then(|| (*src, data.def(d).known_type()))
    }

    /// `v = f(…)` binding a record variable to a loft function's result that the bind must
    /// copy rather than adopt — the bytecode writer's first-bind test: a callee that hands
    /// over a fresh store, an elided view and a minted store are adopted; anything else is
    /// copied, its source freed when `call_return_frees_source` says the copy owns it.  The
    /// record's store type and that flag; `None` to adopt.
    /// A rebind whose result is fresh on one arm and borrowed on the other (`Own::Join`) that
    /// `join_stmt` does not bind — the call reads the variable, or the variable does not own
    /// what it displaces — is this copy, as the bytecode writer's #360 path copies it.
    fn call_copy(&self, v: u16, x: &Value) -> Option<(u16, bool)> {
        use crate::data::Type;
        use crate::use_analysis as ua;
        let data = self.cx.data;
        let shape = ua::first_bind_shape(self.vars.tp(v)).clone();
        // A `τ?` that `first_bind_shape` leaves wrapped (`LOFT_NO_NULLABLE_RETBUF`) is not
        // bound here, as in the bytecode writer.
        if matches!(shape, Type::Optional(_)) {
            return None;
        }
        let (Type::Reference(d, _) | Type::Enum(d, true, _)) = shape.base().clone() else {
            return None;
        };
        let f = ua::callee_of(data, self.d, x)?;
        if !data.def(f).is_loft_defined() {
            return None;
        }
        if data.def(f).return_adopts_fresh_store()
            || self.vars.is_view_elided(v)
            || ua::adopts_minted_at_bind(data, self.vars, v, x)
        {
            return None;
        }
        Some((
            data.def(d).known_type(),
            ua::call_return_frees_source(data, self.d, x),
        ))
    }

    /// `v = f(…)` where the callee's result is fresh on one arm and a view of `base` on the
    /// other (`Own::Join`): the record type and `base`, the witness `OpBindOrCopy` tests.
    fn join_bind(&self, v: u16, x: &Value) -> Option<(u16, u16)> {
        use crate::data::Type;
        use crate::use_analysis as ua;
        let data = self.cx.data;
        let shape = ua::first_bind_shape(self.vars.tp(v)).clone();
        // A `τ?` that `first_bind_shape` leaves wrapped (`LOFT_NO_NULLABLE_RETBUF`) is not
        // bound here, as in the bytecode writer.
        if matches!(shape, Type::Optional(_)) {
            return None;
        }
        let (Type::Reference(d, _) | Type::Enum(d, true, _)) = shape.base().clone() else {
            return None;
        };
        let f = ua::callee_of(data, self.d, x)?;
        if !data.def(f).is_loft_defined()
            || !crate::keys::join_own_enabled()
            || self.vars.is_view_elided(v)
        {
            return None;
        }
        match ua::ownership_of(data, self.d, x) {
            ua::Own::Join { base } if base != u16::MAX => Some((data.def(d).known_type(), base)),
            _ => None,
        }
    }

    /// `v = c[i]` binding an owning record variable to an element of a container: the
    /// element is copied out (the bytecode writer's `gen_set_first_ref_elem_copy` test — no
    /// dependency, no owner witness, a projection of a container).  The store type.
    fn element_copy(&self, v: u16, x: &Value) -> Option<u16> {
        let tp = self.vars.tp(v);
        let d = tp.base().heap_def_nr()?;
        (tp.depend().is_empty()
            && self.vars.owner_witness(v).is_none()
            && crate::use_analysis::projection_container_var(self.cx.data, x).is_some())
        .then(|| self.cx.data.def(d).known_type())
    }

    /// Register record type `tp` for the deep copy, or refuse a kind it does not walk yet.
    fn copyable(&self, tp: u16) -> Result<u16, String> {
        self.cx.copy_type(tp)?;
        Ok(tp)
    }

    /// `v = f(…)` whose result is fresh on one arm and borrowed on the other (`Own::Join`):
    /// a first bind, or a rebind of a variable that owns what it displaces.  False when the
    /// bind is not one.
    fn join_stmt(
        &mut self,
        var: u16,
        x: &Value,
        first: bool,
        name: &str,
        ind: usize,
    ) -> Result<bool, String> {
        let join = self.join_bind(var, x);
        // A rebind releases the store it displaces when the variable owns it, as the bytecode
        // writer's owned reassignment does — never the base's, nor a store the call was handed.
        let rebind_owned = !first
            && join.is_some()
            && self.vars.owns_displaced_store(var, x, self.cx.data)
            && !x.reads_var(var);
        if let Some((tp, base)) = join.filter(|_| first || rebind_owned) {
            // OpBindOrCopy: a result in the base's store is a view, copied into a fresh store;
            // any other is the callee's fresh store, adopted.
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            let w = self.var(base);
            let old = self.temp(CType::Ref);
            self.line(ind, &format!("{old} = {name};"));
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.line(
                ind,
                &format!("if ({r}.store_nr != 0xFFFF && {r}.store_nr == {w}.store_nr) {{"),
            );
            self.line(ind + 1, &format!("{name} = loft_ref_null();"));
            self.line(
                ind + 1,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            self.line(ind, &format!("}} else {{ {name} = {r}; }}"));
            if rebind_owned {
                // Native's guard (`dispatch.rs`'s `_old_<v>` free, @FR-O-NoDiverge): kept only
                // where the new value or the call's pooled buffer (`@FR-O-Buffer`) is that store.
                // Not the base or another argument: the variable owns what it displaces, so an
                // argument naming that store is a view of the variable itself (`f(c ?? d)`'s
                // `__lift`), and keeping it there leaked the displaced record on every rebind.
                let mut keep = vec![format!("{old}.store_nr != {name}.store_nr")];
                if let Some(buf) = self.vars.displaced_buffer_witness(var, x, self.cx.data) {
                    keep.push(format!("{old}.store_nr != {}.store_nr", self.var(buf)));
                }
                self.line(
                    ind,
                    &format!("if ({}) loft_free_ref({old});", keep.join(" && ")),
                );
            }
            self.flush(ind);
            return Ok(true);
        }
        Ok(false)
    }

    /// A rebind's copy lands in a fresh store unless `var` owns the store it displaces
    /// (`Function::owns_displaced_store`, @FR-O-Proxy): a witnessed or borrowing local may hold
    /// a view of another record at that moment, and refilling its store in place wrote the copy
    /// into the viewed record.  An owned store is refilled, which is its free and a fresh mint.
    fn fresh_unless_owned(&mut self, var: u16, x: &Value, first: bool, name: &str, ind: usize) {
        // A witnessed local (`@FR-O-Witness`) never owns the store its slot names — its witness
        // does — and its slot may hold a VIEW into another store when its declaration runs again
        // in a loop; the bytecode's `OpInitRef` resets it there, so the copy mints a fresh store
        // instead of clearing the viewed one (the hash the view was into, measured).
        let witnessed = self.vars.owner_witness(var).is_some();
        if (first && witnessed) || (!first && !self.vars.owns_displaced_store(var, x, self.cx.data))
        {
            self.line(ind, &format!("{name} = loft_ref_null();"));
        }
    }

    /// `p = x` for a `&` parameter of a heap kind — a record, a vector, a struct-enum, a keyed
    /// collection: the caller's variable takes the new value (`OpSetStackRef`), and when the
    /// value is OWNED the one it held is released unless the new one lives in its store
    /// (`OpFreeRefIfDistinct`, @FR-B-Ref-Repoint) — the interpreter's `set_var` on a link.
    fn link_rebind(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        use crate::data::Type;
        let Type::RefVar(inner) = self.vars.tp(var).base() else {
            return Err("a link that is not a parameter".into());
        };
        if !matches!(
            inner.base(),
            Type::Reference(..)
                | Type::Vector(..)
                | Type::Enum(_, true, _)
                | Type::Hash(..)
                | Type::Sorted(..)
                | Type::Index(..)
                | Type::Radix(..)
                | Type::Trie(..)
        ) {
            return Err(format!("a rebind of a & {} parameter", type_word(inner)));
        }
        let owned = matches!(
            crate::use_analysis::ownership_of(self.cx.data, self.d, x),
            crate::use_analysis::Own::Owned
        );
        let e = self.expr(x, ind)?;
        let r = self.temp(CType::Ref);
        let name = self.var(var);
        self.line(ind, &format!("{r} = {};", as_ref(e)?));
        if owned {
            let old = self.temp(CType::Ref);
            self.line(ind, &format!("{old} = {name};"));
            self.line(ind, &format!("{name} = {r};"));
            self.line(ind, &format!("loft_free_displaced({old}, {name});"));
        } else {
            self.line(ind, &format!("{name} = {r};"));
        }
        self.flush(ind);
        Ok(())
    }

    /// `var = x`: a copy where loft copies a record, else the plain assignment.
    /// `@FR-R-TextBorrow` — the bind of a borrowed walk variable: the view the element read
    /// answers.  A view that would point into a temporary of this statement (freed when the
    /// statement ends) is refused by name rather than left dangling.
    fn borrowed_bind(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        // A block (the walk's `iter next` step): its statements run, its value is bound.
        if let Value::Block(b) = x.unspan()
            && let Some(at) = b
                .operators
                .iter()
                .rposition(|o| !matches!(o.unspan(), Value::Line(_)))
        {
            for op in &b.operators[..at] {
                self.stmt(op, ind)?;
            }
            self.flush(ind);
            return self.borrowed_bind(var, &b.operators[at], ind);
        }
        // The element's text field read as the view it is: the analysis proved the loop writes
        // no store, so the bytes stay where they are for as long as the variable is read.
        if let Value::Call(d, args) = x.unspan()
            && self.cx.data.def(*d).name() == "OpGetText"
            && let [r, fld] = args.as_slice()
            && self.lazy_vector(r, "OpGetVectorNullable").is_none()
            && !matches!(r.unspan(), Value::Call(_, ra) if ra.first().is_some_and(|v| self.split_tab_of(v).is_some()))
        {
            let fld = Self::literal(fld, "field")?;
            let before = self.pending.len();
            let r = self.expr(r, ind)?;
            if self.pending.len() != before {
                return Err("a borrowed text would view a temporary".into());
            }
            let view = format!("loft_get_text({}, {fld})", as_ref(r)?);
            return self.borrowed_assign(var, x, &view, ind);
        }
        let before = self.pending.len();
        let e = self.expr(x, ind)?;
        if self.pending.len() != before {
            return Err("a borrowed text would view a temporary".into());
        }
        let view = as_view(e)?;
        self.borrowed_assign(var, x, &view, ind)
    }

    /// A borrowed local takes its view — under `@FR-R-TextBorrow`'s tested clause only while
    /// each root parameter's store is not the return buffer's, which the bind tests; else the
    /// bytes are copied into the local's own text (`loft_own<v>`) and it views that.  A literal,
    /// a null or another variable borrows as it stands.
    fn borrowed_assign(
        &mut self,
        var: u16,
        x: &Value,
        view: &str,
        ind: usize,
    ) -> Result<(), String> {
        let name = self.var(var);
        let roots = self.tested.get(&var).cloned();
        let rb = crate::opt::hoist::retbuf_var(self.cx.data, self.d);
        let (Some(roots), Some(rb)) = (roots, rb) else {
            self.line(ind, &format!("{name} = {view};"));
            return Ok(());
        };
        if matches!(x.unspan(), Value::Text(_) | Value::Null | Value::Var(_)) {
            self.line(ind, &format!("{name} = {view};"));
            return Ok(());
        }
        let b = as_ref(self.expr(&Value::Var(rb), ind)?)?;
        let mut tests = Vec::new();
        for p in roots {
            let r = as_ref(self.expr(&Value::Var(p), ind)?)?;
            tests.push(format!("{r}.store_nr != {b}.store_nr"));
        }
        crate::rewrite_census::fired("R-TextBorrow/tested", 1);
        self.line(
            ind,
            &format!(
                "{{ loft_str lt = {view}; if ({}) {name} = lt; else {{ loft_text_set(&loft_own{var}, lt); {name} = loft_view(&loft_own{var}); }} }}",
                tests.join(" && ")
            ),
        );
        Ok(())
    }

    /// `@FR-R-LazySplit` — the lazy vector `vec` when `v` is `reader(Var(vec), …)`.
    fn lazy_vector(&self, v: &Value, reader: &str) -> Option<u16> {
        let Value::Call(d, args) = v.unspan() else {
            return None;
        };
        if self.cx.data.def(*d).name() != reader {
            return None;
        }
        let Value::Var(vec) = args.first()?.unspan() else {
            return None;
        };
        self.lazy.contains_key(vec).then_some(*vec)
    }

    /// `@FR-R-LazySplit` — the three operators a lazy loop's vector is mentioned by: its element
    /// read is the iterator's next piece, its length test (`len <= i`, the loop's exit) is the
    /// iterator having run out, and the mint of the split's own buffer, which only this call
    /// served, is nothing.  `None` for every other operator.
    fn lazy_split_op(&self, name: &str, args: &[Value]) -> Option<E> {
        match (name, args) {
            ("OpGetText", [elem, _]) => {
                let vec = self.lazy_vector(elem, "OpGetVectorNullable")?;
                Some(E::new(
                    format!("loft_ls_next(&ls{vec}, &lsd{vec})"),
                    K::View,
                ))
            }
            ("OpLeInt", [len, _]) => {
                let vec = self.lazy_vector(len, "OpLengthVector")?;
                Some(E::new(format!("(lsd{vec} != 0)"), K::Cond))
            }
            ("OpDatabase", [Value::Var(b), _])
                if self.lazy.values().any(|ls| ls.dead_buf == Some(*b)) =>
            {
                Some(E::new("((void) 0)", K::Void))
            }
            _ => None,
        }
    }

    /// `@FR-R-SplitTable` — the bind of a table's local: the pieces recorded in `st<var>`.  The
    /// vector's own null initialisation binds nothing.
    fn split_tab_bind(
        &mut self,
        var: u16,
        tab: &crate::opt::text_borrow::SplitTable,
        x: &Value,
        ind: usize,
    ) -> Result<(), String> {
        if matches!(x.unspan(), Value::Null) {
            return Ok(());
        }
        let Value::Call(_, args) = x.unspan() else {
            return Err("a split table bound from something other than its call".into());
        };
        let src = args.first().ok_or("a split call without its text")?;
        let e = self.expr(src, ind)?;
        let view = as_view(e)?;
        self.line(
            ind,
            &format!(
                "loft_stab_fill(&st{var}, {view}, {}u);",
                u32::from(tab.separator)
            ),
        );
        crate::rewrite_census::fired("R-SplitTable", 1);
        self.flush(ind);
        Ok(())
    }

    /// The table a vector expression `v` names: a table's local or a walk's alias of one.
    fn split_tab_of(&self, v: &Value) -> Option<u16> {
        let Value::Var(x) = v.unspan() else {
            return None;
        };
        if self.split_tabs.contains_key(x) {
            Some(*x)
        } else {
            self.tab_alias.get(x).copied()
        }
    }

    /// `@FR-R-SplitTable`'s readers: the length, the element's text, and the mint of a buffer
    /// only the split call served.  `None` for every other operator.
    fn split_tab_op(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let data = self.cx.data;
        match (name, args) {
            ("OpLengthVector", [v]) => Ok(self
                .split_tab_of(v)
                .map(|t| E::new(format!("((int64_t) st{t}.n)"), K::Int))),
            ("OpGetText", [elem, _]) => {
                let Value::Call(d, inner) = elem.unspan() else {
                    return Ok(None);
                };
                if !matches!(data.def(*d).name(), "OpGetVectorNullable" | "OpGetVector") {
                    return Ok(None);
                }
                let (Some(v), Some(idx)) = (inner.first(), inner.get(2)) else {
                    return Ok(None);
                };
                let Some(t) = self.split_tab_of(v) else {
                    return Ok(None);
                };
                let i = self.expr(idx, ind)?;
                Ok(Some(E::new(
                    format!("loft_stab_get(&st{t}, {})", as_int(i)?),
                    K::View,
                )))
            }
            ("OpDatabase", [Value::Var(b), _])
                if self.split_tabs.values().any(|t| t.dead_buf == Some(*b)) =>
            {
                Ok(Some(E::new("((void) 0)", K::Void)))
            }
            _ => Ok(None),
        }
    }

    /// `@FR-R-LazySplit` — the bind of a lazy loop's vector: an iterator over the source text.
    /// A text parameter nothing in the function writes is walked in place; any other source is
    /// copied first, so no write in the loop body can reach what is being walked.
    fn lazy_split_bind(
        &mut self,
        var: u16,
        ls: &crate::opt::lazy_split::LazySplit,
        x: &Value,
        ind: usize,
    ) -> Result<(), String> {
        // The vector's own null initialisation: it is never built, so there is nothing to set.
        if matches!(x.unspan(), Value::Null) {
            return Ok(());
        }
        let Value::Call(_, args) = x.unspan() else {
            return Err("a lazy split bound from something other than its call".into());
        };
        let src = args.first().ok_or("a split call without its text")?;
        if !self
            .decls
            .iter()
            .any(|d| d.starts_with(&format!("loft_ls ls{var} ")))
        {
            self.decls.push(format!(
                "loft_ls ls{var} = {{ {{ 0, 0 }}, 0, {{ 0 }}, 0 }};"
            ));
            self.decls.push(format!("uint8_t lsd{var} = 0;"));
        }
        let sep = u32::from(ls.separator);
        let in_place = matches!(src.unspan(), Value::Var(p)
            if self.vars.is_argument(*p)
                && !self.links[*p as usize]
                && self.shapes[*p as usize] == CType::Text
                && !crate::opt::char_walk::text_written(self.code, *p, self.cx.data));
        let e = self.expr(src, ind)?;
        let view = as_view(e)?;
        if in_place {
            self.line(ind, &format!("ls{var} = loft_ls_of({view}, {sep}u);"));
        } else {
            let own = self.temp(CType::Text);
            self.line(ind, &format!("loft_text_set(&{own}, {view});"));
            self.line(
                ind,
                &format!("ls{var} = loft_ls_of(loft_view(&{own}), {sep}u);"),
            );
        }
        self.line(ind, &format!("lsd{var} = 0;"));
        self.flush(ind);
        Ok(())
    }

    /// The binds a rewrite writes itself: a borrowed text's view (`@FR-R-TextBorrow`) and a
    /// lazy split's iterator (`@FR-R-LazySplit`).  `None` for every other bind.
    fn rewritten_bind(&mut self, var: u16, x: &Value, ind: usize) -> Option<Result<(), String>> {
        // A walk's alias names the table; a buffer only the table's split would have filled is
        // never minted.
        if self.tab_alias.contains_key(&var)
            || self.split_tabs.values().any(|t| t.dead_buf == Some(var))
        {
            return Some(Ok(()));
        }
        if let Some(tab) = self.split_tabs.get(&var).cloned() {
            return Some(self.split_tab_bind(var, &tab, x, ind));
        }
        if self.shapes[var as usize] == CType::Text && self.borrowed[var as usize] {
            return Some(self.borrowed_bind(var, x, ind));
        }
        let ls = self.lazy.get(&var).cloned()?;
        Some(self.lazy_split_bind(var, &ls, x, ind))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per kind of bind; the rewritten binds are already helpers"
    )]
    fn set_stmt(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        if self.vr.is_some()
            && let Some(r) = self.vr_set(var, x, ind)
        {
            self.bound[var as usize] = true;
            return r;
        }
        // An `&` local is bound to the variable `OpCreateStack` names: it points there, and
        // every later read and write of it goes through.
        if self.store_links[var as usize] {
            return self.store_link_set(var, x, ind);
        }
        if self.link_repoint(var, x, ind)? {
            return Ok(());
        }
        if self.links[var as usize] && self.shapes[var as usize] == CType::Ref {
            return self.link_rebind(var, x, ind);
        }
        if let Some(a) = self.adopt
            && var == a.v
            && matches!(x.unspan(), Value::Call(d, _) if self.cx.data.def(*d).name() == "OpGetField")
        {
            self.bound[var as usize] = true;
            self.adopted_init(a, ind)?;
            return Ok(());
        }
        let first = !std::mem::replace(&mut self.bound[var as usize], true);
        // An owned vector variable bound to null gets a store of its own whose record holds
        // the (empty) vector — the bytecode writer's `gen_set_first_vector_null`: the variable
        // then names that field.  A skip-free, inline or lazy-buffer one, and a view of another
        // variable (bound to its target later), keep the sentinel.  A lazy buffer's LATER
        // `Set(v, Null)` is its mint (`@FR-O-LazyBuffer`): the scopes pass guards it with
        // `OpRefIsNull(v)`.
        let lazy = self.vars.is_lazy_buffer(var);
        if matches!(x.unspan(), Value::Null)
            && crate::parser::vectors::owns_keyed_store(self.vars.tp(var))
        {
            self.keyed_null(var, first, ind);
            return Ok(());
        }
        if matches!(x.unspan(), Value::Null)
            && let crate::data::Type::Vector(_, dep) = self.vars.tp(var).base()
            && dep.is_empty()
            && (if lazy {
                !first
            } else {
                !(self.vars.is_skip_free(var) || self.vars.is_inline_ref(var))
            })
            && !self.split_tabs.values().any(|t| t.dead_buf == Some(var))
        {
            self.owned_vector_null(var, ind);
            return Ok(());
        }
        let c = self.shapes[var as usize];
        if let Some(done) = self.rewritten_bind(var, x, ind) {
            return done;
        }
        if c == CType::Text && !self.text_held(var) {
            return Err("assigns a text parameter".into());
        }
        let name = self.var(var);
        if self.join_stmt(var, x, first, &name, ind)?
            || self.opaque_callref_stmt(var, x, first, &name, ind)?
        {
            return Ok(());
        }
        if let Some((tp, frees)) = self.call_copy(var, x) {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.fresh_unless_owned(var, x, first, &name, ind);
            if frees {
                // The stores of the call's reference arguments are protected, as the bytecode
                // writer protects them (`protectable_ref_args`): a result that is a view into
                // one of them is not the callee's to give away.
                let mut keep = vec![format!("{r}.store_nr != {name}.store_nr")];
                for a in crate::use_analysis::protectable_ref_args(self.cx.data, self.d, x).0 {
                    keep.push(format!("{r}.store_nr != {}.store_nr", self.var(a)));
                }
                let keep = keep.join(" && ");
                // `@FR-H-SwapRebind`: a result the bind gives up is exchanged into the
                // destination where the store allows; a fn-ref call's result may be a store it
                // borrowed, and a type whose tree holds a stored reference keeps the copy.
                if matches!(x.unspan(), Value::Call(..)) && self.cx.swap_safe(bytes) {
                    crate::rewrite_census::fired("H-SwapRebind", 1);
                    self.line(
                        ind,
                        &format!(
                            "if (!({keep} && loft_swap_rebind({bytes}, {r}, &{name}))) {{ \
                             loft_bind_copy(loft_types, {bytes}, &{name}, {r}); \
                             if ({keep}) loft_free_ref({r}); }}"
                        ),
                    );
                } else {
                    self.line(
                        ind,
                        &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
                    );
                    self.line(ind, &format!("if ({keep}) loft_free_ref({r});"));
                }
            } else {
                self.line(
                    ind,
                    &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
                );
            }
            self.flush(ind);
            return Ok(());
        }
        if let Some(tp) = self.element_copy(var, x) {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.fresh_unless_owned(var, x, first, &name, ind);
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            self.flush(ind);
            return Ok(());
        }
        if let Some((src, tp)) = self.whole_value_bind(var, x) {
            let bytes = self.copyable(tp)?;
            let src = self.var(src);
            self.fresh_unless_owned(var, x, first, &name, ind);
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {src});"),
            );
            return Ok(());
        }
        if !first && c == CType::Ref && self.displacing_rebind(var, x, &name, ind)? {
            return Ok(());
        }
        // A tuple local this function owns gives up the texts of its old value when a call's
        // tuple moves in (`assign`); a parameter's or a link's texts are the caller's.
        let owner = !self.params[var as usize] && !self.links[var as usize];
        let outer = std::mem::replace(&mut self.frees_old_tuple, owner);
        let done = self.assign(x, &name, c, ind);
        self.frees_old_tuple = outer;
        done?;
        self.flush(ind);
        Ok(())
    }

    /// The variable `OpCreateStack(v)` names, whose shape a link to it shares; `None` for any
    /// other value, and for a target that is itself a parameter passed by value.
    /// `var = x` for a `&` variable that `x` re-points (`@FR-B-Ref-Repoint`): it points at a
    /// variable, a tuple member, what another link names, or a store place from here on.
    /// Answers whether it wrote the re-point; a write through the link is no re-point.
    fn link_repoint(&mut self, var: u16, x: &Value, ind: usize) -> Result<bool, String> {
        let repoint = self.links[var as usize]
            && (!self.params[var as usize]
                || crate::scopes::link_set_repoints(self.cx.data, self.vars, var, x));
        if !repoint {
            return Ok(false);
        }
        let to = match self.stack_target(x) {
            Some(to) => Some(to),
            None if self.shapes[var as usize] != CType::Ref => self.direct_place(var, x, ind)?,
            None => None,
        };
        if let Some(to) = to {
            self.bound[var as usize] = true;
            let name = var_name(self.vars.name(var), var);
            self.line(ind, &format!("{name} = {to};"));
            return Ok(true);
        }
        if !self.bound[var as usize] {
            return Err("an & local bound to something other than a variable".into());
        }
        Ok(false)
    }

    /// A link of `var`'s kind re-pointed to a store place — a field or an element — whose bytes
    /// are the C value itself (an 8-byte integer, a float, a single, a character, a boolean, a
    /// plain enum): the address of the place, as `--native` takes it.  An absent place (a null
    /// record) is a temporary holding the null, which a write through the link leaves unseen.
    fn direct_place(&mut self, var: u16, x: &Value, ind: usize) -> Result<Option<String>, String> {
        use crate::data::Type;
        let Value::Call(d, _) = x.unspan() else {
            return Ok(None);
        };
        if !matches!(self.cx.data.def(*d).name(), "OpGetVector" | "OpGetField") {
            return Ok(None);
        }
        let Type::RefVar(tp) = self.vars.tp(var).base() else {
            return Ok(None);
        };
        let direct = match tp.base() {
            Type::Integer(_) => crate::data::NarrowSlot::of_type(tp).is_none(),
            Type::Float | Type::Single | Type::Character | Type::Boolean => true,
            Type::Enum(_, false, _) => true,
            _ => false,
        };
        if !direct {
            return Ok(None);
        }
        let c = self.shapes[var as usize];
        let absent = self.temp(c);
        self.null_into(&absent, c, ind);
        let e = self.expr(x, ind)?;
        Ok(Some(format!(
            "({} *) loft_place({}, &{absent})",
            held(c),
            as_ref(e)?
        )))
    }

    fn stack_target(&self, x: &Value) -> Option<String> {
        let Value::Call(d, args) = x.unspan() else {
            return None;
        };
        match (self.cx.data.def(*d).name(), args.as_slice()) {
            ("OpCreateStack", [Value::Var(t)])
                if !(self.params[*t as usize]
                    && !self.links[*t as usize]
                    && self.shapes[*t as usize] == CType::Text) =>
            {
                Some(format!("&{}", self.var(*t)))
            }
            // A member of a tuple local is a place a link names (`@FR-B-Ref-Lvalue`).
            // A text member of a tuple parameter is the caller's: no link writes it.
            ("OpCreateStack", [Value::TupleGet(t, i)])
                if !self.params[*t as usize]
                    || self.links[*t as usize]
                    || !matches!(self.shapes[*t as usize], CType::Tuple(id)
                        if self.cx.tuple_shapes(id).get(*i as usize) == Some(&CType::Text)) =>
            {
                Some(format!("&{}.f{i}", self.var(*t)))
            }
            // A link to a link names what that link names.
            ("OpVarRef", [Value::Var(t)])
                if self.links[*t as usize] && !self.store_links[*t as usize] =>
            {
                Some(var_name(self.vars.name(*t), *t))
            }
            _ => None,
        }
    }

    /// An owned vector variable bound to null: a fresh store minted as `main_vector<elm>`, the
    /// type the bytecode writer's `OpDatabase` names (`gen_owned_vector_store`), so a later
    /// wrapper mint of it clears it in place.
    fn owned_vector_null(&mut self, var: u16, ind: usize) {
        let crate::data::Type::Vector(elm, _) = self.vars.tp(var).base() else {
            return;
        };
        let data = self.cx.data;
        let known = data.owned_vector_type(elm, data.def(self.d).source());
        let size = self.cx.stores.enum_parent_size(known);
        self.cx.describe_type(known);
        let name = self.var(var);
        self.line(ind, &format!("{name} = loft_ref_null();"));
        self.line(
            ind,
            &format!("loft_database_typed(&{name}, {size}, {known});"),
        );
    }

    /// A keyed local bound to null gets a store too, unless it borrows one
    /// (`gen_keyed_null`); a later bind to null empties that store in place.
    fn keyed_null(&mut self, var: u16, first: bool, ind: usize) {
        let name = self.var(var);
        if first {
            self.line(ind, &format!("{name} = loft_ref_null();"));
        }
        if !(self.vars.is_inline_ref(var) || self.vars.is_overwritten_view(var)) {
            // The store is minted AS the collection type, as `gen_keyed_null`'s `OpDatabase`
            // mints it: its root is that type's record and carries its tag.
            match self.cx.keyed_local_type(self.vars.tp(var)) {
                Some(tp) => {
                    let size = self.cx.stores.enum_parent_size(tp);
                    self.cx.describe_type(tp);
                    self.line(ind, &format!("loft_database_typed(&{name}, {size}, {tp});"));
                }
                None => self.line(ind, &format!("loft_database(&{name}, 8);")),
            }
        }
    }

    /// `v = f(…)` through a function value whose target nothing here resolves
    /// (`use_analysis::opaque_callref_bind`, `@FR-O-Unknown`): the record handed back is adopted
    /// when the call minted its store, and copied into one of `v`'s own when it predates the
    /// call — an argument or a capture, owned further up (`State::bind_fn_ref_result`, native's
    /// `cr_fnref_adopt`).  False when the bind is not one.
    fn opaque_callref_stmt(
        &mut self,
        var: u16,
        x: &Value,
        first: bool,
        name: &str,
        ind: usize,
    ) -> Result<bool, String> {
        let data = self.cx.data;
        let Value::CallRef(fv, _) = x.unspan() else {
            return Ok(false);
        };
        let Some(rec) =
            crate::use_analysis::opaque_callref_bind(data, self.d, self.vars.tp(var), x.unspan())
        else {
            return Ok(false);
        };
        let tp = self.copyable(data.def(rec).known_type())?;
        let size = self.cx.stores.enum_parent_size(tp);
        let witnessed = self.vars.owner_witness(var).is_some();
        let (dst, src, since) = (
            self.temp(CType::Ref),
            self.temp(CType::Ref),
            self.temp(CType::Int),
        );
        let clos = format!("{}.clos", self.var(*fv));
        self.line(ind, &format!("{dst} = {name};"));
        self.line(ind, &format!("{since} = (int64_t) loft_stores_allocated;"));
        let e = self.expr(x, ind)?;
        self.line(ind, &format!("{src} = {};", as_ref(e)?));
        self.line(
            ind,
            &format!("if (loft_fnref_minted({src}, (uint64_t) {since}, {clos})) {{"),
        );
        // The binding is the adopted store's one owner (`cr_fnref_adopt`).
        self.line(ind + 1, &format!("loft_fnref_adopt({src});"));
        // A parameter's displaced store is the caller's (a promoted return buffer the caller
        // pools, `@FR-R-Reuse`): the adopt only rebinds it, as `State::bind_fn_ref_result` does.
        if !witnessed && !self.params[var as usize] {
            self.line(
                ind + 1,
                &format!(
                    "if ({dst}.store_nr != 0xFFFF && {dst}.store_nr != {src}.store_nr) loft_free_ref({dst});"
                ),
            );
        } else if !witnessed && let Some(w) = self.vars.entry_witness(var) {
            // …unless it is not the caller's: a promoted return buffer's entry witness names
            // the store the caller handed (`@FR-O-Buffer`), and any other it displaces is one
            // this frame adopted on an earlier pass.
            self.line(
                ind + 1,
                &format!(
                    "if ({dst}.store_nr != 0xFFFF && {dst}.store_nr != {src}.store_nr && {dst}.store_nr != {}.store_nr) loft_free_ref({dst});",
                    self.var(w)
                ),
            );
        }
        self.line(ind + 1, &format!("{name} = {src};"));
        self.line(ind, "} else {");
        if witnessed && !first {
            self.line(ind + 1, &format!("{name} = loft_ref_null();"));
        }
        self.line(
            ind + 1,
            &format!("loft_database_typed(&{name}, {size}, {tp});"),
        );
        self.line(
            ind + 1,
            &format!("loft_copy_record(loft_types, {tp}, {src}, {name}, 4);"),
        );
        self.line(ind, "}");
        self.flush(ind);
        Ok(true)
    }

    /// A rebind of an owned reference from an expression that produces a store releases the
    /// store it displaces, unless the new value or the call's buffer lives there (native's
    /// `owned_ref_reassign`; the interpreter's post-Set `OpFreeRefIfDistinct`).  False when
    /// the rebind displaces nothing it owns.
    fn displacing_rebind(
        &mut self,
        var: u16,
        x: &Value,
        name: &str,
        ind: usize,
    ) -> Result<bool, String> {
        // A promoted return buffer is an argument, and frees what it displaces only with its
        // entry witness (`@FR-O-Buffer`), which keeps the caller's store: native's
        // `owned_ref_reassign` with `_rb_w_<v>`.
        let entry = self.vars.entry_witness(var);
        if (self.vars.is_argument(var) && entry.is_none())
            || !matches!(
                x.unspan(),
                Value::Call(..)
                    | Value::CallRef(..)
                    | Value::Insert(_)
                    | Value::Block(_)
                    | Value::If(..)
            )
            || !self.vars.owns_displaced_store(var, x, self.cx.data)
        {
            return Ok(false);
        }
        let old = self.temp(CType::Ref);
        self.line(ind, &format!("{old} = {name};"));
        self.assign(x, name, CType::Ref, ind)?;
        let mut keep = format!("{old}.store_nr != {name}.store_nr");
        if let Some(w) = entry.filter(|_| self.vars.is_argument(var)) {
            let _ = write!(keep, " && {old}.store_nr != {}.store_nr", self.var(w));
        }
        if let Some(buf) = self.vars.displaced_buffer_witness(var, x, self.cx.data) {
            let _ = write!(keep, " && {old}.store_nr != {}.store_nr", self.var(buf));
        }
        self.line(ind, &format!("if ({keep}) loft_free_ref({old});"));
        self.flush(ind);
        Ok(true)
    }

    fn block_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        let delivery = self.delivery_block(b);
        self.in_delivery += u32::from(delivery);
        let r = self.ops_stmt(&b.operators, ind);
        self.in_delivery -= u32::from(delivery);
        r
    }

    /// `@FR-R-RetAdopt` — is `b` the exit delivery of an adopted result local?  Inside it the
    /// buffer's clear would wipe the result and its append would copy the buffer into itself,
    /// so both emit nothing; the entry clear (outside any such block) stays, as the buffer's
    /// reuse contract.
    fn delivery_block(&self, b: &Block) -> bool {
        self.adopt.is_some() && b.name == "one_buffer_vec_copy"
    }

    /// `@FR-R-RetAdopt` — the adopted local's bind: it IS the return buffer.  A caller that
    /// offered none gets one minted exactly as the witness would have been, so ownership
    /// travels the same route; an offered one is emptied, as the delivery's clear did.
    fn adopted_init(
        &mut self,
        a: crate::opt::ret_adopt::RetAdopt,
        ind: usize,
    ) -> Result<(), String> {
        let mint =
            self.store_operator("OpDatabase", &[Value::Var(a.buf), Value::Int(a.db_tp)], ind)?;
        self.cx.name_types();
        let buf = self.var(a.buf);
        self.line(
            ind,
            &format!(
                "if ({buf}.store_nr == 0xFFFF || {buf}.rec == 0) {{ {}; }} else {{ loft_vec_clear_release(loft_types, loft_ntypes, {buf}); }}",
                mint.code
            ),
        );
        let v = self.var(a.v);
        self.line(ind, &format!("{v} = {buf};"));
        Ok(())
    }

    /// The adopted return buffer's variable when `v` names it.
    fn adopted_buffer(&self, v: &Value) -> bool {
        matches!((self.adopt, v.unspan()), (Some(a), Value::Var(b)) if a.buf == *b)
    }

    /// Emit the statements `v` needs and put its value in `target`, of shape `c`.
    #[expect(
        clippy::too_many_lines,
        reason = "past the bar where the 32-bit arms met the value-record arms on a join; the split is release work (M-file-split)"
    )]
    fn assign(&mut self, v: &Value, target: &str, c: CType, ind: usize) -> Result<(), String> {
        if self.vr.is_some() && self.vr_assign(v, target, c, ind)? {
            return Ok(());
        }
        match v.unspan() {
            // A tuple that owns texts is copied element by element: a struct copy would share them.
            Value::Var(src) if matches!(c, CType::Tuple(id) if self.cx.tuple_has_text(id)) => {
                let CType::Tuple(id) = c else { unreachable!() };
                for (i, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    let i16 = u16::try_from(i).map_err(|_| "a tuple too wide")?;
                    self.assign(
                        &Value::TupleGet(*src, i16),
                        &format!("{target}.f{i}"),
                        e,
                        ind,
                    )?;
                }
                Ok(())
            }
            // The same for a tuple read out of a tuple local (`pr = t.1`): its texts are the
            // ones the epilogue frees, so the local takes copies — a struct copy shared the
            // source's, and the free released text the caller still held.
            Value::TupleGet(src, i) if matches!(c, CType::Tuple(id) if self.cx.tuple_has_text(id)) =>
            {
                let CType::Tuple(id) = c else { unreachable!() };
                let from = format!("{}.f{i}", self.var(*src));
                self.copy_member(&from, target, CType::Tuple(id), ind);
                Ok(())
            }
            // A function value read out of a store: its definition number and its closure, the
            // two values the bytecode pushes as the pair (`fn_ref_field_read`).
            Value::Block(b) if c == CType::Fn && b.name == "fn_ref_field_read" => {
                let ops: Vec<&Value> = b
                    .operators
                    .iter()
                    .filter(|o| !matches!(o.unspan(), Value::Line(_)))
                    .collect();
                // The definition number is pushed first; statements between the two values
                // (a displaced record's release) run before the closure is read.
                let (Some(d), Some(clos)) = (ops.first(), ops.last()) else {
                    return Err("a function value read with an unexpected shape".into());
                };
                if ops.len() < 2 {
                    return Err("a function value read with an unexpected shape".into());
                }
                let first = self.expr(d, ind)?;
                let dt = self.temp(CType::Int);
                self.line(ind, &format!("{dt} = {};", as_int(first)?));
                for o in &ops[1..ops.len() - 1] {
                    self.stmt(o, ind)?;
                }
                let d = E::new(dt, K::Int);
                let clos = self.expr(clos, ind)?;
                self.line(
                    ind,
                    &format!(
                        "{target} = loft_fn_make({}, {});",
                        as_int(d)?,
                        as_ref(clos)?
                    ),
                );
                Ok(())
            }
            // A function named as a value: its definition number.
            Value::Int(d) if c == CType::Fn && self.cx.is_function(*d) => {
                let f = self.cx.fn_value(d.unsigned_abs());
                self.line(ind, &format!("{target} = {f};"));
                Ok(())
            }
            Value::Null => {
                self.null_into(target, c, ind);
                Ok(())
            }
            Value::Block(b) => {
                let delivery = self.delivery_block(b);
                self.in_delivery += u32::from(delivery);
                let r = self.assign_seq(&b.operators, target, c, ind);
                self.in_delivery -= u32::from(delivery);
                r
            }
            Value::Insert(ops) => self.assign_seq(ops, target, c, ind),
            Value::If(cond, t, e) => self.assign_if(cond, t, e, target, c, ind),
            Value::Tuple(vals) => self.assign_tuple(vals, target, c, ind),
            v @ (Value::Return(_) | Value::Break(_) | Value::Continue(_)) => self.stmt(v, ind),
            // A self tail call's loop (`opt::tail_self`) leaves only by a `return`: it has no
            // value to put anywhere.
            v @ Value::Loop(lp) if lp.name == "tail_self" => self.stmt(v, ind),
            // @PLN204 — a value stored into a 32-bit local is narrowed, so an operation of two
            // 32-bit operands it stores is the 32-bit operation (`narrow32`): a loop's
            // `i = i + 1` runs in `int32_t`.
            v @ (Value::Call(..) | Value::Int(_)) if c == CType::Int32 => {
                let e = self.narrow32(v, ind)?;
                let code = as_int32(e)?;
                self.line(ind, &format!("{target} = {code};"));
                Ok(())
            }
            v => {
                let e = self.expr(v, ind)?;
                // A temporary tuple that owns texts keeps them — its buffers are reused and the
                // epilogue frees them — so the target takes copies; a call's result moves.
                if let CType::Tuple(id) = c
                    && self.cx.tuple_has_text(id)
                    && self.temps.iter().any(|(t, _)| *t == e.code)
                {
                    self.copy_member(&e.code, target, c, ind);
                    return Ok(());
                }
                if let CType::Tuple(id) = c
                    && self.cx.tuple_has_text(id)
                    && self.frees_old_tuple
                {
                    return self.move_tuple_in(e, target, id, ind);
                }
                // A text value held in a statement temporary (a call's fresh result) moves into
                // the target: the temporary is released at the statement's end and nothing else
                // names it, so the target takes its buffer instead of a copy.
                // `LOFT_NO_C_TEXT_MOVE=1` copies.
                if c == CType::Text
                    && let Some(t) = self.pending_text_temp(&e.code)
                {
                    self.line(ind, &format!("loft_text_move(&{target}, &{t});"));
                    return Ok(());
                }
                let s = match c {
                    CType::Text => format!("loft_text_set(&{target}, {});", as_view(e)?),
                    CType::Void => format!("{};", e.code),
                    c => format!("{target} = {};", as_shape(e, c)?),
                };
                self.line(ind, &s);
                Ok(())
            }
        }
    }

    /// A call's tuple `e` moved into the owned tuple local `target`, which may still hold the
    /// texts of the value it had (a loop body's local, every pass after the first): those are
    /// freed once the call has run — it may read them — and before the move (loft#2201).
    fn move_tuple_in(&mut self, e: E, target: &str, id: u32, ind: usize) -> Result<(), String> {
        let c = CType::Tuple(id);
        let mut old = Vec::new();
        self.tuple_text_leaves(target, id, &mut old);
        let mut frees = String::new();
        for t in &old {
            let _ = write!(frees, " loft_text_free(&{t});");
        }
        let value = as_shape(e, c)?;
        self.line(
            ind,
            &format!("{{ {} mv = {value};{frees} {target} = mv; }}", held(c)),
        );
        Ok(())
    }

    /// A tuple literal into `target`, of shape `c`.
    fn assign_tuple(
        &mut self,
        vals: &[Value],
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        let CType::Tuple(id) = c else {
            return Err("a tuple literal into a non-tuple".into());
        };
        let shapes = self.cx.tuple_shapes(id);
        if shapes.len() != vals.len() {
            return Err("a tuple literal of another arity".into());
        }
        // Every element is evaluated before any is stored, as the bytecode pushes them
        // all first: `s = (s.1, s.0)` reads the old members.  A literal reads nothing.
        let constant = |x: &Value| {
            matches!(
                x.unspan(),
                Value::Int(_)
                    | Value::Long(_)
                    | Value::Float(_)
                    | Value::Single(_)
                    | Value::Boolean(_)
                    | Value::Text(_)
                    | Value::Null
            )
        };
        if vals.len() > 1 && !vals.iter().all(constant) {
            let mut held = Vec::with_capacity(vals.len());
            for (x, s) in vals.iter().zip(&shapes) {
                let t = self.temp(*s);
                self.assign(x, &t, *s, ind)?;
                held.push(t);
            }
            for (i, (t, s)) in held.iter().zip(shapes).enumerate() {
                self.copy_member(t, &format!("{target}.f{i}"), s, ind);
            }
            return Ok(());
        }
        for (i, (x, s)) in vals.iter().zip(shapes).enumerate() {
            self.assign(x, &format!("{target}.f{i}"), s, ind)?;
        }
        Ok(())
    }

    fn assign_seq(
        &mut self,
        ops: &[Value],
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        if self.tail_capture(ops).is_some() {
            return self.ops_stmt(ops, ind);
        }
        // The value is the last operator that is not a free: frees after it run once it is
        // taken, as they run under it on the bytecode's evaluation stack.  A `Set` has no value,
        // so the reset that follows a free (`__disp = OpNullRefSentinel()`) is one of them.
        let at = ops
            .iter()
            .rposition(|o| {
                !matches!(o.unspan(), Value::Line(_) | Value::Set(..)) && !self.is_free(o)
            })
            .ok_or("a value block with no value")?;
        // A view the block binds before its value serves the rest of it (`@FR-R-View`), the
        // value included; its frame closes with the block.
        let frames = self.headers.len();
        let r = (|| {
            for i in 0..at {
                self.stmt(&ops[i], ind)?;
                self.bind_view_header(ops, i, ind);
            }
            self.assign(&ops[at], target, c, ind)?;
            self.flush(ind);
            ops[at + 1..].iter().try_for_each(|o| self.stmt(o, ind))
        })();
        self.headers.truncate(frames.min(self.headers.len()));
        r
    }

    /// `to = from` for a value of shape `c` at two C places: a text copied, a tuple member by
    /// member at any depth (its texts the destination's own), anything else assigned.
    pub(super) fn copy_member(&mut self, from: &str, to: &str, c: CType, ind: usize) {
        match c {
            CType::Text => self.line(ind, &format!("loft_text_set(&{to}, loft_view(&{from}));")),
            CType::Tuple(id) => {
                for (j, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.copy_member(&format!("{from}.f{j}"), &format!("{to}.f{j}"), e, ind);
                }
            }
            _ => self.line(ind, &format!("{to} = {from};")),
        }
    }

    /// `t = null` for a tuple whose texts stay where they are: every member but a text set to
    /// its null, so the text buffers are reused rather than lost.
    fn null_tuple_keeping_texts(&mut self, t: &str, c: CType, ind: usize) {
        match c {
            CType::Text => {}
            CType::Tuple(id) => {
                for (j, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.null_tuple_keeping_texts(&format!("{t}.f{j}"), e, ind);
                }
            }
            other => {
                let init = self.cx.initial(other);
                self.line(ind, &format!("{t} = ({}) {init};", super::held(other)));
            }
        }
    }

    /// Every text a tuple of struct `id` at C place `at` holds, nested tuples included.
    pub(super) fn tuple_text_leaves(&self, at: &str, id: u32, out: &mut Vec<String>) {
        for (j, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
            match e {
                CType::Text => out.push(format!("{at}.f{j}")),
                CType::Tuple(inner) => self.tuple_text_leaves(&format!("{at}.f{j}"), inner, out),
                _ => {}
            }
        }
    }

    /// `var = x` on a store link: a re-point takes the new place, anything else is written
    /// through the place (`link_set_repoints` tells them apart, as for the bytecode).
    fn store_link_set(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        if crate::scopes::link_set_repoints(self.cx.data, self.vars, var, x) {
            self.bound[var as usize] = true;
            let name = var_name(self.vars.name(var), var);
            let e = self.expr(x, ind)?;
            self.line(ind, &format!("{name} = {};", as_ref(e)?));
            self.flush(ind);
            return Ok(());
        }
        let write = self.link_access(var, Some(x))?;
        self.stmt(&write, ind)
    }

    /// The read (`value` none) or the write of store link `v`, as the operator the bytecode
    /// uses through the link's `DbRef`: the place's own get or set, a narrow integer's with its
    /// `min`, on `OpVarRef(v)` — the place itself.
    fn link_access(&self, v: u16, value: Option<&Value>) -> Result<Value, String> {
        use crate::data::Type;
        let Type::RefVar(tp) = self.vars.tp(v).base() else {
            return Err("a store link that is not a link".into());
        };
        let narrow = crate::data::NarrowSlot::of_type(tp);
        let (get, set) = match tp.base() {
            Type::Integer(_) if narrow.is_some() => {
                let n = narrow.expect("checked");
                (n.get_op(), n.set_op())
            }
            Type::Integer(_) => ("OpGetInt", "OpSetInt"),
            Type::Character => ("OpGetCharacter", "OpSetCharacter"),
            Type::Single => ("OpGetSingle", "OpSetSingle"),
            Type::Float => ("OpGetFloat", "OpSetFloat"),
            Type::Boolean => ("OpGetBoolean", "OpSetBoolean"),
            Type::Enum(_, false, _) => ("OpGetEnum", "OpSetEnum"),
            Type::Text(_) => ("OpGetText", "OpSetText"),
            _ => return Err("a store link of this kind".into()),
        };
        let data = self.cx.data;
        let op = data.def_nr(if value.is_some() { set } else { get });
        let place = Value::Call(data.def_nr("OpVarRef"), vec![Value::Var(v)]);
        let mut args = vec![place, Value::Int(0)];
        // A narrow kind's ops take the `min` after the field (`OpGetByte(r, fld, min)`).
        if data.def(op).attributes().len() == if value.is_some() { 4 } else { 3 } {
            args.push(Value::Int(narrow.map_or(0, |n| n.min)));
        }
        if let Some(x) = value {
            args.push(x.clone());
        }
        Ok(Value::Call(op, args))
    }

    /// The typed null of an operand of type `tp`, as an operator call, or `None` for a type
    /// whose null the bytecode does not push as one value.
    fn typed_null(&self, tp: &crate::data::Type) -> Option<Value> {
        use crate::data::Type;
        let op = match tp.base() {
            Type::Integer(..) => "OpConvIntFromNull",
            Type::Float => "OpConvFloatFromNull",
            Type::Single => "OpConvSingleFromNull",
            Type::Text(_) => "OpConvTextFromNull",
            Type::Boolean => "OpConvBoolFromNull",
            Type::Character => "OpConvCharacterFromNull",
            Type::Reference(..)
            | Type::Vector(..)
            | Type::Sorted(..)
            | Type::Hash(..)
            | Type::Index(..)
            | Type::Radix(..)
            | Type::Trie(..) => "OpNullRefSentinel",
            _ => return None,
        };
        let nr = self.cx.data.def_nr(op);
        (nr < self.cx.data.definitions()).then(|| Value::Call(nr, Vec::new()))
    }

    /// The C shape a call through function value `v` answers.
    fn call_ref_shape(&self, v: u16) -> Result<CType, String> {
        match self.vars.tp(v).base() {
            crate::data::Type::Function(_, ret, ..) => self
                .cx
                .ctype(ret)
                .ok_or_else(|| format!("a function value returning {}", type_word(ret))),
            _ => Err("a call through a value that is not a function".into()),
        }
    }

    /// The C shape `v` evaluates to, when it is a compound the caller must hold.
    fn shape_of(&self, v: &Value) -> Result<CType, String> {
        let cx = self.cx;
        match v.unspan() {
            Value::Block(b) => cx
                .ctype(&b.result)
                .ok_or_else(|| format!("a block of type {}", type_word(&b.result))),
            Value::Insert(ops) => ops.last().map_or(Ok(CType::Void), |l| self.shape_of(l)),
            Value::If(_, t, e) => match self.shape_of(t) {
                Ok(CType::Void) | Err(_) => self.shape_of(e),
                ok => ok,
            },
            Value::Int(_) | Value::Long(_) => Ok(CType::Int),
            Value::Enum(..) => Ok(CType::Enum),
            Value::Float(_) => Ok(CType::Float),
            Value::Single(_) => Ok(CType::Single),
            Value::Boolean(_) => Ok(CType::Bool),
            Value::Text(_) => Ok(CType::Text),
            Value::Var(v) => Ok(self.shapes[*v as usize]),
            Value::FnRef(..) => Ok(CType::Fn),
            Value::FnRefDnr(_) => Ok(CType::Int),
            Value::CallRef(v, _) => self.call_ref_shape(*v),
            Value::TupleGet(v, i) => match self.shapes[*v as usize] {
                CType::Tuple(id) => cx
                    .tuple_shapes(id)
                    .get(*i as usize)
                    .copied()
                    .ok_or_else(|| "a tuple index out of range".into()),
                _ => Err("a tuple read of a non-tuple".into()),
            },
            Value::Tuple(vals) => {
                let shapes = vals
                    .iter()
                    .map(|x| self.shape_of(x))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CType::Tuple(cx.tuple_id(shapes)))
            }
            Value::Call(d, _) if !is_operator(cx.data, *d) => {
                let tp = cx.data.def(*d).returned();
                cx.ctype(tp)
                    .ok_or_else(|| format!("a call returning {}", type_word(tp)))
            }
            Value::Call(d, _) => {
                let name = cx.data.def(*d).name();
                op_shape(name).ok_or_else(|| format!("operator {name}"))
            }
            other => Err(unsupported(other)),
        }
    }

    /// A C expression for `v`'s value; the statements it needs are emitted first.
    fn expr(&mut self, v: &Value, ind: usize) -> Result<E, String> {
        if self.invariants.iter().any(|f| !f.is_empty())
            && let Some(memo) = self.invariant_of(v)
        {
            return self.invariant_use(&memo, ind);
        }
        match v.unspan() {
            Value::RawExpr(code) if self.raw.contains_key(code) => {
                Ok(E::new(code.clone(), self.raw[code]))
            }
            Value::Int(i) => Ok(E::new(format!("INT64_C({i})"), K::Int)),
            Value::Long(l) if *l == i64::MIN => Ok(E::new("LOFT_NULL", K::Int)),
            Value::Long(l) => Ok(E::new(c_long(*l), K::Int)),
            Value::Boolean(b) => Ok(E::new(if *b { "1" } else { "0" }, K::Cond)),
            Value::Enum(v, _) => Ok(E::new(format!("(uint8_t) {v}"), K::Enum)),
            Value::Float(f) => Ok(E::new(
                format!("loft_f64(UINT64_C({:#x}))", f.to_bits()),
                K::Float,
            )),
            Value::Single(f) => Ok(E::new(
                format!("loft_f32(UINT32_C({:#x}))", f.to_bits()),
                K::Single,
            )),
            Value::Text(s) => Ok(E::new(
                format!("loft_lit({}, {})", c_string(s), s.len()),
                K::View,
            )),
            // `@FR-R-Apart` instance 4: an apart value is its buffers — what a twin call takes, and
            // a C type error anywhere a record was meant.
            Value::Var(var) if let Some(buf) = self.apr_buf(*var) => Ok(E::new(buf, K::Ref)),
            Value::Var(var) if self.apv_vec(*var) => Ok(E::new(format!("loft_apv{var}"), K::Ref)),
            Value::Var(var) if let Some(val) = self.av_val(*var) => Ok(E::new(val, K::Ref)),
            // A link to a field or element reads that place at each use (`@FR-B-Ref-Read`).
            Value::Var(var) if self.store_links[*var as usize] => {
                let read = self.link_access(*var, None)?;
                self.expr(&read, ind)
            }
            Value::Var(var) => {
                let name = self.var(*var);
                Ok(match self.shapes[*var as usize] {
                    CType::Text if self.text_held(*var) => {
                        E::new(format!("loft_view(&{name})"), K::View)
                    }
                    CType::Void => return Err("a void variable".into()),
                    c => E::new(name, kind_of(c)),
                })
            }
            Value::TupleGet(var, i) => {
                let c = self.shape_of(&Value::TupleGet(*var, *i))?;
                let place = format!("{}.f{i}", self.var(*var));
                if c == CType::Text {
                    return Ok(E::new(format!("loft_view(&{place})"), K::View));
                }
                Ok(E::new(place, kind_of(c)))
            }
            Value::Call(d, args) => {
                // `@FR-R-Apart` instance 3: an apart vector's element, push, length or clear.
                if let Some(e) = self.apv_operator(self.cx.data.def(*d).name(), v, args, ind)? {
                    return Ok(e);
                }
                // Instance 2: an element of a record held as a value, its length, its free.
                if let Some(e) = self.av_operator(v, ind)? {
                    return Ok(e);
                }
                self.call(*d, args, ind)
            }
            Value::CallRef(v, args) => self.call_ref(*v, args, ind),
            // The definition number a function value names, as an integer.
            Value::FnRefDnr(v) => Ok(E::new(format!("{}.d", self.var(*v)), K::Int)),
            Value::FnRef(d, clos, _) => {
                let f = self.cx.fn_value(d.unsigned_abs());
                if *clos == u16::MAX {
                    Ok(E::new(f, K::Fn))
                } else {
                    Ok(E::new(
                        format!("loft_fn_make(INT64_C({d}), {})", self.var(*clos)),
                        K::Fn,
                    ))
                }
            }
            compound @ (Value::Block(_) | Value::Insert(_) | Value::If(..) | Value::Tuple(_)) => {
                if let Some(e) = self.view_choice(compound, ind)? {
                    return Ok(e);
                }
                let c = self.shape_of(compound)?;
                if c == CType::Text
                    && let Some(e) = self.nested(|w| w.text_tail_view(compound, ind))?
                {
                    return Ok(e);
                }
                self.nested(|w| {
                    if c == CType::Void {
                        w.stmt(compound, ind)?;
                        Ok(E::new("(void) 0", K::Void))
                    } else {
                        let t = w.temp(c);
                        w.assign(compound, &t, c, ind)?;
                        Ok(w.held(t, c))
                    }
                })
            }
            Value::Null => Err("a null without a type".into()),
            other => Err(unsupported(other)),
        }
    }

    /// `@FR-R-TextBorrow`'s discharge clause, its value: a text choice between views —
    /// `if c { a } else { b }`, as a `?` discharge writes it, or as a text value chooses between
    /// pieces (`out += if last { "=" } else { chars[k..k + 1] }`) — where each arm is a literal,
    /// a borrowed text variable, or a slice of one of those or of a plain text parameter: the
    /// view it picks, the bytes the arm's own, valid as long as that text is, so nothing is
    /// copied.  Each arm is written in its own branch, so the arm not taken computes nothing.
    /// `None` for any other compound.  `LOFT_NO_VIEW_CHOICE_SLICE=1` keeps a slice arm's copy.
    fn view_choice(&mut self, compound: &Value, ind: usize) -> Result<Option<E>, String> {
        let (pre, last): (&[Value], &Value) = match compound.unspan() {
            Value::Block(block) => match block.operators.split_last() {
                Some((value, stmts)) => (stmts, value),
                None => return Ok(None),
            },
            other => (&[], other),
        };
        let Value::If(cond, then, other) = last.unspan() else {
            return Ok(None);
        };
        let slices =
            !crate::env_once!(std::env::var("LOFT_NO_VIEW_CHOICE_SLICE").is_ok_and(|v| v != "0"));
        let (then, other) = (Self::arm_value(then), Self::arm_value(other));
        if !self.viewable_arm(then, slices) || !self.viewable_arm(other, slices) {
            return Ok(None);
        }
        // The block's own statements first — the discharge's bind of the variable it tests.
        for op in pre {
            self.stmt(op, ind)?;
        }
        let test = self.expr(cond, ind)?;
        let test = as_cond(test)?;
        let view = format!("tv{}", self.decls.len() + 1);
        self.decls.push(format!("loft_str {view} = {{ 0, 0 }};"));
        self.line(ind, &format!("if ({test}) {{"));
        let then = self.expr(then, ind + 1)?;
        self.line(ind + 1, &format!("{view} = {};", as_view(then)?));
        self.line(ind, "} else {");
        let other = self.expr(other, ind + 1)?;
        self.line(ind + 1, &format!("{view} = {};", as_view(other)?));
        self.line(ind, "}");
        Ok(Some(E::new(view, K::View)))
    }

    /// An arm's value: the arm itself, or the one value a block holds beside its line marks.
    fn arm_value(arm: &Value) -> &Value {
        if let Value::Block(b) = arm.unspan() {
            let mut ops = b
                .operators
                .iter()
                .filter(|o| !matches!(o.unspan(), Value::Line(_)));
            if let (Some(only), None) = (ops.next(), ops.next()) {
                return only;
            }
        }
        arm
    }

    /// An arm `view_choice` reads as a view: a literal, a borrowed text variable, or — with
    /// `slices` — a slice of a literal, a borrowed text variable or a plain text parameter,
    /// whose bounds are scalars.
    fn viewable_arm(&self, arm: &Value, slices: bool) -> bool {
        let source = |v: &Value| match v.unspan() {
            Value::Text(_) => true,
            Value::Var(var) => {
                let i = *var as usize;
                self.shapes[i] == CType::Text
                    && (self.borrowed[i]
                        || (self.vars.is_argument(*var)
                            && !matches!(self.vars.tp(*var).base(), crate::data::Type::RefVar(_))))
            }
            _ => false,
        };
        match arm.unspan() {
            Value::Text(_) => true,
            Value::Var(var) => {
                self.shapes[*var as usize] == CType::Text && self.borrowed[*var as usize]
            }
            Value::Call(d, args)
                if slices && self.cx.data.def(*d).name() == "OpGetTextSub" && args.len() == 3 =>
            {
                source(&args[0])
            }
            _ => false,
        }
    }

    /// Statements written inside an expression: they free only the temporaries they make, as
    /// the enclosing expression's are still to be read once they are done.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let outer = std::mem::take(&mut self.pending);
        let r = f(self);
        let inner = std::mem::replace(&mut self.pending, outer);
        self.pending.extend(inner);
        r
    }

    /// A text value block whose value is a text local the block leaves alone after it — the
    /// work text a call or a format fills, `{ w = f(…); w }` — is that local's view, as a plain
    /// read of it is, instead of a copy into a temporary: the block's statements run, then its
    /// frees, none of which names the local.  `arg_with` guards the view as it guards a read of
    /// the local.  `None` for every other block, and under `LOFT_NO_C_TAIL_VIEW=1`.
    fn text_tail_view(&mut self, v: &Value, ind: usize) -> Result<Option<E>, String> {
        let Value::Block(b) = v.unspan() else {
            return Ok(None);
        };
        if std::env::var("LOFT_NO_C_TAIL_VIEW").is_ok_and(|x| x != "0") {
            return Ok(None);
        }
        let ops = &b.operators;
        if self.tail_capture(ops).is_some() {
            return Ok(None);
        }
        let Some(at) = ops.iter().rposition(|o| {
            !matches!(o.unspan(), Value::Line(_) | Value::Set(..)) && !self.is_free(o)
        }) else {
            return Ok(None);
        };
        let Value::Var(w) = ops[at].unspan() else {
            return Ok(None);
        };
        let w = *w;
        if !self.text_held(w)
            || ops[at + 1..]
                .iter()
                .any(|o| o.any_node(&mut |n| matches!(n, Value::Var(x) if *x == w)))
        {
            return Ok(None);
        }
        let frames = self.headers.len();
        let r = (|| -> Result<E, String> {
            for i in 0..at {
                self.stmt(&ops[i], ind)?;
                self.bind_view_header(ops, i, ind);
            }
            let e = self.expr(&ops[at], ind)?;
            self.flush(ind);
            ops[at + 1..].iter().try_for_each(|o| self.stmt(o, ind))?;
            Ok(e)
        })();
        self.headers.truncate(frames.min(self.headers.len()));
        let e = r?;
        if e.kind != K::View {
            return Err("a text block's tail read as no view".into());
        }
        Ok(Some(e))
    }

    /// The statement temporary an expression `code` reads, when it is exactly a view of an owned
    /// text temporary this statement made ([`Self::held`]) — a value nothing else names, which a
    /// text target may take by a move.  `None` under `LOFT_NO_C_TEXT_MOVE=1`.
    fn pending_text_temp(&self, code: &str) -> Option<String> {
        let t = code.strip_prefix("loft_view(&")?.strip_suffix(')')?;
        if !self.pending.iter().any(|p| p == t)
            || !self.temps.iter().any(|(n, c)| n == t && *c == CType::Text)
            || std::env::var("LOFT_NO_C_TEXT_MOVE").is_ok_and(|v| v != "0")
        {
            return None;
        }
        Some(t.to_string())
    }

    /// The expression reading temporary `t` of shape `c`.
    fn held(&mut self, t: String, c: CType) -> E {
        match c {
            CType::Text => {
                self.pending.push(t.clone());
                E::new(format!("loft_view(&{t})"), K::View)
            }
            CType::Void => E::new("(void) 0", K::Void),
            c => E::new(t, kind_of(c)),
        }
    }

    /// The name of the text local `v` an operator writes into.
    fn text_dest(&self, v: &Value) -> Result<String, String> {
        match v.unspan() {
            Value::Var(var) if self.work_alias.as_ref().is_some_and(|(w, _)| w == var) => Ok(self
                .work_alias
                .as_ref()
                .map(|(_, d)| d.clone())
                .unwrap_or_default()),
            Value::Var(var) if self.text_held(*var) => Ok(self.var(*var)),
            Value::TupleGet(var, i)
                if matches!(self.shapes[*var as usize], CType::Tuple(id)
                    if self.cx.tuple_shapes(id).get(*i as usize) == Some(&CType::Text)) =>
            {
                Ok(format!("{}.f{i}", self.var(*var)))
            }
            Value::Var(var) => Err(format!(
                "a text operator writing into a {:?} variable",
                self.shapes[*var as usize]
            )),
            other => Err(format!(
                "a text operator writing into {}",
                unsupported(other)
            )),
        }
    }

    /// Evaluate `args` in order: anything but a variable or a literal goes into a temporary
    /// first, because C leaves the order of a call's arguments open and loft does not.
    ///
    /// A variable is read in place unless a later operand may write it (@FR-E-Left: an
    /// operand is read before a later operand writes) — then it is read into a temporary too.
    fn args<V: Borrow<Value>>(&mut self, args: &[V], ind: usize) -> Result<Vec<E>, String> {
        (0..args.len()).map(|i| self.arg(args, i, ind)).collect()
    }

    /// The operands of a text VALUE op (`@FR-R-TextBorrow`: one that reads its operands at once
    /// and writes no store), as `args` evaluates them — except that a text field read no later
    /// operand can disturb (`may_write`) is the field's bytes where the store holds them, as
    /// native's `&str` operand is, not a copy: nothing between the read and the op can grow,
    /// free or rewrite that store.
    /// `LOFT_NO_FIELD_VIEW=1` copies every such field again.
    fn text_reads<V: Borrow<Value>>(&mut self, args: &[V], ind: usize) -> Result<Vec<E>, String> {
        let off = no_field_view();
        (0..args.len())
            .map(|i| {
                let later_writes = args[i + 1..].iter().any(|b| self.may_write(b.borrow()));
                self.text_read(args[i].borrow(), later_writes, off, ind)
            })
            .collect()
    }

    /// One operand of `text_reads`: `later_writes` says whether an operand after it may write.
    fn text_read(
        &mut self,
        a: &Value,
        later_writes: bool,
        off: bool,
        ind: usize,
    ) -> Result<E, String> {
        let data = self.cx.data;
        let field = !off
            && matches!(a.unspan(), Value::Call(d, xs)
            if xs.len() == 2 && data.def(*d).name() == "OpGetText");
        if !field || later_writes {
            return self.arg_with(a, later_writes, ind);
        }
        self.field_view = true;
        let e = self.expr(a, ind);
        self.field_view = false;
        e
    }

    /// Operand `i` of `args`, evaluated in order: a value a later operand may change is held
    /// in a temporary.
    fn arg<V: Borrow<Value>>(&mut self, args: &[V], i: usize, ind: usize) -> Result<E, String> {
        let later_writes = args[i + 1..]
            .iter()
            .any(|later| self.may_write(later.borrow()));
        self.arg_with(args[i].borrow(), later_writes, ind)
    }

    /// Operand `operand`, evaluated as `arg` does; `later_writes` says whether an operand after
    /// it may write.  The operand is the IR node itself, never a copy: a rewrite keyed on a
    /// node's address (`@FR-R-GuardedChain`'s admitted operators) must meet the node it admitted.
    fn arg_with(&mut self, operand: &Value, later_writes: bool, ind: usize) -> Result<E, String> {
        let value = self.expr(operand, ind)?;
        // A text block's tail view (`text_tail_view`) reads a local, as a variable operand does.
        let reads_local = match operand.unspan() {
            Value::Var(_) => true,
            Value::Block(_) => value.kind == K::View && self.pending_text_temp(&value.code).is_none(),
            _ => false,
        };
        if later_writes && reads_local && value.kind == K::View {
            let temp = self.temp(CType::Text);
            self.line(ind, &format!("loft_text_set(&{temp}, {});", value.code));
            return Ok(self.held(temp, CType::Text));
        }
        let simple = match operand.unspan() {
            Value::Var(_) => !later_writes,
            Value::Int(_) | Value::Long(_) | Value::Boolean(_) | Value::Text(_) => true,
            _ => false,
        };
        // A temporary is already stable — and a second one would share the texts a tuple
        // temporary owns, which the epilogue frees through each.
        if simple
            || matches!(value.kind, K::View | K::Void)
            || self.temps.iter().any(|(t, _)| *t == value.code)
        {
            return Ok(value);
        }
        let (shape, code) = match value.kind {
            K::Int => (CType::Int, value.code),
            K::Int32 => (CType::Int32, value.code),
            K::Ref => (CType::Ref, value.code),
            K::Tup(id) => (CType::Tuple(id), value.code),
            K::Enum => (CType::Enum, value.code),
            K::Float => (CType::Float, value.code),
            K::Single => (CType::Single, value.code),
            K::Char => (CType::Char, value.code),
            K::Fn => (CType::Fn, value.code),
            _ => (CType::Bool, as_bool(value)?),
        };
        let temp = self.temp(shape);
        self.line(ind, &format!("{temp} = {code};"));
        Ok(E::new(temp, kind_of(shape)))
    }

    /// May evaluating `v` write a variable?  A statement, a block or a call that is not a
    /// pure operator may; a conservative answer only costs a temporary.
    fn may_write(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Var(_)
            | Value::Int(_)
            | Value::Long(_)
            | Value::Boolean(_)
            | Value::Text(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Enum(..)
            | Value::Null
            | Value::TupleGet(..)
            | Value::FnRef(..) => false,
            Value::Call(d, xs) if is_operator(self.cx.data, *d) => {
                xs.iter().any(|x| self.may_write(x))
            }
            _ => true,
        }
    }

    /// `v.len()` of a vector this body holds — the stdlib wrapper answered as its op
    /// (`@FR-R-Wrapper`) — a split table's count, a held header's length or a push header's.
    /// `None` for any other vector.
    fn held_len(&mut self, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let [v] = args else { return Ok(None) };
        if let Some(t) = self.split_tab_of(v) {
            return Ok(Some(E::new(format!("((int64_t) st{t}.n)"), K::Int)));
        }
        if let Some(h) = self.held_vector(v) {
            return Ok(Some(E::new(format!("{h}.len"), K::Int)));
        }
        self.pushed_mint("OpLengthVector", args, ind)
    }

    /// An operator call: a bare `null` argument is the null of the operand's declared type, as
    /// the bytecode pushes it (`OpSetDbRef(g, 0, null)`, `OpPushInt(v, null)`).
    fn call_operator(&mut self, d: u32, args: &[Value], ind: usize) -> Result<E, String> {
        let cx = self.cx;
        if args.iter().any(|a| matches!(a.unspan(), Value::Null)) {
            let attrs = cx.data.def(d).attributes();
            let typed: Vec<Value> = args
                .iter()
                .enumerate()
                .map(|(i, a)| match a.unspan() {
                    Value::Null => attrs
                        .get(i)
                        .and_then(|at| self.typed_null(&at.typedef))
                        .unwrap_or(Value::Null),
                    _ => a.clone(),
                })
                .collect();
            return self.operator(cx.data.def(d).name(), &typed, ind);
        }
        self.operator(cx.data.def(d).name(), args, ind)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "past the bar where the 32-bit arms met the value-record arms on a join; the split is release work (M-file-split)"
    )]
    fn call(&mut self, d: u32, args: &[Value], ind: usize) -> Result<E, String> {
        let cx = self.cx;
        // Every route below is keyed by the callee's name, and only a standard-library definition
        // is answered by one: a library's function under a stdlib name keeps its own body.
        let route = super::runtime_name(cx.data, d);
        if let Some(e) = self.apr_length(route, args) {
            return Ok(e);
        }
        if is_operator(cx.data, d) {
            return self.call_operator(d, args, ind);
        }
        let def = cx.data.def(d);
        if let Some(e) = self.keyed_native(route, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.par_native(route, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.unicode_native(route, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.struct_json_native(route, args, ind)? {
            return Ok(e);
        }
        if route == "n_env_variables" {
            // The element is the stdlib's `EnvVariable`, sized from this program's table.
            let size = self.cx.stores.size(self.cx.stores.name("EnvVariable"));
            return Ok(E::new(format!("loft_env_variables({size})"), K::Ref));
        }
        let callee = def.original_name();
        // `@FR-R-ValueRecord` — an admitted callee answers its struct and takes no buffer; a
        // tuple parameter takes a struct.
        let (vr_key, dropped, tuple_params) = self.vr_callee(d);
        let ret = match vr_key {
            Some(key) => cx
                .vr_ctype(key)
                .ok_or("a value record without its layout")?,
            None => cx.ctype(def.returned()).ok_or_else(|| {
                format!("a call to {callee} returning {}", type_word(def.returned()))
            })?,
        };
        let shapes: Vec<CType> = def
            .attributes()
            .iter()
            .map(|a| {
                let tp = a.typedef.peel_link();
                cx.ctype(tp).ok_or_else(|| {
                    format!(
                        "a call to {callee} with a {} parameter",
                        type_word(&a.typedef)
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        if shapes.len() != args.len() {
            return Err("a call whose arguments do not match its parameters".into());
        }
        if route == "t_6vector_len"
            && let Some(e) = self.held_len(args, ind)?
        {
            return Ok(e);
        }
        // A native runtime function is written in `int64_t`: a 32-bit integer crosses it widened.
        let native = super::native_c(route).is_some();
        let (ret, shapes) = if native {
            (
                super::native_shape(ret),
                shapes.into_iter().map(super::native_shape).collect(),
            )
        } else {
            (ret, shapes)
        };
        let name = super::native_c(route).map_or_else(|| c_name(cx.data, d), str::to_string);
        // A link argument is the address of the caller's variable; the others are values,
        // evaluated in order.
        let mut xs = vec![String::new(); args.len()];
        let (values, places) = self.link_args(d, args, &shapes, &mut xs, ind)?;
        let ptr = cx.vr_ptr_params(d);
        let tuple_params: Vec<(usize, u16, bool)> = tuple_params
            .into_iter()
            .map(|(i, key)| (i, key, ptr.contains(&i)))
            .collect();
        let values = self.vr_args(args, values, dropped, &tuple_params, &mut xs, ind)?;
        // A callee that runs no store write reads a text argument as a text VALUE op
        // does (`@FR-F-ParamText`) — unless a store place goes to a `&` parameter, which its twin writes.
        let reads = places == 0 && cx.call_keeps_stores(d);
        self.value_args(args, &shapes, &values, &mut xs, reads, ind)?;
        if let Some(i) = dropped {
            xs.remove(i);
        }
        // `@FR-R-Header`'s kept-twin clause: a callee handed a vector this body holds a GROWN
        // header for takes that header by pointer and keeps it current; no refresh follows.
        let mut mask = places;
        if places == 0
            && super::native_c(route).is_none()
            && args.len() < 64
            && let Some(h) = self.kept_header_arg(d, args)
        {
            xs.push(format!("&{h}"));
            self.kh_passed.insert(h);
            mask |= super::KH_BIT;
            crate::rewrite_census::fired("R-KeptTwin", 1);
        }
        // `@FR-R-Inputs`: a callee whose every header input this body holds takes them.
        if mask & super::KH_BIT == 0
            && super::native_c(route).is_none()
            && args.len() < 64
            && !self.apr_twin_call(d, args)
            && let Some(inputs) = self.twin_inputs(d, args, ind)?
        {
            xs.extend(inputs);
            mask |= super::INV_BIT;
        }
        crate::rewrite_census::fired("R-Inputs", usize::from(mask & super::INV_BIT != 0));
        if self.nk_call == Some(d) {
            self.nk_call = None;
            mask |= super::NK_BIT;
        }
        if self.append_call(d, args) {
            mask |= super::AP_BIT;
        }
        if self
            .ds_site
            .is_some_and(|(c, at)| c == d && at == args.as_ptr() as usize)
        {
            self.ds_site = None;
            mask |= super::DS_BIT;
            crate::rewrite_census::fired("R-Destination", 1);
        }
        // `@FR-R-NonSentinelParam`'s twin clause: the parameters this call's arguments prove
        // beyond the program-wide fact go to a `__nn` twin that seeds them.
        if super::native_c(route).is_none() && args.len() <= 26 && !self.apr_twin_call(d, args) {
            let bits = self.nn_twin_bits(d, args);
            if bits != 0 {
                mask |= bits << super::NN_SHIFT;
            }
        }
        // `@FR-R-Apart` instance 4: an apart value handed over goes to the callee's `__apr` twin,
        // which takes it as its buffers (`expr` answered them at that argument).
        if self.apr_twin_call(d, args) {
            mask |= super::APR_BIT;
        }
        let name = if mask == 0 {
            name
        } else if super::native_c(route).is_some() || args.len() > 63 {
            return Err(format!("a store place handed to a & parameter of {callee}"));
        } else {
            cx.request_twin(d, mask);
            super::twin_name(cx.data, d, mask)
        };
        let call = format!("{name}({})", xs.join(", "));
        if let Some(key) = vr_key
            && let Some(buf) = self.vr_forward_target(args, dropped)
        {
            return self.vr_forward(&call, key, buf, ind);
        }
        Ok(match ret {
            CType::Text => {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(ind, &format!("{t} = {call};"));
                self.held(t, CType::Text)
            }
            c => E::new(call, kind_of(c)),
        })
    }

    /// `@FR-R-NonSentinelParam`'s twin clause: the bit set of `d`'s parameters this call's
    /// arguments prove non-sentinel — under this function's own facts, a `__nn` twin's seed
    /// included — that the program-wide fact does not.  Only for a callee where the seed can
    /// buy a plain operator: one that divides or takes a remainder by a literal, itself or in a
    /// function it calls directly.  0 under `LOFT_NO_NN_TWINS=1`.
    fn nn_twin_bits(&mut self, d: u32, args: &[Value]) -> u64 {
        if crate::env_once!(std::env::var("LOFT_NO_NN_TWINS").is_ok_and(|v| v != "0")) {
            return 0;
        }
        let data = self.cx.data;
        let Some(cand) = crate::opt::non_sentinel::candidate_params(data, d) else {
            return 0;
        };
        let divides = |f: u32| {
            let mut hit = false;
            data.def(f).code().walk(&mut |v| {
                if let Value::Call(op, a) = v
                    && (*op as usize) < data.definitions.len()
                    && matches!(data.def(*op).name(), "OpRemInt" | "OpDivInt")
                    && matches!(a.get(1).map(Value::unspan), Some(Value::Int(_)))
                {
                    hit = true;
                }
            });
            hit
        };
        let worth = divides(d) || {
            let mut hit = false;
            data.def(d).code().walk(&mut |v| {
                if !hit
                    && let Value::Call(f, _) = v
                    && (*f as usize) < data.definitions.len()
                    && matches!(data.def(*f).code(), Value::Block(_))
                    && divides(*f)
                {
                    hit = true;
                }
            });
            hit
        };
        if !worth {
            return 0;
        }
        let global = crate::opt::non_sentinel::proven_param_flags(data, d).unwrap_or_default();
        let facts = self.nn_facts();
        let mut bits = 0u64;
        for (i, a) in args.iter().enumerate().take(26) {
            if cand.get(i).copied().unwrap_or(false)
                && !global.get(i).copied().unwrap_or(false)
                && crate::opt::non_sentinel::non_sentinel(data, &facts.0, a)
            {
                bits |= 1 << i;
            }
        }
        if bits != 0 {
            crate::rewrite_census::fired("R-NonSentinelParam/twin", 1);
        }
        bits
    }

    /// The grown header (with its base) an enclosing frame holds for the vector argument of
    /// `d`'s kept-twin position (`opt::apart::kept_growth_param`), when there is one.
    fn kept_header_arg(&self, d: u32, args: &[Value]) -> Option<String> {
        if self.grown_headers.is_empty() || self.gen_elem.is_some() {
            return None;
        }
        let k = crate::opt::apart::kept_growth_param(self.cx.data, d)?;
        let Value::Var(x) = args.get(k)?.unspan() else {
            return None;
        };
        self.headers
            .iter()
            .rev()
            .flat_map(|f| f.iter())
            .find(|(p, h, base)| {
                p.0 == *x && p.1.is_empty() && *base && self.grown_headers.contains(h)
            })
            .map(|(_, h, _)| h.clone())
    }

    /// The headers `d`'s `__inv` twin takes for a call with `args`, as native's
    /// `twin_call_inputs` keys them: each header input's path re-spelled over the argument —
    /// every one held by an enclosing frame, or `None`.  A header passed under growth carries
    /// a base derived at the call, valid while the callee runs, which grows no store.
    /// `@FR-R-Inputs` at a call: the arguments of `d`'s `__inv` twin — each record scalar it
    /// reads, then each header — or `None` where the plain call stays.  Every header must be one
    /// this body holds (the twin reads its vector through it); a scalar is the local this body
    /// holds it in, else its read spelled over the argument.  The twin is taken when at least
    /// one input is held, as rustc takes it (`any_held`): a twin fed only fresh reads would only
    /// move each read to the call.
    fn twin_inputs(
        &mut self,
        d: u32,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<Vec<String>>, String> {
        if self.gen_elem.is_some()
            || (self.headers.iter().all(Vec::is_empty) && self.scalars.iter().all(Vec::is_empty))
        {
            return Ok(None);
        }
        let Some(fi) = self.cx.callee_inputs(d) else {
            return Ok(None);
        };
        let mut held_any = false;
        let mut scalar_args = Vec::new();
        for (p, fld, getter) in &fi.scalars {
            if super::twin_scalar_shape(self.cx, getter).is_none() {
                continue;
            }
            let Some(arg) = args.get(*p as usize) else {
                return Ok(None);
            };
            // A plain variable, or a sub-record path over one (`atlas.a_cv`), keyed on the root
            // and the field's summed offset — the key a loop holds it under, as rustc keys it
            // (`hoist::path_scalar`).  Any other argument is not a pure path: reading the
            // field over it would evaluate it again, so the plain call stays.
            let key = if let Value::Var(av) = arg.unspan() {
                (*av, *fld)
            } else if let Some((k, _)) =
                crate::opt::hoist::path_scalar(self.cx.data, self.vars, arg, *fld)
            {
                k
            } else {
                return Ok(None);
            };
            if let Some((local, _)) = self.held_scalar(&key) {
                held_any = true;
                scalar_args.push(local);
                continue;
            }
            // Not held as a value: read through the record's ADDRESS where this body holds it
            // (`@FR-R-RecPtr`, one load — the read below takes that path), else the plain call
            // stays, as rustc keeps it: a twin fed reads through the store would only move
            // each read to the call.
            if !self.rec_ptrs.iter().any(|(v, _, _)| *v == key.0) {
                return Ok(None);
            }
            held_any = true;
            // The read as the callee spells it, over the caller's pure argument path.
            let Value::Call(g, gargs) = getter.unspan() else {
                return Ok(None);
            };
            let mut over = gargs.clone();
            over[0] = arg.clone();
            let e = self.nested(|w| w.expr(&Value::Call(*g, over), ind))?;
            scalar_args.push(e.code);
        }
        let mut header_args = Vec::with_capacity(fi.headers.len());
        for (p, offs, _) in &fi.headers {
            let Some(arg) = args.get(*p as usize) else {
                return Ok(None);
            };
            let Some((c, mut key)) = crate::opt::writes::vector_path(self.cx.data, arg) else {
                return Ok(None);
            };
            key.extend_from_slice(offs);
            let Some(h) = self.held_path(&(c, key)) else {
                return Ok(None);
            };
            held_any = true;
            header_args.push(h);
        }
        if !held_any || (scalar_args.is_empty() && header_args.is_empty()) {
            return Ok(None);
        }
        scalar_args.extend(header_args);
        Ok(Some(scalar_args))
    }

    /// Evaluate the arguments at `values` into `xs`, in order; a function named where a
    /// function value is passed becomes that value.
    fn value_args(
        &mut self,
        args: &[Value],
        shapes: &[CType],
        values: &[usize],
        xs: &mut [String],
        reads: bool,
        ind: usize,
    ) -> Result<(), String> {
        let mut rest = Vec::new();
        for &i in values {
            match args[i].unspan() {
                Value::Int(d) if shapes[i] == CType::Fn && self.cx.is_function(*d) => {
                    xs[i] = self.cx.fn_value(d.unsigned_abs());
                }
                // A bare null takes the parameter's shape.
                Value::Null if shapes[i] != CType::Void => {
                    xs[i] = match shapes[i] {
                        CType::Text => "loft_null_text()".into(),
                        c => format!("(({}) {})", held(c), self.cx.initial(c)),
                    };
                }
                _ => rest.push(i),
            }
        }
        let off = no_field_view();
        for (k, &i) in rest.iter().enumerate() {
            let later_writes = rest[k + 1..].iter().any(|&j| self.may_write(&args[j]));
            let e = if reads {
                self.text_read(&args[i], later_writes, off, ind)?
            } else {
                self.arg_with(&args[i], later_writes, ind)?
            };
            xs[i] = as_shape(e, shapes[i])?;
        }
        Ok(())
    }

    /// A call through function value `v`: its signature's dispatcher.  A text result's work
    /// buffers the IR appends after the visible arguments are the dispatcher's to make.
    fn call_ref(&mut self, v: u16, args: &[Value], ind: usize) -> Result<E, String> {
        let crate::data::Type::Function(params, ret, ..) = self.vars.tp(v).base().clone() else {
            return Err("a call through a value that is not a function".into());
        };
        let cx = self.cx;
        // A `&` parameter passes the address of the caller's variable, as a direct call does.
        let sig_params = params
            .iter()
            .map(|p| match p.base() {
                crate::data::Type::RefVar(inner) => cx
                    .ctype(inner)
                    .filter(|c| *c != CType::Void)
                    .map(|c| (c, true)),
                p => cx
                    .ctype(p)
                    .filter(|c| *c != CType::Void)
                    .map(|c| (c, false)),
            })
            .collect::<Option<Vec<_>>>()
            .ok_or("a function value with a parameter of no C shape")?;
        let shapes: Vec<CType> = sig_params.iter().map(|(c, _)| *c).collect();
        let ret = cx
            .ctype(&ret)
            .ok_or_else(|| format!("a function value returning {}", type_word(&ret)))?;
        if args.len() < shapes.len() {
            return Err("a call through a function value with arguments left out".into());
        }
        let mut xs = vec![String::new(); shapes.len()];
        let mut values = Vec::new();
        // Each parameter's mode: a value (0), a link to a variable (1), a store place (2) —
        // the last reaches each function through its place twin (`twin_name`).
        let mut modes = Vec::with_capacity(shapes.len());
        for (i, (c, link)) in sig_params.iter().enumerate() {
            if *link {
                if let Some(p) = self.place_arg(&args[i], &params[i], ind)? {
                    xs[i] = p;
                    modes.push((*c, 2));
                } else {
                    xs[i] = self.link_arg(&args[i], *c, ind)?;
                    modes.push((*c, 1));
                }
            } else {
                values.push(i);
                modes.push((*c, 0));
            }
        }
        let sig = cx.fn_sig(modes, ret);
        self.value_args(&args[..shapes.len()], &shapes, &values, &mut xs, false, ind)?;
        let mut list = vec![self.var(v)];
        list.extend(xs);
        let call = format!("loft_call{sig}({})", list.join(", "));
        Ok(match ret {
            CType::Text => {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(ind, &format!("{t} = {call};"));
                self.held(t, CType::Text)
            }
            c => E::new(call, kind_of(c)),
        })
    }

    /// The `&` arguments of a call to `d` into `xs`: the address of a variable, or the
    /// `loft_ref` of a store place — a field, an element, a store link the caller holds —
    /// which calls the callee's place twin, taking that parameter as a `loft_ref`.  Answers
    /// the positions of the value arguments and the twin's mask.
    fn link_args(
        &mut self,
        d: u32,
        args: &[Value],
        shapes: &[CType],
        xs: &mut [String],
        ind: usize,
    ) -> Result<(Vec<usize>, u64), String> {
        let def = self.cx.data.def(d);
        let callee_args = def.variables().arguments();
        let mut values = Vec::new();
        let mut places = 0u64;
        for (i, arg) in args.iter().enumerate() {
            let param = &def.attributes()[i].typedef;
            if !matches!(param.base(), crate::data::Type::RefVar(_)) {
                values.push(i);
                continue;
            }
            let Some(p) = self.place_arg(arg, param, ind)? else {
                xs[i] = self.link_arg(arg, shapes[i], ind)?;
                continue;
            };
            // A store instance takes the place as it is; a text place reaches no other
            // function (the IR retargets each such call to its instance).
            if !callee_args
                .get(i)
                .is_some_and(|&v| def.variables().is_store_text_link(v))
            {
                if matches!(param.peel_link().base(), crate::data::Type::Text(_)) {
                    return Err(format!("a text place handed to {}", def.original_name()));
                }
                places |= 1 << i;
            }
            xs[i] = p;
        }
        Ok((values, places))
    }

    /// A `&` argument that names a store place — a field (`OpGetField`), an element
    /// (`OpGetVector`), a store link the caller holds — as the `loft_ref` of the place, for a
    /// parameter of a kind a store link carries (`store_links`); `None` for a variable.
    fn place_arg(
        &mut self,
        a: &Value,
        param: &crate::data::Type,
        ind: usize,
    ) -> Result<Option<String>, String> {
        let crate::data::Type::RefVar(inner) = param.base() else {
            return Ok(None);
        };
        if !link_kind(inner) {
            return Ok(None);
        }
        match a.unspan() {
            Value::Var(v) if self.store_links[*v as usize] => {
                Ok(Some(var_name(self.vars.name(*v), *v)))
            }
            Value::Call(op, _)
                if matches!(
                    self.cx.data.def(*op).name(),
                    "OpGetField" | "OpGetVector" | "OpVarRef"
                ) =>
            {
                let e = self.expr(a, ind)?;
                let t = self.temp(CType::Ref);
                self.line(ind, &format!("{t} = {};", as_ref(e)?));
                Ok(Some(t))
            }
            _ => Ok(None),
        }
    }

    /// The address a `&` argument passes: the caller's own variable, or a link it holds.
    fn link_arg(&mut self, a: &Value, c: CType, ind: usize) -> Result<String, String> {
        let v = match a.unspan() {
            // A text result's buffer: `{ __work = ""; OpCreateStack(__work) }`.
            Value::Block(b) => {
                let Some((last, rest)) = b.operators.split_last() else {
                    return Err("a link block with no operators".into());
                };
                self.nested(|w| rest.iter().try_for_each(|o| w.stmt(o, ind)))?;
                return self.link_arg(last, c, ind);
            }
            Value::Call(op, xs) if self.cx.data.def(*op).name() == "OpCreateStack" => {
                match xs.as_slice() {
                    [x] => match x.unspan() {
                        Value::Var(v) => *v,
                        Value::TupleGet(tup, member) => {
                            let CType::Tuple(id) = self.shapes[*tup as usize] else {
                                return Err("a link to a member of no tuple".into());
                            };
                            if self.cx.tuple_shapes(id).get(*member as usize) != Some(&c) {
                                return Err("a link to a member of another shape".into());
                            }
                            return self
                                .stack_target(a)
                                .ok_or_else(|| "a link to a tuple member it cannot name".into());
                        }
                        _ => return Err("a link to a computed place".into()),
                    },
                    _ => return Err("OpCreateStack without one operand".into()),
                }
            }
            Value::Var(v) if self.links[*v as usize] => *v,
            Value::Call(op, _) => {
                return Err(format!("a link made by {}", self.cx.data.def(*op).name()));
            }
            other => return Err(format!("a link to {}", unsupported(other))),
        };
        if self.shapes[v as usize] != c || (c == CType::Text && !self.text_held(v)) {
            return Err("a link to a variable of another shape".into());
        }
        Ok(format!("&{}", self.var(v)))
    }

    fn one<V: Borrow<Value>>(&mut self, name: &str, args: &[V], ind: usize) -> Result<E, String> {
        let es = self.args(args, ind)?;
        let [a] = <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
        Ok(a)
    }

    /// `two` for a text VALUE op: its text field operands read in place (`text_reads`).
    fn text_two<V: Borrow<Value>>(
        &mut self,
        name: &str,
        args: &[V],
        ind: usize,
    ) -> Result<(E, E), String> {
        let es = self.text_reads(args, ind)?;
        let [a, b] = <[E; 2]>::try_from(es).map_err(|_| format!("{name} without two operands"))?;
        Ok((a, b))
    }

    fn two<V: Borrow<Value>>(
        &mut self,
        name: &str,
        args: &[V],
        ind: usize,
    ) -> Result<(E, E), String> {
        let es = self.args(args, ind)?;
        let [a, b] = <[E; 2]>::try_from(es).map_err(|_| format!("{name} without two operands"))?;
        Ok((a, b))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "past the bar where the 32-bit arms met the value-record arms on a join; the split is release work (M-file-split)"
    )]
    fn operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        // The text writes through a `&` parameter: the C names the caller's text the same way.
        let name = match name {
            "OpAppendStackText" => "OpAppendText",
            "OpClearStackText" => "OpClearText",
            "OpFormatStackInt" => "OpFormatInt",
            "OpFormatStackFloat" => "OpFormatFloat",
            "OpFormatStackSingle" => "OpFormatSingle",
            "OpFormatStackText" => "OpFormatText",
            "OpFormatStackDatabase" => "OpFormatDatabase",
            n => n,
        };
        if self.ds.is_some()
            && let Some(e) = self.dest_scalar_set(name, args, ind)?
        {
            return Ok(e);
        }
        if let Some(e) = self.apr_operator(name, args, ind)? {
            return Ok(e);
        }
        if self.vr.is_some()
            && let Some(e) = self.vr_operator(name, args, ind)?
        {
            return Ok(e);
        }
        if !self.plain_chains.is_empty()
            && let Some(e) = self.chain_operator(name, args, ind)?
        {
            return Ok(e);
        }
        if !self.headers.is_empty()
            && let Some(e) = self.held_access(name, args, ind)?
        {
            return Ok(e);
        }
        if !self.windows.is_empty()
            && let Some(e) = self.windowed(name, args, ind)?
        {
            return Ok(e);
        }
        if self.push_heads.iter().any(|f| !f.is_empty())
            && let Some(e) = self.pushed_mint(name, args, ind)?
        {
            return Ok(e);
        }
        if self.scalars.iter().any(|f| !f.is_empty())
            && let Some(key) = crate::opt::hoist::scalar_read(name, args)
            && let Some((local, c)) = self.held_scalar(&key)
        {
            return Ok(E::new(local, kind_of(c)));
        }
        if !self.rec_ptrs.is_empty()
            && let Some(e) = self.rec_ptr_access(name, args, ind)?
        {
            return Ok(e);
        }
        if !self.lazy.is_empty()
            && let Some(e) = self.lazy_split_op(name, args)
        {
            return Ok(e);
        }
        if !self.split_tabs.is_empty()
            && let Some(e) = self.split_tab_op(name, args, ind)?
        {
            return Ok(e);
        }
        if let Some(e) = self.keyed_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.file_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.gen_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.range_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.probe_operator(name, args, ind)? {
            return Ok(e);
        }
        if name == "OpNarrow32"
            && let [x] = args
        {
            return self.narrow32(x, ind);
        }
        let arith = match name {
            // The `Nullable` forms answer the same values; only the overflow report differs.
            "OpAddInt" | "OpAddLong" | "OpAddIntNullable" => Some("loft_add"),
            "OpMinInt" | "OpMinLong" | "OpMinIntNullable" => Some("loft_sub"),
            "OpMulInt" | "OpMulLong" | "OpMulIntNullable" => Some("loft_mul"),
            "OpDivInt" | "OpDivLong" | "OpDivIntNullable" => Some("loft_div"),
            "OpRemInt" | "OpRemLong" | "OpRemIntNullable" => Some("loft_rem"),
            "OpLandInt" | "OpLandLong" => Some("loft_and"),
            "OpLorInt" | "OpLorLong" => Some("loft_or"),
            "OpEorInt" | "OpEorLong" => Some("loft_xor"),
            "OpSLeftInt" | "OpSLeftLong" => Some("loft_shl"),
            "OpSRightInt" | "OpSRightLong" => Some("loft_shr"),
            _ => None,
        };
        if let Some(f) = arith {
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(
                format!("{f}({}, {})", as_int(a)?, as_int(b)?),
                K::Int,
            ));
        }
        let cmp = match name {
            "OpEqInt" | "OpEqLong" => Some("=="),
            "OpNeInt" | "OpNeLong" => Some("!="),
            "OpLtInt" | "OpLtLong" => Some("<"),
            "OpLeInt" | "OpLeLong" => Some("<="),
            _ => None,
        };
        if let Some(op) = cmp {
            let (a, b) = self.two32(name, args, ind)?;
            // Two 32-bit operands compare as they are: the null is the lowest value in
            // either width, so the order is the 64-bit one.
            if a.kind == K::Int32 && b.kind == K::Int32 {
                return Ok(compare(&a.code, op, &b.code));
            }
            return Ok(compare(&as_int(a)?, op, &as_int(b)?));
        }
        self.text_operator(name, args, ind)
    }

    /// @PLN204 — the two operands of a binary integer operator, a literal that fits taken as a
    /// 32-bit one, so an operation on 32-bit values can be written in 32 bits.
    fn two32(&mut self, name: &str, args: &[Value], ind: usize) -> Result<(E, E), String> {
        let (a, b) = self.two(name, args, ind)?;
        let lit = |v: &Value, e: E| match v.unspan() {
            Value::Int(i) if *i != i32::MIN => E::new(format!("INT32_C({i})"), K::Int32),
            _ => e,
        };
        Ok((lit(&args[0], a), lit(&args[1], b)))
    }

    /// @PLN204 — `OpNarrow32(x)`, the 32-bit regime's arithmetic: `x` computed and narrowed to
    /// `int32_t`, a value that does not fit reading null.  An add, subtract, multiply, divide
    /// or remainder of two 32-bit operands is written in 32 bits (`loft_add32` …), which
    /// answers the same: the 64-bit operation of two `int32_t` values cannot overflow, so
    /// narrowing its result is exactly the 32-bit operation's overflow check.
    fn narrow32(&mut self, x: &Value, ind: usize) -> Result<E, String> {
        if let Value::Int(lit) = x.unspan()
            && *lit != i32::MIN
            && !super::no_c_int32()
        {
            return Ok(E::new(format!("INT32_C({lit})"), K::Int32));
        }
        if let Value::Call(op_nr, args) = x.unspan()
            && args.len() == 2
            && is_operator(self.cx.data, *op_nr)
        {
            let name = self.cx.data.def(*op_nr).name();
            let op32 = match name {
                "OpAddInt" | "OpAddIntNullable" => Some("loft_add32"),
                "OpMinInt" | "OpMinIntNullable" => Some("loft_sub32"),
                "OpMulInt" | "OpMulIntNullable" => Some("loft_mul32"),
                "OpDivInt" | "OpDivIntNullable" => Some("loft_div32"),
                "OpRemInt" | "OpRemIntNullable" => Some("loft_rem32"),
                _ => None,
            };
            if let Some(op32) = op32
                && !super::no_c_int32()
            {
                let (a, b) = self.two32(name, args, ind)?;
                if a.kind == K::Int32 && b.kind == K::Int32 {
                    return Ok(E::new(format!("{op32}({}, {})", a.code, b.code), K::Int32));
                }
                let wide = &op32[..op32.len() - 2];
                return Ok(E::new(
                    format!("loft_n32({wide}({}, {}))", as_int(a)?, as_int(b)?),
                    K::Int32,
                ));
            }
        }
        let e = self.expr(x, ind)?;
        if super::no_c_int32() {
            return Ok(E::new(
                format!("loft_w64(loft_n32({}))", as_int(e)?),
                K::Int,
            ));
        }
        Ok(E::new(as_int32(e)?, K::Int32))
    }

    /// `target = if cond { t } else { e }`, each arm assigning it; a not-null test's else opens
    /// with `LOFT_COLD_PATH()` (see [`Self::null_test`]).
    fn assign_if(
        &mut self,
        cond: &Value,
        t: &Value,
        e: &Value,
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        let null_test = self.null_test(cond);
        // Only the test is evaluated before the `if`; each arm's work — its call temporaries
        // included — is emitted inside that arm's braces, so it runs only when the arm does.
        // `a && b` and `a || b` arrive as such an `if`, which is what keeps the right operand
        // unevaluated when the left decides (`@FR-E-And`, `@FR-E-Or`).
        let cond = self.expr(cond, ind)?;
        let cond = as_cond(cond)?;
        self.line(ind, &format!("if ({}) {{", bare(&cond)));
        self.assign(t, target, c, ind + 1)?;
        self.line(ind, "} else {");
        if null_test {
            self.line(ind + 1, "LOFT_COLD_PATH();");
        }
        self.assign(e, target, c, ind + 1)?;
        self.line(ind, "}");
        Ok(())
    }

    /// Is `c` a not-null test (`x != null` of a number)?  Its else is the null's path, which
    /// `LOFT_COLD_PATH()` keeps a branch: a discharge is then a predicted jump, not a select in
    /// the dependency chain of the code around it.
    fn null_test(&self, c: &Value) -> bool {
        matches!(c.unspan(), Value::Call(d, a) if a.len() == 1
            && matches!(self.cx.data.def(*d).name(),
                "OpConvBoolFromFloat" | "OpConvBoolFromSingle"
                | "OpConvBoolFromInt" | "OpConvBoolFromLong"))
    }

    /// `@FR-R-Range`, as native writes it: an integer operator whose result the range proof
    /// (`opt::range::op_range`, over this function's facts) shows fits the type cannot fault,
    /// so it is the plain C operator — the exact value, the processor's instruction, and a
    /// signed operation the compiler may reason about (a counted loop's step is one: its
    /// wrapping or checked forms keep GCC from counting the loop).  Both lanes; a division
    /// only where the range excludes a zero divisor.  `LOFT_NO_RANGE_ARITH=1` keeps every
    /// operator checked, as on native.
    fn range_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Some(form) = crate::opt::range::plain_form(name, args.len()) else {
            return Ok(None);
        };
        if crate::env_once!(std::env::var("LOFT_NO_RANGE_ARITH").is_ok_and(|v| v != "0")) {
            return Ok(None);
        }
        let facts = self.nn_facts();
        if crate::opt::range::op_range(self.cx.data, &facts.0, &facts.1, name, args, 0).is_none() {
            return Ok(None);
        }
        self.range_plain(form, name, args, ind)
    }

    /// This function's non-sentinel and range facts (`non_sentinel_vars`, `range_vars`), a
    /// `__nn` twin's seeded parameters included; computed once.
    fn nn_facts(
        &mut self,
    ) -> std::rc::Rc<(
        std::collections::HashMap<u16, bool>,
        crate::opt::range::Ranges,
    )> {
        if let Some(f) = &self.range_facts {
            f.clone()
        } else {
            {
                let data = self.cx.data;
                let def = data.def(self.d);
                let nn = crate::opt::non_sentinel::non_sentinel_vars_seeded(
                    data,
                    def.variables(),
                    def.code(),
                    &self.nn_seed,
                );
                let rv = crate::opt::range::range_vars(
                    data,
                    def.variables(),
                    def.code(),
                    &nn,
                    &self.char_walks,
                    &crate::opt::range::Ranges::default(),
                );
                let f = std::rc::Rc::new((nn, rv));
                self.range_facts = Some(f.clone());
                f
            }
        }
    }

    /// The plain C operator for `name` over `args`, which `op_range` proved fault-free.
    fn range_plain(
        &mut self,
        form: &str,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        crate::rewrite_census::fired("R-Range", 1);
        if form == "wrapping_neg" {
            let a = self.one(name, args, ind)?;
            return Ok(Some(E::new(format!("(-({}))", as_int(a)?), K::Int)));
        }
        let op = match form {
            "wrapping_add" => "+",
            "wrapping_sub" => "-",
            "wrapping_mul" => "*",
            other => other,
        };
        let (a, b) = self.two(name, args, ind)?;
        Ok(Some(E::new(
            format!("(({}) {op} ({}))", as_int(a)?, as_int(b)?),
            K::Int,
        )))
    }

    /// `LOFT_RELEASE_PASS_PROBE=1` (generation time): the release-pass probe's form of an
    /// integer operator or a float compare — the processor's wrapping arithmetic, each wrap defined
    /// (`loft_wrap_*` in `loft_rt.h`) — for exactly the operators and operand shapes
    /// `generation::ops::int_arith::probe_form` gives the Rust generator, so the C and Rust
    /// ship lanes measure the same tier.  The `Nullable` forms and the null test keep their
    /// checks: they are the language's null semantics, not fault protection.  A MEASUREMENT
    /// instrument (@C129), never a build anyone ships.
    fn probe_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if !crate::env_once!(std::env::var("LOFT_RELEASE_PASS_PROBE").is_ok_and(|v| v != "0")) {
            return Ok(None);
        }
        // A float compare is the plain C operator, as native's `FloatCompareEmitter` writes it
        // under the probe (`non_sentinel::plain_float_compare` names the operators).
        if let Some(op) = crate::opt::non_sentinel::plain_float_compare(name)
            && args.len() == 2
        {
            let (a, b) = self.two(name, args, ind)?;
            let (a, b) = if name.ends_with("Single") {
                (as_single(a)?, as_single(b)?)
            } else {
                (as_float(a)?, as_float(b)?)
            };
            return Ok(Some(E::new(format!("(({a}) {op} ({b}))"), K::Cond)));
        }
        let shift_k =
            || matches!(args.get(1).map(Value::unspan), Some(Value::Int(k)) if (0..64).contains(k));
        let form = match (name, args.len()) {
            ("OpAddInt", 2) => "loft_wrap_add",
            ("OpMinInt", 2) => "loft_wrap_sub",
            ("OpMulInt", 2) => "loft_wrap_mul",
            ("OpDivInt", 2) => "loft_wrap_div",
            ("OpRemInt", 2) => "loft_wrap_rem",
            ("OpLandInt", 2) => "&",
            ("OpLorInt", 2) => "|",
            ("OpEorInt", 2) => "^",
            ("OpSRightInt", 2) if shift_k() => ">>",
            ("OpMinSingleInt", 1) => {
                let a = self.one(name, args, ind)?;
                return Ok(Some(E::new(
                    format!("loft_wrap_neg({})", as_int(a)?),
                    K::Int,
                )));
            }
            _ => return Ok(None),
        };
        let (a, b) = self.two(name, args, ind)?;
        let (a, b) = (as_int(a)?, as_int(b)?);
        let text = if form.starts_with("loft_") {
            format!("{form}({a}, {b})")
        } else {
            format!("(({a}) {form} ({b}))")
        };
        Ok(Some(E::new(text, K::Int)))
    }

    /// The boolean and text operators, and printing.
    #[expect(clippy::too_many_lines, reason = "one arm per operator")]
    fn text_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            "OpEqBool" | "OpNeBool" => {
                let (a, b) = self.two(name, args, ind)?;
                let op = if name == "OpEqBool" { "==" } else { "!=" };
                Ok(compare(&as_bool(a)?, op, &as_bool(b)?))
            }
            "OpNot" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| "OpNot without one operand".to_string())?;
                Ok(E::new(format!("(!{})", as_cond(a)?), K::Cond))
            }
            "OpEqText" | "OpNeText" => {
                let (a, b) = self.text_two(name, args, ind)?;
                let neg = if name == "OpNeText" { "!" } else { "" };
                Ok(E::new(
                    format!("{neg}loft_str_eq({}, {})", as_view(a)?, as_view(b)?),
                    K::Cond,
                ))
            }
            "OpLtText" | "OpLeText" => {
                let (a, b) = self.text_two(name, args, ind)?;
                let f = if name == "OpLtText" {
                    "loft_text_lt"
                } else {
                    "loft_text_le"
                };
                Ok(E::new(
                    format!("{f}({}, {})", as_view(a)?, as_view(b)?),
                    K::Cond,
                ))
            }
            "OpAppendText" => {
                let [dest, val] = args else {
                    return Err("OpAppendText without two operands".into());
                };
                if let Some(e) = self.format_into_dest(dest, val, ind)? {
                    return Ok(e);
                }
                let dest = self.text_dest(dest)?;
                // A formatted value — `{ __work = ""; …; __work }` — is appended from its work
                // buffer itself, as native's `append_text(&mut big, &__work)` borrows it: the
                // append reads it at once and nothing writes the buffer in between, where the
                // block as an expression would copy it into a temporary first.
                if let Value::Block(b) = val.unspan()
                    && let Some((last, rest)) = b.operators.split_last()
                    && let Value::Var(w) = last.unspan()
                    && self.shapes[*w as usize] == CType::Text
                    && self.var(*w) != dest
                {
                    let w = *w;
                    self.nested(|wr| rest.iter().try_for_each(|o| wr.stmt(o, ind)))?;
                    let val = self.expr(&Value::Var(w), ind)?;
                    return Ok(E::new(
                        format!("loft_text_concat(&{dest}, {})", as_view(val)?),
                        K::Void,
                    ));
                }
                let [val] = <[E; 1]>::try_from(self.text_reads(std::slice::from_ref(val), ind)?)
                    .map_err(|_| "OpAppendText without its value".to_string())?;
                Ok(E::new(
                    format!("loft_text_concat(&{dest}, {})", as_view(val)?),
                    K::Void,
                ))
            }
            "OpFormatInt" | "OpFormatLong" => self.format_long(name, args, ind),
            "OpFormatText" => {
                let [dest, val, width, dir, token] = args else {
                    return Err("OpFormatText with an unexpected operand list".into());
                };
                let dest = self.text_dest(dest)?;
                let (Value::Int(dir), Value::Int(token)) = (dir.unspan(), token.unspan()) else {
                    return Err("OpFormatText with a computed format".into());
                };
                let (val, width) = self.text_two(name, &[val, width], ind)?;
                Ok(E::new(
                    format!(
                        "loft_format_text(&{dest}, {}, {}, {dir}, {token})",
                        as_view(val)?,
                        as_int(width)?
                    ),
                    K::Void,
                ))
            }
            "OpFormatDatabase" => self.format_database(args, ind),
            "OpFreeText" => {
                let [dest] = args else {
                    return Err("OpFreeText without one operand".into());
                };
                // A member that is itself a tuple frees each text it holds, at any depth, as the
                // bytecode's `emit_tuple_text_free` does.
                if let Value::TupleGet(var, i) = dest.unspan()
                    && let CType::Tuple(id) = self.shapes[*var as usize]
                    && let Some(CType::Tuple(inner)) = self.cx.tuple_shapes(id).get(*i as usize)
                {
                    let mut frees = Vec::new();
                    self.tuple_text_leaves(&format!("{}.f{i}", self.var(*var)), *inner, &mut frees);
                    let code = frees
                        .iter()
                        .map(|t| format!("loft_text_free(&{t})"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Ok(E::new(
                        if code.is_empty() {
                            "((void) 0)".into()
                        } else {
                            format!("({code})")
                        },
                        K::Void,
                    ));
                }
                // A borrowed walk variable views the element: its release frees nothing — but a
                // tested borrow's own text, which its copying path filled.
                if let Value::Var(v) = dest.unspan()
                    && self.borrowed[*v as usize]
                {
                    if self.tested.contains_key(v) {
                        return Ok(E::new(format!("loft_text_free(&loft_own{v})"), K::Void));
                    }
                    return Ok(E::new("((void) 0)", K::Void));
                }
                let dest = self.text_dest(dest)?;
                Ok(E::new(format!("loft_text_free(&{dest})"), K::Void))
            }
            "OpPrint" => {
                let es = self.text_reads(args, ind)?;
                let [a] = <[E; 1]>::try_from(es)
                    .map_err(|_| "OpPrint without one operand".to_string())?;
                Ok(E::new(format!("loft_print({})", as_view(a)?), K::Void))
            }
            _ => self.scalar_operator(name, args, ind),
        }
    }

    /// An integer into a format string: `ops::format_long`.
    fn format_long(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        let [dest, val, radix, width, token, plus, note, dir] = args else {
            return Err(format!("{name} with an unexpected operand list"));
        };
        let dest = self.text_dest(dest)?;
        let (
            Value::Int(radix),
            Value::Int(token),
            Value::Boolean(plus),
            Value::Boolean(note),
            Value::Int(dir),
        ) = (
            radix.unspan(),
            token.unspan(),
            plus.unspan(),
            note.unspan(),
            dir.unspan(),
        )
        else {
            return Err(format!("{name} with a computed format"));
        };
        let (val, width) = self.two(name, &[val, width], ind)?;
        Ok(E::new(
            format!(
                "loft_format_long(&{dest}, {}, {radix}, {}, {token}, {}, {}, {dir})",
                as_int(val)?,
                as_int(width)?,
                u8::from(*plus),
                u8::from(*note)
            ),
            K::Void,
        ))
    }

    /// `T.to_json()` / `T.to_json_pretty()` of a record (`Stores::show_json`): its type's
    /// render function in the JSON form; `T.parse(JsonValue)`, the walk over the tree; `None`
    /// for another native.
    fn struct_json_native(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let pretty = match name {
            "n_struct_to_json" => 0,
            "n_struct_to_json_pretty" => 1,
            // `T.parse(JsonValue)`: the struct walk over the tree (`loft_parse.h`).
            "n_struct_from_jsonvalue" => {
                let [val, tp] = args else {
                    return Err(format!("{name} with an unexpected operand list"));
                };
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.parse_type(tp)?;
                let val = self.expr(val, ind)?;
                return Ok(Some(E::new(
                    format!(
                        "loft_struct_from_jsonvalue(loft_ptypes, {}, {tp})",
                        as_ref(val)?
                    ),
                    K::Ref,
                )));
            }
            _ => return Ok(None),
        };
        let [val, tp] = args else {
            return Err(format!("{name} with an unexpected operand list"));
        };
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        self.cx.show_type(tp)?;
        let val = self.expr(val, ind)?;
        let t = self.temp(CType::Text);
        self.line(ind, &format!("loft_text_free(&{t});"));
        self.line(
            ind,
            &format!(
                "{}(&{t}, {}, {}, 0);",
                super::Cx::show_fn(tp),
                as_ref(val)?,
                2 | pretty
            ),
        );
        Ok(Some(self.held(t, CType::Text)))
    }

    /// `{v}` of a record or a vector: the type's render function (`show.rs`).
    fn format_database(&mut self, args: &[Value], ind: usize) -> Result<E, String> {
        let [dest, val, tp, fmt] = args else {
            return Err("OpFormatDatabase with an unexpected operand list".into());
        };
        let dest = self.text_dest(dest)?;
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let fmt = Self::literal(fmt, "format")?;
        self.cx.show_type(tp)?;
        let val = self.expr(val, ind)?;
        Ok(E::new(
            format!(
                "{}(&{dest}, {}, {fmt}, 0)",
                super::Cx::show_fn(tp),
                as_ref(val)?
            ),
            K::Void,
        ))
    }

    /// The conversions and the text measures.
    fn scalar_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            "OpLengthText" | "OpSizeText" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                let f = if name == "OpLengthText" {
                    "loft_text_length"
                } else {
                    "loft_text_size"
                };
                Ok(E::new(format!("{f}({})", as_view(a)?), K::Int))
            }
            "OpRangeDefault" => {
                let [v, lo, hi, dflt] = args else {
                    return Err("OpRangeDefault with an unexpected operand list".into());
                };
                let lo = Self::literal(lo, "range bound")?;
                let hi = Self::literal(hi, "range bound")?;
                let dflt = Self::literal(dflt, "range default")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_range_default({}, {}, {}, {})",
                        as_int(v)?,
                        c_long(lo),
                        c_long(hi),
                        c_long(dflt)
                    ),
                    K::Int,
                ))
            }
            "OpMinSingleInt" | "OpMinSingleLong" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                Ok(E::new(format!("loft_neg({})", as_int(a)?), K::Int))
            }
            "OpConvBoolFromInt" | "OpConvBoolFromLong" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                Ok(E::new(format!("({} != LOFT_NULL)", as_int(a)?), K::Cond))
            }
            "OpConvIntFromNull" | "OpConvLongFromNull" => Ok(E::new("LOFT_NULL", K::Int)),
            "OpConvEnumFromNull" => Ok(E::new("(uint8_t) 255", K::Enum)),
            "OpConstEnum" => self.expr(args.first().ok_or("OpConstEnum without a value")?, ind),
            "OpConvIntFromEnum" => {
                let a = self.one(name, args, ind)?;
                let a = as_enum(a)?;
                Ok(E::new(
                    format!("({a} == 255 ? LOFT_NULL : (int64_t) {a})"),
                    K::Int,
                ))
            }
            "OpCastEnumFromInt" => {
                let a = self.one(name, args, ind)?;
                let a = as_int(a)?;
                Ok(E::new(
                    format!("({a} == LOFT_NULL ? (uint8_t) 255 : (uint8_t) {a})"),
                    K::Enum,
                ))
            }
            "OpConvBoolFromEnum" => {
                let a = self.one(name, args, ind)?;
                let a = as_enum(a)?;
                Ok(E::new(format!("({a} != 255 && {a} != 0)"), K::Cond))
            }
            "OpCastTextFromEnum" => {
                let [v, tp] = args else {
                    return Err("OpCastTextFromEnum with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                let names = self.cx.enum_names(tp);
                let v = self.expr(v, ind)?;
                let t = self.temp(CType::Enum);
                self.line(ind, &format!("{t} = {};", as_enum(v)?));
                Ok(E::new(
                    format!(
                        "({t} == 0 || {t} == 255 ? loft_null_text() : loft_lit({names}[{t}], strlen({names}[{t}])))"
                    ),
                    K::View,
                ))
            }
            "OpCastVectorFromText" | "OpCastEnumFromText" => self.cast_from_text(name, args, ind),
            _ => self.float_operator(name, args, ind),
        }
    }

    /// A value read from a text: a type's whole record (`Stores::record_from_text`) or a plain
    /// enum's value by its name (`Stores::to_enum`).
    fn cast_from_text(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            // `Stores::record_from_text`: a new store holding the type the text describes.
            "OpCastVectorFromText" => {
                let [v, tp] = args else {
                    return Err("OpCastVectorFromText with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                self.cx.parse_type(tp)?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("loft_record_from_text(loft_ptypes, {}, {tp})", as_view(v)?),
                    K::Ref,
                ))
            }
            // `Stores::to_enum`: the value whose name the text is, 0 for none.
            "OpCastEnumFromText" => {
                let [v, tp] = args else {
                    return Err("OpCastEnumFromText with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                let names = self.cx.enum_names(tp);
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_enum_of_text({names}, sizeof {names} / sizeof *{names}, {})",
                        as_view(v)?
                    ),
                    K::Enum,
                ))
            }
            _ => Err(format!("operator {name}")),
        }
    }

    /// The float and single operators: `Float` names a double, `Single` a float.
    fn float_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        /// `(operator, single?, C spelling)` for the binary arithmetic.
        const INFIX: &[(&str, bool, &str)] = &[
            ("OpAddFloat", false, "+"),
            ("OpMinFloat", false, "-"),
            ("OpMulFloat", false, "*"),
            ("OpDivFloat", false, "/"),
            ("OpDivFloatNullable", false, "/"),
            ("OpAddSingle", true, "+"),
            ("OpMinSingle", true, "-"),
            ("OpMulSingle", true, "*"),
            ("OpDivSingle", true, "/"),
            ("OpDivSingleNullable", true, "/"),
        ];
        /// `(operator, single?, C function)` for the calls of two operands.
        const CALL2: &[(&str, bool, &str)] = &[
            ("OpRemFloat", false, "fmod"),
            ("OpRemFloatNullable", false, "fmod"),
            ("OpPowFloat", false, "pow"),
            ("OpRemSingle", true, "fmodf"),
            ("OpRemSingleNullable", true, "fmodf"),
            ("OpPowSingle", true, "powf"),
        ];
        /// `(operator, single?, C comparison, negated?)`.
        const CMP: &[(&str, bool, &str, bool)] = &[
            ("OpEqFloat", false, "loft_feq", false),
            ("OpNeFloat", false, "loft_feq", true),
            ("OpLtFloat", false, "loft_flt", false),
            ("OpLeFloat", false, "loft_fle", false),
            ("OpEqSingle", true, "loft_feq", false),
            ("OpNeSingle", true, "loft_feq", true),
            ("OpLtSingle", true, "loft_flt", false),
            ("OpLeSingle", true, "loft_fle", false),
        ];
        let shape = |single: bool| -> (K, fn(E) -> Result<String, String>) {
            if single {
                (K::Single, as_single)
            } else {
                (K::Float, as_float)
            }
        };
        if let Some((_, single, op)) = INFIX.iter().find(|(n, _, _)| *n == name) {
            let (k, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(format!("({} {op} {})", conv(a)?, conv(b)?), k));
        }
        if let Some((_, single, f)) = CALL2.iter().find(|(n, _, _)| *n == name) {
            let (k, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(format!("{f}({}, {})", conv(a)?, conv(b)?), k));
        }
        if let Some((_, single, f, neg)) = CMP.iter().find(|(n, _, _, _)| *n == name) {
            let (_, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            let neg = if *neg { "!" } else { "" };
            return Ok(E::new(
                format!("{neg}{f}({}, {})", conv(a)?, conv(b)?),
                K::Cond,
            ));
        }
        let unary = match name {
            "OpMinSingleFloat" => Some((false, "-")),
            "OpMinSingleSingle" => Some((true, "-")),
            "OpAbsFloat" => Some((false, "fabs")),
            "OpAbsSingle" => Some((true, "fabsf")),
            _ => None,
        };
        if let Some((single, f)) = unary {
            let (k, conv) = shape(single);
            let a = self.one(name, args, ind)?;
            return Ok(E::new(format!("({f}({}))", conv(a)?), k));
        }
        self.float_conversion(name, args, ind)
    }

    /// The conversions to and from floats, their math functions and their fields.
    fn float_conversion(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConvFloatFromNull", []) => Ok(E::new("(double) NAN", K::Float)),
            ("OpConvSingleFromNull", []) => Ok(E::new("(float) NAN", K::Single)),
            ("OpConvFloatFromInt" | "OpConvFloatFromLong", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_i2f({})", as_int(a)?), K::Float))
            }
            ("OpConvSingleFromInt", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_i2s({})", as_int(a)?), K::Single))
            }
            ("OpCastIntFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_f2i({})", as_float(a)?), K::Int))
            }
            ("OpCastIntFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(
                    format!("loft_f2i((double) {})", as_single(a)?),
                    K::Int,
                ))
            }
            ("OpConvBoolFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("(!isnan({}))", as_float(a)?), K::Cond))
            }
            ("OpConvBoolFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("(!isnan({}))", as_single(a)?), K::Cond))
            }
            ("OpCastSingleFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("((float) {})", as_float(a)?), K::Single))
            }
            ("OpConvFloatFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("((double) {})", as_single(a)?), K::Float))
            }
            // The constants as the shortest literals that round-trip to Rust's own: `M_PI` and
            // `M_E` are POSIX, and `-std=c99` does not declare them.
            ("OpMathPiFloat", []) => Ok(E::new(format!("{:?}", std::f64::consts::PI), K::Float)),
            ("OpMathEFloat", []) => Ok(E::new(format!("{:?}", std::f64::consts::E), K::Float)),
            ("OpMathFuncFloat" | "OpMathFuncSingle", [id, v]) => {
                let id = Self::literal(id, "math function")?;
                let a = self.expr(v, ind)?;
                Ok(if name.ends_with("Single") {
                    E::new(format!("loft_math1s({id}, {})", as_single(a)?), K::Single)
                } else {
                    E::new(format!("loft_math1({id}, {})", as_float(a)?), K::Float)
                })
            }
            ("OpMathFunc2Float" | "OpMathFunc2Single", [id, a, b]) => {
                let id = Self::literal(id, "math function")?;
                let (a, b) = self.two(name, &[a, b], ind)?;
                Ok(if name.ends_with("Single") {
                    E::new(
                        format!("loft_math2s({id}, {}, {})", as_single(a)?, as_single(b)?),
                        K::Single,
                    )
                } else {
                    E::new(
                        format!("loft_math2({id}, {}, {})", as_float(a)?, as_float(b)?),
                        K::Float,
                    )
                })
            }
            ("OpFormatFloat" | "OpFormatSingle", [dest, val, width, prec, token, plus, dir]) => {
                let dest = self.text_dest(dest)?;
                let (Value::Int(token), Value::Boolean(plus)) = (token.unspan(), plus.unspan())
                else {
                    return Err(format!("{name} with a computed format"));
                };
                let es = self.args(&[val, width, prec, dir], ind)?;
                let [val, width, prec, dir] =
                    <[E; 4]>::try_from(es).map_err(|_| "format operands".to_string())?;
                let (val, single) = if name == "OpFormatSingle" {
                    (format!("(double) {}", as_single(val)?), 1)
                } else {
                    (as_float(val)?, 0)
                };
                Ok(E::new(
                    format!(
                        "loft_format_float(&{dest}, {val}, {}, {}, {token}, {}, (int) {}, {single})",
                        as_int(width)?,
                        as_int(prec)?,
                        u8::from(*plus),
                        as_int(dir)?
                    ),
                    K::Void,
                ))
            }
            _ => self.store_operator(name, args, ind),
        }
    }

    /// An integer literal operand of an operator: a field offset, a size, a type number.
    fn literal(v: &Value, what: &str) -> Result<i64, String> {
        match v.unspan() {
            Value::Int(i) => Ok(i64::from(*i)),
            Value::Long(l) => Ok(*l),
            _ => Err(format!("a computed {what}")),
        }
    }

    /// The operators over stores: records and their fields, vectors.
    /// The frees of a store, or `None` for another operator.
    fn free_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let e = match (name, args) {
            // A function variable's scope-exit free releases its closure record, as native's
            // does (`var_f.1`): a closure handed back by its defining function has no other
            // release.  One built here is also freed through its own record variable first,
            // and a store already free stays as it is (`loft_free_ref`).
            ("OpFreeRef", [Value::Var(v)]) if self.shapes[*v as usize] == CType::Fn => Ok(E::new(
                format!("loft_free_var(&{}.clos)", self.var(*v)),
                K::Void,
            )),
            // A variable's free resets it to the sentinel, as native's does: the next pass's
            // pre-build free of it is then a no-op instead of a release of a reused slot.
            ("OpFreeRef", [Value::Var(v)]) if self.shapes[*v as usize] == CType::Ref => {
                Ok(E::new(format!("loft_free_var(&{})", self.var(*v)), K::Void))
            }
            ("OpFreeRef", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_free_ref({})", as_ref(r)?), K::Void))
            }
            ("OpFreeRefIfDistinct", [Value::Var(v), w])
                if self.shapes[*v as usize] == CType::Ref =>
            {
                let w = self.expr(w, ind)?;
                Ok(E::new(
                    format!("loft_free_var_distinct(&{}, {})", self.var(*v), as_ref(w)?),
                    K::Void,
                ))
            }
            // `OpFreeRefOrHandUp` (@PLN150): the placeholder is freed unless the lambda hands
            // its store back — then the caller decides, as for any value a call through a
            // function value returns (`loft_fnref_minted`), so nothing is released here.
            ("OpFreeRefIfDistinct" | "OpFreeRefOrHandUp", [_, _]) => {
                let (p, w) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_free_displaced({}, {})", as_ref(p)?, as_ref(w)?),
                    K::Void,
                ))
            }
            _ => return Ok(None),
        };
        e.map(Some)
    }

    /// `@FR-R-AppendTwin` — in a twin the buffer is the caller's vector: its length reset is
    /// skipped and it is minted only where the caller handed none.
    fn append_twin_buffer_op(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Some(buf) = self.ap_buf else {
            return Ok(None);
        };
        match (name, args) {
            ("OpSetInt4", [Value::Var(w), ..]) if *w == buf => {
                Ok(Some(E::new("((void) 0)", K::Void)))
            }
            ("OpDatabase", [Value::Var(w), _]) if *w == buf => {
                let b = self.var(*w);
                self.ap_buf = None;
                let mint = self.store_operator(name, args, ind);
                self.ap_buf = Some(buf);
                crate::rewrite_census::fired("R-AppendTwin/mint", 1);
                Ok(Some(E::new(
                    format!("if ({b}.store_nr == 0xFFFF) {{ {}; }}", mint?.code),
                    K::Void,
                )))
            }
            _ => Ok(None),
        }
    }

    fn store_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        if let Some(e) = self.free_operator(name, args, ind)? {
            return Ok(e);
        }
        if let Some(e) = self.append_twin_buffer_op(name, args, ind)? {
            return Ok(e);
        }
        match (name, args) {
            // `@FR-R-RetAdopt` — the adopted local's witness is never minted, so neither is
            // its store nor the zero its vector field would have been given.
            ("OpDatabase" | "OpSetInt4", [Value::Var(w), ..])
                if self.adopt.is_some_and(|a| a.vdb == *w) =>
            {
                Ok(E::new("((void) 0)", K::Void))
            }
            ("OpDatabase", [Value::Var(v), tp]) => {
                let tp = Self::literal(tp, "type")?;
                let tp = u16::try_from(tp).map_err(|_| "a type number out of range".to_string())?;
                let size = self.cx.stores.enum_parent_size(tp);
                self.cx.describe_type(tp);
                let var = self.var(*v);
                if self.cx.stores.is_vector_wrapper(tp) {
                    // A place's old collection is released through the wrapper's descriptor.
                    self.cx.copy_type(tp)?;
                    return Ok(E::new(
                        format!("loft_database_wrapper(loft_types, &{var}, {size}, {tp})"),
                        K::Void,
                    ));
                }
                Ok(E::new(
                    format!("loft_database_typed(&{var}, {size}, {tp})"),
                    K::Void,
                ))
            }
            ("OpRefIsNull", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("({}.store_nr == 0xFFFF)", as_ref(r)?),
                    K::Cond,
                ))
            }
            ("OpNullRefSentinel", []) => Ok(E::new("loft_ref_null()", K::Ref)),
            ("OpConvBoolFromRef", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("({}.rec != 0)", as_ref(r)?), K::Cond))
            }
            ("OpRefAlias", [r]) => self.expr(r, ind),
            ("OpGetInt", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_int({}, {fld})", as_ref(r)?),
                    K::Int,
                ))
            }
            ("OpSetInt" | "OpSetInt4", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[r, val], ind)?;
                let f = if name == "OpSetInt" {
                    "loft_set_int"
                } else {
                    "loft_set_int4"
                };
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpCopyRecord", [src, dst, tp]) => {
                let raw = Self::literal(tp, "type")?;
                let raw =
                    u16::try_from(raw).map_err(|_| "a type number out of range".to_string())?;
                let tp = self.copyable(raw & crate::keys::COPY_TP_MASK)?;
                let free = u8::from(raw & crate::keys::COPY_FREE_SOURCE != 0)
                    | (u8::from(self.cx.swap_safe(tp)) << 1)
                    | (u8::from(raw & crate::keys::COPY_FRESH_DEST != 0) << 2);
                let (src, dst) = self.two(name, &[src, dst], ind)?;
                let (src, dst) = (as_ref(src)?, as_ref(dst)?);
                let copy = format!("loft_copy_record(loft_types, {tp}, {src}, {dst}, {free})");
                // A copy into a field of an element being built: its null arm is the default.
                if self.is_fresh_field_copy(args) {
                    let (s, d) = (self.temp(CType::Ref), self.temp(CType::Ref));
                    let null_arm = self
                        .fresh_null_arm(args, &d)
                        .ok_or("a fresh field without its type")?;
                    self.line(ind, &format!("{s} = {src};"));
                    self.line(ind, &format!("{d} = {dst};"));
                    self.line(ind, &format!("if ({s}.store_nr == 0xFFFF) {{ {null_arm} }} else {{ loft_copy_record(loft_types, {tp}, {s}, {d}, {free}); }}"));
                    return Ok(E::new("((void) 0)", K::Void));
                }
                Ok(E::new(copy, K::Void))
            }
            ("OpMoveRecord" | "OpMoveField", [src, dst, tp]) => {
                self.move_op(name, src, dst, tp, ind)
            }
            ("OpGetField", [r, fld, _] | [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_field({}, {fld})", as_ref(r)?), K::Ref))
            }
            _ => self.ref_operator(name, args, ind),
        }
    }

    /// `OpEvacuate(holder, fld, home, tp)` (`Stores::evacuate`, `@FR-H-RefHome`): the copy is
    /// placed in the holder's store as `OpPlaceRecord` places one.
    fn evacuate(
        &mut self,
        holder: &Value,
        fld: &Value,
        home: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let name = "OpEvacuate";
        let fld = Self::literal(fld, "field")?;
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let tp = self.copyable(tp)?;
        let size = self.cx.stores.enum_parent_size(tp);
        let (image, n) = self.cx.default_image(tp);
        // The copy is placed in the holder's store as `OpPlaceRecord` places one.
        let foreign = self
            .value_type(holder)
            .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
            .and_then(|kt| self.cx.stores.root_vector_element_of(kt))
            .is_some_and(|elem| elem != tp);
        let (holder, home) = self.two(name, &[holder, home], ind)?;
        Ok(E::new(
            format!(
                "loft_evacuate(loft_types, {tp}, {}, {fld}, {}, {size}, {image}, {n}, {})",
                as_ref(holder)?,
                as_ref(home)?,
                u8::from(foreign)
            ),
            K::Void,
        ))
    }

    /// `OpMoveRecord` (`Stores::move_record_out`, `@FR-R-MoveLast`) and `OpMoveField`: a record
    /// moved into the field `dst` names.
    fn move_op(
        &mut self,
        name: &str,
        src: &Value,
        dst: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let tp = Self::literal(tp, "type")?;
        let tp = u16::try_from(tp).map_err(|_| "a type number out of range".to_string())?;
        let tp = self.copyable(tp)?;
        let (src, dst) = self.two(name, &[src, dst], ind)?;
        let f = if name == "OpMoveRecord" {
            "loft_move_record_out"
        } else {
            "loft_move_field"
        };
        Ok(E::new(
            format!("{f}(loft_types, {tp}, {}, {})", as_ref(src)?, as_ref(dst)?),
            K::Void,
        ))
    }

    /// `OpAppendVector(v, other, tp)` (`Stores::vector_add`, a linked element type's records by
    /// `vector_add_array`) and `OpPrependVector` (`Stores::prepend_vector`), `tp` the element.
    /// A vector value that is one scalar slice materialised into a temporary of its own —
    /// `{ <bounds>; <mint the temporary>; OpSliceView(tmp, src, lo, hi, row); tmp }`, the block a
    /// slice used as a value converts to (`Parser::materialise_slice_leaves`) — as the statements
    /// that compute its bounds and the slice op's operands.  `None` for any other value, or when
    /// anything but its own mint, zero, clear or bind mentions the temporary.
    fn slice_fill<'v>(&self, other: &'v Value) -> Option<(Vec<&'v Value>, &'v [Value])> {
        if crate::env_once!(std::env::var("LOFT_NO_SLICE_INTO").is_ok_and(|v| v != "0")) {
            return None;
        }
        let Value::Block(bl) = other.unspan() else {
            return None;
        };
        let (last, rest) = bl.operators.split_last()?;
        let Value::Var(tmp) = last.unspan() else {
            return None;
        };
        if !self.vars.name(*tmp).contains("__iter_vec") {
            return None;
        }
        let mut owned: Vec<u16> = vec![*tmp];
        owned.extend(self.vars.tp(*tmp).depend().iter().copied());
        let names_owned = |v: &Value| {
            let mut hit = false;
            v.walk(&mut |n| {
                if matches!(n, Value::Var(x) if owned.contains(x)) {
                    hit = true;
                }
            });
            hit
        };
        let mut flat: Vec<&Value> = Vec::new();
        fn flatten<'a>(ops: &'a [Value], out: &mut Vec<&'a Value>) {
            for op in ops {
                match op.unspan() {
                    Value::Insert(inner) => flatten(inner, out),
                    _ => out.push(op),
                }
            }
        }
        flatten(rest, &mut flat);
        let mut prelude = Vec::new();
        let mut slice: Option<&[Value]> = None;
        for op in flat {
            let n = op.unspan();
            if let Value::Call(d, args) = n {
                let name = self.cx.data.def(*d).name();
                if matches!(name, "OpSliceView" | "OpSliceVector")
                    && args.len() == 5
                    && matches!(args[0].unspan(), Value::Var(x) if x == tmp)
                    && !args[1..].iter().any(&names_owned)
                    && slice.is_none()
                {
                    slice = Some(&args[1..]);
                    continue;
                }
                if matches!(
                    name,
                    "OpDatabase" | "OpDatabaseNP" | "OpSetInt4" | "OpClearVector"
                ) && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if owned.contains(x))
                    && !args[1..].iter().any(&names_owned)
                {
                    continue;
                }
            }
            // The temporary's own setup — its store's null init and its bind to that store's
            // field — names nothing but the temporary's variables.
            if let Value::Set(x, val) = n
                && owned.contains(x)
            {
                let mut foreign = false;
                val.walk(&mut |m| {
                    if matches!(m, Value::Var(y) if !owned.contains(y)) {
                        foreign = true;
                    }
                });
                if foreign || slice.is_some() {
                    return None;
                }
                continue;
            }
            if names_owned(op) {
                return None;
            }
            prelude.push(op);
        }
        Some((prelude, slice?))
    }

    /// `(R-SliceInto)` (C) — the slice appended straight onto `v`: the bounds' statements, then
    /// `loft_vec_slice(v, src, lo, hi, size)`, which appends `[lo, hi)` clamped as one block —
    /// what the temporary's fill and the append it fed did together, without its store.
    fn slice_into(
        &mut self,
        v: &Value,
        prelude: &[&Value],
        slice: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        let dest = as_ref(self.expr(v, ind)?)?;
        for op in prelude {
            self.stmt(op, ind)?;
        }
        let raw = format!("({dest})");
        self.raw.insert(raw.clone(), K::Ref);
        let mut args = vec![Value::RawExpr(raw)];
        args.extend(slice.iter().cloned());
        crate::rewrite_census::fired("R-SliceInto", 1);
        self.vec_slice(&args, ind, "loft_vec_slice")
    }

    fn append_vector_op(
        &mut self,
        name: &str,
        v: &Value,
        other: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let tp = u16::try_from(Self::literal(tp, "element type")?)
            .map_err(|_| "an element type out of range")?;
        let linked = self.cx.stores.is_linked(tp);
        if name == "OpAppendVector"
            && !linked
            && let Some((prelude, slice)) = self.slice_fill(other)
        {
            return self.slice_into(v, &prelude, slice, ind);
        }
        self.cx.copy_type(tp)?;
        let (v, other) = self.two(name, &[v, other], ind)?;
        let (v, other) = (as_ref(v)?, as_ref(other)?);
        Ok(if name == "OpPrependVector" {
            E::new(
                format!(
                    "loft_vec_prepend(loft_types, {tp}, {v}, {other}, {})",
                    i32::from(linked)
                ),
                K::Bool,
            )
        } else if linked {
            E::new(
                format!("loft_vec_add_array(loft_types, {tp}, {v}, {other})"),
                K::Void,
            )
        } else {
            E::new(
                format!("loft_vec_add(loft_types, {tp}, {v}, {other})"),
                K::Void,
            )
        })
    }

    /// References as values: constants, whole references in fields, raw rebinds.
    fn ref_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConstRef", [d]) => {
                let d = u32::try_from(Self::literal(d, "constant")?)
                    .map_err(|_| "a constant out of range")?;
                Ok(E::new(self.cx.const_ref(d)?, K::Ref))
            }
            ("OpGetDbRef", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_dbref({}, {fld})", as_ref(r)?),
                    K::Ref,
                ))
            }
            ("OpSetDbRef", [r, fld, x]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, x) = self.two(name, &[r, x], ind)?;
                Ok(E::new(
                    format!("loft_set_dbref({}, {fld}, {})", as_ref(r)?, as_ref(x)?),
                    K::Void,
                ))
            }
            // The closure record releases what it captured, running the captured values'
            // drop hooks (`State::drop_fn_ref`).
            ("OpDropFnRef", [Value::Var(f)]) if self.shapes[*f as usize] == CType::Fn => Ok(
                E::new(format!("loft_drop_closure({})", self.var(*f)), K::Void),
            ),
            ("OpPutRef", [Value::Var(v), x]) => {
                let x = self.expr(x, ind)?;
                Ok(E::new(
                    format!("{} = {}", self.var(*v), as_ref(x)?),
                    K::Void,
                ))
            }
            // A reference equality: two absent references are equal, an absent one equals no
            // present one, two present ones when they name the same place.
            ("OpEqContent" | "OpNeContent", [_, _, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.eq_type(tp)?;
                let (a, b) = self.two(name, &args[..2], ind)?;
                let neg = if name == "OpNeContent" { "!" } else { "" };
                Ok(E::new(
                    format!(
                        "{neg}loft_eq_content({}, {}, {})",
                        super::Cx::eq_fn(tp),
                        as_ref(a)?,
                        as_ref(b)?
                    ),
                    K::Cond,
                ))
            }
            ("OpPlaceRecord", [host, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                let size = self.cx.stores.enum_parent_size(tp);
                let (image, n) = self.cx.default_image(tp);
                // `@FR-H-RootExtent`: a record that is not an element of the host store's root
                // vector is noted, so a clear does not reset the store under it.  The host's
                // store is minted as the host value's type, so the Rust store answers here.
                let foreign = self
                    .value_type(host)
                    .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
                    .and_then(|kt| self.cx.stores.root_vector_element_of(kt))
                    .is_some_and(|elem| elem != tp);
                let host = self.expr(host, ind)?;
                Ok(E::new(
                    format!(
                        "loft_place_record({}, {tp}, {size}, {image}, {n}, {})",
                        as_ref(host)?,
                        u8::from(foreign)
                    ),
                    K::Ref,
                ))
            }
            ("OpEvacuate", [holder, fld, home, tp]) => self.evacuate(holder, fld, home, tp, ind),
            ("OpFreeRecordIn", [r, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.copy_type(tp)?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_free_record_in(loft_types, {tp}, {})", as_ref(r)?),
                    K::Void,
                ))
            }
            ("OpStoreLive", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_store_live({})", as_ref(r)?), K::Cond))
            }
            ("OpDistinctStore", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("({}.store_nr != {}.store_nr)", as_ref(a)?, as_ref(b)?),
                    K::Cond,
                ))
            }
            _ => self.field_operator(name, args, ind),
        }
    }

    /// The enum and text fields of a record.
    fn field_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetEnum", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_enum({}, {fld})", as_ref(r)?),
                    K::Enum,
                ))
            }
            ("OpSetEnum", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[r, val], ind)?;
                Ok(E::new(
                    format!("loft_set_byte({}, {fld}, {})", as_ref(r)?, as_enum(val)?),
                    K::Void,
                ))
            }
            ("OpGetText", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let view = std::mem::take(&mut self.field_view);
                let r = self.expr(r, ind)?;
                if view {
                    // The record is named before the read: a call computing it runs here, in
                    // program order, and the view itself reads nothing but the store.
                    let code = as_ref(r)?;
                    let at = if code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                        code
                    } else {
                        let t = self.temp(CType::Ref);
                        self.line(ind, &format!("{t} = {code};"));
                        t
                    };
                    return Ok(E::new(format!("loft_get_text({at}, {fld})"), K::View));
                }
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(
                    ind,
                    &format!("{t} = loft_text_own(loft_get_text({}, {fld}));", as_ref(r)?),
                );
                Ok(self.held(t, CType::Text))
            }
            ("OpSetText" | "OpSetTextReplace", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[r, val], ind)?;
                let f = if name == "OpSetText" {
                    "loft_set_text"
                } else {
                    "loft_refill_text"
                };
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, as_view(val)?),
                    K::Void,
                ))
            }
            ("OpGetFloat" | "OpGetSingle", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let (f, k) = if name == "OpGetFloat" {
                    ("loft_get_float", K::Float)
                } else {
                    ("loft_get_single", K::Single)
                };
                Ok(E::new(format!("{f}({}, {fld})", as_ref(r)?), k))
            }
            ("OpSetFloat" | "OpSetSingle", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[r, val], ind)?;
                let (f, v) = if name == "OpSetFloat" {
                    ("loft_set_float", as_float(val)?)
                } else {
                    ("loft_set_single", as_single(val)?)
                };
                Ok(E::new(format!("{f}({}, {fld}, {v})", as_ref(r)?), K::Void))
            }
            ("OpPushBoolean", [v, val]) => {
                let (v, val) = self.two(name, &[v, val], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_byte({}, {})", as_ref(v)?, as_bool(val)?),
                    K::Void,
                ))
            }
            ("OpPushFloat" | "OpPushSingle", [v, val]) => {
                let (v, val) = self.two(name, &[v, val], ind)?;
                let (f, x) = if name == "OpPushFloat" {
                    ("loft_vec_push_float", as_float(val)?)
                } else {
                    ("loft_vec_push_single", as_single(val)?)
                };
                Ok(E::new(format!("{f}({}, {x})", as_ref(v)?), K::Void))
            }
            _ => self.narrow_operator(name, args, ind),
        }
    }

    /// The narrow integer fields: bytes, shorts and 4-byte integers, each in its encoding.
    fn narrow_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            (
                "OpGetByte" | "OpGetByteNullable" | "OpGetShort" | "OpGetShortRaw"
                | "OpGetShortFull",
                [r, fld, min],
            ) => {
                let fld = Self::literal(fld, "field")?;
                let min = Self::literal(min, "minimum")?;
                let f = match name {
                    "OpGetByte" => "loft_get_byte",
                    "OpGetByteNullable" => "loft_get_byte_nullable",
                    "OpGetShort" => "loft_get_short",
                    _ => "loft_get_short_raw",
                };
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, c_long(min)),
                    K::Int,
                ))
            }
            (
                "OpSetByte" | "OpSetByteNullable" | "OpSetShort" | "OpSetShortRaw",
                [r, fld, min, val],
            ) => {
                let fld = Self::literal(fld, "field")?;
                let min = Self::literal(min, "minimum")?;
                let f = match name {
                    "OpSetByte" => "loft_set_byte_v",
                    "OpSetByteNullable" => "loft_set_byte_nullable",
                    "OpSetShort" => "loft_set_short",
                    _ => "loft_set_short_raw",
                };
                let (r, val) = self.two(name, &[r, val], ind)?;
                Ok(E::new(
                    format!(
                        "{f}({}, {fld}, {}, {})",
                        as_ref(r)?,
                        c_long(min),
                        as_int(val)?
                    ),
                    K::Void,
                ))
            }
            ("OpGetInt4" | "OpGetInt4Raw" | "OpGetInt4Full", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let f = match name {
                    "OpGetInt4" => "loft_get_int4",
                    "OpGetInt4Raw" => "loft_get_int4_raw",
                    _ => "loft_get_int4_full",
                };
                Ok(E::new(format!("{f}({}, {fld})", as_ref(r)?), K::Int))
            }
            ("OpSetInt4Raw", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[r, val], ind)?;
                Ok(E::new(
                    format!("loft_set_int4_raw({}, {fld}, {})", as_ref(r)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpPushInt4", [v, val]) => {
                let (v, val) = self.two(name, &[v, val], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_int4({}, {})", as_ref(v)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpSizeVector", [v, stride]) => {
                let stride = Self::literal(stride, "stride")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("(loft_vec_len({}) * {stride})", as_ref(v)?),
                    K::Int,
                ))
            }
            ("OpFnRefClosure", [Value::Var(f)]) if self.shapes[*f as usize] == CType::Fn => {
                Ok(E::new(format!("{}.clos", self.var(*f)), K::Ref))
            }
            ("OpInitRefSentinel", [Value::Var(v)]) if self.shapes[*v as usize] == CType::Ref => Ok(
                E::new(format!("{} = loft_ref_null()", self.var(*v)), K::Void),
            ),
            _ => self.vector_operator(name, args, ind),
        }
    }

    /// The collection `OpNewRecord`/`OpFinishRecord` add to and its element type: the parent
    /// itself when `fld` is `u16::MAX`, else the parent's field.  A vector, any record
    /// collection `Cx::keyed` names, or a member of a linked group (`@FR-Col-Group`), whose
    /// record enters every member; any other collection is refused.
    pub(super) fn record_collection(
        &mut self,
        data: &Value,
        ptp: &Value,
        fld: &Value,
        ind: usize,
    ) -> Result<(String, u16, Coll), String> {
        use crate::database::Parts;
        let stores = self.cx.stores;
        let ptp = u16::try_from(Self::literal(ptp, "type")?)
            .map_err(|_| "a type out of range".to_string())?;
        let fld = u16::try_from(Self::literal(fld, "field")?)
            .map_err(|_| "a field out of range".to_string())?;
        let data = self.expr(data, ind)?;
        let data = as_ref(data)?;
        if fld == u16::MAX {
            if let Parts::Vector(c) = stores.types[ptp as usize].parts {
                return Ok((data, c, Coll::Vector));
            }
            let Some((_, c)) = self.cx.keyed(ptp) else {
                return Err("a new record in a collection other than a vector".into());
            };
            return Ok((data, c, Coll::Keyed(ptp)));
        }
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &stores.types[ptp as usize].parts
        else {
            return Err("a new record in a field of a non-struct".into());
        };
        let vtp = stores.field_type(ptp, fld);
        let pos = stores.field_position(ptp, fld);
        let others: Vec<u16> = fields
            .get(fld as usize)
            .map(|f| {
                f.other_indexes()
                    .iter()
                    .copied()
                    .filter(|m| *m != u16::MAX)
                    .collect()
            })
            .unwrap_or_default();
        if !others.is_empty() {
            // The parent is named twice — the primary's place and every sibling's — so it is
            // read once.
            let parent = self.temp(CType::Ref);
            self.line(ind, &format!("{parent} = {data};"));
            let mut members = vec![(pos, vtp)];
            for m in others {
                members.push((stores.field_position(ptp, m), stores.field_type(ptp, m)));
            }
            let Some((_, c)) = self.cx.keyed(vtp) else {
                return Err(
                    "a new record in a linked group member that is not a collection".into(),
                );
            };
            return Ok((
                format!("loft_field({parent}, {pos})"),
                c,
                Coll::Group(parent, members),
            ));
        }
        let place = format!("loft_field({data}, {pos})");
        if let Some(Parts::Vector(c)) = stores.types.get(vtp as usize).map(|t| &t.parts) {
            return Ok((place, *c, Coll::Vector));
        }
        let Some((_, c)) = self.cx.keyed(vtp) else {
            return Err("a new record in a field that is not a vector".into());
        };
        Ok((place, c, Coll::Keyed(vtp)))
    }

    /// The operators over vectors.
    fn vector_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpNewRecord", [data, ptp, fld]) => {
                let (v, elem, coll) = self.record_collection(data, ptp, fld, ind)?;
                match coll {
                    Coll::Keyed(tp) => return self.keyed_new(&v, tp, ind),
                    Coll::Group(_, members) => return self.keyed_new(&v, members[0].1, ind),
                    Coll::Vector => {}
                }
                let size = self.cx.stores.size(elem);
                let t = self.temp(CType::Ref);
                self.line(ind, &format!("{t} = loft_vec_append({v}, {size});"));
                // `@FR-R-FieldAppend`: the plain vector's own append, its field resolved here;
                // a field number of `u16::MAX` is the bare clause (`v += [x]`).
                let bare =
                    matches!(fld.unspan(), Value::Int(f) if u16::try_from(*f) == Ok(u16::MAX));
                crate::rewrite_census::fired(
                    if bare {
                        "R-FieldAppend/bare"
                    } else {
                        "R-FieldAppend"
                    },
                    1,
                );
                let ptp = u16::try_from(Self::literal(ptp, "type")?)
                    .map_err(|_| "a type number out of range")?;
                if !self.mint_complete(ptp) {
                    let (image, n) = self.cx.default_image(elem);
                    self.line(ind, &format!("loft_fill({t}, {image}, {n});"));
                }
                Ok(E::new(t, K::Ref))
            }
            ("OpFinishRecord", [data, rec, ptp, fld]) => {
                let (v, _, coll) = self.record_collection(data, ptp, fld, ind)?;
                match coll {
                    Coll::Keyed(tp) => {
                        let rec = self.expr(rec, ind)?;
                        let rec = as_ref(rec)?;
                        return self.keyed_finish(&v, &rec, tp);
                    }
                    Coll::Group(parent, members) => {
                        let rec = self.expr(rec, ind)?;
                        let rec = as_ref(rec)?;
                        return self.group_finish(&parent, &rec, &members);
                    }
                    Coll::Vector => {}
                }
                Ok(E::new(format!("loft_vec_finish({v})"), K::Void))
            }
            ("OpClearVector", [v]) if self.in_delivery > 0 && self.adopted_buffer(v) => {
                Ok(E::new("((void) 0)", K::Void))
            }
            // `@FR-R-AppendTwin` — in a twin a clear of the buffer, or of a view of it, would
            // empty the caller's elements: eligibility admitted it only before the body's first
            // write, where it clears nothing the twin's own result needs.
            ("OpClearVector", [Value::Var(x)]) if self.ap_views.contains(x) => {
                Ok(E::new("((void) 0)", K::Void))
            }
            ("OpClearVector", [v]) => {
                self.cx.name_types();
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_clear_release(loft_types, loft_ntypes, {})",
                        as_ref(v)?
                    ),
                    K::Void,
                ))
            }
            ("OpLengthVector", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_len({})", as_ref(v)?), K::Int))
            }
            ("OpPreAllocVector", [v, count, size]) => {
                let count = Self::literal(count, "count")?;
                let size = Self::literal(size, "element size")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("loft_vec_prealloc({}, {count}, {size})", as_ref(v)?),
                    K::Void,
                ))
            }
            ("OpPushEnum", [v, val]) => {
                let (v, val) = self.two(name, &[v, val], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_byte({}, {})", as_ref(v)?, as_enum(val)?),
                    K::Void,
                ))
            }
            ("OpPushInt", [v, val]) => {
                let (v, val) = self.two(name, &[v, val], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_int({}, {})", as_ref(v)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpGetVector" | "OpGetVectorNullable", [v, size, idx]) => {
                let size = Self::literal(size, "element size")?;
                let (v, idx) = self.two(name, &[v, idx], ind)?;
                Ok(E::new(
                    format!("loft_vec_get({}, {size}, {})", as_ref(v)?, as_int(idx)?),
                    K::Ref,
                ))
            }
            _ => self.collection_operator(name, args, ind),
        }
    }

    /// The character operators: a code point, 0 the null character.
    fn char_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConvCharacterFromInt", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_char_of({})", as_int(v)?), K::Char))
            }
            ("OpConvIntFromCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("loft_char_int({})", as_char(c)?), K::Int))
            }
            ("OpConvBoolFromCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("({} != 0)", as_char(c)?), K::Cond))
            }
            ("OpLengthCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("loft_char_len({})", as_char(c)?), K::Int))
            }
            ("OpTextCharacter" | "OpTextCharacterNullable", [_, _]) => {
                let (t, i) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_text_char({}, {})", as_view(t)?, as_int(i)?),
                    K::Char,
                ))
            }
            ("OpAppendCharacter" | "OpAppendStackCharacter", [dest, c]) => {
                let dest = self.text_dest(dest)?;
                let c = self.expr(c, ind)?;
                Ok(E::new(
                    format!("loft_push_code(&{dest}, {})", as_char(c)?),
                    K::Void,
                ))
            }
            ("OpGetCharacter", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_char({}, {fld})", as_ref(r)?),
                    K::Char,
                ))
            }
            ("OpSetCharacter", [r, fld, c]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, c) = self.two(name, &[r, c], ind)?;
                Ok(E::new(
                    format!("loft_set_char({}, {fld}, {})", as_ref(r)?, as_char(c)?),
                    K::Void,
                ))
            }
            ("OpPushCharacter", [v, c]) => {
                let (v, c) = self.two(name, &[v, c], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_char({}, {})", as_ref(v)?, as_char(c)?),
                    K::Void,
                ))
            }
            _ => self.misc_operator(name, args, ind),
        }
    }

    /// `r += src[lo..hi]` of scalar elements: `OpSliceVector` through `loft_vec_slice`, and
    /// `OpSliceView` through `loft_vec_slice_view`, which views a foreign source instead.
    fn vec_slice(&mut self, args: &[Value], ind: usize, func: &str) -> Result<E, String> {
        let [v, src, lo, hi, tp] = args else {
            return Err("a slice with an unexpected operand list".into());
        };
        let tp = u16::try_from(Self::literal(tp, "element type")?)
            .map_err(|_| "an element type out of range")?;
        // The parser emits these for scalar elements only; anything else is refused.
        let scalar = (tp <= 6 && tp != 5)
            || matches!(
                self.cx.stores.types.get(tp as usize).map(|t| &t.parts),
                Some(
                    crate::database::Parts::Byte(..)
                        | crate::database::Parts::Short(..)
                        | crate::database::Parts::ShortRaw(..)
                        | crate::database::Parts::Int(..)
                        | crate::database::Parts::IntRaw(..)
                )
            );
        if !scalar {
            return Err("a slice of elements that are not scalars".into());
        }
        let size = self.cx.stores.size(tp);
        let es = self.args(&[v, src, lo, hi], ind)?;
        let [v, src, lo, hi] =
            <[E; 4]>::try_from(es).map_err(|_| "a slice without four operands".to_string())?;
        Ok(E::new(
            format!(
                "{func}({}, {}, {}, {}, {size})",
                as_ref(v)?,
                as_ref(src)?,
                as_int(lo)?,
                as_int(hi)?
            ),
            K::Void,
        ))
    }

    /// `OpRemoveVector` of a linked element type: the slot unlinked, the record released.
    fn remove_linked(&mut self, tp: u16, v: &Value, idx: &Value, ind: usize) -> Result<E, String> {
        self.cx.name_types();
        self.cx.describe_type(tp);
        let (v, idx) = self.two("OpRemoveVector", &[v, idx], ind)?;
        Ok(E::new(
            format!(
                "loft_vec_remove_linked(loft_types, {tp}, {}, {})",
                as_ref(v)?,
                as_int(idx)?
            ),
            K::Cond,
        ))
    }

    /// The whole-vector operators: append, replace, remove, repeat — each with the deep
    /// copies its store method makes.
    /// OpAdoptVector (`Stores::vector_adopt`): a local takes a native call's vector answer.
    fn adopt_vector(
        &mut self,
        v: &Value,
        other: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let name = "OpAdoptVector";
        let tp = u16::try_from(Self::literal(tp, "element type")?)
            .map_err(|_| "an element type out of range")?;
        if self.cx.stores.is_linked(tp) {
            return Err("an adopted vector of linked records".into());
        }
        self.cx.copy_type(tp)?;
        let (v, other) = self.two(name, &[v, other], ind)?;
        Ok(E::new(
            format!(
                "loft_vec_adopt(loft_types, {tp}, {}, {})",
                as_ref(v)?,
                as_ref(other)?
            ),
            K::Void,
        ))
    }

    fn collection_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpAdoptVector", [v, other, tp]) => self.adopt_vector(v, other, tp, ind),
            ("OpAppendVector", [v, ..]) if self.in_delivery > 0 && self.adopted_buffer(v) => {
                Ok(E::new("((void) 0)", K::Void))
            }
            ("OpPrependVector" | "OpAppendVector", [v, other, tp]) => {
                self.append_vector_op(name, v, other, tp, ind)
            }
            ("OpReplaceVector", [v, other, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                let f = if self.cx.stores.is_linked(tp) {
                    "loft_vec_replace_array"
                } else {
                    "loft_vec_replace"
                };
                self.cx.copy_type(tp)?;
                let (v, other) = self.two(name, &[v, other], ind)?;
                // The plain vector's clear releases through the type table (`loft_ntypes`).
                let nt = if f == "loft_vec_replace" {
                    "loft_ntypes, "
                } else {
                    ""
                };
                Ok(E::new(
                    format!(
                        "{f}(loft_types, {nt}{tp}, {}, {})",
                        as_ref(v)?,
                        as_ref(other)?
                    ),
                    K::Void,
                ))
            }
            ("OpSliceVector", [_, _, _, _, _]) => self.vec_slice(args, ind, "loft_vec_slice"),
            ("OpSliceView", [_, _, _, _, _]) => self.vec_slice(args, ind, "loft_vec_slice_view"),
            ("OpRemoveVector", [v, tp, idx]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                if self.cx.stores.is_linked(tp) {
                    return self.remove_linked(tp, v, idx, ind);
                }
                let size = self.cx.stores.size(tp);
                let (v, idx) = self.two(name, &[v, idx], ind)?;
                // An element owning heap releases it first, as `remove_vector_at` does.
                if self.cx.stores.type_owns_heap(tp) {
                    self.cx.name_types();
                    self.cx.describe_type(tp);
                    return Ok(E::new(
                        format!(
                            "loft_vec_remove_at(loft_types, {tp}, {}, {})",
                            as_ref(v)?,
                            as_int(idx)?
                        ),
                        K::Cond,
                    ));
                }
                Ok(E::new(
                    format!("loft_vec_remove({}, {size}, {})", as_ref(v)?, as_int(idx)?),
                    K::Cond,
                ))
            }
            ("OpAppendCopy", [v, count, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "vector type")?)
                    .map_err(|_| "a vector type out of range")?;
                let ctp = self.cx.stores.content(tp);
                if self.cx.stores.is_linked(ctp) {
                    return Err("a repeat of a linked element type".into());
                }
                self.cx.copy_type(ctp)?;
                let (v, count) = self.two(name, &[v, count], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_append_copy(loft_types, {ctp}, {}, {})",
                        as_ref(v)?,
                        as_int(count)?
                    ),
                    K::Void,
                ))
            }
            _ => self.vector_move_operator(name, args, ind),
        }
    }

    /// A local vector given up to a field, and a vector cut to a range.
    fn vector_move_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        match (name, args) {
            ("OpMoveVector", [dst, src, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                let f = if self.cx.stores.is_linked(tp) {
                    "loft_move_vector_array"
                } else {
                    "loft_move_vector"
                };
                self.cx.copy_type(tp)?;
                let (dst, src) = self.two(name, &[dst, src], ind)?;
                Ok(E::new(
                    format!("{f}(loft_types, {tp}, {}, {})", as_ref(dst)?, as_ref(src)?),
                    K::Void,
                ))
            }
            ("OpKeepVectorRange", [v, tp, lo, hi]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                // A linked element's slot is its record number, its record removed when dropped.
                let size = if self.cx.stores.is_linked(tp) {
                    0
                } else {
                    self.cx.stores.size(tp)
                };
                // What a dropped element owns is released through its type's descriptor.
                self.cx.copy_type(tp)?;
                // `@FR-R-Refresh`'s keep-range clause: a push header on the path takes the
                // clamped length (and the record the copy form may give it) at the op's own site.
                let held = crate::opt::writes::vector_path(self.cx.data, v)
                    .and_then(|p| self.push_head(&p));
                let es = self.args(&[v, lo, hi], ind)?;
                let [v, lo, hi] = <[E; 3]>::try_from(es)
                    .map_err(|_| "a range without three operands".to_string())?;
                let keep = format!(
                    "loft_keep_vector_range(loft_types, {tp}, {}, {size}, {}, {})",
                    as_ref(v)?,
                    as_int(lo)?,
                    as_int(hi)?
                );
                Ok(E::new(
                    match held {
                        Some((h, hs)) => {
                            format!("((void) ({keep}, {h} = loft_ph_of({h}.v, {hs})))")
                        }
                        None => keep,
                    },
                    K::Void,
                ))
            }
            _ => self.slot_operator(name, args, ind),
        }
    }

    /// Booleans and bytes in records and vectors, absent vectors, store identity, text
    /// clears and the text null.
    /// `OpClear(r, tp)` (`Stores::remove_claims`): the value's claims released, or with
    /// `@FR-R-Reuse`'s reset flag its pooled store reset where `reset_pool_root` allows.
    fn clear_op(&mut self, r: &Value, tp: &Value, ind: usize) -> Result<E, String> {
        let raw = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let tp = raw & crate::keys::POOL_TP_MASK;
        self.cx.copy_type(tp)?;
        let r = self.expr(r, ind)?;
        let r = as_ref(r)?;
        let walk = format!("loft_remove_claims(loft_types, {tp}, {r})");
        if raw & crate::keys::POOL_RESET == 0 {
            return Ok(E::new(walk, K::Void));
        }
        let size = self.cx.stores.enum_parent_size(tp);
        let (image, n) = self.cx.default_image(tp);
        Ok(E::new(
            format!("if (!loft_reset_pool_root({r}, {tp}, {size}, {image}, {n})) {walk}"),
            K::Void,
        ))
    }

    fn slot_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetBoolean", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_bool({}, {fld})", as_ref(r)?),
                    K::Bool,
                ))
            }
            ("OpSetBoolean", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[r, val], ind)?;
                Ok(E::new(
                    format!("loft_set_byte({}, {fld}, {})", as_ref(r)?, as_bool(val)?),
                    K::Void,
                ))
            }
            ("OpPushByte", [v, min, val]) => {
                let min = Self::literal(min, "byte minimum")?;
                let (v, val) = self.two(name, &[v, val], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_push_byte({}, loft_byte_raw({min}, {}))",
                        as_ref(v)?,
                        as_int(val)?
                    ),
                    K::Void,
                ))
            }
            ("OpVectorIsNull", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_absent({})", as_ref(v)?), K::Cond))
            }
            ("OpLengthSorted", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_len({})", as_ref(v)?), K::Int))
            }
            ("OpEqRef" | "OpNeRef", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                let neg = if name == "OpNeRef" { "!" } else { "" };
                Ok(E::new(
                    format!("{neg}loft_ref_eq({}, {})", as_ref(a)?, as_ref(b)?),
                    K::Cond,
                ))
            }
            ("OpClear", [r, tp]) => self.clear_op(r, tp, ind),
            ("OpClearText", [dest]) => {
                let dest = self.text_dest(dest)?;
                Ok(E::new(format!("{dest}.len = 0"), K::Void))
            }
            ("OpConvTextFromNull", []) => Ok(E::new("loft_null_text()", K::View)),
            ("OpConvBoolFromNull", []) => Ok(E::new("LOFT_BOOL_NULL", K::Bool)),
            ("OpCastIntFromText" | "OpCastLongFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(format!("loft_int_of_text({})", as_view(t)?), K::Int))
            }
            ("OpGetTextSub", [_, _, _]) => {
                let es = self.args(args, ind)?;
                let [t, from, till] = <[E; 3]>::try_from(es)
                    .map_err(|_| "OpGetTextSub without three operands".to_string())?;
                Ok(E::new(
                    format!(
                        "loft_text_sub({}, {}, {})",
                        as_view(t)?,
                        as_int(from)?,
                        as_int(till)?
                    ),
                    K::View,
                ))
            }
            ("OpConvCharacterFromNull", []) => Ok(E::new("((uint32_t) 0)", K::Char)),
            _ if name.contains("Character") => self.char_operator(name, args, ind),
            ("OpCastTextFromBool", [b]) => {
                let b = self.expr(b, ind)?;
                Ok(E::new(format!("loft_bool_text({})", as_bool(b)?), K::View))
            }
            ("OpConvBoolFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(
                    format!("(!loft_str_is_null({}))", as_view(t)?),
                    K::Cond,
                ))
            }
            // The chain ends in `misc_operator`, which holds the last operators and the refusal.
            _ => self.misc_operator(name, args, ind),
        }
    }
}

/// `a op b` for two integers or booleans.  A variable compared with itself has a fixed answer
/// (a null equals itself, as in loft), and C compilers reject the tautology under `-Werror`.
fn compare(a: &str, op: &str, b: &str) -> E {
    if a == b && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        let same = matches!(op, "==" | "<=" | ">=");
        return E::new(if same { "1" } else { "0" }, K::Cond);
    }
    E::new(format!("({a} {op} {b})"), K::Cond)
}

/// `n` as a C `int64_t` constant: the null as `LOFT_NULL`, since `INT64_C` cannot spell it.
fn c_long(n: i64) -> String {
    if n == i64::MIN {
        "LOFT_NULL".into()
    } else {
        format!("INT64_C({n})")
    }
}

fn as_int(e: E) -> Result<String, String> {
    match e.kind {
        K::Int => Ok(e.code),
        // @PLN204 — a 32-bit integer read as a 64-bit one: its null stays null.
        K::Int32 => Ok(format!("loft_w64({})", e.code)),
        // The slot the bytecode writes is the integer's: a character's code point, a
        // boolean's byte, a function value's definition number (`FnRefDnr`).
        K::Char | K::Bool => Ok(format!("((int64_t) ({}))", e.code)),
        K::Fn => Ok(format!("(({}).d)", e.code)),
        k => Err(format!("an integer expected, a {k:?} found")),
    }
}

fn as_cond(e: E) -> Result<String, String> {
    match e.kind {
        K::Cond => Ok(e.code),
        K::Bool => Ok(format!("({} == 1)", e.code)),
        k => Err(format!("a condition expected, a {k:?} found")),
    }
}

fn as_bool(e: E) -> Result<String, String> {
    match e.kind {
        K::Bool => Ok(e.code),
        K::Cond => Ok(format!("(uint8_t) ({})", bare(&e.code))),
        k => Err(format!("a boolean expected, a {k:?} found")),
    }
}

fn as_ref(e: E) -> Result<String, String> {
    match e.kind {
        K::Ref => Ok(e.code),
        k => Err(format!("a reference expected, a {k:?} found")),
    }
}

fn as_tuple(e: E, id: u32) -> Result<String, String> {
    match e.kind {
        K::Tup(i) if i == id => Ok(e.code),
        k => Err(format!("a tuple expected, a {k:?} found")),
    }
}

fn as_enum(e: E) -> Result<String, String> {
    match e.kind {
        K::Enum => Ok(e.code),
        k => Err(format!("an enum value expected, a {k:?} found")),
    }
}

fn as_float(e: E) -> Result<String, String> {
    match e.kind {
        K::Float => Ok(e.code),
        k => Err(format!("a float expected, a {k:?} found")),
    }
}

fn as_single(e: E) -> Result<String, String> {
    match e.kind {
        K::Single => Ok(e.code),
        k => Err(format!("a single expected, a {k:?} found")),
    }
}

fn as_char(e: E) -> Result<String, String> {
    match e.kind {
        K::Char => Ok(e.code),
        // An integer written as a character: its low 32 bits, as the store keeps them (an
        // integer null is the character null, 0).
        K::Int => Ok(format!("((uint32_t) ({}))", e.code)),
        // `INT32_MIN`, the 32-bit null, is the character null too: 0.
        K::Int32 => Ok(format!("((uint32_t) loft_w64({}))", e.code)),
        k => Err(format!("a character expected, a {k:?} found")),
    }
}

fn as_fn(e: E) -> Result<String, String> {
    match e.kind {
        K::Fn => Ok(e.code),
        k => Err(format!("a function value expected, a {k:?} found")),
    }
}

fn as_view(e: E) -> Result<String, String> {
    match e.kind {
        K::View => Ok(e.code),
        k => Err(format!("a text expected, a {k:?} found")),
    }
}

/// The value of `e` converted to shape `c`, for a parameter or a store.
/// @PLN204 — `e` as a 32-bit integer: a 64-bit value that does not fit reads null, as the
/// interpreter's `OpNarrow32` answers it.
fn as_int32(e: E) -> Result<String, String> {
    match e.kind {
        K::Int32 => Ok(e.code),
        _ => Ok(format!("loft_n32({})", as_int(e)?)),
    }
}

fn as_shape(e: E, c: CType) -> Result<String, String> {
    match c {
        CType::Int => as_int(e),
        CType::Int32 => as_int32(e),
        CType::Bool => as_bool(e),
        CType::Text => as_view(e),
        CType::Ref => as_ref(e),
        CType::Tuple(id) => as_tuple(e, id),
        CType::Enum => as_enum(e),
        CType::Float => as_float(e),
        CType::Single => as_single(e),
        CType::Char => as_char(e),
        CType::Fn => as_fn(e),
        CType::Void => Err("a void value".into()),
    }
}

/// The operators of each C shape, for the operators this backend writes.
const OP_INT: &[&str] = &[
    "OpAddInt",
    "OpAddLong",
    "OpMinInt",
    "OpMinLong",
    "OpMulInt",
    "OpMulLong",
    "OpDivInt",
    "OpDivLong",
    "OpRemInt",
    "OpRemLong",
    "OpGetInt",
    "OpLengthVector",
    "OpLengthText",
    "OpSizeText",
    "OpMinSingleInt",
    "OpBitNotSingleInt",
    "OpSizeStruct",
    "OpSizeHash",
    "OpMinSingleLong",
    "OpConvIntFromNull",
    "OpConvLongFromNull",
    "OpConvIntFromEnum",
    "OpLengthSorted",
    "OpRangeDefault",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetShortFull",
    "OpGetInt4",
    "OpLandInt",
    "OpLandLong",
    "OpLorInt",
    "OpLorLong",
    "OpEorInt",
    "OpEorLong",
    "OpSLeftInt",
    "OpSLeftLong",
    "OpSRightInt",
    "OpSRightLong",
    "OpConvIntFromCharacter",
    "OpLengthCharacter",
    "OpAddIntNullable",
    "OpMinIntNullable",
    "OpMulIntNullable",
    "OpDivIntNullable",
    "OpRemIntNullable",
    "OpCastIntFromText",
    "OpCastLongFromText",
];
const OP_BOOL: &[&str] = &[
    "OpEqInt",
    "OpEqLong",
    "OpNeInt",
    "OpNeLong",
    "OpLtInt",
    "OpLtLong",
    "OpLeInt",
    "OpLeLong",
    "OpEqBool",
    "OpNeBool",
    "OpNot",
    "OpEqText",
    "OpNeText",
    "OpRefIsNull",
    "OpConvBoolFromInt",
    "OpConvBoolFromLong",
    "OpConvBoolFromEnum",
    "OpConvBoolFromRef",
    "OpGetBoolean",
    "OpVectorIsNull",
    "OpDistinctStore",
    "OpConvBoolFromText",
    "OpConvBoolFromCharacter",
    "OpConvBoolFromNull",
    "OpRemoveVector",
    "OpStoreLive",
    "OpEqContent",
    "OpNeContent",
    "OpEqRef",
    "OpNeRef",
];
const OP_REF: &[&str] = &[
    "OpPlaceRecord",
    "OpConstRef",
    "OpGetDbRef",
    "OpGetField",
    "OpGetVector",
    "OpGetVectorNullable",
    "OpNullRefSentinel",
    "OpInsertVector",
    "OpRefFromChildRec",
    "OpChildRec",
    "OpCoroutineRetain",
];
const OP_ENUM: &[&str] = &[
    "OpGetEnum",
    "OpCastEnumFromInt",
    "OpConvEnumFromNull",
    "OpConstEnum",
];
const OP_TEXT: &[&str] = &[
    "OpGetText",
    "OpCastTextFromEnum",
    "OpCastEnumFromText",
    "OpCastVectorFromText",
    "OpConvTextFromNull",
    "OpCastTextFromBool",
    "OpGetTextSub",
];
const OP_FLOAT: &[&str] = &[
    "OpAddFloat",
    "OpMinFloat",
    "OpMulFloat",
    "OpDivFloat",
    "OpDivFloatNullable",
    "OpRemFloat",
    "OpRemFloatNullable",
    "OpPowFloat",
    "OpMinSingleFloat",
    "OpAbsFloat",
    "OpConvFloatFromNull",
    "OpConvFloatFromInt",
    "OpConvFloatFromLong",
    "OpConvFloatFromSingle",
    "OpMathPiFloat",
    "OpMathEFloat",
    "OpMathFuncFloat",
    "OpMathFunc2Float",
    "OpGetFloat",
];
const OP_SINGLE: &[&str] = &[
    "OpAddSingle",
    "OpMinSingle",
    "OpMulSingle",
    "OpDivSingle",
    "OpDivSingleNullable",
    "OpRemSingle",
    "OpRemSingleNullable",
    "OpPowSingle",
    "OpMinSingleSingle",
    "OpAbsSingle",
    "OpConvSingleFromNull",
    "OpConvSingleFromInt",
    "OpCastSingleFromFloat",
    "OpMathFuncSingle",
    "OpMathFunc2Single",
    "OpGetSingle",
];
const OP_FCMP: &[&str] = &[
    "OpEqFloat",
    "OpNeFloat",
    "OpLtFloat",
    "OpLeFloat",
    "OpEqSingle",
    "OpNeSingle",
    "OpLtSingle",
    "OpLeSingle",
    "OpConvBoolFromFloat",
    "OpConvBoolFromSingle",
];
const OP_FINT: &[&str] = &["OpCastIntFromFloat", "OpCastIntFromSingle"];
const OP_CHAR: &[&str] = &[
    "OpConvCharacterFromInt",
    "OpConvCharacterFromNull",
    "OpTextCharacter",
    "OpTextCharacterNullable",
    "OpGetCharacter",
];

/// The C shape an operator's value is held in, for the operators this backend writes.
fn op_shape(name: &str) -> Option<CType> {
    // @PLN204 — `OpNarrow32` answers an `int32_t` (`FnWriter::narrow32`).
    if name == "OpNarrow32" {
        return Some(if super::no_c_int32() {
            CType::Int
        } else {
            CType::Int32
        });
    }
    let shapes = [
        (OP_INT, CType::Int),
        (OP_BOOL, CType::Bool),
        (OP_REF, CType::Ref),
        (OP_ENUM, CType::Enum),
        (OP_TEXT, CType::Text),
        (OP_FLOAT, CType::Float),
        (OP_SINGLE, CType::Single),
        (OP_FCMP, CType::Bool),
        (OP_FINT, CType::Int),
        (OP_CHAR, CType::Char),
    ];
    shapes
        .iter()
        .find(|(names, _)| names.contains(&name))
        .map(|(_, c)| *c)
}

/// `s` as a C string literal: printable ASCII as itself, every other byte as an octal escape
/// (three digits, so the next character can never extend it), `?` escaped against trigraphs.
pub(crate) fn c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'?' => out.push_str("\\?"),
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push('"');
    out
}

/// `c` without one pair of outer parentheses that enclose all of it: `if ((a == b))` is a
/// warning in every C compiler that reads it as a mistyped assignment.
fn bare(c: &str) -> &str {
    let Some(inner) = c.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return c;
    };
    let mut depth = 0i32;
    for ch in inner.chars() {
        depth += match ch {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            return c;
        }
    }
    if depth == 0 { inner } else { c }
}

fn unsupported(v: &Value) -> String {
    let kind = format!("{v:?}");
    let kind = kind.split(['(', ' ', '{']).next().unwrap_or("value");
    format!("the construct {kind}")
}

/// A kind a store link carries: the place's own get and set read and write it
/// (`FnWriter::link_access`).
fn link_kind(tp: &crate::data::Type) -> bool {
    use crate::data::Type;
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Character
            | Type::Single
            | Type::Float
            | Type::Boolean
            | Type::Enum(_, false, _)
            | Type::Text(_)
    )
}

/// The `&` locals of `def` that link a store place: every bind (`link_set_repoints`) names an
/// element or a field, or another store link (`OpVarRef`), none a variable (`OpCreateStack`),
/// and the kind has a field operator.  A link to a link follows it, so the answer is the
/// fixpoint over the binds.
fn store_links(cx: &Cx, def: &crate::data::Definition, seed: &[u16]) -> Vec<bool> {
    use crate::data::Type;
    let vars = def.variables();
    let n = vars.count() as usize;
    let args = vars.arguments();
    let mut binds: Vec<Vec<&Value>> = vec![Vec::new(); n];
    def.code().walk(&mut |node| {
        if let Value::Set(v, x) = node
            && !args.contains(v)
            && matches!(vars.tp(*v).base(), Type::RefVar(_))
            && crate::scopes::link_set_repoints(cx.data, vars, *v, x)
        {
            binds[*v as usize].push(x);
        }
    });
    let kind_ok = |v: usize| {
        let Type::RefVar(tp) = vars.tp(v as u16).base() else {
            return false;
        };
        link_kind(tp)
    };
    let mut store = vec![false; n];
    for &a in seed {
        store[a as usize] = true;
    }
    loop {
        let mut changed = false;
        for v in 0..n {
            if store[v] || binds[v].is_empty() || !kind_ok(v) {
                continue;
            }
            let all_places = binds[v].iter().all(|x| match x.unspan() {
                Value::Call(d, xs) => match cx.data.def(*d).name() {
                    "OpCreateStack" => false,
                    "OpVarRef" => matches!(xs.as_slice(), [Value::Var(a)] if store[*a as usize]),
                    _ => true,
                },
                _ => true,
            });
            if all_places {
                store[v] = true;
                changed = true;
            }
        }
        if !changed {
            return store;
        }
    }
}

/// Does `v`'s single-dep chain reach one of `bufs`?  As native's `grows_buffer` follows a
/// mover's root.
fn chain_reaches(vars: &crate::variables::Function, v: u16, bufs: &[u16]) -> bool {
    let mut cur = v;
    for _ in 0..vars.var_count() {
        if bufs.contains(&cur) {
            return true;
        }
        match vars.tp(cur).depend().as_slice() {
            [d] if *d != cur => cur = *d,
            _ => return false,
        }
    }
    false
}

/// The member of `params` that `v`'s dep chain ends at, through single-dep views (sixteen
/// steps at most), as native's `param_root`.
fn param_root(vars: &crate::variables::Function, v: u16, params: &[u16]) -> Option<u16> {
    let mut cur = v;
    for _ in 0..16 {
        if params.contains(&cur) {
            return Some(cur);
        }
        let [d] = vars.tp(cur).depend()[..] else {
            return None;
        };
        cur = d;
    }
    None
}

/// The C shape that holds a value of kind `k`, for a value staged into a temporary; `None` for
/// a view or a condition-less kind no temporary can hold whole.
fn shape_of_kind(k: K) -> Option<CType> {
    Some(match k {
        K::Int => CType::Int,
        K::Int32 => CType::Int32,
        K::Cond | K::Bool => CType::Bool,
        K::Ref => CType::Ref,
        K::Enum => CType::Enum,
        K::Float => CType::Float,
        K::Single => CType::Single,
        K::Char => CType::Char,
        K::Fn => CType::Fn,
        K::Tup(id) => CType::Tuple(id),
        K::View | K::Void => return None,
    })
}
