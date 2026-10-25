// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `par` as C: a `for x in v par(r = f(x), n)` loop's queue call runs the worker over the rows
//! one after another — each row's result is the one a thread would compute — into the buffer
//! stack of `loft_keyed.h`, which the loop reads by row and pops.  The interpreter's half is
//! `src/parallel_natives.rs`.

use super::{CType, E, FnWriter, K, as_int, as_ref};
use crate::c::c_name;
use crate::data::{Type, Value};

impl FnWriter<'_> {
    /// The parallel natives, or `None` for another.
    pub(super) fn par_native(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let get = |s: &mut Self, f: &str, k: K| -> Result<Option<E>, String> {
            let i = s.expr(&args[0], ind)?;
            Ok(Some(E::new(format!("{f}({})", as_int(i)?), k)))
        };
        match (name, args) {
            (
                "n_parallel_queue"
                | "n_parallel_queue_text"
                | "n_parallel_queue_ref"
                | "n_parallel_queue_narrow"
                | "n_parallel_queue_fn"
                | "n_parallel_discard",
                [input, esz, _, _, func, rest @ ..],
            ) => self.par_queue(name, input, esz, func, rest, ind).map(Some),
            ("n_parallel_buf_get", [_]) => get(self, "loft_par_get", K::Int),
            ("n_parallel_buf_get_float", [_]) => get(self, "loft_par_get_float", K::Float),
            ("n_parallel_buf_get_single", [_]) => get(self, "loft_par_get_single", K::Single),
            ("n_parallel_buf_get_ref", [_]) => get(self, "loft_par_get_ref", K::Ref),
            ("n_parallel_buf_get_fn", [_]) => get(self, "loft_par_get_fn", K::Fn),
            ("n_parallel_buf_get_text", [i]) => {
                let i = self.expr(i, ind)?;
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(ind, &format!("{t} = loft_par_get_text({});", as_int(i)?));
                Ok(Some(self.held(t, CType::Text)))
            }
            ("n_parallel_buf_get_narrow", [_, _, _]) => {
                let es = self.args(args, ind)?;
                let [i, n, s] = <[E; 3]>::try_from(es)
                    .map_err(|_| "a narrow read without three operands".to_string())?;
                Ok(Some(E::new(
                    format!(
                        "loft_par_get_narrow({}, {}, {})",
                        as_int(i)?,
                        as_int(n)?,
                        as_int(s)?
                    ),
                    K::Int,
                )))
            }
            (
                "n_parallel_buf_drop"
                | "n_parallel_buf_drop_text"
                | "n_parallel_buf_drop_narrow"
                | "n_parallel_buf_drop_fn",
                [],
            ) => Ok(Some(E::new("loft_par_drop(0)", K::Void))),
            ("n_parallel_buf_drop_ref", []) => Ok(Some(E::new("loft_par_drop(1)", K::Void))),
            _ => Ok(None),
        }
    }

    /// `n_parallel_queue…(input, element_size, return_size, threads, worker, n_extra,
    /// extra…)`: the worker over every row, each result in the new buffer; the row count.
    fn par_queue(
        &mut self,
        name: &str,
        input: &Value,
        esz: &Value,
        func: &Value,
        rest: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        let esz = Self::literal(esz, "element size")?;
        let d = u32::try_from(Self::literal(func, "worker")?)
            .map_err(|_| "a worker out of range".to_string())?;
        // The extras come before their count, the last operand.
        let extras = rest.split_last().map_or(&[][..], |(_, x)| x);
        let cx = self.cx;
        let def = cx.data.def(d);
        let input = self.expr(input, ind)?;
        let input = as_ref(input)?;
        let mut extra = Vec::new();
        for x in extras {
            let e = self.expr(x, ind)?;
            extra.push(as_int(e)?);
        }
        let WorkerCall {
            args: call_args,
            texts: buffers,
            vectors: vec_buffers,
        } = Self::worker_call(cx, d, esz, &extra)?;
        let result_shape = cx
            .ctype(def.returned())
            .ok_or("a par worker whose result has no C shape")?;
        let call = format!("{}({})", c_name(cx.data, d), call_args.join(", "));
        let store = match result_shape {
            _ if name == "n_parallel_discard" => format!("(void) {call};"),
            CType::Int => format!("loft_b[loft_i].i = {call};"),
            CType::Bool | CType::Enum | CType::Char => {
                format!("loft_b[loft_i].i = (int64_t) {call};")
            }
            CType::Float => format!("loft_b[loft_i].f = {call};"),
            CType::Single => format!("loft_b[loft_i].g = {call};"),
            CType::Text => format!("loft_b[loft_i].t = {call};"),
            // A result is ADOPTED when the worker made its store (`n_parallel_buf_drop_ref`
            // frees only those); a pass-through of the element names the source's.
            CType::Ref => format!(
                "{{ uint64_t loft_s = loft_stores_allocated; loft_b[loft_i].r = {call}; \
                 loft_b[loft_i].adopted = (uint8_t) loft_fnref_minted(loft_b[loft_i].r, loft_s, loft_ref_null()); }}"
            ),
            CType::Fn => format!("loft_b[loft_i].fn = {call};"),
            CType::Void => format!("{call};"),
            CType::Tuple(_) => return Err("a par worker returning a tuple".into()),
        };
        let n = self.temp(CType::Int);
        self.line(ind, &format!("{n} = loft_vec_len({input});"));
        self.line(ind, "{");
        self.line(
            ind + 1,
            &format!(
                "loft_par_slot *loft_b = {};",
                if name == "n_parallel_discard" {
                    "0".to_string()
                } else {
                    format!("loft_par_push({n})")
                }
            ),
        );
        self.line(ind + 1, "(void) loft_b;");
        self.line(
            ind + 1,
            &format!("for (int64_t loft_i = 0; loft_i < {n}; loft_i++) {{"),
        );
        self.line(
            ind + 2,
            &format!("loft_ref loft_e = loft_vec_get({input}, {esz}, loft_i);"),
        );
        self.line(ind + 2, "(void) loft_e;");
        for w in &buffers {
            self.line(ind + 2, &format!("loft_text {w} = {{ 0, 0, 0 }};"));
        }
        for b in &vec_buffers {
            self.line(ind + 2, &format!("loft_ref {b} = loft_ref_null();"));
            self.line(ind + 2, &format!("loft_database(&{b}, 8);"));
        }
        self.line(ind + 2, &store);
        for w in &buffers {
            self.line(ind + 2, &format!("loft_text_free(&{w});"));
        }
        self.line(ind + 1, "}");
        self.line(ind, "}");
        Ok(E::new(
            if name == "n_parallel_discard" {
                "(void) 0".to_string()
            } else {
                n
            },
            if name == "n_parallel_discard" {
                K::Void
            } else {
                K::Int
            },
        ))
    }
    /// The worker's arguments: the row first, then the extras for its next visible parameters,
    /// a work buffer for each hidden text, a buffer for a hidden heap result.
    fn worker_call(
        cx: &crate::c::Cx<'_>,
        d: u32,
        esz: i64,
        extra: &[String],
    ) -> Result<WorkerCall, String> {
        let def = cx.data.def(d);
        let mut call_args = Vec::new();
        let mut buffers = Vec::new();
        // A vector result's buffer is minted per row (a record result mints its own from null).
        let mut vec_buffers = Vec::new();
        let mut visible = 0;
        for a in def.attributes() {
            if a.hidden {
                match a.typedef.base() {
                    Type::RefVar(t) if matches!(t.base(), Type::Text(_)) => {
                        let w = format!("loft_w{}", buffers.len());
                        call_args.push(format!("&{w}"));
                        buffers.push(w);
                    }
                    Type::Vector(..) => {
                        let b = format!("loft_rb{}", vec_buffers.len());
                        call_args.push(b.clone());
                        vec_buffers.push(b);
                    }
                    Type::Reference(..) | Type::Enum(_, true, _) => {
                        call_args.push("loft_ref_null()".into());
                    }
                    _ => return Err("a par worker with a hidden parameter of another kind".into()),
                }
                continue;
            }
            if visible == 0 {
                let c = cx
                    .ctype(&a.typedef)
                    .ok_or("a par worker over rows of no C shape")?;
                // A primitive row goes by its stored bits (`read_primitive_at`).
                let bits = format!("loft_par_arg(loft_e, {esz})");
                call_args.push(match c {
                    CType::Ref => "loft_e".into(),
                    CType::Text => "loft_get_text(loft_e, 0)".into(),
                    CType::Int => format!("loft_i64_bits({bits})"),
                    CType::Float => format!("loft_f64({bits})"),
                    CType::Single => format!("loft_f32((uint32_t) {bits})"),
                    CType::Char => format!("(uint32_t) {bits}"),
                    CType::Bool | CType::Enum => format!("(uint8_t) {bits}"),
                    _ => return Err("a par worker over rows of tuples or functions".into()),
                });
            } else {
                call_args.push(
                    extra
                        .get(visible - 1)
                        .cloned()
                        .ok_or("a par worker with more parameters than extras")?,
                );
            }
            visible += 1;
        }
        Ok(WorkerCall {
            args: call_args,
            texts: buffers,
            vectors: vec_buffers,
        })
    }
}

/// A worker call's operands, and the buffers made for it per row.
struct WorkerCall {
    args: Vec<String>,
    texts: Vec<String>,
    vectors: Vec<String>,
}
