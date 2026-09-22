// Copyright (c) 2023-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

// Should be removed when actually in use
#![allow(dead_code)]

use crate::keys;
use crate::keys::{Content, DbRef, Key};
use crate::store::Store;
use std::cmp::Ordering;

// Negative values in LEFT or RIGHT position are back links to higher data.
// For normal traversal they should be treated as a leaf node (value 0).
static RB_LEFT: u32 = 0;
static RB_RIGHT: u32 = 4;
static RB_FLAG: u32 = 8;
static RB_MAX_DEPTH: u32 = 30;

// Normally rec holds the position towards the LEFT, RiGHT, FLAG fields.
// However, the compare functions assume pos = 0 for records outside a vector.
/**
Get the lowest matching record, with `before` return the record before the lowest.
The `fields` parameter points to the position inside the record of the fields.
*/
#[must_use]
pub fn find(
    data: &DbRef,
    before: bool,
    fields: u16,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> u32 {
    let store = keys::store(data, stores);
    let mut rec = store.get_i32_raw(data.rec, data.pos) as u32;
    let mut result = DbRef {
        store_nr: data.store_nr,
        rec: 0,
        pos: 0,
    };
    let mut cmp = Ordering::Equal;
    let fast = keys::fast_order(keys, key);
    while rec > 0 {
        result.rec = rec;
        result.pos = 8;
        cmp = keys::order_key(fast.as_ref(), key, &result, stores, keys);
        let action = if cmp == Ordering::Equal {
            if before {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        } else {
            cmp
        };
        let to = store.get_i32_raw(
            rec,
            u32::from(fields)
                + if action == Ordering::Less {
                    RB_LEFT
                } else {
                    RB_RIGHT
                },
        );
        rec = if to >= 0 { to as u32 } else { 0 };
    }
    result.pos = u32::from(fields);
    if cmp == Ordering::Equal {
        if before {
            return previous(store, &result);
        }
        return next(store, &result);
    }
    // loft#691 — the key is NOT in the tree, so the descent stopped at the leaf where it
    // would be inserted.  That leaf sits on whichever side the tree shape put it, so
    // returning it raw answered "a neighbour" rather than "the neighbour asked for":
    // `ix[0..9]` over keys 1..5 skipped key 1, because the descent for the missing `0`
    // ended on key 1 (its SUCCESSOR) and the caller then stepped one past it.  Step to
    // the requested side so `before` always means the last node strictly BELOW `key`,
    // and `!before` the first node strictly ABOVE it — which is what the exact-match
    // lookup, the forward cursors and the reverse cursors all already assumed.
    // `cmp` compares `key` against that leaf.
    if before {
        if cmp == Ordering::Greater {
            result.rec
        } else {
            previous(store, &result)
        }
    } else if cmp == Ordering::Less {
        result.rec
    } else {
        next(store, &result)
    }
}

/// The record that carries exactly `key`, or 0 — the point lookup (@FR-Col-Lookup).
///
/// [`find`] answers a BOUNDARY: it never stops at an equal key, runs to the leaf, and its
/// caller then steps to the neighbour and compares once more — the right shape for a
/// range's ends and for a PARTIAL key, which several records match.  A full key matches
/// at most one (an insert displaces its duplicate, @FR-Col-Insert), so the descent can
/// stop at it: the caller passes a key as long as `keys`, and the comparator is resolved
/// once for the whole descent ([`keys::fast_order`]).
#[must_use]
pub fn find_exact(
    data: &DbRef,
    fields: u16,
    stores: &[Store],
    keys: &[Key],
    key: &[Content],
) -> u32 {
    let store = keys::store(data, stores);
    let fast = keys::fast_order(keys, key);
    // The root is a link like any other, so a negative one is no node: that is what an
    // ABSENT collection's reserved id reads as (loft#1213), and `add` treats it the same.
    let root = store.get_i32_raw(data.rec, data.pos);
    let mut node = DbRef {
        store_nr: data.store_nr,
        rec: if root > 0 { root as u32 } else { 0 },
        pos: 8,
    };
    while node.rec > 0 {
        let to = match keys::order_key(fast.as_ref(), key, &node, stores, keys) {
            Ordering::Equal => return node.rec,
            Ordering::Less => store.get_i32_raw(node.rec, u32::from(fields) + RB_LEFT),
            Ordering::Greater => store.get_i32_raw(node.rec, u32::from(fields) + RB_RIGHT),
        };
        // A negative link is a thread to an in-order neighbour: a leaf, for a descent.
        node.rec = if to >= 0 { to as u32 } else { 0 };
    }
    0
}

/// The `(start, finish)` cursor pair for iterating the range `from ..(=) till`.
///
/// The single home for the fact both the interpreter (`State::iterate`) and the native
/// runtime (`codegen_runtime::OpIterate`) need — they derived it separately before, and
/// the copies drifted: reverse iteration ignored its lower bound, an inclusive upper
/// bound stepped one too far, and an empty range iterated the whole collection
/// (loft#691).
///
/// The range is TWO facts — the lowest and the highest node it contains, both in TREE
/// order — and each direction only names which end it starts from:
/// * forward — `start` = one before `low` (`step` applies `next` before yielding),
///   `finish` = `high`, which `step` yields and then stops on.
/// * reverse — `start` = one past `high`, `finish` = `low`.
///
/// `0` is "no such node", and `next`/`previous` also return it walking off an end, so an
/// EMPTY range cannot be reported as `finish = 0` — `step` reads that as "walk to the
/// tree's end". It returns `finish = u32::MAX` instead, which `step` refuses outright.
/// For the same reason an ABSENT bound keeps `0` rather than the concrete first/last
/// node: `#remove` during iteration restructures the tree under the walk, so a baked end
/// node can stop it early.
///
/// `from` is always INCLUSIVE and `ex` describes `till`, whichever way the keys are
/// declared. A descending key does not move a bound to the other end of the tree:
/// `find` navigates through `keys::key_compare`, which reverses per descending key, so
/// `from` is the tree-earlier bound in EVERY declaration. Direction lives in the
/// comparator and is applied exactly once (@FR-Col-Order-Sign).
#[must_use]
// The arguments are the range's own description; bundling them into a struct would add a
// type whose only job is to be unpacked here and packed at the two call sites.
#[allow(clippy::too_many_arguments)]
pub fn range_cursors(
    data: &DbRef,
    fields: u16,
    stores: &[Store],
    keys: &[Key],
    from: &[Content],
    till: &[Content],
    ex: bool,
    reverse: bool,
) -> (u32, u32) {
    let store = keys::store(data, stores);
    let node = |rec: u32| DbRef {
        store_nr: data.store_nr,
        rec,
        pos: u32::from(fields),
    };
    // Which user bound sits at which end of TREE order.
    let (lo_key, lo_inclusive, hi_key, hi_inclusive) = (from, true, till, !ex);

    let low = if lo_key.is_empty() {
        first(data, fields, stores).rec
    } else if lo_inclusive {
        let below = find(data, true, fields, stores, keys, lo_key);
        if below == 0 {
            first(data, fields, stores).rec
        } else {
            next(store, &node(below))
        }
    } else {
        find(data, false, fields, stores, keys, lo_key)
    };
    let high = if hi_key.is_empty() {
        last(data, fields, stores).rec
    } else if hi_inclusive {
        let above = find(data, false, fields, stores, keys, hi_key);
        if above == 0 {
            last(data, fields, stores).rec
        } else {
            previous(store, &node(above))
        }
    } else {
        find(data, true, fields, stores, keys, hi_key)
    };

    // Each end existing is not yet "the range is non-empty": an inverted or degenerate
    // range (`ix[4..2]`, `ix[3..3]`) leaves both set with `low` past `high`. Comparing
    // the upper bound against `low`'s record settles it without needing tree order.
    let empty = low == 0
        || high == 0
        || (!hi_key.is_empty() && {
            let mut low_ref = node(low);
            low_ref.pos = 8;
            let c = keys::key_compare(hi_key, &low_ref, stores, keys);
            c == Ordering::Less || (!hi_inclusive && c == Ordering::Equal)
        });
    if empty {
        return (0, u32::MAX);
    }
    if reverse {
        let start = if hi_key.is_empty() {
            0
        } else {
            next(store, &node(high))
        };
        (start, if lo_key.is_empty() { 0 } else { low })
    } else {
        let start = if lo_key.is_empty() {
            0
        } else {
            previous(store, &node(low))
        };
        (start, if hi_key.is_empty() { 0 } else { high })
    }
}

/// Add a new record.
///
/// Answers 0 when the record went in, and otherwise the record that already carries its
/// key — the tree is then UNCHANGED (a refused `put` returns before it links or balances
/// anything), so the caller can displace that record and add again (@FR-Col-Insert).  That
/// is what lets an insert find its duplicate on the descent it makes anyway, instead of
/// looking the key up first and descending a second time.
///
/// The comparator is resolved once from the new record's own key
/// ([`keys::fast_order_of`]).  A text key keeps the general one here: `put` rebalances
/// the stores it compares against as it unwinds, so nothing borrowed from them can be held.
pub fn add(data: &DbRef, record: &DbRef, fields: u16, stores: &mut [Store], keys: &[Key]) -> u32 {
    let fast = {
        let mut own = *record;
        own.pos = 8;
        keys::fast_order_of(&own, stores, keys).and_then(|f| f.detached())
    };
    let store = keys::mut_store(data, stores);
    let mut rec = *record;
    rec.pos = u32::from(fields);
    store.set_byte(rec.rec, rec.pos + RB_FLAG, 0, 0);
    store.set_i32_raw(rec.rec, rec.pos + RB_LEFT, 0);
    store.set_i32_raw(rec.rec, rec.pos + RB_RIGHT, 0);
    let mut duplicate = 0;
    let new_top = if store.get_i32_raw(data.rec, data.pos) == 0 {
        rec.rec
    } else {
        let top = store.get_i32_raw(data.rec, data.pos);
        let mut walk = Put {
            rec: &rec,
            keys,
            fast: fast.as_ref(),
            duplicate: &mut duplicate,
        };
        walk.put(0, top, 0, 0, stores) as u32
    };
    if new_top == 0 || new_top == u32::MAX {
        return duplicate; // problem encountered: probably duplicate key
    }
    let store = keys::mut_store(data, stores);
    store.set_i32_raw(data.rec, data.pos, new_top as i32);
    store.set_byte(new_top, rec.pos + RB_FLAG, 0, 0);
    0
}

#[must_use]
/// Return the first element in the tree
pub fn first(data: &DbRef, fields: u16, stores: &[Store]) -> DbRef {
    end(data, fields, stores, RB_LEFT)
}

#[must_use]
/// Return the last element in the tree
pub fn last(data: &DbRef, fields: u16, stores: &[Store]) -> DbRef {
    end(data, fields, stores, RB_RIGHT)
}

fn end(data: &DbRef, fields: u16, stores: &[Store], forward: u32) -> DbRef {
    let store = keys::store(data, stores);
    let mut i = store.get_i32_raw(data.rec, data.pos);
    while i > 0 && store.get_i32_raw(i as u32, u32::from(fields) + forward) > 0 {
        i = store.get_i32_raw(i as u32, u32::from(fields) + forward);
    }
    DbRef {
        store_nr: data.store_nr,
        rec: i as u32,
        pos: 8,
    }
}

fn left(store: &Store, r: &DbRef) -> i32 {
    store.get_i32_raw(r.rec, r.pos + RB_LEFT)
}

fn right(store: &Store, r: &DbRef) -> i32 {
    store.get_i32_raw(r.rec, r.pos + RB_RIGHT)
}

fn flag(store: &Store, r: &DbRef) -> bool {
    store.get_byte(r.rec, r.pos + RB_FLAG, 0) == 1
}

fn set_left(store: &mut Store, r: &DbRef, to: i32) {
    store.set_i32_raw(r.rec, r.pos + RB_LEFT, to);
}

fn set_right(store: &mut Store, r: &DbRef, to: i32) {
    store.set_i32_raw(r.rec, r.pos + RB_RIGHT, to);
}

fn set_flag(store: &mut Store, r: &DbRef, to: bool) {
    store.set_byte(r.rec, r.pos + RB_FLAG, 0, i32::from(to));
}

/// What one insert's descent carries down unchanged: the record, how to order it, and
/// where to report the record whose key it duplicates.
struct Put<'a> {
    rec: &'a DbRef,
    keys: &'a [Key],
    fast: Option<&'a keys::FastOrder<'static>>,
    duplicate: &'a mut u32,
}

impl Put<'_> {
    /// Find the correct position to insert the element
    fn put(&mut self, depth: u32, pos: i32, l: i32, r: i32, stores: &mut [Store]) -> i32 {
        let rec = self.rec;
        if depth > RB_MAX_DEPTH {
            return 0;
        }
        if pos <= 0 {
            let store = &mut stores[rec.store_nr as usize];
            set_flag(store, rec, true);
            set_left(store, rec, -l);
            set_right(store, rec, -r);
            return rec.rec as i32;
        }
        if pos as u32 == rec.rec {
            // duplicate record
            return rec.rec as i32;
        }
        let current = to_ref(rec, pos);
        let cmp = keys::order_record(
            self.fast,
            &compare(rec),
            &compare(&current),
            stores,
            self.keys,
        );
        if cmp == Ordering::Less {
            let next = left(keys::store(rec, stores), &current);
            let p = self.put(depth + 1, next, l, pos, stores);
            if p < 0 {
                return -1;
            }
            set_left(keys::mut_store(rec, stores), &current, p);
        } else if cmp == Ordering::Greater {
            let next = right(keys::store(rec, stores), &current);
            let p = self.put(depth + 1, next, pos, r, stores);
            if p < 0 {
                return -1;
            }
            set_right(keys::mut_store(rec, stores), &current, p);
        } else {
            // double keys: nothing is linked or balanced on the way back up, so the
            // tree is as it was and the caller decides what the duplicate is worth.
            *self.duplicate = pos as u32;
            return -1;
        }
        balance(keys::mut_store(rec, stores), &current)
    }
}

