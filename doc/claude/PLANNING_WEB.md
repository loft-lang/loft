<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PLANNING — web services, repository and web IDE items

Split from [PLANNING.md](PLANNING.md) by subject: H (HTTP / JSON), R (repository), W (web IDE) and E (library ergonomics).  Shipped and withdrawn items are in [PLANNING-history.md](PLANNING-history.md).

---

## H — HTTP / Web Services

Full design rationale and approach comparison: [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md).

The `#json` annotation is the key enabler: it synthesises `to_json` and `from_json` for a
struct, making `Type.from_json` a first-class callable fn-ref that composes with `map` and
`filter`.  The HTTP client is a thin blocking wrapper (via `ureq`) returning a plain
`HttpResponse` struct — no thread-local state, parallel-safe.  All web functionality is
gated behind an `http` Cargo feature.

---

### H1  `#json` annotation — parser and `to_json` synthesis
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, Phase 1
**Description:** Extend the annotation parser to accept `#json` (no value) before a struct
declaration.  For every annotated struct, the compiler synthesises a `to_json` method that
reuses the existing `:j` JSON format flag.  No new Rust dependencies are needed.
**Fix path:**

**Step 1 — Parser** (`src/parser/expressions.rs`):
Extend the annotation-parsing path that currently handles `#rust "..."` to also accept
bare `#json`.  Store a `json: bool` flag on the struct definition node (parallel to how
`#rust` stores its string).  Emit a clear parse error if `#json` is placed on anything
other than a struct.
*Test:* `#json` before a struct compiles without error; `#json` before a `fn` produces a
single clear diagnostic.

**Step 2 — Synthesis** (`src/typedef.rs`):
During type registration, for each struct with `json: true`, synthesise an implicit `pub fn`
definition equivalent to:
```loft
pub fn to_json(self: T) -> text { "{self:j}" }
```
The synthesised def shares the struct's source location for error messages.
*Test:* `"{user:j}"` and `user.to_json()` produce identical output for a `#json` struct.

**Step 3 — Error for missing annotation** (`src/typedef.rs`):
If `to_json` is called on a struct without `#json`, emit a compile error:
`"to_json requires #json annotation on struct T"`.
*Test:* Unannotated struct calling `.to_json()` produces a single clear diagnostic.

**Effort:** Small (parser annotation extension + typedef synthesiser)
**Target:** 0.8.4
**Depends on:** —

---

### H3  `from_json` codegen — scalar struct fields
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, Phase 2
**Description:** For each `#json`-annotated struct whose fields are all primitive types
(`text`, `integer`, `float`, `single`, `boolean`, `character`), the compiler
synthesises a `from_json(body: text) -> T` function.  The result is a normal callable
fn-ref: `User.from_json` can be passed to `map` without any special syntax.
**Fix path:**

**Step 1 — Synthesis** (`src/typedef.rs`):
After H2 is in place, extend the `#json` synthesis pass (H1 Step 2) to also emit
`from_json`.  For each field, select the extractor by type:

| Loft type | Extractor call |
|-----------|---------------|
| `text` | `json_text(body, "field_name")` |
| `integer` | `json_int(body, "field_name")` |
| `float` / `single` | `json_float(body, "field_name")` |
| `boolean` | `json_bool(body, "field_name")` |
| `character` | first char of `json_text(body, "field_name")` |

The synthesised `from_json` body is a struct-literal expression using the above calls.
Fields not in the table (nested structs, enums, vectors) are silently skipped in this
phase (H5 adds them).

**Step 2 — fn-ref validation** (`src/compile.rs` or `src/state/codegen.rs`):
Verify that `Type.from_json` resolves as a callable fn-ref with type
`fn(text) -> Type`, so it can be passed directly to `json_items(...).map(...)` and
`json_items(...).filter(...)`.

