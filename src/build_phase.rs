// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution: the declarative build/test
// phase (@PLN100) — the manifest-driven `loft build` / `loft check` tooling.

//! The `loft build` / `loft check` driver for @PLN100's declarative build phase:
//! resolve a project's declared / default build targets, run its `[[build.asset]]`
//! steps and declared `[[test]]` phase, and tie build + test into one gate.
//!
//! Built-in targets `native` / `html` / `wasi` are **implicit** — a zero-config
//! project (no `[build]` section) still builds.  A `[build.target.<name>]` entry
//! *overlays* a built-in of the same name (changing its `features` / `requires`)
//! or declares a **new** named target that maps to one of the built-in compile
//! shapes.  `[[build.asset]]` steps (Slice 3) turn `inputs` into `outputs`, re-run
//! only when stale; `[[test]]` entries (Slice 4) run over execution-backend
//! `targets`, gated on `needs`, with a cached green run.  See
//! `doc/claude/PACKAGES_BUILD.md § The build phase` and the @PLN100 plan.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use crate::manifest::{BuildAsset, BuildTest, Manifest};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The compile action a target maps to — named by the target's `shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `--native --check`: full native codegen + rustc compile, no run.
    Native,
    /// `--html`: a self-contained browser page (wasm32-unknown-unknown).
    Html,
    /// `--native-wasm`: a WASI `.wasm` (wasm32-wasip2).
    Wasi,
}

impl Shape {
    /// Parse a `shape = "..."` value (`native` | `html` | `wasi`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Shape> {
        match name {
            "native" => Some(Shape::Native),
            "html" => Some(Shape::Html),
            "wasi" => Some(Shape::Wasi),
            _ => None,
        }
    }

    /// The `loft` flags that drive this shape's compile.  `native` uses
    /// `--check` so it compiles + reports without RUNNING the program (a build,
    /// not an execution); `html` / `wasi` write their artifact file.
    #[must_use]
    pub fn compile_args(self) -> &'static [&'static str] {
        match self {
            Shape::Native => &["--native", "--check"],
            Shape::Html => &["--html"],
            Shape::Wasi => &["--native-wasm"],
        }
    }
}

/// A build target resolved from the built-in defaults overlaid with any matching
/// `[build.target.<name>]` manifest entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub name: String,
    pub shape: Shape,
    pub triple: Option<String>,
    pub features: Vec<String>,
    /// rustup targets that must be installed (a HARD requirement — a missing one
    /// means the target cannot build).
    pub requires_rust_targets: Vec<String>,
    /// external tools that should be on `PATH` (a SOFT requirement — a missing
    /// one is a warning; the compile may still succeed with degraded output).
    pub requires_tools: Vec<String>,
}

/// The built-in target for `name`, or `None` if `name` is not a built-in.
fn builtin(name: &str) -> Option<ResolvedTarget> {
    let make =
        |shape, triple: Option<&str>, rust_targets: &[&str], tools: &[&str]| ResolvedTarget {
            name: name.to_string(),
            shape,
            triple: triple.map(str::to_string),
            features: Vec::new(),
            requires_rust_targets: rust_targets.iter().map(|s| (*s).to_string()).collect(),
            requires_tools: tools.iter().map(|s| (*s).to_string()).collect(),
        };
    match name {
        "native" => Some(make(Shape::Native, None, &[], &[])),
        "html" => Some(make(
            Shape::Html,
            Some("wasm32-unknown-unknown"),
            &["wasm32-unknown-unknown"],
            // wasm-opt/binaryen is only needed for asyncify (frame-yielding
            // programs); a compute-only page builds without it, so it is a SOFT
            // requirement (warn, don't block).
            &["wasm-opt"],
        )),
        "wasi" => Some(make(
            Shape::Wasi,
            Some("wasm32-wasip2"),
            &["wasm32-wasip2"],
            &[],
        )),
        _ => None,
    }
}

/// Resolve `name` to a build target: the built-in (if any) overlaid with the
/// matching `[build.target.<name>]` entry (each field the manifest declares
/// replaces the built-in default).
///
/// # Errors
/// Returns a message when `name` is neither a built-in nor a declared target, or
/// when a declared target's `shape` is not one of `native` / `html` / `wasi`.
pub fn resolve_target(name: &str, manifest: &Manifest) -> Result<ResolvedTarget, String> {
    let decl = manifest.build_targets.iter().find(|t| t.name == name);
    let base = builtin(name);
    let Some(decl) = decl else {
        // No manifest entry — the built-in, or an error naming the built-ins.
        return base.ok_or_else(|| {
            format!(
                "unknown build target `{name}` — built-ins are native, html, wasi; \
                 declare a custom one under [build.target.{name}]"
            )
        });
    };
    // Determine the shape: an explicit `shape =`, else the built-in's, else the
    // name itself if it names a shape, else `native`.
    let shape = match decl.shape.as_deref() {
        Some(s) => Shape::from_name(s).ok_or_else(|| {
            format!("build target `{name}`: unknown shape `{s}` (use native | html | wasi)")
        })?,
        None => base
            .as_ref()
            .map(|b| b.shape)
            .or_else(|| Shape::from_name(name))
            .unwrap_or(Shape::Native),
    };
    let mut t = base.unwrap_or_else(|| ResolvedTarget {
        name: name.to_string(),
        shape,
        triple: None,
        features: Vec::new(),
        requires_rust_targets: Vec::new(),
        requires_tools: Vec::new(),
    });
    t.shape = shape;
    if decl.triple.is_some() {
        t.triple.clone_from(&decl.triple);
    }
    if !decl.features.is_empty() {
        t.features.clone_from(&decl.features);
    }
    if !decl.requires_rust_targets.is_empty() {
        t.requires_rust_targets
            .clone_from(&decl.requires_rust_targets);
    }
    if !decl.requires_tools.is_empty() {
        t.requires_tools.clone_from(&decl.requires_tools);
    }
    Ok(t)
}

