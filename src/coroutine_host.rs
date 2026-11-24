// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A generator a hosted C unit runs, held by the interpreter as a coroutine handle: the
//! [`LoftCoroutine`] trait such a generator implements, the per-run table of live handles
//! ([`NativeRun`]) and the entry points the interpreter's coroutine operators call.

use crate::database::Stores;
use crate::keys::DbRef;

// ── N8b.1: Native coroutine runtime ─────────────────────────────────────────

pub use crate::database::NATIVE_COROUTINE_STORE;

/// Return value from `LoftCoroutine::next_i64` when the generator is exhausted.
///
/// Post-2c the `integer` type is i64 — both integer- and long-yielding
/// generators terminate on `op_conv_bool_from_int(val)` returning
/// `false`, which happens exactly when `val == i64::MIN`.  Pre-2c this
/// constant was `i32::MIN as i64` because the integer null sentinel was
/// `i32::MIN`; that's now the wrong value — the sentinel moved to
/// `i64::MIN` when Phase 2c widened integer storage, and
/// `coroutine_next_i64` returning `i32::MIN as i64` was being
/// interpreted as a real value by `op_conv_bool_from_int`, leaving the
/// for-loop iterating forever.
pub const COROUTINE_EXHAUSTED: i64 = i64::MIN;

/// Trait implemented by every native generator (state-machine struct).
/// The generated `fn {name}(stores, args) -> Box<dyn LoftCoroutine>` factory
/// constructs an instance; `alloc_coroutine` stores it and returns a `DbRef`.
///
/// One method per yielded primitive shape:
/// - `next_i64` for integer / float / boolean / single / character yields
///   (everything that fits in 8 bytes).
/// - `next_text` for `text` yields.  Returns `STRING_NULL` ("\0") on
///   exhaustion, matching the for-loop break-test convention used by
///   `dispatch.rs::OpCoroutineNext`.
///
/// A given generator only ever overrides the method matching its yield
/// type; the other keeps its default (returns the type's exhaustion
/// sentinel) so a wrong-channel call drains the loop immediately rather
/// than panicking.
pub trait LoftCoroutine {
    /// Advance an integer-yielding generator one step.  Returns the yielded
    /// value as `i64`, or `COROUTINE_EXHAUSTED` when no further values are
    /// available.
    fn next_i64(&mut self, _stores: &mut Stores) -> i64 {
        COROUTINE_EXHAUSTED
    }

    /// Advance a text-yielding generator one step.  Returns the yielded
    /// `String`, or `STRING_NULL` (`"\0"`) when the generator is exhausted.
    fn next_text(&mut self, _stores: &mut Stores) -> String {
        crate::store::STRING_NULL.to_string()
    }

    /// Advance a Reference-yielding generator one step.  Returns the yielded
    /// `DbRef`, or the null-DbRef sentinel (`store_nr == u16::MAX`, `rec == 0`)
    /// when the generator is exhausted.  Used by `iterator<Struct>` /
    /// `iterator<vector<T>>` / any DbRef-shaped yield (@P326).
    fn next_dbref(&mut self, _stores: &mut Stores) -> DbRef {
        DbRef::NULL
    }

    /// Unified store-backed yield channel (@P327 native; designed to absorb
    /// future yield shapes without adding one trait method per shape — see
    /// `plans/16-coroutine-validation/01-unified-channel.md`).  Advance the
    /// generator one step and write the yielded value's bytes into `dest`
    /// (a caller-provided `i64`-aligned buffer sized for the yielded type).
    /// Returns true on yield, false on exhaustion.
    ///
    /// Each coroutine impl encodes its OWN yield-type layout in this single
    /// method — no new trait method per shape.  This is the migration
    /// target for `next_i64` / `next_dbref` / `next_text`; each can move
    /// to `next_into` one shape at a time.
    fn next_into(&mut self, _stores: &mut Stores, _dest: &mut [i64]) -> bool {
        false
    }

    /// Release the heap locals this generator still owns.
    ///
    /// A generator frees its own locals from the tail of its body, and a consumer that
    /// stops early never reaches that tail — so an abandoned generator's vector stayed
    /// allocated for the rest of the program (loft#835).  Called when the handle is freed
    /// at the end of the scope holding it.  Generated per generator; a generator with no
    /// owned heap local keeps this no-op default.
    ///
    /// Freeing twice is not possible: every scope-exit free the generator DOES run nulls
    /// its own field, and the generated body only frees fields that are still set.
    fn drop_stores(&mut self, _stores: &mut Stores) {}

