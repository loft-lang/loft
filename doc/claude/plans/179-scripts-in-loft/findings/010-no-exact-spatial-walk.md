# No exact distance-ordered walk over `spatial` — the `..:n` form is approximate by design
axis: behaviour
met-by: the games (owner, 2026-09-30) — agents asking for a precise walk; not met by a port yet
status: accepted
fix: a second, slower `spatial` implementation with an exact distance-ordered walk beside the fast Morton one, chosen by the user — one more implementation of the kind construct @PLN91 strand 3 introduces, so it waits on that seam rather than on a second parser branch
ref: plans#91 (strand 3)
probe: 
expect: 
checked: 
holds: unprobed

`spatial<T[x, y]>` answers three queries (`src/spatial.rs`): `within` and `nearest` are exact, resting on the Morton code's monotonicity; the open-ended walk `..:n` (the `Near` form) follows the Z-order curve, which tracks distance closely but jumps at quadrant boundaries, so a truly-near point can arrive late. It is documented as "good enough" for aggro and interest management and never for a correct radius or k-NN — but it is the form a game reaches for when it wants "the next n by distance", and the exact forms cost more than that use wants. Making the walk exact in place would slow every user; a second implementation is cumbersome today because the kind is baked into the parser, typedef, codegen, store layout, reflection and natives (DATABASE_INDEXES.md § the approximate walk). The same seam would admit a `spatial` over `float` coordinates, which the baked kind refuses (loft#1431). A probe cannot be written until the exact form has a name; the measurement then is a walk over a seeded point set whose order must equal the sorted Euclidean distances.
