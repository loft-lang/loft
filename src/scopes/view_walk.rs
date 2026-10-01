// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Which VIEW bindings must be materialised: the in-order walk that finds every view still live
//! across a disturbance of its container, so the scan copies it out instead of aliasing storage
//! that may move.

use super::disturbance::{
    Disturbance, DisturbedParams, RemovedParams, ViewCause, grown_containers, named_place,
    reshaped_containers,
};
use super::places::{
    ParamPlace, base_container_place, call_arg_place, compose_param_place, get_record_literal_keys,
    keyed_payload_view, peel_stack_ref, same_place, value_view_places,
};
use crate::data::{Data, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::use_analysis::ANY_FIELD;
use crate::variables::Function;

/// The place a value block's TAIL views, with the block's own `Set`s added to `env` so a tail
/// that names one of them resolves to the value it was bound to.
/// The place a `text` PAYLOAD binding views: its bind reads `OpGetText(base, offset)`, the
/// field of the subject `base` at `offset` (loft#1665).
///
/// Kept apart from [`value_view_places`] on purpose.  `OpGetText` answers an owned COPY of the
/// characters, so it is no projection for the deps proxy or the ownership readers, and adding
/// it to the shared projection list would change what every one of them answers.  Only a
/// binding whose writes the parser MIRRORS into that field is a view of it, and only the
/// disturbance walk asks.  A subject that is itself a projection names the place IT views; a
/// base that is neither answers nothing, which costs the materialise and keeps today's mirror.
fn text_payload_place(value: &Value, data: &Data, function: &Function) -> Option<(u16, u32)> {
    let Value::Call(d_nr, args) = value.unspan() else {
        return None;
    };
    if data.def(*d_nr).name() != "OpGetText" || args.len() != 2 {
        return None;
    }
    match (args[0].unspan(), args[1].unspan()) {
        (Value::Var(x), Value::Int(off)) if !function.is_compiler_generated(*x) => {
            Some((*x, u32::try_from(*off).ok()?))
        }
        (base, _) => crate::use_analysis::view_source_place(data, base)
            .filter(|(c, _)| !function.is_compiler_generated(*c)),
    }
}

/// The literal key each keyed REMOVAL in `code` names, per container place.
///
/// Only for the record-per-element kinds: a `sorted` removal ends every place the container
/// holds whatever key it named, so recording its key would invite a caller to spare a view
/// the rule does not spare.  Absent from the map means "no literal key here", which every
/// reader must treat as *disturbs everything* — the conservative direction.
///
/// `h[k] = null` lowers to `OpHashRemove(coll, OpGetRecord(coll, tp, n, k…), tp)`, so the key
/// is read out of the removal's own argument through [`get_record_literal_keys`] — the same
/// reader the view side uses, so the two cannot disagree about what a key is.
fn keyed_removal_keys(
    code: &Value,
    data: &Data,
    function: &Function,
) -> HashMap<(u16, u32), Vec<Value>> {
    let mut out: HashMap<(u16, u32), Vec<Value>> = HashMap::default();
    let mut ambiguous: HashSet<(u16, u32)> = HashSet::default();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        if data.def(*d).name() != "OpHashRemove" {
            return;
        }
        let Some(place) = args.first().and_then(|a| named_place(a, data)) else {
            return;
        };
        if place.1 != ANY_FIELD
            || matches!(function.tp(place.0).base(), Type::Sorted(_, _, _))
            || ambiguous.contains(&place)
        {
            return;
        }
        // TWO removals from one container in one statement, or one whose key is computed:
        // either way this statement does not name a single sparable key, so drop the entry
        // and leave the place conservative.
        match args.get(1).and_then(|a| get_record_literal_keys(a, data)) {
            Some(keys) if !out.contains_key(&place) => {
                out.insert(place, keys);
            }
            _ => {
                out.remove(&place);
                ambiguous.insert(place);
            }
        }
    });
    out
}

/// @PLN130 F8 — which `&` parameters of `d_nr` are REASSIGNED wholesale by its body.
///
/// A `&` param's slot is a double indirection into the caller's variable, and @PLN87 P2.2
/// lowers `p = T{…}` on one to *"build a fresh store, write it through, free the store the
/// caller was holding"*. So a callee doing that destroys a store the CALLER may have views
/// into — measured on `--native` as a read of `703`, a `Wide` field belonging to an unrelated
/// later allocation. Nothing at the call site says so, which is why the caller needs this
/// fact about the callee rather than a guess from the argument's shape.
///
/// Answered from the callee's IR: a `Set` targeting an argument slot whose declared type is
/// `RefVar`. Argument slots lead the variable numbering, so the slot number indexes the
/// attribute list. A shape this does not recognise yields no fact and keeps today's
/// behaviour — the same lower-bound stance as [`collect_reshaped_containers`].
fn reassigned_ref_params(data: &Data, d_nr: u32) -> HashSet<u16> {
    let def = data.def(d_nr);
    let mut out: HashSet<u16> = HashSet::default();
    def.code.walk(&mut |v| {
        let Value::Set(slot, rhs) = v else { return };
        if matches!(rhs.unspan(), Value::Null) {
            return;
        }
        if let Some(a) = def.attributes.get(usize::from(*slot))
            && matches!(a.typedef, Type::RefVar(_))
        {
            out.insert(*slot);
        }
    });
    out
}

/// @PLN130 F9 — every container variable `stmt` reshapes THROUGH A CALL: passed as a `&`
/// argument to a callee that removes from that parameter.
///
/// Deliberately separate from [`reshaped_containers`] rather than folded into it, because the
/// two answers feed different decisions and folding them was measured to break a cell. F2's
/// materialise is conservative — *any* reshape copies the view — so counting a callee's removal
/// there would silently COPY a plain view whose element never moves and whose write
/// legitimately lands, which is the regression the earlier attempt at this fix measured
/// (probe 38 cell C1). The REFUSAL wants the wider answer, because a rejected program is not
/// silently anything; the materialise wants the narrower one.
fn reshaped_via_call(stmt: &Value, data: &Data, removed: &RemovedParams) -> HashMap<u16, u32> {
    let mut out: HashMap<u16, u32> = HashMap::default();
    stmt.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let Some(params) = removed.get(d) else { return };
        for k in params {
            if let Some(Value::Var(c)) = args.get(usize::from(*k)).map(|a| peel_stack_ref(a, data))
            {
                out.insert(*c, *d);
            }
        }
    });
    out
}

/// Record `d` for `view`, keeping [`ViewCause::Reshaped`] when both apply.
fn record_cause(map: &mut HashMap<u16, Disturbance>, view: u16, d: Disturbance) {
    let slot = map.entry(view).or_insert(d);
    if d.cause == ViewCause::Reshaped {
        *slot = d;
    }
}

