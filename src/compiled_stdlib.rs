// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I86 — Startup cache & embedded stdlib: the standard library's compiled bodies as a C unit

//! @PLN181 / @PLN225 R1 — the standard library compiled like any other library.
//!
//! A `use`d library runs as a hosted C unit under an interpreted program (@C71, "libraries
//! compile, scripts interpret"; @PLN206): its functions are built into a shared object by the C
//! backend, and the interpreter calls them over the shared store.  The standard library takes
//! that same route, for the functions [`crate::native_gate::export_set`] picks (a loft body
//! that loops, the bridge carries its slots) and the hosted bridge carries:
//!
//! 1. **One unit for every program.**  The unit is generated from the standard library parsed
//!    alone, so it is the same whatever program asks first.  A unit names its types and
//!    functions by key and is mapped to the program's own numbers when it is loaded
//!    (`loft_shared_init`), so it holds for every program, whatever types follow the
//!    standard library's.
//! 2. **Built once, in the user cache.**  The unit is keyed on this loft build, the standard
//!    library's source and the switches a hosted unit is built under; the first interpreted
//!    run after any of them changes builds it with the machine's C compiler (`stdlib-units/`),
//!    and every later run adopts it.  Without a C compiler the standard library interprets:
//!    slower, never wrong, and nothing is said — there is nothing the user must do.
//!
//! `LOFT_NO_COMPILED_STDLIB=1` interprets the standard library's loft bodies (the A/B).

use crate::data::Data;
use crate::database::Stores;
use std::collections::HashSet;
use std::sync::OnceLock;

/// The unit this process adopts: its shared object and the functions it carries, by stored
/// name.  `None` when switched off, when no unit could be built here, or when the binary has no
/// bridge dispatcher.
struct Unit {
    so: std::path::PathBuf,
    names: HashSet<String>,
}

static UNIT: OnceLock<Option<Unit>> = OnceLock::new();

/// Is the compiled standard library switched off (`LOFT_NO_COMPILED_STDLIB=1`)?
fn disabled() -> bool {
    crate::env_once!(std::env::var_os("LOFT_NO_COMPILED_STDLIB").is_some())
}

/// The key of the standard library's unit: this loft build, the standard library's source as
/// it is in `default_dir`, and the switches a hosted unit is built under.
fn unit_key(default_dir: &str) -> Option<u64> {
    let source = if default_dir.is_empty() || !crate::file_access::is_dir(default_dir) {
        0
    } else {
        crate::cache::stdlib_fingerprint(default_dir)?.1
    };
    let parts: [&[u8]; 4] = [
        &crate::cache::loft_build_fingerprint().to_le_bytes(),
        &crate::cache::loft_exe_identity().to_le_bytes(),
        &source.to_le_bytes(),
        &[
            u8::from(crate::keys::no_handover()),
            u8::from(crate::keys::poison_claim()),
            u8::from(crate::keys::hosted_planted()),
        ],
    ];
    Some(crate::cache::stable_hash(&parts))
}

/// Adopt the standard library's unit, building it first when this machine has none for the
/// key.  Answers the functions it carries, by stored name: the driver names them to the parser
/// (`Parser::compiled_bridges`) before it reads a call to one, and [`mark`] routes those calls
/// after the parse.  Empty when switched off or when no unit could be had here.
#[must_use]
pub fn prepare(default_dir: &str) -> HashSet<String> {
    UNIT.get_or_init(|| unit(default_dir))
        .as_ref()
        .map(|u| u.names.clone())
        .unwrap_or_default()
}

/// The functions the adopted unit carries, by stored name (empty before [`prepare`]).
#[must_use]
pub fn names() -> HashSet<String> {
    UNIT.get()
        .and_then(Option::as_ref)
        .map(|u| u.names.clone())
        .unwrap_or_default()
}

