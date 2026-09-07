// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I66 — Bytecode VM / executor

use super::State;
use super::size_ptr;
use crate::keys::{DbRef, Str};
use crate::ops;
use std::cmp::Ordering;

// @PLN104 — `LOFT_TEXT_TIMELINE`: the stack-frame-`String` analogue of `LOFT_STORES=timeline`.
// A loft text value is a Rust `String` living in the stack-frame store as raw bytes, so its
// OWN heap allocation is invisible to the store timeline / `check_store_leaks` (they track
// loft STORES, not the String's buffer).  This tracks each text buffer's heap capacity by
// pointer so an owned-text return orphaned at frame exit (loft#568) is caught RELIABLY — where
// the ASan `ir_read` suppression is not (its stack-substring match false-pos/negs by depth).
//
// Env `LOFT_TEXT_TIMELINE`: any value → exit SUMMARY; `=timeline` → per-op lifeline too.
// Realloc-safe: a grow snapshots the buffer's `(ptr, capacity)` BEFORE and AFTER the mutation,
// so a moved buffer drops its old ptr (freed by the realloc) and records the new; the capacity
// delta is exact.  `free_text` drops the buffer; any ptr still live at program exit LEAKED.
#[derive(Default)]
struct TextTimeline {
    seq: u64,
    live: std::collections::HashMap<usize, TtBuf>,
    live_bytes: usize,
    peak_bytes: usize,
    allocs: u64,
    frees: u64,
}
struct TtBuf {
    seq: u64,
    fn_nr: Option<u32>,
    content: String,
    cap: usize,
}

/// One ledger for the PROCESS, not per thread: a `par` worker mints and frees its buffers on
/// its own thread (`run_to_return`), and a per-thread ledger summarised on the main thread
/// reported "NO text leak" for a script whose worker had orphaned two (loft#1357).  Locked
/// only when the timeline is armed; off, no hook reaches it.
static TEXT_TL: std::sync::LazyLock<std::sync::Mutex<TextTimeline>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(TextTimeline::default()));
thread_local! {
    static TEXT_TL_ON: bool = std::env::var_os("LOFT_TEXT_TIMELINE").is_some();
    static TEXT_TL_VERBOSE: bool = std::env::var("LOFT_TEXT_TIMELINE").as_deref() == Ok("timeline");
}

#[inline]
fn text_tl_on() -> bool {
    TEXT_TL_ON.with(|b| *b)
}

/// Record a text-buffer GROWTH (an append / format that may have (re)allocated).
/// `before`/`after` are `(ptr, capacity)` snapshots around the mutation.
fn text_tl_grow(fn_nr: Option<u32>, before: (usize, usize), after: (usize, usize), content: &str) {
    if !text_tl_on() {
        return;
    }
    let (bp, bc) = before;
    let (ap, ac) = after;
    {
        let mut t = TEXT_TL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A realloc MOVED the buffer → its old allocation is gone.
        if bc > 0
            && bp != ap
            && let Some(b) = t.live.remove(&bp)
        {
            t.live_bytes -= b.cap;
        }
        if ac > 0 {
            if let Some(old) = t.live.get(&ap).map(|b| b.cap) {
                t.live_bytes = t.live_bytes + ac - old;
                if let Some(b) = t.live.get_mut(&ap) {
                    b.cap = ac;
                    b.content = content.to_string();
                }
            } else {
                let s = t.seq;
                t.seq += 1;
                t.allocs += 1;
                t.live_bytes += ac;
                t.live.insert(
                    ap,
                    TtBuf {
                        seq: s,
                        fn_nr,
                        content: content.to_string(),
                        cap: ac,
                    },
                );
            }
            t.peak_bytes = t.peak_bytes.max(t.live_bytes);
        }
    }
    if ac > 0 && TEXT_TL_VERBOSE.with(|b| *b) {
        eprintln!("[text-tl] grow fn={fn_nr:?} cap={ac} ptr={ap:#x} {content:?}");
    }
}

