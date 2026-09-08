// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I63 — Store-resident IR (reader / handle / materializer)

//! @PLN11 arc C — store → native IR reader (the read path).
//!
//! The inverse of [`crate::ir_store`]: walks store `Node` / `TypeT` records
//! through the [`crate::data_store`] handle layer and rebuilds the native
//! [`crate::data::Value`] / [`crate::data::Type`] graph.  Together the two
//! halves give a full round-trip — `native → store → native` — so the
//! materialised store can be validated against the original with the IR's own
//! derived `PartialEq`, the strongest oracle available (stronger than the
//! @PLAN28 JSON re-encode comparison, and needing no JSON at all).
//!
//! Scope of this slice: the two recursive enums (`Value`, 32 variants;
//! `Type`, 24 variants) and every sub-struct reachable from them — `Block`,
//! `Position`, `Key`, `IntegerSpec`, plus `SortKey`/`NameRef`
//! key lists and `vector<integer>` dep lists.  The `Definition`-level
//! components (`Attribute` / `Function` / `Variable` / `Definition` / `Data`)
//! and the whole-`Data` reader are the next increment.
//!
//! A one-element `vector<Node>` / `vector<TypeT>` field reads back as a single
//! `Box<Value>` / `Box<Type>` (the box-of-one the writer used for single
//! recursive children); an N-element vector reads back as a `Vec`.  `Block.name`
//! is a `&'static str` in the native IR — reconstructed via a bounded
//! [`Box::leak`], exactly as the @PLAN28 JSON decoder does (a loaded image holds
//! a small fixed set of block names that live for the whole process anyway).

use crate::data::{
    Attribute, Block, Data, DefType, Definition, ImpureCategory, IntegerSpec, LinkedFieldGroup,
    LinkedFieldKind, Purity, Type, Value,
};
use crate::data_store::{
    self as ds, Record, TypeKind, Value as Node, ValueType, ValuesVector, type_kind,
};
use crate::database::{Field as SchemaField, Parts, Stores, Type as SchemaType};
use crate::keys::{Content, DbRef, Key, Str};
use crate::lexer::Position;
use crate::variables::{Function, RestoredVar};
use std::num::NonZeroU8;

/// Rebuild a native [`Value`] from one store `Node` record.
///
/// # Panics
/// If `slot`'s discriminant byte is not a known `Node` variant — only possible
/// on a record not written by [`crate::ir_store`] (a corrupt or foreign store).
#[must_use]
pub fn read_value(stores: &Stores, slot: Node) -> Value {
    match slot.value_type(stores) {
        // ── leaves ───────────────────────────────────────────────────────────
        ValueType::Null => Value::Null,
        ValueType::Line => Value::Line(slot.field_int(stores, ds::NDLINE_N) as u32),
        ValueType::Int => Value::Int(slot.int_value(stores) as i32),
        ValueType::Enum => Value::Enum(
            slot.field_int(stores, ds::NDENUM_ORD) as u8,
            slot.field_int(stores, ds::NDENUM_TP) as u16,
        ),
        ValueType::Boolean => Value::Boolean(slot.field_bool(stores, ds::NDBOOLEAN_B)),
        ValueType::Float => Value::Float(slot.field_float(stores, ds::NDFLOAT_F)),
        ValueType::Long => Value::Long(slot.field_int(stores, ds::NDLONG_N)),
        ValueType::Single => Value::Single(slot.field_single(stores, ds::NDSINGLE_F)),
        ValueType::Text => Value::Text(slot.field_str(stores, ds::NDTEXT_S).to_string()),
        ValueType::Var => Value::Var(slot.field_int(stores, ds::NDVAR_N) as u16),
        ValueType::Break => Value::Break(slot.field_int(stores, ds::NDBREAK_N) as u16),
        ValueType::Continue => Value::Continue(slot.field_int(stores, ds::NDCONTINUE_N) as u16),
        ValueType::FnRefDnr => Value::FnRefDnr(slot.field_int(stores, ds::NDFNREFDNR_N) as u16),
        ValueType::TupleGet => Value::TupleGet(
            slot.field_int(stores, ds::NDTUPLEGET_VAR) as u16,
            slot.field_int(stores, ds::NDTUPLEGET_IDX) as u16,
        ),
        ValueType::RawExpr => Value::RawExpr(slot.field_str(stores, ds::NDRAWEXPR_S).to_string()),
        // ── scalar(s) + vector<Node> children ────────────────────────────────
        ValueType::Call => Value::Call(
            slot.call_to(stores),
            read_node_list(stores, slot.call_parameters()),
        ),
        ValueType::CallRef => Value::CallRef(
            slot.field_int(stores, ds::NDCALLREF_VAR) as u16,
            read_node_list(stores, slot.field_vec(ds::NDCALLREF_ARGS)),
        ),
        ValueType::Insert => {
            Value::Insert(read_node_list(stores, slot.field_vec(ds::NDINSERT_ITEMS)))
        }
        ValueType::Set => Value::Set(
            slot.field_int(stores, ds::NDSET_VAR) as u16,
            Box::new(read_node_child(stores, slot.field_vec(ds::NDSET_INNER))),
        ),
        ValueType::Return => Value::Return(Box::new(read_node_child(
            stores,
            slot.field_vec(ds::NDRETURN_INNER),
        ))),
        ValueType::If => Value::If(
            Box::new(read_node_child(stores, slot.field_vec(ds::NDIF_COND))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDIF_T))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDIF_F))),
        ),
        ValueType::Drop => Value::Drop(Box::new(read_node_child(
            stores,
            slot.field_vec(ds::NDDROP_INNER),
        ))),
        ValueType::Iter => Value::Iter(
            slot.field_int(stores, ds::NDITER_VAR) as u16,
            Box::new(read_node_child(stores, slot.field_vec(ds::NDITER_CREATE))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDITER_NEXT))),
            Box::new(read_node_child(stores, slot.field_vec(ds::NDITER_INIT))),
        ),
        ValueType::Tuple => Value::Tuple(read_node_list(stores, slot.field_vec(ds::NDTUPLE_ITEMS))),
        ValueType::TuplePut => Value::TuplePut(
            slot.field_int(stores, ds::NDTUPLEPUT_VAR) as u16,
            slot.field_int(stores, ds::NDTUPLEPUT_IDX) as u16,
            Box::new(read_node_child(
                stores,
                slot.field_vec(ds::NDTUPLEPUT_INNER),
            )),
        ),
        ValueType::Yield => Value::Yield(Box::new(read_node_child(
            stores,
            slot.field_vec(ds::NDYIELD_INNER),
        ))),
        ValueType::Parallel => {
            Value::Parallel(read_node_list(stores, slot.field_vec(ds::NDPARALLEL_ARMS)))
        }
        // ── inlined Block ────────────────────────────────────────────────────
        ValueType::Block => Value::Block(Box::new(read_block(stores, slot))),
        ValueType::Loop => Value::Loop(Box::new(read_block(stores, slot))),
        // ── inlined sub-structs ───────────────────────────────────────────────
        ValueType::Span => {
            let position = Position {
                file: slot.field_str(stores, ds::SPAN_POS_FILE).to_string(),
                line: slot.field_int(stores, ds::SPAN_POS_LINE) as u32,
                pos: slot.field_int(stores, ds::SPAN_POS_POS) as u32,
            };
            let inner = read_node_child(stores, slot.field_vec(ds::NDSPAN_INNER));
            Value::Span(Box::new((position, inner)))
        }
        // ── vector of a non-Node struct ───────────────────────────────────────
        ValueType::Keys => Value::Keys(read_keys(stores, slot)),
        // ── carries a vector<TypeT> ────────────────────────────────────────────
        ValueType::FnRef => Value::FnRef(
            slot.field_int(stores, ds::NDFNREF_DEF_NR) as i32,
            slot.field_int(stores, ds::NDFNREF_VAR) as u16,
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::NDFNREF_T, ds::TYPET_STRIDE),
            )),
        ),
        ValueType::Other(d) => panic!("ir_read: unknown Node discriminant {d}"),
    }
}

/// Rebuild a native [`Type`] from one store `TypeT` record.
///
/// # Panics
/// If `slot`'s discriminant byte is not a known `TypeT` variant — only possible
/// on a record not written by [`crate::ir_store`] (a corrupt or foreign store).
#[must_use]
pub fn read_type(stores: &Stores, slot: Record) -> Type {
    match type_kind(slot.discriminant(stores)) {
        TypeKind::Unknown => Type::Unknown(slot.field_int(stores, ds::TYUNKNOWN_N) as u32),
        TypeKind::Null => Type::Null,
        TypeKind::Void => Type::Void,
        TypeKind::Never => Type::Never,
        TypeKind::Integer => Type::Integer(read_int_spec(stores, slot)),
        TypeKind::Boolean => Type::Boolean,
        TypeKind::Float => Type::Float,
        TypeKind::Single => Type::Single,
        TypeKind::Character => Type::Character,
        TypeKind::Text => Type::Text(read_deps(stores, slot, ds::TYTEXT_DEP)),
        TypeKind::Keys => Type::Keys,
        TypeKind::Enum => Type::Enum(
            slot.field_int(stores, ds::TYENUM_N) as u32,
            slot.field_bool(stores, ds::TYENUM_IS_REF),
            read_deps(stores, slot, ds::TYENUM_DEP),
        ),
        TypeKind::Reference => Type::Reference(
            slot.field_int(stores, ds::TYREF_N) as u32,
            read_deps(stores, slot, ds::TYREF_DEP),
        ),
        TypeKind::RefVar => Type::RefVar(Box::new(read_type_child(
            stores,
            slot.field_recvec(ds::TYREFVAR_INNER, ds::TYPET_STRIDE),
        ))),
        TypeKind::Vector => Type::Vector(
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYVECTOR_INNER, ds::TYPET_STRIDE),
            )),
            read_deps(stores, slot, ds::TYVECTOR_DEP),
        ),
        TypeKind::Routine => Type::Routine(slot.field_int(stores, ds::TYROUTINE_N) as u32),
        TypeKind::Iterator => Type::Iterator(
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYITER_STEP, ds::TYPET_STRIDE),
            )),
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYITER_INNER, ds::TYPET_STRIDE),
            )),
        ),
        TypeKind::Sorted => Type::Sorted(
            slot.field_int(stores, ds::TYSORTED_N) as u32,
            read_sort_keys(
                stores,
                slot.field_recvec(ds::TYSORTED_KEYS, ds::SORTKEY_STRIDE),
            ),
            read_deps(stores, slot, ds::TYSORTED_DEP),
        ),
        TypeKind::Index => Type::Index(
            slot.field_int(stores, ds::TYINDEX_N) as u32,
            read_sort_keys(
                stores,
                slot.field_recvec(ds::TYINDEX_KEYS, ds::SORTKEY_STRIDE),
            ),
            read_deps(stores, slot, ds::TYINDEX_DEP),
        ),
        TypeKind::Radix => Type::Radix(
            slot.field_int(stores, ds::TYRADIX_N) as u32,
            read_name_list(
                stores,
                slot.field_recvec(ds::TYRADIX_NAMES, ds::NAMEREF_STRIDE),
            ),
            read_deps(stores, slot, ds::TYRADIX_DEP),
        ),
        TypeKind::Trie => Type::Trie(
            slot.field_int(stores, ds::TYTRIE_N) as u32,
            slot.field_str(stores, ds::TYTRIE_KEY).to_string(),
            read_deps(stores, slot, ds::TYTRIE_DEP),
        ),
        TypeKind::Hash => Type::Hash(
            slot.field_int(stores, ds::TYHASH_N) as u32,
            read_name_list(
                stores,
                slot.field_recvec(ds::TYHASH_NAMES, ds::NAMEREF_STRIDE),
            ),
            read_deps(stores, slot, ds::TYHASH_DEP),
        ),
        TypeKind::Function => Type::Function(
            read_type_list(stores, slot.field_recvec(ds::TYFUNC_ARGS, ds::TYPET_STRIDE)),
            Box::new(read_type_child(
                stores,
                slot.field_recvec(ds::TYFUNC_RESULT, ds::TYPET_STRIDE),
            )),
            read_deps(stores, slot, ds::TYFUNC_DEP),
        ),
        TypeKind::Rewritten => Type::Rewritten(Box::new(read_type_child(
            stores,
            slot.field_recvec(ds::TYREWRITTEN_INNER, ds::TYPET_STRIDE),
        ))),
        TypeKind::Tuple => Type::Tuple(read_type_list(
            stores,
            slot.field_recvec(ds::TYTUPLE_ELEMS, ds::TYPET_STRIDE),
        )),
        // @PLN25 — the `τ?` marker (a box-of-one child, like RefVar). `Type::optional`
        // is the idempotent former (N-Idem); a real in-memory `Optional` never wraps
        // Never/Null, so this reconstructs the written type exactly.
        TypeKind::Optional => Type::optional(read_type_child(
            stores,
            slot.field_recvec(ds::TYOPTIONAL_INNER, ds::TYPET_STRIDE),
        )),
        TypeKind::Other(d) => panic!("ir_read: unknown TypeT discriminant {d}"),
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Read the single element of a box-of-one `vector<Node>` as a `Value`.
fn read_node_child(stores: &Stores, v: ValuesVector) -> Value {
    read_value(stores, v.get(0, stores))
}

