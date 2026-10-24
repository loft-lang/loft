// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The WRITER analysis: may a statement write a store, and so move a record or change a
//! collection's length?  A hoisted vector header, a held element address and a fused push
//! stay valid only across statements this answers "no" for ([`may_write_store`]); the op
//! tables, the store paths and the mint targets here are what it reads.

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use crate::opt::loop_records::{IN_PLACE_SET_OPS, all_scalar_record, is_scalar};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Zero-parameter ops that produce a constant.
///
/// They have to be named, because the structural rule below requires a parameter to read:
/// an op with no parameters at all is either a constant like these or something that
/// reaches state through the frame (`OpParallelJoin`), and the signature cannot tell them
/// apart. `OpConvIntFromNull` is the one that matters in practice — it initialises the
/// index of a `for` loop, so a nested loop carries it inside its parent's body.
pub(crate) const PURE_NULLARY_OPS: [&str; 19] = [
    "OpConvIntFromNull",
    "OpConvBoolFromNull",
    "OpConvCharacterFromNull",
    "OpConvSingleFromNull",
    "OpConvFloatFromNull",
    "OpConvTextFromNull",
    "OpConvEnumFromNull",
    "OpConvRefFromNull",
    "OpNullRefSentinel",
    "OpConstTrue",
    "OpConstFalse",
    "OpMathPiFloat",
    "OpMathEFloat",
    // The libm dispatchers (@PLN157 P4a): pure scalar math whose `const` first
    // parameter is a FUNCTION SELECTOR (`9` = sqrt), not a slot or type id — the
    // one const-param shape the signature rule's rationale does not cover.  Left
    // off this list they read as writers, and a loop calling `sin`/`sqrt`
    // through `seed_wave`-style helpers never hoisted at all.
    "OpMathFuncFloat",
    "OpMathFuncSingle",
    // Their two-argument siblings (`atan2`, `log(x, base)`): the same selector shape.
    // Missing here, one `atan2` on a cold branch of a callee declined the hoist of
    // every loop that reached it, transitively (hex_roof's `roof_match`).
    "OpMathFunc2Float",
    "OpMathFunc2Single",
    // `sizeof` of a scalar expression: consumes the value, answers the `const` size.
    "OpSizeScalar",
    // The checked narrowing into a ranged integer (`z: integer(lo, hi)`): its `const`
    // parameters are the BOUNDS and the default, not a slot or type id, and its one effect
    // outside its value is a recoverable-error report through the logger — no store is
    // touched.  Missing here, one ranged local in a callee (random's `get`) declined the
    // hoist of every loop that called it (`indices` −43 % priced).
    "OpRangeDefault",
];

/// Ops that take a collection or a reference and only READ it.
///
/// Every other op that can name a store is assumed to write one. That is the safe
/// direction: a reader left out of this list only means a loop that keeps re-deriving its
/// headers. Add to it when a loop that should hoist does not — never to make a loop hoist
/// that a measurement said was slow.
pub(crate) const READ_ONLY_COLLECTION_OPS: [&str; 39] = [
    "OpVectorIsNull",
    // the reference's own identity — `store_nr`/`rec` tests that touch no store at all
    // (@PLN157 § V-c: the R1 guard put `OpRefIsNull` in every buffer-building body), and
    // the copy of one (@PLN164 B1b: the entry witness snapshots every promoted buffer);
    // `v != null` / `v == null` on a record view compare two `DbRef`s the same way (the
    // mesh3d `mesh_to_floats` loop declined every header on that test alone)
    "OpRefIsNull",
    "OpConvBoolFromRef",
    "OpDistinctStore",
    "OpRefAlias",
    "OpEqRef",
    "OpNeRef",
    // element address + length: the vector reads themselves
    "OpGetVector",
    "OpGetVectorNullable",
    "OpLengthVector",
    // every other collection's element count and byte footprint: each template reads
    // through `&s.database.allocations`, a shared borrow, so it cannot write (`len(h)` in
    // a loop otherwise declined every header the loop could hold)
    "OpLengthSorted",
    "OpLengthHash",
    "OpLengthIndex",
    "OpLengthSpatial",
    "OpLengthTrie",
    "OpSizeVector",
    "OpSizeHash",
    "OpSizeIndex",
    "OpSizeSpatial",
    "OpSizeTrie",
    // a record's byte size is the compile-time constant; the reference is not read
    "OpSizeStruct",
    // typed field reads through a reference
    "OpGetInt",
    "OpGetInt4",
    "OpGetInt4Raw",
    "OpGetInt4Full",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetShortFull",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetSingle",
    "OpGetFloat",
    "OpGetBoolean",
    "OpGetCharacter",
    "OpGetEnum",
    "OpGetRef",
    "OpGetField",
    "OpGetDbRef",
    // a text field read through a reference: the position word, then the text at it
    // (`@FR-R-TextBorrow` — what let a walk of texts hoist nothing)
    "OpGetText",
];

/// A hoist key (@PLN157 P4d): a vector reached from a local through zero or more CONST
/// field offsets — `v` is `(var, [])`, `lay.best` is `(var, [8])`.  Pure by
/// construction (`OpGetField` is a reader), so the prelude may evaluate the path once
/// and a fused fallback may re-emit it; the bare-`Var` restriction this lifts only ever
/// guarded IMPURE operands, which cannot form a path.
pub type PathKey = (u16, Vec<i64>);

