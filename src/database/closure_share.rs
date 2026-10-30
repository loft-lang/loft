// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The closure-copy share count: the ONE reference count in the store layer.
//!
//! ⚠ **This is not a general mechanism, and nothing else may use it.**  loft releases a store
//! ONCE, by its one owner, and that owner is decided statically (`formal/ownership.md`,
//! `formal/closures.md` `(L-CapOwn)`).  The store layer has no general reference count, by
//! design: a count lets every site that cannot work out who owns a store "just count it",
//! and the ownership rules stop being the answer.  Do not reach for this to fix a double
//! free or a leak somewhere else — find the owner instead.
//!
//! It exists for exactly one case the static rules cannot settle (loft#2273): a fn-ref read
//! out of a struct field and KEPT (`g = h.f`, a returned `h.f`) is a COPY of its closure record
//! (`@FR-B-Copy`), and the copy SHARES every store the record captured (`@FR-L-CapHeap`,
//! `@FR-L-CapWrite`: a write through either reaches the other, and the frame that captured
//! the value still reads it).  When the field's record OWNS one of those stores, two records
//! now release it, and which of them lets go last is a run-time fact: the field can be
//! overwritten, or its host released, before or after the copy dies, in this frame or in a
//! caller's.
//!
//! So each copy adds one SHARE to each owned store it names, and a release of a shared
//! store takes one share back instead of freeing it.  The last holder frees it as the static
//! rules always did.  A store no copy shares has no entry, and its release is unchanged.
//!
//! The boundary, enforced by `tests/closure_share_use.rs`:
//! - a share is RAISED only by [`Stores::copy_closure_record`] (`OpCopyClosure`), through a
//!   function private to this file;
//! - a share is TAKEN BACK only by `Stores::free_named`, through
//!   [`Stores::release_closure_share`].
//!
//! Adding a third use is a design change to the ownership model, not a refactor: it needs an
//! entry in `formal/closures.md` and the owner's agreement.

use crate::database::{Parts, Stores};
use crate::keys::DbRef;

/// The stores a closure COPY shares with the record it was copied from, and how many extra
/// holders each has.  Private on purpose: the only way in is the two functions below.
#[derive(Default)]
pub(crate) struct ClosureShares {
    /// `(store_nr, holders beyond the first)`.  A handful of entries at most — one per owned
    /// capture of a live closure copy — so a vector beats a map.
    extra: Vec<(u16, u32)>,
}

impl Stores {
    /// `OpCopyClosure` — a fn-ref read out of a place and KEPT (`g = h.f`) as a COPY of the
    /// closure record it names (`@FR-B-Copy`, `@FR-L-CapKeep`): a fresh store holding a copy
    /// of the `tp` record at `src`, independent of the place it was read from, so an
    /// overwrite, rebuild or release of that place leaves the copy as it was.
    ///
    /// Every capture is SHARED with the source record.  A capture the record OWNS (the
    /// `dbref` slot shape, `@FR-L-CapOne`) gains one share here, so the copy's own release and
    /// the source's each let go of it once; a BORROWED capture (`dbref_borrow`) belongs to
    /// another owner and gains nothing.  A null `src` (a fn-ref with no closure) answers null.
    pub fn copy_closure_record(&mut self, src: &DbRef, tp: u16) -> DbRef {
        if src.store_nr == u16::MAX || src.rec == 0 || tp == u16::MAX {
            return DbRef::NULL;
        }
        let size = u32::from(self.size(tp));
        let db = self.null();
        let to = self.claim(&db, 1 + size.div_ceil(8));
        // The record's type is what the release cascade reads (`free_named`).
        self.allocations[to.store_nr as usize].set_known_type(tp);
        self.store_mut(&to).zero_fill(to.rec);
        self.store_mut(&to).set_u32_raw(to.rec, 4, u32::from(tp));
        self.copy_block(src, &to, size);
        self.copy_claims(src, &to, tp);
        let owned_shape = self.dbref();
        let owned: Vec<u16> = match &self.types[tp as usize].parts {
            Parts::Struct(fields) => fields
                .iter()
                .filter(|f| f.content == owned_shape)
                .map(|f| f.position)
                .collect(),
            _ => Vec::new(),
        };
        for pos in owned {
            let held = self.get_db_ref(&to, u32::from(pos));
            if held.store_nr != u16::MAX && held.rec != 0 {
                share_with_closure_copy(self, held.store_nr);
            }
        }
        to
    }

    /// The release half: `true` when the store at `store_nr` is shared with a closure copy,
    /// in which case one share is taken back and the store must NOT be freed — another holder
    /// still names it.  `false` for every store no closure copy shares, which is all of them
    /// in a program that never keeps a fn-ref read out of a field.
    ///
    /// ⚠ Called from `Stores::free_named` and nowhere else (see the module doc).
    pub(super) fn release_closure_share(&mut self, store_nr: u16) -> bool {
        let shares = &mut self.closure_shares.extra;
        if shares.is_empty() {
            return false;
        }
        let Some(i) = shares.iter().position(|e| e.0 == store_nr) else {
            return false;
        };
        shares[i].1 -= 1;
        if shares[i].1 == 0 {
            shares.swap_remove(i);
        }
        true
    }
}

/// One more holder of `store_nr`: a closure copy now names the store its source record owns.
/// Private to this file — [`Stores::copy_closure_record`] is its only caller.
fn share_with_closure_copy(stores: &mut Stores, store_nr: u16) {
    if (store_nr as usize) >= stores.allocations.len() || stores.allocations[store_nr as usize].free
    {
        return;
    }
    let shares = &mut stores.closure_shares.extra;
    if let Some(e) = shares.iter_mut().find(|e| e.0 == store_nr) {
        e.1 += 1;
    } else {
        shares.push((store_nr, 1));
    }
}