fn to_ref(rec: &DbRef, to: i32) -> DbRef {
    assert!(to >= 0, "incorrect to_ref");
    DbRef {
        store_nr: rec.store_nr,
        rec: to as u32,
        pos: rec.pos,
    }
}

fn compare(rec: &DbRef) -> DbRef {
    DbRef {
        store_nr: rec.store_nr,
        rec: rec.rec,
        pos: 8,
    }
}

/// When the position is found, re-balance the tree after inserting
fn balance(store: &mut Store, rec: &DbRef) -> i32 {
    if flag(store, rec) {
        return rec.rec as i32;
    }
    let l = left(store, rec);
    let r = right(store, rec);
    if l > 0 && flag(store, &to_ref(rec, l)) {
        let ll = left(store, &to_ref(rec, l));
        if ll > 0 && flag(store, &to_ref(rec, ll)) {
            return fix_ll(store, rec, l, ll);
        }
        let lr = right(store, &to_ref(rec, l));
        if lr > 0 && flag(store, &to_ref(rec, lr)) {
            return fix_lr(store, rec, l, lr);
        }
    }
    if r > 0 && flag(store, &to_ref(rec, r)) {
        let rl = left(store, &to_ref(rec, r));
        if rl > 0 && flag(store, &to_ref(rec, rl)) {
            return fix_rl(store, rec, r, rl);
        }
        let rr = right(store, &to_ref(rec, r));
        if rr > 0 && flag(store, &to_ref(rec, rr)) {
            return fix_rr(store, rec, r, rr);
        }
    }
    rec.rec as i32
}

