// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I83 — Dev server (--serve / --rpc)

//! @PLN16 M5e slice 1 — the **`--serve` foundation**: a local HTTP + WebSocket server
//! that puts the debugger's wire protocol ([`crate::rpc`]) in a browser.  `GET /` returns
//! a minimal shell (the file's source + a **Run** button + a **Program** console); the
//! shell opens a WebSocket and drives the *same* protocol the `--rpc` stdio server does —
//! one JSON request per text frame, one engine method per request ([`crate::rpc::handle`]).
//! Program `print` rides the same capture sink as `--rpc` ([`crate::rpc::print_or_capture`])
//! and streams back as `output` events, so it never collides with the protocol traffic.
//!
//! The transport is the only new thing here: the engine, the protocol, and the message
//! set are unchanged (PROTOCOL.md's invariant — "one protocol, many transports").  Later
//! M5e slices add panels (compiler console, editor, gutter breakpoints) on the same shell
//! and messages; the live game loop (slice 6) is why this is a WebSocket, not request/
//! response — it needs bidirectional server-push a `POST` round-trip can't give.
//!
//! Scope (slice 1): one browser, one debug session, served single-threaded (each
//! connection handled to completion).  Bound to `127.0.0.1` — the data and the engine stay
//! on the machine, reached remotely only via an explicit SSH port-forward (the `make view`
//! shape).

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use crate::repl::ReplSession;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

/// Run the browser debug server on `127.0.0.1:port` for `file`, until the process is
/// killed.  The shell shows `file`'s source and `launch`es it; **Run** executes it and
/// streams its output.
///
/// # Errors
/// Returns an I/O error if the port can't be bound.
pub fn run_serve(
    stdlib_dir: &str,
    lib_dirs: &[String],
    port: u16,
    file: &str,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let source = std::fs::read_to_string(file).unwrap_or_default();
    let shell = render_shell(file, &source);
    // One session for the lifetime of the server (slice 1 is single-session); program
    // output is captured into `output` events, not printed, exactly as `--rpc` does.
    // `lib_dirs` are the `--lib` import paths so the served file can `use` libraries.
    let mut session = ReplSession::new_with_libs(stdlib_dir, lib_dirs)?;
    session.debug_stepping(true);
    session.set_workspace_file(file); // slice 3 — the editor may save back to this one file
    // Program output is captured into `output` events rather than printed (the same sink
    // `--rpc` uses).  Unlike `--rpc` we do NOT silence the panic hook: the protocol rides
    // the WebSocket, not stdout, so a fault printing to stderr is useful server-side
    // logging, not stream corruption — and the per-request `catch_unwind` already turns it
    // into a `terminated` event for the browser.  (A global hook would also leak across the
    // process, e.g. hiding panic messages in a parallel test run.)
    crate::rpc::capture_begin();
    eprintln!("loft debug --serve: open http://127.0.0.1:{port}/ in a browser");
    for conn in crate::net_profile::accepting(&listener, "serve/accept") {
        let Ok(stream) = conn else { continue };
        // A bad connection (malformed request, abrupt close) must not take the server
        // down — drop it and keep serving.
        let _ = serve_connection(&stream, &mut session, &shell);
    }
    crate::rpc::capture_end();
    Ok(())
}

/// Handle one TCP connection: read the request head, then either serve the shell (`GET /`),
/// run the WebSocket protocol loop (an `Upgrade: websocket` request), or 404.
fn serve_connection(
    stream: &TcpStream,
    session: &mut ReplSession,
    shell: &str,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    crate::net_profile::time("serve/request_line", None, || {
        reader.read_line(&mut request_line)
    })?;
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let mut ws_key = None;
    let mut is_upgrade = false;
    loop {
        let mut line = String::new();
        if crate::net_profile::time("serve/header_line", None, || reader.read_line(&mut line))? == 0
        {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break; // end of headers
        }
        if let Some((k, v)) = line.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "upgrade" => is_upgrade = v.trim().eq_ignore_ascii_case("websocket"),
                "sec-websocket-key" => ws_key = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    if is_upgrade && let Some(key) = ws_key {
        let accept = ws_accept_key(&key);
        let resp = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        );
        let mut s = stream;
        crate::net_profile::time("serve/upgrade_write", None, || s.write_all(resp.as_bytes()))?;
        ws_protocol_loop(reader, stream, session)?;
    } else if path == "/" {
        http_respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            shell.as_bytes(),
        )?;
    } else {
        http_respond(stream, "404 Not Found", "text/plain", b"not found")?;
    }
    Ok(())
}

/// Write a minimal HTTP/1.1 response and close (no keep-alive — slice 1 serves the shell
/// once, then the WebSocket carries everything).
fn http_respond(stream: &TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    let mut s = stream;
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    Ok(())
}

/// The WebSocket message loop: read one text frame (a JSON request), dispatch it through
/// the shared protocol driver, write each resulting message back as its own text frame.
/// Mirrors `rpc::run_rpc`'s NDJSON loop — same `handle()`, different framing.
fn ws_protocol_loop(
    mut reader: BufReader<&TcpStream>,
    stream: &TcpStream,
    session: &mut ReplSession,
) -> std::io::Result<()> {
    while let Some((opcode, payload)) = ws_read_frame(&mut reader)? {
        match opcode {
            0x1 => {
                // text frame — a JSON request.  Isolate a panic so one bad request can't
                // kill the connection (the engine state is the slice-1 single session).
                let line = String::from_utf8_lossy(&payload).into_owned();
                let (messages, disconnect) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        crate::rpc::handle(session, &line)
                    }))
                    .unwrap_or_else(|_| (vec!["{\"event\":\"terminated\"}".to_string()], false));
                for m in messages {
                    ws_write_text(stream, &m)?;
                }
                if disconnect {
                    break;
                }
            }
            0x8 => break,                            // close
            0x9 => ws_write_pong(stream, &payload)?, // ping → pong
            _ => {} // ignore binary / continuation / pong in slice 1
        }
    }
    Ok(())
}

