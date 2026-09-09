// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
// loft#885 — loop-invariant vector headers

//! Which vectors a loop may read through a header derived once, instead of re-deriving
//! it per element (loft#885).
//!
//! Reading `v[i]` resolves the store, loads the container slot and loads the length, and
//! every one of those loads is guarded — so LLVM will not lift them out of the loop even
//! with the whole chain inlined (PERFORMANCE.md § Native vs Rust root cause 3c). The
//! emitter can lift them itself, because it is the one that knows where the loop is; what
//! it needs from here is the promise that makes lifting sound: **nothing the loop body
//! runs can write a store.**
//!
//! A write is what moves a vector: `Store::resize` relocates the record it grows, and
//! `remove` rewrites the length in place. A write to some *other* record cannot reach ours
//! — `claim` takes free space, `delete` touches only free blocks, and reallocating a
//! store's backing buffer leaves record numbers (word offsets into it) unchanged. So the
//! question is not "did anything allocate" but "could anything have written *this* vector",
//! and a body that writes no store at all answers it for every vector at once.
//!
//! [`writes_store`] answers that from an ALLOW-list of ops that provably do not write, so
//! an op missing from it costs the optimisation and never correctness — the inverse of the
//! deny-lists in PERFORMANCE.md § Design: P8, where an omission is a silent wrong read.
//! `LOFT_HOIST_VERIFY=1` is the second half: it emits the checking form of every hoisted
//! read, which re-derives the header and panics on a mismatch, so a hole in the allow-list
//! shows up as a failure under one suite run.

use crate::data::{Block, Data, DefType, Type, Value};
use crate::database::Stores;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The two ops that turn `(vector, index)` into the address of an element. A hoisted
/// header replaces the store resolution + slot load + length load in both.
pub const ELEMENT_ADDRESS_OPS: [&str; 2] = ["OpGetVector", "OpGetVectorNullable"];

/// Zero-parameter ops that produce a constant.
///
/// They have to be named, because the structural rule below requires a parameter to read:
/// an op with no parameters at all is either a constant like these or something that
/// reaches state through the frame (`OpParallelJoin`), and the signature cannot tell them
/// apart. `OpConvIntFromNull` is the one that matters in practice — it initialises the
/// index of a `for` loop, so a nested loop carries it inside its parent's body.
const PURE_NULLARY_OPS: [&str; 15] = [
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
];

/// Ops that take a collection or a reference and only READ it.
///
/// Every other op that can name a store is assumed to write one. That is the safe
/// direction: a reader left out of this list only means a loop that keeps re-deriving its
/// headers. Add to it when a loop that should hoist does not — never to make a loop hoist
/// that a measurement said was slow.
const READ_ONLY_COLLECTION_OPS: [&str; 23] = [
    // the reference's own identity — `store_nr`/`rec` tests that touch no store at all
    // (@PLN157 § V-c: the R1 guard put `OpRefIsNull` in every buffer-building body)
    "OpRefIsNull",
    "OpConvBoolFromRef",
    "OpDistinctStore",
    // element address + length: the vector reads themselves
    "OpGetVector",
    "OpGetVectorNullable",
    "OpLengthVector",
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
];

