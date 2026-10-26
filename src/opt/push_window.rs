// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The push and mint window analyses — which counted loops push or mint through a window, and
//! whether the window may stand — moved here from `generation::hoist` unchanged, with the
//! helpers they share, so the C backend can name them without reaching up into the Rust
//! writer.

use crate::data::{Block, Data, Type, Value};
use crate::database::Stores;
use crate::loop_shape::{plain_for_body, range_counters};
use crate::opt::attrs::retbuf_var;
use crate::opt::body_writes::owned_local;
use crate::opt::loop_records::is_scalar;
use crate::opt::loop_records::{IN_PLACE_SET_OPS, SCALAR_GETTERS};
use crate::opt::writes::plain_record_type;
use crate::opt::writes::{
    FUSABLE_PUSHES, HoistOwned, HoistTiers, MintTarget, PathKey, mint_path, mint_target,
    push_operands, vector_path,
};
use crate::opt::writes::{blocks_header_hoist, scalar_file_read};
use std::collections::{HashMap, HashSet};

/// `@FR-R-Base`'s growth clause and `@FR-R-RecPtr`'s remainder clause — can two variables'
/// records be proven to live in DIFFERENT stores, so a growth of the one cannot move the
/// other?  A store's buffer is reallocated only by a growth of a vector living in it, so a
/// base or a record address goes stale only when ITS store grew.
///
/// A variable's store is the END of its dep chain: a view (`e = v[i]?`, `for e in v`,
/// `d = &w`) borrows from its source, a local vector from its hidden `__vdb_N` store witness,
/// an argument from nothing.  Two ends name two stores when they differ and at least one is
/// FRESH — minted by this activation and shared with nothing but its borrowers: a `__vdb_N`
/// witness, or a user-named local record that owns its store.  Never fresh: the witness a
/// return-buffer adoption left unallocated (`RetAdopt`: the local IS the caller's buffer), a
/// buffer `(R-Place)` claimed in a parameter's store, and an emitter-owned buffer or witness
/// ([`HoistOwned`](crate::opt::writes::HoistOwned): a § V-z witness binds an element's slot).  Two arguments, or an argument
/// beside the return buffer, are never proven apart — a caller can hand two fields of one
/// record — and a chain that forks (a tuple's union) or leaves the table answers nothing.
pub struct StoreFacts<'a> {
    pub vars: &'a crate::variables::Function,
    /// The locals whose buffer `(R-Place)` claimed in another record's store.
    pub placed: &'a HashSet<u16>,
    /// `RetAdopt`'s result local and its witness, when the function adopts its buffer.
    pub adopted: Option<(u16, u16)>,
    pub owned: Option<&'a HoistOwned>,
    /// Pairs of variables whose stores a RUNTIME test has proved apart for the code being
    /// emitted: `@FR-R-Alias`'s versioned clause, the loop copy that runs only when
    /// `retbuf.store_nr != p.store_nr` held at its entry.
    pub assumed: &'a [(u16, u16)],
}