/// @PLN130 F2 + F8 — every VIEW binding that is live across a disturbance of its container.
///
/// This is @FR-B-View's materialise clause: a struct-typed projection aliases without `&`, and
/// where the container is DISTURBED (@FR-B-Disturb) while the view is still LIVE the binding
/// gives up the alias, takes its own copy at the bind, and the author is told.  Answering it
/// is this walk's whole job; the two users of the verdict — `scan_set`'s dep strip and the
/// interpreter's materialising emitters — act on it.
///
/// Two disturbances, one question. A RESHAPE (`v.remove(i)`, `e#remove`) renumbers the
/// positions inside the container's store, and a view is a `DbRef` pinned to one — so a view
/// live across it silently starts naming a different element (probes 03-07, 29: a pure READ
/// answered `44/444` where its element held `33/333`, and a write tore a live record). A
/// REASSIGNMENT of the container variable is the other:
///
/// A dep names a VARIABLE, not a store instance, so a view bound from `bx.v[0]` keeps
/// reading "wherever `bx` points" — and `bx = <other value>` re-points it. The view then
/// answers the REPLACEMENT's value (measured 22 where its element held 11, on both
/// backends), and on `--native` the displaced store is freed, so the read is a genuine
/// use-after-free into whatever now occupies the space (measured 703, a `Wide` field).
///
/// A vector local is immune and that is what hid this: a vector literal allocates through a
/// hidden `__vdb_N` owner, so `a = [...]` twice mints TWO stores and the first keeps its
/// identity. A struct local has no such indirection — `bx` IS the owner.
///
/// The question this answers is deliberately about the VIEW, not the container. *"Is the
/// container re-established anywhere in this function"* is a per-FUNCTION proxy for a
/// per-BINDING fact, and it is wrong in the direction that costs correctness elsewhere: an
/// unrolled `for pf in fields(p)` re-establishes `pf` once per field, and the `is`-pattern
/// subject bound from it dies inside its own arm long before the next one — yet the proxy
/// stripped that subject's deps, which put an `OpFreeRef` in a scope where the declaration
/// was not visible and stopped `tests/scripts/45-field-iter.loft` compiling under `--native`.
///
/// So a view is at risk only where a disturbance of its container can be reached WHILE THE
/// VIEW IS STILL LIVE. A disturbance sitting in a block the view's own block has already
/// closed cannot reach it, which is exactly the unrolled-iteration case above.
///
/// Four forms establish a store, matching the four emitters measured to break:
///
/// - `OpDatabase(v, …)` — a struct literal built into `v`'s own store;
/// - `Set(v, Var(src))` — the C86 whole-value bind, which codegen lowers to
///   `OpDatabase` + `OpCopyRecord` into a fresh store (the `bx = other` swap);
/// - `Set(v, Call(f, …))` — a bind from a user function's return (`bx = mk(22)`);
/// - passing `v` as a `&` argument to a callee that reassigns that parameter wholesale
///   ([`reassigned_ref_params`]) — the callee frees the caller's store, so from the view's
///   point of view this is a reassignment that happens to be spelled as a call.
///
/// A container's FIRST establishment is never a risk: it necessarily runs before any view of
/// it can exist, so no separate "is this the definition" test is needed.
///
/// A vector-typed container can never be REASSIGNED into danger — it reaches its store
/// through a hidden `__vdb_N` owner, so re-pointing the local leaves the old store's identity
/// intact — and [`established_stores`] never reports one. It very much can be RESHAPED, which
/// is the F2 half.
///
/// **A disturbance alone is not enough: the view must still be USED afterwards.** Keying on
/// order alone is what F2 shipped (a flat per-function `reshaped_containers` set), and it
/// costs both halves of this plan's closure bar — a write that mainline lands is LOST, and the
/// advice says *"`v` is modified while `c` is in use"* of a `c` that is not in use (probe 39
/// cells L1, L5, L8, L10). So a disturbance only SHAKES the open views of that container;
/// a later read or write of a shaken view is what condemns it. A view whose last use precedes
/// the disturbance keeps its alias and writes through, which is the rustc rule.
///
/// A block frame ends the views the block OWNS, which is not the same as the views bound in
/// it. Re-binding an outer local inside a nested block gives a view that outlives the block,
/// and dropping it at the close is what let loft#1184 through — `a = w.inner` in a loop body,
/// `w = Outer{inner: a}` on the next turn, every heap field of `a` empty from the second
/// iteration on. So a view goes into the frame that owns its VARIABLE (the block it was first
/// bound in), and a LOOP is walked twice: the first pass could only shake views that already
/// existed, and the second supplies the use that condemns one the body itself bound. Two
/// passes reach the fixpoint, because the second binds exactly the views the first did.
///
/// Known lower bound: only a `Var` names a container, so a disturbance reached through some
/// other expression is not recognised — [`reshaped_containers`] and [`established_stores`] are
/// both lower bounds, and a missed case keeps today's behaviour rather than inventing a new
/// one.
pub(super) fn collect_views_to_materialise(
    code: &Value,
    function: &Function,
    data: &Data,
    database: &crate::database::Stores,
    disturbed: Option<&DisturbedParams>,
) -> HashMap<u16, Disturbance> {
    let out = ViewWalk::run(code, function, data, None, disturbed, Some(database), 0);
    if !out.is_empty() && crate::env_once!(std::env::var_os("LOFT_DEBUG_F8").is_some()) {
        let mut names: Vec<String> = out
            .iter()
            .map(|(v, d)| format!("{}({:?})", function.name(*v), d.cause))
            .collect();
        names.sort();
        eprintln!(
            "[f8] views live across a container disturbance: {}",
            names.join(" ")
        );
    }
    // The whole `Disturbance` is kept, not just its cause: it already carries the CONTAINER
    // that was disturbed, and the two report sites used to re-derive one from the right-hand
    // side instead.  That was a restatement — and once a binding can view more than one
    // container it is also a wrong answer, because the RHS names both and only one of them was
    // disturbed.  One home for the fact, named where it was observed.
    out
}

