// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I86 — Startup cache & embedded stdlib: the standard library's compiled bodies, as source

//! @PLN181 — writing the standard library's compiled bodies: which functions are compiled
//! ([`export_set`]) and the source `src/compiled_stdlib_gen.rs` holds ([`generate`],
//! `make compiled-stdlib`).  The interpreter's half — routing calls to those bodies at start-up
//! — is `compiled_stdlib`.

use crate::data::{Data, Value};
use crate::database::Stores;
use crate::opt::native_key::prefix_fingerprint;
use std::collections::HashSet;
use std::fmt::Write as _;

/// The standard-library functions that are compiled: the ones the shared-store bridge can
/// carry, that have a loft body, and whose body LOOPS.  A body without a loop is one or two
/// operations the interpreter already runs as such (`sin` is one `OpMathFuncFloat`), where the
/// bridge would add a call and win nothing, and it is where output and file primitives sit.
/// Derived from the IR, so a function joins the set the day its body starts to loop — which is
/// how a kernel's loft body, restored, is compiled (@PLN181 P2).
#[must_use]
pub fn export_set(data: &Data) -> HashSet<u32> {
    crate::native_gate::shared_store_dispatchable(data)
        .into_iter()
        .filter(|&d| {
            let code = &data.def(d).code;
            *code != Value::Null
                && code.any_node(&mut |n| matches!(n, Value::Loop(_) | Value::Iter(..)))
        })
        .collect()
}

/// The compiled standard library as source to build inside this crate: the library artifact
/// [`crate::native_lib::generate_shared_cdylib_lib_rs`] emits for [`export_set`], with the crate
/// named by an alias instead of `extern crate`, no exported symbols (a user library's cdylib
/// links this crate, and two `#[no_mangle]` definitions of one name do not link), and the
/// lookup table plus the type prefix it was generated against appended.
///
/// `data` and `stores` must be a parse of the standard library alone, so the types it records
/// are exactly the prefix every program starts with; `sources` is that library's source
/// (`cache::collect_stdlib_sources`), whose hash the file records.
#[must_use]
pub fn generate(data: &Data, stores: &Stores, sources: &[(String, String)]) -> String {
    let export = export_set(data);
    let library = crate::native_lib::shared_cdylib_source(
        data,
        stores,
        &export,
        true,
        crate::opt::native_key::type_layout_fingerprint(stores),
    )
    // Built into the host: its bridges reach the host's own table, not one handed over.
    .replace(
        crate::native_lib::HOST_INIT_SRC,
        crate::native_lib::IN_HOST_TABLE_SRC,
    );
    let mut out = String::from(
        "// @generated — DO NOT EDIT BY HAND.\n\
         //\n\
         // The standard library's compiled loft bodies (@PLN181, `src/stdlib_codegen.rs`): the\n\
         // functions `stdlib_codegen::export_set` picks, as the native backend emits them, with\n\
         // one shared-store bridge each.  Regenerate after changing default/*.loft:\n\
         //     make compiled-stdlib\n\
         // `tests/compiled_stdlib.rs::compiled_stdlib_up_to_date` fails when this file is stale.\n\n",
    );
    for line in library.lines() {
        if line == "extern crate loft;" {
            out.push_str("use crate as loft;\n");
        } else if line.trim() == "#[unsafe(no_mangle)]" {
            // Not exported: [`bridge`] finds these by table, and the crate is linked into
            // library cdylibs that carry their own symbols of these names.
        } else if line.starts_with("#![") {
            // Crate-level lint settings; the module carries its own `#[allow]`.
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    let mut bridges: Vec<(String, String)> = export
        .iter()
        .map(|&d| {
            let def = data.def(d);
            let sym = format!(
                "loft_shared_{}",
                crate::opt::names::disambiguated_fn_ident(
                    &crate::opt::names::duplicate_fn_names(data),
                    def
                )
            );
            (def.name().to_string(), sym)
        })
        .collect();
    bridges.sort();
    out.push_str(
        "\n/// `(function, bridge symbol, bridge)` for every compiled function, sorted by name.\n\
         pub(crate) static BRIDGES: &[(&str, &str, super::Bridge)] = &[\n",
    );
    for (name, sym) in &bridges {
        let _ = writeln!(out, "    ({name:?}, {sym:?}, {sym}),");
    }
    out.push_str("];\n");
    let n = stores.types.len();
    let _ = write!(
        out,
        "\n/// The standard library's type table this was generated against: its length and \
         fingerprint.\npub(crate) const PREFIX_TYPES: usize = {n};\n\
         pub(crate) const PREFIX_FINGERPRINT: u64 = {};\n\
         /// The hash of the `default/*.loft` source these bodies were compiled from.\n\
         pub(crate) const SOURCE_HASH: u64 = {};\n",
        prefix_fingerprint(stores, n).unwrap_or(0),
        crate::cache::source_hash(sources)
    );
    out
}
