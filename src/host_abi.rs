// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the ABI between the host and a compiled library artifact

//! The types a compiled library artifact and the process that loads it share: the argument
//! slot of a bridge call ([`LibArg`]), the table the host hands the artifact at load
//! ([`HostTable`]), the id tables the artifact fills from it ([`IdTable`]) and the symbol
//! that records the layout it was built for.  Both sides link this exact code, so its layout
//! is the contract.  Part of the runtime a compiled unit links; the interpreter fills the
//! table (`native_adopt::HOST_TABLE`) and the Rust writer names these types in what it emits
//! (`loft::native_lib::…`, re-exported there).

use crate::database::Stores;

/// @PLN11 Arc N — a uniform 16/24-byte argument/return slot for the
/// **shared-store** native-library bridge (`native_lib::generate_shared_cdylib_lib_rs`).
///
/// The bridge knows each slot's type from the function signature, so no tag is
/// needed: a scalar reads/writes `.scalar` (an `i64`, or float bits, or `0/1` for
/// bool), a `vector`/`reference` reads/writes `.dbref`.  `#[repr(C)]` so the
/// interpreter and the generated cdylib — **both linking this exact type from
/// libloft** — agree on layout with no marshalling.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LibArg {
    /// Scalar payload: an `i64`, `f64::to_bits() as i64`, `f32::to_bits() as i64`,
    /// or a boolean as `0/1`.  Junk for non-scalar slots.
    pub scalar: i64,
    /// Reference payload: the raw stack `DbRef` for a `vector`/`reference` slot
    /// (passed through unchanged — the `--native` body expects the same indirect
    /// form the interpreter holds).  Junk for non-ref slots.
    pub dbref: crate::keys::DbRef,
    /// Text payload pointer: for a `text` arg, the UTF-8 bytes (borrowed from the
    /// caller's store for the call's duration — `--native` takes `&str`).  Null
    /// for non-text slots.
    pub text_ptr: *const u8,
    /// Text payload length (bytes), paired with `text_ptr`.
    pub text_len: usize,
}

impl LibArg {
    /// All-zero slot — the spread base so each `LibArg` literal sets only the one
    /// field its type uses (`LibArg { scalar: x, ..LibArg::ZERO }`, etc.).
    pub const ZERO: LibArg = LibArg {
        scalar: 0,
        dbref: crate::keys::DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        },
        text_ptr: std::ptr::null(),
        text_len: 0,
    };
}

/// @PLN199 C0 — the one channel from the interpreter into a library cdylib it loaded.
///
/// The cdylib links its OWN copy of libloft: `Stores`, `DbRef` and [`LibArg`] have the same
/// layout on both sides (the same rlib builds both), but every `static` and `thread_local!` in
/// libloft exists twice, so the cdylib's coroutine table is not the interpreter's.  What both
/// sides must agree on therefore crosses as a function of the HOST, reached through this
/// table, which the host hands every generated cdylib once, at load (`c_libs::load_one`
/// calls its `loft_shared_init`).
#[repr(C)]
pub struct HostTable {
    /// [`HOST_ABI`] of the host that filled the table.
    pub abi: u32,
    /// Register a generator the cdylib built in the HOST's coroutine table and write its
    /// handle into `ret.dbref`.  `coro` is a `Box<Box<dyn LoftCoroutine>>`, thinned.
    pub coroutine_adopt:
        unsafe extern "C" fn(stores: *mut Stores, coro: *mut std::ffi::c_void, ret: *mut LibArg),
    /// @PLN199 C2 — call the function value `(d_nr, closure.dbref)` with the `n` visible
    /// arguments at `args`, in the interpreter running this bridge call, writing its result
    /// into `ret` (`State::call_back`).
    /// @PLN199 D2 — the host's coroutine table, which a library cdylib's own copy of
    /// `codegen_runtime` forwards to once installed, so a generator handle means the same
    /// frame on both sides.
    pub coroutines: crate::codegen_runtime::CoroutineHost,
    pub call_ref: unsafe extern "C" fn(
        stores: *mut Stores,
        d_nr: u32,
        closure: *const LibArg,
        args: *const LibArg,
        n: usize,
        ret: *mut LibArg,
    ),
    /// @PLN199 G0b-2 — this program's id of the type named `key` (`lib_ids::TypeNames::key`),
    /// or `u16::MAX`; `ids` is what the host passed to `loft_shared_init`.
    pub type_id:
        unsafe extern "C" fn(ids: *const std::ffi::c_void, key: *const u8, len: usize) -> u16,
    /// @PLN199 G0b-2 — this program's number of the function named `key` (`lib_ids::fn_key`),
    /// or `u32::MAX`.
    pub fn_id:
        unsafe extern "C" fn(ids: *const std::ffi::c_void, key: *const u8, len: usize) -> u32,
}

/// The [`HostTable`] layout version; a cdylib refuses a table of another.
pub const HOST_ABI: u32 = 5;

/// @PLN199 G0b-2 — a library artifact's table of ids (`__tid` / `__fid`): written by its
/// `loft_shared_init` before any of its code runs, read by that code after.  The write-once
/// discipline is what makes a plain read sound, and a plain read is what the optimiser can
/// merge with another read of the same slot, where an atomic one it cannot.
pub struct IdTable<T: Copy, const N: usize>(std::cell::UnsafeCell<[T; N]>);

impl<T: Copy, const N: usize> IdTable<T, N> {
    #[must_use]
    pub const fn new(zero: T) -> Self {
        IdTable(std::cell::UnsafeCell::new([zero; N]))
    }

    /// Slot `k`.
    #[inline]
    #[must_use]
    pub fn get(&self, k: usize) -> T {
        unsafe { (*self.0.get())[k] }
    }

    /// Fill slot `k`.
    ///
    /// # Safety
    /// Only while no code that reads the table can run: from `loft_shared_init`.
    pub unsafe fn set(&self, k: usize, v: T) {
        unsafe { (*self.0.get())[k] = v };
    }
}

/// The name of the symbol a generated cdylib exports to declare the type layout
/// it was built against.  Read by [`layout_fp_of_relocated`] before the artifact
/// is adopted.
pub(crate) const LAYOUT_FP_SYMBOL: &str = "loft_type_layout_fp_v1";
