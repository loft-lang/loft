// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! @PLN203 C1 — `--native` through the C backend: the unit `loft::c::emit_program` writes,
//! built by the host's C compiler into a binary cached by content beside the program, as the
//! rustc path caches its own (`.loft/cache/`, the same safety checks and the same atomic
//! publish).  Selected by `--native-c` or `LOFT_NATIVE_BACKEND=c` until the flip (C9).

use loft::file_access as fa;
use loft::platform::process::{Program, Spawn};

/// How a C build is compiled: the three tiers `--native`, `--ship` and
/// `--native-debug` name on the rustc path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tier {
    /// A semantics run: a quick compile, every check kept.
    Check,
    /// A shipped binary: fully optimised.
    Release,
    /// Debug info and no optimisation, for a debugger.
    Debug,
}

/// Is the C backend the one `--native` drives?  `LOFT_NATIVE_BACKEND=c` (or `--native-c`,
/// which the caller folds in).  Any other value of the variable is the rustc writer.
pub(crate) fn c_backend_selected() -> bool {
    std::env::var("LOFT_NATIVE_BACKEND").is_ok_and(|v| v.eq_ignore_ascii_case("c"))
}

/// The C compiler: `LOFT_CC`, else `CC`, else the host's own (`platform::host_c_compiler`:
/// `cc`, or on Windows without one the MSVC `cl` rustc's own target links with).
pub(crate) fn cc_program() -> String {
    loft::platform::c_unit_compiler()
}

/// A compiler's command line: gcc's (`cc`, `gcc`, `clang`) or MSVC's (`cl`, `clang-cl`), told
/// apart by the compiler's name.  The flags are written once, in gcc's spelling, and
/// [`msvc_args`] renders them for the other.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dialect {
    Gnu,
    Msvc,
}

fn dialect(cc: &str) -> Dialect {
    if loft::platform::c_compiler_is_msvc(cc) {
        Dialect::Msvc
    } else {
        Dialect::Gnu
    }
}

/// gcc-spelled flags as `cl` takes them: C11 is the oldest standard `cl` names and holds C99's
/// language as the unit uses it; the warning switches become `cl`'s levels; the libraries a
/// gcc link names (`-lm`, `-pthread`, `-ldl`) are the CRT's own there, and a run path has no
/// meaning to a PE binary.
fn msvc_args(gnu: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for f in gnu {
        let mapped = match f.as_str() {
            "-std=c99" => "/std:c11",
            "-w" => "/W0",
            "-Wall" => "/W3",
            "-Werror" => "/WX",
            "-O0" => "/Od",
            "-O1" => "/O1",
            "-O2" | "-O3" => "/O2",
            "-g" => "/Z7",
            "-pthread" | "-ldl" | "-lm" | "-pedantic" => continue,
            other if other.starts_with("-W") => continue,
            other if let Some(def) = other.strip_prefix("-D") => {
                out.push(format!("/D{def}"));
                continue;
            }
            other if let Some(lib) = other.strip_prefix("-l") => {
                out.push(format!("{lib}.lib"));
                continue;
            }
            other => other,
        };
        out.push(mapped.to_string());
    }
    out
}

/// The compiler as a process, with the environment an MSVC `cl` needs outside a developer
/// prompt (`platform::msvc_env`: empty for gcc's dialect, or where the prompt set it).
fn cc_spawn(cc: &str) -> Spawn {
    let mut s = Spawn::new(Program::os(cc));
    if dialect(cc) == Dialect::Msvc {
        for (k, v) in loft::platform::msvc_env() {
            s.push_env(k, v);
        }
    }
    s
}

