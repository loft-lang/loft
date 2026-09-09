// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the `#c` direct C-ABI caller (@PLN24 arc B).

//! @PLN24 arc B — the interpreter's caller for a `#c` binding.
//!
//! The `--native` backend (arc C) hands the declared signature to rustc, which
//! emits a typed `extern "C"` and gets the ABI right by construction. The
//! interpreter has no compiler at the call site, so it does what the plan's
//! architecture probe validated: resolve the symbol, then call it through a
//! **fixed set of per-arity trampolines** — `extern "C" fn(u64, …) -> u64` —
//! with every integer-class argument collapsed into one `u64` slot.
//!
//! The probe (`tests/fixtures/c_abi/`) is what makes that sound, and it is also
//! what bounds it:
//!
//! - **Arguments cross correctly**, at every arity, including across the
//!   register/stack boundary. A `u64` slot carries an `int`, a `long`, a
//!   pointer or a `char *` intact, because the callee reads the low bits it
//!   wants.
//! - **Returns do not.** A 32-bit C return leaves the upper half of the
//!   register unspecified; on x86-64 it zero-extends, so `-1` read back as a
//!   `u64` is 4294967295 — a plausible large positive, not a crash. Every
//!   return here is therefore truncated to its **declared** width and
//!   re-extended by [`narrow_return`]. That is the one place this file differs
//!   from a naive transmute-and-call, and it is the whole reason the
//!   declaration carries a signature.
//!
//! The trampolines cover arity 0..=32. Beyond that the binding is refused
//! rather than truncated — a wrong arity is silent (the probe called an arity-1
//! symbol through an arity-3 trampoline and got the right answer), so nothing
//! downstream would catch it.
//!
//! @PLN128 arc C — that ceiling is now the CONTRACT's, not this caller's. It was
//! 12 and it bound the interpreter alone, so `--native` (which hands the
//! signature to rustc and can emit any arity) bound shapes the interpreter
//! refused, and a library could ship a binding only half of loft could call.
//! The ladder was extended to 32 and `--native` is held to the same number:
//! unifying by RAISING rather than narrowing, so nothing that compiled stopped
//! compiling. See `c_signature::MAX_C_ARITY` and DESIGN_DECISIONS.md § C106.

#![cfg(feature = "native-extensions")]

use crate::c_signature::{CSignature, CType};

/// ONE rung of the ladder: transmute `f` to the arity the index list names and
/// call it.
///
/// The rung's function TYPE is derived from the same index list that supplies
/// the arguments — `rung!(@ty $i)` maps each index to one `u64` — so the arity
/// is written once per rung instead of twice.  It used to be spelled twice
/// (`extern "C" fn(u64, u64, u64) -> u64, 0, 1, 2`), and a rung whose type and
/// index list disagreed would have transmuted to the wrong arity with NOTHING
/// to catch it: the probe called an arity-1 symbol through an arity-3
/// trampoline and got the right answer, so there is no runtime signal.  One
/// list, no disagreement possible.
macro_rules! rung {
    (@ty $i:literal) => { u64 };
    ($ret:ty, $f:expr, $args:expr; $($i:literal),* $(,)?) => {{
        let g: extern "C" fn($(rung!(@ty $i)),*) -> $ret =
            unsafe { std::mem::transmute($f) };
        g($($args[$i]),*)
    }};
}

/// The arity ladder, written ONCE and expanded per RETURN class.
///
/// A sibling of [`rung`] rather than a macro nested inside it: the arity list
/// is the part that must not drift between the three callers, so it is spelled
/// once here and the return type is what varies.  See [`call_at_arity`] for why
/// arity is the only ARGUMENT dimension.
macro_rules! call_ladder {
    ($ret:ty, $f:expr, $args:expr) => {{
        let (f, args) = ($f, $args);
        Some(match args.len() {
            0 => {
                let g: extern "C" fn() -> $ret = unsafe { std::mem::transmute(f) };
                g()
            }
            1 => rung!($ret, f, args; 0),
            2 => rung!($ret, f, args; 0, 1),
            3 => rung!($ret, f, args; 0, 1, 2),
            4 => rung!($ret, f, args; 0, 1, 2, 3),
            5 => rung!($ret, f, args; 0, 1, 2, 3, 4),
            6 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5),
            7 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6),
            8 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7),
            9 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8),
            10 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9),
            11 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10),
            12 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11),
            13 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12),
            14 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13),
            15 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14),
            16 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15),
            17 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16),
            18 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17),
            19 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18),
            20 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19),
            21 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20),
            22 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21),
            23 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22),
            24 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23),
            25 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24),
            26 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25),
            27 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26),
            28 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27),
            29 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28),
            30 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29),
            31 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30),
            32 => rung!($ret, f, args; 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31),
            _ => return None,
        })
    }};
}

