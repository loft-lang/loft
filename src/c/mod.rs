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
mod show;

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
    /// A plain enum value, `uint8_t`; 255 is null, and so is 0 where a value is read.
    Enum,
    /// A float, `double`; NaN is null.
    Float,
    /// A single, `float`; NaN is null.
    Single,
    /// A character, its code point as `uint32_t`; 0 is null.
    Char,
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
        CType::Enum => "uint8_t".into(),
        CType::Float => "double".into(),
        CType::Single => "float".into(),
        CType::Char => "uint32_t".into(),
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
    /// Every type the unit's deep copies walk, by type number (`loft_types`).
    copy_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every plain enum type whose value names the unit spells, by type number.
    enum_names: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// A copy of the stores the default walk runs in, made on first use.
    scratch: std::cell::RefCell<Option<Stores>>,
    /// Every record type's default bytes the unit names, by type number.
    defaults: std::cell::RefCell<std::collections::BTreeMap<u16, Vec<u8>>>,
    /// Every type the unit renders as text (`loft_show{tp}`), by type number.
    show_types: std::cell::RefCell<std::collections::BTreeSet<u16>>,
    /// Every compile-time constant the unit reads (`loft_const{d}`), by definition number.
    consts: std::cell::RefCell<std::collections::BTreeSet<u32>>,
}

