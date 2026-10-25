// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The C backend (@PLN200 B5, @PLN91 strand 5): loft programs written as C99 and built with
//! `cc`.  It is a part of its own, beside the Rust writer (`generation`) and the interpreter:
//! it reads the IR, the analyses in `opt` and the plans in `lower`, and names neither sibling
//! (`@C153`, `scripts/crate_layers.py`).
//!
//! It starts PLAIN (the owner's decision in the plan): every value takes the general form,
//! and no per-function decision is shared yet.  It carries what its runtime (`loft_rt.h`)
//! carries — integers, booleans, text with its formatting, the operators over them, control
//! flow, calls and returns — and refuses the rest with a [`Refusal`] that names the function
//! and the construct.  Values follow the interpreter's rules; the runtime ports
//! `src/ops.rs`.  What the C does NOT do yet is report: a division by zero the interpreter
//! raises as a recoverable error is a null here and nothing more, because how a C program
//! reaches loft's runtime is @PLN91's decision.

mod body;
mod eq;
mod files;
mod fnval;
mod show;
mod unicode;

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use std::fmt::Write as _;

/// The C runtime every unit starts with.
const RUNTIME: &str = concat!(
    include_str!("loft_cc.h"),
    include_str!("loft_store.h"),
    include_str!("loft_rt.h"),
    include_str!("loft_keys.h"),
    include_str!("loft_hash.h"),
    include_str!("loft_btree.h"),
    include_str!("loft_radix.h"),
    include_str!("loft_radix_walk.h"),
    include_str!("loft_radix_db.h"),
    include_str!("loft_image.h"),
    include_str!("loft_paged.h"),
    include_str!("loft_radix_walk.h"),
    include_str!("loft_paged_load.h"),
    include_str!("loft_keyed.h"),
    include_str!("loft_json.h"),
    include_str!("loft_io.h")
);

/// Why a function cannot be written as C yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The loft name of the function.
    pub function: String,
    /// The construct the C backend does not carry, in a few words.
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.function, self.why)
    }
}

/// The C shapes a loft value takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CType {
    /// A 64-bit integer, `int64_t`; null is `LOFT_NULL`.
    Int,
    /// A boolean, stored as `uint8_t` (0, 1, `LOFT_BOOL_NULL`).
    Bool,
    /// A text: an owned `loft_text` where it is held, a `loft_str` view where it is passed.
    Text,
    /// A reference into a store — a struct, a vector — as `loft_ref`; null has `rec == 0`.
    Ref,
    /// A plain enum value, `uint8_t`; 255 is null, and so is 0 where a value is read.
    Enum,
    /// A float, `double`; NaN is null.
    Float,
    /// A single, `float`; NaN is null.
    Single,
    /// A character, its code point as `uint32_t`; 0 is null.
    Char,
    /// A function value, `loft_fn`: a definition number and a closure record.
    Fn,
    /// A tuple: the C struct [`Cx::tuple_shapes`] numbers, held and passed by value.
    Tuple(u32),
    /// No value.
    Void,
}

/// The C type a local or a result of shape `c` is held in.
pub(crate) fn held(c: CType) -> String {
    match c {
        CType::Int => "int64_t".into(),
        CType::Bool => "uint8_t".into(),
        CType::Text => "loft_text".into(),
        CType::Ref => "loft_ref".into(),
        CType::Enum => "uint8_t".into(),
        CType::Float => "double".into(),
        CType::Single => "float".into(),
        CType::Char => "uint32_t".into(),
        CType::Fn => "loft_fn".into(),
        CType::Tuple(id) => format!("loft_tup{id}"),
        CType::Void => "void".into(),
    }
}

/// The C type a parameter of shape `c` is passed as.
pub(crate) fn passed(c: CType) -> String {
    match c {
        CType::Text => "loft_str".into(),
        other => held(other),
    }
}

/// What every function of a unit shares: the program, its stores' layouts and the tuple
/// structs the unit's functions name.
pub(crate) struct Cx<'a> {
    pub(crate) data: &'a Data,
    pub(crate) stores: &'a Stores,
    tuples: std::cell::RefCell<Vec<Vec<CType>>>,
    /// Every type the unit's deep copies walk, by type number (`loft_types`).
    copy_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Types the runtime reads the shape of — a store's root type, which `OpClearVector` asks
    /// (`Stores::clear_vector_release`) — and every type they reach: rows of the type table,
    /// never a promise that a copy of them is carried (that is `copy_types`' question).
    described: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The program names `loft_types` where no type may be registered (`OpClearVector`): an
    /// empty table is then still written, and only then — an unused one fails `-Werror`.
    names_types: std::cell::Cell<bool>,
    /// Every plain enum type whose value names the unit spells, by type number.
    enum_names: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// A copy of the stores the default walk runs in, made on first use.
    scratch: std::cell::RefCell<Option<Stores>>,
    /// Every record type's default bytes the unit names, by type number.
    defaults: std::cell::RefCell<std::collections::BTreeMap<u16, Vec<u8>>>,
    /// Every type the unit renders as text (`loft_show{tp}`), by type number.
    show_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every compile-time constant the unit reads (`loft_const{d}`), by definition number.
    consts: std::cell::RefCell<std::collections::BTreeSet<u32>>,
    /// The signatures the unit calls function values through: `loft_call{n}`.
    fn_sigs: std::cell::RefCell<Vec<(Vec<CType>, CType)>>,
    /// Every function the unit uses as a value, by definition number.
    fn_values: std::cell::RefCell<std::collections::BTreeSet<u32>>,
    /// Every type the unit compares by content (`loft_eq{tp}`), by type number.
    eq_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every keyed collection type whose key parts the unit names (`loft_keys{tp}`).
    key_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The root types a `store_persist_copy` writes an image of: one sidecar each.
    sidecars: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Whether the unit reports its stores (`store_memory()`), which names every type.
    type_names: std::cell::Cell<bool>,
    /// Every type the unit writes to or reads from a file (`loft_ser{tp}` / `loft_de{tp}`).
    ser_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The Unicode tables the unit reads (`crate::c::unicode`).
    unicode: std::cell::RefCell<std::collections::BTreeSet<String>>,
    /// The generators' frames and prototypes, written before the unit's functions.
    gen_decls: std::cell::RefCell<String>,
    /// The generators' resume and release functions, written after them.
    gen_bodies: std::cell::RefCell<String>,
}

