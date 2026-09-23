// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! @I60 — Scope & dependency/lifetime tracker (deps): the copy-lease half.
//!
//! The copy-lease verdicts on a copy of a droppable (@PLN163): what `formal/heap.md` makes of the
//! line that writes the copy, and — separately — whether the copied value is used afterwards.
//!
//! **The rule read off the line** (`(H-Move)`, `(H-Copy-Refuse)`, @FR-H-Copy-Refuse).  A copy is
//! judged by what it copies and where the value goes, never by what the program does after it.
//! What it copies is classified: a fresh value (a call result, a literal), a member of a container,
//! a `&` link, or a variable — and a compiler temp is judged by what it was given.  Where the value
//! goes is the [`Placement`] the census reads off the surrounding IR: a new structure, a `return`
//! (the function's own variables end with it), a block's result (a variable declared in that block
//! ends with it), or a read through a temporary that nothing outlives.  A fresh value, a read
//! through, a `return` of a variable the function owns, and the result of a block that declares
//! the variable are moves; any other existing value placed is a copy the rule refuses.
//!
//! **The liveness answer** (@FR-H-Move) — is the copied value used after the copy? — was the first
//! version's move rule and no longer decides validity.  It is kept for `(H-Elide)`, where liveness
//! may decide an optimisation, and the census prints it beside the verdict so it stays tested.  The
//! pass is per path: both arms of every `if`, a `loop` to its fixed point, `break` and `continue` to
//! their targets, and a `return` that ends the path.  A use is a read of a variable that holds the
//! value or VIEWS it, decided from its assignments (a projection, a `&` link, a join arm its type
//! also depends on), because a join whose arm lifts the variable into a copy still lists it among
//! its dependencies.  The scope pass's own work is not a use: a release (a free or drop call, the
//! null check and `__hoff_` flag test that guard it, the sentinel written after it), the `__disp_`
//! snapshot a rebind takes, and the `__hoff_` flag writes — each recognised by a call or a name the
//! author cannot write, never by the shape of an `if` alone.
//!
//! Both halves share the sources no position makes legal: a MEMBER of a container (a projection,
//! or a variable assigned one, such as a loop variable or a `match` binding), a PARAMETER or a
//! local holding the caller's record, and a CAPTURED variable.  Nothing that emits code reads this
//! module.

use crate::data::{Data, Definition, Type, Value};
use crate::use_analysis::{projection_root, releases_first_arg};
use crate::variables::Function;
use std::collections::{HashMap, HashSet};

/// Why a copy is not a move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// An existing variable placed into a new structure (`(H-Copy-Refuse)`).
    Copied(u16),
    /// The copied value is used again on some path after the copy (the liveness answer).
    Later { var: u16, line: u32 },
    /// The source is a member of a container, and the container's drop still holds it.
    Container(u16),
    /// The source is a parameter, or reached through one: the caller holds it.
    Caller(u16),
    /// The source is captured by a closure, which holds it.
    Captured(u16),
}

impl Refusal {
    /// The variable the refusal names.
    #[must_use]
    pub fn var(&self) -> u16 {
        match self {
            Self::Later { var, .. } => *var,
            Self::Copied(v) | Self::Container(v) | Self::Caller(v) | Self::Captured(v) => *v,
        }
    }

    /// The census spelling: `copy:a`, `later:a@12`, `container:s`, `caller:p`, `captured:a`.
    #[must_use]
    pub fn describe(&self, func: &Function) -> String {
        match self {
            Self::Copied(v) => format!("copy:{}", func.name(*v)),
            Self::Later { var, line } => format!("later:{}@{line}", func.name(*var)),
            Self::Container(v) => format!("container:{}", func.name(*v)),
            Self::Caller(v) => format!("caller:{}", func.name(*v)),
            Self::Captured(v) => format!("captured:{}", func.name(*v)),
        }
    }

