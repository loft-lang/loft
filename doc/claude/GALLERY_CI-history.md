<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# GALLERY_CI — history

## The target surface read a prebuilt wasm rlib (until 2026-11)

⚠ **A THIRD artefact is derived from the same wasm build and goes stale the same way, but by
a subtler route: `index/target_surface.json`.**  `make surface-gen` asks a **prebuilt** wasm
rlib which builtins exist, and the rebuild it needs is NOT `make wasm`, which builds the
wasm-bindgen package in the table above.  The rlib it reads comes from

```
cargo build --release --target wasm32-wasip2          --lib --no-default-features --features random
cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
```

Run `surface-gen` without those and it records builtins as UNAVAILABLE IN THE BROWSER and
commits that as derived truth.  Measured 2026-09-24: run straight after `make wasm`, it
re-added `store_load_url` and `store_load_url_trusted` to the unavailable list — silently
reverting a fix made on the same branch that morning, when a join had made the identical
mistake by hand.  With both rlibs built first it answers *"2 of 111 builtins unavailable"* and
the file is byte-identical to what was committed.

**Same file, same command, opposite answers, decided only by build ORDER.**  That is what makes
this one dangerous rather than merely annoying: the generator's two failure modes — a real
surface change and a stale read — are indistinguishable in the diff, so the diff is not
evidence on its own.  Read a builtin BECOMING unavailable as a suspected stale read first: a
builtin does not usually leave the browser.  `make ci` gets the order right (it builds both
rlibs before `gen_target_surface.py --check`); a hand-run is where it bites, and CLAUDE.md's
*"rebuild that rlib first"* does not say which rebuild is meant.

⚠ **And "first" is the wrong word — build them LAST.**  Measured again on 2026-09-24's seventh
join, by someone following the paragraph above: both rlibs built, *then* `make wasm` (the bundle
had conflicted in the merge, so it was owed), *then* `make surface-gen` — and it recorded the
falsehood anyway, re-listing `store_load_url`, `store_load_url_trusted`, `load_url` and
`load_url_verified`.  `make wasm` runs wasm-pack, which REBUILDS
`wasm32-unknown-unknown`'s rlib with its own feature set and clobbers the one the two commands
above produce.  So the order is `make wasm` → the two `cargo build --target wasm32-*` lines →
`make surface-gen`: the rlibs go last, after anything that builds wasm at all.  Rebuilding them
after `make wasm` and re-running `surface-gen` took the file back to byte-identical.