/// The record collections beside a plain vector the C carries; each layout is described in
/// `loft_keyed.h`.  An `array` and an `ordered` are the forms a vector and a sorted take when
/// their element type is shared (`@FR-Col-Group`, a stored reference).  A `spatial` and a
/// `trie` are radix trees (`loft_radix.h`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keyed {
    Hash,
    Sorted,
    Index,
    Ordered,
    Array,
    Spatial,
    Trie,
}

impl<'a> Cx<'a> {
    fn new(data: &'a Data, stores: &'a Stores) -> Cx<'a> {
        Cx {
            data,
            stores,
            tuples: std::cell::RefCell::new(Vec::new()),
            copy_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            described: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            names_types: std::cell::Cell::new(false),
            enum_names: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            scratch: std::cell::RefCell::new(None),
            defaults: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            show_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            consts: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            fn_sigs: std::cell::RefCell::new(Vec::new()),
            fn_values: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            eq_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            key_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            sidecars: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            type_names: std::cell::Cell::new(false),
            ser_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            unicode: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            gen_decls: std::cell::RefCell::new(String::new()),
            gen_bodies: std::cell::RefCell::new(String::new()),
        }
    }

    /// The kind of record collection type `tp` and its element type, or `None` for any other
    /// type — a plain vector among them.
    pub(crate) fn keyed(&self, tp: u16) -> Option<(Keyed, u16)> {
        use crate::database::Parts;
        match self.stores.types.get(tp as usize).map(|t| &t.parts)? {
            Parts::Hash(c, _) => Some((Keyed::Hash, *c)),
            Parts::Sorted(c, _) => Some((Keyed::Sorted, *c)),
            Parts::Index(c, _) => Some((Keyed::Index, *c)),
            Parts::Ordered(c, _) => Some((Keyed::Ordered, *c)),
            Parts::Array(c) => Some((Keyed::Array, *c)),
            Parts::Radix(c, _) => Some((Keyed::Spatial, *c)),
            Parts::Trie(c, _) => Some((Keyed::Trie, *c)),
            _ => None,
        }
    }

    /// The C name of keyed collection type `tp`'s key parts (`loft_keys{tp}`) and their count.
    pub(crate) fn keys_of(&self, tp: u16) -> (String, usize) {
        self.key_types.borrow_mut().insert(tp);
        (
            format!("loft_keys{tp}"),
            self.stores.types[tp as usize].keys.len(),
        )
    }

    /// The collection type a keyed local's store is minted as (`gen_keyed_null`: the
    /// `database.hash/sorted/index/spatial/trie` registration of its content and key), asked of
    /// a scratch copy of the stores so the real table is never written; `None` for a type the
    /// parse did not register.
    pub(crate) fn keyed_local_type(&self, tp: &Type) -> Option<u16> {
        let mut scratch = self.scratch.borrow_mut();
        let s = scratch.get_or_insert_with(|| {
            let mut s = self.stores.clone();
            s.max = 0;
            s
        });
        let known = |td: u32| self.data.def(td).known_type();
        let nr = match tp.base() {
            Type::Sorted(td, key, _) => s.sorted(known(*td), key),
            Type::Hash(td, key, _) => s.hash(known(*td), key),
            Type::Index(td, key, _) => s.index(known(*td), key),
            Type::Radix(td, key, _) => s.spatial(known(*td), key),
            Type::Trie(td, key, _) => s.trie(known(*td), key),
            _ => return None,
        };
        (usize::from(nr) < self.stores.types.len()).then_some(nr)
    }

    /// The root type a store holding a value of `tp` is minted as — a keyed local's collection,
    /// a record's type, a vector's `main_vector<T>` — or `None`.
    pub(crate) fn root_type(&self, tp: &Type, source: u16) -> Option<u16> {
        match tp.base() {
            Type::Reference(d, _) | Type::Enum(d, true, _) => Some(self.data.def(*d).known_type()),
            Type::Vector(elm, _) => {
                let nr = self.data.owned_vector_type(elm, source);
                (nr != u16::MAX).then_some(nr)
            }
            other => self.keyed_local_type(other),
        }
    }

    /// Can a value of `tp` hold a stored reference (`Parts::DbRef`) — a field an image's
    /// loader must remap to the slot it lands in (`rebase_stored_refs`)?
    pub(crate) fn holds_stored_refs(&self, tp: u16) -> bool {
        use crate::database::Parts;
        fn walk(s: &Stores, tp: u16, seen: &mut Vec<u16>) -> bool {
            if seen.contains(&tp) || usize::from(tp) >= s.types.len() {
                return false;
            }
            seen.push(tp);
            match &s.types[tp as usize].parts {
                Parts::DbRef => true,
                Parts::Struct(f) | Parts::EnumValue(_, f) => {
                    f.iter().any(|f| walk(s, f.content, seen))
                }
                Parts::Enum(v) => v.iter().any(|(t, _)| *t != u16::MAX && walk(s, *t, seen)),
                Parts::Vector(c)
                | Parts::Sorted(c, _)
                | Parts::Hash(c, _)
                | Parts::Index(c, _)
                | Parts::Ordered(c, _)
                | Parts::Array(c)
                | Parts::ChildRec(c)
                | Parts::Radix(c, _)
                | Parts::Trie(c, _) => walk(s, *c, seen),
                _ => false,
            }
        }
        walk(self.stores, tp, &mut Vec::new())
    }

    /// The C name of the sidecar a `store_persist_copy` of a store minted as `tp` writes.
    pub(crate) fn sidecar_of(&self, tp: u16) -> String {
        self.sidecars.borrow_mut().insert(tp);
        format!("&loft_sidecar{tp}")
    }

    /// The type-name table `store_memory()` reads, by type number: the name `Stores::types`
    /// gives each, so a store's line names its type as the interpreter's does.
    pub(crate) fn type_names(&self) -> &'static str {
        self.type_names.set(true);
        "loft_tnames, loft_ntnames"
    }

    fn type_name_table(&self) -> String {
        if !self.type_names.get() {
            return String::new();
        }
        let names: Vec<String> = self
            .stores
            .types
            .iter()
            .map(|t| body::c_string(&t.name))
            .collect();
        format!(
            "static const char *const loft_tnames[] = {{ {} }};\nstatic const uint32_t loft_ntnames = {};\n",
            names.join(", "),
            names.len()
        )
    }

    /// The sidecars: each type's `.dschema` text split where its slot line goes, and the
    /// layout the loader gates on.  Byte arrays rather than literals: a layout dump passes the
    /// 4095 characters a C99 literal is guaranteed.
    fn sidecar_table(&self) -> String {
        let bytes = |t: &str| -> String {
            let mut v: Vec<String> = t.bytes().map(|b| b.to_string()).collect();
            v.push("0".into());
            format!("{{ {} }}", v.join(", "))
        };
        let mut out = String::new();
        for &tp in self.sidecars.borrow().iter() {
            let id = crate::schema_sidecar::LayoutIdentity::of_store(self.stores, u16::MAX, tp);
            let text = id.to_sidecar();
            let marker = format!("slot={}\n", u16::MAX);
            let (before, after) = text.split_once(&marker).unwrap_or((&text, ""));
            let _ = writeln!(out, "static const char loft_scb{tp}[] = {};", bytes(before));
            let _ = writeln!(out, "static const char loft_sca{tp}[] = {};", bytes(after));
            let _ = writeln!(
                out,
                "static const char loft_scd{tp}[] = {};",
                bytes(&id.dump)
            );
            let _ = writeln!(
                out,
                "static const loft_sidecar loft_sidecar{tp} = {{ {tp}, loft_scb{tp}, loft_sca{tp}, UINT64_C({}), loft_scd{tp}, {} }};",
                id.layout_hash,
                body::c_string(&self.stores.release().oldest)
            );
        }
        out
    }

    /// The C name of plain enum type `tp`'s value names, indexed by value (1-based).
    pub(crate) fn enum_names(&self, tp: u16) -> String {
        self.enum_names.borrow_mut().insert(tp);
        format!("loft_enum{tp}")
    }

    /// The C name of record type `tp`'s default bytes — what the interpreter's default walk
    /// writes into a new record of it (`Stores::set_default_value`), computed by running that
    /// walk on a copy of the stores, so the two cannot disagree.
    pub(crate) fn default_image(&self, tp: u16) -> (String, u32) {
        let size = u32::from(self.stores.size(tp));
        if !self.defaults.borrow().contains_key(&tp) {
            let mut scratch = self.scratch.borrow_mut();
            // The clone keeps the schema and no stores; `max` still counts the original's.
            let s = scratch.get_or_insert_with(|| {
                let mut s = self.stores.clone();
                s.max = 0;
                s
            });
            let r = s.database(size.div_ceil(8) + 1);
            s.set_default_value(tp, &r);
            let bytes = s.store(&r).read_span(r.rec, r.pos, size).to_vec();
            self.defaults.borrow_mut().insert(tp, bytes);
        }
        (format!("loft_dflt{tp}"), size)
    }

    /// The C shape of `tp`, nullable or not — a null is held in the same shape — or `None`.
    pub(crate) fn ctype(&self, tp: &Type) -> Option<CType> {
        match tp.base() {
            // A narrow integer is held as a plain one: its range is enforced where a value
            // enters it (`OpRangeDefault`) and where a field stores it.
            Type::Integer(_) => Some(CType::Int),
            Type::Boolean => Some(CType::Bool),
            Type::Float => Some(CType::Float),
            Type::Single => Some(CType::Single),
            Type::Character => Some(CType::Char),
            Type::Function(..) => Some(CType::Fn),
            Type::Text(_) => Some(CType::Text),
            Type::Reference(..)
            | Type::Vector(..)
            | Type::Hash(..)
            | Type::Sorted(..)
            | Type::Index(..)
            | Type::Radix(..)
            | Type::Trie(..)
            | Type::Iterator(..)
            | Type::Enum(_, true, _) => Some(CType::Ref),
            Type::Enum(_, false, _) => Some(CType::Enum),
            Type::Tuple(elems) => {
                // A text inside a tuple would be copied with the tuple and freed twice.
                let shapes = elems
                    .iter()
                    .map(|e| {
                        self.ctype(e)
                            .filter(|c| !matches!(c, CType::Void | CType::Fn))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(CType::Tuple(self.tuple_id(shapes)))
            }
            Type::Void => Some(CType::Void),
            _ => None,
        }
    }

    /// Does a tuple of these shapes hold a text, which it owns and must copy and free?
    pub(crate) fn tuple_has_text(&self, id: u32) -> bool {
        self.tuple_shapes(id).contains(&CType::Text)
    }

    /// Is tuple struct `id` plain scalars at every depth — no text, no reference, no function
    /// value — so that a struct copy of it is a whole move?
    pub(crate) fn tuple_is_plain(&self, id: u32) -> bool {
        self.tuple_shapes(id).into_iter().all(|c| match c {
            CType::Int | CType::Bool | CType::Enum | CType::Float | CType::Single | CType::Char => {
                true
            }
            CType::Tuple(inner) => self.tuple_is_plain(inner),
            _ => false,
        })
    }

    /// The number of the tuple struct with these element shapes, made on first use.
    pub(crate) fn tuple_id(&self, shapes: Vec<CType>) -> u32 {
        let mut t = self.tuples.borrow_mut();
        if let Some(i) = t.iter().position(|s| *s == shapes) {
            return i as u32;
        }
        t.push(shapes);
        (t.len() - 1) as u32
    }

    /// The element shapes of tuple struct `id`.
    pub(crate) fn tuple_shapes(&self, id: u32) -> Vec<CType> {
        self.tuples.borrow()[id as usize].clone()
    }

    /// The initial value of a local of shape `c`: its null.
    pub(crate) fn initial(&self, c: CType) -> String {
        match c {
            CType::Int => "LOFT_NULL".into(),
            CType::Bool => "LOFT_BOOL_NULL".into(),
            CType::Text => "{ 0, 0, 0 }".into(),
            CType::Ref => "{ 0xFFFF, 0, 0 }".into(),
            CType::Enum => "255".into(),
            CType::Float => "(double) NAN".into(),
            CType::Single => "(float) NAN".into(),
            CType::Char => "0".into(),
            CType::Fn => "{ LOFT_NULL, { 0xFFFF, 0, 0 } }".into(),
            CType::Tuple(id) => {
                let fields: Vec<String> = self
                    .tuple_shapes(id)
                    .into_iter()
                    .map(|e| self.initial(e))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            CType::Void => String::new(),
        }
    }

    /// Register type `tp`, and every type it reaches, in the unit's copy table — or refuse a
    /// record holding a linked group, which a field-by-field copy would split.
    pub(crate) fn copy_type(&self, tp: u16) -> Result<(), String> {
        use crate::database::Parts;
        if self.copy_types.borrow().contains(&tp) {
            return Ok(());
        }
        let t = self
            .stores
            .types
            .get(tp as usize)
            .ok_or("an unknown type")?;
        self.copy_types.borrow_mut().insert(tp);
        let r = match &t.parts {
            // A linked group's members name one record set, which a field-by-field copy
            // would split into one per member.
            Parts::Struct(fields) | Parts::EnumValue(_, fields)
                if fields.iter().any(|f| !f.other_indexes().is_empty()) =>
            {
                Err("a copy of a record holding a linked group".to_string())
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().try_for_each(|f| self.copy_type(f.content))
            }
            Parts::Vector(c)
            | Parts::ChildRec(c)
            | Parts::Sorted(c, _)
            | Parts::Hash(c, _)
            | Parts::Index(c, _)
            | Parts::Ordered(c, _)
            | Parts::Array(c)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _) => self.copy_type(*c),
            Parts::Enum(values) => values
                .iter()
                .filter(|(v, _)| *v != u16::MAX)
                .try_for_each(|(v, _)| self.copy_type(*v)),
            Parts::Base
            | Parts::Byte(..)
            | Parts::Short(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::ShortRaw(..)
            | Parts::DbRef => Ok(()),
        };
        if r.is_err() {
            self.copy_types.borrow_mut().remove(&tp);
        }
        r
    }

    /// The emitted code names the type table whether or not any type is registered.
    pub(crate) fn name_types(&self) {
        self.names_types.set(true);
    }

    /// Put type `tp`, and every type it reaches, in the type table as a description.
    pub(crate) fn describe_type(&self, tp: u16) {
        use crate::database::Parts;
        let Some(t) = self.stores.types.get(tp as usize) else {
            return;
        };
        if !self.described.borrow_mut().insert(tp) {
            return;
        }
        match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    self.describe_type(f.content);
                }
            }
            Parts::Vector(c)
            | Parts::ChildRec(c)
            | Parts::Sorted(c, _)
            | Parts::Hash(c, _)
            | Parts::Index(c, _)
            | Parts::Ordered(c, _)
            | Parts::Array(c) => self.describe_type(*c),
            Parts::Enum(values) => {
                for (v, _) in values {
                    if *v != u16::MAX {
                        self.describe_type(*v);
                    }
                }
            }
            _ => {}
        }
    }

    /// May a value of type `tp` move to another store whole (@FR-H-SwapIn)?  Not when its tree
    /// holds a stored reference, which could name the store it moves from
    /// (`Stores::tree_holds_no_stored_refs`).
    pub(crate) fn swap_safe(&self, tp: u16) -> bool {
        fn walk(stores: &Stores, tp: u16, seen: &mut Vec<u16>) -> bool {
            use crate::database::Parts;
            if seen.contains(&tp) {
                return true;
            }
            seen.push(tp);
            match stores.types.get(tp as usize).map(|t| &t.parts) {
                Some(Parts::Struct(fields) | Parts::EnumValue(_, fields)) => fields
                    .iter()
                    .all(|f| f.target == u16::MAX && walk(stores, f.content, seen)),
                Some(Parts::Enum(values)) => values
                    .iter()
                    .all(|(v, _)| *v == u16::MAX || walk(stores, *v, seen)),
                Some(
                    Parts::Vector(c)
                    | Parts::Sorted(c, _)
                    | Parts::Array(c)
                    | Parts::Ordered(c, _)
                    | Parts::Hash(c, _)
                    | Parts::Index(c, _)
                    | Parts::Radix(c, _)
                    | Parts::Trie(c, _)
                    | Parts::ChildRec(c),
                ) => walk(stores, *c, seen),
                Some(Parts::DbRef) | None => false,
                Some(_) => true,
            }
        }
        walk(self.stores, tp, &mut Vec::new())
    }

    /// A struct's field rows, `loft_fields{tp}`: each field's byte position, its type, whether it
    /// is a view member of a linked group (it borrows its records) and whether it is hidden (a
    /// `#`-named field).
    fn field_rows(tp: u16, fields: &[crate::database::Field]) -> String {
        let list: Vec<String> = fields
            .iter()
            .map(|f| {
                let view = f.other_indexes().first() == Some(&u16::MAX);
                let hidden = u8::from(f.name.starts_with('#'));
                format!(
                    "{{ {}, {}, {}, {hidden} }}",
                    f.position,
                    f.content,
                    u8::from(view)
                )
            })
            .collect();
        format!(
            "static const loft_fld loft_fields{tp}[] = {{ {} }};",
            if list.is_empty() {
                "{ 0, 0, 0, 0 }".into()
            } else {
                list.join(", ")
            }
        )
    }

    /// The copy table: one `loft_tdesc` per registered type, at its type number.
    fn copy_table(&self) -> String {
        use crate::database::Parts;
        let types: std::collections::BTreeSet<u16> = self
            .copy_types
            .borrow()
            .union(&self.described.borrow())
            .copied()
            .collect();
        if types.is_empty() {
            return if self.names_types.get() {
                "static const loft_tdesc *const loft_types = 0;\nstatic const uint32_t loft_ntypes = 0;\n".to_string()
            } else {
                String::new()
            };
        }
        let mut out = String::new();
        let mut rows = Vec::new();
        for &tp in &types {
            let t = &self.stores.types[tp as usize];
            let size = self.stores.size(tp);
            let row = match &t.parts {
                Parts::Base if tp == 5 || t.name == "text" => {
                    format!("{{ LOFT_T_TEXT, {size}, 0, 0, 0, 0 }}")
                }
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    let _ = writeln!(out, "{}", Self::field_rows(tp, fields));
                    format!(
                        "{{ LOFT_T_STRUCT, {size}, 0, {}, loft_fields{tp}, 0 }}",
                        fields.len()
                    )
                }
                Parts::Vector(c) | Parts::Sorted(c, _) => {
                    format!("{{ LOFT_T_VECTOR, {size}, {c}, 0, 0, 0 }}")
                }
                Parts::Hash(c, _) => format!("{{ LOFT_T_HASH, {size}, {c}, 0, 0, 0 }}"),
                Parts::Index(c, _) => format!("{{ LOFT_T_INDEX, {size}, {c}, 0, 0, 0 }}"),
                Parts::Radix(c, _) => format!("{{ LOFT_T_RADIX, {size}, {c}, 0, 0, 0 }}"),
                Parts::Trie(c, _) => format!("{{ LOFT_T_TRIE, {size}, {c}, 0, 0, 0 }}"),
                Parts::Ordered(c, _) | Parts::Array(c) => {
                    format!("{{ LOFT_T_ORDERED, {size}, {c}, 0, 0, 0 }}")
                }
                Parts::ChildRec(c) => format!("{{ LOFT_T_CHILD, {size}, {c}, 0, 0, 0 }}"),
                Parts::Enum(values) if values.iter().any(|(v, _)| *v != u16::MAX) => {
                    let list: Vec<String> = values.iter().map(|(v, _)| v.to_string()).collect();
                    let _ = writeln!(
                        out,
                        "static const uint16_t loft_variants{tp}[] = {{ {} }};",
                        list.join(", ")
                    );
                    format!(
                        "{{ LOFT_T_ENUM, {size}, 0, {}, 0, loft_variants{tp} }}",
                        values.len()
                    )
                }
                Parts::DbRef => format!("{{ LOFT_T_DBREF, {size}, 0, 0, 0, 0 }}"),
                _ => format!("{{ LOFT_T_SCALAR, {size}, 0, 0, 0, 0 }}"),
            };
            // `owns_heap`: the Rust store's own answer (`Stores::type_owns_heap`), never re-derived;
            // a hash's or an index's key parts, which its copy re-files by; `linked`, the type's
            // own flag.
            let keys = if matches!(
                t.parts,
                Parts::Hash(..) | Parts::Index(..) | Parts::Radix(..) | Parts::Trie(..)
            ) {
                let (name, n) = self.keys_of(tp);
                format!("{name}, {n}")
            } else {
                "0, 0".into()
            };
            let row = format!(
                "{}, {}, {keys}, {} }}",
                row.strip_suffix(" }").unwrap_or(&row),
                u8::from(self.stores.type_owns_heap(tp)),
                u8::from(self.stores.is_linked(tp))
            );
            rows.push(format!("    [{tp}] = {row},"));
        }
        let max = types.iter().max().copied().unwrap_or(0);
        let _ = writeln!(
            out,
            "static const loft_tdesc loft_types[{}] = {{\n{}\n}};",
            u32::from(max) + 1,
            rows.join("\n")
        );
        let _ = writeln!(
            out,
            "static const uint32_t loft_ntypes = {};",
            u32::from(max) + 1
        );
        out
    }

    /// The tuple structs, each after the ones it holds, and the default images.
    fn typedefs(&self) -> String {
        let mut out = String::new();
        // The table names the key parts of the hashes in it, so it is written first.
        let table = self.copy_table();
        for tp in self.key_types.borrow().iter() {
            let parts: Vec<String> = self.stores.types[*tp as usize]
                .keys
                .iter()
                .map(|k| format!("{{ {}, {}, {} }}", k.type_nr, k.position, k.start))
                .collect();
            let _ = writeln!(
                out,
                "static const loft_key loft_keys{tp}[] = {{ {} }};",
                if parts.is_empty() {
                    "{ 0, 0, 0 }".into()
                } else {
                    parts.join(", ")
                }
            );
        }
        out.push_str(&table);
        out.push_str(&self.sidecar_table());
        out.push_str(&self.type_name_table());
        for tp in self.enum_names.borrow().iter() {
            let count = match &self.stores.types[*tp as usize].parts {
                crate::database::Parts::Enum(values) => values.len(),
                _ => 0,
            };
            let names: Vec<String> = (1..=count)
                .map(|v| body::c_string(self.stores.enum_val(*tp, v as u8)))
                .collect();
            let _ = writeln!(
                out,
                "static const char *const loft_enum{tp}[] = {{ \"\", {} }};",
                names.join(", ")
            );
        }
        for (tp, bytes) in self.defaults.borrow().iter() {
            let list: Vec<String> = bytes.iter().map(u8::to_string).collect();
            let _ = writeln!(
                out,
                "static const uint8_t loft_dflt{tp}[{}] = {{ {} }};",
                bytes.len().max(1),
                if list.is_empty() {
                    "0".to_string()
                } else {
                    list.join(", ")
                }
            );
        }
        for (id, shapes) in self.tuples.borrow().iter().enumerate() {
            let fields: Vec<String> = shapes
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} f{i};", held(*c)))
                .collect();
            let _ = writeln!(
                out,
                "typedef struct {{ {} }} loft_tup{id};",
                fields.join(" ")
            );
        }
        out.push_str(&self.show_code());
        out.push_str(&self.eq_code());
        out.push_str(&self.ser_code());
        out.push_str(&self.unicode_code());
        out.push_str(&self.const_code());
        out
    }

    /// The C name of compile-time constant `d`'s vector, or a refusal when it was not built.
    pub(crate) fn const_ref(&self, d: u32) -> Result<String, String> {
        if self.data.def(d).const_ref.is_none() {
            return Err("a constant that was not built".into());
        }
        self.consts.borrow_mut().insert(d);
        Ok(format!("loft_const{d}"))
    }

    /// Each constant's store, as words, and `loft_init`, which loads them before `main` runs.
    fn const_code(&self) -> String {
        let mut out = String::new();
        let mut init = String::from("static void loft_init(void) {\n");
        for &d in self.consts.borrow().iter() {
            let Some(r) = self.data.def(d).const_ref else {
                continue;
            };
            let store = self.stores.store(&r);
            let mut n = store.capacity_words();
            while n > 1 && store.get_u32_raw(n - 1, 0) == 0 && store.get_u32_raw(n - 1, 4) == 0 {
                n -= 1;
            }
            let words: Vec<String> = (0..n)
                .flat_map(|w| [store.get_u32_raw(w, 0), store.get_u32_raw(w, 4)])
                .map(|x| format!("{x}u"))
                .collect();
            let _ = writeln!(
                out,
                "static const uint32_t loft_cwords{d}[] = {{ {} }};\nstatic loft_ref loft_const{d};",
                words.join(", ")
            );
            let _ = writeln!(
                init,
                "    loft_const{d} = loft_const_store(loft_cwords{d}, {n});"
            );
        }
        init.push_str("}\n");
        out.push_str(&init);
        out
    }
}

