// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I63 — Store-resident IR (reader / handle / materializer)

//! @PLN11 arc B — native IR → store materializer (the write path).
//!
//! Walks a native [`crate::data::Value`] tree and writes it into store `Node`
//! records through the [`crate::data_store`] handle layer.  This is the
//! @PLAN28 `ir_schema::data_to_json` walk with its JSON sink swapped for a
//! store-writer sink — exactly the convergence the plan describes
//! (`plans/11-data-as-store` § arc B).
//!
//! Coverage: every `Node` variant whose fields are scalars + `vector<Node>`
//! children + the inlined `Block` (30 of 32 variants).  The two that reach
//! into another struct or the `TypeT` half — `Span` (`Position`), `Keys`
//! (`vector<Key>`), `FnRef` (`vector<TypeT>`) — are
//! the next increment (they need the `TypeT`/sub-struct accessors).  A
//! `Box<Value>` single child maps to a one-element `vector<Node>` field
//! (finding 2's box-of-one); a `Vec<Value>` maps to the same field with N
//! elements — both materialize identically.

use crate::data::{
    Attribute, Block, Data, DefType, Definition, ImpureCategory, LinkedFieldGroup, LinkedFieldKind,
    Purity, Type, Value,
};
use crate::data_store::{self as ds, RecVector, Record, Value as Node, ValuesVector};
use crate::database::{Field as SchemaField, Parts, Stores, Type as SchemaType};
use crate::keys::{Content, DbRef};
use crate::variables::{Function, VarSnapshot};

/// Materialize one native `Value` into a freshly-appended slot of `dst`
/// (a `vector<Node>`), recursing into any child vectors.
pub fn materialize_node(stores: &mut Stores, dst: ValuesVector, v: &Value) {
    let slot = dst.push(stores);
    write_into(stores, &slot, v);
}

/// Materialize one native `Type` into a freshly-appended slot of `dst`
/// (a `vector<TypeT>`), recursing into any child type vectors.
pub fn materialize_type(stores: &mut Stores, dst: RecVector, ty: &Type) {
    let slot = dst.push(stores);
    write_type(stores, &slot, ty);
}

/// Materialize every element of `items` into the `vector<Node>` `dst`.
fn push_all(stores: &mut Stores, dst: ValuesVector, items: &[Value]) {
    for it in items {
        materialize_node(stores, dst, it);
    }
}

/// Write a `Box<Type>` single child as a one-element `vector<TypeT>` at `off`
/// (box-of-one).
fn type_child(stores: &mut Stores, slot: &Record, off: u32, ty: &Type) {
    materialize_type(stores, slot.field_recvec(off, ds::TYPET_STRIDE), ty);
}

/// Write a `Vec<Type>` as N elements of the `vector<TypeT>` at `off`.
fn type_list(stores: &mut Stores, slot: &Record, off: u32, tys: &[Type]) {
    let v = slot.field_recvec(off, ds::TYPET_STRIDE);
    for t in tys {
        materialize_type(stores, v, t);
    }
}

/// Write a `Vec<u16>` dep list as a `vector<integer>` at `off`.
fn dep_list(stores: &mut Stores, slot: &Record, off: u32, dep: &[u16]) {
    let v = slot.field_recvec(off, ds::INT_STRIDE);
    for d in dep {
        v.push(stores).set_field_int(stores, 0, i64::from(*d));
    }
}

/// Write a `Vec<(String, bool)>` key list as a `vector<SortKey>` at `off`.
fn sort_key_list(stores: &mut Stores, slot: &Record, off: u32, keys: &[(String, bool)]) {
    let v = slot.field_recvec(off, ds::SORTKEY_STRIDE);
    for (name, asc) in keys {
        let e = v.push(stores);
        e.set_field_str(stores, ds::SORTKEY_NAME, name);
        e.set_field_bool(stores, ds::SORTKEY_ASC, *asc);
    }
}

/// Write a `Vec<String>` name list as a `vector<NameRef>` at `off`.
fn name_list(stores: &mut Stores, slot: &Record, off: u32, names: &[String]) {
    let v = slot.field_recvec(off, ds::NAMEREF_STRIDE);
    for name in names {
        v.push(stores).set_field_str(stores, ds::NAMEREF_NAME, name);
    }
}

/// Write `ty` into the already-allocated `TypeT` record `slot` (zeroed).
fn write_type(stores: &mut Stores, slot: &Record, ty: &Type) {
    match ty {
        // @PLN25 — the `τ?` nullability marker persists as its own variant (a
        // box-of-one child, like RefVar) so a nullable type survives the store
        // round-trip and keeps its N-Store contract. The runtime LAYOUT is still
        // the base's sentinel storage (peeled at every layout site); only the codec
        // keeps the marker.
        Type::Optional(inner) => {
            slot.set_discriminant(stores, ds::TY_OPTIONAL);
            type_child(stores, slot, ds::TYOPTIONAL_INNER, inner);
        }
        Type::Unknown(n) => {
            slot.set_discriminant(stores, ds::TY_UNKNOWN);
            slot.set_field_int(stores, ds::TYUNKNOWN_N, i64::from(*n));
        }
        Type::Null => slot.set_discriminant(stores, ds::TY_NULL),
        Type::Void => slot.set_discriminant(stores, ds::TY_VOID),
        Type::Never => slot.set_discriminant(stores, ds::TY_NEVER),
        Type::Integer(spec) => {
            slot.set_discriminant(stores, ds::TY_INTEGER);
            slot.set_field_int(stores, ds::TYINTEGER_MIN, i64::from(spec.min));
            slot.set_field_int(stores, ds::TYINTEGER_MAX, i64::from(spec.max));
            slot.set_field_bool(stores, ds::TYINTEGER_NOT_NULL, spec.not_null);
            // Option<NonZeroU8> -> sentinel 0 = None.
            let fs = spec.forced_size.map_or(0, |n| i64::from(n.get()));
            slot.set_field_int(stores, ds::TYINTEGER_FORCED, fs);
        }
        Type::Boolean => slot.set_discriminant(stores, ds::TY_BOOLEAN),
        Type::Float => slot.set_discriminant(stores, ds::TY_FLOAT),
        Type::Single => slot.set_discriminant(stores, ds::TY_SINGLE),
        Type::Character => slot.set_discriminant(stores, ds::TY_CHARACTER),
        Type::Text(dep) => {
            slot.set_discriminant(stores, ds::TY_TEXT);
            dep_list(stores, slot, ds::TYTEXT_DEP, dep);
        }
        Type::Keys => slot.set_discriminant(stores, ds::TY_KEYS),
        Type::Enum(n, is_ref, dep) => {
            slot.set_discriminant(stores, ds::TY_ENUM);
            slot.set_field_int(stores, ds::TYENUM_N, i64::from(*n));
            slot.set_field_bool(stores, ds::TYENUM_IS_REF, *is_ref);
            dep_list(stores, slot, ds::TYENUM_DEP, dep);
        }
        Type::Reference(n, dep) => {
            slot.set_discriminant(stores, ds::TY_REFERENCE);
            slot.set_field_int(stores, ds::TYREF_N, i64::from(*n));
            dep_list(stores, slot, ds::TYREF_DEP, dep);
        }
        Type::RefVar(inner) => {
            slot.set_discriminant(stores, ds::TY_REF_VAR);
            type_child(stores, slot, ds::TYREFVAR_INNER, inner);
        }
        Type::Vector(inner, dep) => {
            slot.set_discriminant(stores, ds::TY_VECTOR);
            type_child(stores, slot, ds::TYVECTOR_INNER, inner);
            dep_list(stores, slot, ds::TYVECTOR_DEP, dep);
        }
        Type::Routine(n) => {
            slot.set_discriminant(stores, ds::TY_ROUTINE);
            slot.set_field_int(stores, ds::TYROUTINE_N, i64::from(*n));
        }
        Type::Iterator(step, inner) => {
            slot.set_discriminant(stores, ds::TY_ITERATOR);
            type_child(stores, slot, ds::TYITER_STEP, step);
            type_child(stores, slot, ds::TYITER_INNER, inner);
        }
        Type::Sorted(n, keys, dep) => {
            slot.set_discriminant(stores, ds::TY_SORTED);
            slot.set_field_int(stores, ds::TYSORTED_N, i64::from(*n));
            sort_key_list(stores, slot, ds::TYSORTED_KEYS, keys);
            dep_list(stores, slot, ds::TYSORTED_DEP, dep);
        }
        Type::Index(n, keys, dep) => {
            slot.set_discriminant(stores, ds::TY_INDEX);
            slot.set_field_int(stores, ds::TYINDEX_N, i64::from(*n));
            sort_key_list(stores, slot, ds::TYINDEX_KEYS, keys);
            dep_list(stores, slot, ds::TYINDEX_DEP, dep);
        }
        Type::Trie(n, key, dep) => {
            slot.set_discriminant(stores, ds::TY_TRIE);
            slot.set_field_int(stores, ds::TYTRIE_N, i64::from(*n));
            slot.set_field_str(stores, ds::TYTRIE_KEY, key);
            dep_list(stores, slot, ds::TYTRIE_DEP, dep);
        }
        Type::Radix(n, names, dep) => {
            slot.set_discriminant(stores, ds::TY_RADIX);
            slot.set_field_int(stores, ds::TYRADIX_N, i64::from(*n));
            name_list(stores, slot, ds::TYRADIX_NAMES, names);
            dep_list(stores, slot, ds::TYRADIX_DEP, dep);
        }
        Type::Hash(n, names, dep) => {
            slot.set_discriminant(stores, ds::TY_HASH);
            slot.set_field_int(stores, ds::TYHASH_N, i64::from(*n));
            name_list(stores, slot, ds::TYHASH_NAMES, names);
            dep_list(stores, slot, ds::TYHASH_DEP, dep);
        }
        Type::Function(args, result, dep) => {
            slot.set_discriminant(stores, ds::TY_FUNCTION);
            type_list(stores, slot, ds::TYFUNC_ARGS, args);
            type_child(stores, slot, ds::TYFUNC_RESULT, result);
            dep_list(stores, slot, ds::TYFUNC_DEP, dep);
        }
        Type::Rewritten(inner) => {
            slot.set_discriminant(stores, ds::TY_REWRITTEN);
            type_child(stores, slot, ds::TYREWRITTEN_INNER, inner);
        }
        Type::Tuple(elems) => {
            slot.set_discriminant(stores, ds::TY_TUPLE);
            type_list(stores, slot, ds::TYTUPLE_ELEMS, elems);
        }
    }
}

