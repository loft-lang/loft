// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! A store-backed binary PATRICIA (radix) tree, keyed one bit at a time.
//!
//! Use it when records must stay in **key order** and the key is best produced
//! lazily rather than materialised: the tree never asks for a whole key, only for
//! one bit of it.  That is what lets a spatial index interleave several coordinate
//! axes into a Morton code on demand (@PLN48) without ever building the code.
//!
//! The one rule every operation rests on:
//!
//! > **I1** — each internal node stores an *absolute* bit index `bit(n)`; along any
//! > root→leaf path `bit` strictly increases; every record below `n`'s FALSE child
//! > has `key(bit(n)) = 0`, and every record below its TRUE child has `1`.
//!
//! Descending on `key(bit(n))` therefore lands on the single leaf sharing the
//! longest prefix with the search key.  Bits that path compression skipped are
//! never re-checked, so that leaf is a **candidate**: [`rtree_find`] returns it and
//! the caller compares keys, or [`rtree_get`] does the comparing.  In-order
//! traversal (FALSE subtree, then TRUE) emits records in strictly increasing key
//! order.
//!
//! Because `bit(n)` is absolute, no operation accumulates "bits consumed so far"
//! while descending — the class of off-by-one errors that representation invites
//! cannot be written here.  The bits a node *skips* are never stored: the run
//! length is `bit(n) - bit(parent) - 1`, and the values can be read off any leaf
//! below `n`, because every record there agrees on them (**I2**).
//!
//! # The key the tree actually uses
//!
//! A caller supplies a [`KeySpec`]: bit `b` of a record's key, and how many bits
//! that key has.  The tree keys on neither directly.  It keys on the **infinite bit
//! string**
//!
//! ```text
//!     user bits ‖ 0x00 terminator ‖ 32-bit record id ‖ zeros forever
//! ```
//!
//! which is what makes the structure total rather than fragile:
//!
//! * **No key ever ends,** so a comparison never has to decide what "one key
//!   stopped" means.  Prefix-freeness is not an obligation; it is a consequence.
//! * **Distinct records always differ,** because their ids do.  So there is always
//!   a bit to split on, and insertion cannot fail.
//! * **Order is lexicographic.**  The terminator is what buys this, not the
//!   prefix-freeness: without it, `"ab" ‖ id` versus `"abc" ‖ id` would compare a
//!   record id against `'c'`, so which sorted first would depend on allocation.
//!   `0x00` is below every UTF-8 byte, so `"ab"` precedes `"abc"` always.
//! * **A probe key is the same string with id `0`,** so it sorts to the head of its
//!   own bucket.  [`rtree_seek`] on `"ab"` therefore lands on the first key with
//!   prefix `"ab"` — a prefix query needs no separate entry point.
//! * **Fixed-width keys pay nothing for the suffix.**  Every Morton code has the
//!   same length, so no two records ever diverge inside the terminator; path
//!   compression means no node is created there.  One rule, no modes.
//!
//! Two records may share a user key — several entities in one spatial cell.  They
//! differ only in the id suffix, so they land **adjacent** in key order and a
//! "bucket" is just a contiguous run.  No bucket structure exists or is needed.
//!
//! The single assumption: **no key byte is `0x00`**, true for UTF-8 text and vacuous
//! for fixed-width numeric keys.  Were it violated, the ids still differ, so the
//! tree stays a structurally valid PATRICIA; only lexicographic order would
//! disagree with key order for that pair.  It degrades, it does not corrupt — and
//! nothing here panics on any input.
//!
//! # Two storages, one walk
//!
//! The read-only half of the tree — [`descend_gen`], [`split_point_gen`],
//! [`descend_extreme_gen`], [`RadixIter::step_gen`] and [`seek_gen`], plus
//! [`composed_bit`] and [`first_diff_words`] — is written against a [`TreeNodes`] /
//! [`TreeKeys`] source rather than a `Store` directly.  A resident tree passes
//! [`StoreNodes`]/[`StoreKeys`] and reads exactly as before; a PAGED reader over a
//! persisted image passes its own source and answers a node by fetching (@PLN134).
//! Mutation is not expressible through the trait and stays on `&mut Store`.
//!
//! The point is that the seek algorithm and the key composition have ONE home.  A
//! second reader with its own copy of them is a reader that can silently descend to
//! a different leaf, and nothing about the two answers would say which is right.
//!
//! Design and the step-by-step verification plan:
//! `doc/claude/plans/48-spacial-index/RADIX_TREE.md`.

// The tree is exercised only by its own tests until @PLN48 deliverable S2 wires it
// to `spatial<T[…]>`; the unit tests are what hold it correct in the meantime.
#![allow(dead_code)]

use crate::store::Store;
use std::cmp::Ordering;

/// 64 bits of `rec`'s user key starting at bit `word * 64`, most significant first
/// (bit `word*64` lands in bit 63 of the `u64`).  Zero-padded past the key's end.
///
/// A word accessor rather than a bit accessor because the comparison hot path is
/// `first_diff`: one `XOR` plus `leading_zeros` replaces up to 64 indirect calls.
/// The per-bit read is *derived* from this, so there is no second accessor to keep
/// consistent with it.
pub type WordFn = fn(store: &Store, rec: u32, word: u32) -> u64;

/// How many bits `rec`'s user key has.  A constant for fixed-width keys; `8 * len`
/// for text.
pub type LenFn = fn(store: &Store, rec: u32) -> u32;

/// How to read a record's key.  The tree extends it with a terminator and the
/// record id; see the module header.
#[derive(Clone, Copy)]
pub struct KeySpec {
    pub word: WordFn,
    pub bits: LenFn,
}

/// How the tree reads a record's key.  Every `rtree_*` operation is generic over this,
/// so a caller may key on anything — a fixed `KeySpec` of `fn` pointers (the tests and
/// `spatial::MORTON2D`), or a value that carries runtime state, such as the list of
/// coordinate fields a `spatial<T[…]>` schema discovers.  The bound is `Copy`, met by
/// `KeySpec` and by a shared reference, so the database passes `&its_oracle`.
pub trait KeyOracle {
    /// 64 key bits of `rec` starting at bit `word * 64`; see [`WordFn`].
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64;
    /// How many bits `rec`'s user key has; see [`LenFn`].
    fn bits(&self, store: &Store, rec: u32) -> u32;
}

impl KeyOracle for KeySpec {
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        (self.word)(store, rec, word)
    }
    fn bits(&self, store: &Store, rec: u32) -> u32 {
        (self.bits)(store, rec)
    }
}

impl<T: KeyOracle> KeyOracle for &T {
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        (*self).word(store, rec, word)
    }
    fn bits(&self, store: &Store, rec: u32) -> u32 {
        (*self).bits(store, rec)
    }
}

/// The zero byte that separates a user key from the id suffix, so a shorter key
/// sorts before a longer one that extends it.
pub(crate) const TERM_BITS: u32 = 8;
/// The record id, which makes every key unique.
pub(crate) const ID_BITS: u32 = 32;
/// Everything the tree appends to a user key.
pub(crate) const SUFFIX_BITS: u32 = TERM_BITS + ID_BITS;

// Container-record header, in bytes.  `fld 0..4` is the Store's own claim header.
//
// `pub(crate)` because a PAGED reader walks the very same bytes out of a
// persisted image (`paged_reader::trie_prefix_recs`), and a second copy of these
// offsets is a second thing to keep in step: the tree writes them, so the tree is
// where they live (@PLN134).
/// Root child: `0` empty, `>0` a record id, `<0` a node id.
pub(crate) const TOP: u32 = 4;
/// Number of records held.
pub(crate) const LEN: u32 = 8;
/// Node high-water mark; live ids are drawn from `1..=NODES`.
pub(crate) const NODES: u32 = 12;
/// How many nodes the current claim can hold.
const CAP: u32 = 16;
/// Head of the free-node list, `0` when empty.
const FREE: u32 = 20;
/// First node sits here; a multiple of 8, so nodes stay word-aligned.
const HDR: u32 = 24;
/// `bit: u32`, `parent: u32`, `false: i32`, `true: i32` — exactly two store words.
const NODE_SIZE: u32 = 16;

/// One key as the tree sees it — the infinite bit string of the module header.
///
/// A **record** view carries the record id, so its keys are unique.  A **probe** view
/// carries id `0`, which is what sorts a probe to the head of its own bucket.  The
/// two are the same type, so `first_diff` needs no probe-versus-record special case.
struct View<F: Fn(u32) -> u64> {
    word: F,
    bits: u32,
    id: u32,
}

impl<F: Fn(u32) -> u64> View<F> {
    /// Bit `b` of the composed string: user bits, terminator, record id, then zeros.
    fn bit(&self, b: u32) -> bool {
        composed_bit(&self.word, self.bits, self.id, b)
    }

    /// Bits beyond which the string is all zeros.
    fn total(&self) -> u32 {
        self.bits + SUFFIX_BITS
    }
}

/// Bit `b` of the composed string — user bits, terminator, record id, then zeros —
/// read out of a key that supplies whole 64-bit words.
///
/// The composition rule itself, with no view around it, because a second reader of
/// the same tree has to compose the string EXACTLY the same way or it descends to a
/// different leaf.  A paged walk over a persisted image is such a reader
/// (`paged_reader`, @PLN134), and this is the one place the rule is written.
pub(crate) fn composed_bit(word: impl FnOnce(u32) -> u64, bits: u32, id: u32, b: u32) -> bool {
    if b < bits {
        (word(b / 64) >> (63 - b % 64)) & 1 == 1
    } else if b < bits + TERM_BITS {
        false
    } else if b < bits + SUFFIX_BITS {
        (id >> (ID_BITS - 1 - (b - bits - TERM_BITS))) & 1 == 1
    } else {
        false
    }
}

/// First bit at which two key strings differ, and the first one's value there.
///
/// Over the stretch both keys share, this compares **64 bits at a time**: one `XOR`
/// and one `leading_zeros` locate the differing bit.  Only the tail — past the
/// shorter user key, where terminator and id live — is walked bit by bit, and that
/// happens solely when two keys agree over their whole common length.
///
/// `None` means they agree on every bit.  For two *distinct* records that cannot
/// happen, since their id suffixes differ; every caller reads `None` as "the same
/// key", never as a failure.
fn first_diff<A, B>(a: &View<A>, b: &View<B>) -> Option<(u32, bool)>
where
    A: Fn(u32) -> u64,
    B: Fn(u32) -> u64,
{
    first_diff_words(a.bits, a.id, |w| (a.word)(w), b.bits, b.id, |w| (b.word)(w))
}

/// [`first_diff`] over two keys given as `(bits, id, word accessor)` rather than as
/// two [`View`]s.
///
/// The accessors are `FnMut` for one reason: a reader that FETCHES — a paged image —
/// mutates itself to answer a word, so the closure that reads a record's key over it
/// has to hold `&mut`.  A `Fn` bound compiles for the resident tree and quietly
/// excludes the other reader, which would then need its own copy of the comparison —
/// the piece most worth having exactly one of.
pub(crate) fn first_diff_words<A, B>(
    a_bits: u32,
    a_id: u32,
    mut a_word: A,
    b_bits: u32,
    b_id: u32,
    mut b_word: B,
) -> Option<(u32, bool)>
where
    A: FnMut(u32) -> u64,
    B: FnMut(u32) -> u64,
{
    let common = a_bits.min(b_bits);
    let mut w = 0;
    while w * 64 < common {
        let (av, bv) = (a_word(w), b_word(w));
        let x = av ^ bv;
        if x != 0 {
            let bit = w * 64 + x.leading_zeros();
            if bit < common {
                return Some((bit, composed_bit(&mut a_word, a_bits, a_id, bit)));
            }
            // The difference lies past the shorter key; the tail walk settles it.
            break;
        }
        w += 1;
    }
    let limit = (a_bits + SUFFIX_BITS).max(b_bits + SUFFIX_BITS);
    (common..limit).find_map(|bit| {
        let x = composed_bit(&mut a_word, a_bits, a_id, bit);
        (x != composed_bit(&mut b_word, b_bits, b_id, bit)).then_some((bit, x))
    })
}

/// A child slot: the sign encoding lives here and nowhere else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Child {
    Empty,
    Rec(u32),
    Node(u32),
}

impl Child {
    pub(crate) fn decode(v: i32) -> Child {
        match v.cmp(&0) {
            Ordering::Equal => Child::Empty,
            Ordering::Greater => Child::Rec(v as u32),
            Ordering::Less => Child::Node(v.unsigned_abs()),
        }
    }

    fn encode(self) -> i32 {
        match self {
            Child::Empty => 0,
            Child::Rec(r) => r as i32,
            Child::Node(n) => -(n as i32),
        }
    }
}

/// A cursor: the record under it, and the node above that record (`0` at the root).
///
/// It holds **no path** — climbing runs on the parent index in each node — so a
/// cursor is three words, `Copy`, and costs no allocation.  That is what lets one
/// seek drive two cursors, walking predecessor and successor outward together:
/// @PLN48's proximity query, with no malloc in the frame loop.
///
/// `budget` is what keeps a corrupted tree from hanging its caller.  A walk over
/// `LEN` records cannot legitimately yield more than `LEN` of them, so a cursor that
/// tries simply stops.  Bounding the parent *climb* is not enough: a stale parent
/// leaves the child pointers a valid tree, so `next` keeps returning records and it
/// is the caller's loop that never ends.  Every intended walk is monotone —
/// iteration, and the outward proximity scan — so the bound never fires on a healthy
/// tree.  It degrades; it does not spin, and it does not panic.
#[derive(Clone, Copy)]
pub struct RadixIter {
    rec: u32,
    node: u32,
    budget: u32,
}

impl RadixIter {
    fn empty() -> RadixIter {
        RadixIter {
            rec: 0,
            node: 0,
            budget: 0,
        }
    }

    /// A cursor at `rec` (whose parent is `node`), allowed to walk the whole tree.
    fn at(store: &Store, tree: u32, rec: u32, node: u32) -> RadixIter {
        RadixIter::at_gen(&mut StoreNodes { store, tree }, rec, node)
    }

    /// [`RadixIter::at`] over any storage.
    pub(crate) fn at_gen<N: TreeNodes>(src: &mut N, rec: u32, node: u32) -> RadixIter {
        RadixIter {
            rec,
            node,
            budget: src.len() + 1,
        }
    }

    /// The record under the cursor; `0` once the walk is exhausted.
    #[must_use]
    pub fn rec(&self) -> u32 {
        self.rec
    }

    /// Step to the next record in increasing key order.
    ///
    /// Climb the parent chain to the deepest ancestor entered from its FALSE side,
    /// cross to its TRUE child, then take the FALSE-most path down — the in-order
    /// successor.  Amortised O(1) over a full traversal.
    pub fn next(&mut self, store: &Store, tree: u32) -> Option<u32> {
        self.step(store, tree, true)
    }

    /// Step to the previous record, mirroring [`RadixIter::next`].
    pub fn prev(&mut self, store: &Store, tree: u32) -> Option<u32> {
        self.step(store, tree, false)
    }

    fn step(&mut self, store: &Store, tree: u32, forward: bool) -> Option<u32> {
        self.step_gen(&mut StoreNodes { store, tree }, forward)
    }

