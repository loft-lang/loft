// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `parallel { arm; arm }` as C (loft#2568): each arm is written as a function of its own and
//! the site runs them all on threads at once (`loft_parallel_run`, `loft_keyed.h`), waiting
//! until every one has ended.  The arms run AT ONCE because they may wait on each other through
//! the outside world — one arm accepting on a socket the other connects to — and an arm run
//! after the one it answers never starts.
//!
//! An arm reads the enclosing frame and writes none of it: assigning an enclosing local,
//! mutating one through a reference and capturing a parameter are compile errors
//! (LOFT_CONTROL.md § Parallel blocks).  So an arm's function starts from a COPY of the
//! enclosing locals it reads, as the interpreter's arm runs on a snapshot of the frame
//! (`parallel::run_parallel_block`): the site hands it their addresses, the arm copies each in
//! at its start and lets go of the copy at its end, so its epilogue frees only what the arm
//! made.  A local the arm assigns is the arm's own and starts at its initial value.
//!
//! The arm's function declares what the enclosing function declares — its locals, its
//! temporaries, its held vectors — so the arm's statements are written by the same writer, into
//! a body of their own, and read the same names.  The interpreter's half is
//! `parallel::run_parallel_block`; the rustc writer's is `n_parallel_block_native`.

use super::{EPILOGUE_MARK, FnWriter, var_name};
use crate::data::Value;
use std::collections::HashSet;
use std::fmt::Write as _;

/// One arm's function, written once the enclosing body's declarations are known
/// ([`FnWriter::arm_functions`]).
pub(crate) struct ArmFn {
    /// The function's C name.
    name: String,
    /// The arm's statements as the writer wrote them.
    text: String,
    /// What the arm copies in at its start: the C name, its index in the site's address array,
    /// and for a parameter its declaration (a local's is the enclosing function's own).
    copies: Vec<(String, usize, Option<String>)>,
}

/// Every C identifier `text` names.
fn identifiers(text: &str) -> HashSet<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'))
        .collect()
}

/// The name a declaration line declares: the last identifier before its initialiser.
fn declared_name(decl: &str) -> Option<&str> {
    let head = decl.split(['=', ';']).next()?;
    head.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .rfind(|w| !w.is_empty())
}

/// The variables a `Set` anywhere in `code` binds: an arm's own locals.
fn assigned(code: &Value, out: &mut HashSet<u16>) {
    code.any_node(&mut |n| {
        if let Value::Set(v, _) = n {
            out.insert(*v);
        }
        false
    });
}

