// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Native compilation utilities: rlib management, cache keys, artifact paths.

use std::env;
pub(crate) fn with_trailing_sep(p: &std::path::Path) -> String {
    let mut s = p.to_str().unwrap_or("").to_string();
    if !s.ends_with('/') && !s.ends_with('\\') {
        s.push(std::path::MAIN_SEPARATOR);
    }
    s
}

/// Walk up from the running loft binary to the loft SOURCE tree — the first
/// ancestor directory whose `Cargo.toml` is the `loft` package's own manifest.
/// `Some` in a dev / source checkout (where the native runtime can be rebuilt
/// from source); `None` from an installed bundle (no source to build against).
pub(crate) fn loft_source_tree() -> Option<std::path::PathBuf> {
    let mut dir = env::current_exe().ok()?.parent()?.to_path_buf();
    loop {
        // loft's own manifest carries `name = "loft"`; a consumer crate that
        // merely depends on loft would not, so this never misfires on a user tree.
        if crate::file_access::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|t| t.contains("name = \"loft\""))
        {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Return true if `s` looks like an explicit output path rather than a flag or loft source file.
pub(crate) fn is_output_path(s: &str) -> bool {
    !s.starts_with('-')
        && !crate::file_access::extension(s).is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
}

/// Return (and create) the `.loft/` artifact directory beside `script_path`.
/// Falls back to the current directory's `.loft/` if the parent cannot be determined.
pub(crate) fn loft_artifact_dir(script_path: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(script_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let loft_dir = dir.join(".loft");
    let _ = crate::file_access::create_dir_all(&loft_dir);
    loft_dir
}

/// Return the default output path for a compiled artifact beside `script_path`.
/// `ext` is the file extension without leading dot (e.g. `"wasm"`, `"rs"`).
pub(crate) fn default_artifact_path(script_path: &str, ext: &str) -> std::path::PathBuf {
    let stem = crate::file_access::file_stem(script_path).unwrap_or_else(|| "out".to_string());
    loft_artifact_dir(script_path).join(format!("{stem}.{ext}"))
}

/// @P350: parse the import section of a `loft --html` wasm and verify every
/// import module is one of the raw extern bridges the embedded HTML glue
/// provides (`loft_gl` / `loft_io`).  A wasm-bindgen-feature rlib — the
/// "rlib stomp" `make wasm` leaves at
/// `target/wasm32-unknown-unknown/release/libloft.rlib` — imports
/// `__wbindgen_placeholder__` (35+), which the glue cannot satisfy, so the
/// page never instantiates.  Returns `Ok(())` when every import module is
/// acceptable (a wasm with no imports included), or `Err(sorted distinct bad
/// module names)` otherwise.  Mirrors the import check in
/// `tools/check_html_bundle.mjs`, inline so it guards a bare `loft --html`
/// and not only the `make game` path.
///
/// The parser is deliberately conservative: on ANY shape it doesn't
/// understand (bad magic, truncated section, unknown import-descriptor kind)
/// it returns `Ok(())` rather than risk a false abort on a valid bundle — the
/// only path that errors is one where it successfully read import module
/// names and found one that is not `loft_gl`/`loft_io`.
pub(crate) fn html_wasm_import_modules_ok(wasm: &[u8]) -> Result<(), Vec<String>> {
    let Some(imports) = html_wasm_imports(wasm) else {
        return Ok(());
    };
    // loft's own host imports (`loft_io` / `loft_gl` / @PLN117's `loft_thread`) AND
    // library-bridge host imports (e.g. the crypto bridge's `loft_crypto.random_fill`)
    // all use a `loft_` prefix.  A wasm-bindgen stomp instead imports `__wbindgen_*`,
    // so a non-`loft_` module is the red flag this guards.  The one exception is the
    // shared MEMORY a threaded build imports as `env.memory` (kind 2) — still no
    // function may come from `env`.
    let bad: std::collections::BTreeSet<String> = imports
        .iter()
        .filter(|(m, _, kind, _)| !(m.starts_with("loft_") || (m == "env" && *kind == 2)))
        .map(|(m, _, _, _)| m.clone())
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.into_iter().collect())
    }
}

/// Every `(module, field, descriptor-kind, result-valtype)` a wasm imports, or `None`
/// if it cannot be walked (bad magic, truncated section, unknown descriptor kind).
///
/// The result valtype is the wasm byte (`0x7F` i32, `0x7E` i64, `0x7D` f32, `0x7C`
/// f64) of a function import's single return value, and `None` for a function that
/// returns nothing or for a non-function import.  [`host_import_stub_js`] needs it:
/// a JS stub standing in for a missing host function has to hand back a value of the
/// declared type, and `undefined` where the module expects an i64 is a TypeError at
/// the call rather than the zero the caller can check.
///
/// The ONE import-section walk.  Four callers read it — the `--html` bundle guard
/// ([`html_wasm_import_modules_ok`]), the page-shell chooser
/// ([`html_wasm_import_modules`]), the host-surface check
/// ([`missing_host_imports`]), and the stub emitter — and they used to carry a copy
/// each of the LEB/name/limits decoding.
///
/// Deliberately conservative: on any shape it does not understand it returns `None`
/// rather than risk a false verdict on a valid bundle.  Callers treat `None` as "no
/// opinion", never as "nothing imported".
pub(crate) fn html_wasm_imports(
    wasm: &[u8],
) -> Option<std::collections::BTreeSet<(String, String, u8, Option<u8>)>> {
    fn read_uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    fn read_name<'a>(b: &'a [u8], p: &mut usize) -> Option<&'a [u8]> {
        let len = usize::try_from(read_uleb(b, p)?).ok()?;
        let s = b.get(*p..p.checked_add(len)?)?;
        *p += len;
        Some(s)
    }
    fn skip_limits(b: &[u8], p: &mut usize) -> Option<()> {
        let flag = *b.get(*p)?;
        *p += 1;
        read_uleb(b, p)?; // min
        if flag & 1 == 1 {
            read_uleb(b, p)?; // max
        }
        Some(())
    }
    // Header: magic "\0asm" + version.
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return None;
    }
    let mut p = 8;
    let mut out = std::collections::BTreeSet::new();
    // Result valtype per type index, filled from the type section.  Wasm orders
    // the known sections, so type (1) is already read when import (2) arrives.
    let mut results: Vec<Option<u8>> = Vec::new();
    while p < wasm.len() {
        let id = *wasm.get(p)?;
        p += 1;
        let size = usize::try_from(read_uleb(wasm, &mut p)?).ok()?;
        let section_start = p;
        let section_end = section_start
            .checked_add(size)
            .filter(|e| *e <= wasm.len())?;
        if id == 1 {
            // Type section: u32 count, then `count` functypes
            // (0x60, params vec, results vec).
            let mut tp = section_start;
            let count = read_uleb(wasm, &mut tp)?;
            for _ in 0..count {
                if *wasm.get(tp)? != 0x60 {
                    return None; // not a functype — no opinion
                }
                tp += 1;
                let params = usize::try_from(read_uleb(wasm, &mut tp)?).ok()?;
                tp = tp.checked_add(params)?;
                let n_res = usize::try_from(read_uleb(wasm, &mut tp)?).ok()?;
                results.push(if n_res == 0 {
                    None
                } else {
                    Some(*wasm.get(tp)?)
                });
                tp = tp.checked_add(n_res)?;
            }
        }
        if id == 2 {
            // Import section: u32 count, then `count` (module, field, desc).
            let mut ip = section_start;
            let count = read_uleb(wasm, &mut ip)?;
            for _ in 0..count {
                let module = read_name(wasm, &mut ip)?;
                let field = read_name(wasm, &mut ip)?;
                let kind = *wasm.get(ip)?;
                ip += 1;
                let mut result = None;
                match kind {
                    0 => {
                        let idx = usize::try_from(read_uleb(wasm, &mut ip)?).ok()?; // func: typeidx
                        result = results.get(idx).copied().flatten();
                    }
                    1 => {
                        ip += 1; // table: reftype byte, then limits
                        skip_limits(wasm, &mut ip)?;
                    }
                    2 => skip_limits(wasm, &mut ip)?, // mem: limits
                    3 => ip += 2,                     // global: valtype + mut byte
                    _ => return None,                 // unknown kind — no opinion
                }
                out.insert((
                    String::from_utf8_lossy(module).into_owned(),
                    String::from_utf8_lossy(field).into_owned(),
                    kind,
                    result,
                ));
            }
        }
        p = section_end;
    }
    Some(out)
}

