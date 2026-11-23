// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@PLN206 P7` — a compiled routine handed to the interpreter mid-run.  A pause the debugger
//! asks for while a hosted unit's routine runs reaches the routine the interpreter called at
//! its next loop head (`loft_pause_asked`); it offers its locals by name, and when every local
//! the interpreter's frame holds at that loop's head (`State::loop_sites`, `frame_view`) has
//! its value among them, the interpreter takes the routine over: the unit returns without its
//! own releases (the frame owns its stores now), the dispatcher answers nothing, and the static
//! call becomes the routine's interpreted frame at the loop's head — where the pause then stops
//! it.  Design: `doc/claude/plans/206-one-store-model/p7-design.md`.
use super::State;
use crate::data::{Context, Type};
use crate::host_abi::HandoverEntry;
use crate::keys::{DbRef, Str};

/// One slot of the frame a hand-over lays out.
pub(crate) enum SlotWrite {
    /// The value's bytes, as the slot holds them.
    Bytes(Vec<u8>),
    /// A text local: its own `String`.
    Text(String),
    /// A text argument: a view of the caller's bytes.
    View(*const u8, u32),
}

/// A routine the interpreter takes over: where it continues and the frame it continues with.
pub(crate) struct Handover {
    d_nr: u32,
    pc: u32,
    writes: Vec<(u32, SlotWrite)>,
}

impl State {
    /// `HostedTable::handover` — take the routine `key` over at the head of its loop `scope`,
    /// when the frame the interpreter keeps there can be filled from `entries`.  False leaves
    /// the routine running compiled, to its next loop head or its return.
    pub(crate) fn accept_handover(&mut self, key: &str, scope: u16, entries: &[HandoverEntry]) -> bool {
        if self.debug.is_none() || crate::state::hosted_verify_enabled() {
            return false;
        }
        let handle = self.data_ptr.clone();
        let Some(data) = handle.get() else {
            return false;
        };
        let d_nr = self.bridge_call_d;
        if d_nr == u32::MAX || crate::lib_ids::fn_key(data, d_nr) != key {
            return false;
        }
        let Some(&pc) = self.loop_sites.get(&(d_nr, scope)) else {
            return false;
        };
        let offered: std::collections::HashMap<String, &HandoverEntry> = entries
            .iter()
            .filter_map(|e| {
                // SAFETY: the unit's own NUL-terminated name literal.
                let name = unsafe { std::ffi::CStr::from_ptr(e.name) }.to_str().ok()?;
                Some((name.to_string(), e))
            })
            .collect();
        let vars = data.def(d_nr).variables();
        let view = self.frame_view(d_nr, pc, data);
        let mut writes = Vec::new();
        let mut held_spans: Vec<(u32, u32)> = Vec::new();
        let mut taken = std::collections::HashSet::new();
        for entry in view.iter().filter(|e| e.state.is_held()) {
            let str_slot = vars.holds_str(entry.var_nr);
            let ctx = if str_slot {
                &Context::Argument
            } else {
                &Context::Variable
            };
            let size = u32::from(crate::variables::size(&entry.tp, ctx));
            let Some(e) = offered.get(&entry.name) else {
                return false;
            };
            let Some(w) = slot_write(&entry.tp, size, str_slot, e) else {
                return false;
            };
            taken.insert(entry.name.clone());
            held_spans.push((u32::from(entry.slot), u32::from(entry.slot) + size));
            writes.push((u32::from(entry.slot), w));
        }
        // A store the routine holds that the frame does not would be held by no one.
        if offered
            .iter()
            .any(|(name, e)| e.kind == 3 && e.rr != 0 && !taken.contains(name))
        {
            return false;
        }
        // A local in scope the frame does not hold yet starts empty where its scope's end will
        // release it — a text, a reference — unless its slot is another local's.
        for entry in view.iter().filter(|e| !e.state.is_held()) {
            let str_slot = vars.holds_str(entry.var_nr);
            let ctx = if str_slot {
                &Context::Argument
            } else {
                &Context::Variable
            };
            let (from, size) = (
                u32::from(entry.slot),
                u32::from(crate::variables::size(&entry.tp, ctx)),
            );
            if held_spans.iter().any(|&(a, b)| from < b && a < from + size) {
                continue;
            }
            let w = match entry.tp.base() {
                Type::Text(_) if !str_slot => SlotWrite::Text(String::new()),
                t if crate::data::is_dbref(t) => SlotWrite::Bytes(dbref_bytes(DbRef::NULL)),
                _ => continue,
            };
            writes.push((from, w));
        }
        self.pending_handover = Some(Handover { d_nr, pc, writes });
        true
    }

