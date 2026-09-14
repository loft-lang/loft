// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F56 — live code reload (patch a running program)
// @I78 — Live-reload dispatch

//! @PLN18 phase 02 — tier 0 of the execution tiers: **live per-function
//! reload** of a RUNNING program (`LOFT_LIVE_RELOAD=1`).
//!
//! The mechanism is a pointer patch, not a recompile: the runtime already
//! dispatches fn-refs through `State::fn_positions[d_nr]` per call, and
//! codegen records every direct call site per callee in `State::calls`.
//! So an edit becomes:
//!
//! 1. a **shadow parser session** (the same program parsed through the same
//!    pipeline at startup — parity-checked, so the def-number space is
//!    identical to the running program's) parses the changed function under
//!    a versioned temp name (`__reload_v3_handle_seed`) — no redefinition
//!    machinery, no in-place IR surgery;
//! 2. `State::def_code` appends the new body's bytecode at the END of the
//!    stream (append-only is the invariant: live frames — `run()`'s loop —
//!    keep executing old positions);
//! 3. the original def's targets are patched: `fn_positions[d_nr]` (fn-ref
//!    calls re-resolve per call) plus the recorded `OpCall` sites' embedded
//!    `to` operand.  Self-recursion in the new body names the ORIGINAL def,
//!    so it lands on the new body through the same patch.
//!
//! The poll runs INSIDE the execute loop (a counter-gated check, ~32k ops),
//! so application is frame-boundary-safe by construction: single-threaded,
//! never inside a call to the edited fn's old body mid-frame — the flip is
//! only observable at the next call.
//!
//! v1 boundaries (each rejected gracefully — the program keeps running on
//! the old body, with one warning):
//! - NAMED top-level fns only (closures/lambdas reload only as part of a
//!   named fn that contains them);
//! - the signature must be unchanged (call sites embed arg frame sizes);
//! - generator fns (iterator returns) are not swapped (their call sites
//!   emit `OpCoroutineCreate`, a different shape);
//! - a parse error keeps the old body (stale meaning beats a dead loop).

use std::cell::RefCell;
use std::time::{Duration, Instant};

use crate::state::State;

/// Throttle for the file-content check (the op-counter gets us here far
/// more often than this; the disk read is the real cost).
const CHECK_EVERY: Duration = Duration::from_millis(200);

/// One watched source file of the running program (#351: the watch covers
/// every parsed file, not only the entry — a live edit lands in modules and
/// bundle libraries too).
struct WatchedFile {
    path: String,
    last_content: String,
    /// The def-source id this file's fns parsed under — reload looks the
    /// edited fn up source-aware, so two libraries may share a fn name.
    source: u16,
}

pub struct ReloadHost {
    /// The shadow session: same program, same pipeline, parity-checked
    /// def space.  Its `Data` is where reload parses land; the RUNNING
    /// state's bytecode is where their code is generated.
    session: crate::repl::ReplSession,
    files: Vec<WatchedFile>,
    last_check: Instant,
    version: u32,
}

thread_local! {
    static HOST: RefCell<Option<ReloadHost>> = const { RefCell::new(None) };
}

/// Is live reload active on this thread?  One cheap check for the execute
/// loop's counter-gated slow path.
pub fn active() -> bool {
    HOST.with(|h| h.borrow().is_some())
}

