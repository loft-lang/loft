// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Text / format / buffer Op emitters — the refvar-dependent family.
//!
//! Relocated VERBATIM from the `output_call_inner` match (`src/generation/
//! dispatch.rs`) to retire that match.  These ops delegate to the
//! `Output::format_*` / `append_*` / `clear_*` helpers (`src/generation/text.rs`)
//! and depend on the @P283 refvar→`Stack` rewrite: the ORIGINAL op name
//! (`ctx.def_fn.name`) sets each helper's `stack` flag, while the rewritten
//! `dispatch` name selects the case.  This single emitter reproduces that
//! rewrite internally + the whole sub-match, and is registered for every name
//! below (base + `Stack` variants) so the registry-first guard routes them here.

use super::{EmitCtx, OpEmitter};
use crate::data::{Type, Value};
use std::io;

pub struct TextDispatchEmitter;

impl OpEmitter for TextDispatchEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let name = ctx.def_fn.name().to_string();
        // @P283 — mirror src/state/codegen.rs: a first arg that is a Var of type
        // RefVar(Text) (a `&mut String` work-buffer) rewrites the op to its
        // `Stack` variant for case selection.  `@FR-N-Shape`: a `&text?` link is the same
        // shape — peeled, or its raw-pointer append lands outside the `unsafe` block (loft#1836).
        let refvar_text_first = matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if {
            matches!(
                ctx.output.data.def(ctx.output.def_nr).variables().tp(*v),
                Type::RefVar(inner) if matches!(inner.base(), Type::Text(_))
            )
        });
        let dispatch: &str = if refvar_text_first {
            super::refvar_text_stack_variant(&name).unwrap_or(&name)
        } else {
            &name
        };
        // loft#1371 — a `&text` LOCAL link holds `*mut String`: a RAW pointer, so the source
        // local stays readable while the link is alive (a `&mut String` would freeze it, and
        // loft allows `println("{c}")` beside a live `pc = &c`).  A `&text` PARAMETER holds
        // the `&mut String` and needs no block.  Every `Stack` variant writes THROUGH the
        // destination, so the wrapper goes here, once, rather than per op — a variant added
        // to `refvar_text_stack_variant` cannot then arrive without its dereference.
        let writes_through_dest = dispatch.starts_with("OpAppendStack")
            || dispatch.starts_with("OpClearStack")
            || dispatch.starts_with("OpFormatStack");
        let raw_link = refvar_text_first
            && writes_through_dest
            && matches!(args.first().map(Value::unspan), Some(Value::Var(v))
                if !ctx.output.data.def(ctx.output.def_nr).variables().is_argument(*v));
        if raw_link {
            write!(ctx.w, "unsafe {{ ")?;
        }
        let emitted = match dispatch {
            "OpFormatInt" | "OpFormatStackInt" => {
                ctx.output
                    .format_long(&mut *ctx.w, args, name == "OpFormatStackInt")
            }
            "OpFormatFloat" | "OpFormatStackFloat" => {
                ctx.output
                    .format_float(&mut *ctx.w, args, name == "OpFormatStackFloat")
            }
            "OpFormatSingle" | "OpFormatStackSingle" => {
                ctx.output
                    .format_single(&mut *ctx.w, args, name == "OpFormatStackSingle")
            }
            "OpFormatText" | "OpFormatStackText" => ctx.output.format_text(&mut *ctx.w, args),
            "OpAppendText" => ctx.output.append_text(&mut *ctx.w, args, false),
            "OpAppendStackText" => ctx.output.append_text(&mut *ctx.w, args, true),
            "OpAppendCharacter" | "OpAppendStackCharacter" => {
                ctx.output.append_character(&mut *ctx.w, args)
            }
            "OpClearStackText" | "OpClearText" => ctx.output.clear_stack_text(&mut *ctx.w, args),
            // @PLN157 § V-u (`@FR-R-RetAdopt`) — the delivery pair inside an adopted
            // function's `one_buffer_vec_copy` block: the result local IS the buffer.
            "OpClearVector" | "OpAppendVector"
                if ctx.output.in_adopt_delivery > 0
                    && matches!(args.first().map(crate::data::Value::unspan), Some(crate::data::Value::Var(b))
                        if ctx.output.ret_adopt.is_some_and(|a| a.buf == *b)) =>
            {
                write!(ctx.w, "()")
            }
            "OpClearVector" => ctx.output.clear_vector(&mut *ctx.w, args),
            "OpAppendVector" => ctx.output.append_vector(&mut *ctx.w, args),
            // `@FR-R-TextBorrow`'s checking form — the walk's release of a borrowed loop
            // variable is where the borrow ends, so under LOFT_HOIST_VERIFY=1 the element is
            // read again there and compared with what the iteration held.  The release
            // itself emits nothing: a `&str` owns nothing.
            "OpFreeText"
                if ctx.output.hoist_verify
                    && let [arg] = args
                    && let Value::Var(p) = arg.unspan()
                    && let Some(read) = ctx
                        .output
                        .borrowed_text_locals
                        .get(p)
                        .and_then(|b| b.read.clone()) =>
            {
                let name = super::super::sanitize(
                    ctx.output.data.def(ctx.output.def_nr).variables().name(*p),
                );
                write!(ctx.w, "vector::text_borrow_verify(var_{name}, ")?;
                ctx.output.output_code_inner(&mut *ctx.w, &read)?;
                write!(ctx.w, ")")
            }
            "OpFreeText" | "OpCreateStack" => Ok(()),
            "OpFormatDatabase" | "OpFormatStackDatabase" => {
                // OpFormatDatabase takes a &mut String as the output buffer.
                if let [work_val, record_val, tp_val, fmt_val] = args {
                    write!(ctx.w, "OpFormatDatabase(cell,&mut ")?;
                    // work_val is Var(nr) — strip the leading & that emit() adds.
                    if let Value::Var(nr) = work_val {
                        let nm = super::super::sanitize(
                            ctx.output.data.def(ctx.output.def_nr).variables().name(*nr),
                        );
                        write!(ctx.w, "var_{nm}")?;
                    } else {
                        ctx.emit(work_val)?;
                    }
                    write!(ctx.w, ", ")?;
                    ctx.emit(record_val)?;
                    write!(ctx.w, ", ")?;
                    ctx.emit_i32_slot(tp_val)?;
                    write!(ctx.w, ", ")?;
                    ctx.emit_i32_slot(fmt_val)?;
                    write!(ctx.w, ")")?;
                }
                Ok(())
            }
            // Registered only for the names above, so this is unreachable; fall
            // back to the template path for safety.
            _ => super::default::DefaultEmitter.emit(ctx, args),
        };
        if raw_link {
            write!(ctx.w, " }}")?;
        }
        emitted
    }
}

