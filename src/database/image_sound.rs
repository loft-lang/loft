// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I52 — Persistence: the one walk that says whether a store's numbers hold

//! `@FR-S-Sound` — the numbers INSIDE a store, followed from a value under its type and
//! checked before they are believed.
//!
//! ONE walk, [`Stores::validate_claims`], asked wherever the question comes up: by
//! `store_verify`, by the loader for a file from elsewhere (`store_load_untrusted` and the
//! URL loaders, before anything else reads the image), and by the copy diagnostics.  It
//! never follows a number it has not checked: a record number must name a claimed block,
//! and what each kind keeps in a block must lie inside that block.  A plain `store_load`
//! does not run it — a program that wrote the file itself does not pay for the walk.

use crate::database::{Parts, Stores};
use crate::keys::DbRef;
use crate::store::Store;

/// Where the walk is, for the report: a chain on the stack, rendered only when a number
/// does not hold, so a walk over a million sound elements formats nothing.
#[derive(Clone, Copy)]
struct Path<'a> {
    up: Option<&'a Path<'a>>,
    at: Seg<'a>,
}

#[derive(Clone, Copy)]
enum Seg<'a> {
    Root(&'a str),
    Field(&'a str),
    Index(u32),
    Entry(u32),
    Inline,
    Child,
}

impl std::fmt::Display for Path<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(up) = self.up {
            up.fmt(f)?;
        }
        match self.at {
            Seg::Root(name) => f.write_str(name),
            Seg::Field(name) => write!(f, ".{name}"),
            Seg::Index(i) => write!(f, "[{i}]"),
            Seg::Entry(rec) => write!(f, "{{#{rec}}}"),
            Seg::Inline => f.write_str("{inline}"),
            Seg::Child => f.write_str("->child"),
        }
    }
}

/// A nest this deep is a ring of records naming each other, or as good as one.
const MAX_DEPTH: u32 = 4096;

/// The words of the claimed block that starts at `rec`, or `None` when `rec` names no
/// record: past the store, inside a block, or a free block's.
fn block(store: &Store, rec: u32) -> Option<u32> {
    store.claimed_words(rec)
}

impl Stores {
    /// `@FR-S-Sound` — follow the value at `rec`, of type `tp`, and report on stderr (as
    /// `[cr-check] <path>: …`) each number that does not hold, counting them in `problems`;
    /// the walk stops past eight.  It dereferences nothing it has not checked.
    pub fn validate_claims(&self, rec: &DbRef, tp: u16, path: &str, problems: &mut u32) {
        let root = Path {
            up: None,
            at: Seg::Root(path),
        };
        self.claims_walk(rec, tp, &root, problems, 0);
    }

    /// Whether a value of `tp` holds anything the walk reads: a vector of plain numbers is
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

