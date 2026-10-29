// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The browser host imports: which `#native` functions a page's module imports from the
//! `loft_gl` module the page's script supplies (`doc/loft-gl-wasm.js`), and the wasm signature
//! each is imported with.  One contract for both writers of a browser module — the Rust writer
//! renders it as `extern` declarations (`generation::emit_file_header`), the C writer as
//! `import_module` declarations (`c::browser_import_native`) — so a page hosts the module
//! either one wrote.

use crate::data::{Data, Type};
use std::collections::HashSet;

/// A vector argument's element as the host reads it: the width of the vector's STORAGE
/// stride, not its value range (a plain `vector<integer>` is stored at 8 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elem {
    F32,
    F64,
    U8,
    U32,
    I16,
    I32,
    I64,
}

impl Elem {
    #[must_use]
    pub fn of(tp: &Type) -> Elem {
        match tp.base() {
            Type::Single => Elem::F32,
            Type::Float => Elem::F64,
            Type::Boolean => Elem::U8,
            Type::Character => Elem::U32,
            Type::Integer(s) => match s.vector_narrow_width(false) {
                Some(1) => Elem::U8,
                Some(2) => Elem::I16,
                Some(4) => Elem::I32,
                _ => Elem::I64,
            },
            // A struct or enum element: opaque bytes.
            _ => Elem::U8,
        }
    }

    #[must_use]
    pub fn rust(self) -> &'static str {
        match self {
            Elem::F32 => "f32",
            Elem::F64 => "f64",
            Elem::U8 => "u8",
            Elem::U32 => "u32",
            Elem::I16 => "i16",
            Elem::I32 => "i32",
            Elem::I64 => "i64",
        }
    }

    #[must_use]
    pub fn c(self) -> &'static str {
        match self {
            Elem::F32 => "float",
            Elem::F64 => "double",
            Elem::U8 => "uint8_t",
            Elem::U32 => "uint32_t",
            Elem::I16 => "int16_t",
            Elem::I32 => "int32_t",
            Elem::I64 => "int64_t",
        }
    }
}

/// A scalar as the host passes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    I64,
    I32,
    F64,
    F32,
    /// A boolean, in the `u8` storage form.
    U8,
}

impl Scalar {
    #[must_use]
    pub fn rust(self) -> &'static str {
        match self {
            Scalar::I64 => "i64",
            Scalar::I32 => "i32",
            Scalar::F64 => "f64",
            Scalar::F32 => "f32",
            Scalar::U8 => "u8",
        }
    }

    #[must_use]
    pub fn c(self) -> &'static str {
        match self {
            Scalar::I64 => "int64_t",
            Scalar::I32 => "int32_t",
            Scalar::F64 => "double",
            Scalar::F32 => "float",
            Scalar::U8 => "uint8_t",
        }
    }
}

/// An argument: a text as its bytes and length, a vector as its elements and count, or a
/// scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg {
    Text,
    Vector(Elem),
    Scalar(Scalar),
}

/// One host import: the `#native` symbol it is imported as, its arguments by loft name, and
/// its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostImport {
    pub symbol: String,
    pub args: Vec<(String, Arg)>,
    pub ret: Option<Scalar>,
}

/// The host import definition `d_nr` is, or `None` when the page does not supply it: no
/// `#native` symbol; a symbol a library's wasm bridge serves (`[wasm.bridge].routes`); a heap
/// answer, or a heap argument other than a vector, which a host import cannot carry; or a
/// REACHABLE native whose symbol is its own function name, or that answers a text — the
/// shapes the page has no import for, which each writer refuses or stubs by name.
#[must_use]
pub fn host_import(data: &Data, d_nr: u32, reachable: &HashSet<u32>) -> Option<HostImport> {
    let def = data.def(d_nr);
    if def.native().is_empty() || data.wasm_bridge_routes.contains_key(def.native()) {
        return None;
    }
    let heap_answer = matches!(
        def.returned().base(),
        Type::Vector(_, _) | Type::Reference(_, _)
    );
    let heap_arg = def.attributes().iter().any(|a| {
        !a.name.starts_with("__")
            && a.typedef.heap_dep().is_some()
            && !matches!(a.typedef.base(), Type::Vector(_, _))
    });
    if heap_answer || heap_arg {
        return None;
    }
    if reachable.contains(&d_nr)
        && (def.native() == def.name() || matches!(def.returned().base(), Type::Text(_)))
    {
        return None;
    }
    let args = def
        .attributes()
        .iter()
        .filter(|a| !a.name.starts_with("__"))
        .map(|a| {
            let arg = match a.typedef.base() {
                Type::Text(_) => Arg::Text,
                Type::Vector(elem, _) => Arg::Vector(Elem::of(elem)),
                Type::Integer(s) if s.is_wide() => Arg::Scalar(Scalar::I64),
                Type::Float => Arg::Scalar(Scalar::F64),
                Type::Single => Arg::Scalar(Scalar::F32),
                Type::Boolean => Arg::Scalar(Scalar::U8),
                _ => Arg::Scalar(Scalar::I32),
            };
            (a.name.clone(), arg)
        })
        .collect();
    let ret = match def.returned().base() {
        Type::Void => None,
        Type::Integer(s) if s.is_wide() => Some(Scalar::I64),
        Type::Float => Some(Scalar::F64),
        Type::Single => Some(Scalar::F32),
        Type::Boolean => Some(Scalar::U8),
        _ => Some(Scalar::I32),
    };
    Some(HostImport {
        symbol: def.native().to_string(),
        args,
        ret,
    })
}