/// The path for a vector operand, or `None` when it is not a `Var` or a
/// `OpGetField(path, const fld, const tp)` chain over one.
#[must_use]
/// Enforces `@FR-R-Header`: the pure path a header is keyed on.
pub fn vector_path(data: &Data, v: &Value) -> Option<PathKey> {
    match v.unspan() {
        Value::Var(var) => Some((*var, Vec::new())),
        Value::Call(d, args)
            if args.len() == 3
                && (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpGetField" =>
        {
            let Value::Int(off) = args[1].unspan() else {
                return None;
            };
            let (root, mut offs) = vector_path(data, &args[0])?;
            offs.push(i64::from(*off));
            Some((root, offs))
        }
        _ => None,
    }
}

/// `(R-Mint)`'s rebound clause — what the EMITTER owns the mint or copy of in the current
/// function, so the loop-hoist admission can let those statements through: they emit as a
/// fresh store, a length reset, or nothing at all, and move no record a header describes.
#[derive(Clone, Default, Debug)]
pub struct HoistOwned {
    /// The § V-al loop buffers and the § V-z element-first witnesses: `OpDatabase(b, …)` on
    /// one is a fresh store or its own reset (or, for a witness, never emitted), and a local
    /// bound from `OpGetField(b, …)` names a fresh root or a fresh element's slot.
    pub buffers: HashSet<u16>,
    /// The § V-z paired `(element, field offset)`s: the `OpAppendVector(OpGetField(e, off),
    /// tmp, tp)` copy into one is emitted as NOTHING — the temp was built in the slot.
    pub elem_fields: HashSet<(u16, i64)>,
    /// The `(R-LoopRecord)` locals: `OpDatabase(r, tp)` on one is the kept record's first
    /// mint or a re-establishment of its defaults — a write of r's whole TYPE and nothing
    /// else, so a scalar hoisted off that type is evicted and every other one stands.
    pub records: HashSet<u16>,
    /// The `(R-ValueLocal)` locals the emitter carries as TUPLES: a copy FROM one is one
    /// typed setter per field at the destination (`OpCopyRecordEmitter`), never a block copy.
    pub value_locals: HashSet<u16>,
    /// The DEAD buffers (`dead_buffers`): a value-record call's result buffer that is never
    /// minted — its `OpDatabase`, `OpClear` and frees are emitted as nothing.
    pub dead_buffers: HashSet<u16>,
    /// The same fact for EVERY function of the program, so a CALLEE's body is judged with
    /// its own dead buffers exempt (`call_writes_store`): a program-wide table, shared.
    pub dead_by_fn: Rc<HashMap<u32, HashSet<u16>>>,
    /// The elements `(R-RefillText)`'s collection clause builds into KEPT slots: a mint there
    /// reuses a slot that still owns its previous text, so a text written into it releases
    /// that text — no first write, and `@FR-R-Mint`'s text clause does not take it.
    pub kept_elems: HashSet<u16>,
}

impl HoistOwned {
    /// The emitter-owned facts of callee `d_nr`, as its own body's walk needs them: its
    /// dead buffers, and the table to answer the same for what it calls.
    #[must_use]
    pub fn for_callee(&self, d_nr: u32) -> HoistOwned {
        HoistOwned {
            dead_buffers: self.dead_by_fn.get(&d_nr).cloned().unwrap_or_default(),
            dead_by_fn: Rc::clone(&self.dead_by_fn),
            ..HoistOwned::default()
        }
    }
}

/// The generation-time tiers of the hoist family — one flag per admitted rewrite, each an
/// `LOFT_NO_*` environment switch read once by `Output::new`.  Grouped so the gate, the
/// collector and the emitter read ONE value and cannot disagree about which tiers are on.
// Four bools by design: each IS one independently switchable rewrite tier (R-Switch), and a
// substructure would only put a name between a switch and its rule.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Default)]
pub struct HoistTiers {
    /// `(R-InPlace)` — in-place scalar sets admitted (`LOFT_NO_WRITE_HOIST` off).
    pub in_place: bool,
    /// `(R-Scalar)` — record scalars hoisted (`LOFT_NO_SCALAR_HOIST` off).
    pub scalars: bool,
    /// `(R-Push)` — fusable pushes admitted under a push header (`LOFT_NO_PUSH_HOIST` off).
    pub push: bool,
    /// `(R-Mint)` — the record mint group admitted as a mover (`LOFT_NO_MINT_HOIST` off).
    pub mint: bool,
    /// `(R-PushRec)` — an admitted mint whose element is a plain no-heap struct EMITS
    /// through a push header of its own (`LOFT_NO_RECORD_PUSH` off).  A refinement of
    /// `mint`: it changes what the group's ops emit, never whether the loop hoists.
    pub record_push: bool,
    /// `(R-PushRec)`'s heap clause — an admitted mint whose element OWNS heap emits through
    /// the header too, its slot zeroed at the mint (`LOFT_NO_HEAP_RECORD_PUSH` off).
    pub heap_push: bool,
    /// `(R-Mint)`'s rebound clause — a mover the body REBINDS to a fresh buffer or element
    /// slot takes no holder instead of declining the loop (`LOFT_NO_REBOUND_MOVER` off).
    pub rebound_movers: bool,
    /// `(R-Header)`'s text clause — a text set (`OpSetText`, `OpSetTextReplace`) admitted.
    /// Set ONLY by `hoistable` for the loop's header verdict, never by the native emitter's
    /// tiers (`Output::hoist_tiers`): a text set claims and frees text records, which moves
    /// no vector's record or length but may grow its store, so a reader of the gate that
    /// holds an ADDRESS (a record view's pointer, a push header) must keep declining it.
    pub text_set: bool,
}

/// `@FR-O-LazyBuffer` — the record-buffer pool's mint, which `scopes::reuse_record_buffers`
/// places behind `OpRefIsNull` on the buffer itself: it only ever takes a FRESH store from
/// the null sentinel, never clears one, so no header a loop holds can name the store it
/// fills.  Nor is it a growth for `@FR-R-Base`: a new store is a new slot whose memory is
/// its own allocation (`Store::ptr`), so no live store's memory moves, even when the slot
/// table itself grows.  `LOFT_HOIST_VERIFY=1` re-checks every base at every use.
pub(crate) fn lazy_buffer_mint(
    name: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    let Some(vars) = vars else { return false };
    (name == "OpDatabase" || name == "OpDatabaseNP")
        && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
            if *b < vars.count() && vars.is_lazy_buffer(*b))
}

/// The schema type of a variable that names a PLAIN struct record — a `Reference` to a
/// `DefType::Struct`, reached through any `&` links but not through an `Optional`.  A
/// struct-enum, a variant, a nullable and a synthetic `__nullable<S>` answer `None`: their
/// payload offsets are a layout question this key does not model, so they are neither
/// hoisted nor classified as a write target (which keeps every scalar out of that loop).
pub(crate) fn plain_record_type(data: &Data, tp: &Type) -> Option<u16> {
    if matches!(tp, Type::Optional(_)) {
        return None;
    }
    let Type::Reference(d, _) = tp.peel_link() else {
        return None;
    };
    if data.def_type(*d) != DefType::Struct {
        return None;
    }
    let known = data.def(*d).known_type();
    (known != u16::MAX).then_some(known)
}

