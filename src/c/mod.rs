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
mod deliver;
mod eq;
mod files;
mod fnval;
mod parse;
mod reflect;
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
    include_str!("loft_parse.h"),
    include_str!("loft_reflect.h"),
    include_str!("loft_io.h"),
    include_str!("loft_log.h"),
    include_str!("loft_lazy.h"),
    include_str!("loft_net.h"),
    include_str!("loft_process.h")
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
    /// Every type a `deliver` walks (`loft_dlv{tp}`, `deliver.rs`).
    deliver_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every compile-time constant the unit reads (`loft_const{d}`), by definition number.
    consts: std::cell::RefCell<std::collections::BTreeSet<u32>>,
    /// The signatures the unit calls function values through: `loft_call{n}`.
    /// The dispatcher signatures: each parameter's shape and whether it is a `&` link.
    fn_sigs: std::cell::RefCell<Vec<(Vec<(CType, u8)>, CType)>>,
    /// Every function the unit uses as a value, by definition number.
    fn_values: std::cell::RefCell<std::collections::BTreeSet<u32>>,
    /// Every type the unit compares by content (`loft_eq{tp}`), by type number.
    eq_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every keyed collection type whose key parts the unit names (`loft_keys{tp}`).
    key_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The root types a `store_persist_copy` writes an image of: one sidecar each.
    sidecars: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every type the unit writes to or reads from a file (`loft_ser{tp}` / `loft_de{tp}`).
    ser_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The Unicode tables the unit reads (`crate::c::unicode`).
    unicode: std::cell::RefCell<std::collections::BTreeSet<String>>,
    /// The generators' frames and prototypes, written before the unit's functions.
    pub(crate) gen_decls: std::cell::RefCell<String>,
    /// The generators' resume and release functions, written after them.
    pub(crate) gen_bodies: std::cell::RefCell<String>,
    /// The types a `T.parse` reaches (`parse.rs`): their rows of `loft_ptypes`.
    parse_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The unit reaches `stack_trace()`: every function keeps a frame (`loft_frame_push`).
    pub(crate) frames: std::cell::Cell<bool>,
    /// The unit binds a collection to a lazy source (`store_bind_lazy`): every keyed lookup
    /// consults the binding on a miss (`loft_lazy_miss`).
    pub(crate) lazy: std::cell::Cell<bool>,
    /// The `par` sites written on threads so far, which numbers each one's body and context.
    pub(crate) par_sites: std::cell::Cell<u32>,
    /// The keyed collection types a `sqlite:` plan was written for (`loft_sqlplan{tp}`).
    pub(crate) sql_plans: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The lazy fetch functions written so far (`loft_lazy_fetch{n}`).
    pub(crate) lazy_fetchers: std::cell::Cell<usize>,
    /// The place twins the unit calls (`twin_name`): a function by definition number, with
    /// the mask of its `&` parameters that receive a store place rather than a variable.
    twins: std::cell::RefCell<Vec<(u32, u64)>>,
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
            deliver_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            consts: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            fn_sigs: std::cell::RefCell::new(Vec::new()),
            fn_values: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            eq_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            key_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            sidecars: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            ser_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            unicode: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            gen_decls: std::cell::RefCell::new(String::new()),
            gen_bodies: std::cell::RefCell::new(String::new()),
            parse_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            frames: std::cell::Cell::new(false),
            lazy: std::cell::Cell::new(false),
            par_sites: std::cell::Cell::new(0),
            sql_plans: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            lazy_fetchers: std::cell::Cell::new(0),
            twins: std::cell::RefCell::new(Vec::new()),
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

    /// Always written: the heap ceiling's report (`loft_heap_moved`) names a type by it too.
    fn type_name_table(&self) -> String {
        let names: Vec<String> = self
            .stores
            .types
            .iter()
            .map(|t| body::c_string(&t.name))
            .collect();
        // A C99 initializer holds at least one element: a unit with no types (none the
        // stdlib leaves out) writes a null placeholder its count of 0 never reaches.
        format!(
            "static const char *const loft_tnames[] = {{ {} }};\nstatic const uint32_t loft_ntnames = {};\n",
            if names.is_empty() {
                "0".to_string()
            } else {
                names.join(", ")
            },
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
                "static const loft_sidecar loft_sidecar{tp} = {{ {tp}, loft_scb{tp}, loft_sca{tp}, UINT64_C({}), loft_scd{tp}, {}, {} }};",
                id.layout_hash,
                body::c_string(&self.stores.release().oldest),
                if self.holds_stored_refs(tp) {
                    "loft_types"
                } else {
                    "0"
                }
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
                    .map(|e| self.ctype(e).filter(|c| !matches!(c, CType::Void)))
                    .collect::<Option<Vec<_>>>()?;
                Some(CType::Tuple(self.tuple_id(shapes)))
            }
            Type::Void => Some(CType::Void),
            _ => None,
        }
    }

    /// Does tuple struct `id` hold a text at any depth, which it owns and must copy and free?
    pub(crate) fn tuple_has_text(&self, id: u32) -> bool {
        self.tuple_shapes(id).into_iter().any(|c| match c {
            CType::Text => true,
            CType::Tuple(inner) => self.tuple_has_text(inner),
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
            // A linked group's members name one record set: its views are indexed again over
            // the copied owner (`loft_relink_group_views`), whatever collection it is.
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
    fn field_rows(&self, tp: u16, fields: &[crate::database::Field]) -> String {
        let list: Vec<String> = fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let hidden = u8::from(f.name.starts_with('#'));
                let (view, group) = match Self::group_owner(fields, i) {
                    Some(o) if o != i => {
                        let elem = self.stores.content(f.content);
                        let nullable =
                            elem != u16::MAX && self.stores.nullable_some_variant(elem).is_some();
                        (1 + u8::from(nullable), o + 1)
                    }
                    Some(o) => (0, o + 1),
                    None => (0, 0),
                };
                format!(
                    "{{ {}, {}, {view}, {hidden}, {group} }}",
                    f.position, f.content
                )
            })
            .collect();
        format!(
            "static const loft_fld loft_fields{tp}[] = {{ {} }};",
            if list.is_empty() {
                "{ 0, 0, 0, 0, 0 }".into()
            } else {
                list.join(", ")
            }
        )
    }

    /// The owner of field `i`'s linked group — the member that holds the records, the one
    /// whose `other_indexes` does not start with the view marker — or `None` outside a group.
    fn group_owner(fields: &[crate::database::Field], i: usize) -> Option<usize> {
        let others = fields[i].other_indexes();
        if others.is_empty() {
            return None;
        }
        if others.first() != Some(&u16::MAX) {
            return Some(i);
        }
        others
            .iter()
            .copied()
            .filter(|m| *m != u16::MAX)
            .map(usize::from)
            .find(|&m| fields[m].other_indexes().first() != Some(&u16::MAX))
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
                    let _ = writeln!(out, "{}", self.field_rows(tp, fields));
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
                Parts::Hash(..)
                    | Parts::Index(..)
                    | Parts::Radix(..)
                    | Parts::Trie(..)
                    | Parts::Ordered(..)
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
        out.push_str(&self.deliver_code());
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
/// The C name of `d`'s place twin for `mask` (`_pl<mask>`; `_st` is the IR's own text
/// instances, `parser/store_text.rs`): the function written again with each `&`
/// parameter whose bit is set receiving a store place (a field, an element) as a `loft_ref`
/// — `--native`'s `_st` twins.  Mask 0 is the function itself.
pub(crate) fn twin_name(data: &Data, d: u32, mask: u64) -> String {
    if mask == 0 {
        c_name(data, d)
    } else {
        format!("{}_pl{mask}", c_name(data, d))
    }
}

impl Cx<'_> {
    /// The place twin `(d, mask)` is called: it is written with the unit.
    pub(crate) fn request_twin(&self, d: u32, mask: u64) {
        let mut t = self.twins.borrow_mut();
        if !t.contains(&(d, mask)) {
            t.push((d, mask));
        }
    }
}

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
        "n_store_load" | "n_store_load_untrusted" => Some("loft_image_load"),
        "n_store_load_url" => Some("loft_store_load_url"),
        "n_store_load_url_trusted" => Some("loft_store_load_url_trusted"),
        "n_store_memory" => Some("loft_store_memory"),
        "n_store_verify" => Some("loft_store_verify"),
        "n_store_bind_lazy" => Some("loft_lazy_bind_coll"),
        "n_store_lazy_error" => Some("loft_lazy_error"),
        "n_store_lazy_faults" => Some("loft_lazy_faults"),
        "n_store_lazy_clear" => Some("loft_lazy_clear"),
        "n_store_lazy_fail" => Some("loft_lazy_fail"),
        "n_store_lazy_query" => Some("loft_lazy_query"),
        "n_store_lazy_range" => Some("loft_lazy_range"),
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
        "n_is_symlink" => Some("loft_is_symlink_at"),
        "n_c_library_available" => Some("loft_c_library_available"),
        "n_eprint" => Some("loft_eprint"),
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
        | "n_parallel_fold"
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
        _ => host_native_c(name).or_else(|| json_native_c(name)),
    }
}

