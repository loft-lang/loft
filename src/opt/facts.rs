// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-Fact-Engine` — the state the cross-call analyses keep while they look into callees:
//! the memo each one reads, and the cycle guard of the writes family.
//!
//! One handle, [`Facts`], carries it through every walk instead of a map per parameter.  Each
//! memo lives where its owner keeps it — the C writer keeps the stores-written answers per
//! function body and the write sets and callee inputs per program, native keeps all three on
//! its writer — so the handle BORROWS a memo from its owner, or OWNS a fresh one where a walk
//! starts with an empty memo.  Either way the scope of every memo is the scope it had before
//! the handle existed; `doc/claude/plans/210-c-worst-to-2x/facts-engine.md` § The order.

use crate::data::{Data, Definition, Value};
use crate::opt::hoist::{InputCache, WriteCache};
use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};

/// A memo the handle owns (a walk that starts empty) or borrows from the writer that keeps it.
pub enum Memo<'m, T> {
    Own(T),
    Borrowed(&'m mut T),
}

impl<T> Deref for Memo<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Memo::Own(t) => t,
            Memo::Borrowed(t) => t,
        }
    }
}

impl<T> DerefMut for Memo<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        match self {
            Memo::Own(t) => t,
            Memo::Borrowed(t) => t,
        }
    }
}

/// The state of the cross-call analyses for one walk.
pub struct Facts<'m> {
    /// `call_writes_store`'s answers, and `in_place_only_writer`'s under `IN_PLACE_KEY`.
    pub(crate) stores_written: Memo<'m, HashMap<u32, bool>>,
    /// `callee_writes`' write sets.
    pub(crate) writes: Memo<'m, WriteCache>,
    /// `callee_inputs`' answers; `None` where the walk asks for no callee inputs.
    pub(crate) inputs: Option<Memo<'m, InputCache>>,
    /// The writes family's cycle guard: the functions whose answer is being worked out.  A
    /// question about one of them answers the kind's cycle answer.
    pub(crate) active: HashSet<u32>,
    /// The callee-inputs kind's own cycle guard.
    inputs_guard: HashSet<u32>,
    /// The buffer-only kind's cycle guard, and its memo, which its policy keeps empty.
    buffer_guard: HashSet<u32>,
    pub(crate) buffer_only: HashMap<u32, bool>,
}

/// Where an answer came from — what a kind's memo policy may tell apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Origin {
    /// A function with no loft body, answered from its declaration.
    Native,
    /// A function already under evaluation in the kind's cycle scope.
    Cycle,
    /// The kind's walk of the body.
    Body,
}

/// The cycle guard a kind's questions share (`@FR-Fact-Kind` (g)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Scope {
    /// `call_writes_store`, `in_place_only_writer`, `callee_writes`: one guard, so a question
    /// about a function another of them is evaluating gets the cycle answer.
    Writes,
    /// `callee_inputs`, alone.
    Inputs,
    /// `callee_writes_only_its_buffer`, alone.
    BufferOnly,
}

/// One kind of fact about a call (`@FR-Fact-Kind`): what it answers and how; [`Facts::ask`]
/// does the rest.  A kind's `body` is its walk of the function, every question about a callee
/// in it asked through `ask` again.
pub(crate) trait Kind {
    type Fact: Clone;
    /// What a question carries beside the function (the emitter's owned buffers, re-aimed at
    /// each callee by the body that asks).
    type Ctx: ?Sized;
    const SCOPE: Scope;
    /// The memo the kind keeps its answers in, and the key a function's answer has there.
    fn memo<'a>(&self, fx: &'a mut Facts<'_>) -> &'a mut HashMap<u32, Self::Fact>;
    fn key(&self, d: u32) -> u32 {
        d
    }
    /// A number that names no definition.
    fn unknown(&self) -> Self::Fact;
    /// A function with no loft body (NATIVES `declared`).
    fn native(&self, def: &Definition) -> Self::Fact;
    /// A function already under evaluation in the kind's scope (CYCLES `cut`).
    fn on_cycle(&self) -> Self::Fact;
    /// The memo policy: whether this answer, from `origin`, asked with `ctx`, is kept.
    fn keep(&self, origin: Origin, fact: &Self::Fact, ctx: Option<&Self::Ctx>) -> bool;
    fn body(&self, fx: &mut Facts, data: &Data, d: u32, ctx: Option<&Self::Ctx>) -> Self::Fact;
    /// Every answer computed (not one read from the memo): the rewrite census counts here.
    fn observe(&self, _fact: &Self::Fact) {}
}

