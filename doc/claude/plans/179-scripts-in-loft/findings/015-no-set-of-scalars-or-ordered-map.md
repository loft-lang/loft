# No set of scalars and no insertion-ordered map — a `frozenset` becomes a hand-sorted vector
axis: clarity
met-by: scripts/rule_predicate_audit (strand 3) — sites keyed by a SET of variant names, reported in first-seen order
status: fixed
fix: nothing in the stdlib — a set of scalars IS `sorted<Word[key]>` over a one-field struct (membership `s[x] != null`, `+=` replaces a duplicate, key order, a prefix range); the building and the algebra are the `script` library's (0.2.0): `set(words)` the builder, `put` / `has` / `minus` / `words` the methods, so a script names no type.  The bare `set(…)` is the builder-trigger extension of auto-load (the manifest's `[triggers] builders`), with `script::set(…)` always and a program's own `set` winning.  The two orders a report wants (first-seen beside keyed) stay a vector beside a hash index, which is what the port keeps
ref: the port re-measured over `sorted<Name[key]>` (@PLN192's session): 178 → 166 non-comment lines, `add_name` (10 lines) gone, `one_apart` 12 → 9, the twin same on all three channels in every case on both backends; then over the library's set (`script` 0.2.0, 2026-10-14): 166 → 148, its own `Name` and walk gone, `one_apart` a `minus`, the twin the same on both backends against the library's checkout — green in CI once 0.2.0 reaches the registry.  A scalar-keyed `sorted<text>` (no wrapper struct, no `.key`) would save one line and one field read and is not built: it would be parser sugar over the same tree
probe: 
expect: 
checked: 
holds: no longer holds — the stand-ins the finding counted are the collection the language had

The original keeps `sites: dict[frozenset[str], list]` and prints `a ^ b` for every pair one variant apart. The port carries three stand-ins: `add_name` (insert a name into an ascending `vector<text>` unless present, 10 lines) because `sorted<…>` collections take a struct with a key field and not a bare `text`; `one_apart` (the symmetric difference of two sorted lists, 12 lines); and an `Idx { key, at }` hash beside an `entries: vector<Entry>` because a `hash<Entry[key]>` iterates in hash order and the report wants first-seen order. Together they are a third of the port's 177 non-comment lines against the original's 55 — the rest is the two regexes spelled by hand (finding 011).