/// A native a function value may name: its C function takes the visible arguments and answers
/// the result, as `FnWriter::call` calls it.  The natives with an emission of their own (an
/// intercept in `body/`) and the shared names that are no function are not.
pub(crate) fn plain_native(name: &str) -> Option<&'static str> {
    const OWN: &[&str] = &[
        "n_hash_sorted",
        "n_hash_unsorted",
        "n_radix_sorted",
        "n_spatial_range",
        "n_trie_prefix",
        "t_4hash_len",
        "n_store_load",
        "n_store_load_untrusted",
        "n_store_load_url",
        "n_store_load_url_trusted",
        "n_store_persist_copy",
        "n_store_memory",
        "n_store_verify",
        "n_store_bind_lazy",
        "n_store_lazy_query",
        "n_store_lazy_range",
        "n_struct_from_jsonvalue",
        "n_struct_to_json",
        "n_struct_to_json_pretty",
        "n_env_variables",
    ];
    if OWN.contains(&name) || name.starts_with("n_parallel_") || name.starts_with("n_store_load_") {
        return None;
    }
    native_c(name).filter(|c| !matches!(*c, "loft_unicode" | "loft_show" | "loft_pg_load_keys"))
}

/// The C of the host natives the runtime answers itself: directories, the date, the
/// environment, logging without a configuration, store locks, reflection, stack traces.
fn host_native_c(name: &str) -> Option<&'static str> {
    match name {
        "n_os_cache_dir" => Some("loft_os_cache_dir"),
        "n_directory" => Some("loft_directory"),
        "n_user_directory" => Some("loft_user_directory"),
        "n_program_directory" => Some("loft_program_directory"),
        "n_set_store_lock" => Some("loft_set_store_lock"),
        "n_store_reclaim" => Some("loft_store_reclaim"),
        "n_yield_frame" => Some("loft_yield_frame"),
        "n_reflect_type" => Some("loft_reflect_type"),
        "n_stack_trace" => Some("loft_stack_trace"),
        "n_type_named" => Some("loft_type_named"),
        "n_reflect_field" => Some("loft_reflect_field"),
        "n_reflect_field_path" => Some("loft_reflect_field_path"),
        "n_get_store_lock" => Some("loft_get_store_lock"),
        "n_log_info" => Some("loft_log_info"),
        "n_log_warn" => Some("loft_log_warn"),
        "n_log_error" => Some("loft_log_error"),
        "n_log_fatal" => Some("loft_log_fatal"),
        "n_ymd_days_ago" => Some("loft_ymd_days_ago"),
        "n_env_variables" => Some("loft_env_variables"),
        _ => None,
    }
}

/// Does the unit reach a function whose name `pred` accepts?
fn reachable_has(data: &Data, roots: &[u32], pred: impl Fn(&str) -> bool) -> bool {
    crate::opt::names::reachable_functions(data, roots)
        .iter()
        .any(|&d| pred(data.def(d).name()))
}

/// The C of the JSON natives (`loft_json.h`): the reader, the readers of a JsonValue tree and
/// its builders.
fn json_native_c(name: &str) -> Option<&'static str> {
    match name {
        "n_json_parse" => Some("loft_json_parse_value"),
        "n_json_errors" => Some("loft_json_errors"),
        "n_parse_errors_of" => Some("loft_parse_errors_of"),
        // Written by the type's render function (`struct_json_native`).
        "n_struct_to_json" | "n_struct_to_json_pretty" => Some("loft_show"),
        "n_struct_from_jsonvalue" => Some("loft_struct_from_jsonvalue"),
        "n_json_null" => Some("loft_json_null"),
        "n_json_bool" => Some("loft_json_bool"),
        "n_json_number" => Some("loft_json_number"),
        "n_json_string" => Some("loft_json_string"),
        "n_json_array" => Some("loft_json_array"),
        "n_json_object" => Some("loft_json_object"),
        "t_9JsonValue_fields" => Some("loft_jv_fields"),
        "t_9JsonValue_field" => Some("loft_jv_field"),
        "t_9JsonValue_has_field" => Some("loft_jv_has_field"),
        "t_9JsonValue_item" => Some("loft_jv_item"),
        "t_9JsonValue_len" => Some("loft_jv_len"),
        "t_9JsonValue_as_text" => Some("loft_jv_as_text"),
        "t_9JsonValue_as_number" => Some("loft_jv_as_number"),
        "t_9JsonValue_as_long" => Some("loft_jv_as_long"),
        "t_9JsonValue_as_bool" => Some("loft_jv_as_bool"),
        "t_9JsonValue_kind" => Some("loft_jv_kind"),
        "t_9JsonValue_keys" => Some("loft_jv_keys"),
        "t_9JsonValue_to_json" => Some("loft_jv_to_json"),
        "t_9JsonValue_to_json_pretty" => Some("loft_jv_to_json_pretty"),
        _ => None,
    }
}

/// Is `d` an operator, which a call site writes rather than a function it calls?
pub(crate) fn is_operator(data: &Data, d: u32) -> bool {
    data.def(d).name().starts_with("Op")
}