impl StoreFacts<'_> {
    /// The end of `x`'s dep chain; `None` when the chain forks, loops or leaves the table.
    pub(crate) fn terminal(&self, x: u16) -> Option<u16> {
        let mut cur = x;
        let mut seen: HashSet<u16> = HashSet::new();
        loop {
            if cur as usize >= self.vars.var_count() || !seen.insert(cur) {
                return None;
            }
            let deps: Vec<u16> = self
                .vars
                .tp(cur)
                .depend()
                .into_iter()
                .filter(|d| *d != cur)
                .collect();
            match deps.as_slice() {
                [] => return Some(cur),
                [d] => cur = *d,
                _ => return None,
            }
        }
    }

    /// Is the chain end `t` a store this activation minted and shares with nothing but its
    /// borrowers?
    pub(crate) fn fresh(&self, t: u16) -> bool {
        let vars = self.vars;
        if self.adopted.is_some_and(|(v, vdb)| t == v || t == vdb)
            || self.placed.contains(&t)
            || self
                .owned
                .is_some_and(|o| o.buffers.contains(&t) || o.records.contains(&t))
        {
            return false;
        }
        if vars.name(t).starts_with("__vdb") {
            return true;
        }
        !vars.is_argument(t)
            && !vars.is_captured(t)
            && !vars.is_inline_ref(t)
            && !vars.is_skip_free(t)
            && !vars.is_compiler_generated(t)
            && !matches!(vars.tp(t).base(), Type::RefVar(_))
            && crate::data::is_dbref(vars.tp(t).base())
    }

    /// Is `t` the owner of a store nothing outside this activation can name — `fresh`,
    /// asked by `@FR-R-Header`'s apart clause.
    pub(crate) fn owns_alone(&self, t: u16) -> bool {
        self.fresh(t)
    }

    /// `@FR-R-Header`'s parameter clause: is the store `w` names provably apart from the
    /// store of ARGUMENT `p`?  Only where `w`'s chain ends at a store this activation minted
    /// that is no argument itself — a hidden vector buffer is an argument a caller may have
    /// claimed in `p`'s store (`(R-Place)`) — or where a run-time test proved the pair apart
    /// (`assumed`, `@FR-R-Alias`'s versioned copy).
    #[must_use]
    pub fn apart_from_argument(&self, p: u16, w: u16) -> bool {
        let assumed = |a: u16, b: u16| {
            self.assumed
                .iter()
                .any(|&(x, y)| (x == a && y == b) || (x == b && y == a))
        };
        if assumed(p, w) {
            return true;
        }
        let Some(t) = self.terminal(w) else {
            return false;
        };
        assumed(p, t) || (t != p && !self.vars.is_argument(t) && self.fresh(t))
    }

    /// Do `a`'s and `b`'s records provably live in different stores?
    #[must_use]
    pub fn distinct(&self, a: u16, b: u16) -> bool {
        self.distinct_at(a, b, 0)
    }

    /// The dependencies where `x`'s chain forks: a view bound from a choice between views
    /// (`lo = if c { p } else { q }`) depends on each, and its record is one of theirs.
    pub(crate) fn fork(&self, x: u16) -> Option<Vec<u16>> {
        let mut cur = x;
        let mut seen: HashSet<u16> = HashSet::new();
        loop {
            if cur as usize >= self.vars.var_count() || !seen.insert(cur) {
                return None;
            }
            let deps: Vec<u16> = self
                .vars
                .tp(cur)
                .depend()
                .into_iter()
                .filter(|d| *d != cur)
                .collect();
            match deps.len() {
                0 => return None,
                1 => cur = deps[0],
                _ => return Some(deps),
            }
        }
    }

    pub(crate) fn distinct_at(&self, a: u16, b: u16, depth: u8) -> bool {
        // A forked chain: apart from `b` when every branch is (`@FR-R-RecPtr`'s choice
        // clause) — each branch judged as any chain is, an `if` arm lifted into `__lift_N`
        // being a COPY in a store of its own.  Bounded, as the type's dependency sets are not
        // cycle-free by contract.
        if depth < 4 {
            if let Some(ds) = self.fork(a) {
                return ds.iter().all(|d| self.distinct_at(*d, b, depth + 1));
            }
            if let Some(ds) = self.fork(b) {
                return ds.iter().all(|d| self.distinct_at(a, *d, depth + 1));
            }
        }
        let assumed = |a: u16, b: u16| {
            self.assumed
                .iter()
                .any(|&(x, y)| (x == a && y == b) || (x == b && y == a))
        };
        if assumed(a, b) {
            return true;
        }
        let (Some(ta), Some(tb)) = (self.terminal(a), self.terminal(b)) else {
            return false;
        };
        // A run-time test proved the two ROOTS apart: a view of a parameter's element lives
        // in that parameter's store, so it is apart from whatever the parameter is apart from.
        if assumed(ta, tb) {
            return true;
        }
        ta != tb && (self.fresh(ta) || self.fresh(tb))
    }
}

/// Is `r` a parameter `@FR-R-WorkBuffer` promoted from a local?  Its store is a scratch
/// buffer the caller mints per call site and hands to this call alone, so nothing but this
/// activation reaches it — exclusive, as the owned local it was.  Argument slots follow
/// attribute order.
pub(crate) fn work_buffer_arg(data: &Data, def_nr: u32, r: u16) -> bool {
    let def = data.def(def_nr);
    def.variables()
        .arguments()
        .iter()
        .position(|&a| a == r)
        .and_then(|i| def.attributes().get(i))
        .is_some_and(|a| a.work_buffer)
}

/// Is `v` an INVARIANT scalar expression the fill may evaluate once — a variable other
/// than the loop's own, a literal, plain integer/float arithmetic over those, or a record
/// scalar read off such a variable?  Anything else (a call, a `??` block, an element read,
/// a conversion) keeps the per-element loop: the fill evaluates the expression once and
/// the fallback once more, so it must be pure and free of the loop's counters.
/// The two-operand scalar field reads `simple_invariant` admits (`OpGetInt(r, off)`).
pub(crate) const FILL_FIELD_GETTERS: [&str; 4] =
    ["OpGetInt", "OpGetLong", "OpGetFloat", "OpGetSingle"];