/// `tp` in a word, for a refusal: its variant's name.
pub(crate) fn type_word(tp: &Type) -> String {
    let full = format!("{:?}", tp.base());
    full.split(['(', ' ', '{'])
        .next()
        .unwrap_or("type")
        .to_string()
}

/// The C name of definition `d`: the identifier every compiled target gives it.
#[must_use]
pub fn c_name(data: &Data, d: u32) -> String {
    let dups = crate::opt::names::duplicate_fn_names(data);
    crate::opt::names::disambiguated_fn_ident(&dups, data.def(d))
}

/// The C runtime function that implements native `name` (a stored `n_…` name), if any.
pub(crate) fn native_c(name: &str) -> Option<&'static str> {
    match name {
        "n_assert" => Some("loft_assert"),
        "n_panic" => Some("loft_panic"),
        "n_exit" => Some("loft_exit"),
        "n_protect_store_frees" => Some("loft_protect"),
        "t_4text_to_lowercase" | "t_4text_to_uppercase" => Some("loft_unicode"),
        "t_9character_is_lowercase"
        | "t_9character_is_uppercase"
        | "t_9character_is_numeric"
        | "t_9character_is_alphanumeric"
        | "t_9character_is_alphabetic"
        | "t_9character_is_whitespace"
        | "t_9character_is_control" => Some("loft_unicode"),
        "n_unprotect_store_frees" => Some("loft_unprotect"),
        "n_ticks" => Some("loft_ticks"),
        "n_arguments" => Some("loft_arguments"),
        "n_vector_sum_int" => Some("loft_vector_sum_int"),
        "n_text_from_bytes" => Some("loft_text_from_bytes"),
        "n_text_from_byte_range" => Some("loft_text_from_byte_range"),
        "t_4text_starts_with" => Some("loft_starts_with"),
        "t_4text_ends_with" => Some("loft_ends_with"),
        "t_4text_find" => Some("loft_find"),
        "t_4text_rfind" => Some("loft_rfind"),
        "t_4text_contains" => Some("loft_contains"),
        "t_4text_trim" => Some("loft_trim"),
        "t_4text_trim_start" => Some("loft_trim_start"),
        "t_4text_trim_end" => Some("loft_trim_end"),
        "t_4text_replace" => Some("loft_replace"),
        "t_4text_byte_at" => Some("loft_byte_at"),
        "n_hash_sorted" => Some("loft_hash_sorted"),
        "n_hash_unsorted" => Some("loft_hash_unsorted"),
        "n_radix_sorted" => Some("loft_radix_sorted"),
        "n_spatial_range" => Some("loft_spatial_range"),
        "n_trie_prefix" => Some("loft_trie_prefix"),
        "n_file_map" => Some("loft_file_map"),
        "n_store_persist_copy" => Some("loft_image_write"),
        "n_store_load" => Some("loft_image_load"),
        "n_store_memory" => Some("loft_store_memory"),
        "n_store_verify" => Some("loft_store_verify"),
        "n_store_load_key"
        | "n_store_load_key_text"
        | "n_store_load_keys"
        | "n_store_load_keys_text"
        | "n_store_load_prefix"
        | "n_store_load_box"
        | "n_store_load_range" => Some("loft_pg_load_keys"),
        "n_store_upgrade_owed_next" => Some("loft_store_upgrade_owed_next"),
        "n_store_upgrade_owed_record" => Some("loft_store_upgrade_owed_record"),
        "n_store_upgrade_owed_old" => Some("loft_store_upgrade_owed_old"),
        "n_store_upgrade_owed_from" => Some("loft_store_upgrade_owed_from"),
        "n_store_upgrade_owed_finish" => Some("loft_store_upgrade_owed_finish"),
        "n_store_upgrade_routine" => Some("loft_store_upgrade_routine"),
        "t_4hash_len" => Some("loft_hash_len"),
        "n_read_bytes" => Some("loft_read_bytes"),
        "n_write_bytes" => Some("loft_write_bytes"),
        "n_write_text_raw" => Some("loft_write_text_raw"),
        "n_list_dir" => Some("loft_list_dir"),
        "n_mtime" => Some("loft_mtime"),
        "n_is_dir" => Some("loft_is_dir_at"),
        "n_is_file" => Some("loft_is_file_at"),
        "n_os_temp_dir" => Some("loft_os_temp_dir"),
        "n_source_dir" => Some("loft_source_dir"),
        "n_path_sep" => Some("loft_path_sep"),
        "n_env_variable" => Some("loft_env_variable"),
        "n_now" => Some("loft_now"),
        "n_parallel_queue"
        | "n_parallel_queue_text"
        | "n_parallel_queue_ref"
        | "n_parallel_queue_narrow"
        | "n_parallel_queue_fn"
        | "n_parallel_discard"
        | "n_parallel_buf_get"
        | "n_parallel_buf_get_float"
        | "n_parallel_buf_get_single"
        | "n_parallel_buf_get_ref"
        | "n_parallel_buf_get_fn"
        | "n_parallel_buf_get_text"
        | "n_parallel_buf_get_narrow"
        | "n_parallel_buf_drop"
        | "n_parallel_buf_drop_text"
        | "n_parallel_buf_drop_narrow"
        | "n_parallel_buf_drop_fn"
        | "n_parallel_buf_drop_ref" => Some("loft_par"),
        _ => None,
    }
}

