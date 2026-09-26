// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F50 — introspection (bytecode / native Rust / slot tables)

//! Plan-08 phase 01: introspection CLI.
//!
//! Wraps the existing dump primitives (`State::dump_bytecode`,
//! `Output::output_native`, `variables::validate::dump_variables`) in
//! a single `loft --introspect <file>` flow that emits the program's
//! bytecode, generated Rust, and per-function variable slot tables to
//! stdout (or per-section files).
//!
//! See `doc/claude/plans/08-repl-and-introspection/01-introspection-cli.md`
//! for the surface design.

use crate::compile;
use crate::data::{Data, DefType, Value};
use crate::diagnostics::Level;
use crate::generation;
use crate::log_config::LogConfig;
use crate::parser::Parser;
use crate::scopes;
use crate::state::State;
use crate::variables;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

/// Section selector — mirrors the four things the introspection
/// tool can emit, in canonical order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Bytecode,
    Rust,
    Slots,
    /// Per-function variable types with dependency tracking — useful
    /// for diagnosing lifetime / dep-propagation bugs (e.g. P197
    /// where `s: text` should have read `s: text[a]`).
    Types,
    /// Round-trip check: dump each function's bytecode, re-assemble it
    /// from that text, and compare to the original byte stream.  Proves
    /// the labelled disassembly is a faithful, editable representation.
    Roundtrip,
    /// @PLN103 — per-binding store-ownership: each variable's resolved
    /// `Owned` / `Borrowed(base)` / `Join(base)` verdict (via
    /// `use_analysis::ownership_of`), with owned backing buffers rendered
    /// as `Owned (backing=…)`.  Surfaces the borrowed-vs-owned fact behind
    /// the store-lifetime bug corpus.  Opt-in (`--show-ownership`).
    Ownership,
    /// Per-source name VISIBILITY: which sources exist, how many names each defines
    /// versus can reach, and every import alias.  The state that decides whether an
    /// unqualified name resolves — previously inspectable only by adding an
    /// `eprintln!` to the parser (@PLN120 E.4).  Opt-in (`--show-resolution`).
    Resolution,
}

/// Options for `loft --introspect`.
#[allow(dead_code)] // lib_dirs / install_dir reserved for the standalone `run()` entry,
// unused by main.rs's emit_all path.
pub struct Options {
    /// Sections the user asked for.  Empty = all three.
    pub sections: Vec<Section>,
    /// Per-section output paths.  When unset, the section writes to
    /// the shared stdout sink with a `=== bytecode ===`-style header.
    pub bytecode_out: Option<String>,
    pub rust_out: Option<String>,
    pub slots_out: Option<String>,
    pub types_out: Option<String>,
    /// When set, capture all (non-redirected) section output to a
    /// buffer and `diff -u` it against this baseline file.  Useful
    /// for "did my parser tweak change anything?" — capture once
    /// before the change, then run with `--diff <baseline>` after.
    /// Exits 0 if identical, 1 if diff.  Requires `diff` on PATH.
    pub diff_against: Option<String>,
    /// `--show-types --trace` companion: per-expression type tape
    /// recorded by the parser.  Emitted after each variable table.
    /// Lines are tab-separated `<fn>\t<line>:<col>\t<type>`.
    pub trace_lines: Vec<String>,
    /// `--show-resolution` companion: the resolution CONTEXT the invocation built
    /// (stdlib dir + `--lib` paths), rendered by the caller, which is the only place
    /// that knows it.  An empty `lib_dirs` under a `--lib` invocation IS the @PLN120
    /// E.1 defect, visible without running the program.
    pub resolution_context: Option<String>,
    /// `--why <name>`: instead of the whole table, answer where one name is defined
    /// and from which sources it is reachable.
    pub why: Option<String>,
    /// Emit a single machine-readable JSON object instead of the text
    /// sections — one string field per included section (`bytecode`,
    /// `rust`, `slots`, `types`, …), so a tool / the LSP reads a section
    /// by key rather than splitting on `=== header ===` lines.  Takes
    /// precedence over the per-section `*_out` files and `--diff`.
    pub json: bool,
    /// Restrict every section to functions whose name matches one of
    /// these strings (substring match, like `LOFT_LOG=fn:<name>`).
    /// Empty = include all user functions (filtered further by
    /// `all_fns`).
    pub fn_filter: Vec<String>,
    /// Include `default/` stdlib functions in every section.
    pub all_fns: bool,
    /// Library directories to pass to the parser (mirrors the main
    /// program's `--lib` / `--path` semantics).
    pub lib_dirs: Vec<String>,
    /// Path prefix to the loft installation (`--path` flag).  Empty
    /// means "binary directory" — same default as `loft <file>`.
    pub install_dir: String,
}