pub(crate) fn simple_invariant(v: &Value, data: &Data, banned: &[u16]) -> bool {
    match v.unspan() {
        Value::Var(x) => !banned.contains(x),
        Value::Int(_) | Value::Float(_) | Value::Boolean(_) | Value::Enum(_, _) => true,
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            if matches!(
                name,
                "OpAddInt" | "OpMinInt" | "OpMulInt" | "OpAddFloat" | "OpMinFloat" | "OpMulFloat"
            ) {
                args.len() == 2 && args.iter().all(|a| simple_invariant(a, data, banned))
            } else if SCALAR_GETTERS.contains(&name) && args.len() == 3 {
                matches!(args[0].unspan(), Value::Var(r) if !banned.contains(r))
                    && matches!(args[1].unspan(), Value::Int(_))
                    && matches!(args[2].unspan(), Value::Int(_))
            } else if name.starts_with("OpGet")
                && FILL_FIELD_GETTERS.contains(&name)
                && args.len() == 2
            {
                // A scalar field of a record the loop does not write (`self.width`): the
                // fill's writes land in the vector's own record, never in a field.
                matches!(args[0].unspan(), Value::Var(r) if !banned.contains(r))
                    && matches!(args[1].unspan(), Value::Int(_))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// @PLN157 § V-am (`@FR-R-PushFill`) — a counted loop that PUSHES: `k` scalar pushes to
/// one pure path at the top level of its body, every iteration, with nothing that can
/// leave the loop early and no other write reaching the path.  The reserve form holds for
/// any such loop (`pushes` × the trip count, once, before the loop); the FILL form is the
/// body that is that one push of a simple invariant and nothing else.
pub struct PushLoop<'a> {
    pub path: PathKey,
    pub vector: &'a Value,
    pub rust_type: &'static str,
    pub size: u32,
    pub hi: &'a Value,
    pub inclusive: bool,
    pub index_var: u16,
    pub next_var: Option<u16>,
    /// The pushes per iteration.
    pub pushes: u32,
    /// The invariant value when the body is ONE push of it — the fill form.
    pub fill: Option<&'a Value>,
}

/// Recognise the counted push loop (`@FR-R-PushFill`) in either spelling — the statement
/// `for i in a..b { v += [e] }` and the comprehension `[for i in a..b { e }]`
/// ([`push_loop_body`]).  Declines, and says why under
/// `LOFT_TRACE_PUSH_FILL=1`, whenever the trip count cannot be known before the loop runs
/// or the pushes per iteration cannot be counted: a `break`, a `return`, a `continue`, an
/// inner loop, a push under a branch, a push to a second path, an append or any other
/// write reaching the path, a range end that is not a simple invariant.  The fallback is
/// `None` — the loop runs as it did, which costs the reserve and never a value.
#[must_use]
pub fn push_loop<'a>(lp: &'a Block, data: &Data) -> Option<PushLoop<'a>> {
    push_loop_on(lp, data, None)
}

#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn push_loop_on<'a>(
    lp: &'a Block,
    data: &Data,
    only: Option<&PathKey>,
) -> Option<PushLoop<'a>> {
    let trace = std::env::var("LOFT_TRACE_PUSH_FILL").is_ok() && only.is_none();
    let decline = |why: &str| -> Option<PushLoop<'a>> {
        // A loop with no push of its own is not a shape the rule is for (an outer loop
        // around a counted one): traced, never counted.
        if only.is_none() && why != "no push at the top level of the body" {
            crate::rewrite_census::declined("R-PushFill/loop", &format!("loop {}", lp.scope), why);
        }
        if trace {
            eprintln!("push-fill: loop {} declined — {why}", lp.scope);
        }
        None
    };
    let body = push_loop_body(lp)?;
    let rc = match range_counters(lp, data) {
        Ok(rc) => rc,
        Err(why) => return decline(&why),
    };
    let stmts: Vec<&Value> = body
        .iter()
        .copied()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let push_kind = |d: &u32| -> Option<(&'static str, u32)> {
        if (*d as usize) >= data.definitions.len() {
            return None;
        }
        let name = data.def(*d).name();
        FUSABLE_PUSHES
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, rt, w)| (*rt, *w))
    };
    // The pushes at the top level of the body.
    let mut path: Option<PathKey> = None;
    let mut vector: Option<&'a Value> = None;
    let mut rust_type = "";
    let mut size = 0u32;
    let mut vals: Vec<&'a Value> = Vec::new();
    // A biased push (the byte kind) writes an ENCODED element, so the one-value slice fill
    // (which writes the value as it is) is not admitted for it; the reserve and the window
    // are, since they write through the push emitter, which encodes.
    let mut biased = false;
    for s in &stmts {
        if let Value::Call(d, args) = s.unspan()
            && let Some((rt, w)) = push_kind(d)
            && let Some((vec_arg, bias, val)) = push_operands(data.def(*d).name(), args)
            && let Some(p) = vector_path(data, vec_arg)
            && only.is_none_or(|o| *o == p)
        {
            biased |= bias.is_some();
            match &path {
                Some(pp) if *pp != p => return decline("the pushes reach two paths"),
                Some(_) => {}
                None => {
                    path = Some(p);
                    vector = Some(vec_arg);
                    rust_type = rt;
                    size = w;
                }
            }
            vals.push(val);
        }
    }
    let (Some(path), Some(vector)) = (path, vector) else {
        return decline("no push at the top level of the body");
    };
    // The statements that are neither a counted push nor the path's own reservation (which
    // the parser emits BEFORE the push, so it is counted once the path is known).
    let plain = stmts
        .iter()
        .filter(|s| {
            !matches!(s.unspan(), Value::Call(d, a)
                if (push_kind(d).is_some()
                    || ((*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpPreAllocVector"))
                    && a.first().and_then(|f| vector_path(data, f)).as_ref() == Some(&path))
        })
        .count();
    let early = body.iter().any(|s| {
        s.any_node(&mut |n| {
            matches!(
                n,
                Value::Break(_)
                    | Value::Return(_)
                    | Value::Continue(_)
                    | Value::Loop(_)
                    | Value::Yield(_)
                    | Value::Parallel(_)
            )
        })
    });
    if early {
        return decline("the body can leave early, or loops");
    }
    // Every push to the path is one of the counted ones, and nothing else writes it.
    let mut all = 0usize;
    let mut other = false;
    for s in &body {
        s.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && let Some(first) = args.first()
                && vector_path(data, first).as_ref() == Some(&path)
            {
                let name = data.def(*d).name();
                if push_kind(d).is_some() {
                    all += 1;
                } else if !(name == "OpPreAllocVector"
                    || name.starts_with("OpGet")
                    || name.starts_with("OpLength"))
                {
                    other = true;
                }
            }
            false
        });
    }
    if other {
        return decline("another write reaches the path");
    }
    if all != vals.len() {
        return decline("a push stands under a branch");
    }
    let mut banned = vec![rc.loop_var, rc.index, path.0];
    if let Some(nx) = rc.next {
        banned.push(nx);
    }
    if !simple_invariant(rc.hi, data, &banned) {
        return decline("the range's end is not a simple invariant");
    }
    let fill =
        if !biased && vals.len() == 1 && plain == 0 && simple_invariant(vals[0], data, &banned) {
            Some(vals[0])
        } else {
            None
        };
    let pushes = u32::try_from(vals.len()).ok()?;
    Some(PushLoop {
        path,
        vector,
        rust_type,
        size,
        hi: rc.hi,
        inclusive: rc.inclusive,
        index_var: rc.index,
        next_var: rc.next,
        pushes,
        fill,
    })
}