/// The typed setters an element write can be fused INTO (@PLN157 P4b), with the Rust
/// type each stores.  The write twins of `FUSABLE_GETTERS`, excluded for the same
/// reasons: a setter that re-bases (`OpSetByte`/`OpSetShort`), masks or translates
/// keeps the unfused emission.
/// The scalar pushes a loop may hoist a header for (@PLN157 § V-q, `@FR-R-Push`): the op,
/// the Rust type of the value, and the element width, one [`HoistScalar`] kind each; a
/// push of another kind keeps its template AND blocks the loop, as every growth does,
/// because only a push emitted through the refreshing helper leaves the header current.
///
/// `OpPushBoolean` and `OpPushEnum` are the byte kind at bias 0 — their templates are
/// `append_byte(r, v)`, which is `append_byte_min(r, 0, v)` — and `Store::byte_raw(0, v)`
/// is `v` for every value a native `u8` holds (`byte_fits(0, v)` spans 0..=255, the enum
/// null 255 included), so the element is the value itself and no bias rides along
/// ([`push_operands`] answers none).  Their native value may be a `bool`, so every site
/// that spells the pushed value casts it to the kind's type (`push_value_cast`).
///
/// [`HoistScalar`]: crate::vector::HoistScalar
pub const FUSABLE_PUSHES: [(&str, &str, u32); 8] = [
    ("OpPushByte", "u8", 1),
    ("OpPushBoolean", "u8", 1),
    ("OpPushEnum", "u8", 1),
    ("OpPushInt4", "i32", 4),
    ("OpPushCharacter", "u32", 4),
    ("OpPushInt", "i64", 8),
    ("OpPushSingle", "f32", 4),
    ("OpPushFloat", "f64", 8),
];

/// The operands of a fused push: the vector, the byte kind's `min` bias, the value.  Every
/// kind is `(vector, value)` except `OpPushByte(vector, min, value)`, whose element is the
/// ENCODED byte (`Store::byte_raw`) — so every site that reads a push's value asks here,
/// and none can take the unencoded value for the one kind that has a bias.
#[must_use]
pub fn push_operands<'a>(
    name: &str,
    args: &'a [Value],
) -> Option<(&'a Value, Option<&'a Value>, &'a Value)> {
    match (name, args) {
        ("OpPushByte", [vector, min, val]) => Some((vector, Some(min), val)),
        (_, [vector, val]) if name != "OpPushByte" => Some((vector, None, val)),
        _ => None,
    }
}

/// A push the emitter can route through a hoisted [`crate::vector::PushHeader`].
pub struct FusedPush<'a> {
    /// The pushed path's key.
    pub path: PathKey,
    /// The vector operand, emitted for the growth step's runtime append.
    pub vector: &'a Value,
    /// The value pushed.
    pub val: &'a Value,
    /// The byte kind's `min` bias: the element is `Store::byte_raw(min, val)`.
    pub bias: Option<&'a Value>,
    /// The Rust type of the value.
    pub rust_type: &'static str,
    /// The element width in bytes.
    pub size: u32,
}

/// Recognise `OpPreAllocVector(path, count, size)` over a pure path (@PLN157 § V-q): the
/// reservation the parser emits before a push to a LOCAL vector.  It claims a record only
/// for an ABSENT vector and never moves an existing one, so it cannot stale a header; with a
/// push header active for the path it is redundant (the push grows on demand) and is
/// emitted as nothing.
#[must_use]
pub fn pre_alloc_path(data: &Data, op: &str, args: &[Value]) -> Option<PathKey> {
    if op != "OpPreAllocVector" || args.len() != 3 {
        return None;
    }
    vector_path(data, &args[0])
}

/// Recognise `OpKeepRange(P, lo, hi, tp)` over a pure path (`@FR-R-Refresh`'s keep-range
/// clause) — the ONE definition of the admissible self-slice pop, asked by the gate, the
/// collector and the emitter.  The op keeps the vector's record and writes its length, so
/// it is a LENGTH refresh of P's holder, which the emitter performs at the op's site; the
/// path takes a push header (a mutable holder) and joins the push paths' aliasing
/// decision, since a second holder naming the same vector would keep the old length.
/// Shape only; `None` for every other op, and under `LOFT_NO_KEEP_RANGE_REFRESH`.
#[must_use]
pub fn keep_range_path(data: &Data, op: &str, args: &[Value]) -> Option<PathKey> {
    if op != "OpKeepRange" || args.len() != 4 || !crate::keys::keep_range_refresh_enabled() {
        return None;
    }
    vector_path(data, &args[0])
}

/// Recognise `OpPush<Kind>(path, val)` for a fusable kind over a pure path (@PLN157 § V-q)
/// — the ONE definition of the hoistable push, asked by the gate, the collector and the
/// emitter.  Shape only; the caller confirms the loop hoisted a push header for the path.
#[must_use]
pub fn fused_push<'a>(data: &Data, op: &str, args: &'a [Value]) -> Option<FusedPush<'a>> {
    let (_, rust_type, size) = FUSABLE_PUSHES.iter().find(|(n, _, _)| *n == op)?;
    let (vector, bias, val) = push_operands(op, args)?;
    let path = vector_path(data, vector)?;
    Some(FusedPush {
        path,
        vector,
        val,
        bias,
        rust_type,
        size: *size,
    })
}

/// The plain vector one op of a record MINT group appends to (`@FR-R-Mint`): the path its
/// holder is keyed on, the operand expression that names the vector's `DbRef`, and the
/// vector's schema type (whose content is the element).
pub struct MintTarget {
    pub path: PathKey,
    pub vector: Value,
    pub vector_tp: u16,
}

