<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# DRAWING-history.md — records moved out of [DRAWING.md](DRAWING.md)

The working doc carries the reasoning that holds now; what is here is the superseded v1 design and finished next-steps, kept for the reasoning they record. The current tool is [`sketch/`](../../sketch) ([`sketch/README.md`](../../sketch/README.md)).

## Moved from DRAWING.md on 2026-10-31

### The efficient input primitive — a long-lived canvas fed one line at a time

> **Historical (v1).** The append-and-tail `cmds.txt` model below was replaced by a
> single editable `scene.draw` source the daemon reloads on save (editing *is*
> undo). The *reasoning* — stateful canvas, cheap incremental marks — still holds;
> the current mechanics live in [`sketch/`](../../sketch).

The naive approach (regenerate a Python script per drawing) **reinvents the pen
on every stroke** — wasteful and stateless. The efficient design:

- A **persistent daemon** owns the stroke list (the canvas state).
- It **tails a command file**; each new line is one stroke appended.
- It **re-rasters to a PNG** on every change.
- I `Read` the PNG back to close the perceive loop.

Per-stroke cost for me is then a **single cheap append** —
`echo "Line ..." >> cmds.txt` — with no process restart and no render
orchestration on my side. The daemon holds the state; I just issue marks and look.

Why a *file* and not stdin/FIFO: background tool calls can't keep piping into a
process's stdin across turns, and a FIFO's open/close/EOF semantics are fiddly.
An append-only command file is dead simple, replayable (cat the file to redraw
from scratch), and idempotent.

```
  echo "Line (..) - (..)" >> cmds.txt        Read canvas.png
        │                                          ▲
        ▼                                          │
   ┌─────────────────────────────────────────────────┐
   │  daemon: tail cmds.txt → append stroke → render   │
   │  (holds canvas state across the whole session)    │
   └─────────────────────────────────────────────────┘
```

---

### Command grammar (v0)

> **Historical (v1).** This is the first grammar. The shipped tool adds `Circle`
> (round by default), `Poly`, `name` / `landmark` / `check` (the metric channel),
> and `# SHOULD` comments, and reads [`sketch/scene.draw`](../../sketch/scene.draw) (not a
> `/tmp` `cmds.txt`). **Current grammar: [`sketch/README.md`](../../sketch/README.md).**

Coordinates are **fractions of the paper size (0..1)**, so they're
resolution-independent. Origin top-left, y grows downward (image convention).

```
Line (x1,y1) - (x2,y2)          draw a segment
Line (x1,y1) - (x2,y2) w=N      ... with pen width N px
clear                            erase all strokes
size WxH                         set paper pixel size (resets canvas)
```

Example: `Line (0.02, 0.12) - (0.4, 0.7)`

Reference daemon: `/tmp/sketch/draw.py` (throwaway; design captured here so it's
reconstructable). Renders to `/tmp/sketch/canvas.png`, tails `/tmp/sketch/cmds.txt`.
Intent lives in `/tmp/sketch/intent.txt` — **authored before the first stroke and
frozen**; editing it to match what was drawn turns the loop into theater.

**Marking-vocabulary layer.** The daemon speaks only `Line`. Higher-level shapes
(circle, arc, hill curve, rays, hatch) are composed from line segments by a small
helper (`gen.py`) that *emits* `Line` commands — i.e. just batched appends, not a
second renderer. Curves are polylines; the daemon stays dumb on purpose.

### Open questions — finished items

- **World pass: DONE** (§ The world pass) — produced a coherent *place* (cube→home,
  the sun-ray fix), confirmed it converges, and surfaced the over-cut-sun and
  symbols-carry-affect findings.