/// Call `f` with `args`, at the arity `args.len()` names. `None` when the arity
/// is past the ladder.
///
/// One transmute target per rung: every integer-class C type — `int`, `long`, a
/// pointer, `char *`, an enum — occupies one `u64`, so arity is the only
/// dimension. Thirty-three function types, no combinatorial explosion, and no
/// libffi.
///
/// Rungs 6 and 7 straddle the SysV x86-64 boundary — the first six integer
/// arguments travel in registers and the seventh onward on the stack — so those
/// two are the ones a hand-written trampoline is most likely to get wrong, and
/// the fixture pins both. Every rung above 7 is the same stack-passing shape as
/// 7, which is why extending the ladder is mechanical rather than a new risk
/// per rung.
///
/// The ceiling is [`MAX_C_ARITY`] and it is a **contract** number, not a
/// property of this caller: `--native` hands the signature to rustc and would
/// happily emit any arity, so the ladder is what both backends are held to
/// rather than what only this one can do (@PLN128 arc C).
///
/// # Safety
/// `f` must be a C function whose parameters are all integer-class and whose
/// arity equals `args.len()`. Nothing checks this at runtime — the probe called
/// an arity-1 symbol through an arity-3 trampoline and got the RIGHT answer, so
/// there is no runtime signal to check against. The declaration is the only
/// authority, which is why it is checked when it is parsed.
unsafe fn call_at_arity(f: *const (), args: &[u64]) -> Option<u64> {
    call_ladder!(u64, f, args)
}

/// The same ladder, transmuted to return a `double` (@PLN128 arc E).
///
/// A float RETURN and a float ARGUMENT are not the same problem, and the plan
/// settled them together once by mistake. An argument would need a rung per
/// SUBSET of positions that are float — the register file is chosen per
/// argument, so the family is `2^arity` and genuinely impossible. The return is
/// **one axis**: the value comes back in `xmm0` or it does not, so it costs one
/// more expansion of the same arity list. Every argument stays integer-class,
/// which is exactly the Fortran shape (everything by reference).
///
/// It is what makes the level-1 BLAS *functions* — `ddot_`, `dnrm2_`, `dasum_`
/// — and the LAPACK auxiliaries (`dlange_`, `dlamch_`) bindable at all. Before
/// it they were refused, and the cure the refusal named was an ANSI-C shim per
/// routine, which puts a C toolchain in the build of every numeric package to
/// work around a boundary that can just be correct.
///
/// # Safety
/// As [`call_at_arity`], and `f` must return a C `double`.
unsafe fn call_at_arity_f64(f: *const (), args: &[u64]) -> Option<f64> {
    call_ladder!(f64, f, args)
}

/// The `float` twin of [`call_at_arity_f64`] — `snrm2_`, `sdot_`, `sasum_`.
///
/// A separate rung set rather than a `f64` call narrowed afterwards: a C
/// `float` return leaves `xmm0` holding a single, and reading those bits as a
/// double is a denormal, not the number.
///
/// # Safety
/// As [`call_at_arity`], and `f` must return a C `float`.
unsafe fn call_at_arity_f32(f: *const (), args: &[u64]) -> Option<f32> {
    call_ladder!(f32, f, args)
}

/// Bring a raw return register back to a loft `integer`.
///
/// The register holds whatever the callee left there, and for a return narrower
/// than 64 bits the upper half is **not defined by the ABI** — x86-64 happens to
/// zero-extend, which turns a negative `int` into a large positive rather than
/// into anything that looks wrong. Truncating to the declared width and
/// re-extending by the declared signedness is the fix, and it only exists
/// because the declaration says what the width is.
#[must_use]
pub fn narrow_return(raw: u64, ret: &CType) -> i64 {
    match *ret {
        CType::Int {
            bits: 8,
            signed: true,
        } => i64::from(raw as u8 as i8),
        CType::Int {
            bits: 8,
            signed: false,
        } => i64::from(raw as u8),
        CType::Int {
            bits: 16,
            signed: true,
        } => i64::from(raw as u16 as i16),
        CType::Int {
            bits: 16,
            signed: false,
        } => i64::from(raw as u16),
        CType::Int {
            bits: 32,
            signed: true,
        } => i64::from(raw as u32 as i32),
        CType::Int {
            bits: 32,
            signed: false,
        } => i64::from(raw as u32),
        // 64-bit, and every pointer: the whole register is the value.
        _ => raw as i64,
    }
}

/// Every C library this program declared, each with the outcome of opening it:
/// `None` until it has been tried, then whether it opened.
///
/// Keeping the outcome (rather than draining the list) is what makes a MISSING
/// library cost one `dlopen` for the whole run instead of one per call — a
/// symbol that is genuinely absent is looked up on every iteration of whatever
/// loop calls it.
///
/// REQUIRED entries are in here too, and the miss path opens them like any
/// other. One rule for both backends is the point: the interpreter has already
/// opened them ([`register`]), where re-opening is a no-op that `load_one`
/// dedups by canonical path; a `--native` binary has opened nothing, and a
/// required library whose symbols are all resolved lazily may have lost its
/// `DT_NEEDED` entry to `--as-needed`. A flag consulted here would be a fourth
/// place to get the same fact wrong.
static DECLARED_LIBS: std::sync::Mutex<Vec<(crate::data::CLibrary, Option<bool>)>> =
    std::sync::Mutex::new(Vec::new());