/// The targets `loft build` makes with no explicit names: `default-targets` from
/// the manifest, or `["native"]` so a zero-config project still builds.
#[must_use]
pub fn default_targets(manifest: &Manifest) -> Vec<String> {
    if manifest.build_default_targets.is_empty() {
        vec!["native".to_string()]
    } else {
        manifest.build_default_targets.clone()
    }
}

/// The unmet requirements of a target: missing rustup targets (HARD) and missing
/// tools (SOFT).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Missing {
    pub rust_targets: Vec<String>,
    pub tools: Vec<String>,
}

/// Which of `t`'s requirements are unmet.  `installed_rust_targets` is `None`
/// when the rustup target list could not be queried — then the rustup-target
/// gate is skipped (can't verify → don't block; the compile reports the real
/// error).  `tool_present` answers whether a tool is on `PATH`.
#[must_use]
pub fn missing_requires(
    t: &ResolvedTarget,
    installed_rust_targets: Option<&[String]>,
    tool_present: &dyn Fn(&str) -> bool,
) -> Missing {
    let rust_targets = match installed_rust_targets {
        Some(installed) => t
            .requires_rust_targets
            .iter()
            .filter(|rt| !installed.iter().any(|i| i == *rt))
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    Missing {
        rust_targets,
        tools: t
            .requires_tools
            .iter()
            .filter(|tool| !tool_present(tool))
            .cloned()
            .collect(),
    }
}

/// The rustup targets installed on this host, or `None` if `rustup` could not be
/// run (so the caller skips the rustup-target gate rather than blocking).
#[must_use]
pub fn installed_rust_targets() -> Option<Vec<String>> {
    let out = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(
        text.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// Whether `tool` resolves to an executable on `PATH` (portable `which`).
#[must_use]
pub fn tool_on_path(tool: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|dir| {
        if dir.join(tool).is_file() {
            return true;
        }
        cfg!(windows)
            && (dir.join(format!("{tool}.exe")).is_file()
                || dir.join(format!("{tool}.cmd")).is_file())
    })
}

/// A short one-line description of a target for progress output.
fn describe(t: &ResolvedTarget) -> String {
    match &t.triple {
        Some(triple) => format!("{:?} · {triple}", t.shape),
        None => format!("{:?}", t.shape),
    }
}

/// Drive `loft build`: resolve `requested` (or the default targets when empty),
/// run the applicable `[[build.asset]]` steps (stale ones only, unless `force`),
/// doctor-check each target's `requires`, and compile it by re-invoking this loft
/// binary with the shape's flags on `entry` (from `project_dir`).  Returns `true`
/// only if every asset and target succeeded.
#[must_use]
pub fn run(
    requested: &[String],
    entry: &str,
    manifest: &Manifest,
    project_dir: &Path,
    force: bool,
) -> bool {
    let names: Vec<String> = if requested.is_empty() {
        default_targets(manifest)
    } else {
        requested.to_vec()
    };
    let loft_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loft"));
    let installed = installed_rust_targets();
    let mut all_ok = true;
    eprintln!("loft build: {} → target(s): {}", entry, names.join(", "));
    // @PLN100 Slice 3 — run the asset steps needed by the selected targets FIRST,
    // so an asset that feeds a target compile is ready before it builds.
    for asset in applicable_assets(manifest, &names) {
        if run_asset(asset, project_dir, force) == AssetOutcome::Failed {
            all_ok = false;
        }
    }
    for name in &names {
        let target = match resolve_target(name, manifest) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("loft build: {e}");
                all_ok = false;
                continue;
            }
        };
        let missing = missing_requires(&target, installed.as_deref(), &|t| tool_on_path(t));
        if !missing.rust_targets.is_empty() {
            eprintln!(
                "loft build: target `{name}` cannot build — missing rustup target(s): {}.",
                missing.rust_targets.join(", ")
            );
            for rt in &missing.rust_targets {
                eprintln!("             install:  rustup target add {rt}");
            }
            all_ok = false;
            continue;
        }
        if !missing.tools.is_empty() {
            eprintln!(
                "loft build: target `{name}` — optional tool(s) not on PATH: {} \
                 (building anyway; some output may be degraded).",
                missing.tools.join(", ")
            );
        }
        eprintln!("loft build: building `{name}` ({}) …", describe(&target));
        let status = Command::new(&loft_exe)
            .args(target.shape.compile_args())
            .arg(entry)
            .current_dir(project_dir)
            .status();
        match status {
            Ok(s) if s.success() => eprintln!("loft build: `{name}` ✓"),
            Ok(_) => {
                eprintln!("loft build: target `{name}` FAILED (see errors above).");
                all_ok = false;
            }
            Err(e) => {
                eprintln!("loft build: could not launch the compiler for `{name}`: {e}");
                all_ok = false;
            }
        }
    }
    all_ok
}