fn unit(default_dir: &str) -> Option<Unit> {
    use crate::file_access as fa;
    if disabled() || !cfg!(feature = "native-extensions") {
        return None;
    }
    let fp = unit_key(default_dir)?;
    let out_dir = crate::cache::cache_base_dir().join("stdlib-units");
    let stem = format!("loft_stdlib_{fp:016x}");
    let so = out_dir.join(crate::opt::native_key::platform_cdylib_name(&stem));
    let names_file = out_dir.join(format!("{stem}.names"));
    let read_names = || -> Option<HashSet<String>> {
        if !fa::exists(&so) {
            return None;
        }
        let text = fa::read_to_string(&names_file).ok()?;
        Some(
            text.lines()
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
        )
    };
    if let Some(names) = read_names() {
        return Some(Unit { so, names });
    }
    // A failed build is remembered under the key, so a machine without a C compiler asks once.
    let failed = out_dir.join(format!("{stem}.failed"));
    if fa::exists(&failed) {
        return None;
    }
    fa::create_dir_all(&out_dir).ok()?;
    let lock_path = out_dir.join(format!("{stem}.lock"));
    let lock = fa::create(&lock_path).ok()?;
    lock.lock().ok()?;
    if let Some(names) = read_names() {
        return Some(Unit { so, names });
    }
    match build(default_dir, &out_dir, &stem, fp) {
        Ok(names) => {
            let mut sorted: Vec<&String> = names.iter().collect();
            sorted.sort_unstable();
            let text: String = sorted.iter().fold(String::new(), |mut t, n| {
                t.push_str(n);
                t.push('\n');
                t
            });
            let tmp = out_dir.join(format!("{stem}.names.{}.part", std::process::id()));
            fa::write(&tmp, text).ok()?;
            fa::rename(&tmp, &names_file).ok()?;
            Some(Unit { so, names })
        }
        Err(why) => {
            if crate::keys::hosted_verify() {
                eprintln!("loft: the standard library runs interpreted this run — {why}");
            }
            let _ = fa::write(&failed, why);
            None
        }
    }
}

/// Build the unit from the standard library parsed alone (then its scope pass, as a program's
/// compile runs it), and answer the functions it carries.
fn build(
    default_dir: &str,
    out_dir: &std::path::Path,
    stem: &str,
    fp: u64,
) -> Result<HashSet<String>, String> {
    let mut p = crate::parser::Parser::new();
    p.parse_stdlib(default_dir)
        .map_err(|e| format!("the standard library: {e}"))?;
    crate::scopes::check(&mut p.data, &mut p.database);
    let export = crate::native_gate::export_set(&p.data);
    let carried = crate::c::hosted::hosted_split(&p.data, &p.database, &export).0;
    if carried.is_empty() {
        return Err("no standard library function crosses the C bridge".into());
    }
    let names = carried
        .iter()
        .map(|&d| p.data.def(d).name().to_string())
        .collect();
    crate::c::hosted::build_artifact(
        &mut p.data,
        &mut p.database,
        &carried,
        default_dir,
        out_dir,
        stem,
        fp,
    )?;
    Ok(names)
}

/// Route the compiled standard-library functions of an INTERPRETED program through the unit's
/// bridges, before `byte_code`, as a library's are (`native_adopt::probe_and_mark_exports`),
/// and answer how many it marked, with the unit's path for the loader.  Marks nothing when
/// no unit was adopted ([`prepare`]), or when the unit does not load or names a type this
/// program lacks — those functions run their loft bodies interpreted.
pub fn mark(data: &mut Data, stores: &Stores) -> (usize, Option<std::path::PathBuf>) {
    let Some(Some(unit)) = UNIT.get() else {
        return (0, None);
    };
    let export: HashSet<u32> = unit
        .names
        .iter()
        .map(|n| data.def_nr(n))
        .filter(|&d| d != u32::MAX && data.def(d).code != crate::data::Value::Null)
        .collect();
    if export.len() != unit.names.len() {
        // The program's standard library is not the one the unit was built from.
        return (0, None);
    }
    let probe =
        crate::interp::native_adopt::probe_and_mark_exports(data, stores, &export, &unit.so);
    (probe.marked, (probe.marked > 0).then(|| unit.so.clone()))
}
