// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Tuple MEMBERS that own their element: the frees a tuple local owes per member, which members
//! a call minted, and a member assignment over an element the tuple owns alone.

use super::Scopes;
use super::backings::delivered_work_ref;
use crate::data::{Data, Type, Value, v_if, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// The scope-exit / pre-reassignment frees for a TUPLE local's OWNED elements, in
/// reverse index order.
///
/// `Value::TupleGet(v, idx)` reads one element as a plain `DbRef` / `Str`, so each
/// free is the ordinary op on that read and needs no per-element stack-offset
/// machinery of its own.
///
/// Only an element the tuple OWNS. `owned_elements` answers on the type KIND — is
/// it heap-shaped — which a BORROWED element passes exactly as an owned one does,
/// and freeing that releases the source's store.  The empty-dep test is the
/// ownership half, the same one the scalar branch of `get_free_vars` uses.
///
/// STORE-backed elements only.  A tuple's `text` element is a `Str` VIEW — `put_text`
/// stores the pointer+length pair, never an owning `String` — so there is nothing to
/// release, and `OpFreeText` on the element reads that view as a `String` (a SIGSEGV;
/// loft#1004 was the operand arithmetic underflowing before it got that far).  The
/// bytes a tuple element views belong to whatever built them: a literal, or a `__ncc_N`
/// temp its consumer frees.
/// What the scan knows about a tuple's members when it releases them: which the tuple's latest
/// assignment minted (`Scopes::tuple_call_mint`), which buffers a copy took the release from
/// (`Scopes::drop_transferred`), and which members a move in an arm may have taken
/// (`Scopes::tuple_moved`).  `NONE` knows nothing: the bare release.
#[derive(Clone, Copy)]
pub(super) struct MemberFacts<'a> {
    pub(super) call_mints: Option<&'a HashMap<u16, Option<u16>>>,
    pub(super) handed: Option<&'a HashSet<u16>>,
    pub(super) moved: Option<&'a HashMap<(u16, u16), u16>>,
}

impl MemberFacts<'_> {
    pub(super) const NONE: MemberFacts<'static> = MemberFacts {
        call_mints: None,
        handed: None,
        moved: None,
    };
}

/// `@FR-T-Record` — the frees a tuple LOCAL owes its `text` members at scope exit: one
/// `OpFreeText(TupleGet(v, i))` per member that is text or a tuple carrying text, which the
/// interpreter's codegen expands to every owned text leaf under the member.  A member is the
/// tuple's own text only where the slot owns it (`data::TUPLE_LOCAL_TEXT_OWNED`); a by-value
/// parameter borrows its members, and a never-free binding frees nothing.
pub(super) fn tuple_text_member_frees(
    elems: &[Type],
    v: u16,
    data: &Data,
    function: &crate::variables::Function,
) -> Vec<Value> {
    fn carries_text(t: &Type) -> bool {
        match t.base() {
            Type::Text(_) => true,
            Type::Tuple(inner) => inner.iter().any(carries_text),
            _ => false,
        }
    }
    if !function.tuple_owns_text(v) || function.is_skip_free(v) {
        return Vec::new();
    }
    let free_text = data.def_nr("OpFreeText");
    (0..elems.len())
        .rev()
        .filter(|&i| carries_text(&elems[i]))
        .map(|i| Value::Call(free_text, vec![Value::TupleGet(v, i as u16)]))
        .collect()
}