/// Read every element of a `vector<Node>` as a `Vec<Value>`.
fn read_node_list(stores: &Stores, v: ValuesVector) -> Vec<Value> {
    (0..v.len(stores))
        .map(|i| read_value(stores, v.get(i, stores)))
        .collect()
}

/// Read the single element of a box-of-one `vector<TypeT>` as a `Type`.
fn read_type_child(stores: &Stores, v: ds::RecVector) -> Type {
    read_type(stores, v.get(0, stores))
}

/// Read every element of a `vector<TypeT>` as a `Vec<Type>`.
fn read_type_list(stores: &Stores, v: ds::RecVector) -> Vec<Type> {
    (0..v.len(stores))
        .map(|i| read_type(stores, v.get(i, stores)))
        .collect()
}

/// Read a `vector<integer>` dep list (field `off` of `slot`) as a `Vec<u16>`.
/// Type-dep wrapper: the same u16-list read, tagged space-`Unknown`
/// (the snapshot does not record the address space — H2 DEPS_INVENTORY).
fn read_deps(stores: &Stores, slot: Record, off: u32) -> crate::data::Deps {
    crate::data::Deps::unknown(read_dep_list(stores, slot, off))
}

fn read_dep_list(stores: &Stores, slot: Record, off: u32) -> Vec<u16> {
    let v = slot.field_recvec(off, ds::INT_STRIDE);
    (0..v.len(stores))
        .map(|i| v.get(i, stores).field_int(stores, 0) as u16)
        .collect()
}

/// Read a `vector<SortKey>` as a `Vec<(String, bool)>`.
fn read_sort_keys(stores: &Stores, v: ds::RecVector) -> Vec<(String, bool)> {
    (0..v.len(stores))
        .map(|i| {
            let e = v.get(i, stores);
            (
                e.field_str(stores, ds::SORTKEY_NAME).to_string(),
                e.field_bool(stores, ds::SORTKEY_ASC),
            )
        })
        .collect()
}

/// Read a `vector<NameRef>` as a `Vec<String>`.
fn read_name_list(stores: &Stores, v: ds::RecVector) -> Vec<String> {
    (0..v.len(stores))
        .map(|i| {
            v.get(i, stores)
                .field_str(stores, ds::NAMEREF_NAME)
                .to_string()
        })
        .collect()
}

/// Read an inlined `IntegerSpec` from a `TyInteger` record.  `forced_size`
/// sentinel `0` decodes to `None` (mirrors the writer).
fn read_int_spec(stores: &Stores, slot: Record) -> IntegerSpec {
    IntegerSpec {
        min: slot.field_int(stores, ds::TYINTEGER_MIN) as i32,
        max: slot.field_int(stores, ds::TYINTEGER_MAX) as u32,
        not_null: slot.field_bool(stores, ds::TYINTEGER_NOT_NULL),
        forced_size: NonZeroU8::new(slot.field_int(stores, ds::TYINTEGER_FORCED) as u8),
    }
}

/// Read the referenced `Block` of an `NdBlock` / `NdLoop` record.  `Block.name`
/// is `&'static str` — reconstructed via a bounded leak (see module note).
fn read_block(stores: &Stores, slot: Node) -> Block {
    let blk = slot.block_rec(stores);
    let name: &'static str = Box::leak(
        blk.field_str(stores, ds::BLOCK_NAME)
            .to_owned()
            .into_boxed_str(),
    );
    Block {
        name,
        operators: read_node_list(stores, blk.field_vec(ds::BLOCK_OPERATORS)),
        result: read_type_child(stores, blk.field_recvec(ds::BLOCK_RESULT, ds::TYPET_STRIDE)),
        scope: blk.field_int(stores, ds::BLOCK_SCOPE) as u16,
        var_size: blk.field_int(stores, ds::BLOCK_VAR_SIZE) as u16,
    }
}

/// Read an `NdKeys` record's `vector<Key>` as a `Vec<Key>`.
fn read_keys(stores: &Stores, slot: Node) -> Vec<Key> {
    let kv = slot.field_recvec(ds::NDKEYS_KEYS, ds::KEY_STRIDE);
    (0..kv.len(stores))
        .map(|i| {
            let e = kv.get(i, stores);
            Key {
                type_nr: e.field_int(stores, ds::KEY_TYPE_NR) as i8,
                position: e.field_int(stores, ds::KEY_POSITION) as u16,
                start: e.field_int(stores, ds::KEY_START) as i32,
            }
        })
        .collect()
}

// ─── Definition-level reader (Attribute / Function / Definition / Data) ───────

/// Rebuild a whole native [`Data`] from the store record `root` (the `DbRef`
/// returned by [`crate::ir_store::materialize_data`]).  Derived indices are
/// re-derived via [`Data::rebuild_indices`], mirroring the @PLAN28 JSON loader.
#[must_use]
pub fn read_data(stores: &Stores, root: DbRef) -> Data {
    read_data_with(stores, root, true)
}

/// @PLN11 G2/M6 — like [`read_data`] but leaves each function body in the store
/// (`Definition.code = Null`), for warm-cache store-backed codegen which reads
/// bodies via [`def_body_node`] instead of reconstructing them.  Skipping the
/// body `Value` trees (the bulk of the IR) is the warm-load performance win.
#[must_use]
pub fn read_data_skeleton(stores: &Stores, root: DbRef) -> Data {
    read_data_with(stores, root, false)
}

fn read_data_with(stores: &Stores, root: DbRef, bodies: bool) -> Data {
    let r = Record::new(root);
    let mut data = Data::new();
    data.source = r.field_int(stores, ds::DATA_SOURCE) as u16;
    let defs = r.field_recvec(ds::DATA_DEFINITIONS, ds::DEFINITION_STRIDE);
    for i in 0..defs.len(stores) {
        data.definitions
            .push(read_definition(stores, defs.get(i, stores), bodies));
    }
    // The import tables go in BEFORE the rebuild: `rebuild_indices` replays
    // them to recreate the cross-source `def_names` bindings (loft#1359).
    let imports = r.field_recvec(ds::DATA_IMPORTS, ds::IMPORT_STRIDE);
    let mut applied = Vec::with_capacity(imports.len(stores) as usize);
    for i in 0..imports.len(stores) {
        let ir = imports.get(i, stores);
        let name = ir.field_str(stores, ds::IMPORT_NAME).to_string();
        applied.push(crate::data::AppliedImport {
            lib_source: ir.field_int(stores, ds::IMPORT_LIB_SOURCE) as u16,
            into_source: ir.field_int(stores, ds::IMPORT_INTO_SOURCE) as u16,
            name: if name.is_empty() {
                None
            } else {
                Some((name, ir.field_str(stores, ds::IMPORT_BIND).to_string()))
            },
        });
    }
    data.set_applied_imports(applied);
    let uses = r.field_recvec(ds::DATA_USE_NAMES, ds::USENAME_STRIDE);
    data.set_use_names((0..uses.len(stores)).map(|i| {
        let ur = uses.get(i, stores);
        (
            ur.field_str(stores, ds::USENAME_NAME).to_string(),
            ur.field_int(stores, ds::USENAME_SOURCE) as u16,
        )
    }));
    data.rebuild_indices();
    data
}

/// @PLN11 G2 / M2 — navigate a **persistent program store** (a whole `Data`
/// materialised once via [`crate::ir_store::materialize_data`]) to the body node
/// of definition `d_nr`: root → `definitions` vector → `def[d_nr]` → `code`
/// box-of-one.  Lets store-backed codegen read each function body directly from
/// the persistent store instead of re-materialising it per function (the M5
/// proof's harness) — the foundation for dropping the native graph (M6).
#[must_use]
pub fn def_body_node(stores: &Stores, root: DbRef, d_nr: u32) -> Node {
    let defs = Record::new(root).field_recvec(ds::DATA_DEFINITIONS, ds::DEFINITION_STRIDE);
    let def_rec = defs.get(d_nr, stores);
    def_rec.field_vec(ds::DEF_CODE).get(0, stores)
}

/// @PLN11 G2 / M0 — the read-site migration's equivalence harness.
///
/// Materialise `data` into a fresh in-memory store, read it straight back, and
/// assert the round-trip is bit-for-bit identical with the IR's own
/// `compare_data` oracle.  This is the bedrock guarantee the whole G2 migration
/// stands on: *before* any subsystem (codegen, the interpreter, …) is rewired
/// to read its IR from the store instead of the native graph, this proves the
/// store representation is a faithful mirror of the native `Data`.  Wired into
/// the run path behind `LOFT_IR_CHECK`, it validates that invariant on **any
/// real program** — user code, lazily-loaded libs and all — not just the
/// stdlib-only round-trip tests.
///
/// Needs no schema registration: like [`open_data`], it walks the store through
/// baked offsets.
///
/// # Errors
/// Returns the [`crate::ir_schema::DataDiff`] locating the first field where the
/// store-read `Data` diverges from `data`.
pub fn ir_roundtrip_check(data: &Data) -> Result<(), crate::ir_schema::DataDiff> {
    let mut stores = Stores::new();
    let root = crate::ir_store::materialize_data(&mut stores, data);
    let loaded = read_data(&stores, root);
    crate::ir_schema::compare_data(data, &loaded)
}

/// Open an IR store/bundle file as a READ surface and adopt it into
/// `stores`.  A reopened file store has live headers on disk but an EMPTY
/// in-memory `claims` set (claims are allocation-time artifacts), so the
/// armed `Store::valid` record check would reject every read —
/// `read_only` is its designed exemption for record-walking without
/// claims (mirrors worker stores), and also guards the cached artifact
/// against accidental mutation.  The ONE home for this discipline; all
/// three loaders (`open_data`, `open_bundle`, `open_bundle_into`) route
/// through it.
#[cfg(feature = "mmap")]
fn adopt_read_surface(path: &str, stores: &mut Stores) -> u16 {
    let mut fstore = crate::store::Store::open(path);
    fstore.read_only = true;
    stores.adopt_store(fstore)
}

