// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! # loft-ffi
//!
//! Helpers for writing loft native extension cdylibs.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use loft_ffi::{LoftStr, ret, text};
//!
//! #[unsafe(no_mangle)]
//! pub extern "C" fn n_greet(name_ptr: *const u8, name_len: usize) -> LoftStr {
//!     let name = unsafe { text(name_ptr, name_len) };
//!     ret(format!("Hello, {name}!"))
//! }
//! ```

use std::cell::RefCell;

// ── Null sentinels ─────────────────────────────────────────────────────

/// Null sentinel for `integer` (loft `i32`).
pub const NULL_INT: i32 = i32::MIN;

/// Null sentinel for `long` (loft `i64`).
pub const NULL_LONG: i64 = i64::MIN;

// ── LoftRef: opaque store reference ─────────────────────────────────────

/// Opaque reference to a loft store object (struct, vector, collection).
///
/// The cdylib receives this as an opaque handle.  It cannot dereference
/// the fields — only the interpreter can.  Pass it back to other loft
/// native functions unchanged, or check [`is_null`](LoftRef::is_null).
///
/// Layout matches `DbRef` in the interpreter (`u16 + u32 + u32`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LoftRef {
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}

impl LoftRef {
    /// A null reference (no object).
    pub const NULL: Self = Self {
        store_nr: 0,
        rec: 0,
        pos: 0,
    };

    /// Returns `true` if this reference points to nothing.
    #[must_use]
    pub fn is_null(&self) -> bool {
        self.rec == 0 && self.pos == 0
    }
}

// SAFETY: LoftRef is a plain-old-data handle.  The pointed-to store
// data is only accessed by the interpreter on the calling thread.
unsafe impl Send for LoftRef {}
unsafe impl Sync for LoftRef {}

// ── LoftStore: direct field access to store memory ─────────────────────

/// Handle to a loft store's contiguous memory buffer.
///
/// Provides direct read/write access to struct fields via pointer
/// arithmetic.  The cdylib receives this as the first C-ABI argument
/// when any parameter is a `LoftRef`.
///
/// # Safety contract
///
/// - Reads and writes to **existing** records are safe for the duration
///   of the C-ABI call (the interpreter does not reallocate while the
///   cdylib is running).
/// - The pointer becomes invalid after the call returns — do not cache it.
/// - Field offsets are stable for the process lifetime (computed once at
///   parse time).
/// Opaque context pointer passed to callback functions.
/// The cdylib must not dereference or inspect this — just pass it through.
/// Its low 16 bits are the store number ([`LoftStore::store_nr`] reads them);
/// the bits above belong to the host.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftStoreCtx {
    pub _opaque: *mut (),
}

/// Frees a block a cdylib handed to the host through
/// [`LoftStore::foreign_vector_from_owned`]: `(ptr, len, cap)` are the `Vec<u8>`'s
/// parts.  Called ONCE, by the host, when the last loft handle over the bytes is
/// freed — in the cdylib's own allocator, since the host's may differ.
pub type LoftForeignRelease = unsafe extern "C" fn(*mut u8, usize, usize);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftStore {
    /// Base pointer to the store's memory buffer.
    pub ptr: *mut u8,
    /// Store capacity in 8-byte words (for bounds checking).
    pub size: u32,
    /// Opaque context for callbacks (interpreter-side `&mut Stores` + store_nr).
    pub ctx: LoftStoreCtx,
    /// Allocate `words` 8-byte words in the store. Returns the new record number.
    /// **After calling this, `ptr` may be stale — call `reload()` immediately.**
    pub claim_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32) -> u32>,
    /// Refresh `ptr` and `size` after an allocation that may have reallocated.
    pub reload_fn: Option<unsafe extern "C" fn(LoftStoreCtx, *mut *mut u8, *mut u32)>,
    /// Resize record `rec` to `words` 8-byte words. Returns the (possibly new) record number.
    /// **After calling this, `ptr` may be stale — call `reload()` immediately.**
    pub resize_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32, u32) -> u32>,
    /// Adopt a block of bytes the cdylib owns as a READ-ONLY `vector<u8>` with no
    /// copy: `(ctx, ptr, len, cap, elem_size, release)`.  The host keeps the block
    /// until the last loft handle over it is freed, then calls `release(ptr, len,
    /// cap)`.  Answers the null ref when the host did not take the block (the
    /// caller still owns it).  Reached through
    /// [`foreign_vector_from_owned`](LoftStore::foreign_vector_from_owned).
    ///
    /// The LAST field, on purpose: an older cdylib reads the fields before it and
    /// nothing after, and every host-to-cdylib crossing is the one fixed
    /// [`LoftBridgeFn`] shape, so a longer struct shifts no other argument.  A
    /// cdylib that CALLS it must run on a host that fills it — declare the loft
    /// floor that carries loft-ffi 0.1.2 in `loft.toml`.
    pub foreign_fn: Option<
        unsafe extern "C" fn(
            LoftStoreCtx,
            *mut u8,
            usize,
            usize,
            u32,
            LoftForeignRelease,
        ) -> LoftRef,
    >,
}

// SAFETY: The store is only accessed from the interpreter's thread
// during the C-ABI call.
unsafe impl Send for LoftStore {}
unsafe impl Sync for LoftStore {}

