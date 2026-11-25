// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debug channels: who may drive a session

A debug session evaluates code and edits frames with the rights of the user who started it.
This doc says which channels reach one and what a client must show to use them.  Part of the
debugging guide: [DEBUG.md](DEBUG.md).  The decision is `@C172`
([DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md#c172--a-debug-session-answers-only-whoever-holds-its-runs-token)).

---

## The rule

Every channel that listens on a port answers only a client that names the run's **token**.
A loopback port is reachable by every process on the machine, whatever account runs it, so
the address alone proves nothing; the token does.

- **What it is.**  128 bits from the operating system's random source, drawn once per
  process (`src/debug_token.rs`; the C engine's twin is `loft_eh_debug_token` in
  `src/c/loft_engine.h`), written as 32 hexadecimal digits.  A host whose random source fails
  opens no channel at all.
- **Where it goes.**  In the URL of the opening request, as `?t=<token>`:
  `ws://127.0.0.1:<port>/ws?t=<token>`, and `http://127.0.0.1:<port>/?t=<token>` for the
  `--serve` page.  A browser sets a WebSocket's URL but none of its headers, so the URL is the
  one place every client can put it.  It is compared in constant time.
- **How a client learns it.**  The process prints it on standard output, on the line that
  gives its port, which only the program that started it reads:

  | Process | Line |
  |---|---|
  | `loft debug prog.loft --serve` | `loft debug --serve: open http://127.0.0.1:<port>/?t=<token> in a browser` (stderr) |
  | a game's control listener | `engine_host: debug control on 127.0.0.1:<port> token <token>` |
  | a game server's port | `engine_host: debug token <token>` |

- **How it is handed on.**  A process that starts another one to debug passes its own token in
  `LOFT_DEBUG_TOKEN`: `--serve` to the game it launches, a game to the build it swaps in.  One
  session keeps one token, so the `--serve` page dials the game with the token it was opened
  with, and an agent keeps its connection across a swap.

## The channels

| Channel | Transport | Needs the token |
|---|---|---|
| `loft debug --serve` page and socket | HTTP + WebSocket on `127.0.0.1` | yes: the page and the socket |
| a game's control listener (`LOFT_DEBUG_CONTROL=1`) | WebSocket on `127.0.0.1` | yes: the upgrade is refused without it |
| agent commands (`D!:`) on a game server's port | the game's WebSocket | yes: a peer without it plays the game, and its `D!:` frames go unanswered |
| `loft debug --rpc`, the DAP adapter (`loft-dap`) | standard input and output | no: only the parent holds the pipe |
| a `--html --debug` wasm page | the page's own input queue | no: it opens no listener; frames reach it from the page or relayed by the server it joined |

The checks a channel already had stay: `--serve` answers only a request addressed to its own
loopback name and port, and every channel refuses an upgrade from a page of another site.

## Switches

- `LOFT_DEBUG_CONTROL=1` — a game opens its debug channels: the control listener, and agent
  commands on its game port.  `--serve` sets it for the game it launches.
- `LOFT_DEBUG_TOKEN=<hex>` — the token this process answers, when it holds 32 to 128
  hexadecimal digits and nothing else; anything else is ignored and a token is drawn.  Set by a
  process that starts another one to debug, or by a harness that wants to know the token in
  advance (`tests/engine_host_kernel.rs` does).

## Guards

`src/debug_token.rs` (only the exact token admits); `tests/serve.rs`
`serve_refuses_another_sites_page` (the page and the socket refuse no token and another one);
`tests/engine_host_kernel.rs` `s5_client_swap_under_running_world` and
`s5_c_client_swap_under_running_world` (the control listener, both backends; the swapped-in
build answers the same token) and `an_agent_command_on_the_game_port_needs_the_runs_token`
(the game port, both backends).