/// The compiler flags of one build: the tier's optimisation and call chain, and the store's
/// poison modes when the environment asks for them as the interpreter's switches do.  Every flag is part of
/// the cache key, since each one changes the binary.
fn cc_flags(tier: Tier) -> Vec<String> {
    let mut flags: Vec<String> = vec!["-std=c99".into()];
    match lint() {
        Lint::Off => flags.push("-w".into()),
        Lint::All => flags.extend([
            "-Wall".to_string(),
            "-Wextra".to_string(),
            "-DLOFT_LINT_RUNTIME_UNUSED".to_string(),
        ]),
        on => {
            flags.extend(LINT_WARNINGS.iter().map(|w| (*w).to_string()));
            if on == Lint::Strict {
                flags.push("-Werror".into());
            }
        }
    }
    flags.extend(
        loft::platform::c_thread_link_args()
            .iter()
            .map(|a| (*a).to_string()),
    );
    // A fault in a semantics or a debug build names the frames it fired under; a shipped
    // binary compiles the pushes away, as the Rust writer's lean tier drops its frame names.
    match tier {
        Tier::Check => flags.extend(["-O1".into(), "-DLOFT_CALL_CHAIN".into()]),
        Tier::Release => flags.push("-O3".into()),
        Tier::Debug => flags.extend(["-O0".into(), "-g".into(), "-DLOFT_CALL_CHAIN".into()]),
    }
    if env_on("LOFT_POISON") {
        flags.push("-DLOFT_POISON".into());
    }
    if env_on("LOFT_POISON_CLAIM") {
        flags.push("-DLOFT_POISON_CLAIM".into());
    }
    // `LOFT_STRICT_STORES`: a freed store's slot is never made again and an access through it
    // is reported, and a store live at exit too; either exits 1 (loft_rt.h).
    if env_on("LOFT_STRICT_STORES") {
        flags.push("-DLOFT_STRICT_STORES".into());
    }
    // `LOFT_DOUBLE_FREE` (any value, as `keys::double_free_audit` reads it): the census of
    // re-frees and of freed slots made again, each naming the sites (loft_rt.h).
    if std::env::var_os("LOFT_DOUBLE_FREE").is_some() {
        flags.push("-DLOFT_DOUBLE_FREE".into());
    }
    // `LOFT_KEYED_VERIFY`: every keyed fast path the C has checked against its general form —
    // the hash one-probe insert, the index's exact descent (loft_hash.h, loft_btree.h).
    if env_on("LOFT_KEYED_VERIFY") {
        flags.push("-DLOFT_KEYED_VERIFY".into());
    }
    // `LOFT_COPY_CHECK`: after every deep copy, each nested collection's length compared with
    // its source's (loft_rt.h `loft_report_copy_mismatches`).
    if env_on("LOFT_COPY_CHECK") {
        flags.push("-DLOFT_COPY_CHECK".into());
    }
    // `LOFT_LEAK_SITES` (any value, as the interpreter reads it): a store left at exit is named
    // by where it was made (loft_rt.h `loft_note_site`, loft_log.h's report).
    if std::env::var_os("LOFT_LEAK_SITES").is_some() {
        flags.push("-DLOFT_LEAK_SITES".into());
    }
    // The allocator's bisect switches (store.rs; loft_store.h), read as `Store` reads them: set
    // and not `0`.  Each turns a policy off in the C as it does in the interpreter.
    for switch in [
        "LOFT_NO_PARK",
        "LOFT_NO_LAZY_FREE",
        "LOFT_NO_TOMBSTONE",
        "LOFT_NO_FREE_FOOTER",
        "LOFT_NO_WILD_BUMP",
        "LOFT_NO_CARVE_IN_PLACE",
        "LOFT_NO_WILDERNESS",
    ] {
        if std::env::var(switch).is_ok_and(|v| v != "0") {
            flags.push(format!("-D{switch}"));
        }
    }
    flags
}

/// `LOFT_C_LINT`: the C a program is written as compiled with the warnings below instead of
/// `-w`, on request — `1` reports them, `strict` makes each one an error, `all` adds the
/// `unused` family.  Never on by default: a user does not read warnings about C they did not
/// write, and a lint of every build is time spent for no reader.  The count over the corpus is
/// held at zero by `scripts/c_lint_census.py` (`make c-lint-census`), because a warning can mean
/// a wrong program.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lint {
    Off,
    Report,
    Strict,
    /// `-Wall -Wextra` whole, the `unused` family included (`scripts/c_lint_census.py --unused`).
    All,
}

fn lint() -> Lint {
    match std::env::var("LOFT_C_LINT").as_deref() {
        Ok("strict") => Lint::Strict,
        Ok("all") => Lint::All,
        Ok("1" | "true" | "report") => Lint::Report,
        _ => Lint::Off,
    }
}

/// The lint's warnings: everything `-Wall -Wextra` reports, the `unused` family included — a
/// local nothing reads can be a lost value.  The written code names nothing it does not use
/// (`src/c/tidy.rs`); the one exception is the runtime's helpers a unit does not call, which a
/// pragma around the runtime keeps out (`LOFT_C_LINT=all` lifts it).
const LINT_WARNINGS: &[&str] = &["-Wall", "-Wextra"];