impl LoftStore {
    /// Read an `i32` field (loft `integer`).
    ///
    /// # Safety
    /// `rec`, `pos`, `offset` must point to a valid i32 within the store.
    #[inline]
    pub unsafe fn get_int(&self, rec: u32, pos: u32, offset: u16) -> i32 {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i32>()
                .read_unaligned()
        }
    }

    /// Write an `i32` field.
    ///
    /// # Safety
    /// Same as `get_int`.
    #[inline]
    pub unsafe fn set_int(&self, rec: u32, pos: u32, offset: u16, val: i32) {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i32>()
                .write_unaligned(val);
        }
    }

    /// Read an `i64` field (loft `long`).
    #[inline]
    pub unsafe fn get_long(&self, rec: u32, pos: u32, offset: u16) -> i64 {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i64>()
                .read_unaligned()
        }
    }

    /// Write an `i64` field.
    #[inline]
    pub unsafe fn set_long(&self, rec: u32, pos: u32, offset: u16, val: i64) {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<i64>()
                .write_unaligned(val);
        }
    }

    /// Read an `f64` field (loft `float`).
    #[inline]
    pub unsafe fn get_float(&self, rec: u32, pos: u32, offset: u16) -> f64 {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<f64>()
                .read_unaligned()
        }
    }

    /// Write an `f64` field.
    #[inline]
    pub unsafe fn set_float(&self, rec: u32, pos: u32, offset: u16, val: f64) {
        unsafe {
            self.ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
                .cast::<f64>()
                .write_unaligned(val);
        }
    }

    /// Read a `u8` field (loft `boolean`, simple enum tag).
    #[inline]
    pub unsafe fn get_byte(&self, rec: u32, pos: u32, offset: u16) -> u8 {
        unsafe {
            *self
                .ptr
                .add(rec as usize * 8 + pos as usize + offset as usize)
        }
    }

    /// Write a `u8` field.
    #[inline]
    pub unsafe fn set_byte(&self, rec: u32, pos: u32, offset: u16, val: u8) {
        unsafe {
            *self
                .ptr
                .add(rec as usize * 8 + pos as usize + offset as usize) = val;
        }
    }

    /// Read a text field.  Returns `(ptr, len)` pointing into store memory.
    /// The pointer is valid until the C-ABI call returns.
    /// Returns `(null, 0)` for null text references.
    #[inline]
    pub unsafe fn get_text(&self, rec: u32, pos: u32, offset: u16) -> (*const u8, usize) {
        let str_rec = unsafe { self.get_int(rec, pos, offset) } as u32;
        if str_rec == 0 {
            return (std::ptr::null(), 0);
        }
        let len = unsafe { self.get_int(str_rec, 0, 4) } as usize;
        let ptr = unsafe { self.ptr.add(str_rec as usize * 8 + 8) };
        (ptr, len)
    }

    /// Read a sub-reference field (struct field that is itself a `LoftRef`).
    /// The returned ref's `store_nr` is copied from the parent.
    #[inline]
    pub unsafe fn get_ref(&self, store_nr: u16, rec: u32, pos: u32, offset: u16) -> LoftRef {
        let sub_rec = unsafe { self.get_int(rec, pos, offset) } as u32;
        LoftRef {
            store_nr,
            rec: sub_rec,
            pos: 8,
        }
    }

    // ── Allocation helpers ────────────────────────────────────────────

    /// The store number this handle operates on (encoded in the context).
    #[inline]
    #[must_use]
    pub fn store_nr(&self) -> u16 {
        self.ctx._opaque as usize as u16
    }

    /// Refresh `ptr` and `size` from the interpreter after a potential reallocation.
    ///
    /// Call this after every `claim()` or `resize()` — the raw pointer may
    /// have moved due to store growth.
    ///
    /// # Panics
    /// Panics if `reload_fn` is not set (store was created without callbacks).
    #[inline]
    pub unsafe fn reload(&mut self) {
        let f = self.reload_fn.expect("LoftStore: reload_fn not set");
        unsafe { f(self.ctx, &mut self.ptr, &mut self.size) };
    }

    /// Allocate `words` 8-byte words in the store. Returns the new record number.
    ///
    /// **Automatically reloads `ptr`/`size`** — safe to read/write immediately after.
    ///
    /// # Panics
    /// Panics if `claim_fn` is not set.
    pub unsafe fn claim(&mut self, words: u32) -> u32 {
        let f = self.claim_fn.expect("LoftStore: claim_fn not set");
        let rec = unsafe { f(self.ctx, words) };
        unsafe { self.reload() };
        rec
    }

    /// Resize record `rec` to `words` 8-byte words. Returns the (possibly new)
    /// record number — the record may have been relocated.
    ///
    /// **Automatically reloads `ptr`/`size`** — safe to read/write immediately after.
    ///
    /// # Panics
    /// Panics if `resize_fn` is not set.
    pub unsafe fn resize(&mut self, rec: u32, words: u32) -> u32 {
        let f = self.resize_fn.expect("LoftStore: resize_fn not set");
        let new_rec = unsafe { f(self.ctx, rec, words) };
        unsafe { self.reload() };
        new_rec
    }

    /// Allocate an empty struct record of `words` 8-byte words.
    ///
    /// Returns a `LoftRef` pointing to the start of the data area (pos = 8,
    /// skipping the record header).
    pub unsafe fn alloc_record(&mut self, words: u32) -> LoftRef {
        let rec = unsafe { self.claim(words) };
        LoftRef {
            store_nr: self.store_nr(),
            rec,
            pos: 8,
        }
    }

    /// Allocate an empty vector with space for `capacity` elements.
    ///
    /// `elem_size` is the element size in bytes (4 for integer/single,
    /// 8 for long/float, or the struct record-ref size of 4).
    ///
    /// The vector starts with length 0. Use `vector_push_*` to append.
    /// The minimum allocation is 11 elements (matching interpreter convention).
    pub unsafe fn alloc_vector(&mut self, elem_size: u32, capacity: u32) -> LoftRef {
        let alloc_count = capacity.max(11);
        let words = (alloc_count * elem_size + 15) / 8;
        let vec_rec = unsafe { self.claim(words) };
        // Initialize length = 0 (at byte offset 4 within the record).
        unsafe { self.set_int(vec_rec, 0, 4, 0) };
        LoftRef {
            store_nr: self.store_nr(),
            rec: vec_rec,
            pos: 8,
        }
    }

    /// Current number of elements in a vector.
    ///
    /// # Safety
    /// `vec` must point to a valid vector record in this store.
    #[inline]
    pub unsafe fn vector_len(&self, vec: &LoftRef) -> u32 {
        unsafe { self.get_int(vec.rec, 0, 4) as u32 }
    }

    /// Raw pointer to the first element of a vector's data area.
    ///
    /// The data starts 8 bytes into the vector record (after the header + length).
    /// Valid for `vector_len(vec) * elem_size` bytes. The pointer is only valid
    /// for the duration of the C-ABI call and becomes stale if the store reallocates.
    ///
    /// # Safety
    /// `vec` must point to a valid vector record in this store.
    #[inline]
    pub unsafe fn vector_data_ptr(&self, vec: &LoftRef) -> *const u8 {
        unsafe { self.ptr.add(vec.rec as usize * 8 + 8) }
    }

    /// Ensure the vector has room for one more element, resizing if needed.
    /// Returns the (possibly updated) `vec_rec` — the record may have moved.
    unsafe fn vector_grow(&mut self, vec_rec: u32, elem_size: u32) -> u32 {
        let length = unsafe { self.get_int(vec_rec, 0, 4) as u32 };
        let needed_words = ((length + 1) * elem_size + 15) / 8;
        let new_rec = unsafe { self.resize(vec_rec, needed_words) };
        // resize() already called reload(), ptr is fresh
        new_rec
    }

    /// Append an `i32` to a vector of integers.
    ///
    /// `vec` is updated in place if the record moves during resize.
    ///
    /// # Safety
    /// `vec` must point to a valid `vector<integer>` in this store.
    pub unsafe fn vector_push_int(&mut self, vec: &mut LoftRef, val: i32) {
        let new_rec = unsafe { self.vector_grow(vec.rec, 4) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        // Write value at data offset: 8 + length * 4
        unsafe { self.set_int(new_rec, 8 + length * 4, 0, val) };
        // Increment length
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
    }

    /// Append an `i64` to a vector of longs.
    ///
    /// `vec` is updated in place if the record moves during resize.
    ///
    /// # Safety
    /// `vec` must point to a valid `vector<long>` in this store.
    pub unsafe fn vector_push_long(&mut self, vec: &mut LoftRef, val: i64) {
        let new_rec = unsafe { self.vector_grow(vec.rec, 8) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        unsafe { self.set_long(new_rec, 8 + length * 8, 0, val) };
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
    }

    /// Append an `f64` to a vector of floats.
    ///
    /// `vec` is updated in place if the record moves during resize.
    ///
    /// # Safety
    /// `vec` must point to a valid `vector<float>` in this store.
    pub unsafe fn vector_push_float(&mut self, vec: &mut LoftRef, val: f64) {
        let new_rec = unsafe { self.vector_grow(vec.rec, 8) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        unsafe { self.set_float(new_rec, 8 + length * 8, 0, val) };
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
    }

    /// Append one element of `elem_size` bytes to a vector.
    ///
    /// Returns the byte position of the new element within the vector record.
    /// The caller writes the element's fields at `(vec.rec, returned_pos, field_offset)`.
    ///
    /// # Safety
    /// `vec` must point to a valid vector in this store.
    pub unsafe fn vector_push(&mut self, vec: &mut LoftRef, elem_size: u32) -> u32 {
        let new_rec = unsafe { self.vector_grow(vec.rec, elem_size) };
        vec.rec = new_rec;
        let length = unsafe { self.get_int(new_rec, 0, 4) as u32 };
        let elem_pos = 8 + length * elem_size;
        unsafe { self.set_int(new_rec, 0, 4, length as i32 + 1) };
        elem_pos
    }

    // ── Bulk data helpers ─────────────────────────────────────────────

    /// Copy raw bytes into store memory at a specific position.
    ///
    /// # Safety
    /// The destination `rec * 8 + pos` must be within the store's allocated area.
    /// `src` must point to `len` readable bytes.
    #[inline]
    pub unsafe fn write_bytes(&self, rec: u32, pos: u32, src: *const u8, len: usize) {
        let dst = unsafe { self.ptr.add(rec as usize * 8 + pos as usize) };
        unsafe { std::ptr::copy_nonoverlapping(src, dst, len) };
    }

    /// Allocate a vector and fill it with raw byte data.
    ///
    /// Creates a vector of `count` elements (each `elem_size` bytes) and copies
    /// `data` directly into the vector's data area. The data length must be
    /// exactly `count * elem_size`.
    ///
    /// Returns a `LoftRef` to the vector record.
    ///
    /// # Safety
    /// `data` must point to `count * elem_size` readable bytes.
    pub unsafe fn alloc_vector_from_bytes(
        &mut self,
        elem_size: u32,
        count: u32,
        data: *const u8,
        data_len: usize,
    ) -> LoftRef {
        let mut vec = unsafe { self.alloc_vector(elem_size, count) };
        // Set length to count (alloc_vector starts at 0).
        unsafe { self.set_int(vec.rec, 0, 4, count as i32) };
        // Bulk copy data after the 8-byte vector header.
        if data_len > 0 {
            unsafe { self.write_bytes(vec.rec, 8, data, data_len) };
        }
        vec.pos = 8;
        vec
    }

    /// Hand `data` to loft as a `vector<u8>` WITHOUT copying it: the host serves the
    /// block through a read-only foreign store and frees it — through
    /// [`release_vec`], in this cdylib's allocator — when the last loft handle over
    /// it is freed.  The result is what [`alloc_vector_from_bytes`] answers for the
    /// same bytes, and loft reads it exactly the same way: length, index, iteration,
    /// slice, `text_from_bytes`.  What differs: a WRITE into it (`v[0] = 1`,
    /// `v += […]`) halts the program with the advice to copy first (`w = v`).  So a
    /// library answers this way only where its contract already says "read the
    /// result" — the fast path for a decoded payload, not for a buffer the caller
    /// is meant to grow.
    ///
    /// Falls back to the copy on a host that has no [`foreign_fn`] or declined the
    /// block, so the bytes always arrive.
    ///
    /// [`alloc_vector_from_bytes`]: LoftStore::alloc_vector_from_bytes
    /// [`foreign_fn`]: LoftStore::foreign_fn
    ///
    /// # Safety
    /// The store handle must be the one the bridge was called with, on the same
    /// thread.
    pub unsafe fn foreign_vector_from_owned(&mut self, data: Vec<u8>) -> LoftRef {
        let mut data = std::mem::ManuallyDrop::new(data);
        if let Some(f) = self.foreign_fn {
            let (ptr, len, cap) = (data.as_mut_ptr(), data.len(), data.capacity());
            let r = unsafe { f(self.ctx, ptr, len, cap, 1, release_vec) };
            if !r.is_null() {
                return r;
            }
        }
        // The host did not take the block: it is still ours, so copy and free it.
        let data = std::mem::ManuallyDrop::into_inner(data);
        let len = data.len();
        unsafe { self.alloc_vector_from_bytes(1, len as u32, data.as_ptr(), len) }
    }

    /// Allocate a text string in the store and set a struct field to point to it.
    ///
    /// The text record layout is: `[header(4)] [length(4)] [utf8-bytes...]`.
    /// The field at `(rec, pos, offset)` is set to the text record number.
    ///
    /// # Safety
    /// `rec`, `pos`, `offset` must point to a valid i32 field in the store.
    pub unsafe fn set_text(&mut self, rec: u32, pos: u32, offset: u16, val: &str) {
        let words = ((val.len() + 15) / 8) as u32;
        let str_rec = unsafe { self.claim(words) };
        // Write string length at str_rec + 4 bytes.
        unsafe { self.set_int(str_rec, 0, 4, val.len() as i32) };
        // Copy UTF-8 bytes starting at str_rec * 8 + 8.
        if !val.is_empty() {
            unsafe { self.write_bytes(str_rec, 8, val.as_ptr(), val.len()) };
        }
        // Set the field to point to the text record.
        unsafe { self.set_int(rec, pos, offset, str_rec as i32) };
    }
}

/// The [`LoftForeignRelease`] for a `Vec<u8>` this crate handed over: rebuilds the
/// vector from its parts and drops it, in this cdylib's allocator.
///
/// # Safety
/// `(ptr, len, cap)` must be the parts of a `Vec<u8>` handed to the host by
/// [`LoftStore::foreign_vector_from_owned`], released exactly once.
pub unsafe extern "C" fn release_vec(ptr: *mut u8, len: usize, cap: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, len, cap) });
}

