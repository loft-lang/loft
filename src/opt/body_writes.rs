// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The WRITE SET of a body: which `(type, offset)` places a loop or a function may write
//! ([`body_writes`], `@FR-R-Scalar`), and which locals it rebinds.  A hoisted scalar read
//! stays valid across a body whose write set does not cover it; the value-record analysis
//! asks the same of a callee.

use crate::data::{Block, Data, Type, Value};
use crate::database::{Parts, Stores};
use crate::ir_shape::call_named;
use crate::opt::loop_records::{IN_PLACE_SET_OPS, is_scalar};
use crate::opt::writes::{
    FUSABLE_PUSHES, HoistOwned, RECORD_FREE_OPS, call_writes_store, discharge_buffer_set,
    in_place_copy, in_place_only_writer, lazy_buffer_mint, mint_path, native_op_is_store_free,
    null_buffer_alloc, plain_record_type, retbuf_only_writer, scalar_file_read, scalar_stack_ref,
    vector_path,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The two ops that turn `(vector, index)` into the address of an element. A hoisted
/// header replaces the store resolution + slot load + length load in both.
pub const ELEMENT_ADDRESS_OPS: [&str; 2] = ["OpGetVector", "OpGetVectorNullable"];

/// The variables `body` rebinds — a `Set` or a `TuplePut`, and the RE-MINT of a hidden
/// pass-2 buffer (`OpDatabase`/`OpDatabaseNP` on a `__ref_p2_N`: a literal handed to a
/// call, a `?`-discharge's absent record), which the IR spells as a bare call while it
/// clears the buffer's store and claims a fresh record, so every path rooted at the buffer
/// names another record after it — closed over LINKS: a `&`-bound local reads whatever its
/// target holds NOW, so a rebind of the target repoints every path rooted at the link, and
/// the link counts as rebound too ([`close_over_links`]).  The one home of the question
/// every hoist asks of its root; a site that asks it of a single variable uses
/// [`rebinds_root`].  (Measured without the mint clause, once `(R-InPlace)`'s hidden-buffer
/// allowance took a heap-holding record: a push header hoisted off the literal buffer's
/// vector field before the loop, the buffer re-minted per pass, the element lost —
/// `1575-…`'s c3 read `null(oob)` for `1`.)
pub(crate) fn rebound_vars(
    body: &Block,
    data: &Data,
    vars: &crate::variables::Function,
) -> HashSet<u16> {
    let mut rebound: HashSet<u16> = HashSet::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Set(v, _) | Value::TuplePut(v, _, _) = n {
                rebound.insert(*v);
            }
            // A `__ref_p2_` buffer as the first operand of anything but its mint (a field
            // set, the call it is handed to) is a use, not a rebind.
            if let Some(args) =
                call_named(n, data, "OpDatabase").or_else(|| call_named(n, data, "OpDatabaseNP"))
                && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                && *b < vars.count()
                && vars.name(*b).starts_with("__ref_p2_")
            {
                rebound.insert(*b);
            }
            false
        });
    }
    close_over_links(&mut rebound, vars);
    rebound
}

/// The variables `v` LINKS to: a `g = &e` local's type is a `RefVar` whose deps name its
/// target, and a link to a link follows through.  Empty for a variable that is no link.
pub(crate) fn link_targets(vars: &crate::variables::Function, v: u16) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    let mut todo = vec![v];
    while let Some(x) = todo.pop() {
        if x >= vars.count() || !matches!(vars.tp(x).base(), Type::RefVar(_)) {
            continue;
        }
        for d in vars.tp(x).depend() {
            if d != v && !out.contains(&d) {
                out.push(d);
                todo.push(d);
            }
        }
    }
    out
}

/// Add to `rebound` every link whose target — or whose target's target — is in it.  A
/// header, a base, an address or a memo hoisted off `g.tags` with `g = &e` describes the
/// record `e` named on the way in; once the body rebinds `e`, `g` reads another record
/// while the hoisted fact still describes the first (measured 2026-09-22: `link_view`
/// answered 15 for 16 on native, with every switch on or off).
pub(crate) fn close_over_links(rebound: &mut HashSet<u16>, vars: &crate::variables::Function) {
    loop {
        let mut grew = false;
        for v in 0..vars.count() {
            if !rebound.contains(&v) && link_targets(vars, v).iter().any(|t| rebound.contains(t)) {
                rebound.insert(v);
                grew = true;
            }
        }
        if !grew {
            return;
        }
    }
}