/// Write a `Box<Value>`/`Value` single child as a one-element `vector<Node>` at
/// `off` of a non-`Node` `slot` (box-of-one).
fn node_child(stores: &mut Stores, slot: &Record, off: u32, v: &Value) {
    materialize_node(stores, slot.field_vec(off), v);
}

/// Write a native `Block` into `slot` under discriminant `disc` (`NdBlock` or
/// `NdLoop` — identical inlined-`Block` layout).  Fields are at the `Block`
/// base (`NDBLOCK_BLOCK`) plus their relative offsets.
fn write_block(stores: &mut Stores, slot: &Node, disc: u8, b: &Block) {
    // The block is a box-of-one sub-record now, so the write pushes it once and
    // fills it directly — the fields are its own, not the Node's plus a base.
    let rec = if disc == ds::DISC_LOOP {
        slot.write_loop(stores, b.name)
    } else {
        slot.write_block(stores, b.name)
    };
    push_all(stores, rec.field_vec(ds::BLOCK_OPERATORS), &b.operators);
    rec.set_field_int(stores, ds::BLOCK_SCOPE, i64::from(b.scope));
    rec.set_field_int(stores, ds::BLOCK_VAR_SIZE, i64::from(b.var_size));
    // result: single Type -> one-element vector<TypeT> (box-of-one).
    materialize_type(
        stores,
        rec.field_recvec(ds::BLOCK_RESULT, ds::TYPET_STRIDE),
        &b.result,
    );
}

/// Write one native `Attribute` into the already-allocated record `r`.
fn write_attribute(stores: &mut Stores, r: &Record, a: &Attribute) {
    r.set_field_str(stores, ds::ATTR_NAME, &a.name);
    type_child(stores, r, ds::ATTR_TYPEDEF, &a.typedef);
    r.set_field_bool(stores, ds::ATTR_MUTABLE, a.mutable);
    r.set_field_bool(stores, ds::ATTR_CONSTANT, a.constant);
    r.set_field_bool(stores, ds::ATTR_INIT, a.init);
    r.set_field_bool(stores, ds::ATTR_NULLABLE, a.nullable);
    r.set_field_bool(stores, ds::ATTR_PRIMARY, a.primary);
    r.set_field_bool(stores, ds::ATTR_HIDDEN, a.hidden);
    r.set_field_bool(stores, ds::ATTR_CONST_FIELD, a.const_field);
    node_child(stores, r, ds::ATTR_VALUE, &a.value);
    node_child(stores, r, ds::ATTR_CHECK, &a.check);
    node_child(stores, r, ds::ATTR_CHECK_MESSAGE, &a.check_message);
    r.set_field_int(stores, ds::ATTR_ALIAS_D_NR, i64::from(a.alias_d_nr));
    r.set_field_int(
        stores,
        ds::ATTR_ASSIGNED_LAMBDA_D_NR,
        i64::from(a.assigned_lambda_d_nr),
    );
    // @PLN86 F8b — the group#right member links, joined by spaces (tokens contain no
    // whitespace), so a warm-cached host type keeps its capability links.
    r.set_field_str(stores, ds::ATTR_LINKS, &a.links.join(" "));
}

/// Materialize a `Vec<Attribute>` into the `vector<Attribute>` field at `off`
/// of the (non-`Node`) record `parent`.
pub fn materialize_attributes(stores: &mut Stores, parent: &Record, off: u32, attrs: &[Attribute]) {
    let v = parent.field_recvec(off, ds::ATTRIBUTE_STRIDE);
    for a in attrs {
        let r = v.push(stores);
        write_attribute(stores, &r, a);
    }
}

/// Write one native `LinkedFieldGroup` into the already-allocated record `r`.
/// `kind` stores the native `LinkedFieldKind` discriminant (`Tuple` 0/`Index` 1).
fn write_field_group(stores: &mut Stores, r: &Record, g: &LinkedFieldGroup) {
    let kind = match g.kind {
        LinkedFieldKind::Tuple => 0,
        LinkedFieldKind::Index => 1,
    };
    r.set_field_int(stores, ds::LFG_KIND, kind);
    r.set_field_int(stores, ds::LFG_INSTANCE, i64::from(g.instance));
    dep_list(stores, r, ds::LFG_FIELD_INDICES, &g.field_indices);
    r.set_field_int(stores, ds::LFG_ALIGNMENT, i64::from(g.alignment));
    r.set_field_int(stores, ds::LFG_SIZE, i64::from(g.size));
}

/// Materialize a `Vec<LinkedFieldGroup>` into the `vector<LinkedFieldGroup>`
/// field at `off` of the (non-`Node`) record `parent`.
pub fn materialize_field_groups(
    stores: &mut Stores,
    parent: &Record,
    off: u32,
    groups: &[LinkedFieldGroup],
) {
    let v = parent.field_recvec(off, ds::LFG_STRIDE);
    for g in groups {
        let r = v.push(stores);
        write_field_group(stores, &r, g);
    }
}

/// Write a `Vec<u32>` int list as a `vector<integer>` at `off`.
fn int_list_u32(stores: &mut Stores, slot: &Record, off: u32, vals: &[u32]) {
    let v = slot.field_recvec(off, ds::INT_STRIDE);
    for n in vals {
        v.push(stores).set_field_int(stores, 0, i64::from(*n));
    }
}

/// `DefType` → its stored integer code (the native discriminant order).
fn def_type_code(t: &DefType) -> i64 {
    match t {
        DefType::Unknown => 0,
        DefType::Function => 1,
        DefType::Dynamic => 2,
        DefType::Enum => 3,
        DefType::EnumValue => 4,
        DefType::Struct => 5,
        DefType::Vector => 6,
        DefType::Type => 7,
        DefType::Constant => 8,
        DefType::Generic => 9,
        DefType::Interface => 10,
    }
}

/// `Purity` → a lossless integer code: `Unknown` 0, `Pure` 1, `Impure(cat)`
/// `2 + cat` where `cat` is the `ImpureCategory` discriminant.
fn purity_code(p: Purity) -> i64 {
    match p {
        Purity::Unknown => 0,
        Purity::Pure => 1,
        Purity::Impure(c) => {
            2 + match c {
                ImpureCategory::HostIo => 0,
                ImpureCategory::Prng => 1,
                ImpureCategory::Io => 2,
                ImpureCategory::ParentWrite => 3,
                ImpureCategory::ParCall => 4,
            }
        }
    }
}

/// Write the ten codegen-read fields of one `Variable` (a `VarSnapshot`) into
/// the already-allocated record `r`.
fn write_var_snapshot(stores: &mut Stores, r: &Record, v: &VarSnapshot) {
    r.set_field_str(stores, ds::VAR_NAME, v.name);
    type_child(stores, r, ds::VAR_TYPE_DEF, v.type_def);
    r.set_field_int(stores, ds::VAR_STACK_POS, i64::from(v.stack_pos));
    r.set_field_int(stores, ds::VAR_USES, i64::from(v.uses));
    r.set_field_bool(stores, ds::VAR_ARGUMENT, v.argument);
    r.set_field_bool(stores, ds::VAR_STACK_ALLOCATED, v.stack_allocated);
    r.set_field_bool(stores, ds::VAR_SKIP_FREE, v.skip_free);
    r.set_field_bool(stores, ds::VAR_CAPTURED, v.captured);
    r.set_field_bool(stores, ds::VAR_CALLER_HIDDEN_BUF, v.caller_hidden_buf);
    r.set_field_int(stores, ds::VAR_OWNER_WITNESS, i64::from(v.owner_witness));
}

/// Write a native `Function` inline into `parent` at `base`: `name` / `file`,
/// the `vector<Variable>` debug-symbol table, the `names` map (name → var_nr),
/// and the `inline_ref_vars` set — all via the `variables/mod.rs` snapshot seam.
/// `names` and `inline_refs` are stored (not reconstructible from the variable
/// list: `names` is pruned on scope exit, `inline_ref_vars` is compile-derived)
/// and are read back by codegen on the load path.
fn write_function(stores: &mut Stores, parent: &Record, base: u32, f: &Function) {
    parent.set_field_str(stores, base + ds::FN_NAME, &f.name);
    parent.set_field_str(stores, base + ds::FN_FILE, &f.file);
    let vars = parent.field_recvec(base + ds::FN_VARIABLES, ds::VARIABLE_STRIDE);
    for i in 0..f.snapshot_len() {
        let v = f.snapshot_var(i);
        let r = vars.push(stores);
        write_var_snapshot(stores, &r, &v);
    }
    // names: name -> var_nr.  Store order is irrelevant — the reader collects
    // into a HashMap and the JSON oracle re-sorts on emit.
    let names = parent.field_recvec(base + ds::FN_NAMES, ds::NAMENR_STRIDE);
    for (name, nr) in f.snapshot_names() {
        let e = names.push(stores);
        e.set_field_int(stores, ds::NAMENR_NR, i64::from(nr));
        e.set_field_str(stores, ds::NAMENR_NAME, name);
    }
    // inline_ref_vars: the compile-derived set, as a vector<integer>.
    let refs = parent.field_recvec(base + ds::FN_INLINE_REFS, ds::INT_STRIDE);
    for v in f.snapshot_inline_refs() {
        refs.push(stores).set_field_int(stores, 0, i64::from(v));
    }
}

