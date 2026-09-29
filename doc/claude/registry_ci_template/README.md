<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Registry CI templates

Drop these files into the **`loft-lang/registry`** repo (per
[REGISTRY_BOOTSTRAP.md](../REGISTRY_BOOTSTRAP.md) Step 3) — they're
the PR-validation + post-merge-signing workflows.

| File in this dir | Destination in `loft-lang/registry` | What it does |
|---|---|---|
| `validate.py` | `tools/validate.py` | R9 PR validator: schema lint + tarball sha256 verify + reproducible-build re-check.  ⚠ **Do not deploy this copy as-is — see the warning below.** |
| `pr-validate.yml` | `.github/workflows/pr-validate.yml` | Wires `validate.py` into every PR that touches `index.json`. |
| `registry_README.md` | `README.md` (the registry's own) | Visible-on-GitHub landing page for ecosystem contributors. |
| `SUBMITTING.md` | `SUBMITTING.md` | Author-facing submission guide.  Deploy alongside README so GitHub shows the "SUBMITTING" sidebar link on the repo overview. |
| `../registry_sample.json` | `index.json` (initial seed) | Empty starter index — strip the `_comment` field; set `"packages": {}` if no real package is ready yet. |

> ⚠ **`validate.py` here and `loft-lang/registry:tools/validate.py` are ONE file with
> two homes, and `scripts/check_doc_drift.sh validator` enforces byte-identity.**  They
> drifted for eleven weeks in both directions with neither a superset
> ([loft#1052](https://github.com/loft-lang/loft/issues/1052)) — the deployment had grown
> gate 1's docs gate, the `yanked` type-check and gate 3's chunk-repo handling, while this
> copy had grown a trigger-uniqueness gate and an `api` re-derive.  Deploying either over
> the other removed live checks, and the producer in `registry_maintain.sh` was written
> against the weaker rules, which is how every package published after 2026-06-19 went in
> unmergeable.  Reconciled to the union 2026-08-21.  Change one copy and the drift check
> fails until the other matches; tooling that needs the rules should IMPORT this file
> rather than restate it, the way `scripts/registry_schema_gate.sh` does.

`SUBMITTING.md`'s canonical source is
`doc/claude/REGISTRY_SUBMIT.md` (relative links).  This dir
holds a deploy-ready copy with absolute GitHub URLs.  When you
edit the canonical, regenerate this template:

```sh
cp doc/claude/REGISTRY_SUBMIT.md doc/claude/registry_ci_template/SUBMITTING.md
sed -i.bak \
  -e 's|](PKG_REGISTRY.md|](https://github.com/loft-lang/loft/blob/main/doc/claude/PKG_REGISTRY.md|g' \
  -e 's|](PACKAGES.md|](https://github.com/loft-lang/loft/blob/main/doc/claude/PACKAGES.md|g' \
  doc/claude/registry_ci_template/SUBMITTING.md
# then re-add the deploy-copy header at the top by hand
rm doc/claude/registry_ci_template/SUBMITTING.md.bak
```

Optionally add a JSON Schema file at `schema/index-v1.json` for
editor tooling — useful but not required (`validate.py`'s lint
is sufficient for the gate).

## Signing is NOT in CI

Earlier drafts of this template shipped a `sign-and-commit.yml`
workflow + `sign-index.py` helper that signed `index.json` in
GitHub Actions using a `REGISTRY_SIGNING_KEY_BASE64` secret.

We removed it.  Signing happens locally on the maintainer's
laptop via `loft-keygen sign` — see [REGISTRY_BOOTSTRAP.md § Step 4](../REGISTRY_BOOTSTRAP.md)
maintainer side, and the rationale in [PKG_REGISTRY.md § Index signing](../PKG_REGISTRY_TRUST.md#index-signing--indexjsonsig).

Short version: an early-stage ecosystem publishes weekly at most;
a human maintainer is always in the merge loop anyway; the
private key never needs to leave hardware the maintainer
physically controls.  When the ecosystem scales beyond what
laptop signing handles, the architecture migrates to Path 1
(real server) and signing follows.

## Local dry-run

Run the validator against a candidate index before opening the PR:

```sh
cd loft-lang/registry/
python3 tools/validate.py
```

The reproducible-build gate clones each new homepage repo + runs
`loft package` — slow but the publisher's machine is the right
place to catch mismatches before CI does.

To skip the reproducible-build gate locally (much faster smoke):

```sh
LOFT_VALIDATE_SKIP_REPRO=1 python3 tools/validate.py
```