/// Install the reload host for `path`.  Builds the shadow session through
/// the ordinary program pipeline and parity-checks its def space against
/// the running program's; on any mismatch reload is disabled with a
/// warning — never an error (the program itself is unaffected).
pub fn install(path: &str, stdlib_dir: &str, lib_dirs: &[String], running: &crate::data::Data) {
    let Ok(content) = std::fs::read_to_string(path) else {
        eprintln!("live-reload: cannot read {path}; reload disabled");
        return;
    };
    let mut session = match crate::repl::ReplSession::new_with_libs(stdlib_dir, lib_dirs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("live-reload: shadow session failed ({e}); reload disabled");
            return;
        }
    };
    match session.load_program(path) {
        Ok(Ok(())) => {}
        Ok(Err(diags)) => {
            // The parse gate is errors-only (warnings never disqualify —
            // parity with the main session, #347), and the refusal must be
            // diagnosable: a bare count hid WHICH shadow-only errors fired
            // and sent the consumer chasing their warnings.
            let errors: Vec<_> = diags
                .iter()
                .filter(|e| e.level >= crate::diagnostics::Level::Error)
                .collect();
            eprintln!(
                "live-reload: shadow parse failed with {} error(s) ({} diagnostics total); reload disabled",
                errors.len(),
                diags.len()
            );
            for e in errors.iter().take(3) {
                eprintln!("live-reload:   {}", e.to_string_compact());
            }
            if errors.len() > 3 {
                eprintln!("live-reload:   … {} more", errors.len() - 3);
            }
            return;
        }
        Err(e) => {
            eprintln!("live-reload: shadow load failed ({e}); reload disabled");
            return;
        }
    }
    // Parity: the patch writes MAIN-space def numbers into the running
    // state; the shadow's bytecode embeds SHADOW-space numbers.  They must
    // be the same space — same count, same names, same order.
    let shadow = &session.parser.data;
    if shadow.definitions() != running.definitions() {
        eprintln!(
            "live-reload: def-space mismatch (shadow {} vs running {}); reload disabled",
            shadow.definitions(),
            running.definitions()
        );
        return;
    }
    for d in 0..running.definitions() {
        if shadow.def(d).name != running.def(d).name {
            eprintln!(
                "live-reload: def {d} name mismatch ('{}' vs '{}'); reload disabled",
                shadow.def(d).name,
                running.def(d).name
            );
            return;
        }
    }
    // #351 — the watch set: every file the program parsed (entry, modules,
    // `--lib` packages, bundles), except the stdlib and synthetic sources.
    // Pair each file with its def-source id for source-aware fn lookup.
    let stdlib_prefix = crate::portable_path::plain_canonical_str(stdlib_dir);
    let mut files: Vec<WatchedFile> = vec![WatchedFile {
        path: path.to_string(),
        last_content: content,
        // The entry file parses under `MAIN_SOURCE`, distinct from the stdlib prelude's 0
        // (`Parser::parse`); the id is what a snippet parses under and what an overload set
        // is looked up by, so it has to be the file's own.
        source: crate::data::MAIN_SOURCE,
    }];
    for d in 0..shadow.definitions() {
        let def = shadow.def(d);
        let f = &def.position.file;
        if f.is_empty() || f.starts_with('<') || f == path {
            continue;
        }
        let canon = crate::portable_path::plain_canonical_str(f);
        if canon.starts_with(&stdlib_prefix) {
            continue;
        }
        if files.iter().any(|w| w.path == *f) {
            continue;
        }
        let Ok(c) = std::fs::read_to_string(f) else {
            continue; // unreadable (virtual / moved) — not watchable
        };
        files.push(WatchedFile {
            path: f.clone(),
            last_content: c,
            source: def.source,
        });
    }
    let module_count = files.len() - 1;
    HOST.with(|h| {
        *h.borrow_mut() = Some(ReloadHost {
            session,
            files,
            last_check: Instant::now(),
            version: 0,
        });
    });
    if module_count > 0 {
        eprintln!("live-reload: watching {path} (+{module_count} module/library files)");
    } else {
        eprintln!("live-reload: watching {path}");
    }
}

/// The execute-loop hook: throttled file check; on change, reload every
/// changed named fn.  Returns `true` when bytecode was appended (the loop
/// must refresh its cached length).
pub fn poll(state: &mut State) -> bool {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        let Some(host) = h.as_mut() else {
            return false;
        };
        if host.last_check.elapsed() < CHECK_EVERY {
            return false;
        }
        host.last_check = Instant::now();
        let mut grew = false;
        for i in 0..host.files.len() {
            let Ok(content) = std::fs::read_to_string(&host.files[i].path) else {
                continue; // transient (editor mid-save); next poll retries
            };
            if content == host.files[i].last_content {
                continue;
            }
            let old_fns = fn_blocks(&host.files[i].last_content);
            let new_fns = fn_blocks(&content);
            host.files[i].last_content = content;
            let source = host.files[i].source;
            // A head that is gone is a removal or a re-signature; neither half-applies
            // (`Disp-World`: the world only ever GROWS, and a signature is a call site's frame).
            // The names it touches are refused whole; every other head is judged on its own.
            let mut refused: Vec<&str> = Vec::new();
            for (head, (name, _)) in &old_fns {
                if new_fns.contains_key(head) {
                    continue;
                }
                if new_fns.values().any(|(n, _)| n == name) {
                    eprintln!("live-reload: '{name}' changed its signature; restart to apply");
                } else {
                    eprintln!(
                        "live-reload: '{head}' was removed; the running program keeps it — restart to apply"
                    );
                }
                refused.push(name);
            }
            for (head, (name, new_src)) in &new_fns {
                if refused.iter().any(|r| r == name) {
                    continue;
                }
                match old_fns.get(head) {
                    Some((_, old_src)) if old_src == new_src => {}
                    Some(_) => grew |= reload_fn(host, state, name, head, new_src, source),
                    None => grew |= add_fn_block(host, state, name, head, new_src, source),
                }
            }
        }
        grew
    })
}