/// The scalar in-place setters (@PLN157 P4a): each writes one fixed-width value through
/// an address it is GIVEN — the `set_*` family in `Store` — and cannot resize, insert,
/// remove or re-key anything.  No record moves and no collection's length changes, so
/// every hoisted [`crate::vector::VecHeader`] stays valid across one, aliased or not: an
/// in-place write moves nothing, so there is nothing an alias could observe stale.
/// (Hoisted scalar VALUES would be a different question — offset-keyed invalidation —
/// but this pass hoists headers only.)  Deliberately scalar-only: `OpSetRef`/`OpSetDbRef`
/// hand records to owners, `OpSetText` re-allocates, `OpSetKeyed` re-keys — excluded,
/// and the allow-list doctrine holds: an op missing here costs the hoist, never
/// correctness.
pub const IN_PLACE_SET_OPS: [&str; 12] = [
    "OpSetBoolean",
    "OpSetInt",
    "OpSetInt4",
    "OpSetInt4Raw",
    "OpSetCharacter",
    "OpSetSingle",
    "OpSetFloat",
    "OpSetByte",
    "OpSetByteNullable",
    "OpSetShort",
    "OpSetShortRaw",
    "OpSetEnum",
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

/// The vector paths whose header `body` may derive once up front, each with a clone of
/// the operand expression the prelude evaluates.
///
/// Empty when anything in the loop could write a store — except, when `allow_in_place`
/// (the @PLN157 P4a tier, off under `LOFT_NO_WRITE_HOIST=1`), the [`IN_PLACE_SET_OPS`],
/// which cannot invalidate a header — when the body rebinds the path's ROOT variable, or
/// when nothing indexes a vector at all. Order is the order the accesses appear in, so
/// the generated prelude is stable across runs.
pub fn hoistable_vectors(
    body: &Block,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
) -> Vec<(PathKey, Value)> {
    if body_blocks_hoist(body, data, def_nr, cache, allow_in_place) {
        return Vec::new();
    }
    vector_candidates(body, data, def_nr)
}

/// Does anything in `body` invalidate a hoisted header?  The ONE gate both the vector
/// headers and the scalar hoist (@PLN157 P4c) stand behind: a scalar hoist is admitted only
/// in a loop whose store writes are all in place, so the two cannot disagree about which
/// loops qualify.
fn body_blocks_hoist(
    body: &Block,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
) -> bool {
    body.operators.iter().any(|op| {
        blocks_header_hoist(
            op,
            data,
            cache,
            &mut HashSet::new(),
            allow_in_place,
            Some(data.def(def_nr).variables()),
        )
    })
}

/// The variables `body` rebinds — a `Set` or a `TuplePut` anywhere in it.  A rebind leaves
/// the store untouched and still invalidates anything hoisted off that variable, because the
/// hoist describes what the variable named on the way in.
fn rebound_vars(body: &Block) -> HashSet<u16> {
    let mut rebound: HashSet<u16> = HashSet::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Set(v, _) | Value::TuplePut(v, _, _) = n {
                rebound.insert(*v);
            }
            false
        });
    }
    rebound
}

/// The vector paths `body` indexes, in the order they appear, minus those whose root the
/// body rebinds (a field path's header describes the record the ROOT named; repointing the
/// field itself is an `OpSetRef`, which is not in [`IN_PLACE_SET_OPS`] and blocks outright).
fn vector_candidates(body: &Block, data: &Data, def_nr: u32) -> Vec<(PathKey, Value)> {
    let rebound = rebound_vars(body);
    let mut found: Vec<(PathKey, Value)> = Vec::new();
    let vars = data.def(def_nr).variables();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && args.len() == 3
                && is_element_address(data, *d)
                && let Some(path) = vector_path(data, &args[0])
            {
                // A bare var must TYPE as a vector (an odd non-vector shape stays out); a
                // field path is shape-trusted — it is the first argument of an
                // element-address op, which takes a vector.
                let vector_typed =
                    !path.1.is_empty() || matches!(vars.tp(path.0).base(), Type::Vector(_, _));
                if vector_typed && !found.iter().any(|(p, _)| *p == path) {
                    found.push((path, args[0].clone()));
                }
            }
            false
        });
    }
    found.retain(|(p, _)| !rebound.contains(&p.0));
    found
}

/// The typed field getters whose value is a plain `Copy` scalar in the emitted Rust — the
/// reads @PLN157 P4c may hoist.  Each `#rust` template answers its kind's null sentinel at
/// `rec == 0` and a typed load otherwise, and the prelude evaluates that SAME template
/// once, so the hoisted local holds exactly what a per-iteration read would; nothing here
/// needs a per-kind table.  Reference, text and field-address getters stay out — a `DbRef`
/// or a `Str` is a view, not a value.
pub const SCALAR_GETTERS: [&str; 14] = [
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
];

