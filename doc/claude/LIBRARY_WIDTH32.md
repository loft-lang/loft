# A library that works in a 32-bit program

How a library says that it works on a 32-bit device, what checks the claim, and what to
write so that it holds. The regime itself — what `#width 32` and `--width 32` change — is
[WIDTH32.md](WIDTH32.md); the device is [TANMATSU.md](TANMATSU.md).

## The claim

```toml
[package]
width32 = true
```

A package that declares this claims that **its own tests pass in a 32-bit program**: every
file under the regime, an `integer` 32 bits and a `float` a single. A package that says
nothing claims nothing, and is not asked.

Declare it for a package a device can use: a canvas, text, tiles, sound, geometry, a scene.
A package that needs a GPU, a window host or a desktop service does not declare it; it has
only to compile there when a package that does declare it depends on it.

## What checks it

| Where | What runs |
|---|---|
| The library CI, on every PR | `loft --width 32 --interpret --tests tests` with no display, for a package `loft compat width32` answers yes for |
| `scripts/registry_maintain.sh`, before the tag | the same: a package whose claim does not hold is not published |
| Here | `cd <package> && env -u DISPLAY loft --width 32 --interpret --tests tests` |

With no display, because a device has none: a test that draws through a window returns
at once. The run is the interpreter's; the compiled 32-bit build and the device itself are
measured by the device build's own gate (WIDTH32.md § The image gate).

## What to write

The compiler refuses a 64-bit value passed bare as an `integer` argument. In most other
places it narrows without a word and the value reads null, so these are rules to write by,
not errors to wait for.

- **A value that needs more than 31 bits is an `i64` wherever it is taken, answered or
  held.** A packed colour `0xAARRGGBB`, a key made of two coordinates, a hash, a time in
  microseconds, an id whose top bits are flags. In a 64-bit program `i64` is `integer`, so
  declaring it changes nothing there and `loft compat check` still calls the release a
  drop-in.
- **Widen before the operation, not after.** `(a & 255) << 24` is a 32-bit shift that reads
  null for an alpha of 128 or more; `((a & 255) as i64) << 24` is the 64-bit one. The same
  for a product: `((x + 1048576) as i64) * 2097152`.
- **Mask before you narrow.** `((c >> 16) & 255) as integer` is a value the compiler can see
  fits.
- **A literal past 31 bits carries `l`**: `0xFF000000l`.
- **Not every colour is wide.** A 16-bit RGB565 colour fits an `integer` and its C takes an
  `int32_t`: retype a value for what it holds, never for its name.
- **Decide in whole numbers.** A `float` is a single there, about seven digits: a division
  that lands on 4 as a double lands on 3.9999998. Where an answer is one of a few (which
  quarter of the compass, which cell, which frame of an animation), round to a whole number
  of a small unit first and decide on that. Count time in integers.
- **Where a result must match a reference bit for bit, say `f64`** for the part that
  decides it and keep it off the per-pixel and per-sample path: a resize works its weights
  out in `f64` once per row and adds pixels up in integers.

## Tests that hold at both widths

- A tolerance follows the float: a thousand millionth where it is a double, a few of its
  own steps where it is a single. Measure the step at run time (`1.0 + e > 1.0`), so one
  test file serves both.
- A test that shows what a DOUBLE does (a last-bit miss, a drift in a sum) does its sums in
  `f64` and `d` literals, or asks it of a double only.
- A deliberately huge argument is written so that it is huge at either width
  (`n as integer ?? 2147483647`). <!-- doc-lint: ok -->
- A test's own colours are `i64` and its wide literals carry `l`.

## When the claim cannot be made

A package whose tests do not all pass there leaves the flag out and says in its README what
does not. A test file the regime itself refuses is a loft matter: report it, and keep the
flag off until the file is written another way or loft takes it.