/// Soname → the `#c` symbols declared by the package that declared it. Read by
/// [`library_available`], which is symbol-granular because a library that is
/// PRESENT but of the wrong vintage exports a subset — a file-granular answer
/// would say yes where the call still faults.
static LIB_SYMBOLS: std::sync::Mutex<Vec<(String, Vec<String>)>> =
    std::sync::Mutex::new(Vec::new());

/// Record the C libraries the program declared, for `resolve` to open on
/// demand. Called by [`register`] with the whole list, so a re-registration
/// (the REPL, the test runner) replaces it rather than appending.
pub fn set_declared_libraries(libs: Vec<crate::data::CLibrary>) {
    *DECLARED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        libs.into_iter().map(|l| (l, None)).collect();
}

/// Which declared libraries are EXPECTED to export `symbol`, when the source says so.
///
/// `library_symbol_table` attributes symbols per PACKAGE, and gives a multi-library
/// package an empty list rather than guessing — so this answers EMPTY exactly when the
/// source cannot attribute the symbol, and the caller must widen.
///
/// A package contributes one entry per artefact it owns — its `cc`-built shim AND its
/// declared soname — with the same symbol list on each, so more than one name can claim
/// a symbol. All of them are returned and the caller matches any; picking the first
/// selected the shim path, which no declared library is named after, and the narrowing
/// then never fired.
fn sonames_for_symbol(symbol: &str) -> Vec<String> {
    let guard = LIB_SYMBOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .iter()
        .filter(|(_, syms)| syms.iter().any(|s| s == symbol))
        .map(|(lib, _)| lib.clone())
        .collect()
}

/// Record which `#c` symbols each declared library is expected to provide.
pub fn set_library_symbols(table: Vec<(String, Vec<String>)>) {
    *LIB_SYMBOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = table;
}

/// Build the soname → declared-symbols table from a parsed program.
///
/// One home, called by both backends — the interpreter from [`register`] and
/// the `--native` generator, which bakes the result into the binary. Derived
/// from the same `Data` either way, so the two cannot answer differently.
///
/// **A symbol is attributable to a library only when that library is the only
/// one its package declares.** A `#c` annotation never names its library
/// (`Data::c_owner_pkg` is the finest attribution the source supports), so for a
/// package declaring several libraries there is no fact saying which one exports
/// a given symbol — and guessing produces the worst possible answer: a package
/// binding sqlite AND duckdb would report sqlite unavailable because a duckdb
/// symbol is missing, which is precisely the case optional libraries exist for.
///
/// So a multi-library package gets an empty symbol list, and its libraries are
/// available when they LOAD. Skew detection is the thing given up, and the way
/// to keep it is the arrangement libraries should have anyway: one package per
/// optional library, which is how the `sqldb` fixture is built.
#[must_use]
pub fn library_symbol_table(data: &crate::data::Data) -> Vec<(String, Vec<String>)> {
    let target = crate::c_signature::CTarget::host();
    let mut by_pkg: std::collections::HashMap<&str, Vec<String>> = std::collections::HashMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.c_sig.is_empty() || *def.code() != crate::data::Value::Null {
            continue;
        }
        let Some(Ok(sig)) = crate::c_signature::of(data, d_nr, target) else {
            continue;
        };
        if let Some(pkg) = data.c_owner_pkg(&def.position().file) {
            by_pkg.entry(pkg).or_default().push(sig.symbol);
        }
    }
    data.c_libraries
        .iter()
        .map(|lib| {
            // Only OTHER OPTIONAL libraries make attribution ambiguous. A
            // required entry cannot be the reason a symbol is missing — the
            // package does not load at all without it — and one of them is
            // almost always the package's own `[c] shim`, which loft just
            // built and which is therefore present by construction. Counting
            // those would switch skew detection off for nearly every real
            // package, since nearly every `#c` package ships a shim.
            let alone = data
                .c_libraries
                .iter()
                .filter(|c| c.pkg_dir == lib.pkg_dir && c.optional)
                .count()
                <= 1;
            let mut syms = if alone {
                by_pkg
                    .get(lib.pkg_dir.as_str())
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            syms.sort_unstable();
            syms.dedup();
            (lib.name.clone(), syms)
        })
        .collect()
}

/// Open any declared library not yet tried. Returns whether this call opened
/// one — i.e. whether the set of resolvable symbols just grew, which is the
/// only reason for the caller to look again.
///
/// `only` narrows it to one library by name. A symbol MISS has no name to give —
/// a `#c` annotation never says which library it comes from, so the miss path
/// widens the search across all of them (arc B: one resolver, one meaning per
/// symbol). An availability QUESTION does have one, and answering it by opening
/// every optional library made `c_library_available("libsqlite3.so.0")` dlopen
/// libduckdb as a side effect — 70 MB mapped by a program that never mentions
/// duckdb, which is the cost arc G exists to avoid.
fn open_pending_optional_named(only: Option<&[String]>) -> bool {
    // The entries are cloned out and the guard dropped before any `dlopen`: an
    // optional library's initialisers run arbitrary C, and holding the lock
    // across that would serialise every parallel arm resolving a symbol (the
    // shape P245 fixed in `native_auto_dispatch`).
    let pending: Vec<crate::data::CLibrary> = {
        let mut guard = DECLARED_LIBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter_mut()
            .filter(|(lib, tried)| tried.is_none() && only.is_none_or(|ns| ns.contains(&lib.name)))
            .map(|(lib, _)| lib.clone())
            .collect()
    };
    if pending.is_empty() {
        return false;
    }
    let mut opened_any = false;
    for lib in pending {
        let ok = crate::extensions::load_c_library(&lib.name, &lib.pkg_dir);
        opened_any |= ok;
        let mut guard = DECLARED_LIBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (entry, tried) in guard.iter_mut() {
            if entry.name == lib.name && entry.pkg_dir == lib.pkg_dir {
                *tried = Some(ok);
            }
        }
    }
    opened_any
}