/// Black, rec, Node (Red, l, Node (Red, ll, a, b), c), d
/// -> Red, l, Node (Black, ll, a, b), Node (Black, rec, c, d)
fn fix_ll(store: &mut Store, rec: &DbRef, l: i32, ll: i32) -> i32 {
    let c = right(store, &to_ref(rec, l));
    set_right(store, &to_ref(rec, l), rec.rec as i32);
    set_flag(store, &to_ref(rec, ll), false);
    set_left(store, rec, if c < 0 { -l } else { c });
    l
}

/// Black, rec, Node (Red, l, a, Node (Red, lr, b, c)), d
/// -> Red, lr, Node (Black, l, a, b), Node (Black, rec, c, d)
fn fix_lr(store: &mut Store, rec: &DbRef, l: i32, lr: i32) -> i32 {
    let b = left(store, &to_ref(rec, lr));
    let c = right(store, &to_ref(rec, lr));
    set_left(store, &to_ref(rec, lr), l);
    set_right(store, &to_ref(rec, lr), rec.rec as i32);
    set_flag(store, &to_ref(rec, l), false);
    set_right(store, &to_ref(rec, l), if b < 0 { -lr } else { b });
    set_left(store, rec, if c < 0 { -lr } else { c });
    lr
}

/// Black, p, a, Node (Red, r, Node (Red, rl, b, c), d)
/// -> Red, rl, Node (Black, p, a, b), Node (Black, r, c, d)
fn fix_rl(store: &mut Store, rec: &DbRef, r: i32, rl: i32) -> i32 {
    let b = left(store, &to_ref(rec, rl));
    let c = right(store, &to_ref(rec, rl));
    set_left(store, &to_ref(rec, rl), rec.rec as i32);
    set_right(store, &to_ref(rec, rl), r);
    set_right(store, rec, if b < 0 { -rl } else { b });
    set_flag(store, &to_ref(rec, r), false);
    set_left(store, &to_ref(rec, r), if c < 0 { -rl } else { c });
    rl
}

