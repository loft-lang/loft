// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F128 — compiled libraries (a used library runs as compiled C under the interpreter)

//! `@PLN206 P4d` — `LOFT_HOSTED_VERIFY=1`: each call into a compiled library artifact (a hosted C
//! unit or a rustc one) runs twice — compiled, on a copy of the stores taken before the call, and
//! interpreted, on the stores themselves — and the two must leave the same world: every store
//! that existed before the call alike in its claimed bytes (`src/store/dump.rs`), and the same
//! answer (a scalar exactly, a record as `Stores::show` renders it).  The first call that differs
//! stops the run and names the function and the first difference.  The program then continues
//! in the compiled world, as it would without the switch.
//!
//! What is not compared is what a caller cannot see: a store the call made and freed, and the
//! hidden return buffer, which the compiled bridge fills where the interpreter's call-back makes
//! its own.  Run under `LOFT_POISON_CLAIM` (and a unit built under it), a byte nothing wrote
//! reads the same on both sides.

use std::fmt::Write as _;

use super::State;
use crate::database::Stores;
use crate::host_abi::LibArg;
use crate::keys::DbRef;

/// A compiled library's bridge, as the dispatcher calls it.
pub(crate) type Bridge = unsafe extern "C" fn(*mut Stores, *const LibArg, usize, *mut LibArg);

/// Is `LOFT_HOSTED_VERIFY` set?
pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(crate::keys::hosted_verify)
}

/// Is `LOFT_HOSTED_VERIFY_CONTROL` set: is the "compiled" side interpreted as well, so the
/// verify compares two interpreted runs and any difference is its copy's?
fn control() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_HOSTED_VERIFY_CONTROL").is_some())
}

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The calls compared alike, and those made without a comparison, by why: a silent run is a
/// reading only when it says how many calls it read ([`report`]).
static VERIFIED: AtomicU64 = AtomicU64::new(0);
static UNVERIFIED_TUPLE: AtomicU64 = AtomicU64::new(0);
static UNVERIFIED_FILE: AtomicU64 = AtomicU64::new(0);
static UNVERIFIED_NATIVE: AtomicU64 = AtomicU64::new(0);
static UNVERIFIED_SAMPLED: AtomicU64 = AtomicU64::new(0);

/// Is this call of `d` one the verify compares?  Each copies the whole world, so a function
/// called tens of thousands of times would take the run's time: its first 64 calls are
/// compared, then its 128th, 256th, … — every function's early calls and a thinning sample
/// after.  `LOFT_HOSTED_VERIFY=all` compares every call.
fn sampled(d: u32) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static ALL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ALL.get_or_init(|| std::env::var("LOFT_HOSTED_VERIFY").is_ok_and(|v| v == "all")) {
        return true;
    }
    static CALLS: std::sync::OnceLock<Mutex<HashMap<u32, u64>>> = std::sync::OnceLock::new();
    let Ok(mut calls) = CALLS.get_or_init(|| Mutex::new(HashMap::new())).lock() else {
        return true;
    };
    let n = calls.entry(d).or_insert(0);
    *n += 1;
    *n <= 64 || n.is_power_of_two()
}
/// Of the calls compared alike, those whose stores hold the same values placed otherwise.
static PLACED_OTHERWISE: AtomicU64 = AtomicU64::new(0);

/// Does a call to `d` reach code outside loft — a `#c` binding, a library's native, a standard
/// native that touches the host — whose state no copy of the stores holds (a file, a generator,
/// a handle counter, a device)?  Run twice,
/// such a call answers differently by itself, so it is made once and counted, not compared.
fn reaches_outside(data: &crate::data::Data, d: u32) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static SEEN: std::sync::OnceLock<Mutex<HashMap<u32, bool>>> = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(&known) = seen.lock().ok().and_then(|m| m.get(&d).copied()).as_ref() {
        return known;
    }
    let outside = crate::opt::names::reachable_functions(data, &[d])
        .into_iter()
        .map(|f| data.def(f))
        .any(|f| {
            // A library function an artifact carries is marked with its bridge's symbol
            // (`native_adopt::mark_exports`): loft code, not a native of its own.
            let bridged = f.native().starts_with("loft_shared_");
            let native = !f.native().is_empty() && !bridged;
            // What reaches the host: a definition gated by a capability (`fs#update` on
            // `OpWriteFile`, `fs#read` — the sandbox's mark: a write lands twice, a read moves
            // the position), and a standard native `#impure` toward it (the clock, the
            // generator), whose marks are not complete (`pure_reuse::reads_only`).
            let host = !f.cap.is_empty()
                || (native
                    && matches!(
                        f.purity,
                        crate::data::Purity::Impure(
                            crate::data::ImpureCategory::HostIo
                                | crate::data::ImpureCategory::Prng
                                | crate::data::ImpureCategory::Io
                        )
                    ));
            f.is_c_binding() || host || (native && f.source != crate::data::STD_SOURCE)
        });
    if let Ok(mut m) = seen.lock() {
        m.insert(d, outside);
    }
    outside
}