/// Extract every column-0 `fn`/`pub fn` block, keyed by its declaration HEAD — the first
/// line with `pub ` stripped, whitespace collapsed and the opening `{` dropped — to
/// `(name, full text)`.  The head, not the name, is the key: an overload set spells one name
/// several times (`Disp-Key`, @PLN162), and keyed by name the map held only the last block,
/// so an edit to the first overload was skipped in silence and an added overload read as an
/// edit of the last (measured, IMPL.md step 14).  Repo style: the body's closing `}` sits at
/// column 0 (the same shape the extraction-hygiene scanner relies on).
fn fn_blocks(src: &str) -> std::collections::HashMap<String, (String, String)> {
    let mut out = std::collections::HashMap::new();
    let lines: Vec<&str> = src.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let decl = lines[i].strip_prefix("pub ").unwrap_or(lines[i]);
        if let Some(after) = decl.strip_prefix("fn ") {
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let head = decl
                .trim_end()
                .trim_end_matches('{')
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let start = i;
            while i < lines.len() && lines[i] != "}" {
                i += 1;
            }
            if i < lines.len() && !name.is_empty() {
                out.insert(head, (name, lines[start..=i].join("\n")));
            }
        }
        i += 1;
    }
    out
}

/// Parse one changed fn under a temp name in the shadow session, generate
/// its bytecode into the running state (append-only), and patch the
/// original def's dispatch targets.  Returns `true` when code was appended.
fn reload_fn(
    host: &mut ReloadHost,
    state: &mut State,
    name: &str,
    head: &str,
    new_src: &str,
    source: u16,
) -> bool {
    host.version += 1;
    let v = host.version;
    let data = &host.session.parser.data;
    // Source-aware lookup first (#351): two libraries may define the same fn
    // name; the edited FILE pins which one this is.  Entry-file fns parse
    // under source 0, where the global lookup is the same thing.
    let want = format!("n_{name}");
    let mut orig = u32::MAX;
    for d in 0..data.definitions() {
        if data.def_type(d) == crate::data::DefType::Function
            && data.def(d).source == source
            && data.def(d).name == want
        {
            orig = d;
            break;
        }
    }
    if orig == u32::MAX {
        orig = data.def_nr(&want);
    }
    // An overload set (`Disp-Key`, @PLN162) has no `n_<name>`: its members are keyed by
    // their parameter types, and the one this block belongs to is found by signature once
    // the block has been parsed (below).
    let set = overload_set(data, source, name);
    if orig == u32::MAX && set == u32::MAX {
        eprintln!("live-reload: '{name}' is not a known fn; skipped");
        return false;
    }
    if orig != u32::MAX && is_generator(data, orig) {
        eprintln!("live-reload: '{name}' is a generator; not swappable (restart to apply)");
        return false;
    }
    // Parse under the versioned temp name (the signature guard runs after
    // the parse, comparing typed attributes — text comparison is too weak).
    let temp_name = format!("__reload_v{v}_{name}");
    let decl = format!("fn {name}");
    let Some(rewritten) = rewrite_once(new_src, &decl, &format!("fn {temp_name}")) else {
        eprintln!("live-reload: cannot rewrite '{name}'; skipped");
        return false;
    };
    // Parse under the ORIGINAL fn's source id with the session's import
    // scoping intact (#350) — the snippet then resolves the same library
    // names (qualified and imported) its file did.
    // A set member is found by signature only after the parse; the set's source is the
    // member's (`Disp-Key` joins within one source).
    let orig_source = if orig == u32::MAX {
        host.session.parser.data.def(set).source
    } else {
        host.session.parser.data.def(orig).source
    };
    let parser = &mut host.session.parser;
    let pre_defs = parser.data.definitions();
    let pre_diag = parser.diagnostics.entries().len();
    parser.parse_snippet(&rewritten, "<live-reload>", orig_source);
    let produced = &parser.diagnostics.entries()[pre_diag..];
    if produced
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error)
    {
        for e in produced {
            eprintln!("live-reload: {name}: {}", e.message);
        }
        parser.data.rollback_to(pre_defs);
        eprintln!("live-reload: '{name}' kept its old body (fix the error and save again)");
        return false;
    }
    let temp = parser.data.def_nr(&format!("n_{temp_name}"));
    if temp == u32::MAX {
        parser.data.rollback_to(pre_defs);
        eprintln!("live-reload: '{name}' parsed but produced no def; skipped");
        return false;
    }
    if orig == u32::MAX {
        // The member of the set with this block's signature.
        orig = set_members(&parser.data, set)
            .into_iter()
            .find(|&m| same_signature(&parser.data, m, temp))
            .unwrap_or(u32::MAX);
        if orig == u32::MAX {
            parser.data.rollback_to(pre_defs);
            eprintln!(
                "live-reload: no definition of '{name}' in the running program has the signature '{head}'; restart to apply"
            );
            return false;
        }
        if is_generator(&parser.data, orig) {
            parser.data.rollback_to(pre_defs);
            eprintln!("live-reload: '{name}' is a generator; not swappable (restart to apply)");
            return false;
        }
    }
    // Signature guard: same arity + arg types + return type.
    if !same_signature(&parser.data, orig, temp) {
        parser.data.rollback_to(pre_defs);
        eprintln!("live-reload: '{name}' changed its signature; restart to apply");
        return false;
    }

    let sites = commit(parser, state, pre_defs, &[(orig, temp)]);
    eprintln!(
        "live-reload: '{name}' v{v} live ({sites} call site(s) patched, fn-refs via dispatch)"
    );
    true
}