/// Recognise one op of the record MINT group — `OpNewRecord(P, tp, fld)` /
/// `OpFinishRecord(P, elm, tp, fld)` appending one element to a PLAIN vector (@PLN157
/// § V-s, `@FR-R-Mint`) — the ONE definition of the admissible mint, asked by the gate,
/// the collector, the write-set walk and the emitters.  The append has two spellings and
/// both are one notion:
///
/// - a vector named by a BARE variable — `v += […]` is `OpNewRecord(v, vector_tp, MAX)`;
/// - a vector that is a FIELD of a plain record — `m.verts += […]` is
///   `OpNewRecord(m, parent_tp, fld)`, the parent a pure path to the record and `fld` the
///   field that holds the vector.  Its path is the parent's extended by the field's
///   position, which is the key a read of `m.verts` (`OpGetField(m, pos, tp)`) already
///   has, so the append and the reads share ONE holder (`@FR-R-State`).
///
/// The container must TYPE as a plain vector (the V-m lesson: ask the type, not the op
/// shape) — on a keyed collection (`sorted`, `index`, `hash`, …) the same op names are a
/// KEYED INSERT, a mover of OTHER records, and stay blocking; a field form asks the
/// SCHEMA ([`Stores::plain_vector_field`]), which also declines a linked group's member,
/// whose finish hands the record to its siblings.  `peel_link` and not `base()`: a nullable
/// vector's append is not this unit's shape, so `τ?` stays out — and the field form's root
/// must name a plain record for the same reason (a nullable element's payload is reached
/// through an op that forms no path).  Everything else answers `None`: the mint keeps its
/// template and blocks the loop, as every unclassified store writer does.
#[must_use]
pub fn mint_target(
    data: &Data,
    stores: &Stores,
    op: &str,
    args: &[Value],
    vars: &crate::variables::Function,
) -> Option<MintTarget> {
    let arity = match op {
        "OpNewRecord" => 3,
        "OpFinishRecord" => 4,
        _ => return None,
    };
    if args.len() != arity {
        return None;
    }
    let (Some(Value::Int(tp)), Some(Value::Int(fld))) = (
        args.get(arity - 2).map(Value::unspan),
        args.get(arity - 1).map(Value::unspan),
    ) else {
        return None;
    };
    let tp = u16::try_from(*tp).ok()?;
    let (root, mut offs) = vector_path(data, &args[0])?;
    if root >= vars.count() {
        return None;
    }
    if offs.is_empty() && matches!(vars.tp(root).peel_link(), Type::Vector(_, _)) {
        return Some(MintTarget {
            path: (root, offs),
            vector: args[0].clone(),
            vector_tp: tp,
        });
    }
    if *fld == i32::from(u16::MAX) || !mint_field_enabled() {
        return None;
    }
    plain_record_type(data, vars.tp(root))?;
    let (pos, vector_tp) = stores.plain_vector_field(tp, u16::try_from(*fld).ok()?)?;
    offs.push(i64::from(pos));
    let get_field = data.def_nr("OpGetField");
    (get_field != u32::MAX).then(|| MintTarget {
        path: (root, offs),
        vector: Value::Call(
            get_field,
            vec![
                args[0].clone(),
                Value::Int(i32::from(pos)),
                Value::Int(i32::from(vector_tp)),
            ],
        ),
        vector_tp,
    })
}

/// [`mint_target`]'s path alone — what the gate and the write-set walk ask.
#[must_use]
pub fn mint_path(
    data: &Data,
    stores: &Stores,
    op: &str,
    args: &[Value],
    vars: &crate::variables::Function,
) -> Option<PathKey> {
    mint_target(data, stores, op, args, vars).map(|t| t.path)
}

/// `@FR-R-Mint`'s field clause is on — `LOFT_NO_FIELD_MINT=1` keeps a record append to a
/// vector FIELD on its template (`@FR-R-Switch`).  Read at generation time.
pub(crate) fn mint_field_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_FIELD_MINT").is_ok_and(|v| v != "0"))
}

/// @PLN157 § V-c — the frees whose operand is a RECORD variable of the enclosing body.
///
/// A free releases exactly one store and moves no other, and the store a hoisted header
/// describes belongs to a loop-invariant vector that is live across the loop — so a
/// record's release cannot be it.  The operand's TYPE is what carries the argument: a
/// vector-typed operand (a per-iteration vector local, or a vector work-ref paired by
/// loft#1201) keeps the writer verdict, and so does a free whose body this cannot see
/// (`vars == None`).  `OpFreeRefIfDistinct(v, w)` compares `v` against a witness and
/// releases `v` alone, so only its first operand is the question.
pub(crate) const RECORD_FREE_OPS: [&str; 2] = ["OpFreeRef", "OpFreeRefIfDistinct"];

/// @PLN157 § V-l — does this def write stores ONLY through [`IN_PLACE_SET_OPS`]?
///
/// The P4a argument, one call deep: a scalar set through an address it is given moves no
/// record and changes no length, so every header a CALLER hoisted stays valid across the
/// call, whatever the address — an element of a vector reached through a parameter, a
/// record's field, the callee's own local.  Everything else the callee runs must be
/// store-free: a native op that is neither store-free nor one of those setters (a growth,
/// a free, an `OpDatabase`, a text or reference set), a user callee that is neither
/// store-free nor in-place-only itself, and a `CallRef` / `Parallel` / `Yield` (what runs
/// is not this body) each keep the writer verdict; so does recursion, conservatively.
/// The `composite` row's `set_pixel` — three scalar field reads, one element address, one
/// `set_int` through it — is the shape.  Memoised beside [`call_writes_store`]'s answers
/// under [`IN_PLACE_KEY`]; `LOFT_HOIST_VERIFY=1` is the falsifier.
pub(crate) const IN_PLACE_KEY: u32 = 1 << 31;

/// `@FR-R-TextBorrow`'s text-value clause — the ops that write a text VARIABLE named by
/// their `pos: const u16` operand, or release one.  On native that variable is a Rust
/// `String` (an owned local, a `&mut String` work buffer, the `*mut String` a local link
/// holds) and its release is nothing at all (`text_ops.rs` emits `OpFreeText` as no code),
/// so none of these touches a store: a header, a base or a borrowed element survives every
/// one.  A writer whose `pos` names a RECORD (`OpSetText`) is not here — it takes a
/// reference and allocates in that reference's store.
pub(crate) const TEXT_VARIABLE_OPS: [&str; 16] = [
    "OpAppendText",
    "OpAppendCharacter",
    "OpPutText",
    "OpClearText",
    "OpFreeText",
    "OpFormatText",
    "OpFormatInt",
    "OpFormatSingle",
    "OpFormatFloat",
    "OpAppendStackText",
    "OpAppendStackCharacter",
    "OpClearStackText",
    "OpFormatStackText",
    "OpFormatStackInt",
    "OpFormatStackSingle",
    "OpFormatStackFloat",
];

/// `@FR-R-InPlace`'s copy clause — `OpCopyRecord(src, dst, tp)` with no flag set, over a
/// record type that owns NO heap: `size(tp)` bytes stored at the address of `dst`, which is
/// what an in-place scalar set is at a larger width.  Nothing is claimed, released or moved
/// — the claim walks a heap-owning type needs (`remove_claims`, `copy_claims`) are exactly
/// what `owns_heap` rules out — so every header, base and record address a loop holds stays
/// valid across it.  A flagged copy frees its source store (`COPY_FREE_SOURCE`) or marks a
/// fresh destination, and stays a store writer.  Answers the record type, which a hoisted
/// scalar's write set takes WHOLE.  The write-back idiom is this op: `e = v[i]?; …;
/// v[i] = e` copies the view onto the place it views, which the runtime makes a no-op.
#[must_use]
pub fn in_place_copy(stores: Option<&Stores>, op: &str, args: &[Value]) -> Option<u16> {
    if op != "OpCopyRecord" || args.len() != 3 || !in_place_copy_enabled() {
        return None;
    }
    let Value::Int(tp) = args[2].unspan() else {
        return None;
    };
    let tp = u16::try_from(*tp).ok()?;
    if tp & !crate::keys::COPY_TP_MASK != 0 {
        return None;
    }
    let stores = stores?;
    (stores.is_struct(tp) && !stores.owns_heap(tp)).then_some(tp)
}