/// The calls this run compared, as one line on stderr, and the counts reset: under the switch
/// at the end of each run, nothing without it.
pub(crate) fn report() {
    if !enabled() {
        return;
    }
    let (verified, tuple, file, outside) = (
        VERIFIED.swap(0, Relaxed),
        UNVERIFIED_TUPLE.swap(0, Relaxed),
        UNVERIFIED_FILE.swap(0, Relaxed),
        UNVERIFIED_NATIVE.swap(0, Relaxed),
    );
    let mut why = Vec::new();
    if outside > 0 {
        why.push(format!(
            "{outside} reaching state outside the stores (a C function, a file, the clock)"
        ));
    }
    let sampled_out = UNVERIFIED_SAMPLED.swap(0, Relaxed);
    if sampled_out > 0 {
        why.push(format!("{sampled_out} past each function's sample"));
    }
    if tuple > 0 {
        why.push(format!("{tuple} with a tuple argument or answer"));
    }
    if file > 0 {
        why.push(format!("{file} with a file-backed store live"));
    }
    let placed = PLACED_OTHERWISE.swap(0, Relaxed);
    let placed = if placed > 0 {
        format!(" ({placed} of them alike in value, placed otherwise)")
    } else {
        String::new()
    };
    eprintln!(
        "loft: LOFT_HOSTED_VERIFY: {verified} call(s) compiled and interpreted alike{placed}{}",
        if why.is_empty() {
            String::new()
        } else {
            format!("; made without a comparison: {}", why.join(", "))
        }
    );
}