/// The `Dynamic` dispatcher of `name`'s overload set in `source` (`Disp-Key`, @PLN162), or
/// `u32::MAX` when the name has no set there.  A set's dispatcher carries the bare name; its
/// members are keyed by their parameter types.
fn overload_set(data: &crate::data::Data, source: u16, name: &str) -> u32 {
    let mut d = data.source_nr(source, name);
    if d == u32::MAX {
        d = data.def_nr(name);
    }
    if d != u32::MAX && data.def_type(d) == crate::data::DefType::Dynamic {
        d
    } else {
        u32::MAX
    }
}

/// The members of an overload set, in declaration order.
fn set_members(data: &crate::data::Data, set: u32) -> Vec<u32> {
    data.def(set)
        .attributes
        .iter()
        .filter_map(|a| match a.typedef.base() {
            crate::data::Type::Routine(r) => Some(*r),
            _ => None,
        })
        .collect()
}

fn is_generator(data: &crate::data::Data, d: u32) -> bool {
    matches!(data.def(d).returned(), crate::data::Type::Iterator(_, _))
}

/// Same arity + argument types + return type — the frame a call site embeds.
fn same_signature(data: &crate::data::Data, a: u32, b: u32) -> bool {
    let (a, b) = (data.def(a), data.def(b));
    a.attributes.len() == b.attributes.len()
        && a.attributes
            .iter()
            .zip(b.attributes.iter())
            .all(|(x, y)| format!("{:?}", x.typedef) == format!("{:?}", y.typedef))
        && format!("{:?}", a.returned()) == format!("{:?}", b.returned())
}