/// loft#954 — how many of the module's own functions the `name` custom section
/// names, or `None` when there is no such section.
///
/// This is what makes `--names` checkable rather than hopeful.  The section is
/// what turns the `wasm-function[1073]` frames a browser prints for a trap into
/// function names, and it survives only if binaryen was asked for it (`-g`) and
/// not asked to drop it (`--strip-debug`).  A build that quietly emitted no
/// section would hand over a page whose backtrace is exactly as unreadable as
/// before, with nothing to say so.
///
/// Counts the whole function subsection, imports included — the caller wants to
/// know whether names are THERE, and the import names are present in any build
/// because linking needs them.  Conservative like its neighbours: any shape it
/// cannot walk reads as `None`.
pub(crate) fn html_wasm_named_functions(wasm: &[u8]) -> Option<usize> {
    fn read_uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return None;
    }
    let mut p = 8;
    while p < wasm.len() {
        let id = *wasm.get(p)?;
        p += 1;
        let size = usize::try_from(read_uleb(wasm, &mut p)?).ok()?;
        let start = p;
        let end = start.checked_add(size).filter(|e| *e <= wasm.len())?;
        // Custom section (id 0): a name, then the payload.
        if id == 0 {
            let mut q = start;
            let len = usize::try_from(read_uleb(wasm, &mut q)?).ok()?;
            let name = wasm.get(q..q.checked_add(len)?)?;
            q += len;
            if name == b"name" {
                // Subsections, each `id : u8` + `size : uleb` + payload.  Id 1 is
                // the function subsection: a vec of (function index, name).
                while q < end {
                    let sub = *wasm.get(q)?;
                    q += 1;
                    let sub_size = usize::try_from(read_uleb(wasm, &mut q)?).ok()?;
                    let sub_end = q.checked_add(sub_size).filter(|e| *e <= end)?;
                    if sub == 1 {
                        let mut r = q;
                        return usize::try_from(read_uleb(wasm, &mut r)?).ok();
                    }
                    q = sub_end;
                }
                return Some(0);
            }
        }
        p = end;
    }
    None
}

/// JS that gives every `loft_*` host function this wasm imports a stand-in, for
/// the ones the page shim does not define.
///
/// A canvas cannot do everything a desktop window can, and some calls — a
/// screenshot to a `path`, say — may have no browser meaning at all.  That is
/// fine; what is not fine is finding out at BUILD time.  Whether a call can be
/// served is a fact about this run on this target, not about whether the program
/// is well-formed, and refusing the build forces one source to fork into two
/// that differ only in which calls they may NAME — which destroys the property
/// of running the same source on both renderers (loft#709).
///
/// So the page instantiates and the call answers at runtime: the declared zero
/// (`false` / `0` / `""`-shaped null pointer), which every correct caller
/// already handles, since a screenshot can fail for a dozen reasons besides the
/// target.  It says so once per name in the console — once, because a call in a
/// frame loop would otherwise bury the page in identical lines.
///
/// Emitted for EVERY `loft_*` function import and applied only where the name is
/// still free, so the shim and every library bridge keep precedence and a name
/// the build-time scan misjudged is covered anyway.
pub(crate) fn host_import_stub_js(wasm: &[u8]) -> String {
    let Some(imports) = html_wasm_imports(wasm) else {
        return String::new();
    };
    let mut entries = String::new();
    for (module, field, _, result) in imports
        .iter()
        .filter(|(m, _, k, _)| *k == 0 && m.starts_with("loft_"))
    {
        // The value the stub returns, as JS source: wasm coerces the JS result
        // to the declared type, and an i64 result MUST be a BigInt.
        let zero = match result {
            Some(0x7E) => "0n",
            Some(_) => "0",
            None => "undefined",
        };
        entries.push_str(&format!("[{module:?},{field:?},{zero}],"));
    }
    if entries.is_empty() {
        return String::new();
    }
    format!(
        "\n// loft#709: a call this target cannot serve answers at runtime instead of\n\
         // refusing the build.  Only fills names nothing else defined.\n\
         (function(){{const seen=new Set();for(const[m,f,z]of[{entries}]){{\n\
         \x20 const mod=(imports[m]||(imports[m]={{}}));\n\
         \x20 if(f in mod)continue;\n\
         \x20 mod[f]=function(){{ if(!seen.has(f)){{seen.add(f);\n\
         \x20   console.warn('loft: '+f+' is not available in the browser — returning '+String(z));}}\n\
         \x20   return z; }};\n\
         }}}})();\n"
    )
}

/// Host-import names the `--html` page will NOT provide, as `module.field` strings.
///
/// The browser shim implements a SUBSET of `lib/graphics`' native surface — reasonably,
/// since a canvas cannot do everything a desktop window can.  The defect was that the
/// boundary was invisible until the page loaded, and crossing it killed the WHOLE page:
/// `WebAssembly.instantiate` fails with `LinkError: Import #7 "loft_gl"
/// "loft_gl_window_width": function import requires a callable`, so there is no canvas,
/// no output, not even a `println` — and nothing names the loft call responsible
/// (loft#668).
///
/// Both sides are known at build time — the program's imports are in the wasm, and the
/// shim is a fixed file compiled into this binary — so the answer is decidable before the
/// page is ever written.
///
/// `provided_js` is the shim source; a host function is provided iff it appears there as a
/// definition. Reading the shim rather than a hand-kept list is deliberate: a second list
/// would drift from the shim exactly the way `loft install`'s whitelist drifted from
/// `loft package`. Only FUNCTION imports (kind 0) are judged — a threaded build's
/// `env.memory` is supplied by the loader, not the shim.
///
/// Three definition forms count, because JS has three and a library may use any:
/// `name(…) {` (method shorthand), `name:` (property), and `name =` (assignment, as in
/// `ns.ws_yield = function () {…}`). The assignment form was missing, and the omission was
/// not cosmetic — it REJECTED a program whose library bridge did provide the handler, with
/// a message telling the author to add the very handler they had already written
/// (loft#681, `web`'s `[wasm.bridge] host_js`).
///
/// The asymmetry of the two errors decides how strict to be. Failing to recognise a
/// definition BLOCKS a program that would have run; recognising one too eagerly lets a
/// genuinely missing import through to the `LinkError` this check exists to pre-empt —
/// the status quo before it existed. So when the two are in tension, prefer to admit: a
/// false accept costs the diagnostic, a false reject costs the build.
pub(crate) fn missing_host_imports(wasm: &[u8], provided_js: &str) -> Vec<String> {
    let Some(imports) = html_wasm_imports(wasm) else {
        return Vec::new();
    };
    let defined = |name: &str| {
        provided_js.match_indices(name).any(|(i, _)| {
            // The occurrence must be a whole identifier, not a suffix of a longer one.
            if provided_js[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                return false;
            }
            let rest = &provided_js[i + name.len()..];
            let mut after = rest.chars();
            match after.next() {
                // `name(...) {` — a method shorthand; `name:` — a property.
                Some('(' | ':') => true,
                // `name = …` / `name= …` — an assignment.  `==` / `===` is a COMPARISON,
                // not a definition, so it must not count.
                Some('=') => after.next() != Some('='),
                // `name  = …` / `name :` — allow horizontal space before the punctuation
                // only, so a bare mention in prose (`// ws_yield is the shim`) still does
                // not qualify.
                Some(' ' | '\t') => {
                    let t = rest.trim_start_matches([' ', '\t']);
                    t.starts_with(':') || (t.starts_with('=') && !t.starts_with("=="))
                }
                _ => false,
            }
        })
    };
    imports
        .iter()
        .filter(|(m, _, kind, _)| *kind == 0 && m.starts_with("loft_"))
        .filter(|(_, field, _, _)| !defined(field))
        .map(|(m, field, _, _)| format!("{m}.{field}"))
        .collect()
}

/// Return the distinct import-module names of a `loft --html` wasm, or `None`
/// if the module can't be walked (bad magic, truncated/unknown shape).
///
/// `--html` uses this to pick the page shell. A wasm that imports only
/// `loft_io` needs nothing but text I/O, so it gets the minimal engine-less
/// page (a tiny inline shim — no WebGL2, no asyncify, no canvas). Any other
/// module means the program opted into something bigger — `loft_gl` for
/// graphics/audio, or a `loft_<lib>` library bridge — which needs the full
/// engine page. On `None` the caller falls back to the full page, the shell
/// that satisfies every import.
pub(crate) fn html_wasm_import_modules(wasm: &[u8]) -> Option<std::collections::BTreeSet<String>> {
    Some(
        html_wasm_imports(wasm)?
            .into_iter()
            .map(|(m, _, _, _)| m)
            .collect(),
    )
}

pub(crate) fn project_dir() -> String {
    let Ok(prog) = env::current_exe() else {
        return String::new();
    };
    let Some(dir) = prog.parent() else {
        return String::new();
    };
    project_root_for(dir)
}

/// How far above the binary to look for the stdlib.  `target/<triple>/<profile>`
/// is the deepest cargo layout (3 hops); the extra headroom costs one `is_dir`
/// each and keeps a nested workspace target dir working.
const PROJECT_ROOT_SEARCH_DEPTH: usize = 6;