/// `@FR-R-RecPtr`'s path clause — the record VIEW a scalar field access goes through and the
/// byte offset of the field from the view's first byte.  The access has two spellings and
/// both are one notion: `r.f` is `OpGet<K>(Var(r), f)`, and `r.pos.x` — a field reached
/// through INLINE sub-records — is `OpGet<K>(OpGetField(Var(r), off(pos), _), off(x))`.
/// `OpGetField` adds a constant to the `DbRef`'s position and leaves its record alone, so
/// the field is the view's at `off(pos) + off(x)`, and the null view stays null through
/// the chain.  A base that is anything else — an element address, a pointer field
/// (`OpGetDbRef`), a call — forms no path and answers `None`.
///
/// For the record ADDRESS only: a read through it loads the bytes where they are each
/// time, so no aliasing question arises.  `(R-Scalar)`'s hoisted scalars keep
/// `scalar_read`'s bare-variable key on purpose — a hoist CACHES a value keyed by
/// (record type, offset), and a write through a sub-record view (`p = v.pos; p.x = …`)
/// is typed by the sub-record, which would never evict a key typed by the parent.
#[must_use]
pub fn view_field(data: &Data, base: &Value, fld: &Value) -> Option<(u16, i64)> {
    let Value::Int(fld) = fld.unspan() else {
        return None;
    };
    let (root, offs) = vector_path(data, base)?;
    Some((root, offs.iter().sum::<i64>() + i64::from(*fld)))
}

/// A SUB-RECORD of a record variable, as an `OpGetField` chain names one: the root
/// variable, the summed byte offset the sub-record starts at, and its schema type (the
/// last `OpGetField`'s content).  `v.pos` on a record carried as a tuple is this: the
/// sub-record is a RANGE of the tuple, which is what a hand-off to a tuple parameter and a
/// copy FROM it read (`(R-ValueLocal)`).  A bare variable is not a sub-record.
#[must_use]
pub fn sub_record(data: &Data, v: &Value) -> Option<(u16, i64, u16)> {
    let Value::Call(d, args) = v.unspan() else {
        return None;
    };
    if args.len() != 3
        || (*d as usize) >= data.definitions.len()
        || data.def(*d).name() != "OpGetField"
    {
        return None;
    }
    let Value::Int(content) = args[2].unspan() else {
        return None;
    };
    let (root, offs) = vector_path(data, v)?;
    Some((root, offs.iter().sum(), u16::try_from(*content).ok()?))
}

/// The type of the sub-record of variable `v` that `arg` names, when `arg` is an
/// `OpGetField` chain rooted at `v` ([`sub_record`]); `None` for anything else.
pub(crate) fn sub_record_of(data: &Data, arg: Option<&Value>, v: u16) -> Option<u16> {
    let (root, _, sub) = sub_record(data, arg?)?;
    (root == v).then_some(sub)
}

/// The record type an `OpCopyRecord`'s third operand names, under its flag mask.
pub(crate) fn copy_type(arg: Option<&Value>) -> Option<u16> {
    let Value::Int(k) = arg?.unspan() else {
        return None;
    };
    u16::try_from(*k & i32::from(crate::keys::COPY_TP_MASK)).ok()
}

/// The field a scalar getter or setter with operands `args` addresses on `v`, at its
/// SUMMED offset: `v.f` and `v.pos.x` are one notion ([`view_field`]), and a tuple carries
/// a nested record's scalars at exactly that key.
pub(crate) fn field_on(data: &Data, args: &[Value], v: u16) -> Option<i64> {
    let (root, off) = view_field(data, args.first()?, args.get(1)?)?;
    (root == v).then_some(off)
}