fn env_on(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// The files the unit names in its `/* link: <path> */` lines — a `[c] shim` archive, or a
/// shim's sources where this host has no archiver.  The same lines `scripts/c_sweep.py` and
/// `tests/c_emit.rs` read.
pub(crate) fn link_inputs(c: &str) -> Vec<String> {
    c.lines()
        .filter_map(|l| l.strip_prefix("/* link: ")?.strip_suffix(" */"))
        .map(str::to_string)
        .collect()
}

/// The link inputs the unit marks `/* <kind>: <input> */` — `bound` for a library its package
/// ships, `shim-archive` for a `[c] shim`'s archive — each also named on its own link line.
/// The release tier compiles a `[c] shim`'s sources with the unit under link-time
/// optimisation, so a shim's small functions inline into their callers (random `rand`: -45 %
/// against the linked archive): the unit's link inputs and shims with each archive replaced by
/// its sources, `-flto` and the shim's include directory added to `flags`.  A dynamic shipped
/// program opens its shim's library beside it, and `cl` takes no `-flto`: both keep the
/// archive.  `LOFT_NO_SHIM_LTO=1` links the archive in every tier.
fn release_shim_lto(
    c: &str,
    tier: Tier,
    dynamic: bool,
    cc: &str,
    links: Vec<String>,
    shims: std::collections::HashSet<String>,
    flags: &mut Vec<String>,
) -> (Vec<String>, std::collections::HashSet<String>) {
    if tier != Tier::Release
        || dynamic
        || dialect(cc) != Dialect::Gnu
        || shims.is_empty()
        || env_on("LOFT_NO_SHIM_LTO")
    {
        return (links, shims);
    }
    let (links, sources) = shims_as_sources(c, &links, &shims);
    if !sources.is_empty() {
        flags.push("-flto".to_string());
        let inc =
            loft::platform::scratch_dir().join(format!("loft_shim_inc_{}", std::process::id()));
        let paths: Vec<std::path::PathBuf> = sources.iter().map(std::path::PathBuf::from).collect();
        if fa::create_dir_all(&inc).is_ok()
            && loft::c_shim::put_width_header(&inc).is_ok()
            && loft::c_shim::put_bodies(&inc, &paths).is_ok()
        {
            flags.push("-I".to_string());
            flags.push(inc.to_string_lossy().into_owned());
        }
    }
    (links, sources)
}

/// Is `path` a C source (`.c`, any case)?
fn is_c_source(path: &str) -> bool {
    path.len() > 2
        && path.is_char_boundary(path.len() - 2)
        && path[path.len() - 2..].eq_ignore_ascii_case(".c")
}

/// The unit's link inputs with each `[c] shim` archive in `shims` replaced by the sources the
/// unit names for it (`/* shim-source: */`, the lines after its `/* shim-archive: */`), and
/// those sources: what the release tier compiles with the unit.  An archive whose sources the
/// unit does not name stays linked.
fn shims_as_sources(
    c: &str,
    links: &[String],
    shims: &std::collections::HashSet<String>,
) -> (Vec<String>, std::collections::HashSet<String>) {
    let mut of: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut current: Option<String> = None;
    for l in c.lines() {
        if let Some(a) = l
            .strip_prefix("/* shim-archive: ")
            .and_then(|r| r.strip_suffix(" */"))
        {
            current = Some(a.to_string());
        } else if let Some(src) = l
            .strip_prefix("/* shim-source: ")
            .and_then(|r| r.strip_suffix(" */"))
        {
            if let Some(a) = &current {
                of.entry(a.clone()).or_default().push(src.to_string());
            }
        } else if !l.starts_with("/* link: ") {
            current = None;
        }
    }
    let mut out = Vec::new();
    let mut sources = std::collections::HashSet::new();
    for l in links {
        match of.get(l) {
            Some(srcs) if shims.contains(l) && srcs.iter().all(|s| is_c_source(s)) => {
                for s in srcs {
                    out.push(s.clone());
                    sources.insert(s.clone());
                }
            }
            _ => out.push(l.clone()),
        }
    }
    (out, sources)
}

fn marked(c: &str, kind: &str) -> std::collections::HashSet<String> {
    let open = format!("/* {kind}: ");
    c.lines()
        .filter_map(|l| l.strip_prefix(open.as_str())?.strip_suffix(" */"))
        .map(str::to_string)
        .collect()
}

/// Where a built program runs.  `Local`: from the cache, here, linking each library a package
/// ships where it was built.  `Shipped` (`--out`): anywhere it is copied, opening those
/// libraries at start through loft's search order (@PLN195) — a `[c] shim` inside the binary,
/// or beside it under `--dynamic`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    Local,
    Shipped { dynamic: bool },
}