    /// [`RadixIter::step`] over any storage — the in-order successor/predecessor.
    pub(crate) fn step_gen<N: TreeNodes>(&mut self, src: &mut N, forward: bool) -> Option<u32> {
        src.walk_begin();
        if self.rec == 0 || self.budget == 0 {
            self.rec = 0;
            return None;
        }
        self.budget -= 1;
        let mut from = Child::Rec(self.rec);
        let mut n = self.node;
        while n != 0 {
            // Which side did we come up from?  The children are distinct, so one
            // comparison settles it.
            let came_true = src.child(n, true) == from;
            if came_true != forward {
                let c = src.child(n, forward);
                let (rec, node) = descend_extreme_gen(src, c, !forward, n);
                self.rec = rec;
                self.node = node;
                return Some(rec);
            }
            from = Child::Node(n);
            n = src.node_parent(n);
        }
        self.rec = 0;
        self.node = 0;
        None
    }
}

// ---------------------------------------------------------------------------
// Container + node accessors
// ---------------------------------------------------------------------------

pub(crate) fn node_off(node: u32) -> u32 {
    debug_assert!(node >= 1, "node ids are 1-based");
    HDR + NODE_SIZE * (node - 1)
}

// @PLN134 — the byte offsets a traversal READ, so a measurement can count the
// PAGES behind them.
//
// A descent is cheap in NODES (one root→leaf path) and that says nothing about
// what it costs over a link, because a reader fetches 64 KB pages and node ids
// are handed out in insertion order. Nodes are a contiguous array here
// (`node_off`), so the two questions have different answers and only the page
// one decides whether a trie can be paged.
//
// `#[cfg(test)]`, so production keeps the bare `get_u32_raw`. Off unless a
// measurement calls `touch_begin`; recording is what makes it cost anything.
#[cfg(test)]
thread_local! {
    static TOUCHED: std::cell::RefCell<Option<std::collections::BTreeSet<u32>>> =
        const { std::cell::RefCell::new(None) };
}

/// Start recording touched offsets, discarding anything from a previous window.
#[cfg(test)]
pub(crate) fn touch_begin() {
    TOUCHED.with(|t| *t.borrow_mut() = Some(std::collections::BTreeSet::new()));
}

/// The distinct offsets read since [`touch_begin`], and stop recording.
#[cfg(test)]
pub(crate) fn touch_end() -> std::collections::BTreeSet<u32> {
    TOUCHED.with(|t| t.borrow_mut().take().unwrap_or_default())
}

/// @PLN134 — node ids in BREADTH-FIRST order, so a measurement can ask what a
/// different LAYOUT would cost without building one.
///
/// Ids are handed out in INSERTION order today, which is why a root→leaf path is
/// scattered across the whole node array. BFS is the cheapest reordering that puts
/// a path's early hops together, and every query shares those hops — so this
/// answers "is the scatter the layout's fault?" before anyone changes the layout.
#[cfg(test)]
pub(crate) fn bfs_order(store: &Store, tree: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    if let Child::Node(root) = Child::decode(store.get_i32_raw(tree, TOP)) {
        queue.push_back(root);
    }
    while let Some(n) = queue.pop_front() {
        out.push(n);
        for dir in [false, true] {
            if let Child::Node(c) = child(store, tree, n, dir) {
                queue.push_back(c);
            }
        }
    }
    out
}

/// @PLN134 — node ids in DEPTH-FIRST pre-order, FALSE subtree first.
///
/// The layout that puts a subtree in one contiguous run, so the records of one
/// prefix have their nodes together. It pays for that on the way in: a descent
/// that turns TRUE skips the whole FALSE subtree, and near the root that skip is
/// half the array.
///
/// Also the walk [`node_heights`] runs on, because pre-order puts a parent before
/// every one of its descendants — so reading it backwards settles a subtree before
/// the node above it.
pub(crate) fn dfs_order(store: &Store, tree: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = Vec::new();
    // A walk over a CORRUPTED tree must stop rather than spin: a stale child
    // pointer can close a cycle, and the node high-water mark bounds how many
    // distinct nodes any honest walk can visit. Same discipline as `RadixIter`'s
    // budget — it degrades, it does not hang.
    let budget = store.get_u32_raw(tree, NODES) as usize;
    if let Child::Node(root) = Child::decode(store.get_i32_raw(tree, TOP)) {
        stack.push(root);
    }
    while let Some(n) = stack.pop() {
        if out.len() >= budget {
            break;
        }
        out.push(n);
        // TRUE first, so FALSE comes off the stack first and the order is by key.
        for dir in [true, false] {
            if let Child::Node(c) = child(store, tree, n, dir) {
                stack.push(c);
            }
        }
    }
    out
}

/// @PLN134 — node ids in KEY order (in-order: FALSE subtree, node, TRUE subtree).
///
/// The node-level form of the placement `routing` used for its postings — number
/// by walking the tree in key order, so one prefix is one contiguous interval.
#[cfg(test)]
pub(crate) fn key_order(store: &Store, tree: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack: Vec<(u32, bool)> = Vec::new();
    if let Child::Node(root) = Child::decode(store.get_i32_raw(tree, TOP)) {
        stack.push((root, false));
    }
    while let Some((n, emitted)) = stack.pop() {
        if emitted {
            out.push(n);
            if let Child::Node(c) = child(store, tree, n, true) {
                stack.push((c, false));
            }
        } else {
            stack.push((n, true));
            if let Child::Node(c) = child(store, tree, n, false) {
                stack.push((c, false));
            }
        }
    }
    out
}

/// @PLN134 — every node's height, `1` at a node with no node children.
///
/// Bottom-up over the pre-order list read backwards: pre-order puts a parent
/// before all of its descendants, so reading it in reverse settles every child
/// before the node above it. Iterative because a text trie is as deep as its
/// longest key (a 24-byte word gives ~230 levels) over a million nodes.
fn node_heights(store: &Store, tree: u32, nodes: u32) -> Vec<u32> {
    let mut h = vec![0u32; (nodes + 2) as usize];
    for &n in dfs_order(store, tree).iter().rev() {
        let mut best = 1;
        for dir in [false, true] {
            if let Child::Node(c) = child(store, tree, n, dir) {
                best = best.max(h[c as usize] + 1);
            }
        }
        h[n as usize] = best;
    }
    h
}

/// @PLN134 — node ids in **van Emde Boas** order: the cache-oblivious layout,
/// written against exactly this access pattern.
///
/// A subtree of height `h` is emitted as a top part of height `⌈h/2⌉`, laid out
/// this same way, followed by each subtree hanging below it, laid out this same
/// way. Recursively contiguous, so for *any* page size there is a level of the
/// recursion whose parts are about one page — and a root→leaf path crosses only
/// `O(log_B n)` of them. That is the property BFS lacks: BFS clusters the top,
/// which every query shares, and then scatters exactly where the paths diverge.
///
/// Cache-*oblivious* matters here because the page size is not ours to pick: a
/// local file, an HTTP range read and a browser cache disagree about it, and this
/// layout is near-optimal for all of them at once.
///
/// Measured over 978 842 real words: a prefix query touches 2.8 pages of 64 KB
/// here against 27.1 as built, 15.4 breadth-first and 8.7 in key order — and at
/// 4 KB it barely moves where every other order inflates by half. [`rtree_relayout`]
/// is what applies it.
pub(crate) fn veb_order(store: &Store, tree: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let nodes = store.get_u32_raw(tree, NODES);
    let heights = node_heights(store, tree, nodes);
    if let Child::Node(root) = Child::decode(store.get_i32_raw(tree, TOP)) {
        veb_emit(
            store,
            tree,
            root,
            heights[root as usize],
            &heights,
            &mut out,
            nodes as usize,
        );
    }
    out
}

/// Emit exactly the nodes of `root`'s subtree that lie within `h` levels of it.
///
/// `h` is a BUDGET, not a height. The top half of a split is a truncated view of a
/// deeper subtree, so a recursion that reads the child's own height there emits the
/// whole subtree and the level below re-emits it — duplication that grows with the
/// tree, not a wrong order at the margin. The budget is what keeps the two halves
/// disjoint, and `min` with the real height only tightens the splits.
///
/// The recursion is over HEIGHT, not over nodes, so it nests `log2(height)` deep —
/// about eight for a text trie.
///
/// `cap` is the node high-water mark, and it bounds both the nodes emitted and the
/// width of one level. Neither can legitimately exceed it, and a stale child pointer
/// that closes a cycle makes both grow without bound — the same reason [`RadixIter`]
/// carries a budget. It degrades; it does not spin, and `rtree_relayout`'s own count
/// then refuses the tree rather than writing a truncated layout.
fn veb_emit(
    store: &Store,
    tree: u32,
    root: u32,
    h: u32,
    heights: &[u32],
    out: &mut Vec<u32>,
    cap: usize,
) {
    if out.len() >= cap {
        return;
    }
    if h <= 1 {
        out.push(root);
        return;
    }
    let top_h = h.div_ceil(2);
    let bot_h = h - top_h;
    veb_emit(store, tree, root, top_h, heights, out, cap);
    // The roots of the subtrees that hang below the top part — the level the split
    // cut through. Walked rather than remembered, because remembering it for every
    // recursion level costs more than re-reading two child slots.
    let mut level = vec![root];
    for _ in 0..top_h {
        let mut next = Vec::new();
        for n in level.drain(..) {
            for dir in [false, true] {
                if let Child::Node(c) = child(store, tree, n, dir) {
                    next.push(c);
                }
            }
        }
        if next.len() > cap {
            return;
        }
        level = next;
    }
    for b in level {
        veb_emit(
            store,
            tree,
            b,
            bot_h.min(heights[b as usize]),
            heights,
            out,
            cap,
        );
    }
}

/// One node's four words, read out before the array is rewritten.
struct NodeBody {
    bit: u32,
    parent: u32,
    lo: i32,
    hi: i32,
}

/// @PLN134 — renumber the node array into [`veb_order`], and compact it.
///
/// The tree is unchanged as a TREE: same records, same key order, same answers to
/// every lookup. What changes is where each node SITS, and that is the whole
/// difference between a prefix query that fetches 27 pages of a remote image and
/// one that fetches 3. Node ids are handed out in INSERTION order, which places a
/// root→leaf path nowhere near itself; this places it in one run.
///
/// Apply it when an image is WRITTEN — that is the copy a reader pages, and the one
/// moment the whole tree is in hand. A live tree may be relaid out too; ids are
/// internal, so nothing outside the record refers to them.
///
/// Compacting is the second half: removals leave holes threaded on a free list, and
/// a fresh numbering has no holes to carry. `NODES` becomes the live count and the
/// free list empties, so the array's tail is available again.
///
/// Unlike [`rtree_insert`] this never RELOCATES the tree record — it claims nothing,
/// so the caller's id stays good and there is nothing to write back.
///
/// Answers whether it ran. It REFUSES a tree whose walk does not account for every
/// node it should have — a PATRICIA tree over `n` records has exactly `n-1` internal
/// nodes, which is an independent count, checked before anything is written. A tree
/// that fails it is left exactly as it was.
pub fn rtree_relayout(store: &mut Store, tree: u32) -> bool {
    let order = veb_order(store, tree);
    if order.len() != rtree_len(store, tree).saturating_sub(1) as usize {
        return false;
    }
    if order.is_empty() {
        // No nodes to place — but a tree emptied by removals still carries a free
        // list over ids nobody holds, and leaving that behind is the one way this
        // path can differ from the general one.
        store.set_u32_raw(tree, NODES, 0);
        store.set_u32_raw(tree, FREE, 0);
        return true;
    }
    let high = store.get_u32_raw(tree, NODES);
    let mut new_id = vec![0u32; (high + 2) as usize];
    for (i, &n) in order.iter().enumerate() {
        new_id[n as usize] = u32::try_from(i).unwrap_or(u32::MAX) + 1;
    }
    let moved = |new_id: &[u32], c: Child| match c {
        Child::Node(n) => Child::Node(new_id[n as usize]),
        other => other,
    };
    // Read every node BEFORE writing one. The numbering is a permutation, so an
    // in-place rewrite overwrites nodes the pass has not read yet — and the result
    // is a structurally valid tree holding the wrong records, which no walk reports
    // as broken.
    let body: Vec<NodeBody> = order
        .iter()
        .map(|&n| {
            let parent = node_parent(store, tree, n);
            NodeBody {
                bit: node_bit(store, tree, n),
                parent: if parent == 0 {
                    0
                } else {
                    new_id[parent as usize]
                },
                lo: moved(&new_id, child(store, tree, n, false)).encode(),
                hi: moved(&new_id, child(store, tree, n, true)).encode(),
            }
        })
        .collect();
    for (i, b) in body.iter().enumerate() {
        let n = u32::try_from(i).unwrap_or(u32::MAX) + 1;
        set_node_bit(store, tree, n, b.bit);
        set_node_parent(store, tree, n, b.parent);
        store.set_i32_raw(tree, child_off(n, false), b.lo);
        store.set_i32_raw(tree, child_off(n, true), b.hi);
    }
    let root = moved(&new_id, top(store, tree));
    set_top(store, tree, root);
    store.set_u32_raw(tree, NODES, u32::try_from(body.len()).unwrap_or(u32::MAX));
    store.set_u32_raw(tree, FREE, 0);
    true
}

#[cfg(test)]
fn touch(off: u32) {
    TOUCHED.with(|t| {
        if let Ok(mut b) = t.try_borrow_mut()
            && let Some(s) = b.as_mut()
        {
            s.insert(off);
        }
    });
}

#[cfg(not(test))]
#[inline(always)]
fn touch(_off: u32) {}

fn node_bit(store: &Store, tree: u32, node: u32) -> u32 {
    touch(node_off(node));
    store.get_u32_raw(tree, node_off(node))
}

fn set_node_bit(store: &mut Store, tree: u32, node: u32, bit: u32) {
    store.set_u32_raw(tree, node_off(node), bit);
}

/// The node this node hangs under; `0` for the root.
fn node_parent(store: &Store, tree: u32, node: u32) -> u32 {
    store.get_u32_raw(tree, node_off(node) + 4)
}

fn set_node_parent(store: &mut Store, tree: u32, node: u32, parent: u32) {
    store.set_u32_raw(tree, node_off(node) + 4, parent);
}

pub(crate) fn child_off(node: u32, dir: bool) -> u32 {
    node_off(node) + if dir { 12 } else { 8 }
}

fn child(store: &Store, tree: u32, node: u32, dir: bool) -> Child {
    touch(child_off(node, dir));
    Child::decode(store.get_i32_raw(tree, child_off(node, dir)))
}

fn set_child(store: &mut Store, tree: u32, node: u32, dir: bool, c: Child) {
    store.set_i32_raw(tree, child_off(node, dir), c.encode());
}

fn top(store: &Store, tree: u32) -> Child {
    Child::decode(store.get_i32_raw(tree, TOP))
}

fn set_top(store: &mut Store, tree: u32, c: Child) {
    store.set_i32_raw(tree, TOP, c.encode());
}

fn cap_for_words(words: u32) -> u32 {
    (words * 8).saturating_sub(HDR) / NODE_SIZE
}

