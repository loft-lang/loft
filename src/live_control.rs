// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch: the control surface a compiled program carries

//! What a compiled program needs of the live tier whether or not it runs one: the debug
//! control channel's mailbox, and the entry points for a flip and a breakpoint.  The live
//! tier itself (`live_dispatch`) runs the interpreter, so it is not in the runtime a program
//! links (@PLN191); when it boots it INSTALLS its flip and breakpoint handlers here.  Until
//! then the answers are the ones an unbooted live tier gives: a flip refuses, a breakpoint is
//! queued and acknowledged.
//!
//! The kernel (`engine_host`) owns the TRANSPORT (`D!:` frames, loopback +
//! `LOFT_DEBUG_CONTROL`-gated); `live_dispatch` owns the semantics.  Commands that need the
//! parked State arrive through the MAILBOX: when a dispatch is paused at a breakpoint, the
//! live state is mutably borrowed, so the control handler (which runs inside the pause
//! loop's mini-pump) must never borrow it again — it posts, and the pause loop applies.

#[cfg(not(target_arch = "wasm32"))]
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::sync::OnceLock;

use crate::database::Stores;
use crate::keys::DbRef;

/// The live tier's flip, installed when it boots.
static FLIP: OnceLock<fn(&str, bool) -> bool> = OnceLock::new();
/// The live tier's breakpoint setter, installed when it boots.
#[cfg(not(target_arch = "wasm32"))]
static SET_BP: OnceLock<fn(&str, i64) -> bool> = OnceLock::new();

/// Called by the live tier's boot: route [`set_flip`] to it.
pub fn install_flip(flip: fn(&str, bool) -> bool) {
    let _ = FLIP.set(flip);
}

/// Called by the live tier's boot: route [`debug_set_bp`] to it.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_set_bp(set_bp: fn(&str, i64) -> bool) {
    let _ = SET_BP.set(set_bp);
}

/// Flip `name` between its compiled body and the interpreter.  False when no live tier
/// has booted, or the function is not one it can flip.
pub fn set_flip(name: &str, on: bool) -> bool {
    FLIP.get().is_some_and(|flip| flip(name, on))
}

/// Set a breakpoint at the entry of `name`.  With no live tier booted, the request is
/// queued like one that arrives during a pause, and acknowledged.
#[cfg(not(target_arch = "wasm32"))]
pub fn debug_set_bp(name: &str, cid: i64) -> bool {
    if let Some(set_bp) = SET_BP.get() {
        set_bp(name, cid)
    } else {
        debug_mailbox_push(DebugCmd::SetBp(name.to_string(), cid));
        true
    }
}

/// A control command that needs the parked State (applied by the pause loop)
/// or arrived while one was borrowed.  `i64` = the requesting client's cid.
#[cfg(not(target_arch = "wasm32"))]
pub enum DebugCmd {
    SetBp(String, i64),
    Eval(String, i64),
    Resume(i64),
    Reload(i64),
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static DEBUG_MAILBOX: RefCell<Vec<DebugCmd>> = const { RefCell::new(Vec::new()) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn debug_mailbox_push(cmd: DebugCmd) {
    DEBUG_MAILBOX.with(|m| m.borrow_mut().push(cmd));
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn debug_mailbox_drain() -> Vec<DebugCmd> {
    DEBUG_MAILBOX.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Set by the editor's Apply (`D!:apply`): under `LOFT_LIVE_RELOAD=hold` a saved edit waits
/// for it, because a save is for reading the edit and its warnings, not for running it.
/// The reload poll takes it ([`take_apply`]).
static APPLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the live tier to apply the saved edits at its next poll.
pub fn request_apply() {
    APPLY.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether an Apply waits, without taking it.
#[must_use]
pub fn apply_pending() -> bool {
    APPLY.load(std::sync::atomic::Ordering::Relaxed)
}

/// Take a waiting Apply: true once per request.
pub fn take_apply() -> bool {
    APPLY.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Typed twin for generated code: `live_flip(name: text, on: boolean) -> boolean`.
pub fn n_live_flip(_cell: &UnsafeCell<Stores>, name: &str, on: u8) -> u8 {
    u8::from(set_flip(name, on == 1))
}

/// Interp stack native: under the interpreter everything is already
/// interpreted, so a flip is a no-op `false` — the differential stays clean.
pub fn n_live_flip_stack(stores: &mut Stores, stack: &mut DbRef) {
    let _on = stores.get::<u8>(stack);
    let _name = stores.get::<crate::keys::Str>(stack).str().to_owned();
    stores.put(stack, false);
}