// ── WebSocket framing (RFC 6455, the subset slice 1 needs) ───────────────────────────

/// Read one WebSocket frame from a client (always masked).  Returns `(opcode, payload)`,
/// or `None` on EOF.  Handles the 7-bit, 16-bit, and 64-bit length forms.
fn ws_read_frame(reader: &mut impl BufRead) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 2];
    if read_exact_or_eof(reader, &mut head)? {
        return Ok(None);
    }
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b)?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            reader.read_exact(&mut b)?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; usize::try_from(len).unwrap_or(0)];
    reader.read_exact(&mut payload)?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Some((opcode, payload)))
}

/// True when EOF was hit before filling `buf` (a clean client disconnect).
fn read_exact_or_eof(reader: &mut impl BufRead, buf: &mut [u8]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match crate::net_profile::time("serve/ws_read", None, || reader.read(&mut buf[filled..]))? {
            0 => return Ok(true),
            n => filled += n,
        }
    }
    Ok(false)
}

/// Write a server text frame (FIN, opcode 1, unmasked — server frames are never masked).
fn ws_write_text(stream: &TcpStream, text: &str) -> std::io::Result<()> {
    let mut frame = vec![0x81u8]; // FIN + text
    write_frame_len(&mut frame, text.len());
    frame.extend_from_slice(text.as_bytes());
    let mut s = stream;
    crate::net_profile::time("serve/ws_write", None, || s.write_all(&frame))
}

/// Reply to a ping with a pong carrying the same payload (FIN, opcode 0xA, unmasked).
fn ws_write_pong(stream: &TcpStream, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = vec![0x8Au8];
    write_frame_len(&mut frame, payload.len());
    frame.extend_from_slice(payload);
    let mut s = stream;
    s.write_all(&frame)
}