/// Resolve the project root — the directory holding the stdlib `default/` — from
/// the directory the running binary sits in.
///
/// Split out from [`project_dir`] so every layout can be tested without planting a
/// binary in each one.
pub(crate) fn project_root_for(dir: &std::path::Path) -> String {
    // Strip target/release or target/debug to get the project root.
    if (dir.ends_with("target/release") || dir.ends_with("target\\release"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    if (dir.ends_with("target/debug") || dir.ends_with("target\\debug"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    // Installed binary: binary is in <prefix>/bin/, stdlib in <prefix>/share/loft/.
    if dir.ends_with("bin")
        && let Some(prefix) = dir.parent()
    {
        let share_loft = prefix.join("share").join("loft");
        if crate::file_access::is_dir(&share_loft) {
            return with_trailing_sep(&share_loft);
        }
        return with_trailing_sep(prefix);
    }
    // Any other cargo layout.  `--target <triple>` nests the profile one level
    // deeper (`target/<triple>/release`), a custom profile renames it
    // (`target/<profile>`), and `CARGO_TARGET_DIR` renames `target` itself
    // (`target-da/release`).  Rather than enumerate them, walk up to the first
    // ancestor that actually HOLDS the stdlib — the question every caller is
    // really asking.  Without this a `--target` build resolves its own directory
    // as the project root, so `default/` is never found and every stdlib load
    // fails with "cannot load default library" (loft-lang/loft#638: the ASan
    // sweep is the one CI job that builds with `--target`).
    for ancestor in std::iter::successors(Some(dir), |d| d.parent()).take(PROJECT_ROOT_SEARCH_DEPTH)
    {
        if crate::file_access::is_dir(ancestor.join("default")) {
            return with_trailing_sep(ancestor);
        }
    }
    with_trailing_sep(dir)
}

/// P254 — cache-poisoning defense.  Returns true when a cached
/// native binary at `path` is safe to execute under the current
/// uid.  All platforms reject symlinks (a symlink lets an attacker
/// redirect execution to anything they can name).  On Unix we
/// additionally require:
///
/// - The file owner equals the current effective uid (root-owned
///   files are also rejected when running as a non-root user — a
///   common shared-machine attack drops a SUID binary owned by
///   root).
/// - No group/other permission bits set (mode `& 0o077 == 0`).
///
/// Failed checks return false WITHOUT a side effect; the caller
/// recompiles and overwrites the suspect cache file.  We do not
/// `eprintln!` from the helper so unit tests can assert the
/// boolean cleanly; the recompile path's `eprintln!` covers user
/// visibility.
#[must_use]
pub(crate) fn cache_safe_to_execute(path: &std::path::Path) -> bool {
    // symlink_metadata reports the link itself rather than the
    // target; if the target is owned by the current uid but the
    // link itself isn't, plain metadata() would say "safe" and
    // we'd execute attacker-pointed code.
    let lmd = match crate::file_access::symlink_metadata(path) {
        Ok(md) => md,
        Err(_) => return false,
    };
    if lmd.file_type().is_symlink() {
        return false;
    }
    // Owner-only, and no SUID/SGID: a cached binary must never carry a
    // privilege-escalation bit (`platform::is_private_to_owner`).
    crate::platform::is_private_to_owner(&lmd, true)
}

/// P254 — set the cache directory's mode to `0o700` on Unix.
/// No-op on non-Unix (NTFS / ReFS use ACLs that the parent
/// directory already restricts; the symlink check still applies).
/// Called both immediately after `create_dir_all` and before any
/// cache write to repair pre-existing cache directories left over
/// from earlier loft versions.
pub(crate) fn tighten_cache_dir(dir: &std::path::Path) {
    let _ = crate::platform::set_permission_bits(dir, 0o700);
}

/// P254 — set a freshly written cache binary's mode to `0o700`
/// on Unix (rwx for owner, nothing for group/other).  Called
/// after `std::fs::copy(&binary, &cached_binary)`.  No-op on
/// non-Unix.
pub(crate) fn tighten_cache_binary(path: &std::path::Path) {
    let _ = crate::platform::set_permission_bits(path, 0o700);
}

/// Publish a freshly built native binary to its shared, content-keyed cache path.
///
/// The publish is the one step that several concurrent runs of the SAME source contend
/// on, and it must be ATOMIC.  A plain `fs::copy` onto the destination truncates it in
/// place and then streams the whole binary, and for that entire window a concurrent
/// reader sees a path that EXISTS and passes [`cache_safe_to_execute`] — which checks
/// symlink, owner and mode but never SIZE.  The loser exec'd a 0-byte-and-growing ELF
/// and died with no stdout and no stderr, which is a red gate whose only evidence is an
/// empty output block.  Nor does the mode check save it: once the entry exists at 0700,
/// a later copy truncates it while the mode stays 0700.
///
/// So stage under a private per-process name in the SAME directory (a rename cannot
/// cross a filesystem), tighten the mode while the file is still invisible under its
/// final name, then rename.  POSIX rename is atomic, and a process already exec'ing the
/// previous inode keeps it — which is why the test for this is that the destination's
/// INODE changes: publishing in place is precisely the shape that can be truncated
/// under a reader.
pub(crate) fn publish_cached_binary(
    built: &std::path::Path,
    cached_binary: &std::path::Path,
    cache_dir: &std::path::Path,
    source_stem: &str,
) {
    let leaf = crate::file_access::file_name(cached_binary).unwrap_or_default();
    // The leading `.` keeps a staged file out of the sweep's `native-` prefix.
    let staged = cache_dir.join(format!(".{leaf}.{}.tmp", std::process::id()));
    if crate::file_access::copy(built, &staged).is_ok() {
        // P254 — tighten BEFORE the rename, so the binary is never reachable
        // under its final name with a wider mode.
        tighten_cache_binary(&staged);
        if crate::file_access::rename(&staged, cached_binary).is_err() {
            let _ = crate::file_access::remove_file(&staged);
        }
    } else {
        let _ = crate::file_access::remove_file(&staged);
    }
    // Bound the directory AFTER the publish and never touching the entry just written.
    // Sweeping first would delete the very binary a concurrent run has already accepted
    // as usable, leaving it to exec a path that does not exist.
    sweep_cached_binaries(cache_dir, cached_binary, source_stem, NATIVE_CACHE_KEEP);
}

/// How many compiled programs one `.loft/cache` directory keeps.  A binary is named by
/// what it compiles to, not by its source file, so the same program at two paths in one
/// directory shares it, and nothing per-source says when an entry went stale: the
/// directory keeps the most recently USED entries instead (`touch_cached_binary`).
pub(crate) const NATIVE_CACHE_KEEP: usize = 512;

/// Mark a cached binary as just used, so the bound in `sweep_cached_binaries` keeps it.
/// Best-effort: a failure costs a recompile later, never a wrong answer.  The handle rule
/// (read-only on Unix, write on Windows, where a read-only handle cannot set the time and
/// the bound kept nothing by use) is `file_access::set_modified`'s, via `loft::cache::touch_now`.
pub(crate) fn touch_cached_binary(path: &std::path::Path) {
    loft::cache::touch_now(path);
}

/// Keep the `keep` most recently used `native-<key>` binaries in `cache_dir`, never
/// removing `current`.  Also removes this source's entries from the earlier per-source
/// naming (`<stem>-<16 hex>`), which nothing will look up again.
pub(crate) fn sweep_cached_binaries(
    cache_dir: &std::path::Path,
    current: &std::path::Path,
    source_stem: &str,
    keep: usize,
) {
    let is_key = |s: &str| s.len() == 16 && s.bytes().all(|b| b.is_ascii_hexdigit());
    use crate::file_access::{self as fa, PathText};
    let Ok(entries) = fa::read_dir(PathText::from_os(cache_dir)) else {
        return;
    };
    let current = PathText::from_os(current);
    let mut keyed = Vec::new();
    for path in entries {
        if path == current {
            continue;
        }
        let name = path.parts().last().cloned().unwrap_or_default();
        if let Some(key) = name.strip_prefix("native-") {
            // A C build (`c_driver`) is `native-c-<key>` with the host's executable suffix:
            // the same directory, bounded by the same count.
            let key = key.strip_prefix("c-").unwrap_or(key);
            let key = key
                .strip_suffix(std::env::consts::EXE_SUFFIX)
                .filter(|_| !std::env::consts::EXE_SUFFIX.is_empty())
                .unwrap_or(key);
            if is_key(key) {
                let used = fa::metadata(&path).and_then(|m| m.modified()).ok();
                keyed.push((used, path));
                continue;
            }
        }
        if let Some(key) = name
            .strip_prefix(source_stem)
            .and_then(|r| r.strip_prefix('-'))
        {
            if source_stem != "native" && is_key(key) {
                let _ = fa::remove_file(&path);
            }
        }
    }
    // `current` is one of the kept entries.
    let keep_others = keep.saturating_sub(1);
    if keyed.len() > keep_others {
        keyed.sort_by_key(|e| e.0);
        for (_, path) in keyed.drain(..keyed.len() - keep_others) {
            let _ = fa::remove_file(&path);
        }
    }
}

/// Whether the host-native backend links `#native` packages by C-ABI (their
/// cdylib `.so`) instead of as Rust rlibs — see NATIVE_ARTIFACT_IDENTITY.md § Resolution: separate
/// the API id from the Rust part.  Both the codegen (`Output::native_cabi`) and
/// the linker flags below read this, so they always agree on a given host.
///
/// @PLN26 phase 4 — the C-ABI path (import-library linking + DLL staged beside
/// the binary on Windows, an RPATH'd `.so` elsewhere) is now the default on
/// EVERY host.  Windows was the last holdout; its arm was verified green in the
/// focused CI (`win-cdylib.yml` job `win-cdylib-cabi`, `LOFT_NATIVE_CABI=1`,
/// `native_crate_package_links_and_runs_via_cabi` PASS on `windows-latest`)
/// before this flip.  `LOFT_NATIVE_CABI=0` remains an escape hatch that forces
/// the legacy rlib link on any host should the C-ABI path regress; `=1` is now a
/// no-op (it already matches the default).
#[must_use]
pub(crate) fn native_cabi_enabled() -> bool {
    // The C-ABI path is the default on every host.  A STATIC link — each native package's
    // rlib linked INTO the binary as `extern crate`, one file with nothing to ship beside it —
    // is chosen by, strongest first: `--static` / `--dynamic`, the program's `#link`
    // directive, `LOFT_NATIVE_CABI=0` (the old escape hatch).
    match LINK.load(std::sync::atomic::Ordering::Relaxed) {
        LINK_STATIC => false,
        LINK_DYNAMIC => true,
        _ => !matches!(std::env::var("LOFT_NATIVE_CABI").ok().as_deref(), Some("0")),
    }
}

/// @PLN195 — the directories of the package libraries this program links, as the build
/// resolved them, for a program the driver runs where it was built: they reach it through
/// `LOFT_LIB_PATH` (ahead of any the user set), never through a path written into the
/// binary.  `None` when the program binds nothing itself.
pub(crate) fn lib_path_env(data: &crate::data::Data) -> Option<std::ffi::OsString> {
    if package_libraries(data).is_empty() {
        return None;
    }
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    for lib in package_libraries(data) {
        // Absolute: the program runs from its source directory, not where loft was started,
        // so a directory relative to the start is not one it can find (loft#2234).
        if let Some(dir) = lib.parent().map(crate::file_access::plain_canonical)
            && !dirs.contains(&dir)
        {
            dirs.push(dir);
        }
    }
    if let Some(user) = std::env::var_os("LOFT_LIB_PATH") {
        dirs.extend(std::env::split_paths(&user));
    }
    std::env::join_paths(dirs).ok()
}

/// Every library file the program needs beside it at run time, resolved the way the link
/// resolved it: each native package's library under a dynamic link (`resolve_native_lib`),
/// and each C library a package ships itself (@PLN197) — under a static link only the ones
/// that have no static archive to link into the binary.
fn package_libraries(data: &crate::data::Data) -> Vec<std::path::PathBuf> {
    package_libraries_by_package(data)
        .into_iter()
        .map(|(_, _, lib)| lib)
        .collect()
}

/// [`package_libraries`], each with its package directory and the name the package goes by
/// when its manifest names none — the same fallback the bind table is written with: the
/// crate name for a native package, the directory's name for a C library.
fn package_libraries_by_package(
    data: &crate::data::Data,
) -> Vec<(String, String, std::path::PathBuf)> {
    let dynamic = native_cabi_enabled();
    let mut out = Vec::new();
    if dynamic {
        for (crate_name, pkg_dir) in &data.native_packages {
            let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
                .and_then(|m| m.native)
                .unwrap_or_else(|| crate_name.replace('-', "_"));
            // A C build loads only what is already built — a component — and builds nothing
            // here (`c_target`).
            let so = if loft::native_build::c_target() {
                loft::native_build::prebuilt_native_lib(pkg_dir)
            } else {
                loft::interp::extensions::resolve_native_lib(pkg_dir, &stem)
            };
            if let Some(so) = so {
                out.push((
                    pkg_dir.clone(),
                    crate_name.clone(),
                    std::path::PathBuf::from(so),
                ));
            }
        }
    }
    for (lib, path) in data.shipped_c_libraries() {
        if !dynamic && loft::c_shim::archived_package(data, &lib.pkg_dir) {
            continue;
        }
        let fallback = crate::file_access::file_name(lib.pkg_dir.as_str()).unwrap_or_default();
        out.push((lib.pkg_dir.clone(), fallback, path));
    }
    out
}

/// @PLN195 — `--out <dir>` / `loft build native`: the program `built`, named `name`, and
/// every library file it links, side by side in `dir` — a directory that runs wherever it is
/// copied, since the binary looks for its libraries beside itself and names no directory of
/// the machine that built it.  A static link writes the binary alone.
pub(crate) fn write_dist(
    dir: &std::path::Path,
    built: &std::path::Path,
    name: &str,
    data: &crate::data::Data,
) -> Result<(), String> {
    crate::file_access::create_dir_all(dir).map_err(|e| e.to_string())?;
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    crate::file_access::copy(built, exe.as_path()).map_err(|e| e.to_string())?;
    // The build cache keeps its binaries owner-only (`tighten_cache_binary`); a program
    // written out to ship is an ordinary executable.
    let _ = crate::platform::set_permission_bits(exe.as_path(), 0o755);
    for lib in package_libraries(data) {
        let Some(file) = crate::file_access::file_name(lib.as_path()) else {
            continue;
        };
        crate::file_access::copy(lib.as_path(), dir.join(file).as_path())
            .map_err(|e| format!("{}: {e}", lib.display()))?;
    }
    Ok(())
}

const LINK_UNSET: u8 = 0;
const LINK_STATIC: u8 = 1;
const LINK_DYNAMIC: u8 = 2;
/// The run's link mode, once a flag or the program's `#link` directive has named one.
static LINK: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(LINK_UNSET);
/// Whether a FLAG set [`LINK`]: a flag outranks the program's directive.
static LINK_BY_FLAG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--static` (`true`) / `--dynamic` (`false`): this run's link mode, over the program's own
/// `#link` directive.
pub(crate) fn link_by_flag(static_link: bool) {
    LINK.store(
        if static_link {
            LINK_STATIC
        } else {
            LINK_DYNAMIC
        },
        std::sync::atomic::Ordering::Relaxed,
    );
    LINK_BY_FLAG.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// `--ship` — a shipped program links its native libraries INTO itself unless a flag or the
/// program's `#link` directive named a mode: one file, and a library function called directly
/// rather than through a bound slot and `__tls_get_addr` (NATIVE.md § Optimisation tiers).  Called after
/// [`apply_link_directive`], so it ranks below both.
pub(crate) fn link_static_unless_named() {
    let _ = LINK.compare_exchange(
        LINK_UNSET,
        LINK_STATIC,
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// The link mode a program's header names — `#link static` or `#link dynamic` among the
/// directives before its first declaration — applied unless a flag named one.  Read from the
/// SOURCE, before any parse: the native fast path runs a cached binary without parsing, and
/// its key holds the link mode, so a mode known only after the parse would key the lookup
/// and the save differently.  The parser checks the same lines (`Parser::parse_file`).
pub(crate) fn apply_link_directive(source: &str) {
    if LINK_BY_FLAG.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Some(static_link) = link_directive(source) {
        LINK.store(
            if static_link {
                LINK_STATIC
            } else {
                LINK_DYNAMIC
            },
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// `Some(true)` for `#link static`, `Some(false)` for `#link dynamic`, `None` when the header
/// names no mode.  The header is a `#!` first line, then blank and `//` lines and `#`
/// directives, up to the first line that is none of those.
pub(crate) fn link_directive(source: &str) -> Option<bool> {
    for (n, line) in source.lines().enumerate() {
        let line = line.trim();
        if (n == 0 && line.starts_with("#!")) || line.is_empty() || line.starts_with("//") {
            continue;
        }
        let directive = line.strip_prefix('#')?;
        let mut words = directive.trim_end_matches(';').split_whitespace();
        if words.next() == Some("link") {
            return match words.next() {
                Some("static") => Some(true),
                Some("dynamic") => Some(false),
                _ => None,
            };
        }
    }
    None
}

/// Say something useful when a Windows binary dies before `main`.
///
/// `STATUS_DLL_NOT_FOUND` (`0xC000_0135`) is the loader refusing to start a
/// process whose imports it cannot resolve. It happens BEFORE any user code, so
/// the program writes nothing at all: no stdout, no stderr, just an exit code
/// most people have never seen. That silence is the worst part of the failure,
/// and it is entirely avoidable — loft knows which libraries it staged and where.
///
/// Reports what IS beside the binary rather than guessing which import is
/// missing: naming the missing DLL needs the import table, and the useful
/// question for an author is almost always "did my library get here at all".
/// A `[c]` library resolved from the system (a bare soname) is deliberately not
/// listed — it was never loft's to place.
///
/// No-op off Windows and for any other exit code.
pub(crate) fn explain_windows_startup_failure(
    status: std::process::ExitStatus,
    binary: &std::path::Path,
    data: &crate::data::Data,
) {
    const STATUS_DLL_NOT_FOUND: i32 = crate::platform::STATUS_DLL_NOT_FOUND;
    if !crate::platform::is_dll_not_found(status) {
        return;
    }
    eprintln!(
        "loft: the program could not start — Windows could not find a DLL it \
         imports (STATUS_DLL_NOT_FOUND, 0x{STATUS_DLL_NOT_FOUND:08X}). This \
         happens before any of your code runs, which is why nothing was printed."
    );
    if let Some(dir) = binary.parent() {
        let mut staged: Vec<String> = crate::file_access::read_dir(dir)
            .into_iter()
            .flatten()
            .map(|e| e.file_name().unwrap_or_default().to_string())
            .filter(|n| n.to_ascii_lowercase().ends_with(".dll"))
            .collect();
        staged.sort();
        if staged.is_empty() {
            eprintln!("loft:   no DLLs are staged beside `{}`", dir.display());
        } else {
            eprintln!(
                "loft:   staged beside `{}`: {}",
                dir.display(),
                staged.join(", ")
            );
        }
    }
    for lib in &data.c_libraries {
        let d = std::path::Path::new(&lib.pkg_dir);
        if let Some(found) =
            crate::platform::existing_lib_beside(d, &lib.name, crate::platform::host_lib_os())
        {
            eprintln!("loft:   `[c]` library shipped by the package: {found}");
        }
    }
    eprintln!(
        "loft:   Windows has no RPATH: a DLL is found beside the executable or \
         on PATH. A MinGW-built library may also need its own runtime \
         (libgcc_s_seh-1.dll, libwinpthread-1.dll) on PATH."
    );
}

/// @PLN24 — the `[c]` half of [`stage_native_dlls`]: a C library that ships
/// BESIDE its package has the same run-time problem as a native-package cdylib,
/// and had none of the answer.
///
/// The link succeeds (the import library is right there), and then the binary
/// dies before `main` with `STATUS_DLL_NOT_FOUND` — writing nothing at all, so
/// the failure presents as an empty stdout and an empty stderr. Windows has no
/// RPATH to fall back on, which is exactly why `add_c_library_flags` stopped
/// emitting one there.
///
/// Only libraries that are really PRESENT beside the package are copied:
/// `existing_lib_beside` answers `None` for a bare soname like
/// `libsqlite3.so.0`, and that is the correct answer — a system library is the
/// system's to find, and copying one next to the binary would be wrong.
/// Best-effort, like its sibling: a failed copy leaves the loader's normal
/// search to do what it can.
fn stage_c_library_dlls(exe_dir: &std::path::Path, data: &crate::data::Data) {
    for lib in &data.c_libraries {
        let dir = std::path::Path::new(&lib.pkg_dir);
        let Some(found) =
            crate::platform::existing_lib_beside(dir, &lib.name, crate::platform::host_lib_os())
        else {
            continue; // a bare soname — the system's to resolve, not ours to copy
        };
        let src = dir.join(&found);
        if let Some(name) = crate::file_access::file_name(&src) {
            let dest = exe_dir.join(name);
            if dest != src {
                let _ = crate::file_access::copy(&src, &dest);
            }
        }
    }
}

/// Stage, beside a just-built Windows binary, every C library that ships beside its package
/// ([`stage_c_library_dlls`]): Windows has no RPATH, and the loader finds a DLL beside the
/// `.exe` or on `PATH`.  A native package's own library needs no staging — the program binds
/// it itself (@PLN195, `loft::native_bind`).  No-op off Windows.
pub(crate) fn stage_native_dlls(exe_dir: &std::path::Path, data: &crate::data::Data) {
    if !crate::platform::stages_dlls_beside_binary() {
        return;
    }
    stage_c_library_dlls(exe_dir, data);
}

/// The stdlib must be findable from EVERY cargo layout, not just
/// `target/{release,debug}`.  A `--target <triple>` build nests the profile directory
/// one level deeper, so the project root cannot be derived by assuming a fixed depth
/// above the binary.
///
/// ⚠ Getting this wrong fails far from its cause: `default/` is simply not found, every
/// stdlib load dies with "cannot load default library", and the only job that builds with
/// `--target` is the nightly ASan sweep — so it surfaces there, as an unrelated
/// sandbox-admission test. (loft#638)
#[cfg(test)]
mod project_root_layouts {
    use super::*;

    fn chrono_ish_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    }

    /// A throwaway project root that HOLDS a `default/` dir, plus the binary dir
    /// under it named by `rel`.  Returns (root, binary dir).
    fn layout(tag: &str, rel: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "loft_root_{tag}_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(root.join("default")).unwrap();
        let bin_dir = root.join(rel);
        crate::file_access::create_dir_all(&bin_dir).unwrap();
        (root, bin_dir)
    }

    #[test]
    fn plain_release_and_debug_layouts_resolve_to_the_root() {
        for rel in ["target/release", "target/debug"] {
            let (root, bin_dir) = layout("plain", rel);
            assert_eq!(
                project_root_for(&bin_dir),
                with_trailing_sep(&root),
                "{rel} must resolve to the project root"
            );
            let _ = crate::file_access::remove_dir_all(&root);
        }
    }

    /// The regression: `cargo --target <triple>` (how the ASan gate instruments
    /// the whole build) puts the binary one level deeper.
    #[test]
    fn explicit_target_triple_layout_resolves_to_the_root() {
        for rel in [
            "target/x86_64-unknown-linux-gnu/release",
            "target/aarch64-apple-darwin/release",
            "target/x86_64-unknown-linux-gnu/debug",
        ] {
            let (root, bin_dir) = layout("triple", rel);
            assert_eq!(
                project_root_for(&bin_dir),
                with_trailing_sep(&root),
                "{rel} must resolve to the project root, not the binary's own dir"
            );
            let _ = crate::file_access::remove_dir_all(&root);
        }
    }

    /// `CARGO_TARGET_DIR=target-da` (the debug-assertions calibration build) and
    /// custom cargo profiles rename the dirs; the stdlib must still be found.
    #[test]
    fn renamed_target_dir_and_custom_profile_resolve_to_the_root() {
        for rel in ["target-da/release", "target/ci", "target-da/x86_64-foo/ci"] {
            let (root, bin_dir) = layout("renamed", rel);
            assert_eq!(
                project_root_for(&bin_dir),
                with_trailing_sep(&root),
                "{rel} must resolve to the project root"
            );
            let _ = crate::file_access::remove_dir_all(&root);
        }
    }

    /// An unpacked release dir ships the binary NEXT TO `default/` — the walk must
    /// stop there rather than climbing past it.
    #[test]
    fn binary_beside_the_stdlib_resolves_to_its_own_dir() {
        let (root, _) = layout("beside", "unused");
        assert_eq!(project_root_for(&root), with_trailing_sep(&root));
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// No stdlib anywhere above: unchanged fallback to the binary's own dir (the
    /// caller then reports a missing stdlib, which is the honest answer).
    #[test]
    fn no_stdlib_above_falls_back_to_the_binary_dir() {
        let dir = std::env::temp_dir().join(format!(
            "loft_root_none_{}_{}/deep/deeper",
            std::process::id(),
            chrono_ish_nanos()
        ));
        crate::file_access::create_dir_all(&dir).unwrap();
        assert_eq!(project_root_for(&dir), with_trailing_sep(&dir));
        let _ = crate::file_access::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }
}

#[cfg(test)]
mod p254_cache_safety {
    use super::*;

    #[test]
    fn nonexistent_cache_is_unsafe() {
        let p = std::env::temp_dir().join("loft_p254_does_not_exist_xyz_12345");
        let _ = crate::file_access::remove_file(&p);
        assert!(!cache_safe_to_execute(&p));
    }

    #[test]
    fn freshly_written_owner_only_cache_is_safe() {
        let p = std::env::temp_dir().join(format!(
            "loft_p254_safe_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = crate::file_access::remove_file(&p);
        crate::file_access::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        tighten_cache_binary(&p);
        assert!(cache_safe_to_execute(&p), "owner-only file should be safe");
        let _ = crate::file_access::remove_file(&p);
    }

    /// A host without mode bits (Windows) has nothing to reject here, and answers "safe"
    /// (`platform::is_private_to_owner`); the assertion names each platform's answer.
    #[test]
    fn group_writable_cache_is_unsafe() {
        let p = std::env::temp_dir().join(format!(
            "loft_p254_groupwrite_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = crate::file_access::remove_file(&p);
        crate::file_access::write(&p, b"x").unwrap();
        // 0o766 has group write and other rwx — attacker-modifiable.
        crate::platform::set_permission_bits(&p, 0o766).unwrap();
        assert_eq!(
            cache_safe_to_execute(&p),
            !crate::platform::has_permission_bits()
        );
        let _ = crate::file_access::remove_file(&p);
    }

    // approved exemption: a test of Unix-only semantics (symlinks, inodes, a read-only sysroot); Windows has none of them to test
    #[cfg(unix)]
    #[test]
    fn symlink_cache_is_unsafe() {
        let target = std::env::temp_dir().join(format!(
            "loft_p254_symtarget_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let link = std::env::temp_dir().join(format!(
            "loft_p254_symlink_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = crate::file_access::remove_file(&target);
        let _ = crate::file_access::remove_file(&link);
        crate::file_access::write(&target, b"x").unwrap();
        tighten_cache_binary(&target);
        crate::file_access::symlink(&target, &link).unwrap();
        // Even though the symlink TARGET would pass, the link
        // itself routes through `symlink_metadata` and is rejected.
        assert!(!cache_safe_to_execute(&link));
        let _ = crate::file_access::remove_file(&link);
        let _ = crate::file_access::remove_file(&target);
    }

    /// As `group_writable_cache_is_unsafe`: a host without mode bits answers "safe".
    #[test]
    fn suid_cache_is_unsafe() {
        let p = std::env::temp_dir().join(format!(
            "loft_p254_suid_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = crate::file_access::remove_file(&p);
        crate::file_access::write(&p, b"x").unwrap();
        // 0o4700 — owner rwx + setuid bit.
        crate::platform::set_permission_bits(&p, 0o4700).unwrap();
        assert_eq!(
            cache_safe_to_execute(&p),
            !crate::platform::has_permission_bits()
        );
        let _ = crate::file_access::remove_file(&p);
    }

    /// Lightweight nanosecond suffix to keep test temp paths
    /// from colliding when `cargo test` runs the cases in
    /// parallel without `--test-threads=1`.
    fn chrono_ish_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    }
}

#[cfg(test)]
mod p350_html_wasm_import_check {
    use super::*;

    /// Build a minimal valid wasm (header + one import section) whose single
    /// import has the given module name and is a function import (kind 0).
    /// A minimal wasm importing exactly `module.field` as a function (kind 0).
    fn wasm_with_import(module: &str, field: &str) -> Vec<u8> {
        let mut entry = Vec::new();
        entry.push(u8::try_from(module.len()).expect("short module"));
        entry.extend_from_slice(module.as_bytes());
        entry.push(u8::try_from(field.len()).expect("short field"));
        entry.extend_from_slice(field.as_bytes());
        entry.push(0); // kind: func
        entry.push(0); // typeidx
        let mut body = vec![1u8]; // one import
        body.extend_from_slice(&entry);
        let mut wasm = b"\0asm\x01\x00\x00\x00".to_vec();
        wasm.push(2); // import section
        wasm.push(u8::try_from(body.len()).expect("small section"));
        wasm.extend_from_slice(&body);
        wasm
    }

    fn wasm_with_import_module(module: &str) -> Vec<u8> {
        // import entry: <ulen module><module><ulen field><field><kind=0><typeidx=0>
        let field = "f";
        let mut entry = Vec::new();
        entry.push(module.len() as u8);
        entry.extend_from_slice(module.as_bytes());
        entry.push(field.len() as u8);
        entry.extend_from_slice(field.as_bytes());
        entry.push(0x00); // kind: func
        entry.push(0x00); // typeidx 0
        // import section body: <count=1><entry>
        let mut body = vec![0x01u8];
        body.extend_from_slice(&entry);
        // section: <id=2><usize len><body>
        let mut wasm = b"\0asm".to_vec();
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1
        wasm.push(0x02); // import section id
        wasm.push(body.len() as u8); // section size (small enough for one byte uleb)
        wasm.extend_from_slice(&body);
        wasm
    }

    #[test]
    fn accepts_loft_gl_and_loft_io() {
        assert!(html_wasm_import_modules_ok(&wasm_with_import_module("loft_gl")).is_ok());
        assert!(html_wasm_import_modules_ok(&wasm_with_import_module("loft_io")).is_ok());
    }

    #[test]
    fn rejects_wbindgen_stomp() {
        let bad = html_wasm_import_modules_ok(&wasm_with_import_module("__wbindgen_placeholder__"))
            .expect_err("wbindgen import module must be rejected");
        assert_eq!(bad, vec!["__wbindgen_placeholder__".to_string()]);
    }

    #[test]
    fn no_imports_is_ok() {
        // Bare header, no sections — a wasm with zero imports is acceptable.
        let mut wasm = b"\0asm".to_vec();
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        assert!(html_wasm_import_modules_ok(&wasm).is_ok());
    }

    #[test]
    fn malformed_does_not_false_abort() {
        // Bad magic / truncated input must be conservatively accepted (Ok),
        // never a false rejection of a bundle this parser can't read.
        assert!(html_wasm_import_modules_ok(b"not a wasm").is_ok());
        assert!(html_wasm_import_modules_ok(&[]).is_ok());
        assert!(html_wasm_import_modules_ok(b"\0asm\x01\x00\x00\x00\x02\xff").is_ok());
    }

    /// loft#668 — a host function the page will not provide is decidable at BUILD time,
    /// so it must be reported by name instead of becoming a `LinkError` at instantiate
    /// that names an import index and kills the whole page.
    #[test]
    fn missing_host_import_is_named() {
        let wasm = wasm_with_import("loft_gl", "loft_gl_window_width");
        // Absent from the shim → reported, module-qualified.
        assert_eq!(
            missing_host_imports(&wasm, "loft_gl_use_shader(p) {}"),
            vec!["loft_gl.loft_gl_window_width".to_string()],
        );
        // Present as a method, and as a property — both are definitions.
        let unexpected = missing_host_imports(&wasm, "loft_gl_window_width() { return 1; }");
        assert!(unexpected.is_empty(), "{unexpected:?}");
        let unexpected = missing_host_imports(&wasm, "loft_gl_window_width: () => 1,");
        assert!(unexpected.is_empty(), "{unexpected:?}");
        // A LONGER name that merely CONTAINS the import must not count as providing it —
        // otherwise `loft_gl_window_width_ex` would mask the real gap.
        assert_eq!(
            missing_host_imports(&wasm, "x_loft_gl_window_width() {}").len(),
            1,
            "a longer identifier ending in the name does not define it"
        );
        // The shipped shim really does provide it (this is what the fix added).
        assert!(
            missing_host_imports(&wasm, include_str!("../doc/loft-gl-wasm.js")).is_empty(),
            "doc/loft-gl-wasm.js must define loft_gl_window_width"
        );
        // Unwalkable input yields no opinion, never a false build failure.
        let unexpected = missing_host_imports(b"not a wasm", "");
        assert!(unexpected.is_empty(), "{unexpected:?}");
    }

    /// loft#669 — every handle table in the browser shim hands out 1-BASED handles, so
    /// `0` means failure and nothing else, as `lib/graphics` documents and as the native
    /// backend behaves.  Pinned by reading the shim: a table that goes back to returning
    /// its array index would make the FIRST shader/VAO/texture on a page indistinguishable
    /// from a failure.
    #[test]
    fn browser_gl_handles_are_one_based() {
        let js = include_str!("../doc/loft-gl-wasm.js");
        assert!(
            !js.contains("programs.length; programs.push")
                && !js.contains("vaos.length; vaos.push")
                && !js.contains("textures.length; textures.push"),
            "a handle table returned its 0-based array index"
        );
        // `hold` returns Array.prototype.push's value — the new LENGTH, i.e. index + 1.
        assert!(
            js.contains("const hold = (arr, obj) => arr.push(obj)"),
            "handles must be minted through `hold` (1-based)"
        );
        assert!(
            js.contains("const slot = (arr, h) => (h > 0 && h <= arr.length ? arr[h - 1] : null)"),
            "handles must be resolved through `slot` (rejects 0)"
        );
    }
}

#[cfg(test)]
mod atomics_sysroot_tests {
    use super::*;

    /// Build a fake shape dir `<root>/<triple>/release` with one dep rlib in it,
    /// mirroring what `ensure_loft_runtime_rlib` hands `ensure_atomics_sysroot`.
    fn fake_profile_dir(root: &std::path::Path) -> std::path::PathBuf {
        let profile = root.join("wasm32-unknown-unknown").join("release");
        let deps = profile.join("deps");
        crate::file_access::create_dir_all(&deps).expect("deps");
        crate::file_access::write(deps.join("libcore-abc.rlib"), b"rlib").expect("dep rlib");
        // loft's own rlib must be EXCLUDED from the sysroot (duplicate-candidate
        // errors at link time), so seed one to prove the filter still runs.
        crate::file_access::write(deps.join("libloft-def.rlib"), b"rlib").expect("loft rlib");
        profile
    }

    /// The dev layout is unchanged: the sysroot lands beside the target dir.
    #[test]
    fn assembles_next_to_the_target_dir_when_writable() {
        let root = std::env::temp_dir().join(format!("loft_sysroot_dev_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        let profile = fake_profile_dir(&root);
        let got = ensure_atomics_sysroot(&profile).expect("sysroot assembled");
        assert_eq!(got, root.join("sysroot"), "dev layout must not move");
        let lib = got
            .join("lib/rustlib/wasm32-unknown-unknown/lib")
            .join("libcore-abc.rlib");
        assert!(
            crate::file_access::exists(&lib),
            "dep rlib copied into the sysroot"
        );
        assert!(
            !crate::file_access::exists(
                got.join("lib/rustlib/wasm32-unknown-unknown/lib/libloft-def.rlib")
            ),
            "loft's own rlib must stay out of the sysroot"
        );
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// #619 — the installed layout is root-owned, so the derived location cannot
    /// be created.  The assembly must fall back to the user cache instead of
    /// giving up, which is what silently produced a single-threaded page.
    #[test]
    // approved exemption: a test of Unix-only semantics (symlinks, inodes, a read-only sysroot); Windows has none of them to test
    #[cfg(unix)]
    fn falls_back_to_the_user_cache_when_the_target_dir_is_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("loft_sysroot_ro_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&base);
        let root = base.join("share").join("loft");
        let profile = fake_profile_dir(&root);
        let home = base.join("home");
        crate::file_access::create_dir_all(&home).expect("home");

        // Make the derived sysroot parent unwritable — the installed-layout shape.
        let mut perms = crate::file_access::metadata(&root)
            .expect("meta")
            .permissions();
        perms.set_mode(0o555);
        crate::file_access::set_permissions(&root, perms).expect("chmod ro");

        // SAFETY: single-threaded test process; restored below.
        unsafe { std::env::set_var("LOFT_HOME", &home) };
        let got = ensure_atomics_sysroot(&profile);
        unsafe { std::env::remove_var("LOFT_HOME") };

        let mut perms = crate::file_access::metadata(&root)
            .expect("meta")
            .permissions();
        perms.set_mode(0o755);
        let _ = crate::file_access::set_permissions(&root, perms);

        let got = got.expect("must fall back rather than return None");
        assert!(
            got.starts_with(&home),
            "expected the user cache under {home:?}, got {got:?}"
        );
        assert!(
            crate::file_access::exists(
                got.join("lib/rustlib/wasm32-unknown-unknown/lib/libcore-abc.rlib")
            ),
            "dep rlib copied into the fallback sysroot"
        );
        let _ = crate::file_access::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod host_import_tests {
    use super::missing_host_imports;

    /// A minimal wasm importing exactly one function: `loft_web.ws_yield`.
    /// Hand-built so the test does not need a toolchain: magic + version, a type
    /// section with one `() -> ()`, and an import section naming the function.
    fn wasm_importing(module: &str, field: &str) -> Vec<u8> {
        let mut w = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        // type section: one type, `() -> ()`
        w.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
        // import section: one entry, kind 0 (function), type index 0
        let mut body = vec![0x01];
        body.push(u8::try_from(module.len()).unwrap());
        body.extend_from_slice(module.as_bytes());
        body.push(u8::try_from(field.len()).unwrap());
        body.extend_from_slice(field.as_bytes());
        body.extend_from_slice(&[0x00, 0x00]);
        w.push(0x02);
        w.push(u8::try_from(body.len()).unwrap());
        w.extend_from_slice(&body);
        w
    }

    /// loft#681 — `ns.ws_yield = function () {…}` is how a library's
    /// `[wasm.bridge] host_js` installs a handler, and it was invisible to the check.
    /// The program was refused with a message telling its author to add the handler
    /// they had already written, and there was no way to overrule it.
    #[test]
    fn assignment_form_counts_as_provided() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        let js = "  ns.ws_yield = function () { return 0; };\n";
        assert!(
            missing_host_imports(&wasm, js).is_empty(),
            "an assigned handler is provided"
        );
    }

    /// The two forms that already worked must keep working.
    #[test]
    fn method_and_property_forms_still_count() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        assert!(
            missing_host_imports(&wasm, "  ws_yield() { return 0; },\n").is_empty(),
            "method shorthand"
        );
        assert!(
            missing_host_imports(&wasm, "  ws_yield: () => 0,\n").is_empty(),
            "property"
        );
        assert!(
            missing_host_imports(&wasm, "  ws_yield : () => 0,\n").is_empty(),
            "property with a space before the colon is still a colon form"
        );
    }

    /// The check must still be able to FAIL, or widening it would have quietly
    /// disabled loft#668's whole point. A prose mention is not a definition, and
    /// neither is a comparison.
    #[test]
    fn a_mention_or_comparison_is_not_a_definition() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        for js in [
            "// ws_yield is the asyncify suspend shim\n",
            "if (name === ws_yield) { }\n",
            "  other_thing = function () {};\n",
            "",
        ] {
            assert_eq!(
                missing_host_imports(&wasm, js),
                vec!["loft_web.ws_yield".to_string()],
                "must report missing for: {js:?}"
            );
        }
    }

    /// A longer identifier that merely ENDS with the name is a different function.
    #[test]
    fn a_longer_identifier_is_not_the_same_name() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        assert_eq!(
            missing_host_imports(&wasm, "  do_ws_yield = function () {};\n"),
            vec!["loft_web.ws_yield".to_string()],
            "`do_ws_yield` does not define `ws_yield`"
        );
    }
}

#[cfg(test)]
mod crate_resolution_tests {
    use super::crate_resolution_failure;

    // ── loft#706 — what may trigger a runtime rebuild + retry ────────────────────
    //
    // The classifier is the safety half of the heal: only a toolchain/cache failure
    // may cost the user a rebuild.  A codegen bug must fall straight through, or every
    // broken program pays a minute before seeing its error.

    /// The shape that motivated the issue: the rlib was built by a different rustc
    /// than the one compiling against it.  A rebuild is exactly the cure.
    #[test]
    fn an_incompatible_rlib_is_a_crate_resolution_failure() {
        let stderr = "error[E0514]: found crate `loft` compiled by an incompatible \
                      version of rustc\n = help: please recompile that crate";
        assert!(crate_resolution_failure(stderr));
    }

    /// The same staleness seen through a transitive dep.
    #[test]
    fn a_newer_transitive_crate_is_a_crate_resolution_failure() {
        let stderr = "error[E0460]: found possibly newer version of crate `rand_core`";
        assert!(crate_resolution_failure(stderr));
    }

    /// A bundle that ships no rlib.
    #[test]
    fn a_missing_crate_is_a_crate_resolution_failure() {
        let stderr = "error[E0463]: can't find crate for `loft`";
        assert!(crate_resolution_failure(stderr));
    }

    /// A genuine codegen bug must NOT heal — this is the guard that keeps a rebuild
    /// off the path of every broken program.
    #[test]
    fn a_codegen_error_is_not_a_crate_resolution_failure() {
        for stderr in [
            "error[E0308]: mismatched types\n expected `DbRef`, found `()`",
            "error[E0425]: cannot find value `var_x` in this scope",
            "error[E0506]: cannot assign to `var___work_c1` because it is borrowed",
            "error: aborting due to 1 previous error",
        ] {
            assert!(
                !crate_resolution_failure(stderr),
                "must not rebuild for: {stderr:?}"
            );
        }
    }

    /// BOTH halves are required.  A bare code with no matching explanation is not
    /// enough: `E0463` also fires for a user's own missing `--lib` package, which no
    /// rebuild of loft's runtime can fix.
    #[test]
    fn a_bare_error_code_without_its_explanation_does_not_heal() {
        assert!(!crate_resolution_failure(
            "error[E0463]: something else entirely"
        ));
        assert!(!crate_resolution_failure(
            "compiled by an incompatible version"
        ));
    }
}

#[cfg(test)]
mod dep_search_dirs_tests {
    use super::dep_search_dirs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join("loft-dep-search").join(name);
        let _ = crate::file_access::remove_dir_all(&d);
        crate::file_access::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &std::path::Path) {
        crate::file_access::create_dir_all(p.parent().unwrap()).unwrap();
        crate::file_access::write(p, b"x").unwrap();
    }

    /// The classic layout must keep yielding exactly ONE search dir — the per-unit
    /// fallback is a fallback, not a replacement, and a change here would multiply
    /// every `--native` compile's `-L` count on the platform that works today.
    #[test]
    fn the_classic_single_deps_layout_yields_one_dir() {
        let root = scratch("classic");
        touch(&root.join("deps").join("libsha2-abc.rlib"));
        assert_eq!(dep_search_dirs(&root), vec![root.join("deps")]);
    }

    /// cargo 1.99.0-nightly (2026-07-29) gives each crate its own
    /// `build/<crate>/<hash>/out/`, and no `deps/` at all.
    #[test]
    fn the_per_unit_layout_yields_one_dir_per_crate() {
        let root = scratch("per-unit");
        touch(&root.join("build/sha2/h1/out/libsha2-h1.rlib"));
        touch(&root.join("build/serde/h2/out/libserde-h2.rlib"));
        // A proc-macro ships a dylib, never an rlib.  Excluding these is what made an
        // rlib-only filter fail one crate later on `curve25519_dalek_derive` —
        // indistinguishable, from the error alone, from the fix not working.
        touch(&root.join("build/derive/h3/out/libderive-h3.so"));
        // A build-script output dir carries generated sources, not libraries: passing
        // it would be noise, and there are ~100 of them.
        touch(&root.join("build/somelib/h4/out/generated.rs"));

        let dirs = dep_search_dirs(&root);
        assert_eq!(dirs.len(), 3, "{dirs:?}");
        for c in ["sha2", "serde", "derive"] {
            assert!(
                dirs.iter().any(|d| d.to_string_lossy().contains(c)),
                "{c} missing from {dirs:?}"
            );
        }
        assert!(
            !dirs.iter().any(|d| d.to_string_lossy().contains("somelib")),
            "a source-only out dir must not be searched: {dirs:?}"
        );
    }

    /// The atomics sysroot is assembled from the SAME dep dirs, and its failure mode is
    /// what made the browser gate unreadable: an unassembled sysroot does not fail the
    /// link, it drops `--html --threads` to the default rustc, which then cannot link
    /// the nightly-built rlib and reports `E0514` — an error naming the wrong subject,
    /// two steps from its cause.  So "no readable dep dir" must stay LOUD.
    #[test]
    fn the_sysroot_assembles_from_every_dep_dir_and_fails_loudly_with_none() {
        let root = scratch("sysroot");
        let out = root.join("out");

        // Per-unit layout: two crates, two directories.
        let a = root.join("build/core/h1/out");
        let b = root.join("build/alloc/h2/out");
        touch(&a.join("libcore-h1.rlib"));
        touch(&b.join("liballoc-h2.rlib"));
        // loft's own rlib is deliberately excluded — a third copy on the sysroot search
        // path is how rustc ends up reporting multiple candidates for one crate.
        touch(&b.join("libloft.rlib"));

        super::assemble_atomics_sysroot(&[a, b], &out).expect("assembles from both dirs");
        let lib = out.join("lib/rustlib/wasm32-unknown-unknown/lib");
        assert!(crate::file_access::is_file(lib.join("libcore-h1.rlib")));
        assert!(crate::file_access::is_file(lib.join("liballoc-h2.rlib")));
        assert!(
            !crate::file_access::exists(lib.join("libloft.rlib")),
            "loft's own rlib must not reach the sysroot"
        );

        // Nothing readable: an error, never a silent Ok.
        let err =
            super::assemble_atomics_sysroot(&[root.join("does-not-exist")], &root.join("out2"))
                .expect_err("an unassembled sysroot must not report success");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
    }

    /// Neither layout present: name the classic path anyway, so the failure diagnostic
    /// tells the user where loft looked instead of printing nothing.
    #[test]
    fn an_empty_tree_still_names_the_expected_place() {
        let root = scratch("empty");
        assert_eq!(dep_search_dirs(&root), vec![root.join("deps")]);
    }
}

#[cfg(test)]
mod publish_cached_binary_tests {
    use super::{
        NATIVE_CACHE_KEEP, publish_cached_binary, sweep_cached_binaries, touch_cached_binary,
    };

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join("loft-publish-cache").join(name);
        let _ = crate::file_access::remove_dir_all(&d);
        crate::file_access::create_dir_all(&d).unwrap();
        d
    }

    /// The pre-fix publish, kept as the POSITIVE CONTROL.  Without it the test below
    /// asserts a property nothing was ever measured to violate, and a guard that cannot
    /// fail is not a guard.
    ///
    /// `cfg(unix)` with its one caller: the control exists only for the inode reading, so
    /// on Windows it would be a function nobody calls and `-D warnings` fails the build on
    /// `dead_code`.
    // approved exemption: a test of Unix-only semantics (symlinks, inodes, a read-only sysroot); Windows has none of them to test
    #[cfg(unix)]
    fn publish_by_copy(built: &std::path::Path, cached: &std::path::Path) {
        let _ = crate::file_access::copy(built, cached);
    }

    /// A publish REPLACES the destination rather than rewriting it in place.
    ///
    /// The inode is the deterministic reading of that: `fs::copy` opens the destination
    /// with truncate and streams into the SAME inode, so a concurrent reader that has
    /// already accepted the path watches its bytes vanish and then regrow.  A rename
    /// swaps in a different inode, and a process still exec'ing the old one keeps a
    /// complete file.  This is the property; the timing window is only its symptom.
    ///
    /// UNIX only, because the READING is: `MetadataExt::ino` does not exist on Windows,
    /// and an unguarded `use std::os::unix::fs::MetadataExt` at the module head broke the
    /// Windows build outright (`cannot find unix in os`, then four `no method named ino`)
    /// — a test's import, taking down `cargo test` for the whole target.  The property is
    /// not unix-only and Windows spells its identity `MetadataExt::file_index`; that arm is
    /// deliberately NOT written here, because it cannot be measured from this box and an
    /// assertion nobody has watched fail is the thing this test's own control exists to
    /// refuse.  The sweep test below stays on every platform.
    // approved exemption: a test of Unix-only semantics (symlinks, inodes, a read-only sysroot); Windows has none of them to test
    #[cfg(unix)]
    #[test]
    fn a_publish_swaps_the_inode_instead_of_truncating_in_place() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch("inode");
        let a = dir.join("built-a");
        let b = dir.join("built-b");
        crate::file_access::write(&a, vec![0xAAu8; 4096]).unwrap();
        crate::file_access::write(&b, vec![0xBBu8; 8192]).unwrap();
        let dest = dir.join("prog-deadbeef");

        publish_cached_binary(&a, &dest, &dir, "prog");
        let first = crate::file_access::metadata(&dest).unwrap().ino();
        publish_cached_binary(&b, &dest, &dir, "prog");
        let second = crate::file_access::metadata(&dest).unwrap().ino();

        assert_ne!(
            first, second,
            "a publish must swap a new inode in, never rewrite the live one"
        );
        assert_eq!(crate::file_access::read(&dest).unwrap(), vec![0xBBu8; 8192]);

        // The control: the copy this replaced keeps the inode, so the same assertion
        // fails on it — the probe discriminates.
        let dest2 = dir.join("ctl-deadbeef");
        publish_by_copy(&a, &dest2);
        let c1 = crate::file_access::metadata(&dest2).unwrap().ino();
        publish_by_copy(&b, &dest2);
        let c2 = crate::file_access::metadata(&dest2).unwrap().ino();
        assert_eq!(
            c1, c2,
            "control: a plain copy rewrites the live inode in place"
        );
    }

    /// The publish leaves no staging file behind, and the sweep spares the entry it
    /// just wrote — sweeping first deleted the binary a concurrent run had accepted.
    #[test]
    fn the_sweep_takes_other_hashes_and_spares_the_published_entry() {
        let dir = scratch("sweep");
        let built = dir.join("built");
        crate::file_access::write(&built, vec![0xCCu8; 2048]).unwrap();
        let stale = dir.join("prog-0000000000000000");
        crate::file_access::write(&stale, b"stale").unwrap();
        let other = dir.join("elsewhere-1111111111111111");
        crate::file_access::write(&other, b"other source").unwrap();
        let dest = dir.join("prog-ffffffffffffffff");

        publish_cached_binary(&built, &dest, &dir, "prog");

        assert!(
            crate::file_access::exists(&dest),
            "the entry just published must survive its own sweep"
        );
        assert_eq!(crate::file_access::read(&dest).unwrap(), vec![0xCCu8; 2048]);
        assert!(
            !crate::file_access::exists(&stale),
            "a stale hash for this source is swept"
        );
        assert!(
            crate::file_access::exists(&other),
            "another source's entry is not this sweep's business"
        );
        let staging: Vec<_> = crate::file_access::read_dir(&dir)
            .unwrap()
            .into_iter()
            .filter(|e| e.extension() == Some("tmp"))
            .map(|e| e.file_name().unwrap_or_default().to_string())
            .collect();
        assert!(
            staging.is_empty(),
            "no staging file may be left behind: {staging:?}"
        );
    }

    fn keyed(n: u8) -> String {
        format!("native-{}", format!("{n:x}").repeat(16))
    }

    /// The cache fixtures write and probe through `file_access`, as the code under test does.
    fn put(p: &std::path::Path, bytes: &[u8]) {
        crate::file_access::write(crate::file_access::PathText::from_os(p), bytes).unwrap();
    }

    fn there(p: &std::path::Path) -> bool {
        crate::file_access::exists(crate::file_access::PathText::from_os(p))
    }

    /// The directory keeps the most recently USED entries: a reuse (`touch_cached_binary`)
    /// outranks a later publish, the entry just published always stays, and a name that is
    /// not a key is not this sweep's business.
    #[test]
    fn the_bound_keeps_the_most_recently_used_entries() {
        let dir = scratch("lru");
        let base = std::time::SystemTime::now() - std::time::Duration::from_hours(1);
        for n in 1..=4u8 {
            let p = dir.join(keyed(n));
            put(&p, &[n]);
            crate::file_access::set_modified(
                crate::file_access::PathText::from_os(&p),
                base + std::time::Duration::from_secs(u64::from(n) * 60),
            )
            .unwrap();
        }
        // The oldest entry is reused, so it is now the most recent of the four.
        touch_cached_binary(&dir.join(keyed(1)));
        let foreign = dir.join("native-notakey");
        put(&foreign, b"x");
        let current = dir.join(keyed(5));
        put(&current, &[5]);

        sweep_cached_binaries(&dir, &current, "prog", 3);

        let left = |n: u8| there(&dir.join(keyed(n)));
        assert!(left(5), "the entry just published stays");
        assert!(left(1), "a reused entry outranks newer publishes");
        assert!(left(4), "the most recent publish stays");
        assert!(!left(2) && !left(3), "the least recently used go");
        assert!(there(&foreign), "a name that is not a key is left alone");
    }

    /// A program named `native.loft` must not read every other program's binary as its
    /// own earlier per-source entry.
    #[test]
    fn a_source_named_native_sweeps_nothing_of_others() {
        let dir = scratch("native_stem");
        for n in 1..=3u8 {
            put(&dir.join(keyed(n)), &[n]);
        }
        let current = dir.join(keyed(9));
        put(&current, &[9]);
        sweep_cached_binaries(&dir, &current, "native", NATIVE_CACHE_KEEP);
        for n in [1u8, 2, 3, 9] {
            assert!(there(&dir.join(keyed(n))), "entry {n} survives");
        }
    }
}