    /// What `(H-Copy-Refuse)` says to the author about this copy: the act, why it is refused, and
    /// what to write instead — *"use the value where it is, pass it, build it where it belongs,
    /// or return the owner"*.
    ///
    /// `who` is the name the author wrote for the value, from [`Frame::author_name`], and is
    /// `None` when the refusal reaches only a compiler temp — then the sentence describes the
    /// value by its TYPE instead.  Naming `__lift_2` would describe the compiler's workings to
    /// someone reading about their own program, which is the shape loft#1453 already paid for.
    ///
    /// It never names a LATER line, because no later line decides the verdict: the rule is read
    /// off this one and the declared types alone.
    #[must_use]
    pub fn message(&self, who: Option<&str>, tp: &str) -> String {
        let it = who.map_or_else(|| format!("a value of `{tp}`"), |name| format!("`{name}`"));
        let twice = "so the copy releases it a second time";
        match self {
            // `Later` is the SUPERSEDED liveness reading, which P2r settled does not decide
            // validity; the raise site skips it.  Phrased as a plain copy so this stays total.
            Self::Copied(_) | Self::Later { .. } => format!(
                "cannot copy {it} here — `{tp}` owns a resource, and a copy is a second \
                 structure that releases it a second time. Use it where it is, pass it as an \
                 argument, or build a new value here"
            ),
            Self::Container(_) => {
                let owner = who.map_or_else(|| "the container".to_string(), |n| format!("`{n}`"));
                format!(
                    "cannot copy a member of {owner} here — {owner} still owns that member and \
                     releases it when it goes, {twice}. Read it where it lives, or build a new \
                     value here"
                )
            }
            Self::Caller(_) => format!(
                "cannot copy {it} here — the caller still owns what it passed and releases it, \
                 {twice}. Read it where it lives, or return the owner"
            ),
            Self::Captured(_) => format!(
                "cannot copy {it} here — the closure that captured it still owns it, {twice}. \
                 Read it where it lives, or build a new value here"
            ),
        }
    }
}

/// The verdict on one copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lease {
    /// A move: one of `(H-Move)`'s written positions — or, for the liveness answer, no later use.
    Move,
    /// The copy makes a second structure on a value something still holds.
    Refuse(Refusal),
    /// The liveness pass never reached the copy — an IR shape it does not walk.  Reported rather
    /// than guessed, so a shape the pass misses cannot read as a move.
    Unreached,
}

/// Where the value a copy produces goes: the half of a copy's verdict that is read off the
/// surrounding code rather than off the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Into a new structure: a bind, a field, an element, a tuple member.
    Structure,
    /// Out of the function, through its return buffer.
    Return,
    /// Out of the block at this address, as its result.
    BlockResult(*const Value),
    /// Into a read through a temporary: nothing outlives the expression.
    ReadThrough,
}

/// One value a source expression may produce, classified: what the copy would duplicate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaf {
    /// A whole variable.
    Var(u16),
    /// A member of the container rooted at this variable.
    Member(u16),
    /// A `&` link to this variable.
    Link(u16),
    /// A call result or a literal: a fresh value nothing else holds.
    Fresh,
}

/// What the liveness pass found right after a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtSite {
    /// The pass never visited the copy.
    Unreached,
    /// No path uses the value after the copy.
    Dead,
    /// Some path uses it; the line of one such use.
    Live(u32),
}

/// The function a copy is judged in: its body, its variables, and the facts every verdict reads.
pub struct Frame<'a> {
    data: &'a Data,
    func: &'a Function,
    body: &'a Value,
    ops: Ops,
    /// The arguments the function FILLS rather than receives: the caller's return buffer, and a
    /// local promoted into it.  The caller holds none of them.
    buffers: HashSet<u16>,
    /// Every assignment in the body, as `(variable, the values it may be given)`.
    assignments: Vec<(u16, Vec<Leaf>)>,
}

impl<'a> Frame<'a> {
    #[must_use]
    pub fn new(data: &'a Data, def: &'a Definition) -> Self {
        let func = &def.variables;
        let ops = Ops::new(data);
        let buffers = (0..func.count())
            .filter(|&v| {
                func.is_argument(v)
                    && def
                        .attributes
                        .iter()
                        .any(|a| a.hidden && a.name == func.name(v))
            })
            .collect();
        let mut assignments = Vec::new();
        def.code.walk(&mut |n| {
            if let Value::Set(v, rhs) = n {
                assignments.push((*v, leaves(data, &ops, rhs)));
            }
        });
        Self {
            data,
            func,
            body: &def.code,
            ops,
            buffers,
            assignments,
        }
    }