/// Is `d` an operator, which a call site writes rather than a function it calls?
pub(crate) fn is_operator(data: &Data, d: u32) -> bool {
    data.def(d).name().starts_with("Op")
}

/// C99 for `roots` and every loft function they reach: the runtime, a prototype per function
/// and each body.  A function or construct the backend does not carry refuses the whole unit.
///
/// # Errors
/// The first [`Refusal`], the roots' own first.
pub fn emit_unit(data: &Data, stores: &Stores, roots: &[u32]) -> Result<String, Refusal> {
    // A closure's drop cascade (`OpDropFnRef`) is called through the value, never by name: each
    // lambda the program reaches brings its record's cascade along.
    let mut all: Vec<u32> = roots.to_vec();
    for d in crate::opt::names::reachable_functions(data, roots) {
        let cascade = data.drop_cascade_nr(data.def(d).closure_record());
        if cascade != u32::MAX && !all.contains(&cascade) {
            all.push(cascade);
        }
    }
    let mut fns: Vec<u32> = crate::opt::names::reachable_functions(data, &all)
        .into_iter()
        .filter(|&d| {
            matches!(data.def(d).def_type(), DefType::Function)
                && !is_operator(data, d)
                && native_c(data.def(d).name()).is_none()
        })
        .collect();
    // The roots first, so a refusal names the function asked for before one it reaches.
    fns.sort_unstable_by_key(|d| (!roots.contains(d), *d));
    let cx = Cx::new(data, stores);
    let mut protos = String::new();
    let mut bodies = String::new();
    for &d in &fns {
        let (sig, body) = function(&cx, d).map_err(|why| Refusal {
            function: data.def(d).original_name().clone(),
            why,
        })?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
    }
    let dispatch = cx.dispatch_code(&fns).map_err(|why| Refusal {
        function: data.def(roots[0]).original_name().clone(),
        why,
    })?;
    // `file_map` is the one producer of a foreign store (@PLN174): a unit that does not reach
    // it compiles `loft_at` without the foreign record's test.
    let mut out = String::new();
    if crate::opt::names::reachable_functions(data, &all)
        .iter()
        .any(|&d| data.def(d).name() == "n_file_map")
    {
        out.push_str("#define LOFT_FOREIGN 1\n");
    }
    out.push_str(RUNTIME);
    out.push('\n');
    out.push_str(&cx.typedefs());
    out.push_str(&cx.gen_decls.borrow());
    out.push_str(&protos);
    out.push_str(&dispatch);
    out.push_str(&cx.drop_code(&fns));
    out.push_str(&cx.gen_bodies.borrow());
    out.push_str(&bodies);
    Ok(out)
}