/// The content key of one build: the C text, the compiler — the program named and its
/// identity — the flags, and each linked input's size and modification time (an archive is
/// content-addressed by its name already; a raw shim source is not).  The name is keyed beside
/// the identity because a wrapper (`LOFT_CC=verify-cc`, adding `-DLOFT_VERIFY`) reports the
/// banner of the compiler it wraps: keyed by the banner alone, it is served the binary the
/// plain compiler built.
fn build_key(c: &str, cc: &str, flags: &[String], links: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(c.as_bytes());
    h.update(cc.as_bytes());
    h.update([0]);
    h.update(cc_identity(cc).as_bytes());
    for f in flags {
        h.update(f.as_bytes());
        h.update([0]);
    }
    for l in links {
        h.update(l.as_bytes());
        if let Ok(m) = fa::metadata(l) {
            h.update(m.len().to_le_bytes());
            if let Ok(t) = m.modified()
                && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
            {
                h.update(d.as_nanos().to_le_bytes());
            }
        }
    }
    let digest = h.finalize();
    digest[..8].iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// The compiler's first `--version` line, or its name when it reports none.  Asked once per
/// process.
pub(crate) fn cc_identity(cc: &str) -> String {
    static IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            // `cl` takes no `--version`: run bare, it prints its banner on stderr.
            let msvc = dialect(cc) == Dialect::Msvc;
            let mut probe = cc_spawn(cc);
            if !msvc {
                probe.push_arg("--version");
            }
            probe
                .run(b"")
                .ok()
                .filter(|o| msvc || o.status.success())
                .map_or_else(
                    || cc.to_string(),
                    |o| {
                        String::from_utf8_lossy(if msvc { &o.stderr } else { &o.stdout })
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_string()
                    },
                )
        })
        .clone()
}

/// The toolchain a wasm32 build of the C uses, found once per run: the clang, the WASI sysroot,
/// the `wasm-ld` beside a clang whose own directory has none, and the target triple the
/// sysroot's headers answer to.
struct WasmToolchain {
    cc: String,
    sysroot: Option<String>,
    ld: Option<String>,
    triple: &'static str,
    /// Whether a small program linked under `triple`: a clang with no wasm target (Apple's)
    /// or no WASI libc links under neither, and is a toolchain to install, said before the
    /// program's own C is compiled.
    links: bool,
    /// Whether this clang takes `-mllvm -wasm-use-legacy-eh=false` (LLVM 19 and later).  An
    /// older one has only the legacy encoding, and a WASI host that refuses it says so when
    /// the program runs.
    standard_eh: bool,
    /// Whether this clang's driver takes `--no-wasm-opt` (an older one is refused by it).
    no_wasm_opt: bool,
}

/// The wasm32 C toolchain, from [`loft::platform::wasm_c_toolchain`] (which clang, sysroot and
/// triple a WASI program links under), plus the optional flags this clang takes.
fn wasm_toolchain() -> &'static WasmToolchain {
    static TC: std::sync::OnceLock<WasmToolchain> = std::sync::OnceLock::new();
    TC.get_or_init(|| {
        let found = loft::platform::wasm_c_toolchain();
        let (cc, ld, sysroot) = (found.cc.clone(), found.ld.clone(), found.sysroot.clone());
        let (triple, links) = match found.triple {
            Some(t) => (t, true),
            None => ("wasm32-wasi", false),
        };
        let probe_out = loft::platform::scratch_dir()
            .join(format!("loft_wasm_probe_{}.wasm", std::process::id()));
        // An optional flag is passed only where this clang knows it: a compile of an empty
        // unit with the flag, which an older clang refuses as an unknown argument.
        let takes = |flags: &[&str]| {
            let mut probe = Spawn::new(Program::os(&cc));
            probe.push_arg(format!("--target={triple}"));
            probe.push_args(flags.iter().copied());
            probe
                .push_args(["-c", "-x", "c", "-", "-o"])
                .push_arg(&probe_out);
            let ok = probe
                .run(b"int loft_probe(void) { return 0; }\n")
                .is_ok_and(|r| r.status.success());
            let _ = fa::remove_file(&probe_out);
            ok
        };
        let standard_eh = takes(&["-mllvm", "-wasm-use-legacy-eh=false"]);
        let no_wasm_opt = takes(&["--no-wasm-opt"]);
        WasmToolchain {
            cc,
            sysroot,
            ld,
            triple,
            links,
            standard_eh,
            no_wasm_opt,
        }
    })
}