/// What a loop body writes in place, by RECORD TYPE (a schema type number) and offset
/// (@PLN157 P4c).  A hoisted scalar `(v, fld)` of a record typed `tp` is stale after a
/// write at `(tp, fld)` through ANY route — the variable itself, a `&`-bound alias, an
/// element view into a `vector<tp>`, a callee's parameter — so the key is the type and the
/// offset, never the variable: aliasing is decided by what a write can reach, and a
/// `vector<integer>` element written at offset 0 reaches no record's field.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
/// Enforces `@FR-R-Scalar`: the write set keyed by (record type, offset).
pub struct WriteSet {
    /// `(record type, offset)` pairs a scalar setter wrote through a classified target.
    pub offsets: HashSet<(u16, i64)>,
    /// Record types written at offsets this walk cannot see — a record a body frees, a
    /// copy over a whole record — so every scalar of the type is stale.
    pub whole: HashSet<u16>,
    /// Record types a CALLEE writes only as its own return buffer (§ V-c): whole to a
    /// hoisted scalar, which may name the buffer the caller handed in — but never the
    /// record a by-value PARAMETER views (`(R-ValueLocal)`), since the buffer a caller
    /// hands is a fresh one of its own or, under `(R-Rebind)`, an owned local's record
    /// that no parameter of the caller can alias.
    pub retbufs: HashSet<u16>,
    /// Record types written whole in a record this frame OWNS and nothing outside it can
    /// view: a hidden buffer minted from its sentinel, a discharge buffer re-initialised,
    /// a loop record re-established, a record this frame releases.  Whole to a hoisted
    /// scalar (the store may be reused under a name the prelude read), and nothing to a
    /// by-value parameter, whose record is the caller's.
    pub own: HashSet<u16>,
}

impl WriteSet {
    #[must_use]
    pub fn evicts(&self, tp: u16, fld: i64) -> bool {
        self.whole.contains(&tp)
            || self.retbufs.contains(&tp)
            || self.own.contains(&tp)
            || self.offsets.contains(&(tp, fld))
    }

    /// Does the set reach a record of type `tp` that a CALLER could hold a view of — a
    /// field write, or a whole write into an existing record — which is what a tuple
    /// parameter of that type must be safe against?  A callee's return buffer and this
    /// frame's own records are not such places.
    #[must_use]
    pub fn reaches_record(&self, tp: u16) -> bool {
        self.whole.contains(&tp) || self.offsets.iter().any(|(t, _)| *t == tp)
    }

    pub(crate) fn extend(&mut self, other: &WriteSet) {
        self.offsets.extend(other.offsets.iter().copied());
        self.whole.extend(other.whole.iter().copied());
        self.retbufs.extend(other.retbufs.iter().copied());
        self.own.extend(other.own.iter().copied());
    }
}

/// The per-callee memo of [`callee_writes`]: `None` is a callee this analysis cannot
/// classify, which keeps every scalar of its callers un-hoisted.
pub type WriteCache = HashMap<u32, Option<Rc<WriteSet>>>;

/// Does local `r` OWN the store it names — so no other variable of this frame can name
/// that store (@PLN157 § V-q)?  Enforces `@FR-R-Alias`'s exclusivity test.  Not a parameter (the caller's store), not a `&` link, an
/// empty dep list (`@FR-O-Proxy`), and not captured by a closure.
pub(crate) fn owned_local(vars: &crate::variables::Function, r: u16) -> bool {
    // `.base()`: the shape question sees through a `τ?` slot (`@FR-N-Shape`).
    !vars.is_argument(r)
        && !matches!(vars.tp(r).base(), Type::RefVar(_))
        && !vars.is_captured(r)
        // A local vector's dep list names its OWN hidden store witness (`__vdb_N`), which
        // is the store it owns, not a borrow of someone else's.
        && vars
            .tp(r)
            .depend()
            .iter()
            .all(|d| vars.name(*d).starts_with("__vdb"))
}

/// What an in-place setter's target reaches.
pub(crate) enum Target {
    /// A field of a record of the FIRST type, and — because an inline sub-record is
    /// flattened into its parent — of every enclosing record, each with the byte offset
    /// the addressed record starts at inside it: `s.a.x = …` on `Seg { a: V3, b: V3 }`
    /// writes `(V3, 0 + x)` and `(Seg, off(a) + x)`.  A target the chain cannot see the
    /// parent of (a `&` link to a sub-record, a parameter's view) reaches every type that
    /// holds its record inline ([`inline_parents`]), which only ever declines more.
    Record(Vec<(u16, i64)>),
    /// An element of a scalar vector — no record field can alias it.
    NoRecord,
    /// A shape this analysis does not type.
    Unknown,
}