/// The statements a counted push loop runs once per iteration beside its iterator: the
/// body block of a plain `for` statement, or — a comprehension `[for i in a..b { e }]`
/// having no body block — everything after its iterator (the element's value, then the
/// append).  The two spell ONE loop, so `(R-PushFill)` reads them through one home; a
/// filtered loop of either kind is declined by its `continue`, a `#count` loop by shape.
pub(crate) fn push_loop_body(lp: &Block) -> Option<Vec<&Value>> {
    if let Some(body) = plain_for_body(lp) {
        Some(body.operators.iter().collect())
    } else if lp.name == "For comprehension" {
        Some(lp.operators.iter().skip(1).collect())
    } else {
        None
    }
}

/// `@FR-R-PushFill`'s window clause — may the counted push loop `lp`, which [`push_loop`]
/// answered `p` for, run its pushes through a [`crate::vector::PushWindow`]: a held element
/// base, a local length, and the record's own length written once when the loop ends?
///
/// While a window is open the record's length lags the pushes and the window's base
/// addresses the store's buffer, so two things must hold for the loop's whole extent.
/// NOTHING BUT THESE PUSHES REACHES THE VECTOR: its root is exclusive (`@FR-R-Alias` — an
/// owned local or the return buffer), the body never names the root outside a counted
/// push's own vector operand — no read, no length, no call handed it — and every OTHER
/// variable the body names cannot name the root's store either, by the ownership test
/// `(R-Alias)` keeps a read candidate under: a scalar or a text, a local that owns its own
/// store, or a parameter while the root is no return buffer.  A view taken before the loop
/// (`d = &v`) fails it, which is the point: `len(d)` in the body would go through the
/// runtime and meet the lagging length.  And NOTHING ELSE GROWS A STORE: every other
/// statement, and every pushed value, writes no store at all ([`may_write_store`](crate::opt::writes::may_write_store)), so no
/// buffer is reallocated under the base.  [`push_loop`] already secured the single exit
/// (no `break`, `return`, `continue` or inner loop), so the close that follows the loop
/// runs on every path out of it.
///
/// # Errors
///
/// The condition that does not hold — what `LOFT_TRACE_PUSH_FILL=1` prints.  The caller
/// keeps the header push, which costs the window and never a value.
pub fn push_window_ok(
    lp: &Block,
    p: &PushLoop,
    data: &Data,
    def_nr: u32,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
) -> Result<(), String> {
    if p.fill.is_some() {
        return Err("the loop is one fill".to_string());
    }
    let vars = data.def(def_nr).variables();
    let root = p.path.0;
    let root_is_retbuf = retbuf_var(data, def_nr) == Some(root);
    if root >= vars.count()
        || !(owned_local(vars, root) || root_is_retbuf || work_buffer_arg(data, def_nr, root))
    {
        return Err("the pushed vector's root is not exclusive".to_string());
    }
    let Some(body) = push_loop_body(lp) else {
        return Err("not a counted push loop".to_string());
    };
    let mut parts: Vec<&Value> = Vec::new();
    for s in body {
        // A counted push contributes its VALUE operand alone, and the path's own
        // reservation its count: the vector operand of either is the window's business.
        match s.unspan() {
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && args.first().and_then(|f| vector_path(data, f)).as_ref()
                        == Some(&p.path)
                    && (FUSABLE_PUSHES
                        .iter()
                        .any(|(n, _, _)| *n == data.def(*d).name())
                        || data.def(*d).name() == "OpPreAllocVector") =>
            {
                parts.extend(args.iter().skip(1));
            }
            _ => parts.push(s),
        }
    }
    window_parts_ok(
        &parts,
        &[root],
        root_is_retbuf,
        vars,
        data,
        stores,
        cache,
        None,
    )
}

