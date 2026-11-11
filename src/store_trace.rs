// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Where a store went: bytecode positions resolved to the function, the source position and
//! the variable, for the store-lifetime instruments — `LOFT_STRICT_STORES` (an access to a
//! freed store), `LOFT_REFREE_FAILS` (a second free) and `LOFT_LOG=store_life:<N>` (one
//! store's allocation, frees and the jumps its frames took to each).
//!
//! A position is the innermost construct covering the pc, with its column, so a one-line
//! `if c { a } else { b }` resolves to the arm that ran.  Code a pass inserted without a span
//! of its own resolves to the line of the statement it sits in (`line N`, no column).  The
//! table is built once at the end of code generation, only when one of the instruments is on,
//! and published per thread beside the crash hook's span table (`State::publish_source_spans`).

use crate::data::{Data, DefType};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

/// One generated function's code range.
struct Func {
    end: u32,
    name: String,
    file: &'static str,
}

/// The names a store report needs, keyed by absolute bytecode position.
#[derive(Default)]
pub struct PcNames {
    /// `start pc -> function` for every generated function.
    fns: BTreeMap<u32, Func>,
    /// `pc -> (variable name, instruction length)` for every op that reads a variable.
    vars: BTreeMap<u32, (String, u32)>,
    /// `pc -> line` of each statement's first op (the code generator's `line_numbers`).
    lines: BTreeMap<u32, u32>,
}

thread_local! {
    static PUBLISHED: RefCell<Option<Arc<PcNames>>> = const { RefCell::new(None) };
}

/// Is any instrument on that reads the table?  Asked once per compile.
#[must_use]
pub fn wanted() -> bool {
    crate::keys::strict_stores()
        || crate::keys::refree_fails()
        || std::env::var("LOFT_LOG").is_ok_and(|v| v.contains("store_life"))
}

/// Build the table from the generated code: each function's range, each variable-reading op
/// from the code generator's `pc -> variable` map, and its statement lines.
#[must_use]
pub fn build(
    bytecode: &[u8],
    vars: &crate::fxhash::FxHashMap<u32, u16>,
    lines: &BTreeMap<u32, u32>,
    data: &Data,
) -> PcNames {
    let mut names = PcNames {
        lines: lines.clone(),
        ..PcNames::default()
    };
    let mut owner: BTreeMap<u32, u32> = BTreeMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || def.is_operator() || def.code_length == 0 {
            continue;
        }
        let name = def
            .name()
            .strip_prefix("n_")
            .unwrap_or(def.name())
            .to_string();
        let end = def.code_position + def.code_length;
        names.fns.insert(
            def.code_position,
            Func {
                end,
                name,
                file: def.position().file.as_str(),
            },
        );
        owner.insert(def.code_position, d_nr);
    }
    for (&pc, &v) in vars {
        let Some((&start, func)) = names.fns.range(..=pc).next_back() else {
            continue;
        };
        if pc >= func.end {
            continue;
        }
        let Some(len) = crate::compile::instruction_len(bytecode, pc as usize, data) else {
            continue;
        };
        let var_name = data.def(owner[&start]).variables().name(v).to_string();
        names.vars.insert(pc, (var_name, len as u32));
    }
    names
}

/// Make `names` the table this thread's reports read; `None` clears it.
pub fn publish(names: Option<Arc<PcNames>>) {
    PUBLISHED.with(|p| *p.borrow_mut() = names);
}

fn with_names<R>(f: impl FnOnce(&PcNames) -> Option<R>) -> Option<R> {
    PUBLISHED.with(|p| p.borrow().as_deref().and_then(f))
}

/// The function `pc` lies in, with its first pc and its source file.
fn func_at(n: &PcNames, pc: u32) -> Option<(u32, &Func)> {
    n.fns
        .range(..=pc)
        .next_back()
        .filter(|(_, f)| pc < f.end)
        .map(|(s, f)| (*s, f))
}

/// A resolved position: file, line, and the column when a span covers the pc exactly.
struct Loc {
    file: &'static str,
    line: u32,
    col: Option<u32>,
}

/// The position of `pc`: the innermost span covering it, else the line of the statement it
/// sits in, never a position from another function.
fn locate(pc: u32) -> Option<Loc> {
    if let Some(p) = crate::crash_report::source_loc_for_pc(pc) {
        return Some(Loc {
            file: p.file.as_str(),
            line: p.line,
            col: Some(p.pos),
        });
    }
    with_names(|n| {
        let (start, func) = func_at(n, pc)?;
        let (_, line) = n.lines.range(start..=pc).next_back()?;
        Some(Loc {
            file: func.file,
            line: *line,
            col: None,
        })
    })
}

fn base(file: &str) -> &str {
    file.rsplit(['/', '\\']).next().unwrap_or(file)
}