/// The record type an in-place setter's first operand addresses (@PLN157 P4c): a record
/// variable, a const field path (`OpGetField` carries the field's schema type), or an
/// element of a vector named by either.  Every other shape is [`Target::Unknown`].
/// Is `place` a FRESH record's own place — the variable itself, or a field reached
/// through `OpGetField` chains from it (a literal's inline sub-record)?  A write there
/// reaches no record a hoisted scalar or a by-value parameter can name.
pub(crate) fn fresh_rooted(place: &Value, data: &Data, fresh: &HashSet<u16>) -> bool {
    match place.unspan() {
        Value::Var(v) => fresh.contains(v),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpGetField"
                && !args.is_empty() =>
        {
            fresh_rooted(&args[0], data, fresh)
        }
        _ => false,
    }
}

pub(crate) fn setter_target(
    target: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
) -> Target {
    match target.unspan() {
        Value::Var(u) => {
            let Some(tp) = plain_record_type(data, vars.tp(*u)) else {
                return Target::Unknown;
            };
            let mut chain = vec![(tp, 0)];
            // A record this frame does not own — a parameter's view, a `&` link, an
            // element view — may be a sub-record of a caller's record.
            if !owned_local(vars, *u) {
                chain.extend(inline_parents(stores, tp));
            }
            Target::Record(chain)
        }
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            if name == "OpGetField" && args.len() == 3 {
                let Value::Int(content) = args[2].unspan() else {
                    return Target::Unknown;
                };
                let Value::Int(off) = args[1].unspan() else {
                    return Target::Unknown;
                };
                let Ok(tp) = u16::try_from(*content) else {
                    return Target::Unknown;
                };
                if !stores.is_struct(tp) {
                    return Target::Unknown;
                }
                let mut chain = vec![(tp, 0)];
                match setter_target(&args[0], data, stores, vars) {
                    Target::Record(parents) => {
                        chain.extend(parents.into_iter().map(|(p, b)| (p, b + i64::from(*off))));
                    }
                    Target::NoRecord | Target::Unknown => {
                        chain.extend(inline_parents(stores, tp));
                    }
                }
                return Target::Record(chain);
            }
            if is_element_address(data, *d) && args.len() == 3 {
                return element_target(&args[0], data, stores, vars);
            }
            Target::Unknown
        }
        _ => Target::Unknown,
    }
}

/// Every struct type that holds a record of type `tp` INLINE, transitively, each with the
/// byte offset `tp` starts at inside it: the records a write through a sub-record view
/// may be reaching when the view's parent is not in sight.
pub(crate) fn inline_parents(stores: &Stores, tp: u16) -> Vec<(u16, i64)> {
    let mut out: Vec<(u16, i64)> = Vec::new();
    let mut todo: Vec<(u16, i64)> = vec![(tp, 0)];
    while let Some((cur, base)) = todo.pop() {
        for (p, t) in stores.types.iter().enumerate() {
            let (crate::database::Parts::Struct(fields)
            | crate::database::Parts::EnumValue(_, fields)) = &t.parts
            else {
                continue;
            };
            let Ok(p) = u16::try_from(p) else {
                continue;
            };
            for f in fields.iter().filter(|f| f.content == cur) {
                let at = base + i64::from(f.position);
                if !out.contains(&(p, at)) {
                    out.push((p, at));
                    todo.push((p, at));
                }
            }
        }
    }
    out
}

/// The element type of the vector operand of an element-address op, as a [`Target`].
/// `LOFT_NO_NARROW_ELEMENT_WRITE=1` — a write into a NARROW-integer vector element is
/// untyped again for the scalar hoist, as it was before such an element was read as a slot
/// of the vector's block: the first bisect step for a wrong value read through a hoisted
/// record field in a loop whose callee writes `i32` / `u16` / `u8` vector elements.
pub(crate) fn narrow_element_write_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_NARROW_ELEMENT_WRITE").is_ok_and(|v| v != "0"))
}