/// The state of [`collect_views_to_materialise`]'s in-order walk.
pub(super) struct ViewWalk<'a> {
    function: &'a Function,
    data: &'a Data,
    /// One frame per open block: the views the block OWNS, and the container each one views.
    /// A view dies when the block that owns its VARIABLE closes — which is where the variable
    /// was first bound, not necessarily where this binding was written (see `bound_at`).
    open: Vec<Vec<(u16, u16, u32)>>,
    /// The frame depth each variable was first BOUND at, which is the block that owns it.
    ///
    /// A view goes into the frame that owns its VARIABLE, not the block the binding statement
    /// happens to sit in — the two differ whenever an outer local is re-bound inside a nested
    /// block, and the variable then outlives that block. A hoisted `Set(v, Null)` declaration
    /// is skipped: it is emitted at function scope for every ref- and text-typed local, so
    /// counting it would put EVERY view at function scope and undo the frame model.
    bound_at: HashMap<u16, usize>,
    /// The literal key a view was bound at, when it was bound at one — `c = &h[30]` records
    /// `[30]`.  Absent means the key was computed (or the bind was not a keyed point lookup),
    /// which every reader must treat as *could be any key*: loft#1460's filter may only ever
    /// SPARE a view it can prove names a different record.
    view_keys: HashMap<u16, Vec<Value>>,
    /// The bindings that name a whole CONTAINER rather than a position inside one, each with
    /// the place it names DIRECTLY — the `d = &cv.data` half of `(B-Ref-Alias)`'s
    /// in-versus-to distinction, read off the same walk that answers the place
    /// ([`crate::use_analysis::view_source_place_indexed`]).
    ///
    /// Growing or reshaping that container does not end the place such a binding names: both
    /// move the ELEMENTS, and this one names the field SLOT that holds them, which the growth
    /// repoints and the link re-reads.  Only `(B-Disturb)`'s fourth event does — reassigning
    /// the base leaves the slot itself with nothing to point at — so `shake_places_keyed`
    /// spares these for the other causes and never for that one (loft#1543).
    ///
    /// The DIRECT place is what is stored, not [`Self::resolve_view_root`]'s answer, and the
    /// sparing matches on it.  A binding whose own container is itself a view resolves to the
    /// OUTER container, and growing that one moves the record holding this binding's slot —
    /// which does end its place.  Keyed on the resolved root, such a binding would be spared
    /// from the one disturbance that genuinely reaches it.
    whole_container: HashMap<u16, (u16, u32)>,
    /// Views whose container has been disturbed since the bind, and by what. Being shaken is
    /// not yet a verdict — it becomes one at the next use.
    shaken: HashMap<u16, Disturbance>,
    /// The answer: views USED after their container was disturbed.
    out: HashMap<u16, Disturbance>,
    /// `Some` also counts a CALLEE's removal from a `&` parameter as a reshape (F9's refusal);
    /// `None` stays inside this frame (F2's materialise). See [`reshaped_via_call`] for why the
    /// two questions do not share an answer.
    cross_frame: Option<&'a RemovedParams>,
    /// @PLN164 C3 — the places each callee disturbs through its PARAMETERS, so `(B-Disturb)`
    /// reaches across the frame boundary the way `(B-Ref-Reshape)` says it does.  `None` keeps
    /// this frame's own answer, which is what the REFUSAL still reads: extending it would reject
    /// programs that compile today, a separable change, and refusing less is its safe direction.
    disturbed: Option<&'a DisturbedParams>,
    /// Every place this function REBUILDS in whole — `x.a = [9, 9]` emits an
    /// `OpClearVector` on the field and then exactly the `OpNewRecord`s an append emits, and
    /// the two are SEPARATE statements, so the pairing cannot be seen one statement at a time.
    ///
    /// `(B-Disturb)` is explicit that OVERWRITING a place is not disturbing it — *"`o.inner =
    /// Box{…}` writes INTO the place `o.inner` already occupies, so a view of it survives"* —
    /// and without this subtraction `c = b.vecf; b.vecf = [9, 9]` materialised `c`, which
    /// `bind-copies-or-views-the-whole-boundary` caught on its `(B-View-Base)` cell: the
    /// seventeen-cell guard that exists to pin exactly this line.
    ///
    /// Accumulated for the whole walk rather than per block, which errs the safe way: a place
    /// cleared once and genuinely GROWN later is a MISSED disturbance, costing a materialise,
    /// where the other direction costs a program its meaning.
    cleared: HashSet<(u16, u32)>,
    /// The store, where the caller has one.  Only the field-NUMBER to byte-OFFSET conversion
    /// needs it (`grown_containers`): a growth names its container by field number and a view
    /// carries a byte offset, so a walk without the store cannot see a growth of a container
    /// held in a FIELD at all.
    ///
    /// Both callers pass one.  Without it `grown_containers` leaves every field-qualified
    /// growth UNCOLLECTED, so `(B-Ref-Reshape)`'s answer would depend on where the container is
    /// STORED — `c = &b.v[0]; b.v += [x]` unrefused while the same growth of a plain local is
    /// refused, and a removal from that same field is.  A missed disturbance is the safe
    /// direction for a refusal, but it is not a reason to leave one class of disturbance
    /// invisible.  (`binding.md` D-bind-47)
    database: Option<&'a crate::database::Stores>,
    /// The source line of the statement being walked, tracked from the `Value::Line` markers
    /// a block interleaves with its operators — the only line information the IR carries.
    line: u32,
    /// The collection variable the CURRENT statement gave a whole new value — read by the
    /// growth that fills it, which is a replacement to the author (`Disturbance::replaced`).
    /// Cleared at every `Line` marker, the statement boundary the parser leaves even between
    /// statements on one source line.  One slot, not a set: a statement replaces one
    /// collection local, and the front end's allocation pin counts every walk.
    rebound: Option<u16>,
}

