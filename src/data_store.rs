// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I63 — Store-resident IR

//! @PLN11 — store-backed compiler IR (`Data`).
//!
//! Simple wrappers around the IR's record/vector definitions: each handle hides
//! a `DbRef` and the methods retrieve fields via the already-written
//! Store/`vector` functions.  The wrappers do no Database re-derivation; they
//! read/write a field at its known offset with an existing primitive.
//!
//! The layout — variant discriminants, field byte offsets, the `Node` element
//! stride — is **hard-coded** here, mirroring exactly what loft's schema
//! ([`crate::ir_schema_gen::register_ir_schema`]) lays out.  Hand-written for
//! now to get the shape clean; later these consts come straight from the
//! generated layout, so the access is a folded constant with no lookup.

use crate::database::Stores;
use crate::keys::DbRef;
use crate::vector;

// ─── Hard-coded layout (mirrors the loft schema) ─────────────────────────────
//
// Every const below is pinned to the registered schema by the
// `baked_layout_mirrors_loft_schema` guard test — a mistyped offset fails CI
// rather than silently reading the wrong bytes.  `pub(crate)` so the arc B
// materializer (`crate::ir_store`) writes through the same offsets the readers
// use, keeping a single layout authority.

/// `Node` variant discriminants (1-based), in declaration order.
pub(crate) const DISC_NULL: u8 = 1;
pub(crate) const DISC_LINE: u8 = 2;
pub(crate) const DISC_SPAN: u8 = 3;
pub(crate) const DISC_INT: u8 = 4;
pub(crate) const DISC_ENUM: u8 = 5;
pub(crate) const DISC_BOOLEAN: u8 = 6;
pub(crate) const DISC_FLOAT: u8 = 7;
pub(crate) const DISC_LONG: u8 = 8;
pub(crate) const DISC_SINGLE: u8 = 9;
pub(crate) const DISC_TEXT: u8 = 10;
pub(crate) const DISC_CALL: u8 = 11;
pub(crate) const DISC_CALL_REF: u8 = 12;
pub(crate) const DISC_BLOCK: u8 = 13;
pub(crate) const DISC_INSERT: u8 = 14;
pub(crate) const DISC_VAR: u8 = 15;
pub(crate) const DISC_SET: u8 = 16;
pub(crate) const DISC_RETURN: u8 = 17;
pub(crate) const DISC_BREAK: u8 = 18;
pub(crate) const DISC_CONTINUE: u8 = 19;
pub(crate) const DISC_IF: u8 = 20;
pub(crate) const DISC_LOOP: u8 = 21;
pub(crate) const DISC_DROP: u8 = 22;
pub(crate) const DISC_ITER: u8 = 23;
pub(crate) const DISC_KEYS: u8 = 24;
pub(crate) const DISC_TUPLE: u8 = 25;
pub(crate) const DISC_TUPLE_GET: u8 = 26;
pub(crate) const DISC_TUPLE_PUT: u8 = 27;
pub(crate) const DISC_YIELD: u8 = 28;
pub(crate) const DISC_FN_REF: u8 = 29;
pub(crate) const DISC_FN_REF_DNR: u8 = 30;
pub(crate) const DISC_PARALLEL: u8 = 31;
pub(crate) const DISC_RAW_EXPR: u8 = 32;

// Field byte offsets within each variant's record, relative to the record's
// `pos`.  The `enum` discriminant byte is always at 0.  The layout packs the
// first `vector<Node>` child at 4 and integers / further vectors at 8/12/16/20.
pub(crate) const NDLINE_N: u32 = 8;
pub(crate) const NDINT_N: u32 = 8;
pub(crate) const NDENUM_ORD: u32 = 8;
pub(crate) const NDENUM_TP: u32 = 16;
pub(crate) const NDBOOLEAN_B: u32 = 7;
pub(crate) const NDFLOAT_F: u32 = 8;
pub(crate) const NDLONG_N: u32 = 8;
pub(crate) const NDSINGLE_F: u32 = 4;
pub(crate) const NDTEXT_S: u32 = 4;
pub(crate) const NDCALL_ARGS: u32 = 4;
pub(crate) const NDCALL_DEF_NR: u32 = 8;
pub(crate) const NDCALLREF_ARGS: u32 = 4;
pub(crate) const NDCALLREF_VAR: u32 = 8;
/// Offset of `NdBlock`/`NdLoop`'s box-of-one `vector<Block>` HANDLE.
///
/// The block used to be inlined here, which cost every `Node` in the image the
/// size of the largest variant. `BLOCK_*` below are offsets inside the `Block`
/// record the box holds — reached via [`Node::block_rec`], never added to this.
pub(crate) const NDBLOCK_BLOCK: u32 = 4;
/// Element stride of that box: one `Block` record.
pub(crate) const BLOCK_STRIDE: u32 = 28;
pub(crate) const BLOCK_NAME: u32 = 16;
pub(crate) const BLOCK_OPERATORS: u32 = 20;
pub(crate) const NDINSERT_ITEMS: u32 = 4;
pub(crate) const NDVAR_N: u32 = 8;
pub(crate) const NDSET_VAR: u32 = 8;
pub(crate) const NDSET_INNER: u32 = 4;
pub(crate) const NDRETURN_INNER: u32 = 4;
pub(crate) const NDBREAK_N: u32 = 8;
pub(crate) const NDCONTINUE_N: u32 = 8;
pub(crate) const NDIF_COND: u32 = 4;
pub(crate) const NDIF_T: u32 = 8;
pub(crate) const NDIF_F: u32 = 12;
pub(crate) const NDDROP_INNER: u32 = 4;
pub(crate) const NDITER_VAR: u32 = 8;
pub(crate) const NDITER_CREATE: u32 = 4;
pub(crate) const NDITER_NEXT: u32 = 16;
pub(crate) const NDITER_INIT: u32 = 20;
pub(crate) const NDTUPLE_ITEMS: u32 = 4;
pub(crate) const NDTUPLEGET_VAR: u32 = 8;
pub(crate) const NDTUPLEGET_IDX: u32 = 16;
pub(crate) const NDTUPLEPUT_VAR: u32 = 8;
pub(crate) const NDTUPLEPUT_IDX: u32 = 16;
pub(crate) const NDTUPLEPUT_INNER: u32 = 4;
pub(crate) const NDYIELD_INNER: u32 = 4;
pub(crate) const NDFNREFDNR_N: u32 = 8;
pub(crate) const NDPARALLEL_ARMS: u32 = 4;
pub(crate) const NDRAWEXPR_S: u32 = 4;

// Cross-struct variants — the inlined sub-struct's fields, as absolute offsets
// within the Node record (= the sub-struct's base offset + the field's offset
// inside the sub-struct).  The guard ties each back to base + relative.
pub(crate) const NDSPAN_INNER: u32 = 4;
pub(crate) const SPAN_POS_LINE: u32 = 8; // Position base (8) + Position.line (0)
pub(crate) const SPAN_POS_POS: u32 = 16; // + Position.pos (8)
pub(crate) const SPAN_POS_FILE: u32 = 24; // + Position.file (16)

// `NdKeys` holds a `vector<Key>`; every IR vector is inline `Parts::Vector`
// (probed — none promoted to a linked `Array`), so a stride-parameterised
// vector handle ([`RecVector`]) works for it and every other non-`Node` vector.
pub(crate) const NDKEYS_KEYS: u32 = 4;
pub(crate) const KEY_STRIDE: u32 = 24; // `Key` record size
pub(crate) const KEY_TYPE_NR: u32 = 0;
pub(crate) const KEY_POSITION: u32 = 8;
pub(crate) const KEY_START: u32 = 16;

// `NdFnRef` carries a `vector<TypeT>` — needs the TypeT half below.
pub(crate) const NDFNREF_T: u32 = 4;
pub(crate) const NDFNREF_DEF_NR: u32 = 8;
pub(crate) const NDFNREF_VAR: u32 = 16;

// ─── TypeT half (the IR's second recursive enum) ─────────────────────────────

/// `TypeT` variant discriminants (1-based), in declaration order.
pub(crate) const TY_UNKNOWN: u8 = 1;
pub(crate) const TY_NULL: u8 = 2;
pub(crate) const TY_VOID: u8 = 3;
pub(crate) const TY_NEVER: u8 = 4;
pub(crate) const TY_INTEGER: u8 = 5;
pub(crate) const TY_BOOLEAN: u8 = 6;
pub(crate) const TY_FLOAT: u8 = 7;
pub(crate) const TY_SINGLE: u8 = 8;
pub(crate) const TY_CHARACTER: u8 = 9;
pub(crate) const TY_TEXT: u8 = 10;
pub(crate) const TY_KEYS: u8 = 11;
pub(crate) const TY_ENUM: u8 = 12;
pub(crate) const TY_REFERENCE: u8 = 13;
pub(crate) const TY_REF_VAR: u8 = 14;
pub(crate) const TY_VECTOR: u8 = 15;
pub(crate) const TY_ROUTINE: u8 = 16;
pub(crate) const TY_ITERATOR: u8 = 17;
pub(crate) const TY_SORTED: u8 = 18;
pub(crate) const TY_INDEX: u8 = 19;
pub(crate) const TY_RADIX: u8 = 20;
pub(crate) const TY_HASH: u8 = 21;
pub(crate) const TY_FUNCTION: u8 = 22;
pub(crate) const TY_REWRITTEN: u8 = 23;
pub(crate) const TY_TUPLE: u8 = 24;
pub(crate) const TY_OPTIONAL: u8 = 25; // @PLN25 `τ?` — box-of-one child like RefVar
pub(crate) const TY_TRIE: u8 = 26; // `trie<T[k]>` — ONE text key, appended so 0..=25 keep their numbers

/// Element strides for the non-`Node` vectors the TypeT half uses.
pub(crate) const TYPET_STRIDE: u32 = 33; // `TypeT` enum record size
pub(crate) const INT_STRIDE: u32 = 8; // `vector<integer>` element (dep lists)
pub(crate) const SORTKEY_STRIDE: u32 = 5; // `SortKey` record size
pub(crate) const NAMEREF_STRIDE: u32 = 4; // `NameRef` record size