/// Black, p, a, Node (Red, r, b, Node (Red, rr, c, d))
/// -> Red, r, Node (Black, p, a, b), Node (Black, rr, c, d)
fn fix_rr(store: &mut Store, rec: &DbRef, r: i32, rr: i32) -> i32 {
    let b = left(store, &to_ref(rec, r));
    set_left(store, &to_ref(rec, r), rec.rec as i32);
    set_right(store, rec, if b < 0 { -r } else { b });
    set_flag(store, &to_ref(rec, rr), false);
    r
}

/// Walk to the element to remove
fn remove_iter(
    rec: &DbRef,
    depth: u32,
    pos: u32,
    black: &mut bool,
    stores: &mut [Store],
    keys: &[Key],
) -> u32 {
    assert!(depth <= RB_MAX_DEPTH, "Too many iterations");
    assert_ne!(pos, 0, "Item not found");
    let mut pos_ref = to_ref(rec, pos as i32);
    let compare_to;
    if pos == rec.rec {
        pos_ref.rec = remove_elm(&pos_ref, depth, black, stores, keys);
        if pos_ref.rec == 0 {
            return 0;
        }
        let store = keys::mut_store(rec, stores);
        if left(store, &pos_ref) <= 0 && right(store, &pos_ref) <= 0 {
            assert!(*black, "Cannot change node to black twice in remove()");
            assert!(
                flag(store, &pos_ref),
                "Child of single-child node should be red"
            );
            set_flag(store, &pos_ref, false);
            *black = false;
            return pos_ref.rec;
        }
        compare_to = -1;
    } else {
        let cmp = keys::compare(&compare(rec), &compare(&pos_ref), stores, keys);
        if cmp == Ordering::Less {
            compare_to = -1;
            let cl = left(keys::store(rec, stores), &pos_ref);
            assert!(cl >= 0, "should be a normal node");
            let mut l = remove_iter(rec, depth + 1, cl as u32, black, stores, keys) as i32;
            let store = keys::mut_store(rec, stores);
            if l == 0 {
                l = left(store, rec);
            }
            set_left(store, &pos_ref, l);
        } else {
            compare_to = 1;
            let cr = right(keys::store(rec, stores), &pos_ref);
            assert!(cr >= 0, "should be a normal node");
            let mut r = remove_iter(rec, depth + 1, cr as u32, black, stores, keys) as i32;
            let store = keys::mut_store(rec, stores);
            if r == 0 {
                r = right(store, rec);
            }
            set_right(store, &pos_ref, r);
        }
    }
    if *black {
        pos_ref.rec = repair(&pos_ref, black, compare_to, stores, keys);
    }
    pos_ref.rec
}