/// A hoisted scalar's key (@PLN157 P4c): the record VARIABLE and the field's byte offset.
pub type ScalarKey = (u16, i64);

/// Recognise `OpGet<scalar>(Var(v), const fld, …)` — the ONE definition of the hoistable
/// scalar read, asked by the collector and by the emitter, so the local the prelude bound
/// and the read the loop body replaces cannot name different fields.  Reports shape only;
/// the caller confirms the variable's type and that the loop hoisted it.
#[must_use]
pub fn scalar_read(getter: &str, args: &[Value]) -> Option<ScalarKey> {
    if !SCALAR_GETTERS.contains(&getter) || args.len() < 2 {
        return None;
    }
    let Value::Var(v) = args[0].unspan() else {
        return None;
    };
    let Value::Int(fld) = args[1].unspan() else {
        return None;
    };
    Some((*v, i64::from(*fld)))
}

/// What a loop body writes in place, by RECORD TYPE (a schema type number) and offset
/// (@PLN157 P4c).  A hoisted scalar `(v, fld)` of a record typed `tp` is stale after a
/// write at `(tp, fld)` through ANY route — the variable itself, a `&`-bound alias, an
/// element view into a `vector<tp>`, a callee's parameter — so the key is the type and the
/// offset, never the variable: aliasing is decided by what a write can reach, and a
/// `vector<integer>` element written at offset 0 reaches no record's field.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct WriteSet {
    /// `(record type, offset)` pairs a scalar setter wrote through a classified target.
    pub offsets: HashSet<(u16, i64)>,
    /// Record types written at offsets this walk cannot see — a return buffer a callee
    /// fills (§ V-c), a record a body frees — so every scalar of the type is stale.
    pub whole: HashSet<u16>,
}

impl WriteSet {
    #[must_use]
    pub fn evicts(&self, tp: u16, fld: i64) -> bool {
        self.whole.contains(&tp) || self.offsets.contains(&(tp, fld))
    }

    fn extend(&mut self, other: &WriteSet) {
        self.offsets.extend(other.offsets.iter().copied());
        self.whole.extend(other.whole.iter().copied());
    }
}

/// The per-callee memo of [`callee_writes`]: `None` is a callee this analysis cannot
/// classify, which keeps every scalar of its callers un-hoisted.
pub type WriteCache = HashMap<u32, Option<Rc<WriteSet>>>;

/// What one loop may derive up front (@PLN157 P4c): the vector headers and the record
/// scalars, decided by ONE gate.
#[derive(Default)]
pub struct LoopHoist {
    /// The vector paths, each with the operand expression the prelude evaluates.
    pub vectors: Vec<(PathKey, Value)>,
    /// The scalar reads, each with the getter call the prelude evaluates once.
    pub scalars: Vec<(ScalarKey, Value)>,
}