// ── LoftStr: safe text return ──────────────────────────────────────────

/// A `#[repr(C)]` text value returned from native functions.
///
/// The pointer is borrowed — valid until the next call to [`ret`] on the
/// same thread.  The interpreter copies immediately after the call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl LoftStr {
    /// Empty text (null pointer, zero length).
    pub const EMPTY: Self = Self {
        ptr: std::ptr::null(),
        len: 0,
    };
}

// SAFETY: LoftStr is only used as a return value within a single
// function call scope.  The pointer is never sent across threads.
unsafe impl Send for LoftStr {}
unsafe impl Sync for LoftStr {}

thread_local! {
    static RET_BUF: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Store `s` in a thread-local buffer and return a borrowed view.
///
/// The returned [`LoftStr`] is valid until the next call to `ret` on
/// the same thread.  The interpreter copies the bytes into its own
/// storage immediately after the C-ABI call returns.
///
/// # Example
/// ```rust,ignore
/// use loft_ffi::{ret, LoftStr};
///
/// #[unsafe(no_mangle)]
/// pub extern "C" fn n_hello() -> LoftStr {
///     ret("Hello!".to_string())
/// }
/// ```
#[must_use]
pub fn ret(s: String) -> LoftStr {
    RET_BUF.with(|buf| {
        *buf.borrow_mut() = s;
        let b = buf.borrow();
        LoftStr {
            ptr: b.as_ptr(),
            len: b.len(),
        }
    })
}

/// Return a borrowed view of an existing `&str`.
///
/// Use this when the data already lives in a thread-local or static
/// and doesn't need to be copied into the return buffer.
///
/// # Safety
/// The `&str` must remain valid until the interpreter has copied the
/// bytes (i.e. until the `extern "C"` function returns).
#[must_use]
pub fn ret_ref(s: &str) -> LoftStr {
    LoftStr {
        ptr: s.as_ptr(),
        len: s.len(),
    }
}

// ── LoftValue: uniform FFI transport (plan-25 FFI generated-dispatch, F1) ─
//
// The interpreter already builds a uniform tagged-argument array before
// calling a native function (`ArgVal` in `src/extensions.rs`).  Today that
// array is consumed by a ~98-arm `dispatch_call` that `transmute`s the raw
// fn pointer to a concrete `extern "C"` signature.  Plan-25 replaces that
// match with one uniform bridge per native function (generated by the
// `#[loft_native]` proc-macro, F2): the interpreter marshals the stack into
// `[LoftValue]` and calls a single `LoftBridgeFn` shape; the generated bridge
// decodes each arg to the impl's real typed parameter and calls the real fn.
//
// F1 is purely additive — these types + the registration hook exist but are
// NOT wired into the interpreter yet (the legacy raw-ptr arms still run).

/// Discriminant for [`LoftValue`].  A loft stack cell is always 8 bytes, so a
/// plain `integer` rides as `I64` and the bridge casts to the impl width
/// (`as i32` etc.) — the impl width is the proc-macro's authority, not this
/// tag (see plan-25 § why the macro reads the real Rust signature).  Vectors
/// ride as `Ref` (the dereferenced data record), matching the current marshal.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoftTag {
    Void = 0,
    I64 = 1,
    F64 = 2,
    Bool = 3,
    Text = 4,
    Ref = 5,
}