impl<'a> Cx<'a> {
    fn new(data: &'a Data, stores: &'a Stores) -> Cx<'a> {
        Cx {
            data,
            stores,
            tuples: std::cell::RefCell::new(Vec::new()),
            copy_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            enum_names: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            scratch: std::cell::RefCell::new(None),
            defaults: std::cell::RefCell::new(std::collections::BTreeMap::new()),
            show_types: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            consts: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        }
    }

    /// The C name of plain enum type `tp`'s value names, indexed by value (1-based).
    pub(crate) fn enum_names(&self, tp: u16) -> String {
        self.enum_names.borrow_mut().insert(tp);
        format!("loft_enum{tp}")
    }

    /// The C name of record type `tp`'s default bytes — what the interpreter's default walk
    /// writes into a new record of it (`Stores::set_default_value`), computed by running that
    /// walk on a copy of the stores, so the two cannot disagree.
    pub(crate) fn default_image(&self, tp: u16) -> (String, u32) {
        let size = u32::from(self.stores.size(tp));
        if !self.defaults.borrow().contains_key(&tp) {
            let mut scratch = self.scratch.borrow_mut();
            // The clone keeps the schema and no stores; `max` still counts the original's.
            let s = scratch.get_or_insert_with(|| {
                let mut s = self.stores.clone();
                s.max = 0;
                s
            });
            let r = s.database(size.div_ceil(8) + 1);
            s.set_default_value(tp, &r);
            let bytes = s.store(&r).read_span(r.rec, r.pos, size).to_vec();
            self.defaults.borrow_mut().insert(tp, bytes);
        }
        (format!("loft_dflt{tp}"), size)
    }

    /// The C shape of `tp`, nullable or not — a null is held in the same shape — or `None`.
    pub(crate) fn ctype(&self, tp: &Type) -> Option<CType> {
        match tp.base() {
            // A narrow integer is held as a plain one: its range is enforced where a value
            // enters it (`OpRangeDefault`) and where a field stores it.
            Type::Integer(_) => Some(CType::Int),
            Type::Boolean => Some(CType::Bool),
            Type::Float => Some(CType::Float),
            Type::Single => Some(CType::Single),
            Type::Character => Some(CType::Char),
            Type::Text(_) => Some(CType::Text),
            Type::Reference(..) | Type::Vector(..) | Type::Enum(_, true, _) => Some(CType::Ref),
            Type::Enum(_, false, _) => Some(CType::Enum),
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
            CType::Enum => "255".into(),
            CType::Float => "(double) NAN".into(),
            CType::Single => "(float) NAN".into(),
            CType::Char => "0".into(),
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

    /// Register type `tp`, and every type it reaches, in the unit's copy table — or the
    /// kind of collection the C copy does not walk yet, for a refusal.
    pub(crate) fn copy_type(&self, tp: u16) -> Result<(), String> {
        use crate::database::Parts;
        if self.copy_types.borrow().contains(&tp) {
            return Ok(());
        }
        let t = self
            .stores
            .types
            .get(tp as usize)
            .ok_or("an unknown type")?;
        self.copy_types.borrow_mut().insert(tp);
        match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().try_for_each(|f| self.copy_type(f.content))
            }
            Parts::Vector(c) | Parts::ChildRec(c) => self.copy_type(*c),
            Parts::Enum(values) => values
                .iter()
                .filter(|(v, _)| *v != u16::MAX)
                .try_for_each(|(v, _)| self.copy_type(*v)),
            Parts::Base
            | Parts::Byte(..)
            | Parts::Short(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::ShortRaw(..)
            | Parts::DbRef => Ok(()),
            other => {
                self.copy_types.borrow_mut().remove(&tp);
                let kind = format!("{other:?}");
                Err(format!(
                    "a copy of a record that owns a {}",
                    kind.split(['(', ' '])
                        .next()
                        .unwrap_or("collection")
                        .to_lowercase()
                ))
            }
        }
    }

    /// The copy table: one `loft_tdesc` per registered type, at its type number.
    fn copy_table(&self) -> String {
        use crate::database::Parts;
        let types = self.copy_types.borrow();
        if types.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        let mut rows = Vec::new();
        for &tp in types.iter() {
            let t = &self.stores.types[tp as usize];
            let size = self.stores.size(tp);
            let row = match &t.parts {
                Parts::Base if tp == 5 || t.name == "text" => {
                    format!("{{ LOFT_T_TEXT, {size}, 0, 0, 0, 0 }}")
                }
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    let list: Vec<String> = fields
                        .iter()
                        .map(|f| format!("{{ {}, {} }}", f.position, f.content))
                        .collect();
                    let _ = writeln!(
                        out,
                        "static const loft_fld loft_fields{tp}[] = {{ {} }};",
                        if list.is_empty() {
                            "{ 0, 0 }".into()
                        } else {
                            list.join(", ")
                        }
                    );
                    format!(
                        "{{ LOFT_T_STRUCT, {size}, 0, {}, loft_fields{tp}, 0 }}",
                        fields.len()
                    )
                }
                Parts::Vector(c) => format!("{{ LOFT_T_VECTOR, {size}, {c}, 0, 0, 0 }}"),
                Parts::ChildRec(c) => format!("{{ LOFT_T_CHILD, {size}, {c}, 0, 0, 0 }}"),
                Parts::Enum(values) if values.iter().any(|(v, _)| *v != u16::MAX) => {
                    let list: Vec<String> = values.iter().map(|(v, _)| v.to_string()).collect();
                    let _ = writeln!(
                        out,
                        "static const uint16_t loft_variants{tp}[] = {{ {} }};",
                        list.join(", ")
                    );
                    format!(
                        "{{ LOFT_T_ENUM, {size}, 0, {}, 0, loft_variants{tp} }}",
                        values.len()
                    )
                }
                _ => format!("{{ LOFT_T_SCALAR, {size}, 0, 0, 0, 0 }}"),
            };
            rows.push(format!("    [{tp}] = {row},"));
        }
        let max = types.iter().max().copied().unwrap_or(0);
        let _ = writeln!(
            out,
            "static const loft_tdesc loft_types[{}] = {{\n{}\n}};",
            u32::from(max) + 1,
            rows.join("\n")
        );
        out
    }

    /// The tuple structs, each after the ones it holds, and the default images.
    fn typedefs(&self) -> String {
        let mut out = self.copy_table();
        for tp in self.enum_names.borrow().iter() {
            let count = match &self.stores.types[*tp as usize].parts {
                crate::database::Parts::Enum(values) => values.len(),
                _ => 0,
            };
            let names: Vec<String> = (1..=count)
                .map(|v| body::c_string(self.stores.enum_val(*tp, v as u8)))
                .collect();
            let _ = writeln!(
                out,
                "static const char *const loft_enum{tp}[] = {{ \"\", {} }};",
                names.join(", ")
            );
        }
        for (tp, bytes) in self.defaults.borrow().iter() {
            let list: Vec<String> = bytes.iter().map(u8::to_string).collect();
            let _ = writeln!(
                out,
                "static const uint8_t loft_dflt{tp}[{}] = {{ {} }};",
                bytes.len().max(1),
                if list.is_empty() {
                    "0".to_string()
                } else {
                    list.join(", ")
                }
            );
        }
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
        out.push_str(&self.show_code());
        out.push_str(&self.const_code());
        out
    }

    /// The C name of compile-time constant `d`'s vector, or a refusal when it was not built.
    pub(crate) fn const_ref(&self, d: u32) -> Result<String, String> {
        if self.data.def(d).const_ref.is_none() {
            return Err("a constant that was not built".into());
        }
        self.consts.borrow_mut().insert(d);
        Ok(format!("loft_const{d}"))
    }

    /// Each constant's store, as words, and `loft_init`, which loads them before `main` runs.
    fn const_code(&self) -> String {
        let mut out = String::new();
        let mut init = String::from("static void loft_init(void) {\n");
        for &d in self.consts.borrow().iter() {
            let Some(r) = self.data.def(d).const_ref else {
                continue;
            };
            let store = self.stores.store(&r);
            let mut n = store.capacity_words();
            while n > 1 && store.get_u32_raw(n - 1, 0) == 0 && store.get_u32_raw(n - 1, 4) == 0 {
                n -= 1;
            }
            let words: Vec<String> = (0..n)
                .flat_map(|w| [store.get_u32_raw(w, 0), store.get_u32_raw(w, 4)])
                .map(|x| format!("{x}u"))
                .collect();
            let _ = writeln!(
                out,
                "static const uint32_t loft_cwords{d}[] = {{ {} }};\nstatic loft_ref loft_const{d};",
                words.join(", ")
            );
            let _ = writeln!(
                init,
                "    loft_const{d} = loft_const_store(loft_cwords{d}, {n});"
            );
        }
        init.push_str("}\n");
        out.push_str(&init);
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
///
/// `expected` names the messages a test is expected to fail with (one must appear in its
/// failure), or none for a test that must pass.
pub fn emit_program(
    data: &Data,
    stores: &Stores,
    expected: &dyn Fn(&str) -> Vec<String>,
) -> Result<String, Refusal> {
    let main = data.def_nr("n_main");
    if main >= data.definitions() {
        return emit_tests(data, stores, expected);
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
        "\nint main(void) {{\n    loft_init();\n    {}();\n    return 0;\n}}\n",
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
    // The parameters are the argument variables, in number order: `ref_return` can promote a
    // local into the return buffer's place, so parameter i is not always variable i.
    let args = vars.arguments();
    if args.len() != def.attributes().len() {
        return Err("parameters that do not match their arguments".into());
    }
    let mut shapes = Vec::with_capacity(vars.count() as usize);
    for v in 0..vars.count() {
        // A `&` parameter has its target's shape, passed as a pointer to it.
        let tp = match vars.tp(v) {
            Type::RefVar(inner) if args.contains(&v) => inner,
            tp => tp,
        };
        let c = cx.ctype(tp).filter(|c| *c != CType::Void).ok_or_else(|| {
            let what = if args.contains(&v) {
                "parameter"
            } else {
                "local"
            };
            format!("{what} of type {}", type_word(tp))
        })?;
        shapes.push(c);
    }
    let params: Vec<String> = args
        .iter()
        .map(|&a| {
            let c = shapes[a as usize];
            let name = body::var_name(vars.name(a), a);
            if matches!(vars.tp(a), Type::RefVar(_)) {
                format!("{} *{name}", held(c))
            } else {
                format!("{} {name}", passed(c))
            }
        })
        .collect();
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let sig = format!("static {} {}({params})", held(ret), c_name(data, d));
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
fn emit_tests(
    data: &Data,
    stores: &Stores,
    expected: &dyn Fn(&str) -> Vec<String>,
) -> Result<String, Refusal> {
    let tests = test_functions(data);
    if tests.is_empty() {
        return Err(Refusal {
            function: "main".into(),
            why: "neither a main nor a test".into(),
        });
    }
    let mut out = emit_unit(data, stores, &tests)?;
    out.push_str(
        "\nint main(void) {\n    int failed = 0;\n    loft_init();\n    loft_in_test = 1;\n",
    );
    for &d in &tests {
        let name = data.def(d).original_name();
        let subs = expected(&name);
        if subs.is_empty() {
            let _ = writeln!(
                out,
                "    if (setjmp(loft_test_jmp) == 0) {}(); else {{ failed++; fputs(\"FAIL {name}\\n\", stderr); }}",
                c_name(data, d),
            );
        } else {
            // Passes only by failing, with one of the expected messages in its own.
            let list: Vec<String> = subs
                .iter()
                .map(|s| format!("loft_lit({}, {})", body::c_string(s), s.len()))
                .collect();
            let _ = writeln!(
                out,
                "    {{ const loft_str want[] = {{ {} }};\n      \
                 if (setjmp(loft_test_jmp) == 0) {{ {}(); failed++; fputs(\"FAIL {name} (expected to fail)\\n\", stderr); }}\n      \
                 else if (!loft_failed_with(want, {})) {{ failed++; fputs(\"FAIL {name}\\n\", stderr); }} }}",
                list.join(", "),
                c_name(data, d),
                subs.len()
            );
        }
    }
    out.push_str("    return failed ? 1 : 0;\n}\n");
    Ok(out)
}
