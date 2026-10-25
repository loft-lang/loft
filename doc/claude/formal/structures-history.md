<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/structures-history.md — the deviation register for [structures.md](structures.md)

> **The rules are next door.**  [structures.md](structures.md) states the bytes of the stored
> structures and of an image; this file is its TIMELINE — every place the code was measured not
> to keep them, when, what it cost, and what closed it.

## Deviations

- **D-struct-1 — CLOSED (2026-10-25, loft#2172): a paged hash lookup misses a narrow key.**
  - **Violates:** `S-Paged-Same`, `Col-Axis` (collections.md), `L-Narrow-Decode` (layout.md).
  - **Where:** `paged_reader::key_compare_reader` decoded the 1- and 2-byte key widths from the
    raw bytes with no `start` bias, and read a byte as signed, while the resident compare, the
    digest and the paged spatial reader (`PagedSpatial::axis_value`) all decoded them with it.
    The probe found the right bucket and the compare then rejected the entry.
  - **Effect:** `store_load_key` / `store_load_keys` / a lazily bound hash answered `false` for a
    key the image holds — 13 of 18 present keys over the widths `limit(-100,100)`,
    `limit(0,250)` (128 and up), `limit(-20000,20000)`, `limit(-30000,35000)` and
    `limit(5000,70000)`, measured on main `b36d0449f`, both backends; only the 4-byte width and a
    zero-based byte below 128 loaded.  The whole-image load read every key right.
  - **Closed by:** one paged decode of an integer key or axis field (`paged_reader::field_value`),
    read by the hash compare and the spatial walk alike.  Guard:
    `tests/scripts/2172-a-paged-lookup-decodes-a-narrow-key-as-the-store-does.loft`, falsified
    on `b36d0449f` (exit 1 on both backends).

- **D-struct-2 — CLOSED (2026-10-25, loft#2173): an image is written without its sidecar and the
  call answers true.**
  - **Violates:** `S-Pair`.
  - **Where:** `Stores::persist_copy` wrote the image with a plain `std::fs::write`, then the
    sidecar, and returned `true` whatever the sidecar write answered (the failure was printed
    under `LOFT_LOADER_STATS` only).
  - **Effect:** a sidecar that could not be written left the new image beside the PREVIOUS
    write's sidecar, or beside none; the next load read it under the wrong identity or raw
    (`D-struct-3`).  Measured on main `b36d0449f`, both backends: the second write answered
    `true`, and the path then read the second value beside the first write's sidecar.
  - **Closed by:** both files staged beside the target (`<path>.tmp`,
    `LayoutIdentity::stage_beside`) and renamed into place only when both are written; the call
    answers `false` otherwise and the previous pair stays.  The C backend's writer
    (`loft_image_write`) stages the same way.  Guard:
    `tests/scripts/2173-an-image-and-its-sidecar-are-written-together.loft`, falsified on
    `b36d0449f` (exit 1 on both backends).

- **D-struct-3 — CLOSED (2026-10-25, loft#2174): an image with no sidecar was read raw.**
  - **Violates:** layout.md `L-Sound` (*"a raw handoff is admitted ONLY when the two identities
    are equal"*); `S-Gate`'s last clause records the shipped behaviour that contradicts it.
  - **Where:** `schema_sidecar::check_file` maps a missing sidecar to `Fresh`; the URL loaders map
    a failed sidecar fetch to `Fresh` as well, so a network error also skips the gate.
  - **Effect:** measured on main `b36d0449f`: an image written as
    `struct R { k: integer, a: integer, b: integer }`, its sidecar deleted, read by a program
    declaring `b` before `a`, answers `a=20 b=10` through `store_load` and through
    `store_load_key` — silently swapped.  With the sidecar the same read converts (`a=10 b=20`)
    or, paged, refuses.
  - **Closed by:** the owner's decision for `L-Sound` as written — a bare image is refused, and
    so is a sidecar that cannot be read or fetched.  `check_beside` answers
    `SchemaVerdict::Bare` for a missing sidecar (the project baseline keeps `Fresh`: a first
    build owes none), and every image gate — whole, untrusted, URL, paged, and the C backend's
    `loft_image_gate` — refuses it with one sentence.  A shipped-surface change:
    `REMOTE_STORES.md` had documented the missing sidecar as normal.  Guard:
    `tests/scripts/2174-an-image-without-its-sidecar-is-refused.loft`, falsified on
    `b36d0449f` (exit 1 on both backends).
