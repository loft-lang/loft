---
name: draw
description: >-
  Make a considered image or 3D scene from a described intent the iterative-craft
  way — block in, render, measure, critique against the intent, refine across
  passes — instead of trying to emit a finished result in one shot. Use this
  whenever the user wants to DRAW, sketch, illustrate, depict, compose, or paint a
  2D picture, OR model / build / lay out a 3D scene, asset, or environment (e.g. a
  game prop, a room, a character, a landscape), ESPECIALLY when they give a subject,
  a mood, or a composition to hit ("an old woman at dusk", "a cosy tavern interior",
  "a low-poly pine tree", "make it feel ominous"). Also use when iterating on or
  critiquing such an image/scene. Reach for it even if the user doesn't say
  "iteratively" — the loop is the point, and one-shot guessing is what it replaces.
  Do NOT use for text-to-image prompts handed to a diffusion model, for
  charts / plots / diagrams / dashboards (data visualisation), or for UI / web
  layout.
---

# Draw — iterative visual making (2D and 3D)

## Quick start

1. **Freeze the intent in a file** before any mark — predicates, feeling, composition (§ The loop).
2. **Run `python3 sketch/draw.py scene.draw` in the background** — it re-renders on every save,
   into `$SKETCH_OUT` (default `<tmp>/loft_sketch`): `stats.txt` (checks, density, composition —
   text, read it most passes), `canvas.png`, `canvas_check.png` (red = failed check), `preview.png`.
3. **Loop:** edit the scene → read `stats.txt` → look at an image only for gestalt and feeling →
   critique cold → refine. Grammar: [`references/2d.md`](references/2d.md),
   [`sketch/README.md`](../../../sketch/README.md); 3D: [`references/3d.md`](references/3d.md).

## Why a loop

A finished image is **downstream of perceiving**: you commit a mark, *see the gap*
between what you meant and what appeared, and adjust — over many passes. That is why
one-shot output fails, here as by hand; nothing is final until the loop converges, and
your judgment of the result *is* the engine. The same loop drives **2D drawing** and
**3D scene-making**; the method below is medium-agnostic.

The reasoning behind each choice is in [`doc/claude/DRAWING.md`](../../../doc/claude/DRAWING.md)
(read it to understand *why*; this file is the recipe). Current mechanics live in
[`sketch/README.md`](../../../sketch/README.md) and the two references.

## The loop

1. **Freeze the intent** in a file, *before any mark*, and don't edit it to match
   what you drew (that "goalpost drift" is the deepest self-deception trap). Write
   it as **checkable predicates at the gestalt level** — presence, rough position
   (thirds, not pixels), relative size, orientation, connection — plus the
   **target feeling** and the **composition** (focal point, horizon, what dominates).
   Pitch predicates where your eye can actually judge them: too vague and a critique
   can't bite; too precise ("apex at x=0.41±0.005") and you can't perceive whether
   it's met.