/// Payload of a [`LoftValue`].  `#[repr(C)]` union over every transported
/// shape; the active field is selected by the [`LoftValue::tag`].  Read only
/// through the typed accessors, which assert the tag in debug builds.
#[repr(C)]
#[derive(Clone, Copy)]
pub union LoftPayload {
    pub i: i64,
    pub f: f64,
    pub b: bool,
    pub text: LoftStr,
    pub r: LoftRef,
    /// Zero filler for `Void`.
    pub void: u64,
}

/// One marshalled argument or return value crossing the interpreter↔cdylib
/// boundary on the `--interpret` path.  `#[repr(C)]` so the generated bridge
/// (compiled in the library crate) and the interpreter agree on layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LoftValue {
    pub tag: LoftTag,
    pub payload: LoftPayload,
}

// SAFETY: LoftValue is plain-old-data; the LoftStr/LoftRef it may carry are
// themselves Send/Sync POD handles only valid for the call's duration.
unsafe impl Send for LoftValue {}
unsafe impl Sync for LoftValue {}

impl LoftValue {
    /// The `Void` value — a function with no return writes this into `*ret`.
    pub const VOID: Self = Self {
        tag: LoftTag::Void,
        payload: LoftPayload { void: 0 },
    };

    /// Wrap a 64-bit integer (a loft `integer`/narrow-int cell).
    #[must_use]
    pub fn int(i: i64) -> Self {
        Self {
            tag: LoftTag::I64,
            payload: LoftPayload { i },
        }
    }

