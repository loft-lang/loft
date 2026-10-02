//! Rust code generator for native function bodies and operator dispatch.
//\! @I68 — Native Rust generator
//!
//! - [`generate_lib`] — writes `tests/generated/text.rs` with the native
//!   function table from `#rust` annotations in `default/*.loft`.
//! - [`generate_code_to`] — writes `src/fill.rs` (the interpreter's
//!   bytecode-operator dispatch table) from each operator's `#rust"..."`
//!   annotation in `default/*.loft`.  It emits a `// @generated` header into the
//!   file documenting this; see that header for the per-operator contract.
//!
//! Both are maintenance tools.  Regenerate `src/fill.rs` with `make fill` (the
//! ignored [`regen_fill_rs`] test); its byte-for-byte freshness is enforced by
//! `tests/issues.rs::fill_rs_up_to_date` and `::n9_generated_fill_matches_src`,
//! and `tests/generated/text.rs` by `::native_rs_functions_up_to_date`.
//!
//! The same `#rust` templates are the single source for BOTH backends: this
//! generator emits the interpreter bodies (`s: &mut State`), and native code
//! generation (`src/generation/`) reuses the templates, rewriting `s.<method>`
//! to `stores.*` / `*_runtime` via [`crate::generation`]'s
//! `substitute_template_body`.

use crate::data::{Context, Data, OP_HOT, OP_NORMAL, Type};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;

fn operator_name(operator: &str) -> String {
    let mut result = String::new();
    for (i, c) in operator.chars().enumerate() {
        if i < 2 {
            continue;
        }
        if c.is_uppercase() {
            if i > 2 {
                result += "_";
            }
            result += c.to_lowercase().to_string().as_str();
        } else {
            result.push(c);
        }
    }
    if result == "return" {
        "op_return".to_string()
    } else {
        result
    }
}

