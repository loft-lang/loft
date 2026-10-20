// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-StreamWalk` — the emission of the plan `stream_walk` decides:
//!
//! - a producer's STREAM TWIN `P__sw<k><F: FnMut(cell, field…)>(cell, param…, __walk: &mut F)`:
//!   its body without the result buffer, which the prologue binds to the null reference so
//!   every free and guard the body names of it skips; each append hands the values of the
//!   fields its walk reads to `__walk`, and a `return` of the buffer ends the stream;
//! - a consumer's WALK: the call's statement emits nothing, and the walk is the twin's call
//!   with the loop's body as its closure, where each read of a field of the element is the
//!   closure's parameter for that field.
use std::io::Write;

use super::Output;
use super::stream_walk::{Kind, addr, elem_expr};
use crate::data::Value;

/// A walk whose closure is being emitted.
#[derive(Clone, Debug)]
pub struct SwFrame {
    pub runs: u16,
    pub index: u16,
    /// The element local and its binding statement, when the body binds one.
    pub elem: Option<(u16, usize)>,
    /// `(offset, kind, parameter name)` per field handed over.
    pub params: Vec<(i64, Kind, String)>,
}

impl Output<'_> {
    /// The stream twin being emitted: its producer and its index.
    fn sw_twin_of(&self) -> Option<(u32, usize)> {
        self.sw_twin
    }

    /// The function's name suffix and generics, when it is a stream twin.
    pub(super) fn sw_signature_name(&self) -> Option<String> {
        let (_, k) = self.sw_twin_of()?;
        let fields = &self.sw.twins[k].1;
        let tys: Vec<&str> = fields.iter().map(|(_, kind)| kind.rust()).collect();
        Some(format!(
            "__sw{k}<F: FnMut(&std::cell::UnsafeCell<Stores>, {})>",
            tys.join(", ")
        ))
    }

    /// Whether parameter `i` of `def_nr` is dropped from the twin: the result buffer.
    pub(super) fn sw_param_dropped(&self, def_nr: u32, i: usize) -> bool {
        self.sw_twin_of()
            .is_some_and(|(pd, _)| pd == def_nr && self.sw.producers[&pd].buf_attr == i)
    }

    /// Parameters written after the function's own.
    pub(super) fn sw_trailing_params(&self) -> &'static str {
        if self.sw_twin.is_some() {
            ", __walk: &mut F"
        } else {
            ""
        }
    }

    /// The twin's prologue: the result buffer, bound to the null reference.
    pub(super) fn sw_prologue(&mut self, def_nr: u32) -> String {
        let Some((pd, _)) = self.sw_twin_of() else {
            return String::new();
        };
        if pd != def_nr {
            return String::new();
        }
        let def = self.data.def(pd);
        let attr = &def.attributes()[self.sw.producers[&pd].buf_attr];
        let v = def.variables().var(&attr.name);
        self.declared.insert(v);
        format!(
            "\n  let mut var_{}: DbRef = DbRef::NULL;",
            super::sanitize(&attr.name)
        )
    }

    /// A statement the plan rewrites: a twin's dropped statement or an append it hands over,
    /// a consumer's call or walk, an element binding inside a walk.  Answers whether it wrote
    /// (or dropped) the statement.
    pub(super) fn sw_statement(
        &mut self,
        w: &mut dyn Write,
        operators: &[Value],
        vnr: usize,
    ) -> std::io::Result<bool> {
        let v = &operators[vnr];
        let at = addr(v);
        if let Some((pd, k)) = self.sw_twin_of() {
            let prod = &self.sw.producers[&pd];
            if prod.drop.contains(&at) {
                return Ok(true);
            }
            if prod.groups.contains_key(&at) {
                self.sw_yield(w, operators, vnr, pd, k)?;
                return Ok(true);
            }
        }
        if self
            .sw_frames
            .iter()
            .any(|f| f.elem.is_some_and(|(_, b)| b == at))
        {
            return Ok(true);
        }
        if let Some(&(g, idx)) = self.sw.at.get(&at)
            && g == self.def_nr
        {
            let site = self.sw.sites[&g][idx].clone();
            if site.call == at {
                return Ok(true);
            }
            if site.walk == at {
                let Some(call) = operators.iter().find(|o| addr(o) == site.call) else {
                    return Ok(false);
                };
                self.sw_walk(w, call, v, &site)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The append minted at statement `vnr`: its walked fields' values, handed to the walk.
    fn sw_yield(
        &mut self,
        w: &mut dyn Write,
        operators: &[Value],
        vnr: usize,
        pd: u32,
        k: usize,
    ) -> std::io::Result<()> {
        let group = self.sw.producers[&pd].groups[&addr(&operators[vnr])].clone();
        let fields = self.sw.twins[k].1.clone();
        crate::rewrite_census::fired("R-StreamWalk", 1);
        self.indent(w)?;
        writeln!(w, "{{ //@FR-R-StreamWalk yield")?;
        let mut names = Vec::new();
        let mut evaluated: Vec<&Value> = Vec::new();
        for (j, (off, kind)) in fields.iter().enumerate() {
            let Some(&(_, _, val_at)) = group.sets.iter().find(|(o, _, _)| o == off) else {
                continue;
            };
            let Some(val) = operators[vnr + 1..].iter().find_map(|o| match o.unspan() {
                Value::Call(_, args) => args.get(2).filter(|a| addr(a) == val_at),
                _ => None,
            }) else {
                continue;
            };
            self.indent(w)?;
            if *kind == Kind::Text {
                if matches!(val.unspan(), Value::Null) {
                    writeln!(w, "  let __sw{j}: &str = loft::state::STRING_NULL;")?;
                } else {
                    write!(w, "  let __swt{j} = &(")?;
                    self.output_code_inner(w, val)?;
                    writeln!(w, "); let __sw{j}: &str = AsRef::<str>::as_ref(__swt{j});")?;
                }
            } else {
                write!(w, "  let __sw{j}: {} = ", kind.rust())?;
                self.output_code_inner(w, val)?;
                writeln!(w, ";")?;
            }
            names.push(format!("__sw{j}"));
            evaluated.push(val);
        }
        self.indent(w)?;
        writeln!(w, "  __walk(cell, {});", names.join(", "))?;
        // The texts the evaluated values made, released once the walk has read them.
        for (at, t) in &group.frees {
            if !evaluated.iter().any(|v| {
                v.any_node(&mut |n| matches!(n, Value::Var(x) | Value::Set(x, _) if x == t))
            }) {
                continue;
            }
            if let Some(free) = operators[vnr + 1..].iter().find(|o| addr(o) == *at) {
                self.indent(w)?;
                write!(w, "  ")?;
                self.output_code_inner(w, free)?;
                writeln!(w, ";")?;
            }
        }
        self.indent(w)?;
        writeln!(w, "}}")
    }

    /// The walk: the twin's call with the loop body as its closure.
    fn sw_walk(
        &mut self,
        w: &mut dyn Write,
        call: &Value,
        walk: &Value,
        site: &super::stream_walk::Site,
    ) -> std::io::Result<()> {
        let Value::Set(_, rhs) = call.unspan() else {
            return Ok(());
        };
        let Value::Call(pd, args) = rhs.unspan() else {
            return Ok(());
        };
        let Value::Block(fb) = walk.unspan() else {
            return Ok(());
        };
        let Some(Value::Loop(lb)) = fb
            .operators
            .iter()
            .map(Value::unspan)
            .find(|o| matches!(o, Value::Loop(_)))
        else {
            return Ok(());
        };
        let Some(body) = lb
            .operators
            .iter()
            .filter(|o| matches!(o.unspan(), Value::Block(_)))
            .nth(0)
        else {
            return Ok(());
        };
        let buf_attr = self.sw.producers[pd].buf_attr;
        let fields = self.sw.twins[site.twin].1.clone();
        self.sw_serial += 1;
        let serial = self.sw_serial;
        let params: Vec<(i64, Kind, String)> = fields
            .iter()
            .enumerate()
            .map(|(j, (off, kind))| (*off, *kind, format!("__sf{serial}_{j}")))
            .collect();
        let name = self.fn_ident(self.data.def(*pd));
        self.indent(w)?;
        writeln!(w, "{{ //@FR-R-StreamWalk walk")?;
        self.indent(w)?;
        write!(w, "  {name}__sw{}(cell", site.twin)?;
        for (i, a) in args.iter().enumerate() {
            if i == buf_attr {
                continue;
            }
            write!(w, ", ")?;
            self.output_code_inner(w, a)?;
        }
        let sig: Vec<String> = params
            .iter()
            .map(|(_, kind, n)| format!("{n}: {}", kind.rust()))
            .collect();
        writeln!(
            w,
            ", &mut |cell: &std::cell::UnsafeCell<Stores>, {}| {{",
            sig.join(", ")
        )?;
        self.indent(w)?;
        writeln!(
            w,
            "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        self.indent(w)?;
        writeln!(w, "    let _ = &stores;")?;
        self.sw_frames.push(super::stream_walk_emit::SwFrame {
            runs: site.runs,
            index: site.index,
            elem: site.elem,
            params,
        });
        self.indent(w)?;
        write!(w, "    ")?;
        let r = self.output_code_inner(w, body);
        self.sw_frames.pop();
        r?;
        writeln!(w, ";")?;
        self.indent(w)?;
        writeln!(w, "  }});")?;
        self.indent(w)?;
        write!(w, "}}")
    }

    /// The parameter a field read of a walked element is, when `node` is one.
    fn sw_read(&self, node: &Value) -> Option<String> {
        let Value::Call(d, args) = node.unspan() else {
            return None;
        };
        if (*d as usize) >= self.data.definitions.len() {
            return None;
        }
        let name = self.data.def(*d).name();
        let off = match args.get(1).map(Value::unspan) {
            Some(Value::Int(o)) => i64::from(*o),
            Some(Value::Long(o)) => *o,
            _ => return None,
        };
        let e = args.first()?;
        for f in self.sw_frames.iter().rev() {
            let through = elem_expr(self.data, e, f.runs, f.index)
                || f.elem
                    .is_some_and(|(r, _)| matches!(e.unspan(), Value::Var(x) if *x == r));
            if through {
                return f
                    .params
                    .iter()
                    .find(|(o, k, _)| *o == off && Some(*k) == kind_of_get(name))
                    .map(|(_, _, n)| n.clone());
            }
        }
        None
    }

    /// The hook: a field read of a walked element is its closure parameter; a twin's
    /// `return` of its result buffer ends the stream.
    pub(super) fn sw_expr(&mut self, w: &mut dyn Write, node: &Value) -> std::io::Result<bool> {
        if let Some((pd, _)) = self.sw_twin_of()
            && let Value::Return(r) = node.unspan()
        {
            let def = self.data.def(pd);
            let buf = def
                .variables()
                .var(&def.attributes()[self.sw.producers[&pd].buf_attr].name);
            if matches!(r.unspan(), Value::Var(x) if *x == buf) {
                write!(w, "return")?;
                return Ok(true);
            }
        }
        if !self.sw_frames.is_empty()
            && let Some(p) = self.sw_read(node)
        {
            write!(w, "{p}")?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Operands pre-evaluation may search in a node the hook rewrites: none.
    pub(super) fn sw_operands<'v>(&self, v: &'v Value) -> Option<Vec<&'v Value>> {
        if !self.sw_frames.is_empty() && self.sw_read(v).is_some() {
            return Some(Vec::new());
        }
        None
    }

    /// The stream twins beside producer `dnr`.
    pub(super) fn output_sw_twins(
        &mut self,
        w: &mut dyn Write,
        dnr: u32,
        program_store: Option<&(crate::database::Stores, crate::keys::DbRef)>,
    ) -> std::io::Result<()> {
        if !self.sw.producers.contains_key(&dnr) {
            return Ok(());
        }
        let twins: Vec<usize> = self
            .sw
            .twins
            .iter()
            .enumerate()
            .filter(|(_, (pd, _))| *pd == dnr)
            .map(|(k, _)| k)
            .collect();
        for k in twins {
            self.sw_twin = Some((dnr, k));
            let r = self.output_function(w, dnr, program_store);
            self.sw_twin = None;
            r?;
        }
        Ok(())
    }
}

fn kind_of_get(name: &str) -> Option<Kind> {
    match name {
        "OpGetInt" => Some(Kind::Int),
        "OpGetFloat" => Some(Kind::Float),
        "OpGetSingle" => Some(Kind::Single),
        "OpGetText" => Some(Kind::Text),
        _ => None,
    }
}