/// `pc` as a reader looks it up: `` `fn` file:line:col (pc=N) ``, or `` `fn` file line L
/// (pc=N) `` when only the statement's line is known.
#[must_use]
pub fn site(pc: u32) -> String {
    let func =
        with_names(|n| func_at(n, pc).map(|(_, f)| format!("`{}` ", f.name))).unwrap_or_default();
    match locate(pc) {
        Some(Loc {
            file,
            line,
            col: Some(c),
        }) => format!("{func}{}:{line}:{c} (pc={pc})", base(file)),
        Some(Loc {
            file,
            line,
            col: None,
        }) => format!("{func}{} line {line} (pc={pc})", base(file)),
        None => format!("{func}pc={pc}"),
    }
}

/// `line:col` (or `line L`) alone, for a jump's target.
fn site_short(pc: u32) -> String {
    match locate(pc) {
        Some(Loc {
            line, col: Some(c), ..
        }) => format!("{line}:{c} (pc={pc})"),
        Some(Loc {
            line, col: None, ..
        }) => format!("line {line} (pc={pc})"),
        None => format!("pc={pc}"),
    }
}

/// The source line `pc` resolves to, with a caret under its column, indented for a report.
#[must_use]
pub fn excerpt(pc: u32) -> Option<String> {
    let loc = locate(pc)?;
    let text = crate::file_access::read_to_string(loc.file).ok()?;
    let line = text.lines().nth(loc.line.checked_sub(1)? as usize)?;
    let gutter = loc.line.to_string();
    let pad = " ".repeat(gutter.len());
    let mut out = format!("    {gutter} | {line}");
    if let Some(c) = loc.col {
        let _ = write!(
            out,
            "\n    {pad} | {}^",
            " ".repeat(c.saturating_sub(1) as usize)
        );
    }
    Some(out)
}

/// The variables whose reads lead straight into the op at `pc`: the operands a free or a
/// store op was handed, first operand first.  Empty when an operand is not a plain variable
/// read (a field, a call's answer).
#[must_use]
pub fn operands_at(pc: u32) -> Vec<String> {
    with_names(|n| {
        let mut out = Vec::new();
        let mut at = pc;
        while let Some((&p, (name, len))) = n.vars.range(..at).next_back() {
            if p + len != at {
                break;
            }
            out.push(name.clone());
            at = p;
        }
        out.reverse();
        Some(out)
    })
    .unwrap_or_default()
}

/// The variable a free at `pc` released, or `` `a` (unless `w` names it) `` for a free
/// guarded by a witness; `None` when the freed reference was not a plain variable.
#[must_use]
pub fn freed_variable(pc: u32) -> Option<String> {
    match operands_at(pc).as_slice() {
        [] => None,
        [v] => Some(format!("`{v}`")),
        [v, w, ..] => Some(format!("`{v}` (unless `{w}` names it)")),
    }
}

/// One step of a frame's path: a jump op that ran, and where control went.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// Call depth of the frame that ran it.
    pub depth: usize,
    /// The jump op's position.
    pub pc: u32,
    /// Where control went next.
    pub to: u32,
    /// The op jumped (rather than falling through).
    pub taken: bool,
    /// A conditional op of the `OpGotoFalse` family: jumping means the condition was false.
    pub on_false: bool,
}

/// The jumps of every live frame, consecutive repeats of one step folded into a count (a
/// loop's back jump).
#[derive(Default)]
pub struct Path {
    steps: Vec<(Step, u32)>,
}

impl Path {
    /// Record `step`; the steps of frames deeper than it have returned and are dropped.
    pub fn push(&mut self, step: Step) {
        self.leave(step.depth);
        if let Some((last, n)) = self.steps.last_mut()
            && *last == step
        {
            *n += 1;
            return;
        }
        self.steps.push((step, 1));
    }

    /// Start the frame at `depth` afresh: a store allocated there is followed from its
    /// allocation, not from an earlier pass of a loop.
    pub fn restart(&mut self, depth: usize) {
        while self.steps.last().is_some_and(|(s, _)| s.depth >= depth) {
            self.steps.pop();
        }
    }

    /// Drop the steps of every frame deeper than `depth`.
    pub fn leave(&mut self, depth: usize) {
        while self.steps.last().is_some_and(|(s, _)| s.depth > depth) {
            self.steps.pop();
        }
    }

    /// The last `max` steps of the frame at `depth`, one line each.
    #[must_use]
    pub fn render(&self, depth: usize, max: usize) -> Vec<String> {
        let mine: Vec<&(Step, u32)> = self
            .steps
            .iter()
            .filter(|(s, _)| s.depth == depth)
            .collect();
        let skip = mine.len().saturating_sub(max);
        let mut out = Vec::new();
        if skip > 0 {
            out.push(format!("… {skip} earlier steps"));
        }
        for (s, n) in &mine[skip..] {
            let what = match (s.on_false, s.taken) {
                (true, true) => "condition false, jumped",
                (true, false) => "condition true, fell through",
                (false, true) => "jumped",
                (false, false) => "fell through",
            };
            let times = if *n > 1 {
                format!(" ×{n}")
            } else {
                String::new()
            };
            out.push(format!(
                "{}: {what} to {}{times}",
                site_short(s.pc),
                site_short(s.to)
            ));
        }
        out
    }
}