/// Load a native [`Data`] from a file-backed IR store written by
/// [`crate::ir_store::save_data`] (@PLN11 arc D): mmap the file
/// (`Store::open`) and rebuild the native `Data` from the well-known root
/// record ([`ds::IR_ROOT_REC`]) — no re-parse, no schema registration.
///
/// # Errors
/// Returns `NotFound` if `path` does not exist (a cache miss — the caller
/// falls back to parsing).  `Store::open` panics on an unreadable/corrupt
/// file; integrity-checked loading is arc E's drift-detection job (Q4).
#[cfg(feature = "mmap")]
pub fn open_data(path: &str) -> std::io::Result<Data> {
    if !std::path::Path::new(path).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("IR store not found: {path}"),
        ));
    }
    let mut stores = Stores::new();
    let nr = adopt_read_surface(path, &mut stores);
    let root = DbRef {
        store_nr: nr,
        rec: ds::IR_ROOT_REC,
        pos: ds::IR_ROOT_POS,
    };
    Ok(read_data(&stores, root))
}

/// Load a whole **bundle** written by [`crate::ir_store::save_bundle`] (@PLN11
/// D2a step 4): mmap the file and rebuild both the native [`Data`] and its
/// database type `schema` (`Vec<database::Type>`) — so a warm startup gets a
/// `Data` *and* a valid type schema with no re-parse.  Install the schema into
/// a `Stores` via [`crate::database::Stores::install_schema`].
///
/// # Errors
/// Returns `NotFound` if `path` does not exist (a cache miss).
#[cfg(feature = "mmap")]
pub fn open_bundle(path: &str) -> std::io::Result<(Data, Vec<SchemaType>)> {
    // @PLN11 arc E — reject a missing / non-store / truncated / corrupt bundle
    // up front so a bad file is a clean cache miss (cold parse), not a panic
    // inside `Store::open`'s signature assertion.
    if !crate::store::Store::is_store_file(path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("IR bundle missing or not a valid store: {path}"),
        ));
    }
    let mut stores = Stores::new();
    let nr = adopt_read_surface(path, &mut stores);
    let root = DbRef {
        store_nr: nr,
        rec: ds::IR_ROOT_REC,
        pos: ds::IR_ROOT_POS,
    };
    let data_root = DbRef {
        pos: root.pos + ds::BUNDLE_DATA,
        ..root
    };
    let data = read_data(&stores, data_root);
    let schema = read_schema(&stores, Record::new(root), ds::BUNDLE_TYPES);
    Ok((data, schema))
}

/// @PLN11 G2/M6 — warm-cache store-backed load.  Mmap the bundle into a
/// standalone [`Stores`], reconstruct only the def **table**
/// ([`read_data_skeleton`] — bodies stay in the store), and return the store +
/// the `definitions`-vector root so codegen reads each body via
/// [`def_body_node`].  Skips reconstructing the body `Value` trees (the bulk of
/// the IR) — the warm-load win.  The returned `Stores` must outlive codegen
/// (it backs the bodies).
///
/// # Errors
/// `NotFound` on a missing / non-store / corrupt bundle (a clean cache miss).
#[cfg(feature = "mmap")]
pub fn open_program_store(path: &str) -> std::io::Result<(Stores, DbRef, Data, Vec<SchemaType>)> {
    if !crate::store::Store::is_store_file(path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("IR bundle missing or not a valid store: {path}"),
        ));
    }
    let mut stores = Stores::new();
    let nr = adopt_read_surface(path, &mut stores);
    let root = DbRef {
        store_nr: nr,
        rec: ds::IR_ROOT_REC,
        pos: ds::IR_ROOT_POS,
    };
    let data_root = DbRef {
        pos: root.pos + ds::BUNDLE_DATA,
        ..root
    };
    let data = read_data_skeleton(&stores, data_root);
    let schema = read_schema(&stores, Record::new(root), ds::BUNDLE_TYPES);
    Ok((stores, data_root, data, schema))
}

/// Load a bundle (see [`open_bundle`]) and install its database type schema
/// into `database`, returning the native `Data`.  The `pub` entry point the
/// startup cache (`main.rs`, @PLN11 D2b) uses, since `Stores::install_schema`
/// is `pub(crate)` and the binary crate cannot call it directly.
///
/// # Errors
/// Returns `NotFound` if `path` does not exist (a cache miss).
#[cfg(feature = "mmap")]
pub fn open_bundle_into(path: &str, database: &mut Stores) -> std::io::Result<Data> {
    let (data, types) = open_bundle(path)?;
    database.install_schema(types);
    Ok(data)
}

/// Rebuild one native [`Definition`] from a store `Definition` record.
///
/// `attr_names` is re-derived from the restored attribute list (a derived
/// index, never stored); `code_position` / `code_length` / `const_ref` are
/// codegen-derived and reset (recomputed by the compile pass on load), exactly
/// as the @PLAN28 JSON decoder does.
///
/// # Panics
/// If a stored `def_type` / `purity` code is out of range — only possible on a
/// record not written by [`crate::ir_store`] (a corrupt or foreign store).
#[must_use]
pub fn read_definition(stores: &Stores, r: Record, bodies: bool) -> Definition {
    let name = r.field_str(stores, ds::DEF_NAME).to_string();
    let position = Position {
        file: r
            .field_str(stores, ds::DEF_POSITION + ds::POS_FILE)
            .to_string(),
        line: r.field_int(stores, ds::DEF_POSITION + ds::POS_LINE) as u32,
        pos: r.field_int(stores, ds::DEF_POSITION + ds::POS_POS) as u32,
    };
    let attributes = read_attributes(stores, r);
    let attr_names = attributes
        .iter()
        .enumerate()
        .map(|(i, a)| (a.name.clone(), i))
        .collect();
    let forced = r.field_int(stores, ds::DEF_FORCED_SIZE);
    let synthetic_s = r.field_str(stores, ds::DEF_SYNTHETIC);
    Definition {
        bound_holder: false,
        variables: read_function(stores, r, ds::DEF_VARIABLES),
        attributes,
        attr_names,
        // codegen-derived: recomputed by the compile pass on load.
        code_position: 0,
        code_length: 0,
        const_ref: None,
        source: r.field_int(stores, ds::DEF_SOURCE) as u16,
        def_type: def_type_from_code(r.field_int(stores, ds::DEF_DEF_TYPE)),
        parent: r.field_int(stores, ds::DEF_PARENT) as u32,
        // @PLN11 G2/M6 — `bodies=false` leaves the body in the store
        // (`Value::Null` marker); warm-cache store-backed codegen reads it via
        // `def_body_node` instead of reconstructing it here.
        code: if bodies {
            read_node_child(stores, r.field_vec(ds::DEF_CODE))
        } else {
            Value::Null
        },
        returned: read_type_child(stores, r.field_recvec(ds::DEF_RETURNED, ds::TYPET_STRIDE)),
        returned_not_null: r.field_bool(stores, ds::DEF_RETURNED_NOT_NULL),
        rust: r.field_str(stores, ds::DEF_RUST).to_string(),
        native: r.field_str(stores, ds::DEF_NATIVE).to_string(),
        cap: r.field_str(stores, ds::DEF_CAP).to_string(), // @PLN86
        c_symbol: r.field_str(stores, ds::DEF_C_SYMBOL).to_string(), // @PLN24
        c_sig: r.field_str(stores, ds::DEF_C_SIG).to_string(), // @PLN24
        superseded: r.field_str(stores, ds::DEF_SUPERSEDED).to_string(), // @PLN102 arc C
        op_code: r.field_int(stores, ds::DEF_OP_CODE) as u16,
        known_type: r.field_int(stores, ds::DEF_KNOWN_TYPE) as u16,
        pub_visible: r.field_bool(stores, ds::DEF_PUB_VISIBLE),
        null_safe: r.field_bool(stores, ds::DEF_NULL_SAFE), // @PLN46 W2
        closure_record: r.field_int(stores, ds::DEF_CLOSURE_RECORD) as u32,
        mutated_captures: read_name_list(
            stores,
            r.field_recvec(ds::DEF_MUTATED_CAPTURES, ds::NAMEREF_STRIDE),
        ),
        scalars_to_box: read_name_list(
            stores,
            r.field_recvec(ds::DEF_SCALARS_TO_BOX, ds::NAMEREF_STRIDE),
        ),
        bounds: read_u32_list(stores, r.field_recvec(ds::DEF_BOUNDS, ds::INT_STRIDE)),
        forced_size: if forced == 0 {
            None
        } else {
            Some(forced as u8)
        },
        purity: purity_from_code(r.field_int(stores, ds::DEF_PURITY)),
        field_groups: read_field_groups(stores, r),
        synthetic: if synthetic_s.is_empty() {
            None
        } else {
            Some(Box::leak(synthetic_s.to_owned().into_boxed_str()))
        },
        name,
        position,
    }
}

/// Read a `Definition`'s `vector<Attribute>`.
fn read_attributes(stores: &Stores, parent: Record) -> Vec<Attribute> {
    let v = parent.field_recvec(ds::DEF_ATTRIBUTES, ds::ATTRIBUTE_STRIDE);
    (0..v.len(stores))
        .map(|i| read_attribute(stores, v.get(i, stores)))
        .collect()
}

/// Read one `Attribute` record.
fn read_attribute(stores: &Stores, r: Record) -> Attribute {
    Attribute {
        name: r.field_str(stores, ds::ATTR_NAME).to_string(),
        typedef: read_type_child(stores, r.field_recvec(ds::ATTR_TYPEDEF, ds::TYPET_STRIDE)),
        mutable: r.field_bool(stores, ds::ATTR_MUTABLE),
        constant: r.field_bool(stores, ds::ATTR_CONSTANT),
        const_field: r.field_bool(stores, ds::ATTR_CONST_FIELD),
        // @PLN40 Phase 2 — value_const is NOT yet store-serialised (that needs an
        // ATTRIBUTE_STRIDE bump + a new schema bool offset).  Deferred to Phase 2b: it is
        // set at PARSE time and drives the compile-time chain-walk, which covers the
        // compile-and-run path (all current use).  A store round-trip resets it to false;
        // no store-durable value-const struct exists yet.
        value_const: false,
        init: r.field_bool(stores, ds::ATTR_INIT),
        nullable: r.field_bool(stores, ds::ATTR_NULLABLE),
        primary: r.field_bool(stores, ds::ATTR_PRIMARY),
        hidden: r.field_bool(stores, ds::ATTR_HIDDEN),
        value: read_node_child(stores, r.field_vec(ds::ATTR_VALUE)),
        check: read_node_child(stores, r.field_vec(ds::ATTR_CHECK)),
        check_message: read_node_child(stores, r.field_vec(ds::ATTR_CHECK_MESSAGE)),
        alias_d_nr: r.field_int(stores, ds::ATTR_ALIAS_D_NR) as u32,
        assigned_lambda_d_nr: r.field_int(stores, ds::ATTR_ASSIGNED_LAMBDA_D_NR) as u32,
        // @PLN86 F8b — restore the group#right member links (space-joined; "" = none).
        links: r
            .field_str(stores, ds::ATTR_LINKS)
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        // @PLN35 — `#lexeme` is a parse-time-only marker, not stored; defaults to false.
        lexeme: false,
    }
}

/// Read a `Definition`'s `vector<LinkedFieldGroup>`.
fn read_field_groups(stores: &Stores, parent: Record) -> Vec<LinkedFieldGroup> {
    let v = parent.field_recvec(ds::DEF_FIELD_GROUPS, ds::LFG_STRIDE);
    (0..v.len(stores))
        .map(|i| {
            let r = v.get(i, stores);
            LinkedFieldGroup {
                kind: if r.field_int(stores, ds::LFG_KIND) == 0 {
                    LinkedFieldKind::Tuple
                } else {
                    LinkedFieldKind::Index
                },
                instance: r.field_int(stores, ds::LFG_INSTANCE) as u16,
                field_indices: read_dep_list(stores, r, ds::LFG_FIELD_INDICES),
                alignment: r.field_int(stores, ds::LFG_ALIGNMENT) as u8,
                size: r.field_int(stores, ds::LFG_SIZE) as u16,
            }
        })
        .collect()
}