/// The window clause's aliasing test over the `parts` of a loop body that are NOT the
/// pushes themselves: none names a pushed root, none names a variable that may view a
/// root's store, and none writes a store.  Shared by the scalar window
/// ([`push_window_ok`]), the several-paths one (`generation::hoist::push_windows_ok`) and the record one
/// ([`mint_window_ok`]).  `root_is_retbuf` is whether ANY root is the return buffer.  With
/// `facts`, a variable whose store they prove apart from every root's (`@FR-R-Base`'s growth
/// clause, an `@FR-R-Alias` versioned copy's runtime test) cannot view one either.
#[expect(
    clippy::too_many_arguments,
    reason = "the window's facts, one per argument"
)]
pub(crate) fn window_parts_ok(
    parts: &[&Value],
    roots: &[u16],
    root_is_retbuf: bool,
    vars: &crate::variables::Function,
    data: &Data,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
    facts: Option<&StoreFacts>,
) -> Result<(), String> {
    // The variables that COULD name the root's store: asked per variable through the one
    // naming predicate (`Value::reads_var`), so every spelling of a mention is covered.
    let may_view: Vec<u16> = (0..vars.count())
        .filter(|&x| {
            // `(R-Alias)`: a parameter cannot name a local's store, but it can name a
            // return buffer the caller offered.
            let separate_parameter = vars.is_argument(x) && !root_is_retbuf;
            // A view of a plain NO-HEAP record (`sp_a = pts[i]?`) reads fields and never a
            // length: the window defers only the root's length, and its slots lie past
            // every element such a view can name — so even a view INTO the root sees what
            // it saw.  A record with a vector or text field could name the root through
            // that field, and stays a possible viewer.
            let record_view = plain_record_type(data, vars.tp(x))
                .is_some_and(|tp| stores.is_some_and(|st| !st.owns_heap(tp)));
            // A vector of plain scalars stored apart from every root (`vector<u8>` beside a
            // pushed `vector<integer>`) reaches no store through its elements and cannot BE
            // a root: element storage is part of a vector's type (`(C-Refl)`, D-types-26).
            let scalar_vector_apart = roots
                .iter()
                .all(|&r| scalar_vector_apart(vars.tp(x), vars.tp(r)));
            // A `File` the body only READS a scalar from (`@FR-R-InPlace`'s read clause):
            // the read touches the File record's own cursor fields in place, never a
            // vector's length, and claims nothing.  A record cannot be an element of a
            // vector of scalars, so beside such roots the File cannot view one.
            let file_read_only = roots.iter().all(|&r| scalar_vector(vars.tp(r)))
                && parts
                    .iter()
                    .all(|p| !names_beside_file_reads(p, x, data, vars));
            !roots.contains(&x)
                && !facts.is_some_and(|f| roots.iter().all(|&r| f.distinct(x, r)))
                && !is_scalar(vars.tp(x))
                && !matches!(vars.tp(x).base(), Type::Text(_))
                && !owned_local(vars, x)
                && !separate_parameter
                && !record_view
                && !scalar_vector_apart
                && !file_read_only
        })
        .collect();
    for part in parts {
        if roots.iter().any(|&r| part.reads_var(r)) {
            return Err("the body names the pushed vector outside its pushes".to_string());
        }
        if let Some(x) = may_view.iter().find(|&&x| part.reads_var(x)) {
            return Err(format!(
                "the body names `{}`, which may view the pushed vector",
                vars.name(*x)
            ));
        }
        // Asked as the header hoist asks it — with the variable table and the in-place
        // tier — so its allowances hold here too: a null-discharge buffer's mint and its
        // default writes, the frame's own return buffer, a record free, a scalar set in
        // place.  None reallocates the root's store, which is all the window's address
        // stands on; the root's LENGTH is protected by the naming tests above.
        if blocks_header_hoist(
            part,
            data,
            stores,
            cache,
            &mut HashSet::new(),
            Some(vars),
            HoistTiers {
                in_place: true,
                ..HoistTiers::default()
            },
            &mut HashSet::new(),
            None,
        ) {
            return Err("the body writes a store beside its pushes".to_string());
        }
    }
    Ok(())
}

