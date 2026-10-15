// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database (alloc/persistence/journal/snapshot): linked field groups

//! A group of fields stored as one unit (a stored tuple, an index key) and the arithmetic
//! that lays its members out: alignment, offsets, size.  The schema builds it from the IR's
//! definitions and the store reads it at run time, so it sits with the store (@PLN191).

/// A group of fields on a struct/type that belong together as a single
/// logical unit.  Used **exclusively** for tuple elements.  Do not
/// extend for other uses without explicit user direction.
///
/// `Tuple` — synthetic `__tuple<T1,T2,…>` struct's element fields
/// `_0`, `_1`, ….  One group per tuple-shape struct; `field_indices`
/// lists every attribute in element order.  Registered by
/// `Data::tuple_def`].
///
/// An `index<T[key]>` once appended a red-black bookkeeping triple as a second kind; its
/// B+tree (`btree.rs`) keeps its nodes apart and adds no field (@PLN190).
#[derive(Clone, Debug, PartialEq)]
pub enum LinkedFieldKind {
    Tuple,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkedFieldGroup {
    pub kind: LinkedFieldKind,
    /// 0 for Tuple (only one tuple group per synthetic struct).
    pub instance: u16,
    /// Indices into the host struct's field/attribute list, in element order (length =
    /// arity).
    pub field_indices: Vec<u16>,
    /// Alignment the GROUP must be placed at — the MAX of every
    /// member field's alignment.  A tuple `(integer (align 8), byte (align 1))` has
    /// group alignment 8.  Pass this to the layout routine as the
    /// alignment requirement when reserving the group's contiguous
    /// block — the routine must honour it so the int/integer members
    /// land on their natural-alignment boundaries.
    pub alignment: u8,
    /// Total bytes the group occupies when laid out atomically — the
    /// sum of every member's size, **plus** any internal padding
    /// needed to align larger members after smaller ones in element
    /// order.  A tuple `(byte, integer)` requires 7 bytes
    /// of internal padding between them: `1 + 7 + 8 = 16`.
    pub size: u16,
}

impl LinkedFieldGroup {
    /// Compute group alignment as the max of every member's
    /// alignment.  `member_aligns` lists each member's natural
    /// alignment in element order — e.g. for an index triple
    /// `[4, 4, 1]`, returns 4.
    #[must_use]
    pub fn group_alignment(member_aligns: &[u8]) -> u8 {
        member_aligns.iter().copied().max().unwrap_or(1)
    }

    /// Compute the group's total atomic size: each member at its
    /// natural-alignment offset within the group, ending at the
    /// natural-alignment-padded total.  `member_sizes_aligns` lists
    /// `(size, alignment)` per member in element order.
    ///
    /// For an index triple `[(4,4), (4,4), (1,1)]` returns 9.
    /// For a tuple `[(byte=1,1), (integer=8,8)]` returns 16 (the
    /// 1-byte byte then 7 bytes padding then 8-byte integer).
    #[must_use]
    pub fn group_size(member_sizes_aligns: &[(u16, u8)]) -> u16 {
        let mut pos: u16 = 0;
        for &(size, align) in member_sizes_aligns {
            // Pad up to the member's natural alignment.
            let align_u16 = u16::from(align.max(1));
            let rem = pos % align_u16;
            if rem != 0 {
                pos += align_u16 - rem;
            }
            pos += size;
        }
        pos
    }

    /// Number of fields in this group.  For Tuple = arity, for Index
    /// = 3 (always `[left, right, color]`).
    #[must_use]
    pub fn arity(&self) -> usize {
        self.field_indices.len()
    }

    /// Index into the host struct's field/attribute list of the
    /// `member_idx`-th group member.  Returns `None` if `member_idx`
    /// is out of range.  Safe entry point — replaces ad-hoc
    /// `group.field_indices[i]` with bounds-checked access.
    #[must_use]
    pub fn member_field_index(&self, member_idx: usize) -> Option<u16> {
        self.field_indices.get(member_idx).copied()
    }

    /// Per-member offsets inside the group, in element order.  Used
    /// by the layout routine (post-group-atomic placement) to assign
    /// each member's position relative to the group's anchor.
    /// Mirrors `group_size`'s internal packing — first member at 0,
    /// each subsequent member at the next natural-alignment offset.
    #[must_use]
    pub fn record_member_offsets(member_sizes_aligns: &[(u16, u8)]) -> (Vec<u16>, u16) {
        // `@FR-L-Tuple` / `@FR-L-Struct` (@C139) — a stored TUPLE is a record, packed as any
        // record is: the members with the LARGEST alignment first, members of equal alignment
        // in written order, each on its natural boundary; the size rounded up to the largest
        // alignment.  The ORDER of the bytes changes nothing a program sees — member `i` is
        // still `t.i`, and the tuple prints in written order.  An index group keeps
        // [`Self::group_member_offsets`]: its consumers read `color` at `left + 8`.
        let mut order: Vec<usize> = (0..member_sizes_aligns.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(member_sizes_aligns[i].1));
        let mut offsets = vec![0u16; member_sizes_aligns.len()];
        let mut pos: u16 = 0;
        let mut max_align: u16 = 1;
        for i in order {
            let (size, align) = member_sizes_aligns[i];
            let align_u16 = u16::from(align.max(1));
            max_align = max_align.max(align_u16);
            pos = pos.next_multiple_of(align_u16);
            offsets[i] = pos;
            pos += size;
        }
        (offsets, pos.next_multiple_of(max_align))
    }

    /// Per-member offsets inside a group in WRITTEN order — the index group's layout; a
    /// tuple's is [`Self::record_member_offsets`].
    #[must_use]
    pub fn group_member_offsets(member_sizes_aligns: &[(u16, u8)]) -> Vec<u16> {
        let mut offsets = Vec::with_capacity(member_sizes_aligns.len());
        let mut pos: u16 = 0;
        for &(size, align) in member_sizes_aligns {
            let align_u16 = u16::from(align.max(1));
            let rem = pos % align_u16;
            if rem != 0 {
                pos += align_u16 - rem;
            }
            offsets.push(pos);
            pos += size;
        }
        offsets
    }
}