    /// `(H-Copy-Refuse)`'s verdict on a copy of `src` whose value goes to `placement`, read off
    /// the line: nothing the program does after the copy changes it.
    ///
    /// Every value `src` may produce is judged — each arm of a join, the tail of a block — and the
    /// copy is a move only when every one of them is.
    #[must_use]
    pub fn written_verdict(&self, src: &Value, placement: Placement) -> Lease {
        let mut followed = HashSet::new();
        for leaf in leaves(self.data, &self.ops, src) {
            let verdict = self.written_leaf(leaf, placement, &mut followed);
            if verdict != Lease::Move {
                return verdict;
            }
        }
        Lease::Move
    }

    /// The author's variables a placement of `src` SPENDS: each value this function OWNS that
    /// `(H-Move)` moves into the new structure, whose name is spent from the end of the
    /// statement (`(H-Spent)`).  A leaf `written_verdict` refuses spends nothing — it is a copy,
    /// and `(H-Copy-Refuse)` reports it — and neither does a fresh value, a buffer, or a compiler
    /// temp, whose own sources are followed as `written_leaf` follows them.
    #[must_use]
    pub fn spends(&self, src: &Value, placement: Placement) -> Vec<u16> {
        let mut out = Vec::new();
        let mut followed = HashSet::new();
        for leaf in leaves(self.data, &self.ops, src) {
            self.spent_leaf(leaf, placement, &mut followed, &mut out);
        }
        out
    }

    fn spent_leaf(
        &self,
        leaf: Leaf,
        placement: Placement,
        followed: &mut HashSet<u16>,
        out: &mut Vec<u16>,
    ) {
        let Leaf::Var(var) = leaf else {
            return;
        };
        if placement == Placement::ReadThrough {
            return;
        }
        // A buffer the compiler named is its own; a local promoted onto the return buffer keeps
        // the author's name, and the author's reads of it follow the move like any local's.
        let promoted = self.buffers.contains(&var) && !self.func.name(var).starts_with("__");
        if self.buffers.contains(&var) && !promoted {
            return;
        }
        if promoted {
            if self
                .data
                .type_owns_droppable_anywhere(self.func.tp(var).base())
                && !out.contains(&var)
            {
                out.push(var);
            }
            return;
        }
        if self.func.is_compiler_generated(var) && !self.func.is_argument(var) {
            if followed.insert(var) {
                for given in self.assigned(var) {
                    self.spent_leaf(given, placement, followed, out);
                }
            }
            return;
        }
        // The same branch order as `written_leaf`: only what reaches its final `Move` is owned.
        if self.written_leaf(Leaf::Var(var), placement, &mut HashSet::new()) == Lease::Move
            && self
                .data
                .type_owns_droppable_anywhere(self.func.tp(var).base())
            && !out.contains(&var)
        {
            out.push(var);
        }
    }

    /// `(H-Copy-Refuse)`'s verdict on a copy of the whole variable `var` whose value goes to
    /// `placement` — a bind `x = a`, or a whole-tuple copy the parser lowers into member copies.
    #[must_use]
    pub fn written_var_verdict(&self, var: u16, placement: Placement) -> Lease {
        self.written_leaf(Leaf::Var(var), placement, &mut HashSet::new())
    }

    /// The liveness answer on the copy at `site`, whose source expression is `src`.  `site` must be
    /// a node of the function's body (it is found by identity).
    #[must_use]
    pub fn liveness_verdict(&self, site: &Value, src: &Value) -> Lease {
        for leaf in leaves(self.data, &self.ops, src) {
            let verdict = match leaf {
                Leaf::Member(root) => Lease::Refuse(self.member_owner(self.resolve_view(root).0)),
                Leaf::Var(v) | Leaf::Link(v) => self.whole_var(site, v),
                Leaf::Fresh => Lease::Move,
            };
            if verdict != Lease::Move {
                return verdict;
            }
        }
        Lease::Move
    }

    /// The liveness answer on copying the whole variable `var` at `site`.
    #[must_use]
    pub fn liveness_var_verdict(&self, site: &Value, var: u16) -> Lease {
        self.whole_var(site, var)
    }