impl FnWriter<'_> {
    /// `parallel { arm; … }`: each arm written into a function of its own, the site an array
    /// of the addresses the arms read and a `loft_parallel_run` over the functions.
    pub(super) fn parallel_block(&mut self, arms: &[Value], ind: usize) -> Result<(), String> {
        if arms.is_empty() {
            return Ok(());
        }
        if self.gen_elem.is_some() {
            return Err("a parallel block in a generator".to_string());
        }
        if arms
            .iter()
            .any(|a| a.any_node(&mut |n| matches!(n, Value::Return(_) | Value::Yield(_))))
        {
            return Err("a return inside a parallel arm".to_string());
        }
        self.cx.threads.set(true);
        let id = self.cx.par_sites.get();
        self.cx.par_sites.set(id + 1);
        // What the enclosing body holds when the block starts: an arm reads only these.
        let temps_before = self.temps.len();
        let decls_before = self.decls.len();
        let saved_body = std::mem::take(&mut self.body);
        let saved_pending = std::mem::take(&mut self.pending);
        let mut texts = Vec::new();
        let mut failed = None;
        for a in arms {
            if let Err(e) = self.stmt(a, 1) {
                failed = Some(e);
                break;
            }
            self.flush(1);
            texts.push(std::mem::take(&mut self.body));
        }
        self.body = saved_body;
        self.pending = saved_pending;
        if let Some(e) = failed {
            return Err(e);
        }
        // The site's addresses, shared by every arm: each name once.
        let mut addrs: Vec<String> = Vec::new();
        for (k, (arm, text)) in arms.iter().zip(texts).enumerate() {
            let names = identifiers(&text);
            let mut own = HashSet::new();
            assigned(arm, &mut own);
            let mut copies = Vec::new();
            let mut copy = |name: String, decl: Option<String>, addrs: &mut Vec<String>| {
                let at = addrs.iter().position(|a| *a == name).unwrap_or_else(|| {
                    addrs.push(name.clone());
                    addrs.len() - 1
                });
                copies.push((name, at, decl));
            };
            for v in 0..self.shapes.len() {
                let v16 = v as u16;
                let name = var_name(self.vars.name(v16), v16);
                if !names.contains(name.as_str()) || own.contains(&v16) {
                    continue;
                }
                if self.params[v] {
                    let decl = self.param_decls.get(&v16).cloned().ok_or_else(|| {
                        format!("a parallel arm reads parameter {}", self.vars.name(v16))
                    })?;
                    copy(name, Some(decl), &mut addrs);
                } else if !self.apr_holds(v16) && !self.undeclared.contains(&v16) {
                    copy(name, None, &mut addrs);
                }
            }
            for (t, _) in &self.temps[..temps_before] {
                if names.contains(t.as_str()) {
                    copy(t.clone(), None, &mut addrs);
                }
            }
            for d in &self.decls[..decls_before] {
                if let Some(n) = declared_name(d)
                    && names.contains(n)
                {
                    copy(n.to_string(), None, &mut addrs);
                }
            }
            self.arm_fns.push(ArmFn {
                name: format!("loft_arm{id}_{k}"),
                text,
                copies,
            });
        }
        let first = self.arm_fns.len() - arms.len();
        let fns: Vec<String> = self.arm_fns[first..]
            .iter()
            .map(|a| a.name.clone())
            .collect();
        let pv = if addrs.is_empty() {
            "0".to_string()
        } else {
            addrs
                .iter()
                .map(|a| format!("(void *) &{a}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        self.line(ind, "{");
        self.line(ind + 1, &format!("void *const loft_pv[] = {{ {pv} }};"));
        self.line(
            ind + 1,
            &format!("static const loft_arm_fn loft_pf[] = {{ {} }};", fns.join(", ")),
        );
        self.line(
            ind + 1,
            &format!("loft_parallel_run({}, loft_pf, loft_pv);", fns.len()),
        );
        self.line(ind, "}");
        Ok(())
    }

    /// The arms' functions, each declaring what the enclosing function declares (`prologue`),
    /// copying in what it reads, and freeing at its end only what it made: the copies are let
    /// go of before the epilogue, which frees every text the function holds.  Answers the
    /// functions' declarations and their bodies.
    pub(super) fn arm_functions(&self, prologue: &str) -> (String, String) {
        let mut decls = String::new();
        let mut bodies = String::new();
        for a in &self.arm_fns {
            let _ = writeln!(decls, "static void {}(void *const *loft_pv);", a.name);
            let _ = writeln!(bodies, "\nstatic void {}(void *const *loft_pv) {{", a.name);
            bodies.push_str(prologue);
            for (name, at, decl) in &a.copies {
                if let Some(d) = decl {
                    let _ = writeln!(bodies, "    {d};");
                }
                let _ = writeln!(bodies, "    memcpy(&{name}, loft_pv[{at}], sizeof {name});");
            }
            bodies.push_str("    (void) loft_pv;\n");
            let mut inner = a.text.clone();
            for (name, _, _) in &a.copies {
                let _ = writeln!(inner, "    memset(&{name}, 0, sizeof {name});");
            }
            let _ = writeln!(inner, "    {EPILOGUE_MARK}");
            bodies.push_str(&self.expand_epilogues_in(&inner));
            bodies.push_str("}\n");
        }
        (decls, bodies)
    }
}