impl Options {
    /// Default introspect options — all three sections, no filter,
    /// stdout sink for everything.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
            bytecode_out: None,
            rust_out: None,
            slots_out: None,
            types_out: None,
            diff_against: None,
            trace_lines: Vec::new(),
            resolution_context: None,
            why: None,
            json: false,
            fn_filter: Vec::new(),
            all_fns: false,
            lib_dirs: Vec::new(),
            install_dir: String::new(),
        }
    }

    fn includes(&self, s: Section) -> bool {
        self.sections.is_empty() || self.sections.contains(&s)
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the introspection tool against `filename`.
///
/// Standalone entry: parses the file from scratch, runs scopes +
/// codegen, then dispatches to `emit_all`.  `main.rs` skips this
/// and calls `emit_all` directly because it has already parsed
/// the file by the time the `--introspect` branch fires.  Kept
/// public + `#[allow(dead_code)]` because `tests/` may use it.
///
/// # Errors
/// Returns I/O errors from sink writes; propagates parser fatal
/// diagnostics by exiting with status 1 (mirrors `loft <file>`'s
/// behaviour).
///
/// # Panics
/// Panics if the default stdlib directory cannot be parsed (mirrors
/// `loft <file>`'s `unwrap()` on `parse_dir(default/)`).
#[allow(dead_code)]
pub fn run(filename: &str, opts: &Options) -> std::io::Result<()> {
    let abs_file = crate::portable_path::plain_canonical_str(filename);
    let mut p = Parser::new();
    p.lib_dirs.clone_from(&opts.lib_dirs);
    let default_dir = if opts.install_dir.is_empty() {
        "default".to_string()
    } else {
        format!("{}default", opts.install_dir)
    };
    p.parse_dir(&default_dir, true, false)
        .expect("parse default/ stdlib");
    let start_def = p.data.definitions();
    p.parse(&abs_file, false);
    if !p.diagnostics.is_empty() {
        for entry in p.diagnostics.entries() {
            if entry.level == Level::Debug {
                continue;
            }
            eprintln!("{}", entry.to_string_compact());
        }
        if p.diagnostics.level() >= Level::Error {
            std::process::exit(1);
        }
    }
    scopes::check(&mut p.data, &mut p.database);
    let mut state = State::new(p.database);
    compile::byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let _ = start_def;
    emit_all(&mut p.data, &mut state, end_def, opts)
}

/// Emit the requested sections from an already-parsed program.
/// Used by the `loft --introspect` CLI dispatch in `main.rs` to
/// avoid re-parsing — the main flow has already run the parser +
/// scopes::check + compile::byte_code by the time the introspect
/// branch is reached.
///
/// When `opts.diff_against` is set, sections without an explicit
/// `*_out` file go into an in-memory buffer that is then `diff -u`'d
/// against the baseline.  Sections with an explicit `*_out` still
/// write to their files (the diff only covers stdout-bound output).
///
/// # Errors
/// Propagates I/O errors from any section's writer.  Also returns an
/// error if `--diff` is requested but `diff` is missing from PATH.
pub fn emit_all(
    data: &mut Data,
    state: &mut State,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    // JSON mode is a distinct, machine-readable rendering — one object over the
    // same section outputs — so it short-circuits the text/redirect/diff paths.
    if opts.json {
        return emit_json(data, state, end_def, opts);
    }
    let stdout = std::io::stdout();
    // When diffing, accumulate all stdout-bound output into one
    // buffer so we can `diff -u baseline buffer` afterwards.
    // Per-section `*_out` paths are honoured independently.
    let diff_mode = opts.diff_against.is_some();
    let mut buffer: Vec<u8> = Vec::new();
    if opts.includes(Section::Bytecode) {
        if let Some(path) = opts.bytecode_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_bytecode(&mut writer, state, data, opts)?;
        } else if diff_mode {
            writeln!(buffer, "=== bytecode ===")?;
            emit_bytecode(&mut buffer, state, data, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer, "=== bytecode ===")?;
            emit_bytecode(&mut writer, state, data, opts)?;
        }
    }
    if opts.includes(Section::Rust) {
        if let Some(path) = opts.rust_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_rust(&mut writer, data, &state.database, end_def)?;
        } else if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== rust ===")?;
            emit_rust(&mut buffer, data, &state.database, end_def)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== rust ===")?;
            emit_rust(&mut writer, data, &state.database, end_def)?;
        }
    }
    if opts.includes(Section::Slots) {
        if let Some(path) = opts.slots_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_slots(&mut writer, data, end_def, opts)?;
        } else if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== slots ===")?;
            emit_slots(&mut buffer, data, end_def, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== slots ===")?;
            emit_slots(&mut writer, data, end_def, opts)?;
        }
    }
    if opts.includes(Section::Types) {
        if let Some(path) = opts.types_out.as_deref() {
            let mut writer = BufWriter::new(File::create(path)?);
            emit_types(&mut writer, data, end_def, opts)?;
        } else if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== types ===")?;
            emit_types(&mut buffer, data, end_def, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== types ===")?;
            emit_types(&mut writer, data, end_def, opts)?;
        }
    }
    // @PLN103 — opt-in only (`--show-ownership`), NOT part of the no-flags
    // "all sections" default, so a plain `introspect <file>` is unchanged.
    if opts.sections.contains(&Section::Ownership) {
        if diff_mode {
            writeln!(buffer)?;
            writeln!(buffer, "=== ownership ===")?;
            emit_ownership(&mut buffer, data, end_def, opts)?;
        } else {
            let mut writer = stdout.lock();
            writeln!(writer)?;
            writeln!(writer, "=== ownership ===")?;
            emit_ownership(&mut writer, data, end_def, opts)?;
        }
    }
    // Opt-in only (`--show-resolution`) — a cross-cutting view of the whole program
    // rather than a per-function dump, so it never joins the no-flags default.
    if opts.sections.contains(&Section::Resolution) {
        let mut writer = stdout.lock();
        writeln!(writer)?;
        writeln!(writer, "=== resolution ===")?;
        emit_resolution(&mut writer, data, opts)?;
    }
    // Opt-in only (a verification check, not a dump) — NOT part of the
    // no-flags "all sections" default, so it never pollutes a plain dump.
    if opts.sections.contains(&Section::Roundtrip) {
        let mut writer = stdout.lock();
        writeln!(writer)?;
        writeln!(writer, "=== roundtrip ===")?;
        emit_roundtrip(&mut writer, state, data, opts)?;
    }
    if let Some(baseline) = &opts.diff_against {
        run_diff_against_baseline(baseline, &buffer)?;
    }
    Ok(())
}

