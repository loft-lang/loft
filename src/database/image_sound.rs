// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I52 — Persistence: is a store image sound before it is adopted

//! `@FR-S-Sound` — the numbers INSIDE a store image, checked before the image is adopted.
//!
//! The frame check (`Store::validate_structure`) says the file is a chain of blocks.  It says
//! nothing of what the blocks hold: a record number in a field, a vector's length, a text's
//! length.  Every read after the load believes those, so an image that came from somewhere
//! else — a save passed on, a pack downloaded — decides where the program reads and writes.
//! This walk follows the root's type through the image and refuses it unless every record
//! number it meets names a claimed block and what each kind keeps in a block lies inside it.

use crate::database::{Parts, Stores};
use crate::store::{PRIMARY, Store};

/// The first byte of a record's content, past its header and its kind's half-word.
const PAYLOAD: u32 = 8;

impl Stores {
    /// `@FR-S-Sound` — whether the image in `slot` may be read under the slot's type: `Err`
    /// names the first number that does not hold.  It follows what the type OWNS in line —
    /// struct fields, texts, vectors and sorted vectors, child records — and stops at a
    /// keyed collection, whose own tables it does not read yet (so what such a collection
    /// holds is not walked either).
    pub(super) fn image_sound(&self, slot: u16) -> Result<(), String> {
        let store = &self.allocations[slot as usize];
        let root_tp = store.known_type;
        if root_tp == u16::MAX || !store.has_root_record() {
            return Ok(());
        }
        let root_words = block_words(store, PRIMARY, "the root")?;
        self.fits(root_tp, PAYLOAD, root_words, "the root")?;
        // A record is walked once: a second field naming it has nothing new to say, and an
        // image whose records name each other in a ring would otherwise never end.
        let mut walked = vec![0u64; (store.capacity_words() as usize).div_ceil(64)];
        let mut first_visit = |rec: u32| {
            let (word, bit) = ((rec >> 6) as usize, 1u64 << (rec & 63));
            let new = walked[word] & bit == 0;
            walked[word] |= bit;
            new
        };
        // (record, byte in it, type there, words of the record)
        let mut work = vec![(PRIMARY, PAYLOAD, root_tp, root_words)];
        while let Some((rec, pos, tp, words)) = work.pop() {
            if tp as usize >= self.types.len() {
                return Err(format!("type {tp} is not one of this program's"));
            }
            match &self.types[tp as usize].parts {
                Parts::Base if tp == 5 => text_sound(store, store.read::<u32>(rec, pos))?,
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    for f in fields {
                        work.push((rec, pos + u32::from(f.position), f.content, words));
                    }
                }
                Parts::Enum(values) => {
                    let tag = i64::from(store.read::<u8>(rec, pos)) - 1;
                    if let Some((variant, _)) =
                        usize::try_from(tag).ok().and_then(|t| values.get(t))
                        && *variant != u16::MAX
                    {
                        self.fits(*variant, pos, words, "an enum value")?;
                        work.push((rec, pos, *variant, words));
                    }
                }
                Parts::Vector(elem) | Parts::Sorted(elem, _) => {
                    let vec_rec = store.read::<u32>(rec, pos);
                    if vec_rec == 0 {
                        continue;
                    }
                    let vec_words = block_words(store, vec_rec, "a vector")?;
                    let len = u64::from(store.read::<u32>(vec_rec, 4));
                    let size = u64::from(self.size(*elem));
                    if u64::from(PAYLOAD) + len * size > u64::from(vec_words) * 8 {
                        return Err(format!(
                            "vector record {vec_rec} says {len} elements of {size} bytes \
                             and is {vec_words} words"
                        ));
                    }
                    if first_visit(vec_rec) && self.holds_numbers(*elem) {
                        for i in (0..len).rev() {
                            // In range: `len * size` fits the block, checked above.
                            let at = PAYLOAD + (i * size) as u32;
                            work.push((vec_rec, at, *elem, vec_words));
                        }
                    }
                }
                Parts::ChildRec(child) => {
                    let child_rec = store.read::<u32>(rec, pos);
                    if child_rec == 0 {
                        continue;
                    }
                    let child_words = block_words(store, child_rec, "a child record")?;
                    self.fits(*child, PAYLOAD, child_words, "a child record")?;
                    if first_visit(child_rec) {
                        work.push((child_rec, PAYLOAD, *child, child_words));
                    }
                }
                // Not read yet, each named so that a new kind is a decision made here: a
                // keyed collection's own tables and nodes (and so what it holds), and a
                // stored reference, which the load rebases after this walk.
                Parts::Array(_)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _)
                | Parts::Radix(_, _)
                | Parts::Trie(_, _)
                | Parts::DbRef => {}
                // Values: no record number and no length to be wrong.
                Parts::Base
                | Parts::Byte(_, _)
                | Parts::Short(_, _)
                | Parts::Int(_, _)
                | Parts::ShortRaw(_, _)
                | Parts::IntRaw(_, _) => {}
            }
        }
        Ok(())
    }

    /// Whether a value of `tp` at byte `pos` lies inside a record of `words` words.
    fn fits(&self, tp: u16, pos: u32, words: u32, what: &str) -> Result<(), String> {
        let need = u64::from(pos) + u64::from(self.size(tp));
        if need > u64::from(words) * 8 {
            return Err(format!(
                "{what} needs {need} bytes and its record is {words} words"
            ));
        }
        Ok(())
    }

    /// Whether a value of `tp` holds anything this walk reads: a vector of plain numbers is
    /// sound by its length alone, and its elements are not visited one by one.
    fn holds_numbers(&self, tp: u16) -> bool {
        match self.types.get(tp as usize).map(|t| &t.parts) {
            Some(Parts::Base) => tp == 5,
            Some(Parts::Struct(fields) | Parts::EnumValue(_, fields)) => {
                fields.iter().any(|f| self.holds_numbers(f.content))
            }
            Some(
                Parts::Byte(_, _)
                | Parts::Short(_, _)
                | Parts::Int(_, _)
                | Parts::ShortRaw(_, _)
                | Parts::IntRaw(_, _),
            ) => false,
            _ => true,
        }
    }
}

/// The words of the claimed block that starts at `rec`, or why `rec` is not one: a number
/// in an image names a record only when the chain has a claimed block starting there.
fn block_words(store: &Store, rec: u32, what: &str) -> Result<u32, String> {
    store
        .claimed_words(rec)
        .ok_or_else(|| format!("{what} is said to be record {rec}, which is not a record"))
}

/// A text field's record: its length inside its block, its bytes UTF-8 (they are handed out
/// as a `str` unchecked).  0 and the null forms name no record.
fn text_sound(store: &Store, rec: u32) -> Result<(), String> {
    if rec == 0 || rec > i32::MAX as u32 {
        return Ok(());
    }
    let words = block_words(store, rec, "a text")?;
    let len = store.read::<u32>(rec, 4);
    if u64::from(PAYLOAD) + u64::from(len) > u64::from(words) * 8 {
        return Err(format!(
            "text record {rec} says {len} bytes and is {words} words"
        ));
    }
    if std::str::from_utf8(&store.read_span(rec, PAYLOAD, len)).is_err() {
        return Err(format!("text record {rec} is not UTF-8"));
    }
    Ok(())
}
