// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-H-RefHome` — a record a stored `reference<T>` names lives as long as the store of its
//! holder.  The record's home is the variable that holds it: a local, or the compiler's
//! temporary for a literal or a call result.  A home lasts for the block that assigned it, so
//! a temporary assigned in a loop body lasts one pass.  Three routes reach a holder that lives
//! longer:
//!
//! - the WRITE itself, into a parameter, a variable of an enclosing block, or a record that
//!   lives in one (`h.r = items[1]` with `h` a parameter);
//! - a COPY of a holder into such a place (`out.hs += [h]`): the copy carries the reference;
//! - a CALL whose callee stores a reference to one argument's record into another argument
//!   (`fn set(h: H, p: Bag) { h.r = p.items[0] }`), checked at the call site, where the two
//!   arguments' lifetimes are known.  The callee's half is a summary per function, closed over
//!   the functions it calls.
//!
//! [`Walk::resolve`] keeps such a home where it can: a named local held from an enclosing block
//! has its store moved out to that block ([`Hoist`]; a refill of it in a loop removes the
//! records it held, so a reference still naming one reads null); a literal, a call result or a
//! refilled local's own record is built in the holder's store ([`Place`]); a record a
//! parameter's fixed place names is copied into the parameter's store before its home is freed
//! ([`Evacuation`]).  Every other case is refused where it is written.
//!
//! A record reached through another stored reference (`h.r = g.r`) has a home this frame
//! cannot see, and is not judged.

use super::reshape_refusals::ReshapeRefusal;
use crate::data::{Data, DefType, Deps, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// A callee stores a reference to a record of argument `home` into argument `holder`; `root`
/// when it names the argument's own record rather than one inside it.
#[derive(Clone, PartialEq, Eq)]
struct ParamPair {
    holder: u16,
    home: u16,
    field: String,
    root: bool,
}

/// A store that moves out to the block of a holder that outlives it: `owner` (the variable
/// owning the store) of function `d_nr` lives in block `block`, the walk's block numbering.
/// `insert` when it is assigned deeper: a null-init goes in front of statement `stmt` of that
/// block, so the scope pass scopes the store there.
pub struct Hoist {
    pub d_nr: u32,
    pub owner: u16,
    pub block: u32,
    pub stmt: usize,
    pub insert: bool,
}

/// A record built where its holder lives: the value of reference write number `site` of
/// function `d_nr` (the walk's order of `OpSetDbRef`s) fills `home`, the compiler's
/// temporary for a literal or the buffer a call result is delivered in.  The record is
/// claimed in the store of `host`, the holder's root, so it lasts as long as that store.
pub struct Place {
    pub d_nr: u32,
    pub site: usize,
    pub home: u16,
    pub host: u16,
    /// For a call result: the callee's buffer argument and the record type it builds.
    pub call: Option<(usize, u16)>,
    /// A named local's own record (`p = P {…}; h.r = p`): every mint of `home` in the
    /// function is claimed in `host`'s store, and `home` borrows it.
    pub own: bool,
    /// The holder's block path: holders of one own record must share it.
    lasts: Vec<u32>,
    /// The refusal this placement replaces, kept for a conflict between two of them.
    refusal: Option<ReshapeRefusal>,
}

/// A record copied out of its home before the home's store is freed: the reference at byte
/// `fld` of parameter `holder` of function `d_nr` may name a record of the store `owner`
/// owns, so every free of `owner` is preceded by `OpEvacuate(holder, fld, owner, tp)`.
pub struct Evacuation {
    pub d_nr: u32,
    pub holder: u16,
    pub fld: u16,
    pub home: u16,
    pub owners: Vec<u16>,
    pub tp: u16,
    /// The refusal this copy-out replaces, kept for a conflict between two of them.
    refusal: Option<ReshapeRefusal>,
}

/// What the analysis finds: the writes it refuses, and the stores it moves out, the records
/// it places and the records it copies out instead.
#[derive(Default)]
pub struct StoredRefs {
    pub refusals: Vec<ReshapeRefusal>,
    pub hoists: Vec<Hoist>,
    pub places: Vec<Place>,
    pub evacuations: Vec<Evacuation>,
    /// Every local whose store holds a record some stored reference names, however long
    /// the holder lasts: the hold's home and every variable its deps reach, per function.
    /// A rewrite that moves such a store's records elsewhere moves them out from under the
    /// reference (`@FR-H-RefHome`, loft#2118).
    pub named: Vec<(u32, u16)>,
}

/// How a hold whose home ends before its holder is kept: the home's store moved out to the
/// holder's block, the record built in the holder's store, or not at all.
enum Resolution {
    Hoist,
    Place(Place),
    Evacuate(Evacuation),
    Refuse,
}

/// The reference write a hold reaches its holder by: its number in walk order and the value
/// written.  A copy or a callee's link has none, and only a write can be lowered in place.
#[derive(Clone, Copy)]
struct Site<'v> {
    ordinal: usize,
    value: &'v Value,
}

