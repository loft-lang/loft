// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @F54 — Browser / WASM target: the `deliver` FFI boundary that hands the host
// (the browser JS bridge) a self-describing handle to a live loft value.
//
// @PLN105 Phase 1 — the `deliver` boundary + its LOOPBACK host.
//
// `deliver(tag, value)` hands the host a self-describing handle to a live loft
// value; the host reads the value's layout DESCRIPTOR (@PLN105 Phase 0) and walks
// the bytes with no serialization. Phase 1 wires the boundary with a LOOPBACK host
// (no browser): the host reconstructs the value from the descriptor and prints a
// deterministic line. The whole point is the parity gate — `deliver` lowers to ONE
// `#rust` body (`stores.deliver_reconstruct(...)`) that feeds BOTH the interpreter
// (via fill.rs) and native codegen, and a value passed BY VALUE arrives as the same
// record `DbRef` on both backends (matching native's `file_to_bytes(self)`
// convention, sidestepping the interp/native slot-vs-record deref asymmetry). So
// `--interpret` and `--native` must print byte-identical output for any value.
//
// The descriptor reconstruction reuses `read_via_descriptor`, already proven
// (Phase 0) to reproduce `read_data`'s bytes; Phase 1 proves the HANDLE reaches the
// host correctly and identically across the loft-call boundary on both backends.

use crate::database::Stores;
use crate::keys::DbRef;

impl Stores {
    /// @PLN105 Phase 1 loopback host — reconstruct the delivered value from its
    /// layout descriptor and print a deterministic line. Called from the `OpDeliver`
    /// `#rust` body on both backends, so its output is the parity oracle.
    ///
    /// `val` is the value's record `DbRef` (a by-value struct/collection argument
    /// arrives already deref'd to its record on both backends). Scalars delivered by
    /// value are not record-shaped and are out of the Phase-1 subset.
    ///
    /// `&mut self` because the browser path may MATERIALISE a keyed collection into a scratch
    /// vector before delivery (Phase 3 pre-flatten); the loopback path only reads.
    pub fn deliver_reconstruct(&mut self, tag: i64, val: DbRef, db_tp: u16) {
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        {
            self.deliver_browser(tag, val, db_tp);
        }
        // native / interpreter / wasi — the Phase-1 LOOPBACK host (the both-backend parity oracle).
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
        {
            self.deliver_loopback(tag, val, db_tp);
        }
    }

    /// @PLN105 Phase 3 — `expose(tag, value)`: a LONG-LIVED `deliver` whose borrow spans FRAMES.
    /// Hands the host the same descriptor handle as `deliver`, but then PINS the value's store with
    /// a read-only lock (AFTER materialising the keyed collections — a claim on a locked store
    /// panics) so its wasm-memory addresses stay stable across frames until `release` unpins it.
    /// The host stashes the handle by `tag` and re-reads it each frame (re-deriving its view — a
    /// `memory.grow` detaches the old buffer). Non-browser targets just lock (no host to hand to).
    pub fn expose_value(&mut self, tag: i64, val: DbRef, db_tp: u16) {
        if val.rec == 0 {
            return;
        }
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        {
            use std::collections::BTreeMap;
            let mut desc = self.layout_descriptor(&[db_tp]);
            let mut flat: BTreeMap<u64, u32> = BTreeMap::new();
            self.collect_keyed(&desc, db_tp, val, &mut flat);
            Self::rewrite_iterated(&mut desc);
            let json = desc.to_delivery_json(&flat);
            self.lock_store(&val);
            let store_base = self.allocations[val.store_nr as usize].ptr as usize;
            crate::loft_host_expose(
                tag,
                store_base,
                val.rec,
                val.pos,
                u32::from(db_tp),
                json.as_ptr(),
                json.len(),
            );
        }
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
        {
            let _ = (tag, db_tp);
            self.lock_store(&val);
        }
    }

    /// @PLN105 Phase 3 — `release(tag, value)`: unpin the store `expose` pinned (via the value's
    /// `DbRef`, so no tag→store table is needed) and tell the host to drop its stashed handle.
    pub fn release_value(&mut self, tag: i64, val: DbRef, db_tp: u16) {
        let _ = db_tp;
        if val.rec == 0 {
            return;
        }
        self.unlock_store(&val);
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        crate::loft_host_release(tag);
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
        let _ = tag;
    }

