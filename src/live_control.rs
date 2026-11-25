// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch: the control surface a compiled program carries

//! What a compiled program needs of the live tier whether or not it runs one: the debug
//! control channel's mailbox, and the entry points for a flip and a breakpoint.  The live
//! tier itself (`live_dispatch`) runs the interpreter, so it is not in the runtime a program
//! links (@PLN191); when it boots it INSTALLS its flip and breakpoint handlers here, and so
//! does a program run on the interpreter under `LOFT_DEBUG_CONTROL=1` (`@C170`).  Until
//! then the answers are the ones an unbooted live tier gives: a flip refuses, a breakpoint is
//! queued and acknowledged.
//!
//! The kernel (`engine_host`) owns the TRANSPORT (`D!:` frames, loopback +
//! `LOFT_DEBUG_CONTROL`-gated); `live_dispatch` owns the semantics.  Commands that need the
//! parked State arrive through the MAILBOX: when a dispatch is paused at a breakpoint, the
//! live state is mutably borrowed, so the control handler (which runs inside the pause
//! loop's mini-pump) must never borrow it again — it posts, and the pause loop applies.

use std::cell::UnsafeCell;
use std::sync::OnceLock;

use crate::database::Stores;
use crate::keys::DbRef;

use crate::engine_host::EngineCtl;

/// The live tier's flip, installed when it boots.  It answers for the program whose engine
/// control `EngineCtl` is: the parked interpreter is held there.
static FLIP: OnceLock<fn(&mut EngineCtl, &str, bool) -> bool> = OnceLock::new();
/// The live tier's breakpoint setter, installed when it boots.
#[cfg(not(target_arch = "wasm32"))]
static SET_BP: OnceLock<fn(&mut EngineCtl, &str, i64) -> bool> = OnceLock::new();

/// Called by the live tier's boot: route [`set_flip`] to it.
pub(crate) fn install_flip(flip: fn(&mut EngineCtl, &str, bool) -> bool) {
    let _ = FLIP.set(flip);
}

/// Called by the live tier's boot: route [`debug_set_bp`] to it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn install_set_bp(set_bp: fn(&mut EngineCtl, &str, i64) -> bool) {
    let _ = SET_BP.set(set_bp);
}

/// Flip `name` between its compiled body and the interpreter.  False when no live tier
/// has booted, or the function is not one it can flip.
pub(crate) fn set_flip(ctl: &mut EngineCtl, name: &str, on: bool) -> bool {
    FLIP.get().is_some_and(|flip| flip(ctl, name, on))
}

/// Set a breakpoint at the entry of `name`.  With no live tier booted, the request is
/// queued like one that arrives during a pause, and acknowledged.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn debug_set_bp(ctl: &mut EngineCtl, name: &str, cid: i64) -> bool {
    if let Some(set_bp) = SET_BP.get() {
        set_bp(ctl, name, cid)
    } else {
        debug_mailbox_push(&mut ctl.debug, DebugCmd::SetBp(name.to_string(), cid));
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

/// `@PLN226` — a program's debug channel state (`engine_host::EngineCtl::debug`): the
/// commands waiting for the pause loop, the breakpoint names (the identity that survives
/// reloads and re-resolution), the control client that hears `D:hit`, and the breakpoints the
/// interpreter's dispatch loop is to take at its next op boundary.
#[cfg(not(target_arch = "wasm32"))]
pub struct DebugCx {
    mailbox: Vec<DebugCmd>,
    pub(crate) bps: Vec<String>,
    pub(crate) sub: i64,
    pub(crate) queued: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for DebugCx {
    fn default() -> Self {
        DebugCx {
            mailbox: Vec::new(),
            bps: Vec::new(),
            sub: -1,
            queued: Vec::new(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn debug_mailbox_push(d: &mut DebugCx, cmd: DebugCmd) {
    d.mailbox.push(cmd);
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn debug_mailbox_drain(d: &mut DebugCx) -> Vec<DebugCmd> {
    std::mem::take(&mut d.mailbox)
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
pub fn n_live_flip(cell: &UnsafeCell<Stores>, name: &str, on: u8) -> u8 {
    let stores: &mut Stores = unsafe { &mut *cell.get() };
    u8::from(set_flip(&mut stores.engine.ctl, name, on == 1))
}

/// Interp stack native: under the interpreter everything is already
/// interpreted, so a flip is a no-op `false` — the differential stays clean.
pub fn n_live_flip_stack(stores: &mut Stores, stack: &mut DbRef) {
    let _on = stores.get::<u8>(stack);
    let _name = stores.get::<crate::keys::Str>(stack).str().to_owned();
    stores.put(stack, false);
}