    /// `@FR-G-Arg-Rebind` — the heap arguments this generator was handed, as its factory
    /// stored them, pushed onto `out`: [`alloc_coroutine_held`] holds their stores for the
    /// frame's life.  Generated per lazy generator; an eager one ran its body in its factory
    /// and keeps this empty default.
    fn held_args(&self, _out: &mut Vec<DbRef>) {}

    /// @FR-G-Next / @FR-G-Done — has this generator reached its end?  Asked only when an advance
    /// answered its channel's end sentinel, because that sentinel is also a value a
    /// generator may yield — `i64::MIN` is the integer null, `"\0"` the text null,
    /// `DbRef::NULL` a null reference — and `(G-Next)` produces it like any other.
    /// Every emitted generator overrides this with its own flag; the default keeps the
    /// sentinel meaning "done" for a generator that has no flag to ask.
    fn done(&self) -> bool {
        true
    }
}

/// A generated program's run-time tables — the `--native` twins of the interpreter's
/// `State::coroutines` and `State::fnref_bufs` / `State::fnref_borrowed_return`.  Owned by
/// the program's `Stores` (`Stores::native`, `@FR-C-Own`); a `par` worker's stores start
/// with empty ones, as a worker thread did.
#[derive(Default)]
pub struct NativeRun {
    /// The generator frames, each tagged with the generation that identifies THIS occupant
    /// of the slot.  Slot 0 is always `None` (the null sentinel), pushed by the first
    /// allocation.
    ///
    /// A slot is freed on exhaustion and handed to the next generator, while the variable
    /// holding the handle lives to the end of its scope, where its free fires — so the
    /// index alone does not identify a coroutine for long enough.  Mirrors
    /// `CoroutineFrame::generation` on the interpreter side.
    coroutines: Vec<Option<(u32, Box<dyn LoftCoroutine>)>>,
    /// The stamp the last coroutine was given; the next one takes the one after (never 0).
    generation: u32,
    /// `(G-Hold)` — each slot's holder count, the interpreter's `CoroutineFrame::holds`
    /// (loft#1708).  Read only for a live handle, so a stale count of a freed slot is never
    /// consulted: `alloc_coroutine` resets it to one.
    holds: Vec<u32>,
    /// `@FR-G-Arg-Rebind` — per slot, the argument stores the frame holds
    /// ([`alloc_coroutine_held`]); emptied when the frame ends.
    held: Vec<Vec<DbRef>>,
    /// The buffer [`alloc_coroutine_held`] fills before the frame has a slot.
    held_scratch: Vec<DbRef>,
    /// The return buffers a fn-ref dispatch arm allocated and the callee DELIVERED through,
    /// innermost frame last ([`cr_fnref_buf`]).  A frame's own buffers are the entries above
    /// the mark [`FnRefBufGuard`] took at entry — a watermark rather than a frame number, so
    /// nothing has to agree about depth.
    pub(crate) fnref_bufs: Vec<(DbRef, u64)>,
    /// @PLN150 — the store the most recent fn-ref call handed back that it did NOT mint
    /// ([`cr_fnref_minted`]).
    pub(crate) fnref_borrowed: Option<DbRef>,
}

impl Clone for NativeRun {
    /// A copied `Stores` is a new context: it holds no generator and no buffer yet.
    fn clone(&self) -> Self {
        NativeRun::default()
    }
}

/// The run-time tables of the program running over `cell`.
#[inline]
#[expect(
    clippy::mut_from_ref,
    reason = "the generated program's `UnsafeCell<Stores>` is its one context"
)]
pub(crate) fn run_of(cell: &std::cell::UnsafeCell<Stores>) -> &mut NativeRun {
    // SAFETY: generated code holds `cell` for the whole run and calls this between its own
    // statements; the borrow ends with the statement that took it.
    unsafe { &mut (*cell.get()).native }
}

/// Give back the argument holds of the frame in slot `idx` (`@FR-G-Arg-Rebind`): the frame
/// ended, by exhaustion or by its last holder's release.
fn release_native_held(idx: usize, stores: &mut Stores) {
    // The slot keeps its buffer's capacity for the next frame.
    let Some(held) = stores.native.held.get_mut(idx) else {
        return;
    };
    let mut held = std::mem::take(held);
    for db in held.drain(..) {
        stores.release_frame_hold(&db);
    }
    if let Some(slot) = stores.native.held.get_mut(idx) {
        *slot = held;
    }
}

