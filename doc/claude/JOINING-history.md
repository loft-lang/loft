# JOINING — history

The record behind [JOINING.md](JOINING.md): measurements and rulings, dated.  The working doc
keeps only what still applies.

## The twenty-minute bound (2026-10-16)

The owner ruled that every join routine must finish well inside twenty minutes and that an
hour is unacceptable.  Measured the same day on the tuxedo box: `scripts/join.py rederive`
ran 1h27m — `make rewrite-census` built the crate in release (20–40 min there), then the
browser bundle (a wasm build) and `cargo test --release --test doc_hygiene` for the
ignored-tests baseline were regenerated locally although `derive.yml` regenerates both on a
runner; `verify` ran a 544 s pre-flight, the optional ratchet twice, two clippys and three
release test builds the PR gate repeats.  The census itself takes 30 s on the dev binary.
`rederive` now skips `ci` artefacts by default and the census row uses the dev binary;
`verify` lost its duplicated ratchet row and runs its test rows in the dev profile.
