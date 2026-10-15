// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database (alloc/persistence/journal/snapshot): a store type from an IR type

//! The store's type number for an IR type: `db_type` registers what it needs, `keyed_type`
//! finds a keyed collection's.  Both read the program's definitions, so they live with the
//! compiler; the store a compiled program links only reads the types they registered
//! (@PLN191).  An extension trait, so a call site keeps its `stores.db_type(…)` spelling.

use crate::data::{Data, Type};
use crate::database::Stores;

/// Registering and finding a store type for an IR type.
pub trait StoreTypeOf {
    /// The store type of `tp`, registered on first use.
    fn db_type(&mut self, tp: &Type, data: &Data) -> u16;
    /// The registered database type of a keyed collection, or `u16::MAX` when this program
    /// never registered it.  ONE home for the names [`Parser::get_type`] registers under
    /// (`hash<T[k]>`, `spatial<T[k]>`, `trie<T[k]>`, `sorted<…>` or its `ordered<…>` spelling,
    /// `index<…>`), so the parser's lookup and the native `par` emitter, which sizes a
    /// worker's keyed result by it, cannot disagree.
    ///
    /// [`Parser::get_type`]: crate::parser::Parser
    #[must_use]
    fn keyed_type(&self, data: &Data, tp: &Type) -> u16;
}

impl StoreTypeOf for Stores {
    fn db_type(&mut self, tp: &Type, data: &Data) -> u16 {
        match tp {
            crate::data::Type::Integer(crate::narrow::IntegerSpec {
                min: minimum,
                not_null,
                ..
            }) => {
                let nullable = !not_null;
                let s = tp.size(nullable);
                if s == 1 {
                    self.byte(*minimum, nullable)
                } else if s == 2 {
                    self.short(*minimum, nullable)
                } else {
                    self.name("integer")
                }
            }
            crate::data::Type::Enum(_, false, _) => self.name("byte"),
            // #250: a nested-vector element type must resolve recursively — the
            // `_` fallback below (`def(type_def_nr(tp)).known_type`) mis-resolves
            // `vector<T>` to an unrelated default-library id (e.g. FieldValue),
            // so a 3+-deep `vector<vector<vector<X>>>` copy got a bogus type-id
            // and `copy_claims` dispatched as the wrong type → OOB panic.
            //
            // The element id comes from `Data::vector_element_type` — the one
            // derivation of that fact, shared with the parser's `vector_of` and
            // `typedef.rs::fill_database`.  Recursing through `db_type` instead
            // re-entered the Integer arm below, which knows nothing of the
            // element-side narrow forms (no `4 → int`, no `ShortRaw`) and so
            // widened every narrow element to 8-byte `integer` (loft#624 nested).
            crate::data::Type::Vector(elem, _) => {
                let e = match data.vector_element_type(elem, self) {
                    Some(e) => e,
                    None => self.db_type(elem, data),
                };
                self.vector(e)
            }
            _ => data.def(data.type_def_nr(tp)).known_type,
        }
    }

    fn keyed_type(&self, data: &Data, tp: &Type) -> u16 {
        let tp = tp.base();
        match tp {
            Type::Hash(d, key, _) | Type::Radix(d, key, _) => {
                let kind = if matches!(tp, Type::Hash(..)) {
                    "hash"
                } else {
                    "spatial"
                };
                let mut name = format!("{kind}<{}[", data.def(*d).name());
                self.field_name(data.def(*d).known_type(), key, &mut name);
                self.name(&name)
            }
            Type::Trie(d, key, _) => self.name(&format!("trie<{}[{key}]>", data.def(*d).name())),
            Type::Sorted(d, key, _) => {
                let r = self.name(&keyed_name("sorted", data.def(*d).name(), key));
                if r != u16::MAX {
                    return r;
                }
                self.name(&keyed_name("ordered", data.def(*d).name(), key))
            }
            Type::Index(d, key, _) => self.name(&keyed_name("index", data.def(*d).name(), key)),
            _ => u16::MAX,
        }
    }
}

/// `<kind><T[k1,-k2]>` — the spelling an ordered keyed collection is registered under: each
/// key field by name, a descending one prefixed `-`.
fn keyed_name(kind: &str, content: &str, key: &[(String, bool)]) -> String {
    let mut name = format!("{kind}<{content}[");
    for (k_nr, (k, asc)) in key.iter().enumerate() {
        if k_nr > 0 {
            name += ",";
        }
        if !asc {
            name += "-";
        }
        name += k;
    }
    name += "]>";
    name
}