impl<'m> Facts<'m> {
    /// A walk that starts with every memo empty.
    #[must_use]
    pub fn fresh() -> Facts<'static> {
        Facts {
            stores_written: Memo::Own(HashMap::new()),
            writes: Memo::Own(WriteCache::default()),
            inputs: None,
            active: HashSet::new(),
            inputs_guard: HashSet::new(),
            buffer_guard: HashSet::new(),
            buffer_only: HashMap::new(),
        }
    }

    /// A walk over the stores-written memo its owner keeps; the other memos start empty.
    pub fn over(stores_written: &'m mut HashMap<u32, bool>) -> Facts<'m> {
        Facts {
            stores_written: Memo::Borrowed(stores_written),
            writes: Memo::Own(WriteCache::default()),
            inputs: None,
            active: HashSet::new(),
            inputs_guard: HashSet::new(),
            buffer_guard: HashSet::new(),
            buffer_only: HashMap::new(),
        }
    }

    /// A walk over all three memos its owner keeps.
    pub fn with(
        stores_written: &'m mut HashMap<u32, bool>,
        writes: &'m mut WriteCache,
        inputs: Option<&'m mut InputCache>,
    ) -> Facts<'m> {
        Facts {
            stores_written: Memo::Borrowed(stores_written),
            writes: Memo::Borrowed(writes),
            inputs: inputs.map(Memo::Borrowed),
            active: HashSet::new(),
            inputs_guard: HashSet::new(),
            buffer_guard: HashSet::new(),
            buffer_only: HashMap::new(),
        }
    }

    /// `@FR-Fact-Engine` — kind `k`'s answer for function `d`: the memo's when it has one;
    /// otherwise the kind's unknown, native or cycle answer, or its walk of the body under the
    /// kind's cycle guard — counted (`Kind::observe`) and kept as the kind's policy says.
    pub(crate) fn ask<K: Kind>(
        &mut self,
        k: &K,
        data: &Data,
        d: u32,
        ctx: Option<&K::Ctx>,
    ) -> K::Fact {
        if (d as usize) >= data.definitions.len() {
            return k.unknown();
        }
        if let Some(f) = k.memo(self).get(&k.key(d)) {
            return f.clone();
        }
        let def = data.def(d);
        let (origin, fact) = if matches!(def.code(), Value::Null) {
            (Origin::Native, k.native(def))
        } else if !self.guard(K::SCOPE).insert(d) {
            (Origin::Cycle, k.on_cycle())
        } else {
            let f = k.body(self, data, d, ctx);
            self.guard(K::SCOPE).remove(&d);
            (Origin::Body, f)
        };
        k.observe(&fact);
        if k.keep(origin, &fact, ctx) {
            k.memo(self).insert(k.key(d), fact.clone());
        }
        fact
    }

    fn guard(&mut self, s: Scope) -> &mut HashSet<u32> {
        match s {
            Scope::Writes => &mut self.active,
            Scope::Inputs => &mut self.inputs_guard,
            Scope::BufferOnly => &mut self.buffer_guard,
        }
    }

    /// The callee-inputs memo of a walk that keeps one (`hoistable` asks only when it does).
    pub(crate) fn inputs(&mut self) -> &mut InputCache {
        self.inputs
            .as_deref_mut()
            .expect("callee inputs asked of a walk that keeps none")
    }

    /// Run `f` with an EMPTY cycle guard and put the current one back after: the walks that
    /// start a guard of their own (a top-level question asked from inside another walk) keep
    /// doing so.
    pub(crate) fn fresh_guard<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = std::mem::take(&mut self.active);
        let r = f(self);
        self.active = saved;
        r
    }
}

/// The direction of a [`solve`] (`@FR-Fact-Reach` CYCLES `solve`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fixpoint {
    /// A MUST fact, or the complement of a MAY one: every candidate assumed, each round
    /// dropping those that do not hold against the assumption, until none drops.
    Greatest,
    /// Every candidate left out, each round adding those that hold against what is in,
    /// until none is added — so a candidate that holds only by assuming itself never joins.
    Least,
}

/// `@FR-Fact-Reach` CYCLES `solve` — the whole-program fixpoint of a fact over `cands`.  Each
/// round asks `holds` of every candidate still to decide against the set as the ROUND found
/// it, in `cands`' order, and applies the changes together, so a round's answer does not
/// depend on the order inside it.  `holds` may see the same set several times in a row: a
/// round's set is the previous round's with that round's changes applied.
pub(crate) fn solve<K, S>(
    cands: &[K],
    dir: Fixpoint,
    mut holds: impl FnMut(K, &HashSet<K, S>) -> bool,
) -> HashSet<K, S>
where
    K: Copy + Eq + std::hash::Hash,
    S: std::hash::BuildHasher + Default,
{
    let mut set: HashSet<K, S> = match dir {
        Fixpoint::Greatest => cands.iter().copied().collect(),
        Fixpoint::Least => HashSet::with_hasher(S::default()),
    };
    loop {
        let flip: Vec<K> = cands
            .iter()
            .copied()
            .filter(|k| match dir {
                Fixpoint::Greatest => set.contains(k) && !holds(*k, &set),
                Fixpoint::Least => !set.contains(k) && holds(*k, &set),
            })
            .collect();
        if flip.is_empty() {
            return set;
        }
        for k in flip {
            match dir {
                Fixpoint::Greatest => set.remove(&k),
                Fixpoint::Least => set.insert(k),
            };
        }
    }
}