// ── @PLN100 Slice 3 — custom asset steps ────────────────────────────────────

/// What happened to an asset step in one `loft build`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetOutcome {
    /// Was stale → the `run` command executed and re-stamped.
    Built,
    /// Fingerprint + TTL agreed it was up to date → skipped.
    Skipped,
    /// The `run` command failed (or the asset was malformed).
    Failed,
}

/// The assets that apply to a build of `target_names`: an asset with no `targets`
/// is global (always applies); otherwise it applies when one of its `targets` is
/// being built.
#[must_use]
pub fn applicable_assets<'a>(
    manifest: &'a Manifest,
    target_names: &[String],
) -> Vec<&'a BuildAsset> {
    manifest
        .build_assets
        .iter()
        .filter(|a| a.targets.is_empty() || a.targets.iter().any(|t| target_names.contains(t)))
        .collect()
}

/// Why an asset needs (re)building — or that it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    /// Up to date; skip.
    Fresh,
    /// `--force` / `--fresh` requested a rebuild regardless.
    Forced,
    /// A declared output file is missing.
    OutputMissing,
    /// No prior build stamp (first build).
    FirstBuild,
    /// An input file's content changed.
    InputsChanged,
    /// A `lifetime` TTL elapsed since the last build.
    Expired,
}

impl Staleness {
    /// Whether this state requires the asset's `run` command to execute.
    #[must_use]
    pub fn needs_build(self) -> bool {
        self != Staleness::Fresh
    }

    /// A short human tag for progress output.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Staleness::Fresh => "fresh",
            Staleness::Forced => "forced",
            Staleness::OutputMissing => "output missing",
            Staleness::FirstBuild => "first build",
            Staleness::InputsChanged => "inputs changed",
            Staleness::Expired => "lifetime expired",
        }
    }
}

/// Decide whether an asset is stale, given its current inputs `fingerprint`, the
/// `stored` `(fingerprint, unix_secs)` from the last build (if any), whether its
/// outputs are present, its `lifetime_secs` (if a TTL is set), the wall-clock
/// `now`, and `force`.  Pure so it is unit-tested exhaustively.
#[must_use]
pub fn compute_staleness(
    fingerprint: u64,
    stored: Option<(u64, u64)>,
    outputs_present: bool,
    lifetime_secs: Option<u64>,
    now: u64,
    force: bool,
) -> Staleness {
    if force {
        return Staleness::Forced;
    }
    if !outputs_present {
        return Staleness::OutputMissing;
    }
    let Some((stored_fp, stored_time)) = stored else {
        return Staleness::FirstBuild;
    };
    if stored_fp != fingerprint {
        return Staleness::InputsChanged;
    }
    if let Some(ttl) = lifetime_secs
        && now.saturating_sub(stored_time) > ttl
    {
        return Staleness::Expired;
    }
    Staleness::Fresh
}

/// Parse a freshness-`lifetime` string into seconds: a positive integer followed
/// by a unit — `s` (sec), `m` (min), `h` (hour), `d` (day), `w` (week), `mo`
/// (month = 30d), `y` (year = 365d).  `"30d"` → `2_592_000`.  `None` if malformed.
#[must_use]
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    // Longest unit first so `mo` wins over `m`.
    const UNITS: &[(&str, u64)] = &[
        ("mo", 2_592_000),
        ("s", 1),
        ("m", 60),
        ("h", 3_600),
        ("d", 86_400),
        ("w", 604_800),
        ("y", 31_536_000),
    ];
    for (suffix, secs) in UNITS {
        if let Some(num) = s.strip_suffix(suffix) {
            let n: u64 = num.trim().parse().ok()?;
            return n.checked_mul(*secs);
        }
    }
    None
}

/// Wall-clock seconds since the Unix epoch (0 if the clock is before the epoch).
#[must_use]
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A content fingerprint of every file matching an asset's `input` globs
/// (relative to `project_dir`): folds each matched file's path + sha256 content
/// hash, sorted so it is order-independent.  An asset with no inputs fingerprints
/// to a constant (then only `lifetime` / a missing output drive rebuilds).
#[must_use]
pub fn inputs_fingerprint(project_dir: &Path, inputs: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut files: Vec<PathBuf> = inputs
        .iter()
        .flat_map(|g| glob_expand(project_dir, g))
        .collect();
    files.sort();
    files.dedup();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for f in &files {
        let rel = f.strip_prefix(project_dir).unwrap_or(f);
        rel.to_string_lossy().hash(&mut h);
        if let Some(content) = f.to_str().and_then(crate::cache::file_hash) {
            content.hash(&mut h);
        }
    }
    h.finish()
}

/// Expand one glob `pattern` (relative to `base`) to the matching files.  `*` /
/// `?` match within a path segment, `**` matches any number of directory
/// segments; a pattern with no wildcard is a literal path.
#[must_use]
pub fn glob_expand(base: &Path, pattern: &str) -> Vec<PathBuf> {
    let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    match_segments(base, &segments, &mut out);
    out.sort();
    out.dedup();
    out
}

