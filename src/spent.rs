// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `formal/heap.md` `(H-Spent)`: a name whose value has MOVED to a new owner is spent from the
//! end of the statement that moved it, and reading it is a compile-time error that names where
//! the value went.
//!
//! What a move is has one home, `src/lease.rs` (`Frame::spends`, the `(H-Move)` verdict read off
//! the line).  This module asks it at every placement the parser writes, and walks the body in
//! execution order to find the reads that follow.  It reads the body BEFORE the scope pass: that
//! pass adds its own reads of every local — releases, hook calls, displacement snapshots — and
//! an error that fired on one of those would refuse a program the rules permit.
//!
//! A move under a branch spends the name on that path only, and a read reached from any path
//! that spent it is refused: the compiler cannot know which path runs, so the read may be of a
//! value whose release has already moved elsewhere.  A reassignment refills the name.

use crate::data::{Data, DefType, Value};
use crate::lease::{Frame, Placement};
use crate::lexer::Position;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// One read of a spent name.
#[derive(Clone, Debug)]
pub struct SpentRead {
    /// Where the read was written, when the statement carries a full position.
    pub pos: Option<Position>,
    /// The nearest line before the read.
    pub line: u32,
    /// The name as the author wrote it.
    pub name: String,
    /// The line of the statement that moved the value away.
    pub moved_line: u32,
}

/// The reads found per function, by definition number: filled before the scope pass, taken by
/// the census that raises the lease errors (`use_analysis::drop_copy_census`).
static FOUND: Mutex<Option<HashMap<u32, Vec<SpentRead>>>> = Mutex::new(None);

/// Record the spent reads of every function not yet through the scope pass.  Called at the top
/// of `scopes::check`, while each body is still the parser's.
pub fn record_all(data: &Data) {
    if !crate::keys::lease_refuse_enabled() || !data.any_drop_hook() {
        return;
    }
    let mut found = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || def.variables.done {
            continue;
        }
        let reads = spent_reads(data, d_nr);
        if !reads.is_empty() {
            found.push((d_nr, reads));
        }
    }
    let mut guard = FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let map = guard.get_or_insert_with(HashMap::new);
    for (d_nr, reads) in found {
        map.insert(d_nr, reads);
    }
}

/// The spent reads recorded for `d_nr`, removed so each is raised once.
pub fn take(d_nr: u32) -> Vec<SpentRead> {
    let mut guard = FOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .as_mut()
        .and_then(|m| m.remove(&d_nr))
        .unwrap_or_default()
}

/// The reads of a spent name in function `d_nr`'s body.
#[must_use]
pub fn spent_reads(data: &Data, d_nr: u32) -> Vec<SpentRead> {
    let def = data.def(d_nr);
    let mut flow = Flow {
        data,
        frame: Frame::new(data, def),
        func: &def.variables,
        copy_d: data.def_nr("OpCopyRecord"),
        append_d: data.def_nr("OpAppendVector"),
        loops: Vec::new(),
        line: 0,
        pos: None,
        seen: HashSet::new(),
        found: Vec::new(),
    };
    flow.stmt(&def.code, Some(HashMap::new()));
    flow.found
}

/// The names that may be spent at a point, with the line that moved each, or `None` where the
/// point cannot be reached (after a `return`, a `break` or a `continue`).
type State = Option<HashMap<u16, u32>>;

fn union(a: State, b: State) -> State {
    match (a, b) {
        (None, s) | (s, None) => s,
        (Some(mut a), Some(b)) => {
            for (v, l) in b {
                a.entry(v).or_insert(l);
            }
            Some(a)
        }
    }
}

struct Loop {
    breaks: State,
    continues: State,
}

struct Flow<'a> {
    data: &'a Data,
    frame: Frame<'a>,
    func: &'a crate::variables::Function,
    copy_d: u32,
    append_d: u32,
    loops: Vec<Loop>,
    line: u32,
    pos: Option<Position>,
    seen: HashSet<(u32, u32, u16)>,
    found: Vec<SpentRead>,
}