/// The vector headers and the record scalars `body` may derive once up front.
///
/// The gate is [`hoistable_vectors`]' gate; a body it declines hoists nothing.  Under it
/// a scalar read `v.fld` (see [`scalar_read`]) of a plain-struct record variable the body
/// does not rebind is hoisted unless the body's [`WriteSet`] — its own in-place setters,
/// what an admitted callee writes, what a free or a return buffer may touch — reaches the
/// record's type at that offset, or the body writes through a target this analysis cannot
/// type (then no scalar hoists).  `scalars` off (`LOFT_NO_SCALAR_HOIST=1`) answers the
/// vector headers alone.
// The eight parameters are the loop, the two things that type it (the IR and the schema),
// the two memos shared across every loop of the program, and the two switches; a struct
// would put a name between each and the one call site without removing anything.
#[allow(clippy::too_many_arguments)]
pub fn hoistable(
    body: &Block,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    allow_in_place: bool,
    scalars: bool,
) -> LoopHoist {
    if body_blocks_hoist(body, data, def_nr, cache, allow_in_place) {
        return LoopHoist::default();
    }
    let mut out = LoopHoist {
        vectors: vector_candidates(body, data, def_nr),
        scalars: Vec::new(),
    };
    if !scalars {
        return out;
    }
    let vars = data.def(def_nr).variables();
    let rebound = rebound_vars(body);
    // (key, record type, the call) in first-appearance order.
    let mut found: Vec<(ScalarKey, u16, Value)> = Vec::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && let Some(key) = scalar_read(data.def(*d).name(), args)
                && !rebound.contains(&key.0)
                && let Some(tp) = plain_record_type(data, vars.tp(key.0))
                && !found.iter().any(|(k, _, _)| *k == key)
            {
                found.push((key, tp, n.clone()));
            }
            false
        });
    }
    if found.is_empty() {
        return out;
    }
    let mut written = WriteSet::default();
    for op in &body.operators {
        let Some(w) = body_writes(op, data, stores, vars, cache, writes, &mut HashSet::new())
        else {
            return out;
        };
        written.extend(&w);
    }
    out.scalars = found
        .into_iter()
        .filter(|(key, tp, _)| !written.evicts(*tp, key.1))
        .map(|(key, _, call)| (key, call))
        .collect();
    out
}

/// The schema type of a variable that names a PLAIN struct record — a `Reference` to a
/// `DefType::Struct`, reached through any `&` links but not through an `Optional`.  A
/// struct-enum, a variant, a nullable and a synthetic `__nullable<S>` answer `None`: their
/// payload offsets are a layout question this key does not model, so they are neither
/// hoisted nor classified as a write target (which keeps every scalar out of that loop).
fn plain_record_type(data: &Data, tp: &Type) -> Option<u16> {
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

/// What an in-place setter's target reaches.
enum Target {
    /// A field of a record of this schema type.
    Record(u16),
    /// An element of a scalar vector — no record field can alias it.
    NoRecord,
    /// A shape this analysis does not type.
    Unknown,
}

/// The record type an in-place setter's first operand addresses (@PLN157 P4c): a record
/// variable, a const field path (`OpGetField` carries the field's schema type), or an
/// element of a vector named by either.  Every other shape is [`Target::Unknown`].
fn setter_target(
    target: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
) -> Target {
    match target.unspan() {
        Value::Var(u) => {
            plain_record_type(data, vars.tp(*u)).map_or(Target::Unknown, Target::Record)
        }
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            if name == "OpGetField" && args.len() == 3 {
                let Value::Int(content) = args[2].unspan() else {
                    return Target::Unknown;
                };
                return match u16::try_from(*content) {
                    Ok(tp) if stores.is_struct(tp) => Target::Record(tp),
                    _ => Target::Unknown,
                };
            }
            if is_element_address(data, *d) && args.len() == 3 {
                return element_target(&args[0], data, stores, vars);
            }
            Target::Unknown
        }
        _ => Target::Unknown,
    }
}

/// The element type of the vector operand of an element-address op, as a [`Target`].
fn element_target(
    vector: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
) -> Target {
    match vector.unspan() {
        Value::Var(w) => match vars.tp(*w).peel_link() {
            Type::Vector(elem, _) => {
                if is_scalar(elem) {
                    Target::NoRecord
                } else {
                    plain_record_type(data, elem).map_or(Target::Unknown, Target::Record)
                }
            }
            _ => Target::Unknown,
        },
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpGetField"
                && args.len() == 3 =>
        {
            let Value::Int(content) = args[2].unspan() else {
                return Target::Unknown;
            };
            let Ok(vec_tp) = u16::try_from(*content) else {
                return Target::Unknown;
            };
            let elem = stores.content(vec_tp);
            if elem == u16::MAX {
                Target::Unknown
            } else if stores.is_struct(elem) {
                Target::Record(elem)
            } else if stores.is_base(elem) {
                Target::NoRecord
            } else {
                Target::Unknown
            }
        }
        _ => Target::Unknown,
    }
}

