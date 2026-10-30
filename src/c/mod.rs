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
pub mod component;
mod deliver;
mod eq;
mod files;
mod fnval;
mod parse;
mod reflect;
mod show;
pub(crate) mod tls;
mod unicode;

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use std::fmt::Write as _;

/// The C runtime every unit starts with.
const RUNTIME: &str = concat!(
    include_str!("loft_cc.h"),
    include_str!("loft_text.h"),
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
    include_str!("loft_sock.h"),
    include_str!("loft_https.h"),
    include_str!("loft_net.h"),
    include_str!("loft_process.h"),
    include_str!("loft_engine.h"),
    // A library's native part behind the loft-ffi ABI (@PLN205): the ABI's own header, then
    // the host side.
    "#ifdef LOFT_FFI\n",
    include_str!("../../loft-ffi/include/loft_ffi.h"),
    include_str!("loft_ffi_host.h"),
    "#endif\n"
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
    /// @PLN204 — an integer whose type fits 32 bits (`i32`, `u8`, a `limit` inside `i32`,
    /// every plain `integer` of a `#width 32` file), `int32_t`; null is `LOFT_NULL32`.  Read
    /// where a 64-bit value is wanted it widens (`loft_w64`), and a 64-bit value stored into
    /// one narrows (`loft_n32`), so only the operations written for it run in 32 bits.
    Int32,
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

/// @PLN204 — `LOFT_NO_C_INT32=1` holds every integer in `int64_t`, as before `CType::Int32`:
/// the first bisect step for a C-only wrong answer on a program with 32-bit integers.
pub(crate) fn no_c_int32() -> bool {
    static NO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO.get_or_init(|| std::env::var_os("LOFT_NO_C_INT32").is_some_and(|v| v != "0"))
}

/// The C shape a native runtime function takes or answers for loft shape `c`: the runtime is
/// written in `int64_t`, so a 32-bit integer crosses it widened.
pub(crate) fn native_shape(c: CType) -> CType {
    if c == CType::Int32 { CType::Int } else { c }
}

/// The C type a local or a result of shape `c` is held in.
pub(crate) fn held(c: CType) -> String {
    match c {
        CType::Int => "int64_t".into(),
        CType::Int32 => "int32_t".into(),
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
    /// The prebuilt native parts the unit calls ([`library_native`]), each as its package
    /// directory and file: a unit that calls one is compiled with `LOFT_FFI`, and a shipped
    /// program opens each at start by search order (`loft_cboundlibs`).
    pub(crate) ffi_libs: std::cell::RefCell<std::collections::BTreeSet<(String, String)>>,
    /// The unit is a browser page's module (`--native-c --html`, @PLN203 C6): a library native
    /// the page supplies is a `loft_gl` import (`host_imports`), and the program starts from
    /// the exported `loft_start`.
    pub(crate) browser: bool,
    /// The `par` sites written on threads so far, which numbers each one's body and context.
    pub(crate) par_sites: std::cell::Cell<u32>,
    /// The keyed collection types a `sqlite:` plan was written for (`loft_sqlplan{tp}`).
    pub(crate) sql_plans: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// The lazy fetch functions written so far (`loft_lazy_fetch{n}`).
    pub(crate) lazy_fetchers: std::cell::Cell<usize>,
    /// The place twins the unit calls (`twin_name`): a function by definition number, with
    /// the mask of its `&` parameters that receive a store place rather than a variable.
    twins: std::cell::RefCell<Vec<(u32, u64)>>,
    /// The compiled-target rewrite switches (`LOFT_NO_*`), read once, as native reads them.
    pub(crate) switches: crate::lower::switches::Switches,
    /// The loop verdict's memos shared across the unit (`hoist::hoistable`,
    /// `hoist::callee_inputs`): which callees write a store, their write sets, their inputs.
    pub(crate) hoist_memo: std::cell::RefCell<HoistMemo>,
    /// `@FR-R-ValueRecord`'s plan (`opt::value_records`), computed on first use: empty under
    /// `LOFT_NO_VALUE_RECORD=1` and for a program whose plan holds a form the C does not
    /// write (`Cx::value_records`).
    vr: std::cell::OnceCell<std::rc::Rc<crate::opt::value_records::layout::ValueRecords>>,
    /// `vr_ptr_params`'s answers, per function.
    vr_ptr: std::cell::RefCell<std::collections::HashMap<u32, std::rc::Rc<Vec<usize>>>>,
    apr: std::cell::OnceCell<std::rc::Rc<crate::lower::apart_records::RecPlan>>,
    ap2: std::cell::OnceCell<std::rc::Rc<crate::lower::apart_value::ApartPlan>>,
}

/// The memos [`Cx::hoist_memo`] keeps.
#[derive(Default)]
pub(crate) struct HoistMemo {
    pub(crate) writes: crate::opt::body_writes::WriteCache,
    pub(crate) inputs: crate::opt::hoist::InputCache,
    pub(crate) stores_written: std::collections::HashMap<u32, bool>,
}

/// The mask bit of an `__inv` twin (`@FR-R-Inputs`): the function written again taking the
/// headers its caller holds for the vectors it indexes (`hoist::callee_inputs`).  The bits
/// below it are the place mask, one per `&` parameter.
pub(crate) const INV_BIT: u64 = 1 << 63;

/// The mask bit of a `__nk` twin (`@FR-R-HandOver`'s rotate clause): a suffix-keep callee
/// written again without its keep statement, which the rotate has done
/// (`lower::hand_over::keep_statement`).
pub(crate) const NK_BIT: u64 = 1 << 62;

/// The mask bit of an `__apr` twin (`@FR-R-Apart` instance 4): the function written again taking
/// its apart parameter as the value's field buffers (`loft_text *loft_aprp`), not a record.
pub(crate) const APR_BIT: u64 = 1 << 61;

/// The mask bit of an `__apv` twin (`@FR-R-Apart` instance 2): the function written again
/// answering its record of fixed-length float vectors as a value through `loft_apo`, taking its
/// record parameters as values (`const loft_ap_<T> *`) and no return buffer.
pub(crate) const APV_BIT: u64 = 1 << 60;

/// The mask bit of a `__kh` twin (`@FR-R-Header`'s growth half, the kept-twin clause): the
/// function written again taking its caller's header for its one vector parameter by pointer
/// (`loft_vh *khp`), reading through it and deriving it again after each growth, so the
/// caller's header is current when the call returns (`opt::apart::kept_growth_param`).
pub(crate) const KH_BIT: u64 = 1 << 59;

/// The bits of a `__nn` twin (`@FR-R-NonSentinelParam`'s twin clause), one per parameter from
/// bit 32: the function written again with those parameters seeded as non-sentinel, called by
/// a call site whose arguments prove them where the program-wide fact cannot (another caller
/// passes a value that may be null).
pub(crate) const NN_SHIFT: u32 = 32;
pub(crate) const NN_MASK: u64 = ((1u64 << 26) - 1) << NN_SHIFT;

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
    fn new(data: &'a Data, stores: &'a Stores, browser: bool) -> Cx<'a> {
        Cx {
            data,
            stores,
            browser,
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
            ffi_libs: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            par_sites: std::cell::Cell::new(0),
            sql_plans: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            lazy_fetchers: std::cell::Cell::new(0),
            twins: std::cell::RefCell::new(Vec::new()),
            switches: crate::lower::switches::Switches::from_env(),
            hoist_memo: std::cell::RefCell::new(HoistMemo::default()),
            vr: std::cell::OnceCell::new(),
            vr_ptr: std::cell::RefCell::new(std::collections::HashMap::new()),
            apr: std::cell::OnceCell::new(),
            ap2: std::cell::OnceCell::new(),
        }
    }

    /// The value-record plan the C writes from, native's own (`opt::value_records`), or an
    /// empty one: the plan is program-wide, so the C takes it whole or not at all, and it
    /// declines a plan with a function-value dispatch answering a tuple
    /// (`@FR-R-FnRefValue`), which it does not write yet.
    /// `@FR-R-Apart` instance 4's plan for the program (`lower::apart_records::plan`, native's
    /// decision): the records of record vectors held apart, their owners and their twins.
    /// Empty under `LOFT_NO_APART_RECORDS=1`.
    pub(crate) fn apr_plan(&self) -> std::rc::Rc<crate::lower::apart_records::RecPlan> {
        self.apr
            .get_or_init(|| {
                std::rc::Rc::new(if crate::lower::apart_records::disabled() {
                    crate::lower::apart_records::RecPlan::default()
                } else {
                    crate::lower::apart_records::plan(self.data, self.stores)
                })
            })
            .clone()
    }

    /// `@FR-R-Apart` instance 2's plan for the program (`lower::apart_value::plan`, native's
    /// decision), after the value records, whose functions it leaves alone.  Empty under
    /// `LOFT_NO_APART=1`.
    pub(crate) fn ap2_plan(&self) -> std::rc::Rc<crate::lower::apart_value::ApartPlan> {
        self.ap2
            .get_or_init(|| {
                let vr = self.value_records();
                let value_fns: crate::fxhash::FxHashSet<u32> = vr
                    .fns
                    .keys()
                    .copied()
                    .chain(vr.params.keys().copied())
                    .collect();
                std::rc::Rc::new(crate::lower::apart_value::plan(
                    self.data,
                    self.stores,
                    &value_fns,
                ))
            })
            .clone()
    }

    /// The C struct of instance 2's type `rec`, by its layout's name.
    pub(crate) fn ap2_ctype(&self, rec: u32) -> Option<String> {
        self.ap2_plan()
            .types
            .get(&rec)
            .map(|l| format!("loft_ap_{}", l.rust))
    }

    pub(crate) fn value_records(
        &self,
    ) -> std::rc::Rc<crate::opt::value_records::layout::ValueRecords> {
        self.vr
            .get_or_init(|| {
                use crate::opt::value_records::layout as vl;
                let vr = crate::opt::value_records::value_records(self.data, self.stores, false);
                crate::rewrite_census::fired("R-ValueRecord", vr.fns.len());
                let views = vr
                    .types
                    .values()
                    .any(|t| t.fields.iter().any(|(_, rt)| vl::is_view_part(rt)));
                let fnref = vr.fnref_sites.keys().any(|(d, v)| {
                    vl::fnref_value(&vr.fnref_sites, &vr.fns, *d, *v).is_some()
                }) || vr.fnref_sites.values().any(|(arms, _)| {
                    arms.iter()
                        .any(|a| vr.fns.contains_key(a) || vr.params.contains_key(a))
                });
                if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                    eprintln!(
                        "[valuerec] the C takes {} function(s), {} with tuple parameters (views {views}, fn-ref {fnref})",
                        vr.fns.len(),
                        vr.params.len()
                    );
                }
                if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                    for (d, key) in &vr.fns {
                        let view = vr.types.get(key).is_some_and(|t| t.fields.iter().any(|(_, rt)| vl::is_view_part(rt)));
                        eprintln!("[valuerec] the C's plan: {} key {key}{}", self.data.def(*d).name(), if view { " (view)" } else { "" });
                    }
                }
                if fnref {
                    return std::rc::Rc::new(vl::ValueRecords::default());
                }
                std::rc::Rc::new(vr)
            })
            .clone()
    }

    /// The C shape of value-record layout `key`: a `loft_tup` of its fields — `i64`, `f64`,
    /// `f32` and `bool` as integer, float, single and boolean — and, for a nullable layout,
    /// the presence word as a boolean after them.
    /// `d`'s value locals (`opt::value_records::value_locals_in`): each with its layout key.
    pub(crate) fn vr_locals(&self, d: u32) -> std::collections::HashMap<u16, u16> {
        let vr = self.value_records();
        if vr.fns.is_empty() && vr.params.is_empty() {
            return std::collections::HashMap::new();
        }
        crate::opt::value_records::value_locals_in(
            self.data,
            d,
            &vr.fns,
            &vr.params,
            &vr.view_offs,
            &vr.fnref_sites,
            &vr.sp,
        )
    }

    pub(crate) fn vr_ctype(&self, key: u16) -> Option<CType> {
        let vr = self.value_records();
        let layout = vr.types.get(&key)?;
        let mut shapes: Vec<CType> = layout
            .fields
            .iter()
            .map(|(_, rt)| match *rt {
                "f64" => CType::Float,
                "f32" => CType::Single,
                "bool" => CType::Bool,
                rt if crate::opt::value_records::layout::is_view_part(rt) => CType::Ref,
                _ => CType::Int,
            })
            .collect();
        if layout.nullable {
            shapes.push(CType::Bool);
        }
        Some(CType::Tuple(self.tuple_id(shapes)))
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

    /// @PLN204 — the shape a LOCAL of type `tp` is held in, in a function whose body is
    /// `body32` (written under the 32-bit regime, `Data::body_width32`): `int32_t` for an
    /// integer whose type fits 32 bits, because there every value is narrowed before it is
    /// held (`OpNarrow32`, a narrowed call result, `loft_n32` where a 64-bit value is stored).
    /// Everywhere else [`Self::ctype`]: a 64-bit body's `i32` local keeps the wide value it was
    /// given, as the interpreter's 8-byte slot does (`931b-i32-accepted-forms-run`), and a
    /// parameter, a result, a tuple member and a function type stay 64-bit, so two bodies of
    /// different regimes meet through `loft_w64` / `loft_n32`, which keep null as null.
    pub(crate) fn local_ctype(&self, tp: &Type, body32: bool) -> Option<CType> {
        match tp.base() {
            Type::Integer(spec) if body32 && spec.is_width32() && !no_c_int32() => {
                Some(CType::Int32)
            }
            _ => self.ctype(tp),
        }
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
            CType::Int32 => "LOFT_NULL32".into(),
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
            // `@FR-H-Handle`: 2 for the `File` record, 1 for a type that can hold one.
            let file = if t.name == "File" {
                2
            } else {
                u8::from(self.stores.type_holds_file(tp))
            };
            let row = format!(
                "{}, {}, {keys}, {}, {file} }}",
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
        // `@FR-R-Apart` instance 2: each record of fixed-length float vectors as its value.
        if let Some(ap2) = self.ap2.get() {
            let mut types: Vec<&crate::lower::apart_value::ApLayout> = ap2.types.values().collect();
            types.sort_by(|a, b| a.rust.cmp(&b.rust));
            for t in types {
                let fields: Vec<String> = t
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(k, f)| format!("double f{k}[{}];", f.len.max(1)))
                    .collect();
                let _ = writeln!(
                    out,
                    "typedef struct {{ {} }} loft_ap_{};",
                    fields.join(" "),
                    t.rust
                );
            }
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
/// An `__inv` twin ([`INV_BIT`]) adds `__inv` after the place mask's name.
pub(crate) fn twin_name(data: &Data, d: u32, mask: u64) -> String {
    let places = mask & !INV_BIT & !NK_BIT & !APR_BIT & !APV_BIT & !KH_BIT & !NN_MASK;
    let inv = if mask & INV_BIT == 0 { "" } else { "__inv" };
    let kh = if mask & KH_BIT == 0 { "" } else { "__kh" };
    let nn_bits = (mask & NN_MASK) >> NN_SHIFT;
    let kh = if nn_bits == 0 {
        kh.to_string()
    } else {
        format!("{kh}__nn{nn_bits:x}")
    };
    let nk = if mask & NK_BIT == 0 { "" } else { "__nk" };
    let apr = if mask & APR_BIT == 0 { "" } else { "__apr" };
    let apv = if mask & APV_BIT == 0 { "" } else { "__apv" };
    if places == 0 {
        format!("{}{inv}{nk}{apr}{apv}{kh}", c_name(data, d))
    } else {
        format!("{}_pl{places}{inv}{nk}{apr}{apv}{kh}", c_name(data, d))
    }
}

impl Cx<'_> {
    /// `d`'s invariant inputs (`@FR-R-Inputs`, `hoist::callee_inputs`): the vectors it indexes
    /// that a caller holding their headers may hand it.  `None` when it has none, under
    /// `LOFT_NO_CALLEE_INPUTS=1` / `LOFT_NO_VECTOR_HOIST=1`, or when it is not a loft body.
    pub(crate) fn callee_inputs(
        &self,
        d: u32,
    ) -> Option<std::rc::Rc<crate::opt::hoist::CalleeInputs>> {
        if self.switches.callee_inputs_disabled
            || std::env::var("LOFT_NO_VECTOR_HOIST").is_ok_and(|v| v != "0")
        {
            return None;
        }
        let memo = &mut *self.hoist_memo.borrow_mut();
        crate::opt::hoist::callee_inputs(
            d,
            self.data,
            self.stores,
            &mut memo.stores_written,
            &mut memo.writes,
            &mut memo.inputs,
        )
    }

    /// Does a call to `d` run no store write?  Then a text argument may view the store it was
    /// read from (`@FR-F-ParamText`, `FnWriter::text_reads`): nothing the call runs can move
    /// or rewrite its bytes.  The verdict the hoist holds a vector's base across a call on.
    /// `@FR-R-ValueRecord`'s pointer clause: the parameter positions of `d` at which a value
    /// record arrives as `const loft_tupN *` instead of by value — a struct of three or more
    /// members the body only READS (`vr_param_read_only`), so the callee reads the caller's
    /// struct where it is and nothing is copied onto the stack at the call.  Every variant of
    /// `d` and every call to it ask this one place, so they agree.  `LOFT_NO_VR_PTR=1` passes
    /// every value record by value again.
    pub(crate) fn vr_ptr_params(&self, d: u32) -> std::rc::Rc<Vec<usize>> {
        if let Some(known) = self.vr_ptr.borrow().get(&d) {
            return known.clone();
        }
        let mut out = Vec::new();
        let off = crate::env_once!(std::env::var("LOFT_NO_VR_PTR").is_ok_and(|v| v != "0"));
        let vr = self.value_records();
        // An `__apv` twin's call site writes its own arguments (`av_twin_call`): such a
        // function keeps every value record by value, in every variant.
        let apv = self.ap2_plan().twins.contains_key(&d);
        if !off
            && !apv
            && let Some(params) = vr.params.get(&d)
        {
            let def = self.data.def(d);
            let vars = def.variables();
            let args = vars.arguments();
            for (&i, &key) in params {
                let Some(CType::Tuple(id)) = self.vr_ctype(key) else {
                    continue;
                };
                let Some(&a) = args.get(i) else {
                    continue;
                };
                if self.tuple_shapes(id).len() >= 3
                    && !matches!(vars.tp(a).base(), Type::RefVar(_))
                    && vr_param_read_only(self.data, def.code(), a)
                {
                    out.push(i);
                }
            }
        }
        out.sort_unstable();
        let out = std::rc::Rc::new(out);
        self.vr_ptr.borrow_mut().insert(d, out.clone());
        out
    }

    pub(crate) fn call_keeps_stores(&self, d: u32) -> bool {
        let memo = &mut *self.hoist_memo.borrow_mut();
        let mut active = std::collections::HashSet::new();
        !crate::opt::writes::call_writes_store(
            d,
            self.data,
            &mut memo.stores_written,
            &mut active,
            None,
        )
    }

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
    emit_unit_for(data, stores, roots, false)
}

/// [`emit_unit`] for a browser page's module when `browser` (`Cx::browser`).
///
/// # Errors
/// The first function the C backend does not carry yet.
pub fn emit_unit_for(
    data: &Data,
    stores: &Stores,
    roots: &[u32],
    browser: bool,
) -> Result<String, Refusal> {
    crate::opt::non_sentinel::begin_c_generation();
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
    let cx = Cx::new(data, stores, browser);
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
    out.push_str(&c_libraries(data, &cx.ffi_libs.borrow()));
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
         loft_fault_logger = loft_log_production_fault;\n    \
         loft_locked_logger = loft_log_locked_write;\n",
    );
    // The type table, for what a store's free walks (`@FR-H-Handle`): a unit that describes no
    // type declares none, and the runtime's pointer stays null.
    if out.contains("static const uint32_t loft_ntypes =") {
        bind.push_str("    loft_T = loft_types;\n    loft_T_n = loft_ntypes;\n");
    }
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
/// built into its static archive for the compiler that links the unit (`c_shim::cc_archive`;
/// off Windows the same artefact a `--static` program links) and named in a `/* link: <path> */` line for whoever
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
fn c_libraries(data: &Data, ffi_libs: &std::collections::BTreeSet<(String, String)>) -> String {
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
    // one's package, version and file, never its directory — a package's `#c` libraries, and
    // the prebuilt native parts the unit calls (`loft_ffi_sym` finds its symbols there).
    let row = |pkg_dir: &str, file: String, shim: bool| {
        let (package, version) = package_identity(pkg_dir);
        format!(
            "{{ {}, {}, {}, {} }}",
            body::c_string(&package),
            body::c_string(&version),
            body::c_string(&file),
            u8::from(shim)
        )
    };
    let mut rows: Vec<String> = data
        .shipped_c_libraries()
        .into_iter()
        .map(|(lib, path)| {
            row(
                &lib.pkg_dir,
                crate::file_access::file_name(path.as_path()).unwrap_or_default(),
                !lib.shim.is_empty(),
            )
        })
        .collect();
    for (pkg_dir, lib) in ffi_libs {
        rows.push(row(
            pkg_dir,
            crate::file_access::file_name(lib.as_str()).unwrap_or_default(),
            false,
        ));
    }
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
            // Archived for the compiler that links this unit (`c_shim::cc_archive`), whatever
            // the host's static-link archiver for rustc is.  The sources are named
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
    let runtimes: Vec<&str> = reached
        .iter()
        .filter_map(|&d| in_binary_native(data.def(d).native()))
        .collect();
    if runtimes
        .iter()
        .any(|r| matches!(*r, "loft_process_run" | "loft_git_query"))
    {
        out.push_str("#define LOFT_PROCESS 1\n");
    }
    if runtimes.iter().any(|r| {
        r.starts_with("loft_kernel_") || matches!(*r, "loft_live_flip" | "loft_swap_retired")
    }) {
        out.push_str("#define LOFT_ENGINE 1\n");
    }
    out.push_str(&fetch_defines(data, reached, &runtimes, cx.lazy.get()));
    // A page's module jumps nowhere unless a lazy driver's fault must be contained, so it is
    // built without wasm's exception handling, which asyncify cannot carry (`c_driver`).
    if cx.browser && !cx.lazy.get() {
        out.push_str("#define LOFT_NO_JUMPS 1\n");
    }
    if !cx.ffi_libs.borrow().is_empty() {
        out.push_str("#define LOFT_FFI 1\n");
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
    // The call-depth cap both other backends enforce (`limits::MAX_CALL_DEPTH`), stated here so
    // the runtime cannot hold a copy of the number that drifts from it.
    let _ = writeln!(
        out,
        "#define LOFT_MAX_CALL_DEPTH {}",
        crate::limits::MAX_CALL_DEPTH
    );
    (out, parses, reflects)
}

/// A unit that fetches over a URL: `LOFT_NET` for the whole-image loads and for the paged loaders
/// and lazy bindings, whose image may be an `http(s)://` one read by range (`loft_net.h`), and for
/// any of them or `http_fetch` the TLS client its `https://` requests run over
/// (`tls::unit_lines`).
fn fetch_defines(
    data: &Data,
    reached: &std::collections::HashSet<u32>,
    runtimes: &[&str],
    lazy: bool,
) -> String {
    let loads = lazy
        || reached.iter().any(|&d| {
            let name = data.def(d).name();
            matches!(name, "n_store_load_url" | "n_store_load_url_trusted")
                || native_c(name) == Some("loft_pg_load_keys")
        });
    let mut out = String::new();
    if loads {
        out.push_str("#define LOFT_NET 1\n");
    }
    if loads || runtimes.contains(&"loft_kernel_http_fetch") {
        out.push_str(&tls::unit_lines());
    }
    out
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
    // The rewrites this unit admitted, written where `LOFT_REWRITE_CENSUS` asks, as the native
    // writer writes its own after emitting: the census can then set the two writers side by side.
    let out = emit_program_counted(data, stores, expected);
    crate::rewrite_census::write();
    out
}

fn emit_program_counted(
    data: &Data,
    stores: &Stores,
    expected: &dyn Fn(&str) -> Vec<String>,
) -> Result<String, Refusal> {
    let main = data.def_nr("n_main");
    if main >= data.definitions() {
        return emit_tests(data, stores, expected);
    }
    let main = program_main(data, main)?;
    let mut out = emit_unit(data, stores, &[main])?;
    let _ = write!(
        out,
        "\nint main(int argc, char **argv) {{\n    loft_bind_tables();\n    loft_start(argc, argv);\n    {}\n    {}();\n    loft_leak_report();\n    return loft_had_fatal;\n}}\n",
        anchor(stores),
        c_name(data, main)
    );
    Ok(out)
}

/// The program's `main`, refused when it takes arguments.
fn program_main(data: &Data, main: u32) -> Result<u32, Refusal> {
    if data.def(main).attributes().is_empty() {
        Ok(main)
    } else {
        Err(Refusal {
            function: "main".into(),
            why: "a main that takes arguments".into(),
        })
    }
}

/// C99 for a browser page's module (`--native-c --html`, @PLN203 C6): the unit as
/// [`emit_unit_for`] writes it for the browser, and the exported `loft_start` the page's script
/// runs it by — under asyncify, which suspends it at each frame the program yields and resumes
/// it from the same export.  The libc's constructors run first, once: a page never calls the
/// WASI `_start` that would; and since the program ends by returning rather than by `exit`, the
/// export flushes what it printed itself.
///
/// # Errors
/// A program without a `main` that takes nothing, or the first [`Refusal`].
pub fn emit_browser_program(data: &Data, stores: &Stores) -> Result<String, Refusal> {
    let main = data.def_nr("n_main");
    if main >= data.definitions() {
        return Err(Refusal {
            function: "main".into(),
            why: "a page without a main".into(),
        });
    }
    let main = program_main(data, main)?;
    let mut out = emit_unit_for(data, stores, &[main], true)?;
    out.push_str(ASYNCIFY_REGION);
    let _ = write!(
        out,
        "\nextern void __wasm_call_ctors(void);\n\
         __attribute__((export_name(\"loft_start\"))) void loft_page_start(void) {{\n    \
         static int started = 0;\n    if (!started) {{ started = 1; __wasm_call_ctors(); }}\n    \
         loft_bind_tables();\n    loft_start(0, 0);\n    {}\n    {}();\n    loft_leak_report();\n    \
         fflush(stdout);\n    fflush(stderr);\n}}\n\
         int main(void) {{ return 0; }}\n",
        anchor(stores),
        c_name(data, main)
    );
    Ok(out)
}

/// loft#950 — the asyncify save region a page's module owns, as the Rust writer's
/// (`emit_asyncify_region`): the 8-byte control struct asyncify takes, the save area after it,
/// and a canary after that, so an overflow is reported (`loft_asyncify_ok`) instead of
/// overwriting the next static.  The page's controller (`doc/loft-asyncify.js`) reads where it
/// is and how large; it never picks an address itself.
const ASYNCIFY_REGION: &str = r#"
#define LOFT_ASYNCIFY_SAVE 16384
static struct __attribute__((aligned(16))) {
    uint8_t ctrl[8];
    uint8_t save[LOFT_ASYNCIFY_SAVE];
    uint8_t canary[8];
} loft_asyncify_region = { { 0 }, { 0 }, { 0x4c, 0x4f, 0x46, 0x54, 0x39, 0x35, 0x30, 0x21 } };
__attribute__((export_name("loft_asyncify_data"))) uint32_t loft_asyncify_data(void) {
    return (uint32_t) (uintptr_t) &loft_asyncify_region;
}
__attribute__((export_name("loft_asyncify_size"))) uint32_t loft_asyncify_size(void) {
    return LOFT_ASYNCIFY_SAVE;
}
__attribute__((export_name("loft_asyncify_ok"))) int32_t loft_asyncify_ok(void) {
    static const uint8_t canary[8] = { 0x4c, 0x4f, 0x46, 0x54, 0x39, 0x35, 0x30, 0x21 };
    return memcmp(loft_asyncify_region.canary, canary, 8) == 0;
}
"#;

/// The statement a generated `main` starts with: the directory a relative path resolves against
/// (the program's own, as the interpreter's `Stores::resolve_path` anchors it) and whether
/// the program anchors there at all (`#cwd` says no).
fn anchor(stores: &Stores) -> String {
    format!("loft_set_anchor({});", i32::from(stores.program_relative))
}

/// The C shape of each of a function's variables, in number order: a `&` parameter its target's,
/// a local nothing binds or one only ever bound to `null` the integer null's.
fn variable_shapes(
    cx: &Cx,
    vars: &crate::variables::Function,
    args: &[u16],
    body32: bool,
) -> Result<Vec<CType>, String> {
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
        let shape = if args.contains(&v) {
            cx.ctype(tp)
        } else {
            cx.local_ctype(tp, body32)
        };
        let c = shape.filter(|c| *c != CType::Void).ok_or_else(|| {
            let what = if args.contains(&v) {
                "parameter"
            } else {
                "local"
            };
            format!("{what} of type {}", type_word(tp))
        })?;
        shapes.push(c);
    }
    Ok(shapes)
}

/// Whether `body` only READS value-record parameter `a`: every mention of it is a field read,
/// a sub-record read, a presence test, a copy's source, a release (a tuple holds no store, so
/// it is nothing), a whole-value bind to another variable or a by-value argument of a user
/// function.  An allow-list: a mention of any other kind — a rebind, a field write, a copy
/// into it, a `&` argument, an operator not listed — answers false, and the parameter stays
/// by value (`Cx::vr_ptr_params`, `@FR-R-ValueRecord`).
fn vr_param_read_only(data: &crate::data::Data, body: &Value, a: u16) -> bool {
    let is_a = |v: &Value| matches!(v.unspan(), Value::Var(x) if *x == a);
    let mentions = |v: &Value| v.any_node(&mut |n| matches!(n, Value::Var(x) if *x == a));
    let mut ok = true;
    body.any_node(&mut |n| {
        match n {
            Value::Set(v, _) | Value::TuplePut(v, _, _) if *v == a => ok = false,
            Value::Call(op, args) if (*op as usize) < data.definitions.len() => {
                let def = data.def(*op);
                let name = def.name();
                for (i, x) in args.iter().enumerate() {
                    let writes_into =
                        (name.starts_with("OpSet") && i == 0) || (name == "OpCopyRecord" && i == 1);
                    if writes_into && mentions(x) {
                        ok = false;
                    } else if is_a(x) {
                        let read = crate::opt::loop_records::SCALAR_GETTERS.contains(&name)
                            || matches!(
                                name,
                                "OpGetField"
                                    | "OpConvBoolFromRef"
                                    | "OpRefIsNull"
                                    | "OpFreeRef"
                                    | "OpFreeRefIfDistinct"
                                    | "OpClear"
                            )
                            || (name == "OpCopyRecord" && i == 0)
                            || (!name.starts_with("Op")
                                && def
                                    .attributes()
                                    .get(i)
                                    .is_some_and(|p| !matches!(p.typedef.base(), Type::RefVar(_))));
                        if !read {
                            ok = false;
                        }
                    }
                }
            }
            _ => {}
        }
        !ok
    });
    ok
}

/// `@FR-R-Inline`: a function of at most twelve statements is declared `inline`.  The hint
/// raises the compiler's inlining budget for it, so a small helper called from a hot loop —
/// a push through `put`, an accessor — is taken into its caller, and the caller's locals stay
/// in registers across what was a call.  GCC left such a helper out of line and reloaded the
/// caller's spilled state after every call (bench 20: -69 % cycles); Clang decides alone.
/// Measured over every bench: nothing else moved beyond noise.  `LOFT_NO_INLINE_HINT=1` drops
/// the hint.
fn small_function(statements: usize) -> bool {
    statements <= 12
        && !crate::env_once!(std::env::var("LOFT_NO_INLINE_HINT").is_ok_and(|v| v != "0"))
}

/// The signature and the body of function `d`.
#[expect(clippy::too_many_lines)]
fn function(cx: &Cx, d: u32, mask: u64) -> Result<(String, String), String> {
    let data = cx.data;
    let places = mask & !INV_BIT & !NK_BIT & !APR_BIT & !APV_BIT & !KH_BIT & !NN_MASK;
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
    // `@FR-R-ValueRecord` — an admitted function returns its record's fields as a struct.
    let vr = cx.value_records();
    let vr_key = vr.fns.get(&d).copied();
    let ret = match vr_key {
        Some(key) => cx
            .vr_ctype(key)
            .ok_or("a value record without its layout")?,
        None => cx
            .ctype(def.returned())
            .ok_or_else(|| format!("returns {}", type_word(def.returned())))?,
    };
    if unloaded {
        // A page supplies a library's natives as host imports (@PLN203 C6).
        if cx.browser
            && let Some(imp) =
                crate::host_imports::host_import(cx.data, d, &std::iter::once(d).collect())
        {
            return browser_import_native(cx, d, ret, &imp);
        }
        // …and a library native it has no import for cannot be called from a page: refused at
        // build, as the Rust writer refuses it (`compile_error!` naming the function).
        if cx.browser && cx.data.native_symbol_crates.contains_key(def.native()) {
            return Err(format!(
                "native {} (`#native \"{}\"` has no browser host import, so it cannot be \
                 called from a page; avoid it on the browser target)",
                def.original_name(),
                def.native()
            ));
        }
        if let Some(runtime) = in_binary_native(def.native()) {
            return forwarded_native(cx, d, ret, runtime);
        }
        if let Some((pkg_dir, lib)) = prebuilt_library(cx.data, def.native()) {
            return library_native(cx, d, ret, &pkg_dir, &lib);
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
    // `@FR-R-Apart` instance 2: an `__apv` twin answers its value through `loft_apo` and takes
    // no return buffer; its record parameters arrive as values.
    let apv = if mask & APV_BIT == 0 {
        None
    } else {
        let plan = cx.ap2_plan();
        let rec = *plan
            .twins
            .get(&d)
            .ok_or("an __apv twin of a function the plan has no twin for")?;
        let roles = plan.twin_roles.get(&d).cloned().unwrap_or_default();
        Some((rec, roles, def.hidden_return_buffer_attr()))
    };
    let ret = if apv.is_some() { CType::Void } else { ret };
    let vars = def.variables();
    // The parameters are the argument variables, in number order: `ref_return` can promote a
    // local into the return buffer's place, so parameter i is not always variable i.
    let args = vars.arguments();
    if args.len() != def.attributes().len() {
        return Err("parameters that do not match their arguments".into());
    }
    let mut shapes = variable_shapes(cx, vars, &args, data.body_width32(d))?;
    let dropped = vr_shapes(cx, d, vr_key, &mut shapes);
    // `@FR-R-Apart` instance 4: an `__apr` twin takes its apart parameter as the field buffers.
    let apr_at = if mask & APR_BIT == 0 {
        None
    } else {
        Some(
            cx.apr_plan()
                .twins
                .get(&d)
                .ok_or("an __apr twin of a function the plan has no twin for")?
                .1,
        )
    };
    let vr_ptr = cx.vr_ptr_params(d);
    let params: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| dropped != Some(*i))
        .filter(|(i, _)| apv.as_ref().is_none_or(|(_, _, rb)| *rb != Some(*i)))
        .map(|(i, &a)| {
            let c = shapes[a as usize];
            let name = body::var_name(vars.name(a), a);
            let apv_rec = apv.as_ref().and_then(|(_, roles, _)| roles.apart.get(&a));
            if let Some(t) = apv_rec.and_then(|r| cx.ap2_ctype(*r)) {
                format!("const {t} *loft_avp{a}")
            } else if apr_at == Some(i) {
                "loft_text *loft_aprp".to_string()
            } else if places & (1 << i) != 0 || vars.is_store_text_link(a) {
                format!("loft_ref {name}")
            } else if matches!(vars.tp(a).base(), Type::RefVar(_)) {
                format!("{} *{name}", held(c))
            } else if vr_ptr.contains(&i) {
                format!("const {} *{name}", passed(c))
            } else {
                format!("{} {name}", passed(c))
            }
        })
        .collect();
    // An `__inv` twin takes the header of each vector it indexes from its caller, which holds
    // them; its body serves those vectors' accesses from them as a loop's held headers.
    let mut params = params;
    if let Some((rec, _, _)) = &apv {
        let t = cx.ap2_ctype(*rec).ok_or("an __apv twin of no apart type")?;
        params.push(format!("{t} *loft_apo"));
    }
    let (mut inputs, scalar_inputs) = inv_inputs(cx, d, mask, &mut params)?;
    // A `__kh` twin: the caller's header for its vector parameter, by pointer — held, with its
    // base, and derived again through the pointer after each growth (`kept`).
    let kept = mask & KH_BIT != 0;
    if kept {
        let k = crate::opt::apart::kept_growth_param(data, d)
            .ok_or("a __kh twin of a function that keeps no header")?;
        params.push("loft_vh *khp".to_string());
        inputs.push((
            (
                u16::try_from(k).map_err(|_| "a parameter out of range")?,
                Vec::new(),
            ),
            "(*khp)".to_string(),
            true,
        ));
    }
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let name = twin_name(data, d, mask);
    let sig = format!("static {} {name}({params})", held(ret));
    // `LOFT_REWRITE_CENSUS_FN`: this body's admissions under its own name, phase `c`, so they
    // line up with the native writer's per function.
    let _census_body = crate::rewrite_census::InBody::enter("c", def.name());
    if let Type::Iterator(elem, _) = def.returned().base() {
        if mask != 0 {
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
        .inputs(inputs)
        .scalar_inputs(scalar_inputs)
        .kept_growth(kept)
        .nn_seed(mask)
        .nokeep(mask & NK_BIT != 0)?
        .apart(mask & APR_BIT != 0)
        .apart_value(mask & APV_BIT != 0)
        .write()?;
    let (body, statements) = body;
    // `@FR-R-Inline`: a small function is declared `inline`, its frame wrapper with it.
    let inline = small_function(statements);
    let sig = if inline {
        sig.replacen("static ", "static inline ", 1)
    } else {
        sig
    };
    // Every function keeps its frame: the pushes are macros that a build without the call
    // chain (`LOFT_CALL_CHAIN`, `LOFT_FRAMES`) compiles to nothing, the wrapper then a call
    // the compiler folds into its caller.
    Ok((sig, framed(cx, d, &name, &params, ret, &body, inline)))
}

/// The shape an `__inv` twin takes a scalar input in (`@FR-R-Inputs`' scalar clause): the
/// getter's result, for the kinds a loop holds a record scalar in; `None` for any other, which
/// the twin reads from its record as before.  Asked by the twin's signature and by its call
/// site alike, so the two cannot disagree on which inputs cross.
pub(crate) fn twin_scalar_shape(cx: &Cx, getter: &Value) -> Option<CType> {
    let Value::Call(g, _) = getter.unspan() else {
        return None;
    };
    let c = cx.ctype(cx.data.def(*g).returned())?;
    matches!(
        c,
        CType::Int | CType::Float | CType::Single | CType::Bool | CType::Enum | CType::Char
    )
    .then_some(c)
}

/// An `__inv` twin's inputs, appended to `params` — first each record scalar it reads
/// (`is{k}`, rustc's `__is_k`), then each header (`hi{k}`) — and the frames its body serves
/// them from: the scalars as a loop's held scalars, the headers as its held headers.  None for
/// any other mask.
#[allow(clippy::type_complexity)]
fn inv_inputs(
    cx: &Cx,
    d: u32,
    mask: u64,
    params: &mut Vec<String>,
) -> Result<
    (
        Vec<(crate::opt::writes::PathKey, String, bool)>,
        Vec<(crate::opt::hoist::ScalarKey, String, CType)>,
    ),
    String,
> {
    let mut inputs = Vec::new();
    let mut scalars = Vec::new();
    if mask & INV_BIT != 0 {
        let fi = cx
            .callee_inputs(d)
            .ok_or("an __inv twin of a function with no inputs")?;
        for (k, (p, fld, getter)) in fi.scalars.iter().enumerate() {
            if let Some(c) = twin_scalar_shape(cx, getter) {
                params.push(format!("{} is{k}", held(c)));
                scalars.push(((*p, *fld), format!("is{k}"), c));
            }
        }
        for (k, (p, offs, _)) in fi.headers.iter().enumerate() {
            params.push(format!("loft_vh hi{k}"));
            inputs.push(((*p, offs.clone()), format!("hi{k}"), true));
        }
        if inputs.is_empty() && scalars.is_empty() {
            return Err("an __inv twin of a function with no inputs".into());
        }
    }
    Ok((inputs, scalars))
}

/// `@FR-R-ValueLocal` — `d`'s value locals (the tuple parameters among them) take their struct
/// shapes, and an admitted function's return buffer is no parameter: answers the attribute
/// the signature drops.
fn vr_shapes(cx: &Cx, d: u32, vr_key: Option<u16>, shapes: &mut [CType]) -> Option<usize> {
    let locals = cx.vr_locals(d);
    crate::rewrite_census::fired("R-ValueLocal", locals.len());
    for (v, key) in &locals {
        if let Some(c) = cx.vr_ctype(*key)
            && (*v as usize) < shapes.len()
        {
            shapes[*v as usize] = c;
        }
    }
    vr_key.and_then(|_| crate::opt::attrs::ret_buffer_attr(cx.data.def(d)))
}

/// A `#c` binding (`c_signature`, `c_call`): a wrapper declaring the callee by the author's own
/// signature text — so it agrees with a system header that declares it too — and calling it with
/// each scalar cast to its declared C type and each text as a NUL-terminated copy; an integer
/// result widened to 64 bits by its signedness, a floating one as is.  A vector argument is not
/// carried yet.
#[expect(
    clippy::too_many_lines,
    reason = "one wrapper: the callee's declaration in both link forms, each argument's cast, the result's read"
)]
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
                 extern {ret_text} ({})({params_text});\n    \
                 {ret_text} (*const loft_cfp)({params_text}) = ({});\n\
                 #endif",
                sig.symbol,
                sig.symbol,
                sym = body::c_string(&sig.symbol),
            ),
            format!("loft_cfp({})", args.join(", ")),
        )
    } else {
        // The name in parentheses: a C library may also define it as a function-like macro
        // (macOS's fortified `<string.h>` does for `memcpy`), and a parenthesised name is
        // never expanded, so the prototype and the call name the function itself.
        (
            format!("    extern {ret_text} ({})({params_text});", sig.symbol),
            format!("({})({})", sig.symbol, args.join(", ")),
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
        // `lib/engine_host`'s kernel (`loft_engine.h`).  Its build swap and background rebuild
        // (`swap_world`, `swap_start`, `rebuild_*`) are not carried: a unit that calls one refuses.
        "n_kernel_listen" => Some("loft_kernel_listen"),
        "n_kernel_pump" => Some("loft_kernel_pump"),
        "n_kernel_next_event" => Some("loft_kernel_next_event"),
        "n_kernel_event_cid" => Some("loft_kernel_event_cid"),
        "n_kernel_event_kind" => Some("loft_kernel_event_kind"),
        "n_kernel_event_payload" => Some("loft_kernel_event_payload"),
        "n_kernel_event_status" => Some("loft_kernel_event_status"),
        "n_kernel_tick_due" => Some("loft_kernel_tick_due"),
        "n_kernel_send" => Some("loft_kernel_send"),
        "n_kernel_broadcast" => Some("loft_kernel_broadcast"),
        "n_kernel_http_fetch" => Some("loft_kernel_http_fetch"),
        "n_live_flip" => Some("loft_live_flip"),
        "n_kernel_swap_step" => Some("loft_kernel_swap_step"),
        "n_swap_retired" => Some("loft_swap_retired"),
        "n_kernel_idle" => Some("loft_kernel_idle"),
        "n_kernel_clients" => Some("loft_kernel_clients"),
        "n_kernel_post" => Some("loft_kernel_post"),
        "n_kernel_alive" => Some("loft_kernel_alive"),
        "n_kernel_stop" => Some("loft_kernel_stop"),
        "n_kernel_frame" => Some("loft_kernel_frame"),
        "n_kernel_udp_bound" => Some("loft_kernel_udp_bound"),
        "n_kernel_sync_class" => Some("loft_kernel_sync_class"),
        "n_kernel_sync_class_keyed" => Some("loft_kernel_sync_class_keyed"),
        "n_kernel_keyframe" => Some("loft_kernel_keyframe"),
        "n_kernel_sync_next" => Some("loft_kernel_sync_next"),
        "n_kernel_sync_cid" => Some("loft_kernel_sync_cid"),
        "n_kernel_sync_seq" => Some("loft_kernel_sync_seq"),
        "n_kernel_sync_payload" => Some("loft_kernel_sync_payload"),
        "n_kernel_connect" => Some("loft_kernel_connect"),
        "n_kernel_client_pump" => Some("loft_kernel_client_pump"),
        "n_kernel_client_alive" => Some("loft_kernel_client_alive"),
        "n_kernel_client_next_event" => Some("loft_kernel_client_next_event"),
        "n_kernel_client_event_kind" => Some("loft_kernel_client_event_kind"),
        "n_kernel_client_event_cid" => Some("loft_kernel_client_event_cid"),
        "n_kernel_client_event_payload" => Some("loft_kernel_client_event_payload"),
        "n_kernel_client_event_status" => Some("loft_kernel_client_event_status"),
        "n_kernel_client_tick_due" => Some("loft_kernel_client_tick_due"),
        "n_kernel_client_idle" => Some("loft_kernel_client_idle"),
        "n_kernel_client_send" => Some("loft_kernel_client_send"),
        "n_kernel_client_sync_next" => Some("loft_kernel_client_sync_next"),
        "n_kernel_client_sync_seq" => Some("loft_kernel_client_sync_seq"),
        "n_kernel_client_sync_payload" => Some("loft_kernel_client_sync_payload"),
        "n_kernel_client_udp_bound" => Some("loft_kernel_client_udp_bound"),
        "n_kernel_client_frame" => Some("loft_kernel_client_frame"),
        "n_kernel_client_stop" => Some("loft_kernel_client_stop"),
        "n_kernel_default_host" => Some("loft_kernel_default_host"),
        "n_kernel_local" => Some("loft_kernel_local"),
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

/// The library a `#native` symbol's package ships prebuilt for this host (`prebuilt/<triple>/`,
/// the loft-ffi C ABI): the file the interpreter loads for it too
/// (`native_build::prebuilt_native_lib`), with the package's directory.  None for a package
/// without one — never a build.
fn prebuilt_library(data: &Data, native: &str) -> Option<(String, String)> {
    let krate = data.native_symbol_crates.get(native)?;
    let (name, pkg_dir) = data
        .native_packages
        .iter()
        .find(|(c, _)| c.replace('-', "_") == *krate)?;
    let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .and_then(|m| m.native)
        .unwrap_or_else(|| name.replace('-', "_"));
    crate::native_build::prebuilt_native_lib(pkg_dir, &stem).map(|lib| (pkg_dir.clone(), lib))
}

/// A library native's arguments as the loft-ffi ABI takes them: the typed parameters, the
/// expression for each, the statements that compute a vector's data record first, and the
/// argument whose store is handed over.
struct FfiArgs {
    params: Vec<String>,
    args: Vec<String>,
    prep: String,
    store_arg: Option<usize>,
    /// Every heap argument in declaration order, with whether it is a vector.
    heap: Vec<(usize, bool)>,
}

fn ffi_args(cx: &Cx, d: u32) -> Result<FfiArgs, String> {
    let def = cx.data.def(d);
    let mut ffi_params: Vec<String> = Vec::new();
    let mut ffi_args: Vec<String> = Vec::new();
    let mut prep = String::new();
    let mut store_arg: Option<usize> = None;
    let mut heap = Vec::new();
    for (i, a) in def.attributes().iter().enumerate() {
        let tp = a.typedef.base();
        if tp.heap_dep().is_some() {
            store_arg.get_or_insert(i);
            heap.push((i, matches!(tp, Type::Vector(_, _))));
            ffi_params.push("loft_ffi_ref".into());
            if matches!(tp, Type::Vector(_, _)) {
                let _ = writeln!(
                    prep,
                    "    loft_ffi_ref r{i} = {{ a{i}.store_nr, a{i}.rec ? loft_collection_rec(loft_store_of(a{i}), a{i}.rec, a{i}.pos) : 0, 0 }};"
                );
            } else {
                let _ = writeln!(
                    prep,
                    "    loft_ffi_ref r{i} = {{ a{i}.store_nr, a{i}.rec, a{i}.pos }};"
                );
            }
            ffi_args.push(format!("r{i}"));
            continue;
        }
        match tp {
            Type::Text(_) => {
                ffi_params.push("const uint8_t *".into());
                ffi_params.push("size_t".into());
                ffi_args.push(format!("(const uint8_t *) a{i}.p"));
                ffi_args.push(format!("a{i}.len"));
            }
            Type::Integer(s) if s.forced_size.is_none() => {
                ffi_params.push("int64_t".into());
                ffi_args.push(format!("a{i}"));
            }
            Type::Integer(_) => {
                ffi_params.push("int32_t".into());
                ffi_args.push(format!("(int32_t) a{i}"));
            }
            Type::Float => {
                ffi_params.push("double".into());
                ffi_args.push(format!("a{i}"));
            }
            Type::Single => {
                ffi_params.push("float".into());
                ffi_args.push(format!("a{i}"));
            }
            Type::Boolean => {
                ffi_params.push("bool".into());
                ffi_args.push(format!("a{i} == 1"));
            }
            _ => {
                return Err(format!(
                    "native {} (a library parameter of type {})",
                    def.original_name(),
                    type_word(&a.typedef)
                ));
            }
        }
    }
    Ok(FfiArgs {
        params: ffi_params,
        args: ffi_args,
        prep,
        store_arg,
        heap,
    })
}

/// The loft-ffi C type a library native answers with (`loft-ffi/include/loft_ffi.h`), or the
/// refusal for an answer the ABI does not carry.
fn ffi_return_ctype(def: &crate::data::Definition) -> Result<&'static str, String> {
    Ok(match def.returned().base() {
        Type::Void => "void",
        Type::Integer(s) if s.forced_size.is_none() => "int64_t",
        Type::Integer(_) => "int32_t",
        Type::Float => "double",
        Type::Single => "float",
        Type::Boolean => "uint8_t",
        Type::Text(_) => "loft_ffi_str",
        // A heap answer comes back as a ref into the store handed over.
        Type::Vector(_, _) | Type::Reference(_, _) => "loft_ffi_ref",
        other => {
            return Err(format!(
                "native {} (a library answer of type {})",
                def.original_name(),
                type_word(other)
            ));
        }
    })
}

/// A `#native` function of a package whose native part is a prebuilt library (@PLN205): a body
/// that resolves the library's typed symbol on the first call and calls it with the loft-ffi
/// signature (`loft-ffi/include/loft_ffi.h`): a store first when an argument or the answer is
/// a heap value, a text as its bytes and length, a vector as its data record.
fn library_native(
    cx: &Cx,
    d: u32,
    ret: CType,
    pkg_dir: &str,
    lib: &str,
) -> Result<(String, String), String> {
    let def = cx.data.def(d);
    let params = declared_params(cx, d)?;
    let FfiArgs {
        params: mut ffi_params,
        args: mut ffi_args,
        prep,
        store_arg,
        heap,
    } = ffi_args(cx, d)?;
    let ffi_ret = ffi_return_ctype(def)?;
    let heap_answer = ffi_ret == "loft_ffi_ref";
    // The store handed over: the first heap argument's, else — for a heap answer — one minted
    // for the answer, as the interpreter's bridge mints it (`Stores::bridge_args`).
    let minted = store_arg.is_none() && heap_answer;
    let (mut select, mut copies) = (String::new(), String::new());
    let post = foreign_vector_args(&heap, heap_answer, &mut select, &mut copies);
    // loft#2227 — a heap answer pinned to an argument's store is copied out of it after the
    // call, and only what the call claimed there is deleted (`loft_ffi_own_*`): log it.  A store
    // minted for the call (`pminted`) is never owned, as `Stores::own_bridge_answer` skips it.
    let owned_answer = store_arg.is_some() && heap_answer && def.native_answer_tp != u16::MAX;
    if owned_answer {
        cx.copy_type(def.native_answer_tp)?;
    }
    let mut log = String::new();
    if let Some(i) = store_arg {
        ffi_params.insert(0, "loft_ffi_store".into());
        // The first heap argument's store, or `pin` when a vector argument may be foreign
        // (`foreign_vector_args` chooses it then).
        let store = if post.is_empty() {
            format!("a{i}.store_nr")
        } else {
            "pin".to_string()
        };
        ffi_args.insert(0, format!("loft_ffi_store_of({store})"));
        if owned_answer {
            call_store_copy(def, &store, post.is_empty(), &mut select);
            // After the copies: their records are `post`'s to delete, never the log's.
            let only = if post.is_empty() {
                ""
            } else {
                "if (!pminted) "
            };
            let _ = writeln!(log, "    {only}loft_ffi_log_begin({store});");
        }
    } else if minted {
        log.push_str("    uint16_t ns = (uint16_t) loft_store_new();\n");
        ffi_params.insert(0, "loft_ffi_store".into());
        ffi_args.insert(0, "loft_ffi_store_of(ns)".into());
    }
    cx.ffi_libs
        .borrow_mut()
        .insert((pkg_dir.to_string(), lib.to_string()));
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
    let mut body = String::new();
    let _ = writeln!(
        body,
        "    static {ffi_ret} (*fp)({}) = 0;",
        if ffi_params.is_empty() {
            "void".to_string()
        } else {
            ffi_params.join(", ")
        }
    );
    // The build's own path for a run in place; a shipped program finds the file by name among
    // the libraries it opened at start (`loft_c_bind_libraries`).  ISO C has no conversion from
    // an object pointer to a function pointer, so the symbol is copied into `fp` (strict C99
    // refuses the cast; every host's ABI makes the bytes the same).
    let _ = writeln!(
        body,
        "    if (!fp) {{ void *s_ = loft_ffi_sym({}, {}, {}); memcpy(&fp, &s_, sizeof fp); }}",
        body::c_string(lib),
        body::c_string(&crate::file_access::file_name(lib).unwrap_or_default()),
        body::c_string(def.native())
    );
    // The pin and its copy first, then the arguments read, the foreign copies, the log.
    body.push_str(&select);
    body.push_str(&prep);
    body.push_str(&copies);
    body.push_str(&log);
    let call = format!("fp({})", ffi_args.join(", "));
    let answer_store = if owned_answer {
        AnswerStore::Owned(def.native_answer_tp)
    } else if minted {
        AnswerStore::Minted
    } else {
        AnswerStore::Given
    };
    library_answer(&mut body, def, ret, ffi_ret, &call, &post, answer_store);
    Ok((sig, body))
}

/// loft#2239 — a store a running `par` shares is never grown: a library call that builds its
/// answer in `store` works in a copy of it (`loft_ffi_call_store`), and every argument that
/// lived there is renamed to the copy BEFORE the arguments are read and the foreign copies
/// claimed.  With foreign vector arguments (`!plain`) `store` is the `pin` chosen at run time,
/// and a `pin` minted for the call is its own already.  Writes `pn` and `cs` into `select`.
fn call_store_copy(def: &crate::data::Definition, store: &str, plain: bool, select: &mut String) {
    let (pn, cs) = if plain {
        (store.to_string(), "loft_ffi_call_store(pn)")
    } else {
        (
            "(uint16_t) pin".to_string(),
            "pminted ? pn : loft_ffi_call_store(pn)",
        )
    };
    let _ = writeln!(select, "    uint16_t pn = {pn};\n    uint16_t cs = {cs};");
    for (j, a) in def.attributes().iter().enumerate() {
        if a.typedef.base().heap_dep().is_some() {
            let _ = writeln!(select, "    if (a{j}.store_nr == pn) a{j}.store_nr = cs;");
        }
    }
    if !plain {
        let _ = writeln!(select, "    pin = cs;");
    }
}

/// Where a library native's heap answer is built, which decides what follows the call.
#[derive(Clone, Copy)]
enum AnswerStore {
    /// The store handed over is an argument's, and the answer is not copied out of it.
    Given,
    /// A store minted for the answer (`ns`); it goes when there is no answer.
    Minted,
    /// An argument's store, the answer copied out of it with this type (loft#2227).
    Owned(u16),
}

/// The statements after a library native's call (`call`): the answer read in its C shape,
/// `post` run (the copies a foreign argument took going), and the loft value returned — a text
/// owned, a heap answer behind its header (`codegen_runtime::from_loft_ref` /
/// `from_loft_record`), a narrow integer widened with its null kept.
fn library_answer(
    body: &mut String,
    def: &crate::data::Definition,
    ret: CType,
    ffi_ret: &str,
    call: &str,
    post: &str,
    answer_store: AnswerStore,
) {
    match def.returned().base() {
        Type::Void => {
            let _ = writeln!(body, "    {call};");
            body.push_str(post);
        }
        Type::Text(_) => {
            let _ = writeln!(body, "    loft_ffi_str t = {call};");
            body.push_str(post);
            let _ = writeln!(
                body,
                "    return loft_text_own(loft_lit(t.ptr ? (const char *) t.ptr : \"\", t.len));"
            );
        }
        // A vector answer behind its header, a record as it is: the conversion
        // `codegen_runtime::from_loft_ref` / `from_loft_record` make.
        tp if ffi_ret == "loft_ffi_ref" => {
            let _ = writeln!(body, "    loft_ffi_ref r = {call};");
            let convert = if matches!(tp.base(), Type::Vector(_, _)) {
                "loft_ffi_vector_answer"
            } else {
                "loft_ffi_record_answer"
            };
            if let AnswerStore::Owned(answer_tp) = answer_store {
                let own = if matches!(tp.base(), Type::Vector(_, _)) {
                    "loft_ffi_own_vector"
                } else {
                    "loft_ffi_own_record"
                };
                // Copied out BEFORE `post`, which deletes the foreign copies an identity answer
                // may name (`Stores::own_bridge_answer` before `bridge_args_done`).
                let minted_pin = if post.is_empty() {
                    String::new()
                } else {
                    format!("pminted ? {convert}(r) : ")
                };
                let _ = writeln!(
                    body,
                    "    loft_ref v = {minted_pin}{own}(loft_types, {answer_tp}, {convert}(r));"
                );
                body.push_str(post);
                // loft#2239 — the copy of a shared store the call worked in goes.
                let _ = writeln!(body, "    loft_ffi_drop_call_store(cs, pn);");
                let _ = writeln!(body, "    return v;");
                return;
            }
            body.push_str(post);
            if matches!(answer_store, AnswerStore::Minted) {
                // No answer: the store minted for it goes.
                let _ = writeln!(
                    body,
                    "    if (r.rec == 0 && r.pos == 0) {{ loft_ref m = {{ ns, 0, 0 }}; loft_free_ref(m); }}"
                );
            }
            let _ = writeln!(body, "    return {convert}(r);");
        }
        // `loft_ffi::widen_i32`: a narrow answer's null is `INT32_MIN`, which a plain widening
        // would turn into an ordinary negative number.
        Type::Integer(_) if ffi_ret == "int32_t" && ret == CType::Int => {
            let _ = writeln!(body, "    int32_t r = {call};");
            body.push_str(post);
            let _ = writeln!(body, "    return r == INT32_MIN ? LOFT_NULL : (int64_t) r;");
        }
        _ if post.is_empty() => {
            let _ = writeln!(body, "    return {call};");
        }
        _ => {
            let _ = writeln!(body, "    {ffi_ret} r = {call};");
            body.push_str(post);
            let _ = writeln!(body, "    return r;");
        }
    }
}

/// `Stores::bridge_args` for a library call: the library reads every vector argument through
/// the ONE store it is handed (`ptr + rec * 8`), so a foreign vector — a `file_map`, whose bytes
/// are in no store — is copied into a record of that store for the call, in one block, and never
/// pins it: the first other heap argument does, or a store is minted (`pin`).  Writes the choice
/// of `pin` into `select` (before the arguments are read, so a copy of a shared `pin` can
/// replace it), the copies into `prep`, and answers the statements after the call
/// (`bridge_args_done`): a copy the library changed is a write the program could not make,
/// refused as one; then the copies go, or the minted store whole when no heap answer lives in
/// it.  Empty — no `pin`, the plain call — when no argument is a vector.
fn foreign_vector_args(
    heap: &[(usize, bool)],
    heap_answer: bool,
    select: &mut String,
    prep: &mut String,
) -> String {
    let mut post = String::new();
    let vec_args: Vec<usize> = heap.iter().filter(|(_, v)| *v).map(|(i, _)| *i).collect();
    if vec_args.is_empty() {
        return post;
    }
    let _ = writeln!(select, "    uint32_t pin = UINT32_MAX; int pminted = 0;");
    for (i, is_vec) in heap {
        let guard = if *is_vec {
            format!(" && !loft_ffi_is_foreign(a{i})")
        } else {
            String::new()
        };
        let _ = writeln!(
            select,
            "    if (pin == UINT32_MAX{guard}) pin = a{i}.store_nr;"
        );
    }
    let _ = writeln!(
        select,
        "    if (pin == UINT32_MAX) {{ pin = loft_store_new(); pminted = 1; }}"
    );
    for i in &vec_args {
        let _ = writeln!(
            prep,
            "    uint32_t cp{i} = loft_ffi_is_foreign(a{i}) ? loft_ffi_copy_in(pin, a{i}) : 0;"
        );
        let _ = writeln!(
            prep,
            "    if (cp{i}) r{i} = (loft_ffi_ref) {{ (uint16_t) pin, cp{i}, 0 }};"
        );
    }
    let _ = writeln!(post, "    int changed = 0;");
    for i in &vec_args {
        let _ = writeln!(
            post,
            "    if (cp{i}) changed |= loft_ffi_copy_changed(a{i}, pin, cp{i});"
        );
    }
    if heap_answer {
        let _ = writeln!(post, "    if (!loft_stores[pin].read_only) {{");
    } else {
        let _ = writeln!(
            post,
            "    if (pminted) loft_free_ref((loft_ref) {{ (uint16_t) pin, 1, 0 }});"
        );
        let _ = writeln!(post, "    else if (!loft_stores[pin].read_only) {{");
    }
    for i in &vec_args {
        let _ = writeln!(
            post,
            "        if (cp{i}) loft_st_delete(&loft_stores[pin], cp{i});"
        );
    }
    let _ = writeln!(post, "    }}");
    let _ = writeln!(post, "    if (changed) loft_locked_write();");
    post
}

/// A library native in a browser page's module: the host import the page's script supplies
/// under the native's symbol (`host_imports`, the Rust writer's `loft_gl` block), declared once,
/// and a body that hands it a text's bytes and length, a vector's elements and count, and each
/// scalar in the import's width.
fn browser_import_native(
    cx: &Cx,
    d: u32,
    ret: CType,
    imp: &crate::host_imports::HostImport,
) -> Result<(String, String), String> {
    use crate::host_imports::Arg;
    let def = cx.data.def(d);
    let params = declared_params(cx, d)?;
    let import = format!("loft_host_{}", imp.symbol);
    let mut decl = Vec::new();
    let mut args = Vec::new();
    let mut body = String::new();
    let visible = def
        .attributes()
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.name.starts_with("__"));
    for ((i, _), (_, arg)) in visible.zip(&imp.args) {
        match arg {
            Arg::Text => {
                decl.push("const uint8_t *".to_string());
                decl.push("uint32_t".to_string());
                args.push(format!("(const uint8_t *) a{i}.p"));
                args.push(format!("(uint32_t) a{i}.len"));
            }
            Arg::Vector(elem) => {
                let e = elem.c();
                decl.push(format!("const {e} *"));
                decl.push("uint32_t".to_string());
                let _ = writeln!(
                    body,
                    "    uint32_t r{i} = a{i}.rec ? loft_collection_rec(loft_store_of(a{i}), a{i}.rec, a{i}.pos) : 0;"
                );
                args.push(format!(
                    "r{i} ? (const {e} *) loft_elem_base(loft_store_of(a{i}), r{i}) : 0"
                ));
                args.push(format!("(uint32_t) loft_vec_len(a{i})"));
            }
            Arg::Scalar(sc) => {
                decl.push(sc.c().to_string());
                args.push(if *sc == crate::host_imports::Scalar::U8 {
                    format!("(uint8_t) (a{i} == 1)")
                } else {
                    format!("({}) a{i}", sc.c())
                });
            }
        }
    }
    let ret_c = imp.ret.map_or("void", crate::host_imports::Scalar::c);
    let _ = writeln!(
        cx.gen_decls.borrow_mut(),
        "__attribute__((import_module(\"loft_gl\"), import_name(\"{}\"))) {ret_c} {import}({});",
        imp.symbol,
        if decl.is_empty() {
            "void".to_string()
        } else {
            decl.join(", ")
        }
    );
    let call = format!("{import}({})", args.join(", "));
    match imp.ret {
        None => {
            let _ = writeln!(body, "    {call};");
        }
        Some(crate::host_imports::Scalar::U8) => {
            let _ = writeln!(body, "    return (uint8_t) ({call} != 0);");
        }
        Some(_) => {
            let _ = writeln!(body, "    return ({}) {call};", held(ret));
        }
    }
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
    Ok((sig, body))
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
    // A package whose native part needs no build — a component, a library shipped prebuilt per
    // host (@PLN205) — is installed, never rebuilt: say what is missing.
    let unbuildable = cx
        .data
        .native_symbol_crates
        .get(def.native())
        .and_then(|krate| {
            cx.data
                .native_packages
                .iter()
                .find(|(c, _)| c.replace('-', "_") == *krate)
        })
        .and_then(|(name, pkg_dir)| crate::native_build::unbuildable_native_reason(pkg_dir, name));
    // Any other native is a Rust crate's, which a C build never links (`native_build::c_target`):
    // the interpreter's "cdylib missing or stale / a poisoned call index" reading does not apply
    // to a call the C makes by name, so the message says what does.
    let library = cx.data.native_symbol_crates.get(def.native()).map_or_else(
        || "its library".to_string(),
        |krate| {
            cx.data
                .native_packages
                .iter()
                .find(|(c, _)| c.replace('-', "_") == *krate)
                .map_or_else(|| krate.clone(), |(_, pkg_dir)| package_identity(pkg_dir).0)
        },
    );
    let msg = if let Some(why) = unbuildable {
        format!("native function {}: {why}", def.native())
    } else {
        format!(
            "native function {} is not in this build: its library ({library}) has only a Rust \
             native part, which the C backend does not link.  Run it with --native-rust or \
             --interpret, or give the library a C native part (a `[c] shim`, a prebuilt C \
             library, or a component).",
            def.native()
        )
    };
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
fn framed(
    cx: &Cx,
    d: u32,
    name: &str,
    params: &str,
    ret: CType,
    body: &str,
    inline: bool,
) -> String {
    let def = cx.data.def(d);
    let storage = if inline { "static inline" } else { "static" };
    let _ = writeln!(
        cx.gen_decls.borrow_mut(),
        "{storage} {} {name}__fr({params});",
        held(ret)
    );
    let _ = write!(
        cx.gen_bodies.borrow_mut(),
        "\n{storage} {} {name}__fr({params}) {{\n{body}}}\n",
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
        "    loft_frame_push({}, {}, {});\n",
        body::c_string(&frame_name),
        body::c_string(&def.position().file),
        def.position().line
    );
    // The body names the texts it holds (`LOFT_HOLD`); the frame drops them on its return.
    let push = format!("{push}    LOFT_HOLD_MARK(hm);\n");
    // What calls through function values handed back and no bind adopted is the frame's, freed
    // as it returns — unless it answers a heap value, which may be one of them: those go up to
    // the caller (`FnRefBufGuard`'s `hands_up`).
    let hands_up = matches!(
        def.returned().base(),
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
    );
    let (push, release) = if hands_up {
        (push, String::new())
    } else {
        (
            format!("{push}    size_t fm = loft_nfnrefs;\n"),
            "    if (loft_nfnrefs != fm) loft_fnref_release(fm);\n".to_string(),
        )
    };
    if ret == CType::Void {
        format!("{push}    {call};\n{release}    LOFT_HOLD_BACK(hm);\n    loft_frame_pop();\n")
    } else {
        format!(
            "{push}    {} r = {call};\n{release}    LOFT_HOLD_BACK(hm);\n    loft_frame_pop();\n    return r;\n",
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