/// Emit every requested section as ONE JSON object — `{"<section>": "<text>", …}`
/// — via loft's own serializer (the "own your dependencies" rule).  Each value is
/// the SAME text the human-readable mode prints, captured into a buffer, so this
/// is a faithful machine-readable envelope over the existing emitters with no new
/// analysis: a tool / the LSP reads a section by key instead of splitting on the
/// `=== header ===` boundaries.  (Structuring the tabular sections — slots / types
/// — into per-function arrays is a later step; the byte-for-byte text is stable
/// today.)  Object key order is the canonical section order.
fn emit_json(
    data: &mut Data,
    state: &mut State,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    use crate::json::Parsed;
    let as_str = |buf: Vec<u8>| Parsed::Str(String::from_utf8_lossy(&buf).into_owned());
    let mut fields: Vec<(String, usize, Parsed)> = Vec::new();
    if opts.includes(Section::Bytecode) {
        let mut buf = Vec::new();
        emit_bytecode(&mut buf, state, data, opts)?;
        fields.push(("bytecode".to_string(), 0, as_str(buf)));
    }
    if opts.includes(Section::Rust) {
        let mut buf = Vec::new();
        emit_rust(&mut buf, data, &state.database, end_def)?;
        fields.push(("rust".to_string(), 0, as_str(buf)));
    }
    if opts.includes(Section::Slots) {
        let mut buf = Vec::new();
        emit_slots(&mut buf, data, end_def, opts)?;
        fields.push(("slots".to_string(), 0, as_str(buf)));
    }
    if opts.includes(Section::Types) {
        let mut buf = Vec::new();
        emit_types(&mut buf, data, end_def, opts)?;
        fields.push(("types".to_string(), 0, as_str(buf)));
    }
    // Opt-in only, matching the text path (never in the no-flags default).
    if opts.sections.contains(&Section::Ownership) {
        let mut buf = Vec::new();
        emit_ownership(&mut buf, data, end_def, opts)?;
        fields.push(("ownership".to_string(), 0, as_str(buf)));
    }
    if opts.sections.contains(&Section::Resolution) {
        let mut buf = Vec::new();
        emit_resolution(&mut buf, data, opts)?;
        fields.push(("resolution".to_string(), 0, as_str(buf)));
    }
    let mut w = std::io::stdout().lock();
    writeln!(
        w,
        "{}",
        crate::json::to_json_string(&Parsed::Object(fields))
    )
}