/// C99 for the whole program: [`emit_unit`] from `main`, and the C `main` that calls it.
///
/// # Errors
/// A program without a `main` that takes nothing, or the first [`Refusal`].
///
/// `expected` names the messages a test is expected to fail with (one must appear in its
/// failure), or none for a test that must pass.
pub fn emit_program(
    data: &Data,
    stores: &Stores,
    expected: &dyn Fn(&str) -> Vec<String>,
) -> Result<String, Refusal> {
    let main = data.def_nr("n_main");
    if main >= data.definitions() {
        return emit_tests(data, stores, expected);
    }
    if !data.def(main).attributes().is_empty() {
        return Err(Refusal {
            function: "main".into(),
            why: "a main that takes arguments".into(),
        });
    }
    let mut out = emit_unit(data, stores, &[main])?;
    let _ = write!(
        out,
        "\nint main(int argc, char **argv) {{\n    {}\n    loft_start(argc, argv);\n    {}();\n    return 0;\n}}\n",
        anchor(stores),
        c_name(data, main)
    );
    Ok(out)
}

/// The statement a generated `main` starts with: the directory a relative path resolves against
/// (the program's own, as the interpreter's `Stores::resolve_path` anchors it) and whether
/// the program anchors there at all (`#cwd` says no).
fn anchor(stores: &Stores) -> String {
    format!(
        "loft_set_anchor({}, {});",
        body::c_string(&stores.source_dir),
        i32::from(stores.program_relative)
    )
}

