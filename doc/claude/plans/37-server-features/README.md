// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN37 — Language Features for Server and Game Client Libraries

**Status — CLOSED 2026-07-31.**  Three of the five shipped; the two that did not
are kept below because nothing else records their design.

This was never plan-shaped work.  Five independent language features, no phasing
and no cross-arc dependency, share nothing but the evaluation that surfaced them —
a LIST, which a plan directory adds nothing to.  What decides whether the
remaining two are worth building is a real consumer asking for them, and the web
and game consumers live in their own repos; a loft-side plan cannot hold that
signal, so holding it open only aged the shipped half into a false "future".

## What shipped

| ID | Feature | Where it is documented |
|----|---------|------------------------|
| C55 | Type aliases (`type Handler = fn(Request) -> Response;`) | [LOFT_DECLARATIONS.md § Types and type aliases](../../LOFT_DECLARATIONS.md) |
| C56 | `?? return` null early-exit | [LOFT.md § Null handling](../../LOFT.md) |
| A15 | `parallel { }` blocks | [LOFT_CONTROL.md § Parallel blocks](../../LOFT_CONTROL.md), [THREADING.md](../../THREADING.md) |

A15 ships its SYNTAX; the arms currently run sequentially, so the concurrency the
game-server pattern wanted is not delivered.  That residue belongs with threading,
not here — see THREADING.md.

## What did not, and what would revive it

Neither is blocked on design; both are blocked on demand.  **The trigger for each
is a consumer asking**, which now arrives as feedback from the repos that build on
loft rather than from this plan.

