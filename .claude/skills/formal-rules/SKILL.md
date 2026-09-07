---
name: formal-rules
description: Set up and use a NAMED-RULE register with code-site citations in any project — the mechanism that makes "which sites enforce this rule?" a grep and "is this rule already implemented?" a lookup. Use this when adopting the @FR- tag convention in a new project, when writing or citing a formal rule, when checking that every citation resolves, or when asking which rule a piece of code enforces. Tree-agnostic method with a bindings section for this repo; the portable checker is scripts/rule_tags.py (vendor it and repoint two env vars).
user-invocable: false
---

# Formal rules — named invariants, cited from code

A rule written once, somewhere that is not the code, and *cited by every site that
enforces it*, survives growth that shape-based search does not: two implementations
of one rule stop looking alike long before they stop being one rule. This skill is
the operational half of that idea (the design-side rationale is the
`design-protocol` skill, § *As the system grows, anchor the question on the RULE*).

The doctrine that makes the register worth having: **the rules do not change to
match the code; the code changes to match the rules.** A rule already written
settles a question an issue may present as open — read the register BEFORE
deliberating a fix that has a choice in it.

## The convention (portable)

1. **A rules home** — a directory of Markdown docs, one per domain. A rule is
   DEFINED by a line `  (Name)  prose` inside a fenced code block (the rules-block
   shape), or by a deviation-register entry (`### D-xxx-N — …` header or
   `> **D-xxx-N — …` blockquote). Section headers and parenthesised mentions in
   prose are NOT definitions — both produced false positives when treated as such.
2. **A namespaced citation tag: `@FR-<Name>`** in a code comment at each site that
   enforces the rule. A bare `@Name` is not unambiguous in a tree that already uses
   `@` for anything else (measured in loft: a bare-`@` scan returned 4142 hits, not
   one a rule). Pick a prefix that no other tag family can collide with.
3. **Boundary-exact matching.** A general rule and its refinements share a stem
   (`B-View` / `B-View-Base`), so a citation matches only when the next character
   cannot continue a tag. Never rename rules to dodge a matcher.
4. **Only a DEFINED rule is a citation target.** Family prefixes that appear in
   prose (`D-own`, `B-Ref`) read like rules and are not; citing one is an error the
   checker reports.
5. **The checker gates**: every citation resolves, no rule defined twice. Run it in
   CI so a renamed or deleted rule cannot leave dangling citations.

## The instrument

`rule_tags.py` (~210 lines, stdlib-only Python) implements the whole mechanism:

```bash
python3 scripts/rule_tags.py list          # every defined rule + its doc
python3 scripts/rule_tags.py check         # citations resolve; no double definition (exit 1)
python3 scripts/rule_tags.py sites <tag>   # which code sites enforce this rule
python3 scripts/rule_tags.py dups          # rules cited from 2+ sites — duplication by MEANING
```

**Adopting it in another project**: vendor the file unchanged and point two
environment variables at your layout —

```bash
RULES_DIR=docs/rules CITE_DIRS=src:lib CITE_EXTS=.rs,.py python3 rule_tags.py check
```

(`RULES_DIR` = where the rules docs live; `CITE_DIRS` = colon-separated dirs whose
files are scanned for citations; `CITE_EXTS` = comma-separated extensions,
default `.rs`.) Then write the first rules doc in the definition shape above, cite
it from one site, and wire `check` into CI before the register grows.

## Working with the register

- **Writing a fix with a choice in it** → read the relevant rules doc first; a rule
  that already answers it makes one of the "two ways to close it" inadmissible.
- **A site enforces a rule** → cite it (`// @FR-B-Copy: …` beside the enforcing
  code). The citation is written at the only moment the fact is reliably known.
- **An edge the rules cannot express** → the RULE wants extending; extend the doc,
  then cite. Record a deliberate divergence as a numbered deviation and drive the
  open count to zero.
- **The standing audit** (loft: STABILITY_METHOD.md § The rule-led walk): pick a
  rule, split it into the questions its sites ask, find each question's one home,
  verify related cases against it — the defects are in the disagreements.

## In the loft tree (bindings — skip outside it)

- Rules home: `doc/claude/formal/` — `formal/README.md` § Rule tags is the
  convention's authority; `formal/IMPLEMENTATIONS.md` indexes merged enforcement.
- Checker: `scripts/rule_tags.py` (defaults already point here); `check` runs in CI.
- Tag family context: CLAUDE.md § Tracker tags (why `@FR-` cannot collide with
  `@F<digits>`, `@PLN`, `@P`, corpus annotations).