    /// The refusal a `return value` carries on its own, or `None`.
    ///
    /// Returning a parameter, or a place reached through one, copies nothing in the callee — the
    /// caller's bind of the result makes the copy — so the return is the site the rule is decided
    /// at (`(H-Copy-Refuse)`: the caller holds the parameter).  A compiler temp holding the result
    /// (`__ret_1 = a ?? d`) returns what it was assigned.  Any other returned value is judged at the
    /// copy into the return buffer, where the census finds it.
    #[must_use]
    pub fn return_refusal(&self, value: &Value) -> Option<Refusal> {
        let mut work = leaves(self.data, &self.ops, value);
        let mut followed = HashSet::new();
        while let Some(leaf) = work.pop() {
            let root = match leaf {
                Leaf::Var(v) if self.func.is_compiler_generated(v) && !self.func.is_argument(v) => {
                    if followed.insert(v) {
                        work.extend(self.assigned(v));
                    }
                    continue;
                }
                Leaf::Var(v) | Leaf::Member(v) | Leaf::Link(v) => v,
                Leaf::Fresh => continue,
            };
            let holder = self.member_container(root).unwrap_or(root);
            if self.caller_holds(holder) {
                return Some(Refusal::Caller(holder));
            }
        }
        None
    }

    /// The variable a compiler-generated view temp stands for, and whether the view goes through a
    /// member of it.
    ///
    /// The parser reads a tuple through such temps — `_tuphold_1 = inner` for a nested copy,
    /// `__ref_2 = u` for a destructure, `_tuphold_2 = t.2` for a member that is itself a tuple —
    /// and a copy out of the temp is a copy of what it views.  A variable the author wrote is its
    /// own answer: a user view is judged as the member view it is.
    #[must_use]
    pub fn resolve_view(&self, var: u16) -> (u16, bool) {
        let mut current = var;
        let mut through_member = false;
        let mut seen = HashSet::new();
        while self.func.is_compiler_generated(current)
            && !self.func.is_argument(current)
            && seen.insert(current)
        {
            let values = self.assigned(current);
            let next = match values.as_slice() {
                // A view of `x`: its type depends on `x` — or, for a tuple hold, on exactly the
                // member backings `x`'s type depends on (`__ref_3 = t` in a returned tuple).
                [Leaf::Var(x)]
                    if self.func.tp(current).depend().contains(x)
                        || (matches!(self.func.tp(current).base(), Type::Tuple(_))
                            && !self.func.tp(current).depend().is_empty()
                            && self.func.tp(current).depend() == self.func.tp(*x).depend()) =>
                {
                    *x
                }
                [Leaf::Member(x)] => {
                    through_member = true;
                    *x
                }
                _ => break,
            };
            current = next;
        }
        (current, through_member)
    }

    /// Is `var` one of the function's return buffers — an argument it fills, not one it receives?
    #[must_use]
    pub fn is_buffer(&self, var: u16) -> bool {
        self.buffers.contains(&var)
    }

    /// The name the AUTHOR wrote for the value a refusal names, or `None` when the refusal
    /// reaches only a compiler temp.
    ///
    /// A refusal's variable is not always one a reader can see.  [`Self::member_owner`] answers
    /// `Container(root)` with whatever root [`Self::member_container`] found, and for a lifted
    /// join arm or a call-result projection that root is a `__lift_N` — measured on the corpus
    /// 2026-09-17, four sites in `1506-a-call-result-projection-releases-once.loft` read
    /// `refuse:container:__lift_2`.  [`Self::resolve_view`] walks the view temps back to what
    /// they stand for; when the answer is still generated, the caller has no name to print and
    /// says so instead of printing that one.
    ///
    /// A PARAMETER is an author name even though it is an argument, which is why the argument
    /// test sits beside the generated one rather than inside it.
    #[must_use]
    pub fn author_name(&self, refusal: &Refusal) -> Option<&str> {
        let var = self.resolve_view(refusal.var()).0;
        (!self.func.is_compiler_generated(var) || self.func.is_argument(var))
            .then(|| self.func.name(var))
    }