/**
    Write a library file with the known library functions.
    # Errors
    When the file cannot be written correctly.
*/
pub fn generate_lib(data: &Data) -> std::io::Result<()> {
    let mut into = File::create("tests/generated/text.rs")?;
    writeln!(
        into,
        "#![allow(clippy::cast_possible_wrap)]
#![allow(non_snake_case)]
use crate::database::Stores;
use crate::keys::{{DbRef, Str}};
use crate::state::{{Call, State}};

pub const FUNCTIONS: &[(&str, Call)] = &["
    )?;
    for d_nr in 0..data.definitions() {
        let d = data.def(d_nr);
        let n = &d.name;
        if !d.is_operator() && !d.rust.is_empty() {
            writeln!(into, "    (\"{n}\", {n}),")?;
        }
    }
    writeln!(
        into,
        "];

pub fn init(state: &mut State) {{
    for (name, implement) in FUNCTIONS {{
        state.static_fn(name, *implement);
    }}
}}"
    )?;
    for d_nr in 0..data.definitions() {
        let d = data.def(d_nr);
        let n = &d.name;
        if d.is_operator() || d.rust.is_empty() {
            continue;
        }
        writeln!(into, "\nfn {n}(stores: &mut Stores, stack: &mut DbRef) {{")?;
        for a in data.def(d_nr).attributes.iter().rev() {
            let tp = data.rust_type(&a.typedef, &Context::Argument);
            writeln!(into, "    let v_{} = stores.get::<{tp}>(stack);", a.name)?;
            if let Type::RefVar(var) = &a.typedef
                && let Type::Text(_) = **var
            {
                writeln!(
                    into,
                    "    let v_{} = stores.store_mut(&v_{}).addr_mut::<String>(v_{}.rec, v_{}.pos);",
                    a.name, a.name, a.name, a.name
                )?;
            }
        }
        let mut res = data.def(d_nr).rust.clone();
        replace_attributes(data, d_nr, &mut res);
        if d.returned == Type::Void {
            writeln!(into, "    {res}")?;
        } else {
            writeln!(into, "    let new_value = {{ {res} }};")?;
            writeln!(into, "    stores.put(stack, new_value);")?;
        }
        writeln!(into, "}}")?;
    }
    drop(into);
    let _ = std::process::Command::new("rustfmt")
        .args(["--edition", "2024", "tests/generated/text.rs"])
        .status();
    Ok(())
}

fn replace_attributes(data: &Data, d_nr: u32, res: &mut String) {
    for a_nr in 0..data.attributes(d_nr) {
        let name = "@".to_string() + &data.attr_name(d_nr, a_nr);
        let mut repl = "v_".to_string();
        repl += &data.attr_name(d_nr, a_nr);
        if matches!(data.attr_type(d_nr, a_nr), Type::Text(_)) {
            repl += ".str()";
        }
        *res = res.replace(&name, &repl);
    }
}

/// Create the content of the fill.rs file from the default library definitions.
/// # Errors
/// When the resulting file cannot be correctly written.
pub fn generate_code(data: &Data) -> std::io::Result<()> {
    generate_code_to(data, "tests/generated/fill.rs").map(|_| ())
}

/// Write fill.rs content to `path`, then format it with rustfmt and return the result.
/// Use this when you need a formatted copy at a custom path (e.g. to avoid file-write races).
/// # Errors
/// When the file cannot be written.
pub fn generate_code_to(data: &Data, path: &str) -> std::io::Result<String> {
    let mut into = File::create(path)?;
    generate_code_into(data, &mut into)?;
    drop(into);
    let _ = std::process::Command::new("rustfmt")
        .args(["--edition", "2024", path])
        .status();
    std::fs::read_to_string(path)
}

/// `@FR-R-OpPriority` — the operators in slot order: the `#hot` ones first, then those with no priority, then the
/// `#cold` ones, each class in declaration order — the numbering `Data::op_code` gives them
/// from the two class sizes returned beside it.  A one-byte opcode is a slot below 255, so
/// every hot operator gets one, and the cold ones are what moves into the two-byte escape.
///
/// # Panics
/// When more than 255 operators are `#hot`.
fn operator_slots(data: &Data) -> (Vec<u32>, usize, usize) {
    let ops: Vec<u32> = (0..data.definitions())
        .filter(|&d_nr| data.def(d_nr).is_operator())
        .collect();
    let class = |d_nr: u32| data.def(d_nr).op_priority;
    let mut slots = ops.clone();
    slots.sort_by_key(|&d_nr| class(d_nr));
    let hot_count = ops.iter().filter(|&&d_nr| class(d_nr) == OP_HOT).count();
    let normal_count = ops.iter().filter(|&&d_nr| class(d_nr) == OP_NORMAL).count();
    assert!(
        hot_count <= 255,
        "{hot_count} operators are #hot; at most 255 one-byte opcodes exist"
    );
    (slots, hot_count, normal_count)
}

/// The byte width of an operand of Rust type `tp` in the bytecode stream, or `None` when it
/// has none fixed.
fn fixed_width(tp: &str) -> Option<u32> {
    match tp {
        "u8" | "i8" => Some(1),
        "u16" | "i16" => Some(2),
        "u32" | "i32" | "f32" => Some(4),
        "u64" | "i64" | "f64" => Some(8),
        _ => None,
    }
}

/// `@FR-R-FastTable` — an operator body with its stack accessors spelled for the access mode
/// `F` of the generic it is generated into (`State::get_stack_m` and its siblings).
fn respell_stack_accessors(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .replace("s.get_stack::<", "s.get_stack_m::<F, ")
        .replace("s.get_var::<", "s.get_var_m::<F, ")
        .replace("s.put_stack(", "s.put_stack_m::<F, _>(")
        .replace("s.put_var(", "s.put_var_m::<F, _>(")
}

/// Write fill.rs content directly to an arbitrary writer (no rustfmt).
/// # Errors
/// When the writer reports an error.
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the operators: the table, the names, and each body"
)]
pub fn generate_code_into(data: &Data, into: &mut dyn Write) -> std::io::Result<()> {
    let (slots, hot_count, normal_count) = operator_slots(data);
    let mut slot_of: HashMap<u32, u16> = HashMap::new();
    for (slot, &d_nr) in slots.iter().enumerate() {
        slot_of.insert(d_nr, slot as u16);
    }
    writeln!(
        into,
        "// @generated — DO NOT EDIT BY HAND.
//
// The interpreter's bytecode-operator dispatch table, generated from the
// `#rust\"...\"` operator annotations in default/*.loft by
// `src/create.rs::generate_code_into` — each `fn op_*(s: &mut State)` body is
// that operator's `#rust` template (written in `s: &mut State` vocabulary).
//
// Regenerate after changing ANY `#rust` operator template:
//     make fill        (runs the ignored `regen_fill_rs` test, which calls
//                       `create::generate_code_to(.., \"src/fill.rs\")`)
// Byte-for-byte equality of this file with that regeneration is enforced by
// `tests/issues.rs::fill_rs_up_to_date` and `::n9_generated_fill_matches_src`,
// so hand-edits fail CI — edit the `#rust` template in default/*.loft instead.
//
// The SAME templates feed native code generation (`src/generation/`): there the
// `s.<method>` calls below are rewritten to their `stores.*` / `*_runtime`
// equivalents by `src/generation/calls.rs::substitute_template_body`.
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::inline_always)]
#![allow(unused_parens)]

use crate::codegen_runtime;
use crate::hash;
use crate::keys::{{DbRef, Str}};
use crate::ops;
use crate::state::{{Hot, Regs, State}};
use crate::tree;
use crate::vector;