pub(super) fn tuple_owned_elem_frees(
    elems: &[Type],
    v: u16,
    data: &Data,
    function: &crate::variables::Function,
    facts: MemberFacts,
    only: Option<usize>,
) -> Vec<Value> {
    let MemberFacts {
        call_mints,
        handed,
        moved,
    } = facts;
    let mut out = Vec::new();
    for &(_offset, idx) in crate::data::owned_elements(elems).iter().rev() {
        // loft#1532 — a member ASSIGNMENT asks for its ONE element: the same ownership test,
        // with no pairing handed in, is the bare release `(H-Drop-Not)` gives a displaced field.
        if only.is_some_and(|o| o != idx) {
            continue;
        }
        // @FR-O-Proxy asks free, and @FR-O-Override vetoes it at every such site — this is one:
        // the element test concludes "this element owns its store" from an empty dep list,
        // which is @FR-O-Proxy and unsound alone.  `OpFreeRef(TupleGet(v, i))` releases
        // storage reached through `v`, so a binding the parser marked never-free is
        // never-free here too — and a tuple has no dep list of its own to say it through.
        //
        // The test reads NEGATED, guarding a `continue`, so the free is on the FALL-THROUGH
        // and the site concludes ownership exactly as a positive test would.  Reading the
        // `!` as "this asks whether it is a borrow" is what kept this site out of
        // `scripts/o_proxy_check.py`'s obligation set entirely.
        //
        // ⚠ NOT [`crate::variables::Function::proxy_says_owned`], and it cannot be: the proxy
        // is read off a tuple ELEMENT's type and the veto off the CONTAINER binding `v`, so
        // the question has TWO subjects where that predicate has one (@PLN155 phase 1).
        // Folding it would have to invent a dep list for the element, which is the very thing
        // a tuple does not have.
        //
        // A member the tuple's LATEST assignment minted is owned whatever the tuple's type
        // says: that type is one per binding and names the deps of whichever assignment parsed
        // last, so after `u = (mk(1), 0); … u = t` it read the first bind's minted member as a
        // view of the move's backing and released nothing (D-heap-15).  The scan's
        // `tuple_call_mint` is per assignment (`@FR-O-Latest`) and answers it.  A member whose
        // deps name its own claimant is that claimant's record, released through the
        // claimant's own free (the move's backing), so only a claimant the deps do NOT name
        // overrides them.
        let deps = elems[idx].depend();
        let minted = call_mints
            .and_then(|m| m.get(&(idx as u16)))
            .is_some_and(|claim| claim.is_none_or(|w| !deps.contains(&w)));
        // @FR-O-Proxy asks free — the element release below, vetoed by @FR-O-Override.
        if function.is_skip_free(v)
            || (!minted && !deps.is_empty())
            || matches!(elems[idx].base(), Type::Text(_))
        {
            continue;
        }
        // loft#1511 / `(H-Drop)` — an element whose record was minted by the element's own
        // initializing CALL is a frame-owned droppable: run the type's cascade before the
        // free (the bare free released the resource without its hook), then disarm the
        // call's hidden buffer, whose own scope-end drop+free claims the same record on the
        // callees that deliver into it.  The sentinel is also what makes a LOOP re-mint per
        // iteration instead of overwriting the record in place (`OpDatabase` reuses a live
        // slot, which would lose every hook but the last one's).
        if let Some(&buf) = call_mints.and_then(|m| m.get(&(idx as u16))) {
            let elem = || Value::TupleGet(v, idx as u16);
            // A member whose release a copy took (`call_minted_member_handoff`) keeps its free
            // and loses only the hook: the copy is the member's owner now.
            let handed_off = buf.is_some_and(|b| handed.is_some_and(|h| h.contains(&b)));
            if !handed_off && let Some(d) = elems[idx].base().heap_def_nr() {
                let cascade = data.drop_cascade_nr(d);
                if cascade != u32::MAX {
                    let hook = v_if(
                        Value::Call(data.def_nr("OpConvBoolFromRef"), vec![elem()]),
                        Value::Call(cascade, vec![elem()]),
                        Value::Null,
                    );
                    // A move written in an arm took the hook on the path that ran it (loft#1645).
                    out.push(match moved.and_then(|m| m.get(&(v, idx as u16))) {
                        Some(&flag) => v_if(Value::Var(flag), Value::Null, hook),
                        None => hook,
                    });
                }
            }
            out.push(Value::Call(data.def_nr("OpFreeRef"), vec![elem()]));
            if let Some(b) = buf {
                out.push(v_set(
                    b,
                    Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                ));
            }
            continue;
        }
        out.push(Value::Call(
            data.def_nr("OpFreeRef"),
            vec![Value::TupleGet(v, idx as u16)],
        ));
    }
    out
}

