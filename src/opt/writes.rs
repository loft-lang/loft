// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The WRITER analysis: may a statement write a store, and so move a record or change a
//! collection's length?  A hoisted vector header, a held element address and a fused push
//! stay valid only across statements this answers "no" for ([`may_write_store`]); the op
//! tables, the store paths and the mint targets here are what it reads.

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
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
    /// Set ONLY by [`hoistable`] for the loop's header verdict, never by the emitter's
    /// [`Output`](super::Output) tiers: a text set claims and frees text records, which moves
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
/// type each stores.  The write twins of [`FUSABLE_GETTERS`], excluded for the same
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
/// that spells the pushed value casts it to the kind's type ([`push_value_cast`]).
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