*Tests:*
- `User.from_json(body)` returns a struct with all fields set from JSON.
- `json_items(resp.body).map(User.from_json)` returns a `vector<User>`.
- Absent JSON key sets the field to its zero value (0, "", false).
- Struct with a nested `#json` struct field compiles without error (nested field gets zero value until H5).

**Effort:** Medium (typedef synthesiser + fn-ref type check)
**Target:** 0.8.4
**Depends on:** H1, H2

---

### H4  HTTP client stdlib and `HttpResponse`
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, stdlib additions; PROBLEMS #55
**Description:** Add blocking HTTP functions to `default/06_web.loft` backed by `ureq`.
All functions return `HttpResponse` — a plain struct — so there is no thread-local status
state and the API is parallel-safe (see PROBLEMS #55).
**Fix path:**

**Step 1 — `HttpResponse` struct** (`default/06_web.loft`):
```loft
pub struct HttpResponse {
    status: integer
    body:   text
}

pub fn ok(self: HttpResponse) -> boolean {
    self.status >= 200 and self.status < 300
}
// Mirror the File read interface so HTTP sources are interchangeable with
// file sources in any function that processes text.
pub fn content(self: HttpResponse) -> text {
    self.body
}
pub fn lines(self: HttpResponse) -> vector<text> {
    self.body.split('\n')  // strips \r so CRLF bodies match LF bodies
}
```
No `#rust` needed; all three methods are plain loft.  `lines()` uses the same
CRLF-stripping logic as `File.lines()` — HTTP/1.1 bodies frequently use CRLF.

**Optical similarity with `File`:** the shared method names let processing
functions accept either source without modification:
```loft
fn process(rows: vector<text>) { ... }
process(file("local/data.txt").lines());
process(http_get("https://example.com/data").lines());
```

**Step 2 — HTTP functions declaration** (`default/06_web.loft`):
```loft
// Body-less requests
pub fn http_get(url: text) -> HttpResponse;
pub fn http_delete(url: text) -> HttpResponse;

// Body requests (body is a text string, typically to_json() output)
pub fn http_post(url: text, body: text) -> HttpResponse;
pub fn http_put(url: text, body: text) -> HttpResponse;
pub fn http_patch(url: text, body: text) -> HttpResponse;

// With explicit headers (each entry: "Name: Value")
pub fn http_get_h(url: text, headers: vector<text>) -> HttpResponse;
pub fn http_post_h(url: text, body: text, headers: vector<text>) -> HttpResponse;
pub fn http_put_h(url: text, body: text, headers: vector<text>) -> HttpResponse;
```

**Step 3 — Rust implementation** (`src/native_http.rs`):
Use `ureq::get(url).call()` / `.send_string(body)`.  Parse each `"Name: Value"` header
entry by splitting at the first `:`.  On network error, connection refused, or timeout,
return `HttpResponse { status: 0, body: "" }` — never panic.  Set a default timeout of
30 seconds.
```rust
fn http_get(url: &str) -> HttpResponse {
    match ureq::get(url).call() {
        Ok(resp) => HttpResponse {
            status: resp.status() as i32,
            body:   resp.into_string().unwrap_or_default(),
        },
        Err(_) => HttpResponse { status: 0, body: String::new() },
    }
}
```

**Step 4 — Content-Type default**:
`http_post` and `http_put` set `Content-Type: application/json` automatically when the
body is non-empty (the common case).  Callers who need a different content type use the
`_h` variants to supply their own `Content-Type` header.

*Tests (run with a local mock server or httpbin.org):*
- `http_get("https://httpbin.org/get").ok()` is `true`.
- `http_get("https://httpbin.org/status/404").status` is `404`.
- `http_post` with a JSON body returns the echoed body from `/post`.
- Network failure (bad URL) returns `HttpResponse { status: 0, body: "" }`.
- Header variants set the supplied headers (verify via httpbin.org `/headers`).

**Effort:** Medium (`ureq` integration + 8 native functions)
**Target:** 0.8.4
**Depends on:** H2 (for the `http` Cargo feature; `ureq` added there)

---

### H5  Nested/array/enum `from_json` and integration tests
**Sources:** [WEB_SERVICES.md](lib_plans/06-web-services/HTTP_CLIENT.md) § Approach B, Phases 3–4
**Description:** Extend the H3 `from_json` synthesiser to handle nested `#json` structs,
`vector<T>` array fields, and plain enum fields.  Add an integration test suite that calls
real HTTP endpoints and verifies the full round-trip.
**Fix path:**

**Step 1 — Nested `#json` struct fields** (`src/typedef.rs`):
For a field `addr: Address` where `Address` is `#json`-annotated, emit:
```loft
addr: Address.from_json(json_nested(body, "addr"))
```
The compiler must verify that `Address` is `#json` at the point of synthesis; if not,
emit: `"field 'addr' has type Address which is not annotated with #json"`.

**Step 2 — `vector<T>` array fields** (`src/typedef.rs`):
For a field `items: vector<Item>` where `Item` is `#json`, emit:
```loft
items: json_items(json_nested(body, "items")).map(Item.from_json)
```
This relies on `map` with fn-refs, which already works.  If `Item` is not `#json`, emit
a compile error.

**Step 3 — Plain enum fields** (`src/typedef.rs`):
For a field `status: Status` where `Status` is a plain (non-struct) enum, emit a `match`
on the string value:
```loft
status: match json_text(body, "status") {
    "Active"   => Status::Active,
    "Inactive" => Status::Inactive,
    _          => Status::Active,   // first variant as default
}
```
The default fallback uses the first variant; a compile-time warning notes it.
Struct-enum variants in JSON (e.g. `{"type": "Paid", "amount": 42}`) are not supported
in this phase — a compile error is emitted if a struct-enum field appears in a `#json` struct.

**Step 4 — `not null` field validation** (`src/typedef.rs`):
Fields declared `not null` whose JSON key is absent should emit a runtime warning (via the
logger) and keep the zero value rather than panicking.  This matches loft's general approach
of never crashing on bad data.

**Step 5 — Integration test suite** (`tests/web/`):
Write loft programs that call public stable APIs and assert on the response.  Tests should
be skipped if the `http` feature is not compiled in or if the network is unavailable:
- `GET https://httpbin.org/json` → parse known struct, assert fields.
- `POST https://httpbin.org/post` with JSON body → assert echoed body round-trips.
- `GET https://httpbin.org/status/500` → `resp.ok()` is `false`, `resp.status` is `500`.
- Nested struct: `GET https://httpbin.org/json` contains a nested `slideshow` object.
- Array field: `GET https://httpbin.org/json` contains a `slides` array.

**Effort:** Medium–High (3 codegen extensions + integration test infrastructure)
**Target:** 0.8.4
**Depends on:** H3, H4

---

## R — Repository

Standalone `loft` repository created (2026-03-16).  The remaining R item is the
workspace split needed before starting the Web IDE.

**Presentation / first-contact work** — the first-run experience, README
truthfulness, doc-site discoverability and repo surface — has its own design in
[FIRST_CONTACT.md](FIRST_CONTACT.md), from an external review carrying the
visitor's perspective (2026-07-24).  Batched B1–B5 there; B1 (first-run bugs)
is the sequencing head.

---

### R1  Add `cdylib` + `rlib` crate types for WASM compilation
**Sources:** WASM_IMPLEMENTATION_PLAN.md § Step 1, W1.1
**Description:** The loft interpreter must be compiled as a `cdylib` (dynamic library) to produce a `.wasm` file via `wasm-bindgen`, and as an `rlib` so the existing native tests and `cargo test` continue to work against the library API.  No workspace split is required for 0.8.3 — the binary targets (`[[bin]] loft`, `[[bin]] gendoc`) are separate compilation units and will not be included in the `cdylib` output.

**Fix path:**

**Step 1 — Add `[lib]` section to `Cargo.toml`:**
```toml
[lib]
name = "loft"
crate-type = ["cdylib", "rlib"]
```
If a `[lib]` section already exists, just add the `crate-type` line.

**Step 2 — Add `src/lib.rs` if not present:**
`src/lib.rs` should already exist and re-export the public API (`pub mod parser`, `pub mod compile`, `pub mod state`, etc.).  Verify it compiles cleanly as a library target with `cargo build --lib`.

**Step 3 — Verify no `main.rs` symbols leak into the `cdylib`:**
`cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` must pass.  Any use of `std::process::exit`, `std::env::args`, or `dirs::home_dir` in `src/lib.rs`-reachable modules must be feature-gated (done in W1.3–W1.6).

**Step 4 — Deferred workspace split (post-1.0):**
A full workspace split into `loft-core / loft-cli / loft-gendoc` reduces incremental build times and isolates CLI from the library API.  This is deferred until the Web IDE (W2+) makes it necessary.  The current single-crate layout is sufficient for 0.8.3.

**Verify:** `cargo check` ✔  `cargo test` ✔  `cargo check --target wasm32-unknown-unknown --features wasm --no-default-features` ✔

**Effort:** Small (one `Cargo.toml` change; no logic changes)
**Depends on:** repo creation (done)
**Target:** 0.8.3

---

## W — Web IDE

A fully serverless, single-origin HTML application that lets users write, run, and
explore Loft programs in a browser without installing anything.  The existing Rust
interpreter is compiled to WebAssembly via a new `wasm` Cargo feature; the IDE shell
is plain ES-module JavaScript with no required build step after the WASM is compiled
once.  Full design in [WEB_IDE.md](lib_plans/62-web-ide/README.md).

---


### W2  Editor Shell
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M2
**Severity/Value:** High — the visible IDE; needed by all later W items
**Description:** A single `index.html` users can open directly (no bundler).
- `ide/src/loft-language.js` — CodeMirror 6 `StreamLanguage` tokenizer: keywords, types, string interpolation `{...}`, line/block comments, numbers
- `ide/src/editor.js` — CodeMirror 6 instance with line numbers, bracket matching, `setDiagnostics()` for gutter icons and underlines
- Layout: toolbar (project switcher + Run button), editor left, Console + Problems panels bottom

JS tests (5): keyword token, string interpolation span, line comment, type names, number literal.
**Effort:** Medium (CodeMirror 6 setup + Loft grammar)
**Depends on:** W1
**Target:** 1.0.0

---

### W3  Symbol Navigation
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M3
**Severity/Value:** Medium — go-to-definition and find-usages; significant IDE quality uplift
**Description:**
- `src/wasm.rs`: implement `get_symbols()` — walk `parser.data.def_names` and variable tables; return `[{name, kind, file, line, col, usages:[{file,line,col}]}]`
- `ide/src/symbols.js`: `buildIndex()`, `findAtPosition()`, `formatUsageList()`
- Editor: Ctrl+click → jump to definition; hover tooltip showing kind + file
- Outline panel (sidebar): lists all functions, structs, enums; clicking navigates

JS tests (3): find function definition, format usage list, no-match returns null.
**Effort:** Medium (Rust symbol walk + JS index)
**Depends on:** W1, W2
**Target:** 1.0.0

---

### W4  Multi-File Projects
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M4
**Severity/Value:** High — essential for any real program; single-file is a toy
**Description:** All projects persist in IndexedDB.  Project schema: `{id, name, modified, files:[{name,content}]}`.
- `ide/src/projects.js` — `listProjects()`, `loadProject(id)`, `saveProject(project)`, `deleteProject(id)`; auto-save on edit (debounced 2 s)
- UI: project-switcher dropdown, "New project" dialog, file-tree panel, tab bar, `use` filename auto-complete

JS tests (4): save/load roundtrip, list all projects, delete removes entry, auto-save updates timestamp.
**Effort:** Medium (IndexedDB wrapper + UI wiring)
**Depends on:** W2
**Target:** 1.0.0

---

### W5  Documentation & Examples Browser
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M5
**Severity/Value:** Medium — documentation access without leaving the IDE; example projects lower barrier to entry
**Description:**
- Build-time script `ide/scripts/bundle-docs.js`: parse `doc/*.html` → `assets/docs-bundle.json` (headings + prose + code blocks)
- `ide/src/docs.js` — renders bundle with substring search
- `ide/src/examples.js` — registers `tests/docs/*.loft` as one-click example projects ("Open as project")
- Right-sidebar tabs: **Docs** | **Examples** | **Outline**

Run the bundler automatically from `build.sh` after `cargo run --bin gendoc`.
**Effort:** Small–Medium (bundler script + panel UI)
**Depends on:** W2
**Target:** 1.0.0

---

### W6  Export, Import & PWA
**Sources:** [WEB_IDE.md](lib_plans/62-web-ide/README.md) — M6
**Severity/Value:** Medium — closes the loop between browser and local development
**Description:**
- `ide/src/export.js`: `exportZip(project)` → `Blob` (JSZip); `importZip(blob)` → project object; drag-and-drop import
- Export ZIP layout: `<name>/src/*.loft`, `<name>/lib/*.loft` (if any), `README.md`, `run.sh`, `run.bat` — matches `loft`'s `use` resolution path so unzip + run works locally
- `ide/sw.js` — service worker pre-caches all IDE assets; offline after first load
- `ide/manifest.json` — PWA manifest
- URL sharing: single-file programs encoded as `#code=<base64>` in URL

JS tests (4): ZIP contains `src/main.loft`, `run.sh` invokes `loft`, import roundtrip preserves content, URL encode/decode.
**Effort:** Small–Medium (JSZip + service worker)
**Depends on:** W4
**Target:** 1.0.0

---
## E — Library Ergonomics

Features motivated by the `server` and `game_client` library designs.
Full design rationale and before/after code examples: [SERVER_FEATURES.md](plans/37-server-features/README.md).

---

### C57  Route decorator syntax

**Problem:** route registration is separated from the handler by potentially
hundreds of lines.  With 30+ routes, finding which URL maps to which handler
requires scrolling between the handler and the `main()` registration block.
Keeping them in sync is error-prone.

```loft
// Today — URL and handler are far apart; easy to get out of sync:
fn handle_health(req: Request) -> Response { ... }
// ... 400 lines ...
fn main() {
    get(app, "/health", fn handle_health);   // must match spelling above
}
```

**Fix:** `@annotation` syntax before `fn` declarations synthesises registration
calls at compile time.

```loft
@get("/health")
fn handle_health(req: Request) -> Response { ... }

@post("/login")
fn handle_login(req: Request) -> Response { ... }

@ws("/ws/chat")
fn handle_chat(req: Request, ws: &WebSocket) { ... }

fn main() {
    app = new_app(srv);
    register_routes(app);   // generated: registers each annotated handler
    serve(app);
}
```

The `server` library defines `@get`, `@post`, `@ws`, etc. as annotations.  At
compile time a synthetic `register_routes(app: &App)` function is generated from
them.  The `app` reference is **not** captured at annotation time — it is passed
at call time, avoiding implicit global state.

**What annotations are NOT:** not runtime metadata, not general macros, not
Turing-complete.  They are fixed templates that apply only to `fn` definitions.

**Implementation:** new token `Token::At`; parser reads `@name(args)` before `fn`;
annotation declarations (`annotation get(pattern: text) expands ...`); synthesis
pass generates `register_routes`; two-pass compiler already supports this pattern.

**Effort:** H — new token, new definition form, annotation registry, synthesis pass.
**Target:** 1.1+

---