/// Write one native `Definition` into the already-allocated record `r` (the
/// 23-field store subset — the same set the @PLAN28 codec serialises).
fn write_definition(stores: &mut Stores, r: &Record, d: &Definition) {
    r.set_field_str(stores, ds::DEF_NAME, &d.name);
    r.set_field_int(stores, ds::DEF_SOURCE, i64::from(d.source));
    r.set_field_int(stores, ds::DEF_DEF_TYPE, def_type_code(&d.def_type));
    r.set_field_int(stores, ds::DEF_PARENT, i64::from(d.parent));
    // position (inlined): line / pos / file at DEF_POSITION + relative.
    r.set_field_int(
        stores,
        ds::DEF_POSITION + ds::POS_LINE,
        i64::from(d.position.line),
    );
    r.set_field_int(
        stores,
        ds::DEF_POSITION + ds::POS_POS,
        i64::from(d.position.pos),
    );
    r.set_field_str(stores, ds::DEF_POSITION + ds::POS_FILE, &d.position.file);
    materialize_attributes(stores, r, ds::DEF_ATTRIBUTES, &d.attributes);
    node_child(stores, r, ds::DEF_CODE, &d.code);
    type_child(stores, r, ds::DEF_RETURNED, &d.returned);
    r.set_field_bool(stores, ds::DEF_RETURNED_NOT_NULL, d.returned_not_null);
    r.set_field_str(stores, ds::DEF_RUST, &d.rust);
    r.set_field_str(stores, ds::DEF_NATIVE, &d.native);
    r.set_field_str(stores, ds::DEF_CAP, &d.cap); // @PLN86
    r.set_field_str(stores, ds::DEF_C_SYMBOL, &d.c_symbol); // @PLN24
    r.set_field_str(stores, ds::DEF_C_SIG, &d.c_sig); // @PLN24
    r.set_field_str(stores, ds::DEF_SUPERSEDED, &d.superseded); // @PLN102 arc C
    r.set_field_int(stores, ds::DEF_OP_CODE, i64::from(d.op_code));
    r.set_field_int(stores, ds::DEF_KNOWN_TYPE, i64::from(d.known_type));
    write_function(stores, r, ds::DEF_VARIABLES, &d.variables);
    r.set_field_bool(stores, ds::DEF_PUB_VISIBLE, d.pub_visible);
    r.set_field_bool(stores, ds::DEF_NULL_SAFE, d.null_safe); // @PLN46 W2
    r.set_field_int(stores, ds::DEF_CLOSURE_RECORD, i64::from(d.closure_record));
    name_list(stores, r, ds::DEF_MUTATED_CAPTURES, &d.mutated_captures);
    name_list(stores, r, ds::DEF_SCALARS_TO_BOX, &d.scalars_to_box);
    int_list_u32(stores, r, ds::DEF_BOUNDS, &d.bounds);
    // forced_size: Option<u8> -> 0 = None.
    r.set_field_int(
        stores,
        ds::DEF_FORCED_SIZE,
        i64::from(d.forced_size.unwrap_or(0)),
    );
    r.set_field_int(stores, ds::DEF_PURITY, purity_code(d.purity));
    materialize_field_groups(stores, r, ds::DEF_FIELD_GROUPS, &d.field_groups);
    // synthetic: Option<&str> -> "" = None.
    r.set_field_str(stores, ds::DEF_SYNTHETIC, d.synthetic.unwrap_or(""));
}

/// Materialize a whole native `Data` into a fresh `Data` store record and
/// return its `DbRef` (the root).  Arc B's capstone entry point.
pub fn materialize_data(stores: &mut Stores, data: &Data) -> DbRef {
    let root = stores.database(ds::DATA_ROOT_WORDS);
    materialize_data_at(stores, root, data);
    root
}

/// Materialize a whole native `Data` into a caller-provided root `Data` record.
///
/// Same as [`materialize_data`] but writes into an already-claimed `root`
/// (16-word `Data` record) instead of allocating a fresh in-memory store —
/// e.g. a record claimed in a **file-backed** store (`Store::open`), so the
/// materialised IR lands directly on disk for the mmap load path (@PLN11 arc
/// D).  The whole IR (records, inline vectors, and interned strings) lives in
/// `root`'s store, so persisting/mmapping that one store captures everything.
///
/// `source`, `definitions` and the two import tables (`imports`, `use_names`)
/// are stored; every other index rebuilds from those on load.  Manifest-derived parse state (`native_lib_regs`,
/// the `wasm_bridge_*` tables) is NOT in the bundle — the whole-program cache
/// carries it on the side via the drift-manifest's `nlib`/`wbroute`/`wbpkg`/
/// `wbhostjs` lines and replays it in `startup_cache::warm_load_program` (#310,
/// #444).  Do not add it here: that would double-restore.
pub fn materialize_data_at(stores: &mut Stores, root: DbRef, data: &Data) {
    // Clear the root's Data fields (claim/database do not zero) so the
    // `definitions` vector header starts empty.
    stores
        .store_mut(&root)
        .zero_range(root.rec, root.pos, ds::DATA_STRIDE);
    let r = Record::new(root);
    r.set_field_int(stores, ds::DATA_SOURCE, i64::from(data.source));
    let defs = r.field_recvec(ds::DATA_DEFINITIONS, ds::DEFINITION_STRIDE);
    for d in &data.definitions {
        let dr = defs.push(stores);
        write_definition(stores, &dr, d);
    }
    // The import tables: `rebuild_indices` reconstructs `def_names` under each
    // definition's OWN source and then REPLAYS these, so an image without them
    // loads with every `use lib::*` binding missing (loft#1359).
    let imports = r.field_recvec(ds::DATA_IMPORTS, ds::IMPORT_STRIDE);
    for imp in data.applied_imports() {
        let ir = imports.push(stores);
        ir.set_field_int(stores, ds::IMPORT_LIB_SOURCE, i64::from(imp.lib_source));
        ir.set_field_int(stores, ds::IMPORT_INTO_SOURCE, i64::from(imp.into_source));
        // A wildcard import stores an empty name; a selective one never has one.
        let (name, bind) = imp
            .name
            .as_ref()
            .map_or(("", ""), |(n, b)| (n.as_str(), b.as_str()));
        ir.set_field_str(stores, ds::IMPORT_NAME, name);
        ir.set_field_str(stores, ds::IMPORT_BIND, bind);
    }
    let uses = r.field_recvec(ds::DATA_USE_NAMES, ds::USENAME_STRIDE);
    for (name, source) in data.use_name_pairs() {
        let ur = uses.push(stores);
        ur.set_field_str(stores, ds::USENAME_NAME, &name);
        ur.set_field_int(stores, ds::USENAME_SOURCE, i64::from(source));
    }
}

// ─── Database type schema materializer (D2a) ─────────────────────────────────

/// Materialize the database type schema (`Stores.types`) into the
/// `vector<DbType>` field at `off` of `parent` (@PLN11 arc D / D2a) so a
/// cache-loaded `Data`'s baked `known_type`s stay valid with no parse-time
/// `fill_all` rebuild.  `Type.parents` is a derived back-reference index
/// (rebuilt on load) and is **not** written.
pub fn materialize_schema(stores: &mut Stores, parent: &Record, off: u32, schema: &[SchemaType]) {
    let v = parent.field_recvec(off, ds::DBTYPE_STRIDE);
    for t in schema {
        let r = v.push(stores);
        write_db_type(stores, &r, t);
    }
}

/// Write one native `database::Type` into the already-allocated `DbType` record.
fn write_db_type(stores: &mut Stores, r: &Record, t: &SchemaType) {
    r.set_field_str(stores, ds::DBTYPE_NAME, &t.name);
    r.set_field_int(stores, ds::DBTYPE_SIZE, i64::from(t.size_bytes()));
    r.set_field_int(stores, ds::DBTYPE_ALIGN, i64::from(t.align_bytes()));
    r.set_field_bool(stores, ds::DBTYPE_COMPLEX, t.is_complex());
    r.set_field_bool(stores, ds::DBTYPE_LINKED, t.is_linked_flag());
    // parts: single Parts -> one-element vector<DbParts> (box-of-one).
    let pr = r
        .field_recvec(ds::DBTYPE_PARTS, ds::DBPARTS_STRIDE)
        .push(stores);
    write_db_parts(stores, &pr, &t.parts);
    // keys: vector<Key> (same record as the IR's NdKeys element).
    let kv = r.field_recvec(ds::DBTYPE_KEYS, ds::KEY_STRIDE);
    for k in &t.keys {
        let e = kv.push(stores);
        e.set_field_int(stores, ds::KEY_TYPE_NR, i64::from(k.type_nr));
        e.set_field_int(stores, ds::KEY_POSITION, i64::from(k.position));
        e.set_field_int(stores, ds::KEY_START, i64::from(k.start));
    }
    materialize_field_groups(stores, r, ds::DBTYPE_FIELD_GROUPS, &t.field_groups);
}