    #[expect(clippy::too_many_lines, reason = "one arm per kind of the type")]
    fn claims_walk(&self, rec: &DbRef, tp: u16, path: &Path, problems: &mut u32, depth: u32) {
        if *problems > 8 {
            return;
        }
        let bad = |problems: &mut u32, why: String| {
            crate::loft_eprintln!("[cr-check] {path}: {why}");
            *problems += 1;
        };
        if depth > MAX_DEPTH {
            bad(
                problems,
                format!("nested more than {MAX_DEPTH} deep — records that name each other"),
            );
            return;
        }
        if rec.store_nr as usize >= self.allocations.len() {
            bad(
                problems,
                format!(
                    "ref #{}.{},{} — store out of range",
                    rec.store_nr, rec.rec, rec.pos
                ),
            );
            return;
        }
        let store = &self.allocations[rec.store_nr as usize];
        if store.free {
            bad(
                problems,
                format!(
                    "ref #{}.{},{} — store FREED",
                    rec.store_nr, rec.rec, rec.pos
                ),
            );
            return;
        }
        let cap = store.capacity_words();
        if (tp as usize) >= self.types.len() {
            bad(problems, format!("type {tp} out of range"));
            return;
        }
        // The value itself lies inside the store, whatever it is: every arm reads it.
        let size = u64::from(self.size(tp)).max(1);
        if rec.rec >= cap || u64::from(rec.rec) * 8 + u64::from(rec.pos) + size > u64::from(cap) * 8
        {
            bad(
                problems,
                format!(
                    "ref #{}.{},{} — beyond store capacity ({cap} words)",
                    rec.store_nr, rec.rec, rec.pos
                ),
            );
            return;
        }
        let at = |rec_nr: u32, pos: u32| DbRef {
            store_nr: rec.store_nr,
            rec: rec_nr,
            pos,
        };
        let step = |seg: Seg<'_>, to: &DbRef, tp: u16, problems: &mut u32| {
            let next = Path {
                up: Some(path),
                at: seg,
            };
            self.claims_walk(to, tp, &next, problems, depth + 1);
        };
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                let cur = store.read::<u32>(rec.rec, rec.pos);
                // 0 and the forms above `i32::MAX` name no record (`Store::get_str`).
                if cur == 0 || cur > i32::MAX as u32 {
                    return;
                }
                let Some(words) = block(store, cur) else {
                    bad(problems, format!("text record {cur} is not a record"));
                    return;
                };
                let len = store.read::<u32>(cur, 4);
                if 8 + u64::from(len) > u64::from(words) * 8 {
                    bad(
                        problems,
                        format!("text record {cur} says {len} bytes and is {words} words"),
                    );
                } else if std::str::from_utf8(&store.read_span(cur, 8, len)).is_err() {
                    bad(problems, format!("text record {cur} is not UTF-8"));
                }
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    if self.holds_numbers(f.content) {
                        let to = at(rec.rec, rec.pos + u32::from(f.position));
                        step(Seg::Field(&f.name), &to, f.content, problems);
                    }
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let cur = store.read::<u32>(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                let Some(words) = block(store, cur) else {
                    bad(problems, format!("vector record {cur} is not a record"));
                    return;
                };
                let len = store.read::<u32>(cur, 4);
                let size = u32::from(self.size(*v));
                if 8 + u64::from(len) * u64::from(size) > u64::from(words) * 8 {
                    bad(
                        problems,
                        format!(
                            "vector record {cur} says {len} elements of {size} bytes and is \
                         {words} words"
                        ),
                    );
                    return;
                }
                if self.holds_numbers(*v) {
                    for i in 0..len {
                        step(Seg::Index(i), &at(cur, 8 + size * i), *v, problems);
                        if *problems > 8 {
                            return;
                        }
                    }
                }
            }
            Parts::ChildRec(ct) => {
                let r = store.read::<u32>(rec.rec, rec.pos);
                if r == 0 {
                    return;
                }
                let Some(words) = block(store, r) else {
                    bad(problems, format!("child record {r} is not a record"));
                    return;
                };
                if 8 + u64::from(self.size(*ct)) > u64::from(words) * 8 {
                    bad(
                        problems,
                        format!("child record {r} is {words} words, too small for its type"),
                    );
                    return;
                }
                step(Seg::Child, &at(r, 8), *ct, problems);
            }
            Parts::Enum(values) => {
                // Mirrors `copy_claims`' Enum arm (direct index, no -1 shift).
                let e_nr = i32::from(store.read::<u8>(rec.rec, rec.pos)) - 1;
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let etp = values[e_nr as usize].0;
                    if etp != u16::MAX {
                        self.claims_walk(rec, etp, path, problems, depth + 1);
                    }
                }
            }
            Parts::Hash(v, _) => {
                // Mirrors hash.rs: the root word holds the table record; its header word is
                // `room`, buckets from `BUCKET0`, `elms = (room - RESERVED_WORDS) * 8 / sb`.
                let cur = store.read::<u32>(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                let Some(room) = block(store, cur) else {
                    bad(problems, format!("hash table record {cur} is not a record"));
                    return;
                };
                if room < crate::hash::RESERVED_WORDS {
                    bad(
                        problems,
                        format!("hash table record {cur} is {room} words, too small"),
                    );
                    return;
                }
                // `(Col-Inline)` — an inline table's entries are positions in the table
                // record itself, listed by the hash's own walk.
                if crate::hash::is_inline(store, cur) {
                    for entry in crate::hash::records(rec, &self.allocations) {
                        step(Seg::Inline, &entry, *v, problems);
                    }
                    return;
                }
                let sb = crate::hash::slot_bytes(store, cur);
                let elms = (room - crate::hash::RESERVED_WORDS) * 8 / sb;
                // @PLN135 arc H — a slot holds an arena INDEX for a hash that owns its
                // entries, and a record number for one that borrows them.  The stride the
                // table records is the discriminator, and the chunk directory has to be
                // reached before any slot can be decoded.
                let stride = crate::hash::stride(store, cur);
                let dir = store.read::<u32>(cur, crate::arena::DIR_FLD);
                if stride != 0 && block(store, dir).is_none() {
                    bad(
                        problems,
                        format!("hash arena directory {dir} is not a record"),
                    );
                    return;
                }
                for i in 0..elms {
                    let slot = store.read::<u32>(cur, crate::hash::BUCKET0 + i * sb);
                    if slot == 0 {
                        continue;
                    }
                    // The slot's high bit says it names a RECORD rather than an arena
                    // index (`hash::SLOT_RECORD`) — decided per slot.
                    let (entry, pos) = if stride == 0 || slot & crate::hash::SLOT_RECORD != 0 {
                        let entry = slot & !crate::hash::SLOT_RECORD;
                        if block(store, entry).is_none() {
                            bad(
                                problems,
                                format!("hash entry record {entry} is not a record"),
                            );
                            if *problems > 8 {
                                return;
                            }
                            continue;
                        }
                        (entry, crate::store::RECORD_PAYLOAD)
                    } else {
                        match crate::arena::slot(store, cur, slot, stride) {
                            Some(found) => found,
                            None => continue,
                        }
                    };
                    step(Seg::Entry(entry), &at(entry, pos), *v, problems);
                }
            }
            // An ARRAY / ORDERED holds a record of element REFERENCES.
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let cur = store.read::<u32>(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                let Some(words) = block(store, cur) else {
                    bad(problems, format!("array record {cur} is not a record"));
                    return;
                };
                let len = store.read::<u32>(cur, 4);
                if 8 + u64::from(len) * 4 > u64::from(words) * 8 {
                    bad(
                        problems,
                        format!("array record {cur} says {len} elements and is {words} words"),
                    );
                    return;
                }
                for i in 0..len {
                    let elm = store.read::<u32>(cur, 8 + 4 * i);
                    if elm == 0 {
                        continue;
                    }
                    let fits = block(store, elm)
                        .is_some_and(|w| 8 + u64::from(self.size(*v)) <= u64::from(w) * 8);
                    if !fits {
                        bad(
                            problems,
                            format!(
                                "array element {i} is said to be record {elm}, which is not one that holds it"
                            ),
                        );
                        return;
                    }
                    step(Seg::Index(i), &at(elm, 8), *v, problems);
                    if *problems > 8 {
                        return;
                    }
                }
            }
            // A RADIX / TRIE root word claims the tree record.  The LEAVES belong to the
            // primary collection and are validated through it, so this checks the spine's
            // own record and stops.
            Parts::Radix(_, _) | Parts::Trie(_, _) => {
                let cur = store.read::<u32>(rec.rec, rec.pos);
                if cur != 0 && block(store, cur).is_none() {
                    bad(problems, format!("tree record {cur} is not a record"));
                }
            }
            // A stored 12-byte DbRef points OUT of this store, so the only thing checkable
            // here without following it into another allocation is its store number.
            Parts::DbRef => {
                let sn = store.read::<u32>(rec.rec, rec.pos);
                if sn != u32::from(u16::MAX) && sn as usize >= self.allocations.len() {
                    bad(
                        problems,
                        format!("stored DbRef names store #{sn}, out of range"),
                    );
                }
            }
            // Deliberately nothing, and each for its own reason — not a catch-all.
            //
            // `Index` BORROWS its rows from the primary collection, so there is no claim of
            // its own to check and walking into the primary's rows would report twice.
            //
            // The narrow-integer kinds and a non-text `Base` hold VALUES, not claims.
            Parts::Index(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Base => {}
        }
    }
}