/// `Disp-World` (@PLN162 step 14): a `fn` block whose head the file did not have before.
///
/// Tier 0 used to skip it — *"nothing calls it yet"* — which is true of a new NAME and false
/// of a new OVERLOAD: every site of the set is its caller, and the selection those sites
/// compiled was made in a world that did not have it.  So an overload joins the running
/// program's world: the block is parsed under its own name in the shadow session (it joins
/// the set by `Disp-Key`), every specialisation of the name — the per-spelling `__sel_` stubs
/// and `__dyn_` dispatchers the open profile lowers every call of the set to — is rebuilt in
/// the new world, and each is swapped in behind its old def through the same patch a body
/// edit takes, so the running loop's next call selects in the new world and no body selected
/// in an earlier one runs again.  The whole step is one transaction: a rebuild the new set
/// cannot decide (a served tuple now ambiguous, Q3 — the ADD is refused, as the design
/// proposes) rolls everything back and the world is unchanged.  A new name is still skipped,
/// and a second definition of a name that was ONE function is refused: its sites were direct
/// calls with nothing to rebuild.
fn add_fn_block(
    host: &mut ReloadHost,
    state: &mut State,
    name: &str,
    head: &str,
    src: &str,
    source: u16,
) -> bool {
    let data = &host.session.parser.data;
    let set = overload_set(data, source, name);
    if set == u32::MAX {
        if data.source_nr(source, &format!("n_{name}")) != u32::MAX {
            eprintln!(
                "live-reload: '{head}' would make '{name}' an overload set, which the running program did not start with; restart to apply"
            );
        }
        // A brand-new name: nothing calls it yet — the next full run picks it up.
        return false;
    }
    // The block joins under the SET's source (`Disp-Key` joins within one source only), which
    // is the file's own id whatever the watcher recorded for it.
    let set_source = data.def(set).source;
    host.version += 1;
    let world = host.version;
    let parser = &mut host.session.parser;
    let pre_defs = parser.data.definitions();
    let pre_diag = parser.diagnostics.entries().len();
    parser.parse_snippet(src, "<live-reload>", set_source);
    let produced = &parser.diagnostics.entries()[pre_diag..];
    if produced
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error)
    {
        for e in produced {
            eprintln!("live-reload: {name}: {}", e.message);
        }
        parser.data.rollback_to(pre_defs);
        eprintln!("live-reload: '{head}' is refused; the running world is unchanged");
        return false;
    }
    if !set_members(&parser.data, set)
        .iter()
        .any(|&m| m >= pre_defs)
    {
        parser.data.rollback_to(pre_defs);
        eprintln!("live-reload: '{head}' parsed but did not join the set of '{name}'; skipped");
        return false;
    }
    // Every specialisation of the name from the worlds before this one — the defs the running
    // program's sites call, whose numbers stay the dispatch home; a rebuild from an earlier
    // world (`__w<k>`) is reached only through one of these and is not rebuilt again.
    let prefix_dyn = format!("n_{name}__dyn_");
    let prefix_sel = format!("n_{name}__sel_");
    let specs: Vec<u32> = (0..pre_defs)
        .filter(|&d| {
            let def = parser.data.def(d);
            def.synthetic() == Some("dynamic_dispatcher")
                && !def.name.contains("__w")
                && (def.name.starts_with(&prefix_dyn) || def.name.starts_with(&prefix_sel))
        })
        .collect();
    let lex_pre = parser.lexer.diagnostics().entries().len();
    let mut swaps: Vec<(u32, u32)> = Vec::with_capacity(specs.len());
    for &old in &specs {
        match parser.rebuild_specialisation(old, world) {
            Some(new) => swaps.push((old, new)),
            None => break,
        }
    }
    // The rebuilt specialisations are definitions pass 2 never saw: an owned text return
    // among them takes its `___tret` retbuf here, exactly as the originals took theirs at
    // the program's parse — the running program's sites push that buffer.
    if swaps.len() == specs.len() {
        parser.after_pass2();
    }
    let refusals: Vec<String> = parser.lexer.diagnostics().entries()[lex_pre..]
        .iter()
        .filter(|e| e.level >= crate::diagnostics::Level::Error)
        .map(|e| e.message.clone())
        .collect();
    parser.reported_dynamic_refusal = false;
    if swaps.len() != specs.len() || !refusals.is_empty() {
        for m in &refusals {
            eprintln!("live-reload: {name}: {m}");
        }
        parser.data.rollback_to(pre_defs);
        eprintln!(
            "live-reload: adding '{head}' is refused — it leaves a call of '{name}' the set cannot decide; the running world is unchanged"
        );
        return false;
    }
    let sites = commit(parser, state, pre_defs, &swaps);
    eprintln!(
        "live-reload: '{head}' added — world {world}: {} specialisation(s) of '{name}' rebuilt, {sites} call site(s) patched",
        swaps.len()
    );
    true
}