/// Resolve a C symbol for the running process.
///
/// Searches the cdylibs loft has already loaded, then the process itself —
/// which is what makes a libc binding work with nothing declared and nothing
/// installed. A declared REQUIRED library is already open by the time this runs
/// ([`register`]); an OPTIONAL one is opened here, on the miss, which is the
/// whole of arc G's interpreter half.
///
/// One resolver, so a symbol cannot mean two different things depending on
/// which caller asked for it — arc B's rule, and the lazy path keeps it by
/// widening the search rather than adding a second search.
#[must_use]
pub fn resolve(symbol: &str) -> Option<*const ()> {
    if let Some(p) = resolve_in_loaded(symbol) {
        return Some(p);
    }
    // Miss: an optional library that has not been opened yet may export it.
    //
    // Try the library the SOURCE attributes this symbol to first. Widening to every
    // declared library still happens below when that does not answer — the arc B rule
    // is unchanged, a symbol still means one thing — but it is no longer the first
    // move, because it dlopens libraries the program never mentions. With four
    // one-library packages declared, resolving `sqlite3_open` mapped libduckdb's 70 MB
    // as a side effect, which is the cost `[c] optional-libs` exists to avoid.
    let attributed = sonames_for_symbol(symbol);
    if !attributed.is_empty()
        && open_pending_optional_named(Some(&attributed))
        && let Some(p) = resolve_in_loaded(symbol)
    {
        return Some(p);
    }
    if open_pending_optional_named(None) {
        return resolve_in_loaded(symbol);
    }
    None
}

/// @PLN24 arc G — [`resolve`], for a `--native` binary.
///
/// A compiled program never runs [`register`], so nothing has told it which
/// libraries may be opened on demand; the generated crate carries the list as a
/// static and hands it here on the first lazy call. After that this IS
/// [`resolve`] — the two backends share the resolver, which is the only way
/// they can agree about what a symbol means.
///
/// The list carries EVERY declared library, not just the optional ones: a
/// required library is normally a `DT_NEEDED` entry and already in the process,
/// but with its symbols resolved lazily there may be no undefined reference
/// left for the linker to keep it alive (`--as-needed`), and re-opening it by
/// name costs nothing when it is already there.
#[must_use]
pub fn resolve_native(
    symbol: &str,
    libs: &[(&str, &str)],
    syms: &[(&str, &[&str])],
) -> Option<*const ()> {
    register_native(libs, syms);
    resolve(symbol)
}

/// Hand a `--native` binary's baked-in C-library tables to the runtime.
///
/// Idempotent, and called from two places for one reason each: the generated
/// `main` calls it so [`library_available`] can answer in a program that never
/// makes a lazy call at all, and [`resolve_native`] calls it so a `#c` call
/// reaching the runtime BEFORE `main`'s prelude still resolves.
pub fn register_native(libs: &[(&str, &str)], syms: &[(&str, &[&str])]) {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        set_declared_libraries(
            libs.iter()
                .map(|(name, pkg_dir)| crate::data::CLibrary {
                    name: (*name).to_string(),
                    pkg_dir: (*pkg_dir).to_string(),
                    // The flag decides eager loading and the link line, both of
                    // which are already settled by the time a binary exists.
                    // Nothing downstream of here reads it.
                    optional: true,
                })
                .collect(),
        );
        set_library_symbols(
            syms.iter()
                .map(|(lib, s)| {
                    (
                        (*lib).to_string(),
                        s.iter().map(|x| (*x).to_string()).collect(),
                    )
                })
                .collect(),
        );
    });
}

/// @PLN24 arc G — is this C library usable RIGHT NOW?
///
/// True when the library opens **and** every `#c` symbol declared against it
/// resolves. Both halves are load-bearing: a library of the wrong vintage opens
/// and exports a subset, so a file-granular answer would say yes where the call
/// still faults — the version-skew hole that makes a naive query worse than no
/// query at all.
///
/// A library the program never declared answers false rather than probing the
/// dynamic linker for it. Asking about one is a program bug, and false is the
/// answer that keeps a caller on its fallback path instead of into a fault
/// (C80: no runtime errors, ever).
/// [`library_available`], for generated Rust — the main binary AND every
/// auto-built package cdylib.
///
/// A cdylib links its own copy of loft, so the tables [`register`] filled in
/// the interpreter's process are NOT the tables a function compiled into a
/// package can see: measured, `duckdb_available()` answered false from inside
/// the package while the identical call from the program answered true. The
/// generated source carries the tables, so passing them is what makes one
/// question have one answer wherever it is asked from.
#[must_use]
pub fn library_available_native(
    name: &str,
    libs: &[(&str, &str)],
    syms: &[(&str, &[&str])],
) -> bool {
    register_native(libs, syms);
    library_available(name)
}