    /// Wrap a 64-bit float (loft `float`; `single` is widened to f64 here).
    #[must_use]
    pub fn float(f: f64) -> Self {
        Self {
            tag: LoftTag::F64,
            payload: LoftPayload { f },
        }
    }

    /// Wrap a boolean.
    #[must_use]
    pub fn boolean(b: bool) -> Self {
        Self {
            tag: LoftTag::Bool,
            payload: LoftPayload { b },
        }
    }

    /// Wrap a borrowed text view.
    #[must_use]
    pub fn text(text: LoftStr) -> Self {
        Self {
            tag: LoftTag::Text,
            payload: LoftPayload { text },
        }
    }

    /// Wrap a store reference (also carries vectors as their data record).
    #[must_use]
    pub fn reference(r: LoftRef) -> Self {
        Self {
            tag: LoftTag::Ref,
            payload: LoftPayload { r },
        }
    }

    /// Read as `i64`.  Debug-asserts the tag is `I64`.
    #[must_use]
    pub fn as_i64(&self) -> i64 {
        debug_assert_eq!(
            self.tag,
            LoftTag::I64,
            "LoftValue::as_i64 on {:?}",
            self.tag
        );
        // SAFETY: tag selects the active union field.
        unsafe { self.payload.i }
    }

    /// Read as `f64`.  Debug-asserts the tag is `F64`.
    #[must_use]
    pub fn as_f64(&self) -> f64 {
        debug_assert_eq!(
            self.tag,
            LoftTag::F64,
            "LoftValue::as_f64 on {:?}",
            self.tag
        );
        // SAFETY: tag selects the active union field.
        unsafe { self.payload.f }
    }

    /// Read as `bool`.  Debug-asserts the tag is `Bool`.
    #[must_use]
    pub fn as_bool(&self) -> bool {
        debug_assert_eq!(
            self.tag,
            LoftTag::Bool,
            "LoftValue::as_bool on {:?}",
            self.tag
        );
        // SAFETY: tag selects the active union field.
        unsafe { self.payload.b }
    }

    /// Read as a borrowed text view.  Debug-asserts the tag is `Text`.
    #[must_use]
    pub fn as_text(&self) -> LoftStr {
        debug_assert_eq!(
            self.tag,
            LoftTag::Text,
            "LoftValue::as_text on {:?}",
            self.tag
        );
        // SAFETY: tag selects the active union field.
        unsafe { self.payload.text }
    }

    /// Read as a store reference.  Debug-asserts the tag is `Ref`.
    #[must_use]
    pub fn as_ref(&self) -> LoftRef {
        debug_assert_eq!(
            self.tag,
            LoftTag::Ref,
            "LoftValue::as_ref on {:?}",
            self.tag
        );
        // SAFETY: tag selects the active union field.
        unsafe { self.payload.r }
    }
}

impl core::fmt::Debug for LoftValue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // SAFETY: each arm reads the union field its tag selects.
        unsafe {
            match self.tag {
                LoftTag::Void => write!(f, "LoftValue::Void"),
                LoftTag::I64 => write!(f, "LoftValue::I64({})", self.payload.i),
                LoftTag::F64 => write!(f, "LoftValue::F64({})", self.payload.f),
                LoftTag::Bool => write!(f, "LoftValue::Bool({})", self.payload.b),
                LoftTag::Text => write!(f, "LoftValue::Text(len={})", self.payload.text.len),
                LoftTag::Ref => {
                    let r = self.payload.r;
                    write!(f, "LoftValue::Ref({},{},{})", r.store_nr, r.rec, r.pos)
                }
            }
        }
    }
}

/// Widen a narrow-int (`i32`) native return to the i64 loft cell, preserving
/// the null sentinel: cdylibs return `i32::MIN` to mean "null", and a naive
/// `i64::from(i32::MIN)` would become a regular negative number, so the
/// loft-side `if !x { … }` null check would miss the failure.  Map
/// `i32::MIN → i64::MIN` so the sentinel survives.  Mirrors the interpreter's
/// `widen_int` in `src/extensions.rs`; the generated bridge calls this for
/// every `i32` return.
#[must_use]
pub fn widen_i32(v: i32) -> i64 {
    if v == i32::MIN {
        i64::MIN
    } else {
        i64::from(v)
    }
}

