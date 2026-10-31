// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! Generators as C (@F34): a function returning `iterator<T>` becomes a frame struct holding
//! every local, a creator that fills the frame from the arguments, a resume function that
//! restores the locals, jumps to the label after the last `yield` and saves them again at the
//! next one, and a release.  The body is written as any other; a `yield` and a `return` are the
//! only statements that read differently.  The runtime half is the generator table in
//! `loft_rt.h`.

use super::{E, FnWriter, K, as_ref, as_shape, as_view};
use crate::c::{CType, held, passed};
use crate::data::{Type, Value};
use std::fmt::Write as _;

/// What a generator's C is, beside its creator's body: the frame struct and the prototypes,
/// written before the unit's functions, and the resume and release functions, after them.
pub(crate) struct GenParts {
    pub(crate) decls: String,
    pub(crate) bodies: String,
}

impl FnWriter<'_> {
    /// `yield x`: the value through `out`, the locals into the frame, and the label the next
    /// resume jumps to.
    pub(super) fn gen_yield(&mut self, x: &Value, ind: usize) -> Result<(), String> {
        let c = self.gen_elem.ok_or("a yield outside a generator")?;
        // A tuple holding a text is copied into the consumer's value member by member: the
        // texts are the consumer's own copies, the rest moves (`(G-Own)`).
        if let CType::Tuple(id) = c
            && self.cx.tuple_has_text(id)
        {
            let t = self.temp(c);
            self.assign(x, &t, c, ind)?;
            self.copy_member(&t, &format!("(*({} *) out)", held(c)), c, ind);
        } else {
            let e = self.expr(x, ind)?;
            let put = match c {
                CType::Text => format!("loft_text_set((loft_text *) out, {});", as_view(e)?),
                other => format!("*({} *) out = {};", held(other), as_shape(e, other)?),
            };
            self.line(ind, &put);
        }
        // `(G-Own)`: the consumer owns what it is handed, so the generator forgets the temps
        // holding it before it suspends — its release and its tail then find them null, as the
        // interpreter's yield does (`coroutine_layout::yield_handed_temps`).  A temp declared
        // inside the yielded value ended with its block and is skipped.
        let mut inner_scopes = std::collections::HashSet::new();
        x.walk(&mut |n| {
            if let Value::Block(b) = n {
                inner_scopes.insert(b.scope);
            }
        });
        for t in crate::coroutine_layout::yield_handed_temps(self.cx.data, self.d, x) {
            if inner_scopes.contains(&self.vars.scope(t)) {
                continue;
            }
            match self.shapes.get(t as usize) {
                Some(CType::Ref) => self.line(ind, &format!("{} = loft_ref_null();", self.var(t))),
                Some(CType::Fn) => {
                    self.line(ind, &format!("{}.clos = loft_ref_null();", self.var(t)));
                }
                _ => {}
            }
        }
        self.flush(ind);
        self.yields += 1;
        let k = self.yields;
        self.line(ind, &format!("fr->state = {k};"));
        self.line(ind, &format!("LOFT_SAVE{};", self.d));
        self.line(ind, "return;");
        self.line(0, &format!("loft_y{k}:;"));
        Ok(())
    }

    /// The generator operators, or `None` for another.
    pub(super) fn gen_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        match (name, args) {
            // The next value: the iterator's element type, its null when the generator has
            // run to its end.
            // A tuple's transport kinds follow the size: only `--native`'s channel reads them.
            ("OpCoroutineNext", [g, _, ..]) => {
                let tp = self
                    .held_type(g)
                    .ok_or("a generator step on a value of no known type")?;
                let Type::Iterator(elem, _) = tp.base() else {
                    return Err("a generator step on a value that is not an iterator".into());
                };
                let c = self
                    .cx
                    .ctype(elem)
                    .filter(|c| *c != CType::Void)
                    .ok_or("a generator of nothing")?;
                let g = if let Value::Var(v) = g.unspan() {
                    self.var(*v)
                } else {
                    let e = self.expr(g, ind)?;
                    as_ref(e)?
                };
                let t = self.temp(c);
                if c == CType::Text {
                    self.line(ind, &format!("loft_text_set(&{t}, loft_null_text());"));
                    self.line(ind, &format!("loft_gen_next({g}, &{t});"));
                    return Ok(Some(self.held(t, CType::Text)));
                }
                // An exhausted generator answers null members: a text member reads its null,
                // set in the buffer the step reuses.
                let mut leaves = Vec::new();
                if let CType::Tuple(id) = c {
                    self.tuple_text_leaves(&t, id, &mut leaves);
                }
                if leaves.is_empty() {
                    self.null_into(&t, c, ind);
                } else {
                    self.null_tuple_keeping_texts(&t, c, ind);
                    for l in &leaves {
                        self.line(ind, &format!("loft_text_set(&{l}, loft_null_text());"));
                    }
                }
                self.line(ind, &format!("loft_gen_next({g}, &{t});"));
                Ok(Some(E::new(t, super::kind_of(c))))
            }
            ("OpCoroutineRetain", [g]) => {
                let g = self.expr(g, ind)?;
                Ok(Some(E::new(
                    format!("loft_gen_retain({})", as_ref(g)?),
                    K::Ref,
                )))
            }
            ("OpCoroutineExhausted", [g]) => {
                let g = self.expr(g, ind)?;
                Ok(Some(E::new(
                    format!("loft_gen_exhausted({})", as_ref(g)?),
                    K::Bool,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The static type of `v` where a handle is held: a variable, a field of a record (an
    /// `OpGetDbRef` at a field's position) or an element of a vector (`OpGetVector`, whose
    /// type is the element's) — `(G-Hold)`: a handle is a value in any of them.
    fn held_type(&self, v: &Value) -> Option<Type> {
        let data = self.cx.data;
        let peel = |t: Type| match t {
            Type::RefVar(inner) | Type::Optional(inner) => *inner,
            t => t,
        };
        match v.unspan() {
            Value::Var(x) => Some(peel(self.vars.tp(*x).clone())),
            Value::TupleGet(x, i) => match self.vars.tp(*x).base() {
                Type::Tuple(elems) => elems.get(*i as usize).cloned(),
                _ => None,
            },
            Value::Call(d, args) => match (data.def(*d).name(), &args[..]) {
                ("OpGetVector", [vec, ..]) => match self.held_type(vec)?.base() {
                    Type::Vector(elem, _) => Some((**elem).clone()),
                    _ => None,
                },
                // A keyed lookup: the collection's record.
                ("OpGetRecord", [coll, ..]) => match self.held_type(coll)?.base() {
                    Type::Hash(r, ..)
                    | Type::Sorted(r, ..)
                    | Type::Index(r, ..)
                    | Type::Radix(r, ..)
                    | Type::Trie(r, ..) => Some(Type::Reference(*r, crate::data::Deps::default())),
                    _ => None,
                },
                ("OpGetDbRef" | "OpGetField", [rec, pos, ..]) => {
                    let Value::Int(pos) = pos.unspan() else {
                        return None;
                    };
                    let tp = self.held_type(rec)?;
                    match tp.base() {
                        Type::Reference(r, _) => {
                            let def = data.def(*r);
                            let known = def.known_type();
                            def.attributes()
                                .iter()
                                .find(|a| {
                                    i32::from(self.cx.stores.position(known, &a.name)) == *pos
                                })
                                .map(|a| peel(a.typedef.clone()))
                        }
                        // A stored tuple: the member at that offset of its `__tuple<…>` record.
                        Type::Tuple(elems) => {
                            let at =
                                crate::data::stored_tuple_offsets(data, self.cx.stores, elems)?;
                            at.iter()
                                .position(|o| i32::from(*o) == *pos)
                                .map(|i| elems[i].clone())
                        }
                        // An element that is no record is the value at its start.
                        _ if *pos == 0 => Some(tp),
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The generator: its creator's body, the frame, the resume and the release.
    pub(crate) fn write_gen(mut self) -> Result<(String, GenParts), String> {
        let d = self.d;
        // A frame saves and restores every local as a struct copy — a move, the texts a tuple
        // holds included: the local and its field never both own them.
        let code = self.code;
        self.stmt(code, 1)?;
        self.line(0, "loft_gen_end:");
        self.epilogue(1);
        self.line(1, "fr->done = 1;");
        // `@FR-G-Arg-Rebind`: a frame that ran to its end gives back its argument holds.
        let holds = self.held_args();
        for n in &holds {
            self.line(1, &format!("loft_release_frame_hold(fr->hold_{n});"));
            self.line(1, &format!("fr->hold_{n} = loft_ref_null();"));
        }
        // Every variable and temporary is a field: a parameter as it is passed, the rest as
        // they are held.
        // A `&` parameter is the pointer to the caller's variable, kept in the frame as passed.
        let mut fields: Vec<(String, String, CType, bool)> = (0..self.shapes.len())
            .map(|v| {
                let c = self.shapes[v];
                let ty = if self.links[v] {
                    format!("{} *", held(c))
                } else if self.params[v] {
                    passed(c)
                } else {
                    held(c)
                };
                let name = super::var_name(self.vars.name(v as u16), v as u16);
                // A link is the caller's: nothing of it is the frame's to copy or free.
                (
                    ty,
                    name,
                    if self.links[v] { CType::Void } else { c },
                    self.params[v],
                )
            })
            .collect();
        for (t, c) in &self.temps {
            fields.push((held(*c), t.clone(), *c, false));
        }
        let mut decls = String::from("typedef struct { int32_t state; int32_t done;");
        for (ty, n, c, param) in &fields {
            let _ = write!(decls, " {ty} {n};");
            if *param && *c == CType::Text {
                let _ = write!(decls, " loft_text own_{n};");
            }
        }
        for n in &holds {
            let _ = write!(decls, " loft_ref hold_{n};");
        }
        let _ = writeln!(
            decls,
            " }} loft_gen{d};\nstatic void loft_resume{d}(void *frp, void *out);\nstatic void loft_genfree{d}(void *frp);"
        );
        // A generator with a link names its locals IN the frame: a link to one points at the
        // frame field, which lives across resumes, where a restored copy would not.
        let in_frame = self.links.iter().any(|l| *l);
        let saves: Vec<String> = if in_frame {
            Vec::new()
        } else {
            fields
                .iter()
                .map(|(_, n, _, _)| format!("fr->{n} = {n};"))
                .collect()
        };
        let mut bodies = format!(
            "\n#define LOFT_SAVE{d} do {{ {} }} while (0)\nstatic void loft_resume{d}(void *frp, void *out) {{\n    loft_gen{d} *fr = (loft_gen{d} *) frp;\n",
            saves.join(" ")
        );
        for (ty, n, _, _) in &fields {
            if in_frame {
                let _ = writeln!(bodies, "#define {n} (fr->{n})");
            } else {
                let _ = writeln!(bodies, "    {ty} {n} = fr->{n};");
            }
        }
        bodies.push_str("    (void) out;\n    switch (fr->state) {\n");
        for k in 1..=self.yields {
            let _ = writeln!(bodies, "    case {k}: goto loft_y{k};");
        }
        bodies.push_str("    default: break;\n    }\n");
        bodies.push_str(&self.expand_epilogues());
        let _ = writeln!(bodies, "}}\n#undef LOFT_SAVE{d}");
        if in_frame {
            for (_, n, _, _) in &fields {
                let _ = writeln!(bodies, "#undef {n}");
            }
        }
        self.write_release(&mut bodies, &fields);
        let mut creator = self.write_creator(&fields);
        let _ = writeln!(
            creator,
            "    return loft_gen_new(fr, loft_resume{d}, loft_genfree{d});"
        );
        Ok((creator, GenParts { decls, bodies }))
    }

    /// The release: a generator stopped before its end still holds its texts — its own, the
    /// ones its tuples hold, and its copies of text arguments.
    /// The parameters whose store the frame holds for its life (`@FR-G-Arg-Rebind`): every
    /// heap argument — a record, a vector, a collection — passed by value.
    fn held_args(&self) -> Vec<String> {
        (0..self.shapes.len())
            .filter(|&v| {
                self.params[v]
                    && !self.links[v]
                    && self.shapes[v] == CType::Ref
                    && crate::data::is_dbref(self.vars.tp(v as u16).base())
            })
            .map(|v| super::var_name(self.vars.name(v as u16), v as u16))
            .collect()
    }

    /// The heap locals a generator stopped before its end still owns, which its body frees only
    /// at its tail: `State::owned_store_locals_in_bytes`'s set — every local of a heap type, or
    /// a generator handle, that owns its store and that no closure adopted, and every generator
    /// handle it was handed (`(G-Hold)`); any other argument is the caller's.
    fn abandoned_stores(&self) -> Vec<String> {
        (0..self.shapes.len())
            .filter(|&v| {
                let var = v as u16;
                let tp = self.vars.tp(var);
                let generator = matches!(tp.base(), crate::data::Type::Iterator(_, _));
                !self.links[v]
                    && self.shapes[v] == CType::Ref
                    && (!self.params[v] || generator)
                    && (crate::data::is_dbref(tp) || generator)
                    && self.vars.owns_store(var)
                    && !self.vars.is_captured(var)
            })
            .map(|v| super::var_name(self.vars.name(v as u16), v as u16))
            .collect()
    }

    fn write_release(&self, bodies: &mut String, fields: &[(String, String, CType, bool)]) {
        let d = self.d;
        let _ = writeln!(
            bodies,
            "static void loft_genfree{d}(void *frp) {{\n    loft_gen{d} *fr = (loft_gen{d} *) frp;\n    if (!fr->done) {{"
        );
        for (_, n, c, param) in fields {
            if *c == CType::Text && !param {
                let _ = writeln!(bodies, "        loft_text_free(&fr->{n});");
            }
            if let CType::Tuple(id) = c
                && !param
            {
                let mut leaves = Vec::new();
                self.tuple_text_leaves(&format!("fr->{n}"), *id, &mut leaves);
                for l in leaves {
                    let _ = writeln!(bodies, "        loft_text_free(&{l});");
                }
            }
        }
        for n in self.abandoned_stores() {
            let _ = writeln!(
                bodies,
                "        if (fr->{n}.rec != 0) {{ loft_free_ref(fr->{n}); fr->{n} = loft_ref_null(); }}"
            );
        }
        bodies.push_str("    }\n");
        for (_, n, c, param) in fields {
            if *c == CType::Text && *param {
                let _ = writeln!(bodies, "    loft_text_free(&fr->own_{n});");
            }
        }
        for n in self.held_args() {
            let _ = writeln!(bodies, "    loft_release_frame_hold(fr->hold_{n});");
        }
        bodies.push_str("    free(fr);\n}\n");
    }

    /// The creator: the frame, its arguments — a text copied, since the generator outlives the
    /// caller's view — and every other field at its initial value.
    fn write_creator(&self, fields: &[(String, String, CType, bool)]) -> String {
        let d = self.d;
        let mut creator = format!(
            "    loft_gen{d} *fr = (loft_gen{d} *) calloc(1, sizeof *fr);\n    if (!fr) {{ fputs(\"loft: out of memory\\n\", stderr); abort(); }}\n"
        );
        for (ty, n, c, param) in fields {
            if *param {
                if *c == CType::Text {
                    let _ = writeln!(
                        creator,
                        "    fr->own_{n} = loft_text_own({n});\n    fr->{n} = loft_view(&fr->own_{n});"
                    );
                } else {
                    let _ = writeln!(creator, "    fr->{n} = {n};");
                }
            } else if *c == CType::Void {
                // A `&` local: no target until it is bound.
                let _ = writeln!(creator, "    fr->{n} = 0;");
            } else {
                let _ = writeln!(creator, "    fr->{n} = ({ty}) {};", self.cx.initial(*c));
            }
        }
        // `@FR-G-Arg-Rebind`: the frame holds each heap argument's store for its life.
        for n in self.held_args() {
            let _ = writeln!(
                creator,
                "    fr->hold_{n} = loft_hold_for_frame({n}) ? {n} : loft_ref_null();"
            );
        }
        creator
    }
}