/// Record a text-buffer FREE (`free_text`). `ptr`/`cap` snapshot the buffer before its clear.
fn text_tl_free(fn_nr: Option<u32>, ptr: usize, cap: usize, content: &str) {
    if !text_tl_on() || cap == 0 {
        return;
    }
    {
        let mut t = TEXT_TL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(b) = t.live.remove(&ptr) {
            t.frees += 1;
            t.live_bytes -= b.cap;
        }
    }
    if TEXT_TL_VERBOSE.with(|b| *b) {
        eprintln!("[text-tl] free fn={fn_nr:?} cap={cap} ptr={ptr:#x} {content:?}");
    }
}

/// Run a formatting op `f` on buffer `s`, recording its growth (a format may (re)allocate,
/// e.g. a wide number into an empty buffer).  `tl_fn` is `Some(fn_nr)` when the timeline is on
/// (captured before the caller's `&mut String` borrow), `None` off — off is a straight call, no
/// overhead.  Lets every `format_*` op hook the timeline with one line.
// The nested Option encodes three real states: `None` = timeline off (no work), `Some(None)`
// = on but no owning fn, `Some(Some(fn))` = on with the fn number — so it is not collapsible.
#[allow(clippy::option_option)]
fn text_tl_fmt(tl_fn: Option<Option<u32>>, s: &mut String, f: impl FnOnce(&mut String)) {
    match tl_fn {
        Some(fn_nr) => {
            let before = (s.as_ptr() as usize, s.capacity());
            f(s);
            text_tl_grow(
                fn_nr,
                before,
                (s.as_ptr() as usize, s.capacity()),
                s.as_str(),
            );
        }
        None => f(s),
    }
}

/// @PLN104 — the exit summary (no-op unless `LOFT_TEXT_TIMELINE`).  Mirrors the store
/// `timeline_summary`: allocs / frees / peak live bytes, then — the load-bearing line — every
/// text buffer still live at exit (an orphaned `String`, the loft#568 leak class).  Called from
/// `State`'s exit path next to the store timeline summary.
pub fn text_timeline_summary() {
    if !text_tl_on() {
        return;
    }
    {
        let t = TEXT_TL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let leaked = t.live.len();
        let verdict = if leaked == 0 {
            "NO text leak (every buffer freed)".to_string()
        } else {
            format!(
                "{leaked} text buffer(s) LEAKED ({} bytes) — orphaned Strings (loft#568 class)",
                t.live_bytes
            )
        };
        eprintln!(
            "[text-timeline] SUMMARY: {} allocs, {} frees, peak {} bytes live — {verdict}",
            t.allocs, t.frees, t.peak_bytes
        );
        let mut leaks: Vec<&TtBuf> = t.live.values().collect();
        leaks.sort_by_key(|b| b.seq);
        for b in leaks {
            eprintln!(
                "[text-timeline]   LEAKED #{} fn={:?} {} bytes {:?}",
                b.seq, b.fn_nr, b.cap, b.content
            );
        }
    }
}

impl State {
    pub fn conv_text_from_null(&mut self) {
        self.put_stack(Str::new(super::STRING_NULL));
    }

    pub fn string_from_code(&mut self) {
        let size = self.code::<u8>();
        unsafe {
            self.set_string(
                i32::from(size),
                self.bytecode.as_ptr().offset(self.code_pos as isize),
            );
        }
        self.code_pos += u32::from(size);
    }