impl ViewWalk<'_> {
    /// Walk `code` in source order and answer which views were used after their container was
    /// disturbed.
    ///
    /// `start_line` seeds the line tracking with the definition's own: a block emits a
    /// `Value::Line` marker only where the line CHANGES, so a body whose first statement is on
    /// the signature's line carries no marker at all and would otherwise report line 0.
    pub(super) fn run<'a>(
        code: &Value,
        function: &'a Function,
        data: &'a Data,
        cross_frame: Option<&'a RemovedParams>,
        disturbed: Option<&'a DisturbedParams>,
        database: Option<&'a crate::database::Stores>,
        start_line: u32,
    ) -> HashMap<u16, Disturbance> {
        let mut walk = ViewWalk {
            function,
            data,
            open: vec![Vec::new()],
            bound_at: HashMap::default(),
            view_keys: HashMap::default(),
            whole_container: HashMap::default(),
            shaken: HashMap::default(),
            out: HashMap::default(),
            cross_frame,
            disturbed,
            database,
            cleared: HashSet::default(),
            line: start_line,
            rebound: None,
        };
        walk.walk_block(std::slice::from_ref(code));
        walk.out
    }

    fn walk_block(&mut self, stmts: &[Value]) {
        for stmt in stmts {
            if let Value::Line(n) = stmt.unspan() {
                self.line = *n;
                self.rebound = None;
            }
            self.walk_stmt(stmt);
        }
    }

    /// Descend in SOURCE ORDER, because the whole point is which came first.
    ///
    /// Only the block forms listed here are descended into; anything else — a `match`, say —
    /// is handled whole by [`Self::leaf`], which both shakes and reads uses over the entire
    /// statement. That is deliberately coarse in both directions for a form we cannot order
    /// internally, and it is the safety net that keeps an unrecognised construct from hiding
    /// a removal.
    fn walk_stmt(&mut self, stmt: &Value) {
        match stmt.unspan() {
            // `Insert` is spliced into the enclosing block rather than forming its own, so
            // its statements are siblings and a view bound there stays live here.
            Value::Insert(ops) => self.walk_block(ops),
            Value::Block(b) => self.scoped(&b.operators),
            Value::Loop(b) => {
                // A loop body runs again, so a disturbance ANYWHERE inside it precedes every
                // use inside it on the next iteration. Shaking before the body is walked is
                // what makes a view held from OUTSIDE the loop and used at the top of the
                // body come out live across a removal at the bottom of it.
                self.disturb(stmt);
                self.scoped(&b.operators);
                // The BACK EDGE. The shake above could only reach views that already existed;
                // a view the body itself binds is disturbed by the same statements one turn
                // later, and nothing had seen it yet. So shake again and re-walk — the second
                // pass is what supplies the USE that condemns it (loft#1184). One extra pass
                // reaches the fixpoint: it binds exactly the views the first pass bound, so a
                // third would read the same state.
                let before: HashSet<u16> = self.shaken.keys().copied().collect();
                self.disturb(stmt);
                if self.shaken.keys().any(|v| !before.contains(v)) {
                    self.scoped(&b.operators);
                }
            }
            Value::If(cond, t, e) => {
                // The condition is evaluated before either branch and is not part of one.
                self.leaf(cond);
                // The arms are ALTERNATIVES (`(B-Disturb)` holds per path), so each starts from
                // the views open before the `if`, and a rebind in one arm does not end a view
                // the other arm binds.  What is open or shaken afterwards is what EITHER path
                // leaves; a literal key survives only where both paths agree, because a key may
                // only spare a view it proves names a different record (loft#1460).
                let open_before = self.open.clone();
                let shaken_before = self.shaken.clone();
                let keys_before = self.view_keys.clone();
                let mut ends = Vec::with_capacity(2);
                for branch in [t.unspan(), e.unspan()] {
                    self.open.clone_from(&open_before);
                    self.shaken.clone_from(&shaken_before);
                    self.view_keys.clone_from(&keys_before);
                    match branch {
                        Value::Block(b) => self.scoped(&b.operators),
                        Value::Insert(ops) => self.scoped(ops),
                        other => self.leaf(other),
                    }
                    ends.push((
                        std::mem::take(&mut self.open),
                        std::mem::take(&mut self.shaken),
                        std::mem::take(&mut self.view_keys),
                    ));
                }
                let (e_open, e_shaken, e_keys) = ends.pop().expect("an else arm");
                let (mut open, mut shaken, t_keys) = ends.pop().expect("a then arm");
                for (frame, extra) in open.iter_mut().zip(e_open) {
                    for item in extra {
                        if !frame.contains(&item) {
                            frame.push(item);
                        }
                    }
                }
                for (v, d) in e_shaken {
                    shaken.entry(v).or_insert(d);
                }
                self.open = open;
                self.shaken = shaken;
                self.view_keys = t_keys
                    .into_iter()
                    .filter(|(v, k)| e_keys.get(v) == Some(k))
                    .collect();
            }
            // A `Set` whose VALUE carries statements — a value `if` or `match`, a block —
            // runs them BEFORE the target is written, so they are walked in that order and
            // the target is recorded after.  Read whole by `leaf` instead, a view bound
            // inside one arm and the reassignment of its container in the SAME arm were
            // never separated in time: `got = match sh { Holder{inner} => { sh = Empty{…};
            // inner.a }, … }` was shaken and used in one indivisible step, so nothing
            // materialised and nothing was said, on both backends (loft#1394).  `leaf`'s own
            // doc calls that coarseness deliberate in both directions, and it is — for a
            // form whose internal order is unknown.  A `Set`'s is not: the value first, the
            // target after.
            Value::Set(_, rhs)
                if matches!(
                    rhs.unspan(),
                    Value::If(_, _, _) | Value::Block(_) | Value::Insert(_)
                ) =>
            {
                self.note_binding_depth(stmt);
                self.walk_stmt(rhs);
                // The TARGET's own establishment comes after its value, because that is when
                // the slot is written — and it is read off the whole statement, since a
                // struct-enum literal's mint names a work-ref and only the `Set` says which
                // variable took it (`established_stores`).  Skipping it here left an enum
                // subject reassigned inside a branch arm disturbing nothing at all.
                self.disturb(stmt);
                self.record_target(stmt);
            }
            other => self.leaf(other),
        }
    }

    /// Walk a nested block in its own frame: a view bound inside one dies with it.
    fn scoped(&mut self, stmts: &[Value]) {
        self.open.push(Vec::new());
        self.walk_block(stmts);
        self.open.pop();
    }

    /// Shake for everything `stmt` disturbs, at any depth inside it.
    fn disturb(&mut self, stmt: &Value) {
        if let Value::Set(v, _) = stmt.unspan()
            && !self.function.is_compiler_generated(*v)
            && matches!(
                self.function.tp(*v).peel_link(),
                Type::Vector(..)
                    | Type::Hash(..)
                    | Type::Index(..)
                    | Type::Sorted(..)
                    | Type::Radix(..)
                    | Type::Trie(..)
            )
        {
            self.rebound = Some(*v);
        }
        self.shake_places_keyed(
            &reshaped_containers(stmt, self.data, self.function),
            ViewCause::Reshaped,
            None,
            &keyed_removal_keys(stmt, self.data, self.function),
        );
        stmt.walk(&mut |v| {
            let Value::Call(d, args) = v else { return };
            if self.data.def(*d).name() != "OpClearVector" {
                return;
            }
            if let Some(place) = args
                .first()
                .and_then(|a| base_container_place(a, self.data))
            {
                self.cleared.insert(place);
            }
        });
        self.shake_places(
            &grown_containers(stmt, self.data, self.function, self.database, &self.cleared),
            ViewCause::Grown,
            None,
        );
        if let Some(removed) = self.cross_frame {
            for (container, callee) in reshaped_via_call(stmt, self.data, removed) {
                self.shake(
                    &HashSet::from_iter([container]),
                    ViewCause::Reshaped,
                    Some(callee),
                );
            }
        }
        self.disturb_via_calls(stmt);
        let established = established_stores(stmt, self.function, self.data);
        self.shake(&established, ViewCause::Reassigned, None);
    }

    /// @PLN164 C3 (@FR-B-Disturb) — shake for every container place a CALLEE `stmt` invokes
    /// disturbs through the arguments it was handed.
    ///
    /// `(B-Disturb)`'s events end a place wherever they happen, and the producers above see
    /// only this frame's ops, so a view survived a growth one frame down and kept reading the
    /// address its elements had left: measured, `e = sc.els[0]?; grow(sc); e.a + e.b` answered
    /// `4294967401` on BOTH backends where the same append written inline answers `3` and says
    /// so, and a removal one frame down read the element that shifted in. One shape, two
    /// meanings, decided by which side of a call the append sits on.
    ///
    /// The callee's fact is [`DisturbedParams`]; [`compose_param_place`] maps it onto the
    /// argument this frame passed. The cause travels with it so the advice names the growth or
    /// the removal, and `via` names the callee so the report points at the call rather than at
    /// the container's own line.
    fn disturb_via_calls(&mut self, stmt: &Value) {
        let Some(disturbed) = self.disturbed else {
            return;
        };
        let mut hits: Vec<(ParamPlace, ViewCause, u32)> = Vec::new();
        let data = self.data;
        stmt.walk(&mut |v| {
            let Value::Call(d, args) = v else { return };
            let Some(places) = disturbed.get(d) else {
                return;
            };
            for (&(slot, inner), &cause) in places {
                let Some(arg) = args.get(usize::from(slot)) else {
                    continue;
                };
                let Some(base) = call_arg_place(arg, data) else {
                    continue;
                };
                // A place this frame CLEARED is being rebuilt, not disturbed — the same
                // subtraction the inline growth makes, applied to the callee's half.
                if let Some(place) = compose_param_place(base, inner)
                    && !self.cleared.contains(&place)
                {
                    hits.push((place, cause, *d));
                }
            }
        });
        for (place, cause, callee) in hits {
            self.shake_plain_places(&HashSet::from_iter([place]), cause, Some(callee));
        }
    }

    /// [`Self::shake_places`], with the prior state restored afterwards for every binding in
    /// [`Self::whole_container`].  The callee path's shake — [`Self::disturb_via_calls`] is the
    /// only caller.
    ///
    /// What it spares is `(B-Ref-Alias)`'s in-versus-to distinction, NOT a `&`-versus-plain one.
    /// Entry into that map needs BOTH `is_amp_container_link` and a chain that read no element,
    /// so it holds exactly the `&` links naming a container WHOLE (`d = &cv.data`).  A `&` link
    /// INTO a container (`e = &v[0]`) is shaken here like any other view, deliberately: a growth
    /// moves the element that link names, where it only repoints the field SLOT the
    /// whole-container link re-reads.
    ///
    /// The restore is keyed by VIEW, where [`Self::names_container_itself`] matches the place
    /// EXACTLY — so a whole-container link is spared from every place one call disturbs,
    /// including a disturbance of the whole variable, which [`compose_param_place`] answers as
    /// `ANY_FIELD` and [`same_place`] matches by wildcard.
    ///
    /// What the sparing is for: `d = &cv.data; grow(cv, 7); grow(cv, 8); d[2]` is
    /// `157-view-header`'s `grown_between`, which must read `11`.  Shaking the link gives `0`.
    ///
    /// TWO consumers read this one answer and want opposite things from the `&` links it does
    /// shake.  @FR-B-View materialises a plain bind, because a plain bind already meant value
    /// semantics and losing the alias is consistent with what it meant.  @FR-B-Ref-Reshape
    /// REFUSES a `&` reference into a disturbed container — *"loft will not quietly downgrade
    /// the reference to a copy"* — and `reshape_refusals` reads this walk's answer for the
    /// callee's half too (`binding.md` D-bind-48), which is why this function may not start
    /// sparing `&` links INTO a container.
    fn shake_plain_places(
        &mut self,
        places: &HashSet<(u16, u32)>,
        cause: ViewCause,
        via: Option<u32>,
    ) {
        // A callee hit carries `Grown` or `Reshaped` and never `Reassigned`:
        // `disturbed_param_places` inserts only those two, and `Reassigned` is raised on the
        // INLINE path alone (`self.shake(&established, ViewCause::Reassigned, None)`) — a
        // callee cannot re-establish its caller's binding, which is what that event means.
        //
        // So `names_container_itself`'s `cause != Reassigned` clause is always true HERE and
        // load-bearing only on the inline side: one predicate, one live dimension per path.
        // Asserted rather than described, because the premise lives in another function and a
        // third cause added there would make this silently load-bearing on a path no cell
        // exercises (loft#1543).
        debug_assert!(
            cause != ViewCause::Reassigned,
            "a callee hit carried Reassigned — `disturbed_param_places` grew a cause, and \
             `names_container_itself` is now load-bearing on the callee path too"
        );
        let links: Vec<(u16, u16, u32)> = self
            .open
            .iter()
            .flatten()
            .filter(|(view, _, _)| self.whole_container.contains_key(view))
            .copied()
            .collect();
        let before: HashMap<u16, Option<Disturbance>> = links
            .iter()
            .map(|(v, _, _)| (*v, self.shaken.get(v).copied()))
            .collect();
        self.shake_places(places, cause, via);
        for (view, prior) in before {
            match prior {
                Some(d) => {
                    self.shaken.insert(view, d);
                }
                None => {
                    self.shaken.remove(&view);
                }
            }
        }
    }

    /// One statement, in the order its parts take effect: what it disturbs, then what it
    /// uses, then what it (re)binds.
    fn leaf(&mut self, stmt: &Value) {
        self.note_binding_depth(stmt);
        self.disturb(stmt);
        // Reading or writing a shaken view is what makes the disturbance matter.
        self.note_uses(stmt);
        self.record_target(stmt);
    }

    /// What a `Set` does to the walk's state once its VALUE has been accounted for — the last
    /// of [`Self::leaf`]'s three steps, and the only one a value-branch `Set` still owes after
    /// its arms have been walked in their own order.
    ///
    /// A `Set` REPLACES whatever the slot held, so the old binding's troubles end here and a
    /// view bound by this statement is live from here on.  Recorded LAST, so a statement that
    /// re-establishes a container and binds a view of the NEW value does not mark the fresh
    /// view against its own establishment.
    fn record_target(&mut self, stmt: &Value) {
        if let Value::Set(v, rhs) = stmt.unspan() {
            // A write through a place link binds nothing: the link still names the place it
            // named, so its open view and any shake of it stand (`note_uses` saw the write).
            if is_place_link(self.function, *v)
                && !link_set_repoints(self.data, self.function, *v, rhs)
            {
                return;
            }
            self.shaken.remove(v);
            for frame in &mut self.open {
                frame.retain(|(view, _, _)| view != v);
            }
            // Re-read per bind, exactly as `view_keys` is: a slot rebound from an element
            // read must not keep an earlier bind's whole-container answer (loft#1543).
            self.whole_container.remove(v);
            // Through `base()`: a nullable `S?` view is the same storage behind a
            // nullability marker (@FR-L-Null), so it is at risk exactly as its dense twin is.
            //
            // A COLLECTION-typed view is here too, because `(B-View-Depth)` makes an index
            // read a view "whatever the element type" and the materialise arm now has a
            // collection case (loft#1377): the record path strips the container dep and lets
            // the BIND copy, which for a collection is decided at PARSE time and cannot hear
            // a scope-pass strip, so the copy is emitted here instead.
            //
            // A bare `&` link to a whole VARIABLE is not recorded either, and that is
            // `base_container_var`'s doing rather than this list's: it answers `None` unless
            // the right-hand side is a PROJECTION, so `pe = &e` names no container while
            // `pw = &w[0]` names `w`.  That is the in-versus-to distinction `(B-Ref-Alias)`
            // needs, and it lives in one place.
            //
            // ⚠ That answer does not reach a link to a whole container held in a FIELD:
            // `pd = &w.data` IS a projection, so it names `(w, off_data)` — the same place
            // `w.data[0]` names — and the distinction has to be drawn one level finer.  It is
            // drawn below, on `Self::whole_container`, and spent at the shake rather than here:
            // such a link must still be shaken by a REASSIGNMENT of `w`, which is the one
            // disturbance that leaves its slot with nothing to point at (loft#1543).
            // ⚠ THESE TWO TESTS ANSWER DIFFERENT QUESTIONS, AND BOTH ARE LOAD-BEARING.  The
            // type list says WHICH BINDINGS CAN BE VIEWS AT ALL; `value_view_place` says
            // WHICH PLACE a value views.  They were widened for different defects, on
            // different branches, and met here at a cherry-pick — so the pairing reads as an
            // accident of adjacency in the history and is not one.  Narrowing either silently
            // un-fixes a shipped defect, and none of them fails loudly:
            //
            //   * drop `Type::Vector` (or the `.base()` that reaches it through a `τ?`) and a
            //     COLLECTION view is never named, which is loft#1377 and loft#1399 — the
            //     latter answers correctly on `--native` either way, so the interpreter goes
            //     quietly wrong on one backend only;
            //   * drop `value_view_place` back to `base_container_place` and a BRANCH-valued
            //     binding names no container — which costs loft#1396 AND loft#1399, since the
            //     latter's binding is branch-valued too, and a `??`-DISCHARGED projection names
            //     none either, which is loft#1401.
            //
            // Both narrowings were MEASURED rather than reasoned, by making each one and
            // running the guards: dropping `Type::Vector` fails
            // `1377-a-collection-typed-element-view-materialises-too` and
            // `a-collection-projection-arm-of-a-branch-materialises` while loft#1396's guard
            // stays green; dropping the namer fails loft#1396's guard AND loft#1399's, because
            // a branch-valued binding is not named at all without it.
            //
            // Naming and copy have to land together — a named binding whose deps are stripped
            // with no emitter copy owns a store it only views (loft#778's class), which is why
            // widening the type list ALONE was measured unsound.  The guards for those three
            // issues are this line's regression net; nothing names the pairing itself, so it
            // is named here.  `binding-history.md` D-bind-23 carries the history.
            // A binding the loop ITERATES is not materialised: the iteration depends on its
            // identity, so a store of its own makes the loop walk a COPY while the body's
            // `#remove` empties the original — which does not terminate.  Measured: once a
            // removal reached through a FIELD became a disturbance (`D-bind-26`),
            // `for e in d.items { e#remove; }` shook the loop's own source temp — a view of
            // `(d, off_items)` by every test this walk applies — and `903-loop-remove` went
            // from 0.06s to a 300s corpus timeout.
            //
            // ⚠ The obvious wider rule is WRONG here, and was measured wrong: *"a view the
            // author cannot name"* excludes a `match` PAYLOAD binding too, which the parser
            // renames to `_mv_<field>_N` and which the author very much wrote — so
            // `a-payload-binding-warns-when-its-subject-is-given-another-variant` read its
            // subject's new variant.  The fact belongs on the variable the lowering created,
            // not on the shape of its name.
            // A `&` link to a SCALAR or TEXT place (`c = &v[1]`, `c = &o.v[0].n`, `t = &o.s`) is
            // a view by the same relation a record `&` link is: it holds the place's `DbRef`,
            // and a growth or removal moves the element under it.  `(B-Ref-Reshape)` keys on
            // that aliasing relation, not on the element type, so it opens here too.  A link to
            // a LOCAL (`c = &x`) names no container — `value_view_places` answers nothing for
            // `OpCreateStack` — so it opens nothing.  The materialise side never meets one:
            // an `&` link is refused at the disturbance rather than copied.
            // A `text` PAYLOAD binding whose writes are mirrored into its subject's field
            // (#673) is a view of that field on the same terms: the mirror is its write-through,
            // and `(B-View)` ends it where the subject is disturbed (loft#1665).
            if !self.function.is_iteration_source(*v)
                && (matches!(
                    self.function.tp(*v).base(),
                    Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                ) || is_place_link(self.function, *v)
                    || self.function.text_payload_views.contains(v)
                    || self.function.group_write_views.contains_key(v)
                    || keyed_payload_view(self.function, *v)
                    // A keyed `&` link (`a = &o.h`) is a view on the same terms as the vector
                    // one, and was never opened here, so reassigning `o` left it following the
                    // NEW value where `(B-Ref-Reshape)` refuses the program (loft#1759).  The
                    // refusal reads `is_amp_container_link` too; the materialise does not copy
                    // it — it is not a `keyed_views` binding.
                    || (crate::parser::vectors::is_keyed(self.function.tp(*v))
                        && self.function.is_amp_container_link(*v)))
            {
                // The view belongs to the frame that owns its VARIABLE. Re-binding an outer
                // local inside a nested block gives a view that outlives the block, and
                // dropping it at the block's close is what let loft#1184 through: `a =
                // w.inner` in a loop body, `w = Outer{inner: a}` on the next turn.
                let depth = self.bound_at.get(v).copied().unwrap_or(self.open.len());
                let idx = depth.min(self.open.len()).saturating_sub(1);
                // `(B-Disturb)` ends the place a view names when its CONTAINER is disturbed,
                // and a chain of views names one place however many statements it is spelled
                // over.  `base_container_place` resolves a chain inside ONE expression
                // (`dv.tiles.proto` names `dv`); split through a local it does not —
                // `t = dv.tiles; prev = t.proto` recorded `prev` as a view of `t`, so
                // reassigning `dv` shook `t` and left `prev` reading the new value, on both
                // backends and with nothing said, where the one-expression spelling
                // materialises and says so (loft#1393).  Resolve through the views already
                // open, which is the same walk one level out.
                // One entry per PLACE the value can name.  The frame already holds a
                // `(view, container, field)` triple per pair, so a binding that views two
                // containers is two entries and `shake_places` matches either — no new shape,
                // and `record_target`'s own `retain` clears them all when the slot is rebound.
                // loft#1460 — the key this view names, when it names a literal one.  Read
                // from the SAME `OpGetRecord` reader the removal side uses, and re-read on
                // every rebind (the `retain` above already dropped the old entry), so a slot
                // rebound from a computed key cannot keep an earlier bind's literal.
                match get_record_literal_keys(rhs, self.data) {
                    Some(keys) => {
                        self.view_keys.insert(*v, keys);
                    }
                    None => {
                        self.view_keys.remove(v);
                    }
                }
                // `(B-Ref-Alias)`'s in-versus-to distinction, one level finer than the
                // `pe = &e` case the comment above describes: `d = &cv.data` is a reference
                // TO the container, where `e = &cv.data[0]` is one INTO it.  Both name the
                // place `(cv, off_data)` — a place is one variable and one field offset — and
                // `(B-Disturb)`'s growth tells them apart: it moves every ELEMENT, ending the
                // second, while it only repoints the field SLOT the first re-reads.
                //
                // The `&` is what qualifies, and asking for it is not belt-and-braces: a PLAIN
                // whole-collection bind reaches this walk too.  Off an owned base and off a
                // borrowed PARAMETER it copies at parse time into its own `__vdb_N` backing,
                // whose container is compiler-generated and already unnamed — but off a LOOP
                // VARIABLE it aliases, and `for b in bv { c = b.vecf; b.vecf += [9] }`
                // materialises `c` today.  `(B-View)` says it must keep doing so: a plain bind
                // already meant value semantics, so losing write-through is consistent with
                // what it asked for, and only a `&` is the ownership decision loft may not
                // quietly downgrade.  Measured before this was written.
                // A `match` PAYLOAD binding (`_mv_<field>`) is the other reference TO a
                // container: the parser routes a write through it to the payload's own field
                // (`items += [x]` appends to `e.items`), so it re-reads that slot exactly as a
                // `&` link does, and the growth its own append causes must not end it.  Ended
                // there, it was materialised, the append landed in the payload and every read
                // of `items` saw the copy — `len(items)` answered 1 after `items += [121]` for a
                // fused push into a variant's vector (the unfused append happened to be spared
                // because its field NUMBER could not be placed on the enum's type).
                let payload_view = self.function.name(*v).starts_with("_mv_");
                if (self.function.is_amp_container_link(*v) || payload_view)
                    && let Some((place, false)) =
                        crate::use_analysis::view_source_place_indexed(self.data, rhs)
                {
                    self.whole_container.insert(*v, place);
                }
                let mut places = value_view_places(rhs, self.data, self.function);
                if places.is_empty() && self.function.text_payload_views.contains(v) {
                    places.extend(text_payload_place(rhs, self.data, self.function));
                }
                for (container, field) in places {
                    let (container, field) = self.resolve_view_root(container, field);
                    if !self.open[idx].contains(&(*v, container, field)) {
                        self.open[idx].push((*v, container, field));
                    }
                }
            }
        }
    }

    /// The container a view ultimately names, following views this walk has already opened.
    ///
    /// A view whose container is itself a view names a place inside the OUTER container, so a
    /// disturbance of that outer one ends it: `t = dv.tiles; prev = t.proto` makes `prev` a
    /// place inside `dv`, exactly as the single-expression `prev = dv.tiles.proto` does.  The
    /// FIELD that survives is the outermost one — the one a disturbance of the root can name —
    /// which is the rule [`base_container_place`] states for a chain inside one expression.
    ///
    /// Bounded, and it stops at the first container that is not an open view: a view rebound
    /// to name a chain that leads back to itself would otherwise walk forever, and a lower
    /// bound is what this walk answers everywhere else.
    fn resolve_view_root(&self, mut container: u16, mut field: u32) -> (u16, u32) {
        for _ in 0..16 {
            let Some(&(_, outer, outer_field)) = self
                .open
                .iter()
                .flatten()
                .find(|(view, _, _)| *view == container)
            else {
                return (container, field);
            };
            // Stop at a COMPILER-GENERATED container.  A vector local is bound to its own
            // backing store — `v = OpGetField(__vdb_1, 0, …)` — so it is an open "view" of a
            // local nothing in the program can disturb, and following it moved `c = &v[0]`
            // off `v` and onto `__vdb_1`: the `(B-Ref-Reshape)` refusal for `v.remove(2)`
            // under a live link then had no container to match and stopped firing.  A place
            // is only inside a container the author can reassign.
            if outer == container || self.function.is_compiler_generated(outer) {
                return (container, field);
            }
            container = outer;
            field = outer_field;
        }
        (container, field)
    }

    /// Note where each variable is first BOUND, which is [`Self::leaf`]'s frame for a view of it.
    ///
    /// A `Set(v, Null)` is the hoisted declaration every ref- and text-typed local gets at
    /// function scope, not a binding, so it is not what owns the variable.
    fn note_binding_depth(&mut self, stmt: &Value) {
        if let Value::Set(v, rhs) = stmt.unspan()
            && !matches!(rhs.unspan(), Value::Null)
        {
            self.bound_at.entry(*v).or_insert(self.open.len());
        }
    }

    /// Mark every open view of one of `containers` as disturbed. Not a verdict yet — only a
    /// use after this point makes the view wrong, which is what separates one that is live
    /// across the disturbance from one that is already dead.
    fn shake(&mut self, containers: &HashSet<u16>, cause: ViewCause, via: Option<u32>) {
        if containers.is_empty() {
            return;
        }
        let places: HashSet<(u16, u32)> = containers.iter().map(|&c| (c, ANY_FIELD)).collect();
        self.shake_places(&places, cause, via);
    }

    /// [`Self::shake`] where the disturbance names a PLACE rather than a whole variable — a
    /// growth of one FIELD does not end the places inside its siblings.  A view and a
    /// disturbance match when [`same_place`] says they name the same storage.
    fn shake_places(&mut self, places: &HashSet<(u16, u32)>, cause: ViewCause, via: Option<u32>) {
        self.shake_places_keyed(places, cause, via, &HashMap::default());
    }

    /// Does `view` name the CONTAINER at `place` itself, so that `cause` does not end it?
    ///
    /// Three of `(B-Disturb)`'s four events — a removal, a growth, a re-key — move the
    /// ELEMENTS of a container.  A reference INTO it names one of those positions, so they end
    /// it; a reference TO it names the field SLOT the container lives in, which a growth
    /// repoints and the link re-reads, so they do not.  The fourth event does end it:
    /// reassigning the base gives that slot a new value and leaves nothing to re-read, which
    /// is why `Reassigned` is excluded here rather than handled by the caller (loft#1543).
    ///
    /// Matched EXACTLY against the place the binding names directly — not through
    /// [`same_place`], whose wildcard would let a disturbance of the whole variable spare a
    /// binding that names one field of it.  Sparing less is the safe direction: it costs a
    /// materialise, where the other costs a program its meaning.
    fn names_container_itself(&self, view: u16, place: (u16, u32), cause: ViewCause) -> bool {
        cause != ViewCause::Reassigned && self.whole_container.get(&view) == Some(&place)
    }

    /// [`Self::shake_places`] with the keys a keyed REMOVAL named, so a view of a DIFFERENT
    /// record is spared (loft#1460).
    ///
    /// ⚠ The filter may only ever SPARE, and only on proof.  Both sides must be literal and
    /// they must differ; an absent key on either side means *could be the same record* and
    /// shakes, which is what keeps `sorted`, every computed key, and every spelling this
    /// cannot read exactly where they were.  Getting that direction backwards turns a
    /// conservative rule into a silent one, which is the defect this closes.
    fn shake_places_keyed(
        &mut self,
        places: &HashSet<(u16, u32)>,
        cause: ViewCause,
        via: Option<u32>,
        removal_keys: &HashMap<(u16, u32), Vec<Value>>,
    ) {
        if places.is_empty() {
            return;
        }
        let spared = |view: u16, place: (u16, u32)| -> bool {
            let (Some(removed), Some(held)) = (removal_keys.get(&place), self.view_keys.get(&view))
            else {
                return false;
            };
            removed.len() == held.len() && removed != held
        };
        let hit: Vec<(u16, u16, u32)> = self
            .open
            .iter()
            .flatten()
            .filter(|(view, container, field)| {
                places.iter().any(|&p| {
                    same_place((*container, *field), p)
                        && !spared(*view, p)
                        && !self.names_container_itself(*view, p, cause)
                })
            })
            .copied()
            .collect();
        for (view, container, _) in hit {
            let d = Disturbance {
                cause,
                line: self.line,
                via,
                container,
                replaced: cause == ViewCause::Grown
                    && via.is_none()
                    && self.rebound == Some(container),
            };
            record_cause(&mut self.shaken, view, d);
        }
    }

    /// Condemn every shaken view this statement reads or writes.
    ///
    /// A `Set`'s target is a `u16` slot rather than a `Value::Var`, so a walk for `Var` nodes
    /// already counts only genuine reads — a rebind does not read the binding it replaces.
    fn note_uses(&mut self, stmt: &Value) {
        if self.shaken.is_empty() {
            return;
        }
        let mut used: Vec<u16> = Vec::new();
        stmt.walk(&mut |v| {
            if let Value::Var(x) = v {
                used.push(*x);
            }
        });
        // A `Set` that writes THROUGH a place link uses it as surely as a read does: the
        // write lands on the place the link names (`c = &v[1]; v += [x]; c = 99`).  Only a
        // `Set` that re-points it is a new binding, which `record_target` handles.
        if let Value::Set(x, rhs) = stmt.unspan()
            && is_place_link(self.function, *x)
            && !link_set_repoints(self.data, self.function, *x, rhs)
        {
            used.push(*x);
        }
        // A write through a group-member binding is spelled against its origin FIELD and names
        // the binding only through the element it builds, whose type records the binding as
        // what it lives in — so that element is a use of the binding (loft#1664).
        let via_elements: Vec<u16> = used
            .iter()
            .flat_map(|x| self.function.tp(*x).depend())
            .filter(|d| self.function.group_write_views.contains_key(d))
            .collect();
        used.extend(via_elements);
        for v in used {
            if let Some(cause) = self.shaken.get(&v).copied() {
                record_cause(&mut self.out, v, cause);
            }
        }
    }
}

