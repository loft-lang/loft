<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft in Neovim — install & keystrokes (LSP + debugging)

Everything `loft-lsp` and `loft-dap` provide, and how to reach it from Neovim. The LSP
side is **dependency-free** (built-in `vim.lsp`); debugging needs **`nvim-dap`**.

## Feature coverage — is it all possible in Neovim?

**Yes.** Every `loft-lsp` capability and every `loft-dap` capability maps to a Neovim
mechanism. The LSP rows were verified live (a headless Neovim attaching `loft-lsp` to a
`.loft` file and round-tripping the requests); the DAP rows map advertised `loft-dap`
capabilities to `nvim-dap` API.

### Code intelligence (`loft-lsp` → built-in `vim.lsp`)

| Feature | Server capability | Neovim | Default key | Status |
|---|---|---|---|---|
| Diagnostics | publishDiagnostics | `vim.diagnostic` | `]d` / `[d`, `<C-w>d` | ✅ built-in |
| Hover (sig + `///` doc; on `??`, `match`, `v[i]`, `+=` … the feature that documents it; on a library's item or `use` line, the library's card; or a tracker-tag card) | hoverProvider | `vim.lsp.buf.hover` | `K` | ✅ |
| Go to definition | definitionProvider | `vim.lsp.buf.definition` | `gd` | ✅ |
| Find references | referencesProvider | `vim.lsp.buf.references` | `grr` | ✅ |
| Rename (locals + cross-file; refuses stdlib) | renameProvider + prepare | `vim.lsp.buf.rename` | `grn` | ✅ |
| Code actions (quick-fix, `#superseded` steer) | codeActionProvider | `vim.lsp.buf.code_action` | `gra` | ✅ |
| **Extract function** | codeActionProvider (`refactor.extract`) | `code_action` on a **visual** selection | `gra` (visual) | ✅ verified (`Extract to function`) |
| **What can I use here?** — every language feature, every library, and what the type under the cursor can do, as linked Markdown pages | codeActionProvider (`source.loft.overview`) + executeCommand `loft.overview` → `window/showDocument` | `code_action` anywhere; on a type name for its own page | `gra` | ✅ verified (Neovim 0.11: opens `index.md`, and `type-<T>.md` on a type) |
| Outline / document symbols | documentSymbolProvider | `vim.lsp.buf.document_symbol` | `gO` | ✅ |
| Format (`loft fmt`) | documentFormattingProvider | `vim.lsp.buf.format` | `<leader>f` | ✅ |
| Completion (each item documented when shown: a keyword's feature, a name's signature and doc) | completionProvider + `completionItem/resolve` | omni-complete / autotrigger / `nvim-cmp` | `<C-x><C-o>` | ✅ (nicer with a cmp plugin) |
| Semantic highlighting | semanticTokensProvider | automatic | — | ✅ (Neovim 0.9+) |
| Inlay hints (inferred types) | inlayHintProvider | `vim.lsp.inlay_hint` | auto-on | ✅ **Neovim 0.10+** only |
| Tag links (`@PLN63` → issue) | documentLinkProvider | — | — | ⚠️ advertised, but Neovim has no built-in document-link UI — hover (`K`) shows the tag card; a clickable link needs a small handler (below) |
| Signature help | — | — | — | n/a — `loft-lsp` does not offer it yet |

### Debugging (`loft-dap` → `nvim-dap`)

| Feature | Server capability | `nvim-dap` | Suggested key | Status |
|---|---|---|---|---|
| Launch / continue | launch, continue | `dap.continue()` | `<F5>` | ✅ |
| Breakpoint | setBreakpoints | `dap.toggle_breakpoint()` | `<F9>` / `<leader>db` | ✅ |
| Conditional breakpoint | supportsConditionalBreakpoints | `dap.set_breakpoint(cond)` | `<leader>dB` | ✅ |
| Step over / into / out | next / stepIn / stepOut | `dap.step_over/into/out()` | `<F10>` / `<F11>` / `<S-F11>` | ✅ |
| Stop on entry | launch `stopOnEntry` | config flag | (config) | ✅ |
| Evaluate expression | supportsEvaluateForHovers | `dap.eval()` / dap-ui hover | `<leader>de` | ✅ |
| Set a variable's value | supportsSetVariable | edit in dap-ui **Scopes** | (dap-ui) | ✅ |
| Variables + **struct/vector expansion** (VE) | variables tree | dap-ui **Scopes** | (dap-ui) | ✅ |
| **Multi-frame call stack** (SF) | stackTrace | dap-ui **Stacks** | (dap-ui) | ✅ |
| **Data breakpoints** (watch a variable) (DB) | supportsDataBreakpoints | session request (helper below) | `<leader>dw` | ✅ via a 6-line helper (no first-class dap command) |
| **Reverse execution** (RX) | supportsStepBack | `dap.step_back()` / `dap.reverse_continue()` | `<F12>` / `<leader>dR` | ✅ |
| Terminate | supportsTerminateRequest | `dap.terminate()` | `<leader>dt` | ✅ |