/// Dump each user function's bytecode, re-assemble it from that text via
/// [`compile::reassemble_function`], and compare to the original byte
/// stream.  Reports `ok` / `DIFFERS` / `error` per function plus a tally.
fn emit_roundtrip(
    w: &mut dyn Write,
    state: &mut State,
    data: &mut Data,
    opts: &Options,
) -> std::io::Result<()> {
    let (mut ok, mut bad) = (0u32, 0u32);
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function | DefType::Dynamic)
            || def.is_operator()
            || def.code_length == 0
        {
            continue;
        }
        let from_default = crate::portable_path::is_stdlib_source(&def.position.file);
        let pass_filter =
            opts.fn_filter.is_empty() || opts.fn_filter.iter().any(|f| def.name.contains(f));
        if (from_default && !opts.all_fns) || !pass_filter {
            continue;
        }
        let name = def.name.clone();
        let start = def.code_position as usize;
        let len = def.code_length as usize;
        let original = state.bytecode[start..start + len].to_vec();

        let mut buf: Vec<u8> = Vec::new();
        if let Err(e) = state.dump_code(&mut buf, d_nr, data, true) {
            writeln!(w, "  {name}: dump error: {e}")?;
            bad += 1;
            continue;
        }
        let dump = String::from_utf8_lossy(&buf);
        match compile::reassemble_function(&dump, data, &state.library_names) {
            Ok(rebuilt) if rebuilt == original => {
                writeln!(w, "  ok      {name}  ({len} bytes)")?;
                ok += 1;
            }
            Ok(rebuilt) => {
                let at = original
                    .iter()
                    .zip(&rebuilt)
                    .position(|(a, b)| a != b)
                    .unwrap_or(original.len().min(rebuilt.len()));
                writeln!(
                    w,
                    "  DIFFERS {name}  (orig {} B, rebuilt {} B, first diff at byte {at})",
                    original.len(),
                    rebuilt.len()
                )?;
                bad += 1;
            }
            Err(e) => {
                writeln!(w, "  error   {name}: {e}")?;
                bad += 1;
            }
        }
    }
    writeln!(w, "  ── {ok} identical, {bad} differing/error ──")?;
    Ok(())
}

/// Write `buffer` (the captured stdout-bound introspection output)
/// to a temp file, then exec `diff -u <baseline> <tmp>` so the user
/// sees a familiar unified-diff output.  Exits the process with
/// `diff`'s status — 0 for "no difference", 1 for "differs", 2 for
/// "trouble".  Requires `diff` on PATH; falls back to a "use system
/// diff yourself" message if unavailable.
fn run_diff_against_baseline(baseline: &str, buffer: &[u8]) -> std::io::Result<()> {
    if !std::path::Path::new(baseline).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("baseline file '{baseline}' not found"),
        ));
    }
    let tmp = std::env::temp_dir().join(format!("loft_introspect_diff_{}.txt", std::process::id()));
    std::fs::write(&tmp, buffer)?;
    let status = std::process::Command::new("diff")
        .arg("-u")
        .arg(baseline)
        .arg(&tmp)
        .status();
    let _ = std::fs::remove_file(&tmp);
    if let Ok(s) = status {
        // 0 = identical, 1 = differs.  Both are valid outcomes;
        // mirror diff's exit code.
        std::process::exit(s.code().unwrap_or(2));
    } else {
        eprintln!(
            "loft: --diff requires `diff` on PATH; \
             fall back to redirecting --introspect output and diffing manually."
        );
        std::process::exit(2);
    }
}

fn emit_bytecode(
    w: &mut dyn Write,
    state: &mut State,
    data: &mut Data,
    opts: &Options,
) -> std::io::Result<()> {
    let mut config = LogConfig::static_only();
    config.annotate_slots = true;
    config.show_all_functions = opts.all_fns;
    if !opts.fn_filter.is_empty() {
        config.show_functions = Some(opts.fn_filter.clone());
    } else if !opts.all_fns {
        // Match the slots/types sections: restrict to user functions.
        // show_code already skips `default/`, but compiler-synthesized
        // runtime helpers (`i_parse_*`) carry an empty position and a
        // non-`n_` name, so they'd otherwise leak in.  List the user
        // `n_` functions explicitly (show_functions is a substring
        // include-list); empty = no user functions = empty section.
        let user_fns: Vec<String> = (0..data.definitions())
            .filter_map(|d| {
                let def = data.def(d);
                (def.def_type == DefType::Function
                    && def.name.starts_with("n_")
                    && !def.name.starts_with("n___lambda_")
                    && !crate::compile::is_default_file(&def.position.file))
                .then(|| def.name.clone())
            })
            .collect();
        config.show_functions = Some(user_fns);
    }
    state
        .dump_bytecode(w, &config, data)
        .map_err(|e| std::io::Error::other(format!("dump_bytecode: {e}")))
}