/// The [`WriteSet`] of a body that passed the hoist gate, or `None` when it writes through
/// something this analysis cannot type.  Walks every call: an [`IN_PLACE_SET_OPS`] setter
/// contributes its target's `(type, offset)`; a record free contributes the record's whole
/// type; a user callee that writes contributes what [`callee_writes`] answers for it; a
/// store-free op contributes nothing.  Anything else — a native writer the gate would have
/// blocked, a `CallRef`, a `Parallel`, a `Yield` — answers `None`, as does a setter whose
/// offset is not a constant.
fn body_writes(
    node: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    active: &mut HashSet<u32>,
) -> Option<WriteSet> {
    let mut set = WriteSet::default();
    let mut ok = true;
    node.any_node(&mut |n| match n {
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                ok = false;
                return true;
            }
            let def = data.def(*d);
            let name = def.name();
            if IN_PLACE_SET_OPS.contains(&name) {
                let fld = args.get(1).map(Value::unspan);
                let Some(Value::Int(fld)) = fld else {
                    ok = false;
                    return true;
                };
                match args
                    .first()
                    .map_or(Target::Unknown, |t| setter_target(t, data, stores, vars))
                {
                    Target::Record(tp) => {
                        set.offsets.insert((tp, i64::from(*fld)));
                    }
                    Target::NoRecord => {}
                    Target::Unknown => {
                        ok = false;
                        return true;
                    }
                }
            } else if RECORD_FREE_OPS.contains(&name) {
                let freed = match args.first().map(Value::unspan) {
                    Some(Value::Var(r)) => plain_record_type(data, vars.tp(*r)),
                    _ => None,
                };
                let Some(tp) = freed else {
                    ok = false;
                    return true;
                };
                set.whole.insert(tp);
            } else if matches!(def.code(), Value::Null) {
                if !native_op_is_store_free(def) {
                    ok = false;
                    return true;
                }
            } else if call_writes_store(*d, data, cache, active) {
                let Some(w) = callee_writes(*d, data, stores, cache, writes, active) else {
                    ok = false;
                    return true;
                };
                set.extend(&w);
            }
            false
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => {
            ok = false;
            true
        }
        _ => false,
    });
    ok.then_some(set)
}

/// What a WRITING callee the hoist gate admits can reach (@PLN157 P4c): an in-place-only
/// writer (§ V-l) answers its own body's [`WriteSet`], with its parameters typed by its own
/// variable table — a write through `l: Lay` is `(Lay, fld)` whichever caller record was
/// passed; a return-buffer writer (§ V-c) answers its buffer's record type WHOLE, because the
/// buffer may be a record the caller offered (§ V-d).  A callee that is neither, or one met
/// while its own verdict is still open (recursion), answers `None`.  Memoised per callee.
fn callee_writes(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    active: &mut HashSet<u32>,
) -> Option<Rc<WriteSet>> {
    if let Some(known) = writes.get(&d_nr) {
        return known.clone();
    }
    let def = data.def(d_nr);
    let answer = if matches!(def.code(), Value::Null) || !active.insert(d_nr) {
        None
    } else {
        let inner = if in_place_only_writer(d_nr, data, cache, active) {
            body_writes(
                def.code(),
                data,
                stores,
                def.variables(),
                cache,
                writes,
                active,
            )
        } else if retbuf_only_writer(d_nr, data, cache, active) {
            def.hidden_return_buffer_attr()
                .and_then(|attr| plain_record_type(data, &def.attributes()[attr].typedef))
                .map(|tp| WriteSet {
                    offsets: HashSet::new(),
                    whole: HashSet::from([tp]),
                })
        } else {
            None
        };
        active.remove(&d_nr);
        inner.map(Rc::new)
    };
    // A `None` reached on a recursive edge is memoised too: it only ever withholds a hoist.
    writes.insert(d_nr, answer.clone());
    answer
}