impl Flow<'_> {
    fn locate(&mut self, node: &Value) {
        if let Some(p) = node.span_pos() {
            self.line = p.line;
            self.pos = Some(p.clone());
        } else if let Value::Line(n) = node {
            self.line = *n;
            self.pos = None;
        }
    }

    /// The operators of a block, grouped into the author's statements.  The parser writes a
    /// construction as SIBLING operators of the statement that holds it (a literal's mint, then
    /// one write per field), so an operator is not a statement: a statement runs from one line
    /// marker to the next.  Its reads are checked against what arrived at its start, and what it
    /// moves is spent at its end — `N { h: c, tag: c.id }` reads `c` after the field write that
    /// moved it, inside the one statement.
    fn block(&mut self, ops: &[Value], mut st: State) -> State {
        let mut group = Group::default();
        for op in ops {
            let starts_statement = matches!(op, Value::Line(_)) || op.span_pos().is_some();
            let control = matches!(
                op.unspan(),
                Value::If(..)
                    | Value::Loop(_)
                    | Value::Break(_)
                    | Value::Continue(_)
                    | Value::Return(_)
            ) || matches!(op.unspan(), Value::Block(b) if holds_statements(&b.operators));
            if starts_statement || control {
                st = group.apply(st);
            }
            if control {
                st = self.stmt(op, st);
            } else {
                self.locate(op);
                if let Some(arrived) = &st {
                    // What this statement already assigned is live again for its later reads;
                    // what it moves is spent only at its end.
                    let mut live = group.start.get_or_insert_with(|| arrived.clone()).clone();
                    for (v, moved) in &group.events {
                        if moved.is_none() {
                            live.remove(v);
                        }
                    }
                    self.check_reads(op.unspan(), &live);
                    self.collect(op.unspan(), &mut group);
                }
            }
        }
        group.apply(st)
    }

    fn stmt(&mut self, node: &Value, st: State) -> State {
        self.locate(node);
        match node.unspan() {
            Value::Block(b) => self.block(&b.operators, st),
            Value::Insert(ops) => self.block(ops, st),
            Value::If(test, t, f) => {
                let st = self.leaf(test, st);
                let a = self.stmt(t, st.clone());
                let b = self.stmt(f, st);
                union(a, b)
            }
            Value::Loop(body) => {
                let mut head = st;
                loop {
                    self.loops.push(Loop {
                        breaks: None,
                        continues: None,
                    });
                    let end = self.block(&body.operators, head.clone());
                    let lp = self.loops.pop().expect("loop frame");
                    let next = union(head.clone(), union(end, lp.continues));
                    if next == head {
                        return lp.breaks;
                    }
                    head = next;
                }
            }
            Value::Break(n) => {
                let depth = self.loops.len().checked_sub(1 + *n as usize);
                if let Some(i) = depth {
                    self.loops[i].breaks = union(self.loops[i].breaks.take(), st);
                }
                None
            }
            Value::Continue(n) => {
                let depth = self.loops.len().checked_sub(1 + *n as usize);
                if let Some(i) = depth {
                    self.loops[i].continues = union(self.loops[i].continues.take(), st);
                }
                None
            }
            Value::Return(v) => {
                let st = self.leaf(v, st);
                // The function's own values end with it, a returned one included.
                let _ = st;
                None
            }
            other => self.leaf(other, st),
        }
    }

    /// A lone operator in an expression position (an `if` test, a returned value): one
    /// statement of its own.
    fn leaf(&mut self, node: &Value, st: State) -> State {
        let arrived = st?;
        self.check_reads(node, &arrived);
        let mut group = Group {
            start: Some(arrived.clone()),
            ..Group::default()
        };
        self.collect(node, &mut group);
        group.apply(Some(arrived))
    }

    fn check_reads(&mut self, node: &Value, arrived: &HashMap<u16, u32>) {
        if arrived.is_empty() {
            return;
        }
        let mut reads = Vec::new();
        node.walk(&mut |n| {
            let v = match n.unspan() {
                Value::Var(v)
                | Value::TupleGet(v, _)
                | Value::TuplePut(v, _, _)
                | Value::CallRef(v, _)
                | Value::FnRefDnr(v)
                | Value::Iter(v, _, _, _) => *v,
                _ => return,
            };
            if !reads.contains(&v) {
                reads.push(v);
            }
        });
        for v in reads {
            if let Some(&moved_line) = arrived.get(&v)
                && self.seen.insert((self.line, moved_line, v))
            {
                self.found.push(SpentRead {
                    pos: self.pos.clone(),
                    line: self.line,
                    name: self.func.name(v).to_string(),
                    moved_line,
                });
            }
        }
    }

    /// What `node` moves and what it assigns, added to the statement's group: the moves first,
    /// since an assignment's value is computed before it lands.
    fn collect(&self, node: &Value, group: &mut Group) {
        let line = self.line;
        let mut moved: Vec<(u16, u32)> = Vec::new();
        let mut refilled: Vec<u16> = Vec::new();
        node.walk(&mut |n| match n.unspan() {
            Value::Call(d, args) if *d == self.copy_d && args.len() >= 3 => {
                let placement = match args[1].unspan() {
                    Value::Var(w) if self.func.name(*w).starts_with("__disp_") => return,
                    Value::Var(w) if self.frame.is_buffer(*w) => Placement::Return,
                    _ => Placement::Structure,
                };
                moved.extend(
                    self.frame
                        .spends(&args[0], placement)
                        .into_iter()
                        .map(|v| (v, line)),
                );
            }
            Value::Call(d, args) if *d == self.append_d && args.len() >= 2 => {
                moved.extend(
                    self.frame
                        .spends(&args[1], Placement::Structure)
                        .into_iter()
                        .map(|v| (v, line)),
                );
            }
            // A whole-tuple bind `u = t` moves `t` (`(H-Move)`); the parser lowers it onto one
            // member copy per member and names the block so (`tuple_member_move`).
            Value::Block(b) if b.name == "tuple_member_move" => {
                n.walk(&mut |m| {
                    if let Value::TupleGet(t, _) = m.unspan()
                        && self.frame.written_var_verdict(*t, Placement::Structure)
                            == crate::lease::Lease::Move
                        && !self.func.is_compiler_generated(*t)
                        && self
                            .data
                            .type_owns_droppable_anywhere(self.func.tp(*t).base())
                        && !moved.iter().any(|(v, _)| v == t)
                    {
                        moved.push((*t, line));
                    }
                });
            }
            Value::Set(x, rhs) => {
                refilled.push(*x);
                if !self.func.is_compiler_generated(*x)
                    && self
                        .data
                        .type_owns_droppable_anywhere(self.func.tp(*x).base())
                {
                    moved.extend(
                        self.frame
                            .spends(rhs, Placement::Structure)
                            .into_iter()
                            .filter(|v| v != x)
                            .map(|v| (v, line)),
                    );
                }
            }
            _ => {}
        });
        group
            .events
            .extend(moved.into_iter().map(|(v, l)| (v, Some(l))));
        group.events.extend(refilled.into_iter().map(|v| (v, None)));
    }
}

