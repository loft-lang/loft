// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I82 — Sandbox policy & enforcement

//! @PLN86 step 1.1 — the sandbox **policy model**: capability-group allow-lists
//! plus the coarse bans, and the `loft.toml` `[sandbox]` / `[profile.*]` parser
//! that builds them.
//!
//! Nothing here executes — it is policy DATA the compile-time admission walk
//! reads (`SandboxProfile::allows` is the central capability check used by the
//! group-membership admission, @PLN86 step 2.3).  Deny-by-default: a `group#right`
//! capability link is permitted only when an `allow` entry has the same right and
//! its group is a dotted-segment prefix of the link's group.
//!
//! Enforces @FR-Cap-Deny — no rule admitting an expression means it is rejected AT LOAD,
//! never at the call — and holds the policy that @FR-Cap-Call, @FR-Cap-Read and
//! @FR-Cap-Write consult.  @FR-Cap-Trusted is enforced by [`reachable_set`], where a
//! trusted symbol is a leaf: its body is the host's contract and is not walked.

use crate::data::{Data, DefType, Position, Type, Value};
use std::collections::{HashMap, HashSet};

/// A named admission profile: which capability **groups** a sandboxed script may
/// reach, plus the always-on coarse bans (never native/rustc, never a `[native]`
/// cdylib — both RCE surfaces).  Defaults are the safe ones: deny every
/// capability, interpret-only, no FFI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SandboxProfile {
    /// Libraries allowed **wholesale** (e.g. `"text"`, `"crypto"`).  Every symbol
    /// from an allow-listed library admits with NO capability link — a complete,
    /// host-vetted library is included as a unit.  Links (`allow`) are only for
    /// carving a library in half ("include `files`, but reads only").
    pub allow_libs: Vec<String>,
    /// Granted `group#right` capability tokens (e.g. `"fs#read"`, `"game#update"`).
    /// The fine-grained layer, used when a library is NOT allowed wholesale.  A
    /// grant matches a symbol's link when the RIGHT is exactly equal and the
    /// grant's group is a dotted-segment prefix of the link's group (`"game#read"`
    /// grants `"game.entity#read"` but NOT `"game#update"`).
    pub allow: Vec<String>,
    /// May this profile reach a vetted `[native]` cdylib bridge (dlopen of an external
    /// crate = RCE)?  Default false — admission rejects a reachable external FFI bridge
    /// unless this is set.  (This is the surface the dropped interpret-only force was
    /// really guarding; the admission proof itself is backend-agnostic, so a sandboxed
    /// program runs on whatever backend the host picks.)
    pub native_ffi: bool,
    /// @PLN86 §8 (P7.3) — the data envelope.  The host declares the input bounds the
    /// complexity degrees are expressed in plus a peak-heap budget; admission proves
    /// the footprint fits or rejects (F11).  `0` = unset: an unset bound that a degree
    /// actually depends on makes the footprint unprovable (rejected); an unset
    /// `data_budget` disables the check (report-only).
    ///
    /// Largest input collection / range size `n` (the variable the `O(n^d)` degrees
    /// are in).
    pub max_input_n: u64,
    /// Maximum structure nesting depth the host will feed in.
    pub max_depth: u64,
    /// Maximum dynamic-string length (bytes) — bounds an otherwise-uncapped string
    /// allocation (F10).
    pub max_string_len: u64,
    /// Peak live-heap budget in bytes.  `coeff · max_input_n^degree` must be ≤ this,
    /// else the script is rejected at load (F11).  `0` disables the budget check.
    pub data_budget: u64,
}

impl SandboxProfile {
    /// True iff library `lib` is allow-listed wholesale (exact name match).  A
    /// match admits every symbol from that library — including its untagged
    /// functions and its native bridges — because the host vetted it as a unit.
    #[must_use]
    pub fn allows_lib(&self, lib: &str) -> bool {
        !lib.is_empty() && self.allow_libs.iter().any(|l| l == lib)
    }

    /// True iff a symbol's `group#right` capability link `token` is granted by this
    /// profile.
    ///
    /// Enforces @FR-Cap-Call — a gated call is admitted only when its `g#r` token is in the
    /// policy (or its library is allow-listed, which `allows_lib` answers) — and with it
    /// @FR-Cap-Deny, the closing rule: this function returning `false` is what makes an
    /// expression REJECTED AT LOAD rather than at the call.  Every `false` here is a
    /// program that will not run, so widening the match widens the sandbox.
    ///
    /// **Deny-by-default**: a grant matches only when the RIGHT is exactly
    /// equal AND the grant's group is a dotted-segment prefix of the link's group —
    /// so `"game#read"` grants `"game.entity#read"` but NOT `"game#update"` (a
    /// different right) or `"gameover#read"` (not a segment boundary).  A token (or
    /// grant) without a `#right` matches nothing.
    #[must_use]
    pub fn allows(&self, token: &str) -> bool {
        let Some((group, right)) = token.split_once('#') else {
            return false;
        };
        if group.is_empty() || right.is_empty() {
            return false;
        }
        self.allow.iter().any(|a| {
            a.split_once('#')
                .is_some_and(|(ag, ar)| ar == right && cap_prefix_match(ag, group))
        })
    }
}

/// @PLN86 P6.1 — the access rights a `group#right` capability link selects.
/// `read` observes (a field read / index read / variant match), `update` mutates an
/// existing slot in place (`e.health = 0`), `append` introduces something new (grows
/// a structure, or constructs a host enum variant).  These three are independent, so
/// an append-only member (grow a log, never alter it) is expressible — `update` does
/// not imply `append`.
///
/// `default` (§7.2, F7) is the odd one out: it tags a function PARAMETER, not a value,
/// and is the inverse polarity — allow-by-default.  A modder may always call a function
/// (subject to its call gate) and pass any untagged argument, but a parameter the host
/// pinned with `…#default` is forced to its default unless the modder ALSO holds the
/// lock.  So it gates an *argument override*, not a read/write of host state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Right {
    Read,
    Update,
    Append,
    Default,
}

impl Right {
    /// Parse the suffix of a `group#right` token; `None` if `s` is not a known right.
    #[must_use]
    pub fn parse(s: &str) -> Option<Right> {
        match s {
            "read" => Some(Right::Read),
            "update" => Some(Right::Update),
            "append" => Some(Right::Append),
            "default" => Some(Right::Default),
            _ => None,
        }
    }

    /// The token suffix for this right (`"read"` / `"update"` / `"append"` / `"default"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Right::Read => "read",
            Right::Update => "update",
            Right::Append => "append",
            Right::Default => "default",
        }
    }
}

/// Dotted-segment prefix match: `allow` matches `group` iff `group == allow` or
/// `group` continues `allow` at a `.` boundary.  So `"game"` ⊇ `"game.read"` but
/// not `"gameover"`; `"game.read"` is exact.
#[must_use]
pub fn cap_prefix_match(allow: &str, group: &str) -> bool {
    if allow.is_empty() {
        return false;
    }
    group == allow
        || (group.len() > allow.len()
            && group.starts_with(allow)
            && group.as_bytes()[allow.len()] == b'.')
}

/// The whole sandbox configuration from a program's `loft.toml`: the named
/// profiles plus the designations mapping source selectors (file globs /
/// `fn:<name>`) to a profile.  The host owns this — a script cannot designate
/// itself (@PLN86 §1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxConfig {
    /// profile name -> profile
    pub profiles: HashMap<String, SandboxProfile>,
    /// (selector, profile-name) — which sources compile under which profile.
    /// A selector is a file glob (`"mods/**/*.loft"`) or `"fn:<name>"`.
    pub designations: Vec<(String, String)>,
}

impl SandboxConfig {
    /// True when this config designates any sandboxed source at all.
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.designations.is_empty()
    }

    /// The profile a selector maps to, if designated.
    #[must_use]
    pub fn profile_for_selector(&self, selector: &str) -> Option<&SandboxProfile> {
        self.designations
            .iter()
            .find(|(s, _)| s == selector)
            .and_then(|(_, p)| self.profiles.get(p))
    }

    /// The profile NAME a `fn:<name>` designation maps to, if the function is
    /// designated sandboxed.  The compiler resolves each parsed function against
    /// this (@PLN86 step 1.2).
    #[must_use]
    pub fn fn_designation(&self, fn_name: &str) -> Option<&str> {
        let selector = format!("fn:{fn_name}");
        self.designations
            .iter()
            .find(|(s, _)| *s == selector)
            .map(|(_, p)| p.as_str())
    }

    /// The profile NAME designating a function, by either selector form: the
    /// per-function `fn:<name>`, or a **path selector** matching the source file
    /// the function is defined in (`"src/oplog.loft"`, `"plugins/**/*.loft"`).
    ///
    /// A path selector is what makes the correct policy for plugin-shaped code a
    /// single line (#631).  Admission analyses only the functions it is told about,
    /// so a module's private helpers must be designated too — naming each one is
    /// tedious enough that the tempting alternative is to allow-list the whole
    /// library, which is the one policy that silently switches the guards off.
    /// Designating the FILE covers every helper in it and keeps them analysed.
    #[must_use]
    pub fn designation_for(&self, fn_name: &str, file: &str) -> Option<&str> {
        let selector = format!("fn:{fn_name}");
        self.designations
            .iter()
            .find(|(s, _)| {
                *s == selector || (!s.starts_with("fn:") && path_selector_matches(s, file))
            })
            .map(|(_, p)| p.as_str())
    }
}

/// Does path selector `pat` designate source file `file`?
///
/// `*` matches within one path segment, `**` across segments, `?` one character.
/// The selector is written relative to the project (`src/oplog.loft`) while the
/// parser carries whatever path it opened the file by, so a match against any
/// trailing segment boundary of `file` counts — anchoring on the full path would
/// make an otherwise-correct selector silently designate nothing, and a sandbox
/// policy that silently covers nothing is the failure mode this whole area exists
/// to prevent.
#[must_use]
pub fn path_selector_matches(pat: &str, file: &str) -> bool {
    let file = crate::portable_path::portable_str(file);
    if glob_matches(pat.as_bytes(), file.as_bytes()) {
        return true;
    }
    file.match_indices('/')
        .any(|(i, _)| glob_matches(pat.as_bytes(), &file.as_bytes()[i + 1..]))
}

/// A path selector a diagnostic can suggest for `file`: its last two segments
/// (`src/oplog.loft`), which is how the policy would be written.  The parser
/// resolves sources to absolute paths, and echoing one back would read as a
/// machine-specific policy nobody can commit.
#[must_use]
pub fn suggested_path_selector(file: &str) -> String {
    let file = crate::portable_path::portable_str(file);
    let mut segs = file.rsplit('/');
    match (segs.next(), segs.next()) {
        (Some(base), Some(dir)) if !dir.is_empty() => format!("{dir}/{base}"),
        (Some(base), _) => base.to_string(),
        _ => file,
    }
}

