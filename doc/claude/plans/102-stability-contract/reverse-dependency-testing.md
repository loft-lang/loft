<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Compatibility by testing: the reverse-dependency check

> **Status: DESIGN (2026-07-16).** The trust a library ecosystem needs is not "is the
> author honest" — it is "did a *good* author break something they cannot see." The
> answer is not a declared contract (capability flags — considered and **rejected**
> below); it is to **run the old unit tests and the mechanical checks on every axis of
> change**. Two of the three axes exist (`vet-lib`, `revalidate-libs`); this doc
> designs the missing one — **reverse-dependency testing** — and the property that
> makes all three aid the author *and* the consumer.

## The frame — trust is not-breaking-by-accident

"Library trust" is easy to mis-frame as a *security* question (is the author honest,
could this be malicious). That part is handled the ordinary way — reputation, and the
implicit rule that a library works as advertised and its functions document their
goals. The load-bearing problem is the opposite: an author who *intends* the library
to keep working, and for whom **not** breaking a consumer is genuinely hard, because
the break is invisible from where they stand. Trust is the mechanical assurance that
the accident was caught.

## Capability flags — considered and REJECTED

A declared capability surface in the manifest (`capabilities = ["net", "fs:read"]`,
compiler-enforced) was considered — first as a consumer trust-signal, then as an
author guardrail against accidental scope creep. **Rejected on both readings:**

- Flags do not *describe* what a library does — the docs and function goals do, far
  better. "net/fs/proc" can't distinguish reading `~/.ssh` from fetching a weather API.
