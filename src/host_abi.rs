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
/// layout on both sides (the same rlib builds both), but every process-wide `static` in libloft
/// exists twice, so the cdylib's coroutine table is not the interpreter's.  What both
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
    pub coroutines: crate::coroutine_host::CoroutineHost,
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
    /// `@PLN206 P4` — what a HOSTED C unit asks of this interpreter's stores.  Last, so the
    /// fields a rustc artifact reads keep their offsets.
    pub hosted: &'static HostedTable,
}

/// The [`HostTable`] layout version; a cdylib refuses a table of another.
pub const HOST_ABI: u32 = 8;

/// `@PLN206 P4` — the store table as a hosted C unit reads it, kept inside `Stores`
/// (`Stores::hosted_view`) at an offset the unit is built with (`LOFT_STORES_VIEW_AT`): set
/// before each bridge call and after each act the unit leaves to the interpreter, the only times
/// the table changes while a unit runs, so the unit reads it without a call back.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TableView {
    pub table: *mut u8,
    pub n: u32,
    pub serial: u64,
}

impl Default for TableView {
    fn default() -> Self {
        TableView {
            table: std::ptr::null_mut(),
            n: 0,
            serial: 0,
        }
    }
}

// SAFETY: a copy of the table's address, read only on the thread whose `Stores` holds it.
unsafe impl Send for TableView {}
unsafe impl Sync for TableView {}

impl TableView {
    /// The view of `stores`' table as it stands.
    #[must_use]
    pub fn of(stores: &mut Stores) -> TableView {
        TableView {
            table: stores.allocations.as_mut_ptr().cast(),
            n: stores.allocations.len() as u32,
            serial: stores.stores_allocated,
        }
    }
}

/// `@PLN206 P4` — the acts a hosted C unit leaves to the interpreter (`p3-p4-design.md`): the
/// store table itself, and a store's birth and death, which touch the interpreter's own fields
/// and its table's bookkeeping.  Everything inside a store the unit does itself, on the shared
/// `loft_store_t` prefix (`store.rs`).  `#[repr(C)]`: the unit reads it by these offsets.
#[repr(C)]
pub struct HostedTable {
    /// The store table: its address, its length in `*n`, and the store counter in `*serial`.
    /// Asked again after every act below, which may move the table.
    pub table: unsafe extern "C" fn(stores: *mut Stores, n: *mut u32, serial: *mut u64) -> *mut u8,
    /// A fresh store with no record, live and counted: its number.
    pub store_new: unsafe extern "C" fn(stores: *mut Stores) -> u32,
    /// Release store `nr` as the interpreter releases one (`Stores::free_named`).
    pub store_free: unsafe extern "C" fn(stores: *mut Stores, nr: u32),
    /// The unit's `LOFT_HOST_STORE_TAIL`, which `store::HOST_STORE_TAIL` must equal.
    pub store_tail: u32,
    /// The program's directory (`Stores::source_dir`): its bytes, their count in `*len` —
    /// what `source_dir()` answers — and in `*relative` whether a relative path resolves
    /// against it (`Stores::program_relative`; `#cwd` and `LOFT_PATHS` turn it off).
    pub source_dir:
        unsafe extern "C" fn(stores: *mut Stores, len: *mut usize, relative: *mut u32) -> *const u8,
    /// A generator the unit made, handed to the interpreter's coroutine table: its frame's
    /// slot in the unit, the kind of what it yields (0 integer, 1 float, 2 text, 3 reference,
    /// 4 boolean), and the unit's step and release; its handle written into `ret.dbref`.
    pub gen_adopt: unsafe extern "C" fn(
        stores: *mut Stores,
        slot: u32,
        kind: u32,
        next: GenNext,
        free: GenFree,
        ret: *mut LibArg,
    ),
    /// `@PLN206 P7` — the debugger's pause request (`debugger::INTERRUPT`), which a unit's loop
    /// heads read.
    pub pause: &'static std::sync::atomic::AtomicBool,
    /// `@PLN206 P7` — the routine at a loop head (`key`, the loop's `scope`) offers its `n`
    /// locals ([`HandoverEntry`]); 1 when the interpreter takes the routine over from there
    /// (`State::accept_handover`), and the unit then returns without its own releases.
    pub handover: unsafe extern "C" fn(
        stores: *mut Stores,
        key: *const u8,
        key_len: usize,
        scope: u32,
        entries: *const HandoverEntry,
        n: u32,
    ) -> u32,
    /// `@PLN226` — the unit's store context for `stores` ([`HostedContexts`]): the one made
    /// before, or a new one from `make`, which `end` ends when the `Stores` drops.  Appended
    /// last, so a library built against the fields above reads them where they were.
    pub cx_of: unsafe extern "C" fn(
        stores: *mut Stores,
        unit: *const std::ffi::c_void,
        make: unsafe extern "C" fn() -> *mut std::ffi::c_void,
        end: unsafe extern "C" fn(*mut std::ffi::c_void),
    ) -> *mut std::ffi::c_void,
}