/// Backtracking glob match of `pat` against `text` (`**` spans `/`, `*` does not).
fn glob_matches(pat: &[u8], text: &[u8]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some(b'*') => {
            if pat.starts_with(b"**") {
                let rest = &pat[2..];
                // `**/` also matches zero segments, so `a/**/b.loft` covers `a/b.loft`.
                let skip = usize::from(rest.first() == Some(&b'/'));
                return (0..=text.len()).any(|i| glob_matches(&pat[2..], &text[i..]))
                    || (skip == 1 && glob_matches(&rest[1..], text));
            }
            let stop = text.iter().position(|&c| c == b'/').unwrap_or(text.len());
            (0..=stop).any(|i| glob_matches(&pat[1..], &text[i..]))
        }
        Some(b'?') => !text.is_empty() && text[0] != b'/' && glob_matches(&pat[1..], &text[1..]),
        Some(&c) => text.first() == Some(&c) && glob_matches(&pat[1..], &text[1..]),
    }
}

/// Parse the `[sandbox]` + `[profile.*]` sections of a `loft.toml` body into a
/// [`SandboxConfig`].  Mirrors `manifest::read_manifest`'s hand-rolled line
/// reader (loft.toml is not full TOML); unknown sections/keys are ignored so the
/// sandbox config coexists with the package manifest in one file.
///
/// - `[sandbox]` — `profile-name = ["selector", …]` adds a designation per
///   selector.
/// - `[profile.<name>]` — `allow_libs = ["l", …]`, `allow = ["g#right", …]`,
///   `native_ffi = false`.  (Interpret-only is unconditional, not a key here.)
#[must_use]
pub fn parse_sandbox_config(content: &str) -> SandboxConfig {
    let mut config = SandboxConfig::default();
    let mut section = String::new();
    for line in content.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_string();
            // A named profile section pre-registers the profile so an empty
            // `[profile.x]` (deny-all) still exists to be designated.
            if let Some(p) = section.strip_prefix("profile.") {
                config.profiles.entry(p.to_string()).or_default();
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if section == "sandbox" {
            // `profile = ["selector", …]` — each selector designates that profile.
            for selector in parse_str_list(value) {
                config.designations.push((selector, key.to_string()));
            }
        } else if let Some(name) = section.strip_prefix("profile.") {
            let profile = config.profiles.entry(name.to_string()).or_default();
            match key {
                "allow_libs" => profile.allow_libs = parse_str_list(value),
                "allow" => profile.allow = parse_str_list(value),
                // `backend` is accepted-and-ignored: interpret-only is unconditional
                // for sandboxed code, not a per-profile setting (see SandboxProfile).
                "native_ffi" => profile.native_ffi = value.trim() == "true",
                // @PLN86 §8 (P7.3) — data-envelope bounds; a bare integer, `0`/absent
                // = unset.  An underscore-or-comma separator in the figure is tolerated.
                "max_input_n" => profile.max_input_n = parse_u64(value),
                "max_depth" => profile.max_depth = parse_u64(value),
                "max_string_len" => profile.max_string_len = parse_u64(value),
                "data_budget" => profile.data_budget = parse_u64(value),
                _ => {}
            }
        }
    }
    config
}

