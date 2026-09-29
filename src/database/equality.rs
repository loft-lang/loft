// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-E-Eq`, @C91 — content equality: the ONE runtime home of `a == b` for a value that lives
//! in a store.  Both backends reach it through `OpEqContent` (`default/01_code.loft`).

use crate::database::{Field, Parts, Stores};
use crate::keys::DbRef;

impl Stores {
    /// Whether the values at `a` and `b`, both of known type `tp`, hold the same content
    /// (`@FR-E-Eq`): a scalar by its value (`0.0 == -0.0`, null equals null), a `text` by its
    /// characters, a struct or variant field by field in declaration order, an enum by its
    /// variant and then that variant's fields, a vector / array / sorted / index element by
    /// element in its order, a `hash` by its records in key order, a spatial index or trie by
    /// its elements in the tree's order.  Two handles that are one place answer `true` at
    /// once; that is the fast path, never the meaning.
    ///
    /// A stored `reference<T>` (`Parts::DbRef`) is compared as the handle it is: the schema
    /// row carries no target type to follow it into (deviation D-op-16, formal/operational.md).
    ///
    /// # Panics
    /// On a `tp` that is no stored type, as the schema walks do.
    #[must_use]
    pub fn eq_content(&self, a: &DbRef, b: &DbRef, tp: u16) -> bool {
        if a == b {
            return true;
        }
        // A record that is not there: two absences are one value, and an absence differs from
        // every present value (`@FR-E-Eq`, "null equals null").
        if a.rec == 0 || b.rec == 0 {
            return a.rec == 0 && b.rec == 0;
        }
        let (sa, sb) = (self.store(a), self.store(b));
        match tp {
            0 => return sa.get_int(a.rec, a.pos) == sb.get_int(b.rec, b.pos),
            1 => return sa.get_long(a.rec, a.pos) == sb.get_long(b.rec, b.pos),
            // A float's null is its NaN, so the two are compared by absence first; the values
            // then by `==`, which holds `0.0 == -0.0`.
            2 => {
                let (x, y) = (sa.get_single(a.rec, a.pos), sb.get_single(b.rec, b.pos));
                return if x.is_nan() || y.is_nan() {
                    x.is_nan() && y.is_nan()
                } else {
                    x == y
                };
            }
            3 => {
                let (x, y) = (sa.get_float(a.rec, a.pos), sb.get_float(b.rec, b.pos));
                return if x.is_nan() || y.is_nan() {
                    x.is_nan() && y.is_nan()
                } else {
                    x == y
                };
            }
            4 => return sa.get_byte(a.rec, a.pos, 0) == sb.get_byte(b.rec, b.pos, 0),
            5 => {
                let (na, nb) = (sa.text_is_null(a.rec, a.pos), sb.text_is_null(b.rec, b.pos));
                if na || nb {
                    return na && nb;
                }
                return sa.get_str(sa.get_u32_raw(a.rec, a.pos))
                    == sb.get_str(sb.get_u32_raw(b.rec, b.pos));
            }
            6 => return sa.get_u32_raw(a.rec, a.pos) == sb.get_u32_raw(b.rec, b.pos),
            _ => {}
        }
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => self.eq_fields(a, b, fields),
            Parts::Enum(variants) => {
                let (va, vb) = (sa.get_byte(a.rec, a.pos, 0), sb.get_byte(b.rec, b.pos, 0));
                if va != vb {
                    return false;
                }
                // Absent (0), or a variant without fields: the tag was the whole value.
                let Some(&(variant, _)) =
                    usize::try_from(va - 1).ok().and_then(|v| variants.get(v))
                else {
                    return true;
                };
                match self.types.get(variant as usize).map(|t| &t.parts) {
                    Some(Parts::EnumValue(_, fields)) => self.eq_fields(a, b, fields),
                    _ => true,
                }
            }
            Parts::Byte(from, _) => {
                sa.get_byte(a.rec, a.pos, *from) == sb.get_byte(b.rec, b.pos, *from)
            }
            Parts::Short(from, _) => {
                sa.get_short(a.rec, a.pos, *from) == sb.get_short(b.rec, b.pos, *from)
            }
            Parts::ShortRaw(from, _) => {
                sa.get_i16_raw(a.rec, a.pos, *from) == sb.get_i16_raw(b.rec, b.pos, *from)
            }
            Parts::Int(_, _) => sa.get_i32_raw(a.rec, a.pos) == sb.get_i32_raw(b.rec, b.pos),
            Parts::IntRaw(_, _) => sa.get_u32_raw(a.rec, a.pos) == sb.get_u32_raw(b.rec, b.pos),
            Parts::DbRef => (0..3).all(|w| {
                sa.get_u32_raw(a.rec, a.pos + 4 * w) == sb.get_u32_raw(b.rec, b.pos + 4 * w)
            }),
            Parts::ChildRec(content) => {
                let (ra, rb) = (sa.get_u32_raw(a.rec, a.pos), sb.get_u32_raw(b.rec, b.pos));
                let child = |store_nr, rec| DbRef {
                    store_nr,
                    rec,
                    pos: 8,
                };
                self.eq_content(&child(a.store_nr, ra), &child(b.store_nr, rb), *content)
            }
            Parts::Vector(content)
            | Parts::Sorted(content, _)
            | Parts::Array(content)
            | Parts::Ordered(content, _)
            | Parts::Index(content, _, _) => {
                if let Some(answer) = self.eq_absent_collections(a, b) {
                    return answer;
                }
                let (mut pa, mut pb) = (i32::MAX, i32::MAX);
                loop {
                    let ea = self.next(a, &mut pa, tp);
                    let eb = self.next(b, &mut pb, tp);
                    match (ea.rec == 0, eb.rec == 0) {
                        (true, true) => return true,
                        (false, false) => {
                            if !self.eq_content(&ea, &eb, *content) {
                                return false;
                            }
                        }
                        _ => return false,
                    }
                }
            }
            Parts::Hash(content, _) => {
                if let Some(answer) = self.eq_absent_collections(a, b) {
                    return answer;
                }
                let keys = self.keys(tp);
                let ra = crate::hash::records_sorted(a, &self.allocations, keys);
                let rb = crate::hash::records_sorted(b, &self.allocations, keys);
                ra.len() == rb.len()
                    && ra
                        .iter()
                        .zip(&rb)
                        .all(|(x, y)| self.eq_content(x, y, *content))
            }
            Parts::Radix(content, _) | Parts::Trie(content, _) => {
                if let Some(answer) = self.eq_absent_collections(a, b) {
                    return answer;
                }
                let elements = |r: &DbRef| -> Vec<DbRef> {
                    self.for_each_owned_child(r, tp)
                        .children
                        .iter()
                        .filter_map(|c| c.owning_elem)
                        .map(|rec| DbRef {
                            store_nr: r.store_nr,
                            rec,
                            pos: 8,
                        })
                        .collect()
                };
                let (ea, eb) = (elements(a), elements(b));
                ea.len() == eb.len()
                    && ea
                        .iter()
                        .zip(&eb)
                        .all(|(x, y)| self.eq_content(x, y, *content))
            }
            Parts::Base => panic!(
                "eq_content on the base type {} ({})",
                tp, self.types[tp as usize].name
            ),
        }
    }

    /// The fields of one record type, each with its own type's content `==`, a variant's `enum`
    /// tag by its byte.  A field named
    /// `#…` is the collection's own bookkeeping (a tree link, a back pointer) and a field whose
    /// first other-index is `u16::MAX` is a secondary VIEW of a collection another field
    /// holds — neither is content, exactly as the renderer skips them.
    fn eq_fields(&self, a: &DbRef, b: &DbRef, fields: &[Field]) -> bool {
        fields.iter().all(|f| {
            if f.name.starts_with('#') || f.other_indexes.first() == Some(&u16::MAX) {
                return true;
            }
            let at = |r: &DbRef| DbRef {
                store_nr: r.store_nr,
                rec: r.rec,
                pos: r.pos + u32::from(f.position),
            };
            let (fa, fb) = (at(a), at(b));
            // A variant's tag is typed as its own enum, which would walk back into the
            // variant; it is one byte, and the byte is the whole of it.
            if f.name == "enum" {
                return self.store(&fa).get_byte(fa.rec, fa.pos, 0)
                    == self.store(&fb).get_byte(fb.rec, fb.pos, 0);
            }
            self.eq_content(&fa, &fb, f.content)
        })
    }

    /// Two collection slots of which one or both are ABSENT (a `vector<T>?` holding null):
    /// `Some(both absent)`, or `None` when both hold a collection to compare.
    fn eq_absent_collections(&self, a: &DbRef, b: &DbRef) -> Option<bool> {
        let na = crate::vector::is_absent_collection(a, &self.allocations);
        let nb = crate::vector::is_absent_collection(b, &self.allocations);
        (na || nb).then_some(na && nb)
    }
}