/// @FR-B-Ref-Repoint — whether `Set(var, value)` on a link RE-POINTS it rather than writing
/// through it.  `p = &q` re-points; `p = 99` writes the place.  For a link to a SCALAR the
/// right-hand side's TYPE separates the two: a place (`c = &v[0]`, `f = &o.y`) arrives as the
/// place op itself, declared to return a reference, where a value read out of a place is typed
/// as the scalar.  For a record or collection link an element's VALUE is a reference too, so
/// only the install ops (`OpCreateStack`, `OpVarRef` — a re-point to the link another link
/// holds) count.  Every write through a store-kind text link is parsed as the field's setter,
/// so a `Set` of one is always its bind (@PLN167 decision 2).  Asked by the interpreter's
/// `set_var` and by the view walk, so the two cannot disagree about which statement binds.
pub(crate) fn link_set_repoints(data: &Data, function: &Function, var: u16, value: &Value) -> bool {
    let Type::RefVar(tp) = function.tp(var).base() else {
        return false;
    };
    let scalar_link = matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Enum(_, false, _)
    );
    if let Value::Call(d, _) = value.unspan() {
        let def = data.def(*d);
        matches!(def.name(), "OpCreateStack" | "OpVarRef")
            || (scalar_link && matches!(def.returned.base(), Type::Reference(_, _)))
            || function.is_store_text_link(var)
    } else {
        false
    }
}