fn emit_rust(
    w: &mut dyn Write,
    data: &Data,
    stores: &crate::database::Stores,
    end_def: u32,
) -> std::io::Result<()> {
    let mut out = generation::Output::new(data, stores);
    out.output_native(w, 0, end_def)
}

fn emit_slots(w: &mut dyn Write, data: &Data, end_def: u32, opts: &Options) -> std::io::Result<()> {
    for d_nr in 0..end_def {
        let def = data.def(d_nr);
        if def.def_type != DefType::Function {
            continue;
        }
        // User-callable functions are prefixed with `n_`; skip
        // operator definitions, generic templates, etc.  Mirrors
        // the bytecode path's user-fn filter.
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if !opts.all_fns && crate::compile::is_default_file(&def.position.file) {
            continue;
        }
        if !opts.fn_filter.is_empty()
            && !opts
                .fn_filter
                .iter()
                .any(|f| def.name == *f || def.name == format!("n_{f}") || def.name.contains(f))
        {
            continue;
        }
        if def.variables.count() == 0 {
            continue;
        }
        writeln!(w, "fn {}:", def.name)?;
        variables::dump_variables(w, &def.variables, data)
            .map_err(|e| std::io::Error::other(format!("dump_variables: {e}")))?;
        writeln!(w)?;
    }
    Ok(())
}

