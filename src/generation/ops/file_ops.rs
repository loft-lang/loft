// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! A file read the call site can type statically (`@FR-R-TypedRead`).
//!
//! `bf#read(2) as i16` lowers to `OpReadFile(bf, OpCreateStack(t), 2, i16)` into an
//! `integer` local.  The generic runtime read asks, on every call, whether the type is text
//! and which width and sign it decodes to, and copies the bytes through a slice on the stack;
//! all three are facts this site has.  [`ReadFileEmitter`] renders the same call and routes
//! it to `OpReadFileInt::<W, SIGNED>`, whose parameters are `OpReadFile`'s own.  Every other
//! read — text, a float, a vector, a byte count that is not the type's width, a value slot
//! that is not an `i64` local — keeps `OpReadFile`.

use super::default::DefaultEmitter;
use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use crate::database::Parts;
use crate::generation::Output;
use std::io;

pub struct ReadFileEmitter;

impl OpEmitter for ReadFileEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let shape = crate::keys::typed_read_enabled()
            .then(|| int_read_shape(ctx.output, args))
            .flatten();
        let Some((width, signed)) = shape else {
            return DefaultEmitter.emit(ctx, args);
        };
        crate::rewrite_census::fired("R-TypedRead", 1);
        let mut buf: Vec<u8> = Vec::new();
        DefaultEmitter.emit(
            &mut EmitCtx {
                w: &mut buf,
                def_fn: ctx.def_fn,
                output: &mut *ctx.output,
            },
            args,
        )?;
        let call = String::from_utf8_lossy(&buf);
        let callee = "OpReadFile(";
        match call.find(callee) {
            Some(at) => write!(
                ctx.w,
                "{}OpReadFileInt::<{width}, {signed}>({}",
                &call[..at],
                &call[at + callee.len()..]
            ),
            None => ctx.w.write_all(&buf),
        }
    }
}

/// `(width, signed)` when the read lands a fixed-width integer whole in an `i64` local, else
/// `None`.  The width and sign are the generic decode's (`FileVal for i64`): `integer` and
/// `long` read 8 bytes signed, a byte or short type takes its sign from its range, a 4-byte
/// type reads signed; `boolean` and `character` keep their own arms there and are not taken.
fn int_read_shape(out: &Output, args: &[Value]) -> Option<(usize, bool)> {
    let [_, val, bytes, tp] = args else {
        return None;
    };
    let Value::Call(d, inner) = val.unspan() else {
        return None;
    };
    if out.data.def(*d).name() != "OpCreateStack" {
        return None;
    }
    let [slot] = inner.as_slice() else {
        return None;
    };
    let Value::Var(v) = slot.unspan() else {
        return None;
    };
    let vars = out.data.def(out.def_nr).variables();
    if out.local_rust_type(*v, vars.tp(*v)) != "i64" {
        return None;
    }
    let n = match bytes.unspan() {
        Value::Int(n) => i64::from(*n),
        Value::Long(n) => *n,
        _ => return None,
    };
    let Value::Int(tp) = tp.unspan() else {
        return None;
    };
    let (width, signed) = match *tp {
        0 | 1 => (8, true),
        4 | 6 => return None,
        t => match &out.stores.types.get(usize::try_from(t).ok()?)?.parts {
            Parts::Byte(from, _) => (1, *from < 0),
            Parts::Short(from, _) | Parts::ShortRaw(from, _) => (2, *from < 0),
            Parts::Int(_, _) => (4, true),
            _ => return None,
        },
    };
    (n == width as i64).then_some((width, signed))
}
