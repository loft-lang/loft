<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN147 — The in-browser content editor

> Tracker: [loft-lang/plans#147](https://github.com/loft-lang/plans/issues/147)
> (`subject:libs`, `status:future`). Fourth of the 2-D set:
> [@PLN144](../144-2d-stage/README.md) the stage · [@PLN145](../145-authoring-libs/README.md)
> text/tweens/widgets · [@PLN146](../146-content-delivery/README.md) the pack · this.
> **Part of the 2-D game stack** — four plans cut from one design: @PLN144 the stage ·
> @PLN145 text/tweens/widgets · @PLN146 content + delivery · @PLN147 the editor. Set overview,
> through-lines and where to start: [`plans/README.md` § Plan sets](../README.md#plan-sets--where-four-plans-are-one-piece-of-work).

## Status

**Open — design ready, nothing built.** Deliberately **not** a Godot-shaped desktop editor:
matching an established engine's feature list is a decade of work to arrive second. This is the
editor for **a game's own content**, in the browser, and it is where loft's actual advantages
meet — edit from a link, stream from static hosting, user content that cannot break the host.

## Goal

Open a URL, edit a game's scenes, sprites and animations, and have the running game show the
change — with no export step, because the editor writes the file the game already reads.

## The one invariant

> **The editor edits the same store the game loads.** Not an editor format plus an export
> step: @PLN146's pack, written here and read by the runtime. Editor↔runtime agreement is
> then **structural rather than maintained**.

Every gate below tests that agreement rather than testing the editor, which is the difference
between a claim and a property. Two things follow for free instead of being built: **hot
reload is just reloading the store**, and **the editor's save is the game's save**, so there is
no import path to keep in step — the seam other engines maintain by hand does not exist here.

## Effort + design

- **Effort:** H — 16 phases, none above M. **Design:** ✓ for S/T/U/X, ~ for V.
- **Scope:** 2-D games, following @PLN144's scope exactly.

## Composition matrix — Stage A

The axes this touches: *edit kind* (place · move · delete · re-import · undo) × *target*
(instance · asset · scene) × *round trip* (in-session · after reload · read by the game) ×
*backend* (`--html` · `--interpret`). The off-diagonal cells are the point — a delete, undone,
after a reload, read by the game. Write them as probes before `S1`.

## Sub-arcs

`Verify` is what would go **red if the phase were done wrong**.

| Item | Where | Verify | Status |
|---|---|---|---|
| **S1** — a page that opens a pack, lists it, writes back | `editor` | edit → close the tab → reopen → **it is there** (moros's `B4` sentence, for a pack), and the file written is **byte-identical** to what the game loads. Needs only @PLN146's `F1`, so it starts before the other plans finish | Open |
| **S2** — undo / redo | `editor` | N edits then N undos returns the pack **byte-identical** to the start — the only formulation that means anything, since "looks the same" passes on a lost field. Redo after undo restores exactly | Open |
| **T1** — place, move, delete instances | `editor` | **the scene as loaded by the game renders the pixels the editor showed.** This is the invariant itself, and it is the gate the whole plan exists to be able to write | Open |
| **T2** — pick and select | `editor` | the editor and the game resolve **the same screen point to the same instance**, including through a sprite's transparent texels (@PLN144 `A4`) — click through the tree in both, or in neither | Open |
| **T3** — the presentation knobs, exposed | `editor` | setting origin / `layer` / `depth` reorders the view to match the **same hand-computed occlusion table** @PLN144's `P1` is gated on. One table, two consumers | Open |
| **U1** — drop a PNG in | `editor` | the imported sprite round-trips pixel-exact, and @PLN146's atlas invariants hold on it unchanged — premultiplied, 1 px padding, correct page | Open |
| **U2** — re-import | `editor` | change the PNG and the atlas entry **and the derived collision proxy** follow with **no hand edit**; the proxy still contains every opaque texel within its bound (`F7`) | Open |
| **U3** — an asset the game cannot load is refused **here** | `editor` | a malformed or oversized import fails at the drop with a reason, never at the game's first frame. The editor is the only place a content error can still be cheap | Open |
| *— arc **X**: the sprite editor, with animation —* | | | |
| **X1** — a `.draw` scene renders live in the page | `editor` | the page's render is **pixel-identical to `drawing`'s native render** of the same scene. Same oracle chain @PLN146's `W2` established, now carried across targets — and it generalises `draw.py`'s re-render-on-save daemon into the browser | Open |
| **X2** — select a named element | `editor` | clicking a mark selects the `name` it belongs to, and the editor's answer equals `drawing`'s own hit answer for that point. The grammar's `name <tag>`, which exists for measurement, turns out to be the selection handle already | Open |
| **X3** — a drag edits the **source text** | `editor` | dragging to a position produces the scene text you would get by **typing those numbers**, and drag-then-undo returns it **byte-identical** (`S2`'s discipline). This is the phase that keeps it an editor of source rather than a paint program | Open |
| **X4** — animation: keyframes on named elements | `editor` | sampling the timeline at frame times yields the same images as hand-editing the scene per frame. A walk cycle is **keyframes on tagged marks**, not N drawn bitmaps — so it stays diffable, and re-timing costs nothing | Open |
| **X5** — bake to atlas cells | `editor` + `assets` | the baked cells are pixel-identical to the timeline's samples, and @PLN144's `P4` plays them unchanged. Baking at pack time, not evaluating at run time — the same call `A13`'s blur and `F1`'s atlas already make | Open |
| **V1** — hot reload into a running game | `editor` | a change reaches a running game within N frames **and the game's world state survives it** — a reload that resets the player is a restart wearing a nicer name | Open |
| **V2** — the editor runs the game's own scripts | `editor` | a script that would hang or reach outside its capabilities is **refused at load** by @PLN86 admission, in the editor, before it can reach a player | Open |
| **V3** — multi-client editing | `editor` | deferred behind a trigger; `routing` already proves the shape — two browsers, echo-free, late joiners see current state | Deferred |

## Effort per phase

| Phase | E | What the effort actually is |
|---|---|---|
| **S1** | M | The page shell — @PLN146's pack over the `--html` page filesystem, a list, a write. moros plan 22 has this pattern working for a hex world (`W1` world-as-bytes, `P6` page FS, `B4` survives-reload); the effort is applying it to a pack rather than inventing it. |
| **S2** | S | Undo as a stack of inverse edits, not snapshots — a pack is large and a snapshot per keystroke is a memory leak with a nice name. The byte-identical gate is what forces the inverses to be real. |
| **T1** | M | Instance placement over @PLN144's stage, with the editor's own scene being an ordinary scene. Most of the effort is *not* writing a renderer: the editor renders through `stage` exactly as the game does, which is what makes the pixel gate possible at all. |
| **T2** | S | Reuse `A4`'s pick verbatim. If it needs a second implementation, the seam is wrong — and the gate says so by comparing the two answers rather than checking one. |
| **T3** | S | Three knobs surfaced as controls, and the occlusion table borrowed from `P1` rather than rewritten. A second table would drift; one table checked by two consumers cannot. |
| **U1** | S | A drop target, then `F1`'s packer unchanged. The pipeline other engines maintain by hand is one call here, because the pack already does premultiplication, padding and page choice. |
| **U2** | S | Re-run the derivation and diff. Nothing new — `F7` already derives the proxy from alpha; this proves it stays derived under a change, which is the property that makes it worth having. |
| **U3** | S | Validate at the drop against the same rules the loader enforces. The cost is finding them all; the value is that a content error stops being a runtime bug. |
| **X1** | M | The `drawing` renderer compiled into the editor page, plus a text pane. Most of the effort is the cross-target pixel gate, not the rendering — and passing it proves `drawing` behaves identically in wasm, which nothing else in these plans checks. |
| **X2** | S | Reuse `T2`'s pick, then map the hit mark to its enclosing `name`. Cheap because the grammar already tags marks; had it not, this phase would have needed a selection model invented for it. |
| **X3** | S | The round trip is the whole phase: a drag must write the *source*, and the gate compares against hand-typed numbers rather than against a screenshot. **This is the design decision the arc turns on** — the editor edits `.draw` text, so sprite art stays reviewable in a diff, which a PNG never is. |
| **X4** | M | A timeline over named elements, with a keyframe holding that element's transform. It is why the sprite is a scene and not a bitmap: a walk cycle becomes a handful of poses on tagged marks, re-timing is free, and a fix to the silhouette fixes every frame at once. |
| **X5** | S | Sample, hand each frame to `W6`'s scene-to-pack route, done — the packer already premultiplies, pads and places. Bake rather than evaluate live, so the runtime keeps knowing nothing about `.draw`. |
| **V1** | M | Reload the store and rebind, without resetting the world. The hard part is exactly the second half of the gate — what state is content and what state is play, a line no engine draws for you. |
| **V2** | S | Point @PLN86 admission at the script surface. Cheap, and it is the differentiator: same-process, full-speed, refused at load — not a second VM. |
| **V3** | — | Deferred. Trigger: a second person needs to edit the same pack at the same time. |

## Prior art it must not duplicate

**moros plan 22 already built the shell pattern** — a `--html` editor page whose world is bytes
(`W1`), whose page has a filesystem (`P6`), and where *build something, close the tab, come
back and it is there* has been true since `B4` (2026-08-15). `S1` applies that to a pack; it
does not reinvent it, and it does not schedule moros work — by their own rule a library is
promoted once battle-tested where the consumer lives. See
[`PRIOR_ART.md`](../144-2d-stage/PRIOR_ART.md), shared across all four plans.

`lavition_ui` (@PLN145 `D`) is what the editor's panels are made of. ✅ **`D0` is done and no
longer gates `T3`** — moros published `lavition_ui` 0.1.0 and it resolves from the registry
(2026-08-21). ⚠ Two conditions came with it, both of which shape `T3` rather than block it:
**depend on the panel half and treat verbbar as 0.x that moves** (their `EDITING_MODES` work
makes the verb table data, so `spec_verb`, `spec_verb_on` and `verbbar_build` change shape),
and **13 of its 33 public functions still have no production caller** and are labelled a
proposal in its README — so a `T3` that reaches for one of those thirteen is the first
consumer to agree to it.

## Targets

The editor is a page, so its first target is the browser — including a **phone browser**, which
is the same surface crew_punk's six-consoles constraint assumes. With `--native-android` it can
equally ship as an APK from the same source. That makes `T2`'s pick and `D`'s widgets carry a
touch obligation rather than a mouse one: no hover, and finger-sized targets. Gated on
[loft-libs-graphics#32](https://github.com/loft-lang/loft-libs-graphics/issues/32) for the APK
route; the browser route needs nothing extra.

**Arc X is the toolkit generalised.** `crawler/tools/draw.py` re-renders a scene on save and
stops there: no selection, no direct manipulation, no animation, and it is Python. @PLN146's
arc `W` makes the renderer loft; arc `X` makes it an **editor** — live in a page, with the
marks selectable, dragging that writes the source back, and a timeline over the same tags.

The invariant it adds to this plan's: **the editor edits the SOURCE, not a bitmap.** Every gate
compares a visual edit against the text edit that should equal it. That is what keeps sprite art
**reviewable in a diff** — a `.draw` scene is readable in a pull request and a PNG is not — and
it is why animation costs so little: a walk cycle is keyframes on tagged marks, so re-timing is
free and a fix to the silhouette fixes every frame at once.

## Open design questions

1. **What is content and what is play?** `V1`'s second half needs the line drawn: reloading a
   tileset must not reset the player, reloading a *level* probably must. No engine draws this
   for you and the answer is likely per-record-type in `F1`'s schema.
2. **Does the editor edit a live game, or a pack the game re-reads?** The second is simpler and
   is what `V1` assumes. The first is what people expect from an editor. Settle before `V1`.
3. **Where does the editor itself live** — its own package, or a mode of the game binary? A
   mode ships the editor to every player, which is either the point or a mistake, depending on
   whether user content is a goal.

## Cross-arc dependencies

- **[@PLN146](../146-content-delivery/README.md)** — `F1` is the only hard prerequisite for
  `S1`; `F7` for `U2`; the atlas invariants for `U1`.
- **[@PLN144](../144-2d-stage/README.md)** — `A4` for `T2`, `P1` for `T3`, the stage for `T1`.
- **[@PLN145](../145-authoring-libs/README.md)** ✅ **closed** — `D` is the panel kit and `D0`
  landed it in the registry, so this plan is no longer gated on it; `text2d`, `tween` and
  `stage` 0.18.1 are the rest of what the editor draws with.
- **SANDBOX.md / @PLN86** — `V2` is an application of shipped admission, not new work.
- **The sandbox boundary and package-authoring rules** apply here as in the sibling plans; see
  [LIBRARY_AUTHORING.md](../../LIBRARY_AUTHORING.md) § 2d.

## See also

- [@PLN144](../144-2d-stage/README.md) · [@PLN145](../145-authoring-libs/README.md) ·
  [@PLN146](../146-content-delivery/README.md).
- [REMOTE_STORES.md](../../REMOTE_STORES.md) — why the pack streams from static hosting.
- [loft-lang/plans#147](https://github.com/loft-lang/plans/issues/147).