/// The uniform calling convention every plan-25 bridge exposes.  The
/// interpreter calls only this shape, so it never reconstructs a concrete C
/// signature again.  `store` is for allocating ref/text returns; `args`/`n`
/// is the marshalled argument array (declaration order); the result is
/// written into `*ret` (or [`LoftValue::VOID`] for a `void` fn).
///
/// SAFETY (caller = interpreter): `args` points to `n` initialised
/// `LoftValue`s and `ret` to one writable `LoftValue`, both valid for the
/// call; `store` is valid for the call's duration only.
pub type LoftBridgeFn =
    unsafe extern "C" fn(store: LoftStore, args: *const LoftValue, n: usize, ret: *mut LoftValue);

// ── Text parameter helpers ─────────────────────────────────────────────

/// Convert a `(*const u8, usize)` C-ABI text parameter to `&str`.
///
/// # Safety
/// The caller must ensure `ptr` points to valid UTF-8 of length `len`.
/// This is guaranteed by the loft interpreter for all `text` arguments.
#[must_use]
pub unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
}

/// Convert a `(*const u8, usize)` pair to `Option<&str>`.
///
/// Returns `None` when `ptr` is null or `len` is zero.
///
/// # Safety
/// Same as [`text`].
#[must_use]
pub unsafe fn text_opt<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() || len == 0 {
        None
    } else {
        Some(unsafe { text(ptr, len) })
    }
}

/// Generate an interpreter-aware wrapper for a C-ABI function that takes
/// one vector parameter.  The wrapper receives `LoftStore + LoftRef`,
/// extracts the raw data pointer and element count, and forwards to
/// the original C-ABI function.
///
/// Syntax: `vec_wrapper!(n_name, loft_name(params) -> ret)`
///
/// Use `vec<T>` to mark the vector parameter.
///
/// # Example
///
/// ```rust,ignore
/// loft_ffi::vec_wrapper!(n_gl_upload_canvas, loft_gl_upload_canvas(
///     data: vec<i32>, width: i32, height: i32) -> i32);
///
/// loft_ffi::vec_wrapper!(n_save_png, loft_save_png(
///     path_ptr: *const u8, path_len: usize, w: i32, h: i32, data: vec<i32>) -> bool);
/// ```
#[macro_export]
macro_rules! vec_wrapper {
    // With return type
    ($n_name:ident, $loft_name:ident ( $($params:tt)* ) -> $ret:ty) => {
        $crate::vec_wrapper!(@parse $n_name, $loft_name, -> $ret, before = [], after = [], rest = [ $($params)* ]);
    };
    // Void return
    ($n_name:ident, $loft_name:ident ( $($params:tt)* )) => {
        $crate::vec_wrapper!(@parse $n_name, $loft_name, ->, before = [], after = [], rest = [ $($params)* ]);
    };

    // ── @parse: find the vec<T> parameter ────────────────────────────

    // Found vec<T> with trailing comma — parse remaining into "after"
    (@parse $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [],
        rest = [ $vp:ident : vec < $elem:ty > , $($rest:tt)* ]
    ) => {
        $crate::vec_wrapper!(@after $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* ],
            after = [],
            vec = [ $vp $elem ],
            rest = [ $($rest)* ]
        );
    };

    // Found vec<T> as last param
    (@parse $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [],
        rest = [ $vp:ident : vec < $elem:ty > ]
    ) => {
        $crate::vec_wrapper!(@parse $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* ],
            after = [],
            rest = [ @VEC $vp $elem ]
        );
    };

    // Scalar param with trailing comma — accumulate into "before"
    (@parse $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [],
        rest = [ $pn:ident : $pt:ty , $($rest:tt)* ]
    ) => {
        $crate::vec_wrapper!(@parse $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* $pn : $pt , ],
            after = [],
            rest = [ $($rest)* ]
        );
    };

    // Scalar param as last — accumulate into "before"
    (@parse $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [],
        rest = [ $pn:ident : $pt:ty ]
    ) => {
        $crate::vec_wrapper!(@parse $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* $pn : $pt , ],
            after = [],
            rest = []
        );
    };

    // ── @after: parse params after the vec ─────────────────────────

    // Scalar with trailing comma
    (@after $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [ $($ap:tt)* ],
        vec = [ $vp:ident $elem:ty ],
        rest = [ $pn:ident : $pt:ty , $($rest:tt)* ]
    ) => {
        $crate::vec_wrapper!(@after $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* ],
            after = [ $($ap)* $pn : $pt , ],
            vec = [ $vp $elem ],
            rest = [ $($rest)* ]
        );
    };

    // Scalar as last
    (@after $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [ $($ap:tt)* ],
        vec = [ $vp:ident $elem:ty ],
        rest = [ $pn:ident : $pt:ty ]
    ) => {
        $crate::vec_wrapper!(@after $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* ],
            after = [ $($ap)* $pn : $pt , ],
            vec = [ $vp $elem ],
            rest = []
        );
    };

    // Terminal — all after params parsed
    (@after $n_name:ident, $loft_name:ident, -> $($ret:ty)?,
        before = [ $($bp:tt)* ],
        after = [ $($ap:tt)* ],
        vec = [ $vp:ident $elem:ty ],
        rest = []
    ) => {
        $crate::vec_wrapper!(@parse $n_name, $loft_name, -> $($ret)?,
            before = [ $($bp)* ],
            after = [ $($ap)* ],
            rest = [ @VEC $vp $elem ]
        );
    };

    // ── @parse terminal with vec found — emit ────────────────────────

    // With return type
    (@parse $n_name:ident, $loft_name:ident, -> $ret:ty,
        before = [ $($bpn:ident : $bpt:ty ,)* ],
        after = [ $($apn:ident : $apt:ty ,)* ],
        rest = [ @VEC $vp:ident $elem:ty ]
    ) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $n_name(
            __store: $crate::LoftStore,
            $( $bpn : $bpt, )*
            __vec_ref: $crate::LoftRef,
            $( $apn : $apt, )*
        ) -> $ret {
            let __count = unsafe { __store.vector_len(&__vec_ref) } as u32;
            let __ptr = unsafe { __store.vector_data_ptr(&__vec_ref) } as *const $elem;
            $loft_name($( $bpn, )* __ptr, __count, $( $apn, )*)
        }
    };

    // Void return
    (@parse $n_name:ident, $loft_name:ident, -> ,
        before = [ $($bpn:ident : $bpt:ty ,)* ],
        after = [ $($apn:ident : $apt:ty ,)* ],
        rest = [ @VEC $vp:ident $elem:ty ]
    ) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $n_name(
            __store: $crate::LoftStore,
            $( $bpn : $bpt, )*
            __vec_ref: $crate::LoftRef,
            $( $apn : $apt, )*
        ) {
            let __count = unsafe { __store.vector_len(&__vec_ref) } as u32;
            let __ptr = unsafe { __store.vector_data_ptr(&__vec_ref) } as *const $elem;
            $loft_name($( $bpn, )* __ptr, __count, $( $apn, )*)
        }
    };
}