/// Append a server frame's unmasked length (the 7 / 16 / 64-bit forms).
fn write_frame_len(frame: &mut Vec<u8>, len: usize) {
    if len <= 125 {
        frame.push(len as u8);
    } else if len <= 0xFFFF {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
}

/// Compute the `Sec-WebSocket-Accept` value: base64(sha1(key + the RFC 6455 GUID)).
pub(crate) fn ws_accept_key(key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut input = key.as_bytes().to_vec();
    input.extend_from_slice(GUID.as_bytes());
    crate::base64::encode(&sha1(&input))
}

// ── SHA-1 (inline; the WS handshake's one fixed-length hash — not a security boundary) ──

/// SHA-1 of `data` (RFC 3174).  Inlined rather than pulling a crate for the single
/// handshake hash; covered by the `abc` + RFC 6455 test vectors below.  The single-letter
/// working variables (`a`..`e`, `f`, `k`, `w`, `h`) are the algorithm's own names.
#[allow(clippy::many_single_char_names)]
pub(crate) fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            let o = i * 4;
            *word = u32::from_be_bytes([chunk[o], chunk[o + 1], chunk[o + 2], chunk[o + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

// ── the browser shell (slices 1-3: editable source + gutter, Run/Save, dual console) ──

/// Build the IDE shell for `file`: its `source` in an **editable** pane (a `<textarea>` with
/// a synced line-number gutter), Run + Save buttons, and the **dual console** — a Compiler
/// pane (diagnostics) over a Program pane (stdout).  The page opens a WebSocket, `launch`es +
/// `compile`s the file on connect, marks diagnostic lines in the gutter, and on **Save**
/// writes the buffer back (sandboxed `writeFile`) / on **Run** saves, reloads, and streams
/// the program's output.  Built by token-replacement (not `format!`) so the CSS/JS braces
/// stay literal; the source is HTML-escaped into the textarea body.
fn render_shell(file: &str, source: &str) -> String {
    SHELL_TEMPLATE
        .replace("__TITLE__", &html_escape(file))
        .replace("__SOURCE__", &html_escape(source))
        .replace("__FILE_JS__", &js_string(file))
}

const SHELL_TEMPLATE: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<link rel="icon" href="data:,"><title>loft · __TITLE__</title>
<style>
  :root { color-scheme: light dark; }
  body { margin:0; font:14px/1.5 system-ui,sans-serif; display:flex; flex-direction:column; height:100vh; }
  #bar { padding:8px 12px; border-bottom:1px solid #8884; display:flex; gap:12px; align-items:center; flex:none; }
  #bar button { font:inherit; padding:4px 12px; cursor:pointer; }
  #bar button:disabled { opacity:.4; cursor:default; }
  #dirty { color:#f9a825; font-weight:600; font-size:12px; }
  #status { opacity:.7; }
  #main { flex:1; min-height:0; display:grid; grid-template-columns:1fr 1fr; grid-template-rows:1fr 1fr; }
  #editor { grid-row:1 / 3; border-right:1px solid #8884; display:flex; overflow:hidden; font:13px/1.45 ui-monospace,monospace; }
  #gutter { flex:none; box-sizing:border-box; width:3.5em; padding:8px .7em 8px 0; text-align:right; opacity:.45; user-select:none; overflow:hidden; }
  #gutter .gnum { white-space:pre; cursor:pointer; border-radius:8px; padding:0 .2em; }
  #gutter .gnum:hover { background:#8882; }
  #gutter .gnum.error { color:#e53935; opacity:1; font-weight:700; }
  #gutter .gnum.warning { color:#b8860b; opacity:1; font-weight:700; }
  #gutter .gnum.bp { background:#e53935; color:#fff; opacity:1; }
  #gutter .gnum.cur { background:#ffd60055; opacity:1; }
  #gutter .gnum.bp.cur { box-shadow:inset 0 0 0 2px #ffd600; }
  #src { flex:1; box-sizing:border-box; border:none; outline:none; resize:none; margin:0; padding:8px 12px; font:inherit; line-height:1.45; background:transparent; color:inherit; white-space:pre; overflow:auto; tab-size:4; }
  .pane { display:flex; flex-direction:column; min-height:0; }
  .pane h2 { margin:0; padding:6px 12px; font-size:12px; opacity:.6; border-bottom:1px solid #8884; text-transform:uppercase; letter-spacing:.05em; }
  .body { flex:1; overflow:auto; margin:0; padding:8px 12px; font:13px/1.45 ui-monospace,monospace; white-space:pre-wrap; }
  #diags .d { cursor:pointer; padding:2px 0; }
  #diags .d.error { color:#e53935; } #diags .d.warning { color:#b8860b; }
  #diags .d .loc { opacity:.55; margin-right:.6em; }
  #diags .none { opacity:.45; }
  #steps { display:flex; gap:4px; }
  #bottom { flex:none; height:34%; border-top:2px solid #8886; display:flex; min-height:0; }
  #dbg { flex:none; width:42%; border-right:1px solid #8884; display:flex; flex-direction:column; min-height:0; overflow:hidden; }
  #dbg[hidden] { display:none; }
  #dbg .pane { flex:1; min-height:0; }
  #vars .var, #watches .watch { display:flex; gap:.6em; padding:1px 0; align-items:baseline; }
  #vars .vn, #watches .wx { color:#1976d2; }
  #vars .vv { cursor:pointer; } #vars .vv:hover { text-decoration:underline dotted; }
  #vars .vedit { font:inherit; width:55%; }
  #watches .wv { margin-left:auto; opacity:.85; } #watches .wrm { cursor:pointer; opacity:.5; }
  #vars .none, #watches .none { opacity:.45; }
  #waddin { flex:none; border:none; border-top:1px solid #8884; padding:6px 12px; font:13px/1.45 ui-monospace,monospace; background:transparent; color:inherit; outline:none; }
  #tests { flex:none; width:42%; border-right:1px solid #8884; display:flex; flex-direction:column; min-height:0; overflow:hidden; }
  #tests[hidden] { display:none; }
  #tests .pane { flex:1; min-height:0; }
  #tsum.ok { color:#2e7d32; } #tsum.bad { color:#e53935; }
  #tlist .t { display:flex; gap:.6em; padding:2px 0; align-items:baseline; }
  #tlist .t.clickable { cursor:pointer; } #tlist .t.clickable:hover { text-decoration:underline dotted; }
  #tlist .t .ti { width:1em; flex:none; } #tlist .t.pass .ti { color:#2e7d32; } #tlist .t.fail .ti { color:#e53935; }
  #tlist .t .tn { color:#1976d2; } #tlist .t .tm { opacity:.75; margin-left:auto; padding-left:1em; }
  #tlist .none { opacity:.45; }
  #repl { flex:1; display:flex; flex-direction:column; min-height:0; }
  #replhead { padding:6px 12px; font-size:12px; opacity:.7; border-bottom:1px solid #8884; display:flex; gap:10px; align-items:center; text-transform:uppercase; letter-spacing:.05em; }
  #replctx { font-weight:600; padding:1px 8px; border-radius:9px; text-transform:none; letter-spacing:0; }
  #replctx.top { background:#2196f333; color:#1976d2; }
  #replctx.frame { background:#ab47bc33; color:#8e24aa; }
  #replhead .hint { opacity:.5; margin-left:auto; text-transform:none; letter-spacing:0; }
  #replout { flex:1; overflow:auto; padding:8px 12px; font:13px/1.45 ui-monospace,monospace; white-space:pre-wrap; }
  #replout .in .p { opacity:.5; user-select:none; }
  #replout .val { color:#2e7d32; } #replout .err { color:#e53935; }
  #replin { flex:none; resize:none; border:none; border-top:1px solid #8884; padding:8px 12px; font:13px/1.45 ui-monospace,monospace; background:transparent; color:inherit; outline:none; min-height:1.6em; }
  #gdbg { display:inline-flex; gap:4px; align-items:center; padding-left:10px; border-left:1px solid #8884; }
  #gdbg input { font:12px ui-monospace,monospace; padding:2px 6px; border:1px solid #8886; border-radius:4px; background:transparent; color:inherit; }
  #gstat { font-size:12px; opacity:.7; max-width:26em; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
</style></head><body>
<div id="bar"><button id="run">▶ Run</button><button id="test" title="Run this file's tests">✓ Test</button><button id="suite" title="Run the package suite (tests/*.loft beside the nearest loft.toml)">✓✓ Suite</button><button id="game" title="Launch as a game: a real loft process of its own (native window once the graphics lib lands)">🎮 Game</button><button id="save" disabled>Save</button><span id="dirty"></span>
  <span id="gdbg" hidden><input id="gbpin" spellcheck="false" placeholder="fn name" size="12"><button id="gbp" title="Break at the fn's entry in the RUNNING game (flips it to the interpreter)">⛳</button><button id="gresume" disabled title="Resume the paused game">⏵</button><button id="grebuild" title="Rebuild the game binary in the background (the old build keeps serving)">⟳</button><button id="gswap" disabled title="Swap to the rebuilt binary under the running world">⇄</button><span id="gstat"></span></span>
  <span id="steps"><button id="cont" disabled title="Continue (to next breakpoint)">⏵</button><button id="over" disabled title="Step over">↷</button><button id="into" disabled title="Step into">↓</button><button id="out" disabled title="Step out">↑</button></span>
  <span id="status">connecting…</span></div>
<div id="main">
  <div id="editor"><div id="gutter"></div><textarea id="src" spellcheck="false" wrap="off">__SOURCE__</textarea></div>
  <div class="pane"><h2>Compiler</h2><div id="diags" class="body"></div></div>
  <div class="pane"><h2>Program</h2><pre id="out" class="body"></pre></div>
</div>
<div id="bottom">
  <div id="dbg" hidden>
    <div class="pane"><h2>Variables · <span id="dbgfn"></span></h2><div id="vars" class="body"></div></div>
    <div class="pane"><h2>Watch</h2><div id="watches" class="body"></div><input id="waddin" spellcheck="false" placeholder="+ watch expression (Enter)"></div>
  </div>
  <div id="tests" hidden>
    <div class="pane"><h2>Tests · <span id="tsum"></span></h2><div id="tlist" class="body"></div></div>
  </div>
  <div id="repl">
    <div id="replhead"><span>REPL</span><span id="replctx" class="top">top-level</span><span class="hint">Enter runs · Shift+Enter newline · Esc clears · ↑↓ history</span></div>
    <div id="replout"></div>
    <textarea id="replin" rows="1" spellcheck="false" placeholder="try an expression…"></textarea>
  </div>
</div>
<script>
const FILE = __FILE_JS__;
const $ = id => document.getElementById(id);
let ws, mid = 0;
function esc(s) { const d = document.createElement("div"); d.textContent = s; return d.innerHTML; }
// ── editor — an editable source pane with a synced line-number gutter ──
let saved = "", saveId = 0, runAfterLaunch = 0, pendSnap = "";
function renderGutter() {
  const n = $("src").value.split("\n").length;
  let h = ""; for (let i = 1; i <= n; i++) h += '<div class="gnum" id="g' + i + '">' + i + '</div>';
  $("gutter").innerHTML = h;
  applyBpMarks();
}
function syncScroll() { $("gutter").scrollTop = $("src").scrollTop; }
function isDirty() { return $("src").value !== saved; }
function updateDirty() { const d = isDirty(); $("dirty").textContent = d ? "● unsaved" : ""; $("save").disabled = !d; }
function gnum(n) { return document.getElementById("g" + n); }
function gotoLine(n) {                           // scroll the textarea to line n + caret there
  const t = $("src"), lines = t.value.split("\n"); let i = 0;
  for (let k = 0; k < n - 1 && k < lines.length; k++) i += lines[k].length + 1;
  t.focus(); t.selectionStart = t.selectionEnd = i;
  const lh = parseFloat(getComputedStyle(t).lineHeight) || 18;
  t.scrollTop = Math.max(0, (n - 1) * lh - t.clientHeight / 2); syncScroll();
}
function clearDiags() {
  document.querySelectorAll("#gutter .gnum.error, #gutter .gnum.warning").forEach(g => g.classList.remove("error", "warning"));
  $("diags").innerHTML = "";
}
function showDiags(items) {
  clearDiags();
  if (!items.length) { $("diags").innerHTML = '<div class="none">no problems</div>'; return; }
  for (const it of items) {
    const lvl = (it.level === "error" || it.level === "fatal") ? "error" : "warning";
    const g = gnum(it.line); if (g) g.classList.add(lvl);
    const d = document.createElement("div");
    d.className = "d " + lvl;
    d.innerHTML = '<span class="loc">' + it.line + ':' + it.col + '</span>' + esc(it.message);
    d.onclick = () => gotoLine(it.line);
    $("diags").appendChild(d);
  }
}
// Persist the buffer to disk (the sandboxed writeFile); the reply (tracked by id) clears the
// dirty flag only once the write actually succeeded.
function persist() { const snap = $("src").value; pendSnap = snap; saveId = send("writeFile", {path: FILE, content: snap}); }
function doSave() {
  if (!ws || ws.readyState !== 1 || !isDirty()) return; persist(); send("compile", {file: FILE});
  // A PAUSED game's reload is dispatch-driven (the S3 finding) — poll it now
  // through the pause so the edit acks immediately.  (The 200 ms reload
  // throttle wants the save settled first.)
  if (gws && gamePaused) setTimeout(() => { if (gws && gamePaused) gws.send("D!:reload"); }, 400);
}
// ── debugger — gutter breakpoints, step controls, variables + watch panels ──
let bps = new Set(), paused = false, curLine = 0, watches = [], lastLocals = [], reqcb = {};
function sendCb(req, extra, cb) { const id = send(req, extra); reqcb[id] = cb; return id; }     // route this request's reply to cb
function sendBreakpoints() { send("setBreakpoints", {file: FILE, breakpoints: [...bps].map(line => ({line}))}); }
function applyBpMarks() {                                    // re-apply after every renderGutter (it rebuilds the gutter)
  bps.forEach(n => { const g = gnum(n); if (g) g.classList.add("bp"); });
  if (curLine) { const g = gnum(curLine); if (g) g.classList.add("cur"); }
}
function revealLine(n) {                                     // scroll the editor to line n WITHOUT stealing focus
  const t = $("src"), lh = parseFloat(getComputedStyle(t).lineHeight) || 18;
  t.scrollTop = Math.max(0, (n - 1) * lh - t.clientHeight / 2); syncScroll();
}
function markCurrentLine(n) {
  if (curLine) { const o = gnum(curLine); if (o) o.classList.remove("cur"); }
  curLine = n || 0;
  if (curLine) { const g = gnum(curLine); if (g) g.classList.add("cur"); revealLine(curLine); }
}
function setPaused(on, frame) {                              // the run/paused state machine
  paused = on;
  for (const b of ["cont", "over", "into", "out"]) $(b).disabled = !on;
  for (const b of ["run", "test", "suite"]) $(b).disabled = on;   // no relaunch / test run over a live frame — Continue/step first
  $("dbg").hidden = !on;
  if (on) {
    setCtx("frame");
    $("dbgfn").textContent = (frame && frame.function) || "?";
    markCurrentLine(frame && frame.line);
    renderVars((frame && frame.locals) || []);
    evalWatches();
  } else {
    setCtx("top"); markCurrentLine(0); renderVars([]);
  }
}
function renderVars(locals) {
  lastLocals = locals || [];
  const v = $("vars"); v.innerHTML = "";
  if (!lastLocals.length) { v.innerHTML = '<div class="none">no locals in scope</div>'; return; }
  for (const lc of lastLocals) {
    const row = document.createElement("div"); row.className = "var";
    const nm = document.createElement("span"); nm.className = "vn"; nm.textContent = lc.name;
    const vv = document.createElement("span"); vv.className = "vv"; vv.textContent = lc.value; vv.title = "click to edit (scalars)";
    vv.onclick = () => editVar(lc.name, vv);
    row.append(nm, vv); v.appendChild(row);
  }
}
function editVar(name, vv) {                                 // inline edit → setValue (edit-and-continue)
  const cur = vv.textContent, inp = document.createElement("input");
  inp.className = "vedit"; inp.value = cur; vv.replaceWith(inp); inp.focus(); inp.select();
  let done = false;
  const commit = ok => {
    if (done) return; done = true;
    if (ok && inp.value !== cur) sendCb("setValue", {target: name, value: inp.value},
      m => renderVars(m.ok && m.frame ? m.frame.locals : lastLocals));
    else renderVars(lastLocals);
  };
  inp.addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); commit(true); } else if (e.key === "Escape") { e.preventDefault(); commit(false); } });
  inp.addEventListener("blur", () => commit(true));
}
function renderWatches() {
  const w = $("watches"); w.innerHTML = "";
  if (!watches.length) { w.innerHTML = '<div class="none">no watches</div>'; }
  watches.forEach((expr, i) => {
    const row = document.createElement("div"); row.className = "watch";
    const rm = document.createElement("span"); rm.className = "wrm"; rm.textContent = "×"; rm.title = "remove";
    rm.onclick = () => { watches.splice(i, 1); renderWatches(); };
    const x = document.createElement("span"); x.className = "wx"; x.textContent = expr;
    const vv = document.createElement("span"); vv.className = "wv"; vv.id = "wv" + i; vv.textContent = paused ? "…" : "—";
    row.append(rm, x, vv); w.appendChild(row);
  });
  evalWatches();
}
function evalWatches() {                                     // re-evaluate every watch against the paused frame
  if (!paused) return;
  watches.forEach((expr, i) => sendCb("eval", {expr}, m => {
    const el = $("wv" + i); if (!el) return;
    el.textContent = m.value === undefined ? "?" : (typeof m.value === "object" ? JSON.stringify(m.value) : String(m.value));
  }));
}
// ── REPL — a context-switching prime element ──
let ctx = "top", histTop = [], histFrame = [], histIdx = -1, pending = null;
function curHist() { return ctx === "frame" ? histFrame : histTop; }
function setCtx(c) {
  if (c === ctx) return;
  histFrame = [];                       // a debug context's history is thrown away when it ends / starts
  ctx = c; histIdx = -1;
  const b = $("replctx"); b.className = c; b.textContent = c === "frame" ? "frame (paused)" : "top-level";
}
function replLine(cls, text) {
  const d = document.createElement("div"); d.className = cls; d.textContent = text;
  $("replout").appendChild(d); $("replout").scrollTop = $("replout").scrollHeight;
}
function replEcho(input) {
  const d = document.createElement("div"); d.className = "in";
  d.innerHTML = '<span class="p">' + (ctx === "frame" ? "(dbg) " : "› ") + '</span>' + esc(input);
  $("replout").appendChild(d); $("replout").scrollTop = $("replout").scrollHeight;
}
function autosize(t) { t.style.height = "auto"; t.style.height = Math.min(t.scrollHeight, window.innerHeight * 0.4) + "px"; }
function replSubmit() { const buf = $("replin").value; if (buf.trim()) { pending = buf; send("replEval", {input: buf}); } }
function onReplReply(m) {
  if (m.more) {                          // incomplete → keep the (editable) buffer, continue
    const t = $("replin");
    if (!t.value.endsWith("\n")) t.value += "\n";
    t.selectionStart = t.selectionEnd = t.value.length; autosize(t); return;
  }
  if (pending != null) { replEcho(pending); curHist().push(pending); }
  if (m.value != null) replLine("val", String(m.value));
  if (m.output) replLine("out", m.output.replace(/\n$/, ""));
  pending = null; histIdx = -1;
  const t = $("replin"); t.value = ""; autosize(t);
}
// ── test runner — Test (this file) / Suite (the package's tests/*) → testResult* → testSummary ──
function startTests(req) {
  if (!ws || ws.readyState !== 1 || paused) return;
  if (isDirty()) persist();                      // save first so the tests run the buffer
  $("tlist").innerHTML = ""; $("tsum").textContent = "running…"; $("tsum").className = "";
  $("tests").hidden = false;
  send(req, {file: FILE});
}
function addTestRow(m) {
  const row = document.createElement("div"); row.className = "t " + (m.passed ? "pass" : "fail");
  // A suite result carries its test file; a single-file run omits it (this buffer).
  const label = (m.file ? m.file + " · " : "") + m.name;
  let html = '<span class="ti">' + (m.passed ? "✓" : "✗") + '</span><span class="tn">' + esc(label) + '</span>';
  if (m.message) html += '<span class="tm">' + esc(m.message) + '</span>';
  row.innerHTML = html;
  // Click-to-jump targets THIS buffer, so only single-file failures jump (a suite
  // failure lives in another file; multi-file navigation is a later slice).
  if (!m.passed && !m.file) { row.classList.add("clickable"); row.title = "jump to line " + m.line; row.onclick = () => gotoLine(m.line); }
  $("tlist").appendChild(row);
}
function finishTests(m) {
  const total = m.passed + m.failed;
  if (!total) { $("tlist").innerHTML = '<div class="none">no tests found</div>'; }
  const files = m.files !== undefined ? " · " + m.files + " file" + (m.files === 1 ? "" : "s") : "";
  $("tsum").textContent = (m.failed === 0 ? (m.passed + "/" + total + " passed") : (m.failed + " of " + total + " failed")) + files;
  $("tsum").className = m.failed === 0 ? "ok" : "bad";
}
// ── game debugger (@PLN18 08-S7) — the D!: control channel, dialed DIRECTLY ──
// A kernel game prints its port; this page is then the debug client (the
// channel is loopback-only and enabled by launch_game's LOFT_DEBUG_CONTROL).
// Breakpoints are FN-ENTRY, name-keyed — they survive live reloads, and the
// page re-arms them after a build swap (the S7 loop, in the UI).
let gws = null, gport = 0, gameBps = new Set(), gamePaused = false, gameSwapped = false, reattach = 0;
function gstat(t) { $("gstat").textContent = t; }
function setGamePauseUi() { $("gresume").disabled = !gamePaused; }
function gameVarsOff() { if (!paused) { $("dbg").hidden = true; $("dbgfn").textContent = ""; renderVars([]); } }
function gdial() {
  if (gws) { try { gws.close(); } catch {} }
  gws = new WebSocket("ws://127.0.0.1:" + gport + "/ws");
  gws.onopen = () => {
    reattach = 0; $("gdbg").hidden = false;
    gstat(gameSwapped ? "swapped — debug reattached" : "debug ready");
    gameBps.forEach(fn => gws.send("D!:bp " + fn));      // re-arm across a swap
  };
  gws.onmessage = ev => onGameCtl(String(ev.data));
  gws.onclose = () => {
    if (reattach > 0) { reattach--; setTimeout(gdial, 300); return; }
    $("gdbg").hidden = true; gws = null; gamePaused = false; setGamePauseUi(); gameVarsOff();
  };
  gws.onerror = () => {};                                // close fires after; the retry lives there
}
function renderGameVars(locals) {
  const v = $("vars"); v.innerHTML = "";
  if (!locals.length) { v.innerHTML = '<div class="none">no bindings</div>'; return; }
  for (const lc of locals) {
    const row = document.createElement("div"); row.className = "var";
    const nm = document.createElement("span"); nm.className = "vn"; nm.textContent = lc.name;
    const vv = document.createElement("span"); vv.className = "vv"; vv.textContent = lc.value; vv.title = "game frame (read-only)";
    row.append(nm, vv); v.appendChild(row);
  }
}
function onGameCtl(msg) {
  if (!msg.startsWith("D:")) return;                     // game broadcasts also land on this socket — ignore
  if (msg.startsWith("D:hit ")) {
    gamePaused = true; setGamePauseUi();
    const rest = msg.slice(6), sp = rest.indexOf(" ");
    const fn = sp < 0 ? rest : rest.slice(0, sp);
    const locals = sp < 0 ? [] : rest.slice(sp + 1).split("|").filter(Boolean).map(kv => {
      const eq = kv.indexOf("="); return {name: kv.slice(0, eq), value: kv.slice(eq + 1)};
    });
    $("dbg").hidden = false; $("dbgfn").textContent = fn + " · game";
    renderGameVars(locals);
    gstat("⏸ paused in " + fn);
    $("out").textContent += "\n⏸ game paused in " + fn + "\n";
  }
  else if (msg === "D:resumed") { gamePaused = false; setGamePauseUi(); gstat("running"); gameVarsOff(); }
  else if (msg.startsWith("D:reload ")) $("out").textContent += "[game] live-reload " + msg.slice(9) + (gamePaused ? " (applied through the pause)" : "") + "\n";
  else if (msg.startsWith("D:rebuild ")) {
    const st = msg.slice(10);
    if (st === "started" || st === "1") { gstat("rebuilding…"); setTimeout(() => { if (gws) gws.send("D!:rebuild?"); }, 400); }
    else if (st === "2") { gstat("rebuild ready — ⇄ to swap"); $("gswap").disabled = false; }
    else gstat("rebuild " + st);
  }
  else if (msg.startsWith("D:swap ")) {
    if (msg.endsWith("true")) { gstat("swapping…"); reattach = 20; gameSwapped = true; $("gswap").disabled = true; }
    else gstat("swap refused (rebuild first)");
  }
  else if (msg.startsWith("D:ok bp ")) gstat("⛳ " + msg.slice(8) + " (fn entry)");
  else if (msg.startsWith("D:err") || msg.startsWith("D:quitting")) gstat(msg.slice(2));
}
$("gbp").onclick = () => {
  const fn = $("gbpin").value.trim(); if (!fn || !gws) return;
  gameBps.add(fn); gws.send("D!:bp " + fn); $("gbpin").value = "";
};
$("gbpin").addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); $("gbp").onclick(); } });
$("gresume").onclick = () => { if (gws) gws.send("D!:resume"); };
$("grebuild").onclick = () => { if (gws) gws.send("D!:rebuild"); };
$("gswap").onclick = () => { if (gws) gws.send("D!:swap auto"); };
function gameDebugReset() {
  if (gws) { try { gws.close(); } catch {} }
  gws = null; gport = 0; gameBps.clear(); gamePaused = false; gameSwapped = false; reattach = 0;
  $("gdbg").hidden = true; $("gswap").disabled = true; setGamePauseUi(); gameVarsOff();
}
// ── game launcher (slice 6) — a real loft child process; output arrives via polls ──
// Deliberately NOT disabled while paused: the game is its own process and never touches
// the paused session, so comparing a running game against a frozen frame is fine.
let gameTimer = 0;
function setGameUi(running) { $("game").textContent = running ? "■ Stop game" : "🎮 Game"; }
function pollGame() {
  sendCb("gameStatus", {}, m => {
    if (m.output) {
      $("out").textContent += m.output;
      // A kernel game announces its port — that port is the debug channel.
      if (!gport) {
        const pm = m.output.match(/listening on ws:\/\/0\.0\.0\.0:(\d+)\//)
                || m.output.match(/debug control on 127\.0\.0\.1:(\d+)/);
        if (pm) { gport = +pm[1]; gdial(); }
      }
    }
    if (m.running) { gameTimer = setTimeout(pollGame, 500); }
    else {
      gameTimer = 0;
      if (gameSwapped && gws) {
        // The driver chain retired at the swap; the NEW build serves on.
        $("out").textContent += "\n— build swapped; the game continues (control attached) —\n";
        setGameUi(true);
      } else {
        setGameUi(false); gameDebugReset();
        $("out").textContent += "\n— game exited" + (m.exit !== undefined ? " (code " + m.exit + ")" : "") + " —\n";
      }
    }
  });
}
function gameClick() {
  if (!ws || ws.readyState !== 1) return;
  if (!gameTimer && gameSwapped && gws) {          // a swapped game is no longer our child: quit over control
    gws.send("D!:quit");
    $("out").textContent += "\n— game stopped (quit over the control channel) —\n";
    setGameUi(false); gameSwapped = false; gameDebugReset();
    return;
  }
  if (gameTimer) {                                 // running → stop
    clearTimeout(gameTimer); gameTimer = 0;
    sendCb("stopGame", {}, m => {
      if (m.output) $("out").textContent += m.output;
      $("out").textContent += "\n— game stopped —\n"; setGameUi(false); gameDebugReset();
    });
    return;
  }
  if (isDirty()) persist();                        // the game runs what you see
  sendCb("launchGame", {file: FILE}, m => {
    if (!m.ok) { $("out").textContent += "\n[game] " + (m.error || "launch failed") + "\n"; return; }
    setGameUi(true); $("out").textContent += "\n— game launched —\n";
    gameTimer = setTimeout(pollGame, 300);
  });
}
function connect() {
  ws = new WebSocket("ws://" + location.host + "/ws");
  ws.onopen = () => { $("status").textContent = "connected"; send("launch", {file: FILE}); send("compile", {file: FILE}); };
  ws.onclose = () => { $("status").textContent = "disconnected"; };
  ws.onerror = () => { $("status").textContent = "error"; };
  ws.onmessage = ev => {
    let m; try { m = JSON.parse(ev.data); } catch { return; }
    if (m.ok !== undefined && m.event === undefined && m.context === undefined) {   // a bare response
      if (m.id === saveId) { saveId = 0; if (m.ok) { saved = pendSnap; updateDirty(); } return; }
      if (m.id === runAfterLaunch) { runAfterLaunch = 0;
        if (m.ok) { sendBreakpoints(); send("run", {}); } else $("out").textContent += "\n[not run — fix the errors above]\n"; return; }
      if (reqcb[m.id]) { const cb = reqcb[m.id]; delete reqcb[m.id]; cb(m); return; }   // setValue / watch eval
    }
    if (m.event === "diagnostics") {
      if (m.file === "<repl>") { for (const it of (m.items || [])) replLine("err", it.line + ":" + it.col + "  " + it.message); }
      else showDiags(m.items || []);
    }
    else if (m.event === "output") $("out").textContent += m.text;
    else if (m.event === "stopped") {
      const f = m.frame || {};
      $("out").textContent += "\n⏸ stopped in " + (f.function || "?") + (f.line ? ":" + f.line : "") + "\n";
      setPaused(true, f);
    }
    else if (m.event === "terminated") { $("out").textContent += "\n— done —\n"; setPaused(false); }
    else if (m.event === "testResult") addTestRow(m);
    else if (m.event === "testSummary") finishTests(m);
    else if (m.context !== undefined) onReplReply(m);       // a replEval reply (only it carries context)
    else if (m.ok === false && m.error) $("out").textContent += "\n[error] " + m.error + "\n";
  };
}
function send(req, extra) { const id = ++mid; ws.send(JSON.stringify(Object.assign({id, req}, extra))); return id; }
$("run").onclick = () => {
  if (!ws || ws.readyState !== 1) return;
  $("out").textContent = "";
  if (isDirty()) persist();                       // save first so disk matches the buffer
  send("compile", {file: FILE});                  // refresh the compiler console
  runAfterLaunch = send("launch", {file: FILE});  // reload; run() fires only if this launch is ok
};
$("test").onclick = () => startTests("runTests");
$("suite").onclick = () => startTests("runSuite");
$("game").onclick = gameClick;
const ti = $("replin");
ti.addEventListener("input", () => autosize(ti));
ti.addEventListener("keydown", e => {
  const atEnd = ti.selectionStart === ti.value.length && ti.selectionEnd === ti.value.length;
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); replSubmit(); }
  else if (e.key === "Enter" && !e.shiftKey && atEnd) { e.preventDefault(); replSubmit(); }   // Enter mid-buffer / Shift+Enter fall through = newline (edit anywhere)
  else if (e.key === "Escape") { e.preventDefault(); ti.value = ""; histIdx = -1; autosize(ti); }
  else if (e.key === "ArrowUp" && ti.selectionStart === 0) {
    const h = curHist(); if (!h.length) return; e.preventDefault();
    histIdx = histIdx < 0 ? h.length - 1 : Math.max(0, histIdx - 1); ti.value = h[histIdx]; autosize(ti);
  } else if (e.key === "ArrowDown" && ti.selectionStart === ti.value.length) {
    const h = curHist(); if (histIdx < 0) return; e.preventDefault();
    histIdx++; if (histIdx >= h.length) { histIdx = -1; ti.value = ""; } else ti.value = h[histIdx]; autosize(ti);
  }
});
// ── editor wiring ──
const src = $("src");
src.addEventListener("input", () => { renderGutter(); updateDirty(); });
src.addEventListener("scroll", syncScroll);
src.addEventListener("keydown", e => {
  if ((e.ctrlKey || e.metaKey) && e.key === "s") { e.preventDefault(); doSave(); return; }
  if (e.key === "Tab") {                          // insert spaces, keep focus in the editor
    e.preventDefault(); const s = src.selectionStart;
    src.value = src.value.slice(0, s) + "    " + src.value.slice(src.selectionEnd);
    src.selectionStart = src.selectionEnd = s + 4; renderGutter(); updateDirty();
  }
});
$("save").onclick = doSave;
document.addEventListener("keydown", e => { if ((e.ctrlKey || e.metaKey) && e.key === "s") { e.preventDefault(); doSave(); } });
// ── debugger wiring ──
$("gutter").addEventListener("click", e => {                 // toggle a breakpoint on the clicked line
  const g = e.target.closest(".gnum"); if (!g) return;
  const n = parseInt(g.id.slice(1), 10);
  if (bps.has(n)) { bps.delete(n); g.classList.remove("bp"); } else { bps.add(n); g.classList.add("bp"); }
  if (ws && ws.readyState === 1) sendBreakpoints();
});
$("cont").onclick = () => { if (paused) send("continue", {}); };
$("over").onclick = () => { if (paused) send("stepOver", {}); };
$("into").onclick = () => { if (paused) send("stepIn", {}); };
$("out").onclick = () => { if (paused) send("stepOut", {}); };
$("waddin").addEventListener("keydown", e => {
  if (e.key !== "Enter") return;
  e.preventDefault(); const v = e.target.value.trim();
  if (v) { watches.push(v); e.target.value = ""; renderWatches(); }
});
renderWatches();
saved = src.value; renderGutter(); updateDirty();
connect();
</script></body></html>"##;