/// `TypeT` field offsets.  `dep` (`vector<integer>`) is at 4 for the leaf-ish
/// variants and at higher offsets when other vectors precede it.
pub(crate) const TYUNKNOWN_N: u32 = 8;
pub(crate) const TYINTEGER_MIN: u32 = 8; // IntegerSpec base (8) + min (0)
pub(crate) const TYINTEGER_MAX: u32 = 16; // + max (8)
pub(crate) const TYINTEGER_FORCED: u32 = 24; // + forced_size (16); 0 = None
pub(crate) const TYINTEGER_NOT_NULL: u32 = 32; // + not_null (24)
pub(crate) const TYTEXT_DEP: u32 = 4;
pub(crate) const TYENUM_N: u32 = 8;
pub(crate) const TYENUM_IS_REF: u32 = 3;
pub(crate) const TYENUM_DEP: u32 = 4;
pub(crate) const TYREF_N: u32 = 8;
pub(crate) const TYREF_DEP: u32 = 4;
pub(crate) const TYREFVAR_INNER: u32 = 4;
pub(crate) const TYVECTOR_INNER: u32 = 4;
pub(crate) const TYVECTOR_DEP: u32 = 8;
pub(crate) const TYROUTINE_N: u32 = 8;
pub(crate) const TYITER_STEP: u32 = 4;
pub(crate) const TYITER_INNER: u32 = 8;
pub(crate) const TYSORTED_N: u32 = 8;
pub(crate) const TYSORTED_KEYS: u32 = 4;
pub(crate) const TYSORTED_DEP: u32 = 16;
pub(crate) const TYINDEX_N: u32 = 8;
pub(crate) const TYINDEX_KEYS: u32 = 4;
pub(crate) const TYINDEX_DEP: u32 = 16;
pub(crate) const TYTRIE_KEY: u32 = 4; // the one key NAME (text), where Radix carries a name list
pub(crate) const TYTRIE_N: u32 = 8;
pub(crate) const TYTRIE_DEP: u32 = 16;
pub(crate) const TYRADIX_N: u32 = 8;
pub(crate) const TYRADIX_NAMES: u32 = 4;
pub(crate) const TYRADIX_DEP: u32 = 16;
pub(crate) const TYHASH_N: u32 = 8;
pub(crate) const TYHASH_NAMES: u32 = 4;
pub(crate) const TYHASH_DEP: u32 = 16;
pub(crate) const TYFUNC_ARGS: u32 = 4;
pub(crate) const TYFUNC_RESULT: u32 = 8;
pub(crate) const TYFUNC_DEP: u32 = 12;
pub(crate) const TYREWRITTEN_INNER: u32 = 4;
pub(crate) const TYTUPLE_ELEMS: u32 = 4;
pub(crate) const TYOPTIONAL_INNER: u32 = 4; // @PLN25 `Optional(τ)` child (like RefVar)

/// `SortKey` / `NameRef` element fields.
pub(crate) const SORTKEY_NAME: u32 = 0;
pub(crate) const SORTKEY_ASC: u32 = 4;
pub(crate) const NAMEREF_NAME: u32 = 0;

// ─── Top-level component structs (Block tail, Attribute, LinkedFieldGroup) ────

/// `Block` fields beyond name/operators — relative to the `Block` base
/// (`NDBLOCK_BLOCK`), like `BLOCK_NAME`/`BLOCK_OPERATORS`.
pub(crate) const BLOCK_RESULT: u32 = 24; // vector<TypeT> (box-of-one)
pub(crate) const BLOCK_SCOPE: u32 = 0;
pub(crate) const BLOCK_VAR_SIZE: u32 = 8;

/// `Attribute` record (element of `vector<Attribute>`).
pub(crate) const ATTRIBUTE_STRIDE: u32 = 47; // @PLN86 F8b +4 links; @PLN40 +1 const_field bool
pub(crate) const ATTR_NAME: u32 = 16;
pub(crate) const ATTR_TYPEDEF: u32 = 20; // vector<TypeT> (box-of-one)
pub(crate) const ATTR_VALUE: u32 = 24; // vector<Node> (box-of-one)
pub(crate) const ATTR_CHECK: u32 = 28; // vector<Node> (box-of-one)
pub(crate) const ATTR_CHECK_MESSAGE: u32 = 32; // vector<Node> (box-of-one)
pub(crate) const ATTR_LINKS: u32 = 36; // @PLN86 F8b — text; "" = unlinked member
pub(crate) const ATTR_MUTABLE: u32 = 40;
pub(crate) const ATTR_CONSTANT: u32 = 41;
pub(crate) const ATTR_INIT: u32 = 42;
pub(crate) const ATTR_NULLABLE: u32 = 43;
pub(crate) const ATTR_PRIMARY: u32 = 44;
pub(crate) const ATTR_HIDDEN: u32 = 45;
pub(crate) const ATTR_CONST_FIELD: u32 = 46; // @PLN40 — write-once struct field
pub(crate) const ATTR_ALIAS_D_NR: u32 = 0;
pub(crate) const ATTR_ASSIGNED_LAMBDA_D_NR: u32 = 8;

/// `LinkedFieldGroup` record (element of `vector<LinkedFieldGroup>`).  `kind`
/// is the native `LinkedFieldKind` discriminant (`Tuple` = 0, `Index` = 1).
pub(crate) const LFG_STRIDE: u32 = 36;
pub(crate) const LFG_KIND: u32 = 0;
pub(crate) const LFG_INSTANCE: u32 = 8;
pub(crate) const LFG_FIELD_INDICES: u32 = 32; // vector<integer>
pub(crate) const LFG_ALIGNMENT: u32 = 16;
pub(crate) const LFG_SIZE: u32 = 24;

// ─── The top-level Data graph (Definition / Variable / Function / Data) ──────

/// `Position` field offsets, relative to a `Position` base.
pub(crate) const POS_LINE: u32 = 0;
pub(crate) const POS_POS: u32 = 8;
pub(crate) const POS_FILE: u32 = 16;

/// `Variable` record (element of `Function.variables` = `vector<Variable>`) —
/// the ten codegen-read fields the snapshot seam exposes.
pub(crate) const VARIABLE_STRIDE: u32 = 37;
pub(crate) const VAR_NAME: u32 = 24;
pub(crate) const VAR_TYPE_DEF: u32 = 28; // vector<TypeT> (box-of-one)
pub(crate) const VAR_STACK_POS: u32 = 0;
pub(crate) const VAR_USES: u32 = 8;
pub(crate) const VAR_OWNER_WITNESS: u32 = 16;
pub(crate) const VAR_ARGUMENT: u32 = 32;
pub(crate) const VAR_STACK_ALLOCATED: u32 = 33;
pub(crate) const VAR_SKIP_FREE: u32 = 34;
pub(crate) const VAR_CAPTURED: u32 = 35;
pub(crate) const VAR_CALLER_HIDDEN_BUF: u32 = 36;

/// `Function` field offsets, relative to a `Function` base (it is inlined in
/// `Definition`, never stored in a vector).
pub(crate) const FN_NAME: u32 = 0;
pub(crate) const FN_FILE: u32 = 4;
pub(crate) const FN_VARIABLES: u32 = 8; // vector<Variable>
pub(crate) const FN_NAMES: u32 = 12; // vector<NameNr> (name -> var_nr map)
pub(crate) const FN_INLINE_REFS: u32 = 16; // vector<integer> (inline_ref_vars)

/// `NameNr` record (element of `Function.names` = `vector<NameNr>`).
pub(crate) const NAMENR_STRIDE: u32 = 12;
pub(crate) const NAMENR_NR: u32 = 0; // integer (var_nr)
pub(crate) const NAMENR_NAME: u32 = 8; // text

/// `Definition` record (element of `Data.definitions` = `vector<Definition>`).
/// Inlines `Position` (`DEF_POSITION` base) and `Function` (`DEF_VARIABLES`
/// base).  `def_type` / `purity` store integer codes (see `ir_store`).
pub(crate) const DEFINITION_STRIDE: u32 = 167; // @PLN24 arc A — +8 for the two #c text refs
pub(crate) const DEF_SOURCE: u32 = 0;
pub(crate) const DEF_DEF_TYPE: u32 = 8;
pub(crate) const DEF_PARENT: u32 = 16;
pub(crate) const DEF_POSITION: u32 = 24; // inlined Position base
pub(crate) const DEF_OP_CODE: u32 = 44;
pub(crate) const DEF_KNOWN_TYPE: u32 = 52;
pub(crate) const DEF_CLOSURE_RECORD: u32 = 60;
pub(crate) const DEF_FORCED_SIZE: u32 = 68; // Option<u8>; 0 = None
pub(crate) const DEF_PURITY: u32 = 76;
pub(crate) const DEF_NAME: u32 = 84;
pub(crate) const DEF_ATTRIBUTES: u32 = 88; // vector<Attribute>
pub(crate) const DEF_CODE: u32 = 92; // vector<Node> (box-of-one)
pub(crate) const DEF_RETURNED: u32 = 96; // vector<TypeT> (box-of-one)
pub(crate) const DEF_RUST: u32 = 100;
pub(crate) const DEF_NATIVE: u32 = 104;
pub(crate) const DEF_VARIABLES: u32 = 108; // inlined Function base (20 bytes)
pub(crate) const DEF_MUTATED_CAPTURES: u32 = 128; // vector<NameRef>
pub(crate) const DEF_SCALARS_TO_BOX: u32 = 132; // vector<NameRef>
pub(crate) const DEF_BOUNDS: u32 = 136; // vector<integer>
pub(crate) const DEF_FIELD_GROUPS: u32 = 140; // vector<LinkedFieldGroup>
pub(crate) const DEF_SYNTHETIC: u32 = 144; // Option<&str>; "" = None
pub(crate) const DEF_CAP: u32 = 148; // @PLN86 the group#right call-gate link; "" = unlinked
pub(crate) const DEF_SUPERSEDED: u32 = 152; // @PLN102 arc C #superseded "Y"; "" = not superseded
pub(crate) const DEF_RETURNED_NOT_NULL: u32 = 164;
pub(crate) const DEF_PUB_VISIBLE: u32 = 165;
pub(crate) const DEF_NULL_SAFE: u32 = 166; // @PLN46 W2 #null_safe; false = unannotated
pub(crate) const DEF_C_SYMBOL: u32 = 156; // @PLN24 #c "sym"; "" = not a C binding
pub(crate) const DEF_C_SIG: u32 = 160; // @PLN24 the declared C signature; "" = none