#[must_use]
pub fn library_available(name: &str) -> bool {
    open_pending_optional_named(Some(std::slice::from_ref(&name.to_string())));
    let opened = {
        let guard = DECLARED_LIBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter()
            .filter(|(lib, _)| lib.name == name)
            .map(|(_, tried)| tried.unwrap_or(false))
            .fold(None, |acc: Option<bool>, ok| {
                Some(acc.unwrap_or(false) || ok)
            })
    };
    if opened != Some(true) {
        return false;
    }
    let symbols = {
        let guard = LIB_SYMBOLS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter()
            .find(|(lib, _)| lib == name)
            .map(|(_, s)| s.clone())
            .unwrap_or_default()
    };
    symbols.iter().all(|s| resolve(s).is_some())
}

/// What a `#c` call says when its symbol cannot be resolved.
///
/// One text for both backends: the interpreter panics with it from `dispatch`,
/// and the `--native` emission writes a call to this same function into the
/// generated crate. A message that differed between them would be a divergence
/// in the one place a user is already having a bad day.
#[must_use]
pub fn missing_symbol_message(symbol: &str) -> String {
    format!(
        "`#c` symbol '{symbol}' not found — it is not in this process and no loaded \
         library exports it. If it comes from an `[c] optional-libs` library, that \
         library is not installed: ask `c_library_available(\"<soname>\")` before \
         calling. Otherwise declare the library it comes from, or check the spelling"
    )
}

/// The search itself, over what is already open. Split out so the miss path can
/// re-run exactly the same search after widening it, rather than a similar one.
fn resolve_in_loaded(symbol: &str) -> Option<*const ()> {
    if let Some((p, _)) = crate::extensions::try_dlsym_pub(symbol) {
        return Some(p);
    }
    #[cfg(unix)]
    {
        use libloading::os::unix::Library;
        // `Library::this()` is the process handle: symbols already linked in
        // (libc) and anything loaded with global visibility.
        let this = Library::this();
        let mut name = symbol.to_string();
        name.push('\0');
        if let Ok(sym) = unsafe { this.get::<*const ()>(name.as_bytes()) } {
            return Some(*sym);
        }
    }
    #[cfg(windows)]
    {
        use libloading::os::windows::Library;
        // There is no process-wide symbol table on Windows: `GetProcAddress`
        // answers per MODULE, and the C runtime is its own DLL rather than
        // something linked into the executable's export table. So the Unix
        // `Library::this()` step has no direct twin — searching only the
        // executable finds nothing, which is why `strlen` and `atoi` were
        // unresolvable and the whole `#c`-against-libc surface was Linux/macOS
        // only.
        //
        // Ask the modules the process ALREADY has open, in the order a C
        // symbol is most likely to live: the executable itself (its own
        // exports), then the UCRT, then the legacy CRT shim, then the Win32
        // base DLLs. `open_already_loaded` is `GetModuleHandle` — it never
        // loads anything, so this widens the search without changing what the
        // process has mapped.
        if let Ok(this) = Library::this()
            && let Ok(sym) = unsafe { this.get::<*const ()>(symbol.as_bytes()) }
        {
            return Some(*sym);
        }
        for module in [
            "ucrtbase.dll",
            "api-ms-win-crt-string-l1-1-0.dll",
            "api-ms-win-crt-convert-l1-1-0.dll",
            "api-ms-win-crt-stdio-l1-1-0.dll",
            "api-ms-win-crt-heap-l1-1-0.dll",
            "msvcrt.dll",
            "kernel32.dll",
        ] {
            if let Ok(lib) = Library::open_already_loaded(module)
                && let Ok(sym) = unsafe { lib.get::<*const ()>(symbol.as_bytes()) }
            {
                return Some(*sym);
            }
        }
    }
    None
}

/// Everything the interpreter needs to make one `#c` call, resolved once at
/// wiring time rather than per call.
#[derive(Clone)]
pub struct CBinding {
    pub sig: CSignature,
    /// The loft parameter types, in declaration order — what decides how each
    /// argument is popped off the stack. Kept beside the C signature rather
    /// than re-derived, because the two together are the mapping and either
    /// alone is half of it.
    pub loft_params: Vec<crate::data::Type>,
    /// How each of those parameters is realised in C slots, from
    /// [`crate::c_signature::plan`] — the one place that answer is derived, so
    /// this caller and the `--native` emission cannot disagree about whether a
    /// vector carries a count (@PLN128 arc D).
    pub arg_plan: Vec<crate::c_signature::CArg>,
    pub void_return: bool,
    /// A `char *` return bound to loft `text`, which comes back through the
    /// destination record rather than the value stack (@PLN24 arc D).
    pub text_return: bool,
}