    /// The static call whose routine handed itself over continues as that routine's frame at
    /// its loop head: the arguments where the call left them, the frame entered as `fn_call`
    /// enters one, the eval stack at the loop head's depth, each slot as the hand-over wrote it.
    pub(crate) fn resume_handover(&mut self) {
        let Some(h) = self.pending_handover.take() else {
            return;
        };
        let handle = self.data_ptr.clone();
        let Some(data) = handle.get() else {
            return;
        };
        let args_size: u32 = data
            .def(h.d_nr)
            .attributes()
            .iter()
            .map(|a| {
                crate::variables::aligned_stack_step(u32::from(crate::variables::size(
                    &a.typedef,
                    &Context::Argument,
                )))
            })
            .sum();
        let args_base = self.stack_pos;
        self.stack_pos = args_base + args_size;
        self.fn_call(h.d_nr, args_size as u16, i64::from(h.pc));
        if let Some(&depth) = self.stack.get(&h.pc) {
            self.stack_pos = args_base + u32::from(depth);
        }
        let cur = self.stack_cur;
        for (slot, w) in h.writes {
            let pos = cur.pos + args_base + slot;
            let store = self.database.store_mut(&cur);
            match w {
                SlotWrite::Bytes(b) => {
                    let at = store.addr_span_mut(cur.rec, pos, b.len());
                    // SAFETY: the frame's slot, inside the stack store's room for this frame.
                    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), at, b.len()) };
                }
                SlotWrite::Text(t) => {
                    let at = store.addr_span_mut(cur.rec, pos, std::mem::size_of::<String>());
                    // SAFETY: a text local's slot, written as a fresh `String` (no drop of what
                    // the slot held: nothing of this frame was there).
                    unsafe { std::ptr::write(at.cast::<String>(), t) };
                }
                SlotWrite::View(p, len) => {
                    let at = store.addr_span_mut(cur.rec, pos, std::mem::size_of::<Str>());
                    // SAFETY: a text argument's slot: a view of the caller's bytes.
                    unsafe { std::ptr::write_unaligned(at.cast::<Str>(), Str { ptr: p, len }) };
                }
            }
        }
    }
}

/// The slot write for an offered local of type `tp` taking `size` bytes, or `None` where the
/// offer's kind does not fit the slot.
fn slot_write(tp: &Type, size: u32, str_slot: bool, e: &HandoverEntry) -> Option<SlotWrite> {
    let bits = e.bits;
    Some(match (tp.base(), e.kind) {
        (Type::Text(_), 2) => {
            let bytes: &[u8] = if e.p.is_null() {
                &[]
            } else {
                // SAFETY: the unit's text, valid for the hand-over call.
                unsafe { std::slice::from_raw_parts(e.p, e.len) }
            };
            if str_slot {
                SlotWrite::View(e.p, u32::try_from(e.len).ok()?)
            } else {
                SlotWrite::Text(String::from_utf8_lossy(bytes).into_owned())
            }
        }
        (t, 3) if crate::data::is_dbref(t) => SlotWrite::Bytes(dbref_bytes(DbRef {
            store_nr: u16::try_from(e.rs).ok()?,
            rec: e.rr,
            pos: e.rp,
        })),
        (Type::Integer(_) | Type::Character, 0) if size == 8 => {
            SlotWrite::Bytes(bits.to_le_bytes().to_vec())
        }
        (Type::Integer(_) | Type::Character, 0) if size == 4 => {
            SlotWrite::Bytes((bits as i32).to_le_bytes().to_vec())
        }
        (Type::Float, 1) if size == 8 => SlotWrite::Bytes(bits.to_le_bytes().to_vec()),
        (Type::Single, 5) if size == 4 => SlotWrite::Bytes((bits as u32).to_le_bytes().to_vec()),
        (Type::Boolean | Type::Enum(_, false, _), 4) if size == 1 => {
            SlotWrite::Bytes(vec![bits as u8])
        }
        _ => return None,
    })
}

/// A `DbRef` as a slot holds it.
fn dbref_bytes(r: DbRef) -> Vec<u8> {
    let mut b = vec![0u8; std::mem::size_of::<DbRef>()];
    // SAFETY: `b` is exactly one `DbRef` wide.
    unsafe { std::ptr::write_unaligned(b.as_mut_ptr().cast::<DbRef>(), r) };
    b
}