    /// The rule read off the line, for one value a copy may produce.
    fn written_leaf(&self, leaf: Leaf, placement: Placement, followed: &mut HashSet<u16>) -> Lease {
        if placement == Placement::ReadThrough {
            return Lease::Move;
        }
        let var = match leaf {
            Leaf::Fresh => return Lease::Move,
            // A vector literal's value is a projection of its `__vdb_N` backing, and that store
            // is the literal's own storage, not a container something else owns: the value is
            // fresh (`member_container` says the same of a vector LOCAL's backing).
            Leaf::Member(root) if self.is_vector_backing(self.resolve_view(root).0) => {
                return Lease::Move;
            }
            Leaf::Member(root) => {
                return Lease::Refuse(self.member_owner(self.resolve_view(root).0));
            }
            Leaf::Link(v) => return Lease::Refuse(Refusal::Copied(v)),
            Leaf::Var(v) => v,
        };
        if self.buffers.contains(&var) {
            return Lease::Move;
        }
        // A compiler temp copies what it was given; a temp given nothing was built where it is.
        if self.func.is_compiler_generated(var) && !self.func.is_argument(var) {
            if !followed.insert(var) {
                return Lease::Move;
            }
            for given in self.assigned(var) {
                let verdict = self.written_leaf(given, placement, followed);
                if verdict != Lease::Move {
                    return verdict;
                }
            }
            return Lease::Move;
        }
        if let Some(container) = self.member_container(var) {
            return Lease::Refuse(self.member_owner(container));
        }
        if self.caller_holds(var) {
            return Lease::Refuse(Refusal::Caller(var));
        }
        if self.func.is_captured(var) {
            return Lease::Refuse(Refusal::Captured(var));
        }
        // What reaches here is a value this function OWNS: a buffer, a compiler temp, a member,
        // a parameter-held local and a captured variable have each been answered above.  `(H-Move)`
        // moves it wherever it is placed — its lifetime ENDS in the new structure, which releases
        // it, and the name is SPENT from the end of that statement `(H-Spent)`.
        //
        // The rule's `return` and block-result clauses need no test of their own any more: both
        // name a value the function owns, so both are this answer.  That is why `declared_in` and
        // the assignment count behind it went with the owner's 2026-09-17 ruling — a block yielding
        // its own variable was only ever a narrower way of saying what this line now says.
        //
        // Placing it TWICE, and reading the name afterwards, are the two errors `(H-Spent)` asks
        // for; both are one check, `crate::spent`, which reads this verdict through
        // [`Frame::spends`] and follows the body to the reads after the move.  This verdict stays
        // the copy question alone: a spent name placed again is refused there, as a read.
        Lease::Move
    }

    fn whole_var(&self, site: &Value, var: u16) -> Lease {
        if let Some(container) = self.member_container(var) {
            return Lease::Refuse(self.member_owner(container));
        }
        if self.caller_holds(var) {
            return Lease::Refuse(Refusal::Caller(var));
        }
        if self.func.is_captured(var) {
            return Lease::Refuse(Refusal::Captured(var));
        }
        match self.live_after(site, var) {
            AtSite::Unreached => Lease::Unreached,
            AtSite::Dead => Lease::Move,
            AtSite::Live(line) => Lease::Refuse(Refusal::Later { var, line }),
        }
    }

    /// Who holds a member of `root`: the caller when `root` is reached through a parameter, and
    /// otherwise the container itself.
    /// Is `var` a compiler temp given nothing but members of the tuple `whole` (its null
    /// initialiser aside) — the stash a nullable tuple member is copied through?
    #[must_use]
    pub fn stashes_member_of(&self, var: u16, whole: u16) -> bool {
        if !self.func.is_compiler_generated(var) || self.func.is_argument(var) {
            return false;
        }
        let given: Vec<Leaf> = self
            .assigned(var)
            .into_iter()
            .filter(|l| *l != Leaf::Fresh)
            .collect();
        !given.is_empty()
            && given
                .iter()
                .all(|l| matches!(l, Leaf::Member(x) if self.resolve_view(*x).0 == whole))
    }

    /// A vector's own backing store, which the compiler names `__vdb_N`.
    fn is_vector_backing(&self, var: u16) -> bool {
        self.func.is_compiler_generated(var) && self.func.name(var).starts_with("__vdb_")
    }

    fn member_owner(&self, root: u16) -> Refusal {
        let holder = self.member_container(root).unwrap_or(root);
        if self.caller_holds(holder) {
            Refusal::Caller(holder)
        } else {
            Refusal::Container(root)
        }
    }