/// Generate every def the shadow session added since `pre_defs` into the running state and
/// swap each `(old, new)` pair in: `fn_positions[old]` and every recorded call site of `old`
/// now reach `new`'s body.  Returns the number of sites patched.
fn commit(
    parser: &mut crate::parser::Parser,
    state: &mut State,
    pre_defs: u32,
    swaps: &[(u32, u32)],
) -> usize {
    // The shadow session only PARSES — it never generates bytecode, so its
    // defs carry no code positions.  A call emitted from the new body embeds
    // the callee's `code_position` (codegen's OpCall `to` operand), so sync
    // every def's position from the RUNNING state first (the def spaces are
    // parity-checked equal at install; without this, a reloaded fn calling
    // ANY user fn jumped to position 0 and crashed).
    for d in 0..pre_defs.min(state.fn_positions.len() as u32) {
        parser.data.definitions[d as usize].code_position = state.fn_positions[d as usize];
    }

    // Generate the new bodies (and any lambdas they contain) at the END of the
    // running bytecode stream.  `code_pos` is the LIVE PC — save + restore.
    let saved_pc = state.code_pos;
    state.code_pos = state.bytecode.len() as u32;
    state.database.allocations[crate::database::CONST_STORE as usize].unlock();
    for d in pre_defs..parser.data.definitions() {
        if matches!(
            parser.data.def(d).def_type(),
            crate::data::DefType::Function
        ) && !parser.data.def(d).is_operator()
        {
            state.def_code(d, &mut parser.data, None);
        }
    }
    state.database.allocations[crate::database::CONST_STORE as usize]
        .lock_with_origin("live_reload (CONST_STORE relock)");
    state.code_pos = saved_pc;

    // Patch the dispatch targets: fn-refs re-resolve via fn_positions per
    // call; a direct call site carries an embedded `to`, and `state.calls`
    // records the address of that i64 operand itself.
    //
    // loft#1032 — it used to record the OPCODE's address, and both this site and
    // codegen's own back-patch re-derived the operand as `+ opcode(1) + d_nr(8) +
    // args_size(2)`.  That opcode is one byte only below 255 and TWO at or above it
    // (`state::emit_op`), so the derivation was wrong for `OpCoroutineCreate` in both
    // copies.  Recording the operand address at emission removes the arithmetic from
    // both, which is why this reads `site` and not `site + 11`.
    if state.fn_positions.len() < parser.data.definitions() as usize {
        let mut ext: Vec<u32> = Vec::new();
        for d in state.fn_positions.len() as u32..parser.data.definitions() {
            ext.push(parser.data.def(d).code_position);
        }
        state.fn_positions.extend(ext);
    }
    if std::env::var_os("LOFT_RELOAD_DEBUG").is_some() {
        for &(orig, new) in swaps {
            eprintln!(
                "live-reload: swap {orig} {} -> {new} {}",
                parser.data.def(orig).name,
                parser.data.def(new).name
            );
        }
        for d in 0..parser.data.definitions() {
            let def = parser.data.def(d);
            if def.name.contains("_hit") || def.name == "hit" {
                eprintln!(
                    "live-reload: def {d} {} ({:?}) params {:?}",
                    def.name,
                    def.def_type,
                    def.attributes
                        .iter()
                        .map(|a| format!("{}{}", a.name, if a.hidden { "(h)" } else { "" }))
                        .collect::<Vec<_>>()
                );
            }
        }
        for d in pre_defs..parser.data.definitions() {
            let def = parser.data.def(d);
            eprintln!(
                "live-reload: def {d} {} ({:?}) code {}..{} params {:?}",
                def.name,
                def.def_type,
                def.code_position,
                def.code_position + def.code_length,
                def.attributes
                    .iter()
                    .map(|a| format!("{}{}", a.name, if a.hidden { "(h)" } else { "" }))
                    .collect::<Vec<_>>()
            );
        }
    }
    let mut patched = 0;
    for &(orig, new) in swaps {
        let new_pos = parser.data.def(new).code_position;
        state.fn_positions[orig as usize] = new_pos;
        let sites = state.calls.get(&orig).cloned().unwrap_or_default();
        for site in &sites {
            state.code_put::<i64>(*site, i64::from(new_pos));
        }
        patched += sites.len();
    }
    patched
}

/// First-occurrence textual rewrite; `None` when the needle is absent.
fn rewrite_once(src: &str, from: &str, to: &str) -> Option<String> {
    src.find(from)
        .map(|i| format!("{}{}{}", &src[..i], to, &src[i + from.len()..]))
}