/// Number of records in the tree.
#[must_use]
pub fn rtree_len(store: &Store, tree: u32) -> u32 {
    store.get_u32_raw(tree, LEN)
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// A test's collection field: a one-word host record in store 0 whose slot at offset 4 names
/// the EMPTY tree.  The slot is WRITTEN, as a record's construction writes a collection
/// field's default — a claim promises no value (`@FR-H-Claim`), so a fixture that only
/// claims hands `add` whatever bytes the block held.
#[cfg(test)]
pub(crate) fn empty_collection(store: &mut Store) -> crate::keys::DbRef {
    let rec = store.claim(1);
    store.set_u32_raw(rec, 4, 0);
    crate::keys::DbRef {
        store_nr: 0,
        rec,
        pos: 4,
    }
}

/// Claim an empty tree with room for `initial` nodes; returns its record id.
///
/// The record id is the tree's identity and is stored in the owning field, the
/// way a hash collection stores its bucket record.  [`rtree_insert`] may move the
/// record, so always keep the id it returns.
pub fn rtree_init(store: &mut Store, initial: u32) -> u32 {
    let words = (HDR + NODE_SIZE * initial).div_ceil(8).max(HDR / 8);
    let tree = store.claim(words);
    let cap = cap_for_words(store.record_words(tree));
    store.set_i32_raw(tree, TOP, 0);
    store.set_u32_raw(tree, LEN, 0);
    store.set_u32_raw(tree, NODES, 0);
    store.set_u32_raw(tree, CAP, cap);
    store.set_u32_raw(tree, FREE, 0);
    tree
}

/// Release the tree's own record.  The records it indexed are not touched.
pub fn rtree_free(store: &mut Store, tree: u32) {
    store.delete(tree);
}

/// Grow the node array, returning the (possibly relocated) tree record id.
fn grow(store: &mut Store, tree: u32) -> u32 {
    let words = store.record_words(tree);
    let tree = store.resize(tree, (words * 2).max(words + 2));
    let cap = cap_for_words(store.record_words(tree));
    store.set_u32_raw(tree, CAP, cap);
    tree
}

/// Take a node id from the free list, or mint a fresh one, growing if needed.
fn alloc_node(store: &mut Store, tree: u32) -> (u32, u32) {
    let free = store.get_u32_raw(tree, FREE);
    if free != 0 {
        let next = store.get_i32_raw(tree, child_off(free, false)) as u32;
        store.set_u32_raw(tree, FREE, next);
        return (tree, free);
    }
    let used = store.get_u32_raw(tree, NODES);
    let tree = if used >= store.get_u32_raw(tree, CAP) {
        grow(store, tree)
    } else {
        tree
    };
    store.set_u32_raw(tree, NODES, used + 1);
    (tree, used + 1)
}

/// Thread a dead node onto the free list through its FALSE slot.
fn free_node(store: &mut Store, tree: u32, node: u32) {
    let head = store.get_u32_raw(tree, FREE);
    store.set_i32_raw(tree, child_off(node, false), head as i32);
    store.set_u32_raw(tree, FREE, node);
}

// ---------------------------------------------------------------------------
// Descent
// ---------------------------------------------------------------------------

/// Everything a read-only walk asks of a tree: the root slot, the record count, and
/// each node's bit / parent / children.
///
/// It exists so the tree has ONE descent, one split point and one in-order step,
/// whichever storage they run over.  The resident tree reads a [`Store`]
/// ([`StoreNodes`]); a paged reader reads the same node array out of a persisted
/// image, page by page, and answers a node by FETCHING — which is why every method
/// takes `&mut self`.  `&self` would compile for the resident case and force the
/// paged one to keep its own copy of the geometry (@PLN134).
///
/// Read-only on purpose.  Mutation stays on `&mut Store` and is not expressible
/// here, so no implementor can be asked to write into a page it does not own.
pub(crate) trait TreeNodes {
    /// One bounded walk is about to start — a descent, a split point, an extreme
    /// descent or an in-order step.
    ///
    /// It exists so a source that does not trust its bytes can bound each walk
    /// EXACTLY: a healthy walk visits each node at most once, so `node count` hops
    /// is a ceiling no correct query reaches, and a pointer cycle stops instead of
    /// spinning. One budget for a whole *query* would not do, because a seek runs
    /// four such walks and the ceiling would have to be a guessed multiple —
    /// which under-provisions on a small tree (a 5-node trie answered every
    /// lookup as ABSENT) and over-provisions on a large one.
    ///
    /// The resident tree needs none of this and gets the default no-op.
    fn walk_begin(&mut self) {}
    /// The root child slot.
    fn top(&mut self) -> Child;
    /// How many records the tree holds — a walk's budget.
    fn len(&mut self) -> u32;
    /// The absolute key bit node `n` tests.
    fn node_bit(&mut self, n: u32) -> u32;
    /// The node `n` hangs under; `0` at the root.
    fn node_parent(&mut self, n: u32) -> u32;
    /// Node `n`'s child on side `dir`.
    fn child(&mut self, n: u32, dir: bool) -> Child;
}

/// A [`TreeNodes`] that can also read a stored record's KEY — what a seek needs and
/// a plain traversal does not.
///
/// The split is not tidiness: iteration and teardown walk the tree with no key oracle
/// at all (`rtree_first`, `RadixIter::next`), and requiring one there would make every
/// caller build a key reader it never uses.
pub(crate) trait TreeKeys: TreeNodes {
    /// How many bits `rec`'s user key has.
    fn rec_bits(&mut self, rec: u32) -> u32;
    /// 64 key bits of `rec` starting at bit `word * 64`; see [`WordFn`].
    fn rec_word(&mut self, rec: u32, word: u32) -> u64;
}

/// [`TreeNodes`] over a resident tree — the whole of what the walk needed from
/// `(store, tree)`, in one value.
pub(crate) struct StoreNodes<'a> {
    store: &'a Store,
    tree: u32,
}

impl<'a> StoreNodes<'a> {
    /// A resident source over `tree` in `store`.
    ///
    /// The seek path builds one for itself; this is for a traversal that lives
    /// OUTSIDE this module and is written against [`TreeNodes`] so the paged
    /// reader can drive it too — `radix_db`'s bounding-box walk (@PLN136).
    pub(crate) fn new(store: &'a Store, tree: u32) -> Self {
        StoreNodes { store, tree }
    }
}

impl TreeNodes for StoreNodes<'_> {
    fn top(&mut self) -> Child {
        top(self.store, self.tree)
    }
    fn len(&mut self) -> u32 {
        rtree_len(self.store, self.tree)
    }
    fn node_bit(&mut self, n: u32) -> u32 {
        node_bit(self.store, self.tree, n)
    }
    fn node_parent(&mut self, n: u32) -> u32 {
        node_parent(self.store, self.tree, n)
    }
    fn child(&mut self, n: u32, dir: bool) -> Child {
        child(self.store, self.tree, n, dir)
    }
}

/// [`StoreNodes`] plus the caller's [`KeyOracle`], for the seek path.
pub(crate) struct StoreKeys<'a, K: KeyOracle> {
    nodes: StoreNodes<'a>,
    spec: K,
}

impl<K: KeyOracle> TreeNodes for StoreKeys<'_, K> {
    fn top(&mut self) -> Child {
        self.nodes.top()
    }
    fn len(&mut self) -> u32 {
        self.nodes.len()
    }
    fn node_bit(&mut self, n: u32) -> u32 {
        self.nodes.node_bit(n)
    }
    fn node_parent(&mut self, n: u32) -> u32 {
        self.nodes.node_parent(n)
    }
    fn child(&mut self, n: u32, dir: bool) -> Child {
        self.nodes.child(n, dir)
    }
}

impl<K: KeyOracle> TreeKeys for StoreKeys<'_, K> {
    fn rec_bits(&mut self, rec: u32) -> u32 {
        self.spec.bits(self.nodes.store, rec)
    }
    fn rec_word(&mut self, rec: u32, word: u32) -> u64 {
        self.spec.word(self.nodes.store, rec, word)
    }
}

/// Follow `key_bit` down from `cur` to the candidate leaf, and the node directly
/// above it.  Leaf `0` = nothing there; `parent` starts at what `cur` hangs under.
///
/// The whole descent, for every storage: pass `(top(), 0)` for a root descent and
/// `(Child::Node(f), parent(f))` for a finger one.
pub(crate) fn descend_gen<N: TreeNodes>(
    src: &mut N,
    mut key_bit: impl FnMut(u32) -> bool,
    mut cur: Child,
    mut parent: u32,
) -> (u32, u32) {
    src.walk_begin();
    loop {
        match cur {
            Child::Empty => return (0, 0),
            Child::Rec(r) => return (r, parent),
            Child::Node(n) => {
                let dir = key_bit(src.node_bit(n));
                cur = src.child(n, dir);
                parent = n;
            }
        }
    }
}

/// Always branch the same way; reaches the FALSE-most (or TRUE-most) leaf below
/// `cur` and the node above it.
pub(crate) fn descend_extreme_gen<N: TreeNodes>(
    src: &mut N,
    mut cur: Child,
    dir: bool,
    mut parent: u32,
) -> (u32, u32) {
    src.walk_begin();
    loop {
        match cur {
            Child::Empty => return (0, 0),
            Child::Rec(r) => return (r, parent),
            Child::Node(n) => {
                cur = src.child(n, dir);
                parent = n;
            }
        }
    }
}

/// Follow `key` down to the candidate leaf, and the node directly above it.
/// Leaf `0` = empty tree; parent `0` = the leaf is the root.
fn descend<F: Fn(u32) -> u64>(store: &Store, tree: u32, key: &View<F>) -> (u32, u32) {
    let mut src = StoreNodes { store, tree };
    let start = src.top();
    descend_gen(&mut src, |b| key.bit(b), start, 0)
}

/// Follow `key` from `start` rather than the root — the finger descent.
fn descend_from<F: Fn(u32) -> u64>(
    store: &Store,
    tree: u32,
    key: &View<F>,
    start: u32,
) -> (u32, u32) {
    let mut src = StoreNodes { store, tree };
    let parent = src.node_parent(start);
    descend_gen(&mut src, |b| key.bit(b), Child::Node(start), parent)
}

/// Always branch the same way; reaches the FALSE-most (or TRUE-most) leaf and the
/// node above it.
fn descend_extreme(store: &Store, tree: u32, cur: Child, dir: bool, parent: u32) -> (u32, u32) {
    descend_extreme_gen(&mut StoreNodes { store, tree }, cur, dir, parent)
}

/// Where a new node testing bit `d` must be spliced in: the node whose child slot
/// points at the subtree that diverges there, and which side.  `parent == 0` means
/// the slot is TOP.
///
/// I1 makes `bit(n) == d` impossible on a descent path: a node testing `d` would
/// have sent the search key and the candidate leaf the same way, so they would agree
/// at `d`, contradicting `d` being their first difference.  So `<` and `>` partition
/// the path and no third case exists.
fn split_point<F: Fn(u32) -> u64>(store: &Store, tree: u32, key: &View<F>, d: u32) -> (u32, bool) {
    let mut src = StoreNodes { store, tree };
    let start = src.top();
    split_point_gen(&mut src, |b| key.bit(b), d, start, 0, false)
}

/// [`split_point`] for any storage: walk down from `cur` while the node's bit is at
/// or below `d`, and report the child slot the walk stopped in.
pub(crate) fn split_point_gen<N: TreeNodes>(
    src: &mut N,
    mut key_bit: impl FnMut(u32) -> bool,
    d: u32,
    mut cur: Child,
    mut parent: u32,
    mut dir: bool,
) -> (u32, bool) {
    src.walk_begin();
    while let Child::Node(n) = cur {
        let bit = src.node_bit(n);
        if bit > d {
            break;
        }
        dir = key_bit(bit);
        cur = src.child(n, dir);
        parent = n;
    }
    (parent, dir)
}

/// [`split_point`] resumed at `start` instead of the root.  Sound only when
/// `bit(start) < d`, which every caller establishes.
fn split_point_from<F: Fn(u32) -> u64>(
    store: &Store,
    tree: u32,
    key: &View<F>,
    d: u32,
    start: u32,
) -> (u32, bool) {
    let mut src = StoreNodes { store, tree };
    let dir = key.bit(src.node_bit(start));
    let cur = src.child(start, dir);
    split_point_gen(&mut src, |b| key.bit(b), d, cur, start, dir)
}

/// The child slot `split_point` named — the subtree a split displaces.
pub(crate) fn subtree_below_gen<N: TreeNodes>(src: &mut N, parent: u32, dir: bool) -> Child {
    if parent == 0 {
        src.top()
    } else {
        src.child(parent, dir)
    }
}

fn subtree_below(store: &Store, tree: u32, parent: u32, dir: bool) -> Child {
    subtree_below_gen(&mut StoreNodes { store, tree }, parent, dir)
}

