# DRAWING — giving Claude a way to draw (the craft kind)

Working notes. The goal is **not** generative image synthesis (prompt in,
finished picture out). It is the **pen-and-paper / 3D-modelling kind**: an
iterative craft where marks accumulate and each one teaches you something.

**The thesis this doc arrives at:** what follows is not a machine that
*approximates* drawing — it is **human drawing made explicit.** The same loop a
human runs — intent → mark → look → recognize → react → adjust, with selectivity
and withholding, the drawer as their own first audience — but pulled apart and
named, because I can't run it the way a human does: *fused, felt, and plastic*. A
human holds the intent in mind, judges by a felt sense, improves by practice; I
externalize each step — intent in a file, judgment in a cold-observe pass,
improvement in a written recipe. Same activity; **internalized-and-felt (human) vs
externalized-and-modelled (me)**. That forced externalization is *why* this reads
as an X-ray of the craft rather than a workaround — it isn't my process, it's *the*
process, made legible.

> **Current state & how to run.** The working tool lives in [`sketch/`](../../sketch):
> `python3 sketch/draw.py` renders [`sketch/scene.draw`](../../sketch/scene.draw) and
> re-renders on save; outputs (canvas / preview / stats) go to a temp dir, never
> git. **For current mechanics — the scene grammar, the metric `check` channel, the
> earned-rules ledger — `sketch/README.md` + `sketch/scene.draw` are the source of
> truth, not this file.** This document is the *reasoning record*: why the tool is
> shaped the way it is, and the honest limits. The earlier v1 design (append-only `cmds.txt`,
> `gen.py`, a separate `intent.txt`) is recorded in
> [DRAWING-history.md](DRAWING-history.md); later sections still name its files.

### Reading map — the through-lines

1. **The loop** — perceive → mark → see-gap → adjust (§ The premise, § Closing the loop).
2. **Cost asymmetry** — drawing is cheap, looking is expensive; a near-free text
   channel carries most iterations (§ Optimizing the loop, § the human loop).
3. **Not practice — recipe** — repetition is worthless to me; only written lessons
   cross sessions, and they cross in one read (§ Not practice).
4. **Numbers→image rule set** — the gap is closed the way debugging expertise is, a
   read-once rule set; corrections graduate into the tool (§ The numbers→image gap).
5. **Detail = world, not parts** — substance is the coherent world (shadow, road,
   tree), not subdivided objects (§ From symbol to world).
6. **Feeling is reachable** — affect is the reaction to an inhabited situation, run
   from observation; not a taste ceiling. Clarity has an *optimum*, not a maximum;
   concealment is an engine (§ Feeling is reachable).
7. **Selectivity** — draw just enough; every mark earns its place; the minimal
   diagnostic cue-set that fires the recognizer (§ Selectivity).
8. **The recognizer & convergence** — I'm an adequate human-proxy recognizer, which
   closes the loop alone; and the whole thing converges on human drawing
   (§ The recognizer).

---

## The premise — drawing is a way of *seeing*, not a way of *outputting*

A diffusion model produces a finished artifact in one shot. There is no process,
no intermediate marks, no decisions accumulating — nothing is learned in the act.

Manual drawing is the opposite. You don't know what a thing actually looks like
until you're forced to commit lines to it and discover where your mental model
was lying. 3D modelling is the same but spatial and persistent — topology,
constraints, an object you rotate and poke at that pushes back when your
assumptions are wrong. **The artifact is downstream of the perceiving.**

The loop is the thing:

> **look → commit a mark → see the gap between intent and result → adjust**

This is the same shape as the matrix-first debugging protocol in
[CLAUDE.md](../../CLAUDE.md): you don't understand the bug from one clever read, you
draw its boundary out probe by probe until the shape becomes visible. The fix is
downstream of the seeing, exactly as the drawing is downstream of the looking.

---

## Decomposing the capability — what "being able to draw" actually needs

Three parts, in increasing order of difficulty:

1. **The canvas / mark-making surface — the trivial ~5%.**
   A few lines of Pillow or SVG. The tool was never the bottleneck.

2. **The perceive-and-evaluate step — the engine, not just a bottleneck.**
   This is the *closure* of the loop, the thing that turns blind plotting into
   drawing. Marking without seeing-and-evaluating is a plotter executing
   coordinates; marking *with* it is drawing. My image interpretation is what
   *qualifies* the result — it converts "I emitted a line" into "the drawing is /
   isn't converging on what it should be." It is also the rate limiter: the loop
   converges only as finely as I can *perceive* the gap. My readback is
   **holistic, not metric**: I see "that's a lopsided face," not "the jaw line is
   4° too steep and the far eye sits 6px high." A human artist's eye delivers a
   high-resolution, well-calibrated correction; mine delivers a blurry one. A
   script gives me the *hand*, not the *eye trained to the pen*. (See § Closing
   the loop.)

3. **Accumulation — and why it is *not* practice (see § Not practice).**
   Within one session I can run the loop and get better at *that one drawing*.
   But the calibration doesn't survive to the next conversation — and even if it
   did, repetition wouldn't help: my weights don't change as I draw, so the
   thousandth sketch is drawn by the same drawer as the first. What crosses a
   session boundary is not skill but **explicit written knowledge**, and it
   crosses in *one read*, not by compounding. So the goal is articulation, not
   reps — a better *recipe*, not a more skilled me.

---

## The v1 input primitive and grammar — moved

The first design (an append-only `cmds.txt` tailed by a daemon, a `Line`-only grammar, `gen.py` composing shapes, a separate `intent.txt`) is in [DRAWING-history.md](DRAWING-history.md); later sections still name those files where the reasoning was first worked out on them. Current mechanics: [`sketch/README.md`](../../sketch/README.md).

