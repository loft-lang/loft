// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The C backend (@PLN200 B5, @PLN91 strand 5): loft programs written as C99 and built with
//! `cc`.  It is a part of its own, beside the Rust writer (`generation`) and the interpreter:
//! it reads the IR, the analyses in `opt` and the plans in `lower`, and names neither sibling
//! (`@C153`, `scripts/crate_layers.py`).
//!
//! It starts PLAIN (the owner's decision in the plan): every value takes the general form,
//! and no per-function decision is shared yet.  It carries what its runtime (`loft_rt.h`)
//! carries — integers, booleans, text with its formatting, the operators over them, control
//! flow, calls and returns — and refuses the rest with a [`Refusal`] that names the function
//! and the construct.  Values follow the interpreter's rules; the runtime ports
//! `src/ops.rs`.  What the C does NOT do yet is report: a division by zero the interpreter
//! raises as a recoverable error is a null here and nothing more, because how a C program
//! reaches loft's runtime is @PLN91's decision.

mod body;

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use std::fmt::Write as _;

/// The C runtime every unit starts with.
const RUNTIME: &str = include_str!("loft_rt.h");

/// Why a function cannot be written as C yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The loft name of the function.
    pub function: String,
    /// The construct the C backend does not carry, in a few words.
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.function, self.why)
    }
}

/// The C shapes a loft value takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CType {
    /// A 64-bit integer, `int64_t`; null is `LOFT_NULL`.
    Int,
    /// A boolean, stored as `uint8_t` (0, 1, `LOFT_BOOL_NULL`).
    Bool,
    /// A text: an owned `loft_text` where it is held, a `loft_str` view where it is passed.
    Text,
    /// A reference into a store — a struct, a vector — as `loft_ref`; null has `rec == 0`.
    Ref,
    /// A tuple: the C struct [`Cx::tuple_shapes`] numbers, held and passed by value.
    Tuple(u32),
    /// No value.
    Void,
}

/// The C type a local or a result of shape `c` is held in.
pub(crate) fn held(c: CType) -> String {
    match c {
        CType::Int => "int64_t".into(),
        CType::Bool => "uint8_t".into(),
        CType::Text => "loft_text".into(),
        CType::Ref => "loft_ref".into(),
        CType::Tuple(id) => format!("loft_tup{id}"),
        CType::Void => "void".into(),
    }
}

/// The C type a parameter of shape `c` is passed as.
pub(crate) fn passed(c: CType) -> String {
    match c {
        CType::Text => "loft_str".into(),
        other => held(other),
    }
}

/// What every function of a unit shares: the program, its stores' layouts and the tuple
/// structs the unit's functions name.
pub(crate) struct Cx<'a> {
    pub(crate) data: &'a Data,
    pub(crate) stores: &'a Stores,
    tuples: std::cell::RefCell<Vec<Vec<CType>>>,
}

