// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I87 — Auto-use / lazy library triggers

//! Derive lazy-load trigger details from a library's public loft source
//! (lib_plans/59-lazy-stdlib).
//!
//! A library opts into lazy auto-load with `[triggers] enabled = true` in its
//! `loft.toml` — that is the author's WHOLE declaration.  The concrete trigger
//! surface the resolver needs is then *derived* from the library's own public
//! source at package/publish time and materialised into the registry
//! `index.json`, so the resolver can fire a trigger WITHOUT having fetched the
//! library.  Nothing is hand-listed, so nothing can drift (same source-of-truth
//! principle as the source-scanned `#native` symbols).
//!
//! What is derived:
//!
//!   * **Method triggers** — every `pub fn f(self: T, …)` / `pub fn f(both: T,
//!     …)` becomes `{ name: f, receiver: T }`.  These are the non-obvious
//!     triggers: a call like `line.matches(p)` (receiver `text`, method
//!     `matches`) does not name the package, so the registry must map it.
//!   * **Type triggers** — every `pub struct N` / `pub enum N` becomes `N`.
//!
//!   * **Builder triggers** — the free `pub fn`s the manifest LISTS under
//!     `[triggers] builders = ["set"]`, each checked against the source.  A bare
//!     `set(words)` in a consumer then auto-loads the library and imports that one
//!     name — the only trigger that puts a name into the program's own namespace,
//!     which is why it is declared rather than derived: the author chooses the few
//!     builders worth a bare name, and the shared namespace stays small.  A
//!     consumer's own `fn set` wins; `<pkg>::set(…)` always reaches the library's.
//!
//! What is NOT derived: the namespace.  `<lib>::<anything>` is the *universal
//! default* trigger — any `pkg::…` reference can always auto-load `pkg`, since
//! the namespace already names the package — so it never needs declaring.  Free
//! functions the manifest does not list as builders are reached only via the
//! namespace, so they need no entry either.

/// A derived method trigger: calling method `name` on a receiver of type
/// `receiver` should auto-load the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodTrigger {
    pub name: String,
    pub receiver: String,
}

/// The derived trigger surface of a library (namespace excluded — it is the
/// universal default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Triggers {
    pub methods: Vec<MethodTrigger>,
    pub types: Vec<String>,
    /// The declared builders the source confirms, in the manifest's order.
    pub builders: Vec<String>,
}

impl Triggers {
    /// True when there is nothing for the resolver to index beyond the
    /// always-present namespace default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.methods.is_empty() && self.types.is_empty() && self.builders.is_empty()
    }
}

/// Derive the trigger surface from a library's loft source text.  Pass the
/// concatenation of the library's `src/**/*.loft` (or one file); scanning is
/// line-oriented and only reads `pub` declarations.  `builders` is the manifest's
/// `[triggers] builders` list: a name it carries is kept only when the source
/// declares it as a free `pub fn` (no `self` / `both` receiver) — a method, a
/// private function or a name that is not there is dropped, so the registry
/// never maps a bare call to a function the library cannot answer.
#[must_use]
pub fn derive_triggers(source: &str, builders: &[String]) -> Triggers {
    let mut triggers = Triggers::default();
    let mut free: Vec<String> = Vec::new();
    for raw in source.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("pub fn ") {
            if let Some((name, recv_name, recv_type)) = parse_fn_head(rest) {
                if recv_name == "self" || recv_name == "both" {
                    triggers.methods.push(MethodTrigger {
                        name,
                        receiver: recv_type,
                    });
                } else {
                    free.push(name);
                }
            } else if let Some(open) = rest.find('(') {
                // A parameterless `pub fn name()` is free too.
                let name = rest[..open].trim();
                if !name.is_empty() {
                    free.push(name.to_string());
                }
            }
        } else if let Some(rest) = line.strip_prefix("pub struct ") {
            triggers.types.push(leading_ident(rest));
        } else if let Some(rest) = line.strip_prefix("pub enum ") {
            triggers.types.push(leading_ident(rest));
        }
    }
    for b in builders {
        if free.iter().any(|f| f == b) && !triggers.builders.contains(b) {
            triggers.builders.push(b.clone());
        }
    }
    triggers
}