/// A probe key: the caller's words, then zeros — the same string a record gets, but
/// with id `0`, so it sorts to the head of its own bucket.
fn probe_view<P: Fn(u32) -> u64>(probe: &P, probe_bits: u32) -> View<&P> {
    View {
        word: probe,
        bits: probe_bits,
        id: 0,
    }
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

/// Position at the **candidate** leaf for a probe — the record sharing its longest
/// prefix.
///
/// Despite the name this is not a lookup: descent skips the bits path compression
/// elided, so for an absent key the candidate is some other record entirely.
/// Compare keys before trusting it, or call [`rtree_get`], which does.
#[must_use]
pub fn rtree_find<P>(store: &Store, tree: u32, probe: &P, probe_bits: u32) -> RadixIter
where
    P: Fn(u32) -> u64,
{
    let (rec, node) = descend(store, tree, &probe_view(probe, probe_bits));
    RadixIter::at(store, tree, rec, node)
}

/// Position at the lowest record whose key is `>= probe` — a lower bound — and report
/// the bit at which the probe first left the candidate's key.
///
/// The candidate leaf a descent reaches is *not* generally that record, because the
/// descent skipped the bits path compression elided.  So re-ascend to the subtree that
/// diverges from the probe at bit `d`: by I2 every record in it agrees with the probe
/// below `d` and agrees with the candidate *at* `d`.  If the probe has `0` there it
/// precedes the whole subtree; if `1`, it follows all of it.
fn seek_inner<P, K: KeyOracle + Copy>(
    store: &Store,
    tree: u32,
    probe: &P,
    probe_bits: u32,
    spec: K,
) -> (RadixIter, Option<u32>)
where
    P: Fn(u32) -> u64,
{
    seek_gen(
        &mut StoreKeys {
            nodes: StoreNodes { store, tree },
            spec,
        },
        probe,
        probe_bits,
    )
}

/// [`seek_inner`] over any storage — the lower bound, and where the probe left the
/// candidate.
///
/// The probe reads its own words and never touches `src`, which is what lets the
/// CANDIDATE's key be read through the same `&mut` source: a paged image answers a
/// key by fetching the record's page.
pub(crate) fn seek_gen<S: TreeKeys, P>(
    src: &mut S,
    probe: &P,
    probe_bits: u32,
) -> (RadixIter, Option<u32>)
where
    P: Fn(u32) -> u64,
{
    let probe_bit_at = |b: u32| composed_bit(probe, probe_bits, 0, b);
    let start = src.top();
    let (cand, cand_parent) = descend_gen(src, probe_bit_at, start, 0);
    if cand == 0 {
        return (RadixIter::empty(), None);
    }
    let cand_bits = src.rec_bits(cand);
    let diff = first_diff_words(probe_bits, 0, probe, cand_bits, cand, |w| {
        src.rec_word(cand, w)
    });
    let Some((d, probe_bit)) = diff else {
        return (RadixIter::at_gen(src, cand, cand_parent), None);
    };
    let start = src.top();
    let (parent, dir) = split_point_gen(src, probe_bit_at, d, start, 0, false);
    let sub = subtree_below_gen(src, parent, dir);
    let it = if probe_bit {
        // The probe sorts after every record in the subtree: take its last, step on.
        let (rec, node) = descend_extreme_gen(src, sub, true, parent);
        let mut it = RadixIter::at_gen(src, rec, node);
        it.step_gen(src, true);
        it
    } else {
        let (rec, node) = descend_extreme_gen(src, sub, false, parent);
        RadixIter::at_gen(src, rec, node)
    };
    (it, Some(d))
}

/// Position at the lowest record whose key is `>= probe`.
///
/// Because a probe carries id `0`, seeking a text prefix lands on the first record
/// bearing that prefix.
#[must_use]
pub fn rtree_seek<P, K: KeyOracle + Copy>(
    store: &Store,
    tree: u32,
    probe: &P,
    probe_bits: u32,
    spec: K,
) -> RadixIter
where
    P: Fn(u32) -> u64,
{
    seek_inner(store, tree, probe, probe_bits, spec).0
}

/// Does `rec` carry exactly this user key?  Lets a caller walk a bucket — the
/// contiguous run of records sharing one key — from [`rtree_seek`].
#[must_use]
pub fn rtree_key_eq<P, K: KeyOracle + Copy>(
    store: &Store,
    rec: u32,
    probe: &P,
    probe_bits: u32,
    spec: K,
) -> bool
where
    P: Fn(u32) -> u64,
{
    // Equal lengths mean both views zero-pad identically past the key, so whole words
    // may be compared without masking the tail.
    spec.bits(store, rec) == probe_bits
        && (0..probe_bits.div_ceil(64)).all(|w| spec.word(store, rec, w) == probe(w))
}

/// The first record whose user key equals the probe, or `0` when none does.
///
/// This is the exact lookup [`rtree_find`] deliberately is not.  When several records
/// share the key, it is the head of that run; walk on with [`RadixIter::next`] and
/// [`rtree_key_eq`].
///
/// It re-reads no key bits.  `seek` already found `d`, the first bit at which the probe
/// left the candidate, and
///
/// > the user keys are equal **iff** `d >= probe_bits` and the lengths match.
///
/// Both halves carry weight, and neither implies the other.  `d >= probe_bits` says
/// they agree over the probe's whole length — but a stored key may simply be longer
/// (`"ab"` against `"abc"` diverges at bit 17, past a 16-bit probe), which only the
/// length rejects.  And equal lengths alone say nothing about the bits.  The result
/// holds for `rec`, not just the candidate, because every record in the divergence
/// subtree agrees below `d` (I2).  No `0x00` assumption is needed.
#[must_use]
pub fn rtree_get<P, K: KeyOracle + Copy>(
    store: &Store,
    tree: u32,
    probe: &P,
    probe_bits: u32,
    spec: K,
) -> u32
where
    P: Fn(u32) -> u64,
{
    let (it, d) = seek_inner(store, tree, probe, probe_bits, spec);
    let rec = it.rec();
    if rec == 0 {
        return 0;
    }
    // `d == None` means the strings are identical — unreachable while ids differ.
    let matched = d.is_none_or(|d| d >= probe_bits);
    if matched && spec.bits(store, rec) == probe_bits {
        rec
    } else {
        0
    }
}

/// Position at the lowest record, walking forward from there.
#[must_use]
pub fn rtree_first(store: &Store, tree: u32) -> RadixIter {
    straight(store, tree, false)
}

/// Position at the highest record, walking backward from there.
#[must_use]
pub fn rtree_last(store: &Store, tree: u32) -> RadixIter {
    straight(store, tree, true)
}

fn straight(store: &Store, tree: u32, dir: bool) -> RadixIter {
    let c = top(store, tree);
    let (rec, node) = descend_extreme(store, tree, c, dir, 0);
    RadixIter::at(store, tree, rec, node)
}

// ---------------------------------------------------------------------------
// Mutation
// ---------------------------------------------------------------------------

/// Insert `rec`, returning the (possibly relocated) tree record id.
///
/// Growth can move the container, so the returned id must replace the caller's.
/// Inserting a record the tree already holds is a no-op.  Two *different* records
/// may share a user key; they differ in the id suffix, so they land adjacent.
///
/// This cannot fail: distinct records always differ somewhere in the key string,
/// so there is always a bit to split on.
pub fn rtree_insert<K: KeyOracle + Copy>(store: &mut Store, tree: u32, rec: u32, spec: K) -> u32 {
    if rtree_len(store, tree) == 0 {
        set_top(store, tree, Child::Rec(rec));
        store.set_u32_raw(tree, LEN, 1);
        return tree;
    }

    // Everything the split needs is read before the tree can move.
    let (d, rec_bit, parent, dir, displaced) = {
        let s: &Store = store;
        let rw = |w: u32| spec.word(s, rec, w);
        let ka = View {
            word: rw,
            bits: spec.bits(s, rec),
            id: rec,
        };
        let (cand, _) = descend(s, tree, &ka);
        if cand == rec {
            return tree;
        }
        let cw = |w: u32| spec.word(s, cand, w);
        let kb = View {
            word: cw,
            bits: spec.bits(s, cand),
            id: cand,
        };
        // Unreachable: two distinct records differ in their id suffix.
        let Some((d, rec_bit)) = first_diff(&ka, &kb) else {
            return tree;
        };
        let (parent, dir) = split_point(s, tree, &ka, d);
        let displaced = subtree_below(s, tree, parent, dir);
        (d, rec_bit, parent, dir, displaced)
    };

    link_split(store, tree, rec, d, rec_bit, parent, dir, displaced)
}

/// Remove `rec`; `false` if it is not present.
///
/// Splicing the parent out keeps every internal node at exactly two children,
/// which is what makes `live_nodes == LEN - 1` a total check on the structure.
pub fn rtree_remove<K: KeyOracle + Copy>(store: &mut Store, tree: u32, rec: u32, spec: K) -> bool {
    let (leaf, parent) = {
        let s: &Store = store;
        let rw = |w: u32| spec.word(s, rec, w);
        let key = View {
            word: rw,
            bits: spec.bits(s, rec),
            id: rec,
        };
        descend(s, tree, &key)
    };
    if leaf != rec {
        return false;
    }
    unlink(store, tree, rec, parent);
    true
}

/// Splice `rec` out from under `parent`, and return the node that survives directly
/// above the hole — the **finger** a re-insert can descend from.  `0` when there is
/// none (the tree became empty, or the sibling was promoted to the root).
fn unlink(store: &mut Store, tree: u32, rec: u32, parent: u32) -> u32 {
    let len = rtree_len(store, tree);
    store.set_u32_raw(tree, LEN, len - 1);
    if parent == 0 {
        set_top(store, tree, Child::Empty);
        return 0;
    }
    let grand = node_parent(store, tree, parent);
    let rec_is_true = child(store, tree, parent, true) == Child::Rec(rec);
    let sibling = child(store, tree, parent, !rec_is_true);
    if grand == 0 {
        set_top(store, tree, sibling);
    } else {
        let up_is_true = child(store, tree, grand, true) == Child::Node(parent);
        set_child(store, tree, grand, up_is_true, sibling);
    }
    if let Child::Node(sub) = sibling {
        set_node_parent(store, tree, sub, grand);
    }
    free_node(store, tree, parent);
    grand
}

/// How far a finger may climb before giving up and descending from the root.
///
/// Chosen by measurement, not taste (`probe_finger_savings`).  A drifting object's
/// new key diverges from its old one deep in the string, so the climb is short and the
/// finger visits ~4× fewer nodes.  A teleport diverges near the root, the climb runs
/// the whole way, and an *uncapped* finger is 1.7× **worse** than simply starting over.
/// At `k = 2` the mixed cost is within 1% of the flat optimum while the teleport
/// penalty is held to 1.2×.
const FINGER_CLIMB_CAP: usize = 2;

/// What the read phase decided.
enum Plan {
    /// Already present under this key.
    Noop,
    /// Split below `parent`/`dir`, putting `rec` on side `rec_bit` of a node at `d`.
    Split {
        d: u32,
        rec_bit: bool,
        parent: u32,
        dir: bool,
    },
    /// The finger was no use; start from the root.
    Restart,
}

/// Re-insert `rec` starting from `finger` instead of the root.
///
/// The finger is sound because of I2: every record under a node `A` agrees on all bits
/// below `bit(A)`.  So if the new key first leaves the leaf below the finger at bit
/// `d`, then `bit(A) <= d` says exactly that the new key still belongs under `A` —
/// climb while `bit(A) > d`, and descend from the first `A` that qualifies.
fn insert_with_finger<K: KeyOracle + Copy>(
    store: &mut Store,
    tree: u32,
    rec: u32,
    spec: K,
    finger: u32,
) -> u32 {
    if rtree_len(store, tree) == 0 || finger == 0 {
        return rtree_insert(store, tree, rec, spec);
    }
    let plan = {
        let s: &Store = store;
        let rw = |w: u32| spec.word(s, rec, w);
        let ka = View {
            word: rw,
            bits: spec.bits(s, rec),
            id: rec,
        };
        let diff_below = |from: u32| -> Option<(u32, bool)> {
            let (cand, _) = descend_from(s, tree, &ka, from);
            if cand == rec || cand == 0 {
                return None;
            }
            let cw = |w: u32| spec.word(s, cand, w);
            let kb = View {
                word: cw,
                bits: spec.bits(s, cand),
                id: cand,
            };
            first_diff(&ka, &kb)
        };

        match diff_below(finger) {
            None => Plan::Noop,
            Some((mut d, mut rec_bit)) => {
                let mut anchor = finger;
                if d < node_bit(s, tree, anchor) {
                    let mut climbed = 0;
                    while anchor != 0 && node_bit(s, tree, anchor) > d && climbed < FINGER_CLIMB_CAP
                    {
                        anchor = node_parent(s, tree, anchor);
                        climbed += 1;
                    }
                    if anchor == 0 || node_bit(s, tree, anchor) > d {
                        // The new key belongs somewhere far away; the root is cheaper.
                        Plan::Restart
                    } else {
                        match diff_below(anchor) {
                            None => Plan::Noop,
                            Some((d2, rb2)) => {
                                d = d2;
                                rec_bit = rb2;
                                let (parent, dir) = split_point_from(s, tree, &ka, d, anchor);
                                Plan::Split {
                                    d,
                                    rec_bit,
                                    parent,
                                    dir,
                                }
                            }
                        }
                    }
                } else {
                    let (parent, dir) = split_point_from(s, tree, &ka, d, anchor);
                    Plan::Split {
                        d,
                        rec_bit,
                        parent,
                        dir,
                    }
                }
            }
        }
    };

    match plan {
        Plan::Noop => tree,
        Plan::Restart => rtree_insert(store, tree, rec, spec),
        Plan::Split {
            d,
            rec_bit,
            parent,
            dir,
        } => {
            let displaced = subtree_below(store, tree, parent, dir);
            link_split(store, tree, rec, d, rec_bit, parent, dir, displaced)
        }
    }
}

/// Hang a fresh node testing bit `d` in the slot `(parent, dir)`, with `rec` on side
/// `rec_bit` and the displaced subtree opposite.  Returns the (possibly moved) tree.
#[allow(clippy::too_many_arguments)] // the split is one act; splitting it hides the wiring
fn link_split(
    store: &mut Store,
    tree: u32,
    rec: u32,
    d: u32,
    rec_bit: bool,
    parent: u32,
    dir: bool,
    displaced: Child,
) -> u32 {
    let len = rtree_len(store, tree);
    let (tree, node) = alloc_node(store, tree);
    set_node_bit(store, tree, node, d);
    set_node_parent(store, tree, node, parent);
    set_child(store, tree, node, rec_bit, Child::Rec(rec));
    set_child(store, tree, node, !rec_bit, displaced);
    if let Child::Node(sub) = displaced {
        set_node_parent(store, tree, sub, node);
    }
    if parent == 0 {
        set_top(store, tree, Child::Node(node));
    } else {
        set_child(store, tree, parent, dir, Child::Node(node));
    }
    store.set_u32_raw(tree, LEN, len + 1);
    tree
}

/// Move `rec`: unlink it, let `mutate` rewrite the fields its key is built from, then
/// re-insert from the finger the unlink left behind.
///
/// This is the operation a game loop actually performs — an entity's coordinates
/// change, so its Morton key changes, so the index must be updated.  Because the entity
/// has *not* moved far, its new key shares a long prefix with the old one and the
/// re-insert starts near where it ended up rather than at the root.
///
/// The key must not be read from anywhere but the record, and `mutate` must not touch
/// the tree.
pub fn rtree_move<F, K: KeyOracle + Copy>(
    store: &mut Store,
    tree: u32,
    rec: u32,
    spec: K,
    mutate: F,
) -> u32
where
    F: FnOnce(&mut Store),
{
    let (leaf, parent) = {
        let s: &Store = store;
        let rw = |w: u32| spec.word(s, rec, w);
        let key = View {
            word: rw,
            bits: spec.bits(s, rec),
            id: rec,
        };
        descend(s, tree, &key)
    };
    let finger = if leaf == rec {
        unlink(store, tree, rec, parent)
    } else {
        0
    };
    mutate(store);
    insert_with_finger(store, tree, rec, spec, finger)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Assert I1 and I2 over the whole tree.
///
/// **I1** — strictly increasing bits, two children per node, every leaf on the
/// side its key bit chooses, and the counts a two-children-per-node tree forces
/// (`leaves == LEN`, `live_nodes == LEN - 1`, `live + freed == NODES`).  A leaked
/// node, a double-freed node, and a failed splice each break one of these.
///
/// **I2 — the licence to skip bits.**  Every record under node `n` agrees on
/// *every* bit below `bit(n)`.  This is what makes a skipped bit unnecessary to
/// store, and what [`rtree_seek`] leans on when it argues that the whole divergence
/// subtree sits on one side of the probe.
///
/// **I3** — `parent(child(n, d)) == n`, and the root's parent is `0`.  The parent index
/// is what makes a cursor allocation-free, and the three sites that maintain it
/// (insert's split, remove's splice, the displaced subtree) would each fail silently;
/// this check is what makes them loud.
///
/// I1 and I2 are independent: put two keys that first diverge at bit 0 under a root
/// that tests bit 1, and every I1-shaped check still passes.
///
/// Checking the subtree's least and greatest leaf suffices, by induction: the
/// recursive call establishes that every leaf under a child agrees with its
/// representative on all bits below the child's bit, which strictly exceeds this
/// node's.
#[cfg(test)]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn rtree_validate<K: KeyOracle + Copy>(store: &Store, tree: u32, spec: K) {
    struct Walk<'a, K: KeyOracle> {
        store: &'a Store,
        tree: u32,
        spec: K,
        len: u32,
        leaves: u32,
        live: u32,
        /// Every `(bit, side)` decision on the path down to the current child.
        constraints: Vec<(u32, bool)>,
    }

    impl<K: KeyOracle + Copy> Walk<'_, K> {
        /// Returns a representative leaf from each end of this subtree.
        fn visit(&mut self, cur: Child, parent_bit: Option<u32>, up: u32) -> Option<(u32, u32)> {
            match cur {
                Child::Empty => {
                    assert_eq!(self.len, 0, "empty child in a non-empty tree");
                    None
                }
                Child::Rec(r) => {
                    self.leaves += 1;
                    let rw = |w: u32| self.spec.word(self.store, r, w);
                    let key = View {
                        word: rw,
                        bits: self.spec.bits(self.store, r),
                        id: r,
                    };
                    for &(bit, dir) in &self.constraints {
                        assert_eq!(
                            key.bit(bit),
                            dir,
                            "record {r} sits on the {dir} side of bit {bit}, its key disagrees"
                        );
                    }
                    Some((r, r))
                }
                Child::Node(n) => {
                    self.live += 1;
                    let bit = node_bit(self.store, self.tree, n);
                    if let Some(pb) = parent_bit {
                        assert!(bit > pb, "I1: bit {bit} does not exceed parent bit {pb}");
                    }
                    assert_eq!(
                        node_parent(self.store, self.tree, n),
                        up,
                        "I3: node {n} does not point back at the node above it"
                    );
                    let mut ends = [0u32; 2];
                    for dir in [false, true] {
                        let c = child(self.store, self.tree, n, dir);
                        assert_ne!(c, Child::Empty, "node {n} has no {dir} child");
                        self.constraints.push((bit, dir));
                        let (lo, hi) = self.visit(c, Some(bit), n).expect("a child has leaves");
                        self.constraints.pop();
                        ends[usize::from(dir)] = if dir { hi } else { lo };
                    }
                    let (lo, hi) = (ends[0], ends[1]);

                    // I2: the bits this node skipped are common to the whole subtree,
                    // so they never needed storing.
                    let from = parent_bit.map_or(0, |pb| pb + 1);
                    let (lw, hw) = (
                        |w: u32| self.spec.word(self.store, lo, w),
                        |w: u32| self.spec.word(self.store, hi, w),
                    );
                    let (lk, hk) = (
                        View {
                            word: &lw,
                            bits: self.spec.bits(self.store, lo),
                            id: lo,
                        },
                        View {
                            word: &hw,
                            bits: self.spec.bits(self.store, hi),
                            id: hi,
                        },
                    );
                    for b in from..bit {
                        assert_eq!(
                            lk.bit(b),
                            hk.bit(b),
                            "I2: node {n} skips bit {b} on its way to bit {bit}, \
                             but records {lo} and {hi} below it disagree there"
                        );
                    }
                    Some((lo, hi))
                }
            }
        }
    }

    let len = rtree_len(store, tree);
    let mut walk = Walk {
        store,
        tree,
        spec,
        len,
        leaves: 0,
        live: 0,
        constraints: Vec::new(),
    };
    let _ = walk.visit(top(store, tree), None, 0);
    let (leaves, live) = (walk.leaves, walk.live);
    assert_eq!(leaves, len, "leaf count disagrees with LEN");
    assert_eq!(live, len.saturating_sub(1), "live nodes must be LEN - 1");

    let mut freed = 0u32;
    let mut f = store.get_u32_raw(tree, FREE);
    while f != 0 {
        freed += 1;
        assert!(freed <= store.get_u32_raw(tree, NODES), "cyclic free list");
        f = store.get_i32_raw(tree, child_off(f, false)) as u32;
    }
    assert_eq!(
        live + freed,
        store.get_u32_raw(tree, NODES),
        "live + freed nodes must account for every minted node"
    );
}