impl<'a> Cx<'a> {
    fn new(data: &'a Data, stores: &'a Stores) -> Cx<'a> {
        Cx {
            data,
            stores,
            tuples: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// The C shape of `tp`, nullable or not — a null is held in the same shape — or `None`.
    pub(crate) fn ctype(&self, tp: &Type) -> Option<CType> {
        match tp.base() {
            Type::Integer(spec)
                if spec.forced_size.is_none()
                    && spec.min == i32::MIN + 1
                    && (spec.max == i64::from(i32::MAX) || spec.max == i64::from(u32::MAX)) =>
            {
                Some(CType::Int)
            }
            Type::Boolean => Some(CType::Bool),
            Type::Text(_) => Some(CType::Text),
            Type::Reference(..) | Type::Vector(..) => Some(CType::Ref),
            Type::Tuple(elems) => {
                // A text inside a tuple would be copied with the tuple and freed twice.
                let shapes = elems
                    .iter()
                    .map(|e| {
                        self.ctype(e)
                            .filter(|c| !matches!(c, CType::Void | CType::Text))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(CType::Tuple(self.tuple_id(shapes)))
            }
            Type::Void => Some(CType::Void),
            _ => None,
        }
    }

    /// The number of the tuple struct with these element shapes, made on first use.
    pub(crate) fn tuple_id(&self, shapes: Vec<CType>) -> u32 {
        let mut t = self.tuples.borrow_mut();
        if let Some(i) = t.iter().position(|s| *s == shapes) {
            return i as u32;
        }
        t.push(shapes);
        (t.len() - 1) as u32
    }

    /// The element shapes of tuple struct `id`.
    pub(crate) fn tuple_shapes(&self, id: u32) -> Vec<CType> {
        self.tuples.borrow()[id as usize].clone()
    }

    /// The initial value of a local of shape `c`: its null.
    pub(crate) fn initial(&self, c: CType) -> String {
        match c {
            CType::Int => "LOFT_NULL".into(),
            CType::Bool => "LOFT_BOOL_NULL".into(),
            CType::Text => "{ 0, 0, 0 }".into(),
            CType::Ref => "{ 0xFFFF, 0, 0 }".into(),
            CType::Tuple(id) => {
                let fields: Vec<String> = self
                    .tuple_shapes(id)
                    .into_iter()
                    .map(|e| self.initial(e))
                    .collect();
                format!("{{ {} }}", fields.join(", "))
            }
            CType::Void => String::new(),
        }
    }

    /// What record type `tp` owns past its own bytes — a text, a collection — named for a
    /// refusal, or `None` when copying its bytes copies all of it.
    pub(crate) fn record_owns(&self, tp: u16) -> Option<String> {
        use crate::database::Parts;
        let t = self.stores.types.get(tp as usize)?;
        let fields = match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields,
            Parts::Base if t.name == "text" => return Some("text".into()),
            Parts::Base
            | Parts::Byte(..)
            | Parts::Short(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::ShortRaw(..)
            | Parts::Enum(_) => return None,
            other => {
                let kind = format!("{other:?}");
                return Some(
                    kind.split(['(', ' '])
                        .next()
                        .unwrap_or("collection")
                        .to_lowercase(),
                );
            }
        };
        fields.iter().find_map(|f| self.record_owns(f.content))
    }

    /// The tuple structs, each after the ones it holds.
    fn typedefs(&self) -> String {
        let mut out = String::new();
        for (id, shapes) in self.tuples.borrow().iter().enumerate() {
            let fields: Vec<String> = shapes
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} f{i};", held(*c)))
                .collect();
            let _ = writeln!(
                out,
                "typedef struct {{ {} }} loft_tup{id};",
                fields.join(" ")
            );
        }
        out
    }
}

/// `tp` in a word, for a refusal: its variant's name.
pub(crate) fn type_word(tp: &Type) -> String {
    let full = format!("{:?}", tp.base());
    full.split(['(', ' ', '{'])
        .next()
        .unwrap_or("type")
        .to_string()
}

/// The C name of definition `d`: the identifier every compiled target gives it.
#[must_use]
pub fn c_name(data: &Data, d: u32) -> String {
    let dups = crate::opt::names::duplicate_fn_names(data);
    crate::opt::names::disambiguated_fn_ident(&dups, data.def(d))
}

/// The C runtime function that implements native `name` (a stored `n_…` name), if any.
pub(crate) fn native_c(name: &str) -> Option<&'static str> {
    match name {
        "n_assert" => Some("loft_assert"),
        _ => None,
    }
}

/// Is `d` an operator, which a call site writes rather than a function it calls?
pub(crate) fn is_operator(data: &Data, d: u32) -> bool {
    data.def(d).name().starts_with("Op")
}

/// C99 for `roots` and every loft function they reach: the runtime, a prototype per function
/// and each body.  A function or construct the backend does not carry refuses the whole unit.
///
/// # Errors
/// The first [`Refusal`], the roots' own first.
pub fn emit_unit(data: &Data, stores: &Stores, roots: &[u32]) -> Result<String, Refusal> {
    let mut fns: Vec<u32> = crate::opt::names::reachable_functions(data, roots)
        .into_iter()
        .filter(|&d| {
            matches!(data.def(d).def_type(), DefType::Function)
                && !is_operator(data, d)
                && native_c(data.def(d).name()).is_none()
        })
        .collect();
    // The roots first, so a refusal names the function asked for before one it reaches.
    fns.sort_unstable_by_key(|d| (!roots.contains(d), *d));
    let cx = Cx::new(data, stores);
    let mut protos = String::new();
    let mut bodies = String::new();
    for &d in &fns {
        let (sig, body) = function(&cx, d).map_err(|why| Refusal {
            function: data.def(d).original_name().clone(),
            why,
        })?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
    }
    let mut out = String::from(RUNTIME);
    out.push('\n');
    out.push_str(&cx.typedefs());
    out.push_str(&protos);
    out.push_str(&bodies);
    Ok(out)
}

/// C99 for the whole program: [`emit_unit`] from `main`, and the C `main` that calls it.
///
/// # Errors
/// A program without a `main` that takes nothing, or the first [`Refusal`].
pub fn emit_program(data: &Data, stores: &Stores) -> Result<String, Refusal> {
    let main = data.def_nr("n_main");
    if main >= data.definitions() {
        return emit_tests(data, stores);
    }
    if !data.def(main).attributes().is_empty() {
        return Err(Refusal {
            function: "main".into(),
            why: "a main that takes arguments".into(),
        });
    }
    let mut out = emit_unit(data, stores, &[main])?;
    let _ = write!(
        out,
        "\nint main(void) {{\n    {}();\n    return 0;\n}}\n",
        c_name(data, main)
    );
    Ok(out)
}

/// The signature and the body of function `d`.
fn function(cx: &Cx, d: u32) -> Result<(String, String), String> {
    let data = cx.data;
    let def = data.def(d);
    if def.code() == &Value::Null {
        return Err(format!("native {}", def.original_name()));
    }
    let ret = cx
        .ctype(def.returned())
        .ok_or_else(|| format!("returns {}", type_word(def.returned())))?;
    let vars = def.variables();
    let mut params = Vec::new();
    let mut shapes = Vec::new();
    for (i, a) in def.attributes().iter().enumerate() {
        let c = cx
            .ctype(&a.typedef)
            .filter(|c| *c != CType::Void)
            .ok_or_else(|| format!("parameter of type {}", type_word(&a.typedef)))?;
        shapes.push(c);
        params.push(format!(
            "{} {}",
            passed(c),
            body::var_name(vars.name(i as u16), i as u16)
        ));
    }
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let sig = format!("static {} {}({params})", held(ret), c_name(data, d));
    for v in def.attributes().len() as u16..vars.count() {
        let c = cx
            .ctype(vars.tp(v))
            .filter(|c| *c != CType::Void)
            .ok_or_else(|| format!("local of type {}", type_word(vars.tp(v))))?;
        shapes.push(c);
    }
    let body = body::FnWriter::new(cx, d, shapes, ret).write()?;
    Ok((sig, body))
}

/// The program's tests, the way `loft --tests` picks them: its own zero-parameter functions,
/// only the `test_*` ones when it names any.
fn test_functions(data: &Data) -> Vec<u32> {
    let mut fns: Vec<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), DefType::Function)
                && def.source == crate::data::MAIN_SOURCE
                && def.name().starts_with("n_")
                && !def.name().starts_with("n___lambda_")
                && def.attributes().is_empty()
                && !matches!(def.returned(), Type::Iterator(..))
        })
        .collect();
    if fns
        .iter()
        .any(|&d| data.def(d).name().starts_with("n_test_"))
    {
        fns.retain(|&d| data.def(d).name().starts_with("n_test_"));
    }
    fns
}

/// A program without `main` is run as its tests: a C `main` that runs each, catches a failed
/// assert, and exits 1 when any test failed — `loft --tests`'s exit status.
fn emit_tests(data: &Data, stores: &Stores) -> Result<String, Refusal> {
    let tests = test_functions(data);
    if tests.is_empty() {
        return Err(Refusal {
            function: "main".into(),
            why: "neither a main nor a test".into(),
        });
    }
    let mut out = emit_unit(data, stores, &tests)?;
    out.push_str("\nint main(void) {\n    int failed = 0;\n    loft_in_test = 1;\n");
    for &d in &tests {
        let _ = writeln!(
            out,
            "    if (setjmp(loft_test_jmp) == 0) {}(); else failed++;",
            c_name(data, d)
        );
    }
    out.push_str("    return failed ? 1 : 0;\n}\n");
    Ok(out)
}