/// Parse `matches(both: text, pattern: text) -> boolean {` into
/// (`matches`, `both`, `text`) — the fn name and its FIRST parameter's name +
/// type.  Returns `None` for a parameterless fn or a malformed head.
fn parse_fn_head(rest: &str) -> Option<(String, String, String)> {
    let open = rest.find('(')?;
    let name = rest[..open].trim();
    if name.is_empty() {
        return None;
    }
    let after = &rest[open + 1..];
    // First parameter ends at the first ',' (more params) or ')' (only param).
    let end = after.find([',', ')'])?;
    let first = after[..end].trim();
    let (pname, ptype) = first.split_once(':')?;
    Some((
        name.to_string(),
        pname.trim().to_string(),
        ptype.trim().to_string(),
    ))
}

/// Take the leading identifier of a type declaration body, dropping any
/// generic params / brace / whitespace: `Shape<T> {` → `Shape`.
fn leading_ident(rest: &str) -> String {
    rest.trim()
        .split([' ', '{', '<', '('])
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the `regex` library's public surface: free functions
    /// (`find`/`split`, pattern-first) plus the three text methods.
    const REGEX_SRC: &str = r"
// internal — not pub, must be ignored
fn match_start(pattern: text, input: text) -> integer;
#native

// free functions — pattern-first, reached via the namespace default
pub fn find(pattern: text, input: text) -> integer {
  match_start(pattern, input)
}
pub fn split(pattern: text, input: text) -> iterator<text> {
  yield from split_iter(pattern, input);
}

// text methods — these need derived triggers
pub fn matches(both: text, pattern: text) -> boolean {
  is_match(pattern, both)
}
pub fn regex_find(self: text, pattern: text) -> integer {
  match_start(pattern, self)
}
pub fn regex_split(self: text, pattern: text) -> iterator<text> {
  yield from split_iter(pattern, self);
}
";

    #[test]
    fn derives_only_text_method_triggers() {
        let t = derive_triggers(REGEX_SRC, &[]);
        assert_eq!(
            t.methods,
            vec![
                MethodTrigger {
                    name: "matches".into(),
                    receiver: "text".into()
                },
                MethodTrigger {
                    name: "regex_find".into(),
                    receiver: "text".into()
                },
                MethodTrigger {
                    name: "regex_split".into(),
                    receiver: "text".into()
                },
            ],
            "only self/both-receiver pub fns become method triggers"
        );
        assert!(t.types.is_empty(), "regex exposes no public types");
        assert!(
            t.builders.is_empty(),
            "nothing declared, so nothing is a builder"
        );
    }

    /// A declared builder is kept only when the source has it as a free `pub fn`: the
    /// methods, a private function and a name that is not there are dropped.
    #[test]
    fn a_builder_is_declared_and_checked_against_the_source() {
        let declared: Vec<String> = ["find", "matches", "match_start", "absent", "find"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let t = derive_triggers(REGEX_SRC, &declared);
        assert_eq!(t.builders, vec!["find".to_string()]);
        assert!(!t.is_empty());
        let t = derive_triggers("pub fn now() -> integer { 0 }\n", &["now".to_string()]);
        assert_eq!(
            t.builders,
            vec!["now".to_string()],
            "a parameterless fn is free"
        );
    }

    #[test]
    fn ignores_free_functions_and_internals() {
        let t = derive_triggers(REGEX_SRC, &[]);
        let names: Vec<&str> = t.methods.iter().map(|m| m.name.as_str()).collect();
        assert!(!names.contains(&"find"), "free `find` is not a trigger");
        assert!(!names.contains(&"split"), "free `split` is not a trigger");
        assert!(
            !names.contains(&"match_start"),
            "internal (non-pub) fns are not triggers"
        );
    }

    #[test]
    fn derives_public_types() {
        let src = "pub struct Match {\n  text: text,\n}\npub enum Mode { Greedy, Lazy }\n";
        let t = derive_triggers(src, &[]);
        assert_eq!(t.types, vec!["Match".to_string(), "Mode".to_string()]);
    }

    #[test]
    fn empty_surface_is_empty() {
        assert!(derive_triggers("fn private() {}\n", &[]).is_empty());
    }
}