fn remove_elm(
    rec: &DbRef,
    depth: u32,
    black: &mut bool,
    stores: &mut [Store],
    keys: &[Key],
) -> u32 {
    let store = keys::mut_store(rec, stores);
    let l = left(store, rec);
    let r = right(store, rec);
    let rd = flag(store, rec);
    if l <= 0 {
        // left is empty: return right as replacement
        *black = !rd;
        if r <= 0 {
            return 0;
        }
        assert!(!rd, "Expected node with single-child to be black");
        if left(store, &to_ref(rec, r)) < 0 {
            set_left(store, &to_ref(rec, r), l);
        }
        return r as u32;
    }
    if r <= 0 {
        // left is empty: return left as replacement
        *black = !rd;
        assert!(!rd, "Expected node with single-child to be black");
        if right(store, &to_ref(rec, l)) < 0 {
            set_right(store, &to_ref(rec, l), r);
        }
        return l as u32;
    }
    // both left and right as not empty: remove previous element in tree
    // (=max(l)) and then make that the replacement
    let pos = max(store, &to_ref(rec, l));
    let mut new_left = remove_iter(&pos, depth + 1, l as u32, black, stores, keys) as i32;
    let store = keys::mut_store(rec, stores);
    if new_left == 0 {
        new_left = left(store, &pos);
    }
    set_right(store, &pos, r);
    set_flag(store, &pos, rd);
    set_left(store, &pos, new_left);
    let pv = previous(store, &pos);
    let nx = next(store, &pos);
    if pv > 0 && right(store, &to_ref(rec, pv as i32)) < 0 {
        set_right(store, &to_ref(rec, pv as i32), -(pos.rec as i32));
    }
    if nx > 0 && left(store, &to_ref(rec, nx as i32)) < 0 {
        set_left(store, &to_ref(rec, nx as i32), -(pos.rec as i32));
    }
    pos.rec
}

