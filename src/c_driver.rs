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

/// The C compiler: `LOFT_CC`, else `CC`, else `cc`.
pub(crate) fn cc_program() -> String {
    std::env::var("LOFT_CC")
        .or_else(|_| std::env::var("CC"))
        .unwrap_or_else(|_| "cc".to_string())
}

/// The compiler flags of one build: the tier's optimisation and call chain, and the store's
/// poison modes when the environment asks for them as the interpreter's switches do.  Every flag is part of
/// the cache key, since each one changes the binary.
fn cc_flags(tier: Tier) -> Vec<String> {
    let mut flags: Vec<String> = vec!["-std=c99".into()];
    match lint() {
        Lint::Off => flags.push("-w".into()),
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
    flags
}

/// `LOFT_C_LINT`: the C a program is written as compiled with the warnings below instead of
/// `-w`, on request — `1` reports them, `strict` makes each one an error.  Never on by default:
/// a user does not read warnings about C they did not write, and a lint of every build is time
/// spent for no reader.  It is the instrument for a larger C99 program the moment there is a
/// reason to look (a new compiler, a new platform, a crash that smells of UB).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lint {
    Off,
    Report,
    Strict,
}

fn lint() -> Lint {
    match std::env::var("LOFT_C_LINT").as_deref() {
        Ok("strict") => Lint::Strict,
        Ok("1" | "true" | "report") => Lint::Report,
        _ => Lint::Off,
    }
}

/// The lint's warnings: everything `-Wall -Wextra` reports that can mean a wrong program (a
/// conversion, an uninitialised read, a missing declaration, a format), without the `unused`
/// family, which generated code trips by design (a parameter the body ignores, a helper the
/// unit does not call).
const LINT_WARNINGS: &[&str] = &[
    "-Wall",
    "-Wextra",
    "-Wno-unused-parameter",
    "-Wno-unused-function",
    "-Wno-unused-variable",
    "-Wno-unused-but-set-variable",
    "-Wno-unused-label",
    "-Wno-unused-value",
];

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

/// The content key of one build: the C text, the compiler's identity, the flags, and each
/// linked input's size and modification time (an archive is content-addressed by its name
/// already; a raw shim source is not).
fn build_key(c: &str, cc: &str, flags: &[String], links: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(c.as_bytes());
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
            Spawn::new(Program::os(cc))
                .arg("--version")
                .run(b"")
                .ok()
                .filter(|o| o.status.success())
                .map_or_else(
                    || cc.to_string(),
                    |o| {
                        String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .to_string()
                    },
                )
        })
        .clone()
}

/// `--native-wasm` through the C (@PLN203 C6): the unit built for wasm32-wasi by clang into
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
) -> Result<(), String> {
    let cc = std::env::var("LOFT_WASM_CC").unwrap_or_else(|_| "clang".to_string());
    let scratch = loft::platform::scratch_dir();
    let src = scratch.join(format!("loft_wasm_c_{}.c", std::process::id()));
    fa::write(&src, c).map_err(|e| format!("cannot write {}: {e}", src.display()))?;
    // A unit that jumps nowhere (`LOFT_NO_JUMPS`, a page's module) is built without setjmp,
    // so without wasm's exception handling: asyncify, which a page's frame yield needs, cannot
    // carry it.
    let jumps = !c.contains("#define LOFT_NO_JUMPS");
    let mut cmd = Spawn::new(Program::os(&cc));
    cmd.push_args([
        "--target=wasm32-wasi",
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
        if !browser {
            cmd.push_args(["-mllvm", "-wasm-use-legacy-eh=false"]);
        }
    }
    if !ship {
        cmd.push_arg("-DLOFT_CALL_CHAIN");
    }
    // The WASI sysroot: the one `WASI_SYSROOT` names, else the system package's when it is
    // there (a clang that does not look in it falls back to the host's headers).
    let sysroot = std::env::var("WASI_SYSROOT")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            fa::exists("/usr/share/wasi-sysroot").then(|| "/usr/share/wasi-sysroot".to_string())
        });
    if let Some(root) = sysroot {
        cmd.push_arg(format!("--sysroot={root}"));
    }
    cmd.push_arg(&src);
    cmd.push_args(shim_sources);
    cmd.push_arg("-o").push_arg(out);
    cmd.push_args(["-Wl,-z,stack-size=8388608", "-lm"]);
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
        if binds && shims.contains(l) {
            if !dynamic {
                link_args.extend(loft::platform::c_whole_archive_args(l));
                export = true;
            }
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
    fa::write(&src, c)
        .map_err(|e| BuildError::Rejected(format!("cannot write {}: {e}", src.display())))?;
    let mut cmd = Spawn::new(Program::os(&cc));
    cmd.push_args(&flags);
    cmd.push_arg(&src).push_arg("-o").push_arg(&binary);
    cmd.push_args(&link_args);
    cmd.push_arg("-lm");
    let out = run_cc(&mut cmd);
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