/// `@FR-R-PushFill`'s record clause — a counted loop whose body appends RECORD elements to
/// one plain vector through mint groups, possibly under `if` arms, and nothing else.
pub struct MintLoop {
    pub path: PathKey,
    pub vector: Value,
    /// The element's width in bytes.
    pub size: u32,
    pub inclusive: bool,
    pub index_var: u16,
    pub next_var: Option<u16>,
    /// The most mint groups any one pass runs — one per arm of a branch, summed along a
    /// straight line — which times the trip count bounds the appends.
    pub mints_per_pass: u32,
}

/// Recognise the counted RECORD-append loop (`@FR-R-PushFill`'s record clause): `for i in
/// a..b { if … { v += [R { … }] } else { v += [S { … }] } }` — a plain `for` over a counted
/// range whose body's only writes to the path are mint groups (the parser's reservation,
/// mint, field sets on the fresh element, finish), each group over ONE plain vector whose
/// element qualifies for the record push, standing at the body's top level or under `if`
/// arms.  Declines — and says why under `LOFT_TRACE_PUSH_FILL=1` — on an early exit or an
/// inner loop, a group on a second path, any other write reaching the path, or a range end
/// that is not a simple invariant.  Answers the loop's facts and the `hi` end; the aliasing
/// test is [`mint_window_ok`]'s.
///
/// # Errors
/// The reason the loop is not one — what `LOFT_TRACE_PUSH_FILL=1` prints.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn mint_loop<'a>(
    lp: &'a Block,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    heap_push: bool,
) -> Result<(MintLoop, &'a Value), String> {
    let Some(body) = plain_for_body(lp) else {
        return Err("not a plain counted loop".to_string());
    };
    let rc = range_counters(lp, data)?;
    let early = body.operators.iter().any(|s| {
        s.any_node(&mut |n| {
            matches!(
                n,
                Value::Break(_)
                    | Value::Return(_)
                    | Value::Continue(_)
                    | Value::Loop(_)
                    | Value::Yield(_)
                    | Value::Parallel(_)
            )
        })
    });
    if early {
        return Err("the body can leave early, or loops".to_string());
    }
    // Every mint in the body, all on one qualifying path.
    let mut target: Option<MintTarget> = None;
    let mut mints = 0usize;
    for s in &body.operators {
        let mut bad: Option<String> = None;
        s.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpNewRecord"
            {
                match mint_target(data, stores, "OpNewRecord", args, vars) {
                    Some(t) if mint_push_qualifies(stores, t.vector_tp, heap_push) => {
                        match &target {
                            Some(tt) if tt.path != t.path => {
                                bad = Some("the mints reach two paths".to_string());
                            }
                            Some(_) => {}
                            None => target = Some(t),
                        }
                        mints += 1;
                    }
                    _ => {
                        bad = Some(
                            "a mint's element does not qualify for the record push".to_string(),
                        )
                    }
                }
            }
            false
        });
        if let Some(why) = bad {
            return Err(why);
        }
    }
    let Some(target) = target else {
        return Err("no record mint in the body".to_string());
    };
    if mints == 0 {
        return Err("no record mint in the body".to_string());
    }
    // Every op naming the path is a mint group's own: the reservation, the mint, the finish.
    let mut other: Option<&str> = None;
    for s in &body.operators {
        s.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && let Some(first) = args.first()
                && vector_path(data, first).as_ref() == Some(&target.path)
                && !matches!(
                    data.def(*d).name(),
                    "OpPreAllocVector" | "OpNewRecord" | "OpFinishRecord"
                )
            {
                other.get_or_insert(data.def(*d).name());
            }
            false
        });
    }
    if let Some(op) = other {
        return Err(format!("another op reaches the path ({op})"));
    }
    let mut banned = vec![rc.loop_var, rc.index, target.path.0];
    if let Some(nx) = rc.next {
        banned.push(nx);
    }
    if !simple_invariant(rc.hi, data, &banned) {
        return Err("the range's end is not a simple invariant".to_string());
    }
    // The most groups one pass runs: arms of an `if` take the larger, a straight line sums.
    fn most_mints(
        n: &Value,
        data: &Data,
        stores: &Stores,
        vars: &crate::variables::Function,
        path: &PathKey,
    ) -> u32 {
        match n.unspan() {
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && mint_path(data, stores, "OpNewRecord", args, vars).as_ref()
                        == Some(path) =>
            {
                1
            }
            Value::If(c, t, e) => {
                most_mints(c, data, stores, vars, path)
                    + most_mints(t, data, stores, vars, path)
                        .max(most_mints(e, data, stores, vars, path))
            }
            _ => {
                let mut total = 0u32;
                n.for_each_child(&mut |child| total += most_mints(child, data, stores, vars, path));
                total
            }
        }
    }
    let mints_per_pass = body
        .operators
        .iter()
        .map(|s| most_mints(s, data, stores, vars, &target.path))
        .sum::<u32>()
        .max(1);
    let elem = stores.content(target.vector_tp);
    let size = u32::from(stores.size(elem));
    Ok((
        MintLoop {
            path: target.path,
            vector: target.vector,
            size,
            inclusive: rc.inclusive,
            index_var: rc.index,
            next_var: rc.next,
            mints_per_pass,
        },
        rc.hi,
    ))
}