/// The typed getters an element read can be fused INTO, with the Rust type each reads and
/// the null sentinel its `#rust` template answers at the absent element.
///
/// All three are `if rec != 0 && valid(..) { *addr } else { <sentinel> }` in `Store`, which
/// is what makes one fused load able to stand for the pair. A getter with a different shape
/// — `OpGetBoolean` masks, `OpGetByte` re-bases, `OpGetCharacter` decodes — is left out
/// rather than approximated; it keeps the unfused emission.
const FUSABLE_GETTERS: [(&str, &str, &str); 3] = [
    ("OpGetInt", "i64", "i64::MIN"),
    ("OpGetSingle", "f32", "f32::NAN"),
    ("OpGetFloat", "f64", "f64::NAN"),
];

/// An element read the emitter can collapse into ONE load: a scalar getter reading field
/// `fld` out of `vector[index]`, where the vector is a plain variable.
pub struct FusedRead<'a> {
    /// The vector operand — a `Var` or a pure `OpGetField` chain over one, so it
    /// re-emits without side effects.
    pub vector: &'a Value,
    pub path: PathKey,
    pub size: &'a Value,
    pub index: &'a Value,
    pub fld: &'a Value,
    /// Rust type of the load, e.g. `"f32"`.
    pub rust_type: &'static str,
    /// What the getter answers at an absent element, e.g. `"f32::NAN"`.
    pub absent: &'static str,
}

/// Recognise `OpGet<scalar>(OpGetVector*(Var(v), size, index), fld)`, given the outer
/// getter's op NAME.
///
/// The ONE definition of the fused shape: the emitter and the pre-eval collector both ask
/// here, so the pre-eval cannot hoist an inner read that the emitter then folds away (which
/// would leave the read happening twice). Answers `None` for every other shape — including
/// an indexed read whose vector operand is an expression rather than a variable.
///
/// The caller still has to confirm the vector HAS a hoisted header; this only reports shape.
#[must_use]
pub fn fused_element_read<'a>(
    data: &Data,
    getter: &str,
    args: &'a [Value],
) -> Option<FusedRead<'a>> {
    let [inner, fld] = args else { return None };
    let (_, rust_type, absent) = FUSABLE_GETTERS
        .iter()
        .find(|(name, _, _)| *name == getter)?;
    let Value::Call(elem_op, elem_args) = inner.unspan() else {
        return None;
    };
    if !is_element_address(data, *elem_op) {
        return None;
    }
    let [vector, size, index] = &elem_args[..] else {
        return None;
    };
    let path = vector_path(data, vector)?;
    Some(FusedRead {
        vector,
        path,
        size,
        index,
        fld,
        rust_type,
        absent,
    })
}

/// The typed setters an element write can be fused INTO (@PLN157 P4b), with the Rust
/// type each stores.  The write twins of [`FUSABLE_GETTERS`], excluded for the same
/// reasons: a setter that re-bases (`OpSetByte`/`OpSetShort`), masks or translates
/// keeps the unfused emission.
const FUSABLE_SETTERS: [(&str, &str); 3] = [
    ("OpSetInt", "i64"),
    ("OpSetSingle", "f32"),
    ("OpSetFloat", "f64"),
];

/// An element write the emitter can collapse into ONE store: a scalar setter writing
/// field `fld` of `vector[index]`, where the vector is a plain variable.
pub struct FusedWrite<'a> {
    /// The vector operand — a `Var` or a pure `OpGetField` chain over one, so it
    /// re-emits without side effects.
    pub vector: &'a Value,
    pub path: PathKey,
    pub size: &'a Value,
    pub index: &'a Value,
    pub fld: &'a Value,
    pub val: &'a Value,
    /// Rust type of the store, e.g. `"f64"`.
    pub rust_type: &'static str,
}