fn repair(
    rec: &DbRef,
    black: &mut bool,
    compare_to: i32,
    stores: &mut [Store],
    keys: &[Key],
) -> u32 {
    let mut repair1 = 0;
    let mut repair2 = 0;
    let store = keys::mut_store(rec, stores);
    match compare_to.cmp(&0) {
        Ordering::Less => {
            let r = right(store, rec);
            if r < 0 {
                // Expecting a normal node
                return 0;
            }
            child_to_red(store, &to_ref(rec, r), &mut repair1, &mut repair2);
        }
        Ordering::Greater => {
            let l = left(store, rec);
            if l < 0 {
                // Expecting a normal node
                return 0;
            }
            child_to_red(store, &to_ref(rec, l), &mut repair1, &mut repair2);
        }
        Ordering::Equal => {}
    }
    if flag(store, rec) {
        set_flag(store, rec, false);
        *black = false;
    }
    let mut p = *rec;
    if repair1 != 0 {
        p = do_repair(&to_ref(rec, repair1), &p, 0, stores, keys);
    }
    if repair2 != 0 {
        p = do_repair(&to_ref(rec, repair2), &p, 0, stores, keys);
    }
    if *black && flag(keys::store(rec, stores), &p) {
        set_flag(keys::mut_store(rec, stores), &p, false);
        *black = false;
    }
    p.rec
}

fn child_to_red(store: &mut Store, rec: &DbRef, l: &mut i32, r: &mut i32) {
    if flag(store, rec) {
        *l = left(store, rec);
        assert!(*l > 0, "Incorrect child_to_red");
        set_flag(store, &to_ref(rec, *l), true);
        *r = right(store, rec);
        assert!(*r > 0, "Incorrect child_to_red");
        set_flag(store, &to_ref(rec, *r), true);
    } else {
        set_flag(store, rec, true);
        *l = rec.rec as i32;
    }
}