2. **Compose first (Stage 0).** Before any object exists, decide the arrangement of
   the whole: focal point on a third (not dead-centre — that's static), the horizon,
   the eye-path, scale contrast, what's negative space. Composition carries mood by
   *placement alone* — a tiny house under a vast dark mass reads as foreboding with
   no detail at all.
3. **Block in coarse, then refine.** Big forms first, detail last. Low spatial
   frequency before high. This matches the cheap feedback channel's resolution and
   lets you revert a bad massing before you've invested detail on top of it.
4. **Measure on the cheap channel (text, near-free).** Encode positional/structural
   intent as exact checks and read them as text — *do not* ask your eye to estimate
   "is it high enough / big enough / balanced." That's what measurement is for.
5. **Look (the expensive channel), sparingly.** A real image look costs vision
   tokens; spend it on what no measurement can give — the **gestalt** ("does it read
   as the thing?") and the **affect** ("does it feel right?").
6. **Critique cold — from the image, never the intent** (see below).
7. **Apply selective corrections, then iterate.** Judge **convergence across
   passes**, not arrival in one. A first pass is *supposed* to look like a block-in;
   mood and finish are late-pass phenomena. Keep going until the gap closes or a
   named floor stops you.

## The cold-observe critic

This is the heart of it, and it has two parts. **Run both from the rendered image
as if you'd never seen the intent** — reconstruct what the marks *actually show*. If
you critique from what you *meant*, you'll feel your intention, not your drawing.

- **Recognition:** "What does this read as?" Not "is it the cat I intended" but,
  cold, *what would a viewer call it*. The gap is between that and the intent. A
  failure usually evokes the *wrong* thing (a cube evokes "toy block"), not nothing.
- **Affect:** feeling is a *derivation*, not a mystery. Observe → reconstruct the
  situation the image depicts → imagine being in it → name your reaction → compare
  to the target feeling. If a "dusk" scene makes the inhabitant feel "bright midday,"
  it failed the affect target even at 100% structural checks. The fix is usually a
  *situational cue* (lower the sun, lengthen the shadow, darken the mass, light a
  window), not a vague "make it better."

You are an adequate stand-in for the human viewer **because your recognition is
human-shaped** — when you read the image cold and think "tree" / "old woman" /
"dusk," that's a strong proxy for the typical human read. That is what lets the loop
close without a human in the room — most reliably for recognition, well for affect,
weakest for "is it beautiful." For high-stakes affect, get a real human look or a
reference image to diff against.

## Disciplines (why each matters)

- **Every mark must earn its place** — it establishes the situation, carries a
  feeling-cue, or is forced by the world (a cast shadow, a road implying the house is
  reached). If it does none, omit it. This bounds cost *and* serves the next point.
- **Clarity has an optimum, not a maximum.** The viewer *completes* what you
  withhold, and under ambiguity the imagination fills with charge (especially fear).
  Over-rendering removes that job and the image goes flat/clinical. Render to
  *establish*; withhold at the charged points (fog, shadow, the dark you can't see
  into). Concealment is an engine, not a defect.
- **Detail = the coherent world, not subdivided objects.** Realness lives in the
  *relationships* — the shadow proving sun and house share a world, the road proving
  the house is reached — not inside any one object. Zooming in to add a doorknob
  loses the world. Much of this is *derivable* (a shadow's direction from the light)
  and so can be computed/checked, not guessed.
- **Minimal ≠ symbolic.** A trunk, a fork, a canopy edge → every viewer thinks
  "tree." But pick **characteristic, irregular** fragments (the diagnostic cues),
  not the **generic regular icon** — a perfect cube reads as "the *idea* of a box,"
  because regularity is the signature of *abstraction*. Same economy, opposite read.
- **Single pass ≠ arrival.** Judge whether it's *converging*. Reverting to a "did
  one pass nail it?" verdict is the one-shot mindset this method exists to replace.

## The failure taxonomy (name which kind before "fixing")

Per gap, decide which it is — the fixes are different:

1. **Drew-it-wrong** — fixable in the loop (wrong numbers/placement).
2. **Tool-can't-express-it** — a vocabulary gap (e.g. no tone, no colour). Don't
   "correct" forever at something the medium physically can't do; record it and
   either withhold or grow the tool.
3. **Checks-don't-cover-the-intent** — the measurements pass but the image misses,
   because you encoded only what was measurable and dropped the point (mood, light).
   A green board is false confidence. Keep the affect judgment *outside* the checks.
4. **Developmentally-correct baseline** — not a defect, just where you are on the
   arc (a competent first pass / a symbolic stage). Don't pathologise it.
5. **Uncanny — realistic-but-*wrong*.** Specific to the realism arc: once some channels
   go realistic (modelled form, catchlit eyes), leftover **regularity** in the others
   reads as unsettling rather than unfinished — mirror-symmetric features, a dead-centre
   gaze, one textured region beside flat ones. Adding *more* realism **deepens** it. The
   escape is to pick a side: retreat to **confident stylization** (break the symmetry,
   off-axis the gaze, simplify or exaggerate so it reads as deliberate art) or push
   **through** to higher realism — which usually means **growing the tool first** (e.g.
   colored strokes for hair before you can texture a beard). Don't hover in the middle
   adding half-measures. 3D hits this hardest — it's the classic uncanny-valley medium.

## Earned rules (general; medium-specific lists are in the references)

- Symbols carry **default affect** — the cheerful ray-sun imports "daytime." Pick
  cues whose built-in mood matches the intent.
- **Colour is the biggest single lever for mood** (warm light vs cool dark = dusk);
  a grey face reads dead, warm skin reads alive.
- **Concealment/darkness** carries dread precisely because it withholds — render it
  *dark and soft-edged*, not as a crisp outline.
- A **composition/notan critic that tracks dark mass misses the bright focal
  point** — in a light-on-dark scene the focal is the *bright* region; judge that by
  eye.
- **The late detail pass adds *recognition* cues, not subdivision.** Once the massing
  converges, one pass of small **diagnostic** marks pays off — a catchlight (eye →
  alive), lattice on the sails (crossed sticks → windmill), the band that completes a
  plaid. The test per mark: does it make the thing read as *more itself* (keep), or
  just subdivide an interior a viewer won't credit (a doorknob — skip)?
- **Realism is a consistent texture-*level*, not maximum detail anywhere.** Texturing
  one region doesn't resolve a smooth/flat mismatch — it **relocates** it to the next
  boundary (a textured beard then fights smooth skin, then a flat hat). Raise the whole
  image's texture level together, or keep it uniformly low; one hyper-detailed island
  looks wronger than none. (3D: the same for material and poly-density across a scene.)

## Pick your medium

- **2D drawing / illustration** → read [`references/2d.md`](references/2d.md): the
  shipped tool (`sketch/draw.py`), its scene grammar, the cheap channels, the worked
  example, and the full 2D earned-rules list.
- **3D scene / asset / environment** → read [`references/3d.md`](references/3d.md):
  how the method transfers (and fits *better*), what's new (a scene-spec + renderer
  analog, camera / lights / materials, multi-view checking), the sweet spot, and the
  honest edge.