/// `--native-wasm` through the C (@PLN203 C6): the unit built for WASI by clang into
/// `out`, as `scripts/c_sweep.py --wasm` builds and validates it.  setjmp is wasm's
/// exception-handling proposal in its standard encoding (a host runs it with exceptions on —
/// `wasmtime -W exceptions=y`); `clock` and `signal` are WASI's emulation libraries; the stack
/// is a native program's 8 MiB, where wasm-ld's default is 64 KiB.  A `[c] shim` of several
/// sources is compiled from its sources here — its archive is the host's.
///
/// # Errors
/// No clang, or a build that fails, with the compiler's output.
pub(crate) fn build_wasm(
    c: &str,
    out: &std::path::Path,
    shim_sources: &[String],
    ship: bool,
    browser: bool,
    names: bool,
) -> Result<(), String> {
    let tc = wasm_toolchain();
    let cc = tc.cc.clone();
    if !tc.links {
        return Err(format!(
            "--native-wasm: the wasm32 toolchain is incomplete — `{cc}` links a small WASI \
             program under neither wasm32-wasip1 nor wasm32-wasi; {}",
            loft::platform::c_wasm_toolchain_cure()
        ));
    }
    let scratch = loft::platform::scratch_dir();
    let src = scratch.join(format!("loft_wasm_c_{}.c", std::process::id()));
    fa::write(&src, c).map_err(|e| format!("cannot write {}: {e}", src.display()))?;
    // A unit that jumps nowhere (`LOFT_NO_JUMPS`, a page's module) is built without setjmp,
    // so without wasm's exception handling: asyncify, which a page's frame yield needs, cannot
    // carry it.
    let jumps = !c.contains("#define LOFT_NO_JUMPS");
    let mut cmd = Spawn::new(Program::os(&cc));
    cmd.push_arg(format!("--target={}", tc.triple));
    cmd.push_args([
        "-std=c99",
        if ship { "-O3" } else { "-O2" },
        "-w",
        "-D_WASI_EMULATED_PROCESS_CLOCKS",
        "-D_WASI_EMULATED_SIGNAL",
    ]);
    // setjmp in wasm's exception-handling proposal: its standard encoding for a WASI host
    // (wasmtime refuses the legacy one), the legacy one for a browser page, which every
    // current browser runs.
    if jumps {
        cmd.push_args(["-mllvm", "-wasm-enable-sjlj"]);
        if !browser && tc.standard_eh {
            cmd.push_args(["-mllvm", "-wasm-use-legacy-eh=false"]);
        }
    }
    if !ship {
        cmd.push_arg("-DLOFT_CALL_CHAIN");
    }
    // `--names` (loft#954): wasm-ld writes the function names, but clang's driver then runs
    // `wasm-opt` itself when one is on PATH, without `-g`, which drops them.  The page runs its
    // own `wasm-opt` (`--strip-dwarf -g` under `--names`), so clang's pass is skipped here.
    // And each loft function stays a frame of its own, so a trap's backtrace names the function
    // the author wrote, not the caller it was inlined into.
    // A clang whose driver does not take `--no-wasm-opt` runs `wasm-opt` only for an
    // optimised link, so a `--names` build there is unoptimised (the last `-O` wins).
    if names {
        cmd.push_arg(if tc.no_wasm_opt {
            "--no-wasm-opt"
        } else {
            "-O0"
        });
        cmd.push_arg("-fno-inline");
    }
    // A shim built for a page sees `LOFT_BROWSER`, as `c_shim::wasm_objects` defines it for
    // the Rust writer's page: there it can call the page's `loft_gl` imports.
    if browser {
        cmd.push_arg("-DLOFT_BROWSER");
    }
    if let Some(root) = &tc.sysroot {
        cmd.push_arg(format!("--sysroot={root}"));
    }
    if let Some(ld) = &tc.ld {
        cmd.push_arg(format!("--ld-path={ld}"));
    }
    cmd.push_arg(&src);
    cmd.push_args(shim_sources);
    cmd.push_arg("-o").push_arg(out);
    // The stack FIRST, at the bottom of memory (wasm-ld's `--stack-first`, the Rust writer's
    // layout): an overflow then traps below address 0 instead of running down into the data —
    // a page's asyncify save region among it (loft#950).
    cmd.push_args(["-Wl,-z,stack-size=8388608", "-Wl,--stack-first", "-lm"]);
    if jumps {
        cmd.push_arg("-lsetjmp");
    }
    cmd.push_args(["-lwasi-emulated-process-clocks", "-lwasi-emulated-signal"]);
    let ran = cmd.run(b"");
    if std::env::var_os("LOFT_KEEP_NATIVE_C").is_none() {
        let _ = fa::remove_file(&src);
    }
    let ran = ran.map_err(|e| {
        format!(
            "--native-wasm needs clang and `{cc}` could not be run ({e}); {}",
            loft::platform::c_wasm_toolchain_cure()
        )
    })?;
    if ran.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&ran.stderr);
    // A missing sysroot, linker or builtins library is a toolchain to install, not a defect.
    let missing = [
        "wasm-ld",
        "sysroot",
        "libclang_rt",
        "stdio.h",
        "cannot find",
        "unable to find",
        // A header found on the HOST's path: no WASI sysroot was found at all.
        "/usr/include/",
    ]
    .iter()
    .any(|m| said.contains(m));
    if missing {
        return Err(format!(
            "--native-wasm: the wasm32 toolchain is incomplete; {}.\n{}",
            loft::platform::c_wasm_toolchain_cure(),
            said.trim_end()
        ));
    }
    Err(format!(
        "`{cc}` rejected the C this program was written as for wasm32 — a defect of the C \
         backend (LOFT_KEEP_NATIVE_C=1 keeps the source).\n{}",
        said.trim_end()
    ))
}