/// Does the body copy a WHOLE RECORD that owns heap fields into one of the fresh elements?
///
/// Such a copy is the element's own bytes only while the record owns no heap: a `text` or a
/// collection field is deep-copied into the VECTOR's store, which can grow and move under a push
/// window's cached base (`push_window`).  A generic instance delivers that way, having no return
/// buffer: its `v += [same(q)]` is `__lift = same(q); OpCopyRecord(__lift, elm)`, and the window
/// wrote through a stale base — a segfault once the store grew (loft#1934).  An unknown copy
/// type, or no schema to ask, answers yes: declining a window costs speed, admitting it wrongly
/// costs memory safety.
pub(crate) fn copies_heap_into_fresh(
    body: &Block,
    fresh: &HashSet<u16>,
    data: &Data,
    stores: Option<&Stores>,
) -> bool {
    let copy_d = data.def_nr("OpCopyRecord");
    body.operators.iter().any(|s| {
        s.any_node(&mut |n| {
            matches!(n, Value::Call(d, args) if *d == copy_d
            && args.len() == 3
            && matches!(args[1].unspan(), Value::Var(e) if fresh.contains(e))
            && match (args[2].unspan(), stores) {
                (Value::Int(tp), Some(st)) => {
                    st.owns_heap((*tp as u16) & crate::keys::COPY_TP_MASK)
                }
                _ => true,
            })
        })
    })
}

/// `@FR-R-PushFill`'s record clause — may the record-append loop `lp`, which [`mint_loop`]
/// answered `m` for, run its mints through a [`crate::vector::PushWindow`]?  The scalar
/// window's condition, with a mint group's own ops as "the pushes": the fresh element's
/// field sets write the slot the window handed out and are not store writes to this test,
/// while their VALUE operands, the group's reservation count and every other statement are
/// the parts that must name neither the root nor a variable that may view its store, and
/// write no store.
///
/// # Errors
///
/// The condition that does not hold — what `LOFT_TRACE_PUSH_FILL=1` prints.
#[expect(
    clippy::too_many_lines,
    reason = "inherited; the window test's store facts wrapped its call"
)]
pub fn mint_window_ok(
    lp: &Block,
    m: &MintLoop,
    data: &Data,
    def_nr: u32,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
) -> Result<(), String> {
    let vars = data.def(def_nr).variables();
    let root = m.path.0;
    let root_is_retbuf = retbuf_var(data, def_nr) == Some(root);
    if root >= vars.count()
        || !(owned_local(vars, root) || root_is_retbuf || work_buffer_arg(data, def_nr, root))
    {
        return Err("the appended vector's root is not exclusive".to_string());
    }
    let Some(body) = plain_for_body(lp) else {
        return Err("not a plain counted loop".to_string());
    };
    // The elements minted by the groups: their field sets are the group's own writes.
    let mut fresh: HashSet<u16> = HashSet::new();
    for s in &body.operators {
        s.any_node(&mut |n| {
            if let Value::Set(e, rhs) = n
                && matches!(rhs.unspan(), Value::Call(d, args)
                    if (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpNewRecord"
                        && args.first().and_then(|f| vector_path(data, f)).as_ref() == Some(&m.path))
            {
                fresh.insert(*e);
            }
            false
        });
    }
    fn collect<'v>(
        n: &'v Value,
        data: &Data,
        path: &PathKey,
        fresh: &HashSet<u16>,
        parts: &mut Vec<&'v Value>,
    ) {
        match n.unspan() {
            Value::If(c, t, e) => {
                parts.push(c);
                collect(t, data, path, fresh, parts);
                collect(e, data, path, fresh, parts);
            }
            Value::Block(b) => {
                for op in &b.operators {
                    collect(op, data, path, fresh, parts);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    collect(op, data, path, fresh, parts);
                }
            }
            Value::Line(_) => {}
            Value::Set(e, rhs) if fresh.contains(e) => {
                // The mint itself: its operands beyond the vector.
                if let Value::Call(_, args) = rhs.unspan() {
                    parts.extend(args.iter().skip(1));
                }
            }
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && args.first().and_then(|f| vector_path(data, f)).as_ref() == Some(path)
                    && matches!(data.def(*d).name(), "OpPreAllocVector" | "OpFinishRecord") =>
            {
                // The finish's second operand is the fresh element itself — the group's
                // own slot, not a view the window must fear.
                let skip = if data.def(*d).name() == "OpFinishRecord" {
                    2
                } else {
                    1
                };
                parts.extend(args.iter().skip(skip));
            }
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && IN_PLACE_SET_OPS.contains(&data.def(*d).name())
                    && args
                        .first()
                        .and_then(|f| vector_path(data, f))
                        .is_some_and(|(root, _)| fresh.contains(&root)) =>
            {
                // A field set on the fresh element — its own field, or one reached
                // through its inline sub-records (`pos.x`): the value is the part.
                parts.extend(args.iter().skip(1));
            }
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpCopyRecord"
                    && args.len() == 3
                    && matches!(args[1].unspan(), Value::Var(e) if fresh.contains(e)) =>
            {
                // A whole-record copy INTO the fresh element — a builder's tuple delivered
                // into the slot: writes of the element's own bytes; the source is the part.
                parts.push(&args[0]);
            }
            _ => parts.push(n),
        }
    }
    if copies_heap_into_fresh(body, &fresh, data, stores) {
        return Err("a whole-record copy into the element owns heap fields".to_string());
    }
    let mut parts: Vec<&Value> = Vec::new();
    for s in &body.operators {
        collect(s, data, &m.path, &fresh, &mut parts);
    }
    window_parts_ok(
        &parts,
        &[root],
        root_is_retbuf,
        vars,
        data,
        stores,
        cache,
        None,
    )
}

