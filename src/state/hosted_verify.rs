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

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The calls compared alike, and those made without a comparison, by why: a silent run is a
/// reading only when it says how many calls it read ([`report`]).
static VERIFIED: AtomicU64 = AtomicU64::new(0);
static UNVERIFIED_TUPLE: AtomicU64 = AtomicU64::new(0);
static UNVERIFIED_FILE: AtomicU64 = AtomicU64::new(0);

/// The calls this run compared, as one line on stderr, and the counts reset: under the switch
/// at the end of each run, nothing without it.
pub(crate) fn report() {
    if !enabled() {
        return;
    }
    let (v, t, f) = (
        VERIFIED.swap(0, Relaxed),
        UNVERIFIED_TUPLE.swap(0, Relaxed),
        UNVERIFIED_FILE.swap(0, Relaxed),
    );
    let mut why = Vec::new();
    if t > 0 {
        why.push(format!("{t} with a tuple argument"));
    }
    if f > 0 {
        why.push(format!("{f} with a file-backed store live"));
    }
    eprintln!(
        "loft: LOFT_HOSTED_VERIFY: {v} call(s) compiled and interpreted alike{}",
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
        let mut compiled = Box::new(copy);
        compiled.hosted_view = crate::host_abi::TableView::of(&mut compiled);
        unsafe { bridge(&raw mut *compiled, args.as_ptr(), args.len(), ret) };
        let ret_c = unsafe { *ret };
        // The interpreted run, on the world itself; the call-back's bookkeeping is put back.
        let (fb, fc) = (self.fnref_bufs.len(), self.fnref_calls.len());
        let ret_i = self.call_back(d_nr, DbRef::NULL, &visible);
        self.fnref_bufs.truncate(fb);
        self.fnref_calls.truncate(fc);
        let mut diff: Option<String> = None;
        for nr in before {
            let (a, b) = (
                &self.database.allocations[nr as usize],
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
                diff = Some(first_difference(nr, &x, &y));
                break;
            }
        }
        if diff.is_none() {
            diff = self.answer_difference(data, d_nr, &ret_i, &ret_c, &compiled);
        }
        if let Some(why) = diff {
            eprintln!(
                "loft: LOFT_HOSTED_VERIFY: `{}` leaves another world compiled than interpreted: {why}",
                def.original_name()
            );
            std::process::exit(1);
        }
        VERIFIED.fetch_add(1, Relaxed);
        // The program goes on in the compiled world, as it does without the switch.  The
        // interpreter's own stacks are the run's, not the call's: the compiled world takes them
        // over as they stand, and the cached stack base is read again (`restore_checkpoint`).
        for k in 0..self
            .database
            .allocations
            .len()
            .min(compiled.allocations.len())
        {
            if self.database.allocations[k].stack_buffer {
                std::mem::swap(
                    &mut self.database.allocations[k],
                    &mut compiled.allocations[k],
                );
            }
        }
        std::mem::swap(&mut self.database, &mut *compiled);
        self.stack_base = super::stack_base_of(&self.database, &self.stack_cur);
    }

    /// The answers of the two runs, compared by the returned type.
    fn answer_difference(
        &self,
        data: &crate::data::Data,
        d_nr: u32,
        interp: &LibArg,
        comp: &LibArg,
        compiled: &Stores,
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
                    self.database.show(&mut a, &interp.dbref, tp, false);
                }
                if comp.dbref.rec != 0 {
                    compiled.show(&mut b, &comp.dbref, tp, false);
                }
                (a != b).then(|| format!("the answer is {a} interpreted, {b} compiled"))
            }
            _ => (interp.scalar != comp.scalar).then(|| {
                format!(
                    "the answer's bits are {:#x} interpreted, {:#x} compiled",
                    interp.scalar, comp.scalar
                )
            }),
        }
    }
}

/// Where two images of store `nr` first differ (`Store::claimed_image` lines).
fn first_difference(nr: u16, x: &str, y: &str) -> String {
    for (a, b) in x.lines().zip(y.lines()) {
        if a != b {
            let rec = a.split(' ').nth(1).unwrap_or("?");
            return format!(
                "store {nr} record {rec} differs: `{a:.80}` interpreted, `{b:.80}` compiled"
            );
        }
    }
    format!(
        "store {nr} holds {} blocks interpreted, {} compiled",
        x.lines().count(),
        y.lines().count()
    )
}