/// Write the place twins asked for since `written` — a twin's body can ask for more, which this
/// writes too — as a prototype into `protos` and a definition into `bodies`.
fn write_twins(
    cx: &Cx<'_>,
    written: &mut usize,
    protos: &mut String,
    bodies: &mut String,
) -> Result<(), Refusal> {
    while *written < cx.twins.borrow().len() {
        let (d, mask) = cx.twins.borrow()[*written];
        *written += 1;
        let (sig, body) = function(cx, d, mask).map_err(|why| Refusal {
            function: cx.data.def(d).original_name().clone(),
            why,
        })?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
    }
    Ok(())
}

/// @PLN203 C3 — the stdlib's natives the C runtime has no arm for: a function with no loft
/// body, no `#c` signature and no library `#native`, which [`native_c`] does not map.  A
/// program that reaches one is refused by its name (`function`); this is the same question
/// asked of the whole stdlib at once, so a new native cannot arrive without a C arm unseen.
#[must_use]
pub fn unported_natives(data: &Data) -> Vec<String> {
    // An interface's method declarations and a bound's stubs are resolved to the type's own
    // method before any code is written, so no unit calls one.
    let members: std::collections::HashSet<u32> = (0..data.definitions())
        .filter(|&d| matches!(data.def_type(d), DefType::Interface))
        .flat_map(|d| data.children_of(d))
        .collect();
    let mut out: Vec<String> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), DefType::Function)
                && !is_operator(data, d)
                && !members.contains(&d)
                && !Data::is_bound_stub_name(def.name())
                && def.code() == &Value::Null
                && def.c_sig.is_empty()
                && def.native().is_empty()
                && native_c(def.name()).is_none()
        })
        .map(|d| data.def(d).name().to_string())
        .collect();
    out.sort();
    out
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
    // Every lazy driver is a root (@PLN133 S9): a miss calls it through the table the unit
    // writes, and a driver left out would be a pointer to a body never written.
    let drivers = data.lazy_fetch_drivers();
    if let Ok(found) = &drivers {
        for &(_, d) in found {
            if !all.contains(&d) {
                all.push(d);
            }
        }
    }
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
    cx.frames
        .set(reachable_has(data, &all, |n| n == "n_stack_trace"));
    cx.lazy
        .set(reachable_has(data, &all, |n| n == "n_store_bind_lazy"));
    let mut protos = String::new();
    let mut bodies = String::new();
    for &d in &fns {
        let (sig, body) = function(&cx, d, 0).map_err(|why| Refusal {
            function: data.def(d).original_name().clone(),
            why,
        })?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
    }
    // The place twins the functions call, and the ones those call in turn.
    let mut written = 0;
    write_twins(&cx, &mut written, &mut protos, &mut bodies)?;
    // The dispatchers call place twins too: those are written, and the dispatchers again,
    // until nothing new is asked.
    let dispatch = loop {
        let dispatch = cx.dispatch_code(&fns).map_err(|why| Refusal {
            function: data.def(roots[0]).original_name().clone(),
            why,
        })?;
        if written == cx.twins.borrow().len() {
            break dispatch;
        }
        write_twins(&cx, &mut written, &mut protos, &mut bodies)?;
    };
    // `file_map` is the one producer of a foreign store (@PLN174): a unit that does not reach
    // it compiles `loft_at` without the foreign record's test.
    let mut out = String::new();
    let reached = crate::opt::names::reachable_functions(data, &all);
    let (defines, parses, reflects) = unit_defines(data, stores, &cx, &reached);
    out.push_str(&defines);
    out.push_str(RUNTIME);
    out.push('\n');
    out.push_str(&c_libraries(data));
    out.push_str(&cx.typedefs());
    if parses {
        out.push_str(&cx.parse_table());
        out.push_str(&cx.main_vector_test());
    }
    if reflects {
        out.push_str(&cx.reflect_table());
        out.push_str(REFLECT_ENTRY);
    }
    out.push_str(&cx.gen_decls.borrow());
    out.push_str(&protos);
    out.push_str(&dispatch);
    out.push_str(&cx.drop_code(&fns));
    out.push_str(&cx.gen_bodies.borrow());
    out.push_str(&bodies);
    // The unit's tables, for the runtime (`loft_tn`, `loft_cl`, the lazy drivers): a generated
    // `main` binds them before it starts.  Written last, so it can name every function.
    let mut bind = String::from(
        "\nstatic void loft_bind_tables(void) {\n    loft_tn = loft_tnames;\n    loft_ntn = loft_ntnames;\n    \
         loft_cl = loft_clibs;\n    loft_ncl = loft_nclibs;\n    loft_cls = loft_clibsyms;\n    \
         loft_col = loft_coptlibs;\n    loft_ncol = loft_ncoptlibs;\n    \
         loft_cbl = loft_cboundlibs;\n    loft_ncbl = loft_ncboundlibs;\n    \
         loft_fault_logger = loft_log_production_fault;\n",
    );
    if cx.lazy.get() {
        out.push_str(&lazy_driver_table(&cx, drivers)?);
        bind.push_str(
            "    loft_lazy_drivers = loft_lazy_driver_table;\n    \
             loft_nlazy_drivers = loft_nlazy_driver_table;\n    \
             loft_lazy_driver_refusal = loft_lazy_driver_refusal_text;\n",
        );
    }
    bind.push_str("}\n");
    out.push_str(&bind);
    Ok(out)
}

