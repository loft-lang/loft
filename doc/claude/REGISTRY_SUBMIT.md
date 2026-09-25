<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Submitting a library to the loft registry

This is the author-facing guide for publishing a loft library —
adding a new package, releasing a new version of an existing
package, or yanking a broken release.  Companion docs:

- [PKG_REGISTRY.md](PKG_REGISTRY.md) — registry design + schema reference.
- [PACKAGES.md](PACKAGES.md) — `loft.toml` package format.
- `loft-lang/registry/README.md` — the registry repo's own
  landing page (lives in the registry, not here).

If you're a *consumer* (`loft install <name>`), this doc isn't
for you — just run the command.  Read on only if you maintain a
library you want others to install.

---

## Prerequisites

Before you can submit, you need:

1. **A loft package** — a directory containing a valid
   `loft.toml` and source.  See
   [PACKAGES.md § Package layout](PACKAGES.md#package-layout) for
   the minimum.  In brief:

   ```text
   my-lib/
   ├── loft.toml          # [package] name, version, loft, repository
   ├── src/<name>.loft    # entry point (or [library] entry = "...")
   ├── tests/             # optional, but expected
   └── native/            # optional cdylib if you have native code
   ```

2. **A public source repo on GitHub** — at minimum, the
   reproducible-build re-check (gate 3 below) needs to clone the
   tag and run `loft package`.  The repo URL becomes the
   package's `homepage`.

3. **A loft binary** ≥ the version your package requires.
   Build from [github.com/loft-lang/loft](https://github.com/loft-lang/loft)
   if your distro doesn't ship one new enough.

You do NOT need:

- A signed-commit setup (the registry maintainer signs the
  index; you sign nothing).
- A GitHub Action workflow on your own repo (the registry's CI
  does the validation).
- An account on any registry server (the MVP is a static
  GitHub repo; you submit via PR).

---

## The five-step submit flow

### 1. Tag the release in your source repo

The tag is **`<pkg>-v<version>`** whatever the repo layout: the loft-lang
libraries ship as **domain monorepos** (`loft-libs-core`, `loft-libs-net`,
`loft-libs-graphics`, `loft-libs-game`, `loft-libs-world`, `loft-libs-assets`),
several packages per repo, and `loft package` names the release it expects that
way for a one-package repo too.

```sh
cd loft-libs-core/
git tag crypto-v0.2.1              # <pkg>-v<version>
git push --tags
```

The tag is always `<pkg>-v<version>`, and the repo it lives at comes from
`[package] repository` when the manifest declares one:

```toml
[package]
name = "crypto"
version = "0.2.1"
repository = "loft-libs-core"      # → tag crypto-v0.2.1 at loft-lang/loft-libs-core
                                   #   (a value with "/" is a full owner/repo)
```

Undeclared is fine: both commands then read the package directory's own
`git remote get-url origin`, which is how a monorepo package gets a correct url
without a manifest line.  With neither, the url is REFUSED rather than guessed —
`loft package` used to invent a `loft-lang/loft-<pkg>` repo that exists for no
package in any `loft-libs-*` monorepo, so the two commands printed different
urls for the same package and only the registry's fetch caught it (loft#1083).

The version in the tag MUST match `[package] version` — the registry's
reproducible-build re-check (gate 3) clones the tag and re-runs `loft package`.

### 2. Build the tarball with `loft package`

```sh
loft package
```

> `loft publish --dry-run` prints the same entry with `subpath` and the `api`
> surface snapshot filled in, so prefer it when you have the checkout in hand.
> Both derive the `url` from the same place, so they cannot disagree (loft#1083).
> Neither fills `deps` — a monorepo declares none, so copy them from the
> previous version's entry.

Output in cwd:

```text
my-lib-0.1.0.tar.gz
  package:  my-lib v0.1.0
  size:     <N> bytes (<N.N> kB)
  sha256:   <hex>

Index entry to paste into loft-lang/registry/index.json (PKG_REGISTRY.md schema):
  "0.2.1": {
    "url": "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz",
    "sha256": "<hex>",
    "size": <N>,
    "loft": ">=0.8",
    "published": "<ISO-8601 UTC timestamp>"
  }
```

The `url` is generated from `[package] repository` (the `<pkg>-v<version>` tag at
that repo); with no `repository` it falls back to `loft-<pkg>/…/v<version>/…`.
The tarball is **deterministic** — same source dir → same sha256 across runs.
This is what gate 3 will re-check.

### 3. Upload the tarball as a GitHub release asset

Use the SAME tag you pushed in step 1 (`<pkg>-v<version>` for a monorepo):

```sh
gh release create crypto-v0.2.1 crypto-0.2.1.tar.gz \
    --title "crypto 0.2.1" \
    --notes "Patch release."
```

The asset's URL — printed by `gh release create` and matching
the `url` field in the index entry above — is what `loft
install` will fetch.

**Don't edit the release assets after this point.**  If you
re-upload a tarball, its bytes change (gzip timestamp, etc.)
and the sha256 in your in-flight PR will no longer match —
gate 2 will reject the PR.  If you need to fix the release,
yank the version and ship `v0.1.1`.

### 4. Open a PR against `loft-lang/registry`

Fork [github.com/loft-lang/registry](https://github.com/loft-lang/registry),
then edit `index.json`:

```diff
   "packages": {
+    "my-lib": {
+      "description": "One sentence on what the library does.",
+      "homepage": "https://github.com/<owner>/<repo>",
+      "categories": ["<category>"],
+      "yanked": [],
+      "versions": {
+        "0.1.0": {
+          "url": "https://github.com/<owner>/<repo>/releases/download/v0.1.0/my-lib-0.1.0.tar.gz",
+          "sha256": "<hex from step 2>",
+          "size": <N from step 2>,
+          "loft": ">=0.8",
+          "deps": {},
+          "conflicts": [],
+          "replaces": [],
+          "provides": [],
+          "binaries": {},
+          "prerelease": false,
+          "published": "<ISO-8601 UTC timestamp from step 2>"
+        }
+      }
+    }
   }
```

Most *version* fields are optional — the minimum is `url`, `sha256`,
`size`, `loft`, `published`.  Drop the empty arrays/objects
you don't use.  See
[PKG_REGISTRY.md § Schema](PKG_REGISTRY.md#schema) for the
full reference.

⚠ **The three PACKAGE-level fields are not optional.**  `description`
(≥ 10 characters), `homepage` (an http(s) URL) and `categories` (a
**non-empty** list) are gate 1 of the registry's `tools/validate.py`, and
`"categories": []` is the one people paste — the placeholder above is a
placeholder.  Reuse a tag the catalogue already carries (`geometry`
`graphics` `text` `net` `world` `game` `time` `math` `random` `crypto`
`encoding` `cli` `plugins` `asset-format` `animation`) rather than minting
one, so your library lands in a group somebody browses.  The same three
fields are required in a `submissions/` file (§ 4 below).

> **A third, learned submitting the toolchain entry:** do not write the artifact's
> own `published` stamp into the index's `updated`.  That field says when the INDEX
> last changed, and a toolchain entry is normally submitted well after the release it
> names — 2026.8.0 was published on the 1st and submitted on the 31st — with libraries
> landing in between, so it dated the index a month earlier than packages it already
> carried.  Nothing in the client compares the field, so the only thing that could
> catch it was a reader of the diff; `gen-toolchain-entry.py` takes the later of the
> two now, under
> `mock_registry::splicing_the_toolchain_entry_never_moves_updated_backwards`.
>
> **Two things a programmatic edit gets wrong** (learned publishing crypto
> 0.3.3): for a **multi-package repo** (e.g. `loft-libs-core`), copy the existing
> entries' **`subpath`** field (`"subpath": "crypto"`) — `loft package` omits it,
> while `loft publish --dry-run` emits it (and the `api` surface snapshot) for you.
> And if you edit `index.json` with a script, **match the file's CURRENT
> unicode convention** — check whether descriptions carry raw `—` or escaped
> `—` and pass the matching `ensure_ascii` to `json.dump`, then verify
> with `git diff` that ONLY your entry changed.  (The convention has flipped
> once already: this note used to say "always `ensure_ascii=True`", and
> following it against a raw-unicode index rewrote every description line —
> the mirror image of the failure it warned about.)
>
> **Maintainer step — merge first, then re-sign.** An author PR edits
> `index.json` but cannot sign it. `registry_maintain.sh` merges the green
> PR into `main` **first** (so a merge that fails partway never blocks the
> run before it reaches the signing step) — `main` briefly carries a merged
> `index.json` and a now-**stale** `index.json.sig`, a deliberate transient
> window — then, at the very end of the same run, re-signs via
> `scripts/registry-sign.sh`, which commits the fresh `.sig` **together
> with** the index (one atomic change). Skipping that final re-sign leaves
> the stale signature in place and fails verification for *all* installs
> (see [REGISTRY_RECOVERY.md](REGISTRY_RECOVERY.md)). The raw signing
> commands, for manual recovery:
>
> ```
> loft-keygen sign   --in index.json --key ~/.loft/trust-root/registry-signing-key.bin --out index.json.sig
> loft-keygen verify --in index.json --sig index.json.sig --pub "$(cat ~/.loft/trust-root/registry-signing-key.pub)"
> ```

Open the PR.  Title format: `add my-lib 0.1.0` (or for
subsequent versions, `add my-lib 0.2.0`).

### 4 (recommended) — stage a `submissions/` file instead of editing `index.json`

Editing `index.json` directly (above) has two drawbacks: your PR can **race** the
signed index (two PRs touching the same file, or a maintainer publish landing
between your CI run and the merge), and nothing **validates** your library before it
is trusted.  The **`submissions/` path** fixes both (@PLN102 C96): you add a small
staging file that *never touches `index.json`*, and the maintainer's `loft ship` run
puts it through the full validation gate before folding it in.

> ⚠ **Only the maintainer half of this is wired, and the registry repo says
> otherwise.**  `scripts/registry_maintain.sh` drains `submissions/` (vet → fold →
> re-sign → `git rm`, one atomic commit) and treats an absent directory as an empty
> one, so staging a file here works.  But `loft-lang/registry` has **no
> `submissions/` directory**, nothing in its `tools/validate.py` or
> `.github/` mentions one, and its own `SUBMITTING.md` documents the
> `index.json` edit — so a submitter reading the repo they are opening a PR
> against is told to do the opposite of this page, and the two have to be
> reconciled THERE before this can be called the recommended route.  What a
> submitter should expect meanwhile: their PR creates that directory's first
> file, the registry's PR CI validates `index.json` and therefore says nothing
> about the submission, and the vetting happens later on the maintainer's run
> rather than on the PR.  A contributor who follows the registry's own page
> instead has not made a mistake.  Reported by a consumer submitting
> `lavition_ui` 0.1.0, who followed `SUBMITTING.md` on exactly that reasoning.

Add **one file**, `submissions/<name>-<version>.json`, in your registry PR — nothing
else:

```json
{
  "name": "my-lib",
  "version": "0.1.0",
  "repo": "<owner>/<repo>",
  "tag": "v0.1.0",
  "subpath": "my-lib",
  "description": "One sentence on what the library does.",
  "homepage": "https://github.com/<owner>/<repo>",
  "categories": ["text"],
  "entry": {
    "url": "https://github.com/<owner>/<repo>/releases/download/v0.1.0/my-lib-0.1.0.tar.gz",
    "sha256": "<hex from step 2>",
    "size": <N from step 2>,
    "loft": ">=0.8",
    "subpath": "my-lib",
    "deps": {}
  }
}
```

Fields:

| Field | Required | Meaning |
|---|---|---|
| `name` | ✓ | the package name (matches its `loft.toml`) |
| `version` | ✓ | the version being submitted (matches `loft.toml`) |
| `repo` | ✓ | `<owner>/<repo>` GitHub source — the vetter clones it |
| `tag` | ✓ | the **release tag** from step 1 (`v0.1.0`, or `<name>-v<version>` in a multi-package repo) |
| `subpath` | ✓ for a multi-package repo | the package **dir** inside the repo (e.g. `crypto`); defaults to `name` |
| `entry` | ✓ | the index entry — exactly what `loft package` prints (step 2), so `url` / `sha256` / `size` / `loft`; `deps` etc. as needed. **Omit `published`** — the fold stamps it. |
| `description`, `homepage` | optional for an existing package | seed a brand-new package's index metadata (ignored if the package already exists) |
| `categories` | ✓ for a brand-new package | non-empty list of catalogue tags.  The fold **refuses** a package the index has never seen without one, because gate 1 rejects an empty list and a submission folded in that way would redden every later PR.  Ignored (the curated list wins) if the package already exists. |

Generating it is copy-paste from step 2: `loft package` already prints the `entry`
body; wrap it and add `name`/`version`/`repo`/`tag`/`subpath`.

What the maintainer's `loft ship` does with it (no human step unless flagged):

1. **Vets** `repo@tag` through `scripts/vet-lib.sh` — the same V1–V6 gate own libs
   pass: it compiles + runs your tests against the current loft, checks metadata, and
   flags any `#rust`/`#native` code.
2. On **PASS** (pure loft), folds `entry` into `index.json`, deletes the staging file,
   and **re-signs** — all in one atomic commit.  The sign step re-verifies your
   `sha256` against the actual release tarball, so a wrong hash is caught there.
3. **`#native`/`#rust`** code → a one-time human **review** before admission (arbitrary
   native code is never auto-trusted); a **gate failure** → reported, not admitted.

Because the submission never edits `index.json`, it can't race the signed index, and
because it is vetted before it is folded, a broken or incompatible library never
reaches consumers.  Title the PR `submit my-lib 0.1.0`.  (The direct-`index.json` edit
above still works and is merged the same way, but it is being superseded by this path.)

### 5. Wait for CI + maintainer review

The registry's CI runs `tools/validate.py` automatically.
Four gates:

| Gate | What it checks | Common failure cause |
|---|---|---|
| Schema lint | Required fields, correct types, `schema_version` unchanged | Typo in field name, wrong type (`size` as string instead of int), forgot `published` |
| Tarball verify | Download `url`, hash it, compare to PR's `sha256` | Re-uploaded the GitHub release asset after opening the PR; pasted wrong sha256 |
| Reproducible-build re-check | Clone `<homepage>` at `v<version>`, run `loft package`, compare sha256 | Source repo's tag points at different bytes than the uploaded tarball; build environment leaked content (e.g. uncommitted files) into the tarball |
| Trigger uniqueness | Every Tier-1 `method:receiver` trigger is owned by at most one package across the whole registry | Your `[triggers]`-enabled package declares a `pub fn` method-on-type (`matches:text`, …) that another package already claims |

If a gate fails, CI surfaces the error as a PR comment.  Fix
the underlying cause, push to your PR branch, CI re-runs.

> **Trigger uniqueness, in plain terms.** If your package opts into
> `[triggers]` (so consumers can call `obj.method()` and have your
> library auto-loaded), every `method:receiver` pair you expose must
> be globally unique — because a consumer writing `line.matches(p)`
> auto-loads *the* package that owns `matches:text`, and there can be
> only one.  `loft publish` warns you locally when a trigger you are
> about to claim is already taken (checked against your cached
> catalog); gate 4 enforces it as a hard reject.  The fix is to rename
> the method, or drop the `[triggers]` opt-in and let consumers reach
> your library with an explicit `use`.

When all four gates pass, a registry maintainer reviews the
PR — typically a sanity check on the description, homepage URL,
and tarball provenance.  After approval, `registry_maintain.sh`
merges the PR into `main` and, at the end of that same run,
re-signs the resulting `index.json` locally (see
[PKG_REGISTRY.md § Why laptop signing](PKG_REGISTRY.md#why-laptop-signing-not-ci)).

**Once merged, `loft install my-lib` works for everyone.**
Typical time-to-publish from PR open to merge: hours to days
depending on maintainer availability.

---

## Subsequent releases

For a new version after one already shipped (monorepo example, `crypto 0.2.1`):

1. Bump `version` in `loft.toml`.
2. `git tag crypto-v0.2.1 && git push --tags`  (bare `v0.2.1` for a per-package repo).
3. `loft package`.
4. `gh release create crypto-v0.2.1 crypto-0.2.1.tar.gz`.
5. PR adding ONLY the new version row (don't touch the
   existing rows):

   ```diff
       "versions": {
   +    "0.2.0": {
   +      "url": "https://github.com/.../v0.2.0/my-lib-0.2.0.tar.gz",
   +      "sha256": "<new hex>",
   +      "size": <new N>,
   +      "loft": ">=0.8",
   +      "published": "<new timestamp>"
   +    },
        "0.1.0": {
          ...
        }
       }
   ```

The registry **never deletes** version entries.  Old versions
stay so existing `loft.lock` pins keep resolving.  If a
version turns out to be broken or vulnerable, yank it
(below) rather than removing the row.

> **In-tree-tested libraries** (the loft dogfood libs — `graphics`,
> `shapes`, `gridmesh`, `imaging`, `arguments`, `game_protocol`, `web`,
> `hex_world`, `time`) have one extra step: re-sync the loft monorepo
> test fixture so the compiler suite tracks the new tag.  See
> [LIBRARY_AUTHORING.md § 5d](LIBRARY_AUTHORING.md#5d-re-sync-the-loft-monorepo-fixture-in-tree-tested-libs-only).
> A pure registry-only library has no fixture and skips it.

---

## The toolchain entry (`loft` itself) — @PLN78

`loft` is in the registry so `self-update` can find a release, and it is the only
package that is **not** submitted by the flow above.  Nothing here is typed by hand.

**Why it is in the index at all.**  A release ships `loft-<v>-<triple>.zip` plus a
`.zip.sha256` sidecar, and that sidecar rides the same transport as the artifact —
it catches a corrupted download, not a substituted one.  `index.json` is the only
signed thing we publish, so naming the binaries from the index is what puts them
under a signature.  One key ceremony per release rather than five:

```
index.json                        ← the ONE signature (Ed25519, 4 trust roots)
 ├ binaries[triple].sha256          → loft-<v>-<triple>.zip   checked once, at download
 │  └ manifest_sha256              → SHA256SUMS             checked any time, on what is INSTALLED
 │     └ bin/loft, default/*.loft, and every other file the bundle shipped
 └ version.sha256                   → loft-<v>-src.zip        the source the release was built from
```

**The submit.**  The release workflow attaches `loft-<v>-registry-entry.json`,
generated from the artifacts of the run that built them.  Splice it in and open a PR:

```bash
# Download EVERY asset, then let the tool merge -- `--splice-into` regenerates the
# entry from the artifacts it hashes, so the directory must hold all of them:
gh release download v<version> -R loft-lang/loft -D <assets>
scripts/gen-toolchain-entry.py --version <version> --dir <assets> \
    --splice-into <registry-checkout>/index.json
```

Downloading only `loft-<version>-registry-entry.json` and pasting it in is the
route that looks cheaper and is the one this section warns against below: there is
no flag that splices an existing entry FILE, so taking that shortcut leaves you
hand-merging the `versions` map.  The two agree — the regenerated entry was
verified byte-identical to the attached one for 2026.8.0 — so use the tool and keep
the attached JSON as the thing to diff against if a hash is ever disputed.

Never paste the entry over the existing one: the `versions` map **adds**.  Replacing
it drops every earlier release, and it does so silently — resolution still succeeds,
those versions simply cease to exist for the users still on them.  `--splice-into`
merges, refuses to overwrite a version already present, and leaves `yanked` alone.

**Two things the entry must keep.**  `loft_ffi_fp` is **absent** on every binary (it
gates cdylib ABI compatibility and means nothing for an executable that links nothing
of the host's), and the triples are the published ones — `x86_64-unknown-linux-musl`,
never `-gnu`.  Both are pinned by tests rather than by this paragraph:
`generated_toolchain_entry_parses_and_drives_self_update` (`tests/mock_registry.rs`)
and `installer_and_self_update_agree_on_the_published_triples`.

**Registry-side gates.**  `tools/validate.py` exempts the toolchain from gate 3
(reproducible build): there is no `loft.toml` at loft's repo root to re-package, and
the version artifact is a `git archive` zip, not a `loft package` tarball.  Gate 2b
verifies every `binaries` hash by download, so the exemption does not leave the
binaries — the things users actually run — unchecked.

> **Both landed in [loft-lang/registry#22](https://github.com/loft-lang/registry/pull/22),
> merged 2026-08-31** — before which `loft` was in no index at all, and had never been.
> The gates were verified end-to-end first, in a throwaway clone, because a doc that
> says a gate works is the thing that stops anyone checking: with #22's validator,
> splicing 2026.8.0's entry passes every gate and gate 2b downloads each platform zip
> and re-checks its sha256; with the pre-#22 validator, the same index reproduces
> `` `loft package` failed: exit status 1 `` exactly.  #22 sat open for weeks partly
> because it carries no CI of its own — the registry validates submission PRs, not
> changes to the validator, so nothing was ever going to go green to signal it was
> ready.
>
> The first toolchain submission is
> [loft-lang/registry#31](https://github.com/loft-lang/registry/pull/31) (`loft
> 2026.8.0`), opened, validated (2m37s) and merged the same day, then signed with
> `registry-sign.sh --expect loft@2026.8.0` — which reported *"exactly loft@2026.8.0 —
> nothing else added, removed or altered"*, re-downloaded the 18.5 MB source archive to
> re-check its sha256, and passed the trust gate.  Since that commit the published
> bundle answers the third question for the first time:
>
> ```
> $ bin/loft verify-self
>   ok      files: 21 file(s) match
>   ok      stdlib set: 6 file(s), none added
>   ok      origin: matches the signed registry index
> matches the release published in the signed registry index
> ```
>
> **Expect a lag when you check.**  The index is cached under a TTL, so a client whose
> cache predates the merge reports `no releases published to compare against` — the
> same words as an empty index, which reads as a failed submission.  Pass `--refresh`
> (`loft self-update --dry-run --refresh`) when verifying a release you just published.  This paragraph described them as current
> from the day it was written; the live validator had no toolchain case at all —
> gate 3 skipped only a package with no `homepage`, and the toolchain has one — so
> the first real submission (2026.8.0) failed on `` `loft package` failed: exit
> status 1 ``.  Gate 2b was missing too, which is the half that matters: exempting
> gate 3 on its own would have let four unverified platform zips into the signed
> index.  Written down because a doc that describes a gate as existing is the one
> thing that stops anyone checking whether it does.

**Which release.**  The first entry names a release carrying `loft-<v>-src.zip`.
Published releases are immutable, so v2026.7.2 and earlier can never gain one.

---

## Yanking a broken release

A yanked version stays listed in the index (lockfile pins
still resolve) but new `loft install` calls skip it unless
the user passes `--allow-yanked`.

To yank `v0.1.2`:

```diff
   "my-lib": {
     ...
-    "yanked": [],
+    "yanked": ["0.1.2"],
     "versions": {
       ...
```

PR title: `yank my-lib 0.1.2 — <reason>`.  Use the PR body to
explain why (security issue, broken build, packaging error).
The maintainer will merge after a brief review.

---

## What NOT to include in your package

`loft package` excludes a sensible default list (`.git`,
`target`, `.loft`, `node_modules`, `.vscode`, `.idea`, any
`*.tar.gz` / `*.tar`).  But the responsibility for "what's in
the tarball" is yours.  Common mistakes:

- **Build artefacts in non-standard locations** — anything
  under `target/` is excluded, but if your build leaves
  artefacts under e.g. `out/` they ship in the tarball.
  Inspect with `tar tzf my-lib-0.1.0.tar.gz | sort` before
  uploading.
- **Local config files** — `.envrc`, `.tool-versions`,
  IDE-specific configs.  These don't usually break anything
  but bloat the tarball and confuse downstream consumers.
- **Secrets** — `.env`, credentials.  The registry doesn't
  scan for these; you do.  Use a `.loftignore` if you need
  finer-grained exclusion (planned for a future MVP
  iteration; currently `loft package` only uses the built-in
  list).
- **Test fixtures with private data** — if `tests/` has
  recorded API responses from a service you have credentials
  for, scrub them before tagging the release.

---

## Etiquette

- **Compatibility is declared, not implied by the version
  number.**  `MAJOR.MINOR.PATCH` is only an identity; what
  says a release is safe to take is its three declared levels
  (`loft`, `api_compatible_with`, `data_compatible_with`), and
  raising a floor is how a break is declared.  `loft compat
  check --full` verifies the claim before you tag
  ([LIBRARY_AUTHORING.md § 3](LIBRARY_AUTHORING.md)).
- **`loft = ">=X.Y"`** in your version entry should match the
  oldest loft you actually tested against.  Don't claim
  `>=0.8` if you used a 0.8.4-only feature.
- **Deprecation is a signal, never a removal**: point at the
  successor in the docs and keep the old version working
  ([COMPATIBILITY.md](COMPATIBILITY.md)).  Yank only a version
  that is broken or vulnerable, never one that is merely
  superseded.
- **Multiple maintainers**: file an issue against
  `loft-lang/registry` requesting co-maintainer status.  The
  registry maintainers will add a co-author to the package's
  GitHub repo and update the metadata.

---

## Troubleshooting

### "Tarball sha256 mismatch"

You re-uploaded the GitHub release asset after opening the
PR (or after running `loft package`).  Options:

- Easiest: yank-and-bump.  Delete the v0.1.0 release, bump to
  v0.1.1, re-run from step 1.
- Or: re-run `loft package` against the unchanged source,
  upload the FRESH tarball to the same release, update the
  PR's `sha256` field.

### "Reproducible-build sha256 mismatch"

CI cloned `<homepage>` at `v<version>` and ran `loft
package`, but the resulting sha256 doesn't match.  Causes:

- The tag was force-pushed AFTER you generated the original
  tarball.  Don't force-push tags.
- Your local `loft package` saw files the clean clone doesn't.
  `loft package` now skips files git IGNORES (so leftover
  `tests/_tmp_*.bin` from a `loft test` run no longer leak into
  the tarball), but UNTRACKED-and-not-ignored files and
  uncommitted edits to tracked files still change the bytes.
  Inspect with:

  ```sh
  git status --porcelain   # uncommitted edits + untracked files
  git clean -ndx           # what a clean would remove (untracked + ignored)
  ```

  Commit (or `.gitignore`) anything that shows up, then re-tag.
  `./release.sh` (scaffolded by `loft new`) commits + checks a
  clean tree before tagging, which avoids this.
- Different `loft` versions produce different tarballs (this
  is a known limitation of the MVP — `loft package` should
  pin its output format across versions, tracked as a
  follow-up).  Use the same loft version you'll declare in
  `loft = ">=X.Y"`.

### "Validation says my dep is missing"

Your `[dependencies]` entry references a package not in the
registry yet.  Either submit that dep first, or use a `path =`
local reference (path-deps aren't installable via the registry
but they ARE accepted in the lockfile — the consumer must
fetch them out of band).

---

## Mirroring the registry

The registry is a single static `index.json` on GitHub.  Anyone
can mirror it:

```sh
git clone https://github.com/loft-lang/registry
# host the resulting dir however you like
export LOFT_REGISTRY_URL=https://<your-mirror>/index.json
loft install my-lib
```

A mirror with an unmodified `index.json.sig` works
transparently — clients verify the upstream signature.  A
mirror that wants to use a different signing key needs its
public key added to the loft binary's `TRUSTED_PUBLIC_KEYS`;
file an issue against `loft-lang/loft` to discuss.
