// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! Which rule OWNS a per-site buffer (`__vdb_N`, the store a literal or a local is built in) —
//! the front's one answer, so no rule that wants a buffer works out the others' claims on its
//! own (`formal/rewrites.md` `(R-Phase)`, ONE PLACE).  A buffer has at most one owner, by this
//! priority: the rule that removes the most work at the site wins.
//!
//! 1. `@FR-R-Apart` — its inline buffer (a vector read at constant indices kept out of the store
//!    entirely; its companions are the buffers that would have backed it) and its apart RECORD
//!    owners (instance 4: a record local held as its fields, no store at all).
//! 2. `@FR-R-LitHoist` — a literal built ONCE: the witness of a flat build group, and every
//!    buffer a one-`Set` literal's value is built from (the hoist reads their plain
//!    declarations as invariant).
//! 3. `@FR-R-ElemFirst` — a temp built inside its appended element (the pairs the writers
//!    still apply; the IR's own pairs no longer show a buffer).
//! 4. `@FR-R-MoveAppend` — a move host.
//! 5. `@FR-R-RetAdopt` — the adopted result's witness.
//!
//! A buffer or record local none of them owns is free for `@FR-R-LoopBuffer` and
//! `@FR-R-LoopRecord`, the rules that ask.  The apart-record plan is the whole program's
//! (`opt::apart_records::plan`): a caller computes it once and hands it to [`owners`].
//! Each owner is asked of the IR the writers will see, so the answer is theirs too.
use crate::data::{Data, Value};
use crate::database::Stores;
use crate::opt::hoist;
use std::collections::{HashMap, HashSet};

/// The rule that owns a buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    Apart,
    ApartRecord,
    LitHoist,
    ElemFirst,
    MoveHost,
    RetAdopt,
}

impl Owner {
    /// The owner as a trace names it.
    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Owner::Apart => "an inline buffer's companion",
            Owner::ApartRecord => "an apart record",
            Owner::LitHoist => "an invariant literal's",
            Owner::ElemFirst => "an element-first pair's",
            Owner::MoveHost => "a move host",
            Owner::RetAdopt => "the adopted result's witness",
        }
    }
}

/// Every owned buffer and record local of function `d`, with its one owner (the module's
/// priority); `apr` is the program's apart-record plan.
#[must_use]
pub fn owners(
    data: &Data,
    stores: &Stores,
    d: u32,
    apr: &crate::opt::apart_records::RecPlan,
) -> HashMap<u16, Owner> {
    let def = data.def(d);
    let mut out: HashMap<u16, Owner> = HashMap::new();
    let mut claim = |v: u16, o: Owner| {
        out.entry(v).or_insert(o);
    };
    if let Some(apart) = crate::opt::apart_vector::roles(data, d) {
        for v in apart.apart.iter().chain(apart.companions.iter()) {
            claim(*v, Owner::Apart);
        }
    }
    if let Some(recs) = apr.owners.get(&d) {
        for v in recs.keys() {
            claim(*v, Owner::ApartRecord);
        }
    }
    let lits = hoist::invariant_literals(data, d);
    for v in lits.flat.keys() {
        claim(*v, Owner::LitHoist);
    }
    let mut in_hoisted: HashSet<u16> = HashSet::new();
    def.code().walk(&mut |n| {
        if let Value::Set(w, init) = n
            && lits.wrapped.contains(w)
        {
            init.walk(&mut |m| {
                if let Value::Var(x) = m {
                    in_hoisted.insert(*x);
                }
            });
        }
    });
    for v in in_hoisted {
        claim(v, Owner::LitHoist);
    }
    for v in hoist::element_first(data, stores, d, None).by_vdb.keys() {
        claim(*v, Owner::ElemFirst);
    }
    for p in hoist::move_appends(data, d).values() {
        claim(p.host_vdb, Owner::MoveHost);
    }
    if let Some(a) = hoist::ret_adopt(data, d) {
        claim(a.vdb, Owner::RetAdopt);
    }
    out
}