/// The program's lazy drivers as the runtime calls them (`loft_lazy_miss`): an adapter per
/// driver with the fixed shape `(coll, source, key_int, key_text) -> integer`, the table of
/// them by the element type each serves (`Data::lazy_fetch_drivers`, the one answer both
/// backends read), or — when the drivers were refused — the reason, which every miss reports.
fn lazy_driver_table(
    cx: &Cx,
    drivers: Result<Vec<(String, u32)>, String>,
) -> Result<String, Refusal> {
    let mut out = String::new();
    let (found, refusal) = match drivers {
        Ok(found) => (found, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let mut rows = Vec::new();
    for (i, (element, d)) in found.iter().enumerate() {
        let def = cx.data.def(*d);
        if def.attributes().iter().any(|a| a.hidden) {
            return Err(Refusal {
                function: def.original_name().clone(),
                why: "a lazy driver with a hidden parameter".into(),
            });
        }
        let _ = writeln!(
            out,
            "static int64_t loft_lazydrv{i}(loft_ref c, loft_str s, int64_t ki, loft_str kt) {{ \
             return {}(c, s, ki, kt); }}",
            c_name(cx.data, *d)
        );
        rows.push(format!(
            "{{ {}, loft_lazydrv{i} }}",
            body::c_string(element)
        ));
    }
    let _ = writeln!(
        out,
        "static const loft_lazy_driver loft_lazy_driver_table[] = {{ {} }};\n\
         static const uint32_t loft_nlazy_driver_table = {};\n\
         static const char *const loft_lazy_driver_refusal_text = {};",
        if rows.is_empty() {
            "{ 0, 0 }".to_string()
        } else {
            rows.join(", ")
        },
        rows.len(),
        refusal.map_or_else(|| "0".to_string(), |w| body::c_string(&w))
    );
    Ok(out)
}

/// The `[c]` libraries the program's packages declare, as the unit carries them: a `[c] shim` is
/// built into its static archive by `cc` + `ar` (`c_shim::cc_archive`; off Windows the same
/// artefact a `--static` program links) and named in a `/* link: <path> */` line for whoever
/// compiles the unit, its sources in `/* shim-source: <path> */` lines for a wasm32 build.  A
/// shim is never a part of the unit's own translation unit: its sources may carry `static`
/// names of their own (the vendored BearSSL does), and a binding declares its callee by the
/// signature the package wrote, which need only match the shim's ABI (`void *` against the
/// shim's `void **`), not its spelling.  A declared library (`libs`, `optional-libs`)
/// is named the same way, for the linker.  `scripts/c_sweep.py` and `tests/c_emit.rs` read
/// the `link:` lines.
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the declared libraries: the header, the shim's three forms, the link lines"
)]
fn c_libraries(data: &Data) -> String {
    let mut out = String::new();
    // A required library is linked: the file the package ships, else its soname in the system
    // linker's spelling.  An optional one (`[c] optional-libs`) never is — it is
    // opened when its package's symbol is first called, or when `c_library_available` asks.
    let os = crate::platform::host_lib_os();
    let mut links: Vec<String> = Vec::new();
    let mut carried: Vec<String> = Vec::new();
    // A library is available only when it exports every symbol its package declares
    // (`c_libs::library_available`): the symbols each one is asked for at run time.
    let pkg_syms = |pkg: &str| -> Vec<String> {
        let mut own: Vec<String> = (0..data.definitions())
            .filter(|&d| {
                let def = data.def(d);
                !def.c_sig.is_empty()
                    && def.code() == &Value::Null
                    && data.c_owner_pkg(def.position().file.as_str()) == Some(pkg)
            })
            .filter_map(|d| {
                crate::c_signature::of(data, d, crate::c_signature::CTarget::host())
                    .and_then(Result::ok)
                    .map(|s| body::c_string(&s.symbol))
            })
            .collect();
        own.sort();
        own.dedup();
        own
    };
    let mut syms: Vec<Vec<String>> = Vec::new();
    let mut optional: Vec<String> = Vec::new();
    // The link inputs that are a library the package ships: a shipped program opens these at
    // start instead (`/* bound: */`, read by `c_driver::build`).
    let mut bound: Vec<String> = Vec::new();
    for lib in data.c_libraries.iter().filter(|l| l.shim.is_empty()) {
        let dir = std::path::Path::new(&lib.pkg_dir);
        let shipped = crate::platform::existing_lib_beside(dir, &lib.name, os);
        if lib.optional {
            // Where `dlopen` looks: the file beside the package, else the name's spellings.
            let mut cands: Vec<String> = Vec::new();
            if let Some(file) = &shipped {
                cands.push(body::c_string(&crate::file_access::portable(
                    &dir.join(file),
                )));
            }
            for v in crate::platform::lib_variants(&lib.name, os) {
                let c = body::c_string(&v);
                if !cands.contains(&c) {
                    cands.push(c);
                }
            }
            cands.push("0".to_string());
            let mut own = pkg_syms(&lib.pkg_dir);
            own.push("0".to_string());
            let i = optional.len();
            let _ = writeln!(
                out,
                "static const char *const loft_coptc{i}[] = {{ {} }};\n\
                 static const char *const loft_copts{i}[] = {{ {} }};",
                cands.join(", "),
                own.join(", ")
            );
            optional.push(format!(
                "{{ {}, {}, loft_coptc{i}, loft_copts{i} }}",
                body::c_string(&lib.name),
                body::c_string(&lib.pkg_dir)
            ));
            continue;
        }
        if let Some(file) = &shipped {
            let args = crate::platform::c_link_args_for_file(&dir.join(file));
            bound.extend(args.iter().cloned());
            links.extend(args);
        } else {
            links.push(crate::platform::c_link_arg_for_soname(&lib.name));
        }
        carried.push(body::c_string(&lib.name));
        syms.push(pkg_syms(&lib.pkg_dir));
    }
    let _ = writeln!(
        out,
        "static const loft_c_optlib loft_coptlibs[] = {{ {} }};\n\
         static const uint32_t loft_ncoptlibs = {};",
        if optional.is_empty() {
            "{ 0, 0, 0, 0 }".to_string()
        } else {
            optional.join(", ")
        },
        optional.len()
    );
    // @PLN195 — the libraries a shipped program opens at start (`loft_c_bind_libraries`): each
    // one's package, version and file, never its directory.
    let rows: Vec<String> = data
        .shipped_c_libraries()
        .into_iter()
        .map(|(lib, path)| {
            let (package, version) = package_identity(&lib.pkg_dir);
            format!(
                "{{ {}, {}, {}, {} }}",
                body::c_string(&package),
                body::c_string(&version),
                body::c_string(&crate::file_access::file_name(path.as_path()).unwrap_or_default()),
                u8::from(!lib.shim.is_empty())
            )
        })
        .collect();
    let _ = writeln!(
        out,
        "static const loft_c_boundlib loft_cboundlibs[] = {{ {} }};\n\
         static const uint32_t loft_ncboundlibs = {};",
        if rows.is_empty() {
            "{ 0, 0, 0, 0 }".to_string()
        } else {
            rows.join(", ")
        },
        rows.len()
    );
    // A shim of a package with optional libraries is reached through the program's own
    // symbols (`loft_c_lazy_sym`), which the linker exports only when asked.
    if data.c_libraries.iter().any(|l| {
        !l.shim.is_empty()
            && data
                .c_libraries
                .iter()
                .any(|o| o.optional && o.pkg_dir == l.pkg_dir)
    }) && let Some(arg) = crate::platform::c_export_dynamic_arg()
    {
        links.push(arg.to_string());
    }
    // A shim is linked into the binary: its symbols are there, and asked for nothing.
    for lib in data.c_libraries.iter().filter(|l| !l.shim.is_empty()) {
        carried.push(body::c_string(&lib.name));
        syms.push(Vec::new());
    }
    for (i, own) in syms.iter().enumerate() {
        let mut list = own.clone();
        list.push("0".to_string());
        let _ = writeln!(
            out,
            "static const char *const loft_clibsym{i}[] = {{ {} }};",
            list.join(", ")
        );
    }
    let tables: Vec<String> = (0..syms.len())
        .map(|i| format!("loft_clibsym{i}"))
        .collect();
    let _ = writeln!(
        out,
        "static const char *const *const loft_clibsyms[] = {{ {} }};",
        if tables.is_empty() {
            "0".to_string()
        } else {
            tables.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "static const char *const loft_clibs[] = {{ {} }};\nstatic const uint32_t loft_nclibs = {};",
        if carried.is_empty() {
            "0".to_string()
        } else {
            carried.join(", ")
        },
        carried.len()
    );
    for lib in &data.c_libraries {
        if lib.shim.is_empty() {
            let _ = writeln!(
                out,
                "/* `#c` library {} (package {}) */",
                lib.name, lib.pkg_dir
            );
        } else if let Ok(archive) = crate::c_shim::cc_archive(&lib.pkg_dir, &lib.shim) {
            // Archived by `cc` + `ar` whatever the host's static-link archiver is: `cc` links
            // this unit, and MinGW's `ld` cannot read an MSVC archive.  The sources are named
            // too, for a wasm32 build, which compiles them instead of linking the host archive.
            // A package with optional libraries reaches its shim through `dlsym`, so the
            // archive is linked whole — nothing references its symbols directly.
            let _ = writeln!(
                out,
                "/* `[c] shim` of package {}, {} sources */",
                lib.pkg_dir,
                lib.shim.len()
            );
            let path = crate::file_access::portable(&archive);
            let lazy = data
                .c_libraries
                .iter()
                .any(|o| o.optional && o.pkg_dir == lib.pkg_dir);
            let args = if lazy {
                crate::platform::c_whole_archive_args(&path)
            } else {
                // A shipped program links it whole, or not at all when its library goes beside
                // the program (`c_driver::build`).
                let _ = writeln!(out, "/* shim-archive: {path} */");
                vec![path]
            };
            for a in args {
                let _ = writeln!(out, "/* link: {a} */");
            }
            for src in &lib.shim {
                let _ = writeln!(
                    out,
                    "/* shim-source: {} */",
                    crate::file_access::portable_str(src)
                );
            }
        } else {
            let _ = writeln!(
                out,
                "/* `[c] shim` of package {}: compile and link these sources with this unit */",
                lib.pkg_dir
            );
            for src in &lib.shim {
                let _ = writeln!(out, "/* link: {} */", crate::file_access::portable_str(src));
            }
        }
    }
    for l in bound {
        let _ = writeln!(out, "/* bound: {l} */");
    }
    for l in links {
        let _ = writeln!(out, "/* link: {l} */");
    }
    out
}

/// A package's name and version as its manifest states them — the directory's name and `0`
/// where it states none, as the Rust writer's bind table spells them.
fn package_identity(pkg_dir: &str) -> (String, String) {
    let manifest = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"));
    let package = manifest
        .as_ref()
        .and_then(|m| m.name.clone())
        .or_else(|| crate::file_access::file_name(pkg_dir))
        .unwrap_or_default();
    let version = manifest
        .and_then(|m| m.version)
        .unwrap_or_else(|| "0".to_string());
    (package, version)
}

/// The macros a unit's runtime is compiled under, from what it reaches: a foreign store, the
/// JSON reader and tree, the typed parse, reflection, frames — and whether it parses and
/// reflects, which add tables after the runtime.
fn unit_defines(
    data: &Data,
    stores: &Stores,
    cx: &Cx,
    reached: &std::collections::HashSet<u32>,
) -> (String, bool, bool) {
    let mut out = String::new();
    if reached.iter().any(|&d| data.def(d).name() == "n_file_map") {
        out.push_str("#define LOFT_FOREIGN 1\n");
    }
    // The JSON reader (`loft_json.h`) is compiled in for a unit that reads JSON: the JsonValue
    // tree with its layout read from this program's type table for one that reaches a JSON
    // native, the typed walk (`loft_parse.h`) for one that parses a type.
    let json_tree = reached.iter().any(|&d| {
        let n = data.def(d).name();
        n.starts_with("n_json_") || n.starts_with("t_9JsonValue_") || n == "n_struct_from_jsonvalue"
    });
    let parses = !cx.parse_types.borrow().is_empty()
        || reached
            .iter()
            .any(|&d| data.def(d).name() == "n_parse_errors_of");
    if json_tree || parses {
        out.push_str("#define LOFT_JSON 1\n");
    }
    if json_tree {
        out.push_str(&json_layout(stores));
    }
    if parses {
        out.push_str("#define LOFT_PARSE 1\n");
    }
    let reflects = reached.iter().any(|&d| {
        matches!(
            data.def(d).name(),
            "n_reflect_type" | "n_type_named" | "n_reflect_field" | "n_reflect_field_path"
        )
    });
    if reflects {
        out.push_str("#define LOFT_REFLECT 1\n");
    }
    if cx.lazy.get() {
        // A driver's fault is contained and the program goes on, so its frames' texts are
        // released at the jump (`LOFT_HOLD`, loft#2198) — or every failed fetch leaks them.
        out.push_str(
            "#define LOFT_LAZY 1\n#ifndef LOFT_HALT_RELEASE\n#define LOFT_HALT_RELEASE 1\n#endif\n",
        );
    }
    if reached.iter().any(|&d| {
        matches!(
            data.def(d).name(),
            "n_store_load_url" | "n_store_load_url_trusted"
        )
    }) {
        out.push_str("#define LOFT_NET 1\n");
    }
    if reached
        .iter()
        .any(|&d| in_binary_native(data.def(d).native()).is_some())
    {
        out.push_str("#define LOFT_PROCESS 1\n");
    }
    if cx.frames.get() {
        let sf = stores.name("StackFrame");
        let at = |f: &str| stores.position(sf, f);
        let _ = writeln!(
            out,
            "#define LOFT_FRAMES 1\n#define LOFT_SF_SIZE {}\n#define LOFT_SF_FUNCTION {}\n\
             #define LOFT_SF_FILE {}\n#define LOFT_SF_LINE {}\n#define LOFT_SF_ARGS {}\n\
             #define LOFT_SF_VARS {}",
            stores.size(sf),
            at("function"),
            at("file"),
            at("line"),
            at("arguments"),
            at("variables")
        );
    }
    (out, parses, reflects)
}

/// `reflect_type` and `type_named` over the emitted table (`reflect.rs`, `loft_reflect.h`).
const REFLECT_ENTRY: &str = r##"
static loft_ref loft_reflect_type(int64_t kt) {
    uint16_t k = (uint16_t) kt;
    if (k < loft_nrtypes) return loft_reflect_build(&loft_rlay, &loft_rtypes[k], loft_rtypes[k].name);
    char b[16];
    snprintf(b, sizeof b, "#%u", (unsigned) k);
    return loft_reflect_build(&loft_rlay, &loft_rnone, b);
}

static loft_ref loft_reflect_field(loft_ref value, int64_t position, int64_t kt) {
    return loft_reflect_field_at(&loft_rlay, loft_rtypes, loft_nrtypes, value, value.pos, (uint16_t) kt, position);
}

static loft_ref loft_reflect_field_path(loft_ref value, loft_ref path, int64_t kt) {
    return loft_reflect_field_path_at(&loft_rlay, loft_rtypes, loft_nrtypes, value, path, (uint16_t) kt);
}

static loft_ref loft_type_named(loft_str name) {
    for (uint32_t k = 0; k < loft_nrtypes; k++)
        if (strlen(loft_rtypes[k].lookup) == name.len && memcmp(loft_rtypes[k].lookup, name.p, name.len) == 0)
            return loft_reflect_build(&loft_rlay, &loft_rtypes[k], loft_rtypes[k].name);
    return loft_ref_null();
}
"##;

/// The offsets of the JsonValue tree, as `native.rs` reads them through
/// `Stores::position`: each variant's payload, a JsonField's name and value, and the sizes of
/// a JsonValue, a JsonField and a text.
fn json_layout(stores: &Stores) -> String {
    let at = |tp: &str, field: &str| stores.position(stores.name(tp), field);
    format!(
        "#define LOFT_JV_SIZE {}\n#define LOFT_JF_SIZE {}\n#define LOFT_TEXT_SIZE {}\n\
         #define LOFT_JF_NAME {}\n#define LOFT_JF_VALUE {}\n#define LOFT_JV_BOOL_VALUE {}\n\
         #define LOFT_JV_NUMBER_VALUE {}\n#define LOFT_JV_INT_VALUE {}\n#define LOFT_JV_STRING_VALUE {}\n\
         #define LOFT_JV_ITEMS {}\n#define LOFT_JV_FIELDS {}\n",
        stores.size(stores.name("JsonValue")),
        stores.size(stores.name("JsonField")),
        stores.size(stores.name("text")),
        at("JsonField", "name"),
        at("JsonField", "value"),
        at("JBool", "value"),
        at("JNumber", "value"),
        at("JInteger", "value"),
        at("JString", "value"),
        at("JArray", "items"),
        at("JObject", "fields"),
    )
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
        "\nint main(int argc, char **argv) {{\n    loft_bind_tables();\n    loft_start(argc, argv);\n    {}\n    {}();\n    loft_leak_report();\n    return loft_had_fatal;\n}}\n",
        anchor(stores),
        c_name(data, main)
    );
    Ok(out)
}