/// Per-function variable types with dependency tracking.
///
/// Each variable's full `Type` is rendered via `Type::show(data, vars)`,
/// which includes the `[dep_var, …]` suffix for types that carry
/// lifetime dependencies (`Text`, `Reference`, `Vector`, `Hash`,
/// etc.).  Designed to surface dep-propagation bugs at a glance —
/// e.g. P197 showed `s: text` (no deps) for a tuple-element text
/// read that should have inherited the host's `[a]` dependency.
/// @PLN103 P1 — per-binding store ownership.
///
/// For each user function, resolve every variable's ownership via
/// `use_analysis::ownership_of` over the COMMITTED `def.code` (P0.1: the borrowed
/// SOURCE binding survives synthesis, so no pre-synthesis snapshot is needed), and
/// render it with the P0.2 rule (`render_own`): an owned backing buffer reads
/// `Owned (backing=…)`, a genuine alias reads `Borrowed(base=…)`, a runtime split
/// reads `Join(base=…)`.  The function header shows `return_ownership`.
/// @PLN103 / crawler-H2 — report a temp that FREES a store it does not own.
///
/// A struct-returning call writes its result into a caller-allocated NRVO buffer
/// (`fn n_mk(d: D, __retbuf: E) -> E`), so the value it hands back IS that buffer.
/// When the result is bound to a temp inside a LOOP and the temp gets a scope-exit
/// `OpFreeRef`, the buffer — allocated ONCE outside the loop — is freed once per
/// iteration. Later iterations then write through freed memory that has since been
/// reallocated, silently corrupting data appended earlier. No crash, and the leak
/// counter reports only the downstream EFFECT (`Def×4 not freed`); this names the
/// CAUSE, which crawler's H2 needed three theories to reach.
///
/// The check: bind a var from `Call(f, [.., Var(buf)])` where `f` takes an
/// `__retbuf` parameter and `buf` was allocated at a shallower loop depth, then flag
/// any `OpFreeRef` of that var deeper than `buf`'s allocation.
fn per_iteration_frees(data: &Data, def: &crate::data::Definition) -> Vec<(String, String)> {
    let sets = data.op_sets();
    let free_nrs = &sets.unconditional_ref_frees;
    let db_nr = data.def_nr("OpDatabase");
    // var -> loop depth at which its store was allocated
    let mut alloc_depth: HashMap<u16, u32> = HashMap::new();
    // var -> (buffer var it aliases, that buffer's alloc depth)
    let mut aliases: HashMap<u16, (u16, u32)> = HashMap::new();
    let mut found: Vec<(String, String)> = Vec::new();

    /// Does `d` take a trailing `__retbuf` parameter (the NRVO shape)?
    fn takes_retbuf(data: &Data, d: u32) -> bool {
        let f = data.def(d);
        f.attributes()
            .last()
            .is_some_and(|a| a.name.starts_with("__retbuf"))
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        node: &Value,
        depth: u32,
        data: &Data,
        free_nrs: &crate::fxhash::FxHashSet<u32>,
        db_nr: u32,
        vars: &crate::variables::Function,
        alloc: &mut HashMap<u16, u32>,
        aliases: &mut HashMap<u16, (u16, u32)>,
        out: &mut Vec<(String, String)>,
    ) {
        match node.unspan() {
            // A `for` lowers to `Iter`, a `while`/`loop` to `Loop` — both repeat
            // their body, so both raise the depth.  Missing `Iter` made an earlier
            // version of this check silently see no loops at all.
            Value::Loop(bl) => {
                for op in &bl.operators {
                    walk(
                        op,
                        depth + 1,
                        data,
                        free_nrs,
                        db_nr,
                        vars,
                        alloc,
                        aliases,
                        out,
                    );
                }
            }
            Value::Iter(_, create, next, extra) => {
                walk(
                    create, depth, data, free_nrs, db_nr, vars, alloc, aliases, out,
                );
                for part in [next, extra] {
                    walk(
                        part,
                        depth + 1,
                        data,
                        free_nrs,
                        db_nr,
                        vars,
                        alloc,
                        aliases,
                        out,
                    );
                }
            }
            Value::Set(v, rhs) => {
                // The BINDING depth, for every var — an NRVO return buffer is declared
                // `__ref_N = null` at function top and filled by the callee, so keying
                // only on an explicit `OpDatabase` never saw the buffer at all.
                alloc.entry(*v).or_insert(depth);
                match rhs.unspan() {
                    Value::Call(d, args) if takes_retbuf(data, *d) => {
                        if let Some(Value::Var(buf)) = args.last().map(Value::unspan)
                            && let Some(&bd) = alloc.get(buf)
                        {
                            aliases.insert(*v, (*buf, bd));
                        }
                    }
                    _ => {}
                }
                walk(rhs, depth, data, free_nrs, db_nr, vars, alloc, aliases, out);
            }
            // `OpDatabase(Var(v), …)` mints a store INTO v in place — it is not a
            // `Set`, which an earlier version of this check assumed and so never saw
            // the buffer's allocation at all.
            Value::Call(d, args) if *d == db_nr && !args.is_empty() => {
                if let Value::Var(v) = args[0].unspan() {
                    alloc.entry(*v).or_insert(depth);
                }
            }
            Value::Call(d, args) if free_nrs.contains(d) && args.len() == 1 => {
                if let Value::Var(v) = args[0].unspan()
                    && let Some(&(buf, bd)) = aliases.get(v)
                    && depth > bd
                {
                    out.push((
                        vars.name(*v).to_string(),
                        format!(
                            "frees `{}` — the NRVO return buffer it ALIASES, allocated at loop \
                             depth {bd} and freed here at depth {depth}: once per iteration, \
                             while the enclosing scope still owns it",
                            vars.name(buf)
                        ),
                    ));
                }
            }
            other => other.for_each_child(&mut |c| {
                walk(c, depth, data, free_nrs, db_nr, vars, alloc, aliases, out);
            }),
        }
    }

    walk(
        def.code(),
        0,
        data,
        free_nrs,
        db_nr,
        &def.variables,
        &mut alloc_depth,
        &mut aliases,
        &mut found,
    );
    found
}

/// `--show-resolution` — the per-source name-visibility table, or `--why <name>`.
///
/// Answers the question that cost @PLN120 E.1 and E.4 a consumer report each: *which
/// names can this source see, and where did they come from.*  Reading it needed a
/// compiler edit before; it is now a flag on the shipped binary.
fn emit_resolution<W: Write>(w: &mut W, data: &Data, opts: &Options) -> std::io::Result<()> {
    if let Some(ctx) = opts.resolution_context.as_deref() {
        writeln!(w, "context: {ctx}")?;
    }
    if let Some(name) = opts.why.as_deref() {
        return emit_why(w, data, name);
    }
    let (sources, aliases) = data.resolution_view();
    writeln!(w, "sources:")?;
    for s in &sources {
        // `visible` counts every reachable name, `defined` only this source's own —
        // so the difference is precisely what its imports bought.
        writeln!(
            w,
            "  {:<4} defined {:<6} visible {:<6} {}",
            s.nr, s.defined, s.visible, s.name
        )?;
    }
    if aliases.is_empty() {
        // Not a cosmetic case: this is exactly what a rebuild that cannot reproduce
        // its derived state looks like, so name it rather than printing nothing.
        writeln!(
            w,
            "aliases: none — no name is reachable from a source other than its own              (a program with a `use` should have some; an empty list here is the              @PLN120 E.4 shape)"
        )?;
        return Ok(());
    }
    let n = aliases.len();
    writeln!(
        w,
        "aliases ({n} import binding{}):",
        if n == 1 { "" } else { "s" }
    )?;
    for a in &aliases {
        writeln!(
            w,
            "  src {:<4} <- src {:<4} #{:<6} {}",
            a.into_source, a.from_source, a.def_nr, a.name
        )?;
    }
    Ok(())
}

