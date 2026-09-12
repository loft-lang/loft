// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN155 phase 4 — the HEAP half of the free licence: `formal/heap.md`'s fault rules.
//!
//! The rest of @PLN155 is about the COMPILE-TIME licence — which binding may be freed, derived
//! from `deps` and the oracle.  These four rules are the other half: given that a free is
//! emitted, what does the RUNTIME refuse?  `(H-Free)`'s side conditions, `(H-FreeNull)`,
//! `(H-FreeTwice)`, `(H-FreeStack)`, `@FR-H-FreeAny` and `@FR-H-FreeAll`.
//!
//! **They are tested here rather than in `tests/scripts/` because no loft program can express
//! them.** A double free, a free of the evaluation stack, a free out of allocation order — the
//! compiler does not emit any of those, by construction, which is exactly why the runtime's
//! refusals are untested by every `.loft` cell in the corpus. The subject is `Stores`, so the
//! test drives `Stores` directly.
//!
//! **The phase's question, and its answer.** *Does the compile-time refusal cover the
//! record/element frees, or do they need their own gate?*  Neither: they already HAVE one.
//! `Stores::free_named` is a genuine chokepoint — every free of a live store reaches it, and
//! the three paths that set `Store::free` without it are a sentinel constructor, a unit test
//! and the debugger's own teardown. It enforces three of the four rules on its own, and the
//! compile-time licence never needs to reach them.
//!
//! The fourth WAS the finding: `(H-FreeLIFO)` was enforced nowhere and had been deliberately
//! retired — `Stores::free_bits` (S29) says so in its own doc, *"eliminates the LIFO-order
//! requirement on `free()` that the old cascade-based scan imposed"*, and `rule_tags.py` agreed
//! from the other side with zero citations, alone among the five.  **Closed 2026-09-12 by owner
//! ruling: the rules were rewritten to how the mechanism functions.**  `@FR-H-FreeAny` now
//! states the positive fact — the store released is the one the reference NAMES, whatever its
//! allocation order — and `lifo_order_is_not_a_fault` is its guard rather than a reading of a
//! rule nobody obeyed.  `(H-Free)`'s own premise carried the same retired requirement and was
//! corrected with it; so was its `free_protected` side condition, which `free_named` never
//! checks (that gate lives at the deep-copy call sites).
//!
//! ⚠ **Each cell records whether it was FALSIFIED, and two were not.**  Every guard here was
//! removed by hand and the suite re-run; `freeing_the_stack_store_is_refused` went red and the
//! other two did not.  A cell that cannot fail is not a guard, and saying which is which is
//! worth more than five cells that all look alike — the same discipline the rest of @PLN155
//! applies to its instruments, turned on this file.

use loft::database::Stores;
use loft::keys::DbRef;

/// A store with something in it, so a free has work to do.
fn alloc(stores: &mut Stores, name: &str) -> DbRef {
    stores.database_named(64, name)
}

fn is_free(stores: &Stores, db: &DbRef) -> bool {
    stores.allocations[db.store_nr as usize].is_free()
}

/// `(H-FreeNull)` — `free(nullref)` is a no-op.
///
/// The null sentinel is `u16::MAX`, so a free of it must not reach the allocation table.  The
/// control is the second half: a real store at the same call still frees, so the early return
/// is not swallowing every free.
///
/// ⚠ **NOT FALSIFIED — characterisation, not a guard.**  Removing the `store_nr == u16::MAX`
/// early return leaves this cell GREEN, because the out-of-range check below it catches the
/// same call in a release build.  The cell cannot tell which of the two guards did the work,
/// so it pins the BEHAVIOUR and not the mechanism.
#[test]
fn freeing_null_is_a_no_op() {
    let mut stores = Stores::new();
    let before = stores.allocations.len();
    stores.free_named(&DbRef::NULL, "null_probe");
    assert_eq!(
        stores.allocations.len(),
        before,
        "freeing the null sentinel must not touch the allocation table"
    );

    // The control: the same call on a REAL store does free it.  Without this the test above
    // passes on a `free_named` that returns unconditionally.
    let db = alloc(&mut stores, "real");
    assert!(!is_free(&stores, &db), "a fresh store is live");
    stores.free_named(&db, "real");
    assert!(is_free(&stores, &db), "a real store still frees");
}

