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
        // The element is named as the schema REGISTERED it, which is what the mints spell
        // (`Stores::hash` and its siblings name a type from `types[content].name`): a second
        // source's struct of a name already taken is registered source-qualified
        // (`typedef::fill_database`), so its definition name finds the FIRST source's
        // collection — a `hash<Nm[k]>` read then probed with the other struct's key layout
        // and every lookup missed, with no diagnostic.  An element not registered yet has no
        // collection either.
        let (Type::Hash(d, ..)
        | Type::Radix(d, ..)
        | Type::Trie(d, ..)
        | Type::Sorted(d, ..)
        | Type::Index(d, ..)) = tp
        else {
            return u16::MAX;
        };
        let content = data.def(*d).known_type();
        let Some(element) = self.types.get(content as usize).map(|t| t.name.as_str()) else {
            return u16::MAX;
        };
        match tp {
            Type::Hash(_, key, _) | Type::Radix(_, key, _) => {
                let kind = if matches!(tp, Type::Hash(..)) {
                    "hash"
                } else {
                    "spatial"
                };
                let mut name = format!("{kind}<{element}[");
                self.field_name(content, key, &mut name);
                self.name(&name)
            }
            Type::Trie(_, key, _) => self.name(&format!("trie<{element}[{key}]>")),
            Type::Sorted(_, key, _) => {
                let r = self.name(&keyed_name("sorted", element, key));
                if r != u16::MAX {
                    return r;
                }
                self.name(&keyed_name("ordered", element, key))
            }
            Type::Index(_, key, _) => self.name(&keyed_name("index", element, key)),
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