pub static OPERATORS: &[fn(&mut State)] = &["
    )?;
    for &d_nr in &slots {
        writeln!(
            into,
            "    {}::<false>,",
            operator_name(&data.def(d_nr).name)
        )?;
    }
    writeln!(into, "];")?;
    // `@FR-R-FastTable` — the same operators with the stack access mode fixed to the direct
    // path, dispatched only when `State::fast_stack` holds.
    writeln!(
        into,
        "\n/// [`OPERATORS`] with the direct stack path compiled in (`@FR-R-FastTable`): valid\n\
         /// only while `State::fast_stack` holds, which is fixed for a run.\n\
         pub static OPERATORS_FAST: &[fn(&mut State)] = &["
    )?;
    for &d_nr in &slots {
        writeln!(into, "    {}::<true>,", operator_name(&data.def(d_nr).name))?;
    }
    writeln!(into, "];")?;
    // `@FR-R-RegisterTable` — the same direct-path operators with the position and the stack
    // top passed in registers (`State::regs_in` / `regs_out`), dispatched by the lean loop.
    writeln!(
        into,
        "\n/// [`OPERATORS_FAST`] with the bytecode position and the stack top passed in and\n\
         /// returned in registers ([`Regs`]), so no op reads back what the previous op stored.\n\
         pub static OPERATORS_REG: &[fn(&mut State, Regs) -> Regs] = &["
    )?;
    for &d_nr in &slots {
        writeln!(into, "    {}_r,", operator_name(&data.def(d_nr).name))?;
    }
    writeln!(into, "];")?;
    // The loft name of every slot above, in the same order: what a stdlib parsed at run
    // time is checked against (`stdlib_ops::verify`), because the table is POSITIONAL —
    // one declaration more or fewer in `default/` moves every later body under the wrong
    // opcode, and nothing but this list can tell.
    writeln!(
        into,
        "\n/// The loft name of each [`OPERATORS`] slot, in slot order — the operator declarations\n\
         /// of the `default/` this binary was generated from.\n\
         pub const OPERATOR_NAMES: &[&str] = &["
    )?;
    for &d_nr in &slots {
        writeln!(into, "    \"{}\",", data.def(d_nr).name)?;
    }
    writeln!(into, "];")?;
    // The class sizes `Data::op_code` numbers a declaration from: `#hot` operators take the
    // first slots, `#cold` ones the last, the rest between, each class in declaration order.
    writeln!(
        into,
        "\n/// How many operators are `#hot` (slots `0..OP_HOT`, all one-byte opcodes): the lean\n\
         /// loop runs them inline.  `Data::op_code` numbers a declaration from this and\n\
         /// [`OP_NORMAL`], the order the tables above are laid out in.\n\
         pub const OP_HOT: u16 = {hot_count};\n\
         /// How many operators are neither `#hot` nor `#cold` (the slots after the hot ones); the\n\
         /// `#cold` operators follow them, into the two-byte opcodes.\n\
         pub const OP_NORMAL: u16 = {normal_count};"
    )?;
    let mut hot_arms: Vec<(u16, String)> = Vec::new();
    for d_nr in 0..data.definitions() {
        let n = &data.def(d_nr).name;
        if !data.def(d_nr).is_operator() {
            continue;
        }
        let name = operator_name(n);
        // Each op body is written to a buffer first: its stack accessors are respelled with
        // the access mode `F` (`@FR-R-FastTable`, `State::get_stack_m` and its siblings).
        let outer: &mut dyn Write = &mut *into;
        let mut inner: Vec<u8> = Vec::new();
        let into: &mut dyn Write = &mut inner;
        let mut res = data.def(d_nr).rust.clone();
        // `@FR-R-OperandSpan` — the operands (the non-mutable attributes), each at its offset.  When
        // every one has a fixed width they are bounds-checked and stepped over ONCE
        // (`State::operands`); otherwise each is read on its own, as `State::code` does.
        let operands: Vec<(&str, String)> = data
            .def(d_nr)
            .attributes
            .iter()
            .filter(|a| !(a.name.starts_with('_') || res.is_empty() || a.mutable))
            .map(|a| {
                (
                    a.name.as_str(),
                    data.rust_type(&a.typedef, &Context::Argument),
                )
            })
            .collect();
        let widths: Option<Vec<u32>> = operands.iter().map(|(_, tp)| fixed_width(tp)).collect();
        match widths {
            Some(widths) if !operands.is_empty() => {
                let total: u32 = widths.iter().sum();
                writeln!(into, "    let operands = s.operands({total});")?;
                let mut off = 0;
                for ((name, tp), w) in operands.iter().zip(&widths) {
                    writeln!(into, "    let v_{name} = operands.get::<{tp}>({off});")?;
                    off += w;
                }
            }
            _ => {
                for (name, tp) in &operands {
                    writeln!(into, "    let v_{name} = s.code::<{tp}>();")?;
                }
            }
        }
        for a in data.def(d_nr).attributes.iter().rev() {
            if a.name.starts_with('_') || res.is_empty() {
                continue;
            }
            let tp = data.rust_type(&a.typedef, &Context::Argument);
            if a.mutable {
                if matches!(a.typedef, Type::Text(_)) {
                    writeln!(into, "    let v_{} = s.string();", a.name)?;
                } else if matches!(a.typedef, Type::Character) {
                    // character values on the stack may be the
                    // `i32::MIN` (0x80000000) coroutine-exhaustion sentinel
                    // pushed by `push_null_value`. That bit pattern is not a
                    // valid Unicode scalar value, so reading the bytes as
                    // `s.get_stack::<char>()` is undefined behaviour: the
                    // release-mode optimiser then assumes the resulting
                    // `char` is a valid scalar and elides any sentinel
                    // check, causing `for c in iterator<character>()` loops
                    // to hang forever. Read the raw `u32` and map invalid
                    // bytes to the null character `'\0'` so the op
                    // functions always see a valid `char`.
                    writeln!(
                        into,
                        "    let v_{} = char::from_u32(s.get_stack::<u32>()).unwrap_or('\\0');",
                        a.name
                    )?;
                } else if matches!(a.typedef, Type::Boolean) {
                    // @PLN17 spike: booleans are tri-state (0=false, 1=true,
                    // 255=null).  Reading the byte as `bool` is UB for 255, so read
                    // the raw u8 — truthiness ops coerce (255 -> false) and
                    // value-movement / comparison ops preserve it.
                    writeln!(into, "    let v_{} = s.get_stack::<u8>();", a.name)?;
                } else {
                    writeln!(into, "    let v_{} = s.get_stack::<{tp}>();", a.name)?;
                }
            }
        }
        replace_attributes(data, d_nr, &mut res);
        res = res.replace("stores.", "s.database.");
        let returned = &data.def(d_nr).returned;
        if res.is_empty() {
            writeln!(into, "    s.{name}();")?;
        } else if *returned == Type::Void
            || (matches!(*returned, Type::Text(_)) && data.def(d_nr).name.starts_with("OpConst"))
        {
            writeln!(into, "    {res}")?;
        } else {
            writeln!(into, "    let new_value = {res};")?;
            writeln!(into, "    s.put_stack(new_value);")?;
        }
        let mut body: Vec<u8> = Vec::new();
        // Inlined into its register-table entry (`{name}_r`), which is what lets the body's
        // reads of `code_pos` / `stack_pos` fold to the registers; the tables take the address.
        writeln!(
            body,
            "\n#[inline(always)]\nfn {name}<const F: bool>(s: &mut State) {{"
        )?;
        body.extend_from_slice(&inner);
        writeln!(body, "}}")?;
        // The register-table entry: the registers written into `State`, the direct-path body
        // inlined, the registers read back.
        writeln!(
            body,
            "\nfn {name}_r(s: &mut State, r: Regs) -> Regs {{\n    s.regs_in(r);\n    {name}::<true>(s);\n    s.regs_out()\n}}"
        )?;
        outer.write_all(respell_stack_accessors(&body).as_bytes())?;
        // `#hot`: the same body again, on the lean loop's registers (`State::Hot`), which offers
        // only what such a body may use — a template that needs more does not compile as hot.
        if data.def(d_nr).op_priority == OP_HOT {
            let mut hot: Vec<u8> = Vec::new();
            writeln!(hot, "\n#[inline(always)]\nfn {name}_h(s: &mut Hot) {{")?;
            hot.extend_from_slice(&inner);
            writeln!(hot, "}}")?;
            outer.write_all(&hot)?;
            hot_arms.push((slot_of[&d_nr], name.clone()));
        }
    }
    // `@FR-R-HotInline` — the lean loop's dispatch: each `#hot` operator inline on the loop's
    // registers, every other one through the register table.
    writeln!(
        into,
        "\n/// The lean loop's dispatch: a `#hot` operator runs inline on the loop's registers\n\
         /// ([`Hot`]), every other one through [`OPERATORS_REG`].\n\
         #[expect(\n    clippy::too_many_lines,\n    reason = \"one arm per #hot operator: the table IS the dispatch, and splitting it adds a call the inline arms exist to avoid\"\n)]\n\
         #[inline(always)]\n\
         pub(crate) fn dispatch_lean(s: &mut State, opcode: u16, r: Regs) -> Regs {{\n    match opcode {{"
    )?;
    for (code, name) in &hot_arms {
        writeln!(
            into,
            "        {code} => {{\n            let mut h = Hot::new(s, r);\n            {name}_h(&mut h);\n            h.finish()\n        }}"
        )?;
    }
    writeln!(
        into,
        "        _ => OPERATORS_REG[usize::from(opcode)](s, r),\n    }}\n}}"
    )?;
    Ok(())
}