/// loft#1511 — the elements of a tuple-literal RHS that are MINTED by their own initializing
/// call: element index → the call's hidden return buffer (`None` for a bufferless mint, e.g.
/// a nullable return).  A tuple a generator's advance produces is minted whole (`(G-Own)`).  `Some(map)` whenever the RHS is a tuple literal (so a reassignment
/// replaces a stale pairing with an empty one); `None` for any other RHS, telling the caller
/// to clear what it tracked.
///
/// Only a pairing this can PROVE is recorded — a loft-defined callee whose return either
/// delivers through the hidden buffer minted for this argument slot or adopts a fresh store
/// (`return_adopts_fresh_store`).  A callee whose return is tied to a real argument answers
/// neither and is skipped: the element then views storage somebody else owns, and the
/// fall-through keeps today's bare free.
pub(super) fn tuple_call_mints(
    rhs: &Value,
    function: &Function,
    data: &Data,
) -> Option<HashMap<u16, Option<u16>>> {
    // `formal/coroutines.md` `(G-Own)` — a tuple an ADVANCE produces is the consumer's whole:
    // every member arrives in a store the generator handed over, so each is its record's only
    // name and its release runs the type's hook, exactly as for an adopting call.
    if let Value::Call(d, args) = rhs.unspan()
        && *d == data.def_nr("OpCoroutineNext")
        && let Some(Value::Var(g)) = args.first().map(Value::unspan)
        && let Type::Iterator(inner, _) = function.tp(*g).base()
        && let Type::Tuple(elems) = inner.base()
        && crate::coroutine_layout::yield_handed_over(inner)
    {
        return Some((0..elems.len() as u16).map(|i| (i, None)).collect());
    }
    let Value::Tuple(members) = rhs.unspan() else {
        return None;
    };
    let mut out = HashMap::default();
    for (idx, m) in members.iter().enumerate() {
        if let Some(claim) = member_mint(m, function, data) {
            out.insert(idx as u16, claim.claimant());
        }
    }
    Some(out)
}

/// loft#1645 — [`tuple_call_mints`] for a value BRANCH whose every tail is a tuple literal
/// (`u = if c { (mk(1), 0) } else { (mk(2), 1) }`): a member every tail mints is the binding's
/// own.  Where the tails name the same claimant it stays; where they differ the member joins
/// as a sole owner, and each claimant is disarmed after the bind (the returned statements) —
/// a claimant's live record can only be that member, since each is the buffer of the one call
/// or construction that filled it.  `None` when a tail is not a literal: nothing is proven.
pub(super) fn branch_tuple_call_mints(
    rhs: &Value,
    function: &Function,
    data: &Data,
    sentinel: u32,
) -> Option<(HashMap<u16, Option<u16>>, Vec<Value>)> {
    fn tails<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
        match v.unspan() {
            Value::If(_, a, b) => {
                tails(a, out);
                tails(b, out);
            }
            Value::Block(bl) => match bl.operators.last() {
                Some(last) => tails(last, out),
                None => out.push(v),
            },
            _ => out.push(v),
        }
    }
    let mut ends = Vec::new();
    tails(rhs, &mut ends);
    let maps: Vec<HashMap<u16, Option<u16>>> = ends
        .iter()
        .map(|t| tuple_call_mints(t, function, data))
        .collect::<Option<_>>()?;
    let (first, rest) = maps.split_first()?;
    let mut out = HashMap::default();
    let mut claimants: Vec<u16> = Vec::new();
    for (&idx, &claim) in first {
        let others: Vec<Option<u16>> = rest.iter().filter_map(|m| m.get(&idx).copied()).collect();
        if others.len() != rest.len() {
            continue;
        }
        if others.iter().all(|&o| o == claim) {
            out.insert(idx, claim);
        } else {
            out.insert(idx, None);
            claimants.extend(std::iter::once(claim).chain(others).flatten());
        }
    }
    claimants.sort_unstable();
    claimants.dedup();
    let disarms = claimants
        .into_iter()
        .map(|w| v_set(w, Value::Call(sentinel, vec![])))
        .collect();
    Some((out, disarms))
}

/// What delivered a tuple member's record, where [`member_mint`] can PROVE it.
#[derive(Clone, Copy)]
enum MemberMint {
    /// A construction's work-ref or a call's hidden buffer still names the record: the
    /// claimant to disarm.
    Claimed(u16),
    /// A call that adopts a fresh store: the member is the record's only name.
    Adopted,
}

impl MemberMint {
    /// The spelling `Scopes::tuple_call_mint` keeps: the claimant, or `None` for a sole owner.
    fn claimant(self) -> Option<u16> {
        match self {
            Self::Claimed(w) => Some(w),
            Self::Adopted => None,
        }
    }
}