/// `@FR-R-InPlace`'s read clause — `OpReadFile(f, OpCreateStack(t), n, tp)` into a SCALAR
/// local writes that local and the File record's own cursor fields (`#next`, `#pos`, the
/// handle number on first use), each in place: nothing is claimed, grown or freed.  A read
/// into a text, a vector or a record builds or fills a value in a store and stays a writer.
/// The scalar tier types it as a write of the File record's type (`body_writes`).
pub(crate) fn scalar_file_read(
    data: &Data,
    op: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    op == "OpReadFile"
        && matches!(args.get(1).map(Value::unspan), Some(Value::Call(c, a))
            if (*c as usize) < data.definitions.len()
                && scalar_stack_ref(data.def(*c).name(), a, vars))
}

/// The address of a SCALAR local — `OpCreateStack(t)`, the operand the parser wraps a
/// by-reference hand-off in — writes nothing; whatever writes through it is judged on its own.
pub(crate) fn scalar_stack_ref(
    op: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    op == "OpCreateStack"
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t))
            if vars.is_some_and(|v| is_scalar(v.tp(*t))))
}

/// The address of a TEXT local (`OpCreateStack(t)`, a `&text` hand-off such as a callee's
/// output text) writes nothing a header describes: on `--native` a text local is a Rust
/// `String`, outside every store, and the link is its address.  Whatever writes through it is
/// judged on its own.
pub(crate) fn text_stack_ref(
    op: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    op == "OpCreateStack"
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t))
            if vars.is_some_and(|v| is_text_shape(v.tp(*t))))
}

/// A text, seen through `τ?` and any `&` link.
pub(crate) fn is_text_shape(tp: &Type) -> bool {
    matches!(tp.peel_link(), Type::Text(_))
}

/// `@FR-R-InPlace`'s link clause — the address of a local naming a record that owns NO heap
/// (`&rows[i]` handed to `fn f(p: &(integer, integer))`) writes nothing either: through it a
/// callee can only write the record's scalars in place or rebind the local, and neither claims,
/// grows or frees.  What the callee writes is judged by its own summary.  A record that owns a
/// text or a collection stays a writer — a callee writing it through the link re-claims.
pub(crate) fn record_stack_ref(
    stores: Option<&Stores>,
    data: &Data,
    op: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    op == "OpCreateStack"
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t))
        if vars.zip(stores).is_some_and(|(v, st)| {
            plain_record_type(data, v.tp(*t))
                .is_some_and(|tp| st.is_struct(tp) && !st.owns_heap(tp))
        }))
}

/// `LOFT_NO_COPY_IN_PLACE=1` — a no-heap record copy is a store writer again
/// (`@FR-R-Switch`).  Read at generation time.
pub(crate) fn in_place_copy_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_COPY_IN_PLACE").is_ok_and(|v| v != "0"))
}

/// Could running `node` write any store?
///
/// Descends through called functions, so a loop calling a stdlib reader (`len(v)` is
/// `t_6vector_len`, whose whole body is `OpLengthVector`) still qualifies. A recursion
/// cycle, a call through a runtime fn-ref, a parallel arm and a `yield` all answer yes —
/// the first because the fixed point is not worth computing here, the rest because what
/// runs is not this body.
///
/// `cache` memoises per definition. A `true` may have come from a broken recursion cycle
/// and is still sound to reuse (it only declines a hoist); a `false` cannot have, because
/// a cycle contributes `true` and any caller of it answers `true` too.
pub fn may_write_store(node: &Value, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
    writes_store(node, data, cache, &mut HashSet::new(), None, None)
}

pub(crate) fn writes_store(
    node: &Value,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    vars: Option<&crate::variables::Function>,
    owned: Option<&HoistOwned>,
) -> bool {
    blocks_header_hoist(
        node,
        data,
        None,
        cache,
        active,
        vars,
        HoistTiers::default(),
        &mut HashSet::new(),
        owned,
    )
}

/// @PLN157 § V-ad — `OpDatabase`/`OpDatabaseNP` into a hidden null-discharge buffer
/// (`__ref_p2_N`, the record `e = tbl[i]?` mints an ABSENT element into): the allocation
/// takes a store of its own from a null slot, or clears the buffer's OWN store — a store
/// only the site's `__ncc_N` temp reaches, and the site rebinds that temp on every run, so
/// no loop-invariant path names anything in it and no header or base can go stale, whatever
/// the record holds; a scalar hoist can be reached only through the buffer's own type, which
/// the caller reads as written whole.  Answers that type.  (Until 2026-09-22 an all-scalar
/// record was required — the `Chunk { …, hexes: vector<Hex> }` a game's chunk lookup
/// discharges kept its loop on no header at all; a growth of the buffer's vector field from
/// the body is a push through a non-pure path, which the gate declines on its own.)  Only
/// the pass-2 discharge buffers qualify: a `__ref_N` work-ref may be a return buffer, and a
/// return buffer may be a record the caller offered.
/// Enforces `@FR-R-InPlace` (the hidden-buffer allowance).
pub(crate) fn null_buffer_alloc(
    name: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
    data: &Data,
) -> Option<u16> {
    if !crate::keys::null_buffer_hoist_enabled()
        || !(name == "OpDatabase" || name == "OpDatabaseNP")
    {
        return None;
    }
    let vars = vars?;
    let Some(Value::Var(b)) = args.first().map(Value::unspan) else {
        return None;
    };
    if *b >= vars.count() || !vars.name(*b).starts_with("__ref_p2_") {
        return None;
    }
    plain_record_type(data, vars.tp(*b))
}

/// § V-ad's discharge buffer, WRITTEN: a set of any kind (`OpSet*` — a text field's
/// `OpSetText` included, which is not an in-place scalar set) whose target is the pass-2
/// discharge buffer `null_buffer_alloc` admits the mint of.  The buffer's store is private
/// to its site — only the site's `__ncc_N` temp reaches it, rebound per use — so a write
/// into it moves nothing a header, base or address describes, in the caller's loop and in
/// an admitted callee alike; the fallback record's text field (`Sequence { …, q_name: "" }`)
/// was the one op keeping `frame_of` a writing callee.  Answers the buffer's record type.
/// Enforces `@FR-R-InPlace` (the hidden-buffer allowance) and `@FR-R-Callee`.
pub(crate) fn discharge_buffer_set(
    name: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
    data: &Data,
) -> Option<u16> {
    if !name.starts_with("OpSet") || !crate::keys::null_buffer_hoist_enabled() {
        return None;
    }
    let vars = vars?;
    let Some(Value::Var(b)) = args.first().map(Value::unspan) else {
        return None;
    };
    if *b >= vars.count() || !vars.name(*b).starts_with("__ref_p2_") {
        return None;
    }
    plain_record_type(data, vars.tp(*b))
}