/// Drop a trailing `# comment` (loft.toml line comments).  Quote-aware: a `#`
/// INSIDE a quoted string is kept, because the `group#right` capability tokens
/// (`"fs#read"`) carry a literal `#` — only a `#` outside quotes starts a comment.
fn strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Parse a value that is either a TOML array `["a", "b"]` or a bare/quoted
/// scalar into a trimmed, unquoted, non-empty list.
/// @PLN86 §8 — parse a data-envelope figure: a bare unsigned integer, tolerating
/// `_`/`,` digit groupers and surrounding quotes (`"1_000_000"`).  Unparseable / empty
/// → `0` (treated as unset).
fn parse_u64(value: &str) -> u64 {
    value
        .trim()
        .trim_matches('"')
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

fn parse_str_list(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(value);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// The **sandbox-reachable set**: the transitive closure of references (calls + fn-ref
/// literals) from every sandboxed def, **descending only into other sandboxed defs**.
///
/// Enforces @FR-Cap-Trusted — a trusted / allow-listed symbol is a *leaf*: recorded, so the
/// admission walk still capability-checks the reference to it, but never descended into,
/// because its body is the host's contract (§4 / L-host) rather than sandboxed code.
/// Descending would ask the host's own implementation to satisfy the script's profile.  `sandboxed` maps each
/// sandboxed def_nr to its profile (the parser's `def_sandbox`).
///
/// Returns every reachable def_nr: the sandboxed entries, the sandboxed defs
/// descended into, and the trusted leaves they reference.  Step 2.3 capability-checks
/// **only the non-sandboxed members** — which is @FR-Cap-Own: what a script does to data
/// it OWNS is free, and only reaching into the host is checked.  So the sandboxed /
/// non-sandboxed split this function computes is exactly the Cap-Own boundary.
#[must_use]
pub fn reachable_set(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> HashSet<u32> {
    let mut reachable = HashSet::new();
    let mut worklist: Vec<u32> = sandboxed.keys().copied().collect();
    let mut descended = HashSet::new();
    while let Some(d) = worklist.pop() {
        if !descended.insert(d) {
            continue;
        }
        reachable.insert(d);
        let mut refs = HashSet::new();
        referenced_defs(data, d, &mut refs);
        add_recorded_fn_refs(fn_refs, d, &mut refs);
        for r in refs {
            reachable.insert(r);
            // Descend ONLY into sandboxed defs; trusted symbols are leaves.
            if sandboxed.contains_key(&r) {
                worklist.push(r);
            }
        }
    }
    reachable
}

/// @PLN86 L4 — fold `f`'s parse-recorded fn-ref targets into its reference set.
/// These are the functions `f` named as a VALUE (caught at the creation site), so
/// an indirect call through a fn-ref returned / stored in a field / put in a
/// collection cannot escape the admission walk.
fn add_recorded_fn_refs(fn_refs: &HashMap<u32, HashSet<u32>>, f: u32, out: &mut HashSet<u32>) {
    if let Some(targets) = fn_refs.get(&f) {
        out.extend(targets.iter().copied());
    }
}

/// Collect every def_nr referenced in `fn_dnr`'s body — both direct calls AND
/// fn-ref literals.  L4 hazard (verified in the IR): a non-capturing fn-ref is
/// emitted as a bare `Value::Int(def_nr)`, indistinguishable from an integer
/// literal except by **type context** — so an `Int`/`Long` is a reference only
/// where a `Function` type is expected: a `fn(...)`-typed call argument or an
/// assignment to a `fn(...)`-typed variable.  Missing this lets
/// `f = read_file; f(x)` escape admission.
///
/// RESIDUAL (tracked, closes before step 2.3 capability admission relies on this
/// set): a fn-ref flowing through a `Function`-typed RETURN value, struct field,
/// or collection element is not yet detected.  Until then the design's host
/// contract (§4 / L-host: trusted APIs do not hand untrusted code arbitrary
/// fn-refs) covers the trusted-boundary case; the sandboxed-internal vectors are
/// the follow-up.
fn referenced_defs(data: &Data, fn_dnr: u32, out: &mut HashSet<u32>) {
    let def = data.def(fn_dnr);
    let func = &def.variables;
    def.code.walk(&mut |v| match v {
        Value::Call(d, args) => {
            out.insert(*d);
            // A `fn(...)`-typed parameter receives its argument as a fn-ref.
            let callee = &data.def(*d).variables;
            let params = callee.arguments();
            for (i, arg) in args.iter().enumerate() {
                if let Some(slot) = params.get(i)
                    && matches!(callee.tp(*slot), Type::Function(..))
                    && let Some(r) = fnref_target(arg)
                {
                    out.insert(r);
                }
            }
        }
        Value::Set(slot, value) => {
            // Assigning a fn-ref into a `fn(...)`-typed local.
            if matches!(func.tp(*slot), Type::Function(..))
                && let Some(r) = fnref_target(value)
            {
                out.insert(r);
            }
        }
        Value::FnRef(d, _, _) if *d >= 0 => {
            out.insert(*d as u32);
        }
        _ => {}
    });
}

/// Extract the referenced function def_nr from a value used in a `Function`-typed
/// position — a fn-ref is carried as `FnRef`, a bare `Int`, or a `Long` def_nr.
fn fnref_target(v: &Value) -> Option<u32> {
    match v.unspan() {
        Value::Int(d) | Value::FnRef(d, _, _) if *d >= 0 => Some(*d as u32),
        Value::Long(d) if *d >= 0 => Some(*d as u32),
        _ => None,
    }
}

/// @PLN86 step 1.4 — the external `[native]` cdylib bridges a sandboxed program
/// reaches: defs in `reachable` whose `#native` symbol is owned by an external
/// native package (`native_symbol_crates`), NOT a built-in interpreter
/// primitive.  Built-in stdlib functions use inline `#rust` bodies — their
/// `native()` is empty — and built-in `#native` ops live in `default/`, not under
/// a `native_packages` dir, so neither enters the crate map; only an external
/// cdylib bridge does.  Generating Rust + dlopen'ing a cdylib is RCE by
/// construction, so unless a profile sets `native_ffi = true` a non-empty result
/// is an admission rejection (consumed by the §4 walk).  Sorted for stable
/// diagnostics.
#[must_use]
pub fn reachable_ffi_bridges(data: &Data, reachable: &HashSet<u32>) -> Vec<u32> {
    let mut bridges: Vec<u32> = reachable
        .iter()
        .copied()
        .filter(|d| {
            let sym = data.def(*d).native();
            !sym.is_empty() && data.native_symbol_crates.contains_key(sym)
        })
        .collect();
    bridges.sort_unstable();
    bridges
}

/// @PLN86 step 2.3 — a capability-admission violation.  `from` is the sandboxed
/// def that makes the reference, `symbol` the trusted def it reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapViolation {
    /// `symbol`'s `#cap` link is not granted by any `allow` entry.
    UngrantedCap {
        from: u32,
        symbol: u32,
        group: String,
    },
    /// `symbol` carries no `#cap` group — deny-by-default: an unclassified symbol
    /// cannot be proven safe.  Until 2.2 tags the surface this fires for every
    /// untagged stdlib symbol, which is correct (nothing is admitted yet).
    UntaggedSymbol { from: u32, symbol: u32 },
    /// `symbol` is an external native cdylib bridge and the profile does not set
    /// `native_ffi` (dlopen is RCE by construction).
    ExternalFfi {
        from: u32,
        symbol: u32,
        crate_name: String,
    },
    /// @PLN24 — `symbol` is a `#c` binding and the profile does not set
    /// `native_ffi`.  Same gate as [`Self::ExternalFfi`] and for a stronger
    /// reason: a `#c` call enters a C library with no marshalling layer at all.
    CBinding {
        from: u32,
        symbol: u32,
        c_symbol: String,
    },
}

/// @PLN86 — the library a def belongs to, the wholesale-admission key for
/// `allow_libs`.  Derived from the def's source file: the basename minus a
/// leading `NN_` number prefix and the extension.  Stdlib modules →
/// `code`/`files`/`text`/`json`; a single-file external lib → its package name
/// (`crypto`).  `None` for a synthetic / empty source.
#[must_use]
pub fn def_library(data: &Data, def_nr: u32) -> Option<String> {
    let def = data.def(def_nr);
    let file = &*def.position().file;
    let base = std::path::Path::new(file).file_stem()?.to_str()?;
    // Strip a leading `\d+_` (stdlib module naming: `01_code` -> `code`).
    let name = match base.find('_') {
        Some(i) if i > 0 && base.as_bytes()[..i].iter().all(u8::is_ascii_digit) => &base[i + 1..],
        _ => base,
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// @PLN86 step 2.3 — the capability-admission walk, **library-first**.  Each
/// sandboxed def is admitted under ITS OWN profile; every trusted symbol it
/// references (a non-sandboxed leaf, §4) is admitted if **either**:
///   1. its **library** is allow-listed wholesale (`allow_libs`) — a complete,
///      host-vetted library is included as a unit, NO `#cap` tag needed; or
///   2. its `#cap` **link** is granted (`allow`) — the fine-grained
///      layer, for carving a library in half ("include `files`, reads only").
///
/// A symbol in no allowed library and with no allowed cap is the violation.  A
/// reference to another sandboxed def is fine (admitted under its own profile),
/// so the union of per-def checks covers the whole reachable closure.  This is
/// L4-complete: `referenced_defs` resolves indirect fn-refs (`f = read_file`) to
/// their target, so an indirect call cannot escape.
///
/// Returns every violation, deterministically ordered, for diagnostics (2.5).
#[must_use]
pub fn admit_capabilities(
    data: &Data,
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<CapViolation> {
    let mut violations = Vec::new();
    let mut entries: Vec<u32> = sandboxed.keys().copied().collect();
    entries.sort_unstable();
    for from in entries {
        let profile = config.profiles.get(&sandboxed[&from]);
        let mut refs = HashSet::new();
        referenced_defs(data, from, &mut refs);
        add_recorded_fn_refs(fn_refs, from, &mut refs);
        let mut refs: Vec<u32> = refs.into_iter().collect();
        refs.sort_unstable();
        for symbol in refs {
            // Another sandboxed def: admitted under its own profile, not a leaf.
            if sandboxed.contains_key(&symbol) {
                continue;
            }
            // LIBRARY-FIRST: a wholesale-allowed library admits every symbol in
            // it — untagged functions AND native bridges — the host vetted it.
            if let Some(lib) = def_library(data, symbol)
                && profile.is_some_and(|p| p.allows_lib(&lib))
            {
                continue;
            }
            let def = data.def(symbol);
            // Not in an allowed library — apply the fine-grained gates.
            // 1.4: an external FFI bridge is gated by `native_ffi`, not caps.
            let native = def.native();
            if !native.is_empty()
                && let Some(crate_name) = data.native_symbol_crates.get(native)
            {
                if !profile.is_some_and(|p| p.native_ffi) {
                    violations.push(CapViolation::ExternalFfi {
                        from,
                        symbol,
                        crate_name: crate_name.clone(),
                    });
                }
                continue;
            }
            // @PLN24 — a `#c` binding is FFI too, and the check above cannot see
            // it: `reachable_ffi_bridges` and the branch above both key on
            // `def.native()`, which arc A leaves EMPTY on a `#c` definition on
            // purpose, so the Rust dispatch path cannot claim one.  Measured
            // before it was closed: a sandboxed script reaching a `#c` binding
            // tagged `db#read`, under a profile granting `db#read` and leaving
            // `native_ffi` at its default false, ran arbitrary code out of a
            // declared `.so`.
            //
            // A capability grant says what DATA a script may touch.  It cannot
            // say "and arbitrary machine code may run in this process", which is
            // exactly the line `native_ffi` draws — so a `#c` binding is gated
            // there, never by its `#cap` tag.  (An allow-listed library still
            // admits it, unchanged: that is the host vetting the library as a
            // unit, the same answer `#native` bridges already get.)
            if !def.c_sig.is_empty() {
                if !profile.is_some_and(|p| p.native_ffi) {
                    violations.push(CapViolation::CBinding {
                        from,
                        symbol,
                        c_symbol: def.c_symbol.clone(),
                    });
                }
                continue;
            }
            // 2.3: capability check (deny-by-default).
            let cap = def.cap();
            if cap.is_empty() {
                violations.push(CapViolation::UntaggedSymbol { from, symbol });
            } else if !profile.is_some_and(|p| p.allows(cap)) {
                violations.push(CapViolation::UngrantedCap {
                    from,
                    symbol,
                    group: cap.to_string(),
                });
            }
        }
    }
    violations
}

/// @PLN86 step 2.2 — the capability-coverage lint (L3-cap): every public function
/// the host exposes must carry a `group#right` call-gate link, or admission can only deny-by-
/// default reject it.  Returns the public, non-synthetic function defs that lack
/// a group, sorted — the work-list for tagging the stdlib / library surface.  The
/// END state is an empty result; until the surface is tagged this lists it.
#[must_use]
pub fn untagged_public_symbols(data: &Data) -> Vec<u32> {
    let mut out: Vec<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            def.pub_visible
                && matches!(def.def_type(), DefType::Function)
                && def.synthetic().is_none()
                && def.cap().is_empty()
        })
        .collect();
    out.sort_unstable();
    out
}

impl CapViolation {
    /// The sandboxed def that makes the offending reference.
    #[must_use]
    pub fn from(&self) -> u32 {
        match self {
            Self::UngrantedCap { from, .. }
            | Self::UntaggedSymbol { from, .. }
            | Self::ExternalFfi { from, .. }
            | Self::CBinding { from, .. } => *from,
        }
    }

    /// The trusted def it reaches.
    #[must_use]
    pub fn symbol(&self) -> u32 {
        match self {
            Self::UngrantedCap { symbol, .. }
            | Self::UntaggedSymbol { symbol, .. }
            | Self::ExternalFfi { symbol, .. }
            | Self::CBinding { symbol, .. } => *symbol,
        }
    }
}

/// A def's human-readable name for diagnostics — the stored `n_<fn>` global
/// prefix stripped; method names (`t_<len><Type>_<m>`) are left as-is.
fn display_name(data: &Data, def_nr: u32) -> String {
    let n = data.def(def_nr).name();
    n.strip_prefix("n_").unwrap_or(n).to_string()
}

/// @PLN86 step 2.5 — the source position of the FIRST reference to `symbol` in
/// `from`'s body, for pointing the diagnostic at the exact construct.  Tracks the
/// nearest enclosing `Span` as it descends.  `None` (caller falls back to the
/// function's own position) when the reference is a bare `Int` fn-ref with no
/// span — rare, and the function position is still actionable.
#[must_use]
pub fn reference_position(data: &Data, from: u32, symbol: u32) -> Option<Position> {
    fn scan(v: &Value, target: u32, cur: Option<&Position>, found: &mut Option<Position>) {
        if found.is_some() {
            return;
        }
        if let Value::Span(b) = v {
            scan(&b.1, target, Some(&b.0), found);
            return;
        }
        match v {
            Value::Call(d, _) if *d == target => {
                *found = cur.cloned();
                return;
            }
            Value::FnRef(d, _, _) if *d >= 0 && *d as u32 == target => {
                *found = cur.cloned();
                return;
            }
            _ => {}
        }
        v.for_each_child(&mut |c| scan(c, target, cur, found));
    }
    let mut found = None;
    scan(&data.def(from).code, symbol, None, &mut found);
    found
}

/// @PLN86 step 2.5 — turn a `CapViolation` into a correct, specific, actionable
/// admission error (§6): it names the construct's position, the symbol, the rule
/// it breaks, the profile's allowed set, AND the fix — *before* the script runs.
/// Under library-first every fix offers BOTH the wholesale option (allow the
/// library) and the fine-grained one (the cap / `native_ffi`).
#[must_use]
pub fn describe_violation(
    data: &Data,
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    v: &CapViolation,
) -> String {
    let from = v.from();
    let symbol = v.symbol();
    let from_name = display_name(data, from);
    let sym_name = display_name(data, symbol);
    let lib = def_library(data, symbol);
    let libhint = lib.as_deref().unwrap_or("<unknown>");
    let pos =
        reference_position(data, from, symbol).unwrap_or_else(|| data.def(from).position().clone());
    let profile_name = sandboxed.get(&from);
    let profile = profile_name.and_then(|n| config.profiles.get(n));
    let allowed_libs = profile.map(|p| p.allow_libs.join(", ")).unwrap_or_default();
    let allowed_caps = profile.map(|p| p.allow.join(", ")).unwrap_or_default();
    // loft#1042 — a designation may name a profile that has no `[profile.<name>]`
    // section at all.  Reporting its grants then prints `[]`, which says *nothing is
    // granted here* when the truth is *that profile does not exist* — and the two ask
    // for opposite next moves.  The parser already tells them apart: it pre-registers
    // an empty profile whenever the SECTION is present, so a deliberate deny-all is
    // `Some(empty)` and a typo is `None`.
    let undefined_profile = match (profile_name, profile) {
        (Some(name), None) => Some(name.as_str()),
        _ => None,
    };
    // What a refusal says about the policy's reach: either which grants exist, or that
    // the profile holding them does not.
    let grants = |with_libs: bool| -> String {
        match undefined_profile {
            Some(name) => format!(
                "profile `{name}` is designated but never DEFINED — the grant lists are \
                 empty because the section is missing, not because it grants nothing.\n  \
                 fix: add a `[profile.{name}]` section and put `allow_libs` / `allow` \
                 there — under `[sandbox]` they are ignored, since it reads every key as \
                 `<profile> = [selectors]`."
            ),
            None if with_libs => {
                format!(
                    "allowed libraries: [{allowed_libs}]; allowed capabilities: [{allowed_caps}]"
                )
            }
            None => format!("allowed capabilities: [{allowed_caps}]"),
        }
    };
    match v {
        CapViolation::UngrantedCap { group, .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}`, which needs capability \
             `{group}` — not granted.\n  fix: add `{group}` to `allow`, or add its \
             library `{libhint}` to `allow_libs`.\n  {}",
            grants(false)
        ),
        // The fix ORDER matters here (#631).  When the unreachable symbol is the
        // sandboxed code's OWN private helper, leading with `allow_libs` teaches the
        // one policy that must never be written: allow-listing the script's own
        // library marks every helper in it host-vetted, which silently switches OFF
        // the raw-write guard and drops the helper's loops from the data envelope.
        // `self_allow_list_violations` now rejects that policy outright, so the
        // diagnostic must lead with the fix that works — designate the helper.
        CapViolation::UntaggedSymbol { .. } if lib.is_some() && lib == def_library(data, from) => {
            format!(
                "{pos}: sandboxed `{from_name}` reaches `{sym_name}`, a helper in its OWN \
                 library `{libhint}`, which is not designated sandboxed.\n  fix: designate \
                 it too — add `\"fn:{sym_name}\"` to the `[sandbox]` selector list, or \
                 designate the whole file with a path selector (`\"{}\"`).\n  note: do NOT \
                 add `{libhint}` to `allow_libs` — a library holding sandboxed code cannot \
                 vet itself, and allow-listing it would disable the raw-write guard and \
                 under-report the data envelope for every helper in it.",
                suggested_path_selector(&data.def(symbol).position().file)
            )
        }
        CapViolation::UntaggedSymbol { .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}` (library `{libhint}`), which \
             is neither an allowed library nor a granted capability.\n  fix: add `{libhint}` \
             to `allow_libs` to include the whole library, or tag `{sym_name}` with a `#cap` \
             group and grant it.\n  {}",
            grants(true)
        ),
        CapViolation::ExternalFfi { crate_name, .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}`, a native FFI bridge from \
             crate `{crate_name}` — native code is not permitted (`native_ffi = false`).\n  \
             fix: add its library `{libhint}` to `allow_libs` (you vet its native code), or \
             set `native_ffi = true` for this profile."
        ),
        CapViolation::CBinding { c_symbol, .. } => format!(
            "{pos}: sandboxed `{from_name}` reaches `{sym_name}`, a `#c` binding to the C \
             symbol `{c_symbol}` — native code is not permitted (`native_ffi = false`).\n  \
             fix: add its library `{libhint}` to `allow_libs` (you vet the C library it \
             binds), or set `native_ffi = true` for this profile.\n  note: a `#cap` grant \
             cannot admit this — a capability says what data the script may touch, and a C \
             call runs machine code loft cannot inspect."
        ),
    }
}