/// `@FR-R-FoldCompare` — a text predicate (`starts_with`, `ends_with`, `==`, `!=`) one of
/// whose operands is a case fold the program builds only to compare: the fold's argument
/// and the other operand go to `codegen_runtime::fold_compare`, which answers byte by byte
/// while both are ASCII and builds the fold only where they are not.  Any other shape —
/// no fold operand, or the rule off (`LOFT_NO_FOLD_COMPARE`) — is the template.
pub struct FoldCompareEmitter {
    /// The predicate, a `codegen_runtime::fold_op` constant.
    pub op: u8,
}

/// The fold a predicate operand is, when it is one the predicate is its only reader of: a
/// bare `to_lowercase` / `to_uppercase` call, or the parser's `synth text dest` block that
/// fills a work buffer with one and answers that buffer.  Answers the fold's argument and
/// whether it is the upper fold.  The fallback is `None` — the operand is built as written,
/// which costs the rewrite and never a value.
pub(crate) fn case_fold_operand<'a>(
    data: &crate::data::Data,
    v: &'a Value,
) -> Option<(&'a Value, bool)> {
    let call = match v.unspan() {
        Value::Block(bl) if bl.name == "synth text dest" && bl.operators.len() == 2 => {
            let (Value::Set(w, rhs), Value::Var(r)) =
                (bl.operators[0].unspan(), bl.operators[1].unspan())
            else {
                return None;
            };
            if w != r {
                return None;
            }
            rhs.unspan()
        }
        other => other,
    };
    let Value::Call(d, args) = call else {
        return None;
    };
    let [arg] = args.as_slice() else {
        return None;
    };
    if (*d as usize) >= data.definitions.len() {
        return None;
    }
    match data.def(*d).name() {
        "t_4text_to_lowercase" => Some((arg, false)),
        "t_4text_to_uppercase" => Some((arg, true)),
        _ => None,
    }
}

pub(crate) fn fold_compare_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_NO_FOLD_COMPARE").map_or(true, |v| v == "0"))
}

impl OpEmitter for FoldCompareEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(site) = ctx.output.fold_compare_site(args) {
            crate::rewrite_census::fired("R-FoldCompare", 1);
            if std::env::var_os("LOFT_TRACE_FOLD_COMPARE").is_some() {
                eprintln!(
                    "[fold-compare] {}: `{}` over the {} fold",
                    ctx.output.data.def(ctx.output.def_nr).name(),
                    ctx.def_fn.name(),
                    if site.upper { "upper" } else { "lower" }
                );
            }
            let (op, fold_right, upper) = (self.op, site.fold_right, site.upper);
            write!(
                ctx.w,
                "loft::codegen_runtime::fold_compare({op}, {fold_right}, {upper}, &*("
            )?;
            ctx.emit(site.arg)?;
            write!(ctx.w, "), &*(")?;
            ctx.emit(site.other)?;
            return write!(ctx.w, "))");
        }
        super::default::DefaultEmitter.emit(ctx, args)
    }
}

/// A `@FR-R-FoldCompare` site: the fold's argument, the other operand, and where the fold
/// stood.
pub(crate) struct FoldSite<'a> {
    pub arg: &'a Value,
    pub other: &'a Value,
    pub fold_right: bool,
    pub upper: bool,
}

impl crate::generation::Output<'_> {
    /// The `@FR-R-FoldCompare` site a predicate's `args` are, when the rule is on, one
    /// operand is a fold the predicate alone reads, and neither the fold's argument nor the
    /// other operand holds work a `let _pre_N` would lift: then the emitter reads both
    /// inline, in either order, and the fold is never built.  ONE question for the two
    /// sides — `pre_eval` must not lift the fold the emitter does not read, or it is built
    /// in front of the statement for nothing.  `None` keeps the template and its lifts.
    pub(crate) fn fold_compare_site<'a>(&self, args: &'a [Value]) -> Option<FoldSite<'a>> {
        if !fold_compare_on() {
            return None;
        }
        let [left, right] = args else {
            return None;
        };
        for (fold_right, (folded, other)) in [(false, (left, right)), (true, (right, left))] {
            if let Some((arg, upper)) = case_fold_operand(self.data, folded)
                && self.plain_operand(arg)
                && self.plain_operand(other)
            {
                return Some(FoldSite {
                    arg,
                    other,
                    fold_right,
                    upper,
                });
            }
        }
        None
    }
}