pub(crate) fn frees_a_record(
    name: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    let Some(vars) = vars else { return false };
    RECORD_FREE_OPS.contains(&name)
        && matches!(args.first().map(Value::unspan), Some(Value::Var(v))
            if *v < vars.count()
                && matches!(vars.tp(*v).base(), Type::Reference(_, _) | Type::Enum(_, true, _)))
}

/// What a callee's body may run and still be an in-place-only or return-buffer-only writer,
/// beside its own sets: the header hoist's own allowances — a null-discharge buffer's mint,
/// a lazy buffer's, a record free (it moves nothing), and a DEAD buffer's mint, clear or
/// free.  Asked of one native op; `(R-Callee)`.
pub(crate) fn callee_allowance(
    name: &str,
    args: &[Value],
    vars: &crate::variables::Function,
    data: &Data,
    owned: Option<&HoistOwned>,
) -> bool {
    null_buffer_alloc(name, args, Some(vars), data).is_some()
        || discharge_buffer_set(name, args, Some(vars), data).is_some()
        || lazy_buffer_mint(name, args, Some(vars))
        || (crate::keys::retbuf_hoist_enabled() && frees_a_record(name, args, Some(vars)))
        || dead_buffer_op(name, args, owned)
}

/// `(R-ValueRecord)` — a DEAD buffer's mint, clear and free are emitted as nothing: the
/// callee answers a tuple and the buffer serves no call.
pub(crate) fn dead_buffer_op(name: &str, args: &[Value], owned: Option<&HoistOwned>) -> bool {
    matches!(
        name,
        "OpDatabase" | "OpDatabaseNP" | "OpClear" | "OpFreeRef" | "OpFreeRefIfDistinct"
    ) && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
        if owned.is_some_and(|o| o.dead_buffers.contains(b)))
}

/// `@FR-R-RecPtr`'s mint clause — a copy INTO the fresh element of the window being judged
/// writes that element's own bytes and moves nothing; its free-source flag releases the
/// builder's buffer, which no held address names.
pub(crate) fn fresh_copy(name: &str, args: &[Value], fresh: &HashSet<u16>) -> bool {
    name == "OpCopyRecord"
        && args.len() == 3
        && matches!(args[1].unspan(), Value::Var(e) if fresh.contains(e))
}

/// `@FR-R-Mint`'s text clause — the first text written into an element the body has just
/// minted (`e.f = t` between `OpNewRecord` and `OpFinishRecord`): a claim in the store the
/// mint itself claims in, so it can move nothing the admitted mint cannot, and the mint's
/// aliasing decision covers it.  The slot holds no text yet, so nothing is released.
pub(crate) fn fresh_text_set(name: &str, args: &[Value], fresh: &HashSet<u16>) -> bool {
    name == "OpSetText"
        && args.len() == 3
        && fresh_text_set_enabled()
        && matches!(args[0].unspan(), Value::Var(e) if fresh.contains(e))
}

/// The text-set operators `(R-Header)`'s text clause admits: each writes one text slot of
/// the record it is given, claiming the new text's record and freeing the old one in that
/// record's store.  Neither changes a vector's record or length, so a held header survives.
pub(crate) fn is_text_set(name: &str) -> bool {
    matches!(name, "OpSetText" | "OpSetTextReplace")
}

/// `LOFT_NO_FRESH_TEXT_SET=1` — a text written into a just-minted element blocks the loop's
/// header hold again.
pub(crate) fn fresh_text_set_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_FRESH_TEXT_SET").is_ok_and(|v| v != "0"))
}

/// A write into the fresh element that moves nothing a held header describes: § V-d's delivery
/// copy (under the in-place tier) or its first text (`@FR-R-Mint`'s text clause, the mint tier).
pub(crate) fn fresh_write(
    name: &str,
    args: &[Value],
    fresh: &HashSet<u16>,
    tiers: HoistTiers,
    owned: Option<&HoistOwned>,
) -> bool {
    (tiers.in_place && fresh_copy(name, args, fresh))
        || (tiers.mint
            && fresh_text_set(name, args, fresh)
            && !matches!(args[0].unspan(), Value::Var(e)
                if owned.is_some_and(|o| o.kept_elems.contains(e))))
}

/// The address of a scalar local, or (`@FR-R-Mint`'s text clause) of a text local.
pub(crate) fn local_stack_ref(
    op: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
) -> bool {
    scalar_stack_ref(op, args, vars) || (fresh_text_set_enabled() && text_stack_ref(op, args, vars))
}