fn match_segments(dir: &Path, segs: &[&str], out: &mut Vec<PathBuf>) {
    let Some((first, rest)) = segs.split_first() else {
        // No more segments: `dir` matches if it is a file.
        if dir.is_file() {
            out.push(dir.to_path_buf());
        }
        return;
    };
    if *first == "**" {
        // `**` matches zero segments (recurse with the rest here)…
        match_segments(dir, rest, out);
        // …or one+ segments: descend into each subdir keeping the `**`.
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    match_segments(&p, segs, out);
                }
            }
        }
        return;
    }
    if first.contains('*') || first.contains('?') {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let name = e.file_name();
                if segment_matches(&name.to_string_lossy(), first) {
                    match_segments(&e.path(), rest, out);
                }
            }
        }
        return;
    }
    let next = dir.join(first);
    if next.exists() {
        match_segments(&next, rest, out);
    }
}

/// Whether a file `name` matches a single glob segment `pattern` (`*` = any run,
/// `?` = one char; no `/`).
#[must_use]
pub fn segment_matches(name: &str, pattern: &str) -> bool {
    fn go(n: &[u8], p: &[u8]) -> bool {
        match (n.first(), p.first()) {
            (_, Some(b'*')) => go(n, &p[1..]) || (!n.is_empty() && go(&n[1..], p)),
            (Some(_), Some(b'?')) => go(&n[1..], &p[1..]),
            (Some(nc), Some(pc)) if nc == pc => go(&n[1..], &p[1..]),
            (None, None) => true,
            _ => false,
        }
    }
    go(name.as_bytes(), pattern.as_bytes())
}

/// The fingerprint-sidecar path for an asset named `name` under `project_dir`:
/// `.loft/build/<sanitized-name>.stamp`, holding the inputs fingerprint and the
/// last-build wall-clock time.
fn asset_stamp_path(project_dir: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    project_dir
        .join(".loft")
        .join("build")
        .join(format!("{safe}.stamp"))
}

/// Read an asset's stored `(fingerprint, unix_secs)`, if a stamp exists.
fn read_stamp(project_dir: &Path, name: &str) -> Option<(u64, u64)> {
    let text = std::fs::read_to_string(asset_stamp_path(project_dir, name)).ok()?;
    let mut lines = text.lines();
    let fp = lines.next()?.trim().parse().ok()?;
    let time = lines.next()?.trim().parse().ok()?;
    Some((fp, time))
}

/// Stamp an asset's `fingerprint` + build `time` (best-effort).
fn write_stamp(project_dir: &Path, name: &str, fingerprint: u64, time: u64) {
    let path = asset_stamp_path(project_dir, name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{fingerprint}\n{time}\n"));
}

/// Whether all of an asset's declared `outputs` exist under `project_dir`.  An
/// asset with no declared outputs is treated as present (staleness then rests on
/// the fingerprint + TTL alone).
fn outputs_present(project_dir: &Path, asset: &BuildAsset) -> bool {
    asset.outputs.iter().all(|o| project_dir.join(o).exists())
}

/// Run one asset step if it is stale: compute its inputs fingerprint, compare to
/// the stamp + TTL, and — when stale — execute its `run` command and re-stamp.
#[must_use]
pub fn run_asset(asset: &BuildAsset, project_dir: &Path, force: bool) -> AssetOutcome {
    let name = asset.name.as_deref().unwrap_or("<unnamed>");
    let Some(run_cmd) = asset.run.as_deref() else {
        eprintln!("loft build: asset `{name}` has no `run` command — skipping.");
        return AssetOutcome::Skipped;
    };
    let fingerprint = inputs_fingerprint(project_dir, &asset.inputs);
    let lifetime = asset.lifetime.as_deref().and_then(parse_duration);
    if asset.lifetime.is_some() && lifetime.is_none() {
        eprintln!(
            "loft build: asset `{name}` — bad lifetime `{}` (use e.g. 30d, 4w, 1mo); ignoring TTL.",
            asset.lifetime.as_deref().unwrap_or("")
        );
    }
    let stored = read_stamp(project_dir, name);
    let state = compute_staleness(
        fingerprint,
        stored,
        outputs_present(project_dir, asset),
        lifetime,
        unix_now(),
        force,
    );
    if !state.needs_build() {
        eprintln!("loft build: asset `{name}` up to date — skipping.");
        return AssetOutcome::Skipped;
    }
    eprintln!(
        "loft build: asset `{name}` ({}) — running `{run_cmd}` …",
        state.reason()
    );
    match run_asset_command(run_cmd, project_dir) {
        Ok(s) if s.success() => {
            // Re-fingerprint (an input-generating step may have changed inputs) and
            // stamp the build time for the TTL.
            let fresh = inputs_fingerprint(project_dir, &asset.inputs);
            write_stamp(project_dir, name, fresh, unix_now());
            // A step that named its `outputs` promised them.  Exiting zero without
            // producing them is a failed step, not a passed one — reporting the miss
            // and then `✓` in the same breath says both, and whoever reads the log
            // takes the `✓`.  The cost of getting this wrong is paid downstream, where
            // the reason is no longer in view: a `[[embed]]` naming that output, or a
            // target compile, fails one step later against a file nobody built.
            if !outputs_present(project_dir, asset) {
                eprintln!(
                    "loft build: asset `{name}` FAILED — it exited zero but did not \
                     produce: {}.\n  \
                     A `run` script resolves its paths against the SCRIPT, not the \
                     project root, so `scripts/pack.loft` writing `assets/x` writes \
                     `scripts/assets/x`.  Set LOFT_PATHS=cwd, or write the path the \
                     step declares.",
                    asset.outputs.join(", ")
                );
                return AssetOutcome::Failed;
            }
            eprintln!("loft build: asset `{name}` ✓");
            AssetOutcome::Built
        }
        Ok(_) => {
            eprintln!("loft build: asset `{name}` FAILED (its `run` command exited non-zero).");
            AssetOutcome::Failed
        }
        Err(e) => {
            eprintln!("loft build: asset `{name}` — could not run `{run_cmd}`: {e}.");
            AssetOutcome::Failed
        }
    }
}

/// Execute an asset's `run`: a single `.loft` script runs with this loft binary;
/// anything else runs through the platform shell (trusted-by-declaration — it is
/// the project's own manifest, @PLN100 open question 3 / @PLN86).
fn run_asset_command(cmd: &str, project_dir: &Path) -> std::io::Result<std::process::ExitStatus> {
    let is_loft_script = !cmd.contains(char::is_whitespace)
        && Path::new(cmd)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("loft"));
    if is_loft_script {
        let loft_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loft"));
        Command::new(loft_exe)
            .arg(cmd)
            .current_dir(project_dir)
            .status()
    } else if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", cmd])
            .current_dir(project_dir)
            .status()
    } else {
        Command::new("sh")
            .args(["-c", cmd])
            .current_dir(project_dir)
            .status()
    }
}