    /// Does the caller hold `var`'s value: a parameter, or a local that holds the caller's record
    /// on every path?
    fn caller_holds(&self, var: u16) -> bool {
        (self.func.is_argument(var) && !self.buffers.contains(&var))
            || self.func.holds_caller_record(var)
    }

    /// The values `var` is assigned anywhere in the body.
    fn assigned(&self, var: u16) -> Vec<Leaf> {
        self.assignments
            .iter()
            .filter(|(v, _)| *v == var)
            .flat_map(|(_, l)| l.iter().copied())
            .collect()
    }

    /// The container `var` is a member view of: the root of a projection an assignment binds it
    /// to (a loop variable, a `match` binding, `x = s.h`).  A vector local's own backing store
    /// (`v = OpGetField(__vdb_1, …)`) is the local's storage, not a container it views.
    fn member_container(&self, var: u16) -> Option<u16> {
        self.assigned(var).into_iter().find_map(|leaf| match leaf {
            Leaf::Member(root) if root != var && !self.func.name(root).starts_with("__vdb") => {
                Some(root)
            }
            _ => None,
        })
    }

    /// `var` and every variable that views it, directly or through another view.
    fn readers_of(&self, var: u16) -> HashSet<u16> {
        let mut out = HashSet::from([var]);
        loop {
            let before = out.len();
            for (w, values) in &self.assignments {
                if out.contains(w) {
                    continue;
                }
                let views = values.iter().any(|leaf| match *leaf {
                    Leaf::Member(r) | Leaf::Link(r) => out.contains(&r),
                    Leaf::Var(r) => out.contains(&r) && self.func.tp(*w).depend().contains(&r),
                    Leaf::Fresh => false,
                });
                if views {
                    out.insert(*w);
                }
            }
            if out.len() == before {
                return out;
            }
        }
    }

    /// Is `var`'s value used after `site` on some path?
    fn live_after(&self, site: &Value, var: u16) -> AtSite {
        let mut pass = Pass {
            data: self.data,
            func: self.func,
            ops: self.ops,
            var,
            uses: self.readers_of(var),
            target: std::ptr::from_ref(site.unspan()),
            lines: HashMap::new(),
            loops: Vec::new(),
            found: AtSite::Unreached,
        };
        let mut line = 0;
        mark_lines(self.body, &mut line, &mut pass.lines);
        pass.before(self.body, None);
        pass.found
    }
}

/// The tuple variable a tuple literal copies WHOLE, or `None`.
///
/// `u = t` is lowered into one member copy per member — the same spelling a tuple literal of
/// members has — so the copy is recognised by its content: every member of ONE tuple, in order,
/// and nothing else.  Such a literal copies the tuple's whole value, which is judged as one
/// variable rather than as a member of a container.
#[must_use]
pub fn whole_tuple_source(func: &Function, rhs: &Value) -> Option<u16> {
    let Value::Tuple(items) = rhs.unspan() else {
        return None;
    };
    let mut base = None;
    for (i, item) in items.iter().enumerate() {
        let (b, idx) = member_read(item)?;
        if usize::from(idx) != i || base.is_some_and(|x| x != b) {
            return None;
        }
        base = Some(b);
    }
    let base = base?;
    match func.tp(base).base() {
        Type::Tuple(elms) if elms.len() == items.len() => Some(base),
        _ => None,
    }
}

/// The `(base, index)` a tuple literal item reads: a bare `t.i`, or the member-copy block the
/// parser builds around one (its `OpCopyRecord` source).
fn member_read(item: &Value) -> Option<(u16, u16)> {
    let mut found = None;
    item.walk(&mut |n| {
        if found.is_none()
            && let Value::TupleGet(b, i) = n
        {
            found = Some((*b, *i));
        }
    });
    found
}

/// Every value `src` may produce — each arm of a join, the tail of a block — classified.
fn leaves(data: &Data, ops: &Ops, src: &Value) -> Vec<Leaf> {
    let mut raw = Vec::new();
    source_leaves(src, &mut raw);
    raw.into_iter()
        .map(|leaf| {
            if let Some(root) = projection_root(leaf, data) {
                return Leaf::Member(root);
            }
            match leaf.unspan() {
                Value::Var(v) => Leaf::Var(*v),
                Value::Call(op, args) if *op == ops.create_stack => {
                    match args.first().map(Value::unspan) {
                        Some(Value::Var(v)) => Leaf::Link(*v),
                        _ => Leaf::Fresh,
                    }
                }
                _ => Leaf::Fresh,
            }
        })
        .collect()
}