/// Side table: library index -> the binding that index calls. Populated by
/// [`register`], read by [`dispatch`].
static C_BINDINGS: std::sync::Mutex<Option<std::collections::HashMap<u16, CBinding>>> =
    std::sync::Mutex::new(None);

/// Wire every `#c` declaration into the interpreter's static-call table.
///
/// The call site needs no change: a body-less definition with no `#native`
/// symbol already resolves through `library_names` under its OWN name
/// (`state/codegen.rs`, the `lib_lookup` fallback), so registering under that
/// name is what routes the call here.
pub fn register(state: &mut crate::state::State, data: &crate::data::Data) {
    let target = crate::c_signature::CTarget::host();
    // @PLN24 arc D — open what the program declared, BEFORE any symbol is
    // looked up: `resolve` searches loaded libraries first, so a declared
    // library has to be loaded by the time a call happens. A failure is left
    // for the call site to report, where the binding that needed it can be
    // named.
    // @PLN24 arc G — an OPTIONAL library is deliberately not opened here. It is
    // opened by `resolve`'s miss path, when a symbol that needs it is first
    // looked up, so a program that never calls into it runs on a machine where
    // it is not installed.
    for lib in data.c_libraries.iter().filter(|c| !c.optional) {
        crate::extensions::load_c_library(&lib.name, &lib.pkg_dir);
    }
    // The miss path and the availability query both work off the WHOLE list:
    // re-opening an already-open required library is a no-op, and the query has
    // to be able to answer about any declared library, not only optional ones.
    set_declared_libraries(data.c_libraries.clone());
    set_library_symbols(library_symbol_table(data));
    let mut table = std::collections::HashMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.c_sig.is_empty() || *def.code() != crate::data::Value::Null {
            continue;
        }
        let Some(Ok(sig)) = crate::c_signature::of(data, d_nr, target) else {
            continue; // reported at the declaration
        };
        let loft_params: Vec<crate::data::Type> = def
            .attributes()
            .iter()
            .filter(|a| !a.name.starts_with("__") && !a.name.starts_with('#'))
            .map(|a| a.typedef.clone())
            .collect();
        // A declaration whose shapes do not fit was already reported at the
        // declaration; skipping it here leaves the call site to report the
        // missing binding rather than marshalling against a guess.
        let Ok(arg_plan) = crate::c_signature::plan(&loft_params, &sig) else {
            continue;
        };
        let binding = CBinding {
            sig,
            loft_params,
            arg_plan,
            void_return: matches!(def.returned(), crate::data::Type::Void),
            text_return: crate::state::codegen::is_c_text_call(def),
        };
        state.static_fn(def.name(), dispatch);
        if let Some(&idx) = state.library_names.get(def.name()) {
            table.insert(idx, binding);
        }
    }
    *C_BINDINGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(table);
}

/// Make one `#c` call: pop the loft arguments, marshal them into `u64` slots,
/// call through the trampoline, bring the return back at its declared width.
///
/// Arguments pop in REVERSE declaration order (the stack is LIFO), which is why
/// the slots are built backwards and reversed — the same shape every native
/// handler in `native.rs` uses.
fn dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    use crate::c_signature::CArg;
    use crate::keys::{DbRef, Str};

    let idx = crate::extensions::current_lib_idx();
    let binding = {
        let guard = C_BINDINGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref().and_then(|t| t.get(&idx)) {
            Some(b) => b.clone(),
            None => panic!("no `#c` binding registered for library index {idx}"),
        }
    };

    // The NUL-terminated copies a C string argument needs. Held here so they
    // outlive the call: loft text is UTF-8 plus a LENGTH, and C reads to the
    // first NUL, so the copy is what makes the two the same thing.
    let mut owned: Vec<Vec<u8>> = Vec::new();
    let mut slots: Vec<u64> = Vec::with_capacity(binding.sig.params.len());
    for arg in binding.arg_plan.iter().rev() {
        match arg {
            CArg::TextPointer => {
                let s = stores.get::<Str>(stack);
                let bytes = s.str().as_bytes();
                let mut b = Vec::with_capacity(bytes.len() + 1);
                b.extend_from_slice(bytes);
                b.push(0u8);
                slots.push(b.as_ptr() as u64);
                owned.push(b);
            }
            CArg::VectorPtr | CArg::VectorPtrCount => {
                // A loft vector is an OUTER record whose word at (rec, pos)
                // names the data record; the elements start at byte 8 and the
                // count sits at byte 4. Pushed as count-then-pointer because
                // the slots are reversed below.
                let r = stores.get::<DbRef>(stack);
                let data_rec = if r.rec == 0 || r.pos == 0 {
                    0
                } else {
                    stores.store(&r).get_u32_raw(r.rec, r.pos)
                };
                let (ptr, count) = if data_rec == 0 {
                    (0u64, 0u64)
                } else {
                    let st = stores.store(&r);
                    let count = u64::from(st.get_u32_raw(data_rec, 4));
                    // Valid for the duration of the call only — the arena may
                    // move on the next claim, which is the contract the
                    // declaration's mapping states.
                    let p = std::ptr::from_ref(st.addr::<u8>(data_rec, 8)) as u64;
                    (p, count)
                };
                // @PLN128 arc D — the count goes only where the C signature has
                // a parameter for it. A Fortran routine takes each argument as
                // a bare pointer, so pushing a count there would land it where
                // the callee expects the NEXT pointer.
                if *arg == CArg::VectorPtrCount {
                    slots.push(count);
                }
                slots.push(ptr);
            }
            CArg::Scalar => {
                slots.push(stores.get::<i64>(stack) as u64);
            }
        }
    }
    slots.reverse();

    let Some(f) = resolve(&binding.sig.symbol) else {
        panic!("{}", missing_symbol_message(&binding.sig.symbol));
    };
    // @PLN128 arc E — a float return goes down its own rung, because the value
    // is in `xmm0` and no amount of casting an integer register reaches it.
    // Taken before the integer path so the arity failure below is reported once,
    // in the same words, whichever class the return is.
    if let CType::Float { bits } = binding.sig.ret {
        let ok = if bits == 32 {
            (unsafe { call_at_arity_f32(f, &slots) }).map(|v| stores.put(stack, v))
        } else {
            (unsafe { call_at_arity_f64(f, &slots) }).map(|v| stores.put(stack, v))
        };
        assert!(
            ok.is_some(),
            "{}",
            over_arity_message(&binding, slots.len())
        );
        return;
    }
    let Some(raw) = (unsafe { call_at_arity(f, &slots) }) else {
        panic!("{}", over_arity_message(&binding, slots.len()));
    };

    if binding.void_return {
        return;
    }
    if binding.text_return {
        // The dest was stashed by `n_set_bridge_dest`, which the call site emits
        // immediately before this one (`gen_cdylib_text_dest_call`). Write into
        // it and push nothing — the codegen's stack accounting expects exactly
        // that, the same contract the cdylib bridge honours.
        let Some(dest) = stores.bridge_text_dest.take() else {
            panic!(
                "`#c` text binding '{}' was called with no destination — a text return \
                 reaches C only through `gen_cdylib_text_dest_call`",
                binding.sig.symbol
            );
        };
        let s = c_text(raw);
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&s);
        return;
    }
    // The declared width is what makes this right; read raw, a negative `int`
    // would arrive as a large positive.
    stores.put(stack, narrow_return(raw, &binding.sig.ret));
}