/// The signature and the body of function `d`.
fn function(cx: &Cx, d: u32) -> Result<(String, String), String> {
    let data = cx.data;
    let def = data.def(d);
    if def.code() == &Value::Null {
        return Err(format!("native {}", def.original_name()));
    }
    let ret = cx
        .ctype(def.returned())
        .ok_or_else(|| format!("returns {}", type_word(def.returned())))?;
    let vars = def.variables();
    // The parameters are the argument variables, in number order: `ref_return` can promote a
    // local into the return buffer's place, so parameter i is not always variable i.
    let args = vars.arguments();
    if args.len() != def.attributes().len() {
        return Err("parameters that do not match their arguments".into());
    }
    let mut shapes = Vec::with_capacity(vars.count() as usize);
    for v in 0..vars.count() {
        // A `&` parameter has its target's shape, passed as a pointer to it.
        let tp = match vars.tp(v).base() {
            Type::RefVar(inner) => inner,
            tp => tp,
        };
        // A name the first pass met before its declaration became a local of no type; the
        // second pass resolved it, so nothing binds it and the scope pass never scoped it.
        // It is declared as a placeholder nothing reads.
        if tp.is_unknown() && !args.contains(&v) && vars.scope(v) == u16::MAX {
            shapes.push(CType::Int);
            continue;
        }
        let c = cx.ctype(tp).filter(|c| *c != CType::Void).ok_or_else(|| {
            let what = if args.contains(&v) {
                "parameter"
            } else {
                "local"
            };
            format!("{what} of type {}", type_word(tp))
        })?;
        shapes.push(c);
    }
    let params: Vec<String> = args
        .iter()
        .map(|&a| {
            let c = shapes[a as usize];
            let name = body::var_name(vars.name(a), a);
            if matches!(vars.tp(a).base(), Type::RefVar(_)) {
                format!("{} *{name}", held(c))
            } else {
                format!("{} {name}", passed(c))
            }
        })
        .collect();
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let sig = format!("static {} {}({params})", held(ret), c_name(data, d));
    if let Type::Iterator(elem, _) = def.returned().base() {
        // A tuple rides the frame and the yield as a struct copy, which is a move only for
        // plain scalars: a text or a record a tuple yields is handed over (`(G-Own)`).
        let elem = cx
            .ctype(elem)
            .filter(|c| match c {
                CType::Void => false,
                CType::Tuple(id) => cx.tuple_is_plain(*id),
                _ => true,
            })
            .ok_or_else(|| format!("a generator of {}", type_word(elem)))?;
        let (body, parts) = body::FnWriter::new(cx, d, shapes, CType::Void)
            .yielding(elem)
            .write_gen()?;
        cx.gen_decls.borrow_mut().push_str(&parts.decls);
        cx.gen_bodies.borrow_mut().push_str(&parts.bodies);
        return Ok((sig, body));
    }
    let body = body::FnWriter::new(cx, d, shapes, ret).write()?;
    Ok((sig, body))
}