Not applicable (by design — `loft-dap` doesn't advertise them, so no client offers a control
it can't honour): `pause` (no async interrupt) and hit-count conditional breakpoints.

## Install

**Neovim 0.10+ recommended** (0.9 works for everything except inlay hints).

### 1 · Build the binaries

```sh
cargo build --release --bin loft-lsp --bin loft-dap    # in the loft repo
# put them on $PATH (or pass absolute paths to setup()):
ln -s "$PWD/target/release/loft-lsp" ~/.local/bin/loft-lsp
ln -s "$PWD/target/release/loft-dap" ~/.local/bin/loft-dap
```

### 2 · Plugins

- **LSP only:** none — `require('loft').setup()` uses built-in `vim.lsp`.
- **Debugging:** [`mfussenegger/nvim-dap`](https://github.com/mfussenegger/nvim-dap), and for
  the variables/stack/REPL UI [`rcarriga/nvim-dap-ui`](https://github.com/rcarriga/nvim-dap-ui)
  (+ its `nvim-nio` dep). Optional: `nvim-cmp` for popup completion.

### 3 · A complete `init.lua` (lazy.nvim)

```lua
require("lazy").setup({
  -- The loft plugin (LSP wiring + syntax + ftdetect). Point `dir` at your checkout.
  { dir = "/abs/path/to/loft/editors/nvim", name = "loft",
    config = function() require("loft").setup() end },   -- add { loft_lsp = "…" } if not on $PATH

  -- Debugging
  { "mfussenegger/nvim-dap" },
  { "rcarriga/nvim-dap-ui", dependencies = { "nvim-neotest/nvim-nio" },
    config = function()
      local dap, dapui = require("dap"), require("dapui")
      dapui.setup()
      dap.listeners.after.event_initialized["dapui"] = function() dapui.open() end
      dap.listeners.before.event_terminated["dapui"] = function() dapui.close() end
      dap.listeners.before.event_exited["dapui"]     = function() dapui.close() end
    end },
})

-- `require('loft').setup()` already registered `dap.adapters.loft` + a "Run current file"
-- launch config, so these keymaps work on any .loft buffer with no further config:
local dap = require("dap")
vim.keymap.set("n", "<F5>",     dap.continue,          { desc = "debug: launch/continue" })
vim.keymap.set("n", "<F10>",    dap.step_over,         { desc = "debug: step over" })
vim.keymap.set("n", "<F11>",    dap.step_into,         { desc = "debug: step into" })
vim.keymap.set("n", "<S-F11>",  dap.step_out,          { desc = "debug: step out" })
vim.keymap.set("n", "<F12>",    dap.step_back,         { desc = "debug: STEP BACK (reverse)" })
vim.keymap.set("n", "<F9>",     dap.toggle_breakpoint, { desc = "debug: toggle breakpoint" })
vim.keymap.set("n", "<leader>dB", function()
  dap.set_breakpoint(vim.fn.input("Breakpoint condition: "))
end, { desc = "debug: conditional breakpoint" })
vim.keymap.set("n", "<leader>dR", dap.reverse_continue, { desc = "debug: reverse-continue to start" })
vim.keymap.set("n", "<leader>de", function() require("dapui").eval() end, { desc = "debug: evaluate" })
vim.keymap.set("n", "<leader>dr", dap.repl.toggle,     { desc = "debug: REPL" })
vim.keymap.set("n", "<leader>dt", dap.terminate,       { desc = "debug: terminate" })
vim.keymap.set("n", "<leader>du", function() require("dapui").toggle() end, { desc = "debug: toggle UI" })

-- Data breakpoint (DB): watch the variable under the cursor at a stop.  nvim-dap has no
-- first-class command, so ask loft-dap directly via the session request.
vim.keymap.set("n", "<leader>dw", function()
  local session = dap.session()
  if not session then return vim.notify("no active debug session", vim.log.levels.WARN) end
  local name = vim.fn.expand("<cword>")
  session:request("dataBreakpointInfo", { name = name, variablesReference = 0 }, function(err, info)
    if err or not info or not info.dataId then
      return vim.notify("cannot watch `" .. name .. "`", vim.log.levels.WARN)
    end
    session:request("setDataBreakpoints", { breakpoints = { { dataId = info.dataId } } }, function()
      vim.notify("watching `" .. name .. "` — continue to break on change")
    end)
  end)
end, { desc = "debug: data breakpoint on <cword>" })
```

> The LSP keymaps (`gd`, `K`, `grn`, `grr`, `gra`, `gO`, `<leader>f`) are set **for you** on
> attach by `loft.setup()` — you only bind the debug keys above.

## Using it

- **Open a `.loft` file** — `loft-lsp` attaches automatically (`:LspInfo` to confirm). Coloring,
  diagnostics, hover, completion, etc. are live.
- **Extract a function** — visually select whole statement lines (`V`, `j`…), then `gra` and
  pick **Extract to function**. (A single-line / bare-cursor selection offers nothing — extract
  needs a multi-line statement slice.)
- **Debug** — `<F5>` on a `.loft` file runs it under `loft-dap` ("Run current file"); set
  breakpoints with `<F9>`; the dap-ui panels show scopes (expand structs/vectors), the call
  stack, and the REPL. `<F12>` steps **backward**; `<leader>dw` watches a variable.

## Notes & honest limits

- **Inlay hints** need Neovim **0.10+** (`vim.lsp.inlay_hint`); on 0.9 everything else works.
- **Tag links** (`@PLN63` → the tracker issue): `loft-lsp` publishes them, but Neovim has no
  built-in UI for `textDocument/documentLink`. Hover (`K`) shows the tag card; for clickable
  navigation add a `documentLink` handler or a plugin. Not a blocker.
- **Reverse execution** is interpreter-mode and does **not** reverse I/O (a `print` stays in
  the console); depth is bounded (`LOFT_REVERSE_DEPTH`, default 200). **Data breakpoints** are
  set at a stop and watch a scalar local (or a nested field). Both are documented in
  [DAP_ADVANCED.md](../../doc/claude/lib_plans/63-lsp/DAP_ADVANCED.md).
- Completion works through omni-complete out of the box; a completion plugin (`nvim-cmp`,
  `blink.cmp`) gives popup-as-you-type over the same server.