fn source_leaves<'v>(src: &'v Value, out: &mut Vec<&'v Value>) {
    match src.unspan() {
        Value::If(_, then, els) => {
            source_leaves(then, out);
            source_leaves(els, out);
        }
        Value::Block(bl) => {
            if let Some(tail) = bl.operators.last() {
                source_leaves(tail, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(tail) = ops.last() {
                source_leaves(tail, out);
            }
        }
        other => out.push(other),
    }
}

/// The line of every variable read, by node identity, so a use can be reported where it is.
fn mark_lines(node: &Value, line: &mut u32, lines: &mut HashMap<*const Value, u32>) {
    if let Some(p) = node.span_pos() {
        *line = p.line;
    } else if let Value::Line(n) = node {
        *line = *n;
    }
    let inner = node.unspan();
    if matches!(inner, Value::Var(_) | Value::TupleGet(..)) {
        lines.insert(std::ptr::from_ref(inner), *line);
    }
    inner.for_each_child(&mut |c| mark_lines(c, line, lines));
}

/// The definition numbers the pass recognises, looked up once.
#[derive(Clone, Copy)]
struct Ops {
    copy_record: u32,
    database: u32,
    create_stack: u32,
    conv_bool_from_ref: u32,
    null_ref_sentinel: u32,
}

impl Ops {
    fn new(data: &Data) -> Self {
        Self {
            copy_record: data.def_nr("OpCopyRecord"),
            database: data.def_nr("OpDatabase"),
            create_stack: data.def_nr("OpCreateStack"),
            conv_bool_from_ref: data.def_nr("OpConvBoolFromRef"),
            null_ref_sentinel: data.def_nr("OpNullRefSentinel"),
        }
    }
}

struct Pass<'a> {
    data: &'a Data,
    func: &'a Function,
    ops: Ops,
    /// The variable whose value is followed; a `Set` of it is a kill.
    var: u16,
    /// The variables whose reads are uses of `var`'s value.
    uses: HashSet<u16>,
    target: *const Value,
    lines: HashMap<*const Value, u32>,
    /// Per enclosing loop, innermost last: `(live at its exit, live at its header)`.
    loops: Vec<(Option<u32>, Option<u32>)>,
    found: AtSite,
}

