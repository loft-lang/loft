// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I86 — Startup cache & embedded stdlib: the standard library's compiled bodies inside the binary

//! @PLN181 — the standard library compiled like any other library.
//!
//! A `use`d library is compiled for an interpreted program (@C71, "libraries compile, scripts
//! interpret") and called over the shared-store bridge: `native_gate::shared_store_dispatchable`
//! picks the functions, `native_lib::generate_shared_cdylib_lib_rs` emits their compiled loft
//! bodies with one `loft_shared_<fn>` bridge each, `native_adopt::mark_exports` routes the calls
//! through `OpStaticCall`, and `extensions::wire_shared_native_fns` hands them to
//! `shared_store_dispatch`.  The standard library takes that same path, with two differences:
//!
//! 1. **No rustc at run time.**  The compiled source is generated with loft itself
//!    (`stdlib_codegen::generate`, `make compiled-stdlib`) into `src/compiled_stdlib_gen.rs`
//!    and built INTO the binary; [`bridge`] answers the lookup a library answers with `dlsym`.
//! 2. **One artifact for every program.**  A library artifact hard-codes type indices, so it is
//!    keyed on the caller's whole type table.  The standard library's types register before any
//!    program's, so the generated code is valid for every program whose table STARTS with the
//!    same types — checked at start-up ([`mark`]), and a program where it does not runs the loft
//!    bodies interpreted: slower, never wrong.
//!
//! `LOFT_NO_COMPILED_STDLIB=1` interprets the standard library's loft bodies (the A/B).

use crate::data::{Data, Value};
use crate::database::Stores;

#[path = "compiled_stdlib_gen.rs"]
#[rustfmt::skip]
#[allow(warnings, clippy::all, clippy::pedantic)]
mod generated;


pub(crate) use crate::opt::native_key::prefix_fingerprint;

/// A shared-store bridge's signature, as `native_lib::shared_bridge_wrapper` emits it.
pub(crate) type Bridge =
    extern "C" fn(*mut Stores, *const crate::host_abi::LibArg, usize, *mut crate::host_abi::LibArg);

/// Is the compiled standard library switched off (`LOFT_NO_COMPILED_STDLIB=1`)?
fn disabled() -> bool {
    crate::env_once!(std::env::var_os("LOFT_NO_COMPILED_STDLIB").is_some())
}

/// Route the compiled standard-library functions of an INTERPRETED program through their
/// bridges, before `byte_code`: sets `def.native` as `native_adopt::mark_exports` does for a
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
        || crate::cache::stdlib_fingerprint(default_dir)
            .is_none_or(|(_, h)| h != generated::SOURCE_HASH)
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

/// The functions the standard library carries a compiled body for, by stored name.
/// [`mark`] points their calls at those bodies after the program is parsed, so the driver
/// hands this set to the parser first (`Parser::compiled_bridges`).
#[must_use]
pub fn names() -> std::collections::HashSet<String> {
    generated::BRIDGES
        .iter()
        .map(|(n, _, _)| (*n).to_string())
        .collect()
}

/// The compiled bridge for a `loft_shared_…` symbol, when the standard library carries it.
#[must_use]
pub fn bridge(sym: &str) -> Option<*const ()> {
    generated::BRIDGES
        .iter()
        .find(|(_, s, _)| *s == sym)
        .map(|(_, _, f)| *f as *const ())
}