/// [`alloc_coroutine`] for a generator call: the frame also takes a hold on the store of
/// every heap argument it was handed (`@FR-G-Arg-Rebind`), the native twin of the bytecode
/// VM's `State::hold_arg_stores`.
#[must_use]
pub fn alloc_coroutine_held(
    cell: &std::cell::UnsafeCell<Stores>,
    coro: Box<dyn LoftCoroutine>,
) -> DbRef {
    // The buffers circulate between the scratch and the slots, so a generator call allocates
    // nothing here once the table has warmed up.
    let stores: &mut Stores = unsafe { &mut *cell.get() };
    let mut held = std::mem::take(&mut stores.native.held_scratch);
    held.clear();
    coro.held_args(&mut held);
    if !held.is_empty() {
        held.retain(|db| stores.hold_for_frame(db));
    }
    let gen_ref = alloc_coroutine(stores, coro);
    let idx = gen_ref.rec as usize;
    let h = &mut stores.native.held;
    if h.len() <= idx {
        h.resize(idx + 1, Vec::new());
    }
    std::mem::swap(&mut h[idx], &mut held);
    held.clear();
    stores.native.held_scratch = held;
    gen_ref
}

fn set_native_holds(run: &mut NativeRun, idx: usize, holds: u32) {
    if run.holds.len() <= idx {
        run.holds.resize(idx + 1, 0);
    }
    run.holds[idx] = holds;
}

/// `OpCoroutineRetain` — one more holder for the generator `gen_ref` names (`(G-Hold)`,
/// loft#1708).  A handle naming no live generator takes nothing.  Answers the handle.
#[must_use]
pub fn coroutine_retain(gen_ref: DbRef, stores: &mut Stores) -> DbRef {
    let run = &mut stores.native;
    if native_coroutine_live(&run.coroutines, gen_ref)
        && let Some(n) = run.holds.get_mut(gen_ref.rec as usize)
    {
        *n += 1;
    }
    gen_ref
}

/// Does `gen_ref` still name the frame it was made for — right slot AND right occupant?
fn native_coroutine_live(
    coroutines: &[Option<(u32, Box<dyn LoftCoroutine>)>],
    gen_ref: DbRef,
) -> bool {
    gen_ref.store_nr == NATIVE_COROUTINE_STORE
        && gen_ref.rec != 0
        && coroutines
            .get(gen_ref.rec as usize)
            .and_then(Option::as_ref)
            .is_some_and(|(generation, _)| *generation == gen_ref.pos)
}

/// Store a native coroutine generator and return a `DbRef` that identifies it.
///
/// `pos` carries the generation stamp: a coroutine reference addresses no store, so the
/// field is free, and the handle then names the generator rather than the index it occupies.
#[must_use]
pub fn alloc_coroutine(stores: &mut Stores, coro: Box<dyn LoftCoroutine>) -> DbRef {
    let run = &mut stores.native;
    run.generation = run.generation.wrapping_add(1).max(1);
    let generation = run.generation;
    if run.coroutines.is_empty() {
        run.coroutines.push(None);
    }
    let idx = if let Some(free) = run.coroutines.iter().skip(1).position(Option::is_none) {
        free + 1
    } else {
        run.coroutines.push(None);
        run.coroutines.len() - 1
    };
    run.coroutines[idx] = Some((generation, coro));
    set_native_holds(run, idx, 1);
    DbRef {
        store_nr: NATIVE_COROUTINE_STORE,
        rec: idx as u32,
        pos: generation,
    }
}

/// Free one heap local an abandoned generator still owns (loft#835).
///
/// A nested generator handle routes back to the coroutine table so its own locals are
/// released too; anything else is an ordinary store.  Called only from a generated
/// `drop_stores`, which has already checked the field is set.
pub fn coroutine_drop_local(stores: &mut Stores, db: DbRef, name: &str) {
    if db.store_nr == NATIVE_COROUTINE_STORE {
        free_native_coroutine(db, stores);
    } else if db.store_nr != u16::MAX && (db.store_nr as usize) < stores.allocations.len() {
        stores.free_named(&db, name);
    }
}