* **I13 — iterator protocol.**  `for x in <custom type>` still refuses ("cannot
  iterate over T"); the design is below.  Depends on method dispatch on reference
  types, and is worth more with interface enforcement (@PLN-interfaces).
* **C57 — route decorators.**  `@get("/health")` still refuses; the design is
  below.  The plan itself judged this "the most complex and the most easily
  replaced by clear documentation" — putting registration beside the handler in a
  comment costs nothing and needs no language change.

---

## I13 — Iterator protocol

**Motivation:** a WebSocket receive loop must be written as an explicit
infinite `for` loop with a mutable flag because loft's `for` only supports
built-in collections.

```loft
// Today — awkward infinite loop with mutable exit flag:
running = true;
for _ in 0..1000000 if running {
    msg = ws_receive(ws);
    match msg {
        Close { _, _ } => running = false,
        Text { content } => process(content),
        _ => {},
    }
}
```

The same pattern appears in the game client's event loop, state sync loop, and
ping tracker.

**Feature:** any type that defines `fn next(self: &Self) -> T?` can be used
directly in a `for` loop.  Returning `null` from `next` terminates the loop.

```loft
// WebSocket implements the iterator protocol (in websocket.loft):
pub fn next(self: &WebSocket) -> WsMessage? {
    msg = ws_receive(self);
    match msg {
        Close { _, _ } => null,
        other          => other,
    }
}

// Handler becomes clean and idiomatic:
for msg in ws {
    match msg {
        Text { content } => process(content),
        _ => {},
    }
}
```

### Formal definition

```loft
// The iterator protocol — not a formal interface yet (deferred to I5+),
// but a structural convention recognised by the for-loop desugaring:
//   if T has fn next(self: &T) -> Item?
//   then  for x in val : T  is valid

// Desugaring:
for x in val {
    body
}
// becomes:
for _ in 0..2147483647 {
    x = val.next();
    if x == null { break; }
    body
}
```

When the full interface system (I5+) lands, `Iterator<T>` becomes a formal
interface and the protocol is enforced at compile time.  Before that, the
for-loop desugaring applies to any type that structurally has a matching `next`
method — duck-typing at the parser level.

### Types that benefit from the iterator protocol

| Type | Item | Loop pattern |
|------|------|-------------|
| `WebSocket` | `WsMessage` | Receive loop until Close |
| `WsClient` (game_client) | `WsMessage` | Client receive loop |
| `ConnectionRegistry` | `(text, WebSocket)` | Iterate all connections |
| User-defined producers | any | Custom iterators |

### Loop attributes with custom iterators

`#count` works as usual (counts completed iterations).
`#first` works as usual (true on the first call to `next()`).
`#index` and `#remove` are not available for custom iterators.

### Implementation

1. **Parser (collections.rs):** in `parse_for()`, after resolving the iterable
   type, check if it has a method `next` returning `T?`.  If so, construct the
   desugared `Value::For` block rather than emitting a collection iterator.
2. **Codegen:** desugaring is entirely at the parser/IR level — no new opcodes
   needed.  The generated IR is a standard `for _ in 0..MAX` with a
   `ws.next()` call and a null-break.
3. **Type checking:** the inferred type of the loop variable is `T` (the inner
   type of `T?` returned by `next`).

**Tests:** `for msg in ws` on a mock WebSocket that returns three messages then
null; verify 3 iterations; verify the `break` fires on null.

**Effort:** MH — parser change in `parse_for()` + type resolution for method
lookup; no new opcodes; requires method dispatch to work on reference types.
Formally depends on I5+ for enforcement; structurally can land without it.

---

---

## C57 — Route decorator syntax

**Motivation:** in a real application, route registrations appear far from the
handler functions they register.  With 20–40 routes, finding the URL for a
handler requires scrolling to the `main()` registration block.

```loft
// Today — handler and its URL are in different places:
fn handle_health(req: Request) -> Response { response_ok("ok") }
fn handle_login(req: Request) -> Response { ... }
fn handle_user(req: Request) -> Response { ... }
// ... 300 lines ...
fn main() {
    app = new_app(srv);
    get(app,  "/health",  fn handle_health);
    post(app, "/login",   fn handle_login);
    get(app,  "/users",   fn handle_user);
    // easy to get out of sync
}
```

**Feature:** `@annotation` syntax before function definitions.  An annotation
is a compile-time registration call synthesised by the library that defines the
annotation.

```loft
@get("/health")
fn handle_health(req: Request) -> Response { response_ok("ok") }

@post("/login")
fn handle_login(req: Request) -> Response { ... }

@ws("/ws/chat")
fn handle_chat(req: Request, ws: &WebSocket) { ... }

fn main() {
    app = new_app(Server { port: 8080 });
    register_routes(app);   // generated: calls get/post/ws for each annotation
    serve(app);
}
```

### Annotation expansion model

The `server` library declares annotations:

```loft
// In server.loft:
annotation get(pattern: text)       // expands to: get(app, pattern, fn decorated)
annotation post(pattern: text)      // expands to: post(app, pattern, fn decorated)
annotation put(pattern: text)
annotation delete(pattern: text)
annotation ws(pattern: text)        // expands to: route_ws(app, pattern, fn decorated)
```

At compile time:
1. The parser collects all `@get(...)`, `@post(...)`, etc. annotations.
2. A synthetic `register_routes(app: &App)` function is generated containing
   one registration call per annotation, in declaration order.
3. The user calls `register_routes(app)` in `main()`.

The `app` variable is **not** captured at annotation time — it is passed to
`register_routes` when the user calls it.  This avoids introducing
implicit global state.

### Annotation definition syntax

```loft
// Define an annotation named 'get' with one parameter 'pattern':
annotation get(pattern: text)
    expands fn(handler: fn(Request) -> Response) {
        get(app, pattern, fn handler)
    }
```

The `app` name in the expansion is a well-known identifier resolved from the
`register_routes` call site — similar to how Rust procedural macros emit code
that resolves names at the call site.

### What annotations are NOT

- Not runtime metadata — annotations have no runtime representation.
- Not general macros — they apply only to function definitions, not expressions.
- Not Turing-complete — the expansion body is a fixed template; no conditional
  logic or loops are allowed in annotation expansion.

### Why this cannot be done with closures alone (A5)

Even when A5 (closure capture) lands, the co-location problem remains: the
route URL and handler would still be registered in `main()`.  Decorators co-
locate the URL directly with the handler declaration, which is a different
ergonomic property than closures provide.

### Implementation

1. **Lexer:** `@` as a new token `Token::At`.
2. **Parser (definitions.rs):** before parsing a `fn`, check for one or more
   `@name(args)` annotations.  Each is stored as `Annotation { name, args }`
   on the `FnDef`.
3. **Annotation registry:** the compiler maintains a table of declared
   annotations (from `annotation name(params) expands ...` declarations).
   These are collected in the first pass.
4. **Synthesis pass:** after parsing all definitions, generate the
   `register_routes` function body by iterating annotated functions and
   expanding each annotation's template.
5. **Error reporting:** unknown annotation name → compile error.  Mismatched
   parameter count → compile error.  Annotation on a non-function → compile
   error.

**Tests:** two annotated handlers; `register_routes(app)` calls both; verify
routing; test error cases (unknown annotation, wrong parameter type).

**Effort:** H — new token, new definition form, annotation registry, synthesis
pass; requires the two-pass parser infrastructure already present.

---