/// `@FR-C-Boundary` — the store contexts hosted C units keep for one `Stores`, one per unit it called
/// into: what a unit's code keeps of the calling thread's own (its frames, its fault jump, its
/// scratch; `src/c/loft_cx.h`), made by the unit at the first call and ended by it when this
/// `Stores` drops.  A `par` worker's `Stores` is its own, so its calls into a unit work in a
/// context of their own (formal/concurrency.md `C-Par`), as its stores are.
#[derive(Default)]
pub struct HostedContexts(Vec<HostedContext>);

struct HostedContext {
    unit: *const std::ffi::c_void,
    cx: *mut std::ffi::c_void,
    end: unsafe extern "C" fn(*mut std::ffi::c_void),
}

// SAFETY: a context is used only by the thread whose `Stores` holds it, as `TableView` is.
unsafe impl Send for HostedContexts {}
unsafe impl Sync for HostedContexts {}

impl HostedContexts {
    /// Unit `unit`'s context: the one it has, or a new one from `make`.
    pub fn of(
        &mut self,
        unit: *const std::ffi::c_void,
        make: unsafe extern "C" fn() -> *mut std::ffi::c_void,
        end: unsafe extern "C" fn(*mut std::ffi::c_void),
    ) -> *mut std::ffi::c_void {
        if let Some(c) = self.0.iter().find(|c| c.unit == unit) {
            return c.cx;
        }
        // SAFETY: the unit's own constructor, which answers a context it owns.
        let cx = unsafe { make() };
        self.0.push(HostedContext { unit, cx, end });
        cx
    }
}

impl Drop for HostedContexts {
    fn drop(&mut self) {
        for c in self.0.drain(..) {
            // SAFETY: the unit's own end, for the context its `make` answered.
            unsafe { (c.end)(c.cx) };
        }
    }
}

/// `@PLN206 P7` — one local a unit's routine offers at a hand-over (`loft_ho_entry`): kind 0
/// integer, 1 float, 2 text, 3 reference, 4 boolean or plain enum, 5 single.
#[repr(C)]
pub struct HandoverEntry {
    pub name: *const std::ffi::c_char,
    pub kind: u32,
    pub rs: u32,
    pub bits: i64,
    pub p: *const u8,
    pub len: usize,
    pub rr: u32,
    pub rp: u32,
}

/// A hosted unit's generator step: the frame in `slot` runs to its next `yield`, whose value
/// of `kind` lands in `out`; 1 when it yielded, 0 when it ran to its end.
pub type GenNext =
    unsafe extern "C" fn(stores: *mut Stores, slot: u32, kind: u32, out: *mut LibArg) -> u32;
/// A hosted unit's generator release: the frame in `slot` gives back what it holds.
pub type GenFree = unsafe extern "C" fn(stores: *mut Stores, slot: u32);

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