/// Find rec starting at pos.
/// Balance pos for each level down.
fn do_repair(rec: &DbRef, pos: &DbRef, depth: u32, stores: &mut [Store], keys: &[Key]) -> DbRef {
    assert!(depth <= RB_MAX_DEPTH, "Too many iterations");
    let store = keys::mut_store(rec, stores);
    let bal = balance(store, pos);
    assert!(bal > 0, "Incorrect balance");
    let result = to_ref(rec, bal);
    if result.rec != rec.rec {
        let cmp = keys::compare(&compare(rec), &compare(&result), stores, keys);
        if cmp == Ordering::Less {
            let l = left(keys::store(rec, stores), &result);
            assert!(l > 0, "Incorrect repair left");
            let r = do_repair(rec, &to_ref(rec, l), depth + 1, stores, keys);
            set_left(keys::mut_store(rec, stores), &result, r.rec as i32);
        } else {
            let r = right(keys::store(rec, stores), &result);
            assert!(r > 0, "Incorrect repair right");
            let r = do_repair(rec, &to_ref(rec, r), depth + 1, stores, keys);
            set_right(keys::mut_store(rec, stores), &result, r.rec as i32);
        }
    }
    result
}

/// Safe validation version of `rb_first`
fn min(store: &Store, rec: &DbRef) -> DbRef {
    let mut depth = 0;
    if rec.rec == 0 {
        return *rec;
    }
    let mut p = *rec;
    loop {
        let l = left(store, &p);
        if l <= 0 || depth > RB_MAX_DEPTH {
            return p;
        }
        p.rec = l as u32;
        depth += 1;
    }
}

/// Safe validation version of `rb_last`
fn max(store: &Store, rec: &DbRef) -> DbRef {
    let mut depth = 0;
    if rec.rec == 0 {
        return *rec;
    }
    let mut p = *rec;
    loop {
        let l = right(store, &p);
        if l <= 0 || depth > RB_MAX_DEPTH {
            return p;
        }
        p.rec = l as u32;
        depth += 1;
    }
}

/// Walk through the tree validating ordering, returning the number of elements.
/// Validating that each branch has the same number of black elements.
/// Check if no two elements into the tree are both red.
fn verify(rec: &DbRef, blacks: u32, max_blacks: &mut u32, stores: &[Store], keys: &[Key]) -> u32 {
    assert!(blacks <= RB_MAX_DEPTH, "Too deep structure");
    let store = keys::store(rec, stores);
    let l = left(store, rec);
    let r = right(store, rec);
    assert!(
        l.unsigned_abs() != rec.rec && r.unsigned_abs() != rec.rec,
        "Linked to self on {rec:?}"
    );
    let nb = blacks + u32::from(!flag(store, rec));
    //println!("rec:{} l:{l}, r:{r} nb:{nb} max:{max_blacks}", rec.rec);
    1 + v_side(rec, l, true, nb, max_blacks, stores, keys)
        + v_side(rec, r, false, nb, max_blacks, stores, keys)
}

fn v_side(
    rec: &DbRef,
    side: i32,
    left: bool,
    depth: u32,
    max_blacks: &mut u32,
    stores: &[Store],
    keys: &[Key],
) -> u32 {
    assert!(depth < RB_MAX_DEPTH, "Too deep structure");
    if side > 0 {
        let s = to_ref(rec, side);
        // println!("side:{side}={:?} rec:{}={:?} depth:{depth}", keys::get_key(&compare(&s), stores, keys),rec.rec,keys::get_key(&compare(rec), stores, keys));
        let cmp = keys::compare(&compare(&s), &compare(rec), stores, keys);
        assert!(
            rec.rec != s.rec && cmp != Ordering::Equal,
            "Duplicate key {rec:?} and {s:?}"
        );
        assert_eq!(
            u8::from(left) ^ u8::from(cmp == Ordering::Less),
            0,
            "Ordering not correct"
        );
        let store = keys::store(rec, stores);
        assert!(
            !flag(store, rec) || !flag(store, &s),
            "Two adjacent red nodes {rec:?} and {s:?}"
        );
        verify(&s, depth, max_blacks, stores, keys)
    } else if *max_blacks == 0 {
        *max_blacks = depth;
        0
    } else if *max_blacks != depth {
        panic!("Not balanced {depth} != {} on {}", *max_blacks, rec.rec);
    } else {
        0
    }
}