// ── Registration macro ────────────────────────────────────────────────

/// Generate a `loft_register_v1` entry point that registers native functions.
///
/// Each entry is a function identifier.  The registration name is derived
/// automatically from the identifier via `stringify!`, so the Rust function
/// name **must** match the `#native "..."` annotation in the `.loft` file.
///
/// For the rare case where the Rust function name differs from the
/// registration name (e.g. an interpreter-aware `n_` variant registered
/// under a `loft_` name), use the `name => fn` form.
///
/// # Example
///
/// ```rust,ignore
/// loft_ffi::loft_register! {
///     loft_gl_clear,                                  // name = "loft_gl_clear"
///     loft_gl_draw,                                   // name = "loft_gl_draw"
///     loft_gl_upload_vertices => n_gl_upload_vertices, // name = "loft_gl_upload_vertices"
/// }
/// ```
#[macro_export]
macro_rules! loft_register {
    ( $( $name:ident $( => $func:ident )? ),* $(,)? ) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn loft_register_v1(
            __loft_reg_cb: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
            __loft_reg_ctx: *mut (),
        ) {
            unsafe {
                $(
                    $crate::loft_register!(@one __loft_reg_cb, __loft_reg_ctx, $name $( => $func )?);
                )*
            }
        }
    };

    // Internal: same-name registration (common case).
    (@one $cb:ident, $ctx:ident, $name:ident) => {
        $cb(
            stringify!($name).as_ptr(),
            stringify!($name).len(),
            $name as *const (),
            $ctx,
        )
    };

    // Internal: remapped registration (name differs from function).
    (@one $cb:ident, $ctx:ident, $name:ident => $func:ident) => {
        $cb(
            stringify!($name).as_ptr(),
            stringify!($name).len(),
            $func as *const (),
            $ctx,
        )
    };
}