// ── @PLN100 Slice 4 — declared test phase ───────────────────────────────────

/// The execution backend a `[[test]]` target names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestBackend {
    /// The interpreter: `loft <run>`.
    Interpret,
    /// The native backend: `loft --native <run>`.
    Native,
    /// A target with no headless test runner yet (`html` / `wasi` / unknown) —
    /// reported and skipped, never silently passed.
    Unsupported(String),
}

/// Map a `[[test]]` target name to its execution backend.  `interpret` (or an
/// empty default) and `native` are runnable; everything else has no headless
/// runner yet.  Mirrors loft's own interpret+native harness (@PLN100 question 6).
#[must_use]
pub fn test_backend(target: &str) -> TestBackend {
    match target {
        "interpret" | "interpreter" | "" => TestBackend::Interpret,
        "native" => TestBackend::Native,
        other => TestBackend::Unsupported(other.to_string()),
    }
}

/// What happened to a `[[test]]` (aggregated over its target backends).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOutcome {
    /// Ran green (or a cached green) on every runnable backend.
    Passed,
    /// No runnable backend, or no `run` — reported, not counted as a failure.
    Skipped,
    /// A run failed, or a needed asset was not built.
    Failed,
}

/// The asset outputs a test `needs` that are not present, or `None` if all met.
fn unmet_needs(test: &BuildTest, manifest: &Manifest, project_dir: &Path) -> Option<String> {
    let mut missing = Vec::new();
    for need in &test.needs {
        match manifest
            .build_assets
            .iter()
            .find(|a| a.name.as_deref() == Some(need.as_str()))
        {
            Some(asset) if !outputs_present(project_dir, asset) => {
                missing.extend(asset.outputs.iter().cloned());
            }
            Some(_) => {}
            None => missing.push(format!("{need} (unknown asset)")),
        }
    }
    if missing.is_empty() {
        None
    } else {
        Some(missing.join(", "))
    }
}

/// The cache fingerprint for a test on one backend: the run-script content, the
/// content of its `inputs` data files, and the target name.  A green run stores
/// it; a match on the next `loft check` skips the run (a cached pass).
#[must_use]
pub fn test_fingerprint(project_dir: &Path, run: &str, inputs: &[String], target: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    target.hash(&mut h);
    if let Some(script) = project_dir
        .join(run)
        .to_str()
        .and_then(crate::cache::file_hash)
    {
        script.hash(&mut h);
    }
    inputs_fingerprint(project_dir, inputs).hash(&mut h);
    h.finish()
}

/// The green-run cache stamp path for a test on a backend:
/// `.loft/test/<name>__<target>.stamp`.
fn test_stamp_path(project_dir: &Path, name: &str, target: &str) -> PathBuf {
    let safe = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    project_dir
        .join(".loft")
        .join("test")
        .join(format!("{}__{}.stamp", safe(name), safe(target)))
}

/// Whether a test's last green run on `target` matches `fp` (a cached pass).
fn test_cached_green(project_dir: &Path, name: &str, target: &str, fp: u64) -> bool {
    std::fs::read_to_string(test_stamp_path(project_dir, name, target))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .is_some_and(|stored| stored == fp)
}

/// Record a test's green run on `target` (best-effort).
fn write_test_stamp(project_dir: &Path, name: &str, target: &str, fp: u64) {
    let path = test_stamp_path(project_dir, name, target);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, fp.to_string());
}