pub(crate) fn element_target(
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
                    // An element is a whole record of its own, never a sub-record.
                    plain_record_type(data, elem)
                        .map_or(Target::Unknown, |t| Target::Record(vec![(t, 0)]))
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
                Target::Record(vec![(elem, 0)])
            } else if stores.is_base(elem)
                || narrow_element_write_enabled()
                    && matches!(
                        stores.types[elem as usize].parts,
                        Parts::Byte(..)
                            | Parts::Short(..)
                            | Parts::Int(..)
                            | Parts::ShortRaw(..)
                            | Parts::IntRaw(..)
                    )
            {
                // A scalar element — a base value or a narrow integer — is a slot in the
                // vector's block, never a field of a record a hoisted scalar reads.
                Target::NoRecord
            } else {
                Target::Unknown
            }
        }
        _ => Target::Unknown,
    }
}

/// `LOFT_TRACE_HOIST_DECLINE=1` — name the node that made [`body_writes`] answer `None`.
pub(crate) fn trace_write_decline(n: &Value) {
    if std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
        let shown = format!("{n:?}");
        eprintln!(
            "hoist: write set untyped at {}",
            &shown[..shown.len().min(160)]
        );
    }
}

/// The [`WriteSet`] of a body that passed the hoist gate, or `None` when it writes through
/// something this analysis cannot type.  Walks every call: an [`IN_PLACE_SET_OPS`] setter
/// contributes its target's `(type, offset)`; a record free contributes the record's whole
/// type; a user callee that writes contributes what [`callee_writes`] answers for it; a
/// store-free op contributes nothing.  Anything else — a native writer the gate would have
/// blocked, a `CallRef`, a `Parallel`, a `Yield` — answers `None`, as does a setter whose
/// offset is not a constant.
///
/// `fresh` carries the element variables the body has MINTED so far (@PLN157 § V-s,
/// `@FR-R-Mint`): a `Set(v, OpNewRecord(P, …))` over an admissible mint path inserts `v`,
/// any other assignment to `v` and the group's own `OpFinishRecord` remove it, and a
/// setter whose target is a fresh variable contributes NOTHING — a record minted inside
/// the body cannot be named by a variable whose getter the prelude already ran, so no
/// hoisted scalar can go stale through it.  The walk is preorder, which is execution
/// order for the straight-line `_elm_N = OpNewRecord; sets; OpFinishRecord` group the
/// parser emits (the only producer of `OpNewRecord`); a group any FUTURE lowering splits
/// across branches merely fails to register, which costs the exemption, never soundness —
/// except a mint and a write of the SAME variable split across two arms, which the
/// parser's per-group `_elm_N` temps (fresh variable per group, assigned nowhere else)
/// keep unconstructible.
/// Enforces `@FR-R-Scalar`: the write set of a body that passed the gate.
#[allow(clippy::too_many_arguments)]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn body_writes(
    node: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    active: &mut HashSet<u32>,
    fresh: &mut HashSet<u16>,
    owned: Option<&HoistOwned>,
) -> Option<WriteSet> {
    let mut set = WriteSet::default();
    let mut ok = true;
    node.any_node(&mut |n| match n {
        // The allocate-or-reuse guard of a record the walk holds as FRESH (`b = OpDatabase(b,
        // …)`) re-establishes that same record: it stays fresh.
        Value::Set(v, inner)
            if fresh.contains(v)
                && matches!(inner.unspan(), Value::Call(d, a)
                    if (*d as usize) < data.definitions.len()
                        && matches!(data.def(*d).name(), "OpDatabase" | "OpDatabaseNP")
                        && matches!(a.first().map(Value::unspan), Some(Value::Var(b)) if b == v)) =>
        {
            false
        }
        Value::Set(v, inner) => {
            if matches!(inner.unspan(), Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && mint_path(data, stores, "OpNewRecord", args, vars).is_some())
            {
                fresh.insert(*v);
            } else {
                fresh.remove(v);
            }
            false
        }
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                ok = false;
                trace_write_decline(n);
                return true;
            }
            let def = data.def(*d);
            let name = def.name();
            if IN_PLACE_SET_OPS.contains(&name) {
                let fld = args.get(1).map(Value::unspan);
                let Some(Value::Int(fld)) = fld else {
                    ok = false;
                    trace_write_decline(n);
                    return true;
                };
                // A write into a freshly minted element reaches no record a hoisted
                // scalar can name (`@FR-R-Mint`) — nor does one into its inline sub-record.
                if args.first().is_some_and(|t| fresh_rooted(t, data, fresh)) {
                    return false;
                }
                match args
                    .first()
                    .map_or(Target::Unknown, |t| setter_target(t, data, stores, vars))
                {
                    Target::Record(chain) => {
                        for (tp, base) in chain {
                            set.offsets.insert((tp, base + i64::from(*fld)));
                        }
                    }
                    Target::NoRecord => {}
                    Target::Unknown => {
                        ok = false;
                        trace_write_decline(n);
                        return true;
                    }
                }
            } else if FUSABLE_PUSHES.iter().any(|(p, _, _)| *p == name)
                || name == "OpPreAllocVector"
            {
                // @PLN157 § V-q — a push (or the reservation before one) grows a vector and
                // rewrites its handle slot, which no scalar getter reads; the container
                // record itself does not move.
            } else if mint_path(data, stores, name, args, vars).is_some() {
                // @PLN157 § V-s (`@FR-R-Mint`) — the mint's growth moves the pushed
                // vector's record without changing any value, and its element defaults land
                // in the fresh record only.  The close of the group ends the variable's
                // fresh window.
                if name == "OpFinishRecord"
                    && let Some(Value::Var(e)) = args.get(1).map(Value::unspan)
                {
                    fresh.remove(e);
                }
            } else if name == "OpCopyRecord"
                && args.len() == 3
                && fresh_rooted(&args[1], data, fresh)
            {
                // § V-d's delivery tail into a fresh mint variable (`@FR-R-Mint`): the copy
                // writes the fresh element, and its source-free releases the builder's own
                // this-iteration buffer — neither is a record a hoisted scalar can name.
                // A copy into a FIELD of a fresh record (a literal's sub-record) is the same.
            } else if let Some(tp) = in_place_copy(Some(stores), name, args) {
                // `@FR-R-InPlace`'s copy clause — every field of the destination is written,
                // whichever record of that type it is.
                set.whole.insert(tp);
            } else if let Some(tp) = null_buffer_alloc(name, args, Some(vars), data)
                .or_else(|| discharge_buffer_set(name, args, Some(vars), data))
            {
                // § V-ad — the discharge buffer is re-initialised whole, and its fallback's
                // fields written; only a scalar hoisted off ITS type could observe that, and
                // the buffer's view is rebound per use.
                set.own.insert(tp);
            } else if matches!(name, "OpDatabase" | "OpDatabaseNP")
                && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                && fresh.contains(b)
            {
                // A mint that (re-)establishes a record the walk already holds as FRESH —
                // the frame's own return buffer, seeded by a walk that asks about parameters
                // (`tuple_param_candidates`): the store is minted from the sentinel, or the
                // one handed in is cleared and claimed again, and neither is a place a
                // parameter of this frame views.
            } else if name == "OpAppendVector"
                && let Some(Value::Call(gd, gargs)) = args.first().map(Value::unspan)
                && (*gd as usize) < data.definitions.len()
                && data.def(*gd).name() == "OpGetField"
                && let (Some(Value::Var(e)), Some(Value::Int(off))) = (
                    gargs.first().map(Value::unspan),
                    gargs.get(1).map(Value::unspan),
                )
                && (fresh.contains(e)
                    || owned.is_some_and(|o| o.elem_fields.contains(&(*e, i64::from(*off)))))
            {
                // The literal's copy of a vector into a FRESH element's field (`@FR-R-Mint`:
                // a record minted in the body, which no hoisted scalar can name) — or the § V-z
                // paired copy the emitter elides outright, the temp having been built in the
                // slot.  Neither writes a record a getter the prelude ran could read.
            } else if matches!(name, "OpDatabase" | "OpDatabaseNP")
                && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                && owned.is_some_and(|o| o.buffers.contains(b) || o.records.contains(b))
            {
                // `(R-Mint)`'s rebound clause — an emitter-owned mint.  A vector BUFFER (a § V-al
                // loop buffer, a § V-z witness) is a fresh store or a length reset that holds no
                // record a hoisted scalar can name — the local bound from it is rebound in the
                // body and never a candidate.  A `(R-LoopRecord)` local's mint re-establishes
                // its record's defaults: a write of that whole type.
                if owned.is_some_and(|o| o.records.contains(b)) {
                    let Some(tp) = plain_record_type(data, vars.tp(*b)) else {
                        ok = false;
                        trace_write_decline(n);
                        return true;
                    };
                    set.own.insert(tp);
                }
            } else if lazy_buffer_mint(name, args, Some(vars)) {
                // `@FR-O-LazyBuffer` — a fresh store for the buffer from its sentinel; only a
                // scalar hoisted off the buffer's own record type could observe it.
                let minted = match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => plain_record_type(data, vars.tp(*b)),
                    _ => None,
                };
                let Some(tp) = minted else {
                    ok = false;
                    trace_write_decline(n);
                    return true;
                };
                set.own.insert(tp);
            } else if scalar_stack_ref(name, args, Some(vars)) {
                // The address of a scalar local writes nothing; what writes through it (the
                // read below) is judged on its own.
            } else if scalar_file_read(data, name, args, Some(vars))
                && let Some(Value::Var(f)) = args.first().map(Value::unspan)
                && let Some(tp) = plain_record_type(data, vars.tp(*f))
            {
                // `@FR-R-InPlace`'s read clause, typed: a scalar `f#read` writes the File
                // record's own fields (its cursor, its handle on first use) and a local — a
                // write of that whole record type, and of no other.
                set.whole.insert(tp);
            } else if RECORD_FREE_OPS.contains(&name) {
                let freed = match args.first().map(Value::unspan) {
                    Some(Value::Var(r)) => plain_record_type(data, vars.tp(*r)),
                    _ => None,
                };
                let Some(tp) = freed else {
                    ok = false;
                    trace_write_decline(n);
                    return true;
                };
                set.own.insert(tp);
            } else if matches!(def.code(), Value::Null) {
                if !native_op_is_store_free(def) {
                    ok = false;
                    trace_write_decline(n);
                    return true;
                }
            } else if call_writes_store(*d, data, cache, active, owned) {
                let inner = owned.map(|o| o.for_callee(*d));
                let Some(w) =
                    callee_writes(*d, data, stores, cache, writes, active, inner.as_ref())
                else {
                    ok = false;
                    trace_write_decline(n);
                    return true;
                };
                set.extend(&w);
            }
            false
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => {
            ok = false;
            trace_write_decline(n);
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
/// Enforces `@FR-R-Scalar` through `@FR-R-Callee`: what an admitted callee reaches.
pub(crate) fn callee_writes(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    active: &mut HashSet<u32>,
    owned: Option<&HoistOwned>,
) -> Option<Rc<WriteSet>> {
    if let Some(known) = writes.get(&d_nr) {
        return known.clone();
    }
    let def = data.def(d_nr);
    let answer = if matches!(def.code(), Value::Null) || !active.insert(d_nr) {
        None
    } else {
        let inner = if in_place_only_writer(d_nr, data, cache, active, owned) {
            // A callee's body: its own emitter-owned mints are not this frame's, so none
            // is admitted here (the conservative side — an untyped mint declines).
            body_writes(
                def.code(),
                data,
                stores,
                def.variables(),
                cache,
                writes,
                active,
                &mut HashSet::new(),
                None,
            )
        } else if retbuf_only_writer(d_nr, data, cache, active, owned) {
            def.hidden_return_buffer_attr()
                .and_then(|attr| plain_record_type(data, &def.attributes()[attr].typedef))
                .map(|tp| WriteSet {
                    offsets: HashSet::new(),
                    whole: HashSet::new(),
                    retbufs: HashSet::from([tp]),
                    own: HashSet::new(),
                })
        } else {
            None
        };
        active.remove(&d_nr);
        inner.map(Rc::new)
    };
    // A `None` reached on a recursive edge is memoised too: it only ever withholds a hoist.
    crate::rewrite_census::fired("R-Callee", usize::from(answer.is_some()));
    writes.insert(d_nr, answer.clone());
    answer
}

pub fn is_element_address(data: &Data, d_nr: u32) -> bool {
    (d_nr as usize) < data.definitions.len() && ELEMENT_ADDRESS_OPS.contains(&data.def(d_nr).name())
}