impl State {
    /// Run `bridge` for function `d_nr` with `args` as the dispatcher would, and the function's
    /// own body beside it, and compare the two worlds (module doc).  `popped` holds one slot per
    /// attribute, as the dispatcher popped them.
    ///
    /// # Safety
    /// As the dispatcher's own call: `bridge` is a loaded artifact's bridge for `d_nr`, `args`
    /// the slots it takes, and `ret` room for its answer.
    #[expect(
        clippy::too_many_lines,
        reason = "the twin's run and the comparison of the two worlds, in order"
    )]
    pub(crate) unsafe fn verify_compiled_call(
        &mut self,
        d_nr: u32,
        bridge: Bridge,
        args: &[LibArg],
        ret: *mut LibArg,
        popped: &[LibArg],
    ) {
        let handle = self.data_ptr.clone();
        let Some(data) = handle.get() else {
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        };
        let def = data.def(d_nr);
        // A generator's frame lives outside the stores, in a coroutine table: its answer is a
        // handle, and running it twice would make two frames.
        if reaches_outside(data, d_nr)
            || matches!(def.returned().base(), crate::data::Type::Iterator(..))
        {
            UNVERIFIED_NATIVE.fetch_add(1, Relaxed);
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        }
        if !in_twin() && !sampled(d_nr) {
            UNVERIFIED_SAMPLED.fetch_add(1, Relaxed);
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        }
        // A tuple argument crosses as several slots, not lined up with the attributes; a tuple
        // answer has no case in the call-back the twin runs: such a call is made unverified
        // (inside a twin too, where it then runs compiled).
        if def.attributes().len() != popped.len()
            || matches!(def.returned().base(), crate::data::Type::Tuple(_))
        {
            UNVERIFIED_TUPLE.fetch_add(1, Relaxed);
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        }
        let mut visible = Vec::new();
        let mut hidden: Vec<u16> = Vec::new();
        // The caller's hidden return buffers, which the interpreted twin answers into as an
        // interpreted call does (`State::call_back_into`).
        let mut dests: Vec<DbRef> = Vec::new();
        for (a, p) in def.attributes().iter().zip(popped) {
            if a.hidden {
                let text_work = crate::native_gate::is_text_work_buffer(&a.typedef);
                // The twin takes every caller buffer but a text work buffer (its own), so its
                // parameters line up with the call's.
                if !text_work {
                    dests.push(p.dbref);
                }
                // A work buffer is the call's scratch (a table a body builds, a text it writes
                // into), which a compiled body may keep elsewhere: not compared.  An answer
                // buffer is.
                if text_work || a.work_buffer {
                    hidden.push(p.dbref.store_nr);
                }
            } else {
                visible.push(*p);
            }
        }
        // Inside an interpreted twin every library call is interpreted too: the twin is what
        // the interpreter alone answers, so a call it makes is neither compiled nor verified
        // again (a carried function that calls another, `mesh3d`'s `cube` its `vec3`).
        if in_twin() {
            let r = self.twin_call(d_nr, &visible, &dests);
            unsafe { *ret = r };
            return;
        }
        // The caller's answer buffers are compared too: the twin answers into them as the
        // compiled call does (`State::call_back_into`), so a vector answer is held to the
        // compiled one by its store.
        let before: Vec<u16> = self
            .database
            .allocations
            .iter()
            .enumerate()
            .filter(|(nr, s)| s.live && !s.stack_buffer && !hidden.contains(&(*nr as u16)))
            .map(|(nr, _)| nr as u16)
            .collect();
        // The compiled run, on a copy of the world.  A world holding a file-backed store has
        // no copy: that call is made unverified.
        let Some(copy) = self.database.world_copy() else {
            UNVERIFIED_FILE.fetch_add(1, Relaxed);
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        };
        let mut twin = Box::new(copy);
        // The caller's text destination is the program's, which the dispatcher fills after
        // the compiled call: the twin answers its text through its own buffer.
        twin.bridge_text_dest = None;
        // A text argument views the bytes of the store it lies in: the twin reads the same
        // bytes in its own copy of that store.
        let rebased: Vec<LibArg> = visible
            .iter()
            .map(|a| {
                let mut a = *a;
                a.text_ptr = rebase(&self.database, &twin, a.text_ptr);
                a
            })
            .collect();
        // `&` arguments name slots of the caller's variables, on the interpreter's stacks, which
        // the twin is lent and the compiled call writes first: the twin starts from each slot
        // as it was before the call, and the two answers are compared like a store's.
        let slots: Vec<(String, DbRef, usize)> = def
            .attributes()
            .iter()
            .zip(popped)
            .filter(|(a, _)| !a.hidden)
            .filter_map(|(a, p)| match a.typedef.base() {
                crate::data::Type::RefVar(inner) => Some((
                    a.name.clone(),
                    p.dbref,
                    if crate::data::is_dbref_slot(inner.base()) {
                        std::mem::size_of::<DbRef>()
                    } else {
                        8
                    },
                )),
                _ => None,
            })
            .collect();
        let before_call: Vec<Vec<u8>> = slots
            .iter()
            .map(|(_, r, n)| slot_bytes(&self.database, r, *n))
            .collect();
        // The compiled run, on the world itself — what the program goes on with, as it does
        // without the switch: the caller's views into its stores stay where they point, and
        // what the unit makes once (its constants) lives in the world the program keeps.
        // `LOFT_HOSTED_VERIFY_CONTROL` — the verify's own control: the call interpreted on the
        // world too, so a difference is the copy's, not the unit's.
        let ret_c = if control() {
            let (fb, fc) = (self.fnref_bufs.len(), self.fnref_calls.len());
            let r = self.twin_call(d_nr, &visible, &dests);
            self.fnref_bufs.truncate(fb);
            self.fnref_calls.truncate(fc);
            unsafe { *ret = r };
            r
        } else {
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            unsafe { *ret }
        };
        // The interpreted twin, on the copy taken before the call.  The interpreter's own
        // stacks are lent to it (its frames run on them), and handed back after; the
        // call-back's bookkeeping is put back, as the compiled world is the one kept.
        let compiled_slots: Vec<Vec<u8>> = slots
            .iter()
            .map(|(_, r, n)| slot_bytes(&self.database, r, *n))
            .collect();
        for ((_, r, _), b) in slots.iter().zip(&before_call) {
            set_slot_bytes(&mut self.database, r, b);
        }
        let (fb, fc) = (self.fnref_bufs.len(), self.fnref_calls.len());
        lend_stacks(&mut self.database, &mut twin);
        std::mem::swap(&mut self.database, &mut *twin);
        let ret_i = self.twin_call(d_nr, &rebased, &dests);
        std::mem::swap(&mut self.database, &mut *twin);
        lend_stacks(&mut self.database, &mut twin);
        self.stack_base = super::stack_base_of(&self.database, &self.stack_cur);
        self.database.hosted_view = crate::host_abi::TableView::of(&mut self.database);
        self.fnref_bufs.truncate(fb);
        self.fnref_calls.truncate(fc);
        let mut slot_diff = None;
        for ((name, r, n), compiled_b) in slots.iter().zip(&compiled_slots) {
            let interp_b = slot_bytes(&self.database, r, *n);
            if slot_diff.is_none() && &interp_b != compiled_b {
                slot_diff = Some(format!(
                    "the `&` argument `{name}` holds {} interpreted, {} compiled",
                    hex(&interp_b),
                    hex(compiled_b)
                ));
            }
            set_slot_bytes(&mut self.database, r, compiled_b);
        }
        let (interp, compiled): (&Stores, &Stores) = (&twin, &self.database);
        let (mut diff, placed_otherwise) = stores_difference(&before, interp, compiled);
        // Which function's code made the store that differs: its bytecode position, mapped to
        // the function whose code holds it.
        if let Some(why) = &mut diff
            && let Some(nr) = why
                .strip_prefix("store ")
                .and_then(|r| r.split(' ').next())
                .and_then(|n| n.parse::<usize>().ok())
            && let Some(st) = interp.allocations.get(nr)
        {
            let at = st.created_at;
            let maker = (0..self.fn_positions.len())
                .filter(|&f| self.fn_positions[f] != u32::MAX && self.fn_positions[f] <= at)
                .max_by_key(|&f| self.fn_positions[f])
                .map_or_else(
                    || "?".to_string(),
                    |f| data.def(f as u32).original_name().clone(),
                );
            let _ = write!(why, " — made at bytecode {at}, in `{maker}`");
        }
        if diff.is_none() {
            diff = slot_diff;
        }
        if diff.is_none() {
            diff = answer_difference(data, d_nr, (&ret_i, interp), (&ret_c, compiled));
        }
        if let Some(why) = diff {
            eprintln!(
                "loft: LOFT_HOSTED_VERIFY: `{}` leaves another world compiled than interpreted: {why}",
                def.original_name()
            );
            std::process::exit(1);
        }
        VERIFIED.fetch_add(1, Relaxed);
        if placed_otherwise {
            PLACED_OTHERWISE.fetch_add(1, Relaxed);
        }
    }
}