/// Why [`build`] made no binary: no C compiler to run at all — which a run that did not ask for
/// native answers by interpreting, as the rustc path does without rustc — or a compiler that
/// rejected the unit, a defect of the backend.
pub(crate) enum BuildError {
    NoCompiler(String),
    Rejected(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::NoCompiler(m) | BuildError::Rejected(m) => f.write_str(m),
        }
    }
}

/// `cc` run to completion.  A deadline that falls in the compile names it (`blocked=`), as the
/// rustc path does, rather than reading as `phase=parse`.
fn run_cc(cmd: &mut Spawn) -> std::io::Result<loft::platform::process::Ran> {
    loft::timeout::blocked_on("cc compiling the program");
    let out = cmd.run(b"");
    loft::timeout::unblocked();
    out
}

/// The compiler's command building `src` into `binary`, in its own dialect: gcc's links the
/// libraries after the objects, where it resolves them; `cl` is told where its object goes,
/// which it writes into the working directory otherwise.
fn compile_command(
    cc: &str,
    flags: &[String],
    src: &std::path::Path,
    binary: &std::path::Path,
    obj: &std::path::Path,
    link_args: &[String],
) -> Spawn {
    let mut cmd = cc_spawn(cc);
    match dialect(cc) {
        Dialect::Gnu => {
            cmd.push_args(flags);
            cmd.push_arg(src).push_arg("-o").push_arg(binary);
            cmd.push_args(link_args);
            cmd.push_arg("-lm");
            cmd.push_args(loft::platform::c_system_link_libs());
            // A program's main thread gets the stack a Linux one has (8 MiB): Windows defaults
            // to 1 MiB, and the same recursion must not overflow on one platform only.
            cmd.push_args(loft::platform::c_main_stack_args(false));
        }
        Dialect::Msvc => {
            cmd.push_arg("/nologo").push_args(msvc_args(flags));
            cmd.push_args(loft::platform::c_main_stack_args(true));
            cmd.push_arg(src);
            cmd.push_arg(format!("/Fe:{}", binary.display()));
            cmd.push_arg(format!("/Fo:{}", obj.display()));
            cmd.push_args(msvc_args(link_args));
        }
    }
    cmd
}

/// How the unit's `link:` inputs reach the linker, placed as [`Placement`] says: the run paths
/// to add to the flags, and the link arguments.  `binds`: the program opens the libraries its
/// packages ship at start, so a `bound` library is not linked and a shim is linked whole into a
/// static program (and exported) or opened beside a dynamic one.
fn link_plan(
    links: &[String],
    bound: &std::collections::HashSet<String>,
    shims: &std::collections::HashSet<String>,
    placement: Placement,
    binds: bool,
) -> (Vec<String>, Vec<String>) {
    let relocatable = placement != Placement::Local;
    let dynamic = placement == (Placement::Shipped { dynamic: true });
    let mut export = false;
    // Where a linked shared library is found at run time: its own directory, or the binary's
    // when the program is written out with its libraries beside it (`--out`).
    let ext = format!(".{}", loft::platform::dll_extension());
    let mut rpaths: Vec<String> = Vec::new();
    let mut link_args: Vec<String> = Vec::new();
    for l in links {
        if binds && bound.contains(l) {
            continue;
        }
        // A shim stays inside a static shipped program, linked whole and exported, so a symbol
        // found by name (`loft_c_bound_sym`) is there; a dynamic one opens its library instead.
        // A shim's source compiled with the unit (the release tier) is in it whole already.
        if binds && shims.contains(l) && is_c_source(l) {
            link_args.push(l.clone());
            export = true;
            continue;
        }
        if binds && shims.contains(l) {
            if !dynamic {
                link_args.extend(loft::platform::c_whole_archive_args(l));
                export = true;
            }
            continue;
        }
        // A system library arrives as the linker's own argument (`-l:libpq.so.5`,
        // `c_link_arg_for_soname`), not as a file: passed through as it is.  Read as a path it
        // became `-L` and `-l:-l:libpq.so.5`, and the library was never linked.
        if l.starts_with('-') {
            link_args.push(l.clone());
            continue;
        }
        let p = std::path::Path::new(l);
        let shared = l.ends_with(&ext) || l.contains(".so.");
        if !shared {
            link_args.push(l.clone());
            continue;
        }
        link_args.extend(loft::platform::c_link_shared_args(p));
        let dir = if relocatable { None } else { p.parent() };
        if let Some(arg) = loft::platform::c_rpath_arg(dir)
            && !rpaths.contains(&arg)
        {
            rpaths.push(arg);
        }
    }
    if export && let Some(arg) = loft::platform::c_export_dynamic_arg() {
        link_args.push(arg.to_string());
    }
    (rpaths, link_args)
}

