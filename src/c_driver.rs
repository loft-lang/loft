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
    let mut flags: Vec<String> = vec!["-std=c99".into(), "-w".into()];
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
) -> Result<(), String> {
    let cc = std::env::var("LOFT_WASM_CC").unwrap_or_else(|_| "clang".to_string());
    let scratch = loft::platform::scratch_dir();
    let src = scratch.join(format!("loft_wasm_c_{}.c", std::process::id()));
    fa::write(&src, c).map_err(|e| format!("cannot write {}: {e}", src.display()))?;
    let mut cmd = Spawn::new(Program::os(&cc));
    cmd.push_args([
        "--target=wasm32-wasi",
        "-std=c99",
        if ship { "-O3" } else { "-O2" },
        "-w",
        "-mllvm",
        "-wasm-enable-sjlj",
        "-mllvm",
        "-wasm-use-legacy-eh=false",
        "-D_WASI_EMULATED_PROCESS_CLOCKS",
        "-D_WASI_EMULATED_SIGNAL",
    ]);
    if !ship {
        cmd.push_arg("-DLOFT_CALL_CHAIN");
    }
    cmd.push_arg(&src);
    cmd.push_args(shim_sources);
    cmd.push_arg("-o").push_arg(out);
    cmd.push_args([
        "-Wl,-z,stack-size=8388608",
        "-lm",
        "-lsetjmp",
        "-lwasi-emulated-process-clocks",
        "-lwasi-emulated-signal",
    ]);
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

/// The outcome of [`build`]: the binary to run and whether the cache supplied it.
pub(crate) struct Built {
    pub binary: std::path::PathBuf,
    pub cached: bool,
}

/// Build the unit `c` into `cache_dir/native-c-<key>` (or reuse it), returning its path;
/// `relocatable` finds the linked libraries beside the binary (`--out`) rather than where they
/// were built.
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
    relocatable: bool,
) -> Result<Built, BuildError> {
    let cc = cc_program();
    let mut flags = cc_flags(tier);
    let links = link_inputs(c);
    // Where a linked shared library is found at run time: its own directory, or the binary's
    // when the program is written out with its libraries beside it (`--out`).
    let ext = format!(".{}", loft::platform::dll_extension());
    let mut rpaths: Vec<String> = Vec::new();
    for l in &links {
        let p = std::path::Path::new(l);
        let shared = l.ends_with(&ext) || l.contains(".so.");
        let dir = if relocatable { None } else { p.parent() };
        if shared
            && let Some(arg) = loft::platform::c_rpath_arg(dir)
            && !rpaths.contains(&arg)
        {
            rpaths.push(arg);
        }
    }
    flags.extend(rpaths);
    let key = build_key(c, &cc, &flags, &links);
    let exe = std::env::consts::EXE_SUFFIX;
    let cached = cache_dir.join(format!("native-c-{key}{exe}"));
    if !no_cache && fa::exists(&cached) && crate::native_utils::cache_safe_to_execute(&cached) {
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
    cmd.push_args(&links);
    cmd.push_arg("-lm");
    let out = cmd.run(b"");
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