/// The statement a generated `main` starts with: the directory a relative path resolves against
/// (the program's own, as the interpreter's `Stores::resolve_path` anchors it) and whether
/// the program anchors there at all (`#cwd` says no).
fn anchor(stores: &Stores) -> String {
    format!("loft_set_anchor({});", i32::from(stores.program_relative))
}

/// The signature and the body of function `d`.
fn function(cx: &Cx, d: u32, places: u64) -> Result<(String, String), String> {
    let data = cx.data;
    let def = data.def(d);
    // A `#c` binding (@PLN24): the C function itself, called by its declared signature.
    if def.code() == &Value::Null && !def.c_sig.is_empty() {
        return c_binding(cx, d);
    }
    // A `#native` function of a library the program does not load: the interpreter's stub
    // (`compile.rs`) answers every call with this panic.
    let unloaded = def.code() == &Value::Null && !def.native().is_empty();
    if def.code() == &Value::Null && !unloaded {
        return Err(format!("native {}", def.original_name()));
    }
    let ret = cx
        .ctype(def.returned())
        .ok_or_else(|| format!("returns {}", type_word(def.returned())))?;
    if unloaded {
        if let Some(runtime) = in_binary_native(def.native()) {
            return forwarded_native(cx, d, ret, runtime);
        }
        // A native loft's own binary implements is never unloaded there: the stub would answer
        // what the interpreter never does, so a native the runtime does not carry refuses.
        if crate::native::FUNCTIONS
            .iter()
            .any(|(name, _)| *name == def.native())
        {
            return Err(format!(
                "native {} (in loft's binary; the C runtime does not carry it)",
                def.original_name()
            ));
        }
        return unloaded_native(cx, d, ret);
    }
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
        // A local only ever bound to `null` (`x = null;`, never given a type) holds the
        // integer null; nothing reads it as another kind.
        if matches!(tp, Type::Null) && !args.contains(&v) {
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
        .enumerate()
        .map(|(i, &a)| {
            let c = shapes[a as usize];
            let name = body::var_name(vars.name(a), a);
            if places & (1 << i) != 0 || vars.is_store_text_link(a) {
                format!("loft_ref {name}")
            } else if matches!(vars.tp(a).base(), Type::RefVar(_)) {
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
    let name = twin_name(data, d, places);
    let sig = format!("static {} {name}({params})", held(ret));
    if let Type::Iterator(elem, _) = def.returned().base() {
        if places != 0 {
            return Err("a generator handed a store place for a & parameter".into());
        }
        // A tuple rides the frame as a struct copy, a move; a yield copies its texts into the
        // consumer's value (`gen_yield`), and a record it holds is handed over (`(G-Own)`).
        let elem = cx
            .ctype(elem)
            .filter(|c| *c != CType::Void)
            .ok_or_else(|| format!("a generator of {}", type_word(elem)))?;
        let (body, parts) = body::FnWriter::new(cx, d, shapes, CType::Void)
            .yielding(elem)
            .write_gen()?;
        cx.gen_decls.borrow_mut().push_str(&parts.decls);
        cx.gen_bodies.borrow_mut().push_str(&parts.bodies);
        return Ok((sig, body));
    }
    let body = body::FnWriter::new(cx, d, shapes, ret)
        .places(places)
        .write()?;
    // Every function keeps its frame: the pushes are macros that a build without the call
    // chain (`LOFT_CALL_CHAIN`, `LOFT_FRAMES`) compiles to nothing, the wrapper then a call
    // the compiler folds into its caller.
    Ok((sig, framed(cx, d, &name, &params, ret, &body)))
}

/// A `#c` binding (`c_signature`, `c_call`): a wrapper declaring the callee by the author's own
/// signature text — so it agrees with a system header that declares it too — and calling it with
/// each scalar cast to its declared C type and each text as a NUL-terminated copy; an integer
/// result widened to 64 bits by its signedness, a floating one as is.  A vector argument is not
/// carried yet.
fn c_binding(cx: &Cx, d: u32) -> Result<(String, String), String> {
    use crate::c_signature::{self as cs, CArg};
    let data = cx.data;
    let def = data.def(d);
    let sig = cs::of(data, d, cs::CTarget::host())
        .ok_or("a C binding without a signature")?
        .map_err(|e| format!("a C binding whose signature does not parse: {e}"))?;
    let params: Vec<Type> = def.attributes().iter().map(|a| a.typedef.clone()).collect();
    let plan = cs::plan(&params, &sig).map_err(|_| "a C binding whose arguments do not fit")?;
    // The author's spelling: `<ret>(<param>, …)`.
    let text = def.c_sig.trim();
    let open = text
        .find('(')
        .ok_or("a C signature without its parameters")?;
    let ret_text = text[..open].trim();
    let inner = text[open + 1..].trim_end().trim_end_matches(')').trim();
    let c_params: Vec<&str> = if inner.is_empty() || inner == "void" {
        Vec::new()
    } else {
        inner.split(',').map(str::trim).collect()
    };
    let ret = cx
        .ctype(def.returned())
        .ok_or_else(|| format!("returns {}", type_word(def.returned())))?;
    let mut decl_params = Vec::new();
    let mut body = String::new();
    let mut args = Vec::new();
    let mut frees = Vec::new();
    for (i, (arg, tp)) in plan.iter().zip(&params).enumerate() {
        let c = cx
            .ctype(tp)
            .filter(|c| *c != CType::Void)
            .ok_or_else(|| format!("parameter of type {}", type_word(tp)))?;
        decl_params.push(format!("{} a{i}", passed(c)));
        let ct = c_params.get(args.len()).copied().unwrap_or("int");
        match arg {
            CArg::Scalar => args.push(format!("({ct}) a{i}")),
            CArg::TextPointer => {
                let _ = writeln!(body, "    char *s{i} = loft_cstr_own(a{i});");
                args.push(format!("s{i}"));
                frees.push(format!("    free(s{i});"));
            }
            // A vector crosses as its element pointer, and a count where the C signature has
            // one (@C107), exactly as the interpreter's `c_call` hands them.
            CArg::VectorPtr | CArg::VectorPtrCount => {
                args.push(format!("({ct}) loft_cvec_ptr(a{i})"));
                if *arg == CArg::VectorPtrCount {
                    let cn = c_params.get(args.len()).copied().unwrap_or("int64_t");
                    args.push(format!("({cn}) loft_vec_len(a{i})"));
                }
            }
        }
    }
    let params_text = if c_params.is_empty() {
        "void".to_string()
    } else {
        c_params.join(", ")
    };
    // A symbol of a package with `[c] optional-libs` is resolved when first called, through
    // the program and then the package's optional libraries (`loft_c_lazy_sym`):
    // a `#c` declaration does not name its library, so the whole package goes this way.
    let file = def.position().file.to_string();
    let (decl, call) = if data.c_symbol_is_lazy(&file) {
        let pkg = data.c_owner_pkg(&file).unwrap_or_default();
        (
            format!(
                "    static {ret_text} (*loft_cfp)({params_text}) = 0;\n    \
                 if (!loft_cfp) {{ void *loft_p = loft_c_lazy_sym({}, {}); \
                 memcpy(&loft_cfp, &loft_p, sizeof loft_p); }}\n    \
                 if (!loft_cfp) loft_c_missing_symbol({});",
                body::c_string(&sig.symbol),
                body::c_string(pkg),
                body::c_string(&sig.symbol)
            ),
            format!("loft_cfp({})", args.join(", ")),
        )
    } else if let Some(when) = bound_by(data, &file) {
        // @PLN195 — a library a shipped program opens at start is not linked, so its symbol is
        // found once, in the program and then in that library; an ordinary build calls it.
        (
            format!(
                "#if defined({when})\n    \
                 static {ret_text} (*loft_cfp)({params_text}) = 0;\n    \
                 if (!loft_cfp) {{ void *loft_p = loft_c_bound_sym({sym}); \
                 memcpy(&loft_cfp, &loft_p, sizeof loft_p); }}\n    \
                 if (!loft_cfp) loft_c_missing_symbol({sym});\n\
                 #else\n    \
                 extern {ret_text} {}({params_text});\n    \
                 {ret_text} (*const loft_cfp)({params_text}) = {};\n\
                 #endif",
                sig.symbol,
                sig.symbol,
                sym = body::c_string(&sig.symbol),
            ),
            format!("loft_cfp({})", args.join(", ")),
        )
    } else {
        (
            format!("    extern {ret_text} {}({params_text});", sig.symbol),
            format!("{}({})", sig.symbol, args.join(", ")),
        )
    };
    let mut out = format!("{decl}\n{body}");
    c_binding_result(cx, &mut out, &sig.ret, ret, &call, &frees);
    let sig_c = format!(
        "static {} {}({})",
        held(ret),
        c_name(data, d),
        if decl_params.is_empty() {
            "void".to_string()
        } else {
            decl_params.join(", ")
        }
    );
    Ok((sig_c, out))
}

/// The macro under which a `#c` symbol declared in `file` is found at run time rather than
/// linked: `LOFT_BIND_LIBS` when its package ships a library of its own (a shipped program opens
/// that at start), `LOFT_BIND_SHIMS` when it has only a `[c] shim` (opened at start only by a
/// `--dynamic` build).  None for a package with neither — its symbols are the system's.
fn bound_by(data: &Data, file: &str) -> Option<&'static str> {
    let pkg = data.c_owner_pkg(file)?;
    let shipped = data.shipped_c_libraries();
    let own = shipped.iter().filter(|(l, _)| l.pkg_dir == pkg);
    let mut shim = false;
    for (l, _) in own {
        if l.shim.is_empty() {
            return Some("LOFT_BIND_LIBS");
        }
        shim = true;
    }
    shim.then_some("LOFT_BIND_SHIMS")
}

/// The call of a `#c` binding and its result: an integer widened to 64 bits by its signedness, a
/// floating one as is, each text argument's copy freed after the call.
fn c_binding_result(
    cx: &Cx,
    out: &mut String,
    c_ret: &crate::c_signature::CType,
    ret: CType,
    call: &str,
    frees: &[String],
) {
    use crate::c_signature as cs;
    match (c_ret, ret) {
        (cs::CType::Void, _) | (_, CType::Void) => {
            let _ = writeln!(out, "    {call};");
            for f in frees {
                let _ = writeln!(out, "{f}");
            }
            if ret != CType::Void {
                let _ = writeln!(
                    out,
                    "    {{ {} r = {}; return r; }}",
                    held(ret),
                    cx.initial(ret)
                );
            }
        }
        // A `char *` return is copied up to its NUL, NULL the null text (`c_call`).
        (cs::CType::Pointer { .. }, CType::Text) => {
            let _ = writeln!(out, "    loft_text r = loft_text_from_cstr({call});");
            for f in frees {
                let _ = writeln!(out, "{f}");
            }
            out.push_str("    return r;\n");
        }
        (r, c) => {
            let widened = match r {
                cs::CType::Int { signed: false, .. } => format!("(int64_t) (uint64_t) {call}"),
                cs::CType::Int { .. } => format!("(int64_t) {call}"),
                // A handle: the pointer's value as an integer.
                cs::CType::Pointer { .. } => format!("(int64_t) (intptr_t) {call}"),
                _ => format!("(double) {call}"),
            };
            let value = match c {
                CType::Float => format!("(double) ({widened})"),
                CType::Single => format!("(float) ({widened})"),
                CType::Bool => format!("(uint8_t) (({widened}) != 0)"),
                _ => format!("(int64_t) ({widened})"),
            };
            let _ = writeln!(out, "    {} r = {value};", held(c));
            for f in frees {
                let _ = writeln!(out, "{f}");
            }
            out.push_str("    return r;\n");
        }
    }
}

/// The body of a `#native` function whose library is not loaded: the panic the interpreter's
/// stub raises (`compile.rs`), then the result's null for a caller that is past it.
/// The runtime function behind a native of an `in_binary` library — one whose natives live in
/// loft's own binary (`[native] in_binary = true`, `native.rs`' table): the C runtime carries
/// each one the C programs reach.
fn in_binary_native(native: &str) -> Option<&'static str> {
    match native {
        "n_process_run" => Some("loft_process_run"),
        "n_git_query" => Some("loft_git_query"),
        _ => None,
    }
}

/// The parameters of a body-less declaration — its attributes, a `&` one as a pointer — each
/// declared as `a<i>`.
fn declared_params(cx: &Cx, d: u32) -> Result<Vec<String>, String> {
    let def = cx.data.def(d);
    let mut params = Vec::new();
    for (i, a) in def.attributes().iter().enumerate() {
        let c = cx
            .ctype(a.typedef.peel_link())
            .filter(|c| *c != CType::Void)
            .ok_or_else(|| format!("parameter of type {}", type_word(&a.typedef)))?;
        params.push(if matches!(a.typedef.base(), Type::RefVar(_)) {
            format!("{} *a{i}", held(c))
        } else {
            format!("{} a{i}", passed(c))
        });
    }
    Ok(params)
}

/// An `in_binary` native the runtime carries: a body that hands its arguments on.
fn forwarded_native(
    cx: &Cx,
    d: u32,
    ret: CType,
    runtime: &str,
) -> Result<(String, String), String> {
    let params = declared_params(cx, d)?;
    let sig = format!(
        "static {} {}({})",
        held(ret),
        c_name(cx.data, d),
        if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        }
    );
    let args: Vec<String> = (0..params.len()).map(|i| format!("a{i}")).collect();
    let call = format!("{runtime}({})", args.join(", "));
    let body = if ret == CType::Void {
        format!("    {call};\n")
    } else {
        format!("    return {call};\n")
    };
    Ok((sig, body))
}

fn unloaded_native(cx: &Cx, d: u32, ret: CType) -> Result<(String, String), String> {
    let def = cx.data.def(d);
    // A body-less declaration has no variables: its parameters are its attributes.
    let params = declared_params(cx, d)?;
    let sig = format!(
        "static {} {}({})",
        held(ret),
        c_name(cx.data, d),
        if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        }
    );
    let msg = format!(
        "native function {} not loaded. Either (a) its library's native cdylib is missing or \
         stale — commonly built against a different libloft.rlib; rebuild with `make \
         rebuild-native-cdylibs`, or `cargo build --release` in the library's `native/` dir — or \
         (b) a freed store was read and its poisoned contents were used as the call index, in \
         which case the real fault is a store lifetime bug that happened EARLIER. Tell them \
         apart: if the run is green with LOFT_POISON unset, or an earlier line reports a `BUG` \
         refusal of a store free / a strict-store violation, it is (b) — re-run with `LOFT_STRICT_STORES=1` to \
         name the access and the free.",
        def.native()
    );
    let mut out = String::new();
    for i in 0..params.len() {
        let _ = writeln!(out, "    (void) a{i};");
    }
    let _ = writeln!(
        out,
        "    loft_panic(loft_lit({}, {}), loft_lit(\"\", 0), 0);",
        body::c_string(&msg),
        msg.len()
    );
    if ret != CType::Void {
        let _ = writeln!(
            out,
            "    {{ {} r = {}; return r; }}",
            held(ret),
            cx.initial(ret)
        );
    }
    Ok((sig, out))
}