/// HTML-escape `<`, `>`, `&` for embedding text in element content.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render `s` as a JavaScript double-quoted string literal (escaping the few characters
/// that would break out of it).
fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003c"), // keep out of a `</script>` break
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vectors() {
        // RFC 3174 "abc".
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // Empty string.
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn ws_accept_rfc6455_example() {
        // RFC 6455 §1.3 worked example.
        assert_eq!(
            ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn frame_len_forms() {
        let mut f = Vec::new();
        write_frame_len(&mut f, 5);
        assert_eq!(f, vec![5]);
        f.clear();
        write_frame_len(&mut f, 200);
        assert_eq!(f, vec![126, 0, 200]);
        f.clear();
        write_frame_len(&mut f, 0x1_0000);
        assert_eq!(f[0], 127);
    }

    #[test]
    fn read_masked_text_frame() {
        // A client text frame "hi", masked — must unmask to "hi".
        let mask = [0xAA, 0xBB, 0xCC, 0xDD];
        let payload = [b'h' ^ mask[0], b'i' ^ mask[1]];
        let mut bytes = vec![0x81, 0x80 | 2]; // FIN+text, masked, len 2
        bytes.extend_from_slice(&mask);
        bytes.extend_from_slice(&payload);
        let mut r = std::io::Cursor::new(bytes);
        let (opcode, data) = ws_read_frame(&mut r).unwrap().unwrap();
        assert_eq!(opcode, 0x1);
        assert_eq!(data, b"hi");
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write as _;
        bytes.iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }
}