/// The program's tests, the way `loft --tests` picks them: its own zero-parameter functions,
/// only the `test_*` ones when it names any.
fn test_functions(data: &Data) -> Vec<u32> {
    let mut fns: Vec<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), DefType::Function)
                && def.source == crate::data::MAIN_SOURCE
                && def.name().starts_with("n_")
                && !def.name().starts_with("n___lambda_")
                && def.attributes().is_empty()
                && !matches!(def.returned().base(), Type::Iterator(..))
        })
        .collect();
    if fns
        .iter()
        .any(|&d| data.def(d).name().starts_with("n_test_"))
    {
        fns.retain(|&d| data.def(d).name().starts_with("n_test_"));
    }
    fns
}

/// A program without `main` is run as its tests: a C `main` that runs each, catches a failed
/// assert, and exits 1 when any test failed — `loft --tests`'s exit status.
fn emit_tests(
    data: &Data,
    stores: &Stores,
    expected: &dyn Fn(&str) -> Vec<String>,
) -> Result<String, Refusal> {
    let tests = test_functions(data);
    if tests.is_empty() {
        return Err(Refusal {
            function: "main".into(),
            why: "neither a main nor a test".into(),
        });
    }
    let mut out = emit_unit(data, stores, &tests)?;
    let _ = write!(
        out,
        "\nint main(int argc, char **argv) {{\n    int failed = 0;\n    {}\n    loft_start(argc, argv);\n    loft_in_test = 1;\n",
        anchor(stores)
    );
    for &d in &tests {
        let name = data.def(d).original_name();
        let subs = expected(&name);
        if subs.is_empty() {
            let _ = writeln!(
                out,
                "    if (setjmp(loft_test_jmp) == 0) {}(); else {{ failed++; fputs(\"FAIL {name}\\n\", stderr); }}",
                c_name(data, d),
            );
        } else {
            // Passes only by failing, with one of the expected messages in its own.
            let list: Vec<String> = subs
                .iter()
                .map(|s| format!("loft_lit({}, {})", body::c_string(s), s.len()))
                .collect();
            let _ = writeln!(
                out,
                "    {{ const loft_str want[] = {{ {} }};\n      \
                 if (setjmp(loft_test_jmp) == 0) {{ {}(); failed++; fputs(\"FAIL {name} (expected to fail)\\n\", stderr); }}\n      \
                 else if (!loft_failed_with(want, {})) {{ failed++; fputs(\"FAIL {name}\\n\", stderr); }} }}",
                list.join(", "),
                c_name(data, d),
                subs.len()
            );
        }
    }
    out.push_str("    return failed ? 1 : 0;\n}\n");
    Ok(out)
}
