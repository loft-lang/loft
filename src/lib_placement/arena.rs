// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I116 — Library placement — in-process, a worker, or another machine
// @PLN119 arc B — the call arena: the store a compound value crosses in.

//! Scalars fit in a frame; a struct or a vector does not. It is a graph of
//! records, and the whole point of this plan is that it crosses without being
//! re-encoded into a second vocabulary on the way.
//!
//! So it crosses as itself. An arena is an ordinary loft [`Store`] backed by an
//! mmapped file that both processes map, and marshalling a value is
//! [`Stores::copy_claims`] — the same deep copy an assignment uses — with the
//! arena as the destination store. The receiving side reads the record in place.
//!
//! # Why this works at all
//!
//! Nothing inside a store is a [`DbRef`]. Interior pointers — a text's bytes, a
//! vector's element block, a child record — are all plain `u32` record ids
//! (`Stores::relocate_ptr_fields` is the exhaustive list). A record graph is
//! therefore independent of both the address the store is mapped at **and** the
//! store number it is registered under, so each side names the arena with its
//! own index and no pointer is ever translated.
//!
//! # One writer per arena
//!
//! There are two arenas per placed library, one per direction. That is not
//! caution: a `Store` keeps allocator state on the Rust side (`claims`,
//! `free_root`) as well as in the mapping, so two processes claiming out of one
//! store would each allocate against their own idea of the free list and hand
//! out the same block twice. [`Arena::resync`] is the one sanctioned exception —
//! the callee may WRITE to an argument (loft passes a compound by reference), so
//! the worker re-derives the allocator state from the mapping before it runs.
//!
//! # Bound only while a call is crossing
//!
//! A store sitting in `Stores::allocations` at exit is a leak by definition, and
//! the arena is not the program's data. [`Arena::bind`] borrows a slot for the
//! crossing and [`Arena::unbind`] gives it back, which is what keeps the parity
//! gate's leak half exactly as strict under `process` as under `inproc` rather
//! than needing an exemption.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use crate::database::Stores;
use crate::keys::DbRef;
use crate::store::Store;
use std::io;
use std::path::{Path, PathBuf};

/// One direction of a placed library's call arena.
///
/// Created by the caller (which owns the file), attached by the worker. The
/// `Store` is `None` exactly while the arena is bound into a `Stores`.
pub struct Arena {
    path: PathBuf,
    store: Option<Store>,
    /// Only the process that created the file removes it.
    owner: bool,
}

impl Drop for Arena {
    fn drop(&mut self) {
        // Unmap before unlinking, so the file is gone only once nothing holds it.
        self.store = None;
        if self.owner {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Arena {
    /// Create the backing file and map it. The caller side.
    ///
    /// # Errors
    /// Any failure to create or map the file.
    pub fn create(path: &Path) -> io::Result<Arena> {
        let _ = std::fs::remove_file(path);
        let store = open_store(path)?;
        Ok(Arena {
            path: path.to_path_buf(),
            store: Some(store),
            owner: true,
        })
    }

    /// Map an arena another process created. The worker side.
    ///
    /// # Errors
    /// A missing, unmappable, or non-store file.
    pub fn attach(path: &Path) -> io::Result<Arena> {
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no call arena at {}", path.display()),
            ));
        }
        let store = open_store(path)?;
        Ok(Arena {
            path: path.to_path_buf(),
            store: Some(store),
            owner: false,
        })
    }

    /// Register the arena into `stores` for the duration of one crossing, and
    /// answer the store number it landed on.
    ///
    /// The number is not stable between calls and nothing may remember it — that
    /// is the point of [`the module header](self): a record graph does not name
    /// its own store.
    ///
    /// # Panics
    /// If the arena is already bound.
    pub fn bind(&mut self, stores: &mut Stores) -> u16 {
        let store = self
            .store
            .take()
            .expect("a call arena was bound while already bound");
        stores.adopt_store(store)
    }

    /// Take the arena back out of `stores` and give the slot back to the pool.
    ///
    /// `release_slot` is the half that matters: `take_store` alone leaves the
    /// slot reserved (it is written for a store meant to OUTLIVE the table), so
    /// a crossing that only took would burn one slot per call.
    pub fn unbind(&mut self, stores: &mut Stores, nr: u16) {
        self.store = Some(stores.take_store(nr));
        stores.release_slot(nr);
    }

    /// Empty the arena — the writer's per-call reset.
    ///
    /// `Store::init` rather than `Stores::clear`: `clear` REPLACES a file-backed
    /// store with a fresh heap one ([#513](https://github.com/loft-lang/loft/issues/513)),
    /// which would silently drop the mapping the other process is reading.
    ///
    /// Resetting rather than freeing record by record is sound because a loft
    /// borrow cannot outlive the call it was passed to, so nothing the callee
    /// kept may still point here when the next call starts.
    ///
    /// # Panics
    /// If the arena is currently bound into a `Stores`.
    pub fn reset(&mut self) {
        self.store
            .as_mut()
            .expect("a call arena was reset while bound")
            .init();
    }

