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

use crate::data::{Context, Data, Type};
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
#![allow(unused_parens)]

use crate::codegen_runtime;
use crate::hash;
use crate::keys::{{DbRef, Str}};
use crate::ops;
use crate::state::State;
use crate::tree;
use crate::vector;

pub static OPERATORS: &[fn(&mut State)] = &["
    )?;
    for d_nr in 0..data.definitions() {
        let n = &data.def(d_nr).name;
        if data.def(d_nr).is_operator() {
            writeln!(into, "    {}::<false>,", operator_name(n))?;
        }
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
    for d_nr in 0..data.definitions() {
        let n = &data.def(d_nr).name;
        if data.def(d_nr).is_operator() {
            writeln!(into, "    {}::<true>,", operator_name(n))?;
        }
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
    for d_nr in 0..data.definitions() {
        let n = &data.def(d_nr).name;
        if data.def(d_nr).is_operator() {
            writeln!(into, "    \"{n}\",")?;
        }
    }
    writeln!(into, "];")?;
    for d_nr in 0..data.definitions() {
        let n = &data.def(d_nr).name;
        if !data.def(d_nr).is_operator() {
            continue;
        }
        let name = operator_name(n);
        // Each op body is written to a buffer first: its stack accessors are respelled with
        // the access mode `F` (`@FR-R-FastTable`, `State::get_stack_m` and its siblings).
        let outer: &mut dyn Write = &mut *into;
        let mut body: Vec<u8> = Vec::new();
        let into: &mut dyn Write = &mut body;
        writeln!(into, "\nfn {name}<const F: bool>(s: &mut State) {{")?;
        let mut res = data.def(d_nr).rust.clone();
        for a in &data.def(d_nr).attributes {
            if a.name.starts_with('_') || res.is_empty() {
                continue;
            }
            let tp = data.rust_type(&a.typedef, &Context::Argument);
            if !a.mutable {
                writeln!(into, "    let v_{} = s.code::<{tp}>();", a.name)?;
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
        writeln!(into, "}}")?;
        outer.write_all(respell_stack_accessors(&body).as_bytes())?;
    }
    Ok(())
}