/// @PLN86 / #631 — a profile that allow-lists a library which itself holds
/// sandboxed code.  `allow_libs` means *"the host vetted this library, include it
/// as a unit"*; naming the script's own library asserts that the script vets
/// itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelfAllowListViolation {
    pub profile: String,
    pub library: String,
    /// A sandboxed def in that library, for the position the diagnostic points at.
    pub def: u32,
}

/// @PLN86 / #631 — every profile that allow-lists a library holding one of its
/// OWN sandboxed defs.
///
/// This is the policy that silently turns admission off.  `allow_libs` makes each
/// symbol in the named library a **trusted leaf**: not descended into, not
/// capability-checked, its loops absent from the data envelope, its raw writes
/// unattributed.  That is exactly right for a host library and exactly wrong for
/// the sandboxed module itself, where it means an escaping mutation admits as soon
/// as it moves into a one-line helper (`push(out, i)` passes where the identical
/// `out.v += [i]` is rejected), and a plugin whose entry point delegates to helpers
/// reports `O(1)` while being `O(n)`.
///
/// Both effects are invisible in the output — the reason this is a hard rejection
/// rather than a lint.  A helper belonging to the sandboxed unit is designated, not
/// allow-listed; [`describe_violation`] points at that fix.
#[must_use]
pub fn self_allow_list_violations(
    data: &Data,
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
) -> Vec<SelfAllowListViolation> {
    let mut out: Vec<SelfAllowListViolation> = Vec::new();
    for (&def, profile_name) in sandboxed {
        let Some(library) = def_library(data, def) else {
            continue;
        };
        let Some(profile) = config.profiles.get(profile_name) else {
            continue;
        };
        if !profile.allows_lib(&library) {
            continue;
        }
        // One report per (profile, library) — the lowest def_nr, for determinism.
        if let Some(seen) = out
            .iter_mut()
            .find(|v| v.profile == *profile_name && v.library == library)
        {
            seen.def = seen.def.min(def);
        } else {
            out.push(SelfAllowListViolation {
                profile: profile_name.clone(),
                library,
                def,
            });
        }
    }
    out.sort_unstable();
    out
}

/// @PLN86 / #631 — render a self-allow-list violation as an actionable error.
#[must_use]
pub fn describe_self_allow_list_violation(data: &Data, v: &SelfAllowListViolation) -> String {
    let SelfAllowListViolation {
        profile, library, ..
    } = v;
    let name = display_name(data, v.def);
    format!(
        "{}: profile `{profile}` allow-lists `{library}`, the library its own sandboxed \
         `{name}` lives in — a library holding sandboxed code cannot vet itself.\n  This \
         would mark every function in `{library}` a trusted leaf, silently disabling the \
         raw-write guard for them and dropping their loops from the data envelope.\n  fix: \
         remove `{library}` from `allow_libs`, and designate the helpers it was covering \
         (add each `\"fn:<name>\"`, or a path selector for the whole file) to the \
         `[sandbox]` list.  Keep `allow_libs` for host libraries only.",
        data.def(v.def).position()
    )
}

/// @PLN86 step P3 — a totality violation: a sandboxed script that cannot be
/// proven to terminate.  An admitted script always runs to completion, so these
/// are rejected at LOAD (never a runtime hang).
#[derive(Debug, Clone, PartialEq)]
pub enum TotalityViolation {
    /// An unbounded `while` loop in `def` (recorded at parse).  v1 admits only
    /// `for` over a finite collection / range; a `while` has no proven bound.
    UnboundedLoop { def: u32, position: Position },
    /// A cycle in the sandboxed call graph — recursion.  v1 is acyclic-only; the
    /// `cycle` lists the sandboxed defs forming the back-edge.
    Recursion { cycle: Vec<u32> },
    /// `def` reaches a **partial op** that faults the script (3.3): `assert` /
    /// `panic` / `log_fatal` abort, which would break the host.  The arithmetic
    /// ops are NOT here — the interpreter makes them total (div/mod-by-zero →
    /// null, OOB → null, overflow wraps), and the sandbox is interpret-only, so
    /// they never fault.  Only the explicit-abort ops are excluded.
    PartialOp {
        def: u32,
        op: u32,
        position: Option<Position>,
    },
}

/// @PLN86 step 3.3 — the explicit-abort ops excluded from sandboxed code: they
/// terminate the script (a fault the host must never see), and unlike the
/// arithmetic ops cannot be given a total result.  By stored name so the check
/// is a set membership over the reachable refs.
pub(crate) const ABORT_OPS: &[&str] = &["n_assert", "n_panic", "n_log_fatal"];

/// The sandboxed defs that `from` calls — the edges of the sandboxed call graph
/// (trusted callees are not followed; they are total by the host contract).
fn sandboxed_calls(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
    from: u32,
) -> Vec<u32> {
    let mut refs = HashSet::new();
    referenced_defs(data, from, &mut refs);
    add_recorded_fn_refs(fn_refs, from, &mut refs); // L4 — indirect calls are edges too
    let mut calls: Vec<u32> = refs
        .into_iter()
        .filter(|r| sandboxed.contains_key(r))
        .collect();
    calls.sort_unstable();
    calls
}