/// The message for a call past the ladder, worded once.
///
/// Every return class ends here on the same failure, and a caller who hit the
/// ceiling should not be told a different story depending on whether the symbol
/// answers an integer or a double.
fn over_arity_message(binding: &CBinding, slots: usize) -> String {
    format!(
        "`#c` symbol '{}' needs {slots} arguments; the caller covers 0..={}. Wrap it \
         in an ANSI-C shim with fewer parameters",
        binding.sig.symbol,
        crate::c_signature::MAX_C_ARITY
    )
}

/// Bring a C `char *` return back as loft text.
///
/// Three decisions, and both backends make the same three (`--native` emits the
/// twin of this in `output_c_direct_call`) — a text return that differed between
/// them would be exactly the divergence arc C was built to stop:
///
/// - **NULL is loft null.** text-null is CONTENT-based (`STRING_NULL`), so the
///   record carries it and `??` / `!` / `match` read it as null.
/// - **The bytes end at the first NUL**, because that is what `char *` means. A
///   loft text carries a length and can hold an interior NUL; a C string cannot,
///   so the crossing truncates there rather than inventing a length.
/// - **Invalid UTF-8 is replaced, not refused** — loft text is UTF-8, and a
///   locale-encoded byte from C must not take the program down (C80).
///
/// The pointer is COPIED and never freed. See the return check in
/// `c_signature::check` for why borrowed is the only safe default.
#[must_use]
fn c_text(raw: u64) -> String {
    if raw == 0 {
        return crate::state::STRING_NULL.to_string();
    }
    let p = raw as *const std::ffi::c_char;
    // SAFETY: the declaration says this symbol returns `char *`, and that
    // declaration is the sole authority (there is no runtime signal to check it
    // against — the plan's central measurement). A non-NUL pointer from a
    // correctly declared symbol points at a NUL-terminated string.
    let bytes = unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes();
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's central finding, as a unit: a 32-bit C return read raw is a
    /// plausible large POSITIVE, and only the declared width recovers it.
    #[test]
    fn a_narrow_return_is_recovered_by_its_declared_width() {
        // What the register actually holds after `int f() { return -1; }` on
        // x86-64: the low half is 0xFFFFFFFF, the upper half zero-extended.
        let raw = 0x0000_0000_FFFF_FFFF_u64;
        assert_eq!(
            raw as i64, 4_294_967_295,
            "read raw, it is a large positive"
        );
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: true
                }
            ),
            -1,
            "read at its declared width, it is -1"
        );
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: false
                }
            ),
            4_294_967_295,
            "and an unsigned declaration means the large positive was right"
        );
    }

    #[test]
    fn every_narrow_width_round_trips_its_sign() {
        for (bits, neg) in [(8u8, -5i64), (16, -300), (32, -70000)] {
            let signed = CType::Int { bits, signed: true };
            let raw = neg as u64;
            assert_eq!(narrow_return(raw, &signed), neg, "{bits}-bit signed");
        }
        assert_eq!(
            narrow_return(
                u64::MAX,
                &CType::Int {
                    bits: 64,
                    signed: true
                }
            ),
            -1
        );
    }

    /// libc is linked into this test binary, so resolution is checkable with
    /// nothing installed — and a symbol that does not exist must answer None
    /// rather than a stale or null pointer.
    #[test]
    fn resolves_a_process_symbol_and_refuses_a_missing_one() {
        assert!(resolve("strlen").is_some(), "libc is in the process");
        assert!(resolve("loft_definitely_not_a_symbol_9z").is_none());
    }

    /// The trampolines, exercised against libc directly — the same thing the
    /// interpreter will do, without the interpreter in the way.
    #[test]
    fn the_trampolines_call_libc() {
        let strlen = resolve("strlen").expect("strlen");
        let s = c"hello";
        let n = unsafe { call_at_arity(strlen, &[s.as_ptr() as u64]) }.expect("arity 1");
        assert_eq!(n, 5);

        let atoi = resolve("atoi").expect("atoi");
        let neg = c"-1";
        let raw = unsafe { call_at_arity(atoi, &[neg.as_ptr() as u64]) }.expect("arity 1");
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: true
                }
            ),
            -1,
            "atoi returns a 32-bit int; raw it reads {raw}"
        );

        let abs = resolve("abs").expect("abs");
        let raw = unsafe { call_at_arity(abs, &[(-7i64) as u64]) }.expect("arity 1");
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: true
                }
            ),
            7
        );
    }

    /// Beyond the ladder the answer is None, never a truncated call: a wrong
    /// arity is SILENT (the probe got the right answer from an arity-3 call to
    /// an arity-1 symbol), so nothing downstream would catch it.
    #[test]
    fn an_arity_past_the_ladder_is_refused_not_truncated() {
        let f = resolve("abs").expect("abs");
        let too_many = vec![0u64; crate::c_signature::MAX_C_ARITY + 1];
        assert!(unsafe { call_at_arity(f, &too_many) }.is_none());
        assert!(unsafe { call_at_arity(f, &[0u64; crate::c_signature::MAX_C_ARITY]) }.is_some());
    }

    /// @PLN24 arc G — the construct `--native` emits for an optional library's
    /// symbol, proven here before the generator was taught to write it.
    ///
    /// A lazily resolved pointer must keep the one thing arc C bought: the
    /// DECLARED width, applied by rustc at the ABI. Transmuted to the typed
    /// signature, `atoi("-1")` is -1 — the cell arc C used, now reached through
    /// a pointer resolved at run time rather than a linked `extern "C"`.
    ///
    /// The width-blind alternative is not asserted here, because it has no
    /// hand-computable value to assert: a C `int` return leaves the upper half
    /// of `rax` UNSPECIFIED (measured on this host: `u64::MAX`, not the
    /// zero-extended 4294967295 the interpreter's trampoline reads). That is a
    /// stronger reason to transmute to the declared signature than a wrong
    /// constant would be, and the deterministic half of the claim is
    /// `a_narrow_return_is_recovered_by_its_declared_width`, which builds the
    /// raw register value by hand.
    #[test]
    fn a_lazily_resolved_pointer_keeps_its_declared_width() {
        let p = resolve("atoi").expect("atoi is in the process");
        let typed: unsafe extern "C" fn(*const std::ffi::c_char) -> i32 =
            unsafe { std::mem::transmute::<*const (), _>(p) };
        assert_eq!(unsafe { typed(c"-1".as_ptr()) }, -1);
        assert_eq!(unsafe { typed(c"2147483647".as_ptr()) }, i32::MAX);
    }

    /// The miss path, end to end: a symbol that is in NO loaded library and NOT
    /// in the process resolves only once its library has been opened on demand.
    ///
    /// The control is what makes it a measurement rather than an agreement with
    /// itself — the same symbol is unresolvable BEFORE the optional library is
    /// declared, so a harness that always answered `Some` would fail here.
    #[test]
    fn an_optional_library_opens_on_the_miss_that_needs_it() {
        let so = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/c_abi/liblc_types.so"
        );
        if !std::path::Path::new(so).exists() {
            // The fixture is built by `make` in that directory; without it this
            // test has nothing to measure and must not read as a pass.
            eprintln!("SKIP: {so} not built");
            return;
        }

        set_declared_libraries(Vec::new());
        assert!(
            resolve("lc_strlen").is_none(),
            "control: the fixture's symbol is not in the process"
        );

        set_declared_libraries(vec![crate::data::CLibrary {
            name: so.to_string(),
            pkg_dir: String::new(),
            optional: true,
        }]);
        let p = resolve("lc_strlen").expect("resolved after the miss opened the library");

        let f: unsafe extern "C" fn(*const std::ffi::c_char) -> i64 =
            unsafe { std::mem::transmute::<*const (), _>(p) };
        assert_eq!(
            unsafe { f(c"hello".as_ptr()) },
            5,
            "and the call through it is right"
        );
    }
}
