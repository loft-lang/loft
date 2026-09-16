<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# assets — a game's content pack, which IS a loft store

```sh
loft install assets
```

Art, audio, fonts, and the **scenes** that place them, in one pack a game reads from
disk or straight off a static file server. There is no codec and no parse step: the
struct definitions in `src/assets.loft` **are** the file layout, so a browser reading a
pack by HTTP range fetches the pages one lookup touches and nothing else — with no
server-side code at all.

> **Needs a loft newer than v2026.8.0.** Reading a pack back exercises the store
> allocator's linear scan, and a compaction-on-load defect there made `store_load`
> never return — no diagnostic, no bound, on a call that only reads a file. The fix
> is on `loft-lang/loft@main` and is not in the v2026.8.0 release, so on that release
> a pack read **hangs**. Until the next loft release carries it, run with
> `LOFT_NO_COMPACT_ON_LOAD=1`, which skips the rebuild the defect lives in.

```loft
use assets;

fn main() {
  // Build a pack: a texture page's shape in the metadata, its pixels in the bytes.
  p = new_pack();
  blobs: hash<Blob[bl_key]> = [];
  p.pages += [PageInfo { pi_name: "mobs", pi_w: 64, pi_h: 32, pi_blob: "page/mobs" }];
  pixels: vector<u8> = [];
  for i in 0..64 * 32 * 4 { pixels += [(i % 251) as u8 ?? 0]; }
  blob_put(blobs, "page/mobs", Rgba, pixels);          // premultiplied RGBA
  _ = pack_write(p, blobs, "game");                    // game.meta.store + game.blobs.store

  // Read it back — from a path here, from "https://cdn.example/game" unchanged.
  q = new_pack();
  _ = pack_read(q, "game");                            // the metadata, whole
  res: hash<Blob[bl_key]> = [];
  _ = prefetch(res, blobs_path("game"), ["page/mobs"]);
  page = res["page/mobs"];                             // resident: no fetch in the frame
  println("{q.pages["mobs"].pi_w}x{q.pages["mobs"].pi_h}, {len(page.bl_bytes)} bytes");
}
```

```
64x32, 8192 bytes
```

## Why a pack is two files

Split by how each half is **read**:

| file | holds | read |
|---|---|---|
| `<base>.meta.store` | `PageInfo` `Cell` `Seq` `Def` `Anim` `Scene` `Placed` | **whole**, at boot |
| `<base>.blobs.store` | every byte: page pixels, audio, fonts | **paged**, one key at a time |

The metadata is small and every part of it is needed from the first frame; the art is
large and a level needs a slice of it. Mixing them makes the boot load pay for the
world. The bulk half is rooted on the `hash` itself rather than reached through a field,
because a collection reached through one records the *wrapper* as the store's type and
the paged loaders refuse such a store.

## A scene is definitions plus placed instances

GameMaker's object/room split, not a flat node dump — a flat dump cannot say that two
goblins are the same goblin. A `Def` is the object (and a **light is a `Def` too**, placed
exactly like a sprite); a `Placed` is one instance of it in a `Scene`. A walk cycle is
asset data rather than code, so `(action, facing) -> sequence` lives in the pack's own
animation table:

```loft
seq = anim_of(pack, "goblin", "walk", 2);      // "goblin_walk_n"
```

## The five contracts a signature does not carry

Every key is a bare `text` and every blob a bare `vector<u8>`, so each row below is a
well-typed call that does something other than what was meant.

| contract | where it is pinned |
|---|---|
| **A pack is two files, and `pack_write` / `pack_read` name them.** Handing the base path straight to `store_load` reads nothing, because the base is not a file — use `meta_path` / `blobs_path` to name a half. | [`tests/pack.loft`](tests/pack.loft) |
| **`+=` on a `&hash` parameter retypes it** to a `vector<Blob>` and the program stops compiling. `blob_put` is the keyed insert that does not. | [`tests/pack.loft`](tests/pack.loft) |
| **`prefetch` answers what it FETCHED, not what is resident.** `0` means everything asked for was already there — which is what a frame asserts — and never that the read failed. A key the pack does not have is skipped, not invented. | [`tests/pack.loft`](tests/pack.loft) |
| **`keys_near` is a boundary call, never a frame call.** A range read is a round trip and a frame is 16 ms, so a frame that DISCOVERS it needs an asset stutters. Ask at a load or a level edge; a negative radius means the whole scene. A light contributes no key, because it draws nothing. | [`tests/scene.loft`](tests/scene.loft) |
| **The layout IS the format.** A field that sits four bytes further along on one target reads a *neighbour's* value on the other, silently, and the file looks corrupt rather than the layout looking wrong. Compare `layout_fingerprint()` across targets before reading a pack built elsewhere. | [`tests/layout.loft`](tests/layout.loft) |

## Reading a pack over HTTP

One `base` drives both halves whether it is a path, an `http(s)://` URL or a `file://`
one, so "develop against a file, ship against a CDN" is a change of one string:

```loft
base = env_variable("PACK");               // or "https://cdn.example/game"
if base == "" { base = "game" }
_ = pack_read(q, base);
_ = prefetch(res, blobs_path(base), want);
```

The server needs to honour `Range` and nothing else — no code, no API, a directory of
files. Measured on a 4 MB pack: **two 64 KiB pages per key**, about 3 % of the file.

The two halves reach different loaders underneath and do *not* accept the same
spellings: the metadata half reads a `file://` URL, the paged half refuses one. This
package drops the scheme for the paged read so one base works for both — without it, the
same string would load a game's scenes and silently none of its art.

A URL is fetched from a source you TRUST. The image is structurally validated, so a
corrupt file is refused rather than adopted, but its contents are not authenticated:
serve packs from your own origin, or verify the download yourself with `store_load_url`
(which pins a SHA-256) before reading it.

A pack is written on a desktop and read anywhere: `pack_write` needs a filesystem, and a
browser reads packs rather than building them.

## What this package does not do

It stores bytes and says what they are (`BlobKind`); it does not **decode** them. PNG
decoding, audio decoding and GL upload belong to the consumer — which is what keeps this
package headless, so an asset pipeline needs no GPU.

## License

LGPL-3.0-or-later — see [LICENSE](../LICENSE).