/// A function as the unit writes it: its body becomes `<name>__fr`, and the function
/// pushes its frame (`cr_call_push`'s name and file), calls the body and pops the frame, so
/// every return of the body leaves the frame stack as it found it.
fn framed(cx: &Cx, d: u32, name: &str, params: &str, ret: CType, body: &str) -> String {
    let def = cx.data.def(d);
    let _ = writeln!(
        cx.gen_decls.borrow_mut(),
        "static {} {name}__fr({params});",
        held(ret)
    );
    let _ = write!(
        cx.gen_bodies.borrow_mut(),
        "\nstatic {} {name}__fr({params}) {{\n{body}}}\n",
        held(ret)
    );
    let args: Vec<String> = if params == "void" {
        Vec::new()
    } else {
        params
            .split(", ")
            .map(|p| p.rsplit([' ', '*']).next().unwrap_or("").to_string())
            .collect()
    };
    let call = format!("{name}__fr({})", args.join(", "));
    // The frame's name as `--native`'s `cr_call_push` and the interpreter spell it: a plain
    // function by its source name, a method by its definition name (`t_4Tick_go`).
    let frame_name = if def.name().starts_with("n_") {
        def.original_name()
    } else {
        def.name().to_string()
    };
    let push = format!(
        "    loft_frame_push({}, {});\n",
        body::c_string(&frame_name),
        body::c_string(&def.position().file)
    );
    // The body names the texts it holds (`LOFT_HOLD`); the frame drops them on its return.
    let push = format!("{push}    LOFT_HOLD_MARK(hm);\n");
    if ret == CType::Void {
        format!("{push}    {call};\n    LOFT_HOLD_BACK(hm);\n    loft_frame_pop();\n")
    } else {
        format!(
            "{push}    {} r = {call};\n    LOFT_HOLD_BACK(hm);\n    loft_frame_pop();\n    return r;\n",
            held(ret)
        )
    }
}