/// Read the inlined `Function` (at `base`) of a `Definition` record: `name` /
/// `file`, the nine codegen-read fields of each `Variable`, the `names` map
/// (name → var_nr), and the `inline_ref_vars` set — all from the store, so the
/// `Function` is reconstructed losslessly (the store grew to hold `names` /
/// `inline_refs` because neither is reconstructible from the variable list and
/// codegen reads both on the load path; see plan README § arc C).
fn read_function(stores: &Stores, parent: Record, base: u32) -> Function {
    let name = parent.field_str(stores, base + ds::FN_NAME).to_string();
    let file = parent.field_str(stores, base + ds::FN_FILE).to_string();
    let vars_vec = parent.field_recvec(base + ds::FN_VARIABLES, ds::VARIABLE_STRIDE);
    let mut vars = Vec::with_capacity(vars_vec.len(stores) as usize);
    for i in 0..vars_vec.len(stores) {
        let vr = vars_vec.get(i, stores);
        vars.push(RestoredVar {
            name: vr.field_str(stores, ds::VAR_NAME).to_string(),
            type_def: read_type_child(stores, vr.field_recvec(ds::VAR_TYPE_DEF, ds::TYPET_STRIDE)),
            stack_pos: vr.field_int(stores, ds::VAR_STACK_POS) as u16,
            uses: vr.field_int(stores, ds::VAR_USES) as u16,
            argument: vr.field_bool(stores, ds::VAR_ARGUMENT),
            stack_allocated: vr.field_bool(stores, ds::VAR_STACK_ALLOCATED),
            skip_free: vr.field_bool(stores, ds::VAR_SKIP_FREE),
            captured: vr.field_bool(stores, ds::VAR_CAPTURED),
            caller_hidden_buf: vr.field_bool(stores, ds::VAR_CALLER_HIDDEN_BUF),
            view_elided: vr.field_bool(stores, ds::VAR_VIEW_ELIDED),
            owner_witness: vr.field_int(stores, ds::VAR_OWNER_WITNESS) as u16,
        });
    }
    let names_vec = parent.field_recvec(base + ds::FN_NAMES, ds::NAMENR_STRIDE);
    let names = (0..names_vec.len(stores))
        .map(|i| {
            let e = names_vec.get(i, stores);
            (
                e.field_str(stores, ds::NAMENR_NAME).to_string(),
                e.field_int(stores, ds::NAMENR_NR) as u16,
            )
        })
        .collect();
    let inline_refs = read_dep_list(stores, parent, base + ds::FN_INLINE_REFS);
    Function::from_snapshot(&name, &file, vars, names, inline_refs)
}

/// Read a `vector<integer>` (field `off` of `slot`) as a `Vec<u32>`.
fn read_u32_list(stores: &Stores, v: ds::RecVector) -> Vec<u32> {
    (0..v.len(stores))
        .map(|i| v.get(i, stores).field_int(stores, 0) as u32)
        .collect()
}

/// `DefType` from its stored integer code (inverse of `ir_store::def_type_code`).
fn def_type_from_code(c: i64) -> DefType {
    match c {
        0 => DefType::Unknown,
        1 => DefType::Function,
        2 => DefType::Dynamic,
        3 => DefType::Enum,
        4 => DefType::EnumValue,
        5 => DefType::Struct,
        6 => DefType::Vector,
        7 => DefType::Type,
        8 => DefType::Constant,
        9 => DefType::Generic,
        10 => DefType::Interface,
        other => panic!("ir_read: unknown DefType code {other}"),
    }
}

/// `Purity` from its stored integer code (inverse of `ir_store::purity_code`):
/// `0` Unknown, `1` Pure, `2 + cat` Impure with `ImpureCategory` `cat`.
fn purity_from_code(c: i64) -> Purity {
    match c {
        0 => Purity::Unknown,
        1 => Purity::Pure,
        n => Purity::Impure(match n - 2 {
            0 => ImpureCategory::HostIo,
            1 => ImpureCategory::Prng,
            2 => ImpureCategory::Io,
            3 => ImpureCategory::ParentWrite,
            4 => ImpureCategory::ParCall,
            other => panic!("ir_read: unknown Purity/ImpureCategory code {}", other + 2),
        }),
    }
}

// ─── Database type schema reader (D2a) ───────────────────────────────────────

/// Rebuild the database type schema (`Vec<database::Type>`) from the
/// `vector<DbType>` field at `off` of `parent` — the inverse of
/// [`crate::ir_store::materialize_schema`].  Each `Type.parents` (a derived
/// back-reference index, read only by parse-time layout validation + debug
/// display) is restored empty; the load path skips the validation that uses it.
#[must_use]
pub fn read_schema(stores: &Stores, parent: Record, off: u32) -> Vec<SchemaType> {
    let v = parent.field_recvec(off, ds::DBTYPE_STRIDE);
    (0..v.len(stores))
        .map(|i| read_db_type(stores, v.get(i, stores)))
        .collect()
}

/// Read one `DbType` record into a native `database::Type`.
fn read_db_type(stores: &Stores, r: Record) -> SchemaType {
    let parts = read_db_parts(
        stores,
        r.field_recvec(ds::DBTYPE_PARTS, ds::DBPARTS_STRIDE)
            .get(0, stores),
    );
    let keysv = r.field_recvec(ds::DBTYPE_KEYS, ds::KEY_STRIDE);
    let keys: Vec<Key> = (0..keysv.len(stores))
        .map(|i| {
            let e = keysv.get(i, stores);
            Key {
                type_nr: e.field_int(stores, ds::KEY_TYPE_NR) as i8,
                position: e.field_int(stores, ds::KEY_POSITION) as u16,
                start: e.field_int(stores, ds::KEY_START) as i32,
            }
        })
        .collect();
    let field_groups = read_field_groups_at(stores, r, ds::DBTYPE_FIELD_GROUPS);
    SchemaType::from_stored(
        r.field_str(stores, ds::DBTYPE_NAME).to_string(),
        parts,
        keys,
        r.field_bool(stores, ds::DBTYPE_COMPLEX),
        r.field_bool(stores, ds::DBTYPE_LINKED),
        r.field_int(stores, ds::DBTYPE_SIZE) as u16,
        r.field_int(stores, ds::DBTYPE_ALIGN) as u8,
        field_groups,
    )
}

/// Read one `DbParts` record into a native `database::Parts`.
fn read_db_parts(stores: &Stores, r: Record) -> Parts {
    let content = || r.field_int(stores, ds::PTCONTENT) as u16;
    match r.discriminant(stores) {
        ds::PT_BASE => Parts::Base,
        ds::PT_STRUCT => Parts::Struct(read_db_fields(stores, r, ds::PTSTRUCT_FIELDS)),
        ds::PT_ENUM => Parts::Enum(read_enum_pairs(stores, r, ds::PTENUM_VALUES)),
        ds::PT_ENUM_VALUE => Parts::EnumValue(
            r.field_int(stores, ds::PTENUMVALUE_VALUE) as u8,
            read_db_fields(stores, r, ds::PTENUMVALUE_FIELDS),
        ),
        ds::PT_BYTE => Parts::Byte(
            r.field_int(stores, ds::PTNUM_START) as i32,
            r.field_bool(stores, ds::PTNUM_NULLABLE),
        ),
        ds::PT_SHORT => Parts::Short(
            r.field_int(stores, ds::PTNUM_START) as i32,
            r.field_bool(stores, ds::PTNUM_NULLABLE),
        ),
        ds::PT_INT => Parts::Int(
            r.field_int(stores, ds::PTNUM_START) as i32,
            r.field_bool(stores, ds::PTNUM_NULLABLE),
        ),
        ds::PT_INT_RAW => Parts::IntRaw(
            r.field_int(stores, ds::PTNUM_START) as i32,
            r.field_bool(stores, ds::PTNUM_NULLABLE),
        ),
        ds::PT_SHORT_RAW => Parts::ShortRaw(
            r.field_int(stores, ds::PTNUM_START) as i32,
            r.field_bool(stores, ds::PTNUM_NULLABLE),
        ),
        ds::PT_VECTOR => Parts::Vector(content()),
        ds::PT_ARRAY => Parts::Array(content()),
        ds::PT_SORTED => Parts::Sorted(content(), read_key_fields(stores, r, ds::PTKEYS)),
        ds::PT_ORDERED => Parts::Ordered(content(), read_key_fields(stores, r, ds::PTKEYS)),
        ds::PT_HASH => Parts::Hash(content(), read_dep_list(stores, r, ds::PTFIELDS)),
        ds::PT_INDEX => Parts::Index(
            content(),
            read_key_fields(stores, r, ds::PTKEYS),
            r.field_int(stores, ds::PTINDEX_LEFT) as u16,
        ),
        ds::PT_RADIX => Parts::Radix(content(), read_dep_list(stores, r, ds::PTFIELDS)),
        ds::PT_TRIE => Parts::Trie(content(), r.field_int(stores, ds::PTTRIE_KEY) as u16),
        ds::PT_DB_REF => Parts::DbRef,
        ds::PT_CHILD_REC => Parts::ChildRec(content()),
        other => panic!("ir_read: unknown DbParts discriminant {other}"),
    }
}

/// Read a `vector<DbField>` (field `off` of `parent`) into a `Vec<Field>`.
fn read_db_fields(stores: &Stores, parent: Record, off: u32) -> Vec<SchemaField> {
    let v = parent.field_recvec(off, ds::DBFIELD_STRIDE);
    (0..v.len(stores))
        .map(|i| {
            let r = v.get(i, stores);
            let default = read_db_content(
                stores,
                r.field_recvec(ds::DBFIELD_DEFAULT, ds::DBCONTENT_STRIDE)
                    .get(0, stores),
            );
            SchemaField::from_stored(
                r.field_str(stores, ds::DBFIELD_NAME).to_string(),
                r.field_int(stores, ds::DBFIELD_CONTENT) as u16,
                r.field_int(stores, ds::DBFIELD_POSITION) as u16,
                default,
                r.field_bool(stores, ds::DBFIELD_NULLABLE),
                read_dep_list(stores, r, ds::DBFIELD_OTHER_INDEXES),
            )
        })
        .collect()
}

/// Read one `DbContent` record into a native `keys::Content`.  A `Str` default
/// is reconstructed via a bounded `Box::leak` (mirrors `database::snapshot`).
fn read_db_content(stores: &Stores, r: Record) -> Content {
    match r.discriminant(stores) {
        ds::DC_LONG => Content::Long(r.field_int(stores, ds::DCLONG_V)),
        ds::DC_FLOAT => Content::Float(r.field_float(stores, ds::DCFLOAT_V)),
        ds::DC_SINGLE => Content::Single(r.field_single(stores, ds::DCSINGLE_V)),
        ds::DC_STR => {
            let s = r.field_str(stores, ds::DCSTR_V);
            if s.is_empty() {
                Content::Str(Str::new(""))
            } else {
                Content::Str(Str::new(Box::leak(s.to_owned().into_boxed_str())))
            }
        }
        other => panic!("ir_read: unknown DbContent discriminant {other}"),
    }
}

/// Read a `vector<EnumPair>` (field `off`) into a `Vec<(u16, String)>`.
fn read_enum_pairs(stores: &Stores, parent: Record, off: u32) -> Vec<(u16, String)> {
    let v = parent.field_recvec(off, ds::ENUMPAIR_STRIDE);
    (0..v.len(stores))
        .map(|i| {
            let e = v.get(i, stores);
            (
                e.field_int(stores, ds::ENUMPAIR_NR) as u16,
                e.field_str(stores, ds::ENUMPAIR_NAME).to_string(),
            )
        })
        .collect()
}

