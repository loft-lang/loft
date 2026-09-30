// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN181 — the standard library compiled like any other library.
//!
//! A `use`d library is compiled for an interpreted program (C71, "libraries compile, scripts
//! interpret") and called over the shared-store bridge: `native_gate::shared_store_dispatchable`
//! picks the functions, `native_lib::generate_shared_cdylib_lib_rs` emits their compiled loft
//! bodies with one `loft_shared_<fn>` bridge each, `native_lib::mark_exports` routes the calls
//! through `OpStaticCall`, and `extensions::wire_shared_native_fns` hands them to
//! `shared_store_dispatch`.  The standard library takes that same path, with two differences:
//!
//! 1. **No rustc at run time.**  The compiled source is generated with loft itself
//!    ([`generate`], `make compiled-stdlib`) into `src/compiled_stdlib_gen.rs` and built INTO
//!    the binary; [`bridge`] answers the lookup a library answers with `dlsym`.
//! 2. **One artifact for every program.**  A library artifact hard-codes type indices, so it is
//!    keyed on the caller's whole type table.  The standard library's types register before any
//!    program's, so the generated code is valid for every program whose table STARTS with the
//!    same types — checked at start-up ([`mark`]), and a program where it does not runs the loft
//!    bodies interpreted: slower, never wrong.
//!
//! `LOFT_NO_COMPILED_STDLIB=1` interprets the standard library's loft bodies (the A/B).

use crate::data::{Data, Value};
use crate::database::Stores;
use std::collections::HashSet;
use std::fmt::Write as _;

#[path = "compiled_stdlib_gen.rs"]
#[rustfmt::skip]
#[allow(warnings, clippy::all, clippy::pedantic)]
mod generated;

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

/// A hash that is the same on every toolchain: these values are written into a COMMITTED file,
/// and `DefaultHasher` promises stability only within one Rust release.
fn stable_hash(parts: &[&[u8]]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    let d = h.finalize();
    u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
}

/// The fingerprint of the first `n` types of `stores`, or `None` when it has fewer.  The same
/// facts `native_lib`'s whole-table fingerprint hashes (name, size, alignment), over a prefix.
fn prefix_fingerprint(stores: &Stores, n: usize) -> Option<u64> {
    if stores.types.len() < n {
        return None;
    }
    let mut facts: Vec<Vec<u8>> = Vec::with_capacity(n + 1);
    facts.push((n as u64).to_le_bytes().to_vec());
    for t in &stores.types[..n] {
        let mut f = t.name.as_bytes().to_vec();
        f.extend_from_slice(&t.size_bytes().to_le_bytes());
        f.push(t.align_bytes());
        facts.push(f);
    }
    let parts: Vec<&[u8]> = facts.iter().map(Vec::as_slice).collect();
    Some(stable_hash(&parts))
}

/// The hash of the standard-library source a program loads: every `default/*.loft` file's name
/// and content (`cache::collect_stdlib_sources`, the input the program cache keys on too).
fn source_hash(sources: &[(String, String)]) -> u64 {
    let mut parts: Vec<&[u8]> = Vec::with_capacity(sources.len() * 2);
    for (name, content) in sources {
        parts.push(name.as_bytes());
        parts.push(content.as_bytes());
    }
    stable_hash(&parts)
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
    let library = crate::native_lib::generate_shared_cdylib_lib_rs(data, stores, &export);
    let mut out = String::from(
        "// @generated — DO NOT EDIT BY HAND.\n\
         //\n\
         // The standard library's compiled loft bodies (@PLN181, `src/compiled_stdlib.rs`): the\n\
         // functions `compiled_stdlib::export_set` picks, as the native backend emits them, with\n\
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
                crate::generation::disambiguated_fn_ident(
                    &crate::generation::duplicate_fn_names(data),
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
        source_hash(sources)
    );
    out
}

/// A shared-store bridge's signature, as `native_lib::shared_bridge_wrapper` emits it.
pub(crate) type Bridge = extern "C" fn(
    *mut Stores,
    *const crate::native_lib::LibArg,
    usize,
    *mut crate::native_lib::LibArg,
);

/// Is the compiled standard library switched off (`LOFT_NO_COMPILED_STDLIB=1`)?
fn disabled() -> bool {
    crate::env_once!(std::env::var_os("LOFT_NO_COMPILED_STDLIB").is_some())
}

/// Route the compiled standard-library functions of an INTERPRETED program through their
/// bridges, before `byte_code`: sets `def.native` as `native_lib::mark_exports` does for a
/// library.  Declines — and returns 0 — when switched off, when the bridge dispatcher is not
/// built in, when `stores` does not start with the standard library's types exactly as the
/// generated code expects them, or when the standard library in `default_dir` is not the source
/// the bodies were compiled from.  The last one is the edit loop: loft reads `default/*.loft` at
/// run time, so an edited stdlib function would otherwise run its OLD compiled body — it runs
/// its current loft body interpreted instead, until `make compiled-stdlib`.  Returns how many functions dispatch to their compiled bodies,
/// counting the marks a warm start replays from the program cache.
pub fn mark(data: &mut Data, stores: &Stores, default_dir: &str) -> usize {
    if disabled()
        || !cfg!(feature = "native-extensions")
        || generated::BRIDGES.is_empty()
        || prefix_fingerprint(stores, generated::PREFIX_TYPES)
            != Some(generated::PREFIX_FINGERPRINT)
        || source_hash(&crate::cache::collect_stdlib_sources(default_dir)) != generated::SOURCE_HASH
    {
        return 0;
    }
    let mut marked = 0;
    for (name, sym, _) in generated::BRIDGES {
        let d = data.def_nr(name);
        if d == u32::MAX || data.def(d).code == Value::Null {
            continue;
        }
        // A warm start replays the marks the program cache saved with the bundle.
        if data.def(d).native() == *sym {
            marked += 1;
        } else if data.def(d).native().is_empty() {
            data.def_mut(d).native = (*sym).to_string();
            marked += 1;
        }
    }
    marked
}

/// The compiled bridge for a `loft_shared_…` symbol, when the standard library carries it.
#[must_use]
pub fn bridge(sym: &str) -> Option<*const ()> {
    generated::BRIDGES
        .iter()
        .find(|(_, s, _)| *s == sym)
        .map(|(_, _, f)| *f as *const ())
}