/// Whether `v` is a `&` link the author wrote, in either of its two spellings: a struct
/// projection the parser leaves unlowered and marks (`Function::is_amp_link`), or a scalar or
/// text place it LOWERS to a `RefVar` local (`c = &v[1]`, `t = &o.s`).  A local is typed
/// `RefVar` only by a `&` bind; a `&` PARAMETER is `RefVar` too but is an argument, bound by
/// its caller and never by a `Set` the walk sees.  @FR-B-Ref-Reshape keys on the aliasing
/// relation, so both spellings reach it.
pub(super) fn is_place_link(function: &Function, v: u16) -> bool {
    !function.is_argument(v) && matches!(function.tp(v).base(), Type::RefVar(_))
}

/// Every variable whose store `stmt` establishes, at any depth.
///
/// Nested blocks are included deliberately: `if flag { bx = T{…} }` establishes `bx` as far
/// as a view bound outside the `if` is concerned.
fn established_stores(stmt: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    let mut out: HashSet<u16> = HashSet::default();
    // @FR-L-Null — `.base()` at each of the three record tests below: a `?` is a compile-time
    // bit over the SAME storage, so `S?` establishes a store exactly as `S` does.  Asked bare,
    // a nullable local's reassignment established nothing, so a view into it was neither
    // materialised nor reported: `o: Q? = Q { … }; v = o.p; o = Q { … }; v.a` read a released
    // record on both backends while its dense twin copied `v` out and said so (loft#1442).
    let note = |v: u16, out: &mut HashSet<u16>| {
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && !function.is_compiler_generated(v)
        {
            out.insert(v);
        }
    };
    stmt.walk(&mut |v| match v {
        Value::Call(d, args) if data.def(*d).name() == "OpDatabase" => {
            if let Some(Value::Var(t)) = args.first().map(Value::unspan) {
                note(*t, &mut out);
            }
        }
        // A call that reassigns one of its `&` parameters displaces the caller's store.
        Value::Call(d, args) if data.def(*d).name.starts_with("n_") => {
            let reassigned = reassigned_ref_params(data, *d);
            for (i, arg) in args.iter().enumerate() {
                if !u16::try_from(i).is_ok_and(|i| reassigned.contains(&i)) {
                    continue;
                }
                if let Value::Var(t) = peel_stack_ref(arg, data) {
                    note(*t, &mut out);
                }
            }
        }
        Value::Set(t, rhs) => {
            // `Set(v, Null)` is the in-place re-init prelude that PRECEDES an `OpDatabase`
            // for the same var, not an establishment of its own.
            let establishes = match rhs.unspan() {
                Value::Var(src) => matches!(
                    function.tp(*src).base(),
                    Type::Reference(_, _) | Type::Enum(_, true, _)
                ),
                Value::Call(f, _) => data.def(*f).name.starts_with("n_"),
                // A struct-ENUM literal builds into a work-ref and hands the variable that
                // block: `sh = { OpDatabase(__ref_p2_2, …); …; __ref_p2_2 }`.  The tail is
                // the same `Var` establishment the first arm names, one wrapper out — and
                // read only through the wrapper's own `OpDatabase`, which names the COMPILER
                // work-ref and is filtered out, no reassignment of a struct-enum local
                // established anything.  A payload view of it then survived the subject's
                // reassignment with no materialise and no warning, where the plain-struct
                // twin (which builds in place, `OpDatabase(h)`) materialises and says so.
                // `Value::tail` stops at an `If`, so a `??` or a branch-valued right-hand
                // side still answers "no" here.
                Value::Block(_) | Value::Insert(_) => matches!(
                    rhs.tail().unspan(),
                    Value::Var(src) if matches!(
                        function.tp(*src).base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                ),
                _ => false,
            };
            if establishes {
                note(*t, &mut out);
            }
        }
        _ => {}
    });
    out
}