/// The outcome of [`build`]: the binary to run and whether the cache supplied it.
pub(crate) struct Built {
    pub binary: std::path::PathBuf,
    pub cached: bool,
}

/// Build the unit `c` into `cache_dir/native-c-<key>` (or reuse it), returning its path,
/// placed as [`Placement`] says.
/// The binary is compiled under the scratch directory and published by an atomic rename, so
/// a concurrent run of the same program reads either no binary or a complete one.  With
/// `no_cache` the binary stays a per-process scratch file the caller deletes after the run.
///
/// # Errors
/// No C compiler, or a compile that fails — the message carries the compiler's own output,
/// since a C the backend wrote that does not compile is a defect of the backend.
pub(crate) fn build(
    c: &str,
    tier: Tier,
    cache_dir: &std::path::Path,
    no_cache: bool,
    placement: Placement,
) -> Result<Built, BuildError> {
    let cc = cc_program();
    let mut flags = cc_flags(tier);
    let links = link_inputs(c);
    let relocatable = placement != Placement::Local;
    // A shipped program opens the libraries its packages ship at start, where the host can
    // (`loft_c_bind_libraries`); elsewhere it links them, found beside it.
    let binds = relocatable && loft::platform::c_binds_libraries_at_start();
    let dynamic = placement == (Placement::Shipped { dynamic: true });
    let (bound, shims) = (marked(c, "bound"), marked(c, "shim-archive"));
    let (links, shims) = release_shim_lto(c, tier, dynamic, &cc, links, shims, &mut flags);
    if binds {
        flags.push("-DLOFT_BIND_LIBS".to_string());
        if dynamic {
            flags.push("-DLOFT_BIND_SHIMS".to_string());
        }
    }
    let (rpaths, link_args) = link_plan(&links, &bound, &shims, placement, binds);
    flags.extend(rpaths);
    let key = build_key(c, &cc, &flags, &links);
    let exe = std::env::consts::EXE_SUFFIX;
    let cached = cache_dir.join(format!("native-c-{key}{exe}"));
    // A lint compiles afresh, so its warnings are seen every time it is asked for.
    let linting = lint() != Lint::Off;
    if !no_cache
        && !linting
        && fa::exists(&cached)
        && crate::native_utils::cache_safe_to_execute(&cached)
    {
        crate::native_utils::touch_cached_binary(&cached);
        return Ok(Built {
            binary: cached,
            cached: true,
        });
    }
    let scratch = loft::platform::scratch_dir();
    let pid = std::process::id();
    let src = scratch.join(format!("loft_native_c_{pid}.c"));
    let binary = scratch.join(format!("loft_native_bin_{pid}{exe}"));
    // A unit written with source lines (the debug tier) hands the code between its loft
    // functions back to this file, under the name it is written as here.
    let text = if tier == Tier::Debug {
        std::borrow::Cow::Owned(loft::c::resolve_line_resets(c, &src.to_string_lossy()))
    } else {
        std::borrow::Cow::Borrowed(c)
    };
    fa::write(&src, &*text)
        .map_err(|e| BuildError::Rejected(format!("cannot write {}: {e}", src.display())))?;
    let obj = scratch.join(format!("loft_native_obj_{pid}.obj"));
    let mut cmd = compile_command(&cc, &flags, &src, &binary, &obj, &link_args);
    let out = run_cc(&mut cmd);
    let _ = fa::remove_file(&obj);
    let keep = std::env::var_os("LOFT_KEEP_NATIVE_C").is_some();
    if !keep {
        let _ = fa::remove_file(&src);
    }
    let out = out.map_err(|e| {
        BuildError::NoCompiler(format!(
            "the C backend needs a C compiler and `{cc}` could not be run ({e}); {}",
            loft::platform::c_host_toolchain_cure()
        ))
    })?;
    if !out.status.success() {
        let _ = fa::remove_file(&binary);
        return Err(BuildError::Rejected(format!(
            "`{cc}` rejected the C this program was written as — a defect of the C backend \
             (LOFT_KEEP_NATIVE_C=1 keeps the source).\n{}",
            String::from_utf8_lossy(&out.stderr).trim_end()
        )));
    }
    if linting && !out.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    if keep {
        eprintln!("loft: C source preserved at {}", src.display());
    }
    if no_cache {
        return Ok(Built {
            binary,
            cached: false,
        });
    }
    if fa::create_dir_all(cache_dir).is_ok() {
        crate::native_utils::tighten_cache_dir(cache_dir);
        crate::native_utils::publish_cached_binary(&binary, &cached, cache_dir, "native");
    }
    if fa::exists(&cached) {
        let _ = fa::remove_file(&binary);
        Ok(Built {
            binary: cached,
            cached: false,
        })
    } else {
        Ok(Built {
            binary,
            cached: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The release tier compiles each `[c] shim` the unit links from the sources it names after
    /// that archive; an archive with no sources named, or a lazy one (not marked
    /// `shim-archive`), stays linked, and every other input passes through in order.
    /// Falsified at c3defa8ff: with the source list not closed at a line that is no link,
    /// the lazy archive's source attached to `libq.a` and replaced it.
    #[test]
    fn a_release_unit_compiles_its_shims_from_their_sources() {
        let c = "/* `[c] shim` of package p, 2 sources */\n/* shim-archive: /s/libp.a */\n\
                 /* link: /s/libp.a */\n/* shim-source: /p/c/a.c */\n/* shim-source: /p/c/b.c */\n\
                 int x;\n/* shim-archive: /s/libq.a */\n/* link: /s/libq.a */\nint y;\n\
                 /* link: /s/liblazy.a */\n/* shim-source: /z/c/z.c */\n";
        let links: Vec<String> = ["-lpq", "/s/libp.a", "/s/libq.a", "/s/liblazy.a"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let shims = marked(c, "shim-archive");
        let (out, sources) = shims_as_sources(c, &links, &shims);
        assert_eq!(
            out,
            ["-lpq", "/p/c/a.c", "/p/c/b.c", "/s/libq.a", "/s/liblazy.a"]
        );
        assert_eq!(sources.len(), 2);
        assert!(sources.contains("/p/c/a.c") && sources.contains("/p/c/b.c"));
    }

    /// A wrapper compiler reports the banner of the one it wraps; the key must still tell
    /// them apart, or a `-DLOFT_VERIFY` wrapper runs the plain compiler's binary.
    #[test]
    fn build_key_tells_a_wrapper_from_the_compiler_it_wraps() {
        let cc = cc_program();
        let wrapper = format!("{cc}-verify-wrapper");
        assert_ne!(
            build_key("int main(void){return 0;}", &cc, &[], &[]),
            build_key("int main(void){return 0;}", &wrapper, &[], &[])
        );
    }

    #[test]
    fn a_cl_compiler_takes_the_flags_in_its_own_spelling() {
        assert_eq!(dialect("cc"), Dialect::Gnu);
        assert_eq!(dialect("/usr/bin/clang"), Dialect::Gnu);
        assert_eq!(dialect("cl"), Dialect::Msvc);
        assert_eq!(dialect("C:/VS/bin/Hostx64/x64/CL.EXE"), Dialect::Msvc);
        assert_eq!(dialect("clang-cl.exe"), Dialect::Msvc);
        let gnu: Vec<String> = [
            "-std=c99",
            "-w",
            "-O1",
            "-DLOFT_CALL_CHAIN",
            "-pthread",
            "-ldl",
            "-lm",
            "-Wall",
            "-Wno-unused-label",
            "-Werror",
            "-O3",
            "-g",
            "shim.lib",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        assert_eq!(
            msvc_args(&gnu),
            [
                "/std:c11",
                "/W0",
                "/O1",
                "/DLOFT_CALL_CHAIN",
                "/W3",
                "/WX",
                "/O2",
                "/Z7",
                "shim.lib"
            ]
        );
    }

    #[test]
    fn a_system_library_reaches_the_linker_as_its_own_argument() {
        // `c_link_arg_for_soname` writes a versioned system library as `-l:libpq.so.5`; read
        // as a path it carried `.so.` and became `-L` + `-l:-l:libpq.so.5`, linking nothing.
        let none = std::collections::HashSet::new();
        for flag in ["-l:libmariadb.so.3", "-lpq"] {
            let (rpaths, args) =
                link_plan(&[flag.to_string()], &none, &none, Placement::Local, false);
            assert_eq!(args, vec![flag.to_string()], "{flag}");
            assert!(rpaths.is_empty(), "{flag}: {rpaths:?}");
        }
    }

    #[test]
    fn link_lines_are_read_exactly() {
        let c = "/* `[c] shim` of package p, 2 sources */\n/* link: /a/b.a */\nint x;\n\
                 /* link: /c d.c */\n/* not a link: x */\n";
        assert_eq!(
            link_inputs(c),
            vec!["/a/b.a".to_string(), "/c d.c".to_string()]
        );
    }

    #[test]
    fn every_flag_and_the_text_move_the_key() {
        let flags = cc_flags(Tier::Check);
        let base = build_key("int x;", "cc", &flags, &[]);
        assert_eq!(base, build_key("int x;", "cc", &flags, &[]));
        assert_ne!(base, build_key("int y;", "cc", &flags, &[]));
        assert_ne!(
            base,
            build_key("int x;", "cc", &cc_flags(Tier::Release), &[])
        );
        assert_ne!(
            base,
            build_key("int x;", "cc", &flags, &["/nowhere".to_string()])
        );
    }
}