/// loft#1511 — one tuple MEMBER's mint, or `None` when this cannot PROVE one.  Asked of each
/// member of a literal ([`tuple_call_mints`]) and of the value a member assignment writes
/// (loft#1532), so the two cannot disagree about what was minted.
fn member_mint(m: &Value, function: &Function, data: &Data) -> Option<MemberMint> {
    // A member built by an inline CONSTRUCTION (`u = (S { … }, 9)`) is the literal
    // spelling of the same mint: the record is delivered through the construction's
    // work-ref, which both the element and the work-ref then name.  The element free
    // owes the hook exactly as for a call mint, and the work-ref is the buffer to
    // disarm — without it the work-ref's own scope-end cascade ran the hook on the
    // store the element free had already released (freed-then-hooked, poison-visible).
    if let Some(w) = delivered_work_ref(m, function, data) {
        return Some(MemberMint::Claimed(w));
    }
    let Value::Call(fn_nr, args) = m.unspan() else {
        return None;
    };
    let def = data.def(*fn_nr);
    if !def.is_loft_defined() {
        return None;
    }
    if let Some(i) = def.hidden_return_buffer_attr() {
        if let Some(Value::Var(vr)) = args.get(i).map(Value::unspan)
            && function.is_caller_hidden_buf(*vr)
        {
            return Some(MemberMint::Claimed(*vr));
        }
        None
    } else if def.return_adopts_fresh_store() && def.returned().base().heap_def_nr().is_some() {
        Some(MemberMint::Adopted)
    } else {
        None
    }
}

/// loft#1532 — every `(tuple variable, element)` a member assignment writes in `code`.
pub(super) fn written_tuple_members_in(code: &Value) -> HashSet<(u16, u16)> {
    let mut out = HashSet::default();
    code.walk(&mut |v| {
        if let Value::TuplePut(t, i, _) = v {
            out.insert((*t, *i));
        }
    });
    out
}

impl Scopes<'_> {
    /// loft#1532 — a member ASSIGNMENT `t.i = e` over an element the tuple owns ALONE: its
    /// pairing is `None`, handed over at the literal (see `written_tuple_members`) or a
    /// bufferless mint.  `layout.md (L-Tuple)` makes the member a field, so the record it
    /// displaces is released and its hook is not run (`heap.md (H-Drop-Not)`); the claimant of
    /// the value written is disarmed after the put, so the element stays the sole owner and the
    /// pairing is the same on every path.  Anything else keeps the plain put: an element with
    /// another claimant, or one this scan cannot pair, releases nothing here.
    ///
    /// The release is skipped when the value READS the tuple (`t.1 = f(t.1)`): released before
    /// the put, it would be read after its release, so a leak is the fallback and never a
    /// use-after-free.  A value whose mint cannot be proven retires the pairing, so a later write
    /// releases nothing the tuple does not own.
    pub(super) fn member_write(
        &mut self,
        v: u16,
        idx: u16,
        value: Value,
        function: &Function,
        data: &Data,
    ) -> Value {
        let put = |value: Value| Value::TuplePut(v, idx, Box::new(value));
        if !matches!(
            self.tuple_call_mint.get(&v).and_then(|m| m.get(&idx)),
            Some(None)
        ) {
            return put(value);
        }
        let Type::Tuple(elems) = function.tp(v).base() else {
            return put(value);
        };
        let elems = elems.clone();
        let release = tuple_owned_elem_frees(
            &elems,
            v,
            data,
            function,
            MemberFacts::NONE,
            Some(idx as usize),
        );
        if release.is_empty() {
            return put(value);
        }
        let claim = member_mint(&value, function, data);
        let mut ops = Vec::new();
        if !value.reads_var(v) {
            ops.extend(release);
        }
        ops.push(put(value));
        if let Some(&flag) = self.tuple_moved.get(&(v, idx)) {
            ops.push(v_set(flag, Value::Boolean(false)));
        }
        match claim {
            Some(MemberMint::Claimed(w)) => ops.push(v_set(
                w,
                Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
            )),
            Some(MemberMint::Adopted) => {}
            None => {
                if let Some(m) = self.tuple_call_mint.get_mut(&v) {
                    m.remove(&idx);
                }
            }
        }
        Value::Insert(ops)
    }
}