/// The `--why <name>` half of [`emit_resolution`]: where one name lives and who can
/// reach it.  A name nothing can see prints that, which is the answer to "why does
/// this call not resolve".
fn emit_why<W: Write>(w: &mut W, data: &Data, name: &str) -> std::io::Result<()> {
    let Some((def_nr, own, reachable)) = data.visibility_of(name) else {
        writeln!(
            w,
            "`{name}` is not defined in any source, and no source can reach it"
        )?;
        return Ok(());
    };
    writeln!(w, "`{name}` is #{def_nr}, defined in source {own}")?;
    for (src, is_own) in reachable {
        if is_own {
            writeln!(w, "  visible in source {src} (its own)")?;
        } else {
            writeln!(w, "  visible in source {src} (import alias)")?;
        }
    }
    Ok(())
}

fn emit_ownership(
    w: &mut dyn Write,
    data: &Data,
    end_def: u32,
    opts: &Options,
) -> std::io::Result<()> {
    // @PLN103 P2.0 — ownership is BACKEND-SHARED: the interp bytecode and native Rust
    // lower this SAME verdict (both read `use_analysis::ownership_of`), so there is no
    // per-backend column. A runtime interp≠native value-identity split is a codegen bug,
    // caught by the differential value oracle / leak+ASan gates (and P3's per-backend timeline).
    writeln!(
        w,
        "# store ownership is backend-shared (interp + native lower the same verdict)"
    )?;
    for d_nr in 0..end_def {
        let def = data.def(d_nr);
        if def.def_type != DefType::Function {
            continue;
        }
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        for (var, why) in per_iteration_frees(data, def) {
            writeln!(w, "!! {}: `{var}` {why}", def.name)?;
        }
        if !opts.all_fns && crate::compile::is_default_file(&def.position.file) {
            continue;
        }
        if !opts.fn_filter.is_empty()
            && !opts
                .fn_filter
                .iter()
                .any(|f| def.name == *f || def.name == format!("n_{f}") || def.name.contains(f))
        {
            continue;
        }
        if def.variables.count() == 0 {
            continue;
        }
        let vars = &def.variables;
        let ret = crate::use_analysis::return_ownership(data, d_nr);
        writeln!(
            w,
            "fn {} -> {}:",
            def.name,
            crate::use_analysis::fmt_own(ret, vars)
        )?;
        // @PLN104 — surface the loft#568 interpreter-orphan risk STATICALLY: a text return
        // backed frame-locally with no hidden `&text` retbuf hands owned text back by value,
        // which the interpreter orphans (native RAII drops it).  Same predicate the promotion
        // oracle uses, so the overlay names the leaker class without ASan.
        if let Some(kind) = crate::use_analysis::text_return_orphan_risk(data, d_nr) {
            writeln!(
                w,
                "  ⚠ loft#568: owned text returned by value ({kind}) — no &text retbuf; \
                 interpreter orphans it"
            )?;
        }
        writeln!(w, "  {:<4} {:<4} {:<22} ownership", "#", "arg", "name")?;
        writeln!(w, "  {}", "-".repeat(66))?;
        for v in 0..vars.count() {
            let arg = if vars.is_argument(v) { "arg" } else { "" };
            // Only heap-backed vars have store ownership; a scalar is by-value.
            let rendered = if vars.tp(v).heap_dep().is_some() {
                let own = crate::use_analysis::ownership_of(data, d_nr, &Value::Var(v));
                crate::use_analysis::render_own(own, vars, v)
            } else {
                "—  (scalar)".to_string()
            };
            writeln!(w, "  {v:<4} {arg:<4} {:<22} {rendered}", vars.name(v))?;
        }
        // @PLN103 (temporal extension) — the ownership verdicts above are
        // temporal-agnostic (identical for a correct free and a use-after-free).
        // This overlay names a free-before-dependent-read the static ownership
        // cannot show: a store freed while a live view into it is still read.
        for (store, via) in crate::use_analysis::free_before_dependent_read(data, d_nr) {
            writeln!(
                w,
                "  ⚠ UAF: `{}` is read AFTER `OpFreeRef({})` — `{}` views the freed \
                 store (backing={}); free-before-dependent-read",
                vars.name(via),
                vars.name(store),
                vars.name(via),
                vars.name(store)
            )?;
        }
        // @PLN35 — the return-alias sibling: a record the return value ALIASES, freed
        // with a plain OpFreeRef before the return (the P4-records safe form is
        // OpFreeRefIfDistinct). Invisible to the deref overlay above — the store is
        // delivered, not dereferenced in-frame.
        for store in crate::use_analysis::return_source_freed(data, d_nr) {
            writeln!(
                w,
                "  ⚠ UAF: `{}` is a RETURN SOURCE freed by a plain `OpFreeRef` before the \
                 return — the caller reads a freed store (use OpFreeRefIfDistinct); \
                 return-source-free",
                vars.name(store)
            )?;
        }
        // loft#759 — the second escape route. A `return` is not the only way a value
        // leaves this frame: a write through a `&` parameter publishes into storage the
        // caller owns, and the buffer that write delivered must not then be plain-freed.
        for store in crate::use_analysis::ref_param_publish_freed(data, d_nr) {
            writeln!(
                w,
                "  ⚠ UAF: `{}` was PUBLISHED through a `&` parameter, then freed by a \
                 plain `OpFreeRef` — the caller reads and writes a freed store (use \
                 OpFreeRefIfDistinct); ref-param-publish-free",
                vars.name(store)
            )?;
        }
        // @PLN103 P1.5 — the delivery lens: WHO frees the storage a heap return
        // names.  The verdict itself lives in `use_analysis::heap_return_delivery`,
        // because @PLN119 decides from the same three-way whether a library
        // function can be placed at all, and an overlay that said something else
        // would be describing a different program.
        let delivery = crate::use_analysis::heap_return_delivery(data, d_nr);
        if delivery != crate::use_analysis::HeapDelivery::NotHeap {
            let note = match delivery {
                crate::use_analysis::HeapDelivery::Owned => "owned (fresh store)".to_string(),
                crate::use_analysis::HeapDelivery::RetBuf => {
                    "materialised → return buffer".to_string()
                }
                _ => {
                    let names: Vec<&str> = def
                        .returned
                        .heap_dep()
                        .map(|deps| {
                            deps.iter()
                                .map(|&d| if d == u16::MAX { "?" } else { vars.name(d) })
                                .collect()
                        })
                        .unwrap_or_default();
                    format!("borrows {} (view returned)", names.join(", "))
                }
            };
            writeln!(w, "  delivery: {}  — {note}", def.returned.show(data, vars))?;
        }
        writeln!(w)?;
    }
    Ok(())
}