/// [`coroutine_drop_local`] for a generator that owns more than one heap local: two of them
/// can hold ONE store — a lazily-lowered loop's record buffer (`__ref_*`) and the local that
/// adopted the record the call built in it — and releasing that store twice frees a slot
/// another record may already hold.  `seen` collects the stores this `drop_stores` has
/// released; a nested generator handle is keyed by its table slot as well, because every
/// handle shares the one coroutine store number.
pub fn coroutine_drop_local_once(
    stores: &mut Stores,
    db: DbRef,
    name: &str,
    seen: &mut Vec<(u16, u32)>,
) {
    let key = if db.store_nr == NATIVE_COROUTINE_STORE {
        (db.store_nr, db.rec)
    } else {
        (db.store_nr, 0)
    };
    if seen.contains(&key) {
        return;
    }
    seen.push(key);
    coroutine_drop_local(stores, db, name);
}

/// Copy the `tp` record `src` into the generator's snapshot store `snap` and answer the
/// copy's handle.  An eager factory runs its whole loop before the consumer reads a
/// value, so a pushed handle to a per-iteration local would alias the local's FINAL
/// state; the copy pins the value as it was at the yield.  One store per generator —
/// allocated by the first snapshot, every later one a record claimed in it — so a long
/// loop costs records, not store slots.  A null yield stays null.
pub fn coroutine_snapshot(
    cell: &std::cell::UnsafeCell<Stores>,
    snap: &mut DbRef,
    src: DbRef,
    tp: i32,
) -> DbRef {
    if src.store_nr == u16::MAX {
        return src;
    }
    let stores: &mut Stores = unsafe { &mut *cell.get() };
    let tp = tp as u16;
    if snap.store_nr == u16::MAX {
        *snap = stores.null();
        stores.clear(snap);
        stores.allocations[snap.store_nr as usize].set_known_type(tp);
    }
    // Same record shape `OpDatabase` claims: a word of header, then `size` bytes.
    let size = stores.enum_parent_size(tp);
    let r = stores.claim(snap, 1 + u32::from(size).div_ceil(8));
    stores.store_mut(&r).set_u32_raw(r.rec, 4, u32::from(tp));
    stores.set_default_value(tp, &r);
    crate::codegen_runtime::OpCopyRecord(cell, src, r, i32::from(tp));
    r
}

/// [`coroutine_snapshot`] of a value the generator HANDS over (`(G-Own)`): a fresh record in
/// a store of its own that nothing else holds, so the snapshot is a move and that store is
/// released here.  Its resource travels with the snapshot, so no drop hook runs.
pub fn coroutine_snapshot_moved(
    cell: &std::cell::UnsafeCell<Stores>,
    snap: &mut DbRef,
    src: DbRef,
    tp: i32,
) -> DbRef {
    let r = coroutine_snapshot(cell, snap, src, tp);
    if src.store_nr != u16::MAX {
        let stores: &mut Stores = unsafe { &mut *cell.get() };
        if (src.store_nr as usize) < stores.allocations.len() {
            stores.free_named(&src, "__yield");
        }
    }
    r
}

/// Hand the consumer an eagerly-collected value of its own — `formal/coroutines.md`
/// `(G-Own)`: what an advance produces is the consumer's, which releases it, while an eager
/// factory keeps every value in ONE snapshot store the generator releases.  So each value
/// leaves in a fresh store: a copy of its snapshot record, typed `tp` as
/// [`coroutine_snapshot`] typed it.  A null stays null.
pub fn coroutine_hand_out(cell: &std::cell::UnsafeCell<Stores>, src: DbRef, tp: i32) -> DbRef {
    if src.store_nr == u16::MAX {
        return src;
    }
    let own = crate::codegen_runtime::OpDatabase(cell, DbRef::NULL, tp);
    crate::codegen_runtime::OpCopyRecord(cell, src, own, tp);
    own
}

/// Release a generator's snapshot store (see [`coroutine_snapshot`]) — at exhaustion,
/// and from `drop_stores` when the generator is abandoned.  Idempotent.
pub fn coroutine_release_snapshots(stores: &mut Stores, snap: &mut DbRef) {
    if snap.store_nr != u16::MAX && (snap.store_nr as usize) < stores.allocations.len() {
        stores.free_named(snap, "__snap");
    }
    *snap = DbRef::NULL;
}