/// Read a `vector<KeyField>` (field `off`) into a `Vec<(u16, bool)>`.
fn read_key_fields(stores: &Stores, parent: Record, off: u32) -> Vec<(u16, bool)> {
    let v = parent.field_recvec(off, ds::KEYFIELD_STRIDE);
    (0..v.len(stores))
        .map(|i| {
            let e = v.get(i, stores);
            (
                e.field_int(stores, ds::KEYFIELD_NR) as u16,
                e.field_bool(stores, ds::KEYFIELD_ASC),
            )
        })
        .collect()
}

/// Read a `vector<LinkedFieldGroup>` at an explicit `off` (the `Definition`
/// reader's `read_field_groups` is hard-wired to `DEF_FIELD_GROUPS`).
fn read_field_groups_at(stores: &Stores, parent: Record, off: u32) -> Vec<LinkedFieldGroup> {
    let v = parent.field_recvec(off, ds::LFG_STRIDE);
    (0..v.len(stores))
        .map(|i| {
            let r = v.get(i, stores);
            LinkedFieldGroup {
                kind: if r.field_int(stores, ds::LFG_KIND) == 0 {
                    LinkedFieldKind::Tuple
                } else {
                    LinkedFieldKind::Index
                },
                instance: r.field_int(stores, ds::LFG_INSTANCE) as u16,
                field_indices: read_dep_list(stores, r, ds::LFG_FIELD_INDICES),
                alignment: r.field_int(stores, ds::LFG_ALIGNMENT) as u8,
                size: r.field_int(stores, ds::LFG_SIZE) as u16,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Deps;
    use crate::data_store::{RecVector, ValuesVector};
    use crate::ir_schema_gen::register_ir_schema;
    use crate::ir_store::{materialize_node, materialize_type};

    /// `native Value → store → native Value` must round-trip identically.
    fn round_trip_value(v: &Value) {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = ValuesVector::new(stores.database(16));
        materialize_node(&mut stores, root, v);
        let back = read_value(&stores, root.get(0, &stores));
        assert_eq!(*v, back, "Value round-trip mismatch");
    }

    /// `native Type → store → native Type` must round-trip identically.
    fn round_trip_type(t: &Type) {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = RecVector::new(stores.database(16), ds::TYPET_STRIDE);
        materialize_type(&mut stores, root, t);
        let back = read_type(&stores, root.get(0, &stores));
        assert_eq!(*t, back, "Type round-trip mismatch");
    }

    #[test]
    fn value_leaves_round_trip() {
        for v in [
            Value::Null,
            Value::Line(42),
            Value::Int(-7),
            Value::Enum(5, 6),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Float(2.5),
            Value::Long(-99),
            Value::Single(1.5),
            Value::Text("hi".into()),
            Value::Var(7),
            Value::Break(2),
            Value::Continue(1),
            Value::FnRefDnr(4),
            Value::TupleGet(1, 2),
            Value::RawExpr("x".into()),
        ] {
            round_trip_value(&v);
        }
    }

    #[test]
    fn value_recursive_round_trips() {
        // Call / CallRef / Insert / Tuple / Parallel (scalar + Vec<Value>).
        round_trip_value(&Value::Call(144, vec![Value::Int(7), Value::Int(9)]));
        round_trip_value(&Value::CallRef(3, vec![Value::Int(1), Value::Null]));
        round_trip_value(&Value::Insert(vec![Value::Int(1), Value::Int(2)]));
        round_trip_value(&Value::Tuple(vec![Value::Boolean(true), Value::Long(8)]));
        round_trip_value(&Value::Parallel(vec![Value::Int(1)]));
        // Box-of-one children.
        round_trip_value(&Value::Set(9, Box::new(Value::Long(42))));
        round_trip_value(&Value::Return(Box::new(Value::Int(3))));
        round_trip_value(&Value::Drop(Box::new(Value::Int(5))));
        round_trip_value(&Value::Yield(Box::new(Value::Int(6))));
        round_trip_value(&Value::TuplePut(2, 1, Box::new(Value::Int(7))));
        round_trip_value(&Value::If(
            Box::new(Value::Int(1)),
            Box::new(Value::Return(Box::new(Value::Int(2)))),
            Box::new(Value::Drop(Box::new(Value::Int(3)))),
        ));
        round_trip_value(&Value::Iter(
            5,
            Box::new(Value::Int(10)),
            Box::new(Value::Int(11)),
            Box::new(Value::Int(12)),
        ));
    }

    #[test]
    fn value_block_and_loop_round_trip() {
        round_trip_value(&Value::Block(Box::new(Block {
            name: "loop_body",
            operators: vec![Value::Call(5, vec![Value::Int(1)]), Value::Null],
            result: Type::Boolean,
            scope: 3,
            var_size: 16,
        })));
        round_trip_value(&Value::Loop(Box::new(Block {
            name: "spin",
            operators: vec![Value::Break(0)],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        })));
    }

    #[test]
    fn value_inline_substructs_round_trip() {
        round_trip_value(&Value::Span(Box::new((
            Position {
                file: "f.loft".into(),
                line: 12,
                pos: 3,
            },
            Value::Int(7),
        ))));
        round_trip_value(&Value::Keys(vec![
            Key {
                type_nr: 3,
                position: 7,
                start: -128,
            },
            Key {
                type_nr: -1,
                position: 42,
                start: 0,
            },
        ]));
        round_trip_value(&Value::FnRef(42, 3, Box::new(Type::Boolean)));
    }

    #[test]
    fn type_all_variants_round_trip() {
        for t in [
            Type::Unknown(7),
            Type::Null,
            Type::Void,
            Type::Never,
            Type::Boolean,
            Type::Float,
            Type::Single,
            Type::Character,
            Type::Keys,
            Type::Integer(IntegerSpec::wide()),
            Type::Integer(IntegerSpec::u8()),
            Type::Integer(IntegerSpec::signed32()),
            Type::Text(crate::data::Deps::none()),
            Type::Text(crate::data::Deps::unknown(vec![1, 2, 3])),
            Type::Enum(4, true, crate::data::Deps::unknown(vec![5])),
            Type::Enum(4, false, crate::data::Deps::none()),
            Type::Reference(9, crate::data::Deps::unknown(vec![0, 1])),
            Type::RefVar(Box::new(Type::Boolean)),
            Type::Vector(
                Box::new(Type::Text(crate::data::Deps::none())),
                crate::data::Deps::unknown(vec![2]),
            ),
            Type::Routine(11),
            Type::Iterator(
                Box::new(Type::Integer(IntegerSpec::wide())),
                Box::new(Type::Null),
            ),
            Type::Sorted(
                3,
                vec![("name".into(), true), ("age".into(), false)],
                crate::data::Deps::unknown(vec![1]),
            ),
            Type::Index(3, vec![("key".into(), true)], crate::data::Deps::none()),
            Type::Radix(
                3,
                vec!["x".into(), "y".into()],
                crate::data::Deps::unknown(vec![1]),
            ),
            Type::Hash(3, vec!["id".into()], crate::data::Deps::none()),
            Type::Function(
                vec![Type::Integer(IntegerSpec::wide()), Type::Text(Deps::none())],
                Box::new(Type::Boolean),
                Deps::unknown(vec![0]),
            ),
            Type::Rewritten(Box::new(Type::Text(Deps::none()))),
            Type::Tuple(vec![
                Type::Integer(IntegerSpec::wide()),
                Type::Text(Deps::none()),
            ]),
        ] {
            round_trip_type(&t);
        }
    }

    #[test]
    fn type_nested_recursion_round_trips() {
        // vector<vector<reference>> and a function returning a vector.
        round_trip_type(&Type::Vector(
            Box::new(Type::Vector(
                Box::new(Type::Reference(2, Deps::none())),
                Deps::none(),
            )),
            Deps::none(),
        ));
        round_trip_type(&Type::Function(
            vec![
                Type::Integer(IntegerSpec::i32()),
                Type::Text(Deps::unknown(vec![1, 2])),
            ],
            Box::new(Type::Vector(
                Box::new(Type::Boolean),
                Deps::unknown(vec![3]),
            )),
            Deps::unknown(vec![7]),
        ));
    }

    /// `forced_size` is ignored by `IntegerSpec`'s `PartialEq`, so the `==`
    /// round-trip above can't catch a wrong width — assert it explicitly.
    #[test]
    fn integer_forced_size_round_trips_exactly() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);
        let root = RecVector::new(stores.database(16), ds::TYPET_STRIDE);
        materialize_type(&mut stores, root, &Type::Integer(IntegerSpec::u8()));
        let Type::Integer(spec) = read_type(&stores, root.get(0, &stores)) else {
            panic!("expected Integer");
        };
        assert_eq!(spec.forced_size, NonZeroU8::new(1));
    }

    // ── Definition / Data reader (whole-stdlib round-trip) ──────────────────

    use crate::data::Data;
    use crate::ir_schema::{compare_data, definition_to_json};
    use crate::ir_store::materialize_data;

    /// Parse the real `default/` stdlib once, materialize, read back.
    fn stdlib_round_trip() -> (Data, Data) {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let fresh = p.data;
        let mut ir = Stores::new();
        let _ids = register_ir_schema(&mut ir);
        let root = materialize_data(&mut ir, &fresh);
        let loaded = read_data(&ir, root);
        (fresh, loaded)
    }

    /// Capstone: the whole real `default/` stdlib round-trips
    /// `native → store → native` **bit-for-bit**.  The full `compare_data`
    /// oracle — every `Definition` field including the per-function variable
    /// table (`vars` + `names` + `inline_refs`) — is green across all
    /// definitions.  This is arc C's read-path milestone: a store-materialised
    /// `Data` is indistinguishable from a fresh parse.
    #[test]
    fn read_whole_stdlib_compare_data_green() {
        let (fresh, loaded) = stdlib_round_trip();
        assert!(
            fresh.definitions() > 100,
            "stdlib should have many definitions, got {}",
            fresh.definitions()
        );
        if let Err(diff) = compare_data(&fresh, &loaded) {
            panic!("store round-trip diverged from fresh parse: {diff:?}");
        }
    }

    /// cache_verify: `compare_data` only checks the serialized `definitions` +
    /// the Data header — NOT the DERIVED indices (`def_names` / `operators` /
    /// `possible` / `use_names`) that a warm cache load rebuilds.  That gap is
    /// exactly how the cross-source synthetic-wrapper bug (#290 / the crawler
    /// `build_walls` cache panic) shipped: the dropped binding lived in
    /// `def_names`, invisible to `compare_data`.  This asserts the rebuilt
    /// indices match a fresh parse for the whole stdlib — surfacing any binding
    /// the round-trip silently drops.
    #[test]
    fn stdlib_round_trip_preserves_derived_indices() {
        let (fresh, loaded) = stdlib_round_trip();
        let diff = fresh.derived_indices_diff(&loaded);
        assert!(
            diff.is_empty(),
            "stdlib round-trip drops/changes {} derived-index binding(s):\n{}",
            diff.len(),
            diff.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
        );
    }

    /// The warm-cache contract for a MULTI-source program (an importing `main`
    /// beside a `use`d lib): the serialized definitions AND the rebuilt derived
    /// indices survive a materialize→read round-trip.  The stdlib alone cannot
    /// test this — it is effectively single-source.  The cross-source part
    /// rests on the two import tables the image carries: `rebuild_indices`
    /// reconstructs `def_names` under each definition's OWN source and then
    /// replays the imports to recreate the `use lib::*` bindings, and
    /// `use_names` (`{"importlib": 1}`) is restored verbatim because an alias
    /// names a source, not a definition. (loft#1359)
    #[test]
    fn multi_source_round_trip_preserves_derived_indices() {
        let s = std::path::MAIN_SEPARATOR;
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        p.parse(&format!("tests{s}lib{s}wildcard_import_main.loft"), false);
        let fresh = p.data;
        let mut ir = Stores::new();
        let _ids = register_ir_schema(&mut ir);
        let root = materialize_data(&mut ir, &fresh);
        let loaded = read_data(&ir, root);
        if let Err(diff) = compare_data(&fresh, &loaded) {
            panic!("multi-source definitions diverged: {diff:?}");
        }
        let diff = fresh.derived_indices_diff(&loaded);
        assert!(
            diff.is_empty(),
            "multi-source round-trip drops/changes {} derived-index binding(s):\n{}",
            diff.len(),
            diff.iter().take(40).cloned().collect::<Vec<_>>().join("\n")
        );
    }

    /// @PLN11 G2 / M0 — the equivalence harness wrapper (`ir_roundtrip_check`)
    /// the run path invokes under `LOFT_IR_CHECK`.  It materialises + reads back
    /// without registering the IR schema (baked offsets) and returns `Ok` when
    /// the whole stdlib round-trips bit-for-bit.
    #[test]
    fn ir_roundtrip_check_stdlib_ok() {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        assert!(p.data.definitions() > 100);
        assert!(
            ir_roundtrip_check(&p.data).is_ok(),
            "stdlib store round-trip must equal native Data"
        );
    }

    /// Spot-check that a function-body definition's variable table — the part
    /// that needed the schema growth — survives intact: `vars` count, the
    /// `names` map, and `inline_ref_vars` all re-encode identically.
    #[test]
    fn read_stdlib_function_variables_round_trip() {
        let (fresh, loaded) = stdlib_round_trip();
        let mut checked = 0u32;
        for d_nr in 0..fresh.definitions() {
            let rf = fresh.def(d_nr);
            if rf.variables.snapshot_len() == 0 {
                continue;
            }
            // definition_to_json embeds the full variable block (vars + names +
            // inline_refs); equality here proves the populated table round-trips.
            assert_eq!(
                definition_to_json(rf),
                definition_to_json(loaded.def(d_nr)),
                "function def {d_nr} ({}) variable table differs",
                rf.name
            );
            checked += 1;
        }
        assert!(checked > 20, "expected many function defs, got {checked}");
    }

    /// @PLN11 arc D probe — the full mmap loop end-to-end: materialize the
    /// real stdlib `Data` into a **file-backed** store, drop it (flush to
    /// disk), reopen the file via mmap, and reconstruct native `Data` with
    /// `read_data` — **no re-parse, no schema registration** (the reader walks
    /// the mapped bytes through baked offsets).  The rebuilt `Data` is
    /// bit-for-bit identical to the fresh parse (`compare_data`).  This proves
    /// "serialize the IR to a Store file, mmap it back, push it into a native
    /// `Data`" works today.
    #[cfg(feature = "mmap")]
    #[test]
    fn mmap_file_round_trip_stdlib() {
        use crate::ir_schema::compare_data;
        use crate::ir_store::materialize_data_at;
        use crate::store::Store;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let fresh = p.data;

        let path = std::env::temp_dir().join(format!("loft_ir_mmap_{}.store", std::process::id()));
        let path_str = path.to_str().expect("utf-8 temp path");
        let _ = std::fs::remove_file(&path);

        // ── write: materialize the IR straight into a file-backed store ──
        let root = {
            let fstore = Store::open(path_str); // create + mmap (writable)
            let mut stores = Stores::new();
            let nr = stores.adopt_store(fstore);
            let rec = stores.allocations[nr as usize].claim(16);
            let root = DbRef {
                store_nr: nr,
                rec,
                pos: 8,
            };
            materialize_data_at(&mut stores, root, &fresh);
            root
            // `stores` dropped here → the file-backed Store is unmapped/flushed
        };

        // ── load: mmap the file back, reconstruct native Data (no re-parse) ──
        let fstore2 = Store::open(path_str);
        let mut s2 = Stores::new();
        let nr2 = s2.adopt_store(fstore2);
        let root2 = DbRef {
            store_nr: nr2,
            rec: root.rec,
            pos: root.pos,
        };
        let loaded = read_data(&s2, root2);

        let result = compare_data(&fresh, &loaded);
        let _ = std::fs::remove_file(&path);
        if let Err(diff) = result {
            panic!("mmap file round-trip diverged from fresh parse: {diff:?}");
        }
    }

    /// @PLN11 arc D step D1 — the public `Data::save` / `Data::open` API:
    /// save the stdlib to a `.store` file, load it back (mmap, no re-parse),
    /// and confirm bit-for-bit equality.  Also checks that opening a missing
    /// file is a clean `NotFound` (the cache-miss path the caller falls back on).
    #[cfg(feature = "mmap")]
    #[test]
    fn data_save_open_round_trip_stdlib() {
        use crate::data::Data;
        use crate::ir_schema::compare_data;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let fresh = p.data;

        let path = std::env::temp_dir().join(format!("loft_data_api_{}.store", std::process::id()));
        let path_str = path.to_str().unwrap();

        fresh.save(path_str).expect("Data::save");
        let loaded = Data::open(path_str).expect("Data::open");
        let result = compare_data(&fresh, &loaded);
        let _ = std::fs::remove_file(&path);
        if let Err(diff) = result {
            panic!("Data::save/open round-trip diverged: {diff:?}");
        }

        // Opening a non-existent store is a clean cache-miss (NotFound).
        let miss = Data::open("/nonexistent/loft_no_such_store.store");
        assert_eq!(
            miss.err().map(|e| e.kind()),
            Some(std::io::ErrorKind::NotFound)
        );
    }

    /// @PLN86 2.2-persist — a NON-EMPTY `#cap` group survives the store round-trip
    /// (the path the `LOFT_STDLIB_CACHE` bundle uses), so a tagged stdlib loaded
    /// from cache still gates correctly.  Without persistence this reloads as `""`
    /// and admission silently over-rejects.
    #[cfg(feature = "mmap")]
    #[test]
    fn cap_annotation_survives_store_round_trip() {
        use crate::data::Data;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let src = "fn capped() -> integer fs#read;\n#native\n";
        let lpath = std::env::temp_dir().join(format!("loft_cap_rt_{}.loft", std::process::id()));
        std::fs::write(&lpath, src).unwrap();
        p.parse(lpath.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&lpath);
        assert_eq!(p.data.def(p.data.def_nr("n_capped")).cap(), "fs#read");

        let fresh = p.data;
        let spath = std::env::temp_dir().join(format!("loft_cap_rt_{}.store", std::process::id()));
        let spath_str = spath.to_str().unwrap();
        fresh.save(spath_str).expect("Data::save");
        let loaded = Data::open(spath_str).expect("Data::open");
        let _ = std::fs::remove_file(&spath);
        assert_eq!(
            loaded.def(loaded.def_nr("n_capped")).cap(),
            "fs#read",
            "cap must survive the store round-trip"
        );
        crate::ir_schema::compare_data(&fresh, &loaded).expect("round-trip equal incl. cap");
    }

    /// @PLN25 — the `Optional` (`τ?`) nullability marker survives the store round-trip.
    /// Before the `TyOptional` codec variant, `write_type` peeled the wrapper (persisting
    /// `Optional(Integer)` as bare `Integer`), so a reloaded nullable type silently lost
    /// its N-Store contract — the root of the `*_round_trip` failures once F1b(b) put
    /// `integer?` overloads in the stdlib.
    #[cfg(feature = "mmap")]
    #[test]
    fn optional_type_survives_store_round_trip() {
        use crate::data::{Data, Type};

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let src = "fn optret() -> integer? { null }\n";
        let lpath = std::env::temp_dir().join(format!("loft_opt_rt_{}.loft", std::process::id()));
        std::fs::write(&lpath, src).unwrap();
        p.parse(lpath.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&lpath);

        let nr = p.data.def_nr("n_optret");
        assert!(
            p.data.def(nr).returned.peel_optional().1,
            "sanity: `integer?` return parses as Optional under DN1"
        );

        let fresh = p.data;
        let spath = std::env::temp_dir().join(format!("loft_opt_rt_{}.store", std::process::id()));
        let spath_str = spath.to_str().unwrap();
        fresh.save(spath_str).expect("Data::save");
        let loaded = Data::open(spath_str).expect("Data::open");
        let _ = std::fs::remove_file(&spath);

        let (base, is_opt) = loaded
            .def(loaded.def_nr("n_optret"))
            .returned
            .peel_optional();
        assert!(
            is_opt,
            "the `Optional` marker must survive the store round-trip (was dropped pre-TyOptional)"
        );
        assert!(
            matches!(base, Type::Integer(_)),
            "base type preserved through the round-trip"
        );
        crate::ir_schema::compare_data(&fresh, &loaded).expect("round-trip equal incl. Optional");
    }

    /// @PLN86 F8b — a member's `group#right` capability links survive the store
    /// round-trip (the `LOFT_STDLIB_CACHE` path), so a warm-cached host type / library
    /// keeps its field-access rights instead of losing them with the parser side-maps.
    /// Without persistence the links reload empty and admission silently over-rejects.
    #[cfg(feature = "mmap")]
    #[test]
    fn member_links_survive_store_round_trip() {
        use crate::data::Data;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let src = "struct Inv { items: integer }\n";
        let lpath = std::env::temp_dir().join(format!("loft_ml_rt_{}.loft", std::process::id()));
        std::fs::write(&lpath, src).unwrap();
        p.parse(lpath.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&lpath);

        // Stamp the F8b carrier directly (the parse→links finalize is the next wiring
        // step); this proves the CODEC round-trips a non-empty link list.
        let inv = p.data.def_nr("Inv");
        let links = vec!["bag#read".to_string(), "bag#append".to_string()];
        p.data.definitions[inv as usize].attributes[0].links = links.clone();

        let fresh = p.data;
        let spath = std::env::temp_dir().join(format!("loft_ml_rt_{}.store", std::process::id()));
        let spath_str = spath.to_str().unwrap();
        fresh.save(spath_str).expect("Data::save");
        let loaded = Data::open(spath_str).expect("Data::open");
        let _ = std::fs::remove_file(&spath);

        let reloaded_inv = loaded.def_nr("Inv");
        assert_eq!(
            loaded.def(reloaded_inv).attributes[0].links,
            links,
            "member links must survive the store round-trip"
        );
        crate::ir_schema::compare_data(&fresh, &loaded).expect("round-trip equal incl. links");
    }

    /// @PLN46 W2-persist — `#null_safe` survives the store round-trip (the
    /// `LOFT_STDLIB_CACHE` path), so a null-safe stdlib helper loaded from cache
    /// still suppresses the undefended-fault warning at call sites.  Without
    /// persistence it reloads as `false` and the warning falsely re-fires.
    #[cfg(feature = "mmap")]
    #[test]
    fn null_safe_survives_store_round_trip() {
        use crate::data::Data;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let src = "fn tolerant(c: character) -> boolean { c == 'a' }\n#null_safe\n";
        let lpath = std::env::temp_dir().join(format!("loft_ns_rt_{}.loft", std::process::id()));
        std::fs::write(&lpath, src).unwrap();
        p.parse(lpath.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&lpath);
        assert!(
            p.data.def(p.data.def_nr("n_tolerant")).null_safe(),
            "parsed `#null_safe` must set the flag"
        );

        let fresh = p.data;
        let spath = std::env::temp_dir().join(format!("loft_ns_rt_{}.store", std::process::id()));
        let spath_str = spath.to_str().unwrap();
        fresh.save(spath_str).expect("Data::save");
        let loaded = Data::open(spath_str).expect("Data::open");
        let _ = std::fs::remove_file(&spath);
        assert!(
            loaded.def(loaded.def_nr("n_tolerant")).null_safe(),
            "null_safe must survive the store round-trip"
        );
        crate::ir_schema::compare_data(&fresh, &loaded).expect("round-trip equal incl. null_safe");
    }

    #[cfg(feature = "mmap")]
    #[test]
    /// @PLN102 arc C step 1 — `#superseded "Y"` parses to the bare successor name
    /// and survives both the binary-store and JSON round-trips (so a marked stdlib
    /// symbol keeps its mark when loaded from the `LOFT_STDLIB_CACHE` bundle).
    fn superseded_survives_store_round_trip() {
        use crate::data::Data;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let src = "fn old_add(a: integer, b: integer) -> integer { a + b }\n#superseded \"new_add\"\nfn new_add(a: integer, b: integer) -> integer { a + b }\n";
        let lpath = std::env::temp_dir().join(format!("loft_sup_rt_{}.loft", std::process::id()));
        std::fs::write(&lpath, src).unwrap();
        p.parse(lpath.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&lpath);
        assert_eq!(
            p.data.def(p.data.def_nr("n_old_add")).superseded(),
            "new_add",
            "parsed `#superseded \"new_add\"` must store the bare successor name"
        );

        let fresh = p.data;
        let spath = std::env::temp_dir().join(format!("loft_sup_rt_{}.store", std::process::id()));
        let spath_str = spath.to_str().unwrap();
        fresh.save(spath_str).expect("Data::save");
        let loaded = Data::open(spath_str).expect("Data::open");
        let _ = std::fs::remove_file(&spath);
        assert_eq!(
            loaded.def(loaded.def_nr("n_old_add")).superseded(),
            "new_add",
            "the #superseded mark must survive the store round-trip"
        );
        crate::ir_schema::compare_data(&fresh, &loaded).expect("round-trip equal incl. superseded");
    }

    /// @PLN24 arc A — a `#c` binding survives the store round-trip.
    ///
    /// The signature is the SOLE authority on how the C function is called
    /// (nothing at runtime can check it), so a cached program that came back
    /// with an empty one would be a binding silently unbound — the failure this
    /// whole arc exists to make impossible.  Guarding it here, while nothing
    /// consumes the field yet, is the cheap moment: arc B and arc C would each
    /// inherit the hole otherwise.
    #[cfg(feature = "mmap")]
    #[test]
    fn c_binding_survives_store_round_trip() {
        use crate::data::Data;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let src = "pub fn c_len(s: text) -> integer;\n#c \"strlen\" \"size_t(const char*)\"\n";
        let lpath = std::env::temp_dir().join(format!("loft_c_rt_{}.loft", std::process::id()));
        std::fs::write(&lpath, src).unwrap();
        p.parse(lpath.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&lpath);
        let fresh = p.data;
        let d = fresh.def(fresh.def_nr("n_c_len"));
        assert_eq!(d.c_symbol, "strlen", "parsed `#c` must store the symbol");
        assert_eq!(d.c_sig, "size_t(const char*)", "and the signature verbatim");

        let spath = std::env::temp_dir().join(format!("loft_c_rt_{}.store", std::process::id()));
        let spath_str = spath.to_str().unwrap();
        fresh.save(spath_str).expect("Data::save");
        let loaded = Data::open(spath_str).expect("Data::open");
        let _ = std::fs::remove_file(&spath);
        let d = loaded.def(loaded.def_nr("n_c_len"));
        assert_eq!(
            d.c_symbol, "strlen",
            "the symbol must survive the round-trip"
        );
        assert_eq!(
            d.c_sig, "size_t(const char*)",
            "and so must the signature — an empty one is a binding nobody can check"
        );
        // The text is what persists; the WIDTHS it means are re-derived per
        // target, so the accessor is what a consumer must go through.
        let sig = crate::c_signature::of(
            &loaded,
            loaded.def_nr("n_c_len"),
            crate::c_signature::CTarget {
                long_bits: 64,
                char_signed: true,
            },
        )
        .expect("a #c definition reports a signature")
        .expect("and it parses");
        assert_eq!(sig.symbol, "strlen");
        assert_eq!(sig.params.len(), 1);
        crate::ir_schema::compare_data(&fresh, &loaded).expect("round-trip equal incl. #c");
    }

    #[test]
    /// @PLN102 arc C step 2 — `source_is_owned` / `caller_source_is_owned`
    /// distinguish the OWNED entry project (`MAIN_SOURCE`) from a re-parsed
    /// dependency (source `2..`, what a `use`d library gets) and the stdlib
    /// (`STD_SOURCE`).  This is the caller-provenance fact the arc-C steer gate
    /// (step 3) reads so a `#superseded` warning reaches only whoever can act.
    fn source_ownership_distinguishes_entry_dependency_stdlib() {
        use crate::data::{MAIN_SOURCE, STD_SOURCE};

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");

        // The bare fact on each source class.
        assert!(
            p.data.source_is_owned(MAIN_SOURCE),
            "entry (MAIN_SOURCE) is owned"
        );
        assert!(!p.data.source_is_owned(STD_SOURCE), "stdlib is NOT owned");
        assert!(
            !p.data.source_is_owned(2),
            "a dependency source (2..) is NOT owned"
        );

        // The stdlib parse produced defs, and none of them reads as owned.
        assert!(
            (0..p.data.definitions.len()).any(|d| p.data.def(d as u32).source() == STD_SOURCE),
            "stdlib parse produced STD_SOURCE defs"
        );

        // An entry-file def, parsed at MAIN_SOURCE, IS owned; and the current
        // compile source is owned right after.
        p.parse_snippet(
            "fn my_entry_fn() -> integer { 42 }\n",
            "<main>",
            MAIN_SOURCE,
        );
        assert!(
            p.data.caller_source_is_owned(),
            "MAIN_SOURCE current -> owned"
        );
        let entry = p.data.source_nr(MAIN_SOURCE, "n_my_entry_fn");
        assert_ne!(entry, u32::MAX, "entry fn resolves at MAIN_SOURCE");
        assert!(
            p.data.source_is_owned(p.data.def(entry).source()),
            "an entry-file def IS owned"
        );

        // A dependency def, parsed at source 2 (what a `use`d library gets), is
        // NOT owned; and the current compile source is NOT owned right after.
        p.parse_snippet("fn my_dep_fn() -> integer { 7 }\n", "<dep>", 2);
        assert!(
            !p.data.caller_source_is_owned(),
            "source 2 current -> NOT owned"
        );
        let dep = p.data.source_nr(2, "n_my_dep_fn");
        assert_ne!(dep, u32::MAX, "dependency fn resolves at source 2");
        assert!(
            !p.data.source_is_owned(p.data.def(dep).source()),
            "a dependency def is NOT owned"
        );
    }

    #[test]
    /// @PLN102 arc C step 3 — the recommended-idiom STEER fires for a call to a
    /// `#superseded` symbol FROM OWNED source and is SILENT from a dependency
    /// source (the caller-provenance Q3 gate).  Same shape both times — a marked
    /// fn plus a caller — differing ONLY in the source number, so the owned case
    /// proves the mechanism can steer and the dependency case proves the gate
    /// blocks it.  The steer lives in the shared parser (`call_nr`), so this is
    /// backend-agnostic; `LOFT_NO_STEER` opt-out + the both-backends surface are
    /// covered by the CLI probe.
    fn steer_fires_from_owned_source_silent_from_dependency() {
        use crate::data::MAIN_SOURCE;
        use crate::diagnostics::Level;

        // {0} is superseded by {1} and FOLDED onto it (a shim); {2} calls {0}
        // (the owned-source call that steers).
        let marked = |names: (&str, &str, &str)| {
            format!(
                "fn {0}() -> integer {{ {1}() }}\n#superseded \"{1}\"\nfn {1}() -> integer {{ 2 }}\nfn {2}() -> integer {{ {0}() }}\n",
                names.0, names.1, names.2
            )
        };
        // ADVICE, not Warning: the steer's own prose says "the old form keeps working",
        // so ignoring it cannot produce a wrong result — and gating on it would make a
        // shipped library fail its own CI the moment loft supersedes a symbol it uses.
        // Its sibling below (a `#superseded` body that never calls its successor) stays
        // a Warning: there, two implementations can silently diverge.
        let is_steer = |e: &crate::diagnostics::DiagEntry| {
            e.level == Level::Advice && e.message.contains("is superseded")
        };

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");

        // OWNED (MAIN_SOURCE): a call to a #superseded fn steers.
        let pre = p.diagnostics.entries().len();
        p.parse_snippet(
            &marked(("old_fn", "new_fn", "caller")),
            "<main>",
            MAIN_SOURCE,
        );
        assert!(
            p.diagnostics.entries()[pre..].iter().any(is_steer),
            "a #superseded call from OWNED source must steer"
        );

        // DEPENDENCY (source 2): the SAME shape is silent — the caller is not owned.
        let pre = p.diagnostics.entries().len();
        p.parse_snippet(&marked(("dep_old", "dep_new", "dep_caller")), "<dep>", 2);
        assert!(
            !p.diagnostics.entries()[pre..].iter().any(is_steer),
            "a #superseded call from a DEPENDENCY source must NOT steer"
        );
    }

    #[test]
    /// @PLN102 arc C step 4 — the fold lint: a `#superseded "Y"` symbol must
    /// RESOLVE Y (an unresolvable successor is a hard Error — a dangling steer
    /// never ships) and its body must CALL Y (a shim over the successor — an
    /// un-folded steer is an advisory Warning); a properly-folded one is clean.
    fn fold_lint_flags_dangling_and_unfolded_superseded() {
        use crate::diagnostics::{Diagnostics, Level};

        let check = |src: &str| -> Vec<(Level, String)> {
            let mut p = crate::parser::Parser::new();
            p.parse_dir("default", true, false).expect("parse stdlib");
            p.parse_snippet(src, "<main>", crate::data::MAIN_SOURCE);
            let mut diags = Diagnostics::new();
            crate::use_analysis::superseded_fold_diagnostics(&p.data, &mut diags, "<main>");
            diags
                .entries()
                .iter()
                .map(|e| (e.level, e.message.clone()))
                .collect()
        };

        // (1) folded → clean: old_f is a shim over new_f.
        let clean = check(
            "fn old_f() -> integer { new_f() }\n#superseded \"new_f\"\nfn new_f() -> integer { 2 }\n",
        );
        assert!(
            clean.is_empty(),
            "a folded #superseded symbol must be clean; got {clean:?}"
        );

        // (2) un-folded → advisory Warning: old_g does not call new_g.
        let unfolded = check(
            "fn old_g() -> integer { 1 }\n#superseded \"new_g\"\nfn new_g() -> integer { 2 }\n",
        );
        assert!(
            unfolded
                .iter()
                .any(|(l, m)| *l == Level::Warning && m.contains("never calls")),
            "an un-folded #superseded symbol must warn; got {unfolded:?}"
        );

        // (3) dangling successor → hard Error.
        let dangling = check("fn old_h() -> integer { 1 }\n#superseded \"nonesuch\"\n");
        assert!(
            dangling
                .iter()
                .any(|(l, m)| *l == Level::Error && m.contains("no such successor")),
            "a dangling #superseded successor must error; got {dangling:?}"
        );

        // (4) a METHOD successor resolves (`t_<LEN><Type>_<succ>`, not just `n_<succ>`):
        // a folded method is clean; an un-folded one warns by its bare method name.
        let clean_method = check(
            "struct Box { n: integer }\nfn old_m(self: Box) -> integer { self.new_m() }\n#superseded \"new_m\"\nfn new_m(self: Box) -> integer { self.n }\n",
        );
        assert!(
            clean_method.is_empty(),
            "a folded #superseded METHOD must resolve its successor and be clean; got {clean_method:?}"
        );
        let unfolded_method = check(
            "struct Box { n: integer }\nfn keep_m(self: Box) -> integer { self.n }\n#superseded \"other_m\"\nfn other_m(self: Box) -> integer { self.n + 1 }\n",
        );
        assert!(
            unfolded_method.iter().any(|(l, m)| *l == Level::Warning
                && m.contains("`keep_m`")
                && m.contains("never calls")),
            "an un-folded #superseded METHOD must warn by its bare name; got {unfolded_method:?}"
        );
    }

    #[test]
    /// Generic instantiation is caller-source-agnostic: a stdlib fn (parsed under
    /// `self.default`, i.e. `STD_SOURCE`) can call a GENERIC stdlib fn, exactly as a user
    /// program can.  Gating `parse_call`'s generic-instantiation block on the caller's
    /// source resolves a stdlib-internal generic call to "Unknown function". (loft#653)
    fn stdlib_fn_can_call_a_generic() {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).expect("parse stdlib");
        let pre = p.diagnostics.entries().len();
        // Parse a helper WITH default=true (the stdlib parse context) that calls the
        // generic `min_of` — the exact shape that used to fail.
        p.parse_source(
            "fn my_std_helper(v: vector<integer>) -> integer { min_of(v) ?? 0 }\n",
            "<std-generic-test>",
            true,
        );
        let new_diags: Vec<String> = p.diagnostics.entries()[pre..]
            .iter()
            .map(|e| e.message.clone())
            .collect();
        assert!(
            !new_diags.iter().any(|m| m.contains("Unknown function")),
            "a stdlib fn calling a generic must resolve, not 'Unknown function'; got {new_diags:?}"
        );
        assert_ne!(
            p.data.source_nr(crate::data::STD_SOURCE, "n_my_std_helper"),
            u32::MAX,
            "the stdlib-context helper that calls a generic must parse"
        );
    }

    /// @PLN11 arc D micro-bench — wall-clock of producing the native stdlib
    /// `Data` two ways: a fresh `parse_dir` (today's cold-start path) vs
    /// `Store::open` + `read_data` (mmap the precompiled store, rebuild native).
    /// Run with: `cargo test --release --lib bench_stdlib_load -- --ignored --nocapture`.
    #[cfg(feature = "mmap")]
    #[test]
    #[ignore = "perf bench — run manually with --ignored --nocapture"]
    #[allow(clippy::cast_precision_loss)] // micro-second counts are tiny
    fn bench_stdlib_load_mmap_vs_parse() {
        use crate::ir_store::materialize_data_at;
        use crate::store::Store;
        use std::time::Instant;

        let parse = || {
            let mut p = crate::parser::Parser::new();
            p.parse_dir("default", true, false).expect("parse default/");
            p.data
        };

        // Build the .store file once.
        let path = std::env::temp_dir().join(format!("loft_ir_bench_{}.store", std::process::id()));
        let path_str = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);
        let (rec, pos) = {
            let fresh = parse();
            let fstore = Store::open(path_str);
            let mut stores = Stores::new();
            let nr = stores.adopt_store(fstore);
            let rec = stores.allocations[nr as usize].claim(16);
            let root = DbRef {
                store_nr: nr,
                rec,
                pos: 8,
            };
            materialize_data_at(&mut stores, root, &fresh);
            (rec, root.pos)
        };

        let iters = 25;
        let median = |mut v: Vec<u128>| {
            v.sort_unstable();
            v[v.len() / 2]
        };

        let mut parse_us = Vec::new();
        for _ in 0..iters {
            let t = Instant::now();
            let d = parse();
            std::hint::black_box(&d);
            parse_us.push(t.elapsed().as_micros());
        }

        let mut load_us = Vec::new();
        for _ in 0..iters {
            let t = Instant::now();
            let fstore = Store::open(path_str);
            let mut s2 = Stores::new();
            let nr2 = s2.adopt_store(fstore);
            let root2 = DbRef {
                store_nr: nr2,
                rec,
                pos,
            };
            let d = read_data(&s2, root2);
            std::hint::black_box(&d);
            load_us.push(t.elapsed().as_micros());
        }

        let file_bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
        let _ = std::fs::remove_file(&path);

        let (p_min, p_med) = (parse_us.iter().copied().min().unwrap(), median(parse_us));
        let (l_min, l_med) = (load_us.iter().copied().min().unwrap(), median(load_us));
        eprintln!("\n=== stdlib load: parse vs mmap+read_data ({iters} iters) ===");
        eprintln!("store file size : {} KiB", file_bytes / 1024);
        eprintln!("parse_dir       : min {p_min} us   median {p_med} us");
        eprintln!("mmap+read_data  : min {l_min} us   median {l_med} us");
        eprintln!(
            "speedup (median): {:.2}x   (min) {:.2}x",
            p_med as f64 / l_med as f64,
            p_min as f64 / l_min as f64
        );
    }

    /// @PLN11 G2 — warm-load cost breakdown: full `read_data` vs
    /// `read_data_skeleton` (def table + variable tables, bodies left in the
    /// store).  The difference is the body-tree reconstruction cost; the skeleton
    /// is the variable-table-dominated cost the plan flags as the warm-load
    /// bottleneck (recommendation #3 — measure before committing to E2).
    /// Run: `cargo test --release --lib bench_read_data_breakdown -- --ignored --nocapture`.
    #[cfg(feature = "mmap")]
    #[test]
    #[ignore = "perf bench — run manually with --ignored --nocapture"]
    #[allow(clippy::cast_precision_loss)]
    fn bench_read_data_breakdown() {
        use crate::ir_store::materialize_data_at;
        use crate::store::Store;
        use std::time::Instant;

        let path = std::env::temp_dir().join(format!("loft_brk_{}.store", std::process::id()));
        let path_str = path.to_str().unwrap();
        let _ = std::fs::remove_file(&path);
        let (rec, pos, ndefs, nvars, nnames) = {
            let mut p = crate::parser::Parser::new();
            p.parse_dir("default", true, false).expect("parse default/");
            let fresh = p.data;
            let nd = fresh.definitions();
            let nv: usize = (0..nd)
                .map(|d| fresh.def(d).variables().count() as usize)
                .sum();
            let nn: usize = (0..nd)
                .map(|d| fresh.def(d).variables().snapshot_names().len())
                .sum();
            let fstore = Store::open(path_str);
            let mut stores = Stores::new();
            let nr = stores.adopt_store(fstore);
            let r = stores.allocations[nr as usize].claim(16);
            let root = DbRef {
                store_nr: nr,
                rec: r,
                pos: 8,
            };
            materialize_data_at(&mut stores, root, &fresh);
            (r, root.pos, nd, nv, nn)
        };

        let iters = 40;
        let median = |mut v: Vec<u128>| {
            v.sort_unstable();
            v[v.len() / 2]
        };
        let time_it = |f: &dyn Fn(&Stores, DbRef) -> Data| {
            let mut us = Vec::new();
            for _ in 0..iters {
                let fstore = Store::open(path_str);
                let mut s2 = Stores::new();
                let nr2 = s2.adopt_store(fstore);
                let root2 = DbRef {
                    store_nr: nr2,
                    rec,
                    pos,
                };
                let t = Instant::now();
                let d = f(&s2, root2);
                us.push(t.elapsed().as_micros());
                std::hint::black_box(&d);
            }
            median(us)
        };

        let full = time_it(&|s, r| read_data(s, r));
        let skel = time_it(&|s, r| read_data_skeleton(s, r));
        // Variable-table reconstruction in isolation: walk every def record and
        // rebuild just its Function (read_function), nothing else.
        let mut vt = Vec::new();
        for _ in 0..iters {
            let fstore = Store::open(path_str);
            let mut s2 = Stores::new();
            let nr2 = s2.adopt_store(fstore);
            let root2 = DbRef {
                store_nr: nr2,
                rec,
                pos,
            };
            let defs = Record::new(root2).field_recvec(ds::DATA_DEFINITIONS, ds::DEFINITION_STRIDE);
            let t = Instant::now();
            for i in 0..defs.len(&s2) {
                let f = read_function(&s2, defs.get(i, &s2), ds::DEF_VARIABLES);
                std::hint::black_box(&f);
            }
            vt.push(t.elapsed().as_micros());
        }
        let vt = median(vt);
        let _ = std::fs::remove_file(&path);
        eprintln!("\n=== read_data breakdown ({iters} iters) ===");
        eprintln!("defs {ndefs}  vars {nvars}  name-entries {nnames}");
        eprintln!("read_data (full)      : {full} us");
        eprintln!("read_data_skeleton    : {skel} us   (def table + variable tables)");
        eprintln!(
            "body-tree reconstruct : {} us   (full - skeleton)",
            full.saturating_sub(skel)
        );
        eprintln!("variable tables only  : {vt} us   (read_function over all defs)");
        eprintln!(
            "def fields (non-var)  : {} us   (skeleton - variable tables)",
            skel.saturating_sub(vt)
        );
    }

    /// @PLN11 D2a step 3 — `read_schema` is the inverse of `materialize_schema`:
    /// materialize the real stdlib's database type schema into a store, read it
    /// back, and assert the reconstructed `Vec<database::Type>` equals the
    /// original (with the derived `parents` index cleared on both — it is not
    /// stored and the load path doesn't need it).
    #[test]
    fn read_stdlib_schema_round_trips() {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let mut cold = p.database.types.clone();
        assert!(cold.len() > 50, "stdlib schema small? got {}", cold.len());

        let mut ir = Stores::new();
        let _ids = register_ir_schema(&mut ir);
        let host = Record::new(ir.database(16));
        crate::ir_store::materialize_schema(&mut ir, &host, 0, &p.database.types);

        let loaded = read_schema(&ir, host, 0);
        assert_eq!(loaded.len(), cold.len(), "type count");
        for t in &mut cold {
            t.clear_parents();
        }
        assert_eq!(loaded, cold, "database type schema round-trip mismatch");
    }

    /// @PLN11 D2a step 5 (capstone) — the warm-load bundle round-trip: save the
    /// real stdlib's `Data` **and** its database type schema to one file via
    /// `save_bundle`, load both back via `open_bundle` (mmap, no re-parse), and
    /// assert: (1) `compare_data` bit-for-bit on the `Data`, (2) the schema
    /// equals the fresh parse's (parents cleared), and (3) installing the loaded
    /// schema into a `Stores` repopulates `types` + `names`.  This is the load
    /// half of the @PLAN28 schema-rebuild wall, resolved by caching the schema.
    #[cfg(feature = "mmap")]
    #[test]
    fn bundle_save_open_round_trips_stdlib() {
        use crate::ir_schema::compare_data;
        use crate::ir_store::save_bundle;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");

        let path = std::env::temp_dir().join(format!("loft_bundle_{}.store", std::process::id()));
        let path_str = path.to_str().unwrap();
        save_bundle(&p.data, &p.database.types, path_str).expect("save_bundle");

        let (loaded_data, loaded_types) = open_bundle(path_str).expect("open_bundle");
        let _ = std::fs::remove_file(&path);

        // (1) Data round-trips bit-for-bit.
        if let Err(diff) = compare_data(&p.data, &loaded_data) {
            panic!("bundle Data diverged from fresh parse: {diff:?}");
        }
        // (2) Schema round-trips (parents are derived, cleared on both sides).
        assert_eq!(loaded_types.len(), p.database.types.len(), "type count");
        let mut cold = p.database.types.clone();
        for t in &mut cold {
            t.clear_parents();
        }
        assert_eq!(
            loaded_types, cold,
            "bundle schema diverged from fresh parse"
        );
        // (3) The loaded schema installs into a fresh Stores.
        let mut s = Stores::new();
        s.install_schema(loaded_types);
        assert_eq!(
            s.types.len(),
            p.database.types.len(),
            "installed type count"
        );
        assert!(s.has_type("text"), "base type 'text' present after install");
    }
}