/// Write one native `database::Parts` into the already-allocated `DbParts` record.
fn write_db_parts(stores: &mut Stores, r: &Record, parts: &Parts) {
    match parts {
        Parts::Base => r.set_discriminant(stores, ds::PT_BASE),
        Parts::Struct(fields) => {
            r.set_discriminant(stores, ds::PT_STRUCT);
            write_db_fields(stores, r, ds::PTSTRUCT_FIELDS, fields);
        }
        Parts::Enum(values) => {
            r.set_discriminant(stores, ds::PT_ENUM);
            let v = r.field_recvec(ds::PTENUM_VALUES, ds::ENUMPAIR_STRIDE);
            for (nr, name) in values {
                let e = v.push(stores);
                e.set_field_int(stores, ds::ENUMPAIR_NR, i64::from(*nr));
                e.set_field_str(stores, ds::ENUMPAIR_NAME, name);
            }
        }
        Parts::EnumValue(value, fields) => {
            r.set_discriminant(stores, ds::PT_ENUM_VALUE);
            r.set_field_int(stores, ds::PTENUMVALUE_VALUE, i64::from(*value));
            write_db_fields(stores, r, ds::PTENUMVALUE_FIELDS, fields);
        }
        Parts::Byte(start, nullable) => write_pt_num(stores, r, ds::PT_BYTE, *start, *nullable),
        Parts::Short(start, nullable) => write_pt_num(stores, r, ds::PT_SHORT, *start, *nullable),
        Parts::Int(start, nullable) => write_pt_num(stores, r, ds::PT_INT, *start, *nullable),
        Parts::IntRaw(start, nullable) => {
            write_pt_num(stores, r, ds::PT_INT_RAW, *start, *nullable);
        }
        Parts::ShortRaw(start, nullable) => {
            write_pt_num(stores, r, ds::PT_SHORT_RAW, *start, *nullable);
        }
        Parts::Vector(c) => write_pt_content(stores, r, ds::PT_VECTOR, *c),
        Parts::Array(c) => write_pt_content(stores, r, ds::PT_ARRAY, *c),
        Parts::Sorted(c, keys) => {
            r.set_discriminant(stores, ds::PT_SORTED);
            r.set_field_int(stores, ds::PTCONTENT, i64::from(*c));
            write_key_fields(stores, r, ds::PTKEYS, keys);
        }
        Parts::Ordered(c, keys) => {
            r.set_discriminant(stores, ds::PT_ORDERED);
            r.set_field_int(stores, ds::PTCONTENT, i64::from(*c));
            write_key_fields(stores, r, ds::PTKEYS, keys);
        }
        Parts::Hash(c, fields) => {
            r.set_discriminant(stores, ds::PT_HASH);
            r.set_field_int(stores, ds::PTCONTENT, i64::from(*c));
            dep_list(stores, r, ds::PTFIELDS, fields);
        }
        Parts::Index(c, keys, left) => {
            r.set_discriminant(stores, ds::PT_INDEX);
            r.set_field_int(stores, ds::PTCONTENT, i64::from(*c));
            write_key_fields(stores, r, ds::PTKEYS, keys);
            r.set_field_int(stores, ds::PTINDEX_LEFT, i64::from(*left));
        }
        // The IR codec needs its own `PT_TRIE` discriminant — a SCHEMA change
        // (`data_store.rs` pins each id and `ir_schema_roundtrip` guards the drift).
        // Deferred to step 3 for the same reason as the descriptor: it should land with
        // a test that can construct a trie.
        Parts::Trie(c, key) => {
            r.set_discriminant(stores, ds::PT_TRIE);
            r.set_field_int(stores, ds::PTCONTENT, i64::from(*c));
            r.set_field_int(stores, ds::PTTRIE_KEY, i64::from(*key));
        }
        Parts::Radix(c, fields) => {
            r.set_discriminant(stores, ds::PT_RADIX);
            r.set_field_int(stores, ds::PTCONTENT, i64::from(*c));
            dep_list(stores, r, ds::PTFIELDS, fields);
        }
        Parts::DbRef => r.set_discriminant(stores, ds::PT_DB_REF),
        Parts::ChildRec(c) => write_pt_content(stores, r, ds::PT_CHILD_REC, *c),
    }
}

/// A narrow-integer `Parts` variant (`Byte`/`Short`/`Int`/`ShortRaw`): `(i32, bool)`.
fn write_pt_num(stores: &mut Stores, r: &Record, disc: u8, start: i32, nullable: bool) {
    r.set_discriminant(stores, disc);
    r.set_field_int(stores, ds::PTNUM_START, i64::from(start));
    r.set_field_bool(stores, ds::PTNUM_NULLABLE, nullable);
}

/// A content-only `Parts` variant (`Vector`/`Array`/`ChildRec`): one `u16` type-nr.
fn write_pt_content(stores: &mut Stores, r: &Record, disc: u8, content: u16) {
    r.set_discriminant(stores, disc);
    r.set_field_int(stores, ds::PTCONTENT, i64::from(content));
}

/// Write a `Vec<Field>` into the `vector<DbField>` at `off` of record `parent`.
fn write_db_fields(stores: &mut Stores, parent: &Record, off: u32, fields: &[SchemaField]) {
    let v = parent.field_recvec(off, ds::DBFIELD_STRIDE);
    for f in fields {
        let r = v.push(stores);
        r.set_field_str(stores, ds::DBFIELD_NAME, &f.name);
        r.set_field_int(stores, ds::DBFIELD_CONTENT, i64::from(f.content));
        r.set_field_int(stores, ds::DBFIELD_POSITION, i64::from(f.position));
        // @PLN127 arc D — carried through the round trip, or a schema read back
        // from a store would answer "not nullable" for every field.
        r.set_field_bool(stores, ds::DBFIELD_NULLABLE, f.nullable);
        // default: single Content -> one-element vector<DbContent> (box-of-one).
        let dr = r
            .field_recvec(ds::DBFIELD_DEFAULT, ds::DBCONTENT_STRIDE)
            .push(stores);
        write_db_content(
            stores,
            &dr,
            &crate::database::Field::default_to_wire(f.default.as_ref()),
        );
        dep_list(stores, &r, ds::DBFIELD_OTHER_INDEXES, f.other_indexes());
    }
}

/// Write one native `keys::Content` into the already-allocated `DbContent` record.
fn write_db_content(stores: &mut Stores, r: &Record, c: &Content) {
    match c {
        Content::Long(n) => {
            r.set_discriminant(stores, ds::DC_LONG);
            r.set_field_int(stores, ds::DCLONG_V, *n);
        }
        Content::Float(f) => {
            r.set_discriminant(stores, ds::DC_FLOAT);
            r.set_field_float(stores, ds::DCFLOAT_V, *f);
        }
        Content::Single(f) => {
            r.set_discriminant(stores, ds::DC_SINGLE);
            r.set_field_single(stores, ds::DCSINGLE_V, *f);
        }
        Content::Str(s) => {
            r.set_discriminant(stores, ds::DC_STR);
            r.set_field_str(stores, ds::DCSTR_V, s.str());
        }
    }
}

/// Write a `Vec<(u16, bool)>` key list into the `vector<KeyField>` at `off`.
fn write_key_fields(stores: &mut Stores, parent: &Record, off: u32, keys: &[(u16, bool)]) {
    let v = parent.field_recvec(off, ds::KEYFIELD_STRIDE);
    for (nr, asc) in keys {
        let e = v.push(stores);
        e.set_field_int(stores, ds::KEYFIELD_NR, i64::from(*nr));
        e.set_field_bool(stores, ds::KEYFIELD_ASC, *asc);
    }
}

/// Serialize `data` to a file-backed IR store at `path` (@PLN11 arc D).
///
/// Materializes the whole `Data` into a fresh file-backed store (`Store::open`),
/// with the root claimed as record [`ds::IR_ROOT_REC`] so [`crate::ir_read::open_data`]
/// can find it with no sidecar.  Dropping the store unmaps + flushes it to disk.
/// The file is removed first so the root lands at the well-known first record.
///
/// # Errors
/// Propagates a file-remove error other than "not found"; `Store::open` itself
/// panics on an unopenable path (consistent with the rest of the store layer).
///
/// # Panics
/// If the materialized root does not land at [`ds::IR_ROOT_REC`] (would mean the
/// store was not fresh) — a guard so `open_data` never reads from the wrong record.
#[cfg(feature = "mmap")]
pub fn save_data(data: &Data, path: &str) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let fstore = crate::store::Store::open(path);
    let mut stores = Stores::new();
    let nr = stores.adopt_store(fstore);
    let rec = stores.allocations[nr as usize].claim(ds::DATA_ROOT_WORDS);
    assert_eq!(
        rec,
        ds::IR_ROOT_REC,
        "IR store root must be the first record of a fresh store"
    );
    let root = DbRef {
        store_nr: nr,
        rec,
        pos: ds::IR_ROOT_POS,
    };
    materialize_data_at(&mut stores, root, data);
    // Drop `stores` → the file-backed Store is unmapped and flushed to `path`.
    drop(stores);
    Ok(())
}

/// Materialize a whole **bundle** — the IR `data` plus its database type
/// `schema` (`Stores.types`) — into a caller-provided `Bundle` root record
/// (@PLN11 D2a step 4).  `Data` is inlined at `BUNDLE_DATA` (offset 0, so the
/// `Data` writer's offsets land correctly); the schema vector goes at
/// `BUNDLE_TYPES`.  The whole root is zeroed HERE: `materialize_data_at` clears
/// only its own `DATA_STRIDE` bytes, and the schema vector's header sits past
/// them, so an uncleared header is a junk record id on the first push.
pub fn materialize_bundle(stores: &mut Stores, root: DbRef, data: &Data, schema: &[SchemaType]) {
    stores
        .store_mut(&root)
        .zero_range(root.rec, root.pos, ds::BUNDLE_STRIDE);
    let data_root = DbRef {
        pos: root.pos + ds::BUNDLE_DATA,
        ..root
    };
    materialize_data_at(stores, data_root, data);
    materialize_schema(stores, &Record::new(root), ds::BUNDLE_TYPES, schema);
}

