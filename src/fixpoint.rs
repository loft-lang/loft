// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — shared by the native Rust and the C99 generators

//! `@FR-Fact-Reach` CYCLES `solve` — the whole-program fixpoint of a fact over an ordered list
//! of candidates.  A core module: the effect-free and total analyses (`pure_reuse`,
//! `shared_call`) live in the core layer and run on it; `opt::facts` re-exports it.

use std::collections::HashSet;

/// The direction of a [`solve`] (`@FR-Fact-Reach` CYCLES `solve`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fixpoint {
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
pub fn solve<K, S>(
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
