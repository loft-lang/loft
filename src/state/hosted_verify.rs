// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
/// Of the calls compared alike, those whose stores hold the same values placed otherwise.
static PLACED_OTHERWISE: AtomicU64 = AtomicU64::new(0);

/// Does a call to `d` reach code outside loft — a `#c` binding or a library's native — whose
/// state no copy of the stores holds (a generator, a handle counter, a device)?  Run twice,
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
            f.is_c_binding()
                || (!f.native().is_empty() && !bridged && f.source != crate::data::STD_SOURCE)
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
    let (v, t, f, n) = (
        VERIFIED.swap(0, Relaxed),
        UNVERIFIED_TUPLE.swap(0, Relaxed),
        UNVERIFIED_FILE.swap(0, Relaxed),
        UNVERIFIED_NATIVE.swap(0, Relaxed),
    );
    let mut why = Vec::new();
    if n > 0 {
        why.push(format!("{n} reaching a C function's own state"));
    }
    if t > 0 {
        why.push(format!("{t} with a tuple argument"));
    }
    if f > 0 {
        why.push(format!("{f} with a file-backed store live"));
    }
    let placed = PLACED_OTHERWISE.swap(0, Relaxed);
    let placed = if placed > 0 {
        format!(" ({placed} of them alike in value, placed otherwise)")
    } else {
        String::new()
    };
    eprintln!(
        "loft: LOFT_HOSTED_VERIFY: {v} call(s) compiled and interpreted alike{placed}{}",
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
        // A tuple argument crosses as several slots: not lined up with the attributes, so the
        // call is made unverified.
        if reaches_outside(data, d_nr) {
            UNVERIFIED_NATIVE.fetch_add(1, Relaxed);
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        }
        if def.attributes().len() != popped.len() {
            UNVERIFIED_TUPLE.fetch_add(1, Relaxed);
            unsafe { bridge(&raw mut self.database, args.as_ptr(), args.len(), ret) };
            return;
        }
        let mut visible = Vec::new();
        let mut hidden: Vec<u16> = Vec::new();
        for (a, p) in def.attributes().iter().zip(popped) {
            if a.hidden {
                hidden.push(p.dbref.store_nr);
            } else {
                visible.push(*p);
            }
        }
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
        // The compiled run, on the world itself — what the program goes on with, as it does
        // without the switch: the caller's views into its stores stay where they point, and
        // what the unit makes once (its constants) lives in the world the program keeps.
        // `LOFT_HOSTED_VERIFY_CONTROL` — the verify's own control: the call interpreted on the
        // world too, so a difference is the copy's, not the unit's.
        let ret_c = if control() {
            let (fb, fc) = (self.fnref_bufs.len(), self.fnref_calls.len());
            let r = self.call_back(d_nr, DbRef::NULL, &visible);
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
        let (fb, fc) = (self.fnref_bufs.len(), self.fnref_calls.len());
        lend_stacks(&mut self.database, &mut twin);
        std::mem::swap(&mut self.database, &mut *twin);
        let ret_i = self.call_back(d_nr, DbRef::NULL, &rebased);
        std::mem::swap(&mut self.database, &mut *twin);
        lend_stacks(&mut self.database, &mut twin);
        self.stack_base = super::stack_base_of(&self.database, &self.stack_cur);
        self.database.hosted_view = crate::host_abi::TableView::of(&mut self.database);
        self.fnref_bufs.truncate(fb);
        self.fnref_calls.truncate(fc);
        let (interp, compiled): (&Stores, &Stores) = (&twin, &self.database);
        let mut diff: Option<String> = None;
        let mut placed_otherwise = false;
        for nr in before {
            let (a, b) = (
                &interp.allocations[nr as usize],
                &compiled.allocations[nr as usize],
            );
            if a.live != b.live {
                diff = Some(format!(
                    "store {nr} is live only {}",
                    if a.live { "interpreted" } else { "compiled" }
                ));
                break;
            }
            // `@PLN206 P5` — the root's type, which a hosted unit writes through its translation
            // by key (`loft_tp_out`): another type here is a crossing left untranslated.
            if a.known_type != b.known_type {
                diff = Some(format!(
                    "store {nr} holds type {} interpreted, {} compiled",
                    a.known_type, b.known_type
                ));
                break;
            }
            let (x, y) = (a.claimed_image(), b.claimed_image());
            if x != y {
                // The same value placed otherwise — a record claimed elsewhere, a text written
                // anew where the other side wrote it in place (`@FR-H-TextReplace` is admitted
                // by whole-program facts an open unit does not have) — is no wrong value: the
                // root, rendered through its type, follows references, not record numbers.
                if a.known_type != u16::MAX {
                    let root = DbRef {
                        store_nr: nr,
                        rec: 1,
                        pos: 8,
                    };
                    let (mut p, mut q) = (String::new(), String::new());
                    interp.show(&mut p, &root, a.known_type, false);
                    compiled.show(&mut q, &root, b.known_type, false);
                    if p == q {
                        placed_otherwise = true;
                        continue;
                    }
                }
                diff = Some(first_difference(nr, &x, &y));
                break;
            }
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
        Type::Void | Type::Null | Type::Text(_) => None,
        t if crate::data::is_dbref(t) => {
            let tp = ret
                .heap_def_nr()
                .map_or(u16::MAX, |r| data.def(r).known_type());
            if tp == u16::MAX {
                return None;
            }
            let (mut a, mut b) = (String::new(), String::new());
            if interp.dbref.rec != 0 {
                interp_world.show(&mut a, &interp.dbref, tp, false);
            }
            if comp.dbref.rec != 0 {
                compiled.show(&mut b, &comp.dbref, tp, false);
            }
            if a != b {
                return Some(format!("the answer is {a} interpreted, {b} compiled"));
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
        t => {
            // A value crosses in its own width: a boolean or a plain enum is its byte, a
            // character or a single its 32 bits; the slot's bits above are not the answer.
            let mask: u64 = match t {
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
                return format!(
                    "store {nr} record {rec} is {wa} words interpreted, {wb} compiled"
                );
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