/// Serialize a bundle (`data` + database type `schema`) to a file-backed store
/// at `path` (@PLN11 D2a step 4) — the cacheable artifact a warm startup loads
/// via [`crate::ir_read::open_bundle`].  Same fresh-store / well-known-root
/// discipline as [`save_data`].
///
/// Written **atomically** (@PLN11 arc E): materialized into a `<path>.<pid>.tmp`
/// file-backed store, then `rename`d onto `path`, so a process killed mid-write
/// can never leave a half-written bundle at `path` for the next run to load.
///
/// # Errors
/// Propagates a file-remove or `rename` error.
///
/// # Panics
/// If the materialized root does not land at [`ds::IR_ROOT_REC`].
#[cfg(feature = "mmap")]
pub fn save_bundle(data: &Data, schema: &[SchemaType], path: &str) -> std::io::Result<()> {
    let tmp = format!("{path}.{}.tmp", std::process::id());
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    {
        let fstore = crate::store::Store::open(&tmp);
        let mut stores = Stores::new();
        let nr = stores.adopt_store(fstore);
        let rec = stores.allocations[nr as usize].claim(ds::BUNDLE_ROOT_WORDS);
        assert_eq!(
            rec,
            ds::IR_ROOT_REC,
            "bundle root must be the first record of a fresh store"
        );
        let root = DbRef {
            store_nr: nr,
            rec,
            pos: ds::IR_ROOT_POS,
        };
        materialize_bundle(&mut stores, root, data, schema);
        // Drop `stores` → the file-backed Store unmaps + flushes `tmp` fully
        // before the atomic rename publishes it at `path`.
    }
    std::fs::rename(&tmp, path)
}