/// `(H-FreeTwice)` — freeing an already-freed store is a FAULT, and the runtime refuses it
/// rather than corrupting the table.
///
/// The refusal is what matters: the second free must not run the release again.
///
/// ⚠ **NOT FALSIFIED — characterisation, and the reason is worth having.**  Removing the
/// `if store.free { return }` early return leaves this cell GREEN: re-running the release sets
/// an already-set flag and an already-set free BIT, both idempotent, and neither the flag nor
/// the table length moves.  **The real consequence of a second release is the CASCADE** —
/// `free_named` reads a closure record's adopted `DbRef` fields out of the store and frees
/// those too, so a second pass reads bytes that now belong to whatever reused the slot.
/// Reaching it needs a `__closure_`-typed record built by hand, a heavier fixture than this
/// file carries.  Recorded rather than faked, so whoever strengthens it knows which observable
/// to reach for.
#[test]
fn freeing_twice_is_refused() {
    let mut stores = Stores::new();
    let db = alloc(&mut stores, "twice");
    stores.free_named(&db, "twice");
    assert!(is_free(&stores, &db), "the first free releases");

    let len_after_first = stores.allocations.len();
    stores.free_named(&db, "twice");
    assert!(
        is_free(&stores, &db),
        "the store stays freed — the second free is a no-op, not a re-release"
    );
    assert_eq!(
        stores.allocations.len(),
        len_after_first,
        "a refused double free must not disturb the allocation table"
    );
}

/// `(H-FreeTwice)`, the half a `free` flag cannot show: the slot is REUSED after a free, so a
/// second release would be releasing whatever now lives there.
///
/// This is why the refusal is a hard invariant rather than a tidiness rule — and it is the
/// reason `(H-FreeLIFO)` could be retired: `free_bits` reuses a freed slot below `max`, which
/// is precisely what a LIFO discipline would forbid.
///
/// A CHARACTERISATION cell by intent: it pins slot reuse, so removing reuse fails here.
#[test]
fn a_freed_slot_is_reused_which_is_why_a_double_free_bites() {
    let mut stores = Stores::new();
    let first = alloc(&mut stores, "a");
    let _second = alloc(&mut stores, "b");
    stores.free_named(&first, "a");

    let third = alloc(&mut stores, "c");
    assert_eq!(
        third.store_nr, first.store_nr,
        "the allocator reuses the freed slot, so a second free of `a` would release `c`"
    );
    assert!(!is_free(&stores, &third), "the reused slot is live again");
}

/// `(H-FreeLIFO)` — **measured, and it is not a fault.**
///
/// The rule says freeing a store that is not the top of the allocation order is a FAULT.  The
/// runtime does not check it, `rule_tags.py` reports zero citation sites for it (alone among
/// heap.md's five free rules), and `Stores::free_bits` records that the requirement was
/// deliberately removed.  This asserts the behaviour that actually holds, so the rule's state
/// is a reading rather than an assumption — and so that a future run REINTRODUCING a LIFO
/// fault fails here rather than in a consumer.
#[test]
fn lifo_order_is_not_a_fault() {
    let mut stores = Stores::new();
    let first = alloc(&mut stores, "first");
    let second = alloc(&mut stores, "second");

    // Free the OLDER store while the newer one is live — the shape `(H-FreeLIFO)` calls a
    // fault.  It succeeds, and the newer store is untouched.
    stores.free_named(&first, "first");
    assert!(
        is_free(&stores, &first),
        "the older store frees out of order"
    );
    assert!(
        !is_free(&stores, &second),
        "and the newer store, allocated after it, is unaffected"
    );

    // Then the newer one, so the run leaves nothing behind.
    stores.free_named(&second, "second");
    assert!(is_free(&stores, &second));
}

/// `(H-FreeStack)` — freeing store 0, the evaluation stack, is a FAULT and is refused.
///
/// ✅ **FALSIFIED** — removing the `al == 0 && self.stack_store_at_zero` guard turns this cell
/// red, and it is the only cell here that does.
///
/// Guarded by `stack_store_at_zero`, because store 0 is only the stack when the interpreter
/// put it there; a bare `Stores` uses slot 0 as an ordinary store and freeing it is fine.  The
/// flag is set here to reach the refusal, and the control is the same free with the flag
/// CLEARED, which must succeed — without it this test passes on a `free_named` that refuses
/// slot 0 unconditionally.
#[test]
fn freeing_the_stack_store_is_refused() {
    let mut stores = Stores::new();
    let zero = alloc(&mut stores, "slot0");
    assert_eq!(zero.store_nr, 0, "the first allocation takes slot 0");

    stores.stack_store_at_zero = true;
    stores.free_named(&zero, "slot0");
    assert!(
        !is_free(&stores, &zero),
        "@FR-H-FreeStack — the evaluation stack is never an owned heap store"
    );

    // The control: with the flag cleared, slot 0 is an ordinary store and does free.
    stores.stack_store_at_zero = false;
    stores.free_named(&zero, "slot0");
    assert!(
        is_free(&stores, &zero),
        "the refusal is about the STACK, not about slot 0"
    );
}