/// Free a native coroutine handle: release the heap locals the generator still owns, then
/// its slot.  A handle whose generator already exhausted names a frame that is gone, and
/// the generation check keeps this off whatever inherited the slot (loft#835).
pub fn free_native_coroutine(gen_ref: DbRef, stores: &mut Stores) {
    if let Some(h) = forwarded() {
        return forward_cold(|| (h.free)(gen_ref, stores));
    }
    // Take the generator OUT of the table before running its cleanup: `drop_stores` frees
    // stores, and a nested generator handle among them re-enters this table.
    let run = &mut stores.native;
    let taken = if native_coroutine_live(&run.coroutines, gen_ref) {
        // `(G-Hold)`: one of several holders gives its hold back and nothing more.
        match run.holds.get_mut(gen_ref.rec as usize) {
            Some(n) if *n > 1 => {
                *n -= 1;
                None
            }
            _ => run.coroutines[gen_ref.rec as usize].take(),
        }
    } else {
        None
    };
    if let Some((_, mut coro)) = taken {
        coro.drop_stores(stores);
        release_native_held(gen_ref.rec as usize, stores);
    }
}

/// The occupant a generator's slot holds while that generator runs: the advance takes the
/// generator out of the table, so this answers nothing — every method keeps its default.
struct Advancing;
impl LoftCoroutine for Advancing {}

/// Run one advance of the generator `gen_ref` names, or answer `dead` where no live generator
/// has that handle.  `step` answers the value and whether the generator is now exhausted,
/// which frees its slot (P2-R7 parity).
///
/// The generator is taken OUT of the table for the advance, with [`Advancing`] holding its
/// slot and generation: its body may advance another generator — one it was handed, or one
/// it made — or allocate one, and each of those borrows the table again.  Advanced under the
/// table's borrow, a generator that consumed another panicked with "already borrowed".
fn advance_native<R>(
    gen_ref: DbRef,
    stores: &mut Stores,
    dead: R,
    step: impl FnOnce(&mut dyn LoftCoroutine, &mut Stores) -> (R, bool),
) -> R {
    let taken = {
        let coroutines = &mut stores.native.coroutines;
        if native_coroutine_live(coroutines, gen_ref) {
            coroutines[gen_ref.rec as usize]
                .as_mut()
                .map(|(_, coro)| std::mem::replace(coro, Box::new(Advancing)))
        } else {
            None
        }
    };
    let Some(mut coro) = taken else {
        return dead;
    };
    let (val, exhausted) = step(coro.as_mut(), stores);
    let ended = {
        let coroutines = &mut stores.native.coroutines;
        let idx = gen_ref.rec as usize;
        // Put back only into the slot it left: a free during the advance empties the slot,
        // and a later allocation may already hold it under another generation.
        if !native_coroutine_live(coroutines, gen_ref) {
            false
        } else if exhausted {
            coroutines[idx] = None;
            true
        } else {
            if let Some((_, slot)) = coroutines[idx].as_mut() {
                *slot = coro;
            }
            false
        }
    };
    if ended {
        release_native_held(gen_ref.rec as usize, stores);
    }
    val
}

/// Advance a native coroutine and return the yielded value as `i64`.
/// Frees the coroutine slot automatically when it is exhausted.
pub fn coroutine_next_i64(gen_ref: DbRef, stores: &mut Stores) -> i64 {
    advance_native(gen_ref, stores, COROUTINE_EXHAUSTED, |coro, stores| {
        let val = coro.next_i64(stores);
        (val, val == COROUTINE_EXHAUSTED && coro.done())
    })
}

/// Advance a generator through the unified store-backed yield channel
/// (@P327 native).  Writes the yielded value's bytes into `dest` and
/// returns true; returns false and frees the slot on exhaustion.
pub fn coroutine_next_into(gen_ref: DbRef, stores: &mut Stores, dest: &mut [i64]) -> bool {
    advance_native(gen_ref, stores, false, |coro, stores| {
        let ok = coro.next_into(stores, dest);
        (ok, !ok)
    })
}

/// Advance a Reference-yielding native coroutine.  Returns the yielded
/// `DbRef`, or the null-DbRef sentinel (store_nr=u16::MAX, rec=0) when
/// exhausted.  Frees the coroutine slot automatically on exhaustion,
/// mirroring `coroutine_next_i64` (@P326).
pub fn coroutine_next_dbref(gen_ref: DbRef, stores: &mut Stores) -> DbRef {
    // Null sentinel matches `vector::null_ref` and `OpNullRefSentinel`: store_nr == u16::MAX
    // is the universal "no record" indicator.  The for-loop's `OpCoroutineExhausted(gen)`
    // check also flips to true here because exhaustion frees the slot.
    advance_native(gen_ref, stores, DbRef::NULL, |coro, stores| {
        let val = coro.next_dbref(stores);
        (val, val.store_nr == u16::MAX && coro.done())
    })
}

