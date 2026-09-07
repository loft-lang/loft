---
name: loft-plan-workflow
description: How to run plan-shaped work end to end — decide whether something even IS a plan, open one, cut its phases so each can fail on its own, close or defer it, and route a finding to the right repo. Use this whenever the task mentions a plan, a phase, a roadmap, an issue tracker for planned work, or promoting a design doc; whenever you are about to create a plan directory or close one; and whenever you are weighing "is this a plan or just a TODO?". Also use it in a consumer/dogfood repo deciding where an upstream defect belongs. Tree-agnostic method with one bindings section at the bottom.
user-invocable: false
---

## What this skill is — and what's portable

The methodology below is **tree-agnostic**: the way you decide *whether* work is a
plan, open it, run an investigation, and close it is identical in any project that
organizes multi-phase work as plans.  The concrete paths, templates, tracker, and
naming that bind it to *this* repository live in **one place** — [§ Bind to your
tree](#bind-to-your-tree) at the bottom.  Carrying this skill into another tree
means keeping the body unchanged and repointing that one section.

This skill is **procedural-only**.  Definitions, rationale, and templates live in
the source docs the bindings point at — not restated here.

**Cross-cuts:** branch / commit / push policy and bug-filing policy live in the
repo's master instructions (loft: `CLAUDE.md`).  This skill assumes you've already
followed those.

---

## Pick the lightest workflow that fits

The lightest path that holds the work is the right one.  Promote up a tier only
when the work genuinely needs it — most TODOs never become plans, even ones that
span several sessions.

| Work shape | Path |
|---|---|
| **Bug fix** (single root cause, fits one commit) | Fix + regression test + commit.  Reference the tracker issue if one exists; otherwise the fix + test *are* the record.  No plan, no open-work row, no archive entry. |
| **A defect in something UPSTREAM** (the engine, a library you consume) | File it in **that** repo, with a minimal repro.  **Never a plan here** — see [`references/consumer-projects.md`](references/consumer-projects.md). |
| **Tiny deliverable** (demo deploy, version bump) | One overview/roadmap row, or nothing.  No plan. |
| **Operational change** (CI tweak, doc fix) | Direct commit. |
| **Light TODO** *(the default)* — fits in one row of a reference doc's table | An `## Open work` section in the relevant reference doc.  Same lifecycle as a plan, just one row.  Edit that doc's architecture content directly when you implement; the row and the design share a file. |
| **Plan** — multi-phase, explicit phasing, design-before-build, cross-arc deps, or its own document space | A full plan directory.  Cap active plans at 2–3. |

The light flow is the default.  A plan earns its directory only when the work is
genuinely multi-phase **and** benefits from its own document space.  The sharpest
promote-up triggers (including the one-liner *"about to `#[ignore]` an existing
test"*) are in `plans/README.md § When a problem should escalate` — check it when
the tier is unclear.

---

## A plan's identity is its tracker issue

One model, stated once:

- **Identity = the issue number** in the shared plan tracker — *not* a local
  directory integer.  **File (or find) the issue FIRST, then name the directory after
  the number the tracker returns.**  Never pick the number by scanning existing plan
  directories: a sibling *branch* may already hold an unmerged `<N>-<slug>/` for that
  number (e.g. a `15-debugger/` on another branch while `main` shows only up to 14), so
  scanning mints a duplicate that collides the moment that branch merges — and the
  issue number, once GitHub assigns it, is immutable, so the collision is expensive to
  unwind.  The tracker's auto-incremented issue number is the one global, collision-free
  source of truth; the local directory tree is not.
- **The directory is flat**: `<id>-<slug>/`.  There are **no
  `future/`/`finished/`/`deferred/` subdirectories** — lifecycle **state lives on
  the issue** (a label), not in the path.
- **The tracker is the source of truth; `ROADMAP.md` is the reading view and IS
  maintained** — when a plan is added, completed, or its dependencies change, update
  its ROADMAP row (owner's standing practice; `plans/README.md § Light flow`).  Don't
  curate any OTHER parallel table of plans.

Everything below assumes this model.  A tree mid-migration will still have **legacy
plans** in the older layout — lifecycle subdirectories plus hand-maintained
overview rows.  Leave them where they are (or migrate opportunistically), but
author **new** plans in the model above; don't extend the legacy layout.  Your
tree's legacy specifics are in [§ Bind to your tree](#bind-to-your-tree).

---

## Opening a plan (standard shape)

Use when the deliverable is a feature ship or a fix landing.

1. **Identity — claim the issue FIRST.**  Create (or find) the tracker issue *before*
   the directory; the number it returns is the plan's id.  Do **not** derive the number
   from the local directory tree — that misses unmerged plans on sibling branches and
   mints colliding duplicates (see [§ A plan's identity](#a-plans-identity-is-its-tracker-issue)).
2. **A flat FILE** `<id>-<slug>.md` in the plans home, named for the returned issue
   number, is the default; a big design or a probe-bearing investigation gets the
   directory form `<id>-<slug>/` with a README from the standard template.
3. **Fill Status + Goal first.**  Add Sub-arcs / Phase ordering / Open questions /
   Cross-arc dependencies / See-also as the design clarifies.  Link the source
   issue(s) and carry the plan id in the body.
4. **Label the issue** — subject + status (`future` planned · `active` in
   progress · `finished`).  (No `plan` type label: in a dedicated plans repo
   every issue is a plan, so it partitioned nothing — retired 2026-06-14.)
5. **Keep `ROADMAP.md` in step** (add/adjust the plan's row), but no per-plan entry
   in the master-instructions index — the plan is discoverable via the tracker and
   the plans-overview doc.  Add a
   master-index entry *only* if the plan introduces a genuinely new top-level
   reference concept (vanishingly rare).

**Length budget: 100–300 lines.**  Longer means reference content is leaking into
the plan — extract it to its own reference doc.

**When NOT to use this shape:** if the plan's first phase is *characterize the
problem space* rather than *design + build*, use [`references/investigation-plans.md`](references/investigation-plans.md).

---

## Cutting a phase — two bounds, not one

Effort letters size a phase; they do not tell you where to cut it.  The rule that
does:

> **A phase should be as small as possible while STILL BEING VALIDATED.**
>
> **Upper bound (safety).** The old path and the new one can both run at once and be
> compared *exactly*.  If the only way to see whether it worked is to swap and look,
> the phase is too big.
>
> **Lower bound (validity).** The phase can go red **on its own, for a real reason**.
> If the only way to test it is to also do the next phase, they are one phase and
> splitting them buys a green tick on an empty claim.

So two questions per phase, and it has to pass both:

1. *When this phase is half done, what exactly am I comparing against?*  "Nothing, I
   look at it afterwards" means **too big** — a lump wearing a small phase's effort
   letter, whose failure mode is `git revert`.
2. *What would go red if I did this phase wrong?*  "Nothing until the next phase
   lands" means **too small** — merge it forward.

**The lower bound is the one that gets skipped**, because a phase that ends with
something built and *called by nobody* cannot fail — it is green by construction.
Splitting *"add the function"* from *"call it"* manufactures that state on purpose.
If the first half cannot go red, it was never a phase.

**A self-test is not validation.**  The discriminator is not *is there an assert*, it
is **could this assert ever be surprised**.  "The table exists and every key maps to
one entry" is checked against the table and cannot fail for any reason a reader cares
about.  A guard that cannot fire is the same defect wearing a different hat — verify
the guard is even compiled in and can go red before trusting a sweep that used it
(loft: `[profile.dev.package.loft]` sets `debug-assertions = false`, so a
`debug_assert` in library code is *absent* from every standard build).  This is the
planning-time face of the master instructions' matrix rule — *prove the harness can
fail; a no-output cell is vacuous* (loft: `CLAUDE.md § Debugging policy`).

**Three shapes that pass both bounds:**

- **Parallel run.** Build the new thing beside the old, compare exactly (bytes, IR,
  a histogram), *then* delete the old.
- **A probe first.** An XS phase whose only job is to try to falsify the design
  before anything is built on it — the cheapest phase in any plan, and the one that
  kills a bad design for the cost of a compile.
- **One call site at a time, each with its own comparison.**  "Wire four callers" is
  four phases, and each wants the same gate: the old call and the new call leave the
  same world.

**The comparison is the phase; the edit is the easy part.**  A three-line change that
alters behaviour under every caller is safe only because something written beside it
can see that.

---

## Closing or deferring a plan

**Close the moment the work is FINISHED — do NOT gate on merging to main.**  "Finished"
means design + build + tests + docs are all done (on the branch); at *that* moment the plan
closes.  A plan issue is a claim about the DESIGN being settled and delivered, not about the
commit having reached the trunk — the code lands on main later, on its own clock, usually
**batched** with other work.  So:

- The doc-side closing below (trim the README, move reference content, rewrite links, swap
  the label + close the issue) is **doc-only work that does not need the code on main** —
  do it when the work is finished.
- **Close plans in a batch, never a PR-per-plan.**  A full PR/CI cycle (CI_BUDGET.md's
  20-minute rule) to close a single plan is waste; bundle the closing doc-changes (across
  several plans, even unrelated) into the next substantive push
  (`doc/claude/DEVELOPMENT.md` § bundle many subjects into one PR).
- Because the close is a **hand-close** (issue closed before/independent of a `Closes
  @PLN<n>` PR merge), you MUST swap the status label yourself — see step 4's hand-close note.

**Pick the outcome first:**

- All phases shipped → **close**.
- Some/all phases paused with a **concrete trigger** → **defer** (Status table
  grows SHIPPED / DEFERRED rows; the deferred phases keep their full design
  content).  Label per `_LIFECYCLE.md`'s outcome table: `status:parked` if a floor
  shipped, `status:future` if nothing did.
- Paused with **no** concrete trigger → the design moves to the closed-by-decision
  register (`doc/claude/DESIGN_DECISIONS.md`), not a deferred state.

**For each shipped phase:**

1. **Tag** sections REFERENCE / CLOSURE-RECORD / HISTORICAL.
2. **Move reference content OUT** to its home doc — either create-and-move (a phase
   that grew a whole subsystem → its own reference doc) or trim-only (content
   already has a home → just delete the duplicate).
3. **Trim the README** to a lead `Status — DONE/SHIPPED <date>` line + a cross-link
   to where the reference content now lives.

**Common to close + defer:**

4. Reclassify any overview rows (shipped parts leave; deferred parts stay if
   tracked).  Set the lifecycle **state on the issue** (not a directory move):
   closing → swap `status:active` for `status:finished` **and close the issue**;
   deferring → `status:parked` when a floor SHIPPED and the rest is paused,
   `status:future` only when nothing shipped — the outcome table in
   `_LIFECYCLE.md` § Pick the outcome is the authority — issue stays open.  **Don't rely on
   GitHub's `Fixes #N` — it's same-repo only and can't reach the plans repo.**
   Instead the finishing PR carries a cross-repo close directive (`Closes
   @PLN<n>`); on merge to the trunk a close-on-merge workflow runs the repo's
   close-shipped-plans script to do the `status:finished` + close, with a
   stale-plan audit as the drift safety net.  The manual `gh issue` edit is the
   fallback / out-of-band path.
   - **A CLOSED plan's status label must be `status:finished` (delivered) or
     `status:declined` (de-scoped) — NEVER a live status (`active` / `future` /
     `next` / `closing`).**  The swap is automatic ONLY when the finishing PR used
     `Closes @PLN<n>`; a **hand-close** (the issue closed directly) or a PR that used
     **`Refs`** instead of `Closes` closes the issue but leaves the label stale, so
     you must swap it yourself: `gh issue edit <n> -R loft-lang/plans --remove-label
     status:<live> --add-label status:finished`.  This drifts silently — plans
     107-110 were all closed-but-mislabeled (`active`/`future`/`next`) from
     `Refs`-only or hand closes — so when you touch a closed plan, verify the label
     matches the state; don't trust the audit to have caught it.
5. **Grep + rewrite incoming links — THE most-skipped step.**  Reference content
   embedded in a finished plan gets linked to from other docs; those links rot when
   the content moves.  Grep every doc for the plan's path, rewrite the links to the
   new home, then run the repo's drift checker to catch what grep missed.
6. **Check the feature catalogue against what actually shipped** — see
   [`references/catalogue-check.md`](references/catalogue-check.md).  A plan that shipped, changed, renamed or removed a feature is not closed
   until that feature's own issue and labels describe the built thing.

---

## Promoting a reference doc to a plan

1. **Audit shipped status FIRST.**  A doc titled "design" often has shipped phases
   in its body.  Grep the body for `shipped|done|implemented|landed|completed` and
   check the tree for matching code *before* choosing a destination — misroute risk
   is real, and a mid-promotion status question from the user is the brake: apply it.
2. **Route by the audit:** mostly shipped → close it directly · mostly
   trigger-deferred → defer · genuinely-future + multi-phase → a plan ·
   genuinely-future + one row → leave the doc in place and add an `## Open work`
   section (the light flow).  Reference content with an open tail stays a doc + an
   `## Open work` section — a thin "pointer-plan" that only links back to a doc is
   over-engineering; don't create one.
3. If promoting to a full plan: move the doc into the flat plan directory, apply
   the opening steps from step 4 onward, and rewrite incoming links to the old path
   (same grep as closing, step 5).

---

## Transferable pitfalls

The body above states the procedure; these are the three judgement calls that are
easy to get wrong and are not implied by any step.

1. **Split when broad; single-file when bounded.**  A doc with broad
   intended-to-finish scope → split into focused files.  A doc with an explicit
   scope ceiling ("never going to ship past X") → one file with a status block.
2. **Named value categories beat numbered tiers.**  Categories that name the *kind*
   of value are stable across sessions; numbered tiers (V1/V2/V3) get re-ranked
   constantly.  Re-categorize only when scope actually changes.
3. **Never calendar-time language** in plans, roadmaps, or memory — "2–3 weeks"
   ships in 2 days and "quick fix" takes weeks.  Use effort letters.  Historical
   retrospectives that *document* the rule's validity are fine to keep.

---

## Filing bugs found during plan work

Two independent questions, and both must be answered before you file: **which repo
owns it** ([`references/consumer-projects.md`](references/consumer-projects.md))
and **is it already on the mainline** (below).

A tracker issue is a **claim about the mainline**.  So, working inside a plan:
**file a problem only when it reproduces *outside* the plan — already on the
mainline.**  A pre-existing mainline bug you stumble on during plan work gets filed
(and cross-linked to the plan); a breakage the plan's own in-progress work caused
is branch-internal — it lives in the plan's docs and is fixed on the branch, never
filed.  Investigation plans are the strongest case: the probes + cluster docs
already document every shape, so a separate issue would just double-document it.

Full policy in the master instructions (loft: `CLAUDE.md § Bug-filing policy`).

---

## Bind to your tree

Everything above is tree-agnostic.  This section is the **only** loft-specific part
— repoint these to port the skill to another tree.

| Generic term | loft binding |
|---|---|
| Plan tracker / issue id | [`loft-lang/plans`](https://github.com/loft-lang/plans/issues) issues; id = `@PLN<N>`; next free: `gh issue list -R loft-lang/plans --state all --limit 1` |
| Plan directory (new model) | `doc/claude/plans/<N>-<slug>/` — **library plans too**.  A single-file plan is `doc/claude/plans/<N>-<slug>.md`; one with companions gets the directory.  ⚠ **`doc/claude/lib_plans/` is CLOSED to new work** (its README, 2026-06-19): it is a legacy archive being migrated here, so read it and never add to it.  The drift is real — @PLN141 landed there 2026-08-18 and @PLN144–147 did too before being moved, both because this row used to say libraries went there. |
| Legacy layout (mid-migration) | only `doc/claude/plans/finished/<N>-<slug>/` remains (e.g. plan 51); ROADMAP.md rows are still maintained.  New plans use the flat/tracker model above. |
| Standard plan template | [`doc/claude/plans/_TEMPLATE.md`](../../../doc/claude/plans/_TEMPLATE.md) |
| Investigation template | [`doc/claude/plans/_INVESTIGATION_TEMPLATE.md`](../../../doc/claude/plans/_INVESTIGATION_TEMPLATE.md) — canonical example `plans/finished/51-hidden-buffer-aliasing/` (5 clusters; its README carries the probe census) |
| Close / defer procedure (full) | [`doc/claude/plans/_LIFECYCLE.md`](../../../doc/claude/plans/_LIFECYCLE.md) |
| Docs-vs-plans rule, three workflows, lifecycle, value categories | [`doc/claude/plans/README.md`](../../../doc/claude/plans/README.md) |
| Reference-doc `## Open work` homes | `NATIVE.md` / `PERFORMANCE.md` / `PACKAGES.md` / `QUALITY.md` |
| Value-category letters | `S/R/G/F/U/C/Q/N` — ROADMAP row tags, NOT tracker labels (definitions in `plans/README.md § Value categories`) |
| Feature catalogue (canonical) | [`loft-lang/features`](https://github.com/loft-lang/features/issues) issues; id = `@F<N>` (`kind:feature`) / `@I<N>` (`kind:infra`).  The ISSUE is the source; @PLN92 is the catalogue's own plan |
| Feature catalogue tags | `kind:feature` \| `kind:infra` — exactly one.  The wrong one files a language feature under infrastructure, where nobody looking for it will filter |
| Feature generated shadow (never hand-edit) | `index/features.json` + `doc/features/` + `tests/docs/features/*.loft` |
| Feature regenerate + drift guard | `make features-fetch && make features-gen`, then `make features-check` (fails on hand-edits or a stale shadow) |
| Feature example → test | the generator promotes the **first** ` ```loft ` fence in the issue body into a RUN test — put the runnable example first, never a teaching snippet |
| Feature coverage gate — and its blind spot | `scripts/feature_coverage.sh --check` + `scripts/feature_hygiene.sh --check` (CI), baselined in `scripts/.feature_coverage_baseline`.  It counts **uncataloged FILES**, so a new capability landing in an already-tagged file passes at baseline 0.  Measured 2026-08-04: gate green while ~12 shipped capabilities had no entry.  Green here means "no new untagged file", never "the catalogue is complete" |
| Drift checker | `scripts/check_doc_drift.sh` |
| Incoming-link grep (close/promote) | `./scripts/idx incoming:<path>` (preferred), or `grep -rn "plans/<NN>-<slug>" CLAUDE.md doc/claude/ --include="*.md"` |
| Investigation regression suite | `tests/scripts/NN-<slug>.loft` |
| Probe gates (leak / exit) | `LOFT_STORES=warn`; the `loft_suite` leak gate; both backends = interpreter + native |
| Execution modes to verify across | `--interpret` and `--native` |
| Canonical examples | partial defer: plan-28 / plan-12 · create-and-move close: `31-html-export → HTML_EXPORT.md` · trim-only close: `04-slot-assignment-redesign → SLOTS.md` |
| Consumer (dogfood) repos + their trackers | `moros` ([`jjstwerff/moros`](https://github.com/jjstwerff/moros/issues), `plans/<N>-<slug>/`, its own `plan-check` make target) · `dryopea` · `crawler` · `lib/markdown`.  Each keeps its own issue numbers and its own `plans/README.md` binding; the method above is shared verbatim |
| Where a consumer files an ENGINE defect | `loft-lang/loft` issues (`bug_report`), `sev:`/`area:` + a verified `wa:*` label — never a plan in the consumer repo |
| Branch / commit / bug-filing policy | `CLAUDE.md` |
| Full dev procedures (rebase, commit hygiene) | `doc/claude/DEVELOPMENT.md` |


---

## Reference files

Read one when you are in its situation; none is needed for the common path above.

| Read | When |
|---|---|
| [`references/investigation-plans.md`](references/investigation-plans.md) | The deliverable is mechanism understanding before fix design — probes, clusters, verified-vs-hypothesized accountability |
| [`references/consumer-projects.md`](references/consumer-projects.md) | You are in a project built ON the engine and need to know which repo a finding belongs to |
| [`references/catalogue-check.md`](references/catalogue-check.md) | The work adds, changes, renames or removes a feature whose tracker issue is the canonical description |