/// Recognise `OpSet<scalar>(OpGetVector*(Var(v), size, index), fld, val)`, given the
/// setter's op NAME — the write twin of [`fused_element_read`], and like it the ONE
/// definition of the fused shape: the emitter and the pre-eval collector both ask here,
/// so the pre-eval cannot lift an inner element address the emitter then folds away
/// (which would resolve the element twice).
///
/// The caller still has to confirm the vector HAS a hoisted header; this only reports
/// shape.
#[must_use]
pub fn fused_element_write<'a>(
    data: &Data,
    setter: &str,
    args: &'a [Value],
) -> Option<FusedWrite<'a>> {
    let [inner, fld, val] = args else { return None };
    let (_, rust_type) = FUSABLE_SETTERS.iter().find(|(name, _)| *name == setter)?;
    let Value::Call(elem_op, elem_args) = inner.unspan() else {
        return None;
    };
    if !is_element_address(data, *elem_op) {
        return None;
    }
    let [vector, size, index] = &elem_args[..] else {
        return None;
    };
    let path = vector_path(data, vector)?;
    Some(FusedWrite {
        vector,
        path,
        size,
        index,
        fld,
        val,
        rust_type,
    })
}

/// How many parameters `d_nr` declares, as [`call_writes_store`] counts them.
///
/// Exposed for the test that pins where a native op's parameters live. The verdict "this
/// op cannot name a store" is read off that list, so an empty list is indistinguishable
/// from "takes only scalars" — and every mutator would read as a reader.
#[must_use]
pub fn parameters_declared(data: &Data, d_nr: u32) -> usize {
    if (d_nr as usize) >= data.definitions.len() {
        return 0;
    }
    data.def(d_nr).attributes().len()
}

/// True when `d_nr` is one of the element-address ops a header serves.
#[must_use]
pub fn is_element_address(data: &Data, d_nr: u32) -> bool {
    (d_nr as usize) < data.definitions.len() && ELEMENT_ADDRESS_OPS.contains(&data.def(d_nr).name())
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
    writes_store(node, data, cache, &mut HashSet::new(), None)
}