/// A block the walk follows statement by statement: one holding the author's statements (a line
/// marker) or control flow of its own — a `{ }` block, an arm, a `for` loop's wrapper.  A block
/// that only computes a value (a formatted string, a literal) is part of its statement.
fn holds_statements(ops: &[Value]) -> bool {
    ops.iter().any(|o| {
        matches!(
            o.unspan(),
            Value::Line(_)
                | Value::If(..)
                | Value::Loop(_)
                | Value::Break(_)
                | Value::Continue(_)
                | Value::Return(_)
        )
    })
}

/// One author statement being read: the state it started from, and what it moves (`Some(line)`)
/// and assigns (`None`), in the order its operators run.  Order matters because the parser does
/// not mark every statement: `a = mk(); k = Hold { h: a }` can reach this as one group, and there
/// the assignment comes first and the move after it.
#[derive(Default)]
struct Group {
    start: Option<HashMap<u16, u32>>,
    events: Vec<(u16, Option<u32>)>,
}

impl Group {
    /// The state after the statement: its moves spend, then its assignments refill — an
    /// assignment's value is computed before it lands.  The group is emptied for the next one.
    fn apply(&mut self, st: State) -> State {
        let events = std::mem::take(&mut self.events);
        self.start = None;
        let mut st = st?;
        for (v, moved) in events {
            match moved {
                Some(line) => {
                    st.entry(v).or_insert(line);
                }
                None => {
                    st.remove(&v);
                }
            }
        }
        Some(st)
    }
}