/// `Data` record (the root).
pub(crate) const DATA_SOURCE: u32 = 0;
pub(crate) const DATA_DEFINITIONS: u32 = 8; // vector<Definition>
pub(crate) const DATA_IMPORTS: u32 = 12; // vector<AppliedImport> (loft#1359)
pub(crate) const DATA_USE_NAMES: u32 = 16; // vector<UseName> (loft#1359)
/// The root record's size in bytes.
pub(crate) const DATA_STRIDE: u32 = 20;
/// What a `Data` root CLAIMS: `Store::claim` counts 8-byte words and the block
/// starts with an 8-byte header (the root's fields begin at byte 8), so a claim
/// sized from the stride alone leaves the last field in the next block.
pub(crate) const DATA_ROOT_WORDS: u32 = (8 + DATA_STRIDE).div_ceil(8);

/// `AppliedImport` record — one retained import, replayed by `rebuild_indices`.
pub(crate) const IMPORT_LIB_SOURCE: u32 = 0;
pub(crate) const IMPORT_INTO_SOURCE: u32 = 8;
pub(crate) const IMPORT_NAME: u32 = 16; // "" = wildcard
pub(crate) const IMPORT_BIND: u32 = 20;
pub(crate) const IMPORT_STRIDE: u32 = 24;

/// `UseName` record — a `use` short name or alias and its source number.
pub(crate) const USENAME_NAME: u32 = 8;
pub(crate) const USENAME_SOURCE: u32 = 0;
pub(crate) const USENAME_STRIDE: u32 = 12;

/// Well-known location of the `Data` root record in a saved IR store
/// (@PLN11 arc D).  A freshly-opened file-backed store's first `claim(16)`
/// deterministically yields record `1`, data at byte offset `8` — so a
/// persisted IR store always has its root here, and `open` needs no sidecar
/// to find it (the save path asserts the root landed at `IR_ROOT_REC`).
///
/// Gated on `mmap` — the only consumers are `ir_store::save_data` /
/// `ir_read::open_data`, both `#[cfg(feature = "mmap")]`; without it (e.g. the
/// `--no-default-features` wasm build) these would be dead code.
#[cfg(feature = "mmap")]
pub(crate) const IR_ROOT_REC: u32 = 1;
#[cfg(feature = "mmap")]
pub(crate) const IR_ROOT_POS: u32 = 8;

// ─── Database type schema (D2a) — `Stores.types` cached alongside the IR ──────
//
// Mirrors `src/database`: `DbType` (`database::Type`), `DbParts`
// (`database::Parts`, 17 variants), `DbField` (`database::Field`), `DbContent`
// (`keys::Content`), plus `EnumPair`/`KeyField` tuple-element records and the
// `Bundle { data, types }` store root.  `Key` / `LinkedFieldGroup` are reused.

/// `DbContent` (a field's default value) — enum record.
pub(crate) const DBCONTENT_STRIDE: u32 = 16;
pub(crate) const DC_LONG: u8 = 1;
pub(crate) const DC_FLOAT: u8 = 2;
pub(crate) const DC_SINGLE: u8 = 3;
pub(crate) const DC_STR: u8 = 4;
pub(crate) const DCLONG_V: u32 = 8; // integer (i64)
pub(crate) const DCFLOAT_V: u32 = 8; // float
pub(crate) const DCSINGLE_V: u32 = 4; // single
pub(crate) const DCSTR_V: u32 = 4; // text

/// `DbField` record (element of `Parts::Struct` / `EnumValue` field vectors).
pub(crate) const DBFIELD_STRIDE: u32 = 29;
pub(crate) const DBFIELD_CONTENT: u32 = 0; // u16 known_type
pub(crate) const DBFIELD_POSITION: u32 = 8; // u16 byte offset
pub(crate) const DBFIELD_NAME: u32 = 16;
pub(crate) const DBFIELD_DEFAULT: u32 = 20; // vector<DbContent> (box-of-one)
pub(crate) const DBFIELD_OTHER_INDEXES: u32 = 24; // vector<integer>
pub(crate) const DBFIELD_NULLABLE: u32 = 28; // @PLN127 arc D — declared nullable (1 byte)

/// `EnumPair` `(u16, text)` element of `Parts::Enum`.
pub(crate) const ENUMPAIR_STRIDE: u32 = 12;
pub(crate) const ENUMPAIR_NR: u32 = 0;
pub(crate) const ENUMPAIR_NAME: u32 = 8;

/// `KeyField` `(u16, bool)` element of a Sorted/Ordered/Index key list.
pub(crate) const KEYFIELD_STRIDE: u32 = 9;
pub(crate) const KEYFIELD_NR: u32 = 0;
pub(crate) const KEYFIELD_ASC: u32 = 8;

/// `DbParts` (`database::Parts`) — enum record, 19 variants.
pub(crate) const DBPARTS_STRIDE: u32 = 24;
pub(crate) const PT_BASE: u8 = 1;
pub(crate) const PT_STRUCT: u8 = 2;
pub(crate) const PT_ENUM: u8 = 3;
pub(crate) const PT_ENUM_VALUE: u8 = 4;
pub(crate) const PT_BYTE: u8 = 5;
pub(crate) const PT_SHORT: u8 = 6;
pub(crate) const PT_INT: u8 = 7;
pub(crate) const PT_SHORT_RAW: u8 = 8;
pub(crate) const PT_VECTOR: u8 = 9;
pub(crate) const PT_ARRAY: u8 = 10;
pub(crate) const PT_SORTED: u8 = 11;
pub(crate) const PT_ORDERED: u8 = 12;
pub(crate) const PT_HASH: u8 = 13;
pub(crate) const PT_INDEX: u8 = 14;
pub(crate) const PT_RADIX: u8 = 15;
pub(crate) const PT_DB_REF: u8 = 16;
pub(crate) const PT_CHILD_REC: u8 = 17;
pub(crate) const PT_TRIE: u8 = 18;
pub(crate) const PT_INT_RAW: u8 = 19;
// `DbParts` field offsets (shared where variant shapes coincide).
pub(crate) const PTSTRUCT_FIELDS: u32 = 4; // vector<DbField>
pub(crate) const PTENUM_VALUES: u32 = 4; // vector<EnumPair>
pub(crate) const PTENUMVALUE_VALUE: u32 = 8;
pub(crate) const PTENUMVALUE_FIELDS: u32 = 4; // vector<DbField>
pub(crate) const PTNUM_START: u32 = 8; // Byte/Short/Int/ShortRaw/IntRaw (i32)
pub(crate) const PTNUM_NULLABLE: u32 = 7; // Byte/Short/Int/ShortRaw/IntRaw
pub(crate) const PTCONTENT: u32 = 8; // Vector/Array/Sorted/Ordered/Hash/Index/Radix/ChildRec
pub(crate) const PTKEYS: u32 = 4; // Sorted/Ordered/Index (vector<KeyField>)
pub(crate) const PTFIELDS: u32 = 4; // Hash/Radix (vector<integer>)
pub(crate) const PTINDEX_LEFT: u32 = 16;
pub(crate) const PTTRIE_KEY: u32 = 16; // Trie's key FIELD INDEX (content is at PTCONTENT)

/// `DbType` (`database::Type`) record — `parents` is derived, not stored.
pub(crate) const DBTYPE_STRIDE: u32 = 34;
pub(crate) const DBTYPE_SIZE: u32 = 0; // u16
pub(crate) const DBTYPE_ALIGN: u32 = 8; // u8
pub(crate) const DBTYPE_NAME: u32 = 16;
pub(crate) const DBTYPE_PARTS: u32 = 20; // vector<DbParts> (box-of-one)
pub(crate) const DBTYPE_KEYS: u32 = 24; // vector<Key>
pub(crate) const DBTYPE_FIELD_GROUPS: u32 = 28; // vector<LinkedFieldGroup>
pub(crate) const DBTYPE_COMPLEX: u32 = 32;
pub(crate) const DBTYPE_LINKED: u32 = 33;

/// `Bundle { data: Data, types: vector<DbType> }` — the saved-bundle store root
/// (D2a step 4): `Data` inlined at offset 0, the schema vector at `BUNDLE_TYPES`.
pub(crate) const BUNDLE_DATA: u32 = 0;
pub(crate) const BUNDLE_TYPES: u32 = 20; // right after the inlined 20-byte `Data`
/// The bundle root's size in bytes, and its claim in words (see `DATA_ROOT_WORDS`).
pub(crate) const BUNDLE_STRIDE: u32 = 24;
#[cfg(feature = "mmap")] // only `save_bundle` claims a bundle root
pub(crate) const BUNDLE_ROOT_WORDS: u32 = (8 + BUNDLE_STRIDE).div_ceil(8);

/// The bit mask loft uses for a stored `boolean` field (`generation` emits
/// `get_boolean(rec, off, 1)`).
pub(crate) const BOOL_MASK: u8 = 1;

/// Element stride of a `vector<Node>` (the `Node` enum's record size).
pub(crate) const NODE_STRIDE: u32 = 28;

/// Which `Node` variant a [`Value`] is.  Mirrors the native `data::Value`
/// variants 1:1; `Other(discriminant)` only on an unrecognised byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Null,
    Line,
    Span,
    Int,
    Enum,
    Boolean,
    Float,
    Long,
    Single,
    Text,
    Call,
    CallRef,
    Block,
    Insert,
    Var,
    Set,
    Return,
    Break,
    Continue,
    If,
    Loop,
    Drop,
    Iter,
    Keys,
    Tuple,
    TupleGet,
    TuplePut,
    Yield,
    FnRef,
    FnRefDnr,
    Parallel,
    RawExpr,
    Other(u8),
}

/// Which `TypeT` variant a [`Record`] holds (when that record is a `TypeT`).
/// Mirrors the native `data::Type` variants 1:1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Unknown,
    Null,
    Void,
    Never,
    Integer,
    Boolean,
    Float,
    Single,
    Character,
    Text,
    Keys,
    Enum,
    Reference,
    RefVar,
    Vector,
    Routine,
    Iterator,
    Sorted,
    Index,
    Radix,
    Trie,
    Hash,
    Function,
    Rewritten,
    Tuple,
    Optional,
    Other(u8),
}

