<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN78 — implementation design: the toolchain is an artifact in the index we already have

**Status — DESIGN, 2026-07-31.**  Supersedes the phase sizing in
[README.md](README.md), which was written 2026-05-31, before the package trust
chain existed.  The threat model and trust-chain analysis there still stand and
are not repeated here.

## What changed under the plan

The plan sized five phases against a world with no signing infrastructure.  That
world is gone: building the *package* registry built almost every mechanism the
*binary* distribution needs.  Re-audited against the tree, not the prose:

| Phase | Plan's assumption | Today |
|---|---|---|
| 30.1 reproducible builds | to be built | **open, off the critical path** — `make-release.sh` emits `SHA256SUMS`, which is integrity, not byte-identical rebuild; homed at [RELEASE_PUBLISH.md § Reproducible builds](../../RELEASE_PUBLISH.md#reproducible-builds) |
| 30.2 signing + registry entries for binaries | to be built | **mechanism exists** — `registry_index::BinaryEntry { url, sha256, loft_ffi_fp }`, keyed per target triple, already downloaded and sha256-verified by `install.rs` for package cdylibs.  What is missing is an *entry*, not a mechanism |
| 30.3 `install.sh` | to be built | **open** — genuinely new (see § the two asymmetries) |
| 30.4 `loft self-update` | to be built | **half exists** — the verify half is `install.rs`'s existing path; the replace half is new |
| 30.5 key rotation + LTS | M, 2-3 days | **built** — `registry_keys.rs` embeds MULTIPLE Ed25519 trust roots and accepts a signature from any; rotation is "ship a release that adds the new key, ship a later one that drops the old" |
| dependency: @PLAN12 §6.7 advisory channel | blocking | **exists** — `registry_advisories.rs` (`Advisory`, `AdvisoryFeed`, `classify`) |

The plan's own words were *"30 + 6.7 together close the loop; either alone is
half a system."*  6.7 landed.  This is the other half, and it is much smaller
than 10–15 work-days because most of it is already running in production for
packages.

## The invariant

> **The loft toolchain is one more signed artifact in the index, verified by the
> same chokepoint every other artifact goes through.**

A case never tested behaves correctly for the same reason the tested ones do:
`install.rs` fetches a signature-verified index, reads a per-target
`BinaryEntry`, downloads it, and refuses it unless the sha256 matches.  A
toolchain binary is that, with a different destination.

## Re-assertion sites — the brittleness, counted before any code

The invariant must be re-stated at every site that admits an artifact.  Today:

| # | Site | Status |
|---|---|---|
| 1 | `install.rs` package tarball | exists, verified |
| 2 | `install.rs` prebuilt cdylib (`BinaryEntry`) | exists, verified |
| 3 | `loft self-update` | **new — must route through the same function, not a copy** |
| 4 | `install.sh` (bootstrap) | **new — CANNOT route through it: there is no loft yet** |

Site 3 is free if self-update calls the existing verify path; writing a second
sha256 check there is the failure mode, and it is silent (a wrong artifact
installs, nothing errors).  **Site 4 is the irreducible one** — a shell script
cannot verify a signed index.  The cure is not to make the shell smarter but to
shrink what it must be trusted for:

> `install.sh` carries **one pinned sha256** of the bootstrap binary and hands
> off immediately.  It never parses the index, never checks a signature, never
> chooses a version.  Its whole trust surface is one constant a reader can eyeball
> — and the first thing the installed binary does is verify itself against the
> signed index (step 2), so the shell's claim is checked by loft within seconds.

That drives `N × silence` down: sites 1–3 share one chokepoint, and site 4's
omission is loud (the pinned hash is the only thing it does).

## Where the invariant does NOT reach — the two asymmetries

Named because "the toolchain is just another package" is the elegant claim, and
elegant claims are where designs break.  Two places it genuinely does not hold:

1. **Bootstrap has no verifier.**  Installing a package happens *inside* loft;
   installing loft cannot.  Handled by the handoff above — not by pretending the
   shell can do what loft does.
2. **Self-replacement mutates the running program.**  No package install does
   this.  A running binary cannot be overwritten in place on Windows, and on Unix
   unlinking it while running is legal but leaves the process on the old inode.
   The mechanism is rename-then-replace, and it is the one genuinely new piece of
   engineering in this plan.

A third asymmetry is a non-issue and was checked rather than assumed:
`compare_semver` parses `major.minor.patch` as integers, so loft's **calendar**
versions (`2026.7.2`) order correctly with no special case.  `BinaryEntry`'s
`loft_ffi_fp` is a cdylib-ABI gate and must be absent for the toolchain entry.

## The steps

Each is independently landable, each is verified by the one before it, and
nothing is trusted before it is checkable.  Steps 1–3 cannot damage an
installation: they add data and read-only commands.

**Step 0 — remove the false claim (docs, no code).**  DONE 2026-07-31.
`LIBRARIES.md` stated that `loft install` / self-update verifies the release
sha256s.  Neither does: `install.rs` has no toolchain path, there is no registry
entry for the binary, and the sha256s are read from the release's `.zip.sha256`
sidecars purely to be displayed.  It now says what is true — verify by hand with
`sha256sum -c`, no automatic path yet — and points at this plan.  The sentence
lives in `scripts/gen-library-catalogue.py`, since `LIBRARIES.md` is generated and
not committed; the comment there says what to reword when steps 1–2 land.

`RELEASE.md` was checked and left alone: its two mentions sit in a release
checklist that the very next paragraph marks `[build]` (open work), so it
describes a target state and says so.  Correcting it would have been the
over-reach — it was already honest.
*Verify:* every surviving `self-update` mention names future work (`RELEASE.md`'s
open-work paragraph, `ROADMAP.md`'s @PLN78 row).

**Step 1 — make the client tolerate the entry BEFORE it exists.**  DONE 2026-07-31.

*This step was rewritten by its own probe.*  It read "publish a toolchain entry
— no client reads it yet, so this cannot break anything."  That was false, and
the cheapest check said so: `parse_index` propagated a package's parse error with
`?`, so ONE unreadable entry rejected the WHOLE index.  A healthy `regex` beside
one under-specified `loft` entry resolved *nothing*.  Publishing the entry first
would not have been inert — it would have taken registry access away from every
deployed loft until they upgraded.

That is a fragility independent of this plan: one index serves every client, so a
publishing mistake in any single package used to be an ecosystem-wide outage.
`parse_index` now **skips** an unreadable package, reports it on stderr, and
records it in `RegistryIndex::skipped` for a caller that must be strict (a publish
check should refuse to sign a malformed index).  Sound because the signature is
verified over the raw document *before* parsing, so skipping is a choice about
already-authenticated data and admits nothing an attacker controls.  Structural
damage — bad JSON, unsupported `schema_version` — stays fatal, since then nothing
in the document can be trusted to mean what it says.

*Verified:* the original probe now yields `PARSED — 1 packages: ["regex"]` with
the skip reported.  Three guards pin it: the malformed entry is skipped and named,
a clean index skips nothing (the control — a parser that skipped *everything*
would also pass the first), and a structurally broken index is still refused.

**Step 1b — publish the toolchain entry (owner action, cross-repo).**
Our-side work DONE 2026-07-31; what remains is the cross-repo submit + signature.

The open question was what artifact the version-level `url` / `sha256` / `size`
name, since a release is four per-target zips and none of them is "the release".
Answered by adding a fifth: `make-release.sh --source` emits
`loft-<v>-src.zip` via `git archive`, so a toolchain entry means exactly what a
package entry means — source, plus prebuilt binaries per target.  `git archive`
rather than a hand-rolled zip because it reads the object database, making the
bytes reproducible from the commit on any runner; and rather than GitHub's
auto-generated source tarballs, which are not guaranteed byte-stable and so cannot
carry a sha256 we publish.

The entry itself is **derived, not written**: `scripts/gen-toolchain-entry.py`
hashes the artifacts of the run that produced them, and `release.yml` attaches the
result to the draft.  A hand-copied entry is correct exactly once — the next
release changes four hashes, four URLs, a size and a date, and the failure mode of
getting it wrong is the worst kind available here: a *signed* index attesting to
last release's hashes, which fails at the user, far from the mistake.  The script
reads its target triples from `self_update::PUBLISHED_TRIPLES` rather than
restating them, and exits non-zero on a missing artifact rather than emitting an
entry that quietly omits a platform.  `loft_ffi_fp` is absent, as the probe
required.

*Verified:* `generated_toolchain_entry_parses_and_drives_self_update`
(`tests/mock_registry.rs`) runs the generator and puts its output through the real
`parse_index` and the real `self_update::plan` — the two things that consume it in
production — asserting an `Available` update for a published triple and
`NoBuildForTarget` for one we do not publish.  Shown non-vacuous by breaking the
generator's `published` field: the entry is then skipped and the test fails.  That
failure also confirms the step-1 property this depends on — a malformed toolchain
entry costs the *package*, not the index.

*Remaining (owner):* submit the generated entry to `loft-lang/registry` and re-sign.

**Step 2 — `loft verify-self` (read-only).**  DONE 2026-07-31, local half.

The design said "hash the running executable, compare against the published
entry".  What is published is the sha256 of the **zip**, not of the binary, and
after installation the user has an unpacked bundle — so there is nothing to
compare a running executable against until step 1b decides what artifact the
version-level entry names.  The half that does not depend on that is what shipped.

A release bundle carries two manifests: `stdlib.manifest` (a digest per
`default/*.loft` plus a `combined` trailer) and `SHA256SUMS` (every file,
`bin/loft` included).  `verify-self` checks the installation against both,
offline, and changes nothing.

**Both manifests ship inside the bundle they describe**, so they establish that
the installation is INTACT, not that it is AUTHENTIC — someone who replaced the
binary could rewrite the manifest beside it.  The output says exactly that, in
those words.  Reporting a bare "verified" would be the same species of claim step
0 just removed from the catalogue: a check that sounds like more than it did.

Intactness is worth having on its own, because it names the failure that actually
happens: a **partial upgrade** — a new `bin/loft` beside an old `default/`.  loft
resolves its stdlib at `<binary-dir>/../default`, so such an installation runs,
misbehaves subtly, and reads as a compiler bug.  `verify-self` names the file.

*Verified:* on a constructed release bundle it reports `stdlib: 6 file(s) match` /
`bundle: 8 file(s) match` and exits 0; editing one stdlib file makes it report
`FAILED  stdlib: 1 changed: default/01_code.loft` and exit 1 — the positive
control, since a check that cannot fail is not a check.  In a source checkout it
says "not a release bundle" and exits 0 rather than failing vacuously.  Seven unit
tests cover the manifest dialects, the intact / edited / missing cases, a manifest
path that tries to escape the bundle, and the dev-tree skip.

*The `--published` half landed 2026-07-31* — and needed a schema change to be
possible at all.  The published `sha256` covers the ZIP, which is verifiable
exactly once, at download; what a user runs is an unpacked directory.  So the
entry gained `binaries.<triple>.manifest_sha256`, the digest of that bundle's
`SHA256SUMS`, and `verify-self` compares an INSTALLED tree against it.  Absent
means "not anchored" and is reported, never counted as a pass.

The same pass collapsed the bundle to ONE manifest: `stdlib.manifest` described a
strict subset of what `SHA256SUMS` already covered, which is two ways to validate
one installation.  And it closed a hole neither manifest could see — a digest
check is silent about an ADDED file, while `collect_stdlib_sources` globs every
`*.loft` under `default/`, so dropping one in had it loaded past a green
`verify-self`.  The property is *the stdlib that loads is the stdlib that
shipped*, which needs the file SET.

**Step 3 — `loft self-update` (resolve + report).**  DONE 2026-07-31.

Blocked on 1b for DATA, not for code — so the decision is a pure function of
(index, running version, host triple), unit-tested against a constructed index.
That is the only way to know the resolver is right BEFORE the entry is published
rather than after, and writing it is what forced the entry's shape to be decided.
Four outcomes, each its own answer: no entry published, already current, an update
available, and *published but not built for this target* — because "no update" and
"no build for your platform" send a user to different places.

Two rules live in the planner rather than at the call site.  Yanked and prerelease
exclusion REUSES `find_best_version` instead of restating it, so the two cannot
drift.  And an update is only ever offered UPWARDS: everything here is a report,
but a downgrade is the one outcome that could hand a user back a release an
advisory already covers, so a registry offering only older versions reports
"current" rather than walking anyone backwards.

`--dry-run` is accepted but not required: plain `self-update` reports and stops,
so a script written now keeps its meaning when step 4 lands, and mutation will be
the thing that has to be asked for rather than the default that arrived by
surprise.  The index comes from `install::load_index` — site 3 of the table
above, wired to the existing signature-verified loader, not a second copy.

*Verified:* eight unit tests (including the downgrade guard and calendar ordering
— `2026.10.0` is newer than `2026.9.0`, which string order gets wrong), plus all
four outcomes end to end against a `file://` mock index.

**Step 3a — a rejected index no longer poisons the cache.**  DONE 2026-07-31.

Found by accident while driving step 3 against a mock registry, and worth its own
line because it is a live-system fault, not a plan item: `load_index_inner` wrote
the fetched index to the shared cache BEFORE verifying its signature.  A fetch
that was correctly REFUSED therefore outlived the run that made it — the next
command read those bytes against the previous, still-valid `.sig`, failed
identically, and kept failing.  A correct refusal became a persistent outage that
no retry could clear; it took deleting `~/.loft/registry/index.json` by hand.  (It
cost exactly that here: every registry-touching test failed until I cleared it.)

Verification now precedes the write, so nothing unverified is ever kept.  The
absent-signature fallback is unchanged — that is the offline / bundle-import path
— but the verdict is reached before anything is written.
*Verified:* a rejected fetch leaves the cached index byte-identical.

**Step 4 — apply a bundle, and always allow a local one.**  DONE 2026-07-31.

Two owner decisions reshaped this step, and both belong in the design rather than
in a commit message.

**Installing from a local artifact is a first-class route, not a fallback.**  Just
as a library installs from a local path, loft must — the people who need it are
exactly those who cannot or will not compile it, and a registry-only updater
strands them the moment the network, the firewall, or the registry is not there.
`loft self-update --from <dir>` installs an unpacked bundle with no registry and
no network.

**The strictness binds US, not the user.**  We never publish a release that cannot
be fully verified; that is a rule about `make-release.sh` and the registry entry,
enforced where we publish.  What someone installs on their own machine is theirs
to decide, so nothing here refuses a bundle its owner wants: one with no manifests
installs with a clear note that nothing could be checked, and one that actively
contradicts its manifest needs `--force`, because that is nearly always a
truncated copy rather than an intention.  Informed, not obstructed.

*The unit of update is not a directory.*  `bundle_root` is `<binary-dir>/..`,
which on a system install is a shared PREFIX — `/usr/local`, whose `bin/` holds
every other binary on the machine.  Renaming it, or even `bin/`, would take
unrelated software with it.  So the unit is exactly the files the bundle CLAIMS:
`SHA256SUMS` lists everything a release ships, which makes a bundle
self-describing about what it owns, and a bundle without one owns what it
contains.  Anything else is untouched by construction, not by a rule someone has
to remember.

*And it is not atomic across that set* — pretending otherwise would be the
dangerous design.  What makes it safe instead: nothing moves until the staged
bundle has verified, every replaced file is backed up so a failure restores, and
the residual window (a crash between two file replacements) is exactly what
`verify-self` detects and names.  That is why step 2 came first.

Three things measurement produced that the design did not:

  * `SHA256SUMS` **cannot list itself** — a file cannot carry its own digest — so
    the set it defines never includes it.  Left out, an update installs new files
    under the OLD manifest and the result fails `verify-self` forever.
  * A manifest-less install leaves the installation's old manifests describing
    files that no longer exist, so `verify-self` would report a permanent,
    meaningless failure.  They are retired: "not a release bundle" is the truthful
    answer for what the user now has.
  * The repo's own `portable_path` guard caught a blind backslash replace in the
    directory walk — on Unix a backslash is a legal filename character, and
    rewriting it would rename someone's file on the way into the set.

*Verified:* 16 unit tests, and end to end — a running loft replaced its own
installation from a local bundle, the new stdlib landed, **a neighbour's binary in
the same prefix survived**, the result verified, and the replaced binary ran.  A
manifest-less bundle installs; a contradicting one is refused and then installs
under `--force`.

*Download-and-unpack landed 2026-07-31.*  `Plan::Available` fetches, checks the
hash the signed index publishes BEFORE opening the archive, unpacks, and hands to
the same staged-install path `--from` uses — so both routes share their checks,
refusals and rollback and differ only in what is known about origin.

*Remaining, and now homed outside this plan:* per-OS verification of the swap.
The design already handles the Windows constraint — `apply_bundle` renames the
target aside and copies in, because a running executable cannot be overwritten
but can be renamed — and the unit tests exercise that on the daily Windows leg.
What no test can cover is replacing the actually-running `loft.exe`, which needs
a published release and a Windows box.  It is therefore a per-release check, not
plan work: [RELEASE_PUBLISH.md § The bundles and the registry entry](../../RELEASE_PUBLISH.md#the-bundles-and-the-registry-entry).

**Step 5 — advisory integration.**  DONE 2026-07-31.

The plan called this and @PLAN12 §6.7 "half a system" each: 6.7 produces the
signals, this reacts to them.  The load-bearing choice turned out to be WHICH
version to check.  Checking only the candidate misses the case that matters most —
a registry that is stalled, pinned, or simply has nothing newer offers no update at
all, so a user sitting on a flagged release would be told "you are up to date":
technically true and exactly wrong.  The RUNNING version is checked whether or not
an update exists, and that is the loop closing.

It reports; it does not restrict.  Whether to keep running a flagged release is the
user's call on their machine, and a tool that refuses to start is one they work
around rather than heed.  What it owes them is the advisory id, what it is, and
where the fix landed.

Silent when the registry hosts no feed (a 404 is "nothing known", not "nothing
wrong") and silent when clean — see the output note below.
*Verified:* four unit tests (reported with its fix, most-severe-first, silent on a
fixed release, silent outside the affected range) plus end to end against a feed on
disk: flagged while the registry offers nothing newer, which is the shape the whole
step exists for.

**Output — boring by construction (owner principle, 2026-07-31).**
Applied across steps 2-5 after the fact, and recorded in
[GOALS.md § The destination is BORING](../../GOALS.md): loft should be noticed only
in its absence, so a command says nothing when nothing needs acting on.  What
changed: the advisory line is gone when clean; `verify-self` in a source checkout
says one line instead of three; the closing explanation appears only on failure,
where it is wanted; and the `@PLN78 step 1b` / "not implemented yet" notes are out
of user-facing text entirely — a user asking whether an update exists deserves the
answer, not our backlog.  The first draft had all of these, which is how the
principle earned its section.

**Step 6 — `install.sh` (bootstrap).**  DONE 2026-07-31.

The only step that works TODAY without 1b, because it fetches from the GitHub
release rather than the registry — and releases already carry per-target zips with
`.zip.sha256` sidecars.

Deliberately the least clever piece, and that is the design: a shell script cannot
verify a signed index, so rather than teach it to try, its job is small enough to
audit in one sitting — pick the artifact for this host, check one sha256, unpack,
hand off to `loft verify-self`.  Everything it is trusted for fits on one page.
The script says so itself: the sidecar catches a truncated or corrupted download,
which is the failure that actually happens, but it is not a signature — script and
artifact arrive over the same transport, so anyone who could substitute one could
substitute the other.  Authenticity comes from the installed binary checking the
signed index.

`--prefix`, `--version`, `--list`, and `LOFT_INSTALL_BASE` for a mirror or an
air-gapped copy — the same "always allow a local route" that shaped step 4.

*Found by writing it:* `self_update::host_triple` composed `x86_64-unknown-linux-gnu`
(the BUILD triple) while releases publish `x86_64-unknown-linux-musl`.  Every Linux
user would have been told "published, but not built for your platform" about the
artifact meant for them.  Deriving the same name in shell is what surfaced it; the
two derivations are now pinned together by `PUBLISHED_TRIPLES` and a doc-hygiene
test, because they are one fact written twice in two languages and drift between
them is silent.

*Verified:* against a real release zip built and served locally — installs, the
installed loft runs, `verify-self` passes; a corrupted download is refused with the
digests named and the existing installation left untouched; and an upgrade over an
existing install works.

**Step 7 — reproducible builds (30.1), off the critical path.**
Nothing above depends on it.  It upgrades the *meaning* of step 1's sha256 from
"this is the artifact the maintainer uploaded" to "this is the artifact the
source produces", which is the stronger claim, but the chain works without it.
Sequenced last on purpose so it cannot block a user-visible installer.

## Failure paths

Enumerated because writing them down is what surfaced the handoff design above.

| Failure | Consequence | Handled by |
|---|---|---|
| Index unreachable / stale | self-update can't resolve | reuse `install.rs`'s existing cached-index fallback |
| Index signature invalid | a forged index picks the binary | existing `verify_or_explain`; never bypassed for the toolchain |
| sha256 mismatch on the downloaded binary | a wrong/tampered binary installs | step 3's verify precedes step 4's replace, always in that order |
| Trust root empty (fresh tree) | everything "verifies" vacuously | existing rule: signature-verified flows require explicit `--allow-unsigned` |
| Replacement interrupted | unusable installation | rename-based swap; a crash leaves old or new, never partial |
| Downgrade attack (index pinned to an old version) | user silently kept on a vulnerable release | step 5: the advisory feed is consulted against the RUNNING version, not only the candidate |
| `install.sh` served tampered | pinned hash is wrong too | irreducible for bootstrap; mitigated by the script being short, public-git-hosted, and readable, and by `verify-self` re-checking against the signed index immediately after |
| Toolchain entry mistaken for a library | `loft install loft` does something odd | DECIDED 2026-07-31: `self-update` owns the toolchain, `install` refuses it.  The refusal sits in `resolve_recursive`, the one point the direct ask, a `deps` entry and the parser's `use` fallback all pass — each otherwise ends at the same confusing place, downloading a source zip and failing to untar it as a library.  A dependency naming `loft` is told which package declared it, since the user never asked for it.  Separately, `auto_install_if_in_catalog` treats the toolchain as *absent*: before the entry existed `use loft;` was a miss that fell through to the parser's remaining strategies, and publishing a registry entry must not change what an existing program means. |

## What was probed, and what was not

Run against the tree before writing this, because the plan's own sizing was the
thing most likely to be stale:

* **Survived:** `BinaryEntry` + per-triple `binaries` map exists and is already
  sha256-verified in `install.rs` · `registry_keys.rs` embeds multiple trust
  roots, so rotation (30.5) is built · `registry_advisories.rs` exists, so the
  blocking dependency is satisfied · `compare_semver` orders calendar versions
  correctly.
* **Falsified:** the plan's "30.5 key rotation, M, 2-3 days" and its framing of
  30.2 as unbuilt.
* **Not probed — the residue, and the honest tail:** whether the rename-based
  swap is safe on Windows in practice (step 4's own verification is the probe);
  whether the registry CI can publish a toolchain entry without schema changes;
  whether reproducible builds are achievable for this crate graph at all (step 7
  may discover it is not, which is why nothing depends on it).