impl Pass<'_> {
    /// Liveness before `node`, given liveness after it.  `Some(line)` is live, with one use.
    fn before(&mut self, node: &Value, after: Option<u32>) -> Option<u32> {
        let node = node.unspan();
        if std::ptr::eq(node, self.target) {
            self.found = after.map_or(AtSite::Dead, AtSite::Live);
        }
        if self.is_scaffold(node) {
            return after;
        }
        match node {
            Value::Var(w) => self.read(*w, node).or(after),
            Value::TupleGet(base, _) => self.read(*base, node).or(after),
            Value::Set(w, rhs) => {
                let after_set = if *w == self.var { None } else { after };
                self.before(rhs, after_set)
            }
            Value::Call(op, args) => {
                if *op == self.ops.database
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == self.var)
                {
                    // An in-place rebuild: the variable is given a new record here.
                    return None;
                }
                self.seq(args, after)
            }
            Value::CallRef(w, args) => {
                let live = self.seq(args, after);
                self.read(*w, node).or(live)
            }
            Value::Block(bl) => self.seq(&bl.operators, after),
            Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => self.seq(ops, after),
            Value::If(cond, then, els) => {
                let t = self.before(then, after);
                let e = self.before(els, after);
                self.before(cond, t.or(e))
            }
            Value::Loop(bl) => self.fixpoint(after, |p, header| p.seq(&bl.operators, header)),
            Value::Iter(_, a, b, c) => self.fixpoint(after, |p, header| {
                let l = p.before(c, header);
                let l = p.before(b, l);
                p.before(a, l)
            }),
            Value::Break(n) => self.loop_ctx(*n).map_or(after, |(exit, _)| exit),
            Value::Continue(n) => self.loop_ctx(*n).map_or(after, |(_, header)| header),
            Value::Return(v) => self.before(v, None),
            Value::Drop(v) | Value::Yield(v) => self.before(v, after),
            Value::TuplePut(base, _, val) => {
                let live = self.read(*base, node).or(after);
                self.before(val, live)
            }
            _ => after,
        }
    }

    /// A read of `w`: live, with its line, when `w` reads `var`'s value.
    fn read(&self, w: u16, node: &Value) -> Option<u32> {
        self.uses.contains(&w).then(|| {
            self.lines
                .get(&std::ptr::from_ref(node))
                .copied()
                .unwrap_or(0)
        })
    }

    fn seq(&mut self, ops: &[Value], after: Option<u32>) -> Option<u32> {
        ops.iter()
            .rev()
            .fold(after, |live, op| self.before(op, live))
    }

    /// A loop's liveness at its header: the body falls through to the header, and the header is
    /// what the loop is entered at.  Boolean in whether it is live, so two rounds decide it.
    fn fixpoint(
        &mut self,
        exit: Option<u32>,
        body: impl Fn(&mut Self, Option<u32>) -> Option<u32>,
    ) -> Option<u32> {
        let mut header = None;
        loop {
            self.loops.push((exit, header));
            let entry = body(self, header);
            self.loops.pop();
            if entry.is_some() == header.is_some() {
                return entry.or(header);
            }
            header = entry;
        }
    }

    fn loop_ctx(&self, n: u16) -> Option<(Option<u32>, Option<u32>)> {
        let depth = self.loops.len().checked_sub(1 + usize::from(n))?;
        self.loops.get(depth).copied()
    }

    /// Is `node` the scope pass's own work — a release, a snapshot, a hand-off flag — rather than
    /// something the program does with a value?
    fn is_scaffold(&self, node: &Value) -> bool {
        match node {
            Value::Call(op, args) => {
                releases_first_arg(self.data, *op)
                    || (*op == self.ops.copy_record && self.names_prefix(args.get(1), "__disp_"))
                    || (*op == self.ops.database && self.names_prefix(args.first(), "__disp_"))
            }
            Value::Set(w, rhs) => {
                let name = self.func.name(*w);
                name.starts_with("__hoff_")
                    || name.starts_with("__disp_")
                    || matches!(rhs.unspan(), Value::Call(op, _) if *op == self.ops.null_ref_sentinel)
            }
            Value::If(cond, then, els) => {
                self.is_guard(cond)
                    && self.scaffold_or_null(then)
                    && self.scaffold_or_null(els)
                    && (self.has_release(then) || self.has_release(els))
            }
            Value::Block(bl) => {
                !bl.operators.is_empty()
                    && bl.operators.iter().all(|o| self.is_scaffold(o.unspan()))
            }
            Value::Insert(ops) => {
                !ops.is_empty() && ops.iter().all(|o| self.is_scaffold(o.unspan()))
            }
            _ => false,
        }
    }

    fn scaffold_or_null(&self, node: &Value) -> bool {
        let node = node.unspan();
        matches!(node, Value::Null) || self.is_scaffold(node)
    }

    /// A release guard: a null check of a reference, or a hand-off flag.
    fn is_guard(&self, cond: &Value) -> bool {
        match cond.unspan() {
            Value::Call(op, _) => *op == self.ops.conv_bool_from_ref,
            Value::Var(w) => self.func.name(*w).starts_with("__hoff_"),
            _ => false,
        }
    }

    /// Does `node` contain a release or a snapshot — the part of a scaffold the author cannot
    /// write?  An `if` over nothing but nulls is the author's own null check.
    fn has_release(&self, node: &Value) -> bool {
        node.any_node(&mut |n| {
            matches!(n, Value::Call(op, args) if releases_first_arg(self.data, *op)
                || (*op == self.ops.copy_record && self.names_prefix(args.get(1), "__disp_")))
        })
    }

    fn names_prefix(&self, arg: Option<&Value>, prefix: &str) -> bool {
        matches!(arg.map(Value::unspan), Some(Value::Var(v)) if self.func.name(*v).starts_with(prefix))
    }
}