/// Map a `TypeT` discriminant byte to its [`TypeKind`].
#[must_use]
pub fn type_kind(disc: u8) -> TypeKind {
    match disc {
        TY_UNKNOWN => TypeKind::Unknown,
        TY_NULL => TypeKind::Null,
        TY_VOID => TypeKind::Void,
        TY_NEVER => TypeKind::Never,
        TY_INTEGER => TypeKind::Integer,
        TY_BOOLEAN => TypeKind::Boolean,
        TY_FLOAT => TypeKind::Float,
        TY_SINGLE => TypeKind::Single,
        TY_CHARACTER => TypeKind::Character,
        TY_TEXT => TypeKind::Text,
        TY_KEYS => TypeKind::Keys,
        TY_ENUM => TypeKind::Enum,
        TY_REFERENCE => TypeKind::Reference,
        TY_REF_VAR => TypeKind::RefVar,
        TY_VECTOR => TypeKind::Vector,
        TY_ROUTINE => TypeKind::Routine,
        TY_ITERATOR => TypeKind::Iterator,
        TY_SORTED => TypeKind::Sorted,
        TY_INDEX => TypeKind::Index,
        TY_RADIX => TypeKind::Radix,
        TY_TRIE => TypeKind::Trie,
        TY_HASH => TypeKind::Hash,
        TY_FUNCTION => TypeKind::Function,
        TY_REWRITTEN => TypeKind::Rewritten,
        TY_TUPLE => TypeKind::Tuple,
        TY_OPTIONAL => TypeKind::Optional,
        other => TypeKind::Other(other),
    }
}

/// A handle to one IR `Value` (a `Node` record), hiding its [`DbRef`].
#[derive(Debug, Clone, Copy)]
pub struct Value {
    rec: DbRef,
}

/// A handle to a `vector<Node>` field, hiding the vector internals.  `rec`
/// points at the field slot in the owning record (where the vector header
/// lives), as the `vector::*` functions expect.
#[derive(Debug, Clone, Copy)]
pub struct ValuesVector {
    rec: DbRef,
}

/// A handle to a non-`Node` struct record (`Key`, `SortKey`, `NameRef`,
/// `IntegerSpec`, …) — generic field access only, no variant discriminant.
/// The element type produced by [`RecVector`].
#[derive(Debug, Clone, Copy)]
pub struct Record {
    rec: DbRef,
}

/// A handle to an inline `vector<T>` field of fixed element `stride`, for the
/// non-`Node` element types.  Every IR vector is inline `Parts::Vector` (probed),
/// so the same raw `vector::*` primitives the `Node` vector uses apply here with
/// the element's stride.
#[derive(Debug, Clone, Copy)]
pub struct RecVector {
    rec: DbRef,
    stride: u32,
}

impl Value {
    /// Wrap a `Node` record.
    #[must_use]
    #[inline]
    pub fn new(rec: DbRef) -> Self {
        Value { rec }
    }

    /// The hidden `DbRef`, for rust code that drives the existing functions
    /// directly.
    #[must_use]
    #[inline]
    pub fn db_ref(&self) -> DbRef {
        self.rec
    }

    /// The 1-based variant discriminant stored at the record's `enum` byte
    /// (offset 0).
    #[must_use]
    pub fn discriminant(&self, stores: &Stores) -> u8 {
        stores
            .store(&self.rec)
            .get_byte(self.rec.rec, self.rec.pos, 0) as u8
    }

    /// Which variant this value is.
    #[must_use]
    pub fn value_type(&self, stores: &Stores) -> ValueType {
        match self.discriminant(stores) {
            DISC_NULL => ValueType::Null,
            DISC_LINE => ValueType::Line,
            DISC_SPAN => ValueType::Span,
            DISC_INT => ValueType::Int,
            DISC_ENUM => ValueType::Enum,
            DISC_BOOLEAN => ValueType::Boolean,
            DISC_FLOAT => ValueType::Float,
            DISC_LONG => ValueType::Long,
            DISC_SINGLE => ValueType::Single,
            DISC_TEXT => ValueType::Text,
            DISC_CALL => ValueType::Call,
            DISC_CALL_REF => ValueType::CallRef,
            DISC_BLOCK => ValueType::Block,
            DISC_INSERT => ValueType::Insert,
            DISC_VAR => ValueType::Var,
            DISC_SET => ValueType::Set,
            DISC_RETURN => ValueType::Return,
            DISC_BREAK => ValueType::Break,
            DISC_CONTINUE => ValueType::Continue,
            DISC_IF => ValueType::If,
            DISC_LOOP => ValueType::Loop,
            DISC_DROP => ValueType::Drop,
            DISC_ITER => ValueType::Iter,
            DISC_KEYS => ValueType::Keys,
            DISC_TUPLE => ValueType::Tuple,
            DISC_TUPLE_GET => ValueType::TupleGet,
            DISC_TUPLE_PUT => ValueType::TuplePut,
            DISC_YIELD => ValueType::Yield,
            DISC_FN_REF => ValueType::FnRef,
            DISC_FN_REF_DNR => ValueType::FnRefDnr,
            DISC_PARALLEL => ValueType::Parallel,
            DISC_RAW_EXPR => ValueType::RawExpr,
            other => ValueType::Other(other),
        }
    }