    /// @PLN105 Phase 2/3 — the browser host path: hand the value to JS driven by its layout
    /// descriptor, with no serialization (the value bytes stay in wasm linear memory). A struct /
    /// scalar / vector is read IN PLACE (§2). A KEYED collection has no byte layout a reader can
    /// walk, so it is PRE-FLATTENED: `collect_keyed` walks the WHOLE value (into records AND vector
    /// elements) and materialises every hash/radix/index INSTANCE — the same `for x in coll` path —
    /// into the `flat` redirect map keyed by its `(rec, pos)`; `rewrite_iterated` then turns the
    /// (type-shared) `Iterated` nodes into `FlatArray` (redirect-read) or, for `sorted`, an in-place
    /// `Vector`. The reader looks each `FlatArray`'s data record up in `flat` by the current
    /// `(rec, pos)`, so ONE type node serves every element of a `vector<Bag>`. SYNCHRONOUS — the
    /// borrow ends when this returns, so `desc`/`flat` stay owned across the `loft_host_deliver` call.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn deliver_browser(&mut self, tag: i64, val: DbRef, db_tp: u16) {
        use std::collections::BTreeMap;
        if val.rec == 0 {
            return;
        }
        let mut desc = self.layout_descriptor(&[db_tp]);
        let mut flat: BTreeMap<u64, u32> = BTreeMap::new();
        self.collect_keyed(&desc, db_tp, val, &mut flat);
        Self::rewrite_iterated(&mut desc);
        self.emit_deliver(tag, val, db_tp, &desc.to_delivery_json(&flat));
    }

    /// The `flat` redirect key for a keyed collection at `(rec, pos)` — matches the reader's
    /// `"<rec>_<pos>"` lookup (`to_delivery_json`).
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn flat_key(rec: u32, pos: u32) -> u64 {
        (u64::from(rec) << 32) | u64::from(pos)
    }

    /// Walk the WHOLE value (records + their fields, vector/array elements — mirroring
    /// `read_via_descriptor`'s traversal so the `(rec, pos)` of each keyed collection matches what
    /// the reader computes) and, for every hash/radix/index INSTANCE, materialise its element array
    /// (`build_*_sorted_vec`, key/Morton order) into `flat[(rec,pos)] = data_record`. `sorted` is an
    /// in-place vector (handled by `rewrite_iterated`, no entry); `ordered` is unsupported.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn collect_keyed(
        &mut self,
        desc: &crate::database::LayoutDesc,
        node_id: u16,
        at: DbRef,
        flat: &mut std::collections::BTreeMap<u64, u32>,
    ) {
        use crate::database::{Iterated, LayoutNode};
        match desc.nodes.get(&node_id) {
            Some(LayoutNode::Iterated(it)) => {
                let scratch = match it {
                    Iterated::Hash { .. } => self.build_hash_sorted_vec(&at, node_id),
                    // A trie shares the radix TREE, so the same in-order walk
                    // materialises it — only the key oracle above differs.
                    Iterated::Radix { .. } | Iterated::Trie { .. } => {
                        self.build_radix_sorted_vec(&at, node_id)
                    }
                    Iterated::Index { .. } => self.build_index_sorted_vec(&at, node_id),
                    Iterated::Sorted { .. } | Iterated::Ordered { .. } => return,
                };
                let data = self.allocations[scratch.store_nr as usize]
                    .get_u32_raw(scratch.rec, scratch.pos);
                flat.insert(Self::flat_key(at.rec, at.pos), data);
            }
            Some(LayoutNode::Record(fields)) | Some(LayoutNode::EnumValue(_, fields)) => {
                for f in fields.clone() {
                    if !f.is_data() {
                        continue;
                    }
                    let field_at = DbRef {
                        store_nr: at.store_nr,
                        rec: at.rec,
                        pos: at.pos + u32::from(f.position),
                    };
                    self.collect_keyed(desc, f.content, field_at, flat);
                }
            }
            // A VECTOR of inline records: elements at `(v_rec, 8 + elem_size*i)` — descend into each.
            Some(LayoutNode::Vector(elem)) => {
                let elem = *elem;
                let size = u32::from(desc.sizes.get(&elem).copied().unwrap_or(0));
                let v_rec = self.allocations[at.store_nr as usize].get_u32_raw(at.rec, at.pos);
                if v_rec == 0 {
                    return;
                }
                let len = self.allocations[at.store_nr as usize].get_u32_raw(v_rec, 4);
                for i in 0..len {
                    let elem_at = DbRef {
                        store_nr: at.store_nr,
                        rec: v_rec,
                        pos: 8 + size * i,
                    };
                    self.collect_keyed(desc, elem, elem_at, flat);
                }
            }
            // A BY-REF ARRAY: each element is a record at `(elm_rec, 8)`.
            Some(LayoutNode::Array(elem)) => {
                let elem = *elem;
                let v_rec = self.allocations[at.store_nr as usize].get_u32_raw(at.rec, at.pos);
                if v_rec == 0 {
                    return;
                }
                let len = self.allocations[at.store_nr as usize].get_u32_raw(v_rec, 4);
                let elm_recs: Vec<u32> = (0..len)
                    .map(|i| self.allocations[at.store_nr as usize].get_u32_raw(v_rec, 8 + 4 * i))
                    .collect();
                for elm_rec in elm_recs {
                    let elem_at = DbRef {
                        store_nr: at.store_nr,
                        rec: elm_rec,
                        pos: 8,
                    };
                    self.collect_keyed(desc, elem, elem_at, flat);
                }
            }
            // Scalars / text / ref / childrec — nothing keyed below.
            _ => {}
        }
    }

    /// Rewrite the (type-shared) descriptor nodes for a keyed collection into an array-shaped node
    /// the reader can walk: hash/radix/index → `FlatArray` (its data record is looked up per
    /// instance in `flat`); `sorted` → an in-place `Vector` (already an inline, key-ordered vector,
    /// `sorted_finish` maintains it on every `+=`). `ordered` is left unsupported.  The C backend
    /// writes the same rewritten descriptor into a page's unit (`src/c/deliver.rs`).
    pub(crate) fn rewrite_iterated(desc: &mut crate::database::LayoutDesc) {
        use crate::database::{Iterated, LayoutNode};
        for node in desc.nodes.values_mut() {
            let replacement = match node {
                // A hash's entries are arena SLOTS, so its scratch holds `(record, offset)`
                // pairs; every other kind still materialises record numbers. The stride
                // travels with the node so the reader does not have to know which kind it
                // came from — see `LayoutNode::FlatArray`.
                LayoutNode::Iterated(Iterated::Hash { elem, .. }) => LayoutNode::FlatArray {
                    elem: *elem,
                    stride: 8,
                },
                LayoutNode::Iterated(
                    Iterated::Radix { elem, .. }
                    | Iterated::Trie { elem, .. }
                    | Iterated::Index { elem, .. },
                ) => LayoutNode::FlatArray {
                    elem: *elem,
                    stride: 4,
                },
                LayoutNode::Iterated(Iterated::Sorted { elem, .. }) => LayoutNode::Vector(*elem),
                _ => continue,
            };
            *node = replacement;
        }
    }

    /// Call the `loft_host_deliver` import: `Store.ptr` is the store buffer's base in wasm linear
    /// memory, so the reader addresses everything as `store_base + rec*8 + pos`.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    fn emit_deliver(&self, tag: i64, r: DbRef, type_id: u16, desc: &str) {
        let store_base = self.allocations[r.store_nr as usize].ptr as usize;
        crate::loft_host_deliver(
            tag,
            store_base,
            r.rec,
            r.pos,
            u32::from(type_id),
            desc.as_ptr(),
            desc.len(),
        );
    }

    /// The Phase-1 loopback host — reconstruct the delivered value from its descriptor and print a
    /// deterministic line. The parity oracle for `--interpret` == `--native`. Split out of
    /// `deliver_reconstruct` so the browser (Phase 2) and loopback paths are each cfg-clean. Only
    /// the non-browser targets keep it (the browser calls the `loft_host_deliver` import instead).
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
    fn deliver_loopback(&self, tag: i64, val: DbRef, db_tp: u16) {
        let name = self
            .types
            .get(db_tp as usize)
            .map_or_else(|| format!("#{db_tp}"), |t| t.name.clone());
        let desc = self.layout_descriptor(&[db_tp]);
        let mut bytes = Vec::new();
        match self.read_via_descriptor(&desc, &val, db_tp, true, &mut bytes) {
            Ok(()) => {
                use std::fmt::Write as _;
                let mut hex = String::with_capacity(bytes.len() * 2);
                for b in &bytes {
                    let _ = write!(hex, "{b:02x}");
                }
                println!("deliver tag={tag} type={name} bytes={hex}");
            }
            Err(e) => println!("deliver tag={tag} type={name} error={e}"),
        }
    }
}