fn verify_walk(start: &DbRef, end: &DbRef, dir: bool, size: u32, stores: &[Store], keys: &[Key]) {
    let mut step = 1;
    let mut elm = *start;
    let store = keys::store(start, stores);
    while elm.rec != end.rec {
        assert!(step <= size, "Too long at {elm:?}");
        let n = if dir {
            next(store, &elm)
        } else {
            previous(store, &elm)
        };
        assert_ne!(n, 0, "Incorrect at {elm:?}");
        let mut cmp = keys::compare(
            &compare(&to_ref(start, n as i32)),
            &compare(&elm),
            stores,
            keys,
        );
        if dir {
            cmp = cmp.reverse();
        }
        assert_eq!(cmp, Ordering::Less, "Not ascending at {elm:?}");
        elm.rec = n;
        step += 1;
    }
    assert_eq!(step, size, "Incorrect length");
}

fn moving(store: &Store, rec: &DbRef, first: u32, second: u32) -> u32 {
    if rec.rec == 0 {
        return 0;
    }
    let mut r = store.get_i32_raw(rec.rec, rec.pos + first);
    let mut depth = 0;
    while r > 0 && store.get_i32_raw(r as u32, rec.pos + second) > 0 {
        r = store.get_i32_raw(r as u32, rec.pos + second);
        if depth > RB_MAX_DEPTH {
            return 0;
        }
        depth += 1;
    }
    r.unsigned_abs()
}

/// Step to the next element in the tree
#[must_use]
pub fn next(store: &Store, rec: &DbRef) -> u32 {
    moving(store, rec, RB_RIGHT, RB_LEFT)
}

/// Step to the previous element in the tree
#[must_use]
pub fn previous(store: &Store, rec: &DbRef) -> u32 {
    moving(store, rec, RB_LEFT, RB_RIGHT)
}

/// Count the records in a red-black tree.
///
/// Walks `first` → `next` (in-order) and increments per record.
/// O(n) in the number of records.  Returns 0 for an empty tree
/// (root pointer == 0).
///
/// Powers `len(ix)` for `index<T[key]>` (P192).
#[must_use]
pub fn count(data: &DbRef, fields: u16, stores: &[Store]) -> u32 {
    let mut cur = first(data, fields, stores).rec;
    if cur == 0 {
        return 0;
    }
    let store = keys::store(data, stores);
    let mut total: u32 = 1;
    loop {
        let r = DbRef {
            store_nr: data.store_nr,
            rec: cur,
            pos: u32::from(fields),
        };
        cur = next(store, &r);
        if cur == 0 {
            break;
        }
        total += 1;
    }
    total
}

/// Validate the tree
/// # Panics
/// When the tree is not correctly defined
pub fn validate(data: &DbRef, fields: u16, stores: &[Store], keys: &[Key]) {
    let store = keys::store(data, stores);
    let rec = DbRef {
        store_nr: data.store_nr,
        rec: store.get_i32_raw(data.rec, data.pos) as u32,
        pos: u32::from(fields),
    };
    if rec.rec == 0 {
        return;
    }
    assert!(rec.rec != 0 && !flag(store, &rec), "Root is not black");
    let mut max_blacks = 0;
    let v = verify(&rec, 0, &mut max_blacks, stores, keys);
    let min = min(store, &rec);
    assert_eq!(
        previous(store, &min),
        0,
        "Incorrect min element {}",
        min.rec
    );
    let max = max(store, &rec);
    assert_eq!(next(store, &max), 0, "Incorrect max element {}", max.rec);
    verify_walk(&min, &max, true, v, stores, keys);
    verify_walk(&max, &min, false, v, stores, keys);
}

pub fn remove(data: &DbRef, rec: &DbRef, fields: u16, stores: &mut [Store], keys: &[Key]) {
    let mut black = false;
    let top = keys::store(data, stores).get_i32_raw(data.rec, data.pos) as u32;
    let r = DbRef {
        store_nr: rec.store_nr,
        rec: rec.rec,
        pos: u32::from(fields),
    };
    let new_top = remove_iter(&r, 0, top, &mut black, stores, keys);
    let s = keys::mut_store(data, stores);
    // Always update the root pointer — when new_top == 0 the tree is now empty
    // and the old root must be cleared; skipping this update caused T0-3 (off-by-one
    // "phantom record" after removing all elements).
    s.set_i32_raw(data.rec, data.pos, new_top as i32);
    if new_top > 0 {
        s.set_byte(new_top, u32::from(fields) + RB_FLAG, 0, 0);
    }
}