/// @PLN86 step 3.2 — every cycle in the sandboxed call graph (recursion).  A
/// standard colour DFS: a back-edge to a node still on the current path is a
/// cycle; the path slice from that node is the cycle.  Deduped by node-set so a
/// cycle is reported once.  Self-recursion (`f` calls `f`) is a length-1 cycle.
#[must_use]
pub fn recursion_cycles(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<Vec<u32>> {
    #[allow(clippy::too_many_arguments)]
    fn dfs(
        n: u32,
        data: &Data,
        sandboxed: &HashMap<u32, String>,
        fn_refs: &HashMap<u32, HashSet<u32>>,
        on_path: &mut HashSet<u32>,
        done: &mut HashSet<u32>,
        path: &mut Vec<u32>,
        cycles: &mut Vec<Vec<u32>>,
        seen: &mut HashSet<Vec<u32>>,
    ) {
        if on_path.contains(&n) {
            if let Some(start) = path.iter().position(|&x| x == n) {
                let cycle = path[start..].to_vec();
                let mut key = cycle.clone();
                key.sort_unstable();
                if seen.insert(key) {
                    cycles.push(cycle);
                }
            }
            return;
        }
        if !done.insert(n) {
            return;
        }
        on_path.insert(n);
        path.push(n);
        for callee in sandboxed_calls(data, sandboxed, fn_refs, n) {
            dfs(
                callee, data, sandboxed, fn_refs, on_path, done, path, cycles, seen,
            );
        }
        path.pop();
        on_path.remove(&n);
    }

    let mut cycles = Vec::new();
    let (mut on_path, mut done, mut path, mut seen) =
        (HashSet::new(), HashSet::new(), Vec::new(), HashSet::new());
    let mut starts: Vec<u32> = sandboxed.keys().copied().collect();
    starts.sort_unstable();
    for n in starts {
        dfs(
            n,
            data,
            sandboxed,
            fn_refs,
            &mut on_path,
            &mut done,
            &mut path,
            &mut cycles,
            &mut seen,
        );
    }
    cycles
}

/// @PLN86 3.1 — is `while cond { body }` PROVABLY terminating?  SOUND and
/// conservative: true ONLY when a decreasing integer variant is found — an integer
/// counter compared in `cond` against a STABLE bound, stepped monotonically toward
/// it by a positive integer CONSTANT on EVERY iteration (an unconditional
/// top-level `c = c + k` / `c = c - k`) and modified no other way.  Flag loops,
/// non-integer / field variants, conditional or non-constant steps, and
/// call/expression bounds all return false → rejected.  Unsoundness here is a HANG
/// (the host's whole point is that it can't be hung), so when in any doubt: false.
///
/// The IR contract (verified): `&&` parses as `if A { B } else false`; `>` / `>=`
/// normalise to a SWAPPED `OpLtInt` / `OpLeInt` (counter on the right); `+`/`-` on
/// integers are `OpAddInt` / `OpMinInt`.
#[must_use]
pub fn while_is_bounded(data: &Data, cond: &Value, body: &Value) -> bool {
    let mut candidates: Vec<(u16, bool)> = Vec::new();
    collect_variant_candidates(data, cond, body, &mut candidates);
    candidates
        .iter()
        .any(|&(counter, increasing)| counter_progresses(data, body, counter, increasing))
}

/// Collect `(counter, increasing)` candidates from `cond`: `counter < bound` /
/// `counter <= bound` (increasing) or `bound < counter` / `bound <= counter`
/// (decreasing) with a stable bound, plus BOTH conjuncts of an `&&` (parsed as
/// `if A { B } else false`) — either can bound the loop.  `||` (`if A { true }
/// else B`, else ≠ false) yields nothing: neither side alone bounds it.
fn collect_variant_candidates(data: &Data, cond: &Value, body: &Value, out: &mut Vec<(u16, bool)>) {
    match cond.unspan() {
        Value::If(a, b, els) if matches!(els.unspan(), Value::Boolean(false)) => {
            collect_variant_candidates(data, a, body, out);
            collect_variant_candidates(data, b, body, out);
        }
        Value::Call(op, args) if args.len() == 2 => {
            let name = data.def(*op).name();
            if name == "OpLtInt" || name == "OpLeInt" {
                if let Value::Var(c) = args[0].unspan()
                    && is_stable_bound(body, &args[1], *c)
                {
                    out.push((*c, true)); // counter < bound → counts up
                }
                if let Value::Var(c) = args[1].unspan()
                    && is_stable_bound(body, &args[0], *c)
                {
                    out.push((*c, false)); // bound < counter → counts down
                }
            }
        }
        _ => {}
    }
}

/// A bound is stable iff it's an integer literal or a variable (not the counter)
/// never assigned in the body — so it cannot move away from the counter.  A call
/// or expression bound is conservatively unstable (hoist it to a local first).
fn is_stable_bound(body: &Value, bound: &Value, counter: u16) -> bool {
    match bound.unspan() {
        Value::Int(_) | Value::Long(_) => true,
        Value::Var(b) => *b != counter && !contains_assignment_to(body, *b),
        _ => false,
    }
}

/// Every assignment to `counter` in `body` must be a TOP-LEVEL (unconditional),
/// monotonic, constant step in the right direction; at least one must exist.  A
/// nested (conditional) or wrong-direction or non-constant assignment fails it.
fn counter_progresses(data: &Data, body: &Value, counter: u16, increasing: bool) -> bool {
    let ops: &[Value] = match body.unspan() {
        Value::Block(b) => &b.operators,
        other => std::slice::from_ref(other),
    };
    let mut stepped = false;
    for stmt in ops {
        if let Value::Set(v, rhs) = stmt.unspan()
            && *v == counter
        {
            if is_monotonic_step(data, rhs, counter, increasing) {
                stepped = true;
            } else {
                return false; // a top-level non-step assignment to the counter
            }
            continue;
        }
        // any other top-level statement must not touch the counter at all (a
        // conditional / nested step can't be proven to run every iteration).
        if contains_assignment_to(stmt, counter) {
            return false;
        }
    }
    stepped
}

/// `rhs` is `counter + k` (increasing) or `counter - k` (decreasing) with `k` a
/// positive integer constant.
fn is_monotonic_step(data: &Data, rhs: &Value, counter: u16, increasing: bool) -> bool {
    let Value::Call(op, args) = rhs.unspan() else {
        return false;
    };
    if args.len() != 2 {
        return false;
    }
    let pos_int = |v: &Value| matches!(v.unspan(), Value::Int(k) if *k > 0);
    let is_counter = |v: &Value| matches!(v.unspan(), Value::Var(c) if *c == counter);
    match (increasing, data.def(*op).name()) {
        // c + k  or  k + c
        (true, "OpAddInt") => {
            (is_counter(&args[0]) && pos_int(&args[1]))
                || (pos_int(&args[0]) && is_counter(&args[1]))
        }
        // c - k  (subtraction is not commutative)
        (false, "OpMinInt") => is_counter(&args[0]) && pos_int(&args[1]),
        _ => false,
    }
}

/// Does any node in `value` assign to `var` (a `Set(var, _)`)?
fn contains_assignment_to(value: &Value, var: u16) -> bool {
    let mut found = false;
    value.walk(&mut |v| {
        if let Value::Set(v_nr, _) = v
            && *v_nr == var
        {
            found = true;
        }
    });
    found
}

/// @PLN86 step P3 — the totality admission walk: a sandboxed script is total iff
/// it has no unbounded loop (3.1) and no recursion cycle (3.2).  `unbounded_loops`
/// is the parser's `sandbox_unbounded_loops` (while-loops recorded at parse).
/// Returns every violation, deterministically ordered.
#[must_use]
pub fn admit_totality(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    unbounded_loops: &HashMap<u32, Position>,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<TotalityViolation> {
    let mut violations = Vec::new();
    // 3.1 — unbounded loops, ordered by def.
    let mut looped: Vec<u32> = unbounded_loops
        .keys()
        .copied()
        .filter(|d| sandboxed.contains_key(d))
        .collect();
    looped.sort_unstable();
    for def in looped {
        violations.push(TotalityViolation::UnboundedLoop {
            def,
            position: unbounded_loops[&def].clone(),
        });
    }
    // 3.2 — recursion cycles.
    for cycle in recursion_cycles(data, sandboxed, fn_refs) {
        violations.push(TotalityViolation::Recursion { cycle });
    }
    // 3.3 — reachable explicit-abort ops (assert / panic / log_fatal).  The
    // arithmetic ops are total on the interpreter, so they are NOT rejected; only
    // the ops that fault the script are excluded.
    let mut entries: Vec<u32> = sandboxed.keys().copied().collect();
    entries.sort_unstable();
    for def in entries {
        let mut refs = HashSet::new();
        referenced_defs(data, def, &mut refs);
        add_recorded_fn_refs(fn_refs, def, &mut refs); // L4 — fn-ref'd abort ops too
        let mut refs: Vec<u32> = refs.into_iter().collect();
        refs.sort_unstable();
        for op in refs {
            if !sandboxed.contains_key(&op) && ABORT_OPS.contains(&data.def(op).name()) {
                violations.push(TotalityViolation::PartialOp {
                    def,
                    op,
                    position: reference_position(data, def, op),
                });
            }
        }
    }
    violations
}

/// @PLN86 step 2.5 / P3 — render a totality violation as an actionable admission
/// error: the construct, the rule, and how to bound it.
#[must_use]
pub fn describe_totality_violation(data: &Data, v: &TotalityViolation) -> String {
    match v {
        TotalityViolation::UnboundedLoop { def, position } => {
            let name = display_name(data, *def);
            format!(
                "{position}: sandboxed `{name}` contains a `while` loop with no provable \
                 bound — a sandboxed script must be provably total.\n  fix: use a bounded \
                 `for x in <collection>` / `for i in 0..N` loop; or give the `while` a \
                 decreasing integer variant — an `int` counter compared against a constant or \
                 unchanging bound and stepped by a constant every iteration, e.g. \
                 `i = 0; while i < n {{ … i = i + 1 }}` (a guard `&& i < N` works too)."
            )
        }
        TotalityViolation::Recursion { cycle } => {
            let chain = cycle
                .iter()
                .map(|&d| display_name(data, d))
                .collect::<Vec<_>>()
                .join(" → ");
            let tail = display_name(data, cycle[0]);
            format!(
                "sandboxed recursion is not permitted (the script must be provably total): \
                 the call cycle `{chain} → {tail}` is unbounded.\n  fix: replace the \
                 recursion with a bounded `for` loop over the work to do.  Walking a tree? \
                 Use the stdlib's recursion-free traversal: satisfy `Walkable` \
                 (`fn children(self: Self) -> vector<Self>`) and iterate \
                 `for n in tree_walk(root, cap) {{ … }}` — level order, total by \
                 construction."
            )
        }
        TotalityViolation::PartialOp { def, op, position } => {
            let from = display_name(data, *def);
            let op_name = display_name(data, *op);
            let at = position
                .as_ref()
                .map_or_else(String::new, |p| format!("{p}: "));
            format!(
                "{at}sandboxed `{from}` calls `{op_name}`, which faults the script — a \
                 sandboxed script may never abort the host.\n  fix: handle the condition \
                 without `{op_name}` (e.g. `if !ok {{ return <default> }}`, or `a / b ?? 0` \
                 for a divide-by-zero — arithmetic faults are already total/null)."
            )
        }
    }
}

/// @PLN86 prevention #3 — a host capability that is NOT total: a `#cap`-tagged
/// function whose call tree (transitively) reaches an explicit-abort op (3.3's
/// [`ABORT_OPS`]), so a script-supplied value could fault the HOST through it.
#[derive(Debug, Clone, PartialEq)]
pub struct CapTotalityViolation {
    /// The `#cap`-tagged capability function.
    pub capability: u32,
    /// The explicit-abort op it can reach.
    pub op: u32,
    /// Where `capability` references `op` directly (None when the op is reached
    /// only transitively, through a helper).
    pub position: Option<Position>,
}

/// @PLN86 prevention #3 — TOTAL host capabilities.  An allow-listed capability is
/// TRUSTED but, unlike sandboxed script code (3.3), un-analysed: if it aborts on a
/// script-supplied value the fault is PAST admission, and a runtime `catch_unwind`
/// only mops up after the break.  This is the host-side MIRROR of 3.3 — over every
/// `#cap`-tagged function it flags those whose call tree reaches an [`ABORT_OPS`] op
/// (`assert` / `panic` / `log_fatal`), so the host makes them total (validate +
/// return a clean error, never abort) before exposing them.
///
/// Follows EVERY callee — a capability's whole implementation must be total, not
/// just its top frame.  A NATIVE capability has no loft body, so its Rust is OPAQUE
/// to this lint; the host vouches for native totality separately.  This catches the
/// loft-bodied (library) capability surface, which grows as libraries expose `#cap`
/// functions.
#[must_use]
pub fn capability_totality_violations(
    data: &Data,
    fn_refs: &HashMap<u32, HashSet<u32>>,
) -> Vec<CapTotalityViolation> {
    let mut violations = Vec::new();
    let mut caps: Vec<u32> = (0..data.definitions())
        .filter(|&d| !data.def(d).cap().is_empty())
        .collect();
    caps.sort_unstable();
    for cap in caps {
        // Transitive closure from the capability, following every callee; a native
        // callee has no loft body, so it is a natural leaf (its Rust is opaque).
        let mut reach = HashSet::new();
        let mut work = vec![cap];
        while let Some(d) = work.pop() {
            if !reach.insert(d) {
                continue;
            }
            let mut refs = HashSet::new();
            referenced_defs(data, d, &mut refs);
            add_recorded_fn_refs(fn_refs, d, &mut refs);
            work.extend(refs);
        }
        // Any reached explicit-abort op (3.3) ⇒ the capability is NOT total.
        let mut hits: Vec<u32> = reach
            .iter()
            .copied()
            .filter(|&r| ABORT_OPS.contains(&data.def(r).name()))
            .collect();
        hits.sort_unstable();
        for op in hits {
            violations.push(CapTotalityViolation {
                capability: cap,
                op,
                position: reference_position(data, cap, op),
            });
        }
    }
    violations
}

/// @PLN86 prevention #3 — render a capability-totality violation as an actionable
/// host-side lint message.
#[must_use]
pub fn describe_cap_totality_violation(data: &Data, v: &CapTotalityViolation) -> String {
    let cap = display_name(data, v.capability);
    let op = display_name(data, v.op);
    let at = v
        .position
        .as_ref()
        .map_or_else(String::new, |p| format!("{p}: "));
    format!(
        "{at}host capability `{cap}` is not total: it can reach `{op}`, which aborts — a \
         script-supplied value could fault the HOST through it (admission cannot see past a \
         trusted capability).\n  fix: make `{cap}` total — validate the input and return a \
         clean error value (e.g. a nullable result), never `{op}`."
    )
}

/// @PLN86 step 3.4 — one def's intrinsic complexity: the deepest loop nesting at
/// which plain work happens (`max_leaf`), and every call with the loop nesting it
/// sits under (`(nesting, callee)`).  A pure single-def walk — the inter-procedural
/// composition is `complexity_degree`.  `Loop` (incl. comprehensions) / `Iter` /
/// each add one nesting level; an admitted script has no `while` (3.1) so
/// every loop is bounded.
fn intrinsic_complexity(data: &Data, f: u32) -> (u32, Vec<(u32, u32)>) {
    fn scan(v: &Value, nesting: u32, max_leaf: &mut u32, calls: &mut Vec<(u32, u32)>) {
        let child_nesting = match v {
            Value::Loop(_) | Value::Iter(..) => nesting + 1,
            _ => nesting,
        };
        if let Value::Call(g, _) = v {
            calls.push((nesting, *g));
        } else {
            *max_leaf = (*max_leaf).max(nesting);
        }
        v.for_each_child(&mut |c| scan(c, child_nesting, max_leaf, calls));
    }
    let (mut max_leaf, mut calls) = (0, Vec::new());
    scan(&data.def(f).code, 0, &mut max_leaf, &mut calls);
    (max_leaf, calls)
}

/// @PLN86 step 3.4 — the polynomial DEGREE of `f`'s worst-case step count in the
/// input size `n` (the largest collection / range a loop iterates): the max over
/// `f`'s body of `loop_nesting + degree(callee)`.  A loop calling a fn that loops
/// composes (O(n²)).  The call graph is acyclic (3.2), so this terminates; the
/// `in_progress` guard is a defensive backstop.  Trusted-leaf ops count as O(1)
/// (degree 0) per call in v1.
fn complexity_degree(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    f: u32,
    memo: &mut HashMap<u32, u32>,
    in_progress: &mut HashSet<u32>,
) -> u32 {
    if let Some(&d) = memo.get(&f) {
        return d;
    }
    if !in_progress.insert(f) {
        return 0; // cycle (rejected by 3.2) — break defensively
    }
    let (max_leaf, calls) = intrinsic_complexity(data, f);
    let mut degree = max_leaf;
    for (nesting, callee) in calls {
        let callee_degree = if sandboxed.contains_key(&callee) {
            complexity_degree(data, sandboxed, callee, memo, in_progress)
        } else {
            0
        };
        degree = degree.max(nesting + callee_degree);
    }
    in_progress.remove(&f);
    memo.insert(f, degree);
    degree
}

/// @PLN86 step 3.4 — the worst-case complexity DEGREE over all sandboxed entries:
/// the script's step count is `O(n^degree)` in the largest input size.  An
/// admitted script is total (3.1–3.3), so this is finite and computable; the host
/// reads it to **bound the inputs** so no single frame stalls (L5).
#[must_use]
pub fn sandbox_complexity_degree(data: &Data, sandboxed: &HashMap<u32, String>) -> u32 {
    let (mut memo, mut in_progress) = (HashMap::new(), HashSet::new());
    sandboxed
        .keys()
        .map(|&f| complexity_degree(data, sandboxed, f, &mut memo, &mut in_progress))
        .max()
        .unwrap_or(0)
}

/// @PLN86 — variables truly RESET within `v`: a `Set(var, rhs)` whose `rhs` does
/// NOT read `var`.  Inside a loop such a var is fresh each iteration, so an append
/// to it does not accumulate (a transient buffer, O(1) space).  A SELF-referential
/// `Set(v, … v …)` — what `v += x` lowers to (`Set(v, OpAppendVector(v, x))`) — is
/// a GROWTH, not a reset, so `v` is NOT collected and stays accumulating.
fn collect_set_targets(v: &Value, out: &mut HashSet<u16>) {
    v.walk(&mut |n| {
        if let Value::Set(var, rhs) = n
            && !rhs.reads_var(*var)
        {
            out.insert(*var);
        }
    });
}

/// @PLN86 P7.1 — heap bytes one appended element costs in the COLLECTION's backing,
/// given the collection var's type (the append's first arg): the per-element SLOT
/// size (`element_size`) — a dense scalar (`i64` = 8), or the `DbRef` slot of a
/// reference / struct / enum-struct element (a `vector<Mob>` stores nullable-enum
/// DbRef slots).  This is the backing footprint per element.  KNOWN v1 under-count:
/// the separately-allocated record BODIES (a `Mob`'s own record, a nested
/// collection's backing) are NOT added here — the same gap §8 already documents;
/// tightening it means deep-sizing through the nullable wrappers.  A keyed
/// collection's element is its declared record (also a DbRef slot).
fn appended_record_size(vars: &crate::variables::Function, var_nr: u16) -> u32 {
    let elem: Type = match vars.tp(var_nr) {
        Type::Vector(e, _) => (**e).clone(),
        Type::Sorted(nr, _, _)
        | Type::Index(nr, _, _)
        | Type::Hash(nr, _, _)
        | Type::Radix(nr, _, _)
        | Type::Trie(nr, _, _) => Type::Reference(*nr, crate::data::Deps::none()),
        _ => return 0,
    };
    crate::data::element_stack_size(&elem) as u32
}

/// @PLN86 space budget — one def's intrinsic SPACE complexity: the deepest loop
/// nesting at which an in-place append (`OpAppend*`) grows a structure NOT reset
/// inside that loop (declared in an enclosing scope), so it ACCUMULATES across the
/// iterations (`O(n)` per nesting level); PLUS the coefficient `Σ record_size` over
/// those accumulating sites (P7.1).  Plus every call with its loop nesting for the
/// inter-procedural composition (`space_degree`).  Conservative — a transient buffer
/// (reset each iteration) is O(1) and excluded; the `Iter` init's once-run allocation
/// is counted in-loop (a safe over-count, like 3.4b).  KNOWN GAPS (v1 under-models,
/// future tightening): a concat-reassign `v = v + [x]` and a call returning a kept
/// growing structure are not yet treated as accumulation.
fn intrinsic_space(data: &Data, f: u32) -> (u32, u32, bool, Vec<(u32, u32)>) {
    /// Accumulator threaded through the space scan (bundled so the recursion stays
    /// within clippy's argument limit): vars reset inside the current loop, the
    /// deepest accumulating nesting, the coefficient, whether a dynamic STRING grows
    /// unboundedly (F10 — its byte size is not a fixed record stride, so it is invisible
    /// to `coeff`), and the outgoing calls.
    struct St<'a> {
        vars: &'a crate::variables::Function,
        reset: HashSet<u16>,
        max_leaf: u32,
        coeff: u32,
        text_growth: bool,
        calls: Vec<(u32, u32)>,
    }
    fn scan(data: &Data, v: &Value, nesting: u32, st: &mut St) {
        // Peeled into a BINDING, not into the match scrutinee — and that distinction is the
        // whole content of this comment.
        //
        // This scan ends in `v.for_each_child(&mut |c| scan(…))`, and `for_each_child`
        // DESCENDS THROUGH a `Span` (`Value::Span(b) => f(&b.1)`).  So the bare `match v`
        // this replaces was already correct: a spanned `Call` took `_ => {}` here and was
        // counted once, one level down.  Writing `match v.unspan()` instead — which is the
        // obvious way to obey `Value::unspan`'s rule — counts it TWICE: once from the peeled
        // scrutinee, then again when the trailing walk descends into the same node.  Measured:
        // that inflated one program's declared bound from `24 · n²` to `36 · n²`, and a
        // `data_budget` between the two then REJECTED a program that fits.
        //
        // Shadowing `v` fixes both halves at once: the match sees the unspanned node and the
        // trailing walk descends from it, so each node is visited exactly once.  Verified
        // behaviour-identical to the original on the same program.
        let v = v.unspan();
        match v {
            Value::Loop(_) | Value::Iter(..) => {
                // vars reset somewhere in this loop don't accumulate across it.
                let mut targets = HashSet::new();
                collect_set_targets(v, &mut targets);
                let fresh: Vec<u16> = targets
                    .into_iter()
                    .filter(|t| st.reset.insert(*t))
                    .collect();
                v.for_each_child(&mut |c| scan(data, c, nesting + 1, st));
                for t in fresh {
                    st.reset.remove(&t);
                }
                return;
            }
            Value::Call(g, args) => {
                let nm = data.def(*g).name();
                // A structure GROWS via an in-place append (`OpAppend*`) or a
                // capacity prealloc for an appended element (`OpPreAlloc*` — what
                // `r += [x]` lowers to).  On a var not reset in the loop it
                // ACCUMULATES across the iterations.
                if nm.starts_with("OpAppend") || nm.starts_with("OpPreAlloc") {
                    if nesting > 0
                        && let Some(Value::Var(t)) = args.first().map(Value::unspan)
                        && !st.reset.contains(t)
                    {
                        st.max_leaf = st.max_leaf.max(nesting);
                        st.coeff += appended_record_size(st.vars, *t);
                        // A growing TEXT has no fixed record stride — its byte length
                        // is unbounded, so `coeff` cannot see it; flag it for F10.  The
                        // var is a `text` or a promoted `&text` work-ref (text-return).
                        let is_text = match st.vars.tp(*t) {
                            Type::Text(_) => true,
                            Type::RefVar(inner) => matches!(**inner, Type::Text(_)),
                            _ => false,
                        };
                        if is_text {
                            st.text_growth = true;
                        }
                    }
                } else {
                    st.calls.push((nesting, *g));
                }
            }
            _ => {}
        }
        v.for_each_child(&mut |c| scan(data, c, nesting, st));
    }
    let def = data.def(f);
    let mut st = St {
        vars: &def.variables,
        reset: HashSet::new(),
        max_leaf: 0,
        coeff: 0,
        text_growth: false,
        calls: Vec::new(),
    };
    scan(data, &def.code, 0, &mut st);
    (st.max_leaf, st.coeff, st.text_growth, st.calls)
}

/// @PLN86 §8 (F10) — does any sandboxed def grow a dynamic STRING unboundedly (an
/// `OpAppend*` to a `text` var inside a loop, not reset there)?  Such a build has no
/// fixed record stride, so the F9 coefficient is blind to it; F10 rejects it unless the
/// profile caps strings with `max_string_len`.  Intra-procedural (the append + its
/// enclosing loop in one def) — a string grown across a call boundary is a documented
/// v1 gap, like the inter-procedural coeff case.
#[must_use]
pub fn sandbox_grows_unbounded_string(data: &Data, sandboxed: &HashMap<u32, String>) -> bool {
    sandboxed.keys().any(|&f| intrinsic_space(data, f).2)
}

/// @PLN86 space budget — `f`'s worst-case PEAK heap as `(degree, coeff)`: the
/// polynomial DEGREE in the input size (`max(intrinsic accumulation, loop_nesting +
/// callee degree)`) and the COEFFICIENT `Σ record_size` over its accumulating sites
/// plus those of every accumulating callee (P7.1).  Acyclic (3.2) so this terminates;
/// `in_progress` is a defensive backstop.
fn space_degree(
    data: &Data,
    sandboxed: &HashMap<u32, String>,
    f: u32,
    memo: &mut HashMap<u32, (u32, u32)>,
    in_progress: &mut HashSet<u32>,
) -> (u32, u32) {
    if let Some(&d) = memo.get(&f) {
        return d;
    }
    if !in_progress.insert(f) {
        return (0, 0);
    }
    // `text_growth` (`.2`) is handled by `sandbox_grows_unbounded_string` (F10),
    // not the degree/coeff footprint, so it is ignored here.
    let (max_leaf, mut coeff, _text_growth, calls) = intrinsic_space(data, f);
    let mut degree = max_leaf;
    for (nesting, callee) in calls {
        let (callee_degree, callee_coeff) = if sandboxed.contains_key(&callee) {
            space_degree(data, sandboxed, callee, memo, in_progress)
        } else {
            (0, 0)
        };
        // Only a callee that ITSELF accumulates composes (its internal O(n^s)
        // build, run under the caller's loops).  A non-accumulating call — a
        // trusted leaf op, or a fn that allocates O(1) — adds NO space here: its
        // result, if kept across a loop, is the CALLER'S append, already counted
        // at the bind site.  (This is where space diverges from the time rule,
        // which adds `nesting` for every call.)
        if callee_degree > 0 {
            degree = degree.max(nesting + callee_degree);
            coeff += callee_coeff;
        }
    }
    in_progress.remove(&f);
    memo.insert(f, (degree, coeff));
    (degree, coeff)
}

/// @PLN86 space budget — worst-case SPACE degree over all sandboxed entries: peak
/// heap is `O(n^degree)` in the largest input.  The host reads it to BOUND the
/// inputs so a bounded loop building a structure cannot OOM — an abort that
/// `catch_unwind` can never see — turning OOM into a load-time concern.  The
/// analogue of the time budget (3.4) for memory.
#[must_use]
pub fn sandbox_space_degree(data: &Data, sandboxed: &HashMap<u32, String>) -> u32 {
    sandbox_space_footprint(data, sandboxed).0
}

/// @PLN86 P7.1 — the worst-case peak-heap FOOTPRINT shape `(degree, coeff)` over all
/// sandboxed entries: peak heap is bounded by `coeff · n^degree` bytes in the largest
/// input `n`.  `degree` is the max over entries; `coeff` sums their per-entry
/// coefficients (conservative — entries that share an accumulating callee count it in
/// each, an over-estimate that never under-bounds).  F11 compares `coeff ·
/// max_input_n^degree` against the profile's `data_budget`.
#[must_use]
pub fn sandbox_space_footprint(data: &Data, sandboxed: &HashMap<u32, String>) -> (u32, u32) {
    let (mut memo, mut in_progress) = (HashMap::new(), HashSet::new());
    let mut degree = 0;
    let mut coeff = 0u32;
    for &f in sandboxed.keys() {
        let (d, c) = space_degree(data, sandboxed, f, &mut memo, &mut in_progress);
        degree = degree.max(d);
        coeff = coeff.saturating_add(c);
    }
    (degree, coeff)
}

/// @PLN86 — the human-readable complexity report (the L5 host hint): worst-case
/// TIME (step count, 3.4) and SPACE (peak heap) degrees, so the host bounds inputs
/// against the tighter of the time and memory budgets.
#[must_use]
pub fn complexity_report(time: u32, space: u32) -> String {
    let big_o = |d: u32| match d {
        0 => "O(1)".to_string(),
        1 => "O(n)".to_string(),
        d => format!("O(n^{d})"),
    };
    format!(
        "worst-case complexity: time {} / space {} (n = the largest input collection \
         / range; bound n to keep each frame within the time AND memory budget)",
        big_o(time),
        big_o(space)
    )
}

/// @PLN86 step 2.4 — a no-raw-write violation: a sandboxed def directly mutates
/// heap data (`x.field = v` / `v[i] = v`).  An admitted script may mutate host
/// data only through allow-listed `*.write` operations, never a raw assignment,
/// so the host's invariants are preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct RawWriteViolation {
    pub def: u32,
    pub position: Position,
}