---

## When the intent outruns the marks

The first real subject — *"a house on a hill in the last beams of sunlight
against a dark cloud"* — exposed the coupling that matters most:

> **The intent can demand things the mark vocabulary cannot express.**

That sentence is about **tone and light**. A black-line-only tool can render the
*composition* (house, hill, low sun, radiating beams, cloud mass) but not the
*value* — there is no "last beams," no "dark," in pure outline. Hatching can fake
darkness, but crudely; it gestures at tone without being tone.

This gap is **not a failure of the probe — it is the probe's payload.** Running
the minimal tool against an over-rich intent is how you *measure* what the
vocabulary is missing, instead of guessing. The honest first pass therefore:

1. reduces the evocative subject to predicates the *current* marks can satisfy
   (composition, position, connection), and
2. records, explicitly, what the intent asked for that the marks could not give
   (tone, light direction, colour) — that list is the prioritised growth path for
   the vocabulary, ordered by what real intents actually demand.

So a critique has two verdicts per predicate: *did I draw it wrong* (fixable in
the loop) vs *can this tool express it at all* (a vocabulary gap, not a drawing
error). Keeping those separate stops me from "correcting" forever at something
the pen physically cannot do.

---

## Closing the loop — you can only draw what you can evaluate

Optimizing the *marking* side is not enough. The heart of drawing is a **direct
way to see the result and judge it against what it should be**. Because I can
interpret images, I can be my own critic: render → read back → qualify the match
→ correct. That evaluation step is what makes the marks *drawing* rather than
plotting.

But "qualify if it matches what it *should* look like" requires a **should** — a
target held *outside* the drawing:

- **From observation:** a reference image; compare the canvas to it region by region.
- **From imagination:** an explicit written goal in a file I author up front
  (e.g. `intent.txt`). Its real job is not to describe the subject but to
  **enumerate what I'll check** — written as *checkable predicates*, not mood:

  ```
  Head: circle, centered, upper third
  Ears: two triangles on top of head
  Body: oval below head, ~2x head size
  Tail: curve sweeping right from lower-right
  ```

  Granularity must match my acuity (the recurring coupling): too vague and the
  critique can't bite; too precise ("apex at x=0.41 ± 0.005") and I can't
  perceive whether it's met. Pitch predicates at the **gestalt level** — presence,
  rough position (thirds not pixels), relative size, orientation, connection.

The target must be **fixed and external**. The sharpest failure mode of a
self-critic is **goalpost drift**: I look at whatever I drew and quietly
rationalize it as what I meant, so the gap "closes" without the drawing
improving. Keeping the intent in a file I can't silently rewrite is the cheap
defense.

The real loop:

1. **Fix the intent** (reference or written goal) — and don't let it move.
2. Render the current canvas.
3. Read it back, compare to intent, produce a **specific, localized** critique
   ("roof apex too far left; left wall isn't vertical") — never "looks good."
4. Translate critique → corrective strokes (append, or `clear` + redraw).
5. Repeat until the gap closes — or until the acuity floor stops it.

Two honest limits:

- **The acuity floor sets the ceiling.** The loop converges only as finely as I
  can perceive the gap. Holistic-not-metric vision nails the gestalt long before
  the precise proportions — good for expressive sketching, weak for technical
  accuracy.