/// Run one declared test over one backend, honouring the green-run cache.
fn run_test_on(
    name: &str,
    run: &str,
    target: &str,
    backend: &TestBackend,
    inputs: &[String],
    project_dir: &Path,
    force: bool,
) -> TestOutcome {
    let fp = test_fingerprint(project_dir, run, inputs, target);
    if !force && test_cached_green(project_dir, name, target, fp) {
        eprintln!("loft check: test `{name}` [{target}] cached (green) — skipping.");
        return TestOutcome::Passed;
    }
    eprintln!("loft check: test `{name}` [{target}] — running `{run}` …");
    let loft_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loft"));
    let mut cmd = Command::new(&loft_exe);
    if *backend == TestBackend::Native {
        cmd.arg("--native");
    }
    match cmd.arg(run).current_dir(project_dir).status() {
        Ok(s) if s.success() => {
            write_test_stamp(project_dir, name, target, fp);
            eprintln!("loft check: test `{name}` [{target}] ✓");
            TestOutcome::Passed
        }
        Ok(_) => {
            eprintln!("loft check: test `{name}` [{target}] FAILED (see output above).");
            TestOutcome::Failed
        }
        Err(e) => {
            eprintln!("loft check: test `{name}` [{target}] — could not run `{run}`: {e}.");
            TestOutcome::Failed
        }
    }
}

/// Run one declared `[[test]]` over each of its target backends, gated on `needs`.
#[must_use]
pub fn run_test(
    test: &BuildTest,
    manifest: &Manifest,
    project_dir: &Path,
    force: bool,
) -> TestOutcome {
    let name = test.name.as_deref().unwrap_or("<unnamed>");
    let Some(run) = test.run.as_deref() else {
        eprintln!("loft check: test `{name}` has no `run` script — skipping.");
        return TestOutcome::Skipped;
    };
    if let Some(missing) = unmet_needs(test, manifest, project_dir) {
        eprintln!(
            "loft check: test `{name}` blocked — needed asset output(s) not built: {missing}."
        );
        return TestOutcome::Failed;
    }
    let targets = if test.targets.is_empty() {
        vec!["interpret".to_string()]
    } else {
        test.targets.clone()
    };
    let mut ran_any = false;
    let mut ok = true;
    for target in &targets {
        match test_backend(target) {
            TestBackend::Unsupported(t) => {
                eprintln!(
                    "loft check: test `{name}` [{t}] — no headless runner for target `{t}` yet; \
                     skipped (use interpret / native)."
                );
            }
            backend => {
                ran_any = true;
                if run_test_on(
                    name,
                    run,
                    target,
                    &backend,
                    &test.inputs,
                    project_dir,
                    force,
                ) == TestOutcome::Failed
                {
                    ok = false;
                }
            }
        }
    }
    if !ran_any {
        TestOutcome::Skipped
    } else if ok {
        TestOutcome::Passed
    } else {
        TestOutcome::Failed
    }
}

/// Run the whole declared `[[test]]` phase.  Returns `true` if no test failed.
#[must_use]
pub fn run_test_phase(manifest: &Manifest, project_dir: &Path, force: bool) -> bool {
    if manifest.build_tests.is_empty() {
        return true;
    }
    eprintln!(
        "loft check: running {} declared test(s) …",
        manifest.build_tests.len()
    );
    let (mut pass, mut fail, mut skip) = (0u32, 0u32, 0u32);
    for test in &manifest.build_tests {
        match run_test(test, manifest, project_dir, force) {
            TestOutcome::Passed => pass += 1,
            TestOutcome::Skipped => skip += 1,
            TestOutcome::Failed => fail += 1,
        }
    }
    eprintln!("loft check: tests — {pass} passed, {fail} failed, {skip} skipped.");
    fail == 0
}