/// Generate a `loft_register_bridges_v1` entry point registering plan-25
/// [`LoftBridgeFn`] bridges (the uniform-ABI wrappers the `#[loft_native]`
/// proc-macro emits as `<fn>__loft_bridge`).
///
/// The interpreter (plan-25 F4) will prefer `loft_register_bridges_v1` and
/// fall back to the legacy raw-ptr [`loft_register!`] (`loft_register_v1`)
/// for libraries not yet migrated — so this is additive and a library may
/// export both during the transition.  The invocation is generated by
/// `loft-ffi-build` (loft symbol literal → bridge ident); the
/// `let _: LoftBridgeFn = $bridge` binding statically checks the bridge has
/// the correct signature.
///
/// # Example
///
/// ```rust,ignore
/// loft_ffi::loft_register_bridges! {
///     "n_load_png" => n_load_png__loft_bridge,
///     "n_save_png" => n_save_png__loft_bridge,
/// }
/// ```
#[macro_export]
macro_rules! loft_register_bridges {
    ( $( $loft_name:literal => $bridge:ident ),* $(,)? ) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn loft_register_bridges_v1(
            __loft_reg_cb: unsafe extern "C" fn(*const u8, usize, *const (), *mut ()),
            __loft_reg_ctx: *mut (),
        ) {
            unsafe {
                $(
                    {
                        let __b: $crate::LoftBridgeFn = $bridge;
                        __loft_reg_cb(
                            $loft_name.as_ptr(),
                            $loft_name.len(),
                            __b as *const (),
                            __loft_reg_ctx,
                        );
                    }
                )*
            }
        }
    };
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── plan-25 F1: LoftValue transport + bridge registration hook ──

    #[test]
    fn loft_value_round_trips() {
        assert_eq!(LoftValue::int(42).as_i64(), 42);
        assert_eq!(LoftValue::int(i64::MIN).as_i64(), i64::MIN);
        assert!((LoftValue::float(3.5).as_f64() - 3.5).abs() < 1e-9);
        assert!(LoftValue::boolean(true).as_bool());
        assert!(!LoftValue::boolean(false).as_bool());
        let s = "hi";
        let t = LoftValue::text(LoftStr {
            ptr: s.as_ptr(),
            len: s.len(),
        })
        .as_text();
        assert_eq!(t.len, 2);
        let r = LoftValue::reference(LoftRef {
            store_nr: 1,
            rec: 2,
            pos: 3,
        })
        .as_ref();
        assert_eq!((r.store_nr, r.rec, r.pos), (1, 2, 3));
        assert_eq!(LoftValue::VOID.tag, LoftTag::Void);
    }

    #[test]
    fn loft_value_debug_matches_tag() {
        assert_eq!(format!("{:?}", LoftValue::int(7)), "LoftValue::I64(7)");
        assert_eq!(format!("{:?}", LoftValue::VOID), "LoftValue::Void");
        assert_eq!(
            format!("{:?}", LoftValue::boolean(true)),
            "LoftValue::Bool(true)"
        );
    }

    #[test]
    fn loft_value_is_repr_c_pod() {
        // Copy POD; the payload covers the widest member (LoftStr = ptr + len).
        assert_eq!(
            core::mem::size_of::<LoftPayload>(),
            core::mem::size_of::<LoftStr>()
        );
        assert!(core::mem::size_of::<LoftValue>() >= core::mem::size_of::<LoftStr>());
        let v = LoftValue::int(9);
        let _copy = v; // Copy
        assert_eq!(v.as_i64(), 9);
    }

    // Fixture for the loft_register_bridges! macro: a dummy bridge of the
    // exact LoftBridgeFn shape, plus the generated registration entry point.
    unsafe extern "C" fn dummy_bridge(
        _store: LoftStore,
        _args: *const LoftValue,
        _n: usize,
        ret: *mut LoftValue,
    ) {
        unsafe { *ret = LoftValue::int(123) };
    }
    mod bridge_fixture {
        use super::*;
        crate::loft_register_bridges! { "n_dummy" => dummy_bridge }
    }

    #[test]
    fn loft_register_bridges_macro_registers_under_loft_name() {
        use std::cell::RefCell;
        thread_local! {
            static GOT: RefCell<Vec<(String, *const ())>> = const { RefCell::new(Vec::new()) };
        }
        unsafe extern "C" fn cb(name: *const u8, len: usize, fp: *const (), _ctx: *mut ()) {
            let s = unsafe {
                std::str::from_utf8(std::slice::from_raw_parts(name, len))
                    .unwrap()
                    .to_string()
            };
            GOT.with(|g| g.borrow_mut().push((s, fp)));
        }
        unsafe { bridge_fixture::loft_register_bridges_v1(cb, std::ptr::null_mut()) };
        GOT.with(|g| {
            let v = g.borrow();
            assert_eq!(v.len(), 1, "one bridge registered");
            assert_eq!(v[0].0, "n_dummy", "registered under the loft symbol");
            assert!(!v[0].1.is_null(), "bridge fn pointer non-null");
        });
    }

    /// Build a read/write test store with no callbacks (allocation not supported).
    fn test_store(buf: &mut [u8]) -> LoftStore {
        LoftStore {
            ptr: buf.as_mut_ptr(),
            size: (buf.len() / 8) as u32,
            ctx: LoftStoreCtx {
                _opaque: std::ptr::null_mut(),
            },
            claim_fn: None,
            reload_fn: None,
            resize_fn: None,
            foreign_fn: None,
        }
    }

    #[test]
    fn ret_returns_borrowed_view() {
        let s = ret("hello".to_string());
        assert_eq!(s.len, 5);
        assert!(!s.ptr.is_null());
        let slice = unsafe { std::slice::from_raw_parts(s.ptr, s.len) };
        assert_eq!(slice, b"hello");
    }

    #[test]
    fn ret_ref_borrows_existing() {
        let data = "world";
        let s = ret_ref(data);
        assert_eq!(s.len, 5);
        assert_eq!(s.ptr, data.as_ptr());
    }

    #[test]
    fn empty_is_null() {
        assert!(LoftStr::EMPTY.ptr.is_null());
        assert_eq!(LoftStr::EMPTY.len, 0);
    }

    #[test]
    fn text_converts() {
        let data = "test";
        let s = unsafe { text(data.as_ptr(), data.len()) };
        assert_eq!(s, "test");
    }

    #[test]
    fn text_opt_none_on_null() {
        assert!(unsafe { text_opt(std::ptr::null(), 0) }.is_none());
    }

    #[test]
    fn text_opt_some_on_valid() {
        let data = "ok";
        assert_eq!(unsafe { text_opt(data.as_ptr(), data.len()) }, Some("ok"));
    }

    #[test]
    fn loft_ref_null() {
        assert!(LoftRef::NULL.is_null());
    }

    #[test]
    fn loft_ref_non_null() {
        let r = LoftRef {
            store_nr: 1,
            rec: 42,
            pos: 8,
        };
        assert!(!r.is_null());
    }

    #[test]
    fn loft_ref_size() {
        // Must be 10 bytes data + padding; repr(C) gives predictable layout.
        assert!(std::mem::size_of::<LoftRef>() <= 12);
    }

    #[test]
    fn loft_store_get_set_int() {
        // Simulate a store: 16 words = 128 bytes.
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        // Write i32 at rec=2, pos=8, offset=4  → byte 2*8+8+4 = 28
        unsafe { store.set_int(2, 8, 4, 42) };
        assert_eq!(unsafe { store.get_int(2, 8, 4) }, 42);
    }

    #[test]
    fn loft_store_get_set_long() {
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        unsafe { store.set_long(2, 8, 0, 123_456_789_012) };
        assert_eq!(unsafe { store.get_long(2, 8, 0) }, 123_456_789_012);
    }

    #[test]
    fn loft_store_get_set_float() {
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        unsafe { store.set_float(2, 8, 0, 3.14) };
        let v = unsafe { store.get_float(2, 8, 0) };
        assert!((v - 3.14).abs() < 1e-10);
    }

    #[test]
    fn loft_store_get_set_byte() {
        let mut buf = vec![0u8; 128];
        let store = test_store(&mut buf);
        unsafe { store.set_byte(2, 8, 0, 255) };
        assert_eq!(unsafe { store.get_byte(2, 8, 0) }, 255);
    }

    // Test vec_wrapper! macro
    #[unsafe(no_mangle)]
    fn loft_test_sum(ptr: *const i32, count: u32, offset: i32) -> i32 {
        let data = unsafe { std::slice::from_raw_parts(ptr, count as usize) };
        data.iter().sum::<i32>() + offset
    }

    vec_wrapper!(n_test_sum, loft_test_sum(data: vec<i32>, offset: i32) -> i32);

    #[test]
    fn vec_wrapper_generates_n_function() {
        let data = [1i32, 2, 3];
        let result = loft_test_sum(data.as_ptr(), 3, 10);
        assert_eq!(result, 16); // 1+2+3+10
    }
}