- For the libraries that could actually hurt you — every genuinely capable one is
  `#native` (`ssh`, `web`, `crypto`) — a flag is **unenforceable** (arbitrary Rust the
  compiler can't police), so it degrades to a coarse second promise beside the doc.
- As an *accidental-drift* guard it still fails, decisively: **you do not open a socket
  by accident.** Reaching the network or the filesystem is always a deliberate line of
  code or a dep you added on purpose — so an enforced capability bound would only ever
  flag changes you meant to make. It guards a failure mode that does not occur.

The accidents that *do* occur are **behavioral** — a return value changes in an edge
case, signature identical — and no declaration catches those. Only **running the
tests** does. So there is no capability manifest; the honest guarantee is the test run.

## The principle — mechanical checks are portable facts, so they aid both sides

Run the old unit tests and the mechanical checks (compile, `api-surface` diff,
reproducible package) on every axis a change can move along. Two properties make this
the right foundation:

1. **It catches the accidents that happen.** A behavioral break is invisible to the
   author's own tests, to `api-surface`, and to any declaration — but a *test* that
   depends on the old behavior goes red.
2. **A mechanical result is a *portable fact*, a promise is not.** "My tests pass
   against your app" and "this version doesn't break me" are the *same fact* read from
   two sides. A doc or a flag is the author's promise and never becomes the consumer's
   guarantee; a green test result means the identical thing to whoever holds it. That
   portability across the author↔consumer boundary is exactly why these tools aid
   **both** — the author runs a check to catch their break, the consumer runs the *same*
   check (or reads its recorded result) to protect themselves.

## The trio — test on every axis the author is blind to

Each axis targets a break the author cannot see *by construction*: they hold their own
tests, but not their consumers', not the future language, not their dep tree's surface.

| axis | trigger | what runs | the author learns | the consumer learns | status |
|---|---|---|---|---|---|
| **own** | a lib version | the lib's own tests + `vet-lib` V1–V6 | "do I pass my own gate" | "was this vetted before I install" | **built** — `scripts/vet-lib.sh` |
| **forward** | a loft change | every published lib vs the new loft | "am I still green on latest loft" | (as a lib author, same) | **built** — `.github/workflows/revalidate-libs.yml` |
| ↳ *warning debt* | a loft change | the same run's warnings, published + source | "will my next PR go red on code I didn't touch" | — | **built** — `scripts/lib_warning_scan.py`, reported not gated |
| **reverse** | a lib version | every consumer's tests vs the new lib | "did I break a consumer" | "will this update break me" | **this doc** |

Reverse is the only axis that catches a **behavioral** break, because the consumer is
the only party holding a test that depends on the old behavior.

## Reverse-dependency testing — the design (mirror of `revalidate-libs`)

### 1. Discover L's consumers
- **Libraries — free, from the registry `index.json`:** every package whose `deps`
  names L, filtered to those whose version range would accept `L@new`. The index *is*
  the reverse-dependency graph; `revalidate-libs`'s discover step already reads it.
  Transitive consumers fall out by recursion.
- **Apps — the gap:** an app (`ssh_home` consumes `graphics`) is not in the registry,
  so nothing knows to test it. Apps **opt in** via a `consumers.json` in the registry
  (a list of `repo@ref` to pull + test), or a per-repo "reverse-test me" registration.
  Unregistered apps are invisible — honest, and `log()`'d.

### 2. Test each consumer against `L@new`
For each consumer P (a matrix leg, like the other two workflows):
- resolve P's dep tree but **pin L → the new version** (a resolver override),
- build P — **interpret is the hard gate**, native best-effort (same split as
  `revalidate-libs`; interpret catches behavioral breaks with no per-lib deps),
- run P's own test suite.

Green ⇒ `L@new` is safe for P; red ⇒ a behavior P depended on changed.

### 3. The verdict — where absolute compat bites
- **All consumers green** ⇒ `L@new` is consumer-safe; ship it.
- **A consumer red** ⇒ L **accidentally broke a behavior a consumer relied on** — the
  invisible break, made visible. Report the consumer + the failing test + (ideally) the
  value diff. Under [absolute compat](../../COMPATIBILITY.md) this **blocks the
  publish**: the author restores the behavior, *or* declares a deliberate breaking bump
  and the consumers migrate (the gate then re-runs the migrated versions). Either way
  the break is caught **before** L ships, mechanically, with the author otherwise blind.

### 4. Scoping (it is the expensive axis)
Direct consumers first (transitive by recursion); a **version-range filter** so you
test only who would actually pick up `L@new`; matrix parallelism + the cached loft
build from `library-ci-reusable`; interpret-only as the cheap hard gate. `log()` every
consumer that could not be resolved — never a silent "all green" that meant "tested
nothing."

### 5. Where it plugs in — two options
- **`reverse-dep-test.yml`** on L's `<name>-v<version>` tag — symmetric with
  `revalidate-libs.yml`, reusing its discover + build-and-test steps almost verbatim.
  Post-hoc, lighter to add.
- **The `loft maintain-registry` pre-publish gate** — fold the reverse-dep run into the ship
  transaction *before* signing `L@new`, so a break **blocks** the publish rather than
  reporting after. Stronger, where absolute compat wants it, but heavier (the ship
  command now waits on N consumer suites).

## Dual-use — one primitive, two callers

Because the check is mechanical (§ the principle), the same operation serves both sides:

- **The reverse-dep primitive is a first-class command:** `loft test --against L@<ver>`
  = *build me with L pinned to this version, run my tests*. That is the **consumer's
  pre-upgrade check** ("will this bump break me?") *and* the per-leg body of the
  **author's gate**. Build it once, expose it to both — do not bury it as an internal
  gate step.
- **The gate's result is a consumer-facing fact:** a version that passed reverse-dep
  against N consumers is **recorded as an attestation** on the registry entry
  ("reverse-tested green against N consumers"), the same way `vet-lib`'s V-checks are.
  The consumer then *reads* the author's run instead of re-running it.
- **`consumers.json` registration is the consumer opting into protection:** an app that
  registers is saying "run me before you ship, so you *cannot* break me." The consumer
  buys protection by contributing a test suite to the gate.

## Reuse vs build

- **Reuse:** discover (the `index.json` read from `revalidate-libs`); build-and-test
  (`library-ci-reusable`); the pre-publish hook (`loft maintain-registry` / `registry_maintain.sh`).
- **Build (small):**
  1. **A resolver dep-override** — build P with L forced to `@new` instead of its locked
     version (`--override L=<ver>` / a temp lockfile edit), exposed as `loft test
     --against`. This is the one genuinely new primitive.
  2. **App opt-in discovery** — `consumers.json` in the registry (a `repo@ref` list).
  3. **The gate** — `reverse-dep-test.yml`, or fold into `loft maintain-registry`; **record the
     attestation** on the registry entry.

## Invariant + verification

> **Invariant:** no library version is published that fails a *registered* consumer's
> test suite — an accidental behavioral break is blocked before it ships.

**Verification (falsifiable):** inject a behavioral change into L that L's *own* tests
miss but a consumer's test catches (change a return value in an edge case the consumer
asserts on); the reverse-dep gate must go **red on that consumer** and block the
publish. The *same* change behind a declared **major** bump, with a migrated consumer
version, must pass. A gate that tested zero consumers (unresolved, silently skipped)
must be visible as such, not reported green.

## See also

- [revalidate-libs.yml](../../../../.github/workflows/revalidate-libs.yml) — the forward dual · `scripts/vet-lib.sh` — the own axis · [library-ship-validation.md](library-ship-validation.md) — the ship / vet / registry apparatus this rides on · [COMPATIBILITY.md](../../COMPATIBILITY.md) — absolute compat (the verdict's authority).
- **Rejected:** a capability manifest — see § above (accidental capability drift is not a real failure mode; the honest guarantee is the test run).