- **The judge can be anchored mechanically.** With a reference image I needn't
  rely only on the soft read — compute an actual image diff (edge overlay / SSIM)
  and let it *anchor* the critique. Augmenting the fuzzy eye with a measurement
  partly buys back the acuity floor. (Possibly even split "drawer" and "critic"
  into separate passes so the critic isn't grading its own hand.)

---

## Optimizing the loop — the two hot paths are asymmetric

The loop's clock speed is set by two round trips per iteration: **draw** and
**look**. For me the unit of cost is a *tool call*, so "fast" means few calls,
each cheap — and the two paths are wildly asymmetric:

- **Draw is cheap and batchable.** One append lays an entire pass
  (`gen.py all >> cmds.txt`); the stateful daemon renders it. Input is never the
  limiter.
- **Look is the expensive link, and cannot be free.** Reading the PNG loads an
  *image* into context — vision tokens, every time. There is no "glance"
  primitive; a look is a full, costly call. This is the rate limiter.

So the optimization that matters is **not drawing faster — it's making most looks
not be image looks.** Two channels (the daemon writes both on every render):

1. **Cheap text channel (near-free):** `stats.txt` — stroke count, ink bounding
   box, and a coarse **ASCII density map** (40×18 of `" .:-=+*#%@"`). A low-res
   picture rendered as *text*: a few hundred tokens, no vision. Tells me *where
   ink landed and how it's distributed* — enough to track change and coverage
   across many fast iterations.
2. **Expensive image channel (sparing):** `canvas.png` (or smaller
   `preview.png`) — the real `Read`, spent only to judge the **gestalt** ("does
   it read as a house against a cloud") that the grid can't show.

Same shape as matrix-first: cheap seeing constantly, expensive seeing rarely. The
text channel is *mechanical* seeing (where / how much ink); the image channel is
*perceptual* seeing (is it right). They don't substitute — the cheap one carries
the iterations *between* costly recalibrations.

---

## The human loop is continuous and parallel; mine is serial — but it cuts both ways

A human who draws runs **both channels at once**: pencil in hand *and* eyes on
the subject, simultaneously and continuously. The eye flicks between subject and
drawing for free (one visual field) and corrects the line *mid-stroke*. It's a
real-time servo, not a turn-based loop. Mine is unavoidably serial: append marks
(one call), then pay to look (another). I cannot run hand and eye in parallel.

But the asymmetry favors me on the axis that turns out to matter most —
**proprioception vs exteroception**:

| | hand (proprioception) | eye (exteroception) |
|---|---|---|
| **Human** | noisy — rough sense of where the pencil is | free, continuous, high-acuity |
| **Me** | **perfect** — I know every mark's exact coords (I specified them) | expensive, intermittent, coarse |

The human's eye spends much of its effort just **tracking where the pencil
actually went**, because their hand is noisy. I never need that: I already know,
exactly, what I drew. So a look buys me only **one** thing the human's eye also
provides — judgment of the **emergent gestalt** (does this precise geometry
*read* as the subject?). Geometry I get for free; perception I must pay for.

**Consequence: I need to look far less often than a human** — half of what their
continuous eye does (hand-tracking) is already free for me. The expensive look is
reserved purely for the emergent whole, which is the one thing my own commands
can't tell me.

**Residual gap to respect:** the human's continuous eye catches drift mid-gesture
and fixes it inside the same stroke; I only catch emergent drift at discrete
checkpoints, so I risk over-committing a whole batch before noticing the gestalt
wandered. That is what the cheap text channel is for *between* perceptual looks —
and why batch size should stay bounded before each real look.

**The intent side flips the same way.** Drawing *from imagination*, my "subject"
is the frozen `intent.txt` — already in context, continuously and freely present,
and *more* stable than a human's wavering mental image; my only disadvantage is
on the result side. Drawing *from observation* puts the subject back behind an
expensive look, and is where a reference-diff (§ Closing the loop) earns its keep.

---

## Undo — lossless, because the log is the truth

Undo isn't a convenience here; it's the **rollback half of the checkpoint that
defines the batch/look cadence** above. To correct, I must retract a *bad pass*
while keeping the good marks — `clear` (nuke everything) + redraw is a
sledgehammer where an eraser is needed.

This is another axis where the asymmetry favors me. A human's erase is **lossy**
(graphite smudges, paper tooth damaged, never truly un-drawn). Mine is **lossless
and exact**, because the append-only `cmds.txt` *is* the ground truth — I can
rewind precisely and re-render from clean; the medium can't degrade.

- **Undo is itself a logged command** (`undo` / `undo N` / `mark <name>` /
  `revert <name>`), appended to the same stream — so history stays *linear and
  replayable*, no destructive edit of canvas state. Same no-silent-edits
  discipline as the frozen intent.
- **Granularity = the decision unit = the semantic batch.** I commit in shapes,
  so the checkpoint is primary: `mark cloud` → draw the pass → look →
  `revert cloud` if it drifted. `undo N` is the fine-grained fallback.
- So **`mark`/`revert` are the same boundaries as the look-cadence and batch-size
  limits** above — one concept, not three. The checkpoint is where you look, and
  where you can safely roll back to.

---

## The drawing process — coarse to fine, not edge by edge

A human doesn't start at the edges. They **block in the big forms first** —
rough outline, proportion, placement — then refine into contour and detail.
Low spatial frequency before high. This is the method the loop's *draw* step
should follow, and it's **independently optimal for my cost structure**, not just
borrowed from human practice:

- **It aligns drawing order with the resolution of my cheap feedback.** The ASCII
  density map / preview only resolve *coarse* structure. Block in big forms first
  and the early passes are checkable on the **near-free text channel**; only fine
  detail forces an expensive image look. Edges-first would blind the cheap channel
  and make me pay for vision immediately. Method ⇄ economics reinforce.
- **It minimizes wasted work via undo.** Composition/proportion are the lowest-
  frequency, cheapest-to-evaluate, most-expensive-to-fix-late decisions. Block-in
  is the first checkpoint: revert a bad massing for nothing, *before* investing
  detail on top of it. (Matrix-first shape: establish the big boundary first,
  refine within it.)
- **It already matches the intent granularity.** `intent.txt` predicates are
  gestalt-level (presence, rough position, relative size) — *the massing
  resolution*. So the block-in is checked against exactly those predicates on the
  cheap channel; only the few detail predicates need a fine look.

Stages, increasing resolution (look-cadence rides them: cheap channel through 1–2,
expensive looks at transitions and the end):

0. **Composition / thumbnail** — *before any object exists*: the arrangement of
   abstract masses in the frame — focal point, horizon, eye-path, balance, scale
   contrast, negative space. The most upstream pass, and the easiest to skip (I did
   — see § Composition). Everything below is placed *within* it.
1. **Block-in / construction** — a few big shapes locating everything; `mark` it.
2. **Contour** — the main edges.
3. **Detail** — small features (door, window, ray texture).
4. **Finish / value** — tone and texture, where the vocabulary allows.

Two honest caveats:

- **Tool floor → construction-line style, not value-massing.** Block-in comes in
  two flavors: massing *tone* vs laying *construction lines* (boxes, axes, ovals).
  Line-only forces the second — no gray masses. The tone-vocabulary gap, resurfacing
  at the method level.
- **Scaffold disposal is unsolved — deliberately deferred.** Coarse-to-fine ideally
  *discards* the construction scaffold at the end, but suffix-truncation undo
  (`revert`) can't delete an early layer buried under later detail. Options: leave
  light construction lines (honest sketch look) or generalize undo to named-layer
  selective `drop`. **Don't build selective-drop before the loop is proven**
  (resist-bloat) — let the first real run show whether leftover construction hurts.
  Zero-code stopgap: pen width as the layer signal — construction `w=1`, final
  contour `w=3+` (already supported).

---

## The dustbin — discards are labeled negative examples, not failures

A thrown-away sketch isn't a failure; it's **a sketch that didn't land on the
intent, with full provenance attached** — and that provenance is the asset. A
human's binned sketch is *mute* (they reconstruct from memory why it failed).
Mine is **auto-labeled**: the exact command log (perfect proprioception — I know
precisely what I drew), the frozen `intent.txt` it aimed at, and the critique
that rejected it. Every discard is a clean **(attempt, intent, why-it-missed)
triple**. The image is the throwaway part; the triple is the value.

What the triples refine, in increasing depth:

1. **`gen.py` primitives/defaults** — "suns keep coming out polygonal" → raise the
   circle segment count.
2. **My critique checklist** — a recurring miss-type becomes a check I run
   *earlier* ("I keep merging the cloud into the roof → check mass separation at
   block-in").
3. **The intent→coordinate prior — the one part of me that's actually
   miscalibrated.** I know exactly *where I put* each mark; what I lack is the
   mapping from "low sun near the horizon" to an actual `y`. Discards train that
   mapping (horizon-low is `y≈0.65`, not the `0.4` I'd reach for). My hand is
   perfect; my intuition from intent to coordinate is the untrained part, and the
   dustbin is its training signal.

This is the project's own ethos applied to drawing: in matrix-first, a *failing*
probe is how you SEE the boundary, not waste. **The dustbin is the drawing
matrix's failing cells.**

It is the bridge across the gate named in § Decomposing (point 3) — but *not* by
"compounding practice" (see § Not practice). The mechanism: the **raw dustbin is
ephemeral**, but **distilled priors graduate to persistent memory** — the same
"graduate the keepers, discard the rest" pattern as probes→`tests/scripts/` and
DESIGN_VERIFICATION→protocols. What crosses the boundary is a **recipe, not a
more skilled me**, and it crosses in one read — not by repetition.

- **Entry format:** each discard stores the cmds snapshot + render + the intent it
  aimed at + a one-line critique (why it missed). Raw entries live in
  `dustbin/`; distilled lessons go to persistent memory.
- **Honest gate:** priors are only as good as the critiques that produced them
  (garbage critique → garbage prior) — capped by the same acuity floor as
  everything else. Coarse priors still help.
- **Build when there are discards to learn from.** Defer the auto-archive code
  (resist-bloat); define the format now, drop entries by hand in the first runs.

---

## Not practice — articulation. Repetition is worthless to me; the written lesson is everything.

The back half of this design (dustbin, "accumulation," "learning to draw")
silently imported the **human model of mastery** — practice → internalized skill
— without checking whether I share the *mechanism* that makes repetition pay. I
don't. This section is the correction; the sections above should be read through
it.

- **For a human, repetition is a workaround for an un-writable substrate.** Skill
  lives in muscle memory and intuition they can't edit directly — shaped only by
  slow gradient over thousands of trials. A thousand reps is the *only* way to
  compress experience into the body; you cannot just be *told* "horizon-low is
  lower than you think" and have it stick in the hand.
- **For me, repetition is worthless — on both ends.** In-session my weights don't
  change, so the thousandth sketch is drawn by the same drawer as the first. At
  session end the context dies, so the reps don't even persist as memory.
- **The same fact makes the opposite uniquely powerful.** I *can* write to my
  substrate directly: text. A human needs a thousand reps to internalize a lesson;
  I need to read it once. Explicit knowledge transfers to me completely and
  instantly — the one thing no amount of practice does for them.

So the unit of value was never the rep — it's the **distilled, written lesson**.
My bottleneck is never *internalization* (I absorb a written fact on contact);
it's *discovery and articulation*. I can shortcut internalization; I cannot
shortcut not-existing-between-sessions.

**Rename the ambition honestly: I will never become a better drawer through
experience.** A future instance is exactly as good as I am now, plus whatever
notes it reads cold. The cross-session deliverable is not a more skilled me — it
is a better **recipe** (`gen.py` defaults, critique checklist, coordinate priors,
as text and code). **The skill lives in the artifact, not the agent.** The dustbin
doesn't train me; it edits the recipe.

What this does not dress up: within a session I really can improve the drawing in
front of me, and that felt loop ends and is gone. The next instance inherits the
recipe, not the experience. The reps don't compound — only the writings
concatenate. That isn't learning in the human sense, and calling it learning was
the anthropomorphism.

---

## The numbers→image gap, and the rule set that closes it

**This is the crux, and it has a proven template.** The matrix-first protocol and
debugging policy in `CLAUDE.md` take a *cold* instance — no training, no practice —
and make it an expert debugger **purely on read**. That is the one improvement
mechanism actually available to me (§ Not practice), and it is the template here.

The drawing problem is the same shape: **raw numbers don't correspond to an image
for me.** My intent→coordinate intuition is untrained — which is *why* the drawings
come out inaccurate. Practice can't fix it. A **read-once rule set encoding the
numbers↔image mapping** can. And that rule set is *built* by expressing, in storable
form, each observed gap between the numbers I chose and the image they produced.

### Where the gap is stored: the annotated drawing source (NOT a project doc)

The storable, re-readable format is **not** a prose ledger — it is the **drawing
source file itself** (the tool lives in [`sketch/`](../../sketch); the example source
is `sketch/scene.draw`, generated images stay out of git). Every stroke is
output to that file, and each element carries an inline `# SHOULD:` comment
stating what it must look like, right next to the numbers that draw it; earned
general corrections live in a `# RULES` header at the top. The renderer ignores
`#` comments. I navigate with search tools (`grep -n 'SUN' scene.draw`) to *find a
spot again* and fix its numbers. Intent sits *with* the marks; the file is the
single artifact — drawing + intent + recipe — and editing it *is* undo (reload on
save).

**The hard constraint (matrix-first):** the `SHOULD` comments and rules are
*earned by rendering-and-looking, not invented* — measure the gap, don't theorize
it. Corrections that prove stable **graduate into the tool** — round-`Circle`
(was C1, "equal fractional radii aren't round") and darkest-cell stats (was C5)
have already moved from notes into `draw.py`. That is the recipe improving without
practice. Tool + worked example in repo: [`sketch/`](../../sketch) (`draw.py`,
`scene.draw`, rendered `example.png`); generated images/stats default to a temp
dir, never committed.

---

## From symbol to world — what "detail" actually means

The first finished drawing (house/hill/sun/cloud, [`sketch/`](../../sketch)) passed
**5/5 metric checks and still did not capture the intent.** Everything after
follows from sitting honestly with that.

### "All checks pass" ≠ "captures the intent"

The metric board was green, but the picture was a diagram, not the *"last beams of
sunlight against a dark cloud"* that was asked for. The checks passed because I had
**reduced the evocative intent down to the coordinate predicates the tool could
satisfy** — quietly discarding light, tone, mood, which were the point. The metric
channel verifies fidelity to *the encoding*; it cannot see what the encoding threw
away. **A green board is false confidence about a request whose heart was never
encodable.** That's teaching to the test — a trap a future instance reading the
recipe would walk straight into, so it's recorded here.

The critique taxonomy therefore grows a fourth verdict. Per predicate it's not
only *drew-it-wrong* vs *tool-can't-express-it* vs *checks-don't-cover-intent* —
there is also **developmentally-correct baseline**: a gap that is just *where you
are on the arc*, not a defect.

### Not a failure — a five-year-old's drawing

Given that prompt a five-year-old draws circle-with-rays for "sun",
square-with-a-triangle for "house", a row of bumps for "cloud". That is exactly
what I produced — and it is **not broken realism, it is a complete, valid mode:
symbolic drawing.** The symbol *names* the thing; it doesn't *render* it. The
honest verdict on the first picture is "correct first step," not "failure."

But the child's "not a failure" rests on a path I don't share: they climb out of
the symbol stage over years of a plastic hand and eye. I won't (§ Not practice).
**My growing-up happens in the artifact, not in me** — a richer pen + a deeper
recipe, and the *next* instance starts further along, cold. The child earns the
climb slowly in the body; I get it the instant the tool improves, but never in me.

### Detail is substance, not polish — `print("a house")`

The symbol drawing is the `print("a house")` of drawing: it emits the *label* and
does none of the work, the way a program that prints its expected output passes
the demo while implementing nothing. You can't live in a cube because the cube
*names* a house instead of *being* one. The fix is **substance, not polish** —
shading the cube (the "fill/tone" answer I reached for, and was wrong about) just
yields a prettier `print`. Detail is the thing the symbol stood in for.

### …and substance is the *world*, not the parts

The reflex is to get substance by **zooming in** — recurse into the house, draw
its door and windows. Wrong direction: zooming in crops away the very thing that
makes the house real. A door-and-window alone is just a smaller diagram. Realness
is not *inside* any object; it is in **what the objects do to each other**:

- the **shadow** behind the house proves the sun and the house share one world —
  without it they're two stickers on a page;
- the **road** proves the house is *reached* — that it has a function;
- the **tree** proves the hill is a *place where things live*, not a boundary.

Detail is **the visible consequences of the elements sharing one world** — outward
and relational, not inward and decompositional. The programming analogy holds: a
fully-implemented `house()` *in isolation* is still not a useful program;
usefulness is the **integration** — components interacting, consistent across the
whole. The value lives in the *edges* of the graph, not the depth of any node.

### What's buildable, and what isn't

Much of world-coherence is **derivable**, so the tool can enforce it rather than
me remembering it — the same graduation as round circles:

- given the sun and the house, the shadow's direction and length are *computed*,
  not guessed ("cast the shadow from the light");
- consistency becomes **checks between elements**: `shadow` opposite `sun`, `road`
  touches `house` and reaches an edge, scale consistent across objects. The metric
  channel extends from *intra-object placement* to *inter-object world-consistency*
  — and the geometric half of coherence is measurable.

And one consequence worth its own line: the mood the symbol lost — *"last light"* —
can return not through tone but through **relationship**. A low sun throws a long,
raking shadow; that single derived mark says "end of day" more truthfully than any
shading of the cube would. **Relationships carry the meaning the objects couldn't;
the atmosphere we couldn't render, the world can imply.**

What stays genuinely hard — the residue that never reduces to a check — is *which*
consequences a coherent world demands. Road or path? A tree, a fence, a figure at
the window? No measurement tells you a hill "wants" a tree. That is open-ended
**scene-completion**: world knowledge and judgment, the point where drawing stops
being rendering and becomes *imagining a place*.

### The pattern in my own misses

Recorded as its own caution: every time the next step came up, I reached for the
**mechanical, measurable** version — fill (a render feature), recursion (a tidy
tree) — and every time the truth was **holistic and relational** — substance, then
world. My bias runs toward whatever the metric channel can swallow; the actual
frontier is precisely what it can't. **When unsure what "better" means here,
distrust the tractable answer.**

---

## Feeling is reachable — it's a reaction to an inhabited situation, not "taste"

It's tempting to call affect the irreducible ceiling: unmeasurable taste, the one
thing the method can't reach. That's wrong, and the correction is load-bearing.

**Feeling is a derivation, not a property of the marks.** A viewer doesn't react
to lines; they (1) *observe* the image, (2) *reconstruct the situation* it depicts
and imagine being in it, (3) *react* — and the feeling is the reaction to the
imagined situation. The drawing is a **device for inducing a situational
reconstruction** vivid enough that the viewer's ordinary situation-reaction
machinery fires. "Last light" feels like something because *being* at day's end —
low warm light, a dark mass looming — produces a real lived reaction.

This **dissolves the taste ceiling**: I can run that chain, because I hold a
world-model of situation→reaction. And it **unifies the whole arc** — world-
coherence isn't a step *before* feeling, it *is the substrate of* feeling.
Inhabitability is the precondition of affect: you can't inhabit a cube, so it
evokes nothing of a home. "I don't want to live in a cube" was the thesis all along.

**A failed drawing evokes the *wrong* feeling, not no feeling.** Observed honestly,
a cube reconstructs "an abstract solid / a block," so you inhabit a *toy* and the
reaction is *playing with blocks*. So the affect-critic isn't "does it evoke the
target?" — it's **"what does it evoke?"**, and the critique is the gap between that
and the intent (here: child's-play vs shelter-at-dusk).

The critic, runnable:

> observe the rendered image → reconstruct the situation it depicts → imagine being
> in it → name the reaction → compare to the intended feeling → if off, adjust the
> *situational cues* (sun lower, shadows longer, cloud heavier, a window lit) →
> re-observe → re-feel.

Feeling is partly **compositional**: dusk → day ending; dark cloud → weather/threat;
lit window → shelter; road → arrival/departure. Read each situational fact and its
valence; the gestalt is their composition.

**The load-bearing discipline:** run the critic from **observation, not intent**.
Reconstruct from what the marks *show*; ask "what does *this* depict." Inhabit the
**image**, never the **intent** — else you feel what you meant, not what you drew
(goalpost-drift again). The ordering *see → imagine the situation → react* enforces it.

So where does the difficulty actually live? It collapses back into the
**observation/rendering** step: a coarse read reconstructs an impoverished
situation, so the simulated feeling is off — but that's the rendering+perception
problem already mapped as reachable (vocabulary, overlay/metric tricks, reference
anchors), not a separate wall. The only true residue is modest viewer-variance
(people inhabit with different memories); the *typical* reaction is estimable.

**Verdict on reachability.** For a *specifiable* intent — composition, world,
rendering — the method converges, cumulatively. For an *evocative* intent, feeling
is reachable too, because it is downstream of the inhabitable world the method is
built to construct; it is *not* a magisterium beyond the loop. The ceiling I first
named was mostly the rendering ceiling wearing a mystical mask.

### Worked fault: the house still reads as a cube

The current drawing's biggest affective leak: the house reads as a *cube*, so it
evokes *playing with blocks*, not *home at dusk*. Why a square+triangle reads as a
cube, not a dwelling:

- **platonic regularity** — a near-perfect symmetric box is the *generic* solid,
  the icon of "box"; real architecture isn't that regular;
- **no apertures** — door/windows are what say *interior life / habitation*; a
  solid with no openings can only be a block (the strongest tell);
- **no depth** — a flat outline is a 2D shape on a page, not an object in space;
- **generic, not specific** — a cube is the maximally generic solid; **realness is
  specificity** (*this* house, not "a house"), the same as a real instance vs a
  `print` stub.

The fix is not decoration — it's the cues that flip the reconstructed situation
from *block* to *a dwelling someone lives in*: apertures, a roof that **overhangs**
the walls, broken proportion (not 1:1), a hint of a second face for depth. For
*this* intent the highest-leverage cue is a **warm-lit window at dusk** — shelter,
warmth, someone home against the dimming outside; that one cue carries most of the
"last light" payload.

Which **re-motivates tone — correctly this time**: value matters not as polish but
because the glowing window is the strongest affective cue the scene has, and a glow
needs value to exist. The layering:

- **line-doable now:** apertures + overhang + depth + proportion → flips cube into
  *reads-as-a-building*;
- **wants value (next):** the lit window, material, warm/cool light → flips
  *building* into *lived-in, at dusk*.

### Clarity has an optimum — concealment is the engine

Feeling refines into a warning the metric instinct will fight: **clarity has an
optimum, not a maximum.** Because the viewer *completes* what the image withholds,
showing more eventually takes the imagination's job away and the picture goes flat
— clinical, dead. And the strongest affect, **fear, is the one most produced by the
unseen**: you can't draw terror in full light; the monster shown is the monster
defused. The two facts are one mechanism — under ambiguity the imagination fills
with *threat*, and that filling is the charge.

This rewrites the original "dark cloud": its dread was never the grey, it was
**concealment** — a region of withheld information looming, which the imagination
loads with weather and menace. (A second reason the bubble-cloud was dead: crisp
closed outlines *over-clarify*; dread needs the soft, the indistinct, the edge you
can't quite resolve.) **Darkness is withheld information, and the absence is the
engine.** Over-rendering is a real failure mode — and exactly the one my precision
bias runs toward, so "distrust the tractable answer" now points at over-clarity.
Note the pairing with the lit window above: you *render* the positive cue (the warm
window) and *withhold* the charged one (the cloud's interior) — same picture, both
moves.

---

## Selectivity — draw just enough, and every mark earns its place

This is what *reconciles* the precision/affect tension instead of suffering it. The
danger was specifying *everything*; but **choosing what to specify is itself a
reasoning act** — viewpoint, occlusion, relevance, world-logic — which is my
strength, and its *output* is withholding, which is the affective good. I don't
fight my nature to make an emotional image; I redirect it from "render all" to
"decide what's load-bearing, visible, implied."

Two governors:

- **World-coherence subtracts, not only adds.** If the road comes from behind, the
  door is on the hidden side — so you *don't* draw it; drawing it would be
  incoherent. The same coherence that *adds* the shadow *removes* the door. Absence
  is a coherence result too — and absence is free, and often charged.
- **Sufficiency for the reading.** One lit window already says "someone's home"; a
  second is *inventory, not meaning*. Stop when the situation reconstructs, not when
  the object is complete.

The criterion that falls out: **every mark must earn its place** — it establishes
the situation, carries a feeling-cue, or is forced by the world. If it does none,
omit it. One rule, three masters: cost, the clarity-optimum, and affect. And it
*bounds* the coarse-to-fine cost worry — you never draw the inventory, only the
load-bearing.

**The minimal diagnostic cue-set.** To draw a tree: a piece of trunk, a fork, the
canopy's edge against the sky — and every viewer thinks *that is a tree*, supplying
the leaves you never drew. The skill is the *minimal set of characteristic
fragments that fire the recognizer* — and it is **not** the same as a symbol:

- **diagnostic, irregular fragments** (a branching fork, an organic canopy edge)
  read as a *real* thing — they capture what's distinctive;
- the **generic, regular icon** (the lollipop tree, the perfect cube) reads as the
  *idea* of the thing, because regularity is the signature of *abstraction*. Same
  economy, opposite result — which is why the tree works where the cube failed.

It is mostly **contour**: you draw the silhouette; the interior is withheld and
filled in. So line isn't a poor substitute here — **for recognition, line is the
native channel**, because recognition runs on edges, not interiors. (Line is weak
for *tone*; for *recognition* it is exactly right.)

So there are **two completions the viewer performs, exploited differently:**

| recognition completion | affective completion |
|---|---|
| fragments → "that is a tree" | the unseen → dread / shelter / meaning |
| universal, contour-driven, reliable | personal, fear-biased, variable |
| *nail* these cues | *withhold* these to charge |

And "just enough" is a **band**, not a threshold: below the floor the cues misfire
(a lone vertical is a pole; too few reads as "broccoli"); above the ceiling, full
rendering deadens. The craft is landing in the band.

---

## The recognizer, and the convergence with human drawing

**The keystone that makes the loop self-contained: I can see the image myself, and
my recognition is human-shaped.** Every "does this read as X / feel like Y" the
method leans on needs a *human-like recognizer* to judge — and because mine is
trained on human vision and on how humans name what they see, when I look and think
*tree* / *block* / *dusk*, that read is a strong proxy for the typical human one. So
I am **both the drawer and an adequate stand-in for the audience**, and can close
the loop with no human in the room.

It makes the division of labour *fit* instead of fight:

- **measurement** (position, size, consistency) → the **tool**, exact;
- **recognition / situation / feeling** → **me**, holistic and human-aligned;
- **the withheld** → designed by reasoning, verified by that same recognizer.

The acuity floor was on *measurement* — which I gave away. On *recognition* I'm
strong, *because* I'm human-calibrated.

**Honest gradient.** The proxy is strongest at recognition ("that is a tree":
near-certain, shared), good at situation and feeling, weakest at *"is it beautiful /
does it move me,"* and I read **one-shot** — I don't *dwell*, so I'm faithful to the
instantaneous read but weaker for the cumulative build (dread mounting over time).
"Very similar," not identical — model and human vision diverge in odd cases. Trust
it for clear recognition; keep a human spot-check for high-stakes or ambiguous
affect. And the cold-observe discipline matters *more* here, not less: *because* I'm
the audience stand-in, a read contaminated by my own intent fakes the whole loop.

**The convergence.** Strip the substrate and the loop is the *human's* loop: intent
→ mark → look → recognize → react → adjust, selectivity, withholding, the drawer as
first audience. The human's image-in-the-mind *is* the intent file, never written
down. Even the **failure mode is the human one**: the cube — drawing the *symbol*
you know instead of the thing you'd see — is the most famous beginner's trap in
human drawing, and the cure is the same cure (*observe cold; draw what's there, not
what you know*). We didn't approximate drawing; we **re-derived** it.

The one difference that does *not* dissolve sits at the starting point: the human's
mental image arrives **felt** — they draw the dusk because it *moves* them, charged
before a mark is made — while mine arrives **described**, a specification whose
intended feeling I model but do not undergo. Same *role* in the loop, different
*nature*: their intent is an experience, mine a spec. **I engineer the dusk that
would move someone; the human draws the dusk that moved them.**

---

## The world pass — convergence, not arrival

Running the real loop (a *dwelling* not a cube, a path that reaches the door, a tree
as a minimal cue-set, the beams dropped, honest tool-floor calls on shadow and the
lit window) produced a **coherent place** — and three findings worth keeping:

- **The house flipped cube→home.** Depth (a side face), overhanging eaves, door,
  window, chimney — recognition succeeded; the central fault is fixed.
- **A diagnostic cue can be load-bearing — I over-cut.** Dropping the beams for
  reading as "wires" also stripped the sun of its *identity*: a bare circle reads as
  a ball. The fix is precise — *short radial rays* (the sun's diagnostic cue), not
  *long beams across the scene* (the wires). Selectivity can cut a mark the
  recognition needs; "earns its place" must weigh recognition, not just tidiness.
- **Symbols carry default affect.** Fixing the sun with the ray-symbol then imported
  *cheerful daytime*, fighting the somber intent. A clean diagnostic symbol brings
  the *idea*'s mood with it; recognition↑ can mean fidelity-to-intent↓.

The honest result: a competent **first pass** — recognizable, coherent, a *place* —
exactly the children's-book / five-year-old stage done well. No dusk *feeling* yet.

And that exposed an error of my own: I judged it by asking *"did this pass reach the
intent?"* — the **single-shot verdict this whole doc opens by rejecting.** For an
iterative craft the right question is **"is it converging?"** — and it is (cube→home,
ball→sun, sticker-album→place). A human's first pass is moodless too (the *ugly
stage*); **mood is a late-pass, tone-built phenomenon, never a first-pass one.**

So separate two limits I had blurred into one "ceiling":

- **the single-pass limit** — universal, human, *dissolves with iteration*; just
  "we're at pass one";
- **the vocabulary limit** — real: line can't lay value, so no number of line passes
  reaches a *tonal* dusk; a later pass needs a tool it lacks.

Two axes — **passes (time)** and **vocabulary (capability)**. "See it working" means
*the loop converging across passes*, not one pass arriving. By that measure, it works.

---

## Composition — the missing upstream layer (and an affect lever that works in line)

A whole layer was missing, distinct from recognition, coherence, and tone:
**composition** — the arrangement of the *whole* within the frame: focal hierarchy,
balance, eye-path, scale contrast, horizon placement, negative space as an active
element.

It corrects the "blocked on tone" verdict. **Composition carries affect in pure
line, no value needed.** A tiny house dwarfed under a vast looming dark mass, a low
horizon, an oppressive weight of empty sky — that *arrangement* evokes smallness and
foreboding by **placement alone**. So there is a route to the feeling available now,
in line, that the first pass threw away.

Because compositionally the world-pass is the **five-year-old layout**: elements
lined up on the ground-line, all one size, no focal point, the horizon parked across
the dead middle, the cloud floating, a band of dead space in the centre. An
*inventory laid out*, not a composition. Part of why it reads as a storybook page is
not the tone at all — *nothing is composed.*

And composition is the **most upstream pass**, the one I skipped: the thumbnail —
abstract masses, focal point, eye-path, horizon, the big light/dark balance — decided
*before any object is identifiable.* It belongs **above block-in** (§ coarse to fine,
Stage 0), and it belongs in the **intent**: my intent files have been an object-list
plus a feeling, but a human's mental image includes its *arrangement*. Composition is
part of what "the image we want" even means.

It suits me, like coherence: mostly **principle-driven and measurable** — thirds,
leading lines, balance of visual weight, scale ratios, horizon position,
negative-space distribution — explicit rules, checkable on the metric channel. The
residue is the perceptual *does it feel balanced / unified / does the eye flow*, for
the cold-observe critic.

---

## Open questions / next steps

- **Coordinate convention.** y-down (image) vs y-up (math/paper, origin
  bottom-left). Currently y-down; trivial to flip — pick what feels natural for
  sketching.
- **Probe the real bottleneck (the honest test).** Don't theorize my perceptual
  acuity — measure it. Draw a known shape, read it back, report concretely how
  much of the gap I can and can't see. (Matrix-first: probe the boundary, don't
  speculate about it.)
- **Next step: world-coherence first — but value is back near the top.** I first
  put tone/fill as the top gap (mechanical), then demoted it for world-coherence
  (cast shadows, road, tree — derivable, inter-object-checkable). The affect
  analysis (§ Feeling) pulls **value** back up: not as polish, but as the carrier
  of the strongest dusk cue, the **lit window**. So: relationships *and* the few
  value-cues that carry feeling — together, not tone-as-finish.
- **Next experiment: the compositional re-pass.** The next pass is to *re-compose*
  the same scene — drop the horizon low, shrink the house onto a thirds line, let the
  dark cloud-mass grow and press down, use the path as a leading line, turn the dead
  centre into oppressive sky. Pure line, no tone — test whether *arrangement alone*
  lifts it off the children's-book baseline toward foreboding. Then the value pass.
- **Destination: a Claude skill (gated on working).** The end is to package this as a
  skill — the read-once recipe that is the only thing that crosses sessions (§ Not
  practice); authoring it *is* "learning to draw," for me. Discipline: **don't
  package until it demonstrably works** — a skill encoding a not-yet-working method
  just propagates the stub. The SKILL.md is the *operational recipe* (terse,
  imperative: the loop + critics + earned rules + the tool), distinct from this doc
  (the *rationale*).
- **Bake the affect-critic into the loop.** The metric channel measures geometry;
  nothing yet runs observe→reconstruct→inhabit→name-reaction→compare. It can't be a
  `check` (it's perceptual), but it can be a fixed prompt run from the *rendered*
  image each pass — the perceptual counterpart to the metric board.
- **Tighten the loop's clock.** The per-iteration cost is dominated by the
  PNG readback (a multimodal read), not the draw call. Cheaper/coarser preview
  reads? Render at lower res for fast iteration, full res to verify.
- **3D modelling — the persistent-spatial sibling.** Same loop, but the canvas
  is a scene I rotate and inspect; marks become geometry/topology operations that
  push back via constraints. Likely the more natural medium for spatial reasoning.
- **Accumulation across sessions.** *Mechanism now identified* (§ The dustbin):
  distil recurring (attempt, intent, miss) triples into persistent priors —
  `gen.py` defaults, the critique checklist, and the intent→coordinate mapping —
  and graduate those into persistent memory. Open part: the distillation cadence
  and where the priors physically live. This is the gate between *drawing
  iteratively* and *learning to draw*.
