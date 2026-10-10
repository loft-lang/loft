// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-H-RefHome` — a stored `reference<T>` does not outlive the home of the record it
//! names.  The home is the variable that holds the record: a local, or the compiler's
//! temporary for a literal or a call result.  A home lasts for the block that assigned it, so
//! a temporary assigned in a loop body lasts one pass.  Three routes reach a holder that lives
//! longer, and each is refused where it is written:
//!
//! - the WRITE itself, into a parameter, a variable of an enclosing block, or a record that
//!   lives in one (`h.r = items[1]` with `h` a parameter);
//! - a COPY of a holder into such a place (`out.hs += [h]`): the copy carries the reference;
//! - a CALL whose callee stores a reference to one argument's record into another argument
//!   (`fn set(h: H, p: Bag) { h.r = p.items[0] }`), checked at the call site, where the two
//!   arguments' lifetimes are known.  The callee's half is a summary per function, closed over
//!   the functions it calls.
//!
//! A record reached through another stored reference (`h.r = g.r`) has a home this frame
//! cannot see, and is not judged.

use super::reshape_refusals::ReshapeRefusal;
use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// A callee stores a reference to a record of argument `home` into argument `holder`.
#[derive(Clone, PartialEq, Eq)]
struct ParamPair {
    holder: u16,
    home: u16,
    field: String,
}

/// Every stored-reference write, copy or call in the program whose holder outlives the
/// record's home.
#[must_use]
pub fn stored_reference_refusals(data: &Data, database: &Stores) -> Vec<ReshapeRefusal> {
    if crate::keys::ref_home_disabled() {
        return Vec::new();
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
            let mut scratch = Vec::new();
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
    let mut out = Vec::new();
    for &d in &functions {
        run(data, database, d, &pairs, &mut out);
    }
    out
}

fn run(
    data: &Data,
    database: &Stores,
    d_nr: u32,
    pairs: &HashMap<u32, Vec<ParamPair>>,
    out: &mut Vec<ReshapeRefusal>,
) -> Vec<ParamPair> {
    let def = data.def(d_nr);
    let mut walk = Walk {
        data,
        database,
        function: &def.variables,
        file: def.position.file.to_string(),
        pairs,
        path: Vec::new(),
        next_block: 0,
        loops: Vec::new(),
        first_set: HashMap::default(),
        last_set: HashMap::default(),
        holds: HashMap::default(),
        found: Vec::new(),
        out,
    };
    walk.node(&def.code, def.position.line);
    walk.found
}

/// A record a holder's reference names: its home's variable and how long that lasts, or the
/// parameter whose record it is (`param`), whose home is the caller's.
#[derive(Clone)]
struct Hold {
    home: u16,
    home_path: Vec<u32>,
    param: Option<u16>,
    field: String,
}

/// How long a place lasts: past this frame (a parameter's), or for a block path.
enum Life {
    Caller(u16),
    Block(Vec<u32>),
}

struct Walk<'a> {
    data: &'a Data,
    database: &'a Stores,
    function: &'a Function,
    file: String,
    pairs: &'a HashMap<u32, Vec<ParamPair>>,
    /// The blocks enclosing the current node, outermost first.
    path: Vec<u32>,
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
    out: &'a mut Vec<ReshapeRefusal>,
}

impl Walk<'_> {
    fn node(&mut self, code: &Value, line: u32) {
        match code.unspan() {
            Value::Block(b) | Value::Loop(b) => {
                if matches!(code.unspan(), Value::Loop(_)) {
                    self.loops.push(self.next_block);
                }
                self.path.push(self.next_block);
                self.next_block += 1;
                self.stmts(&b.operators, line);
                self.path.pop();
            }
            Value::Insert(ops) => self.stmts(ops, line),
            Value::Set(v, rhs) => {
                self.node(rhs, line);
                self.assigned(*v);
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
                        self.write(&args[0], &args[1], &args[2], line);
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

    fn write(&mut self, holder: &Value, pos: &Value, value: &Value, line: u32) {
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
                line,
            );
            return;
        }
        let Some(home) = self.home(value) else {
            return;
        };
        let Some(hold) = self.hold_for(home, field) else {
            return;
        };
        let direct = matches!(holder.unspan(), Value::Var(_));
        self.carry(h, &lasts, vec![hold], "", direct, line);
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
            });
        }
        Some(Hold {
            home,
            home_path: self.home_life(home)?,
            param: None,
            field,
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

    /// `holds` reach the holder rooted at `h`: refuse one whose home ends first, hand a link
    /// between two parameters to the caller, and remember the rest for the copies made of it.
    fn carry(
        &mut self,
        h: u16,
        lasts: &Life,
        holds: Vec<Hold>,
        via: &str,
        direct: bool,
        line: u32,
    ) {
        let mut kept = Vec::new();
        for hold in holds {
            match (hold.param, lasts) {
                (Some(home_param), Life::Caller(holder_param)) => {
                    if *holder_param != home_param {
                        self.found.push(ParamPair {
                            holder: *holder_param,
                            home: home_param,
                            field: hold.field.clone(),
                        });
                    }
                }
                (Some(_), Life::Block(_)) => kept.push(hold),
                (None, _) => {
                    if Self::outlives(lasts, &hold.home_path) {
                        let msg = self.message(h, &hold, lasts, via, direct);
                        self.refuse(line, msg);
                        return;
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
        let holds = self.holds_of(s);
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
            let Some(hold) = self.hold_for(home, link.field.clone()) else {
                continue;
            };
            let via = format!(" (`{}` stores it there)", self.callee_name(callee));
            let before = self.out.len();
            self.carry(
                h,
                &lasts,
                vec![hold],
                &via,
                matches!(ha.unspan(), Value::Var(_)),
                line,
            );
            if self.out.len() > before {
                return;
            }
        }
    }

    fn callee_name(&self, d: u32) -> String {
        let n = self.data.def(d).name();
        n.strip_prefix("n_").unwrap_or(n).to_string()
    }

    fn refuse(&mut self, line: u32, message: String) {
        self.out.push(ReshapeRefusal {
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
