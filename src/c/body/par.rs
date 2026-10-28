// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `par` as C: a `for x in v par(r = f(x), n)` loop's queue call runs the worker over the rows
//! into the buffer stack of `loft_keyed.h`, which the loop reads by row and pops — on threads
//! (`loft_par_run`, @PLN203 C4) when the worker's result is a scalar, a text or nothing, else
//! one row after another, each row's result the one a thread would compute.  The interpreter's
//! half is `src/parallel_natives.rs`.

use super::{CType, E, FnWriter, K, as_int, as_ref, as_shape};
use crate::c::c_name;
use crate::data::{Type, Value};
use std::fmt::Write as _;

/// The dense row array a `par` site's scalar results are written into — its C element type and
/// the push that makes it (`loft_par_push_i64` / `_f64` / `_f32`, eight or four bytes a row
/// instead of a `loft_par_slot`) — or `None` for a result kind the slot holds (a text, a
/// record, a function value).
fn dense_rows(shape: CType) -> Option<(&'static str, &'static str)> {
    match shape {
        CType::Int | CType::Bool | CType::Enum | CType::Char => {
            Some(("int64_t", "loft_par_push_i64"))
        }
        CType::Float => Some(("double", "loft_par_push_f64")),
        CType::Single => Some(("float", "loft_par_push_f32")),
        _ => None,
    }
}

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
            ) => self
                .par_queue(name, input, esz, &args[3], func, rest, ind)
                .map(Some),
            ("n_parallel_fold", [input, init, func, threads, rest @ ..]) => self
                .par_fold(input, init, func, threads, rest, ind)
                .map(Some),
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
    /// A worker whose result is a scalar, a text or nothing, with scalar or reference extras,
    /// runs on threads (`loft_par_run`, @PLN203 C4 tier 1); any other runs its rows in order.
    #[expect(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "the queue native's operands, each its own, and its two routes (threads, in order) side by side"
    )]
    fn par_queue(
        &mut self,
        name: &str,
        input: &Value,
        esz: &Value,
        threads: &Value,
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
        // Each extra is the worker's next visible parameter, in that parameter's shape: a
        // captured scalar of any kind crosses.
        let shapes: Vec<CType> = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden)
            .skip(1)
            .map(|a| cx.ctype(&a.typedef).unwrap_or(CType::Int))
            .collect();
        let mut extra = Vec::new();
        for (i, x) in extras.iter().enumerate() {
            let e = self.expr(x, ind)?;
            extra.push(as_shape(e, shapes.get(i).copied().unwrap_or(CType::Int))?);
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
        let dense = (name != "n_parallel_discard")
            .then(|| dense_rows(result_shape))
            .flatten();
        let store = match result_shape {
            _ if name == "n_parallel_discard" => self.par_discard(&call, result_shape),
            _ if let Some((c, _)) = dense => format!("loft_b[loft_i] = ({c}) {call};"),
            CType::Int32 => format!("loft_b[loft_i].i = loft_w64({call});"),
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
            // Every scalar kind has its dense row (`dense_rows`).
            CType::Int | CType::Bool | CType::Enum | CType::Char | CType::Float | CType::Single => {
                return Err("a scalar par result without its dense rows".into());
            }
        };
        // @PLN203 C4 tier 2 — a struct or struct-enum record result: its type, to size the
        // result store's rows and copy each worker's record with the claims it owns.
        let record = (result_shape == CType::Ref && name != "n_parallel_discard")
            .then(|| def.returned().heap_def_nr())
            .flatten()
            .map(|r| {
                let kt = cx.data.def(r).known_type();
                (kt, u32::from(cx.stores.enum_parent_size(kt)))
            })
            .filter(|(kt, size)| *kt != u16::MAX && *size > 0);
        let threaded = vec_buffers.is_empty()
            && (name == "n_parallel_discard"
                || record.is_some()
                || matches!(
                    result_shape,
                    CType::Int
                        | CType::Bool
                        | CType::Enum
                        | CType::Char
                        | CType::Float
                        | CType::Single
                        | CType::Text
                        | CType::Void
                ))
            && shapes.iter().take(extra.len()).all(|c| {
                matches!(
                    c,
                    CType::Int
                        | CType::Bool
                        | CType::Enum
                        | CType::Char
                        | CType::Float
                        | CType::Single
                        | CType::Ref
                )
            });
        if threaded {
            return self.par_threaded(
                name,
                &input,
                esz,
                threads,
                d,
                &shapes,
                &extra,
                &buffers,
                result_shape,
                record,
                ind,
            );
        }
        let n = self.temp(CType::Int);
        self.line(ind, &format!("{n} = loft_vec_len({input});"));
        self.line(ind, "{");
        let (row_c, push) = dense.unwrap_or(("loft_par_slot", "loft_par_push"));
        self.line(
            ind + 1,
            &format!(
                "{row_c} *loft_b = {};",
                if name == "n_parallel_discard" {
                    "0".to_string()
                } else {
                    format!("{push}({n})")
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
    /// `n_parallel_discard`'s row: the worker's result dropped with what it owns — a text, the
    /// texts of a tuple, a record whose store the worker made (a pass-through of a store the
    /// caller had stays) — as a dropped call's temporary is released.  `(void) call` leaked them.
    fn par_discard(&self, call: &str, shape: CType) -> String {
        match shape {
            CType::Text => format!("{{ loft_text loft_t = {call}; loft_text_free(&loft_t); }}"),
            CType::Tuple(id) => {
                let mut texts = Vec::new();
                self.tuple_text_leaves("loft_t", id, &mut texts);
                let frees = texts.iter().fold(String::new(), |mut f, t| {
                    let _ = write!(f, " loft_text_free(&{t});");
                    f
                });
                format!(
                    "{{ {} loft_t = {call};{frees} (void) loft_t; }}",
                    crate::c::held(shape)
                )
            }
            CType::Ref => format!(
                "{{ uint64_t loft_s = loft_stores_allocated; loft_ref loft_r = {call}; \
                 if (loft_fnref_minted(loft_r, loft_s, loft_ref_null())) loft_free_ref(loft_r); }}"
            ),
            _ => format!("(void) {call};"),
        }
    }

    /// A tier-1 `par` site on threads: its operands in a context struct, its loop over a row
    /// range in a body function (both written before the unit's functions), and the site a
    /// `loft_par_run` over them.  The worker is called with the context's extras in place of
    /// the caller's, and writes only the rows of its range.
    #[expect(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "the site's parts, computed by par_queue, written in the order the unit holds them: the context, the body, the adopt step, the call"
    )]
    fn par_threaded(
        &mut self,
        name: &str,
        input: &str,
        esz: i64,
        threads: &Value,
        d: u32,
        shapes: &[CType],
        extra: &[String],
        buffers: &[String],
        result_shape: CType,
        record: Option<(u16, u32)>,
        ind: usize,
    ) -> Result<E, String> {
        let cx = self.cx;
        let id = cx.par_sites.get();
        cx.par_sites.set(id + 1);
        let fields: Vec<String> = (0..extra.len()).map(|i| format!("loft_c->x{i}")).collect();
        let WorkerCall { args, .. } = Self::worker_call(cx, d, esz, &fields)?;
        let call = format!("{}({})", c_name(cx.data, d), args.join(", "));
        let dense = (name != "n_parallel_discard")
            .then(|| dense_rows(result_shape))
            .flatten();
        let store = match result_shape {
            _ if name == "n_parallel_discard" => self.par_discard(&call, result_shape),
            _ if let Some((c, _)) = dense => format!("loft_b[loft_i] = ({c}) {call};"),
            CType::Int32 => format!("loft_b[loft_i].i = loft_w64({call});"),
            CType::Text => format!("loft_b[loft_i].t = {call};"),
            // As the in-order route: the row's own store is ADOPTED when the worker made it.
            CType::Ref => format!(
                "{{ uint64_t loft_s = loft_stores_allocated; loft_b[loft_i].r = {call}; \
                 loft_b[loft_i].adopted = (uint8_t) loft_fnref_minted(loft_b[loft_i].r, loft_s, loft_ref_null()); }}"
            ),
            _ => format!("{call};"),
        };
        let (row_c, push) = dense.unwrap_or(("loft_par_slot", "loft_par_push"));
        let mut decl =
            format!("typedef struct {{\n    loft_ref in;\n    {row_c} *b;\n    loft_ref res;\n");
        for (i, c) in shapes.iter().take(extra.len()).enumerate() {
            let _ = writeln!(decl, "    {} x{i};", crate::c::held(*c));
        }
        let _ = writeln!(
            decl,
            "}} loft_parctx{id};\nstatic void loft_parbody{id}(void *loft_vc, int64_t loft_lo, int64_t loft_hi);"
        );
        if record.is_some() {
            let _ = writeln!(
                decl,
                "static void loft_paradopt{id}(void *loft_vc, int64_t loft_lo, int64_t loft_hi, uint32_t loft_pn);"
            );
        }
        cx.gen_decls.borrow_mut().push_str(&decl);
        let mut body = format!(
            "\nstatic void loft_parbody{id}(void *loft_vc, int64_t loft_lo, int64_t loft_hi) {{\n    \
             loft_parctx{id} *loft_c = (loft_parctx{id} *) loft_vc;\n    {row_c} *loft_b = loft_c->b;\n    \
             (void) loft_b;\n    for (int64_t loft_i = loft_lo; loft_i < loft_hi; loft_i++) {{\n        \
             loft_ref loft_e = loft_vec_get(loft_c->in, {esz}, loft_i);\n        (void) loft_e;\n"
        );
        for w in buffers {
            let _ = writeln!(body, "        loft_text {w} = {{ 0, 0, 0 }};");
        }
        let _ = writeln!(body, "        {store}");
        for w in buffers {
            let _ = writeln!(body, "        loft_text_free(&{w});");
        }
        body.push_str("    }\n}\n");
        if let Some((kt, size)) = record {
            cx.describe_type(kt);
            body.push_str(&par_adopt_fn(id, kt, size));
        }
        cx.gen_bodies.borrow_mut().push_str(&body);
        let t = self.expr(threads, ind)?;
        let t = as_int(t)?;
        let n = self.temp(CType::Int);
        self.line(ind, &format!("{n} = loft_vec_len({input});"));
        self.line(ind, "{");
        let slots = if name == "n_parallel_discard" {
            "0".to_string()
        } else {
            format!("{push}({n})")
        };
        // A record site's result store, made before the workers fork (loft_par_run), released
        // with the buffer.
        let res = if let Some((_, size)) = record {
            self.line(ind + 1, "loft_ref loft_res = loft_ref_null();");
            self.line(
                ind + 1,
                &format!("loft_database(&loft_res, (uint32_t) ({n} * {size}));"),
            );
            "loft_res"
        } else {
            "{ 0xFFFF, 0, 0 }"
        };
        let mut init = format!("loft_parctx{id} loft_c = {{ {input}, {slots}, {res}");
        for x in extra {
            let _ = write!(init, ", {x}");
        }
        init.push_str(" };");
        self.line(ind + 1, &init);
        if record.is_some() {
            self.line(ind + 1, "loft_par_hold(loft_res);");
            self.line(
                ind + 1,
                &format!(
                    "loft_par_run({n}, {t}, loft_parbody{id}, &loft_c, loft_paradopt{id}, loft_res.store_nr);"
                ),
            );
        } else {
            self.line(
                ind + 1,
                &format!("loft_par_run({n}, {t}, loft_parbody{id}, &loft_c, 0, 0);"),
            );
        }
        self.line(ind, "}");
        Ok(if name == "n_parallel_discard" {
            E::new("(void) 0", K::Void)
        } else {
            E::new(n, K::Int)
        })
    }

    /// `parallel_fold(input, init, fold, threads)` (`parallel::run_parallel_fold`): the rows
    /// split into `min(threads, rows)` slices as `par_pool::parallel_workers` splits them, each
    /// folded from `init`, then the partials folded from `init` in slice order — the answer a
    /// fold that is no monoid gets on the interpreter too.
    fn par_fold(
        &mut self,
        input: &Value,
        init: &Value,
        func: &Value,
        threads: &Value,
        rest: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        if rest.iter().any(|x| !matches!(x.unspan(), Value::Int(0))) {
            return Err("a par fold with extra arguments".into());
        }
        let d = u32::try_from(Self::literal(func, "fold")?)
            .map_err(|_| "a fold out of range".to_string())?;
        let def = self.cx.data.def(d);
        if def.attributes().len() != 2 || self.cx.ctype(def.returned()) != Some(CType::Int) {
            return Err("a par fold whose function is not fold(integer, integer)".into());
        }
        let f = c_name(self.cx.data, d);
        let es = self.args(&[input, init, threads], ind)?;
        let [input, init, threads] =
            <[E; 3]>::try_from(es).map_err(|_| "a par fold without its operands".to_string())?;
        let (input, init, threads) = (as_ref(input)?, as_int(init)?, as_int(threads)?);
        let r = self.temp(CType::Int);
        self.line(ind, "{");
        self.line(
            ind + 1,
            &format!("uint64_t loft_n = (uint64_t) loft_vec_len({input});"),
        );
        self.line(
            ind + 1,
            &format!("uint64_t loft_t = (uint64_t) (int64_t) (int32_t) {threads};"),
        );
        self.line(ind + 1, "if (loft_t < 1) loft_t = 1;");
        self.line(
            ind + 1,
            "if (loft_t > (loft_n > 1 ? loft_n : 1)) loft_t = loft_n > 1 ? loft_n : 1;",
        );
        self.line(ind + 1, &format!("{r} = {init};"));
        self.line(ind + 1, "if (loft_n > 0) {");
        self.line(
            ind + 2,
            "for (uint64_t loft_k = 0; loft_k < loft_t; loft_k++) {",
        );
        self.line(ind + 3, &format!("int64_t loft_p = {init};"));
        self.line(
            ind + 3,
            "for (uint64_t loft_i = loft_k * loft_n / loft_t; loft_i < (loft_k + 1) * loft_n / loft_t; loft_i++)",
        );
        self.line(
            ind + 4,
            &format!(
                "loft_p = {f}(loft_p, loft_i64_bits(loft_par_arg(loft_vec_get({input}, 8, (int64_t) loft_i), 8)));"
            ),
        );
        self.line(ind + 3, &format!("{r} = {f}({r}, loft_p);"));
        self.line(ind + 2, "}");
        self.line(ind + 1, "}");
        self.line(ind, "}");
        Ok(E::new(r, K::Int))
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
                    CType::Int32 => format!("loft_n32(loft_i64_bits({bits}))"),
                    CType::Float => format!("loft_f64({bits})"),
                    CType::Single => format!("loft_f32((uint32_t) {bits})"),
                    CType::Char => format!("(uint32_t) {bits}"),
                    CType::Bool | CType::Enum => format!("(uint8_t) {bits}"),
                    // A function row is its definition number.
                    CType::Fn => format!("loft_fn_of((int64_t) (uint32_t) {bits})"),
                    CType::Tuple(_) => stored_value(cx, "loft_e", a.typedef.base())
                        .ok_or("a par worker over rows of tuples holding references")?,
                    CType::Void => return Err("a par worker over rows of nothing".into()),
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

/// The value of type `tp` stored at place `at` (a `loft_ref` expression), as its C shape: a
/// scalar through its field getter — a narrow integer through its kind's, with its `min` — and
/// a tuple member by member at the offsets of its `__tuple<…>` record, a text lent.  `None` for
/// a kind held by reference (a record).
fn stored_value(cx: &crate::c::Cx<'_>, at: &str, tp: &Type) -> Option<String> {
    Some(match tp.base() {
        Type::Integer(_) => match crate::data::NarrowSlot::of_type(tp) {
            Some(n) => {
                let f = match n.get_op() {
                    "OpGetByte" => "loft_get_byte",
                    "OpGetByteNullable" => "loft_get_byte_nullable",
                    "OpGetShort" => "loft_get_short",
                    _ => "loft_get_short_raw",
                };
                format!("{f}({at}, 0, INT64_C({}))", n.min)
            }
            None => format!("loft_get_int({at}, 0)"),
        },
        Type::Float => format!("loft_get_float({at}, 0)"),
        Type::Single => format!("loft_get_single({at}, 0)"),
        Type::Boolean => format!("loft_get_bool({at}, 0)"),
        Type::Character => format!("loft_get_char({at}, 0)"),
        Type::Enum(_, false, _) => format!("loft_get_enum({at}, 0)"),
        // A tuple parameter's text is the caller's — the callee copies one it writes — so the
        // row lends the stored bytes.
        Type::Text(_) => format!("loft_text_lend(loft_get_text({at}, 0))"),
        Type::Tuple(elems) => {
            let offsets = crate::data::stored_tuple_offsets(cx.data, cx.stores, elems)?;
            let CType::Tuple(id) = cx.ctype(tp)? else {
                return None;
            };
            let members = elems
                .iter()
                .zip(&offsets)
                .map(|(e, o)| stored_value(cx, &format!("loft_field({at}, {o})"), e.base()))
                .collect::<Option<Vec<_>>>()?;
            format!("((loft_tup{id}) {{ {} }})", members.join(", "))
        }
        _ => return None,
    })
}

/// `@PLN203 C4` tier 2 — a record site's adopt step (`loft_par_adopt`): a row whose record lies in
/// a store the worker made is copied into the result store, its bytes and the claims it owns; a
/// pass-through of a store the caller had, or a null, stays as it is.
fn par_adopt_fn(id: u32, kt: u16, size: u32) -> String {
    format!(
        "\nstatic void loft_paradopt{id}(void *loft_vc, int64_t loft_lo, int64_t loft_hi, uint32_t loft_pn) {{\n    \
                 loft_parctx{id} *loft_c = (loft_parctx{id} *) loft_vc;\n    loft_par_slot *loft_b = loft_c->b;\n    \
                 for (int64_t loft_i = loft_lo; loft_i < loft_hi; loft_i++) {{\n        \
                 loft_ref loft_r = loft_b[loft_i].r;\n        \
                 if (loft_r.store_nr == 0xFFFF || loft_r.rec == 0 || loft_r.store_nr < loft_pn) {{ loft_b[loft_i].adopted = 0; continue; }}\n        \
                 loft_ref loft_d = {{ loft_c->res.store_nr, loft_c->res.rec, loft_c->res.pos + (uint32_t) loft_i * {size}u }};\n        \
                 loft_copy_bytes(loft_d, 0, loft_r, 0, {size});\n        \
                 loft_copy_claims(loft_types, {kt}, loft_r, loft_d);\n        \
                 loft_b[loft_i].r = loft_d;\n        \
                 loft_b[loft_i].adopted = 0;\n    }}\n}}\n"
    )
}