    /// Re-derive the allocator state from the mapping.
    ///
    /// The free list and the claim set live in the store's bytes; the `Store`
    /// struct only caches them. A side that did not make the other side's claims
    /// holds a stale cache, and allocating against it would hand out a block that
    /// is already in use. The worker needs this because loft passes a compound
    /// argument BY REFERENCE — a callee that appends to a vector parameter
    /// allocates in the arena the caller just wrote.
    ///
    /// # Panics
    /// If the arena is currently bound into a `Stores`.
    pub fn resync(&mut self) {
        self.store
            .as_mut()
            .expect("a call arena was resynced while bound")
            .resync_allocator();
    }

    /// The arena's current size in 8-byte words — what a reader compares against
    /// its own mapping to notice the writer grew the file.
    ///
    /// # Panics
    /// If the arena is currently bound into a `Stores`.
    #[must_use]
    pub fn words(&self) -> u32 {
        self.store
            .as_ref()
            .expect("a call arena was measured while bound")
            .capacity_words()
    }

    /// The arena's live bytes — what a `remote` placement SENDS instead of
    /// mapping (@PLN119 arc E).
    ///
    /// This is where the plan's "one mechanism at four latencies" survives the
    /// network hop, and it survives it for a concrete reason: a store is a
    /// self-contained image whose interior pointers are `u32` record ids, so
    /// putting it on a socket is a copy of the value's own layout — not an
    /// encoding of it. The remote crossing pays a copy where the local one pays
    /// a page table, and neither pays a per-field marshal.
    ///
    /// Only the LIVE prefix travels. An arena that grew for one big call keeps
    /// its capacity, and sending that afterwards would make every later small
    /// call pay for it.
    ///
    /// # Panics
    /// If the arena is currently bound into a `Stores`.
    #[must_use]
    pub fn image(&self) -> &[u8] {
        let store = self
            .store
            .as_ref()
            .expect("a call arena was read while bound");
        let usage = store.usage();
        // `live_end_words` is only the mark when the block walk COMPLETED; on a
        // short walk it is a lower bound, and sending a lower bound would cut
        // live records off the end. Fall back to the whole buffer, which is
        // never wrong, only bigger.
        let words = if usage.walk_complete {
            usage.live_end_words.max(2)
        } else {
            store.capacity_words()
        };
        let len = (words as usize) * 8;
        unsafe {
            std::slice::from_raw_parts(
                store.base_ptr(),
                len.min(store.capacity_words() as usize * 8),
            )
        }
    }

    /// Adopt bytes another machine's arena sent.
    ///
    /// The image is a store, so this is a copy into the buffer plus a re-derive
    /// of the allocator's cached state — the same [`resync`](Arena::resync) the
    /// local transport needs when the other side has been claiming, and for the
    /// same reason.
    ///
    /// # Panics
    /// If the arena is currently bound into a `Stores`.
    pub fn load_image(&mut self, bytes: &[u8]) {
        if bytes.len() < 16 {
            return; // not a store; the sender had nothing to say
        }
        self.store
            .as_mut()
            .expect("a call arena was written while bound")
            .adopt_image(bytes);
    }

    /// Re-map when the writer has grown the file past what this side mapped.
    ///
    /// A growing `claim` resizes the file and re-mmaps it in the WRITER; the
    /// reader's mapping still covers the old length, and reading past it is a
    /// `SIGBUS` rather than a wrong answer. So the writer publishes its word
    /// count with every frame and the reader maps again when it moved.
    ///
    /// # Errors
    /// A failure to map the grown file.
    ///
    /// # Panics
    /// If the arena is currently bound into a `Stores`.
    pub fn remap_if_grown(&mut self, words: u32) -> io::Result<()> {
        if words == 0 || self.words() == words {
            return Ok(());
        }
        // Drop the old mapping first: two mappings of one growing file is what
        // makes a stale page read as live data.
        self.store = None;
        self.store = Some(open_store(&self.path)?);
        Ok(())
    }
}

/// `LOFT_TRACE_ARENA=1` — narrate what each side put in the arena and read back
/// out of it. Off by default and read once, because the question it answers
/// ("are these the same bytes?") only ever comes up when they are not.
pub fn trace(stores: &Stores, what: &str, r: &DbRef, tp: u16) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("LOFT_TRACE_ARENA").is_some()) {
        return;
    }
    use std::fmt::Write as _;
    let mut bytes = String::new();
    if !r.is_null() && (r.store_nr as usize) < stores.allocations.len() {
        let s = &stores.allocations[r.store_nr as usize];
        for off in 0..4u32 {
            let _ = write!(bytes, " {:08x}", s.get_u32_raw(r.rec, r.pos + off * 4));
        }
    }
    eprintln!(
        "[arena {}] {what} #{}@{},{} tp={tp}{bytes}",
        std::process::id(),
        r.store_nr,
        r.rec,
        r.pos
    );
}