fn writes_store(
    node: &Value,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    vars: Option<&crate::variables::Function>,
) -> bool {
    blocks_header_hoist(node, data, cache, active, false, vars)
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
const RECORD_FREE_OPS: [&str; 2] = ["OpFreeRef", "OpFreeRefIfDistinct"];

fn frees_a_record(name: &str, args: &[Value], vars: Option<&crate::variables::Function>) -> bool {
    let Some(vars) = vars else { return false };
    RECORD_FREE_OPS.contains(&name)
        && matches!(args.first().map(Value::unspan), Some(Value::Var(v))
            if *v < vars.count()
                && matches!(vars.tp(*v).base(), Type::Reference(_, _) | Type::Enum(_, true, _)))
}

/// Does running `node` invalidate a hoisted header?  [`writes_store`] with one
/// extra allowance: under `allow_in_place`, a direct [`IN_PLACE_SET_OPS`] call is
/// not blocking (its target and value subtrees still walk, so a growing op INSIDE
/// either of them blocks on its own).  A user CALL that writes stays blocking even
/// when its writes happen to be in-place — interprocedural in-place classification
/// is not worth its soundness surface here.
fn blocks_header_hoist(
    node: &Value,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    allow_in_place: bool,
    vars: Option<&crate::variables::Function>,
) -> bool {
    node.any_node(&mut |n| match n {
        Value::Call(d, args) => {
            let known = (*d as usize) < data.definitions.len();
            let in_place_setter =
                known && allow_in_place && IN_PLACE_SET_OPS.contains(&data.def(*d).name());
            let record_free = known
                && crate::keys::retbuf_hoist_enabled()
                && frees_a_record(data.def(*d).name(), args, vars);
            if in_place_setter || record_free {
                false
            } else if call_writes_store(*d, data, cache, active) {
                // @PLN157 § V-l — a USER callee that writes, but only in place: admitted
                // under the same tier as a direct in-place setter, for the same reason
                // (its writes move nothing).  The arguments still walk below this node,
                // so a growing op inside one blocks on its own.
                !(known
                    && allow_in_place
                    && crate::keys::inplace_callee_hoist_enabled()
                    && in_place_only_writer(*d, data, cache, active))
            } else {
                false
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    })
}

fn call_writes_store(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
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
        let inner = writes_store(def.code(), data, cache, active, Some(def.variables()));
        // @PLN157 § V-c — a body whose only writes land in its own scalar return
        // buffer moves no header a caller could have hoisted.
        let inner = inner
            && !(crate::keys::retbuf_hoist_enabled()
                && retbuf_only_writer(d_nr, data, cache, active));
        active.remove(&d_nr);
        inner
    } else {
        true // recursion — the conservative answer rather than a fixed point
    };
    // Safe to memoise either way: a `true` only ever declines a hoist, and a `false` cannot
    // have come from the branch above, since a cycle contributes `true` to every caller.
    cache.insert(d_nr, writes);
    writes
}

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
const IN_PLACE_KEY: u32 = 1 << 31;

fn in_place_only_writer(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
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
        Value::Call(op, _) => {
            if (*op as usize) >= data.definitions.len() {
                return true;
            }
            let callee = data.def(*op);
            if matches!(callee.code(), Value::Null) {
                !(native_op_is_store_free(callee) || IN_PLACE_SET_OPS.contains(&callee.name()))
            } else {
                call_writes_store(*op, data, cache, active)
                    && !in_place_only_writer(*op, data, cache, active)
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    });
    active.remove(&d_nr);
    // A verdict reached while a cycle was open is `false` on the recursive edge only, which
    // never admits a hoist; memoising it is safe either way.
    cache.insert(key, only_in_place);
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
fn retbuf_only_writer(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    let def = data.def(d_nr);
    let Some(attr) = def.hidden_return_buffer_attr() else {
        return false;
    };
    let Some(record) = def.attributes()[attr].typedef.heap_def_nr() else {
        return false;
    };
    if !data
        .def(record)
        .attributes()
        .iter()
        .all(|a| a.constant || matches!(a.typedef, Type::Routine(_)) || is_scalar(&a.typedef))
    {
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
            } else {
                call_writes_store(*op, data, cache, active)
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    })
}

/// Can this native op be ruled out as a writer?
///
/// Two ways to qualify, and everything else is assumed to write:
///
/// * it is named above as a constant or a reader; or
/// * it takes at least one parameter and every parameter is a plain runtime scalar.
///
/// The second is the arithmetic, comparison and conversion bulk. What it turns on is that
/// a **`const` parameter is not a value** — it is a compile-time slot number, type id or
/// field offset, and that is precisely the channel through which the scalar-signature ops
/// that DO touch state reach it: `OpDatabase(pos, db_tp)` allocates a store,
/// `OpCoroutineNext(value_size)` resumes a generator that can append to anything,
/// `OpFreeText(pos)` releases one. Read by signature alone those three are
/// indistinguishable from `OpAddInt`; read this way none of them qualifies.
///
/// Parameters come from `attributes()`. A native op has no body and therefore no variable
/// table, so `variables().arguments()` answers the empty list — which "are they all
/// scalar?" accepts, turning every mutator into a reader. That is not a hypothetical: it
/// is how the first cut of this gate let `v.remove(0)` run inside a hoisted loop.
/// `parameters_declared` is the guard that keeps it from coming back.
fn native_op_is_store_free(def: &crate::data::Definition) -> bool {
    if PURE_NULLARY_OPS.contains(&def.name()) || READ_ONLY_COLLECTION_OPS.contains(&def.name()) {
        return true;
    }
    !def.attributes().is_empty()
        && def
            .attributes()
            .iter()
            .all(|a| !a.constant && is_scalar(&a.typedef))
}

/// True for the types that cannot name a store. Anything else — a reference, a collection,
/// text, a tuple, an iterator, an unresolved type — counts as one that can.
fn is_scalar(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Enum(_, false, _)
    )
}
