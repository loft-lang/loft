
# Registry search — `loft search`

Design and open work for registry discovery.  Split out of [PKG_REGISTRY.md](PKG_REGISTRY.md).

---

## Open work — `loft search` (registry discovery, R8+)

`loft search [query]` **exists** (PKG.REG R8: dispatch `src/main.rs:3922`, handler
`search_registry` `src/main.rs:507`; `loft info <name>` alongside it).  Today it loads +
verifies the index, filters case-insensitively over name / description / categories, and
prints one `name <latest> — description` line per hit, sorted alphabetically.  That ships
the *discovery loop everywhere* goal — from any loft project, not just the loft-repo
in-repo catalogue (`LIBRARIES.md`) — but it is the minimal cut.

**Shipped 2026-06-19 (commit `1a129a92`):** steps S0–S5 below brought it to the advertised
surface — shared loader, ranking, `⚡auto-use` marker, `--json` (through loft's own JSON
model), offline cache fallback.  The OPEN work now is **function-level API discovery**
([§ below](#next--function-level-api-discovery-search-the-callable-surface)): make the
*function* surface searchable so an agent finds a *capability*, not just a package — and
the discoverable front door for lazy auto-use
([lib_plans/59-lazy-stdlib](lib_plans/59-lazy-stdlib)).

### Gap — shipped (R8) vs target

| Spec clause | R8 today | Step |
|---|---|---|
| Reuse install's fetch/verify, **do not duplicate** | reuses, but via a hand-copied `loft_install_load_index` (`src/main.rs:2450`) duplicating private `install::load_index` (`src/install.rs:258`) | S0 |
| Rank exact-name > name-prefix > description/category | alphabetical only (`hits.sort_by name`) | S1 |
| Per-hit: `loft install <name>` line + **auto-use** marker when latest declares `triggers` | description line only | S2 |
| `(categories)` on the listing line | omitted | S2 |
| `--json` for tooling | absent | S3 |
| Offline → fall back to cache **and say so** | loader falls back silently; search prints no note | S4 |
| Discoverable in `--help` | `print_help` lists `install`/`pin`, not `search`/`info` | S5 |

### Output contract (the human format these steps converge on)

One line per package; a query also gets an install hint beneath each hit:

```
<name> <latest>[ ⚡auto-use] — <description>  (<cat1>, <cat2>)
    → loft install <name>          # printed only when there is a query
```

`⚡auto-use` appears only when the latest non-yanked, non-prerelease version has a
non-empty `triggers` (`Version.triggers`, `src/registry_index.rs:60`); `(categories)` is
omitted when empty.  This is the public CLI contract — pin it with the tests below so it
cannot drift.

### Design — verifiable steps (S0–S5 · ✅ SHIPPED 2026-06-19, commit `1a129a92`)

All six landed with regression tests — `rank_hits_orders_exact_prefix_then_description`,
`to_json_string_round_trips_through_parse` (the parser's inverse `json::to_json_string`),
and the `load_index_reporting_falls_back_*` e2e; clippy + fmt clean.  Two as-built
deltas from the plan: S4 was *contained* — `load_index` keeps returning the plain index
(install path untouched) and a new `load_index_reporting` carries `stale_fallback`, so the
blast radius stayed off the 8 install callers; and S5 was already present in `--help`
(only the `--json` mention was added).  The steps below are the record.

Effort XS–S total; each step is independently shippable.  Land **S0 first** (it makes the
spec's "reuse, do not duplicate" true and gives the later steps one place to change);
**S4 last** (it widens the shared loader's return type, so it carries the most blast
radius).  Build under `--features registry`.

- **S0 — collapse the duplicate loader.** Make `install::load_index` `pub`
  (`src/install.rs:258`), point `search_registry` + `package_info` at it, delete
  `loft_install_load_index` (`src/main.rs:2450`).
  - *Check:* `grep -c 'fn loft_install_load_index' src/main.rs` → `0`;
    `cargo test --features registry registry` stays green (install + e2e still pass).

- **S1 — ranking.** Extract a pure `rank_hits(index, query) -> Vec<&Package>` into
  `registry_index.rs` (exact-name > name-prefix > description/category hit; alphabetical
  within a tier) and call it from `search_registry`.
  - *Check:* unit test in `registry_index.rs` over a fixture where query `text` matches a
    package named `text` (exact), `text_utils` (prefix), and one whose description
    contains "text" (desc) → asserts that exact ordering.

- **S2 — richer hit line.** Append `(categories)` and the `⚡auto-use` marker (latest
  version `triggers` non-empty) to the listing line; under a query, print the
  `→ loft install <name>` hint per hit.  Mirror `render_catalog`'s `writeln!`-into-String
  style (`src/registry_index.rs:748`).
  - *Check:* e2e test (mirror `tests/registry_e2e.rs` `FixtureServer`) with one package
    whose latest version has `triggers` and one without → output contains `⚡auto-use` for
    the first and not the second, and the `→ loft install` line appears with a query and
    not for the bare-listing run.

- **S3 — `--json`, through loft's own JSON model.** Parse `--json` in the `search` arm
  (`src/main.rs:3922`).  Build the result set as a `json::Parsed` tree (`src/json.rs:33`,
  loft's canonical JSON value) — `Parsed::Array` of `Parsed::Object` with
  `name / version / description / categories / auto_use / install` — and render it; do
  **not** hand-roll a string builder, and do **not** use `ir_schema::value_to_json` (that
  emits the schema-tagged IR dialect `{"k":"Int",…}`, not plain JSON).  The `json` module
  today only *parses*; add its missing inverse `pub fn to_json_string(p: &Parsed) -> String`
  there (store-free, the natural home), reusing the string-escape routine the `to_json`
  native already carries (`src/native.rs:~3872` — lift it to a shared `json::escape_str` if
  that stays clean).  `--json` is then: build the tree, `println!("{}", json::to_json_string(&tree))`.
  - *Check (round-trip via the existing parser):* a `json.rs` unit test asserts string-level
    idempotence — `let s = to_json_string(&sample); assert_eq!(to_json_string(&json::parse(&s).unwrap()), s);`
    (compare the *strings*, not the trees: `Parsed::Object` carries byte offsets that differ
    between a built tree and a parsed one).  Plus
    `target/release/loft search text --json | python3 -m json.tool` exits `0` (this box has `python3`).

- **S4 — offline note.** Widen `load_index` to report cache-vs-fresh (e.g. return
  `LoadedIndex { index, from_cache: bool }`); when `from_cache` because the fetch failed,
  `search`/`info` print one stderr line: `registry unreachable — showing cached index`.
  - *Check:* e2e — prime the cache via a reachable `FixtureServer`, then re-run with
    `LOFT_REGISTRY_URL` pointed at a dead port → stderr contains `cached`; the reachable
    run prints no such line.

- **S5 — discoverability.** Add `search` + `info` to `print_help` after the `install` /
  `pin` lines (`src/main.rs:167`).
  - *Check:* `target/release/loft --help` lists both `search` and `info`.

**Why this is the right next slice:** it finishes the discovery loop's *surface* (ranking,
auto-use signposting, machine-readable output) so library reuse is frictionless from any
project, and it makes lazy auto-use ([lib_plans/59-lazy-stdlib](lib_plans/59-lazy-stdlib))
*discoverable* rather than magic.

### Next — function-level API discovery (search the callable surface)

> **Shipped 2026-06-20 — S6, S6b, S7-publish, S7-CI, S8, S9 (complete).**  `loft search <fn>`
> surfaces function-level hits (signature + one-line doc + how-to-get) across the index `api`
> field AND the embedded stdlib, grouped under their package; `--json` carries per-function
> records `{ source, package, version, signature, doc, get }`.  `Version.api: Vec<ApiItem>`
> parses optional / default-empty (`registry_index.rs`, like `triggers`); `loft publish`
> auto-derives it from `src/*.loft` via the same `parse_pkg_api` extractor
> (`documentation::pkg_api_items` → `extract_api_items`); the stdlib feeds from the binary's
> embedded `default/*.loft` (`stdlib_sources::STDLIB_SOURCES` — one home shared with the WASM
> runtime, no disk dependency).  `registry_index::search_results` is the pure, tested ranker.
>
> **S7-CI — the no-drift trust gate.**  `loft api --json <dir>` emits a source dir's
> function-level surface (via the same `pkg_api_items`, so it equals what publish embeds); the
> registry's `validate.py` `gate_reproducible_build` re-derives it from the cloned source and
> REJECTS a pasted `api` that disagrees — so discovery can never point at a function the source
> lacks.  (A package without a GitHub homepage has its `api` trusted-as-pasted, exactly like
> its sha256 — no source to clone.)  The live registry index carries no `api` yet, so registry
> function-search lights up as packages republish; the stdlib surface works today.

S0–S5 search **package** metadata (name / description / categories).  That is too coarse
for an agent: the real question is *"is there a function that does X, and how do I call
it?"*  Today a registry package's function-level API is visible **only after you install
it** (`loft api` / `.loft/api` stubs read the extracted source) — a discover-to-install
circularity.  This slice breaks it: surface each library's **functions** — signature +
doc — from any project, without the source, **and** put the **stdlib on the same surface**.

**One mechanism, two feeds, one surface.**  There is already a single API-surface
extractor — `parse_pkg_api` / `render_pkg_api_text` (`src/documentation.rs:1122`,`:1231`),
heuristic and *good enough* (it is what `loft api` and the `.loft/api` stubs already use).
Run that one routine in two feeds and render both identically:

```
                 ┌─ publish time, over each library's source  → index.json `api` field (auto, CI-verified)
parse_pkg_api ──>┤
                 └─ search time, over the binary's embedded default/*.loft → stdlib hits (always present)
                                          │
                            loft search / --json  ← one surface; stdlib and libraries the same shape
```

**The invariant** (the load-bearing claim to hold): *the index's `api` field is a pure,
CI-verified function of the published source, and the stdlib feeds the same surface from
the binary's embedded `default/*.loft` — a consumer never extracts from source it does not
have.*

**S6 — index schema: the `api` field (auto-derived, no hand-crafted data).**  `Version`
gains `api: Vec<ApiItem>` where `ApiItem = { sig: String, doc: String }` (a **one-line**
doc summary — full doc stays available via `loft api <pkg>`).  Parse it with the
nested-object idiom already used for `binaries` (`src/registry_index.rs:290`); it is
**optional**, so older indexes lacking it default to empty and never error (same as
`triggers`, `:281`).  It is **emitted automatically** by the publish path — the exact
mirror of how `triggers` are derived from source and written today
(`src/main.rs:2055-2069`, `crate::triggers::derive_triggers` `src/triggers.rs:58`): the
author runs `loft package` and pastes the whole generated entry; **nothing is
hand-written**.

**S7 — CI is the source of truth.**  The registry's `validate.py` gate 3 already clones
the tagged source and runs `loft package` to re-check the tarball sha256
([registry_ci_template/validate.py](registry_ci_template/validate.py)).  Extend that gate
to **re-derive `api` from source and reject (or overwrite) a mismatch** — so the field is
a pure function of the code and cannot drift, even though the author pasted it.  This is
what makes discovery *trustworthy*: an agent never finds a function that no longer exists.

**S8 — stdlib on the same surface.**  The loft binary already embeds `default/*.loft`.
`loft search` runs the **same** extractor over that embedded source at search time, so
stdlib functions appear as hits **identical in shape** to library functions — differing
only in the "how to get it" line ("built in — use directly", no install).  The stdlib API
therefore lives in the binary (always fresh) and **does not bloat the fetched index**.

**S9 — rich results: tell the agent what it needs.**  A function match is a function-level
hit carrying the three things needed to act — *what it does* (doc), *how to call it*
(signature), *how to get it* (install vs built-in).  Group the matching functions under
their package; `--json` carries them structured.

```
<pkg> <ver>
    <signature>
        <one-line doc>
    → loft install <pkg>            #   or:  (stdlib — built in, use directly)
```

`--json` per function: `{ "source": "stdlib"|"registry", "package", "version",
"signature", "doc", "get" }` — everything an agent needs to decide and call, in one place.

**Decided shape** (the three forks, resolved): functions **grouped under their package**
(scannable, agent still sees every match) · **one-line doc summary in the index**, full
doc on demand via `loft api` (keeps the index lean) · **CI re-derives** the `api` field
(no trusted hand-crafted data) · stdlib extracted **on the fly** from embedded source
(simple, always current).

**Failure paths** (the generative pass — each is why a property above is load-bearing):

- *Stale / edited `api` field* → S7's CI re-derive rejects the mismatch; the field can
  only ever equal the source.  (Without S7, discovery becomes confidently wrong — worse
  than absent.)
- *Signature drift across versions* → `api` is **per-`Version`**, so each version carries
  its own surface; an old pin still describes the functions it actually shipped.
- *stdlib vs library name collision in results* → every hit is tagged `source`
  (`stdlib`|`registry`); results never merge the two silently.
- *Index growth* → only a one-line summary per function lives in the index; if the single
  `index.json` still outgrows a single fetch, split to a per-package `api` file fetched on
  demand (the only change that dents the "one agnostic fetch" property — defer until
  measured).
- *Function with no/cryptic doc* → discoverable by name + signature only; quality is
  bounded by the library's doc quality, which ties discovery to
  [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) / [DOC_QUALITY.md](DOC_QUALITY.md) /
  [API_SURFACE.md](API_SURFACE.md) `api-lint`.
- *Heuristic extractor misses an exotic signature* → it degrades to "no hit / raw
  signature line", never a wrong call — `good enough` is the right bar for discovery.

**Sequencing & effort:** S6 schema + parse (XS) → S7 publish auto-derive + CI verify (S)
→ S8 stdlib feed (S) → S9 rich results + `--json` (S).  Builds directly on the shipped
S0–S5; the client stays agnostic throughout (it only reads the enriched index + the
embedded stdlib).

**Why this resolves agent data discovery** (per the evaluation that drove it): the
published ecosystem's *callable* surface becomes visible from any project **without
installing**, **authoritative** because CI-derived, and **unified with the stdlib** — so
"what can do X, and how do I call it?" is answered in one place, with the signature to use
it and the command to get it.

### Phase 2 — richer data + live deployment (S10–S12)

Phase 1 (S0–S9, shipped) built the mechanism; the gathered data is still thin (one-line
summaries) and lives only in the binary's stdlib — the **live registry index has 0 of 20
packages carrying `api`**.  Phase 2 widens the data and gets it into the live index two ways:
future releases self-populate, and a one-time backfill converts everything already published.
**The invariant is unchanged** — `api` is a pure, CI-verified function of a version's
published source, never hand-written, never drifting.

#### S10 — gather functions AND types; match the FULL doc (Google-like) — SHIPPED 2026-06-20

Each `api` item today is `{ sig, doc }` with `doc` = the *first* documentation line (a title),
so search is title-only: "anything about collision?" misses `rects_overlap` unless "collision"
is in its title.  Widen the gathered data to the whole paragraph so search matches ANY keyword
in the docs.  Per public item:

- **`sig`** — the declaration with the body stripped: `pub fn sha256(data: text) -> text`,
  `pub struct Rect`, `pub enum Shape`.  **Functions AND types** (`pub fn` / `pub struct` /
  `pub enum`) — searching `rect` finds both `struct Rect` and `fn rect_circle_overlap`.
  (`pub const` / values stay out: the question is "what can I CALL or USE", and their
  inline-comment tails make noisy signatures.  The extractor filters `pub ` items to
  fn/struct/enum and strips any trailing `// …` from the sig.)
- **`doc`** — the FULL contiguous documentation paragraph above the item (every `//` line,
  joined), not just the first.  This is the keyword corpus search matches.

**Distinguishing the doc from other comments** — the rule (already implemented in
`parse_pkg_api`, made load-bearing here): the doc is the CONTIGUOUS `//` block IMMEDIATELY
above the `pub` item; a blank line or a code line between a comment and the item severs it
(that comment is not the doc); `// --- Section ---` decorative headers are section markers
(they group items + reset the accumulator), never docs; a `#rust "…"` annotation line does not
sever the doc.  This is "good enough" because the rule is mechanical and the conventions are
ALREADY followed everywhere (verified: crypto's `// SHA-256 hash …` paragraphs above each fn,
shapes' `// Axis-aligned bounding box …` above `pub struct Rect`, the `// --- Public API ---`
markers).  A library wanting better search writes a fuller paragraph; one that writes nothing
is still found by name + signature.

Data-model change: `ApiItem.doc` holds the full paragraph (the matching corpus); the search
RESULT still prints the one-line summary (the first sentence) for a clean display; `--json` and
the index carry the full `doc`.  Matching becomes Google-like: lowercase the query, split into
terms, and require EVERY term to appear somewhere in `sig`+`doc` (AND-semantics, the search
default) — so "hash hex" narrows rather than widens.  Cost: the index grows from titles to
paragraphs — still small (a paragraph per item; the whole index is ~22 KB today), and keyword
search is the whole point.

#### S11 — forward: every release self-populates the index

Today a library release is MANUAL (verified): the author tags `<pkg>-v<ver>`, runs
`loft publish` locally, and pastes the entry into a `loft-lang/registry` PR; the registry
`validate.py` is VALIDATE-ONLY (it never writes `index.json`).  `loft publish` already EMITS
`api`, so the field is auto-GENERATED — the author never writes it.  The gap is ENFORCEMENT: an
author who uses `loft package` (the basic entry, no `api`) or hand-edits the row would ship a
wrong or absent field.

Make `validate.py` the authority.  Its `gate_reproducible_build` already clones the tagged
source and runs `loft`; extend it (the S7-CI gate, already in this repo's
`registry_ci_template/validate.py`) so that for every NEW version entry it (1) re-derives `api`
from the cloned tagged source via `loft api --json`, and (2) REQUIRES the entry's `api` to equal
it — rejecting a missing, stale, or hand-edited field.  Then every future release carries the
correct `api` automatically: the author gets it free from `loft publish`, and the gate makes it
non-optional and proven-against-source.  No registry write-access or bot is needed — the author
pastes what `loft publish` produced, the gate proves it equals the source.

Deployment (registry repo, one PR): port the template gate into
`~/workspace/registry/tools/validate.py`; add `api` to `gate_schema`'s required set for new
entries; keep the `registry` feature in the CI's `cargo build --release --bin loft` so
`loft api --json` exists.  The loft side (`loft publish` emitting, `loft api --json`) is already
shipped.  *(Optional convenience: a `--fix` mode that writes the corrected entry, so a wrong
submission is handed the fix rather than only rejected — not required for correctness.)*

#### S12 — backfill: one-time conversion of every existing lib

S11 only fills FUTURE releases; the ~20 already-published packages (0 with `api`) need a
one-time conversion.  A script (registry-repo `scripts/backfill-api.py`):

1. For each package's LATEST version in `index.json` (search shows the latest; all-versions is
   an optional superset), read its tag `<pkg>-v<latest>`.
2. Clone the package's monorepo at that tag into a tempdir — or, since the six `loft-libs-*`
   monorepos are checked out locally and freshly released, use the local subdir when its `main`
   equals the tag (the fast path; fall back to the tag clone when they differ).
3. `loft api --json <subdir>` → the `api` array.
4. Write it into that version entry.
5. Open ONE registry PR with all backfilled entries; the S11 gate verifies each — and PASSES,
   because they were re-derived from the very source the gate re-clones.

The backfill is the SAME derivation as the forward gate, applied to existing rows — consistent
by construction (one extractor, `loft api --json`).  After it merges, registry function-search
lights up for every shipped library; until then only the stdlib surface returns function hits.

**Sequencing & effort:** S10 widen-doc + types **(SHIPPED on `searching`: full doc paragraph
as the keyword corpus, AND-of-terms matching, fn/struct/enum/typedef/interface, consts dropped,
clean type sigs)** → S11 port the gate + require the field (S, registry repo) → S12 the backfill
script + one PR (S, registry repo).  S11 + S12 are registry-repo PRs that depend on a loft
release carrying `loft api --json`.
