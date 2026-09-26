<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN145 — Text, tweens and widgets

> Tracker: [loft-lang/plans#145](https://github.com/loft-lang/plans/issues/145)
> (`subject:libs`, `status:finished`). Split out of [@PLN144](../144-2d-stage/README.md), whose
> scene arcs share a gate family these do not.
> **Part of the 2-D game stack** — four plans cut from one design: @PLN144 the stage ·
> @PLN145 text/tweens/widgets · @PLN146 content + delivery · @PLN147 the editor. Set overview,
> through-lines and where to start: [`plans/README.md` § Plan sets](../README.md#plan-sets--where-four-plans-are-one-piece-of-work).

## Status

**CLOSED 2026-08-21 — every phase shipped, every package resolves from the registry and every Verify clause is met.** `text2d` 0.4.0 (arc B), `tween` 0.1.0 (`C1`), `stage` 0.18.1 (`C2` + `D1` + `D2`, plus the gate fix in § The CI list), `D0b` answered. **`D0` is DONE on both sides** — moros shipped `lavition_ui` 0.1.0 on 2026-08-20, the red gate in `loft-lang/registry` that predated the submission turned out to be a publish-path defect of ours, and with that fixed their PR rebased, merged and the package now resolves from the registry (2026-08-21) — and no `ui` package will be built (§ The `ui` package that is not ours). These are the libraries a game author writes against
*above* the stage. Each has its own gate family — a metrics seam, a headless font, an event
replay — which is why they are not @PLN144's phases.

## Goal

Ship `text2d` and `tween`, and extend `stage`, so a game sets `.text`, tweens a property and
places a button without writing a rasteriser, an integrator or a hit test. ⚠ The widget
*layout* half is moros's `lavition_ui` and arrives through `D0` — see § The `ui` package that
is not ours, decided 2026-08-20.

## Effort + design

- **Effort:** MH — 11 phases, none above M, **all shipped**: `B0` + `B1` 2026-08-19, `B1m` + `B2` 2026-08-20; `C1` + `C2` + `D0b` 2026-08-20; `D1` + `D2` 2026-08-20; `D0` 2026-08-21. **Design:** ✓ — `D0` was the one call this stream did not own, and it came back as *promote it there, do not extract it here*.
- **Scope:** 2-D games. Follows @PLN144's scope exactly.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong** — filled when the phase is
cut, not when it is implemented.

| Item | Where | Verify | Status |
|---|---|---|---|
| **B0** — a built-in fallback font | `text2d` | ✅ **Shipped** as `text2d` 0.1.0 — a 5×7, 56-glyph face carried as data; pure loft, no file, no `#native`, no GL. Gated on ink under plain `loft test`, and on **every digit drawing AND `'1'` carrying less ink than `'8'`** — a coverage assertion alone would pass a face that drew one blob per character. Three controls fire | ✅ **Shipped** |
| **B1** — glyph atlas | `text2d` | ✅ **Shipped** as `text2d` 0.2.0 — **600 relayouts over ten digits: ten sheet writes, then zero.** `atlas_writes` counts exactly what a GL consumer uploads on, so the property is asserted rather than described. Baseline: the sheet path and the direct blitter agree pixel-for-pixel, with a second assertion that they are not blank together. Three controls fire | ✅ **Shipped** |
| **B1m** — the metrics seam | `text2d` | ✅ **Shipped** as `text2d` 0.3.0 — two runs decide fixed-pitch and the advance comes from the **wide** one. The 1/64 gate is stated as a **property, not a war story**: a 60-char line of a 9.6 px face measures 576, and **no integer advance can produce 576 over 60 characters** — swept in the test. The whole-pixel control answers **540**, the 36 px accumulation exactly. `metrics_builtin` answers through the same seam, which also pins **advance extent vs ink extent** — one trailing gap apart, different questions. Nine controls fire | ✅ **Shipped** |
| **B2** — wrapping + alignment | `text2d` | ✅ **Shipped** as `text2d` 0.4.0 — the trap is **measured, not assumed**: `"héllo"[0..5]` is `"héll"`, four characters, and loft snaps a byte cut outward so it under-fills rather than corrupting. ⚠ **`fit_text` had shipped that way in `B1m`** and is fixed here. Hand-computed table on the **built-in** face (the one target measurable without a font); self-consistency gated over fixed-pitch, fractional-advance and proportional metrics. Two design calls stated: a line always takes **≥1 character** (its control HANGS rather than asserting, and the timeout names the termination test), and an overlong line starts **at** its box under every alignment. Nine controls fire | ✅ **Shipped** |
| **C1** — tween core + easing set | `tween` | ✅ **Shipped** as `tween` 0.1.0 — the two gates turned out to be **one defect with two faces**: float seconds sum to `0.99999999999999989` at 30 Hz and `1.00000000000000133` at 60 Hz, so the animation both parts by rate AND never arrives. Integer base units make the two rates equal *to the bit*. The endpoint needed the **other spelling of a lerp**: `a+(b−a)·t` lands on `b` for 5 of 8 pairs and answers **0.0** for `(1e17, 1.0)`; `a(1−t)+bt` lands 8 of 8. Eleven curves cover 33 by reflection, with the clamp **underneath** the reflections — in front of them an in-out at its midpoint asks the raw curve for `in(1)` and answers `0.5000000000000001`. Chains carry their leftover: discarding runs **43 % long at 30 Hz**. 20 tests, both backends, three controls fire | ✅ **Shipped** |
| **C2** — bind to node properties | **`stage`** (0.16.0) | ✅ **Shipped** — and `advance` settled the placement: `stage.advance(dt_us)` already walks every node once a frame, so a bound tween **rides it** and one call steps the sequences and the property tweens together. That keeps `tween` dependency-free and decides the unit too — **microseconds**, because that is what `advance` takes. THE GATE is a parallel run (`draw_list.loft`'s A2 shape): eight frames of a tweened `x` compared **pixel for pixel** against a hand-moved node, with numbers picked so the hand side needs **no library call** (64 px / 800 000 µs sampled every 100 000 µs is exactly 8 px a frame). ⚠ The switch is gated **cell by cell** — each of nine properties must move its own field and no other. ⚠⚠ **At most one tween per (node, property)**; a second replaces it, and a finished one releases the property. 9 tests, suite 144 on both backends, three controls fire | ✅ **Shipped** |
| **D0** — publish `lavition_ui` | upstream | ✅ **moros's half was DONE 2026-08-20** — `lavition_ui-v0.1.0` tagged (`2692bfd`) and released, artifact re-downloaded from the published URL and checked (38 236 bytes, sha `ea646e67…`, 15 entries, no deps), registry PR [#24](https://github.com/loft-lang/registry/pull/24) open. ✅ **The registry blocker is CLEARED 2026-08-21** — it was never about this submission: gate 1 was failing on untouched `registry:main` because `zttext` and `fixstep` carried `"categories": []`, and the producer that put them there (`registry_maintain.sh` seeding a new package with `[]`) is fixed at source. Backfilled + re-signed as registry `e7a7d43`; the push validation on `main` went **failure → success** across it, and #24's own `validate` passed once its merge ref was refreshed. ✅ **All three Verify clauses are MET, each measured here.** *Resolves:* #24 rebased and merged (registry `717b369`, re-signed `d1e5b63`) and a scratch project declaring `lavition_ui = ">=0.1.0"` installs it from the published URL and answers identically on both backends; gate 3 reproduced the tarball from its tag and the sign step re-hashed it (38 236 bytes, MATCH). *Tests pass after the move:* there was no move — the extraction was **struck**, not done (§ The `ui` package that is not ours). *Every function we depend on has a production caller:* **it does now, and it did not when the clause was written.** `panel_hit_test` gained one in moros `10d62fa`, whose subject is the whole finding — *"A click on the panel turned the camera, because `panel_hit_test` had no caller"* — so the surface this plan asked to depend on was not merely unused, it was **wrong**, and only a consumer could say so. Counted here off `use` lines rather than filenames: **13 of 33 public functions still have none**, and `lavition_ui`'s README now names all thirteen as *a proposal* — which is [LIBRARY_AUTHORING § 2e](../../LIBRARY_AUTHORING.md)'s second clause, the honest label rather than a loophole. See below | ✅ **Done — resolves from the registry** |
| **D0b** — does `input` fit a consumer that already exists? | probe only | ✅ **Answered** ([`probe-d0b.loft`](probe-d0b.loft), both backends identical) — **adopt it, and do not expect it to resolve modifiers.** dryopea's 33-row action set expresses through `Bindings` and replays through `input_tick_from_state` exactly as `D1` needs; edge detection is correct headless. ⚠ But a **modifier is not expressible**, and it is structural: `ActionBinding` is a name + key list, `AxisBinding` is two key *codes*, and neither has a slot for *"suppressed while Ctrl is held"*. Measured: `input` answers **identically** for plain `S` and `Ctrl+S` (`pan_south=true, save=true` both times), so all five of dryopea's Ctrl-combos are indistinguishable from their plain twins — and an axis cannot carry the rule either, which is why dryopea's `bnd_axes` is **empty** and its four pan keys are four actions. ⚠⚠ **The premise was wrong**: dryopea ADOPTED `input`, so the widgets would be its *second* consumer, not the fourth — [PRIOR_ART](../144-2d-stage/PRIOR_ART.md) corrected | ✅ **Answered** |
| **D1** — widget states over stage routing | **`stage`** (0.17.0) | ✅ **Shipped** — a replayed stream drives the exact state sequence (8 events, both buttons, every state hand-computed first); press-then-leave-then-release does **not** fire while press-then-drift-a-pixel does; touch never shows `Over`, and **one** recorded stream replayed as mouse and as touch fires identically, so the pointer kind changes only what is *shown*. ⚠⚠ **The invariant: a node reads `Down` exactly when a release would FIRE it** — one predicate serves the picture and the click, so a control that breaks the capture check does not even redden the down-means-fires gate: that class is **unrepresentable**, not caught. ⚠ **The fourth clause was struck**, not met — it rested on a premise that did not survive contact; see § The `ui` package that is not ours. 7 tests, suite 151 both backends, three controls fire | ✅ **Shipped** |
| **D2** — focus, tab order, text field | **`stage`** (0.18.0) | ✅ **Shipped** — replayed keystrokes (incl. multi-character IME commits) produce the exact buffer; tab order is the declared order and wraps both ways. ⚠⚠ **Every index is a CHARACTER index and no byte index ever enters** — a caret *is* a count, and the control proves it: byte-slicing the one edit primitive leaves `"héllo"` **unchanged** after a backspace over the `é`, while all ten ASCII tests still pass. ⚠ **One `splice` does every edit** (insert / backspace / delete / replace-selection / paste). ⚠⚠ **The modifier dilemma was a false dichotomy** — neither answer was needed: `graphics`'s event queue already delivers `gl_event_mods()` per event, and a text field wants *this keystroke with these modifiers*, not an action. 11 tests, suite 162 both backends, three controls fire | ✅ **Shipped** |
## Effort per phase

| Phase | E | What the effort actually is |
|---|---|---|
| **B1** | M ✅ | *Done.* Rasterize glyphs once into an atlas, keep a (font, size, codepoint) → uv map, build a text node as one quad per glyph fed through A3's buffer, so `.text =` re-lays-out quads and uploads nothing. Effort: shelf packing, atlas growth when it fills, and both backends producing the same atlas *shape* even where glyph pixels differ. |
| **B0** | S ✅ | *Done.* A compact bitmap face baked in as data plus a pure-loft blitter — no file, no `#native`, no GL. Small, and it is the phase that unblocks a shipped consumer rather than one that makes an unshipped one faster: today the text path needs a GL context **and** a native rasteriser **and** a font file, so a repo that tests its UI headlessly answers by having no text. |
| **B1m** | XS ✅ | *Done.* Two measured runs at startup, a 1/64-px advance, and three derived helpers — shipped as METHODS on `Metrics`, because `text_width` already exists in `text2d` and two free functions of that name collide. Note also Nearly free — it is `lavition_ui/src/font.loft` lifted, and its shape is a **finding**, not a preference: one run cannot distinguish a fixed-pitch font from the browser's proportional stand-in, and whole-pixel truncation cost that tree a 31 px error on a single line. |
| **B2** | S ✅ | *Done.* Greedy breaking on measured advances, three alignments, and the character-vs-byte trap — `len(text)` counts characters, the indexed read is bytes. Per-target break tables (see the Verify column). |
| **C1** | S ✅ | *Done.* Shipped WITHOUT the setter (that is `C2`) and without a `fixstep` dependency: a duration is an integer count of whatever unit the consumer's clock already runs in, so a second definition of *how long a second is* never gets made. ⚠ The predicted effort — "a clamp everyone forgets" — was the wrong half. The clamp is real but it is a **placement** question, not a forgetting one, and the accumulation defect underneath it is the one that makes a 30 Hz animation never arrive. Sequencing is eleven lines because `advance` answers a **leftover** rather than a boolean. |
| **C2** | XS ✅ | *Done, and the enum-plus-switch prediction held exactly.* What the note did not see is that the switch is also the first public way to **move** a node — `stage` could place one and never shift it, so a consumer had to write `st_nodes[i].nd_x` directly. So `set_prop` ships beside `tween_prop`, through the same switch. ⚠ Filed on the way: [loft#1039](https://github.com/loft-lang/loft/issues/1039) — `lib::Enum.Variant` does not parse, though `lib::Struct{}`, `lib::CONST`, `std::abs()` and `lib::Enum` *as a type* all do. |
| **D0** | — | *Asked.* The conversation cost what it was predicted to cost, and paid for itself twice — it corrected our Verify and found a live bug in their `src/`. |
| **D0b** | XS ✅ | *Done.* One probe program, and it did the job a probe is for — it did **not** retire the dependency, it retired the *reason to doubt it* and replaced it with a named limit `D1` and `D2` can plan around. ⚠ The phase's own framing was the thing that broke: two of the three "wrote their own" consumers were miscounted, one of them because a file was judged by its NAME (`framekey.loft` is a frame-reuse digest, not a keyboard file). |
| **D1** | S ✅ | *Done.* The predicted effort — *"the effort is the replay harness"* — was **wrong in a good way**: A4's path was already injectable (every entry point takes coordinates and reads no device), so the harness cost nothing and no second seam was invented. The real effort was deciding what `Down` MEANS, and that answer made a bug class unrepresentable rather than tested. The **extraction** half turned out not to be ours to do at all. |
| **D2** | M ✅ | *Done, and the effort note called it: the text field WAS the phase.* ⚠ It also predicted *"multi-byte indexing returns for a third time"* — it did, and the cure was to refuse it entry: the model counts characters everywhere and pixels never appear, so caret↔x stays with whoever draws (`text2d`). ⚠ A gap found on the way, **recorded not fixed**: the **stdlib has no character slicing** — `len` (chars), `size` (bytes), `byte_at`, byte-range `s[a..b]`, nothing between. `text2d` hand-rolled `take_chars` for `B2` and this phase needed the same walk again; **two libraries independently** is the admission test for a primitive belonging one level down. |

## Targets

Follows @PLN144: interpreter, `--native`, `--html`, `--native-android`. ⚠ **`D2`'s IME gate
cannot pass on Android today** — NativeActivity delivers `KeyEvent`s and has no `TextEvent`, so
*composing* text needs a new `gl_text_input()` text-stream API in `graphics`. Discover that
before `D2` is cut, not inside it; the desktop and browser halves of the same gate are
unaffected.

## Phase ordering

**`B0` first — it depends on nothing and unblocks a shipped consumer today.** Two UI surfaces
in dryopea have no text at all (`hud.loft` draws digits as rectangles, `picker.loft` shipped
without labels) because the text path needs a GL context *and* a native rasteriser *and* a
font file. Everything else waits on @PLN144: `B1` on its atlas, `C` on its transforms, `D` on
its hit-test — and `D0` on moros.

**@PLN144 is complete (2026-08-20), so nothing in this plan waits on it any more.** `B2`,
`C1`/`C2` and `D0b` are all unblocked and independent of each other; only `D0` is still
someone else's clock — moros's, and now the registry maintainer's, and `D0b` is the phase that must run before `D1` commits to `input`.

**Everything shipped, `D0` last and a day after the rest.** `C1` shipped `tween` 0.1.0, `C2`
bound it to the stage, `D0b` measured `input`, and `D1` + `D2` put the interaction model in
`stage` 0.18.0 (0.18.1 with the gate fix). `D0` was the only phase this stream did not own,
and the thing that actually held it up was neither tree's submission but **our own publish
path** — see § `D0`.

⚠⚠ **Three of arc D's premises did not survive contact, and that is the arc's real lesson.**
`D0b` was scoped on *"three consumers wrote their own input layer"* (the true count was one);
`D1` on *"extract `lavition_ui` into a `ui` package"* (it is moros's, and its zero-dependency
claim forbids the state machine anyway); `D2` on *"this phase must carry a ctrl rule or `input`
must grow one"* (neither — the event queue already carries per-event modifiers). All three read
as sensible when written and all three were settled by **measuring rather than reasoning**, each
for the cost of a probe. A phase's stated dilemma is a hypothesis about the world, and arc D
went 0 for 3.

⚠ **`C1`'s CI check found that `stage` could not pass the repo's gate — fixed, and the
half that outlived it is in § The CI list below.**

### `D0` — asked, answered, and then shipped — 2026-08-20

Asked moros directly. The first answer was **not yet** (*"yes in principle, blocked on making
the hit-test half honest here first"*). Later the same day the maintainer settled the name —
**`lavition_ui` stands** — and moros shipped: tag `lavition_ui-v0.1.0` at `2692bfd`, a GitHub
release, and registry PR [#24](https://github.com/loft-lang/registry/pull/24) (+19/−0), no
dependencies. Their agent verified the artifact by **re-downloading it from the published
URL** rather than trusting the local build (38 236 bytes, sha `ea646e67…`, 15 entries) — so a
later disagreement would be about reproducibility rather than about which file was uploaded.

⚠⚠ **The phase is still not closeable, and the reason is in neither tree.** The revised Verify
below opens with *"the package resolves from the registry"*, and it does not: PR #24's
`validate` check is RED and `lavition_ui` is absent from the live index. Measured here rather
than taken on report — the live `index.json` holds 36 packages, and exactly two, `zttext` and
`fixstep`, carry `"categories": []`, which `tools/validate.py` requires to be a non-empty list.
So **gate 1 fails on untouched `main`** and every submission PR inherits a red tick that has
nothing to do with the submission. Clearing it means editing `index.json` in `loft-lang/registry`
**and re-signing it**, which needs the signing key: a maintainer action, not a loft or a moros
one. Until then `D0` is *shipped upstream, blocked in the registry*.

⚠⚠ **CLEARED 2026-08-21, and the cause was a PRODUCER rather than the data.**
`registry_maintain.sh` seeded a package new to the index with `"categories": []`, and the
docs gate that rejects an empty one landed 2026-06-19 — so **every package first published
since that date went in unmergeable**, and `zttext` and `fixstep` are exactly the two that
had been. That is why the red tick appeared on somebody else's PR: gate 1 runs over the
WHOLE index, so the first submission after a bad publish inherits it, and clearing it needs
the signing key.

Fixed in both directions. The **door**: the own-lib fold and the `submissions/` drain read
`[package] categories` off the manifest and refuse a package the index has never seen without
one. The **chokepoint**: `scripts/registry_schema_gate.sh` runs gate 1 out of the checkout
being signed and `registry-sign.sh` refuses on a rejection, so a hand edit cannot get an
unmergeable index signed either — and it IMPORTS `tools/validate.py` rather than restating
its rules, because a second list of one type's facts drifts — and that one already has, in
both directions ([loft#1052](https://github.com/loft-lang/loft/issues/1052)). Loft-side `583c31da`,
registry `e7a7d43`.

Measured before and after rather than reported: the registry's own push validation on `main`
was **failure** at `205f8398` and **success** at `e7a7d43a`, and PR #24's index with the two
backfills passes gate 1 locally while the same index without them fails on `zttext`.

⚠ **Two operational traps found clearing it, both of which read as the wrong diagnosis.**
A CI **re-run replays the cached merge ref**: after fixing base `main`, re-running #24's failed
job used the merge commit computed BEFORE the fix and failed identically on the same
`zttext` — a fresh `pull_request` event (close + reopen) is what recomputes it. And the same
day's publish rewrote `index.json` wholesale, which left #24 `CONFLICTING` through no fault of
its own; a foreign submission racing a maintenance publish is a standing hazard of a
single-file index, not a one-off.

**Closed 2026-08-21.** #24 rebased onto the rewritten index, `validate` green (gate 3 cloned
`lavition_ui-v0.1.0` and re-packaged it — *reproduces from source*, which proves the uploaded
tarball matches the TAG rather than someone's working tree), merged as `717b369` and re-signed
as `d1e5b63`. ⚠ **A merged registry PR leaves the index unsigned for its content** — `gh pr
merge` changes `index.json` on the remote and nothing re-signs it, so the re-sign is part of
the merge, not a follow-up. The Verify's first clause was then checked by CONSUMING it: a
scratch project declaring `lavition_ui = ">=0.1.0"` installs from the published URL and answers
identically on interpreter and `--native`.

⚠ **`registry_maintain.sh` has no per-package filter** — its worklist is every own lib behind
the index, which was **10** on the day three were wanted. Five published; **five `hex_*` were
refused by the compat gate**, each declaring an `api_compatible_with` that measurably does not
hold (`compat check --full` → `0.1.0: BREAK`). That refusal is the gate working, and those
libraries' own fix: restore compatibility or raise the floor.

⚠ **A doc of ours sent them the wrong way, and they were right not to follow it.**
[REGISTRY_SUBMIT.md](../../REGISTRY_SUBMIT.md) § 4 calls staging a `submissions/` file the
recommended route, citing the race filed as loft#1045. Only the MAINTAINER half of that is
wired (`scripts/registry_maintain.sh` drains the directory); `loft-lang/registry` has no
`submissions/` directory, nothing in its `tools/` or `.github/` mentions one, and its own
`SUBMITTING.md` documents the `index.json` edit. They followed the registry's page, said so in
the PR, and that was the correct call. Our page now says which half exists.

⚠⚠ **The reason lands on the exact property we asked to pin, and it corrects THIS ROW'S
VERIFY.** `panel_hit_test` has **zero production callers** in moros — it is exercised by its
own tests and by nothing else, and their `editor_client.loft` says so in its own words
(*"`verbbar_hit` and `panel_hit_test` were both built, tested green and invoked by nothing —
the commonest defect here"*). Counted there: **15 of 31 public functions have no production
caller**, because that `pub` list is sized for the test suite rather than for a consumer.

So this row's Verify — *"its own tests pass unchanged after the move"* — **could not have
caught that**. It is the right check for the *move* and says nothing about whether the surface
was ever **honoured**. Their observation, and it is correct; the Verify is widened accordingly:

> **`D0` verify (revised):** the package resolves from the registry, its own tests pass
> unchanged after the move, **and every public function we depend on has at least one
> production caller in the tree that owns it** — a surface proven only by its own tests is a
> surface nobody has agreed to.

⚠ **A second warning for `@PLN147`:** the **verbbar** half (`spec_verb`, `spec_verb_on`,
`verbbar_build`) *will* change shape — their `EDITING_MODES` work makes the verb table data, so
adding a house type touches no code and those signatures move. Their advice: **depend on the
panel half; treat verbbar as 0.x that moves.**

Not blocking, measured on their side: 80 pass on both backends, `LOFT_STORE_GUARD=1` clean, and
`LOFT_DENY_WARNINGS=1` fails on two **test** files only. Pin #2 is safe — their `src/` has no
`while` at all, no `#native`, no I/O and no GL, so it is admissible loft by construction; they
would declare a `[sandbox]` policy before publishing.

**The conversation also found a live bug in their `src/`** (not just their tests):
`font.loft:105`–`111` computes a CHARACTER count and applies it as a BYTE range — the same
defect `B2` fixed in `text2d`'s `fit_text`, which is where `font.loft` was lifted from. **Seven
call sites inside the package**: every button label, every hotkey, every list entry, the status
line, the subject line and both verb slots. Every one of their tests is ASCII, which is exactly
why it has stayed green.

### The `ui` package that is not ours — decided 2026-08-20

This plan's goal named `ui` as a third package to ship beside `text2d` and `tween`, and `D1`
carried a clause requiring `panel_hit_test` to answer *"the same `UiHit` it answers today"* —
i.e. that we extract moros's `lavition_ui`. **Doing `D1` turned up three facts that retire
that plan, and it is struck rather than deferred:**

1. **`lavition_ui` is moros's to promote, and `D0` IS that promotion.** @PLN147 already reads
   arc D as being *about that package* — *"`lavition_ui` (@PLN145 `D`) is what the editor's
   panels are made of"*. Their standing rule is that a library is promoted once battle-tested
   **there**; a copy published from here is the shape [PRIOR_ART](../144-2d-stage/PRIOR_ART.md)
   already records that rule as rejecting.
2. **The generic name is the one they deliberately avoided.** moros's `EDITOR_UI.md` chose
   `lavition_ui` over `ui` precisely because a generic name is *"generic enough to collide in
   a shared registry"*. Shipping `ui` would mint exactly that collision.
3. ⚠⚠ **The new half could never have lived there.** `lavition_ui`'s `loft.toml` states an
   empty dependency list as **its claim** — *"nothing here needs a world, a lattice, a window
   or a GL context"*. `D1`'s four states need `stage`'s routing, so putting them in
   `lavition_ui` would break the one property that package advertises.

**So the split is structural, not a compromise:** the *layout and hit-test* half is
`lavition_ui`'s and reaches consumers through `D0`; the *interaction* half is `stage`'s,
because that is where the press/release/capture it extends already live. Neither half wants
the other's dependencies, which is why no third package is needed to hold them.

⚠ **Consequence for the goal line and for `D2`:** this plan ships `text2d` and `tween`, and
extends `stage`. It does not ship a `ui`. `D2`'s interaction half (focus, tab order, caret)
lands in `stage` for `D1`'s reason; its *rendering* half is `lavition_ui`'s.

## The CI list — the packages are published, and three of them are not gated

⚠ **Found by `C1`'s CI check and fixed the same day: `stage` could not pass
`LOFT_DENY_WARNINGS=1 loft test`** — it reddened all sixteen of its files on four
`never-read` parameters. Fixed as `stage` 0.18.1 (`loft-libs-graphics` `a75500a`), and
the four were **not silenced alike**, which is the part worth keeping: two had already
gained a use from `D1`; `ensure_light_target`'s `const Stage` was **removed** because an
offscreen colour target is a Painter and a size and nothing about the scene changes it;
and `gl_draw_band`'s `screen_w` was **kept and marked `_screen_w`**, because its sibling
`gl_composite` uses both and dropping one of two symmetric floats would leave a bare
positional float at three call sites that could take either dimension. A receiver spelled
`self` is not a rename away, and did not need to be.

⚠⚠ **The live half is that nothing gated any of this, and still does not.**
`loft-libs-graphics` holds **seven** package dirs and `library-ci.yml` lists **four** —
`["graphics", "gridmesh", "imaging", "shapes"]`. So `stage`, `text2d` and `tween` are
published to the registry with **no CI at all**, and the reason `stage` was never red is
the same reason it would never have gone green: it was not in the list. That list is
**computed from the package dirs** by `scripts/deploy-library-ci.sh`, so a refresh adds
all three at once.

**Measured here 2026-08-21 on BOTH backends, because the gate runs both:** the reusable
workflow runs `loft --interpret --tests`, `loft --native --tests` and `loft test --deps`, each
with `LOFT_DENY_WARNINGS=1` unless the package carries an `.allow_warnings` file — and no
package in the repo does, so none of this is exempted. All three pass: `stage` 162, `text2d`
35, `tween` 20, zero `Warning[…]` lines, interpreter and `--native` alike. ⚠ The first
measurement was interpreter-only and would not have been enough to recommend landing.

**Refreshed 2026-08-21** — `scripts/deploy-library-ci.sh --branch unify-library-ci-fpm`, one
branch per repo, no PRs (landing is the maintainer's call).

⚠⚠ **The refresh found a second drift, and it is a deployment one rather than a data one.**
The script also writes two org lifecycle stubs (`fpm-apply.yml` / `fpm-strip.yml`, which
label `fixed-pending-merge` on a `Fixes #NNN` push). **Seven of the eight `loft-libs-*` repos
have neither.** Not because the unification failed — every repo's `unify-library-ci` PR
MERGED, back on 2026-07-26 — but because that PR predates the stubs: `loft-libs-core` #26
changed `library-ci.yml` and nothing else. The script grew its second half afterwards and was
never re-run. `loft-libs-graphics` is the only repo that has them, and only because it carried
them by hand all along, which is exactly what hid the gap.

So in seven repos a `Fixes #NNN` push has been labelling nothing, and **nothing fails when a
label is missing** — the same failure shape as the `categories` producer in § `D0`: a fix
applied to the generator, and the generator never re-deployed. A tool whose output is only as
current as its last run needs the run to be part of the change, not a follow-up.

⚠ The stale `unify-library-ci` branches from the merged PRs are still on all eight remotes and
block a same-name re-push; hence the new branch name rather than a force-push over someone
else's merged history.

## The sandbox boundary

Every package here declares which side of loft's admission boundary it is on — **trusted
engine** (unbounded internals, an admitted-safe API) or **admissible loft**. @PLN86 shipped
admission (`src/sandbox.rs`), and the choice is a **design-time property of an API**: cheap
while the signatures are being written, a re-architecture afterwards. Get it right and a mod
is just more admitted code, with no second code path to keep in step. See
[LIBRARY_AUTHORING.md](../../LIBRARY_AUTHORING.md).

`tween` (`C1`) is **admissible loft** and says so in its README: no `#native`, no I/O, and
its one collection loop is a `for` over a bound written down at the loop — `len(steps) + 1`,
which is its exact worst case. ⚠ That shape was chosen for admission, not for style: an
unbounded `while` in a sandboxed def is refused at load, and the natural spelling of a chain
walk is a `while`.

## Package authoring

loft#976 makes a bare `use <mod>` bind the package's own file, so a sibling's basename cannot
amputate a public surface. The lip: `use <pkg>` **inside** `<pkg>` means the *package*, and a
suite written as `tests/<pkg>.loft` is what amputated nine published libraries. Copy moros's
`tools/basenames.sh` guard rather than reinventing it.

## See also

- [@PLN144](../144-2d-stage/README.md) — the stage these sit on; [`PRIOR_ART.md`](../144-2d-stage/PRIOR_ART.md)
  carries what moros and dryopea already built, including `lavition_ui` and the metrics seam.
- [@PLN146](../146-content-delivery/README.md) — fonts arrive through its asset pack.
- [@PLN147](../147-content-editor/README.md) — the editor's panels are `D`'s widgets, so `D0`
  gates that plan as well as this one.
- [loft-lang/plans#145](https://github.com/loft-lang/plans/issues/145).
