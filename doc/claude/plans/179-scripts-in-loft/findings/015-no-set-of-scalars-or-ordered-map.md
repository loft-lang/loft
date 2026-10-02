# No set of scalars and no insertion-ordered map — a `frozenset` becomes a hand-sorted vector
axis: clarity
met-by: scripts/rule_predicate_audit (strand 3) — sites keyed by a SET of variant names, reported in first-seen order
status: open
fix: a `set<T>` of scalars over the existing `sorted<T[key]>` machinery (membership, `+=`, difference, iteration in order), and a keyed collection that also answers "in what order were these added" — or a documented idiom that costs one line, not twelve
ref: 
probe: 
expect: 
checked: 
holds: 

The original keeps `sites: dict[frozenset[str], list]` and prints `a ^ b` for every pair one variant apart. The port carries three stand-ins: `add_name` (insert a name into an ascending `vector<text>` unless present, 10 lines) because `sorted<…>` collections take a struct with a key field and not a bare `text`; `one_apart` (the symmetric difference of two sorted lists, 12 lines); and an `Idx { key, at }` hash beside an `entries: vector<Entry>` because a `hash<Entry[key]>` iterates in hash order and the report wants first-seen order. Together they are a third of the port's 177 non-comment lines against the original's 55 — the rest is the two regexes spelled by hand (finding 011).