/// Advance a text-yielding native coroutine.  Returns the yielded `String`,
/// or `STRING_NULL` (`"\0"`) when exhausted.  Frees the coroutine slot
/// automatically on exhaustion, mirroring `coroutine_next_i64`.
pub fn coroutine_next_text(gen_ref: DbRef, stores: &mut Stores) -> String {
    advance_native(
        gen_ref,
        stores,
        crate::store::STRING_NULL.to_string(),
        |coro, stores| {
            let val = coro.next_text(stores);
            let done = val == crate::store::STRING_NULL && coro.done();
            (val, done)
        },
    )
}

/// Test whether a native coroutine has been exhausted (its slot freed).
#[must_use]
pub fn coroutine_is_exhausted(gen_ref: DbRef, stores: &Stores) -> bool {
    !native_coroutine_live(&stores.native.coroutines, gen_ref)
}

/// @PLN199 D2 — the native coroutine table's entry points as a value: the host's own
/// (`HERE`), which a library cdylib's copy of this module forwards to once its host
/// installed them (`forward_coroutines`).  Every `static` here
/// exists once per copy of libloft, so without the forward a library's generator handle and
/// the host's named two different tables.  Rust fn pointers, as `Box<dyn LoftCoroutine>`
/// crosses: one rlib builds both sides.
#[derive(Clone, Copy)]
pub struct CoroutineHost {
    pub alloc_held: fn(&std::cell::UnsafeCell<Stores>, Box<dyn LoftCoroutine>) -> DbRef,
    pub alloc: fn(&mut Stores, Box<dyn LoftCoroutine>) -> DbRef,
    pub retain: fn(DbRef, &mut Stores) -> DbRef,
    pub free: fn(DbRef, &mut Stores),
    pub next_i64: fn(DbRef, &mut Stores) -> i64,
    pub next_into: fn(DbRef, &mut Stores, &mut [i64]) -> bool,
    pub next_dbref: fn(DbRef, &mut Stores) -> DbRef,
    pub next_text: fn(DbRef, &mut Stores) -> String,
    pub exhausted: fn(DbRef, &Stores) -> bool,
}

impl CoroutineHost {
    /// This copy's own table.
    pub const HERE: CoroutineHost = CoroutineHost {
        alloc_held: alloc_coroutine_held,
        alloc: alloc_coroutine,
        retain: coroutine_retain,
        free: free_native_coroutine,
        next_i64: coroutine_next_i64,
        next_into: coroutine_next_into,
        next_dbref: coroutine_next_dbref,
        next_text: coroutine_next_text,
        exhausted: coroutine_is_exhausted,
    };
}

/// The table a library cdylib's copy forwards to: its host's (`forward_coroutines`), or null.
/// Read only by [`free_native_coroutine`], which the runtime itself reaches from inside a
/// cdylib (`OpFreeRef`, a generator's `drop_stores`).  Every other entry point is reached
/// from generated code, which in a library cdylib calls the host's table directly
/// (`native_lib::HOST_INIT_SRC`'s `__coro`): a test here, even a relaxed load out of line,
/// cost a whole-program generator loop 8 % (measured; the emitted program was identical).
/// A relaxed load: the slot is written once, by the library's `loft_shared_init`, before any
/// of its code runs — an acquire load per advance (`OnceLock::get`) cost a whole-program
/// generator loop 6 %.
static FORWARD: std::sync::atomic::AtomicPtr<CoroutineHost> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Forward this copy's coroutine table to `host`'s.  Called from a library cdylib's
/// `loft_shared_init` only; never in the host, whose own table this is.
pub fn forward_coroutines(host: CoroutineHost) {
    if FORWARD.load(std::sync::atomic::Ordering::Acquire).is_null() {
        FORWARD.store(
            Box::into_raw(Box::new(host)),
            std::sync::atomic::Ordering::Release,
        );
    }
}

/// The forwarded call, kept out of line: the entry points run in a generator's every
/// advance, and the call through the host's table — taken only inside a library cdylib —
/// must not shape their hot body.
#[cold]
#[inline(never)]
fn forward_cold<R>(call: impl FnOnce() -> R) -> R {
    call()
}

/// The host's table this copy forwards to, if any.
#[inline]
fn forwarded() -> Option<&'static CoroutineHost> {
    let p = FORWARD.load(std::sync::atomic::Ordering::Relaxed);
    // SAFETY: set once to a leaked `Box`, never freed or changed after.
    unsafe { p.as_ref() }
}