/// `Store::open` panics on a missing or malformed file. A worker must report
/// that as a load failure the caller can name, not as a crash.
fn open_store(path: &Path) -> io::Result<Store> {
    let p = path.to_string_lossy().into_owned();
    std::panic::catch_unwind(|| Store::open(&p)).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not usable as a call arena", path.display()),
        )
    })
}

/// Claim a record of type `tp` in store `store_nr`, laid out exactly as
/// `OpDatabase` lays one out.
///
/// The same four steps, in the same order: claim a header word plus the payload,
/// record the type on the store and in the record, and default-initialise. A
/// second opinion about any of them is a value the reading side misreads rather
/// than a failure, which is why this mirrors the opcode instead of approximating
/// it.
pub fn alloc_value(stores: &mut Stores, store_nr: u16, tp: u16) -> DbRef {
    let anchor = DbRef {
        store_nr,
        rec: 0,
        pos: 0,
    };
    debug_assert!(
        !stores.allocations[store_nr as usize].read_only,
        "a call arena must be writable by the side that allocates in it"
    );
    let size = stores.enum_parent_size(tp);
    let r = stores.claim(&anchor, 1 + u32::from(size).div_ceil(8));
    stores.allocations[r.store_nr as usize].set_known_type(tp);
    stores.store_mut(&r).set_u32_raw(r.rec, 4, u32::from(tp));
    stores.set_default_value(tp, &r);
    r
}

/// Does this reference name an actual record?
///
/// Three states share one `DbRef`, and only the first two have a name of their
/// own: absent (`store_nr == u16::MAX`), a store with **no record claimed**
/// (`rec == 0` — what `InitRef` leaves behind for a destination the callee has
/// not filled in, and equally a valid-but-empty vector), and a real record.
///
/// Reading `rec == 0` as a destination is what makes a `Point { … }` answer land
/// in the store's own header instead of a record: the bytes go somewhere, the
/// program reads `null`, and nothing reports a fault.
#[must_use]
pub fn names_a_record(r: &DbRef) -> bool {
    !r.is_null() && r.rec != 0
}

/// Mint a store holding one fresh value of type `tp`, exactly as `OpDatabase`
/// mints one for `Point { … }`.
///
/// What the dispatcher does when a compound answer comes back and the caller
/// offered nowhere to put it. In-process that case is the callee minting its own
/// store and handing ownership over, and the caller's emitted code frees it; the
/// same store has to exist here or the caller's `OpFreeRef` would have nothing to
/// free and the answer nowhere to live.
///
/// A FRESH store rather than the caller's empty placeholder, and that is not a
/// detail: the caller's `OpFreeRefIfDistinct(__ref_1, p)` frees the placeholder
/// precisely when it is not what came back, so building the answer inside it
/// would have the caller free the store its own result lives in.
///
/// The normalised `rec = 1, pos = 8` is `alloc_record_at`'s, whose convention
/// this has to match exactly — a fresh store's root record is record one, and
/// every in-language reference to it says so.
pub fn mint_value(stores: &mut Stores, tp: u16) -> DbRef {
    let db = stores.null();
    let r = alloc_value(stores, db.store_nr, tp);
    DbRef {
        store_nr: r.store_nr,
        rec: 1,
        pos: 8,
    }
}

/// Overwrite the value at `dst` with the one at `src`, both of type `tp`.
///
/// The three steps are `OpCopyRecord`'s, in its order, because this IS an
/// assignment — one that happens to have a store the other process can see on
/// one end. Each step covers what the others do not:
///
/// * `remove_claims` frees what the destination already owned. Without it,
///   copying a result into the same variable in a loop strands one text or
///   vector graph per iteration — `copy_claims` overwrites the pointer, not
///   what it pointed at.
/// * `copy_block` moves the record's own bytes. `copy_claims` does NOT: it
///   copies the heap graph a record OWNS and leaves plain fields alone, so a
///   struct of two integers marshalled with `copy_claims` alone arrives holding
///   whatever the destination was initialised to — nulls.
/// * `copy_claims` then re-homes every owned sub-record (a text's bytes, a
///   vector's elements) into the destination's store, which is what makes the
///   copy independent of the source rather than a graph of pointers back into
///   it.
pub fn copy_value(stores: &mut Stores, dst: &DbRef, src: &DbRef, tp: u16) {
    if src.is_null() || (src.store_nr == dst.store_nr && src.rec == dst.rec && src.pos == dst.pos) {
        return;
    }
    let size = u32::from(stores.size(tp));
    stores.remove_claims(dst, tp);
    stores.copy_block(src, dst, size);
    stores.copy_claims(src, dst, tp);
}