/// @PLN86 step 2.4 — the no-raw-write admission: every sandboxed def that the
/// parser recorded a raw field/index write for (`sandbox_raw_writes`).  The
/// check is at parse (the parser knows the LHS is a field/index target, distinct
/// from a bare local or a struct literal), so this just surfaces the record.
#[must_use]
pub fn raw_write_violations(
    sandboxed: &HashMap<u32, String>,
    raw_writes: &HashMap<u32, Position>,
) -> Vec<RawWriteViolation> {
    let mut out: Vec<RawWriteViolation> = raw_writes
        .iter()
        .filter(|(d, _)| sandboxed.contains_key(d))
        .map(|(&def, position)| RawWriteViolation {
            def,
            position: position.clone(),
        })
        .collect();
    out.sort_by_key(|v| v.def);
    out
}

/// @PLN86 step 2.4 — render a no-raw-write violation as an actionable error.
#[must_use]
pub fn describe_raw_write_violation(data: &Data, v: &RawWriteViolation) -> String {
    let from = display_name(data, v.def);
    format!(
        "{}: sandboxed `{from}` performs a raw write to heap data (`x.field = …` / \
         `v[i] = …`) — a sandboxed script may not directly mutate host fields/elements, \
         which could break an invariant.\n  fix: mutate only through an allow-listed write \
         operation (a `*.write` capability such as `damage(e, 10)`); construct new values \
         freely.",
        v.position
    )
}