/// Every stored-reference write, copy or call in the program whose holder outlives the
/// record's home: the home moved out to the holder's block, or the write refused.
#[must_use]
pub fn stored_reference_homes(data: &Data, database: &Stores) -> StoredRefs {
    if crate::keys::ref_home_disabled() || !database.has_reference_fields() {
        return StoredRefs::default();
    }
    let functions: Vec<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type, DefType::Function) && !matches!(def.code, Value::Null)
        })
        .collect();
    // The callee summaries, closed over calls: a function that hands its own parameters to a
    // callee which links them links them too.
    let mut pairs: HashMap<u32, Vec<ParamPair>> = HashMap::default();
    loop {
        let mut changed = false;
        for &d in &functions {
            let mut scratch = StoredRefs::default();
            let found = run(data, database, d, &pairs, &mut scratch);
            let entry = pairs.entry(d).or_default();
            for p in found {
                if !entry.contains(&p) {
                    entry.push(p);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut out = StoredRefs::default();
    for &d in &functions {
        run(data, database, d, &pairs, &mut out);
    }
    one_host_per_own_record(&mut out);
    one_holder_per_copied_home(&mut out);
    out
}

/// A home copied out to two different holders is refused: the two may live in different
/// stores of the caller, and then each would name a copy of its own, where before the copy
/// a write through one showed through the other.
fn one_holder_per_copied_home(out: &mut StoredRefs) {
    let mut holder_of: HashMap<(u32, u16), u16> = HashMap::default();
    let mut conflicted: Vec<(u32, u16)> = Vec::new();
    for e in &out.evacuations {
        let key = (e.d_nr, e.home);
        match holder_of.get(&key) {
            None => {
                holder_of.insert(key, e.holder);
            }
            Some(h) if *h != e.holder => conflicted.push(key),
            Some(_) => {}
        }
    }
    let mut kept = Vec::new();
    let mut reported: Vec<(u32, u16)> = Vec::new();
    for mut e in std::mem::take(&mut out.evacuations) {
        let key = (e.d_nr, e.home);
        if conflicted.contains(&key) {
            // One refusal per home: the holder that differs from the first.
            if holder_of[&key] != e.holder && !reported.contains(&key) {
                reported.push(key);
                out.refusals.extend(e.refusal.take());
            }
        } else {
            kept.push(e);
        }
    }
    out.evacuations = kept;
}

/// `@FR-H-RefHome`, after the scope pass: every free of a copied-out home's owner
/// (`OpFreeRef(owner)`, `OpFreeRefIfDistinct(owner, _)`) is preceded by the copy-out of each
/// reference that may name one of its records, `OpEvacuate(holder, fld, owner, tp)`.  At
/// run time the copy is made only when the reference still names a record of that store.
pub fn insert_evacuations(data: &mut Data, d_nr: u32) {
    let evacuations = data.def(d_nr).variables.ref_evacuations().to_vec();
    if evacuations.is_empty() {
        return;
    }
    let free_ref = data.def_nr("OpFreeRef");
    let free_distinct = data.def_nr("OpFreeRefIfDistinct");
    let evacuate = data.def_nr("OpEvacuate");
    fn visit(node: &mut Value, evs: &[[u16; 4]], frees: (u32, u32), evacuate: u32) {
        if let Some(operators) = statements(node) {
            let mut i = 0;
            while i < operators.len() {
                visit(&mut operators[i], evs, frees, evacuate);
                let owner = match operators[i].unspan() {
                    Value::Call(d, args) if *d == frees.0 || *d == frees.1 => {
                        match args.first().map(Value::unspan) {
                            Some(Value::Var(o)) => Some(*o),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let mut before = Vec::new();
                if let Some(owner) = owner {
                    for [holder, fld, o, tp] in evs {
                        if *o == owner {
                            before.push(Value::Call(
                                evacuate,
                                vec![
                                    Value::Var(*holder),
                                    Value::Int(i32::from(*fld)),
                                    Value::Var(owner),
                                    Value::Int(i32::from(*tp)),
                                ],
                            ));
                        }
                    }
                }
                let n = before.len();
                operators.splice(i..i, before);
                i += n + 1;
            }
            return;
        }
        node.for_each_child_mut(&mut |c| visit(c, evs, frees, evacuate));
    }
    visit(
        &mut data.definitions[d_nr as usize].code,
        &evacuations,
        (free_ref, free_distinct),
        evacuate,
    );
}

/// A named local's own record is claimed in ONE holder's store, whichever holders name it:
/// the first one's, when every holder lives in the same block, so none outlives the store
/// the record lives in.  Holders of different blocks keep their refusals.
fn one_host_per_own_record(out: &mut StoredRefs) {
    let mut first: HashMap<(u32, u16), (u16, Vec<u32>)> = HashMap::default();
    let mut conflicted: Vec<(u32, u16)> = Vec::new();
    let mut reported: Vec<(u32, u16)> = Vec::new();
    for p in out.places.iter().filter(|p| p.own) {
        let key = (p.d_nr, p.home);
        match first.get(&key) {
            None => {
                first.insert(key, (p.host, p.lasts.clone()));
            }
            Some((_, lasts)) if *lasts != p.lasts => conflicted.push(key),
            Some(_) => {}
        }
    }
    let mut kept = Vec::new();
    for mut p in std::mem::take(&mut out.places) {
        if p.own {
            let key = (p.d_nr, p.home);
            if conflicted.contains(&key) {
                // One refusal per record: the holder whose block differs from the first's.
                if p.lasts != first[&key].1 && !reported.contains(&key) {
                    reported.push(key);
                    out.refusals.extend(p.refusal.take());
                }
                continue;
            }
            p.host = first[&key].0;
        }
        kept.push(p);
    }
    out.places = kept;
}

/// Put each hoist's null-init in front of its statement, numbering blocks as [`Walk`] does.
pub fn insert_hoists(code: &mut Value, hoists: &[&Hoist]) {
    fn visit(node: &mut Value, next: &mut u32, hoists: &[&Hoist]) {
        match node.unspan_mut() {
            Value::Block(b) | Value::Loop(b) => {
                let id = *next;
                *next += 1;
                for op in &mut b.operators {
                    visit(op, next, hoists);
                }
                let mut here: Vec<&&Hoist> = hoists.iter().filter(|h| h.block == id).collect();
                here.sort_by_key(|h| std::cmp::Reverse(h.stmt));
                for h in here {
                    b.operators
                        .insert(h.stmt, Value::Set(h.owner, Box::new(Value::Null)));
                }
            }
            other => other.for_each_child_mut(&mut |c| visit(c, next, hoists)),
        }
    }
    visit(code, &mut 0, hoists);
}

/// The operators [`apply_places`] reads and writes, by definition number.
pub struct PlaceOps {
    pub set_db_ref: u32,
    pub database: u32,
    pub place_record: u32,
}

/// Build each placed record in its holder's store.  `places` pairs a [`Place`] with the fresh
/// variable that takes its record: in the value of reference write `site` (numbered as
/// [`Walk`] numbers them, in post-order), the mint `OpDatabase(home, tp)` becomes
/// `fresh = OpPlaceRecord(host, tp)`, and every read of `home` there reads `fresh`.  A fresh
/// variable, because the temporary may serve another literal of the function that is not
/// placed; it is typed as a borrow of the host, so no free is ever emitted for it — a free
/// releases a whole store, and this record lives in the holder's.
pub fn apply_places(
    function: &mut Function,
    code: &mut Value,
    places: &[(&Place, u16, Type)],
    ops: &PlaceOps,
) {
    fn visit(
        node: &mut Value,
        next: &mut usize,
        places: &[(&Place, u16, Type)],
        ops: &PlaceOps,
        before: &mut Vec<Value>,
    ) {
        // A call's buffer is claimed by a statement of its own, in front of the statement
        // holding the write: every statement list takes the claims its statements raise.
        if let Some(operators) = statements(node) {
            let mut i = 0;
            while i < operators.len() {
                let mut here = Vec::new();
                visit(&mut operators[i], next, places, ops, &mut here);
                let n = here.len();
                operators.splice(i..i, here);
                i += n + 1;
            }
            return;
        }
        node.for_each_child_mut(&mut |c| visit(c, next, places, ops, before));
        let Value::Call(d, args) = node else {
            return;
        };
        if *d != ops.set_db_ref || args.len() != 3 {
            return;
        }
        let site = *next;
        *next += 1;
        let Some((place, fresh, _)) = places.iter().find(|(p, _, _)| p.site == site) else {
            return;
        };
        if place.own {
            return;
        }
        match place.call {
            None => place_in(&mut args[2], place, *fresh, ops),
            Some((idx, record)) => {
                if let Value::Call(_, cargs) = args[2].unspan_mut() {
                    cargs[idx] = Value::Var(*fresh);
                    let claim = Value::Call(
                        ops.place_record,
                        vec![Value::Var(place.host), Value::Int(i32::from(record))],
                    );
                    before.push(Value::Set(*fresh, Box::new(claim)));
                }
            }
        }
    }
    for (place, fresh, _) in places {
        if place.call.is_some() {
            function.mark_place_buffer(*fresh);
        }
    }
    visit(code, &mut 0, places, ops, &mut Vec::new());
    for (place, _, _) in places.iter().filter(|(p, _, _)| p.own) {
        claim_every_mint(code, place, ops);
    }
}

/// A named local's own record: every `OpDatabase(home, tp)` of the function becomes
/// `home = OpPlaceRecord(host, tp)`.
fn claim_every_mint(node: &mut Value, place: &Place, ops: &PlaceOps) {
    if let Value::Call(d, args) = node
        && *d == ops.database
        && args.len() == 2
        && matches!(args[0].unspan(), Value::Var(w) if *w == place.home)
    {
        let claim = Value::Call(
            ops.place_record,
            vec![Value::Var(place.host), args[1].clone()],
        );
        *node = Value::Set(place.home, Box::new(claim));
        return;
    }
    node.for_each_child_mut(&mut |c| claim_every_mint(c, place, ops));
}

/// `@FR-H-RefHome`, after the scope pass: a variable bound to a call that was handed a place
/// buffer (`x = mk(…, __place_N)`, the scope pass's lift of the call) holds the record claimed
/// in the holder's store, and the frees the pass emitted for it — `OpFreeRef(x)`,
/// `OpFreeRefIfDistinct(x, _)` — would release that whole store.  They go.  Nothing else
/// names such a variable: the write the call feeds is its one use.
pub fn keep_placed_results(data: &mut Data, d_nr: u32) {
    let def = data.def(d_nr);
    let function = &def.variables;
    if !(0..function.count()).any(|v| function.is_place_buffer(v)) {
        return;
    }
    let mut bound: Vec<u16> = Vec::new();
    def.code.walk(&mut |n| {
        if let Value::Set(x, rhs) = n
            && let Value::Call(_, args) = rhs.unspan()
            && args
                .iter()
                .any(|a| matches!(a.unspan(), Value::Var(p) if function.is_place_buffer(*p)))
        {
            bound.push(*x);
        }
    });
    if bound.is_empty() {
        return;
    }
    let free_ref = data.def_nr("OpFreeRef");
    let free_distinct = data.def_nr("OpFreeRefIfDistinct");
    let is_free = |op: &Value| {
        matches!(op.unspan(), Value::Call(d, args)
            if (*d == free_ref || *d == free_distinct)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if bound.contains(x)))
    };
    fn strip(node: &mut Value, is_free: &dyn Fn(&Value) -> bool) {
        if let Some(operators) = statements(node) {
            operators.retain(|op| !is_free(op));
        }
        node.for_each_child_mut(&mut |c| strip(c, is_free));
    }
    strip(&mut data.definitions[d_nr as usize].code, &is_free);
}

/// The statement list of a block, a loop body or a spliced list.
fn statements(node: &mut Value) -> Option<&mut Vec<Value>> {
    match node.unspan_mut() {
        Value::Block(b) | Value::Loop(b) => Some(&mut b.operators),
        Value::Insert(ops) => Some(ops),
        _ => None,
    }
}

/// `tp` as a borrow of `host`'s store: a record type's deps name the host, its nullability
/// kept.  Anything else is no record a placement claims, and stays as it is.
#[must_use]
pub fn borrowing(tp: &Type, host: u16) -> Type {
    match tp {
        Type::Optional(inner) => Type::Optional(Box::new(borrowing(inner, host))),
        Type::Reference(k, _) => Type::Reference(*k, Deps::frame1(host)),
        other => other.clone(),
    }
}

fn place_in(value: &mut Value, place: &Place, fresh: u16, ops: &PlaceOps) {
    let Value::Block(b) = value.unspan_mut() else {
        return;
    };
    b.result = borrowing(&b.result, place.host);
    let Some(at) = mint_at(b, place.home, |d| d == ops.database) else {
        return;
    };
    for op in &mut b.operators {
        rename(op, place.home, fresh);
    }
    if let Value::Call(_, args) = b.operators[at].unspan() {
        let mint = Value::Call(
            ops.place_record,
            vec![Value::Var(place.host), args[1].clone()],
        );
        b.operators[at] = Value::Set(fresh, Box::new(mint));
    }
}

/// The statement of a literal's block that mints `home` (`OpDatabase(home, tp)`), when only
/// line marks and `home`'s own null-init come before it.
fn mint_at(b: &crate::data::Block, home: u16, is_database: impl Fn(u32) -> bool) -> Option<usize> {
    for (i, op) in b.operators.iter().enumerate() {
        match op.unspan() {
            Value::Line(_) => {}
            Value::Set(v, n) if *v == home && matches!(n.unspan(), Value::Null) => {}
            Value::Call(d, args)
                if is_database(*d)
                    && args.len() == 2
                    && matches!(args[0].unspan(), Value::Var(w) if *w == home) =>
            {
                return Some(i);
            }
            _ => return None,
        }
    }
    None
}

/// Every mention of variable `from` in `node` becomes `to`.  A literal's block names its
/// temporary only as a value or an assignment target.
fn rename(node: &mut Value, from: u16, to: u16) {
    match node.unspan_mut() {
        Value::Var(v) | Value::Set(v, _) if *v == from => *v = to,
        _ => {}
    }
    node.for_each_child_mut(&mut |c| rename(c, from, to));
}

fn run(
    data: &Data,
    database: &Stores,
    d_nr: u32,
    pairs: &HashMap<u32, Vec<ParamPair>>,
    out: &mut StoredRefs,
) -> Vec<ParamPair> {
    let def = data.def(d_nr);
    let mut walk = Walk {
        data,
        database,
        d_nr,
        function: &def.variables,
        file: def.position.file.to_string(),
        pairs,
        path: Vec::new(),
        stmt_at: Vec::new(),
        next_block: 0,
        loops: Vec::new(),
        first_set: HashMap::default(),
        last_set: HashMap::default(),
        holds: HashMap::default(),
        found: Vec::new(),
        writes: 0,
        out,
    };
    walk.node(&def.code, def.position.line);
    walk.found
}

/// A record a holder's reference names: its home's variable and how long that lasts, or the
/// parameter whose record it is (`param`), whose home is the caller's.  `root` when the
/// record is the home's own (`h.r = p`) rather than one inside it (`h.r = items[0]`).
#[derive(Clone)]
struct Hold {
    home: u16,
    home_path: Vec<u32>,
    param: Option<u16>,
    field: String,
    root: bool,
    /// Where the reference sits when that is a fixed place: a variable and the byte offset in
    /// its record.  Kept through a copy of the whole variable into another fixed place.
    at: Option<(u16, u16)>,
    /// The record type the reference names, `u16::MAX` when not known.
    target: u16,
}

/// How long a place lasts: past this frame (a parameter's), or for a block path.
enum Life {
    Caller(u16),
    Block(Vec<u32>),
}

struct Walk<'a> {
    data: &'a Data,
    database: &'a Stores,
    d_nr: u32,
    function: &'a Function,
    file: String,
    pairs: &'a HashMap<u32, Vec<ParamPair>>,
    /// The blocks enclosing the current node, outermost first.
    path: Vec<u32>,
    /// Per block of `path`, the statement of it the current node is in.
    stmt_at: Vec<usize>,
    next_block: u32,
    /// The blocks that are loop bodies: a value assigned in one lasts a pass.
    loops: Vec<u32>,
    /// The block path of each variable's first assignment: where it is declared.
    first_set: HashMap<u16, Vec<u32>>,
    /// The block path of each variable's latest assignment: where its current value began.
    last_set: HashMap<u16, Vec<u32>>,
    /// The references each local holder carries, for the copies made of it.
    holds: HashMap<u16, Vec<Hold>>,
    /// This function's own summary: references it stores between its parameters.
    found: Vec<ParamPair>,
    /// The reference writes (`OpSetDbRef`) met so far: a [`Place`] names its write by this.
    writes: usize,
    out: &'a mut StoredRefs,
}

impl Walk<'_> {
    fn node(&mut self, code: &Value, line: u32) {
        match code.unspan() {
            Value::Block(b) | Value::Loop(b) => {
                if matches!(code.unspan(), Value::Loop(_)) {
                    self.loops.push(self.next_block);
                }
                self.path.push(self.next_block);
                self.stmt_at.push(0);
                self.next_block += 1;
                let mut cur = line;
                for (i, s) in b.operators.iter().enumerate() {
                    if let Some(at) = self.stmt_at.last_mut() {
                        *at = i;
                    }
                    if let Value::Line(n) = s.unspan() {
                        cur = *n;
                    }
                    self.node(s, cur);
                }
                self.stmt_at.pop();
                self.path.pop();
            }
            Value::Insert(ops) => self.stmts(ops, line),
            Value::Set(v, rhs) => {
                self.node(rhs, line);
                self.assigned(*v);
                // `h = t` between two records copies `t` into `h`, references and all.
                if let Value::Var(w) = rhs.unspan()
                    && matches!(self.function.tp(*v), Type::Reference(_, d) if d.is_empty())
                    && matches!(self.function.tp(*w), Type::Reference(_, _))
                {
                    self.copy(rhs, &Value::Var(*v), line);
                }
            }
            Value::Call(d, args) => {
                for a in args {
                    self.node(a, line);
                }
                let name = self.data.def(*d).name();
                match name {
                    // A record minted into a temporary: the temporary's value begins here.
                    "OpDatabase" => {
                        if let Some(Value::Var(w)) = args.first().map(Value::unspan) {
                            self.assigned(*w);
                        }
                    }
                    "OpSetDbRef" if args.len() == 3 => {
                        self.writes += 1;
                        let site = Site {
                            ordinal: self.writes - 1,
                            value: &args[2],
                        };
                        self.write(&args[0], &args[1], site, line);
                    }
                    "OpCopyRecord" if args.len() == 3 => self.copy(&args[0], &args[1], line),
                    _ if !name.starts_with("Op") => {
                        self.call(*d, args, line);
                        // The callee fills the return buffer it is handed: the buffer's value
                        // begins at the call, once per pass in a loop.
                        for a in args {
                            if let Value::Var(w) = a.unspan()
                                && self.function.name(*w).starts_with("__ref")
                            {
                                self.assigned(*w);
                            }
                        }
                    }
                    _ => {}
                }
            }
            other => other.for_each_child(&mut |c| self.node(c, line)),
        }
    }

    fn stmts(&mut self, stmts: &[Value], line: u32) {
        let mut cur = line;
        for s in stmts {
            if let Value::Line(n) = s.unspan() {
                cur = *n;
            }
            self.node(s, cur);
        }
    }

    fn assigned(&mut self, v: u16) {
        self.first_set.entry(v).or_insert_with(|| self.path.clone());
        self.last_set.insert(v, self.path.clone());
        // A new value: the references the old one carried are gone with it.
        self.holds.remove(&v);
    }

    /// The variable a place is rooted at: `h`, `h.f`, `v[i]`, `v[i].f`.
    fn root(&self, place: &Value) -> Option<u16> {
        match place.unspan() {
            Value::Var(v) => Some(*v),
            Value::Call(d, args) => match self.data.def(*d).name() {
                "OpGetField"
                | "OpVectorRef"
                | "OpGetVector"
                | "OpGetVectorNullable"
                | "OpNewRecord"
                | "OpGetRecord" => args.first().and_then(|a| self.root(a)),
                _ => None,
            },
            Value::Block(b) => b.operators.last().and_then(|t| self.root(t)),
            Value::Insert(ops) => ops.last().and_then(|t| self.root(t)),
            _ => None,
        }
    }

    /// A place that names the same bytes wherever it is evaluated: a variable, or a constant
    /// field of one (`h`, `o.h`).  Answers the variable and the byte offset.  An element is
    /// no fixed place: its index, or the collection, may change before the place is read.
    fn fixed_place(&self, place: &Value) -> Option<(u16, u16)> {
        match place.unspan() {
            Value::Var(v) => Some((*v, 0)),
            Value::Call(d, args) if self.data.def(*d).name() == "OpGetField" => {
                let (v, off) = self.fixed_place(args.first()?)?;
                // A field holding a collection names the collection, not bytes of the record.
                let inline = self.record_type(place).is_some_and(|t| {
                    matches!(
                        self.database.types.get(t as usize).map(|t| &t.parts),
                        Some(Parts::Struct(_) | Parts::EnumValue(..))
                    )
                });
                if !inline {
                    return None;
                }
                let Value::Int(pos) = args.get(1)?.unspan() else {
                    return None;
                };
                Some((
                    v,
                    u16::try_from(u32::from(off) + u32::try_from(*pos).ok()?).ok()?,
                ))
            }
            _ => None,
        }
    }

    /// The variable that holds the record a stored reference is set to: the place's root, a
    /// literal's temporary, or the buffer a call result is delivered in.
    fn home(&self, value: &Value) -> Option<u16> {
        match value.unspan() {
            Value::Call(d, args) if !self.data.def(*d).name().starts_with("Op") => {
                args.iter().rev().find_map(|a| match a.unspan() {
                    Value::Var(w) if self.function.name(*w).starts_with("__ref") => Some(*w),
                    _ => None,
                })
            }
            other => self.root(other),
        }
    }

    /// `v` and every variable its deps reach, or the parameter among them: then the place
    /// belongs to the caller.
    fn chain(&self, v: u16) -> Result<Vec<u16>, u16> {
        let mut seen = vec![v];
        let mut i = 0;
        while i < seen.len() {
            let cur = seen[i];
            if self.function.is_argument(cur) {
                return Err(cur);
            }
            for d in self.function.tp(cur).depend() {
                if d < self.function.count() && !seen.contains(&d) {
                    seen.push(d);
                }
            }
            i += 1;
        }
        Ok(seen)
    }

    /// How long the holder rooted at `h` lasts: as long as the outermost variable it is part
    /// of, or past this frame when that is a parameter.
    fn holder_life(&self, h: u16) -> Option<Life> {
        match self.chain(h) {
            Err(p) => Some(Life::Caller(p)),
            Ok(vars) => vars
                .iter()
                .filter_map(|v| self.first_set.get(v))
                .min_by_key(|p| p.len())
                .cloned()
                .map(Life::Block),
        }
    }

    /// How long the value now in `home` lasts.  A named local's records last as long as the
    /// local: a reassignment of one a reference names into empties it in place (`@FR-H-Removed`,
    /// `Parser::referenced_locals`), so the reference reads null rather than a freed store.  A
    /// compiler temporary — a literal's, a call's return buffer — is refilled by its next
    /// assignment with nothing emptied, so its value lasts one pass of the innermost loop
    /// between its declaring block and the assignment.  An expression's own block (a
    /// literal's) ends nothing: the temporary it fills is declared outside it.
    fn home_life(&self, home: u16) -> Option<Vec<u32>> {
        let declared = self.first_set.get(&home)?;
        if !self.hidden(home) {
            return Some(declared.clone());
        }
        let assigned = self.last_set.get(&home)?;
        let inner_loop = (declared.len()..assigned.len())
            .rev()
            .find(|&i| self.loops.contains(&assigned[i]));
        Some(match inner_loop {
            Some(i) => assigned[..=i].to_vec(),
            None => declared.clone(),
        })
    }

    fn outlives(lasts: &Life, home_path: &[u32]) -> bool {
        match lasts {
            Life::Caller(_) => true,
            Life::Block(p) => p.len() < home_path.len() && home_path.starts_with(p),
        }
    }

    /// Is the field at byte `pos` of the record `holder` names a user's `reference<T>`?
    /// A closure's captures are stored the same way but name no target type.
    fn reference_field(&self, holder: &Value, pos: &Value) -> Option<String> {
        let Value::Int(pos) = pos.unspan() else {
            return None;
        };
        let kt = self.record_type(holder)?;
        // A record the compiler builds — a closure's (`__closure_…`), a generator's frame —
        // stores its captures the same way; those are not the author's references.
        if self.database.types.get(kt as usize)?.name.starts_with("__") {
            return None;
        }
        self.database
            .reference_field_name(kt, u16::try_from(*pos).ok()?)
    }

    /// The known record type of the record a place names: a field read carries its type, an
    /// element is its collection's content, and a variable has its declared type.
    fn record_type(&self, place: &Value) -> Option<u16> {
        match place.unspan() {
            Value::Call(d, args) => match self.data.def(*d).name() {
                "OpGetField" => match args.get(2)?.unspan() {
                    Value::Int(kt) => u16::try_from(*kt).ok(),
                    _ => None,
                },
                "OpGetVector" | "OpVectorRef" | "OpGetVectorNullable" => {
                    match args.first()?.unspan() {
                        // A vector local: its element type is declared on it.
                        Value::Var(v) => match self.function.tp(*v).base() {
                            Type::Vector(elm, _) => match elm.base() {
                                Type::Reference(d, _) => Some(self.data.def(*d).known_type()),
                                _ => None,
                            },
                            _ => None,
                        },
                        vector => self.database.collection_content(self.record_type(vector)?),
                    }
                }
                // A keyed lookup (`m[k]`) names its collection's type.
                "OpGetRecord" => match args.get(1)?.unspan() {
                    Value::Int(kt) => self.database.collection_content(u16::try_from(*kt).ok()?),
                    _ => None,
                },
                _ => None,
            },
            Value::Var(v) => match self.function.tp(*v).base() {
                Type::Reference(d, _) => Some(self.data.def(*d).known_type()),
                _ => None,
            },
            Value::Block(b) => b.operators.last().and_then(|t| self.record_type(t)),
            _ => None,
        }
    }

    fn write(&mut self, holder: &Value, pos: &Value, site: Site, line: u32) {
        let value = site.value;
        let Some(field) = self.reference_field(holder, pos) else {
            return;
        };
        let Some(h) = self.root(holder) else {
            return;
        };
        let Some(lasts) = self.holder_life(h) else {
            return;
        };
        // `h.r = g.r` — the record is the one `g`'s reference names, so `h` takes `g`'s holds.
        if let Some(src) = self.reference_read(value) {
            let holds = self.holds_of(src);
            self.carry(
                h,
                &lasts,
                holds,
                "",
                matches!(holder.unspan(), Value::Var(_)),
                None,
                line,
            );
            return;
        }
        let Some(home) = self.home(value) else {
            return;
        };
        let Some(mut hold) = self.hold_for(home, field) else {
            return;
        };
        hold.root = matches!(value.unspan(), Value::Var(_));
        if let (Some((v, off)), Value::Int(pos)) = (self.fixed_place(holder), pos.unspan()) {
            hold.at = u16::try_from(i64::from(off) + i64::from(*pos))
                .ok()
                .map(|o| (v, o));
        }
        if let (Some(kt), Value::Int(pos)) = (self.record_type(holder), pos.unspan()) {
            hold.target = u16::try_from(*pos)
                .ok()
                .and_then(|p| self.database.reference_field_target(kt, p))
                .unwrap_or(u16::MAX);
        }
        let direct = matches!(holder.unspan(), Value::Var(_));
        self.carry(h, &lasts, vec![hold], "", direct, Some(site), line);
    }

    /// What a holder holds when its reference names a record of `home`: the parameter whose
    /// record it is, or the local and how long its value lasts.
    fn hold_for(&self, home: u16, field: String) -> Option<Hold> {
        if let Err(p) = self.chain(home) {
            return Some(Hold {
                home,
                home_path: Vec::new(),
                param: Some(p),
                field,
                root: false,
                at: None,
                target: u16::MAX,
            });
        }
        Some(Hold {
            home,
            home_path: self.home_life(home)?,
            param: None,
            field,
            root: false,
            at: None,
            target: u16::MAX,
        })
    }

    /// The holder a value reads a stored reference out of (`g.r`), when it is one.
    fn reference_read(&self, value: &Value) -> Option<u16> {
        match value.unspan() {
            Value::Call(d, args) if self.data.def(*d).name() == "OpGetDbRef" => {
                self.root(args.first()?)
            }
            _ => None,
        }
    }

    /// The references `v` carries: a parameter's are its caller's, every other the walk has
    /// recorded.
    fn holds_of(&self, v: u16) -> Vec<Hold> {
        match self.chain(v) {
            Err(p) => vec![Hold {
                home: p,
                home_path: Vec::new(),
                param: Some(p),
                field: String::new(),
                root: false,
                at: None,
                target: u16::MAX,
            }],
            Ok(_) => self.holds.get(&v).cloned().unwrap_or_default(),
        }
    }

    fn remember(&mut self, h: u16, holds: &[Hold]) {
        if let Ok(vars) = self.chain(h) {
            for v in vars {
                self.holds
                    .entry(v)
                    .or_default()
                    .extend(holds.iter().cloned());
            }
        }
    }

    /// `holds` reach the holder rooted at `h`: keep one whose home ends first where
    /// [`Self::resolve`] can, refuse it otherwise, hand a link between two parameters to the
    /// caller, and remember the rest for the copies made of it.
    #[expect(
        clippy::too_many_arguments,
        reason = "one judgement over one hold's context"
    )]
    fn carry(
        &mut self,
        h: u16,
        lasts: &Life,
        holds: Vec<Hold>,
        via: &str,
        direct: bool,
        site: Option<Site>,
        line: u32,
    ) {
        let mut kept = Vec::new();
        for hold in &holds {
            if hold.param.is_none()
                && let Ok(vars) = self.chain(hold.home)
            {
                for v in vars {
                    if !self.out.named.contains(&(self.d_nr, v)) {
                        self.out.named.push((self.d_nr, v));
                    }
                }
            }
        }
        for hold in holds {
            match (hold.param, lasts) {
                (Some(home_param), Life::Caller(holder_param)) => {
                    if *holder_param != home_param {
                        self.found.push(ParamPair {
                            holder: *holder_param,
                            home: home_param,
                            field: hold.field.clone(),
                            root: hold.root,
                        });
                    }
                }
                (Some(_), Life::Block(_)) => kept.push(hold),
                (None, _) => {
                    if Self::outlives(lasts, &hold.home_path) {
                        match self.resolve(h, lasts, &hold, site) {
                            Resolution::Refuse => {
                                let msg = self.message(h, &hold, lasts, via, direct);
                                self.refuse(line, msg);
                                return;
                            }
                            Resolution::Hoist => {}
                            Resolution::Place(mut place) => {
                                let message = self.message(h, &hold, lasts, via, direct);
                                place.refusal = Some(ReshapeRefusal {
                                    file: self.file.clone(),
                                    line,
                                    message,
                                });
                                self.out.places.push(place);
                            }
                            Resolution::Evacuate(mut ev) => {
                                let message = self.message(h, &hold, lasts, via, direct);
                                ev.refusal = Some(ReshapeRefusal {
                                    file: self.file.clone(),
                                    line,
                                    message,
                                });
                                self.out.evacuations.push(ev);
                            }
                        }
                        // The record now lasts as long as the holder.
                        let home_path = match lasts {
                            Life::Block(p) => p.clone(),
                            Life::Caller(_) => Vec::new(),
                        };
                        kept.push(Hold { home_path, ..hold });
                        continue;
                    }
                    kept.push(hold);
                }
            }
        }
        self.remember(h, &kept);
    }

    /// A copy of a holder carries its references to wherever the copy lands.
    fn copy(&mut self, src: &Value, dst: &Value, line: u32) {
        let (Some(s), Some(d)) = (self.root(src), self.root(dst)) else {
            return;
        };
        let mut holds = self.holds_of(s);
        // A copy of the whole variable keeps each reference at the same offset of the copy.
        let whole = matches!(src.unspan(), Value::Var(_));
        let dst_at = self.fixed_place(dst);
        for hold in &mut holds {
            hold.at = match (whole, hold.at, dst_at) {
                (true, Some((v, a)), Some((dv, doff))) if v == s => {
                    u16::try_from(u32::from(a) + u32::from(doff))
                        .ok()
                        .map(|o| (dv, o))
                }
                _ => None,
            };
        }
        if holds.is_empty() || self.chain(s).is_err() {
            // A parameter's own references are the caller's to judge: a copy of the whole
            // parameter is the caller's value moving, not a new link.
            return;
        }
        let Some(lasts) = self.holder_life(d) else {
            return;
        };
        let via = format!(" (copied from `{}`)", self.function.name(s));
        self.carry(
            d,
            &lasts,
            holds,
            &via,
            matches!(dst.unspan(), Value::Var(_)),
            None,
            line,
        );
    }

    /// A callee that stores a reference between two of its arguments.
    fn call(&mut self, callee: u32, args: &[Value], line: u32) {
        let Some(links) = self.pairs.get(&callee).cloned() else {
            return;
        };
        for link in links {
            let (Some(ha), Some(ma)) =
                (args.get(link.holder as usize), args.get(link.home as usize))
            else {
                continue;
            };
            let (Some(h), Some(home)) = (self.root(ha), self.home(ma)) else {
                continue;
            };
            let Some(lasts) = self.holder_life(h) else {
                continue;
            };
            let Some(mut hold) = self.hold_for(home, link.field.clone()) else {
                continue;
            };
            hold.root = link.root;
            let via = format!(" (`{}` stores it there)", self.callee_name(callee));
            let before = self.out.refusals.len();
            self.carry(
                h,
                &lasts,
                vec![hold],
                &via,
                matches!(ha.unspan(), Value::Var(_)),
                None,
                line,
            );
            if self.out.refusals.len() > before {
                return;
            }
        }
    }

    fn callee_name(&self, d: u32) -> String {
        let n = self.data.def(d).name();
        n.strip_prefix("n_").unwrap_or(n).to_string()
    }

    /// How a hold that outlives its home is kept, if it can be: a named local's store moves
    /// out to an enclosing holder's block ([`Self::hoist`]); a literal written straight into
    /// the holder is built in the holder's store ([`Self::place`]).  Every other case is
    /// refused — a refusal is never a wrong answer.
    fn resolve(&mut self, h: u16, lasts: &Life, hold: &Hold, site: Option<Site>) -> Resolution {
        if let Life::Block(p) = lasts
            && self.hoist(p, hold)
        {
            return Resolution::Hoist;
        }
        if let Some(site) = site
            && let Some(mut place) = self
                .place(h, hold, site)
                .or_else(|| self.place_own(h, lasts, hold, site))
        {
            if let Life::Block(p) = lasts {
                place.lasts.clone_from(p);
            }
            return Resolution::Place(place);
        }
        if let Some(ev) = self.evacuate(hold) {
            return Resolution::Evacuate(ev);
        }
        Resolution::Refuse
    }

    /// A named local's record held by a parameter's fixed place (`h.r = items[1]`, `fn
    /// f(h: H)`; the return buffer is one) is copied into the parameter's store before the
    /// local's store is freed, and the reference re-pointed at the copy.  Until then the
    /// holder names the local's record itself, so every write through either shows through the
    /// other.  Refused when the record holds a stored reference (it could name the same
    /// store) or anything the copy cannot follow.
    fn evacuate(&self, hold: &Hold) -> Option<Evacuation> {
        let (holder, fld) = hold.at?;
        if self.hidden(hold.home)
            || !self.function.is_argument(holder)
            || hold.target == u16::MAX
            || !self
                .database
                .tree_holds_no_stored_refs(hold.target, &mut Vec::new())
        {
            return None;
        }
        // @FR-O-Proxy asks copy — the stores whose existing frees a copy-out goes in front of;
        // it authorises no free, and a borrow read as an owner has no free to stand before.
        let owners: Vec<u16> = self
            .chain(hold.home)
            .ok()?
            .into_iter()
            .filter(|&v| self.function.tp(v).depend().is_empty())
            .collect();
        if owners.is_empty() {
            return None;
        }
        Some(Evacuation {
            d_nr: self.d_nr,
            holder,
            fld,
            home: hold.home,
            owners,
            tp: hold.target,
            refusal: None,
        })
    }

    /// A named local's own record, refilled by a loop between it and its holder
    /// (`for … { p = P {…}; h.r = p }`), is built in the holder's store: each pass's record
    /// stays there for the holder to name, where a hoisted store would claim the same
    /// position again.  Only when every assignment of the local mints a literal: then `p`
    /// can borrow the holder's store without anything else writing through it.
    fn place_own(&self, h: u16, lasts: &Life, hold: &Hold, site: Site) -> Option<Place> {
        let Life::Block(p) = lasts else {
            return None;
        };
        let refilled = hold
            .home_path
            .iter()
            .any(|b| self.loops.contains(b) && !p.contains(b));
        if !hold.root
            || !refilled
            || self.hidden(hold.home)
            || self.function.is_argument(hold.home)
            || !self.owned_record(hold.home)
            || !self.only_literal_mints(hold.home)
        {
            return None;
        }
        Some(Place {
            d_nr: self.d_nr,
            site: site.ordinal,
            home: hold.home,
            host: h,
            call: None,
            own: true,
            lasts: Vec::new(),
            refusal: None,
        })
    }

    /// Does `v` own a record of its own?  A nullable local is no candidate: `v = null` between
    /// two mints would leave the holder's store a record nothing names, and is not followed.
    fn owned_record(&self, v: u16) -> bool {
        match self.function.tp(v) {
            Type::Optional(_) => false,
            Type::Reference(_, d) => d.is_empty(),
            _ => false,
        }
    }

    /// Is every assignment of `v` in this function a literal's mint (`OpDatabase(v, tp)`),
    /// with nothing else written into it as a whole?  A rebind or a record copied into it
    /// would write through the holder's store once `v` borrows it, so either answers no.
    fn only_literal_mints(&self, v: u16) -> bool {
        let code = &self.data.def(self.d_nr).code;
        let on_v = |args: &[Value], i: usize| matches!(args.get(i).map(Value::unspan), Some(Value::Var(w)) if *w == v);
        let mut mints = 0usize;
        let mut other = false;
        code.walk(&mut |n| match n {
            Value::Call(d, args) => match self.data.def(*d).name() {
                "OpDatabase" if on_v(args, 0) => mints += 1,
                "OpCopyRecord" | "OpMoveRecord" if on_v(args, 1) => other = true,
                _ => {}
            },
            Value::Set(w, rhs) if *w == v && !matches!(rhs.unspan(), Value::Null) => other = true,
            _ => {}
        });
        mints > 0 && !other
    }

    /// A literal written straight into a holder (`h.r = P {…}`, `H { r: P {…} }`) is built in
    /// the holder's store: its value is the literal's own block, minting the compiler's
    /// temporary `hold.home` first.  The holder's root names that store; it is assigned by
    /// the time the write's value runs, since the write is into it.
    fn place(&self, h: u16, hold: &Hold, site: Site) -> Option<Place> {
        if !self.hidden(hold.home) {
            return None;
        }
        let call = if self.literal_mint(site.value, hold.home) {
            None
        } else {
            Some(self.offered_buffer(site.value, hold.home)?)
        };
        Some(Place {
            d_nr: self.d_nr,
            site: site.ordinal,
            home: hold.home,
            host: h,
            call,
            own: false,
            lasts: Vec::new(),
            refusal: None,
        })
    }

    /// A call result written straight into a holder (`h.r = mk(…)`) is built in the holder's
    /// store when its callee writes a record handed to it in place (`(R-Place)`'s callee
    /// clause with the offered-record test, `place_result::callee_keeps_an_offered_record`):
    /// the caller then hands it a record claimed there.  Answers the buffer argument and the
    /// record type.
    fn offered_buffer(&self, value: &Value, home: u16) -> Option<(usize, u16)> {
        let Value::Call(d, args) = value.unspan() else {
            return None;
        };
        let callee = self.data.def(*d);
        let idx = args
            .iter()
            .position(|a| matches!(a.unspan(), Value::Var(w) if *w == home))?;
        if callee.hidden_return_buffer_attr() != Some(idx)
            || !crate::place_result::callee_keeps_an_offered_record(self.data, *d, idx)
        {
            return None;
        }
        match &callee.returned {
            Type::Reference(td, _) => Some((idx, self.data.def(*td).known_type())),
            // A nullable result may answer null without writing the buffer it was offered.
            Type::Optional(_) => None,
            _ => None,
        }
    }

    /// Is `value` a literal's block that mints `home` before anything else reads it?
    fn literal_mint(&self, value: &Value, home: u16) -> bool {
        let Value::Block(b) = value.unspan() else {
            return false;
        };
        mint_at(b, home, |d| self.data.def(d).name() == "OpDatabase").is_some()
    }

    /// Move the store of `hold`'s home out to the holder's block `p`, when that keeps every
    /// record a reference may name: the home is a named local inside `p`, and a refill of it
    /// in a loop between the two removes records the reference may still name, which reads
    /// null — except the home's own record, whose position the refill claims again.
    fn hoist(&mut self, p: &[u32], hold: &Hold) -> bool {
        if self.hidden(hold.home) || p.is_empty() || !self.path.starts_with(p) {
            return false;
        }
        let refilled = hold
            .home_path
            .iter()
            .any(|b| self.loops.contains(b) && !p.contains(b));
        if refilled && hold.root {
            return false;
        }
        let Ok(chain) = self.chain(hold.home) else {
            return false;
        };
        // @FR-O-Proxy asks alloc — the stores whose null-init moves out to the holder's block;
        // where a store is allocated, not whether it is released.
        let owners: Vec<u16> = chain
            .into_iter()
            .filter(|&v| self.function.tp(v).depend().is_empty())
            .collect();
        if owners.is_empty() {
            return false;
        }
        let stmt = self.stmt_at[p.len() - 1];
        for owner in owners {
            let insert = self
                .first_set
                .get(&owner)
                .is_some_and(|f| f.len() > p.len());
            self.out.hoists.push(Hoist {
                d_nr: self.d_nr,
                owner,
                block: p[p.len() - 1],
                stmt,
                insert,
            });
        }
        true
    }

    fn refuse(&mut self, line: u32, message: String) {
        self.out.refusals.push(ReshapeRefusal {
            file: self.file.clone(),
            line,
            message,
        });
    }

    fn message(&self, h: u16, hold: &Hold, lasts: &Life, via: &str, direct: bool) -> String {
        let (field, home, home_path) = (&hold.field, hold.home, &hold.home_path);
        // Name the holder by the outermost variable it lives in: `b` for an element built
        // into `b.hs`, rather than the compiler's temporary for the element.
        let named = match self.chain(h) {
            Err(p) => p,
            Ok(vars) => vars
                .into_iter()
                .filter(|v| !self.hidden(*v))
                .min_by_key(|v| self.first_set.get(v).map_or(usize::MAX, Vec::len))
                .unwrap_or(h),
        };
        let holder = if self.hidden(named) {
            format!("the `{field}` field")
        } else if named == h && direct {
            format!("`{}.{field}`", self.function.name(h))
        } else {
            format!("the `{field}` field inside `{}`", self.function.name(named))
        };
        let home_name = if self.hidden(home) {
            "a temporary".to_string()
        } else {
            format!("`{}`", self.function.name(home))
        };
        let ends = match lasts {
            Life::Caller(_) => "when this function returns",
            Life::Block(p)
                if home_path
                    .iter()
                    .any(|b| self.loops.contains(b) && !p.contains(b)) =>
            {
                "with this loop pass"
            }
            Life::Block(_) => "with its block",
        };
        format!(
            "{holder}{via} would outlive the record it names: that record lives in {home_name}, \
             which ends {ends}; keep the record where it lasts as long as the holder (declare its \
             collection beside the holder), or give the field the record type itself so it holds \
             its own copy"
        )
    }

    fn hidden(&self, v: u16) -> bool {
        let name = self.function.name(v);
        name.starts_with("__") || name.starts_with("_elm")
    }
}