/// Write `v` into the already-allocated `slot` (its bytes are zeroed).
fn write_into(stores: &mut Stores, slot: &Node, v: &Value) {
    match v {
        // ── leaves ──────────────────────────────────────────────────────────
        Value::Null => slot.write_null(stores),
        Value::Line(n) => {
            slot.set_discriminant(stores, ds::DISC_LINE);
            slot.set_field_int(stores, ds::NDLINE_N, i64::from(*n));
        }
        Value::Int(n) => slot.write_int(stores, i64::from(*n)),
        Value::Enum(ord, tp) => {
            slot.set_discriminant(stores, ds::DISC_ENUM);
            slot.set_field_int(stores, ds::NDENUM_ORD, i64::from(*ord));
            slot.set_field_int(stores, ds::NDENUM_TP, i64::from(*tp));
        }
        Value::Boolean(b) => {
            slot.set_discriminant(stores, ds::DISC_BOOLEAN);
            slot.set_field_bool(stores, ds::NDBOOLEAN_B, *b);
        }
        Value::Float(f) => {
            slot.set_discriminant(stores, ds::DISC_FLOAT);
            slot.set_field_float(stores, ds::NDFLOAT_F, *f);
        }
        Value::Long(n) => {
            slot.set_discriminant(stores, ds::DISC_LONG);
            slot.set_field_int(stores, ds::NDLONG_N, *n);
        }
        Value::Single(f) => {
            slot.set_discriminant(stores, ds::DISC_SINGLE);
            slot.set_field_single(stores, ds::NDSINGLE_F, *f);
        }
        Value::Text(s) => {
            slot.set_discriminant(stores, ds::DISC_TEXT);
            slot.set_field_str(stores, ds::NDTEXT_S, s);
        }
        Value::Var(n) => {
            slot.set_discriminant(stores, ds::DISC_VAR);
            slot.set_field_int(stores, ds::NDVAR_N, i64::from(*n));
        }
        Value::Break(n) => {
            slot.set_discriminant(stores, ds::DISC_BREAK);
            slot.set_field_int(stores, ds::NDBREAK_N, i64::from(*n));
        }
        Value::Continue(n) => {
            slot.set_discriminant(stores, ds::DISC_CONTINUE);
            slot.set_field_int(stores, ds::NDCONTINUE_N, i64::from(*n));
        }
        Value::FnRefDnr(n) => {
            slot.set_discriminant(stores, ds::DISC_FN_REF_DNR);
            slot.set_field_int(stores, ds::NDFNREFDNR_N, i64::from(*n));
        }
        Value::TupleGet(var, idx) => {
            slot.set_discriminant(stores, ds::DISC_TUPLE_GET);
            slot.set_field_int(stores, ds::NDTUPLEGET_VAR, i64::from(*var));
            slot.set_field_int(stores, ds::NDTUPLEGET_IDX, i64::from(*idx));
        }
        Value::RawExpr(s) => {
            slot.set_discriminant(stores, ds::DISC_RAW_EXPR);
            slot.set_field_str(stores, ds::NDRAWEXPR_S, s);
        }
        // ── scalar(s) + vector<Node> children ────────────────────────────────
        Value::Call(def_nr, args) => {
            slot.write_call(stores, *def_nr);
            push_all(stores, slot.call_parameters(), args);
        }
        Value::CallRef(var, args) => {
            slot.set_discriminant(stores, ds::DISC_CALL_REF);
            slot.set_field_int(stores, ds::NDCALLREF_VAR, i64::from(*var));
            push_all(stores, slot.field_vec(ds::NDCALLREF_ARGS), args);
        }
        Value::Insert(items) => {
            slot.set_discriminant(stores, ds::DISC_INSERT);
            push_all(stores, slot.field_vec(ds::NDINSERT_ITEMS), items);
        }
        Value::Set(var, inner) => {
            slot.set_discriminant(stores, ds::DISC_SET);
            slot.set_field_int(stores, ds::NDSET_VAR, i64::from(*var));
            materialize_node(stores, slot.field_vec(ds::NDSET_INNER), inner);
        }
        Value::Return(inner) => {
            slot.set_discriminant(stores, ds::DISC_RETURN);
            materialize_node(stores, slot.field_vec(ds::NDRETURN_INNER), inner);
        }
        Value::If(cond, t, f) => {
            slot.set_discriminant(stores, ds::DISC_IF);
            materialize_node(stores, slot.field_vec(ds::NDIF_COND), cond);
            materialize_node(stores, slot.field_vec(ds::NDIF_T), t);
            materialize_node(stores, slot.field_vec(ds::NDIF_F), f);
        }
        Value::Drop(inner) => {
            slot.set_discriminant(stores, ds::DISC_DROP);
            materialize_node(stores, slot.field_vec(ds::NDDROP_INNER), inner);
        }
        Value::Iter(var, create, next, init) => {
            slot.set_discriminant(stores, ds::DISC_ITER);
            slot.set_field_int(stores, ds::NDITER_VAR, i64::from(*var));
            materialize_node(stores, slot.field_vec(ds::NDITER_CREATE), create);
            materialize_node(stores, slot.field_vec(ds::NDITER_NEXT), next);
            materialize_node(stores, slot.field_vec(ds::NDITER_INIT), init);
        }
        Value::Tuple(items) => {
            slot.set_discriminant(stores, ds::DISC_TUPLE);
            push_all(stores, slot.field_vec(ds::NDTUPLE_ITEMS), items);
        }
        Value::TuplePut(var, idx, inner) => {
            slot.set_discriminant(stores, ds::DISC_TUPLE_PUT);
            slot.set_field_int(stores, ds::NDTUPLEPUT_VAR, i64::from(*var));
            slot.set_field_int(stores, ds::NDTUPLEPUT_IDX, i64::from(*idx));
            materialize_node(stores, slot.field_vec(ds::NDTUPLEPUT_INNER), inner);
        }
        Value::Yield(inner) => {
            slot.set_discriminant(stores, ds::DISC_YIELD);
            materialize_node(stores, slot.field_vec(ds::NDYIELD_INNER), inner);
        }
        Value::Parallel(arms) => {
            slot.set_discriminant(stores, ds::DISC_PARALLEL);
            push_all(stores, slot.field_vec(ds::NDPARALLEL_ARMS), arms);
        }
        // ── inlined Block ────────────────────────────────────────────────────
        Value::Block(b) => write_block(stores, slot, ds::DISC_BLOCK, b),
        Value::Loop(b) => write_block(stores, slot, ds::DISC_LOOP, b),
        // ── inlined sub-structs (no TypeT) ────────────────────────────────────
        Value::Span(boxed) => {
            let (position, inner) = &**boxed;
            slot.set_discriminant(stores, ds::DISC_SPAN);
            slot.set_field_int(stores, ds::SPAN_POS_LINE, i64::from(position.line));
            slot.set_field_int(stores, ds::SPAN_POS_POS, i64::from(position.pos));
            slot.set_field_str(stores, ds::SPAN_POS_FILE, &position.file);
            materialize_node(stores, slot.field_vec(ds::NDSPAN_INNER), inner);
        }
        // ── vector of a non-Node struct ───────────────────────────────────────
        Value::Keys(keys) => {
            slot.set_discriminant(stores, ds::DISC_KEYS);
            let kv = slot.field_recvec(ds::NDKEYS_KEYS, ds::KEY_STRIDE);
            for k in keys {
                let e = kv.push(stores);
                e.set_field_int(stores, ds::KEY_TYPE_NR, i64::from(k.type_nr));
                e.set_field_int(stores, ds::KEY_POSITION, i64::from(k.position));
                e.set_field_int(stores, ds::KEY_START, i64::from(k.start));
            }
        }
        // ── carries a vector<TypeT> ────────────────────────────────────────────
        Value::FnRef(def_nr, var, t) => {
            slot.set_discriminant(stores, ds::DISC_FN_REF);
            slot.set_field_int(stores, ds::NDFNREF_DEF_NR, i64::from(*def_nr));
            slot.set_field_int(stores, ds::NDFNREF_VAR, i64::from(*var));
            // t is Box<Type> → one-element vector<TypeT> (box-of-one).
            materialize_type(
                stores,
                slot.field_recvec(ds::NDFNREF_T, ds::TYPET_STRIDE),
                t,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Deps;
    use crate::data::{Attribute, Block, IntegerSpec, LinkedFieldGroup, LinkedFieldKind, Type};
    use crate::data_store::{self as ds, RecVector, Record, TypeKind, ValueType};
    use crate::ir_schema_gen::register_ir_schema;
    use crate::keys::Key;
    use crate::lexer::Position;
    use std::num::NonZeroU8;

    /// A standalone host record whose offset-0 field is the root
    /// `vector<Node>` the materializer writes into.
    fn root_vector(stores: &mut Stores) -> ValuesVector {
        ValuesVector::new(stores.database(16))
    }

    #[test]
    fn materialize_call_tree_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Call(144, vec![Value::Int(7), Value::Int(9)]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let got = root.get(0, &stores);
        assert_eq!(got.value_type(&stores), ValueType::Call);
        assert_eq!(got.call_to(&stores), 144);
        let params = got.call_parameters();
        assert_eq!(params.len(&stores), 2);
        assert_eq!(params.get(0, &stores).int_value(&stores), 7);
        assert_eq!(params.get(1, &stores).int_value(&stores), 9);
    }

    #[test]
    fn materialize_block_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Block(Box::new(Block {
            name: "loop_body",
            operators: vec![Value::Call(5, vec![Value::Int(1)]), Value::Null],
            result: Type::Boolean,
            scope: 3,
            var_size: 16,
        }));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let got = root.get(0, &stores);
        assert_eq!(got.value_type(&stores), ValueType::Block);
        assert_eq!(got.block_name(&stores), "loop_body");

        let ops = got.block_operators(&stores);
        assert_eq!(ops.len(&stores), 2);

        let call = ops.get(0, &stores);
        assert_eq!(call.value_type(&stores), ValueType::Call);
        assert_eq!(call.call_to(&stores), 5);
        assert_eq!(call.call_parameters().get(0, &stores).int_value(&stores), 1);

        assert_eq!(ops.get(1, &stores).value_type(&stores), ValueType::Null);

        // Block tail: scope, var_size, and result (box-of-one vector<TypeT>).
        // Read off the referenced `Block` record — the fields are its own now,
        // not the Node's plus an inline base.
        let blk = got.block_rec(&stores);
        assert_eq!(blk.field_int(&stores, ds::BLOCK_SCOPE), 3);
        assert_eq!(blk.field_int(&stores, ds::BLOCK_VAR_SIZE), 16);
        let result = blk.field_recvec(ds::BLOCK_RESULT, ds::TYPET_STRIDE);
        assert_eq!(result.len(&stores), 1);
        assert_eq!(
            ds::type_kind(result.get(0, &stores).discriminant(&stores)),
            TypeKind::Boolean
        );
    }

    #[test]
    fn materialize_leaf_scalars_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // One of every leaf-scalar variant, wrapped in a tuple so they share a
        // single root vector.
        let native = Value::Tuple(vec![
            Value::Line(3),
            Value::Long(99),
            Value::Float(2.5),
            Value::Single(1.5),
            Value::Boolean(true),
            Value::Text("hi".into()),
            Value::Var(7),
            Value::Break(2),
            Value::Continue(1),
            Value::FnRefDnr(4),
            Value::Enum(5, 6),
            Value::TupleGet(1, 2),
            Value::RawExpr("x".into()),
        ]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let tuple = root.get(0, &stores);
        assert_eq!(tuple.value_type(&stores), ValueType::Tuple);
        let items = tuple.field_vec(ds::NDTUPLE_ITEMS);
        assert_eq!(items.len(&stores), 13);

        let e = |i: u32| items.get(i, &stores);
        assert_eq!(e(0).value_type(&stores), ValueType::Line);
        assert_eq!(e(0).field_int(&stores, ds::NDLINE_N), 3);
        assert_eq!(e(1).value_type(&stores), ValueType::Long);
        assert_eq!(e(1).field_int(&stores, ds::NDLONG_N), 99);
        assert_eq!(e(2).value_type(&stores), ValueType::Float);
        assert_eq!(
            e(2).field_float(&stores, ds::NDFLOAT_F).to_bits(),
            2.5_f64.to_bits()
        );
        assert_eq!(e(3).value_type(&stores), ValueType::Single);
        assert_eq!(
            e(3).field_single(&stores, ds::NDSINGLE_F).to_bits(),
            1.5_f32.to_bits()
        );
        assert_eq!(e(4).value_type(&stores), ValueType::Boolean);
        assert!(e(4).field_bool(&stores, ds::NDBOOLEAN_B));
        assert_eq!(e(5).value_type(&stores), ValueType::Text);
        assert_eq!(e(5).field_str(&stores, ds::NDTEXT_S), "hi");
        assert_eq!(e(6).value_type(&stores), ValueType::Var);
        assert_eq!(e(6).field_int(&stores, ds::NDVAR_N), 7);
        assert_eq!(e(7).value_type(&stores), ValueType::Break);
        assert_eq!(e(7).field_int(&stores, ds::NDBREAK_N), 2);
        assert_eq!(e(8).value_type(&stores), ValueType::Continue);
        assert_eq!(e(9).value_type(&stores), ValueType::FnRefDnr);
        assert_eq!(e(9).field_int(&stores, ds::NDFNREFDNR_N), 4);
        assert_eq!(e(10).value_type(&stores), ValueType::Enum);
        assert_eq!(e(10).field_int(&stores, ds::NDENUM_ORD), 5);
        assert_eq!(e(10).field_int(&stores, ds::NDENUM_TP), 6);
        assert_eq!(e(11).value_type(&stores), ValueType::TupleGet);
        assert_eq!(e(11).field_int(&stores, ds::NDTUPLEGET_VAR), 1);
        assert_eq!(e(11).field_int(&stores, ds::NDTUPLEGET_IDX), 2);
        assert_eq!(e(12).value_type(&stores), ValueType::RawExpr);
        assert_eq!(e(12).field_str(&stores, ds::NDRAWEXPR_S), "x");
    }

    #[test]
    fn materialize_control_flow_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // if 1 { return 2 } else { drop 3 }
        let native = Value::If(
            Box::new(Value::Int(1)),
            Box::new(Value::Return(Box::new(Value::Int(2)))),
            Box::new(Value::Drop(Box::new(Value::Int(3)))),
        );

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let if_node = root.get(0, &stores);
        assert_eq!(if_node.value_type(&stores), ValueType::If);

        let cond = if_node.field_vec(ds::NDIF_COND).get(0, &stores);
        assert_eq!(cond.int_value(&stores), 1);

        let then = if_node.field_vec(ds::NDIF_T).get(0, &stores);
        assert_eq!(then.value_type(&stores), ValueType::Return);
        let ret_inner = then.field_vec(ds::NDRETURN_INNER).get(0, &stores);
        assert_eq!(ret_inner.int_value(&stores), 2);

        let els = if_node.field_vec(ds::NDIF_F).get(0, &stores);
        assert_eq!(els.value_type(&stores), ValueType::Drop);
        let drop_inner = els.field_vec(ds::NDDROP_INNER).get(0, &stores);
        assert_eq!(drop_inner.int_value(&stores), 3);
    }

    #[test]
    fn materialize_box_of_one_and_multi_vector_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // A mix: Set (scalar + box-of-one), CallRef (scalar + Vec),
        // Iter (scalar + three box-of-one children).
        let native = Value::Tuple(vec![
            Value::Set(9, Box::new(Value::Long(42))),
            Value::CallRef(3, vec![Value::Int(1), Value::Int(2)]),
            Value::Iter(
                5,
                Box::new(Value::Int(10)),
                Box::new(Value::Int(11)),
                Box::new(Value::Int(12)),
            ),
        ]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);
        let items = root.get(0, &stores).field_vec(ds::NDTUPLE_ITEMS);

        let set = items.get(0, &stores);
        assert_eq!(set.value_type(&stores), ValueType::Set);
        assert_eq!(set.field_int(&stores, ds::NDSET_VAR), 9);
        assert_eq!(
            set.field_vec(ds::NDSET_INNER)
                .get(0, &stores)
                .field_int(&stores, ds::NDLONG_N),
            42
        );

        let call_ref = items.get(1, &stores);
        assert_eq!(call_ref.value_type(&stores), ValueType::CallRef);
        assert_eq!(call_ref.field_int(&stores, ds::NDCALLREF_VAR), 3);
        let args = call_ref.field_vec(ds::NDCALLREF_ARGS);
        assert_eq!(args.len(&stores), 2);
        assert_eq!(args.get(1, &stores).int_value(&stores), 2);

        let iter = items.get(2, &stores);
        assert_eq!(iter.value_type(&stores), ValueType::Iter);
        assert_eq!(iter.field_int(&stores, ds::NDITER_VAR), 5);
        assert_eq!(
            iter.field_vec(ds::NDITER_CREATE)
                .get(0, &stores)
                .int_value(&stores),
            10
        );
        assert_eq!(
            iter.field_vec(ds::NDITER_NEXT)
                .get(0, &stores)
                .int_value(&stores),
            11
        );
        assert_eq!(
            iter.field_vec(ds::NDITER_INIT)
                .get(0, &stores)
                .int_value(&stores),
            12
        );
    }

    #[test]
    fn materialize_loop_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Loop(Box::new(Block {
            name: "spin",
            operators: vec![Value::Break(0)],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let lp = root.get(0, &stores);
        assert_eq!(lp.value_type(&stores), ValueType::Loop);
        assert_eq!(lp.block_name(&stores), "spin");
        let ops = lp.block_operators(&stores);
        assert_eq!(ops.len(&stores), 1);
        assert_eq!(ops.get(0, &stores).value_type(&stores), ValueType::Break);
    }

    #[test]
    fn materialize_span_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::Span(Box::new((
            Position {
                file: "f.loft".into(),
                line: 12,
                pos: 3,
            },
            Value::Int(7),
        )));

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let span = root.get(0, &stores);
        assert_eq!(span.value_type(&stores), ValueType::Span);
        assert_eq!(span.field_int(&stores, ds::SPAN_POS_LINE), 12);
        assert_eq!(span.field_int(&stores, ds::SPAN_POS_POS), 3);
        assert_eq!(span.field_str(&stores, ds::SPAN_POS_FILE), "f.loft");
        assert_eq!(
            span.field_vec(ds::NDSPAN_INNER)
                .get(0, &stores)
                .int_value(&stores),
            7
        );
    }

    #[test]
    fn materialize_keys_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // vector<Key> — the generic RecVector path (negative i8 included).
        let native = Value::Keys(vec![
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
        ]);

        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let k = root.get(0, &stores);
        assert_eq!(k.value_type(&stores), ValueType::Keys);
        let kv = k.field_recvec(ds::NDKEYS_KEYS, ds::KEY_STRIDE);
        assert_eq!(kv.len(&stores), 2);

        let e0 = kv.get(0, &stores);
        assert_eq!(e0.field_int(&stores, ds::KEY_TYPE_NR), 3);
        assert_eq!(e0.field_int(&stores, ds::KEY_POSITION), 7);
        let e1 = kv.get(1, &stores);
        assert_eq!(e1.field_int(&stores, ds::KEY_TYPE_NR), -1);
        assert_eq!(e1.field_int(&stores, ds::KEY_POSITION), 42);
    }

    /// H7 — EXHAUSTIVE: every `Value` variant survives the STORE codec
    /// (`materialize_node` → `read_value`), the path the startup cache uses.
    ///
    /// The per-shape tests above check the WRITE side (materialized fields) for
    /// ~29 variants; this checks the full write→read round-trip for ALL 32,
    /// including the 5 the per-shape tests miss (`Insert`/`TuplePut`/
    /// `Yield`/`Parallel`) and the rare ones the program corpus can't exercise
    /// (`Yield` is coroutine 1.1+).  The `samples.len() == 34` guard is the
    /// explicit "a new variant must be added here" reminder the F9 schema macro
    /// would automate — until then, a dropped field or a new variant fails here
    /// loudly instead of corrupting the cache silently.
    #[test]
    fn materialize_all_variants_round_trip() {
        use crate::ir_read::read_value;
        let block = || Block {
            name: "b",
            operators: vec![Value::Int(1), Value::Null],
            result: Type::Boolean,
            scope: 2,
            var_size: 8,
        };
        let samples: Vec<Value> = vec![
            Value::Null,
            Value::Line(3),
            Value::Span(Box::new((
                Position {
                    file: "f.loft".to_string(),
                    line: 12,
                    pos: 4,
                },
                Value::Int(9),
            ))),
            Value::Int(42),
            Value::Enum(5, 6),
            Value::Boolean(true),
            Value::Float(2.5),
            Value::Long(99),
            Value::Single(1.5),
            Value::Text("hi".into()),
            Value::Call(7, vec![Value::Var(0)]),
            Value::CallRef(3, vec![Value::Int(1)]),
            Value::Block(Box::new(block())),
            Value::Insert(vec![Value::Null, Value::Int(2)]),
            Value::Var(7),
            Value::Set(1, Box::new(Value::Int(2))),
            Value::Return(Box::new(Value::Var(0))),
            Value::Break(2),
            Value::Continue(1),
            Value::If(
                Box::new(Value::Boolean(true)),
                Box::new(Value::Int(1)),
                Box::new(Value::Int(2)),
            ),
            Value::Loop(Box::new(block())),
            Value::Drop(Box::new(Value::Var(0))),
            Value::Iter(
                1,
                Box::new(Value::Var(0)),
                Box::new(Value::Var(1)),
                Box::new(Value::Int(0)),
            ),
            Value::Keys(vec![Key {
                type_nr: 3,
                position: 7,
                start: 100,
            }]),
            Value::Tuple(vec![Value::Int(1), Value::Text("a".into())]),
            Value::TupleGet(0, 1),
            Value::TuplePut(0, 1, Box::new(Value::Int(2))),
            Value::Yield(Box::new(Value::Var(0))),
            Value::FnRef(2, 1, Box::new(Type::Boolean)),
            Value::FnRefDnr(4),
            Value::Parallel(vec![Value::Int(1)]),
            Value::RawExpr("x".into()),
        ];
        assert_eq!(
            samples.len(),
            32,
            "every Value variant must appear here (add the new one)"
        );
        for v in &samples {
            let mut stores = Stores::new();
            let _ids = register_ir_schema(&mut stores);
            let root = root_vector(&mut stores);
            materialize_node(&mut stores, root, v);
            let node = root.get(0, &stores);
            let back = read_value(&stores, node);
            assert_eq!(&back, v, "store-codec round-trip mismatch for {v:?}");
        }
    }

    /// A standalone host record whose offset-0 field is the root
    /// `vector<TypeT>` the type materializer writes into.
    fn root_type_vector(stores: &mut Stores) -> RecVector {
        RecVector::new(stores.database(16), ds::TYPET_STRIDE)
    }

    #[test]
    fn materialize_type_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // fn(integer{-5..100, !null, size 4}, text[1,2]) -> vector<boolean>[3]
        // with dep [7] — exercises scalars, IntegerSpec, dep lists, box-of-one
        // (result), Vec<Type> (args), and recursion (Vector inner).
        let ty = Type::Function(
            vec![
                Type::Integer(IntegerSpec {
                    min: -5,
                    max: 100,
                    not_null: true,
                    forced_size: NonZeroU8::new(4),
                }),
                Type::Text(Deps::unknown(vec![1, 2])),
            ],
            Box::new(Type::Vector(
                Box::new(Type::Boolean),
                Deps::unknown(vec![3]),
            )),
            Deps::unknown(vec![7]),
        );

        let root = root_type_vector(&mut stores);
        materialize_type(&mut stores, root, &ty);

        let f = root.get(0, &stores);
        assert_eq!(ds::type_kind(f.discriminant(&stores)), TypeKind::Function);

        let args = f.field_recvec(ds::TYFUNC_ARGS, ds::TYPET_STRIDE);
        assert_eq!(args.len(&stores), 2);

        let int = args.get(0, &stores);
        assert_eq!(ds::type_kind(int.discriminant(&stores)), TypeKind::Integer);
        assert_eq!(int.field_int(&stores, ds::TYINTEGER_MIN), -5);
        assert_eq!(int.field_int(&stores, ds::TYINTEGER_MAX), 100);
        assert!(int.field_bool(&stores, ds::TYINTEGER_NOT_NULL));
        assert_eq!(int.field_int(&stores, ds::TYINTEGER_FORCED), 4);

        let text = args.get(1, &stores);
        assert_eq!(ds::type_kind(text.discriminant(&stores)), TypeKind::Text);
        let tdep = text.field_recvec(ds::TYTEXT_DEP, ds::INT_STRIDE);
        assert_eq!(tdep.len(&stores), 2);
        assert_eq!(tdep.get(0, &stores).field_int(&stores, 0), 1);
        assert_eq!(tdep.get(1, &stores).field_int(&stores, 0), 2);

        // result: box-of-one vector<TypeT> holding the Vector type.
        let result = f.field_recvec(ds::TYFUNC_RESULT, ds::TYPET_STRIDE);
        assert_eq!(result.len(&stores), 1);
        let vec_ty = result.get(0, &stores);
        assert_eq!(
            ds::type_kind(vec_ty.discriminant(&stores)),
            TypeKind::Vector
        );
        let inner = vec_ty.field_recvec(ds::TYVECTOR_INNER, ds::TYPET_STRIDE);
        assert_eq!(
            ds::type_kind(inner.get(0, &stores).discriminant(&stores)),
            TypeKind::Boolean
        );
        let vdep = vec_ty.field_recvec(ds::TYVECTOR_DEP, ds::INT_STRIDE);
        assert_eq!(vdep.get(0, &stores).field_int(&stores, 0), 3);

        let fdep = f.field_recvec(ds::TYFUNC_DEP, ds::INT_STRIDE);
        assert_eq!(fdep.len(&stores), 1);
        assert_eq!(fdep.get(0, &stores).field_int(&stores, 0), 7);
    }

    #[test]
    fn materialize_type_sorted_keys_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // Sorted + Radix exercise vector<SortKey> and vector<NameRef>.
        let ty = Type::Sorted(
            9,
            vec![("name".into(), true), ("age".into(), false)],
            Deps::unknown(vec![2]),
        );
        let root = root_type_vector(&mut stores);
        materialize_type(&mut stores, root, &ty);
        let s = root.get(0, &stores);
        assert_eq!(ds::type_kind(s.discriminant(&stores)), TypeKind::Sorted);
        assert_eq!(s.field_int(&stores, ds::TYSORTED_N), 9);
        let keys = s.field_recvec(ds::TYSORTED_KEYS, ds::SORTKEY_STRIDE);
        assert_eq!(keys.len(&stores), 2);
        let k0 = keys.get(0, &stores);
        assert_eq!(k0.field_str(&stores, ds::SORTKEY_NAME), "name");
        assert!(k0.field_bool(&stores, ds::SORTKEY_ASC));
        let k1 = keys.get(1, &stores);
        assert_eq!(k1.field_str(&stores, ds::SORTKEY_NAME), "age");
        assert!(!k1.field_bool(&stores, ds::SORTKEY_ASC));

        let sp = Type::Radix(4, vec!["x".into(), "y".into()], Deps::none());
        let root2 = root_type_vector(&mut stores);
        materialize_type(&mut stores, root2, &sp);
        let spr = root2.get(0, &stores);
        assert_eq!(ds::type_kind(spr.discriminant(&stores)), TypeKind::Radix);
        let names = spr.field_recvec(ds::TYRADIX_NAMES, ds::NAMEREF_STRIDE);
        assert_eq!(names.len(&stores), 2);
        assert_eq!(
            names.get(0, &stores).field_str(&stores, ds::NAMEREF_NAME),
            "x"
        );
        assert_eq!(
            names.get(1, &stores).field_str(&stores, ds::NAMEREF_NAME),
            "y"
        );
    }

    #[test]
    fn materialize_fn_ref_round_trips() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let native = Value::FnRef(42, 3, Box::new(Type::Boolean));
        let root = root_vector(&mut stores);
        materialize_node(&mut stores, root, &native);

        let fr = root.get(0, &stores);
        assert_eq!(fr.value_type(&stores), ValueType::FnRef);
        assert_eq!(fr.field_int(&stores, ds::NDFNREF_DEF_NR), 42);
        assert_eq!(fr.field_int(&stores, ds::NDFNREF_VAR), 3);
        let t = fr.field_recvec(ds::NDFNREF_T, ds::TYPET_STRIDE);
        assert_eq!(t.len(&stores), 1);
        assert_eq!(
            ds::type_kind(t.get(0, &stores).discriminant(&stores)),
            TypeKind::Boolean
        );
    }

    #[test]
    fn materialize_attributes_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let attrs = vec![Attribute {
            name: "count".into(),
            typedef: Type::Integer(IntegerSpec {
                min: 0,
                max: 10,
                not_null: true,
                forced_size: None,
            }),
            mutable: true,
            constant: false,
            const_field: true,
            value_const: false,
            init: false,
            nullable: true,
            primary: false,
            hidden: false,
            value: Value::Int(7),
            check: Value::Null,
            check_message: Value::Text("bad".into()),
            alias_d_nr: 3,
            assigned_lambda_d_nr: 99,
            links: vec!["fs#read".into()],
            lexeme: false,
        }];

        // Host record whose field 0 is the root vector<Attribute>.
        let host = Record::new(stores.database(16));
        materialize_attributes(&mut stores, &host, 0, &attrs);

        let v = host.field_recvec(0, ds::ATTRIBUTE_STRIDE);
        assert_eq!(v.len(&stores), 1);
        let a = v.get(0, &stores);
        assert_eq!(a.field_str(&stores, ds::ATTR_NAME), "count");
        assert!(a.field_bool(&stores, ds::ATTR_MUTABLE));
        assert!(!a.field_bool(&stores, ds::ATTR_CONSTANT));
        // @PLN40 — non-vacuous: the attr was built with const_field: true, so a broken
        // write/offset would read back false here.
        assert!(a.field_bool(&stores, ds::ATTR_CONST_FIELD));
        assert!(a.field_bool(&stores, ds::ATTR_NULLABLE));
        assert_eq!(a.field_int(&stores, ds::ATTR_ALIAS_D_NR), 3);
        assert_eq!(a.field_int(&stores, ds::ATTR_ASSIGNED_LAMBDA_D_NR), 99);

        let td = a.field_recvec(ds::ATTR_TYPEDEF, ds::TYPET_STRIDE);
        assert_eq!(
            ds::type_kind(td.get(0, &stores).discriminant(&stores)),
            TypeKind::Integer
        );
        assert_eq!(
            a.field_vec(ds::ATTR_VALUE)
                .get(0, &stores)
                .int_value(&stores),
            7
        );
        assert_eq!(
            a.field_vec(ds::ATTR_CHECK)
                .get(0, &stores)
                .value_type(&stores),
            ValueType::Null
        );
        assert_eq!(
            a.field_vec(ds::ATTR_CHECK_MESSAGE)
                .get(0, &stores)
                .field_str(&stores, ds::NDTEXT_S),
            "bad"
        );
    }

    #[test]
    fn materialize_field_groups_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let groups = vec![
            LinkedFieldGroup {
                kind: LinkedFieldKind::Index,
                instance: 2,
                field_indices: vec![5, 6, 7],
                alignment: 4,
                size: 9,
            },
            LinkedFieldGroup {
                kind: LinkedFieldKind::Tuple,
                instance: 0,
                field_indices: vec![1],
                alignment: 8,
                size: 16,
            },
        ];

        let host = Record::new(stores.database(16));
        materialize_field_groups(&mut stores, &host, 0, &groups);

        let v = host.field_recvec(0, ds::LFG_STRIDE);
        assert_eq!(v.len(&stores), 2);

        let g0 = v.get(0, &stores);
        assert_eq!(g0.field_int(&stores, ds::LFG_KIND), 1); // Index
        assert_eq!(g0.field_int(&stores, ds::LFG_INSTANCE), 2);
        assert_eq!(g0.field_int(&stores, ds::LFG_ALIGNMENT), 4);
        assert_eq!(g0.field_int(&stores, ds::LFG_SIZE), 9);
        let fi = g0.field_recvec(ds::LFG_FIELD_INDICES, ds::INT_STRIDE);
        assert_eq!(fi.len(&stores), 3);
        assert_eq!(fi.get(2, &stores).field_int(&stores, 0), 7);

        let g1 = v.get(1, &stores);
        assert_eq!(g1.field_int(&stores, ds::LFG_KIND), 0); // Tuple
        assert_eq!(g1.field_int(&stores, ds::LFG_SIZE), 16);
    }

    /// Capstone: materialize the whole real `default/` stdlib `Data` into a
    /// store and verify the structure survives — every definition's name, its
    /// attribute count, and its `Function` variable count round-trip.  This
    /// exercises every Node/TypeT variant and every struct on real IR (the
    /// strongest arc B write-path validation short of a store→native reader).
    #[test]
    fn materialize_whole_stdlib_smoke() {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let data = p.data;
        assert!(
            data.definitions.len() > 100,
            "stdlib should have many definitions, got {}",
            data.definitions.len()
        );

        let mut ir = Stores::new();
        let _ids = register_ir_schema(&mut ir);
        let root = materialize_data(&mut ir, &data);

        let r = Record::new(root);
        assert_eq!(r.field_int(&ir, ds::DATA_SOURCE), i64::from(data.source));
        let defs = r.field_recvec(ds::DATA_DEFINITIONS, ds::DEFINITION_STRIDE);
        assert_eq!(defs.len(&ir) as usize, data.definitions.len());

        let mut saw_attrs = false;
        let mut saw_vars = false;
        for (i, d) in data.definitions.iter().enumerate() {
            let dr = defs.get(i as u32, &ir);
            assert_eq!(dr.field_str(&ir, ds::DEF_NAME), d.name, "def {i} name");

            let attrs = dr.field_recvec(ds::DEF_ATTRIBUTES, ds::ATTRIBUTE_STRIDE);
            assert_eq!(
                attrs.len(&ir) as usize,
                d.attributes.len(),
                "def {i} ({}) attribute count",
                d.name
            );
            saw_attrs |= !d.attributes.is_empty();

            let vars = dr.field_recvec(ds::DEF_VARIABLES + ds::FN_VARIABLES, ds::VARIABLE_STRIDE);
            assert_eq!(
                vars.len(&ir) as usize,
                d.variables.snapshot_len(),
                "def {i} ({}) variable count",
                d.name
            );
            saw_vars |= d.variables.snapshot_len() > 0;
        }
        assert!(saw_attrs, "some stdlib def should have attributes");
        assert!(saw_vars, "some stdlib def should have variables");
    }

    /// @PLN11 D2a step 2 — materialize the real stdlib's database type schema
    /// (`Stores.types`) into a `vector<DbType>` and verify it structurally:
    /// the type count round-trips, every `DbType` name matches, and each
    /// `Parts::Struct`'s field count survives.  (Full field-equivalence is the
    /// step-3 `read_schema` job.)
    #[test]
    fn materialize_stdlib_schema_smoke() {
        use crate::database::Parts;

        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false)
            .expect("parse default/ stdlib");
        let schema = &p.database.types;
        assert!(
            schema.len() > 50,
            "stdlib schema should have many types, got {}",
            schema.len()
        );

        let mut ir = Stores::new();
        let _ids = register_ir_schema(&mut ir);
        let host = Record::new(ir.database(16));
        materialize_schema(&mut ir, &host, 0, schema);

        let dst = host.field_recvec(0, ds::DBTYPE_STRIDE);
        assert_eq!(dst.len(&ir) as usize, schema.len(), "type count");

        let mut saw_struct = false;
        for (i, t) in schema.iter().enumerate() {
            let r = dst.get(i as u32, &ir);
            assert_eq!(r.field_str(&ir, ds::DBTYPE_NAME), t.name, "type {i} name");
            if let Parts::Struct(fields) = &t.parts {
                let pr = r
                    .field_recvec(ds::DBTYPE_PARTS, ds::DBPARTS_STRIDE)
                    .get(0, &ir);
                assert_eq!(pr.discriminant(&ir), ds::PT_STRUCT, "type {i} parts disc");
                let fv = pr.field_recvec(ds::PTSTRUCT_FIELDS, ds::DBFIELD_STRIDE);
                assert_eq!(
                    fv.len(&ir) as usize,
                    fields.len(),
                    "type {i} ({}) field count",
                    t.name
                );
                saw_struct = true;
            }
        }
        assert!(saw_struct, "stdlib schema should have struct types");
    }
}