/// @PLN157 § V-t (`@FR-R-PushRec`) — may this mint group EMIT through a push header?  The
/// schema is asked, not the op shape: `vector_tp` — the type [`mint_target`](crate::opt::writes::mint_target) resolved, from
/// either spelling of the append — must be a plain inline-element vector (`Parts::Vector`
/// — an `array`/`ordered` conversion holds 4-byte handles, and a keyed container's
/// same-named ops place records), and the element a plain struct that owns no heap — a raw slot carries stale
/// bytes, and a heap handle is the one field kind whose stale bytes something could walk
/// before the group's writes land.  The group's writes cover every scalar field explicitly
/// (the IR's literal lowering emits omitted fields' defaults and sentinels itself; a
/// declined delivery is a whole-record copy), so no prefill is owed.
#[must_use]
pub fn mint_push_qualifies(stores: &Stores, vector_tp: u16, heap: bool) -> bool {
    if !stores.is_plain_vector(vector_tp) {
        return false;
    }
    let elem = stores.content(vector_tp);
    // `@FR-R-PushRec` heap clause — a heap-owning element's slot is ZEROED at the mint
    // (the handles' prefill, one range write), so the stale-bytes objection no longer
    // holds; `heap` is the switch that keeps such an element on its templates.
    // The enum clause — a USER struct-enum element is a record too: its literal writes the
    // tag (`OpSetEnum(e, 0, variant)`) beside the variant's fields, and its slot is always
    // zeroed, since a narrower variant leaves the tail of a wider one's slot unwritten.
    elem != u16::MAX
        && (stores.is_struct(elem) || (enum_record_enabled() && stores.is_struct_enum(elem)))
        && (heap || !stores.owns_heap(elem))
}

/// `LOFT_NO_ENUM_RECORD=1` — a struct-enum value is no record to the hoist family again: no
/// push header for a vector of them, no address for a view of one (`@FR-R-Switch`).  Read
/// at generation time.
#[must_use]
pub fn enum_record_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_ENUM_RECORD").is_ok_and(|v| v != "0"))
}

/// Two vectors of plain scalars whose ELEMENT STORAGE differs — never one vector, because
/// the type checker refuses one where the other is declared (loft#751, `(C-Refl)` decided on
/// storage), and neither can name a store through its elements.  `same_element_storage`
/// answers `true` for any pair it does not compare (an integer and a float), so only a
/// definite `false` counts — the safe direction for a test that admits a window.
pub(crate) fn scalar_vector_apart(a: &Type, b: &Type) -> bool {
    let (Type::Vector(ea, _), Type::Vector(eb, _)) = (a.peel_link(), b.peel_link()) else {
        return false;
    };
    is_scalar(ea) && is_scalar(eb) && !ea.same_element_storage(eb)
}

/// A vector whose elements are plain scalars (seen through `&` and `τ?`).
pub(crate) fn scalar_vector(tp: &Type) -> bool {
    matches!(tp.peel_link(), Type::Vector(e, _) if is_scalar(e))
}

/// Does `n` name `x` anywhere but as the file operand of a scalar file read
/// ([`scalar_file_read`])?  The read's other operands are asked like any subtree, and every
/// other spelling of a mention is [`Value::names_var_here`]'s — so the fallback is a
/// mention, the answer that keeps the window closed.
pub(crate) fn names_beside_file_reads(
    n: &Value,
    x: u16,
    data: &Data,
    vars: &crate::variables::Function,
) -> bool {
    let n = n.unspan();
    if let Value::Call(d, args) = n
        && (*d as usize) < data.definitions.len()
        && matches!(args.first().map(Value::unspan), Some(Value::Var(f)) if *f == x)
        && scalar_file_read(data, data.def(*d).name(), args, Some(vars))
    {
        return args[1..]
            .iter()
            .any(|a| names_beside_file_reads(a, x, data, vars));
    }
    if n.names_var_here(x) {
        return true;
    }
    let mut found = false;
    n.for_each_child(&mut |c| {
        if !found && names_beside_file_reads(c, x, data, vars) {
            found = true;
        }
    });
    found
}