    /// `NdCall.def_nr` — the called definition number.
    #[must_use]
    pub fn call_to(&self, stores: &Stores) -> u32 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + NDCALL_DEF_NR) as u32
    }

    /// `NdCall.args` — the call parameters.
    #[must_use]
    pub fn call_parameters(&self) -> ValuesVector {
        ValuesVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + NDCALL_ARGS,
            },
        }
    }

    /// `NdBlock`'s inlined `Block.name`.
    #[must_use]
    pub fn block_name<'a>(&self, stores: &'a Stores) -> &'a str {
        self.block_rec(stores).field_str(stores, BLOCK_NAME)
    }

    /// Set the referenced `Block.name`.
    pub fn block_name_set(&self, stores: &mut Stores, name: &str) {
        let b = self.block_rec(stores);
        b.set_field_str(stores, BLOCK_NAME, name);
    }

    /// The `Block` record this `NdBlock`/`NdLoop` points at.
    ///
    /// The box holds exactly one element, pushed by [`Self::write_block`] /
    /// [`Self::write_loop`], so every reader can take element 0 — the same
    /// box-of-one shape `Block.result` and `DbField.default` already use.
    #[must_use]
    pub fn block_rec(&self, stores: &Stores) -> Record {
        self.field_recvec(NDBLOCK_BLOCK, BLOCK_STRIDE)
            .get(0, stores)
    }

    /// The referenced `Block.operators`.
    #[must_use]
    pub fn block_operators(&self, stores: &Stores) -> ValuesVector {
        self.block_rec(stores).field_vec(BLOCK_OPERATORS)
    }

    /// `NdInt.n` — the integer literal.
    #[must_use]
    pub fn int_value(&self, stores: &Stores) -> i64 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + NDINT_N)
    }

    // ─── Write side (arc B materializer) ─────────────────────────────────────
    //
    // Each setter writes the discriminant byte then any scalar fields, via the
    // same baked offsets the readers use.  Vector children (`NdCall.args`,
    // `NdBlock.operators`) are filled by the caller through the matching
    // `call_parameters()` / `block_operators()` handle + `ValuesVector::push`.

    /// Stamp the variant discriminant byte (offset 0).
    pub(crate) fn set_discriminant(&self, stores: &mut Stores, disc: u8) {
        stores
            .store_mut(&self.rec)
            .set_byte(self.rec.rec, self.rec.pos, 0, i32::from(disc));
    }

    /// Write this slot as `NdNull`.
    pub fn write_null(&self, stores: &mut Stores) {
        self.set_discriminant(stores, DISC_NULL);
    }

    /// Write this slot as `NdInt`.
    pub fn write_int(&self, stores: &mut Stores, n: i64) {
        self.set_discriminant(stores, DISC_INT);
        stores
            .store_mut(&self.rec)
            .set_int(self.rec.rec, self.rec.pos + NDINT_N, n);
    }

    /// Write this slot as `NdCall` (`def_nr` only — fill `args` via
    /// [`Value::call_parameters`] + [`ValuesVector::push`]).
    pub fn write_call(&self, stores: &mut Stores, def_nr: u32) {
        self.set_discriminant(stores, DISC_CALL);
        stores.store_mut(&self.rec).set_int(
            self.rec.rec,
            self.rec.pos + NDCALL_DEF_NR,
            i64::from(def_nr),
        );
    }

    /// Write this slot as `NdBlock` with `name` (fill `operators` via
    /// [`Value::block_operators`] + [`ValuesVector::push`]).
    /// Returns the freshly pushed `Block` record so the caller can fill the
    /// rest of it without looking it up again.
    pub fn write_block(&self, stores: &mut Stores, name: &str) -> Record {
        self.set_discriminant(stores, DISC_BLOCK);
        self.push_block(stores, name)
    }

    /// Push the box-of-one `Block` and set its name.  Shared by
    /// [`Self::write_block`] and [`Self::write_loop`], which differ only in the
    /// discriminant.
    fn push_block(&self, stores: &mut Stores, name: &str) -> Record {
        let b = self.field_recvec(NDBLOCK_BLOCK, BLOCK_STRIDE).push(stores);
        b.set_field_str(stores, BLOCK_NAME, name);
        b
    }

    /// Write this slot as `NdLoop` with `name` — same inlined `Block` layout as
    /// `NdBlock`, so `block_name`/`block_operators` apply unchanged.
    pub fn write_loop(&self, stores: &mut Stores, name: &str) -> Record {
        self.set_discriminant(stores, DISC_LOOP);
        self.push_block(stores, name)
    }

    // ─── Generic typed field access at a baked offset ────────────────────────
    //
    // Lower-level than the named accessors above: the arc B materializer writes
    // arbitrary variants by naming an offset const, without one method per
    // field.  Each folds to one `Store` primitive at one constant offset.

    /// Read the `integer` (8-byte) field at `off`.
    #[must_use]
    pub fn field_int(&self, stores: &Stores, off: u32) -> i64 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `integer` (8-byte) field at `off`.
    pub fn set_field_int(&self, stores: &mut Stores, off: u32, v: i64) {
        stores
            .store_mut(&self.rec)
            .set_int(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `float` (f64) field at `off`.
    #[must_use]
    pub fn field_float(&self, stores: &Stores, off: u32) -> f64 {
        stores
            .store(&self.rec)
            .get_float(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `float` (f64) field at `off`.
    pub fn set_field_float(&self, stores: &mut Stores, off: u32, v: f64) {
        stores
            .store_mut(&self.rec)
            .set_float(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `single` (f32) field at `off`.
    #[must_use]
    pub fn field_single(&self, stores: &Stores, off: u32) -> f32 {
        stores
            .store(&self.rec)
            .get_single(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `single` (f32) field at `off`.
    pub fn set_field_single(&self, stores: &mut Stores, off: u32, v: f32) {
        stores
            .store_mut(&self.rec)
            .set_single(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `boolean` field at `off`.
    #[must_use]
    pub fn field_bool(&self, stores: &Stores, off: u32) -> bool {
        stores
            .store(&self.rec)
            .get_boolean(self.rec.rec, self.rec.pos + off, BOOL_MASK)
    }

    /// Write the `boolean` field at `off`.
    pub fn set_field_bool(&self, stores: &mut Stores, off: u32, v: bool) {
        stores
            .store_mut(&self.rec)
            .set_boolean(self.rec.rec, self.rec.pos + off, BOOL_MASK, v);
    }

    /// Read the `text` field at `off`.
    #[must_use]
    pub fn field_str<'a>(&self, stores: &'a Stores, off: u32) -> &'a str {
        let store = stores.store(&self.rec);
        store.get_str(store.get_u32_raw(self.rec.rec, self.rec.pos + off))
    }

    /// Write the `text` field at `off`.
    pub fn set_field_str(&self, stores: &mut Stores, off: u32, s: &str) {
        let store = stores.store_mut(&self.rec);
        let idx = store.set_str(s);
        store.set_u32_raw(self.rec.rec, self.rec.pos + off, idx);
    }

    /// Handle to the `vector<Node>` field at `off`.
    #[must_use]
    pub fn field_vec(&self, off: u32) -> ValuesVector {
        ValuesVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + off,
            },
        }
    }

    /// Handle to a non-`Node` `vector<T>` field at `off` with element `stride`.
    #[must_use]
    pub fn field_recvec(&self, off: u32, stride: u32) -> RecVector {
        RecVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + off,
            },
            stride,
        }
    }
}

impl ValuesVector {
    /// Wrap the field slot where a `vector<Node>` header lives (e.g. a host
    /// record's field, for the arc B materializer's root).
    #[must_use]
    #[inline]
    pub fn new(rec: DbRef) -> Self {
        ValuesVector { rec }
    }

    /// The hidden `DbRef` of the vector field slot.
    #[must_use]
    #[inline]
    pub fn db_ref(&self) -> DbRef {
        self.rec
    }

    /// Number of elements.
    #[must_use]
    pub fn len(&self, stores: &Stores) -> u32 {
        vector::length_vector(&self.rec, &stores.allocations)
    }

    /// Whether the vector is empty.
    #[must_use]
    pub fn is_empty(&self, stores: &Stores) -> bool {
        self.len(stores) == 0
    }

    /// The `i`-th element.  Out-of-range yields a null [`Value`] (rec 0).
    #[must_use]
    pub fn get(&self, i: u32, stores: &Stores) -> Value {
        let rec = vector::get_vector(&self.rec, NODE_STRIDE, i64::from(i), &stores.allocations);
        Value { rec }
    }

    /// Append one element slot and return a handle to it.  The slot is zeroed
    /// (claim reuses dirty memory), so its `enum` byte and nested vector-header
    /// fields start empty — write a variant into it via `Value::write_*`.
    pub fn push(&self, stores: &mut Stores) -> Value {
        let len = i64::from(self.len(stores));
        let rec = vector::insert_vector(&self.rec, NODE_STRIDE, len, &mut stores.allocations);
        stores
            .store_mut(&rec)
            .zero_range(rec.rec, rec.pos, NODE_STRIDE);
        Value { rec }
    }
}

impl Record {
    /// Wrap a non-`Node` struct record.
    #[must_use]
    #[inline]
    pub fn new(rec: DbRef) -> Self {
        Record { rec }
    }

    /// The hidden `DbRef`.
    #[must_use]
    #[inline]
    pub fn db_ref(&self) -> DbRef {
        self.rec
    }

    /// The variant discriminant byte at offset 0 — only meaningful when this
    /// record is an enum value (e.g. a `TypeT`); pair with [`type_kind`].
    #[must_use]
    pub fn discriminant(&self, stores: &Stores) -> u8 {
        stores
            .store(&self.rec)
            .get_byte(self.rec.rec, self.rec.pos, 0) as u8
    }

    /// Stamp the variant discriminant byte at offset 0 (enum-value records).
    pub fn set_discriminant(&self, stores: &mut Stores, disc: u8) {
        stores
            .store_mut(&self.rec)
            .set_byte(self.rec.rec, self.rec.pos, 0, i32::from(disc));
    }

    /// Read the `integer` (8-byte) field at `off`.
    #[must_use]
    pub fn field_int(&self, stores: &Stores, off: u32) -> i64 {
        stores
            .store(&self.rec)
            .get_int(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `integer` (8-byte) field at `off`.
    pub fn set_field_int(&self, stores: &mut Stores, off: u32, v: i64) {
        stores
            .store_mut(&self.rec)
            .set_int(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `float` (f64) field at `off`.
    #[must_use]
    pub fn field_float(&self, stores: &Stores, off: u32) -> f64 {
        stores
            .store(&self.rec)
            .get_float(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `float` (f64) field at `off`.
    pub fn set_field_float(&self, stores: &mut Stores, off: u32, v: f64) {
        stores
            .store_mut(&self.rec)
            .set_float(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `single` (f32) field at `off`.
    #[must_use]
    pub fn field_single(&self, stores: &Stores, off: u32) -> f32 {
        stores
            .store(&self.rec)
            .get_single(self.rec.rec, self.rec.pos + off)
    }

    /// Write the `single` (f32) field at `off`.
    pub fn set_field_single(&self, stores: &mut Stores, off: u32, v: f32) {
        stores
            .store_mut(&self.rec)
            .set_single(self.rec.rec, self.rec.pos + off, v);
    }

    /// Read the `boolean` field at `off`.
    #[must_use]
    pub fn field_bool(&self, stores: &Stores, off: u32) -> bool {
        stores
            .store(&self.rec)
            .get_boolean(self.rec.rec, self.rec.pos + off, BOOL_MASK)
    }

    /// Write the `boolean` field at `off`.
    pub fn set_field_bool(&self, stores: &mut Stores, off: u32, v: bool) {
        stores
            .store_mut(&self.rec)
            .set_boolean(self.rec.rec, self.rec.pos + off, BOOL_MASK, v);
    }

    /// Read the `text` field at `off`.
    #[must_use]
    pub fn field_str<'a>(&self, stores: &'a Stores, off: u32) -> &'a str {
        let store = stores.store(&self.rec);
        store.get_str(store.get_u32_raw(self.rec.rec, self.rec.pos + off))
    }

    /// Write the `text` field at `off`.
    pub fn set_field_str(&self, stores: &mut Stores, off: u32, s: &str) {
        let store = stores.store_mut(&self.rec);
        let idx = store.set_str(s);
        store.set_u32_raw(self.rec.rec, self.rec.pos + off, idx);
    }

    /// Handle to a nested non-`Node` `vector<T>` field at `off`, element `stride`.
    #[must_use]
    pub fn field_recvec(&self, off: u32, stride: u32) -> RecVector {
        RecVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + off,
            },
            stride,
        }
    }

    /// Handle to a nested `vector<Node>` field at `off` (e.g. `Attribute.value`).
    #[must_use]
    pub fn field_vec(&self, off: u32) -> ValuesVector {
        ValuesVector {
            rec: DbRef {
                store_nr: self.rec.store_nr,
                rec: self.rec.rec,
                pos: self.rec.pos + off,
            },
        }
    }
}

impl RecVector {
    /// Wrap an inline `vector<T>` field slot with element `stride`.
    #[must_use]
    #[inline]
    pub fn new(rec: DbRef, stride: u32) -> Self {
        RecVector { rec, stride }
    }

    /// Number of elements.
    #[must_use]
    pub fn len(&self, stores: &Stores) -> u32 {
        vector::length_vector(&self.rec, &stores.allocations)
    }

    /// Whether the vector is empty.
    #[must_use]
    pub fn is_empty(&self, stores: &Stores) -> bool {
        self.len(stores) == 0
    }

    /// The `i`-th element as a [`Record`].
    #[must_use]
    pub fn get(&self, i: u32, stores: &Stores) -> Record {
        let rec = vector::get_vector(&self.rec, self.stride, i64::from(i), &stores.allocations);
        Record { rec }
    }

    /// Append one element slot and return its [`Record`] handle.  The slot is
    /// zeroed (claim reuses dirty memory) so unwritten fields — including nested
    /// vector headers — start empty.
    pub fn push(&self, stores: &mut Stores) -> Record {
        let len = i64::from(self.len(stores));
        let rec = vector::insert_vector(&self.rec, self.stride, len, &mut stores.allocations);
        stores
            .store_mut(&rec)
            .zero_range(rec.rec, rec.pos, self.stride);
        Record { rec }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir_schema_gen::register_ir_schema;

    /// Claim a fresh `Node` record and write its discriminant.
    fn new_node(stores: &mut Stores, disc: u8) -> DbRef {
        let rec = stores.database(16);
        stores
            .store_mut(&rec)
            .set_byte(rec.rec, rec.pos, 0, i32::from(disc));
        rec
    }

    /// Append an element slot into the `vector<Node>` field at `field`
    /// (delegates to the production [`ValuesVector::push`]).
    fn push(stores: &mut Stores, field: DbRef) -> DbRef {
        ValuesVector::new(field).push(stores).db_ref()
    }

    /// Write an `NdInt` (disc + `n`) into `slot` (delegates to the production
    /// [`Value::write_int`]).
    fn write_int(stores: &mut Stores, slot: DbRef, n: i64) {
        Value::new(slot).write_int(stores, n);
    }

    #[test]
    fn ndcall_reads_back_through_handles() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        let call = new_node(&mut stores, DISC_CALL);
        stores
            .store_mut(&call)
            .set_int(call.rec, call.pos + NDCALL_DEF_NR, 144);

        let args = Value::new(call).call_parameters();
        let s0 = push(&mut stores, args.rec);
        write_int(&mut stores, s0, 7);
        let s1 = push(&mut stores, args.rec);
        write_int(&mut stores, s1, 9);

        let v = Value::new(call);
        assert_eq!(v.value_type(&stores), ValueType::Call);
        assert_eq!(v.call_to(&stores), 144);
        let params = v.call_parameters();
        assert_eq!(params.len(&stores), 2);
        assert_eq!(params.get(0, &stores).value_type(&stores), ValueType::Int);
    }

    #[test]
    fn ndblock_name_and_operators_round_trip() {
        let mut stores = Stores::new();
        let _ids = register_ir_schema(&mut stores);

        // `write_block` pushes the box-of-one `Block`; every reader then takes
        // element 0, so a bare `new_node` + name-set would have nothing to write
        // into.
        let block = Value::new(new_node(&mut stores, DISC_BLOCK));
        block.write_block(&mut stores, "loop_body");

        let ops_vec = block.block_operators(&stores);
        let slot = push(&mut stores, ops_vec.rec);
        write_int(&mut stores, slot, 42);

        assert_eq!(block.value_type(&stores), ValueType::Block);
        assert_eq!(block.block_name(&stores), "loop_body");
        let ops = block.block_operators(&stores);
        assert_eq!(ops.len(&stores), 1);
        assert_eq!(ops.get(0, &stores).value_type(&stores), ValueType::Int);

        block.block_name_set(&mut stores, "x");
        assert_eq!(block.block_name(&stores), "x");
    }

    /// The hard-coded layout must mirror the registered loft schema exactly —
    /// a guard so a schema change can't silently desync the baked consts.
    #[test]
    fn baked_layout_mirrors_loft_schema() {
        use crate::database::Parts;
        let mut stores = Stores::new();
        let ids = register_ir_schema(&mut stores);

        let disc = |tp: u16| {
            if let Parts::EnumValue(d, _) = &stores.get_type(tp).parts {
                *d
            } else {
                0
            }
        };
        // Discriminants — every covered variant.
        assert_eq!(disc(ids.nd_null), DISC_NULL);
        assert_eq!(disc(ids.nd_line), DISC_LINE);
        assert_eq!(disc(ids.nd_span), DISC_SPAN);
        assert_eq!(disc(ids.nd_int), DISC_INT);
        assert_eq!(disc(ids.nd_enum), DISC_ENUM);
        assert_eq!(disc(ids.nd_boolean), DISC_BOOLEAN);
        assert_eq!(disc(ids.nd_float), DISC_FLOAT);
        assert_eq!(disc(ids.nd_long), DISC_LONG);
        assert_eq!(disc(ids.nd_single), DISC_SINGLE);
        assert_eq!(disc(ids.nd_text), DISC_TEXT);
        assert_eq!(disc(ids.nd_call), DISC_CALL);
        assert_eq!(disc(ids.nd_call_ref), DISC_CALL_REF);
        assert_eq!(disc(ids.nd_block), DISC_BLOCK);
        assert_eq!(disc(ids.nd_insert), DISC_INSERT);
        assert_eq!(disc(ids.nd_var), DISC_VAR);
        assert_eq!(disc(ids.nd_set), DISC_SET);
        assert_eq!(disc(ids.nd_return), DISC_RETURN);
        assert_eq!(disc(ids.nd_break), DISC_BREAK);
        assert_eq!(disc(ids.nd_continue), DISC_CONTINUE);
        assert_eq!(disc(ids.nd_if), DISC_IF);
        assert_eq!(disc(ids.nd_loop), DISC_LOOP);
        assert_eq!(disc(ids.nd_drop), DISC_DROP);
        assert_eq!(disc(ids.nd_iter), DISC_ITER);
        assert_eq!(disc(ids.nd_keys), DISC_KEYS);
        assert_eq!(disc(ids.nd_tuple), DISC_TUPLE);
        assert_eq!(disc(ids.nd_tuple_get), DISC_TUPLE_GET);
        assert_eq!(disc(ids.nd_tuple_put), DISC_TUPLE_PUT);
        assert_eq!(disc(ids.nd_yield), DISC_YIELD);
        assert_eq!(disc(ids.nd_fn_ref), DISC_FN_REF);
        assert_eq!(disc(ids.nd_fn_ref_dnr), DISC_FN_REF_DNR);
        assert_eq!(disc(ids.nd_parallel), DISC_PARALLEL);
        assert_eq!(disc(ids.nd_raw_expr), DISC_RAW_EXPR);

        // TypeT discriminants — every variant.
        assert_eq!(disc(ids.ty_unknown), TY_UNKNOWN);
        assert_eq!(disc(ids.ty_null), TY_NULL);
        assert_eq!(disc(ids.ty_void), TY_VOID);
        assert_eq!(disc(ids.ty_never), TY_NEVER);
        assert_eq!(disc(ids.ty_integer), TY_INTEGER);
        assert_eq!(disc(ids.ty_boolean), TY_BOOLEAN);
        assert_eq!(disc(ids.ty_float), TY_FLOAT);
        assert_eq!(disc(ids.ty_single), TY_SINGLE);
        assert_eq!(disc(ids.ty_character), TY_CHARACTER);
        assert_eq!(disc(ids.ty_text), TY_TEXT);
        assert_eq!(disc(ids.ty_keys), TY_KEYS);
        assert_eq!(disc(ids.ty_enum), TY_ENUM);
        assert_eq!(disc(ids.ty_reference), TY_REFERENCE);
        assert_eq!(disc(ids.ty_ref_var), TY_REF_VAR);
        assert_eq!(disc(ids.ty_vector), TY_VECTOR);
        assert_eq!(disc(ids.ty_routine), TY_ROUTINE);
        assert_eq!(disc(ids.ty_iterator), TY_ITERATOR);
        assert_eq!(disc(ids.ty_sorted), TY_SORTED);
        assert_eq!(disc(ids.ty_index), TY_INDEX);
        assert_eq!(disc(ids.ty_radix), TY_RADIX);
        assert_eq!(disc(ids.ty_trie), TY_TRIE);
        assert_eq!(disc(ids.ty_hash), TY_HASH);
        assert_eq!(disc(ids.ty_function), TY_FUNCTION);
        assert_eq!(disc(ids.ty_rewritten), TY_REWRITTEN);
        assert_eq!(disc(ids.ty_tuple), TY_TUPLE);
        assert_eq!(disc(ids.ty_optional), TY_OPTIONAL);

        // Field offsets — every baked offset against its computed position.
        let pos = |tp: u16, f: &str| u32::from(stores.position(tp, f));
        assert_eq!(pos(ids.nd_line, "n"), NDLINE_N);
        assert_eq!(pos(ids.nd_int, "n"), NDINT_N);
        assert_eq!(pos(ids.nd_enum, "ord"), NDENUM_ORD);
        assert_eq!(pos(ids.nd_enum, "tp"), NDENUM_TP);
        assert_eq!(pos(ids.nd_boolean, "b"), NDBOOLEAN_B);
        assert_eq!(pos(ids.nd_float, "f"), NDFLOAT_F);
        assert_eq!(pos(ids.nd_long, "n"), NDLONG_N);
        assert_eq!(pos(ids.nd_single, "f"), NDSINGLE_F);
        assert_eq!(pos(ids.nd_text, "s"), NDTEXT_S);
        assert_eq!(pos(ids.nd_call, "args"), NDCALL_ARGS);
        assert_eq!(pos(ids.nd_call, "def_nr"), NDCALL_DEF_NR);
        assert_eq!(pos(ids.nd_call_ref, "args"), NDCALLREF_ARGS);
        assert_eq!(pos(ids.nd_call_ref, "var"), NDCALLREF_VAR);
        assert_eq!(pos(ids.nd_block, "block"), NDBLOCK_BLOCK);
        assert_eq!(pos(ids.block, "name"), BLOCK_NAME);
        assert_eq!(pos(ids.block, "operators"), BLOCK_OPERATORS);
        assert_eq!(pos(ids.nd_insert, "items"), NDINSERT_ITEMS);
        assert_eq!(pos(ids.nd_var, "n"), NDVAR_N);
        assert_eq!(pos(ids.nd_set, "var"), NDSET_VAR);
        assert_eq!(pos(ids.nd_set, "inner"), NDSET_INNER);
        assert_eq!(pos(ids.nd_return, "inner"), NDRETURN_INNER);
        assert_eq!(pos(ids.nd_break, "n"), NDBREAK_N);
        assert_eq!(pos(ids.nd_continue, "n"), NDCONTINUE_N);
        assert_eq!(pos(ids.nd_if, "cond"), NDIF_COND);
        assert_eq!(pos(ids.nd_if, "t"), NDIF_T);
        assert_eq!(pos(ids.nd_if, "f"), NDIF_F);
        assert_eq!(pos(ids.nd_drop, "inner"), NDDROP_INNER);
        assert_eq!(pos(ids.nd_iter, "var"), NDITER_VAR);
        assert_eq!(pos(ids.nd_iter, "create"), NDITER_CREATE);
        assert_eq!(pos(ids.nd_iter, "next"), NDITER_NEXT);
        assert_eq!(pos(ids.nd_iter, "init"), NDITER_INIT);
        assert_eq!(pos(ids.nd_tuple, "items"), NDTUPLE_ITEMS);
        assert_eq!(pos(ids.nd_tuple_get, "var"), NDTUPLEGET_VAR);
        assert_eq!(pos(ids.nd_tuple_get, "idx"), NDTUPLEGET_IDX);
        assert_eq!(pos(ids.nd_tuple_put, "var"), NDTUPLEPUT_VAR);
        assert_eq!(pos(ids.nd_tuple_put, "idx"), NDTUPLEPUT_IDX);
        assert_eq!(pos(ids.nd_tuple_put, "inner"), NDTUPLEPUT_INNER);
        assert_eq!(pos(ids.nd_yield, "inner"), NDYIELD_INNER);
        assert_eq!(pos(ids.nd_fn_ref_dnr, "n"), NDFNREFDNR_N);
        assert_eq!(pos(ids.nd_parallel, "arms"), NDPARALLEL_ARMS);
        assert_eq!(pos(ids.nd_raw_expr, "s"), NDRAWEXPR_S);

        // Cross-struct inline variants — each absolute offset must equal the
        // sub-struct's base offset (the inlined field's own position) plus the
        // field's offset inside that sub-struct.
        assert_eq!(pos(ids.nd_span, "inner"), NDSPAN_INNER);
        let span_base = pos(ids.nd_span, "pos"); // inlined Position base
        assert_eq!(span_base + pos(ids.position, "line"), SPAN_POS_LINE);
        assert_eq!(span_base + pos(ids.position, "pos"), SPAN_POS_POS);
        assert_eq!(span_base + pos(ids.position, "file"), SPAN_POS_FILE);
        // `NdBlock.block` is a box-of-one HANDLE, so the sub-struct offsets are its
        // own — not a base plus a relative.
        assert_eq!(u32::from(stores.size(ids.block)), BLOCK_STRIDE);

        // `vector<Key>` element layout (the generic RecVector stride + fields).
        assert_eq!(pos(ids.nd_keys, "keys"), NDKEYS_KEYS);
        assert_eq!(u32::from(stores.size(ids.key)), KEY_STRIDE);
        assert_eq!(pos(ids.key, "type_nr"), KEY_TYPE_NR);
        assert_eq!(pos(ids.key, "position"), KEY_POSITION);
        assert_eq!(pos(ids.key, "start"), KEY_START);

        // NdFnRef (carries vector<TypeT>).
        assert_eq!(pos(ids.nd_fn_ref, "def_nr"), NDFNREF_DEF_NR);
        assert_eq!(pos(ids.nd_fn_ref, "var"), NDFNREF_VAR);
        assert_eq!(pos(ids.nd_fn_ref, "t"), NDFNREF_T);

        // TypeT field offsets.
        assert_eq!(pos(ids.ty_unknown, "n"), TYUNKNOWN_N);
        let ispec = pos(ids.ty_integer, "spec"); // inlined IntegerSpec base
        assert_eq!(ispec + pos(ids.integer_spec, "min"), TYINTEGER_MIN);
        assert_eq!(ispec + pos(ids.integer_spec, "max"), TYINTEGER_MAX);
        assert_eq!(
            ispec + pos(ids.integer_spec, "forced_size"),
            TYINTEGER_FORCED
        );
        assert_eq!(
            ispec + pos(ids.integer_spec, "not_null"),
            TYINTEGER_NOT_NULL
        );
        assert_eq!(pos(ids.ty_text, "dep"), TYTEXT_DEP);
        assert_eq!(pos(ids.ty_enum, "n"), TYENUM_N);
        assert_eq!(pos(ids.ty_enum, "is_ref"), TYENUM_IS_REF);
        assert_eq!(pos(ids.ty_enum, "dep"), TYENUM_DEP);
        assert_eq!(pos(ids.ty_reference, "n"), TYREF_N);
        assert_eq!(pos(ids.ty_reference, "dep"), TYREF_DEP);
        assert_eq!(pos(ids.ty_ref_var, "inner"), TYREFVAR_INNER);
        assert_eq!(pos(ids.ty_vector, "inner"), TYVECTOR_INNER);
        assert_eq!(pos(ids.ty_vector, "dep"), TYVECTOR_DEP);
        assert_eq!(pos(ids.ty_routine, "n"), TYROUTINE_N);
        assert_eq!(pos(ids.ty_iterator, "step"), TYITER_STEP);
        assert_eq!(pos(ids.ty_iterator, "inner"), TYITER_INNER);
        assert_eq!(pos(ids.ty_sorted, "n"), TYSORTED_N);
        assert_eq!(pos(ids.ty_sorted, "keys"), TYSORTED_KEYS);
        assert_eq!(pos(ids.ty_sorted, "dep"), TYSORTED_DEP);
        assert_eq!(pos(ids.ty_index, "n"), TYINDEX_N);
        assert_eq!(pos(ids.ty_index, "keys"), TYINDEX_KEYS);
        assert_eq!(pos(ids.ty_index, "dep"), TYINDEX_DEP);
        assert_eq!(pos(ids.ty_radix, "n"), TYRADIX_N);
        assert_eq!(pos(ids.ty_radix, "names"), TYRADIX_NAMES);
        assert_eq!(pos(ids.ty_radix, "dep"), TYRADIX_DEP);
        assert_eq!(pos(ids.ty_trie, "n"), TYTRIE_N);
        assert_eq!(pos(ids.ty_trie, "key"), TYTRIE_KEY);
        assert_eq!(pos(ids.ty_trie, "dep"), TYTRIE_DEP);
        assert_eq!(pos(ids.ty_hash, "n"), TYHASH_N);
        assert_eq!(pos(ids.ty_hash, "names"), TYHASH_NAMES);
        assert_eq!(pos(ids.ty_hash, "dep"), TYHASH_DEP);
        assert_eq!(pos(ids.ty_function, "args"), TYFUNC_ARGS);
        assert_eq!(pos(ids.ty_function, "result"), TYFUNC_RESULT);
        assert_eq!(pos(ids.ty_function, "dep"), TYFUNC_DEP);
        assert_eq!(pos(ids.ty_rewritten, "inner"), TYREWRITTEN_INNER);
        assert_eq!(pos(ids.ty_tuple, "elems"), TYTUPLE_ELEMS);
        assert_eq!(pos(ids.ty_optional, "inner"), TYOPTIONAL_INNER);

        // Non-Node element strides + sub-struct fields.
        assert_eq!(u32::from(stores.size(ids.type_t)), TYPET_STRIDE);
        assert_eq!(u32::from(stores.size(ids.sort_key)), SORTKEY_STRIDE);
        assert_eq!(u32::from(stores.size(ids.name_ref)), NAMEREF_STRIDE);
        assert_eq!(pos(ids.sort_key, "name"), SORTKEY_NAME);
        assert_eq!(pos(ids.sort_key, "asc"), SORTKEY_ASC);
        assert_eq!(pos(ids.name_ref, "name"), NAMEREF_NAME);

        // Block tail (relative to the Block base, like name/operators).
        assert_eq!(pos(ids.block, "result"), BLOCK_RESULT);
        assert_eq!(pos(ids.block, "scope"), BLOCK_SCOPE);
        assert_eq!(pos(ids.block, "var_size"), BLOCK_VAR_SIZE);

        // Attribute record.
        assert_eq!(u32::from(stores.size(ids.attribute)), ATTRIBUTE_STRIDE);
        assert_eq!(pos(ids.attribute, "name"), ATTR_NAME);
        assert_eq!(pos(ids.attribute, "typedef"), ATTR_TYPEDEF);
        assert_eq!(pos(ids.attribute, "value"), ATTR_VALUE);
        assert_eq!(pos(ids.attribute, "check"), ATTR_CHECK);
        assert_eq!(pos(ids.attribute, "check_message"), ATTR_CHECK_MESSAGE);
        assert_eq!(pos(ids.attribute, "links"), ATTR_LINKS);
        assert_eq!(pos(ids.attribute, "mutable"), ATTR_MUTABLE);
        assert_eq!(pos(ids.attribute, "constant"), ATTR_CONSTANT);
        assert_eq!(pos(ids.attribute, "init"), ATTR_INIT);
        assert_eq!(pos(ids.attribute, "nullable"), ATTR_NULLABLE);
        assert_eq!(pos(ids.attribute, "primary"), ATTR_PRIMARY);
        assert_eq!(pos(ids.attribute, "hidden"), ATTR_HIDDEN);
        assert_eq!(pos(ids.attribute, "const_field"), ATTR_CONST_FIELD);
        assert_eq!(pos(ids.attribute, "alias_d_nr"), ATTR_ALIAS_D_NR);
        assert_eq!(
            pos(ids.attribute, "assigned_lambda_d_nr"),
            ATTR_ASSIGNED_LAMBDA_D_NR
        );

        // LinkedFieldGroup record.
        assert_eq!(u32::from(stores.size(ids.linked_field_group)), LFG_STRIDE);
        assert_eq!(pos(ids.linked_field_group, "kind"), LFG_KIND);
        assert_eq!(pos(ids.linked_field_group, "instance"), LFG_INSTANCE);
        assert_eq!(
            pos(ids.linked_field_group, "field_indices"),
            LFG_FIELD_INDICES
        );
        assert_eq!(pos(ids.linked_field_group, "alignment"), LFG_ALIGNMENT);
        assert_eq!(pos(ids.linked_field_group, "size"), LFG_SIZE);

        // Position field offsets (relative; used as a base + relative elsewhere).
        assert_eq!(pos(ids.position, "line"), POS_LINE);
        assert_eq!(pos(ids.position, "pos"), POS_POS);
        assert_eq!(pos(ids.position, "file"), POS_FILE);

        // Variable record.
        assert_eq!(u32::from(stores.size(ids.variable)), VARIABLE_STRIDE);
        assert_eq!(pos(ids.variable, "name"), VAR_NAME);
        assert_eq!(pos(ids.variable, "type_def"), VAR_TYPE_DEF);
        assert_eq!(pos(ids.variable, "stack_pos"), VAR_STACK_POS);
        assert_eq!(pos(ids.variable, "uses"), VAR_USES);
        assert_eq!(pos(ids.variable, "argument"), VAR_ARGUMENT);
        assert_eq!(pos(ids.variable, "stack_allocated"), VAR_STACK_ALLOCATED);
        assert_eq!(pos(ids.variable, "skip_free"), VAR_SKIP_FREE);
        assert_eq!(pos(ids.variable, "captured"), VAR_CAPTURED);
        assert_eq!(
            pos(ids.variable, "caller_hidden_buf"),
            VAR_CALLER_HIDDEN_BUF
        );
        assert_eq!(pos(ids.variable, "owner_witness"), VAR_OWNER_WITNESS);

        // Function record.
        assert_eq!(pos(ids.function, "name"), FN_NAME);
        assert_eq!(pos(ids.function, "file"), FN_FILE);
        assert_eq!(pos(ids.function, "variables"), FN_VARIABLES);
        assert_eq!(pos(ids.function, "names"), FN_NAMES);
        assert_eq!(pos(ids.function, "inline_refs"), FN_INLINE_REFS);

        // NameNr record (element of Function.names).
        assert_eq!(u32::from(stores.size(ids.name_nr)), NAMENR_STRIDE);
        assert_eq!(pos(ids.name_nr, "nr"), NAMENR_NR);
        assert_eq!(pos(ids.name_nr, "name"), NAMENR_NAME);

        // Definition record (inlines Position + Function).
        assert_eq!(u32::from(stores.size(ids.definition)), DEFINITION_STRIDE);
        assert_eq!(pos(ids.definition, "source"), DEF_SOURCE);
        assert_eq!(pos(ids.definition, "def_type"), DEF_DEF_TYPE);
        assert_eq!(pos(ids.definition, "parent"), DEF_PARENT);
        assert_eq!(pos(ids.definition, "position"), DEF_POSITION);
        assert_eq!(pos(ids.definition, "op_code"), DEF_OP_CODE);
        assert_eq!(pos(ids.definition, "known_type"), DEF_KNOWN_TYPE);
        assert_eq!(pos(ids.definition, "closure_record"), DEF_CLOSURE_RECORD);
        assert_eq!(pos(ids.definition, "forced_size"), DEF_FORCED_SIZE);
        assert_eq!(pos(ids.definition, "purity"), DEF_PURITY);
        assert_eq!(pos(ids.definition, "name"), DEF_NAME);
        assert_eq!(pos(ids.definition, "attributes"), DEF_ATTRIBUTES);
        assert_eq!(pos(ids.definition, "code"), DEF_CODE);
        assert_eq!(pos(ids.definition, "returned"), DEF_RETURNED);
        assert_eq!(pos(ids.definition, "rust"), DEF_RUST);
        assert_eq!(pos(ids.definition, "native"), DEF_NATIVE);
        assert_eq!(pos(ids.definition, "variables"), DEF_VARIABLES);
        assert_eq!(
            pos(ids.definition, "mutated_captures"),
            DEF_MUTATED_CAPTURES
        );
        assert_eq!(pos(ids.definition, "scalars_to_box"), DEF_SCALARS_TO_BOX);
        assert_eq!(pos(ids.definition, "bounds"), DEF_BOUNDS);
        assert_eq!(pos(ids.definition, "field_groups"), DEF_FIELD_GROUPS);
        assert_eq!(pos(ids.definition, "synthetic"), DEF_SYNTHETIC);
        assert_eq!(pos(ids.definition, "cap"), DEF_CAP);
        assert_eq!(pos(ids.definition, "c_symbol"), DEF_C_SYMBOL); // @PLN24
        assert_eq!(pos(ids.definition, "c_sig"), DEF_C_SIG); // @PLN24
        assert_eq!(pos(ids.definition, "superseded"), DEF_SUPERSEDED);
        assert_eq!(
            pos(ids.definition, "returned_not_null"),
            DEF_RETURNED_NOT_NULL
        );
        assert_eq!(pos(ids.definition, "pub_visible"), DEF_PUB_VISIBLE);
        assert_eq!(pos(ids.definition, "null_safe"), DEF_NULL_SAFE);

        // Data record (root).
        assert_eq!(pos(ids.data, "source"), DATA_SOURCE);
        assert_eq!(pos(ids.data, "definitions"), DATA_DEFINITIONS);
        assert_eq!(pos(ids.data, "imports"), DATA_IMPORTS);
        assert_eq!(pos(ids.data, "use_names"), DATA_USE_NAMES);
        assert_eq!(u32::from(stores.size(ids.data)), DATA_STRIDE);
        assert_eq!(pos(ids.applied_import, "lib_source"), IMPORT_LIB_SOURCE);
        assert_eq!(pos(ids.applied_import, "into_source"), IMPORT_INTO_SOURCE);
        assert_eq!(pos(ids.applied_import, "name"), IMPORT_NAME);
        assert_eq!(pos(ids.applied_import, "bind"), IMPORT_BIND);
        assert_eq!(u32::from(stores.size(ids.applied_import)), IMPORT_STRIDE);
        assert_eq!(pos(ids.use_name, "name"), USENAME_NAME);
        assert_eq!(pos(ids.use_name, "source"), USENAME_SOURCE);
        assert_eq!(u32::from(stores.size(ids.use_name)), USENAME_STRIDE);

        assert_eq!(u32::from(stores.size(ids.node)), NODE_STRIDE);

        // ── Database type schema (D2a) ──
        assert_eq!(u32::from(stores.size(ids.db_content)), DBCONTENT_STRIDE);
        assert_eq!(disc(ids.dc_long), DC_LONG);
        assert_eq!(disc(ids.dc_float), DC_FLOAT);
        assert_eq!(disc(ids.dc_single), DC_SINGLE);
        assert_eq!(disc(ids.dc_str), DC_STR);
        assert_eq!(pos(ids.dc_long, "v"), DCLONG_V);
        assert_eq!(pos(ids.dc_float, "v"), DCFLOAT_V);
        assert_eq!(pos(ids.dc_single, "v"), DCSINGLE_V);
        assert_eq!(pos(ids.dc_str, "v"), DCSTR_V);

        assert_eq!(u32::from(stores.size(ids.db_field)), DBFIELD_STRIDE);
        assert_eq!(pos(ids.db_field, "content"), DBFIELD_CONTENT);
        assert_eq!(pos(ids.db_field, "position"), DBFIELD_POSITION);
        assert_eq!(pos(ids.db_field, "name"), DBFIELD_NAME);
        assert_eq!(pos(ids.db_field, "default"), DBFIELD_DEFAULT);
        assert_eq!(pos(ids.db_field, "other_indexes"), DBFIELD_OTHER_INDEXES);
        assert_eq!(pos(ids.db_field, "nullable"), DBFIELD_NULLABLE);

        assert_eq!(u32::from(stores.size(ids.enum_pair)), ENUMPAIR_STRIDE);
        assert_eq!(pos(ids.enum_pair, "nr"), ENUMPAIR_NR);
        assert_eq!(pos(ids.enum_pair, "name"), ENUMPAIR_NAME);

        assert_eq!(u32::from(stores.size(ids.key_field)), KEYFIELD_STRIDE);
        assert_eq!(pos(ids.key_field, "nr"), KEYFIELD_NR);
        assert_eq!(pos(ids.key_field, "asc"), KEYFIELD_ASC);

        assert_eq!(u32::from(stores.size(ids.db_parts)), DBPARTS_STRIDE);
        assert_eq!(disc(ids.pt_base), PT_BASE);
        assert_eq!(disc(ids.pt_struct), PT_STRUCT);
        assert_eq!(disc(ids.pt_enum), PT_ENUM);
        assert_eq!(disc(ids.pt_enum_value), PT_ENUM_VALUE);
        assert_eq!(disc(ids.pt_byte), PT_BYTE);
        assert_eq!(disc(ids.pt_short), PT_SHORT);
        assert_eq!(disc(ids.pt_int), PT_INT);
        assert_eq!(disc(ids.pt_short_raw), PT_SHORT_RAW);
        assert_eq!(disc(ids.pt_vector), PT_VECTOR);
        assert_eq!(disc(ids.pt_array), PT_ARRAY);
        assert_eq!(disc(ids.pt_sorted), PT_SORTED);
        assert_eq!(disc(ids.pt_ordered), PT_ORDERED);
        assert_eq!(disc(ids.pt_hash), PT_HASH);
        assert_eq!(disc(ids.pt_index), PT_INDEX);
        assert_eq!(disc(ids.pt_radix), PT_RADIX);
        assert_eq!(disc(ids.pt_trie), PT_TRIE);
        assert_eq!(pos(ids.pt_trie, "content"), PTCONTENT);
        assert_eq!(pos(ids.pt_trie, "key"), PTTRIE_KEY);
        assert_eq!(disc(ids.pt_db_ref), PT_DB_REF);
        assert_eq!(disc(ids.pt_child_rec), PT_CHILD_REC);
        assert_eq!(disc(ids.pt_int_raw), PT_INT_RAW);
        assert_eq!(pos(ids.pt_struct, "fields"), PTSTRUCT_FIELDS);
        assert_eq!(pos(ids.pt_enum, "values"), PTENUM_VALUES);
        assert_eq!(pos(ids.pt_enum_value, "value"), PTENUMVALUE_VALUE);
        assert_eq!(pos(ids.pt_enum_value, "fields"), PTENUMVALUE_FIELDS);
        // The five narrow-integer variants share (start, nullable).
        for v in [
            ids.pt_byte,
            ids.pt_short,
            ids.pt_int,
            ids.pt_short_raw,
            ids.pt_int_raw,
        ] {
            assert_eq!(pos(v, "start"), PTNUM_START);
            assert_eq!(pos(v, "nullable"), PTNUM_NULLABLE);
        }
        // Every content-carrying variant shares `content` at PTCONTENT.
        for v in [
            ids.pt_vector,
            ids.pt_array,
            ids.pt_sorted,
            ids.pt_ordered,
            ids.pt_hash,
            ids.pt_index,
            ids.pt_radix,
            ids.pt_child_rec,
        ] {
            assert_eq!(pos(v, "content"), PTCONTENT);
        }
        for v in [ids.pt_sorted, ids.pt_ordered, ids.pt_index] {
            assert_eq!(pos(v, "keys"), PTKEYS);
        }
        for v in [ids.pt_hash, ids.pt_radix] {
            assert_eq!(pos(v, "fields"), PTFIELDS);
        }
        assert_eq!(pos(ids.pt_index, "left"), PTINDEX_LEFT);

        assert_eq!(u32::from(stores.size(ids.db_type)), DBTYPE_STRIDE);
        assert_eq!(pos(ids.db_type, "size"), DBTYPE_SIZE);
        assert_eq!(pos(ids.db_type, "align"), DBTYPE_ALIGN);
        assert_eq!(pos(ids.db_type, "name"), DBTYPE_NAME);
        assert_eq!(pos(ids.db_type, "parts"), DBTYPE_PARTS);
        assert_eq!(pos(ids.db_type, "keys"), DBTYPE_KEYS);
        assert_eq!(pos(ids.db_type, "field_groups"), DBTYPE_FIELD_GROUPS);
        assert_eq!(pos(ids.db_type, "complex"), DBTYPE_COMPLEX);
        assert_eq!(pos(ids.db_type, "linked"), DBTYPE_LINKED);

        // Bundle root (Data inlined at 0 + the schema vector).
        assert_eq!(u32::from(stores.size(ids.bundle)), BUNDLE_STRIDE);
        assert_eq!(pos(ids.bundle, "data"), BUNDLE_DATA);
        assert_eq!(pos(ids.bundle, "types"), BUNDLE_TYPES);
    }
}