/// Does the program have anything to run — a `main`, or a test?  A program with neither (a
/// library, a file of declarations) compiles to nothing a driver could run.
#[must_use]
pub fn has_entry(data: &Data) -> bool {
    data.def_nr("n_main") < data.definitions() || !test_functions(data).is_empty()
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
    emit_tests_of(data, stores, &tests, expected)
}

/// The tests `tests` as one C unit whose `main` runs each in turn, or only the one its first
/// argument names — the test runner (`loft test --native` on C, @PLN203 C5) runs every test
/// in a process of its own, so each starts from fresh stores as on the interpreter.  The exit
/// status is 1 when any test that ran failed.
///
/// # Errors
/// The first [`Refusal`].
pub fn emit_tests_of(
    data: &Data,
    stores: &Stores,
    tests: &[u32],
    expected: &dyn Fn(&str) -> Vec<String>,
) -> Result<String, Refusal> {
    // A failed test `longjmp`s past its frames' frees: the unit keeps the texts they hold
    // (`LOFT_HOLD`) so the halt releases them (loft#2198).
    let mut out = format!(
        "#define LOFT_HALT_RELEASE 1\n{}",
        emit_unit(data, stores, tests)?
    );
    let _ = write!(
        out,
        "\nint main(int argc, char **argv) {{\n    int failed = 0;\n    const char *loft_only = argc > 1 ? argv[1] : 0;\n    loft_bind_tables();\n    loft_start(argc, argv);\n    {}\n    loft_in_test = 1;\n",
        anchor(stores)
    );
    for &d in tests {
        let name = data.def(d).original_name();
        let subs = expected(&name);
        let _ = writeln!(
            out,
            "    if (!loft_only || !strcmp(loft_only, {})) {{",
            body::c_string(&name)
        );
        if subs.is_empty() {
            let _ = writeln!(
                out,
                "    LOFT_FRAMES_RESET();\n    if (setjmp(loft_test_jmp) == 0) {}(); else {{ failed++; fputs(\"FAIL {name}\\n\", stderr); }}",
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
                "    LOFT_FRAMES_RESET();\n    {{ const loft_str want[] = {{ {} }};\n      \
                 if (setjmp(loft_test_jmp) == 0) {{ {}(); failed++; fputs(\"FAIL {name} (expected to fail)\\n\", stderr); }}\n      \
                 else if (!loft_failed_with(want, {})) {{ failed++; fputs(\"FAIL {name}\\n\", stderr); }} }}",
                list.join(", "),
                c_name(data, d),
                subs.len()
            );
        }
        out.push_str("    }\n");
    }
    out.push_str("    if (!failed) loft_leak_report();\n    return failed ? 1 : 0;\n}\n");
    Ok(out)
}