/// @PLN86 P6.4 (F4) — a sandboxed read of a host field whose `#read` capability the
/// active profile does not grant.  Reads are default-allow, so this fires only on a
/// field the host explicitly marked private with a `#read` link.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldReadViolation {
    pub def: u32,
    pub token: String,
    pub position: Position,
}

/// @PLN86 P6.4 (F4) — the field-read admission: every recorded sandboxed read of a
/// `#read`-linked host field whose token the reading def's profile does not grant.
#[must_use]
pub fn field_read_violations(
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    field_reads: &HashMap<u32, Vec<(String, Position)>>,
) -> Vec<FieldReadViolation> {
    let mut out = Vec::new();
    for (&def, reads) in field_reads {
        if !sandboxed.contains_key(&def) {
            continue;
        }
        let profile = config.profiles.get(&sandboxed[&def]);
        for (token, position) in reads {
            if !profile.is_some_and(|p| p.allows(token)) {
                out.push(FieldReadViolation {
                    def,
                    token: token.clone(),
                    position: position.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.def.cmp(&b.def).then_with(|| a.token.cmp(&b.token)));
    out
}

/// @PLN86 P6.4 (F4) — render a field-read violation as an actionable error.
#[must_use]
pub fn describe_field_read_violation(data: &Data, v: &FieldReadViolation) -> String {
    let from = display_name(data, v.def);
    format!(
        "{}: sandboxed `{from}` reads a host field that needs capability `{}` — not \
         granted.\n  fix: add `{}` to `allow`, or read only fields the profile permits.",
        v.position, v.token, v.token
    )
}

/// @PLN86 P6.4 (F5) — a sandboxed raw write (`e.f = v`) of a host field whose
/// `#update` capability the active profile does not grant.  Only a field that
/// CARRIES an `#update` link reaches here (a field with none stays the coarse 2.4
/// reject); so this fires exactly when the writable field's token is not granted.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldUpdateViolation {
    pub def: u32,
    pub field: String,
    pub token: String,
    pub position: Position,
}

/// @PLN86 P6.4 (F5) — the field-update admission: every recorded sandboxed write of
/// an `#update`-linked host field whose token the writing def's profile does not grant.
#[must_use]
pub fn field_update_violations(
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    member_access: &HashMap<(u32, String), Vec<String>>,
    field_updates: &HashMap<u32, Vec<(u32, String, Position)>>,
) -> Vec<FieldUpdateViolation> {
    let mut out = Vec::new();
    for (&def, writes) in field_updates {
        if !sandboxed.contains_key(&def) {
            continue;
        }
        let profile = config.profiles.get(&sandboxed[&def]);
        for (struct_def, field, position) in writes {
            let updates = member_access
                .get(&(*struct_def, field.clone()))
                .into_iter()
                .flatten()
                .filter(|t| t.ends_with("#update"));
            for token in updates {
                if !profile.is_some_and(|p| p.allows(token)) {
                    out.push(FieldUpdateViolation {
                        def,
                        field: field.clone(),
                        token: token.clone(),
                        position: position.clone(),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.def.cmp(&b.def).then_with(|| a.field.cmp(&b.field)));
    out
}

/// @PLN86 P6.4 (F5) — render a field-update violation as an actionable error.
#[must_use]
pub fn describe_field_update_violation(data: &Data, v: &FieldUpdateViolation) -> String {
    let from = display_name(data, v.def);
    format!(
        "{}: sandboxed `{from}` writes host field `{}`, which needs capability `{}` — not \
         granted.\n  fix: add `{}` to `allow`, or mutate only fields the profile permits.",
        v.position, v.field, v.token, v.token
    )
}

/// @PLN86 P6.4 (F6) — a sandboxed APPEND (`e.f += x`, growing a collection) to a host
/// field whose `#append` capability the active profile does not grant.  Only a field
/// that CARRIES an `#append` link reaches here (a `+=` without one is an `#update` or a
/// coarse reject); so this fires exactly when the appendable field's token is ungranted.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldAppendViolation {
    pub def: u32,
    pub field: String,
    pub token: String,
    pub position: Position,
}

/// @PLN86 P6.4 (F6) — the field-append admission: every recorded sandboxed append to an
/// `#append`-linked host field whose token the writing def's profile does not grant.
#[must_use]
pub fn field_append_violations(
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    member_access: &HashMap<(u32, String), Vec<String>>,
    field_appends: &HashMap<u32, Vec<(u32, String, Position)>>,
) -> Vec<FieldAppendViolation> {
    let mut out = Vec::new();
    for (&def, writes) in field_appends {
        if !sandboxed.contains_key(&def) {
            continue;
        }
        let profile = config.profiles.get(&sandboxed[&def]);
        for (struct_def, field, position) in writes {
            let appends = member_access
                .get(&(*struct_def, field.clone()))
                .into_iter()
                .flatten()
                .filter(|t| t.ends_with("#append"));
            for token in appends {
                if !profile.is_some_and(|p| p.allows(token)) {
                    out.push(FieldAppendViolation {
                        def,
                        field: field.clone(),
                        token: token.clone(),
                        position: position.clone(),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| a.def.cmp(&b.def).then_with(|| a.field.cmp(&b.field)));
    out
}

/// @PLN86 P6.4 (F6) — render a field-append violation as an actionable error.
#[must_use]
pub fn describe_field_append_violation(data: &Data, v: &FieldAppendViolation) -> String {
    let from = display_name(data, v.def);
    format!(
        "{}: sandboxed `{from}` appends to host field `{}`, which needs capability `{}` — \
         not granted.\n  fix: add `{}` to `allow`, or grow only fields the profile permits.",
        v.position, v.field, v.token, v.token
    )
}

/// @PLN86 §7.2 (F7) — a sandboxed call site OVERRODE a parameter the host pinned with a
/// `…#default` lock, but the active profile does not grant the lock.  An argument equal
/// to the parameter's default is NOT an override (the default is what the lock pins to),
/// so only a value that DIFFERS reaches here — and then the modder needs the lock.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamLockViolation {
    /// The sandboxed def whose body holds the overriding call.
    pub def: u32,
    /// The lock token written on the parameter (e.g. `"spawn.count#default"`).
    pub token: String,
    pub position: Position,
}

/// @PLN86 §7.2 (F7) — the parameter-lock admission: every recorded sandboxed override of
/// a `…#default`-locked parameter whose lock token the overriding def's profile does not
/// grant.  `overrides` is keyed by the calling def → each `(lock token, position)` of an
/// argument that differed from the locked parameter's default.
#[must_use]
pub fn param_lock_violations(
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
    overrides: &HashMap<u32, Vec<(String, Position)>>,
) -> Vec<ParamLockViolation> {
    let mut out = Vec::new();
    for (&def, calls) in overrides {
        if !sandboxed.contains_key(&def) {
            continue;
        }
        let profile = config.profiles.get(&sandboxed[&def]);
        for (token, position) in calls {
            if !profile.is_some_and(|p| p.allows(token)) {
                out.push(ParamLockViolation {
                    def,
                    token: token.clone(),
                    position: position.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.def.cmp(&b.def).then_with(|| a.token.cmp(&b.token)));
    out
}

/// @PLN86 §7.2 (F7) — render a parameter-lock violation as an actionable error.
#[must_use]
pub fn describe_param_lock_violation(data: &Data, v: &ParamLockViolation) -> String {
    let from = display_name(data, v.def);
    format!(
        "{}: sandboxed `{from}` overrides a parameter the host pinned with `{}` — not \
         granted.\n  fix: add `{}` to `allow`, or omit the argument so it keeps its default.",
        v.position, v.token, v.token
    )
}

/// @PLN86 §8 (F11) — a data-envelope violation: the proven worst-case peak heap does
/// not fit the profile's declared `data_budget`, or cannot be bounded because a degree
/// depends on an unset `max_input_n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataViolation {
    /// `coeff · max_input_n^degree` bytes exceeds `data_budget`.
    OverBudget {
        profile: String,
        figure: u64,
        budget: u64,
        degree: u32,
        coeff: u32,
        max_input_n: u64,
    },
    /// Peak heap grows with input (degree > 0) but `max_input_n` is unset, so the
    /// figure cannot be bounded — deny-by-default.
    Unprovable { profile: String, degree: u32 },
    /// @PLN86 §8 (F10) — a dynamic string grows unboundedly (no fixed record stride,
    /// invisible to `coeff`) and the profile caps no string length, so the footprint
    /// cannot be bounded.
    UnboundedAlloc { profile: String },
}

/// `coeff · n^degree`, saturating: a genuinely over-budget figure can overflow `u64`,
/// and saturation preserves the only fact we need ("exceeds the budget").
fn footprint_bytes(coeff: u32, n: u64, degree: u32) -> u64 {
    let mut v = u64::from(coeff);
    for _ in 0..degree {
        v = v.saturating_mul(n);
    }
    v
}

/// @PLN86 §8 (F11) — the data-envelope admission: for every designated profile that
/// sets a `data_budget`, prove the sandboxed code's worst-case peak heap
/// (`coeff · max_input_n^degree`, the P7.1 footprint) fits — else reject.  A non-zero
/// degree with an unset `max_input_n` is unprovable (rejected, deny-by-default).  A
/// profile without a `data_budget` is report-only (skipped).  The footprint is the
/// program-global one; a single-profile sandbox (the norm) is exact, a multi-profile
/// one is checked conservatively against each budget.
#[must_use]
pub fn data_envelope_violations(
    data: &Data,
    config: &SandboxConfig,
    sandboxed: &HashMap<u32, String>,
) -> Vec<DataViolation> {
    let (degree, coeff) = sandbox_space_footprint(data, sandboxed);
    let grows_string = sandbox_grows_unbounded_string(data, sandboxed);
    let mut profiles: Vec<&String> = sandboxed.values().collect();
    profiles.sort();
    profiles.dedup();
    let mut out = Vec::new();
    for pname in profiles {
        let Some(p) = config.profiles.get(pname) else {
            continue;
        };
        if p.data_budget == 0 {
            continue; // no envelope declared → report-only (F10 + F11 both opt-in)
        }
        // @PLN86 §8 (F10) — a dynamic string growing under an ACTIVE envelope must be
        // capped, else the footprint is unbounded (and `coeff` cannot see it).
        if grows_string && p.max_string_len == 0 {
            out.push(DataViolation::UnboundedAlloc {
                profile: pname.clone(),
            });
        }
        if degree > 0 && p.max_input_n == 0 {
            out.push(DataViolation::Unprovable {
                profile: pname.clone(),
                degree,
            });
            continue;
        }
        let figure = footprint_bytes(coeff, p.max_input_n, degree);
        if figure > p.data_budget {
            out.push(DataViolation::OverBudget {
                profile: pname.clone(),
                figure,
                budget: p.data_budget,
                degree,
                coeff,
                max_input_n: p.max_input_n,
            });
        }
    }
    out
}

/// @PLN86 §8 (F11) — render a data-envelope violation as an actionable error naming
/// the figure and the fix.
#[must_use]
pub fn describe_data_violation(v: &DataViolation) -> String {
    match v {
        DataViolation::OverBudget {
            profile,
            figure,
            budget,
            degree,
            coeff,
            max_input_n,
        } => format!(
            "profile `{profile}`: sandboxed worst-case peak heap ~{figure} bytes \
             ({coeff} · {max_input_n}^{degree}) exceeds data_budget {budget}.\n  fix: \
             raise data_budget, lower max_input_n, or build less per input."
        ),
        DataViolation::Unprovable { profile, degree } => format!(
            "profile `{profile}`: peak heap grows as O(n^{degree}) but max_input_n is \
             unset, so the footprint cannot be bounded.\n  fix: set max_input_n in the \
             profile, or remove the input-growing allocation."
        ),
        DataViolation::UnboundedAlloc { profile } => format!(
            "profile `{profile}`: a dynamic string grows unboundedly in a loop, so its \
             heap size cannot be bounded.\n  fix: set max_string_len in the profile to \
             cap it, or build the string outside the loop."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_match_is_dotted_segment() {
        assert!(cap_prefix_match("game", "game")); // exact
        assert!(cap_prefix_match("game", "game.read")); // child
        assert!(cap_prefix_match("game", "game.entity.write")); // grandchild
        assert!(!cap_prefix_match("game", "gameover")); // NOT a string prefix
        assert!(!cap_prefix_match("game.read", "game.write")); // sibling
        assert!(!cap_prefix_match("game.read", "game")); // parent is not allowed by child
        assert!(!cap_prefix_match("", "game")); // empty allow never matches
    }

    #[test]
    fn allows_is_deny_by_default() {
        let p = SandboxProfile {
            allow: vec![
                "game#read".into(),
                "math#read".into(),
                "collections#read".into(),
            ],
            ..SandboxProfile::default()
        };
        assert!(p.allows("game#read"));
        assert!(p.allows("math#read"));
        assert!(p.allows("math.trig#read")); // group child of an allowed prefix, same right
        assert!(p.allows("game.entity#read")); // group child of `game`, same right
        assert!(p.allows("collections#read"));
        assert!(!p.allows("collections#update")); // same group, different right — denied
        assert!(!p.allows("game#update")); // same group, different right — denied
        assert!(!p.allows("fs#read")); // group not listed — denied
        assert!(!p.allows("game")); // no `#right` — denied
        assert!(!p.allows("")); // empty — denied
        // an empty profile denies everything
        assert!(!SandboxProfile::default().allows("math#read"));
    }

    #[test]
    fn parses_profiles_and_designations() {
        let toml = r#"
[package]
name = "ztgame"

[sandbox]
mod-script = ["mods/**/*.loft", "fn:player_eval"]

[profile.mod-script]
backend = "interpret"
native_ffi = false
allow = ["game#read", "game#update", "math#read", "collections#read"]
"#;
        let cfg = parse_sandbox_config(toml);
        assert!(cfg.is_active());
        assert_eq!(cfg.designations.len(), 2);
        assert_eq!(
            cfg.designations[0],
            ("mods/**/*.loft".to_string(), "mod-script".to_string())
        );
        assert_eq!(
            cfg.designations[1],
            ("fn:player_eval".to_string(), "mod-script".to_string())
        );
        let p = cfg.profiles.get("mod-script").expect("profile present");
        assert!(!p.native_ffi);
        assert!(p.allows("game#update"));
        assert!(p.allows("collections#read"));
        assert!(!p.allows("fs#read")); // group not granted
        assert!(!p.allows("collections#update")); // same group, different right — denied
        // resolve a designation to its profile
        assert!(
            cfg.profile_for_selector("mods/**/*.loft")
                .is_some_and(|p| p.allows("math#read"))
        );
    }

    #[test]
    fn parses_data_envelope_fields() {
        let toml = r#"
[sandbox]
mod = ["fn:eval"]

[profile.mod]
allow_libs = ["code"]
max_input_n = 10000
max_depth = 64
max_string_len = 256
data_budget = "8_000_000"
"#;
        let cfg = parse_sandbox_config(toml);
        let p = cfg.profiles.get("mod").expect("profile present");
        assert_eq!(p.max_input_n, 10000);
        assert_eq!(p.max_depth, 64);
        assert_eq!(p.max_string_len, 256);
        assert_eq!(p.data_budget, 8_000_000, "underscore-grouped figure parses");
        // an absent envelope field stays 0 (unset).
        let bare = parse_sandbox_config("[sandbox]\nm = [\"fn:e\"]\n[profile.m]\n");
        let bp = bare.profiles.get("m").expect("profile present");
        assert_eq!(
            (bp.max_input_n, bp.data_budget),
            (0, 0),
            "absent → unset (0)"
        );
    }

    #[test]
    fn fn_designation_resolves_only_designated_functions() {
        let cfg = parse_sandbox_config(
            "[sandbox]\nmod-script = [\"fn:player_eval\", \"fn:on_tick\"]\n[profile.mod-script]\n",
        );
        assert_eq!(cfg.fn_designation("player_eval"), Some("mod-script"));
        assert_eq!(cfg.fn_designation("on_tick"), Some("mod-script"));
        assert_eq!(cfg.fn_designation("host_update"), None); // not designated
        assert_eq!(cfg.fn_designation(""), None);
    }

    #[test]
    fn empty_profile_section_registers_a_deny_all_profile() {
        let cfg = parse_sandbox_config("[sandbox]\nlocked = [\"a.loft\"]\n[profile.locked]\n");
        let p = cfg.profiles.get("locked").expect("registered");
        assert!(p.allow.is_empty());
        assert!(p.allow_libs.is_empty());
        assert!(!p.allows("game#read")); // deny-all by default
        assert!(!p.native_ffi); // safe default
    }

    #[test]
    fn no_sandbox_section_is_inactive() {
        let cfg = parse_sandbox_config("[package]\nname = \"x\"\n");
        assert!(!cfg.is_active());
        assert!(cfg.profiles.is_empty());
    }

    /// #631 — a path selector must match the file as an author writes it
    /// (project-relative) against the absolute path the parser carries, and must
    /// not over-match: a selector that designates nothing, or designates the wrong
    /// file, is a sandbox policy that silently covers the wrong code.
    #[test]
    fn path_selector_matches_relative_and_glob_forms() {
        let abs = "/home/u/proj/src/oplog.loft";
        assert!(path_selector_matches("src/oplog.loft", abs));
        assert!(path_selector_matches("oplog.loft", abs));
        assert!(path_selector_matches(abs, abs));
        assert!(path_selector_matches("src/*.loft", abs));
        assert!(path_selector_matches("proj/**/*.loft", abs));
        assert!(path_selector_matches("**/oplog.loft", abs));
        assert!(path_selector_matches("src/oplo?.loft", abs));
        // `**/` also spans zero segments.
        assert!(path_selector_matches("proj/**/src/oplog.loft", abs));

        assert!(!path_selector_matches("src/other.loft", abs));
        assert!(!path_selector_matches("oplog", abs));
        // A partial trailing segment is not a match — only whole segments count.
        assert!(!path_selector_matches("log.loft", abs));
        // `*` stops at a separator, so it cannot reach across a directory: `**` is
        // the form that spans one.  (A bare `*.loft` still matches any `.loft` file,
        // via the trailing-segment rule — that selector IS "every loft file".)
        assert!(!path_selector_matches(
            "src/*.loft",
            "/home/u/proj/src/sub/oplog.loft"
        ));
        assert!(path_selector_matches(
            "src/**/*.loft",
            "/home/u/proj/src/sub/oplog.loft"
        ));
    }

    /// #631 — the selector a diagnostic offers must be writable into a committed
    /// policy, so it is project-relative rather than the machine's absolute path.
    #[test]
    fn suggested_path_selector_is_relative() {
        assert_eq!(
            suggested_path_selector("/home/u/proj/src/oplog.loft"),
            "src/oplog.loft"
        );
        assert_eq!(suggested_path_selector("oplog.loft"), "oplog.loft");
    }
}