// ---------------------------------------------------------------------------
// Tests — the steps of doc/claude/plans/48-spacial-index/RADIX_TREE.md §7
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // ---- fixed-width oracle: a `u32` code at `fld 4` -----------------------

    fn code_word(store: &Store, rec: u32, word: u32) -> u64 {
        if word == 0 {
            u64::from(store.get_u32_raw(rec, 4)) << 32
        } else {
            0
        }
    }

    fn code_bits(_store: &Store, _rec: u32) -> u32 {
        32
    }

    const CODE: KeySpec = KeySpec {
        word: code_word,
        bits: code_bits,
    };

    /// The tree orders on `(code, rec)`: equal codes tie-break on the id suffix.
    fn ordered_key(code: u32, rec: u32) -> u64 {
        (u64::from(code) << 32) | u64::from(rec)
    }

    fn code_probe(code: u32) -> impl Fn(u32) -> u64 {
        move |word| if word == 0 { u64::from(code) << 32 } else { 0 }
    }

    fn add(store: &mut Store, code: u32) -> u32 {
        let rec = store.claim(1);
        store.set_u32_raw(rec, 4, code);
        rec
    }

    /// Deterministic pseudo-random codes — reproducible failures, no `rand` dep.
    ///
    /// Shifts by 32, not 33: a 31-bit code would pin the key's most significant bit
    /// to `0` for every record, so no node would ever branch on it.
    fn lcg(seed: &mut u64) -> u32 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (*seed >> 32) as u32
    }

    fn collect(store: &Store, tree: u32) -> Vec<u32> {
        let mut it = rtree_first(store, tree);
        let mut out = Vec::new();
        let mut r = it.rec();
        while r != 0 {
            out.push(r);
            r = it.next(store, tree).unwrap_or(0);
        }
        out
    }

    // ---- benchmarks -------------------------------------------------------
    //
    // `#[ignore]` by default, as the repo does for heavy Rust work.  Run:
    //   cargo test --release --lib radix_tree::tests::bench -- --ignored --nocapture
    //
    // Absolute ns/op is machine-specific and not worth quoting.  Every figure is
    // therefore reported beside `std::collections::BTreeMap` doing the same work on
    // the same keys — the structure a loft user would otherwise reach for via
    // `sorted<T[k]>`.  The ratio is what travels.

    #[allow(clippy::cast_precision_loss)] // counts here are ≤ 10^6
    fn per_op(d: std::time::Duration, n: usize) -> f64 {
        d.as_secs_f64() * 1e9 / n as f64
    }

    fn row(name: &str, radix: f64, btree: f64) {
        println!(
            "  {name:<22} {radix:>9.1} {btree:>11.1} {:>8.2}x",
            radix / btree
        );
    }

    /// Bulk behaviour at 100k records, against a `BTreeMap` baseline.
    ///
    /// **Best of 3 warm rounds**, as `PERFORMANCE.md` does: a single round swings by
    /// ~20% run to run — enough to invent or hide a 1.2× effect — so one round's
    /// numbers are not quotable.  Each round rebuilds from a fresh store.
    #[test]
    #[ignore = "benchmark — run with --release --ignored --nocapture"]
    fn bench_vs_btreemap() {
        use std::time::Instant;
        const N: usize = 100_000;
        const ROUNDS: usize = 3;

        // [insert, get, walk, remove] for radix, then the same for BTreeMap.
        let mut best = [f64::MAX; 8];
        for _ in 0..ROUNDS {
            let mut store = Store::new_in_use(1 << 16);
            let mut tree = rtree_init(&mut store, 0);
            let mut seed = 0x9e37_79b9_7f4a_7c15;

            // Records exist before timing: the store allocator is not in the insert cost.
            let codes: Vec<u32> = (0..N).map(|_| lcg(&mut seed)).collect();
            let recs: Vec<u32> = codes.iter().map(|&c| add(&mut store, c)).collect();

            let t = Instant::now();
            for &r in &recs {
                tree = rtree_insert(&mut store, tree, r, CODE);
            }
            best[0] = best[0].min(per_op(t.elapsed(), N));

            let t = Instant::now();
            let mut hits = 0u64;
            for &c in &codes {
                hits += u64::from(rtree_get(&store, tree, &code_probe(c), 32, CODE) != 0);
            }
            best[1] = best[1].min(per_op(t.elapsed(), N));
            assert_eq!(hits as usize, N, "every key must be found");

            let t = Instant::now();
            let walked = collect(&store, tree).len();
            best[2] = best[2].min(per_op(t.elapsed(), N));
            assert_eq!(walked, N);

            let t = Instant::now();
            for &r in &recs {
                rtree_remove(&mut store, tree, r, CODE);
            }
            best[3] = best[3].min(per_op(t.elapsed(), N));

            let mut bt = BTreeMap::new();
            let t = Instant::now();
            for (i, &r) in recs.iter().enumerate() {
                bt.insert(ordered_key(codes[i], r), r);
            }
            best[4] = best[4].min(per_op(t.elapsed(), N));

            let t = Instant::now();
            let mut hits = 0u64;
            for (i, &c) in codes.iter().enumerate() {
                hits += u64::from(bt.contains_key(&ordered_key(c, recs[i])));
            }
            best[5] = best[5].min(per_op(t.elapsed(), N));
            assert_eq!(hits as usize, N);

            let t = Instant::now();
            let walked = bt.values().count();
            best[6] = best[6].min(per_op(t.elapsed(), N));
            assert_eq!(walked, N);

            let t = Instant::now();
            for (i, &r) in recs.iter().enumerate() {
                bt.remove(&ordered_key(codes[i], r));
            }
            best[7] = best[7].min(per_op(t.elapsed(), N));
        }

        println!("\nradix_tree vs BTreeMap, n = {N}, best of {ROUNDS}, ns/op (lower is better)");
        println!(
            "  {:<22} {:>9} {:>11} {:>8}",
            "op", "radix", "BTreeMap", "ratio"
        );
        for (i, name) in ["insert", "get (exact)", "walk (in-order)", "remove"]
            .iter()
            .enumerate()
        {
            row(name, best[i], best[i + 4]);
        }
        println!("\n  node bytes: {NODE_SIZE} — one node per record beyond the first");
    }

    /// @PLN48's real workload: a per-chunk index of a few hundred entities, every one
    /// of which moves each frame (remove + reinsert), followed by a proximity query
    /// that walks outward from a point.  This is the number that decides S4.
    ///
    /// Best of 3 warm rounds, for the same reason as [`bench_vs_btreemap`].
    #[test]
    #[ignore = "benchmark — run with --release --ignored --nocapture"]
    fn bench_move_and_proximity() {
        use std::time::Instant;
        const ENTITIES: usize = 512;
        const FRAMES: usize = 2_000;
        const NEIGHBOURS: usize = 8;
        const ROUNDS: usize = 3;

        let (mut best_move, mut best_query) = (f64::MAX, f64::MAX);
        for _ in 0..ROUNDS {
            let mut store = Store::new_in_use(1 << 12);
            let mut tree = rtree_init(&mut store, 0);
            let mut seed = 0xdead_beef_cafe_f00d;
            let recs: Vec<u32> = (0..ENTITIES)
                .map(|_| add(&mut store, lcg(&mut seed)))
                .collect();
            for &r in &recs {
                tree = rtree_insert(&mut store, tree, r, CODE);
            }

            // Every entity moves, every frame.
            let t = Instant::now();
            for _ in 0..FRAMES {
                for &r in &recs {
                    rtree_remove(&mut store, tree, r, CODE);
                    store.set_u32_raw(r, 4, lcg(&mut seed));
                    tree = rtree_insert(&mut store, tree, r, CODE);
                }
            }
            best_move = best_move.min(per_op(t.elapsed(), FRAMES * ENTITIES));

            // Proximity: one seek seeds two cursors that walk outward.  `RadixIter` is
            // `Copy`, so the second cursor costs no allocation.
            let t = Instant::now();
            let mut found = 0u64;
            for _ in 0..FRAMES {
                let probe = code_probe(lcg(&mut seed));
                let start = rtree_seek(&store, tree, &probe, 32, CODE);
                let (mut back, mut fore) = (start, start);
                for _ in 0..NEIGHBOURS {
                    found += u64::from(back.prev(&store, tree).unwrap_or(0) != 0);
                    found += u64::from(fore.next(&store, tree).unwrap_or(0) != 0);
                }
            }
            best_query = best_query.min(per_op(t.elapsed(), FRAMES));
            assert!(found > 0, "the outward walk must find neighbours");
            rtree_validate(&store, tree, CODE);
        }

        println!("\n@PLN48 workload — {ENTITIES} entities, {FRAMES} frames, best of {ROUNDS}");
        println!("  move (remove+insert) : {best_move:>8.1} ns");
        println!("  proximity query      : {best_query:>8.1} ns   (seek + {NEIGHBOURS} each way)");
        println!(
            "  per-frame cost       : {:>8.1} us",
            best_move.mul_add(f64::from(u32::try_from(ENTITIES).unwrap()), best_query) / 1000.0
        );
    }

    /// R0 — a cursor is three words.  A `Vec` is 24 bytes, so this is a standing
    /// guard that `seek`/`first`/`next` stay allocation-free.
    #[test]
    fn r0_cursor_is_two_words_and_allocates_nothing() {
        assert!(
            std::mem::size_of::<RadixIter>() <= 16,
            "a Vec cannot fit here"
        );
        fn assert_copy<T: Copy>() {}
        assert_copy::<RadixIter>();
    }

    /// R1 — a tree that is initialised and freed leaves no claim behind.
    ///
    /// The host record is claimed first so the tree cannot land on `PRIMARY`, whose
    /// deletion would empty `claims` on its own and hide a leak.
    #[test]
    fn r1_init_and_free_leak_nothing() {
        let mut store = Store::new_in_use(8);
        let host = store.claim(1);
        let tree = rtree_init(&mut store, 4);
        assert_ne!(tree, host, "the tree must claim its own record");
        assert_eq!(rtree_len(&store, tree), 0);
        assert_eq!(store.get_u32_raw(tree, CAP), 4, "room for 4 nodes");

        rtree_free(&mut store, tree);
        store.delete(host);
        assert!(store.claims_empty(), "the tree must leave no claim behind");
    }

    /// R2 — the 0→1→2 transitions, and `find` reaching each record.
    #[test]
    fn r2_insert_two_and_find() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 4);
        let a = add(&mut store, 0b1000);
        let b = add(&mut store, 0b0100);

        tree = rtree_insert(&mut store, tree, a, CODE);
        assert_eq!(rtree_len(&store, tree), 1);
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            0,
            "one record needs no node"
        );

        tree = rtree_insert(&mut store, tree, b, CODE);
        assert_eq!(rtree_len(&store, tree), 2);
        assert_eq!(store.get_u32_raw(tree, NODES), 1, "NODES == LEN - 1");
        rtree_validate(&store, tree, CODE);

        for r in [a, b] {
            let code = store.get_u32_raw(r, 4);
            assert_eq!(rtree_get(&store, tree, &code_probe(code), 32, CODE), r);
        }
    }

    /// R2b — `rtree_get` answers exactly, where `rtree_find` only guesses.
    #[test]
    fn r2b_get_is_exact_where_find_is_a_candidate() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 8);
        let recs: Vec<u32> = (0..8).map(|i| add(&mut store, i * 0x0100_0001)).collect();
        for &r in &recs {
            tree = rtree_insert(&mut store, tree, r, CODE);
        }
        for &r in &recs {
            let code = store.get_u32_raw(r, 4);
            assert_eq!(rtree_get(&store, tree, &code_probe(code), 32, CODE), r);
        }
        let absent = code_probe(0xdead_beef);
        assert_ne!(
            rtree_find(&store, tree, &absent, 32).rec(),
            0,
            "find guesses"
        );
        assert_eq!(
            rtree_get(&store, tree, &absent, 32, CODE),
            0,
            "get is exact"
        );
    }

    /// R2c — re-inserting a record the tree already holds is a no-op, not a panic
    /// and not a silent corruption.
    #[test]
    fn r2c_reinserting_the_same_record_is_a_noop() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 4);
        let a = add(&mut store, 7);
        tree = rtree_insert(&mut store, tree, a, CODE);
        tree = rtree_insert(&mut store, tree, a, CODE);
        assert_eq!(rtree_len(&store, tree), 1, "no phantom second copy");
        rtree_validate(&store, tree, CODE);

        let b = add(&mut store, 9);
        tree = rtree_insert(&mut store, tree, b, CODE);
        tree = rtree_insert(&mut store, tree, b, CODE);
        assert_eq!(rtree_len(&store, tree), 2);
        rtree_validate(&store, tree, CODE);
    }

    /// R2d — two records may share a user key.  They differ only in the id suffix,
    /// so they land adjacent: @PLN48's per-cell bucket, with no bucket structure.
    #[test]
    fn r2d_duplicate_user_keys_form_a_contiguous_bucket() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 8);
        // Three records in one "cell" (code 42), plus neighbours either side.
        let below = add(&mut store, 41);
        let cell: Vec<u32> = (0..3).map(|_| add(&mut store, 42)).collect();
        let above = add(&mut store, 43);
        for &r in std::iter::once(&below)
            .chain(&cell)
            .chain(std::iter::once(&above))
        {
            tree = rtree_insert(&mut store, tree, r, CODE);
        }
        assert_eq!(rtree_len(&store, tree), 5, "every record is stored");
        rtree_validate(&store, tree, CODE);

        let walk = collect(&store, tree);
        let mut expect = vec![below];
        let mut sorted = cell.clone();
        sorted.sort_unstable(); // within a cell, order is by record id
        expect.extend(sorted.iter().copied());
        expect.push(above);
        assert_eq!(
            walk, expect,
            "the cell's records are contiguous and in id order"
        );

        // Seeking the cell lands on its head; walking it out yields exactly the cell.
        let probe = code_probe(42);
        let mut it = rtree_seek(&store, tree, &probe, 32, CODE);
        assert_eq!(rtree_get(&store, tree, &probe, 32, CODE), sorted[0]);
        let mut bucket = Vec::new();
        let mut r = it.rec();
        while r != 0 && rtree_key_eq(&store, r, &probe, 32, CODE) {
            bucket.push(r);
            r = it.next(&store, tree).unwrap_or(0);
        }
        assert_eq!(bucket, sorted, "the bucket is a contiguous run");
    }

    /// R2e — `rtree_get`'s shortcut must reject a probe that merely *shares a prefix*.
    ///
    /// `get` no longer re-scans the key: it trusts `d`, the bit at which `seek` saw the
    /// probe leave the candidate.  The two ways that could go wrong are a probe that is
    /// a proper prefix of a stored key, and a probe that properly extends one — in both
    /// cases `d` lands inside the user key, well below `probe_bits + TERM_BITS`.
    #[test]
    fn r2e_get_rejects_prefixes_and_extensions() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        for w in ["abc", "abd", "abf", "zz"] {
            let r = add_text(&mut store, w);
            tree = rtree_insert(&mut store, tree, r, TEXT);
        }
        // "ab" is a proper prefix of a stored key, and is not itself stored.
        assert_eq!(rtree_get(&store, tree, &text_probe("ab"), 16, TEXT), 0);
        // "abcd" properly extends a stored key, and is not itself stored.
        assert_eq!(rtree_get(&store, tree, &text_probe("abcd"), 32, TEXT), 0);
        // "a" shares only one byte.
        assert_eq!(rtree_get(&store, tree, &text_probe("a"), 8, TEXT), 0);
        // "abe" sorts between "abd" and "abf", so `seek` lands on "abf" — the SAME
        // length as the probe.  The length check cannot reject it; only `d >= probe_bits`
        // can.  This is the witness for that half of the condition.
        assert_eq!(rtree_get(&store, tree, &text_probe("abe"), 24, TEXT), 0);
        // But the exact keys are found.
        for w in ["abc", "abd", "abf", "zz"] {
            let bits = w.len() as u32 * 8;
            assert_ne!(
                rtree_get(&store, tree, &text_probe(w), bits, TEXT),
                0,
                "{w}"
            );
        }
        // And `seek("ab")` still lands on the first key bearing that prefix.
        let it = rtree_seek(&store, tree, &text_probe("ab"), 16, TEXT);
        assert!(rtree_key_eq(&store, it.rec(), &text_probe("abc"), 24, TEXT));
    }

    /// R3 — I1 and I2 hold after every one of a thousand inserts.
    #[test]
    fn r3_insert_many_keeps_the_invariant() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x1234_5678_9abc_def0;
        for i in 0..1000u32 {
            let rec = add(&mut store, lcg(&mut seed));
            tree = rtree_insert(&mut store, tree, rec, CODE);
            assert_eq!(rtree_len(&store, tree), i + 1);
            rtree_validate(&store, tree, CODE);
        }
    }

    /// R4 — in-order traversal is strictly increasing; backward is its exact reverse.
    #[test]
    fn r4_walk_is_ordered_both_ways() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xfeed_face_dead_beef;
        let mut expect = Vec::new();
        for _ in 0..500 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            expect.push(ordered_key(code, rec));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        expect.sort_unstable();

        let forward = collect(&store, tree);
        let keys: Vec<u64> = forward
            .iter()
            .map(|&r| ordered_key(store.get_u32_raw(r, 4), r))
            .collect();
        assert_eq!(keys, expect, "in-order walk must be sorted by key");

        let mut it = rtree_last(&store, tree);
        let mut backward = vec![it.rec()];
        while let Some(r) = it.prev(&store, tree) {
            backward.push(r);
        }
        backward.reverse();
        assert_eq!(
            backward, forward,
            "the reverse walk must mirror the forward"
        );
    }

    /// R5 — `seek` is a true lower bound, including for keys the tree never held.
    #[test]
    fn r5_seek_is_a_lower_bound() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x0bad_c0de_0bad_c0de;
        let mut keys = Vec::new();
        for _ in 0..300 {
            let code = lcg(&mut seed) & !1; // even codes: odd probes are absent
            let rec = add(&mut store, code);
            keys.push(ordered_key(code, rec));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        keys.sort_unstable();

        let mut probe_seed = 0x5eed_5eed_5eed_5eed_u64;
        for _ in 0..300 {
            let code = lcg(&mut probe_seed) | 1; // odd: absent from the tree
            let it = rtree_seek(&store, tree, &code_probe(code), 32, CODE);
            // A probe carries id 0, so it sorts at the head of its own cell.
            let want = keys.iter().find(|&&k| k >= ordered_key(code, 0)).copied();
            let got =
                (it.rec() != 0).then(|| ordered_key(store.get_u32_raw(it.rec(), 4), it.rec()));
            assert_eq!(got, want, "seek({code:#x}) must be the least key >= it");
        }
    }

    /// R6 — removal against a `BTreeMap` oracle, ending in an empty, leak-free tree.
    #[test]
    fn r6_remove_matches_a_btreemap() {
        let mut store = Store::new_in_use(64);
        let host = store.claim(1);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xa5a5_5a5a_a5a5_5a5a;
        let mut oracle: BTreeMap<u64, u32> = BTreeMap::new();
        for _ in 0..300 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            oracle.insert(ordered_key(code, rec), rec);
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }

        let mut order: Vec<u32> = oracle.values().copied().collect();
        for i in 0..order.len() {
            let j = (lcg(&mut seed) as usize) % order.len();
            order.swap(i, j);
        }

        for rec in order {
            assert!(rtree_remove(&mut store, tree, rec, CODE), "remove {rec}");
            oracle.retain(|_, &mut v| v != rec);
            rtree_validate(&store, tree, CODE);
            assert_eq!(rtree_len(&store, tree) as usize, oracle.len());
            let want: Vec<u32> = oracle.values().copied().collect();
            assert_eq!(collect(&store, tree), want, "walk tracks the oracle");
            store.delete(rec);
        }

        assert_eq!(rtree_len(&store, tree), 0);
        rtree_free(&mut store, tree);
        store.delete(host);
        assert!(store.claims_empty(), "removal must not leak");
    }

    /// R6b — a removed node's id is reused rather than minted afresh.
    #[test]
    fn r6b_free_list_reuses_node_ids() {
        let mut store = Store::new_in_use(32);
        let mut tree = rtree_init(&mut store, 8);
        let recs: Vec<u32> = (0..8).map(|i| add(&mut store, i * 17)).collect();
        for &r in &recs {
            tree = rtree_insert(&mut store, tree, r, CODE);
        }
        let high_water = store.get_u32_raw(tree, NODES);
        assert!(rtree_remove(&mut store, tree, recs[3], CODE));
        assert_eq!(store.get_u32_raw(tree, NODES), high_water, "no new node");
        tree = rtree_insert(&mut store, tree, recs[3], CODE);
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            high_water,
            "the freed node id must be reused, not appended"
        );
        rtree_validate(&store, tree, CODE);
    }

    /// R7 — growing past `CAP` relocates the record, and the tree survives it.
    #[test]
    fn r7_growth_relocates_and_survives() {
        let mut store = Store::new_in_use(16);
        let mut tree = rtree_init(&mut store, 0);
        assert_eq!(store.get_u32_raw(tree, CAP), 0, "no room for nodes yet");

        let mut seed = 0x2222_3333_4444_5555;
        let mut expect = Vec::new();
        let mut moved = false;
        for _ in 0..200 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            expect.push(ordered_key(code, rec));
            let grown = rtree_insert(&mut store, tree, rec, CODE);
            moved |= grown != tree;
            tree = grown;
            rtree_validate(&store, tree, CODE);
        }
        assert!(
            moved,
            "200 inserts from CAP=0 must have relocated the record"
        );

        expect.sort_unstable();
        let keys: Vec<u64> = collect(&store, tree)
            .iter()
            .map(|&r| ordered_key(store.get_u32_raw(r, 4), r))
            .collect();
        assert_eq!(
            keys, expect,
            "growth must preserve every record and its order"
        );
    }

    // ---- multi-word keys: a 128-bit key from two i64 fields -----------------
    //
    // @PLN48 S2 relies on this: a loft `integer` is an i64, so a 2D Morton key over
    // two integer axes is 128 user bits — more than one word.  These prove the tree's
    // word-at-a-time `first_diff` and `View::bit` handle a key that spans words, and
    // that the terminator + id suffix past the second word still order correctly.

    fn wide_word(store: &Store, rec: u32, word: u32) -> u64 {
        // Two u64s at fld 4 and fld 12 (a 3-word record): word 0 is the high half.
        match word {
            0 => {
                u64::from(store.get_u32_raw(rec, 4)) | (u64::from(store.get_u32_raw(rec, 8)) << 32)
            }
            1 => {
                u64::from(store.get_u32_raw(rec, 12))
                    | (u64::from(store.get_u32_raw(rec, 16)) << 32)
            }
            _ => 0,
        }
    }
    fn wide_bits(_store: &Store, _rec: u32) -> u32 {
        128
    }
    const WIDE: KeySpec = KeySpec {
        word: wide_word,
        bits: wide_bits,
    };

    fn add_wide(store: &mut Store, hi: u64, lo: u64) -> u32 {
        let rec = store.claim(3);
        store.set_u32_raw(rec, 4, hi as u32);
        store.set_u32_raw(rec, 8, (hi >> 32) as u32);
        store.set_u32_raw(rec, 12, lo as u32);
        store.set_u32_raw(rec, 16, (lo >> 32) as u32);
        rec
    }
    fn wide_probe(hi: u64, lo: u64) -> impl Fn(u32) -> u64 {
        move |w| match w {
            0 => hi,
            1 => lo,
            _ => 0,
        }
    }

    /// R9 — a 128-bit key crosses the word boundary and still inserts, finds, and
    /// walks in order.  The differing bit lands in word 0 for some pairs and word 1
    /// for others, so the word-loop's "difference past the first word" branch runs.
    #[test]
    fn r9_multiword_keys_order_across_the_word_boundary() {
        let mut store = Store::new_in_use(1 << 15);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xc0ff_ee00_1234_5678;
        let mut expect: Vec<u128> = Vec::new();
        let mut recs = Vec::new();
        for _ in 0..500 {
            // Deliberately narrow the high word sometimes, so many keys share word 0
            // and first diverge in word 1 — the multi-word path.
            let hi = if lcg(&mut seed) & 1 == 0 {
                0
            } else {
                u64::from(lcg(&mut seed))
            };
            let lo = (u64::from(lcg(&mut seed)) << 32) | u64::from(lcg(&mut seed));
            let rec = add_wide(&mut store, hi, lo);
            recs.push((rec, hi, lo));
            expect.push((u128::from(hi) << 64) | u128::from(lo));
            tree = rtree_insert(&mut store, tree, rec, WIDE);
            rtree_validate(&store, tree, WIDE);
        }
        // exact lookup of every key
        for &(rec, hi, lo) in &recs {
            assert_eq!(rtree_get(&store, tree, &wide_probe(hi, lo), 128, WIDE), rec);
        }
        // in-order walk == u128 sort (ties on (hi,lo) break by rec id, mirrored below)
        expect.sort_unstable();
        let got: Vec<u128> = collect(&store, tree)
            .iter()
            .map(|&r| {
                let (_, hi, lo) = *recs.iter().find(|(x, _, _)| *x == r).unwrap();
                (u128::from(hi) << 64) | u128::from(lo)
            })
            .collect();
        assert_eq!(
            got, expect,
            "multi-word walk must be sorted by the full 128-bit key"
        );
    }

    /// R10 — every @PLN134 layout order is a PERMUTATION of the live nodes.
    ///
    /// The one property a layout measurement cannot survive without, and the one a
    /// printed page count will not reveal: an order that emits a node twice, or drops
    /// one, still produces a number — a better-looking number, because duplicates
    /// crowd a path onto fewer distinct pages. `veb_order`'s recursion splits a
    /// subtree by height and truncates the top half, which is exactly where a node
    /// can be emitted by both halves. So this is the harness's own gate, and it runs
    /// in the ordinary suite rather than beside the `#[ignore]` measurement.
    #[test]
    fn r10_layout_orders_are_permutations() {
        let mut store = Store::new_in_use(1 << 15);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x0bad_c0de_1234_5678;
        for _ in 0..2000 {
            let rec = add(&mut store, lcg(&mut seed));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        let live: std::collections::BTreeSet<u32> = bfs_order(&store, tree).into_iter().collect();
        assert_eq!(
            live.len(),
            rtree_len(&store, tree) as usize - 1,
            "a PATRICIA tree over n records has n-1 internal nodes"
        );
        for (name, order) in [
            ("bfs", bfs_order(&store, tree)),
            ("dfs", dfs_order(&store, tree)),
            ("key", key_order(&store, tree)),
            ("veb", veb_order(&store, tree)),
        ] {
            assert_eq!(
                order.len(),
                live.len(),
                "{name} emits {} of {} nodes",
                order.len(),
                live.len()
            );
            assert_eq!(
                order
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
                live,
                "{name} must emit every live node exactly once"
            );
        }
        // `key_order` claims the walk order the tree itself defines: the node above
        // each record, in key order, appears in that order. Checking the claim rather
        // than only the multiset is what separates a layout from a shuffle.
        let keyed = key_order(&store, tree);
        let mut rank = vec![u32::MAX; (store.get_u32_raw(tree, NODES) + 2) as usize];
        for (i, &n) in keyed.iter().enumerate() {
            rank[n as usize] = i as u32;
        }
        let mut it = rtree_first(&store, tree);
        let (mut prev, mut r) = (0u32, it.rec());
        while r != 0 {
            if it.node != 0 {
                assert!(
                    rank[it.node as usize] >= prev,
                    "key order must not go backwards at record {r}"
                );
                prev = rank[it.node as usize];
            }
            r = it.next(&store, tree).unwrap_or(0);
        }
    }

    /// Every node's four words, so two layouts can be compared byte for byte.
    fn node_array(store: &Store, tree: u32) -> Vec<[i32; 4]> {
        (1..=store.get_u32_raw(tree, NODES))
            .map(|n| {
                [
                    store.get_i32_raw(tree, node_off(n)),
                    store.get_i32_raw(tree, node_off(n) + 4),
                    store.get_i32_raw(tree, child_off(n, false)),
                    store.get_i32_raw(tree, child_off(n, true)),
                ]
            })
            .collect()
    }

    /// R11 — `rtree_relayout` moves every node and changes no answer.
    ///
    /// The pass rewrites the whole node array through a permutation, so the failure
    /// it invites is not a wrong order but a wrong TREE: rewriting in place clobbers
    /// nodes the pass has not read yet, and the result is still a structurally valid
    /// PATRICIA tree — `rtree_validate` alone would pass it. So the gate is the
    /// ANSWERS: the same walk, and the same record for every key that was in it.
    #[test]
    fn r11_relayout_preserves_every_answer() {
        let mut store = Store::new_in_use(1 << 15);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x5eed_1134_5eed_1134;
        let mut codes = Vec::new();
        for _ in 0..2000 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            codes.push((code, rec));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        let before_walk = collect(&store, tree);
        let before_len = rtree_len(&store, tree);
        let before_array = node_array(&store, tree);

        assert!(rtree_relayout(&mut store, tree), "a healthy tree lays out");
        rtree_validate(&store, tree, CODE);
        assert_eq!(rtree_len(&store, tree), before_len, "no record is lost");
        assert_eq!(
            collect(&store, tree),
            before_walk,
            "key order must survive the renumbering"
        );
        for &(code, rec) in &codes {
            assert_eq!(
                rtree_get(&store, tree, &code_probe(code), 32, CODE),
                rec,
                "every key must still find its own record"
            );
        }
        let after_array = node_array(&store, tree);
        assert_ne!(
            after_array, before_array,
            "insertion order and vEB order must differ, or this proves nothing"
        );
        assert_eq!(
            store.get_u32_raw(tree, NODES),
            before_len - 1,
            "a PATRICIA tree over n records has n-1 nodes, and the array now holds \
             exactly those"
        );
        assert_eq!(
            store.get_u32_raw(tree, FREE),
            0,
            "the free list is compacted"
        );

        // Idempotent: the layout is a function of the tree, so a second pass over an
        // already-laid-out tree is the identity. A pass that renumbered relative to
        // the CURRENT ids instead would drift on every persist.
        assert!(rtree_relayout(&mut store, tree));
        assert_eq!(
            node_array(&store, tree),
            after_array,
            "relayout is idempotent"
        );
    }

    /// R11b — removals leave holes; a relayout returns them.
    #[test]
    fn r11b_relayout_compacts_what_removal_freed() {
        let mut store = Store::new_in_use(1 << 15);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xdead_1134_beef_1134;
        let mut codes = Vec::new();
        for _ in 0..500 {
            let code = lcg(&mut seed);
            let rec = add(&mut store, code);
            codes.push((code, rec));
            tree = rtree_insert(&mut store, tree, rec, CODE);
        }
        for &(_, rec) in codes.iter().step_by(2) {
            assert!(rtree_remove(&mut store, tree, rec, CODE));
        }
        let high = store.get_u32_raw(tree, NODES);
        let len = rtree_len(&store, tree);
        assert!(
            high > len - 1,
            "the removals must have left holes, or the compaction is untested"
        );
        assert!(rtree_relayout(&mut store, tree));
        rtree_validate(&store, tree, CODE);
        assert_eq!(store.get_u32_raw(tree, NODES), len - 1);
        assert_eq!(store.get_u32_raw(tree, FREE), 0);
        for (i, &(code, rec)) in codes.iter().enumerate() {
            let want = if i % 2 == 0 { 0 } else { rec };
            assert_eq!(rtree_get(&store, tree, &code_probe(code), 32, CODE), want);
        }
    }

    /// R11c — the degenerate trees: none, one, and two records.
    ///
    /// A tree with no NODES is the shape where "renumber the array" has nothing to
    /// say and every off-by-one lives — an empty tree, and a single record hanging
    /// straight off TOP.
    #[test]
    fn r11c_relayout_handles_trees_with_no_nodes() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        assert!(rtree_relayout(&mut store, tree), "an empty tree lays out");
        assert_eq!(rtree_len(&store, tree), 0);

        let one = add(&mut store, 7);
        tree = rtree_insert(&mut store, tree, one, CODE);
        assert!(rtree_relayout(&mut store, tree));
        rtree_validate(&store, tree, CODE);
        assert_eq!(rtree_get(&store, tree, &code_probe(7), 32, CODE), one);

        let two = add(&mut store, 9);
        tree = rtree_insert(&mut store, tree, two, CODE);
        assert!(rtree_relayout(&mut store, tree));
        rtree_validate(&store, tree, CODE);
        assert_eq!(collect(&store, tree), vec![one, two]);
    }

    // ---- 2D Morton (Z-order) oracle: x at `fld 4`, y at `fld 8` -------------
    //
    // The @PLN48 key.  `bits` is 64, so a whole code is one word: the tree never
    // materialises it, yet `first_diff` compares it with a single XOR.

    /// Spread the 32 bits of `v` into the even bit positions of a `u64`.
    fn spread(v: u32) -> u64 {
        let mut x = u64::from(v);
        x = (x | (x << 16)) & 0x0000_ffff_0000_ffff;
        x = (x | (x << 8)) & 0x00ff_00ff_00ff_00ff;
        x = (x | (x << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
        x = (x | (x << 2)) & 0x3333_3333_3333_3333;
        x = (x | (x << 1)) & 0x5555_5555_5555_5555;
        x
    }

    /// Interleave x (even bits) with y (odd bits) — the Z-order curve.
    fn morton(x: u32, y: u32) -> u64 {
        spread(x) | (spread(y) << 1)
    }

    fn xy_word(store: &Store, rec: u32, word: u32) -> u64 {
        if word == 0 {
            morton(store.get_u32_raw(rec, 4), store.get_u32_raw(rec, 8))
        } else {
            0
        }
    }

    fn xy_bits(_store: &Store, _rec: u32) -> u32 {
        64
    }

    const XY: KeySpec = KeySpec {
        word: xy_word,
        bits: xy_bits,
    };

    fn morton_probe(code: u64) -> impl Fn(u32) -> u64 {
        move |word| if word == 0 { code } else { 0 }
    }

    fn add_xy(store: &mut Store, x: u32, y: u32) -> u32 {
        let rec = store.claim(2);
        store.set_u32_raw(rec, 4, x);
        store.set_u32_raw(rec, 8, y);
        rec
    }

    fn pos(store: &Store, rec: u32) -> (u32, u32) {
        (store.get_u32_raw(rec, 4), store.get_u32_raw(rec, 8))
    }

    fn code_of(store: &Store, rec: u32) -> u64 {
        let (x, y) = pos(store, rec);
        morton(x, y)
    }

    fn dist2(a: (u32, u32), b: (u32, u32)) -> i64 {
        let dx = i64::from(a.0) - i64::from(b.0);
        let dy = i64::from(a.1) - i64::from(b.1);
        dx * dx + dy * dy
    }

    /// Move an object.  The key is derived from the coordinates, so the index must be
    /// updated: remove under the old key, write, reinsert under the new one.
    fn move_to(store: &mut Store, tree: u32, rec: u32, x: u32, y: u32) -> u32 {
        assert!(rtree_remove(store, tree, rec, XY), "object was not indexed");
        store.set_u32_raw(rec, 4, x);
        store.set_u32_raw(rec, 8, y);
        rtree_insert(store, tree, rec, XY)
    }

    /// A small drift, clamped to the grid — how a real entity moves.
    fn drift(seed: &mut u64, p: (u32, u32), grid: u32) -> (u32, u32) {
        let step = |v: u32, s: &mut u64| {
            let d = i64::from(lcg(s) % 5) - 2; // -2 ..= +2
            (i64::from(v) + d).clamp(0, i64::from(grid) - 1) as u32
        };
        (step(p.0, seed), step(p.1, seed))
    }

    /// M1 — objects move through space, and the index reorders them.
    ///
    /// The reordering is the point.  The test asserts it actually happened (otherwise
    /// it would pass vacuously against a structure that ignored coordinates), and that
    /// after *every* move the walk still equals a fresh brute-force sort by
    /// `(morton, rec)`.
    #[test]
    fn m1_moving_objects_reorder_in_the_index() {
        const OBJECTS: usize = 200;
        const MOVES: usize = 300;
        const GRID: u32 = 256;

        let mut store = Store::new_in_use(1 << 14);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x51ed_0011_2233_4455;
        let mut objs = Vec::new();
        for _ in 0..OBJECTS {
            let (x, y) = (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID);
            let r = add_xy(&mut store, x, y);
            objs.push(r);
            tree = rtree_insert(&mut store, tree, r, XY);
        }
        rtree_validate(&store, tree, XY);

        let sorted = |store: &Store, objs: &[u32]| {
            let mut v: Vec<u32> = objs.to_vec();
            v.sort_by_key(|&r| (code_of(store, r), r));
            v
        };
        assert_eq!(collect(&store, tree), sorted(&store, &objs));

        let mut reorderings = 0;
        for i in 0..MOVES {
            let before = collect(&store, tree);
            let who = objs[(lcg(&mut seed) as usize) % OBJECTS];
            // Half the moves drift, half teleport — both must hold.
            let (x, y) = if i % 2 == 0 {
                drift(&mut seed, pos(&store, who), GRID)
            } else {
                (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID)
            };
            tree = move_to(&mut store, tree, who, x, y);

            rtree_validate(&store, tree, XY);
            let after = collect(&store, tree);
            assert_eq!(after.len(), OBJECTS, "no object lost or duplicated");
            assert_eq!(
                after,
                sorted(&store, &objs),
                "walk must track the coordinates"
            );
            if before != after {
                reorderings += 1;
            }
        }
        assert!(
            reorderings > MOVES / 4,
            "moves must really reorder the index ({reorderings} of {MOVES})"
        );
    }

    /// Every object whose position lies in the axis-aligned box, found by scanning the
    /// Morton interval between the box's two corners and filtering.
    ///
    /// This is exact — no false negatives — and the reason is a property of Z-order:
    /// interleaving is monotone per axis, so `x0<=x<=x1` and `y0<=y<=y1` imply
    /// `morton(x0,y0) <= morton(x,y) <= morton(x1,y1)`.  Every point of the box is
    /// therefore inside the code interval.  The interval also contains points *outside*
    /// the box (Z-order leaves the box and comes back), which the filter drops.
    fn box_query(store: &Store, tree: u32, lo: (u32, u32), hi: (u32, u32)) -> Vec<u32> {
        let (lo_code, hi_code) = (morton(lo.0, lo.1), morton(hi.0, hi.1));
        let probe = morton_probe(lo_code);
        let mut it = rtree_seek(store, tree, &probe, 64, XY);
        let mut out = Vec::new();
        let mut r = it.rec();
        while r != 0 {
            if code_of(store, r) > hi_code {
                break;
            }
            let (x, y) = pos(store, r);
            if x >= lo.0 && x <= hi.0 && y >= lo.1 && y <= hi.1 {
                out.push(r);
            }
            r = it.next(store, tree).unwrap_or(0);
        }
        out.sort_unstable();
        out
    }

    /// M2 — iterate from a position: an exact neighbourhood query over moving objects.
    ///
    /// Also pins the caveat @PLN48 warns about.  A raw walk outward from the query
    /// point's own code visits objects in **Morton** order, which is *not* distance
    /// order — Z-order jumps at quadrant boundaries.  The test asserts both: the box
    /// scan is exact, and the raw ±k walk really does miss true neighbours, so `near`
    /// can never be built on it alone.
    #[test]
    fn m2_query_from_a_position_is_exact_while_the_raw_walk_is_not() {
        const OBJECTS: usize = 400;
        const GRID: u32 = 256;
        const RADIUS: u32 = 12;
        const NEIGHBOURS: usize = 8;

        let mut store = Store::new_in_use(1 << 14);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0xbeef_2468_1357_9bdf;
        let mut objs = Vec::new();
        for _ in 0..OBJECTS {
            let (x, y) = (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID);
            let r = add_xy(&mut store, x, y);
            objs.push(r);
            tree = rtree_insert(&mut store, tree, r, XY);
        }
        // Let everything drift a while, so the index has been churned.
        for _ in 0..500 {
            let who = objs[(lcg(&mut seed) as usize) % OBJECTS];
            let (x, y) = drift(&mut seed, pos(&store, who), GRID);
            tree = move_to(&mut store, tree, who, x, y);
        }
        rtree_validate(&store, tree, XY);

        let mut raw_walk_missed = 0;
        for _ in 0..200 {
            let q = (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID);
            let lo = (q.0.saturating_sub(RADIUS), q.1.saturating_sub(RADIUS));
            let hi = ((q.0 + RADIUS).min(GRID - 1), (q.1 + RADIUS).min(GRID - 1));

            // Exact: box scan over the Morton interval.
            let got = box_query(&store, tree, lo, hi);
            let mut want: Vec<u32> = objs
                .iter()
                .copied()
                .filter(|&r| {
                    let (x, y) = pos(&store, r);
                    x >= lo.0 && x <= hi.0 && y >= lo.1 && y <= hi.1
                })
                .collect();
            want.sort_unstable();
            assert_eq!(got, want, "box query at {q:?} must match brute force");

            // Approximate: the raw ±k Morton walk from the query point's own code.
            let probe = morton_probe(morton(q.0, q.1));
            let start = rtree_seek(&store, tree, &probe, 64, XY);
            let (mut back, mut fore) = (start, start);
            let mut found = vec![];
            if start.rec() != 0 {
                found.push(start.rec());
            }
            for _ in 0..NEIGHBOURS {
                if let Some(r) = back.prev(&store, tree) {
                    found.push(r);
                }
                if let Some(r) = fore.next(&store, tree) {
                    found.push(r);
                }
            }
            // The true nearest object, by brute force.
            let nearest = objs
                .iter()
                .copied()
                .min_by_key(|&r| dist2(pos(&store, r), q))
                .unwrap();
            if !found.contains(&nearest) {
                raw_walk_missed += 1;
            }
        }
        assert!(
            raw_walk_missed > 0,
            "a raw Morton walk is supposed to miss true neighbours — if it never does, \
             the test grid is too small to exhibit a Z-order discontinuity"
        );
        println!(
            "raw ±{NEIGHBOURS} Morton walk missed the true nearest in {raw_walk_missed}/200 queries"
        );
    }

    /// M3 — `rtree_move` must build **exactly** the tree that remove-then-insert builds.
    ///
    /// A PATRICIA shape is uniquely determined by its key set, so two trees over the
    /// same records must walk identically.  Both are maintained over the *same* records
    /// through the same move sequence — one by the plain path, one by the finger — and
    /// compared after every single move.  Drifts and teleports are interleaved, so the
    /// climb, the cap, and the root-restart fallback are all exercised.
    #[test]
    fn m3_finger_move_matches_remove_then_insert() {
        const OBJECTS: usize = 200;
        const MOVES: usize = 400;
        const GRID: u32 = 256;

        let mut store = Store::new_in_use(1 << 15);
        let mut plain = rtree_init(&mut store, 0);
        let mut finger = rtree_init(&mut store, 0);
        let mut seed = 0x0f1e_2d3c_4b5a_6978;

        let mut objs = Vec::new();
        for _ in 0..OBJECTS {
            let (x, y) = (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID);
            let r = add_xy(&mut store, x, y);
            objs.push(r);
            plain = rtree_insert(&mut store, plain, r, XY);
            finger = rtree_insert(&mut store, finger, r, XY);
        }
        assert_eq!(collect(&store, plain), collect(&store, finger));

        let mut teleports = 0;
        for i in 0..MOVES {
            let who = objs[(lcg(&mut seed) as usize) % OBJECTS];
            let teleport = i % 3 == 0;
            let (x, y) = if teleport {
                teleports += 1;
                (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID)
            } else {
                drift(&mut seed, pos(&store, who), GRID)
            };

            // Plain tree: unlink under the OLD key, before the coordinates change.
            assert!(rtree_remove(&mut store, plain, who, XY));
            // Finger tree: unlink, mutate, re-insert from the finger.
            finger = rtree_move(&mut store, finger, who, XY, |s| {
                s.set_u32_raw(who, 4, x);
                s.set_u32_raw(who, 8, y);
            });
            // Plain tree: re-insert under the NEW key, from the root.
            plain = rtree_insert(&mut store, plain, who, XY);

            rtree_validate(&store, plain, XY);
            rtree_validate(&store, finger, XY);
            assert_eq!(
                collect(&store, plain),
                collect(&store, finger),
                "finger move diverged from remove+insert at move {i}"
            );
            assert_eq!(
                rtree_len(&store, plain),
                rtree_len(&store, finger),
                "length diverged at move {i}"
            );
        }
        assert!(teleports > 0, "the fallback path must be exercised");
    }

    /// BENCH — the game move: plain remove+insert against the finger, for a drifting
    /// entity and for a teleporting one.
    ///
    /// Run: `cargo test --release --lib radix_tree::tests::bench_move_finger -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark — run with --release --ignored --nocapture"]
    fn bench_move_finger() {
        use std::time::Instant;
        const OBJECTS: usize = 512;
        const FRAMES: usize = 400;
        const GRID: u32 = 1024;
        const ROUNDS: usize = 3;
        const CELL: u32 = 3; // quantize to 8×8 cells

        let build = |store: &mut Store, seed: &mut u64| {
            let mut tree = rtree_init(store, 0);
            let objs: Vec<u32> = (0..OBJECTS)
                .map(|_| add_xy(store, lcg(seed) % GRID, lcg(seed) % GRID))
                .collect();
            for &r in &objs {
                tree = rtree_insert(store, tree, r, XY);
            }
            (tree, objs)
        };

        println!("\n@PLN48 game move — {OBJECTS} entities, {FRAMES} frames, best of {ROUNDS}");
        println!(
            "  {:<10} {:>12} {:>12} {:>9}",
            "motion", "remove+ins", "rtree_move", "speedup"
        );

        for &teleport in &[false, true] {
            let (mut best_plain, mut best_finger) = (f64::MAX, f64::MAX);
            for _ in 0..ROUNDS {
                for &use_finger in &[false, true] {
                    let mut store = Store::new_in_use(1 << 15);
                    let mut seed = 0xa1b2_c3d4_e5f6_0718;
                    let (mut tree, objs) = build(&mut store, &mut seed);

                    let t = Instant::now();
                    for _ in 0..FRAMES {
                        for &r in &objs {
                            let (x, y) = if teleport {
                                (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID)
                            } else {
                                drift(&mut seed, pos(&store, r), GRID)
                            };
                            if use_finger {
                                tree = rtree_move(&mut store, tree, r, XY, |s| {
                                    s.set_u32_raw(r, 4, x);
                                    s.set_u32_raw(r, 8, y);
                                });
                            } else {
                                tree = move_to(&mut store, tree, r, x, y);
                            }
                        }
                    }
                    let ns = per_op(t.elapsed(), FRAMES * OBJECTS);
                    if use_finger {
                        best_finger = best_finger.min(ns);
                    } else {
                        best_plain = best_plain.min(ns);
                    }
                    rtree_validate(&store, tree, XY);
                }
            }
            let label = if teleport { "teleport" } else { "drift ±2" };
            println!(
                "  {label:<10} {best_plain:>9.1} ns {best_finger:>9.1} ns {:>8.2}x",
                best_plain / best_finger
            );
        }

        // The biggest lever is not in the tree at all: quantize the key to a cell, and
        // most drifts do not change it, so the index needs no update whatsoever.
        let mut store = Store::new_in_use(1 << 15);
        let mut seed = 0xa1b2_c3d4_e5f6_0718;
        let (_tree, objs) = build(&mut store, &mut seed);
        let (mut changed_raw, mut changed_cell, mut total) = (0u64, 0u64, 0u64);
        for _ in 0..FRAMES {
            for &r in &objs {
                let old = pos(&store, r);
                let new = drift(&mut seed, old, GRID);
                total += 1;
                changed_raw += u64::from(morton(old.0, old.1) != morton(new.0, new.1));
                changed_cell += u64::from(
                    morton(old.0 >> CELL, old.1 >> CELL) != morton(new.0 >> CELL, new.1 >> CELL),
                );
                store.set_u32_raw(r, 4, new.0);
                store.set_u32_raw(r, 8, new.1);
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let pct = |n: u64| 100.0 * n as f64 / total as f64;
        println!(
            "\n  drifts that change the key: raw coords {:.0}%   quantized to {}×{} cells {:.0}%",
            pct(changed_raw),
            1 << CELL,
            1 << CELL,
            pct(changed_cell)
        );
        println!("  (a key that does not change needs no reindex at all)");
    }

    /// Bit `b` of an object's full key, mirroring `View::bit` for a 64-bit code.
    fn xy_key_bit(code: u64, rec: u32, b: u32) -> bool {
        if b < 64 {
            (code >> (63 - b)) & 1 == 1
        } else if b < 64 + TERM_BITS {
            false
        } else if b < 64 + SUFFIX_BITS {
            (rec >> (103 - b)) & 1 == 1
        } else {
            false
        }
    }

    /// Nodes visited descending by `(code, rec)` from `from`.
    fn count_descend(store: &Store, tree: u32, from: Child, code: u64, rec: u32) -> usize {
        let mut n = 0;
        let mut cur = from;
        while let Child::Node(x) = cur {
            n += 1;
            let b = node_bit(store, tree, x);
            cur = child(store, tree, x, xy_key_bit(code, rec, b));
        }
        n
    }

    /// PROBE — how much could a finger (climb from the old spot) actually save?
    ///
    /// Run: `cargo test --release --lib radix_tree::tests::probe_finger -- --ignored --nocapture`
    ///
    /// For each move it compares the nodes a descent from the ROOT would visit against
    /// `climb + descent from the deepest ancestor whose subtree still holds the new
    /// key`.  The ancestor is found by I2: `bit(A) <= e` iff the new key still belongs
    /// under `A`, where `e` is the first bit at which the old and new codes differ.
    #[test]
    #[ignore = "probe — run with --release --ignored --nocapture"]
    fn probe_finger_savings() {
        const OBJECTS: usize = 512;
        const GRID: u32 = 256;
        const SAMPLES: usize = 4000;

        let mut store = Store::new_in_use(1 << 14);
        let mut tree = rtree_init(&mut store, 0);
        let mut seed = 0x1357_9bdf_2468_ace0;
        let mut objs = Vec::new();
        for _ in 0..OBJECTS {
            let (x, y) = (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID);
            let r = add_xy(&mut store, x, y);
            objs.push(r);
            tree = rtree_insert(&mut store, tree, r, XY);
        }

        // A capped climb: give up after `k` steps and descend from the root instead.
        // cost(k) = climbed + descent-from-A   when the finger reached A within k,
        //           k + descent-from-root      when it did not (the climb is wasted).
        const CAPS: [usize; 7] = [0, 1, 2, 3, 4, 5, 64];
        let mut mixed = [0f64; CAPS.len()];

        for &teleport in &[false, true] {
            let mut root_nodes = 0usize;
            let mut cost = [0usize; CAPS.len()];
            let mut e_sum = 0u64;
            for _ in 0..SAMPLES {
                let who = objs[(lcg(&mut seed) as usize) % OBJECTS];
                let old = code_of(&store, who);
                let (nx, ny) = if teleport {
                    (lcg(&mut seed) % GRID, lcg(&mut seed) % GRID)
                } else {
                    drift(&mut seed, pos(&store, who), GRID)
                };
                let new = morton(nx, ny);

                let ow = |w: u32| (XY.word)(&store, who, w);
                let key = View {
                    word: ow,
                    bits: 64,
                    id: who,
                };
                let (_leaf, parent) = descend(&store, tree, &key);

                // `e`: first bit at which the new code leaves the old.
                let e = if old == new {
                    64
                } else {
                    (old ^ new).leading_zeros()
                };

                // Climb to the deepest ancestor that still contains the new key.
                let (mut a, mut climbed) = (parent, 0usize);
                while a != 0 && node_bit(&store, tree, a) > e {
                    a = node_parent(&store, tree, a);
                    climbed += 1;
                }

                let root = top(&store, tree);
                let from_root = count_descend(&store, tree, root, new, who);
                let from_a = if a == 0 {
                    from_root
                } else {
                    count_descend(&store, tree, Child::Node(a), new, who)
                };
                root_nodes += from_root;
                e_sum += u64::from(e);
                for (i, &k) in CAPS.iter().enumerate() {
                    cost[i] += if climbed <= k {
                        climbed + from_a
                    } else {
                        k + from_root
                    };
                }
                tree = move_to(&mut store, tree, who, nx, ny);
            }

            #[allow(clippy::cast_precision_loss)]
            let n = SAMPLES as f64;
            let label = if teleport { "teleport" } else { "drift ±2" };
            #[allow(clippy::cast_precision_loss)]
            let root_avg = root_nodes as f64 / n;
            #[allow(clippy::cast_precision_loss)] // e_sum <= SAMPLES*64
            let mean_e = e_sum as f64 / n;
            print!("  {label:<9} root {root_avg:>5.2}  mean e {mean_e:>4.1} |");
            for (i, &k) in CAPS.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let c = cost[i] as f64 / n;
                let tag = if k == 64 {
                    "inf".to_string()
                } else {
                    k.to_string()
                };
                print!("  k={tag}:{c:>5.2}");
                // 95% drift / 5% teleport, the shape of a real frame.
                mixed[i] += if teleport { 0.05 * c } else { 0.95 * c };
            }
            println!();
        }
        print!("  {:<9} {:>21} |", "mixed 95/5", "");
        for (i, &k) in CAPS.iter().enumerate() {
            let tag = if k == 64 {
                "inf".to_string()
            } else {
                k.to_string()
            };
            print!("  k={tag}:{:>5.2}", mixed[i]);
        }
        println!();
    }

    // ---- variable-length oracle: a byte string at `fld 8`, length at `fld 4` ----

    fn text_word(store: &Store, rec: u32, word: u32) -> u64 {
        let len = store.get_u32_raw(rec, 4);
        let mut out = 0u64;
        for i in 0..8 {
            let idx = word * 8 + i;
            if idx < len {
                let byte = store.get_byte(rec, 8 + idx, 0) as u64;
                out |= byte << (56 - 8 * i);
            }
        }
        out
    }

    fn text_bits(store: &Store, rec: u32) -> u32 {
        store.get_u32_raw(rec, 4) * 8
    }

    const TEXT: KeySpec = KeySpec {
        word: text_word,
        bits: text_bits,
    };

    fn add_text(store: &mut Store, s: &str) -> u32 {
        let rec = store.claim(2); // 16 bytes: len at 4, up to 8 bytes at 8..16
        store.set_u32_raw(rec, 4, s.len() as u32);
        for (i, b) in s.bytes().enumerate() {
            store.set_byte(rec, 8 + i as u32, 0, i32::from(b));
        }
        rec
    }

    fn text_probe(s: &'static str) -> impl Fn(u32) -> u64 {
        move |word| {
            let b = s.as_bytes();
            let mut out = 0u64;
            for i in 0..8 {
                let idx = (word * 8 + i) as usize;
                if idx < b.len() {
                    out |= u64::from(b[idx]) << (56 - 8 * i);
                }
            }
            out
        }
    }

    /// R8 — variable-length keys sort lexicographically, and a *prefix* probe seeks
    /// to the first record bearing that prefix.
    ///
    /// This is what the `0x00` terminator buys.  Without it `"ab"` versus `"abc"`
    /// would compare a record id against `'c'`, and which sorted first would depend
    /// on allocation order.
    #[test]
    fn r8_text_keys_sort_lexicographically_and_seek_by_prefix() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        // Inserted in an order unrelated to their sort order.
        let words = ["b", "abd", "ab", "abc", "a"];
        let mut rec_of = Vec::new();
        for w in words {
            let r = add_text(&mut store, w);
            rec_of.push((w, r));
            tree = rtree_insert(&mut store, tree, r, TEXT);
        }
        rtree_validate(&store, tree, TEXT);

        let order: Vec<&str> = collect(&store, tree)
            .iter()
            .map(|r| rec_of.iter().find(|(_, x)| x == r).unwrap().0)
            .collect();
        assert_eq!(
            order,
            ["a", "ab", "abc", "abd", "b"],
            "a shorter key must precede the longer key that extends it"
        );

        // A probe carries id 0, so `seek("ab")` lands on "ab" itself, and walking on
        // yields every key with that prefix.
        let it = rtree_seek(&store, tree, &text_probe("ab"), 16, TEXT);
        let head = rec_of.iter().find(|(w, _)| *w == "ab").unwrap().1;
        assert_eq!(it.rec(), head, "seek(\"ab\") lands on \"ab\"");
        assert_eq!(rtree_get(&store, tree, &text_probe("ab"), 16, TEXT), head);

        // `seek` of a prefix that is not itself a key still lands on its first extension.
        let it = rtree_seek(&store, tree, &text_probe("abc"), 24, TEXT);
        assert_eq!(
            it.rec(),
            rec_of.iter().find(|(w, _)| *w == "abc").unwrap().1
        );
        assert_eq!(
            rtree_get(&store, tree, &text_probe("b"), 8, TEXT),
            rec_of[0].1
        );
    }

    /// R8c — the terminator is load-bearing, and only a **large** record id shows it.
    ///
    /// Without the `0x00` terminator, `"ab"` versus `"abc"` compares the id's leading
    /// byte against `'c'` (0x63).  Small ids have a leading byte of `0x00`, which
    /// silently plays the terminator's part — so R8 alone cannot tell the two designs
    /// apart.  These ids lead with `0x70`, which is *above* `'c'`, so dropping the
    /// terminator flips the order.  The tree never dereferences a record, so the
    /// oracle can hand it ids no store would allocate.
    #[test]
    fn r8c_terminator_orders_a_prefix_below_its_extension() {
        const AB: u32 = 0x7000_0001;
        const ABC: u32 = 0x7000_0002;

        fn word(rec: u32) -> &'static [u8] {
            if rec == AB { b"ab" } else { b"abc" }
        }
        fn big_word(_store: &Store, rec: u32, w: u32) -> u64 {
            let b = word(rec);
            let mut out = 0u64;
            for i in 0..8 {
                let idx = (w * 8 + i) as usize;
                if idx < b.len() {
                    out |= u64::from(b[idx]) << (56 - 8 * i);
                }
            }
            out
        }
        fn big_bits(_store: &Store, rec: u32) -> u32 {
            word(rec).len() as u32 * 8
        }
        const BIG: KeySpec = KeySpec {
            word: big_word,
            bits: big_bits,
        };

        assert!(
            AB >> 24 > u32::from(b'c'),
            "the probe needs a leading byte above 'c'"
        );

        let mut store = Store::new_in_use(16);
        let mut tree = rtree_init(&mut store, 4);
        for r in [ABC, AB] {
            tree = rtree_insert(&mut store, tree, r, BIG);
        }
        rtree_validate(&store, tree, BIG);
        assert_eq!(
            collect(&store, tree),
            vec![AB, ABC],
            "\"ab\" must precede \"abc\" whatever the record ids are"
        );
    }

    /// R8b — a text key repeated: distinct records, same key, adjacent, both found.
    #[test]
    fn r8b_repeated_text_keys_are_distinct_records() {
        let mut store = Store::new_in_use(64);
        let mut tree = rtree_init(&mut store, 0);
        let a = add_text(&mut store, "dup");
        let b = add_text(&mut store, "dup");
        let c = add_text(&mut store, "dux");
        for r in [a, b, c] {
            tree = rtree_insert(&mut store, tree, r, TEXT);
        }
        assert_eq!(rtree_len(&store, tree), 3);
        rtree_validate(&store, tree, TEXT);

        let walk = collect(&store, tree);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        assert_eq!(
            walk,
            vec![lo, hi, c],
            "equal keys are adjacent, ordered by id"
        );
        assert_eq!(rtree_get(&store, tree, &text_probe("dup"), 24, TEXT), lo);
    }
}