/// The `n` bytes at slot `r`, as the interpreter holds them.
fn slot_bytes(stores: &Stores, r: &DbRef, n: usize) -> Vec<u8> {
    let store = stores.store(r);
    (0..n as u32)
        .map(|i| *store.addr::<u8>(r.rec, r.pos + i))
        .collect()
}

fn set_slot_bytes(stores: &mut Stores, r: &DbRef, bytes: &[u8]) {
    let store = stores.store_mut(r);
    for (i, b) in bytes.iter().enumerate() {
        *store.addr_mut::<u8>(r.rec, r.pos + i as u32) = *b;
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

thread_local! {
    /// How deep the interpreted twins running on this thread are nested.
    static TWIN: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

fn in_twin() -> bool {
    TWIN.with(std::cell::Cell::get) > 0
}

impl State {
    /// Function `d_nr` interpreted with `args`, answering into the caller's buffers `dests`
    /// (`State::call_back_into`), every library call it makes interpreted as well (`in_twin`).
    fn twin_call(&mut self, d_nr: u32, args: &[LibArg], dests: &[DbRef]) -> LibArg {
        TWIN.with(|t| t.set(t.get() + 1));
        let r = self.call_back_into(d_nr, DbRef::NULL, args, dests);
        TWIN.with(|t| t.set(t.get() - 1));
        r
    }
}

/// The first store of `before` the two worlds hold otherwise — live on one side only, of another
/// root type, or with other bytes that render another value — and whether a store held the same
/// value placed otherwise.
fn stores_difference(before: &[u16], interp: &Stores, compiled: &Stores) -> (Option<String>, bool) {
    let mut placed_otherwise = false;
    for &nr in before {
        let (on_twin, on_world) = (
            &interp.allocations[nr as usize],
            &compiled.allocations[nr as usize],
        );
        if on_twin.live != on_world.live {
            let held = if on_twin.live { on_twin } else { on_world };
            return (
                Some(format!(
                    "store {nr} (type {}, made #{}) is live only {}",
                    held.known_type,
                    held.alloc_serial,
                    if on_twin.live {
                        "interpreted"
                    } else {
                        "compiled"
                    }
                )),
                placed_otherwise,
            );
        }
        // `@PLN206 P5` — the root's type, which a hosted unit writes through its translation by
        // key (`loft_tp_out`): another type here is a crossing left untranslated.
        if on_twin.known_type != on_world.known_type {
            return (
                Some(format!(
                    "store {nr} holds type {} interpreted, {} compiled",
                    on_twin.known_type, on_world.known_type
                )),
                placed_otherwise,
            );
        }
        let (image_twin, image_world) = (on_twin.claimed_image(), on_world.claimed_image());
        if image_twin != image_world {
            // The same value placed otherwise — a record claimed elsewhere, a text written anew
            // where the other side wrote it in place (`@FR-H-TextReplace` is admitted by
            // whole-program facts an open unit does not have) — is no wrong value: the root,
            // rendered through its type, follows references, not record numbers.
            if on_twin.known_type != u16::MAX {
                let root = DbRef {
                    store_nr: nr,
                    rec: 1,
                    pos: 8,
                };
                let (mut shown_twin, mut shown_world) = (String::new(), String::new());
                interp.show(&mut shown_twin, &root, on_twin.known_type, false);
                compiled.show(&mut shown_world, &root, on_world.known_type, false);
                if shown_twin == shown_world {
                    placed_otherwise = true;
                    continue;
                }
            }
            let what = first_difference(nr, &image_twin, &image_world);
            let named = if on_twin.known_type == u16::MAX {
                String::new()
            } else {
                format!(" `{}`", interp.type_name(on_twin.known_type))
            };
            return (
                Some(format!(
                    "{what} (store {nr}: type {}{named}, made #{})",
                    on_twin.known_type, on_twin.alloc_serial
                )),
                placed_otherwise,
            );
        }
    }
    (None, placed_otherwise)
}

/// Exchange the interpreter's stack stores between `a` and `b`: the live ones go to the world
/// the interpreter runs on next, the copies to the other.
fn lend_stacks(a: &mut Stores, b: &mut Stores) {
    for k in 0..a.allocations.len().min(b.allocations.len()) {
        if a.allocations[k].stack_buffer || b.allocations[k].stack_buffer {
            std::mem::swap(&mut a.allocations[k], &mut b.allocations[k]);
        }
    }
}

/// `p` moved from a store of `from` into the same place of the same store of `to`; a pointer
/// into no store (a constant, a text of the interpreter's own) is left as it is.
fn rebase(from: &Stores, to: &Stores, p: *const u8) -> *const u8 {
    if p.is_null() {
        return p;
    }
    for (a, b) in from.allocations.iter().zip(&to.allocations) {
        let base = a.ptr as usize;
        let at = p as usize;
        let len = usize::try_from(a.byte_capacity()).unwrap_or(0);
        if a.live && at >= base && at < base + len {
            return unsafe { b.ptr.add(at - base) };
        }
    }
    p
}

/// The answers of the two runs, each with the world it was made in, compared by the returned
/// type.
fn answer_difference(
    data: &crate::data::Data,
    d_nr: u32,
    (interp, interp_world): (&LibArg, &Stores),
    (comp, compiled): (&LibArg, &Stores),
) -> Option<String> {
    use crate::data::Type;
    let ret = data.def(d_nr).returned();
    match ret.base() {
        Type::Void | Type::Null => None,
        // A text answer: the compiled call's bytes (a hosted unit's, `loft_la_set_text`) against
        // the twin's; a rustc artifact writes its destination itself and answers none.
        Type::Text(_) => {
            if comp.text_ptr.is_null() {
                return None;
            }
            let read = |a: &LibArg| {
                if a.text_ptr.is_null() || a.text_len == 0 {
                    String::new()
                } else {
                    String::from_utf8_lossy(unsafe {
                        std::slice::from_raw_parts(a.text_ptr, a.text_len)
                    })
                    .into_owned()
                }
            };
            let (said_twin, said_world) = (read(interp), read(comp));
            (said_twin != said_world).then(|| {
                format!("the answer is {said_twin:?} interpreted, {said_world:?} compiled")
            })
        }
        t if crate::data::is_dbref(t) => {
            let tp = ret
                .heap_def_nr()
                .map_or(u16::MAX, |r| data.def(r).known_type());
            if tp == u16::MAX {
                return None;
            }
            let (mut shown_twin, mut shown_world) = (String::new(), String::new());
            if interp.dbref.rec != 0 {
                interp_world.show(&mut shown_twin, &interp.dbref, tp, false);
            }
            if comp.dbref.rec != 0 {
                compiled.show(&mut shown_world, &comp.dbref, tp, false);
            }
            if shown_twin != shown_world {
                return Some(format!(
                    "the answer is {shown_twin} interpreted, {shown_world} compiled"
                ));
            }
            // `@PLN206 P5` — the store the answer is the root of holds its type, which a
            // hosted unit writes through its translation by key: the answer's store is the
            // hidden buffer the stores' comparison leaves out, so its type is compared here.
            let root_type = |s: &Stores, r: &DbRef| {
                (r.rec == 1)
                    .then(|| s.allocations.get(usize::from(r.store_nr)))
                    .flatten()
                    .map(|st| st.known_type)
            };
            match (
                root_type(interp_world, &interp.dbref),
                root_type(compiled, &comp.dbref),
            ) {
                (Some(x), Some(y)) if x != y => Some(format!(
                    "the answer's store holds type {x} interpreted, {y} compiled"
                )),
                _ => None,
            }
        }
        _ => {
            // A value crosses in its own width: a boolean or a plain enum is its byte, a
            // character or a single its 32 bits; the slot's bits above are not the answer.
            let mask: u64 = match ret.base() {
                Type::Boolean | Type::Enum(_, false, _) => 0xFF,
                Type::Character | Type::Single => 0xFFFF_FFFF,
                _ => u64::MAX,
            };
            let (x, y) = (interp.scalar as u64 & mask, comp.scalar as u64 & mask);
            (x != y).then(|| format!("the answer's bits are {x:#x} interpreted, {y:#x} compiled"))
        }
    }
}

/// Where two images of store `nr` first differ (`Store::claimed_image` lines).
fn first_difference(nr: u16, x: &str, y: &str) -> String {
    for (a, b) in x.lines().zip(y.lines()) {
        if a != b {
            // `R <rec> <words> <hex>`: name the record and the first byte that differs, with
            // the 16 bytes from there on each side.
            let field = |l: &str, i: usize| l.split(' ').nth(i).unwrap_or("").to_string();
            let rec = field(a, 1);
            let (wa, wb) = (field(a, 2), field(b, 2));
            if wa != wb {
                return format!("store {nr} record {rec} is {wa} words interpreted, {wb} compiled");
            }
            let (ha, hb) = (field(a, 3), field(b, 3));
            let at = ha
                .as_bytes()
                .chunks(2)
                .zip(hb.as_bytes().chunks(2))
                .position(|(p, q)| p != q)
                .unwrap_or(0);
            // Every 4-byte word that differs, as `@byte interpreted/compiled` (little-endian
            // hex as stored), so a field written elsewhere reads as one entry per field.
            let words: Vec<String> = ha
                .as_bytes()
                .chunks(8)
                .zip(hb.as_bytes().chunks(8))
                .enumerate()
                .filter(|(_, (p, q))| p != q)
                .take(8)
                .map(|(i, (p, q))| {
                    format!(
                        "@{} {}/{}",
                        i * 4,
                        String::from_utf8_lossy(p),
                        String::from_utf8_lossy(q)
                    )
                })
                .collect();
            return format!(
                "store {nr} record {rec} differs from byte {at}: {}",
                words.join(", ")
            );
        }
    }
    format!(
        "store {nr} holds {} blocks interpreted, {} compiled",
        x.lines().count(),
        y.lines().count()
    )
}
