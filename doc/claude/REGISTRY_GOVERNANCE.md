
# Registry governance

Procedures for adding third-party libraries to the package registry and for responding to problems in listed packages.  Split out of [PACKAGES.md](PACKAGES.md); the registry design is [PKG_REGISTRY.md](PKG_REGISTRY.md).

---

# Registry Governance

Procedures for adding third-party libraries to the central Loft package registry
and for responding when problems are discovered in listed packages.

> **What shipped**: the registry is the signed `index.json` in `loft-lang/registry`, and a
> yank sets the version's `yanked` field ([LIBRARY_PUBLISH.md § 5b](LIBRARY_PUBLISH.md)).
> The procedures below were written for the earlier `registry.txt` design; their roles and
> rules still hold, and where they name `registry.txt`, read `index.json`.

The registry is a plain text file (`registry.txt`) maintained in a GitHub
repository.  It starts as a personal repository (`loft-lang/registry`)
and can migrate to a shared GitHub organisation (`loft-lang/registry` or
similar) when the community grows to the point where one person cannot handle
the review load alone.  Both hosting models are described here.  The file
format is described in [PKG_REGISTRY.md](PKG_REGISTRY.md).  This document governs who
may add entries and what happens when an entry must be restricted or removed.

---

## Contents
- [Principles](#principles)
- [Shared Registry Hosting](#shared-registry-hosting)
- [Registry Format — Extended Fields](#registry-format--extended-fields)
- [Submission Requirements](#submission-requirements)
- [Review Checklist](#review-checklist)
- [Approval Workflow](#approval-workflow)
- [Native Package Track](#native-package-track)
- [Problem Reporting](#problem-reporting)
- [Severity Classification](#severity-classification)
- [Response Procedures](#response-procedures)
- [Yanking and Deprecation](#yanking-and-deprecation)
- [Author Appeals](#author-appeals)
- [Registry Maintainer Responsibilities](#registry-maintainer-responsibilities)

---

## Principles

1. **Source-visible** — every registered package must have a publicly readable
   source repository.  Binary-only packages are not accepted.
2. **Fast to restrict** — yanking a package is a one-line edit to `registry.txt`
   and takes effect immediately for new installs.  Security response must not
   be slowed by process.
3. **Proportionate** — minor bugs do not trigger yanks.  The response matches
   the severity.
4. **Stable URLs** — a registered URL for a specific version must never change.
   If the file moves, a new version entry is added.  Old entries are not edited.
5. **Scalable authority** — the process starts with one person and scales to a
   small team without changing the rules.  Any single Maintainer may approve a
   submission or act on a security report; consensus is not required for routine
   work.  Policy changes require team discussion.  See
   [Shared Registry Hosting](#shared-registry-hosting).

---

## Shared Registry Hosting

### Solo model (starting point)

The registry begins as a personal repository owned by the project author
(`loft-lang/registry`).  One person handles all submissions, yanks, and
deprecations.  The compiled-in `source:` URL in the interpreter points here.

This model works for a small package ecosystem.  When the submission queue
regularly takes more than one person can process within the response windows,
it is time to migrate to the team model.

### Team model — GitHub organisation

Create a GitHub organisation (e.g. `loft-lang`) and transfer the repository
to `loft-lang/registry`.  Update the compiled-in `source:` URL in
`src/registry.rs` and the official registry file header at the same time.
Users who run `loft registry sync` will pick up the new URL on their next sync;
no interpreter release is required.

#### Roles

| Role | Count | Permissions |
|------|-------|-------------|
| **Admin** | 1–2 | Add/remove Maintainers; change branch protection; modify this governance document |
| **Maintainer** | 2–5 | Approve submissions; yank/deprecate entries; merge PRs to `registry.txt` |
| **Reviewer** | optional | Review pull requests and issues; no merge permission |

**Reviewer** is an informal role — anyone with a GitHub account can comment on
submission issues.  The label is used in issue assignment to acknowledge people
who contribute reviews without holding Maintainer rights.

#### How decisions are made

- **Routine submissions** — any single Maintainer may approve after the review
  period.  No consensus or second approval is required.  First available
  Maintainer picks up the issue.
- **P0 yanks** — any single Maintainer may yank immediately without consulting
  others.  They notify the rest of the team via a comment on the yank commit or
  a GitHub team mention as soon as they act.
- **Rejections** — any single Maintainer may reject.  The author may re-open
  the issue and request that a different Maintainer review if they believe the
  rejection was incorrect.
- **Policy changes** (to this document) — a pull request, open for at least
  7 days, visible to all Maintainers.  No objection from any Maintainer within
  that window constitutes approval.  Objections must be resolved before merging.
- **Team membership** — Admin only.  A new member is added when nominated by
  any Maintainer and no existing Maintainer objects within 7 days.

#### Load balancing

Issues are self-assigned: any Maintainer picks up an unassigned submission.
If a submission sits unassigned for 4 days, GitHub's stale-issue bot pings
the team.  Maintainers are encouraged to claim issues they have domain knowledge
in (e.g. graphics Maintainer reviews graphics packages).

A rotating on-call schedule for P0/P1 security reports is optional but
recommended when the team reaches 3 or more members: one Maintainer per week is
designated as the primary responder for that week's urgent reports.

#### Joining the team

A person is eligible when they have:

1. Contributed at least **3 substantive reviews** on submission or problem
   issues in the registry repository (comments that check requirements, test
   the package, or identify concerns — not just "+1").
2. Been nominated by any existing Maintainer in a GitHub issue titled
   `Team nomination: <handle>`.
3. Received no objection from existing Maintainers within 7 days.

An Admin then adds the person to the GitHub team.  No vote is taken; silence
is consent.

#### Leaving the team

- **Voluntary** — open an issue or message an Admin.  Access is removed promptly.
- **Inactive** — a Maintainer with no review activity for **6 months** receives
  a 30-day notice issue.  If no activity follows, their Maintainer access is
  downgraded to Reviewer by an Admin.  They can rejoin the Maintainer role by
  resuming activity and requesting re-elevation from any Admin.

#### Branch protection settings (recommended)

```
Branch: main
  Require pull request before merging: ON
  Required approvals: 1
  Dismiss stale reviews: ON
  Allow specified actors to push directly: Maintainers (for P0 emergency yanks)
```

Allowing Maintainers to bypass the PR requirement exists solely for P0 yanks
where speed matters more than process.  Every direct push must include a
comment on the registry issue explaining the urgency.

#### Conflict resolution

If two Maintainers disagree on a submission decision:

1. Either may request a second Maintainer review by posting `@loft-lang/maintainers please review`.
2. If a third Maintainer agrees with one side, that side prevails.
3. If the team is evenly split and cannot resolve within 14 days, the submission
   is held and the author is notified.  The team writes up the specific concern
   in the issue so the author can address it directly.

For severity disputes on problem reports, the higher severity always wins
initially: it is safer to over-restrict and loosen later than the reverse.

---

## Registry Format — Extended Fields

The base format (`name version url`) is extended with an optional fourth field
to record governance status:

```
# name  version  url  [status[:detail]]
graphics  0.2.0  https://example.com/graphics-0.2.0.zip
graphics  0.1.0  https://example.com/graphics-0.1.0.zip  yanked:CVE-2026-001
opengl    0.1.0  https://example.com/opengl-0.1.0.zip    deprecated:use-graphics
math      1.0.0  https://example.com/math-1.0.0.zip      yanked:malicious
```

### Status values

| Status | Meaning |
|--------|---------|
| *(absent)* | Active — installable without warning |
| `deprecated:<reason>` | Installable but warns; excluded from "latest" selection |
| `yanked:<reason>` | Not installable; excluded from "latest"; existing installs unaffected |

The `reason` field is a short slug used in diagnostics.  It may reference a
CVE identifier, a GitHub issue number, or a brief human-readable label.

### Installer behaviour

| User action | Active | Deprecated | Yanked |
|-------------|--------|------------|--------|
| `install name` (latest) | installs | skipped — next active version is used | skipped |
| `install name@version` (exact) | installs | installs + warning | fails with reason |
| Existing install | works | works | works (no change to local files) |

When a deprecated version is the only available version:

```
warning: graphics 0.1.0 is deprecated (use-graphics).
  No other version is available.  Installing deprecated version.
```

---

## Submission Requirements

A library is eligible for submission if all of the following are true:

### Required for all packages

- **Public source repository** — hosted on GitHub, GitLab, Codeberg, or similar.
  The URL must be provided in the submission issue.
- **Open-source licence** — any OSI-approved licence is accepted.  The licence
  must appear in the repository root (`LICENSE`, `LICENSE.md`, or `COPYING`).
- **`loft.toml` with `name` and `version`** — both fields must be present and
  match the proposed registry entry.
- **Reproducible tests** — `loft --tests <pkg>/tests/` must pass cleanly on the
  submitter's platform.  Test output must be included in the submission.
- **Stable download URL** — the `.zip` URL must remain permanently accessible.
  GitHub release assets, tagged archives, or static file hosting are all
  acceptable.  Direct repository archive URLs (e.g. `github.com/.../archive/`)
  are *not* acceptable because their content can change silently.
- **No name collision** — the package name must not duplicate an existing
  registry entry (including deprecated entries).  If the intent is to supersede
  a deprecated package, contact the maintainer before submitting.

### Additional requirements for native packages

Native packages ship compiled shared libraries and execute arbitrary code inside
the interpreter process.  They require extra scrutiny:

- **Rust source only** — native extensions must be written in Rust.  Pre-compiled
  blobs with no corresponding source are rejected.
- **No `unsafe` outside the plugin boundary** — `unsafe` is permitted only in
  the `loft_register_v1` entry point and in direct FFI calls to platform APIs.
  All other Rust code must be safe.
- **Dependency audit** — the submission must list all crate dependencies and
  their versions.  Dependencies with known CVEs at submission time are a
  blocking issue.
- **Explicit capability declaration** — the submission must state clearly what
  system resources the native code accesses (network, filesystem, GPU, audio,
  etc.).  This is informational, not restrictive, but must be accurate.

---

## Review Checklist

The maintainer works through this checklist before approving:

### Pure-loft packages

- [ ] Source repository is public and readable
- [ ] Licence file is present and OSI-approved
- [ ] `loft.toml` fields `name` and `version` match the submission
- [ ] Download URL is stable (not a mutable archive URL)
- [ ] `loft --tests` passes (submitter-provided output reviewed)
- [ ] No name collision with existing registry entries
- [ ] Package description in the issue makes the purpose clear
- [ ] Package does not re-implement a core stdlib function
      (acceptable if it extends or specialises it)

### Native packages (all of the above, plus)

- [ ] Rust source is public and the entry point matches `loft_register_v1`
- [ ] `unsafe` is confined to the registration entry point and FFI calls
- [ ] Cargo.toml dependencies list reviewed; no known-vulnerable versions
- [ ] Capability declaration matches what the code actually does
- [ ] At least one reviewer other than the submitter has read the Rust source
      (the maintainer counts; community review is welcome but not required)

---

## Approval Workflow

### Step 1 — Open a submission issue

The package author opens a GitHub issue in the registry repository
(`loft-lang/registry` or `loft-lang/registry` if the team model is active)
using the **Package Submission** template.  Required fields:

- Package name and version
- Download URL (the exact `.zip` URL)
- Source repository URL
- Licence identifier (e.g. `MIT`, `Apache-2.0`, `LGPL-3.0-or-later`)
- Brief description (1–3 sentences)
- Test output paste or link to a CI run
- For native packages: capability declaration and dependency list

### Step 2 — Community review period

The issue remains open for **7 calendar days** before the maintainer makes a
decision.  Community members may:

- Report concerns (security, name confusion, licence issues)
- Confirm they tested the package successfully
- Suggest improvements to the submission

The 7-day period may be waived by any Maintainer for:
- A patch to an already-approved package (same name, new version)
- A dependency of an already-approved package

In the team model, any available Maintainer self-assigns the issue within
4 days of it being opened.  If no one self-assigns, GitHub's stale bot pings
the team.

### Step 3 — Maintainer decision

After the review period any Maintainer may act:

- **Approves** — adds the entry to `registry.txt` via a pull request, closes
  the issue with a link to the commit.
- **Requests changes** — lists specific blockers in the issue.  The author
  addresses them and re-requests review.  The same or a different Maintainer
  may handle the follow-up.  A new 7-day period does not restart unless the
  Maintainer judges the concerns were substantial.
- **Rejects** — closes the issue with a written reason.  Rejection reasons
  include: name collision, licence incompatibility, fails to build or test,
  native package fails the safety checklist, or the package duplicates
  existing stdlib functionality without adding value.  The author may ask a
  different Maintainer to re-review if they believe the rejection was wrong.

### Step 4 — Ongoing versions

Once a package is approved, the author may add new versions by opening a
**New Version** issue (lighter template: URL + test output only).  The 7-day
period applies unless waived.  The maintainer verifies the `loft.toml` version
field increments monotonically and the URL is stable, then appends the new line.

---

## Native Package Track

Native packages (those with `#native` annotations and compiled shared libraries)
follow the same workflow but with a **14-day** review period and a mandatory
Rust source review.  The checklist item "at least one reviewer other than the
submitter has read the Rust source" must be satisfied before any Maintainer
approves.

**Solo model** — if no community reviewer steps forward in 14 days, the single
maintainer performs the source review alone.  This is acceptable for small
packages but uncomfortable for large or complex ones; such packages may be held.

**Team model** — the approving Maintainer must not be the sole reviewer of the
Rust source.  A second Maintainer or a community Reviewer must have commented
confirming they read the native code.  This cross-review requirement is the
primary reason native packages exist as a separate track: with a team, it is
always satisfiable without holding packages indefinitely.

---

## Problem Reporting

Anyone — user, security researcher, or package author — may report a problem by
opening a GitHub issue in the registry repository with the **Problem Report**
label.

Required information:

- Package name and affected versions
- Description of the problem
- Reproduction steps or proof of concept (for security issues: report privately
  first — see below)
- Suggested severity (the maintainer makes the final call)

### Security vulnerabilities — private disclosure

For security issues (malicious code, data exfiltration, privilege escalation,
or any issue where publishing reproduction steps could cause immediate harm),
report privately:

- Use GitHub's **private security advisory** feature on the registry repository
  (works for both the solo and team models — all Maintainers see it).
- Email any individual Maintainer whose address is on their GitHub profile if
  the advisory feature is not available.

Any single Maintainer who receives a credible private report will yank the
affected versions within **24 hours**, before any public disclosure, and notify
the rest of the team immediately after acting.  In the team model, the on-call
Maintainer (if a rotation is in place) is the primary recipient.

---

## Severity Classification

| Severity | Examples | Target response |
|----------|----------|-----------------|
| **P0 — Critical** | Malicious code, data exfiltration, remote code execution, supply-chain attack | Yank within 24 h; no discussion required |
| **P1 — High** | Data loss, crash in common use path, security issue without active exploit | Deprecate within 48 h; yank if no fix in 14 days |
| **P2 — Medium** | Incorrect output, API incompatibility with a published version, failed tests | Notify author; deprecate if no fix in 30 days |
| **P3 — Low** | Documentation error, minor edge-case bug, cosmetic issue | Notify author; no forced action |

Severity is assigned by the maintainer after reviewing the report.  The reporter's
suggested severity is taken as input, not as binding.

---

## Response Procedures

### P0 — Critical

1. **Any single Maintainer** yanks all affected versions immediately — a
   direct push to `registry.txt` is allowed under branch protection for exactly
   this case.  No approval from other Maintainers is needed; speed is paramount.
2. The acting Maintainer posts a team notification (GitHub team mention or email)
   within 1 hour of the yank explaining what was done and why.
3. A public issue is opened describing the problem at a high level (no exploit
   details if not yet public).
4. If the author is reachable and acting in good faith, they are given
   opportunity to release a fixed version before the public issue is opened.
   This window is at most **24 hours**.
5. If the package was malicious or the author is unresponsive, the package is
   permanently removed from the registry (all versions yanked with
   `yanked:malicious` or `yanked:removed`).
6. The public issue references the yank commit and summarises the nature of the
   problem.

### P1 — High

1. Maintainer marks affected versions `deprecated:<issue-number>` within 48 h.
2. Maintainer notifies the package author via the GitHub issue and, if possible,
   via the source repository's issue tracker.
3. Author has **14 days** to release a patched version.
4. If a fix is released and passes the review checklist, the patch version is
   added to the registry and the deprecation reason updated to point to it.
5. If no fix appears in 14 days, the affected versions are yanked.

### P2 — Medium

1. A GitHub issue is opened in the registry repository referencing the problem.
2. The package author is tagged and has **30 days** to respond.
3. If a fix is released within 30 days, the new version is added normally and
   the issue is closed.
4. If no response or fix within 30 days, the affected versions are deprecated.
5. If 60 days pass with no fix, the affected versions are yanked.

### P3 — Low

1. The issue is opened and the author is notified.
2. No forced action.  The issue remains open until the author fixes it or
   closes it as "won't fix".
3. The maintainer may add a deprecation comment in the issue if the bug causes
   significant confusion, but registry entries are not changed.

---

## Yanking and Deprecation

### What yanking does

- The status field for the entry in `registry.txt` changes to `yanked:<reason>`.
- `loft install name` (latest) skips yanked entries.
- `loft install name@version` for a yanked version fails with the reason:
  ```
  error: graphics 0.1.0 has been yanked (CVE-2026-001).
    Install a different version or check the project repository for a fix.
  ```
- Existing local installations are **not removed**.  Yanking affects new installs only.
- A yanked entry is never removed from `registry.txt` entirely — the line
  remains so that users who already have that version can understand why it is
  flagged.

### What deprecation does

- The status field changes to `deprecated:<reason>`.
- `loft install name` (latest) skips deprecated entries and selects the next
  active version.  If no active version exists, the deprecated one is installed
  with a warning.
- `loft install name@version` installs the deprecated version with a warning:
  ```
  warning: graphics 0.1.0 is deprecated (outdated).
    Consider upgrading to graphics 0.2.0.
  ```
- Existing installations are unaffected.

### Permanent removal

In cases of confirmed malicious packages, the entry status is set to
`yanked:removed` and a note is added to the registry changelog.  The URL field
is replaced with a placeholder (`-`) so no download is possible even if a user
edits the status field manually.

---

## Author Appeals

If a package author believes a yank or deprecation was applied incorrectly:

1. Open a GitHub issue in the registry repository titled
   `Appeal: <package> <version>`.
2. Explain why the action was incorrect and provide evidence (fixed code,
   misattributed CVE, etc.).
3. **Solo model** — the maintainer reviews within **7 days**, taking the
   reporter's argument at face value since there is no second opinion available.
4. **Team model** — the appeal is reviewed by a Maintainer who was *not*
   involved in the original decision.  This separation is one of the concrete
   benefits of the team model: appeals are not judged by the person being
   challenged.  Resolution within **7 days**.
5. If the appeal is upheld, the status is removed or changed and a new version
   is added if appropriate.
6. P0 yanks (malicious code) are not subject to appeal.

---

## User-Side Verification

Users can check their installed packages against the latest registry at any time
using two commands (see [PKG_REGISTRY.md](PKG_REGISTRY.md)):

```sh
loft registry sync     # pull latest registry.txt from GitHub
loft registry check    # compare installed packages against registry
```

`loft registry check` exits with code 1 if any installed package is yanked,
making it usable as a CI gate:

```sh
# In a CI pipeline — fails if any yanked package is installed
loft registry sync && loft registry check
```

Typical output when a yank is relevant to the user:

```
  utils  0.3.0  YANKED  CVE-2026-001 — run: loft install utils
```

The staleness warning (registry older than 7 days) reminds users to sync
regularly without being an error.

### How yanks reach users

1. Maintainer edits `registry.txt` — adds `yanked:<reason>` to the affected line.
2. The change is committed and pushed to `loft-lang/registry` on GitHub.
3. Any user who runs `loft registry sync` gets the updated file immediately.
4. `loft registry check` then surfaces the yank in the terminal and in CI.

No action is required from package authors or the loft interpreter itself to
propagate the yank — the registry file is the single source of truth.

---

## Registry Maintainer Responsibilities

These apply to every Maintainer regardless of model.

### Response times (shared commitment)

| Action | Target |
|--------|--------|
| Self-assign an open submission | 4 days |
| Complete submission review after review period | 14 days |
| P0 yank after credible private report | 24 hours |
| P1 deprecation decision | 48 hours |
| Appeal review | 7 days |

Response times are per-team, not per-individual — if the assigned Maintainer
cannot meet a deadline, any other Maintainer may step in.  In the solo model
these are personal commitments; in the team model they are collective ones.

### Record-keeping (all Maintainers)

- `registry.txt` is kept in a git repository with a public commit history.
  Every addition, yank, and deprecation is a traceable commit with the acting
  Maintainer's identity visible in `git log`.
- `REGISTRY_CHANGELOG.md` in the same repository summarises all yanks and
  deprecations in human-readable form, updated with every status change.
- Entries are never removed from `registry.txt` — only the `status` field is
  added.  The file is a permanent auditable record.

### Additional responsibilities in the team model

- **On-call rotation** — when the team has 3 or more Maintainers, maintain a
  weekly on-call schedule for P0/P1 responses.  The schedule is published in
  the repository's `MAINTAINERS.md`.
- **Monthly async review** — post a brief summary to the repository's GitHub
  Discussions each month: open submissions, recent yanks/deprecations, team
  membership changes.  This keeps all Maintainers informed even if they were
  not the ones acting.
- **MAINTAINERS.md** — keep a `MAINTAINERS.md` file in the registry repository
  listing current Maintainers, their GitHub handles, and (if applicable) which
  week they are on call.  Update it when membership changes.

### Stepping down as primary owner (solo → team migration)

When the solo maintainer decides to migrate to the team model:

1. Create the GitHub organisation and transfer the repository.
2. Invite 2–4 people who have already been reviewing submissions as community
   members; they become the first Maintainers.
3. Update the `source:` URL in the registry file and the compiled-in default
   in `src/registry.rs` in a coordinated interpreter patch release.
4. Publish a `REGISTRY_CHANGELOG.md` entry and a GitHub release note explaining
   the transition.

The original owner retains an Admin role in the organisation indefinitely,
but may reduce their Maintainer workload to match the team capacity.

---

## See also

- [PKG_REGISTRY.md](PKG_REGISTRY.md) — file format, install flow, version resolution, implementation
- [Library Package Format](PACKAGES.md#library-package-format) — package format + native extension design
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction) — extraction/migration of `lib/*/` into external repos