fn emit_types(w: &mut dyn Write, data: &Data, end_def: u32, opts: &Options) -> std::io::Result<()> {
    for d_nr in 0..end_def {
        let def = data.def(d_nr);
        if def.def_type != DefType::Function {
            continue;
        }
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if !opts.all_fns && crate::compile::is_default_file(&def.position.file) {
            continue;
        }
        if !opts.fn_filter.is_empty()
            && !opts
                .fn_filter
                .iter()
                .any(|f| def.name == *f || def.name == format!("n_{f}") || def.name.contains(f))
        {
            continue;
        }
        // Skip empty native-only fns (no user variables), which clutter
        // the output without adding signal.
        if def.variables.count() == 0 {
            continue;
        }
        let ret_str = def.returned.show(data, &def.variables);
        writeln!(w, "fn {} -> {ret_str}:", def.name)?;
        writeln!(w, "  {:<4} {:<4} {:<24} type [deps]", "#", "arg", "name")?;
        writeln!(w, "  {}", "-".repeat(70))?;
        for idx in 0..def.variables.count() {
            let arg_flag = if def.variables.is_argument(idx) {
                "arg"
            } else {
                ""
            };
            let var_name = def.variables.name(idx).to_string();
            let type_str = def.variables.tp(idx).show(data, &def.variables);
            writeln!(w, "  {idx:<4} {arg_flag:<4} {var_name:<24} {type_str}")?;
        }
        // Per-expression trace from the parser, if any was recorded.
        // Each line is `<fn_name>\t<line>:<col>\t<type>` where
        // fn_name is the raw user name (`first`), not the
        // `n_`-prefixed def name (`n_first`).
        let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        let fn_trace: Vec<&String> = opts
            .trace_lines
            .iter()
            .filter(|l| l.split('\t').next().is_some_and(|n| n == user_name))
            .collect();
        if !fn_trace.is_empty() {
            writeln!(w)?;
            writeln!(w, "  trace (per-expression types):")?;
            for line in fn_trace {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                if let [_fn, pos, ty] = parts.as_slice() {
                    writeln!(w, "    {pos:<10} {ty}")?;
                }
            }
        }
        writeln!(w)?;
    }
    Ok(())
}
