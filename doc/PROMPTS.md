# Working with Claude — Prompts and Practices

This document covers two things: how to work effectively with Claude in this codebase,
and when to use each of the prepared prompts in § Prompts guide below.

---

## Contents

- [Working Effectively with Claude](#working-effectively-with-claude)
  - [Before asking Claude to implement something](#before-asking-claude-to-implement-something)
  - [Scoping Claude's work](#scoping-claudes-work)
  - [Giving Claude the right context](#giving-claude-the-right-context)
  - [Effective prompts for common tasks](#effective-prompts-for-common-tasks)
  - [What Claude does well in this codebase](#what-claude-does-well-in-this-codebase)
  - [What to watch for](#what-to-watch-for)
  - [Keeping sessions from going in circles](#keeping-sessions-from-going-in-circles)
- [Prompts guide](#prompts-guide)
  - [Session start](#session-start)
  - [Often — documentation health](#often--documentation-health)
  - [Session — active development](#session--active-development)
  - [Finally — end-of-session cleanup](#finally--end-of-session-cleanup)
  - [Occasionally — project health](#occasionally--project-health)
  - [Standalone / one-off prompts](#standalone--one-off-prompts)

---

## Working Effectively with Claude

Claude has been the primary pair-programmer on this codebase since the initial
implementation. The following practices make sessions productive and avoid common
failure modes.

### Before asking Claude to implement something

1. **Write the spec as a loft code example first.** Claude is much more effective
   when given a concrete input/output target than a prose description. Paste the
   loft code you want to work and the expected output. Claude will identify the
   affected pipeline stages without being asked.

2. **Identify the relevant files.** Tell Claude which files you believe are involved.
   This reduces the chance of Claude making changes in the wrong subsystem. The
   pipeline section in `DEVELOPERS.md` is the fastest way to identify the right files.

3. **State the constraints explicitly.** If the feature must not break backwards
   compatibility, say so. If it must not add a new Cargo dependency, say so. Claude
   will respect stated constraints but will not infer them from context.

### Scoping Claude's work

- **One subsystem per session.** When a feature touches the parser *and* codegen *and*
  scope analysis, split the work: first session adds the IR node and parser support
  (with a failing codegen test as the stopping point), second session adds codegen.
  Trying to do everything in one session increases the chance of compounding errors.

- **Ask for tests before implementation.** "Write the test first, then implement."
  Claude writes better tests when it is focused on tests rather than simultaneously
  writing implementation. The test also serves as a specification.

- **Ask for a plan before code.** For anything touching more than two files, ask
  Claude to describe the approach and identify all change points before writing code.
  Review the plan and correct it before Claude starts. A wrong plan caught early costs
  minutes; a wrong plan caught at the end of an implementation costs hours.

### Giving Claude the right context

Claude does not retain memory between sessions by default. At the start of a session
that continues prior work, provide:

- The current state of the failing test or open issue.
- The relevant section from `PROBLEMS.md` if there is an open issue.
- The output of `LOFT_LOG=minimal cargo test -- failing_test` if there is a runtime failure.
- The files most likely to be modified (use the pipeline guide in `DEVELOPERS.md` to identify them).

For debugging sessions, paste the full panic message and the last 50 lines of the
`LOFT_LOG` output. Claude can usually identify the root cause from these two inputs.

### Effective prompts for common tasks

| Task | Effective prompt pattern |
|------|-------------------------|
| Add a syntax feature | "Add `<syntax>` to loft. The expected behaviour is `<loft example>`. The result should be `<value>`. Identify which pipeline stages need to change, write the failing test first, then implement." |
| Fix a runtime crash | "Loft crashes with `<panic message>`. The LOFT_LOG output ends with `<last 20 lines>`. The failing test is `<test name>` in `<file>`. Identify the root cause and fix it." |
| Fix a validate_slots panic | "validate_slots panics with `<message>`. The conflicting variables are `<A>` (live `<interval>`) and `<B>` (live `<interval>`). Are their intervals truly overlapping? If so, find the scope analysis bug. If not, extend find_conflict to exempt this case." |
| Add a standard library function | "Add a function `<name>(args) -> return_type` to the loft standard library. It should `<description>`. Implement it in `default/<file>.loft` if possible, or in `src/native.rs` if it needs access to State internals. Write a test in `tests/docs/<file>.loft`." |
| Understand a subsystem | "Explain how `<subsystem>` works, focusing on `<specific concern>`. Read the relevant source files before answering." |

### What Claude does well in this codebase

- Tracing the pipeline from a loft source construct to the emitted bytecode.
- Identifying which of the five parser submodules owns a given grammar rule.
- Finding the matching `value_code` branch in `codegen.rs` for a given IR node.
- Writing `tests/testing.rs` style tests from a loft code example.
- Diagnosing `validate_slots` conflicts from the panic output.
- Reading `LOFT_LOG=minimal` traces to identify the wrong opcode or bad stack state.

### What to watch for

- **Cascade errors.** A change to the IR (`src/data.rs`) cascades to the parser,
  codegen, scope analysis, debug printer, and stack tracker. Ask Claude to enumerate
  all affected files before making an IR change.
- **Generated file edits.** Claude may propose edits directly to `src/fill.rs`.
  Redirect it: "That file is generated. Modify `src/generation.rs` instead."
- **Over-engineering.** Claude sometimes proposes helper structs, traits, or
  abstractions for one-time operations. Push back: "The simplest change that passes
  the tests is preferred. No new abstractions unless they are immediately reused."
- **Pass confusion.** Claude may generate IR in pass 1. Ask it to check: "Does this
  code have a `first_pass` guard where it needs one?"
- **Missing the negative test.** Claude often writes positive tests first and forgets
  the misuse/error test. Remind it: "Also write the `parse_errors` test for the
  error case."

### Keeping sessions from going in circles

If a session has produced two or more failed attempts at the same fix, stop and:

1. Ask Claude to state what it currently believes is the root cause.
2. Compare that belief to the actual failure evidence (panic, log output).
3. If they do not match, correct the belief before asking for another implementation.
4. If they match but the fix keeps failing, ask Claude to try a different approach
   rather than iterating on the same approach.

Debugging loops usually mean the root cause hypothesis is wrong, not that the
implementation of the fix is wrong.

---

## Prompts guide

A reference for when to use each prompt, what each achieves, and where to be careful.

---

### Session start

#### `remember to always review CLAUDE.md at the project root before starting work`
**When:** Every new session before doing any real work.
**What it does:** Forces Claude to orient itself before acting — CLAUDE.md is the single entry point containing the project overview, execution path, key data structures, conventions, branch policy, and the full documentation index.
**Caveats:** Redundant if the session opened with an auto-memory summary that already loaded the relevant context. Adds token cost up front but prevents much larger recovery cost from acting on wrong assumptions.

#### `remember to always update the doc/claude/ documentation after resolving any issue`
**When:** Pair with any task prompt to reinforce the documentation discipline.
**What it does:** Acts as a standing instruction so that doc updates don't get skipped when a fix feels "small."
**Caveats:** On its own it does nothing — it only has value alongside a concrete task. Can cause over-documentation of trivial fixes if applied too literally.

---

### Often — documentation health

#### `Validate the doc/claude documentation for completeness and functionality for developing the application.`
**When:** After a batch of code changes (features or bug fixes) that may have outpaced the docs. Good to run every few sessions.
**What it does:** Claude reads all `doc/claude/*.md` files, cross-checks them against the current source code, and flags anything stale, missing, or misleading.
**Caveats:** Quality depends on how much of the codebase Claude can scan in one context window. False gaps are possible when a feature is real but the source is buried in a file Claude didn't reach. Best to follow up by reading any flagged files directly.

#### `Optimize the doc/claude documentation files for discoverability and efficiency. Cleanup the parts with almost no value.`
**When:** When the docs feel bloated or when navigation between files has become slow. Run after "Validate" above to avoid conflating missing content with low-value content.
**What it does:** Removes repetition, shortens verbose sections, adds cross-links, and may consolidate small files.
**Caveats:** Claude may delete content it cannot immediately contextualise. Always review deletions before committing — something that looks low-value from the surface may be the only record of a subtle design decision.

---

### Session — active development

#### `Continue with one issue for the 1.0 release, state what you chose and why, validate the resulting changes against CODE.md and update all documentation with the resulting changes.`
**When:** The primary work prompt. Use at the start of a focused development session.
**What it does:** Claude reads `PLANNING.md` and `PROBLEMS.md`, picks the highest-priority actionable issue, explains the choice, implements the fix, runs tests, validates against `CODE.md`, and updates all affected docs.
**Caveats:**
- If `PLANNING.md` priorities are unclear, Claude may pick a lower-impact item. Skim the pick before approving.
- Long sessions (complex fixes + full doc update) risk hitting the context limit before documentation is written. Consider splitting: one session for the fix, a separate "Finally" session for the docs.
- The "validate against CODE.md" step is only as good as CODE.md itself — keep it current.

#### `Evaluate if better temporary logging, an optional internal analysis tool or boundary checking would help by investigating this issue.`
**When:** Prospectively, at the start of a debugging session before committing to a fix strategy.
**What it does:** Claude reads the issue and surrounding code, then proposes whether adding `LOFT_LOG` modes, a `State::validate_*` helper, or an assertion would surface the bug faster than direct code reading.
**Caveats:** Generates suggestions that may not be implemented, creating planning overhead. Most useful when the bug is non-deterministic or deep in the execution trace; skip for obvious one-liner fixes.

#### `Evaluate if better temporary logging, an optional internal analysis tool or boundary checking would have helped by investigating the last issue.`
**When:** Retrospectively, immediately after closing a bug, while the fix is still fresh in context.
**What it does:** Mines the completed fix for observability lessons — what was hard to see, what would have made the root cause obvious faster.
**Caveats:** Only useful if the finding gets written to `PROBLEMS.md` or `TESTING.md`; otherwise the lesson evaporates. Results tend to be generic ("more logging would have helped") unless the bug had a specific bottleneck.

---

### Finally — end-of-session cleanup

#### `Update and cleanup the doc/claude/*.md files with the latest relevant findings. Evaluate this with user documentation in tests/docs/ and doc/ too.`
**When:** At the end of any session that changed behaviour, fixed a bug, or added a feature.
**What it does:** Syncs all internal Claude docs and user-facing HTML docs with what was learned. Cross-references findings between the two doc trees.
**Caveats:** If run while Claude is already near its context limit the updates will be shallow. Better to run this as its own fresh session immediately after the development session ends. Do not skip it — most project knowledge lives only here.

#### `Validate if everything that needs testing is in the tests/loft tests.`
**When:** After adding a feature or closing a bug, to check the test gap wasn't left open.
**What it does:** Claude reads `tests/docs/` and `tests/scripts/` and checks that the new behaviour has direct test coverage.
**Caveats:** "Everything that needs testing" is subjective. Claude may not infer edge cases that only become apparent at runtime. Best combined with a manual review of the changed files.

#### `add unit tests on similar cases as the last fixed issue`
**When:** After a bug fix, when a clear minimal reproducer existed.
**What it does:** Creates regression tests in the same style as the issue's test case, covering the boundary conditions near the fix.
**Caveats:** Only valuable when the fix has a clear, reproducible input. If the bug was a heisenbug or timing-related, the resulting tests may be trivially passing without actually guarding the fix. Review generated tests for whether they actually fail without the fix.

---

### Occasionally — project health

#### `Validate the project to the coding standards, try to remove any clippy ignored annotations.`
**When:** Periodically (e.g. before a release), not after every session.
**What it does:** Runs `cargo clippy`, reviews any `#[allow(...)]` annotations, and either fixes the underlying issue or documents why the annotation is justified.
**Caveats:** Removing a `#[allow(...)]` can expose genuine issues that require deeper fixes — this is not a quick pass. Budget a full session. Confirm each removal compiles and tests pass.

#### `Evaluate splitting source files when logical.`
**When:** When a source file has grown visibly unwieldy (>800–1000 lines) or has clearly separable concerns.
**What it does:** Claude identifies the natural split point, proposes new filenames, and can execute the split including updating all `mod` and `use` statements.
**Caveats:** High blast radius — file splits touch many import paths throughout the codebase. Best done with a specific file in mind rather than as a global sweep. Always run the full test suite immediately after.

#### `Validate the project against CODE.md`
**When:** Before a release, or after a large refactor where conventions may have drifted.
**What it does:** Audits naming conventions, function length, doc comments, and clippy usage against the rules in `CODE.md`.
**Caveats:** Produces a report, not automatic fixes. Acting on the findings is a separate effort. The audit is only as good as `CODE.md` — if the rules are outdated, the report will be misleading.

#### `Validate pdf creation for problems.`
**When:** After any change to `doc/*.html` files, `gendoc.rs`, or `doc/loft-reference.typ`.
**What it does:** Runs `cargo run --bin gendoc` and `typst compile`, checks for render errors, broken cross-references, or content gaps in the PDF.
**Caveats:** Requires `typst` installed locally. Font warnings (missing Liberation Serif / DejaVu Serif) are normal and non-blocking. A clean compile does not mean the layout looks good — visually inspect a few pages of the PDF.

#### `Update the documentation in doc/*.html as generated via doc/suite/*.loft make it useful for entry level programmers with the explanation of key concepts with clear and inviting examples.`
**When:** When user-facing docs have fallen behind the language's current feature set, or when new language features lack examples.
**What it does:** Claude rewrites or extends the HTML topic pages in `doc/` with clearer explanations and beginner-friendly examples that match the examples in `tests/docs/`.
**Caveats:** "Entry level" framing can cause oversimplification — some language semantics (reference lifetimes, LIFO invariant, store ownership) are genuinely complex and must not be glossed over. Always review generated examples by running them through the interpreter.

#### `Identify less important tests in tests/*.rs files to remove, they should at least be covered in tests/scripts.`
**When:** When the test suite has become slow or hard to navigate due to accumulated redundancy.
**What it does:** Claude reads `tests/*.rs` and `tests/scripts/`, identifies Rust-level tests whose coverage is fully replicated in the script-based tests, and proposes removals.
**Caveats:** Removing tests is risky. Claude may label tests "duplicate" when they cover different execution paths or rely on different infrastructure. Review every proposed removal against its actual coverage before deleting. When in doubt, keep the test.

#### `Evaluate the overall code quality of the project, how comprehensible are the function names and module names, are the functions in the correct source file.`
**When:** Periodically, especially after rapid growth that added functions without much architectural review.
**What it does:** Produces a readability and locality report: functions that should be in a different module, names that are ambiguous, modules whose scope has grown unclear.
**Caveats:** Produces a report only; renaming and moving functions is a separate high-effort task. Renames propagate broadly across the codebase — treat the report as input to a deliberate refactor session, not an automatic fix list.

#### `verify the planning and problems for priorities, new steps and clean up the done items.`
**When:** At the start of a planning cycle, or after a productive batch of fixes.
**What it does:** Reads `PLANNING.md` and `PROBLEMS.md`, re-orders items by current priority, removes completed items (delete entirely, no "done" markers), and identifies any newly discovered issues that should be captured.
**Caveats:** Claude may incorrectly mark items as resolved if it cannot verify the fix is complete. Always cross-check against `git log` and the test suite. The "clean up done items" step is irreversible — confirm before committing the changes.

#### `Find spots where the runtime implementation or functions can be made more optimal, though weigh that versus the extra cost in terms of more code or maintainability. Document these results.`
**When:** When profiling has identified hot paths, or before a release where performance matters.
**What it does:** Claude reads `fill.rs`, `state/`, and `database/` looking for algorithmic inefficiencies, unnecessary allocations, or repeated work. Results go to `OPTIMISATIONS.md`.
**Caveats:** Micro-optimisations identified by static reading may not match what a profiler shows at runtime. Treat the output as a hypothesis list. Claude tends to over-recommend optimisations; push back on anything that materially increases complexity.

#### `Find ways to make the full test run more efficient but still as useful.`
**When:** When `cargo test` is noticeably slow and slowing the feedback loop.
**What it does:** Identifies redundant test setup, tests that could be parallelised, or slow integration tests that could be replaced with targeted unit tests.
**Caveats:** Speed and coverage are in tension. Claude may recommend removing the slowest tests, which are often the most valuable end-to-end ones. Any suggested removal must be evaluated against what would be missed.

#### `Is there an obvious place to simplify the code with the same overall algorithm.`
**When:** After a feature or fix has left the surrounding code more complex than necessary.
**What it does:** Looks for duplicated logic, over-engineered abstractions, or code that has been patched multiple times and could be rewritten cleanly.
**Caveats:** "Same overall algorithm" is the important constraint — confirm that any simplification doesn't change observable behavior for edge inputs. Run the full test suite; consider adding a test that captures the before/after behavior.

---

### Standalone / one-off prompts

#### `/btw status`
**When:** Any time you want a quick read on current project health.
**What it does:** Invokes the `/btw` skill which summarises open issues, test status, and recent progress.
**Caveats:** Output quality depends on how much context is loaded. In a fresh session without a summary, Claude may read `PROBLEMS.md` and `PLANNING.md` superficially. Most useful mid-session when context is already warm.

#### `Always /clear between steps`
**When:** When executing a multi-step sequence from this file.
**What it does:** Resets context between steps so each prompt runs with a clean slate and doesn't carry stale state from a previous step.
**Caveats:** Clearing loses cross-step context. If step N's output is needed by step N+1 (e.g. a fix followed by "add tests for the last fix"), do **not** clear between them. Use `/clear` only between genuinely independent steps.

#### `update tests/scripts/ with anything that should be tested even when already existing elsewhere`
**When:** After adding a language feature, to make the script-based test suite self-contained.
**What it does:** Adds test scripts for all new behaviour, even when a Rust-level test already covers the same path. Ensures `tests/scripts/` is a complete standalone test suite.
**Caveats:** "Even when already existing elsewhere" is intentional redundancy — it means duplicate coverage is acceptable here. This can make the test run slower; acceptable tradeoff for coverage completeness.