/// The `loft check` gate: build the targets + assets, then run the declared test
/// phase.  Returns `true` only if both succeed.  Both run even if the first
/// fails, so one invocation surfaces every problem.
#[must_use]
pub fn check(
    requested: &[String],
    entry: &str,
    manifest: &Manifest,
    project_dir: &Path,
    force: bool,
) -> bool {
    let build_ok = run(requested, entry, manifest, project_dir, force);
    let test_ok = run_test_phase(manifest, project_dir, force);
    build_ok && test_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{BuildTarget, Manifest};

    #[test]
    fn builtins_resolve_with_known_shapes_and_requires() {
        let m = Manifest::default();
        let html = resolve_target("html", &m).unwrap();
        assert_eq!(html.shape, Shape::Html);
        assert_eq!(html.triple.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(html.requires_rust_targets, vec!["wasm32-unknown-unknown"]);
        assert_eq!(html.requires_tools, vec!["wasm-opt"]);

        let wasi = resolve_target("wasi", &m).unwrap();
        assert_eq!(wasi.shape, Shape::Wasi);
        assert_eq!(wasi.requires_rust_targets, vec!["wasm32-wasip2"]);

        let native = resolve_target("native", &m).unwrap();
        assert_eq!(native.shape, Shape::Native);
        assert!(
            native.requires_rust_targets.is_empty(),
            "{:?}",
            native.requires_rust_targets
        );
    }

    #[test]
    fn unknown_target_is_an_error() {
        let m = Manifest::default();
        assert!(resolve_target("nope", &m).is_err());
    }

    #[test]
    fn manifest_entry_overlays_the_builtin() {
        let mut m = Manifest::default();
        m.build_targets.push(BuildTarget {
            name: "html".to_string(),
            shape: None,
            triple: None,
            features: vec!["random".to_string(), "png".to_string()],
            requires_rust_targets: Vec::new(),
            requires_tools: vec!["wasm-opt".to_string(), "wasm-strip".to_string()],
        });
        let html = resolve_target("html", &m).unwrap();
        // shape/triple keep the built-in; features + tools are replaced.
        assert_eq!(html.shape, Shape::Html);
        assert_eq!(html.triple.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(html.features, vec!["random", "png"]);
        assert_eq!(html.requires_tools, vec!["wasm-opt", "wasm-strip"]);
    }

    #[test]
    fn custom_target_needs_a_shape() {
        let mut m = Manifest::default();
        m.build_targets.push(BuildTarget {
            name: "mobile".to_string(),
            shape: Some("html".to_string()),
            triple: Some("wasm32-unknown-unknown".to_string()),
            ..Default::default()
        });
        let t = resolve_target("mobile", &m).unwrap();
        assert_eq!(t.shape, Shape::Html);

        m.build_targets.push(BuildTarget {
            name: "weird".to_string(),
            shape: Some("banana".to_string()),
            ..Default::default()
        });
        assert!(resolve_target("weird", &m).is_err());
    }

    #[test]
    fn default_targets_falls_back_to_native() {
        let m = Manifest::default();
        assert_eq!(default_targets(&m), vec!["native"]);
        let m2 = Manifest {
            build_default_targets: vec!["native".to_string(), "html".to_string()],
            ..Default::default()
        };
        assert_eq!(default_targets(&m2), vec!["native", "html"]);
    }

    #[test]
    fn missing_requires_splits_hard_and_soft() {
        let html = resolve_target("html", &Manifest::default()).unwrap();
        // rustup target absent, wasm-opt absent → both reported.
        let installed: Vec<String> = vec![];
        let miss = missing_requires(&html, Some(&installed), &|_| false);
        assert_eq!(miss.rust_targets, vec!["wasm32-unknown-unknown"]);
        assert_eq!(miss.tools, vec!["wasm-opt"]);

        // rustup target present, wasm-opt present → nothing missing.
        let installed = vec!["wasm32-unknown-unknown".to_string()];
        let miss = missing_requires(&html, Some(&installed), &|_| true);
        assert!(miss.rust_targets.is_empty() && miss.tools.is_empty());

        // Can't query rustup → rustup-target gate skipped.
        let miss = missing_requires(&html, None, &|_| true);
        assert!(miss.rust_targets.is_empty(), "{:?}", miss.rust_targets);
    }

    // ── Slice 3 — asset steps ──────────────────────────────────────────────

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration("30d"), Some(30 * 86_400));
        assert_eq!(parse_duration("4w"), Some(4 * 604_800));
        assert_eq!(parse_duration("1mo"), Some(2_592_000)); // `mo` beats `m`
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("2h"), Some(7_200));
        assert_eq!(parse_duration("5m"), Some(300)); // minutes, not months
        assert_eq!(parse_duration("1y"), Some(31_536_000));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("30"), None); // no unit
        assert_eq!(parse_duration("xd"), None); // no number
    }

    #[test]
    fn staleness_precedence() {
        // force wins over everything.
        assert_eq!(
            compute_staleness(1, Some((1, 100)), true, None, 200, true),
            Staleness::Forced
        );
        // missing output beats a matching fingerprint.
        assert_eq!(
            compute_staleness(1, Some((1, 100)), false, None, 200, false),
            Staleness::OutputMissing
        );
        // no stamp → first build.
        assert_eq!(
            compute_staleness(1, None, true, None, 200, false),
            Staleness::FirstBuild
        );
        // changed inputs.
        assert_eq!(
            compute_staleness(2, Some((1, 100)), true, None, 200, false),
            Staleness::InputsChanged
        );
        // TTL expired (age 100 > 50).
        assert_eq!(
            compute_staleness(1, Some((1, 100)), true, Some(50), 201, false),
            Staleness::Expired
        );
        // within TTL → fresh (age 50 <= 50).
        assert_eq!(
            compute_staleness(1, Some((1, 100)), true, Some(50), 150, false),
            Staleness::Fresh
        );
        // no TTL, matching fp, output present → fresh.
        assert_eq!(
            compute_staleness(1, Some((1, 100)), true, None, 999_999, false),
            Staleness::Fresh
        );
    }

    #[test]
    fn segment_matching() {
        assert!(segment_matches("atlas.png", "*.png"));
        assert!(segment_matches("atlas.png", "atlas.*"));
        assert!(segment_matches("a.png", "?.png"));
        assert!(segment_matches("x", "*"));
        assert!(!segment_matches("atlas.jpg", "*.png"));
        assert!(!segment_matches("ab.png", "?.png"));
    }

    #[test]
    fn glob_expand_and_fingerprint_react_to_change() {
        let dir = std::env::temp_dir().join(format!("loft_glob_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("art/sub")).unwrap();
        std::fs::write(dir.join("art/a.png"), b"aaa").unwrap();
        std::fs::write(dir.join("art/sub/b.png"), b"bbb").unwrap();
        std::fs::write(dir.join("art/notes.txt"), b"txt").unwrap();

        // `**/*.png` finds both PNGs at any depth, not the .txt.
        let hits = glob_expand(&dir, "art/**/*.png");
        assert_eq!(hits.len(), 2, "expected 2 pngs, got {hits:?}");

        // `art/*.png` is single-level only.
        assert_eq!(glob_expand(&dir, "art/*.png").len(), 1);

        // Fingerprint changes when an input's content changes…
        let fp1 = inputs_fingerprint(&dir, &["art/**/*.png".to_string()]);
        std::fs::write(dir.join("art/a.png"), b"CHANGED").unwrap();
        let fp2 = inputs_fingerprint(&dir, &["art/**/*.png".to_string()]);
        assert_ne!(fp1, fp2);
        // …and when a new matching file appears.
        std::fs::write(dir.join("art/c.png"), b"ccc").unwrap();
        let fp3 = inputs_fingerprint(&dir, &["art/**/*.png".to_string()]);
        assert_ne!(fp2, fp3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn applicable_assets_respects_target_scope() {
        let mut m = Manifest::default();
        m.build_assets.push(BuildAsset {
            name: Some("global".to_string()),
            ..Default::default()
        });
        m.build_assets.push(BuildAsset {
            name: Some("html-only".to_string()),
            targets: vec!["html".to_string()],
            ..Default::default()
        });
        // Building `native` only → just the global asset.
        let got = applicable_assets(&m, &["native".to_string()]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name.as_deref(), Some("global"));
        // Building `html` → both.
        assert_eq!(applicable_assets(&m, &["html".to_string()]).len(), 2);
    }

    // ── Slice 4 — test phase ───────────────────────────────────────────────

    #[test]
    fn test_backend_mapping() {
        assert_eq!(test_backend("interpret"), TestBackend::Interpret);
        assert_eq!(test_backend(""), TestBackend::Interpret);
        assert_eq!(test_backend("native"), TestBackend::Native);
        assert_eq!(
            test_backend("html"),
            TestBackend::Unsupported("html".to_string())
        );
    }

    #[test]
    fn test_fingerprint_reacts_to_script_input_and_target() {
        let dir = std::env::temp_dir().join(format!("loft_testfp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("tests/smoke.loft"), b"fn main() {}").unwrap();
        std::fs::write(dir.join("assets/data.bin"), b"v1").unwrap();
        let inputs = vec!["assets/data.bin".to_string()];

        let base = test_fingerprint(&dir, "tests/smoke.loft", &inputs, "interpret");
        // A different backend → different key (a green native run must not vouch
        // for interpret, and vice versa).
        assert_ne!(
            base,
            test_fingerprint(&dir, "tests/smoke.loft", &inputs, "native")
        );
        // A changed input data file → different key.
        std::fs::write(dir.join("assets/data.bin"), b"v2").unwrap();
        assert_ne!(
            base,
            test_fingerprint(&dir, "tests/smoke.loft", &inputs, "interpret")
        );
        // A changed run script → different key.
        std::fs::write(dir.join("assets/data.bin"), b"v1").unwrap();
        std::fs::write(dir.join("tests/smoke.loft"), b"fn main() { print(1) }").unwrap();
        assert_ne!(
            base,
            test_fingerprint(&dir, "tests/smoke.loft", &inputs, "interpret")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A step that exits zero without producing what it declared is a FAILED step.
    ///
    /// The shape is easy to write by accident: a `run` script resolves its paths
    /// against the SCRIPT, so `scripts/pack.loft` writing `assets/x` writes
    /// `scripts/assets/x` and the project's `assets/x` never appears. Reported as
    /// `✓`, the reason is gone by the time a `[[embed]]` or a target compile trips
    /// over the missing file.
    #[test]
    fn an_asset_that_does_not_produce_its_declared_output_fails() {
        let dir = std::env::temp_dir().join(format!("loft_asset_out_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let asset = |run: &str| BuildAsset {
            name: Some("pack".to_string()),
            run: Some(run.to_string()),
            outputs: vec!["assets/game.pack".to_string()],
            ..Default::default()
        };
        // Exits zero, writes nothing.
        assert_eq!(
            run_asset(&asset("true"), &dir, true),
            AssetOutcome::Failed,
            "a step that produced none of its declared outputs reported success"
        );
        // The control, and it is one variable: the same declaration, by a command
        // that DOES write the file. Without it the assertion above would also pass
        // for a `run_asset` that could never answer `Built` at all.
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        assert_eq!(
            run_asset(&asset("touch assets/game.pack"), &dir, true),
            AssetOutcome::Built
        );
        // And a step declaring NO outputs is unchanged — staleness rests on the
        // fingerprint alone there, so there is nothing to be missing.
        let bare = BuildAsset {
            name: Some("bare".to_string()),
            run: Some("true".to_string()),
            ..Default::default()
        };
        assert_eq!(run_asset(&bare, &dir, true), AssetOutcome::Built);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unmet_needs_detects_missing_asset_output() {
        let dir = std::env::temp_dir().join(format!("loft_needs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut m = Manifest::default();
        m.build_assets.push(BuildAsset {
            name: Some("atlas".to_string()),
            outputs: vec!["assets/atlas.bin".to_string()],
            ..Default::default()
        });
        let test = crate::manifest::BuildTest {
            needs: vec!["atlas".to_string()],
            ..Default::default()
        };
        // Output absent → unmet.
        assert!(unmet_needs(&test, &m, &dir).is_some());
        // Create the output → met.
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/atlas.bin"), b"x").unwrap();
        assert!(unmet_needs(&test, &m, &dir).is_none());
        // A need on an unknown asset is unmet.
        let bad = crate::manifest::BuildTest {
            needs: vec!["ghost".to_string()],
            ..Default::default()
        };
        assert!(unmet_needs(&bad, &m, &dir).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
