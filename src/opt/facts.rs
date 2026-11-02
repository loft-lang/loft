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
        }
    }

    /// A walk over the stores-written memo its owner keeps; the other memos start empty.
    pub fn over(stores_written: &'m mut HashMap<u32, bool>) -> Facts<'m> {
        Facts {
            stores_written: Memo::Borrowed(stores_written),
            writes: Memo::Own(WriteCache::default()),
            inputs: None,
            active: HashSet::new(),
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