    pub(super) unsafe fn set_string(&mut self, size: i32, off: *const u8) {
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<Str>(self.stack_cur.pos + self.stack_pos);
        let m = self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<Str>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos);
        *m = Str {
            ptr: off,
            len: size as u32,
        };
        self.stack_pos += size_of::<Str>() as u32;
    }

    /// # Panics
    /// Always — `OpConstLongText` is obsolete; long strings now use `OpConstStoreText`.
    #[allow(clippy::unused_self)]
    pub fn string_from_texts(&mut self, _start: i64, _size: i64) {
        panic!("OpConstLongText is obsolete — use OpConstStoreText");
    }

    /// Push a Str pointing into the constant store's string data.
    /// The string was written by `Store::set_str()` during compilation:
    /// length at `get_int(rec, 4)`, bytes at `ptr + rec*8 + 8`.
    pub fn string_from_const_store(&mut self, rec: u32, _pos: u32) {
        let store = &self.database.allocations[crate::database::CONST_STORE as usize];
        let len = store.get_u32_raw(rec, 4) as i32;
        let ptr = unsafe { store.ptr.offset(rec as isize * 8 + 8) };
        unsafe {
            self.set_string(len, ptr);
        }
    }

    #[must_use]
    pub fn string(&mut self) -> Str {
        self.stack_pos -= size_ptr();
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<Str>(self.stack_cur.pos + self.stack_pos);
        *self
            .database
            .store(&self.stack_cur)
            .addr::<Str>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos)
    }

    #[inline]
    pub fn length_character(&mut self) {
        let v_v1 = *self.get_stack::<char>();
        let new_value: i64 = if v_v1 == char::from(0) {
            0
        } else {
            v_v1.to_string().len() as i64
        };
        self.put_stack(new_value);
    }

    /// Append `src` to `dst` where `src` may be BORROWED FROM `dst` itself.
    ///
    /// loft#1062 — `s = s + s` and `s += s` hand the append a [`Str`](crate::keys::Str)
    /// over the destination's own bytes.  `String::push_str` reserves first and copies
    /// second, so the moment the reserve has to MOVE the allocation, the copy reads the
    /// block it just freed: SIGSEGV on `--interpret` from 128 KiB up, where `--native`
    /// answers.  Below that there is spare capacity, nothing moves, and the same program
    /// is fine — which is why it took a size to show at all.
    ///
    /// The borrow checker cannot see it: `Str` carries a raw pointer, so `&mut String`
    /// and a `&str` into it coexist happily at compile time.  The overlap test is one
    /// address compare on the append path; the copy happens only when the two really are
    /// one buffer.
    /// Materialise the append SOURCE when it lies inside the destination's allocation, and
    /// answer `None` when it does not.
    ///
    /// `s = s + s` reaches the interpreter as `OpAppendText(s, s)`: the source `Str` and the
    /// destination slot name ONE buffer, and growing the destination frees the bytes the
    /// source still points at.  Copying is the cure, but WHEN it happens is the rule — the
    /// copy has to be taken while the only live handle on those bytes is the shared one.
    /// Taking `&mut String` to the destination first and copying through it hands the same
    /// bytes to a `&mut` and a `&str` at once, which is exactly what `&mut` promises cannot
    /// happen; a release build is entitled to act on the promise, and does (loft#1216 —
    /// 30/30 SIGSEGV in `--release` against loft#1062's own fixture, clean in `--debug`, and
    /// clean again the moment an added `eprintln` moved the allocations).
    ///
    /// So the caller asks with the destination's raw address and capacity, taken and dropped
    /// before this runs, and only re-borrows once the answer is owned.  Sized on CAPACITY, not
    /// length: the source can point into the destination's spare tail.
    fn aliased_source(dst_at: usize, dst_cap: usize, src: &str) -> Option<String> {
        let src_at = src.as_ptr() as usize;
        (src_at >= dst_at && src_at < dst_at + dst_cap).then(|| src.to_owned())
    }

    #[inline]
    pub fn append_text(&mut self) {
        let text = self.string();
        let pos = self.code::<u16>();
        if cfg!(debug_assertions) {
            self.text_positions
                .insert(self.stack_cur.pos + self.stack_pos + size_ptr() - u32::from(pos));
        }
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let off = pos - size_ptr() as u16;
        // The destination's address and capacity, taken through a borrow that ENDS here — see
        // `aliased_source` for why the copy must not be taken through a live `&mut`.
        // @PLN25 slice (c): a null `text?` accumulator (the STRING_NULL sentinel) IGNORES the
        // append, so `s += x` on a null `s` stays null (propagate — nulls stay visible, never
        // swept into "\0x"). A non-null text is never the sentinel, so this never fires for it.
        // DN1-gated so gate-OFF (the legacy nullable model, where a plain text can still be
        // STRING_NULL) keeps its byte-identical append behaviour.
        let (dst_at, dst_cap, is_null) = {
            let d = self.string_mut(off);
            (
                d.as_ptr() as usize,
                d.capacity(),
                crate::keys::pln25_dn1_enabled() && d.as_str() == super::STRING_NULL,
            )
        };
        if is_null {
            return;
        }
        let v1 = self.string_mut(off);
        let owned = Self::aliased_source(dst_at, dst_cap, text.str());
        let before = (v1.as_ptr() as usize, v1.capacity());
        match &owned {
            Some(o) => v1.push_str(o),
            None => v1.push_str(text.str()),
        }
        if let Some(fn_nr) = tl_fn {
            text_tl_grow(
                fn_nr,
                before,
                (v1.as_ptr() as usize, v1.capacity()),
                v1.as_str(),
            );
        }
    }

    /// `OpCreateStack`: push a `DbRef` pointing into the current stack frame.
    /// Used as null-state for borrowed references; must be overwritten by `OpPutRef`
    /// before any field access.
    #[inline]
    pub fn create_stack(&mut self) {
        let pos = self.code::<u16>();
        let db = DbRef {
            store_nr: self.stack_cur.store_nr,
            rec: self.stack_cur.rec,
            pos: self.stack_cur.pos + self.stack_pos - u32::from(pos),
        };
        self.put_stack(db);
    }

    #[inline]
    pub fn get_stack_text(&mut self) {
        let r = *self.get_stack::<DbRef>();
        let t: &str = self.database.store(&r).addr::<String>(r.rec, r.pos);
        self.put_stack(Str::new(t));
    }

    #[inline]
    pub fn get_stack_ref(&mut self) {
        let fld = self.code::<u16>();
        let r = *self.get_stack::<DbRef>();
        let t = self
            .database
            .store(&r)
            .addr::<DbRef>(r.rec, r.pos + u32::from(fld));
        self.put_stack(*t);
    }

    #[inline]
    pub fn set_stack_ref(&mut self) {
        let v1 = *self.get_stack::<DbRef>();
        let r = *self.get_stack::<DbRef>();
        let t = self.database.store_mut(&r).addr_mut::<DbRef>(r.rec, r.pos);
        *t = v1;
    }

    pub fn append_stack_text(&mut self) {
        let text = self.string();
        let pos = self.code::<u16>();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let off = pos - size_ptr() as u16;
        // Same order as `append_text`: decide and copy before the `&mut` exists.
        let (dst_at, dst_cap) = {
            let d = self.string_ref_mut(off);
            (d.as_ptr() as usize, d.capacity())
        };
        let owned = Self::aliased_source(dst_at, dst_cap, text.str());
        let v1 = self.string_ref_mut(off);
        let before = (v1.as_ptr() as usize, v1.capacity());
        match &owned {
            Some(o) => v1.push_str(o),
            None => v1.push_str(text.str()),
        }
        if let Some(fn_nr) = tl_fn {
            text_tl_grow(
                fn_nr,
                before,
                (v1.as_ptr() as usize, v1.capacity()),
                v1.as_str(),
            );
        }
    }

    pub fn append_stack_character(&mut self) {
        let pos = self.code::<u16>();
        let c = *self.get_stack::<char>();
        if c as u32 != 0 {
            // @PLAN53 cluster 2 / S4: the char pop occupies a stepped span
            // (4 off, 8 aligned) — N = bytes the op's get_stack popped.
            let off = pos - self.stack_step(4) as u16;
            self.string_ref_mut(off).push(c);
        }
    }

    pub fn clear_stack_text(&mut self) {
        let pos = self.code::<u16>();
        let v1 = self.string_ref_mut(pos);
        v1.clear();
    }

    #[inline]
    pub fn append_character(&mut self) {
        let pos = self.code::<u16>();
        let c = *self.get_stack::<char>();
        // @PLAN53 cluster 2 / S4: stepped char-pop span (4 off, 8 aligned).
        let n = self.stack_step(4) as u16;
        // A null character renders as NOTHING — `@FR-F-Render` states that exception for
        // every position, and states why: iterating text past its end must append no
        // garbage.  So the fault tag is dropped here rather than rendered.
        //
        // This op used to render `null(<tag>)` for a tagged null character, to show what
        // produced the missing character instead of an empty space.  That is a defensible
        // thing to want, but it was never true of the language: only the INTERPRETER did
        // it, so `"hi"[9]` read `null(oob)` here and empty under `--native`, and a
        // disagreement between the backends is by definition a bug in whichever one
        // disobeys (`@FR-D-op-1`).  The rule says nothing renders, so this is the side
        // that moves.  Reversing it — carrying the cause on a character hole, on BOTH
        // backends — is a change to F-Render and the owner's call, not this op's.
        //
        // The tag is still TAKEN, so it cannot leak into a later hole in the same string.
        let _ = ops::take_format_fault();
        if c as u32 == 0 {
            return;
        }
        self.string_mut(pos - n).push(c);
    }

    #[inline]
    pub fn text_compare(&mut self) {
        let v2 = *self.get_stack::<char>();
        let v1 = *self.get_stack::<Str>();
        let mut ch = v1.str().chars();
        self.put_stack(if let Some(f_ch) = ch.next() {
            let res = f_ch.cmp(&v2);
            if res == Ordering::Less {
                -1
            } else {
                i32::from(
                    res == Ordering::Greater || (res == Ordering::Equal && ch.next().is_some()),
                )
            }
        } else {
            -1
        });
    }

    #[must_use]
    pub fn lines_text<'b>(val: &'b str, at: &mut i32) -> &'b str {
        if let Some(to) = val[*at as usize..].find('\n') {
            let r = &val[*at as usize..to];
            *at = to as i32 + 1;
            r
        } else {
            *at = i32::MIN;
            ""
        }
    }

    #[must_use]
    pub fn split_text<'b>(val: &'b str, on: &str, at: &mut i32) -> &'b str {
        if on.is_empty() {
            *at = i32::MIN;
            return "";
        }
        if let Some(to) = val[*at as usize..].find(on) {
            let r = &val[*at as usize..to];
            *at = (to + on.len()) as i32;
            r
        } else {
            *at = i32::MIN;
            ""
        }
    }

    #[inline]
    pub fn get_text_sub(&mut self) {
        let mut till = *self.get_stack::<i64>() as i32;
        let mut from = *self.get_stack::<i64>() as i32;
        let v1 = self.string();
        // @FR-Slice-Value — a negative bound is `size + bound`, floored at the start:
        // the same rule the vector slice, `s[-1]` and `char_slice` follow, applied to the
        // FROM end as it already is to the TILL end below.  A byte index is what a text
        // slice takes, so the count is from the end in BYTES, and the boundary snap under
        // this normalises whatever character it lands inside.
        if from < 0 {
            from = (from + v1.len as i32).max(0);
        }
        if from >= v1.len as i32 {
            self.put_stack(Str {
                ptr: v1.ptr,
                len: 0,
            });
            return;
        }
        let mut b = v1.str().as_bytes()[from as usize];
        while b & 0xC0 == 0x80 && from > 0 {
            from -= 1;
            b = v1.str().as_bytes()[from as usize];
        }
        if till == i32::MIN {
            let b = v1.str().as_bytes()[from as usize];
            let ch = if b & 0xE0 == 0xC0 {
                from as u32 + 2
            } else if b & 0xF0 == 0xE0 {
                from as u32 + 3
            } else if b & 0xF0 == 0xF0 {
                from as u32 + 4
            } else {
                from as u32 + 1
            };
            let res = unsafe {
                Str {
                    ptr: v1.ptr.offset(from as isize),
                    len: ch - from as u32,
                }
            };
            self.put_stack(res);
            return;
        }
        if till < 0 {
            till += v1.len as i32;
        }
        let mut len = till - from;
        if len <= 0 {
            self.put_stack(Str {
                ptr: v1.ptr,
                len: 0,
            });
            return;
        }
        if len + from > v1.len as i32 {
            len = v1.len as i32 - from;
        } else if till < v1.len as i32 {
            // Snap `till` FORWARD past continuation bytes so the slice ends on a
            // character boundary.  loft#749 — the bound must be tested against the
            // index about to be READ: the old loop checked `t < len` before the
            // increment and then read `t + 1`, so a slice ending one byte into the
            // string's LAST character walked off the end and panicked
            // ("index out of bounds: the len is 5 but the index is 5").  That is
            // `s[i..s.len()]` on any text whose tail is multi-byte, and it is
            // also what took `split_text` down when a multi-byte character
            // followed the final separator.
            let bytes = v1.str().as_bytes();
            let mut t = till as usize;
            while t < bytes.len() && bytes[t] & 0xC0 == 0x80 {
                t += 1;
                len += 1;
            }
        }
        unsafe {
            self.put_stack(Str {
                ptr: v1.ptr.offset(from as isize),
                len: len as u32,
            });
        }
    }

    pub fn clear_text(&mut self) {
        let pos = self.code::<u16>();
        self.string_mut(pos).clear();
    }

    /// #629 follow-up — free the entry fn's hidden TEXT return buffer, addressed by
    /// its ABSOLUTE stack offset.
    ///
    /// [`free_text`](Self::free_text) takes a bytecode operand measured BACKWARDS
    /// from the current `stack_pos`; the entry's buffers are reserved before the
    /// frame exists and are therefore held as absolute offsets, so they cannot go
    /// through it (passing one walks off the stack store and segfaults on teardown).
    /// Same deallocation, and it reports the free to the text timeline so
    /// `LOFT_TEXT_TIMELINE`'s ledger does not read this as an orphan.
    pub(super) fn free_text_at(&mut self, slot: u32) {
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let rec = self.stack_cur.rec;
        let at = self.stack_cur.pos + slot;
        let s = self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<String>(rec, at);
        if let Some(fn_nr) = tl_fn {
            text_tl_free(fn_nr, s.as_ptr() as usize, s.capacity(), s.as_str());
        }
        s.clear();
        s.shrink_to(0);
    }

    /**
    Free the content of a text variable.
    # Panics
    When the same variable is freed twice.
    */
    pub fn free_text(&mut self) {
        let pos = self.code::<u16>();
        // @PLAN53 cluster 5: deallocate the String's heap buffer.  `clear()` sets
        // len=0 so the following `shrink_to(0)` drops capacity to 0 and frees the
        // buffer.  Previously `clear()` ran ONLY under debug-assertions, so in
        // release — and under Miri, where the lib disables debug-assertions — a
        // non-empty String reached `shrink_to(0)`, which shrinks capacity only down
        // to `len` (never below), leaving the buffer allocated and LEAKED (the store
        // holds the String as raw bytes and never runs its Drop; `free_text` is the
        // sole deallocation point).  Miri's leak checker caught it (cluster 5).
        // (The old debug-only `for _ in 0..s.len()` poison loop was dead code — it
        // ran after `clear()`, so `s.len()` was already 0 and it never executed.)
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_mut(pos);
        if let Some(fn_nr) = tl_fn {
            text_tl_free(fn_nr, s.as_ptr() as usize, s.capacity(), s.as_str());
        }
        s.clear();
        s.shrink_to(0);
        if cfg!(debug_assertions) {
            let var_pos = self.stack_cur.pos + self.stack_pos - u32::from(pos);
            let remove = self.text_positions.remove(&var_pos);
            assert!(remove, "double free");
        }
    }

    /** Get a string reference from a variety of internal string formats.
    # Panics
    When an unknown internal string format is found.
    */
    pub fn string_mut(&mut self, pos: u16) -> &mut String {
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<String>(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        self.database.store_mut(&self.stack_cur).addr_mut::<String>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub(super) fn string_ref_mut(&mut self, pos: u16) -> &mut String {
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<DbRef>(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        let r = *self.database.store(&self.stack_cur).addr::<DbRef>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        );
        self.database
            .store_mut(&self.stack_cur)
            .addr_mut::<String>(r.rec, r.pos)
    }

    /// Plan-04 Phase 2h: positional variant of the removed `text()`.  Writes a
    /// fresh empty `String` at the frame slot reached by
    /// `self.stack_pos - pos` (matches the addressing convention of
    /// `clear_text`, `append_text`, and the other `_text` ops).
    /// Does NOT advance `stack_pos` — the frame is assumed to be
    /// already reserved.
    ///
    /// `pos` is read from the bytecode stream as a `const u16`.
    pub fn init_text(&mut self) {
        let pos = self.code::<u16>();
        if cfg!(debug_assertions) {
            self.text_positions
                .insert(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        }
        let v = self.string_mut(pos);
        let s = String::new();
        unsafe {
            core::ptr::write(v, s);
        }
    }

    pub fn var_text(&mut self) {
        let pos = self.code::<u16>();
        let new_value = Str::new(self.get_var::<String>(pos));
        self.put_stack(new_value);
    }

    pub fn arg_text(&mut self) {
        let pos = self.code::<u16>();
        let new_value = *self.get_var::<Str>(pos);
        self.put_stack(new_value);
    }

    pub fn put_text(&mut self) {
        let pos = self.code::<u16>();
        let str_val = self.string(); // pops 16B Str; stack_pos -= 16
        self.put_var(pos, str_val); // writes Str at (stack_pos + 16 - pos) = elem_abs
    }

    pub fn format_int(&mut self) {
        let pos = self.code::<u16>();
        let radix = self.code::<u8>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let note = self.code::<bool>();
        let dir = self.code::<i8>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<i64>();
        // Plan-07 phase 4e.3 — consume the fault tag set by the
        // 4e.1 format-scope swap's preceding `OpTagFault`.  When the
        // value is the i64 null sentinel AND a tag is set, render
        // `null(<tag>)` instead of bare `null`.  Take + drop the
        // tag unconditionally so it can never leak to a downstream
        // interpolation in the same format string.
        let tag = ops::take_format_fault();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_mut(pos - 16);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_long_with_tag(s, val, tag, radix, width, token, plus, note, dir)
        });
    }

    pub fn format_stack_int(&mut self) {
        let pos = self.code::<u16>();
        let radix = self.code::<u8>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let note = self.code::<bool>();
        let dir = self.code::<i8>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<i64>();
        let tag = ops::take_format_fault();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_ref_mut(pos - 16);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_long_with_tag(s, val, tag, radix, width, token, plus, note, dir)
        });
    }

    pub fn format_float(&mut self) {
        let pos = self.code::<u16>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f64>();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_mut(pos - 24);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_float(s, val, width, precision, token, plus, dir)
        });
    }

    pub fn format_stack_float(&mut self) {
        let pos = self.code::<u16>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f64>();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_ref_mut(pos - 24); // f64(8)+i64(8)+i64(8) = 24 bytes popped
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_float(s, val, width, precision, token, plus, dir)
        });
    }

    pub fn format_single(&mut self) {
        let pos = self.code::<u16>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f32>();
        // @PLAN53 cluster 2 / S4: N = stepped span of the popped i64+i64+f32
        // (20 off; 24 aligned — the f32 rounds 4->8).
        let n = (self.stack_step(8) + self.stack_step(8) + self.stack_step(4)) as u16;
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_mut(pos - n);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_single(s, val, width, precision, token, plus, dir)
        });
    }

    pub fn format_stack_single(&mut self) {
        let pos = self.code::<u16>();
        let token = self.code::<u8>();
        let plus = self.code::<bool>();
        let dir = self.code::<i8>();
        let precision = *self.get_stack::<i64>();
        let width = *self.get_stack::<i64>();
        let val = *self.get_stack::<f32>();
        // @PLAN53 cluster 2 / S4: stepped span of popped i64+i64+f32 (20/24).
        let n = (self.stack_step(8) + self.stack_step(8) + self.stack_step(4)) as u16;
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_ref_mut(pos - n);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_single(s, val, width, precision, token, plus, dir)
        });
    }

    pub fn format_text(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let token = self.code::<u8>();
        let width = *self.get_stack::<i64>();
        let val = self.string();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_mut(pos - 8 - size_ptr() as u16);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_text(s, val.str(), width, dir, token)
        });
    }

    pub fn format_stack_text(&mut self) {
        let pos = self.code::<u16>();
        let dir = self.code::<i8>();
        let token = self.code::<u8>();
        let width = *self.get_stack::<i64>();
        let val = self.string();
        let tl_fn = text_tl_on().then(|| self.call_stack.last().map(|f| f.d_nr));
        let s = self.string_ref_mut(pos - 8 - size_ptr() as u16);
        text_tl_fmt(tl_fn, s, |s| {
            ops::format_text(s, val.str(), width, dir, token)
        });
    }
}