/// Does running `node` invalidate a hoisted header?  [`writes_store`] with one
/// extra allowance: under `allow_in_place`, a direct [`IN_PLACE_SET_OPS`] call is
/// not blocking (its target and value subtrees still walk, so a growing op INSIDE
/// either of them blocks on its own).  A user CALL that writes stays blocking even
/// when its writes happen to be in-place — interprocedural in-place classification
/// is not worth its soundness surface here.
/// Enforces `@FR-R-InPlace` and admits a callee under `@FR-R-Callee`.
// The eight parameters are the statement, the two things that type it (the IR and the
// variables), the two memos, the tier switches, the fresh-element window and the emitter's
// owned buffers; a struct would put a name between each and the five call sites without
// removing anything.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blocks_header_hoist(
    node: &Value,
    data: &Data,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    vars: Option<&crate::variables::Function>,
    tiers: HoistTiers,
    fresh: &mut HashSet<u16>,
    owned: Option<&HoistOwned>,
) -> bool {
    node.any_node(&mut |n| match n {
        // @PLN157 § V-s (`@FR-R-Mint`) — track the element variables the body has minted
        // so far, in the same preorder the write-set walk uses (see `body_writes` for why
        // that order is the execution order of the group).
        Value::Set(v, inner) => {
            if tiers.mint
                && matches!(inner.unspan(), Value::Call(d, args)
                    if (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpNewRecord"
                        && vars.zip(stores).is_some_and(|(vs, st)| {
                            mint_path(data, st, "OpNewRecord", args, vs).is_some()
                        }))
            {
                fresh.insert(*v);
            } else {
                fresh.remove(v);
            }
            false
        }
        Value::Call(d, args) => {
            let known = (*d as usize) < data.definitions.len();
            let in_place_setter = known
                && tiers.in_place
                && (IN_PLACE_SET_OPS.contains(&data.def(*d).name())
                    || scalar_file_read(data, data.def(*d).name(), args, vars)
                    || local_stack_ref(data.def(*d).name(), args, vars)
                    || record_stack_ref(stores, data, data.def(*d).name(), args, vars));
            let record_free = known
                && crate::keys::retbuf_hoist_enabled()
                && frees_a_record(data.def(*d).name(), args, vars);
            // @PLN157 § V-ad — the null-discharge buffer's allocation moves nothing a header
            // describes; its field sets below are in-place and walk on their own.
            // `(R-Mint)`'s rebound clause — so is the mint of a buffer the EMITTER owns: a
            // § V-al loop buffer (a fresh store, or a length reset of its own) and a § V-z
            // element-first witness (never minted: the temp binds the element's slot).
            // Neither moves a record another store's header or base describes, and the
            // local bound from it is REBOUND in the body, so nothing hoists off it.
            // …and the mint of the frame's OWN hidden return buffer: guarded, it runs only
            // where the caller offered no record (`Parser::build_into_return_buffer`), and
            // then claims a fresh store — no record a header or base describes moves.
            let own_retbuf_mint = known
                && matches!(data.def(*d).name(), "OpDatabase" | "OpDatabaseNP")
                && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
                    if vars.is_some_and(|vs| vs.is_argument(*b)
                        && (vs.name(*b) == "__retbuf" || vs.name(*b).starts_with("__ref_"))));
            let buffer_alloc = known
                && (dead_buffer_op(data.def(*d).name(), args, owned)
                    || own_retbuf_mint
                    || null_buffer_alloc(data.def(*d).name(), args, vars, data).is_some()
                    || discharge_buffer_set(data.def(*d).name(), args, vars, data).is_some()
                    || lazy_buffer_mint(data.def(*d).name(), args, vars)
                    || (tiers.rebound_movers
                        && matches!(data.def(*d).name(), "OpDatabase" | "OpDatabaseNP")
                        && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
                            if owned.is_some_and(|o| o.buffers.contains(b) || o.records.contains(b)))));
            // …and the § V-z paired copy into the element's field slot, which is emitted as
            // nothing: the temp was built in that slot.
            let elided_copy = known
                && tiers.rebound_movers
                && data.def(*d).name() == "OpAppendVector"
                && matches!(args.first().map(Value::unspan), Some(Value::Call(gd, gargs))
                    if (*gd as usize) < data.definitions.len()
                        && data.def(*gd).name() == "OpGetField"
                        && matches!((gargs.first().map(Value::unspan), gargs.get(1).map(Value::unspan)),
                            (Some(Value::Var(e)), Some(Value::Int(off)))
                                if owned.is_some_and(|o| o.elem_fields.contains(&(*e, i64::from(*off))))));
            // @PLN157 § V-q (`@FR-R-Push`) — a fusable push over a pure path is admitted
            // under its own tier: it grows one vector whose header the loop keeps current
            // through the push itself; `hoistable` decides the aliasing.  The value operand
            // still walks below this node.
            let fusable_push = known
                && tiers.push
                && (fused_push(data, data.def(*d).name(), args).is_some()
                    || pre_alloc_path(data, data.def(*d).name(), args).is_some()
                    // `@FR-R-Refresh`'s keep-range clause — a self-slice pop keeps the
                    // record and refreshes the held length at its own site; it takes the
                    // push tier's holder and aliasing decision.  Its bound operands still
                    // walk below this node.
                    || keep_range_path(data, data.def(*d).name(), args).is_some());
            // @PLN157 § V-s (`@FR-R-Mint`) — a record MINT into a plain vector is a mover
            // like a push: it grows that one vector and initialises a record no variable
            // bound before the loop can name; `hoistable` decides the aliasing.  The
            // element's field writes and the builder call still walk below this node.
            // § V-d's delivery tail — `OpCopyRecord(result, elm, tp)` when the builder
            // could not build in place — writes the fresh element and frees the builder's
            // own this-iteration buffer, neither of which a prelude holder can name; it is
            // admitted only while its DESTINATION is a fresh mint variable.
            let record_mint = known
                && tiers.mint
                && vars.zip(stores).is_some_and(|(v, st)| {
                    mint_path(data, st, data.def(*d).name(), args, v).is_some()
                });
            let fresh_delivery = known
                && tiers.mint
                && data.def(*d).name() == "OpCopyRecord"
                && args.len() == 3
                && matches!(args[1].unspan(), Value::Var(e) if fresh.contains(e));
            // `@FR-R-InPlace`'s copy clause — a flag-free copy of a no-heap record is an
            // in-place write at a larger width; its operands still walk below this node.
            let record_copy = known
                && tiers.in_place
                && in_place_copy(stores, data.def(*d).name(), args).is_some();
            let fresh_copy = known && fresh_write(data.def(*d).name(), args, fresh, tiers, owned);
            if record_mint
                && data.def(*d).name() == "OpFinishRecord"
                && let Some(Value::Var(e)) = args.get(1).map(Value::unspan)
            {
                fresh.remove(e);
            }
            if in_place_setter
                || record_free
                || buffer_alloc
                || elided_copy
                || fusable_push
                || record_mint
                || fresh_delivery
                || record_copy
                || fresh_copy
                // `(R-Header)`'s text clause: the target and value still walk below.
                || (known && tiers.text_set && is_text_set(data.def(*d).name()))
            {
                false
            } else if call_writes_store(*d, data, cache, active, owned) {
                // @PLN157 § V-l — a USER callee that writes, but only in place: admitted
                // under the same tier as a direct in-place setter, for the same reason
                // (its writes move nothing).  The arguments still walk below this node,
                // so a growing op inside one blocks on its own.
                !(known
                    && tiers.in_place
                    && crate::keys::inplace_callee_hoist_enabled()
                    && in_place_only_writer(*d, data, cache, active, owned))
            } else {
                false
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    })
}

pub(crate) fn call_writes_store(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    owned: Option<&HoistOwned>,
) -> bool {
    if (d_nr as usize) >= data.definitions.len() {
        return true;
    }
    if let Some(known) = cache.get(&d_nr) {
        return *known;
    }
    let def = data.def(d_nr);
    let writes = if matches!(def.code(), Value::Null) {
        !native_op_is_store_free(def)
    } else if active.insert(d_nr) {
        // The callee's OWN dead buffers are exempt in its walk (`(R-ValueRecord)`: a tuple
        // answer's buffer is never minted), read off the program-wide table.
        let callee_owned = owned.map(|o| o.for_callee(d_nr));
        let inner = writes_store(
            def.code(),
            data,
            cache,
            active,
            Some(def.variables()),
            callee_owned.as_ref(),
        );
        // @PLN157 § V-c — a body whose only writes land in its own scalar return
        // buffer moves no header a caller could have hoisted.
        let inner = inner
            && !(crate::keys::retbuf_hoist_enabled()
                && retbuf_only_writer(d_nr, data, cache, active, callee_owned.as_ref()));
        active.remove(&d_nr);
        inner
    } else {
        true // recursion — the conservative answer rather than a fixed point
    };
    // A `false` is final however it was reached (a cycle contributes `true`, so none comes
    // from that branch); a `true` reached WITHOUT the emitter's facts (no dead buffer
    // exempt) may be pessimistic and is not memoised, so a later ask with them can answer.
    if !writes || owned.is_some() {
        cache.insert(d_nr, writes);
    }
    writes
}

/// Enforces `@FR-R-Callee` (the in-place-only half).
pub(crate) fn in_place_only_writer(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    owned: Option<&HoistOwned>,
) -> bool {
    let key = d_nr | IN_PLACE_KEY;
    if let Some(known) = cache.get(&key) {
        return *known;
    }
    let def = data.def(d_nr);
    if matches!(def.code(), Value::Null) || !active.insert(d_nr) {
        return false;
    }
    let only_in_place = !def.code().any_node(&mut |n| match n {
        Value::Call(op, args) => {
            if (*op as usize) >= data.definitions.len() {
                return true;
            }
            let callee = data.def(*op);
            if matches!(callee.code(), Value::Null) {
                !(native_op_is_store_free(callee)
                    || IN_PLACE_SET_OPS.contains(&callee.name())
                    || scalar_file_read(data, callee.name(), args, Some(def.variables()))
                    || scalar_stack_ref(callee.name(), args, Some(def.variables()))
                    || callee_allowance(callee.name(), args, def.variables(), data, owned))
            } else {
                let inner = owned.map(|o| o.for_callee(*op));
                call_writes_store(*op, data, cache, active, inner.as_ref())
                    && !in_place_only_writer(*op, data, cache, active, inner.as_ref())
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    });
    active.remove(&d_nr);
    // A verdict reached while a cycle was open is `false` on the recursive edge only, which
    // never admits a hoist.  A `false` reached WITHOUT the emitter's facts (no dead buffer
    // exempt) may be pessimistic and is not memoised; a `true`, or any verdict with them, is.
    if only_in_place || owned.is_some() {
        cache.insert(key, only_in_place);
    }
    only_in_place
}

/// @PLN157 § V-c — does this def write nothing but fixed-width scalars into its own
/// hidden return buffer?
///
/// Such a call cannot invalidate a header a caller hoisted: a scalar set through an
/// address moves nothing and changes no length (the `IN_PLACE_SET_OPS` argument, one call
/// deep); `OpDatabase` on the buffer allocates from a null slot or clears the buffer's
/// OWN store, and a store is its own allocation; and the buffer's store hosts no hoisted
/// header, because the record is ALL-SCALAR — no collection, text or reference field —
/// and a loop never names its buffer variable.  Every miss keeps the writer verdict: a
/// record with a vector field (it grows), a write to any other place (a parameter, a
/// local), a native op that is neither store-free nor one of those setters, a user call
/// that writes, and a `CallRef` / `Parallel` / `Yield` (what runs is not this body).
/// Enforces `@FR-R-Callee` (the return-buffer half).
pub(crate) fn retbuf_only_writer(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    owned: Option<&HoistOwned>,
) -> bool {
    let def = data.def(d_nr);
    let Some(attr) = def.hidden_return_buffer_attr() else {
        return false;
    };
    let Some(record) = def.attributes()[attr].typedef.heap_def_nr() else {
        return false;
    };
    if !all_scalar_record(data, record) {
        return false;
    }
    let buf = def.variables().var(&def.attributes()[attr].name);
    if buf == u16::MAX {
        return false;
    }
    !def.code().any_node(&mut |n| match n {
        Value::Call(op, args) => {
            if (*op as usize) >= data.definitions.len() {
                return true;
            }
            let callee = data.def(*op);
            if matches!(callee.code(), Value::Null) {
                if native_op_is_store_free(callee) {
                    return false;
                }
                let name = callee.name();
                let into_buffer =
                    matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == buf);
                !(into_buffer && (name == "OpDatabase" || IN_PLACE_SET_OPS.contains(&name)))
                    && !callee_allowance(name, args, def.variables(), data, owned)
            } else {
                let inner = owned.map(|o| o.for_callee(*op));
                call_writes_store(*op, data, cache, active, inner.as_ref())
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    })
}

/// Can this native op be ruled out as a writer?
///
/// Three ways to qualify, and everything else is assumed to write:
///
/// * it is named above as a constant, a reader or a text-variable writer; or
/// * it takes at least one parameter, every parameter is a plain runtime scalar or a text
///   VALUE, and its result is one too, or nothing (`@FR-R-TextBorrow`'s text-value
///   clause: a text value on native is a `&str` or a `String`, never a store — but a
///   result that is a collection is a store the op minted, `list_dir(path) ->
///   vector<text>` being one, which is why the result is asked).
///
/// The second is the arithmetic, comparison, conversion and text bulk. What it turns on is
/// that a **`const` parameter is not a value** — it is a compile-time slot number, type id or
/// field offset, and that is precisely the channel through which the scalar-signature ops
/// that DO touch state reach it: `OpDatabase(pos, db_tp)` allocates a store,
/// `OpCoroutineNext(value_size)` resumes a generator that can append to anything,
/// `OpCastVectorFromText(val, db_tp)` mints one. Read by signature alone those are
/// indistinguishable from `OpAddInt`; read this way none of them qualifies, and the
/// text-variable writers that share the `pos` spelling are admitted by NAME above.
///
/// Parameters come from `attributes()`. A native op has no body and therefore no variable
/// table, so `variables().arguments()` answers the empty list — which "are they all
/// scalar?" accepts, turning every mutator into a reader. That is not a hypothetical: it
/// is how the first cut of this gate let `v.remove(0)` run inside a hoisted loop.
/// `parameters_declared` is the guard that keeps it from coming back.
pub(crate) fn native_op_is_store_free(def: &crate::data::Definition) -> bool {
    if PURE_NULLARY_OPS.contains(&def.name())
        || READ_ONLY_COLLECTION_OPS.contains(&def.name())
        || TEXT_VARIABLE_OPS.contains(&def.name())
    {
        return true;
    }
    let value = |tp: &Type| is_scalar(tp) || matches!(tp.base(), Type::Text(_));
    !def.attributes().is_empty()
        && def
            .attributes()
            .iter()
            .all(|a| !a.constant && value(&a.typedef))
        && (matches!(def.returned(), Type::Void) || value(def.returned()))
}
