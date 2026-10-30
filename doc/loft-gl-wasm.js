// loft-gl-wasm.js — WebGL2 bridge for compiled loft WASM (--html export).
// Returns a WASM imports object with loft_gl.* and loft_io.* modules.
// Uses asyncify for frame yield: gl_swap_buffers suspends execution so
// the browser can render via requestAnimationFrame.

// Asyncify controller — manages suspend/resume across frames.
// Usage: const ac = new AsyncifyCtrl(instance);
//        ac.start('loft_start');  // runs until first swap_buffers
//        // on each rAF: ac.resume('loft_start');
// @P321c Phase 3b — decode base64-embedded PNG assets to raw RGB bytes.
// `rawAssets` is `{name: base64String, ...}` (Phase 3a embed in main.rs);
// resolves to `{name: {width, height, bytes: Uint8Array(rgb)}}` for the
// imaging bridge to look up sync.  Runs once after `WebAssembly.instantiate`
// + before `loft_start` so the wasm-side imaging_query/copy is synchronous.
async function decodeLoftAssets(rawAssets) {
  if (!rawAssets || typeof rawAssets !== 'object') return {};
  const out = {};
  for (const [name, b64] of Object.entries(rawAssets)) {
    if (typeof b64 !== 'string') { out[name] = b64; continue; }
    try {
      const bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
      const blob = new Blob([bin], { type: 'image/png' });
      const bitmap = await createImageBitmap(blob);
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      ctx.drawImage(bitmap, 0, 0);
      const imgData = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
      // Keep the RGBA the browser already decoded.  This used to drop the alpha
      // channel into an RGB buffer, and every consumer then put 255 back — so a
      // PNG with transparency uploaded fully OPAQUE and no caller could tell.
      out[name] = {
        width: bitmap.width,
        height: bitmap.height,
        bytes: new Uint8Array(imgData.data.buffer.slice(0)),
      };
    } catch (e) {
      // Leave the entry undecoded; bridge will treat as missing.
      out[name] = null;
    }
  }
  return out;
}

// AsyncifyCtrl moved to doc/loft-asyncify.js (shared with the headless template);
// the HTML page includes that blob before this one.

function buildLoftImports(canvas, output, getMem, asyncCtrl) {
  const gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  // @PLN117: loftTextDecoder also reads a threaded page's SHARED memory, which
  // a plain TextDecoder refuses; falls back when the page has no threading glue.
  const decoder = typeof loftTextDecoder === 'function' ? loftTextDecoder() : new TextDecoder();
  function readStr(ptr, len) {
    return decoder.decode(new Uint8Array(getMem().buffer, ptr, len));
  }
  // lib/graphics shaders are written for desktop GLSL (#version 330 core);
  // WebGL2 needs GLSL ES 3.00.  The grammar is otherwise compatible for the
  // subset our shaders use (in/out/layout/texture()/discard/gl_Position),
  // so rewrite the version directive + inject default precision and let
  // WebGL2 take everything else as-is.  Fragment shaders need explicit
  // precision for float/int; vertex shaders default to highp.
  function translateShader(src, isFragment) {
    const re = /^\s*#version\s+\d+(\s+\w+)?\s*\n?/;
    const head = isFragment
      ? '#version 300 es\nprecision highp float;\nprecision highp int;\n'
      : '#version 300 es\n';
    if (re.test(src)) return src.replace(re, head);
    return head + src;
  }

  let programs = [], vaos = [], textures = [], fbos = [];
  const keys = new Set();
  let mouseX = 0, mouseY = 0, mouseBtn = 0, wheelAcc = 0;

  // Handles are 1-BASED here, so 0 means failure and nothing else — the contract
  // `lib/graphics` documents ("0 on failure") and what the native backend gives you,
  // since a real GL object name is never 0.  These tables used to hand back the array
  // index, so the FIRST shader, VAO or texture a page created came back as 0: the
  // documented `if (prog == 0) { fail }` rejected working code, and a client using 0 as
  // its "slot is free" sentinel lost the first mesh it uploaded and then handed that slot
  // to the next one (loft#669).  `fbos` was already 1-based; these three now match.
  const hold = (arr, obj) => arr.push(obj);          // push returns the new length = handle
  const slot = (arr, h) => (h > 0 && h <= arr.length ? arr[h - 1] : null);

  function mapKey(code) {
    if (code.startsWith('Key')) return code.charCodeAt(3) + 32;
    if (code.startsWith('Digit')) return code.charCodeAt(5);
    const s = { ArrowUp:128, ArrowDown:129, ArrowLeft:130, ArrowRight:131,
      ShiftLeft:132, ShiftRight:132, ControlLeft:133, ControlRight:133,
      Space:32, Enter:13, Escape:27, Tab:9 };
    return s[code] || 0;
  }
  // ── text (loft#737) ────────────────────────────────────────────────────────
  // The browser already HAS a rasteriser, so the text bridge is a 2D canvas:
  // `measureText` for the metrics and `fillText` for the coverage bitmap.  The
  // desktop backend uses fontdue and returns an 8-bit alpha bitmap whose height is
  // the line height and whose baseline sits `ascent` from the top; drawing white on
  // transparent reproduces exactly that — the alpha channel IS the coverage.
  //
  // A font PATH resolves to a CSS family rather than a file: there is no synchronous
  // way to load font bytes here, and an async load would change the metrics between
  // the measure and the rasterise of the same string.  A page that wants its real
  // font declares it with `[[font]]` in its loft.toml, under the file's base name —
  // `--html` then emits the `@font-face` (or the provider `<link>`) and waits for the
  // family before starting the program (@PLN146 F5/F6, `src/html_fonts.rs`).  A page
  // may equally carry its own CSS.  Either way the base name is the family asked for,
  // and a generic guessed from that name is what CSS falls back to.
  let fonts = [], textCv = null, textCx = null;
  // Audio state.  The context is shared with `audio_play_raw` through the same
  // global it already used, which is also how a test can put an OfflineAudioContext
  // between the bridge and the speakers.
  let audioClips = [], audioSinks = [];
  function audioCtx() {
    if (!window._loftAudioCtx) window._loftAudioCtx = new AudioContext();
    return window._loftAudioCtx;
  }
  // Start (or restart) a sink's source at `offset` seconds, through a gain and a
  // panner that OUTLIVE it.  A buffer source is one-shot — it cannot be paused,
  // moved or replayed — so seeking means building a new one, and keeping the two
  // nodes behind it is what lets the volume and pan a caller already set survive
  // that.  Answers whether it started.
  function startSink(s, pan, offset) {
    try {
      const ctx = audioCtx();
      if (!s.gain) {
        s.gain = ctx.createGain();
        s.gain.gain.value = s.vol;
        s.panner = ctx.createStereoPanner();
        s.gain.connect(s.panner);
        s.panner.connect(ctx.destination);
      }
      s.panner.pan.value = Math.max(-1, Math.min(1, pan || 0));
      const src = ctx.createBufferSource();
      src.buffer = s.buf;
      src.loop = !!s.loop;
      src.connect(s.gain);
      src.start(0, Math.max(0, offset || 0));
      s.src = src;
      return true;
    } catch (e) { return false; }
  }
  function stopSource(s) {
    if (!s || !s.src) return;
    // Stopping an already-ended source is not an error anywhere but in the spec.
    try { s.src.stop(); } catch (e) { /* already ended */ }
    try { s.src.disconnect(); } catch (e) { /* already detached */ }
    s.src = null;
  }
  function text2d() {
    if (!textCx) {
      textCv = document.createElement('canvas');
      textCx = textCv.getContext('2d', { willReadFrequently: true });
    }
    return textCx;
  }
  // The CSS font list for a font's base name: the requested family FIRST, then a
  // generic guessed from the name for CSS to fall back to on its own.
  //
  // @PLN146 F5 — this used to ask `document.fonts.check` whether the page had the
  // family, and that question has no answer: check() is TRUE for a family nothing
  // declares (no unloaded face matches it) and FALSE for an `@font-face` that is
  // still loading.  So the one page that HAD brought its own font took the generic
  // branch, and the answer is cached per handle — locking that handle to
  // `sans-serif` for the whole run, with nothing on stderr.  The name-heuristic
  // branch, meanwhile, was unreachable.  Naming both is right whichever way it
  // goes: a family that resolves wins, and one that does not falls to the generic
  // its NAME suggests rather than always to `sans-serif`.
  function familyFor(base) {
    const quoted = '"' + base.replace(/"/g, '') + '"';
    return quoted + ', ' + genericFor(base);
  }
  function genericFor(base) {
    const b = base.toLowerCase();
    if (/mono|courier|consol|code/.test(b)) return 'monospace';
    if (/serif/.test(b) && !/sans/.test(b)) return 'serif';
    return 'sans-serif';
  }
  // Did the requested family actually resolve, or is text about to draw in the
  // generic?  `document.fonts.check` cannot say (see above), so measure: a family
  // the browser HAS overrides both generics and the two measurements agree, while
  // one it does not have follows each generic and they differ.  The probe string
  // mixes narrow and wide letters so a proportional and a fixed-pitch face cannot
  // measure the same.
  function familyResolves(base) {
    try {
      const quoted = '"' + base.replace(/"/g, '') + '"', cx = text2d();
      const w = (list) => { cx.font = '40px ' + list; return cx.measureText('iiiWWWmmmlll').width; };
      return w(quoted + ', monospace') === w(quoted + ', sans-serif');
    } catch (e) {
      return true;   // no 2-D canvas to measure with — say nothing rather than warn wrongly
    }
  }
  // Metrics for font `fi` at `sz`, in the same terms the desktop backend reports:
  // `asc`/`desc` from the font's own box, `line` the height a bitmap gets.
  function fontMetrics(fi, sz) {
    const f = fonts[fi];
    const cx = text2d();
    cx.font = sz + 'px ' + (f ? f.family : 'sans-serif');
    const m = cx.measureText('Mg');
    const asc = m.fontBoundingBoxAscent || m.actualBoundingBoxAscent || sz * 0.8;
    const desc = m.fontBoundingBoxDescent || m.actualBoundingBoxDescent || sz * 0.2;
    return { cx: cx, asc: asc, line: Math.max(1, Math.ceil(asc + desc)) };
  }
  // Draw `s` white-on-transparent and hand back its RGBA pixels + size, or null when
  // there is nothing to draw.  The single place the bitmap's geometry is decided, so
  // `rasterize_text_into`, `text_texture` and `measure_text` cannot disagree about it.
  function rasterText(fi, s, sz) {
    if (!fonts[fi] || !s) return null;
    const mt = fontMetrics(fi, sz);
    const w = Math.ceil(mt.cx.measureText(s).width);
    const h = mt.line;
    if (w <= 0 || h <= 0) return null;
    textCv.width = w; textCv.height = h;
    // Resizing the canvas resets its context state, so re-apply the font.
    mt.cx.font = sz + 'px ' + fonts[fi].family;
    mt.cx.clearRect(0, 0, w, h);
    mt.cx.fillStyle = '#fff';
    mt.cx.textBaseline = 'alphabetic';
    mt.cx.fillText(s, 0, mt.asc);
    return { w: w, h: h, px: mt.cx.getImageData(0, 0, w, h).data };
  }
  // An 8-bit coverage bitmap as a GL texture. WebGL2 has no TEXTURE_SWIZZLE, which is
  // how the desktop backend makes a RED texture sample as (1,1,1,r), so expand to RGBA
  // here — same sampling result, and the shaders stay identical across targets.
  function alphaTexture(alphaAt, w, h) {
    const px = new Uint8Array(w * h * 4);
    for (let i = 0; i < w * h; i++) {
      px[i * 4] = 255; px[i * 4 + 1] = 255; px[i * 4 + 2] = 255; px[i * 4 + 3] = alphaAt(i);
    }
    const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, px);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return hold(textures, t);
  }
  // ── One space: PHYSICAL pixels ───────────────────────────────────────────
  //
  // The canvas backing store is allocated at logical x devicePixelRatio, so the
  // GPU rasterises at the display's real resolution instead of the browser
  // upscaling a smaller buffer.  Everything a program sees stays in that ONE
  // space: `gl_window_width/height` report it, the pointer answers in it, and
  // `gl_viewport` / `gl_scissor` take it.
  //
  // ⚠ THAT IS THE DOCUMENTED CONTRACT, not a choice made here — `gl_window_width`
  // is *"current window inner size in PHYSICAL pixels ... so projection / 2D ortho
  // / overlays use the real resolution rather than a windowed hint"*.  A program
  // that follows that advice is crisp at any ratio for free.  One that stores the
  // size it ASKED for and reuses it as a viewport renders into part of the
  // surface — the same defect the doc already warns about for fullscreen, now
  // reachable on an ordinary high-density display.
  //
  // Keeping one space is also why the bridge rescales nothing: `gl_viewport` and
  // `gl_scissor` are aimed at an offscreen target as often as at the window
  // (a 1024x1024 shadow map, a thumbnail), and a bridge that scaled them would
  // render those into a fraction of their own texture.
  let dpr = 1;
  const toDevice = v => Math.round(v * dpr);

  function glCap(c) {
    return [0, gl.DEPTH_TEST, gl.BLEND, gl.CULL_FACE, gl.SCISSOR_TEST][c] || c;
  }
  function glBF(f) { return [gl.ZERO, gl.ONE, gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.DST_ALPHA, gl.ONE_MINUS_DST_ALPHA][f] || f; }
  function glMode(m) { return [gl.TRIANGLES, gl.LINES, gl.POINTS][m] || gl.TRIANGLES; }

  canvas.tabIndex = 0;
  canvas.addEventListener('keydown', e => { keys.add(mapKey(e.code)); e.preventDefault(); });
  canvas.addEventListener('keyup', e => keys.delete(mapKey(e.code)));
  canvas.addEventListener('mousemove', e => {
    // ⚠ MAPPED FROM CSS PIXELS INTO THE FRAMEBUFFER'S SPACE.  Browser events are
    // in CSS pixels; everything the program works in is physical.  Scaling the
    // backing store WITHOUT this gives a crisp picture where every click lands at
    // 1/ratio of where the user pointed.  Derived from the canvas's actual box
    // rather than from the ratio, so a page whose stylesheet sizes the canvas
    // (`width: 100%`) is right too.
    const r = canvas.getBoundingClientRect();
    const sx = r.width > 0 ? canvas.width / r.width : 1;
    const sy = r.height > 0 ? canvas.height / r.height : 1;
    mouseX = (e.clientX - r.left) * sx; mouseY = (e.clientY - r.top) * sy;
  });
  canvas.addEventListener('mousedown', e => { mouseBtn |= (1 << e.button); });
  canvas.addEventListener('mouseup', e => { mouseBtn &= ~(1 << e.button); });
  // `gl_mouse_wheel` is documented as the delta accumulated since the last call,
  // positive = UP, with pixel-delta (trackpad) scrolling quantised at 20 px/line.
  // The browser's deltaY is positive DOWNWARD, hence the negation.
  canvas.addEventListener('wheel', e => {
    wheelAcc += e.deltaMode === 0 ? -e.deltaY / 20 : -e.deltaY;
    e.preventDefault();
  }, { passive: false });

  let shouldClose = false, _fsQuad = null;

  // ── graphics 0.10 (@PLN208): the window and GL calls of its `[c] shim` ─────────────────
  // A page built with `--html` compiles graphics' C shim with `LOFT_BROWSER`, and that shim
  // imports these by name: four window calls and one per GL entry point it uses, each of the
  // GL signature (integer names, pointers into the module's memory).  GL objects are numbered
  // here — one table for every kind, 0 for none — and the shim keeps every other piece of
  // state itself, exactly as on the desktop.  Input arrives as events in 0.9's codes.
  function ggImports() {
    const objs = [null], locs = [], shaderType = new Map(), queue = [];
    let open = false;
    const mem = () => getMem().buffer;
    const dv = () => new DataView(mem());
    const add = (o) => { objs.push(o); return objs.length - 1; };
    const get = (id) => objs[id] || null;
    function cstr(ptr) {
      const b = new Uint8Array(mem());
      let end = ptr;
      while (b[end] !== 0) end++;
      return decoder.decode(b.subarray(ptr, end));
    }
    function writeLog(text, max, lenPtr, bufPtr) {
      const bytes = new TextEncoder().encode(text).subarray(0, Math.max(0, max - 1));
      const b = new Uint8Array(mem());
      if (bufPtr && max > 0) { b.set(bytes, bufPtr); b[bufPtr + bytes.length] = 0; }
      if (lenPtr) dv().setInt32(lenPtr, bytes.length, true);
    }
    function gen(n, ptr, make) {
      const d = dv();
      for (let i = 0; i < n; i++) d.setUint32(ptr + 4 * i, add(make()), true);
    }
    function drop(n, ptr, del) {
      const d = dv();
      for (let i = 0; i < n; i++) {
        const id = d.getUint32(ptr + 4 * i, true);
        const o = get(id);
        if (o) { del(o); objs[id] = null; }
      }
    }
    // The desktop key table (graphics' KEY_*): the logical key, ASCII lower-cased; named keys
    // by name; -1 for one the table has no code for.
    const NAMED = { Enter: 13, Escape: 27, Tab: 9, Backspace: 8, ArrowUp: 128, ArrowDown: 129,
      ArrowLeft: 130, ArrowRight: 131, Shift: 132, Control: 133, Delete: 147, Insert: 148,
      Home: 149, End: 150, PageUp: 151, PageDown: 152 };
    const TYPED = { Enter: '\r', Backspace: '\b', Escape: '\x1b', Tab: '\t', Delete: '\x7f' };
    const MOD = { Shift: 1, Control: 2, Alt: 4, Meta: 8 };
    function keyCode(k) {
      if (k.length === 1) {
        const c = k.charCodeAt(0);
        if (c >= 128) return -1;
        return c >= 65 && c <= 90 ? c + 32 : c;
      }
      if (NAMED[k] !== undefined) return NAMED[k];
      const f = /^F([1-9]|1[0-2])$/.exec(k);
      return f ? 134 + Number(f[1]) : -1;
    }
    // The modifiers BEFORE the event, as X reports them: pressing Shift reports no Shift.
    function mods(e, down) {
      let m = (e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.altKey ? 4 : 0) | (e.metaKey ? 8 : 0);
      const own = MOD[e.key] || 0;
      return down ? m & ~own : m | own;
    }
    function point(e) {
      const r = canvas.getBoundingClientRect();
      const sx = r.width > 0 ? canvas.width / r.width : 1;
      const sy = r.height > 0 ? canvas.height / r.height : 1;
      return [Math.trunc((e.clientX - r.left) * sx), Math.trunc((e.clientY - r.top) * sy)];
    }
    const BUTTON = [1, 4, 2];
    canvas.addEventListener('keydown', (e) => {
      if (!open) return;
      queue.push({ tag: 1, v: [keyCode(e.key), mods(e, true), e.repeat ? 1 : 0] });
      let t = e.key.length === 1 ? e.key : TYPED[e.key];
      if (t && e.ctrlKey && /^[a-z]$/i.test(t)) t = String.fromCharCode(t.charCodeAt(0) & 0x1f);
      if (t) queue.push({ tag: 3, text: t });
    });
    canvas.addEventListener('keyup', (e) => {
      if (open) queue.push({ tag: 2, v: [keyCode(e.key), mods(e, false)] });
    });
    canvas.addEventListener('mousedown', (e) => {
      if (open && BUTTON[e.button]) queue.push({ tag: 4, v: [BUTTON[e.button], ...point(e)] });
    });
    canvas.addEventListener('mouseup', (e) => {
      if (open && BUTTON[e.button]) queue.push({ tag: 5, v: [BUTTON[e.button], ...point(e)] });
    });
    canvas.addEventListener('mousemove', (e) => { if (open) queue.push({ tag: 6, v: point(e) }); });
    canvas.addEventListener('wheel', (e) => {
      if (open && e.deltaY !== 0) queue.push({ tag: 7, v: [e.deltaY < 0 ? 1 : -1, ...point(e)] });
    });
    const ok = (gl ? 1 : 0);
    return {
      gg_js_open(w, h, fullscreen) {
        if (!gl) return 0;
        dpr = window.devicePixelRatio || 1;
        if (fullscreen) { w = window.innerWidth; h = window.innerHeight; }
        canvas.width = toDevice(w); canvas.height = toDevice(h);
        canvas.style.width = w + 'px'; canvas.style.height = h + 'px';
        canvas.style.display = 'block';
        output.style.display = 'none';
        open = true;
        return ok;
      },
      gg_js_size(which) { return which === 0 ? canvas.width : canvas.height; },
      gg_present() {
        gl.flush();
        if (asyncCtrl && asyncCtrl.ac) asyncCtrl.ac.suspend();
      },
      gg_js_event(outPtr, textPtr, cap) {
        const ev = queue.shift();
        if (!ev) return 0;
        const d = dv();
        if (ev.tag === 3) {
          const bytes = new TextEncoder().encode(ev.text).subarray(0, cap);
          new Uint8Array(mem()).set(bytes, textPtr);
          d.setInt32(outPtr, bytes.length, true);
        } else {
          for (let i = 0; i < 5; i++) d.setInt32(outPtr + 4 * i, ev.v[i] | 0, true);
        }
        return ev.tag;
      },
      gg_glClearColor(r, g, b, a) { gl.clearColor(r, g, b, a); },
      gg_glClear(m) { gl.clear(m); },
      gg_glColorMask(r, g, b, a) { gl.colorMask(!!r, !!g, !!b, !!a); },
      gg_glViewport(x, y, w, h) { gl.viewport(x, y, w, h); },
      gg_glEnable(c) { gl.enable(c); },
      gg_glDisable(c) { gl.disable(c); },
      gg_glReadBuffer(m) { gl.readBuffer(m); },
      gg_glDrawBuffer(m) { gl.drawBuffers([m]); },
      gg_glPixelStorei(p, v) { gl.pixelStorei(p, v); },
      gg_glReadPixels(x, y, w, h, format, type, ptr) {
        gl.readPixels(x, y, w, h, format, type, new Uint8Array(mem(), ptr, w * h * 4));
      },
      gg_glCreateShader(type) { const id = add(gl.createShader(type)); shaderType.set(id, type); return id; },
      gg_glShaderSource(sh, count, strings, lengths) {
        const d = dv();
        let src = '';
        for (let i = 0; i < count; i++) {
          const p = d.getUint32(strings + 4 * i, true);
          const n = lengths ? d.getInt32(lengths + 4 * i, true) : -1;
          src += n < 0 ? cstr(p) : readStr(p, n);
        }
        gl.shaderSource(get(sh), translateShader(src, shaderType.get(sh) === gl.FRAGMENT_SHADER));
      },
      gg_glCompileShader(sh) { gl.compileShader(get(sh)); },
      gg_glGetShaderiv(sh, pname, out) {
        const v = pname === 0x8B84 ? (gl.getShaderInfoLog(get(sh)) || '').length + 1
                                   : (gl.getShaderParameter(get(sh), pname) ? 1 : 0);
        dv().setInt32(out, v, true);
      },
      gg_glGetShaderInfoLog(sh, max, lenPtr, buf) { writeLog(gl.getShaderInfoLog(get(sh)) || '', max, lenPtr, buf); },
      gg_glDeleteShader(sh) { const o = get(sh); if (o) { gl.deleteShader(o); objs[sh] = null; shaderType.delete(sh); } },
      gg_glCreateProgram() { return add(gl.createProgram()); },
      gg_glAttachShader(p, sh) { gl.attachShader(get(p), get(sh)); },
      gg_glLinkProgram(p) { gl.linkProgram(get(p)); },
      gg_glGetProgramiv(p, pname, out) {
        const v = pname === 0x8B84 ? (gl.getProgramInfoLog(get(p)) || '').length + 1
                                   : (gl.getProgramParameter(get(p), pname) ? 1 : 0);
        dv().setInt32(out, v, true);
      },
      gg_glGetProgramInfoLog(p, max, lenPtr, buf) { writeLog(gl.getProgramInfoLog(get(p)) || '', max, lenPtr, buf); },
      gg_glDeleteProgram(p) { const o = get(p); if (o) { gl.deleteProgram(o); objs[p] = null; } },
      gg_glUseProgram(p) { gl.useProgram(get(p)); },
      gg_glGetUniformLocation(p, namePtr) {
        const loc = gl.getUniformLocation(get(p), cstr(namePtr));
        if (!loc) return -1;
        locs.push(loc);
        return locs.length - 1;
      },
      gg_glUniform1f(at, x) { gl.uniform1f(locs[at], x); },
      gg_glUniform2f(at, x, y) { gl.uniform2f(locs[at], x, y); },
      gg_glUniform3f(at, x, y, z) { gl.uniform3f(locs[at], x, y, z); },
      gg_glUniform4f(at, x, y, z, w) { gl.uniform4f(locs[at], x, y, z, w); },
      gg_glUniform1i(at, v) { gl.uniform1i(locs[at], v); },
      gg_glUniformMatrix4fv(at, count, transpose, ptr) {
        gl.uniformMatrix4fv(locs[at], !!transpose, new Float32Array(mem(), ptr, 16 * count));
      },
      gg_glGenVertexArrays(n, ptr) { gen(n, ptr, () => gl.createVertexArray()); },
      gg_glBindVertexArray(id) { gl.bindVertexArray(get(id)); },
      gg_glDeleteVertexArrays(n, ptr) { drop(n, ptr, (o) => gl.deleteVertexArray(o)); },
      gg_glGenBuffers(n, ptr) { gen(n, ptr, () => gl.createBuffer()); },
      gg_glBindBuffer(target, id) { gl.bindBuffer(target, get(id)); },
      gg_glBufferData(target, size, ptr, usage) {
        if (ptr) gl.bufferData(target, new Uint8Array(mem(), ptr, size), usage);
        else gl.bufferData(target, size, usage);
      },
      gg_glDeleteBuffers(n, ptr) { drop(n, ptr, (o) => gl.deleteBuffer(o)); },
      gg_glVertexAttribPointer(i, size, type, norm, stride, offset) {
        gl.vertexAttribPointer(i, size, type, !!norm, stride, offset);
      },
      gg_glEnableVertexAttribArray(i) { gl.enableVertexAttribArray(i); },
      gg_glVertexAttribDivisor(i, d) { gl.vertexAttribDivisor(i, d); },
      gg_glDrawArrays(mode, first, count) { gl.drawArrays(mode, first, count); },
      gg_glDrawElements(mode, count, type, offset) { gl.drawElements(mode, count, type, offset); },
      gg_glDrawArraysInstanced(mode, first, count, n) { gl.drawArraysInstanced(mode, first, count, n); },
      gg_glDrawElementsInstanced(mode, count, type, offset, n) {
        gl.drawElementsInstanced(mode, count, type, offset, n);
      },
      gg_glBlendFunc(s, d) { gl.blendFunc(s, d); },
      gg_glCullFace(f) { gl.cullFace(f); },
      gg_glDepthMask(m) { gl.depthMask(!!m); },
      gg_glScissor(x, y, w, h) { gl.scissor(x, y, w, h); },
      gg_glFinish() { gl.finish(); },
      gg_glLineWidth(w) { gl.lineWidth(w); },
      // WebGL has no point size state: a shader that draws points writes gl_PointSize.
      gg_glPointSize(_s) {},
      gg_glGenTextures(n, ptr) { gen(n, ptr, () => gl.createTexture()); },
      gg_glBindTexture(target, id) { gl.bindTexture(target, get(id)); },
      gg_glTexImage2D(target, level, internal, w, h, border, format, type, ptr) {
        // WebGL2 takes a float depth texture only by its sized format.
        if (internal === gl.DEPTH_COMPONENT && type === gl.FLOAT) internal = gl.DEPTH_COMPONENT32F;
        const data = !ptr ? null : type === gl.FLOAT ? new Float32Array(mem(), ptr, w * h)
                                                     : new Uint8Array(mem(), ptr, w * h * 4);
        gl.texImage2D(target, level, internal, w, h, border, format, type, data);
      },
      gg_glTexSubImage2D(target, level, x, y, w, h, format, type, ptr) {
        gl.texSubImage2D(target, level, x, y, w, h, format, type, new Uint8Array(mem(), ptr, w * h * 4));
      },
      gg_glTexParameteri(target, pname, v) { gl.texParameteri(target, pname, v); },
      gg_glDeleteTextures(n, ptr) { drop(n, ptr, (o) => gl.deleteTexture(o)); },
      gg_glGenerateMipmap(target) { gl.generateMipmap(target); },
      gg_glActiveTexture(unit) { gl.activeTexture(unit); },
      gg_glGenFramebuffers(n, ptr) { gen(n, ptr, () => gl.createFramebuffer()); },
      gg_glBindFramebuffer(target, id) { gl.bindFramebuffer(target, get(id)); },
      gg_glFramebufferTexture2D(target, att, textarget, id, level) {
        gl.framebufferTexture2D(target, att, textarget, get(id), level);
      },
      gg_glDeleteFramebuffers(n, ptr) { drop(n, ptr, (o) => gl.deleteFramebuffer(o)); },
    };
  }

  // Defensive Number coercion at the WASM-import boundary.  loft's
  // `integer` is i64 since the phase-2c migration; wasm-bindgen's
  // automatic i64 → JsValue path produces BigInt, which silently
  // mis-behaves under the `>>>` / `&` bitwise ops the bridge uses
  // (unpacking RGBA, dispatching enums).  Coerce to Number at entry
  // so every method body can treat args as plain numbers.  Strings
  // pass through (typeof 'foo' !== 'bigint').
  const n = (x) => (typeof x === 'bigint' ? Number(x) : x);
  function coerceArgs(fns) {
    const wrapped = {};
    for (const [name, f] of Object.entries(fns)) {
      wrapped[name] = function(...args) {
        for (let i = 0; i < args.length; i++) args[i] = n(args[i]);
        return f.apply(this, args);
      };
    }
    return wrapped;
  }

  return {
    loft_io: coerceArgs({
      // loft#851 — the page's filesystem, from the loft-fs.js blob the HTML
      // template inlines just above this one.  Spread FIRST so a handler below
      // could deliberately override one; nothing does today.
      ...(typeof loftFSImports === 'function' ? loftFSImports(getMem) : {}),
      loft_host_print(ptr, len) { output.textContent += readStr(ptr, len); },
      // #620: the browser CLOCK bridge.  wasm32-unknown-unknown has no std
      // clock, so without these now()/ticks() returned a hardcoded 0 and every
      // frame timer read the same instant.  performance.now() is monotonic and
      // page-relative — exactly ticks()'s contract.
      loft_host_time_now_ms() { return Date.now(); },
      loft_host_time_ticks_us() { return performance.now() * 1000; },
      // JS -> loft input is a QUEUE, the mirror of loft_host_print: seed it
      // with globalThis.loftInput (a string) before loft_start, push live
      // messages any time with globalThis.loftPush(msg) — e.g. fetch()
      // completions.  Each host_input() call pops one message via a
      // len+copy pair (loft sizes the buffer, the copy pops).
      loft_host_input_len() {
        if (!globalThis.__loftInQ) {
          globalThis.__loftInQ = [];
          if (globalThis.loftInput != null)
            globalThis.__loftInQ.push(new TextEncoder().encode(String(globalThis.loftInput)));
          globalThis.loftPush = (m) =>
            globalThis.__loftInQ.push(new TextEncoder().encode(String(m)));
        }
        const q = globalThis.__loftInQ;
        return q.length ? q[0].length : 0;
      },
      loft_host_input_copy(ptr) {
        const b = (globalThis.__loftInQ || []).shift();
        if (b) new Uint8Array(getMem().buffer, ptr, b.length).set(b);
      },
      // loft -> JS structured messages (host_output()): the page handles
      // them in globalThis.loftOutput(msg) — the request/response pattern is
      // host_output a request, act on it in JS, loftPush the completion.
      loft_host_output(ptr, len) {
        const m = readStr(ptr, len);
        if (globalThis.loftOutput) globalThis.loftOutput(m);
        else console.log("[loft:out]", m);
      },
      // @PLN97 store_load_url_trusted: async fetch() bridged to a SYNCHRONOUS
      // loft call via asyncify — the same driver the headless page uses (see
      // main.rs / WASM_STORE_LOAD_URL.md).  Invoked TWICE per fetch:
      //  (1) NORMAL state: start fetch(url), then ac.suspend() unwinds the
      //      whole wasm stack to the event loop (return value ignored).
      //  (2) REWINDING (===2): the fetch resolved + resume() replayed the
      //      stack here — ac.suspend() stop_rewinds and we RETURN the byte
      //      length (0xFFFFFFFF on error → net::fetch_bytes maps it to Err).
      // The bytes are copied out separately by loft_host_http_get_copy.
      loft_host_http_get(ptr, len) {
        const ac = asyncCtrl && asyncCtrl.ac;
        if (ac && ac.exports.asyncify_get_state() === 2) {
          ac.suspend();
          return asyncCtrl.httpBytes ? asyncCtrl.httpBytes.length : 0xffffffff;
        }
        if (!ac) return 0xffffffff;  // no asyncify driver -> fetch unavailable
        const url = readStr(ptr, len);
        asyncCtrl.httpBytes = null;
        fetch(url)
          .then(async r => { asyncCtrl.httpBytes = r.ok ? new Uint8Array(await r.arrayBuffer()) : null; ac.resume('loft_start'); })
          .catch(() => { asyncCtrl.httpBytes = null; ac.resume('loft_start'); });
        ac.suspend();
        return 0;
      },
      loft_host_http_get_copy(ptr) {
        if (asyncCtrl && asyncCtrl.httpBytes)
          new Uint8Array(getMem().buffer, ptr, asyncCtrl.httpBytes.length).set(asyncCtrl.httpBytes);
      },
      // loft#678 — the RANGE arm of the same bridge, behind the working-set store
      // loaders (store_load_key / store_load_keys / store_load_key_text /
      // store_load_range): one `Range: bytes=off-(off+len-1)` GET instead of a whole
      // file, so a page reads the few pages a lookup touches out of a large hosted
      // store. Same two-phase asyncify shape as loft_host_http_get above, and it
      // shares the one response stash — safe because a suspend bridges a SYNCHRONOUS
      // loft call, so a second request cannot start before this one has rewound.
      // off/len arrive as plain numbers (the import declares f64: exact below 2^53).
      loft_host_http_range(ptr, len, off, n) {
        const ac = asyncCtrl && asyncCtrl.ac;
        if (ac && ac.exports.asyncify_get_state() === 2) {
          ac.suspend();
          return asyncCtrl.httpBytes ? asyncCtrl.httpBytes.length : 0xffffffff;
        }
        if (!ac) return 0xffffffff;  // no asyncify driver -> fetch unavailable
        const url = readStr(ptr, len);
        asyncCtrl.httpBytes = null;
        asyncCtrl.httpTotal = -1;
        const last = off + n - 1;
        fetch(url, { headers: { Range: `bytes=${off}-${last}` } })
          .then(async r => {
            // The total comes from Content-Range (`bytes a-b/TOTAL`) so size() needs
            // no second round trip; Content-Length is the fallback for a 200.
            const cr = r.headers.get('Content-Range');
            if (cr) { const t = cr.split('/').pop(); asyncCtrl.httpTotal = (t && t !== '*') ? Number(t) : -1; }
            else { const cl = r.headers.get('Content-Length'); asyncCtrl.httpTotal = cl ? Number(cl) : -1; }
            if (!r.ok) { asyncCtrl.httpBytes = null; }
            else {
              // 206 = the body IS the range. 200 = the server ignored Range and sent
              // the whole file; slice the window so the answer is right either way.
              const b = new Uint8Array(await r.arrayBuffer());
              asyncCtrl.httpBytes = (r.status === 206) ? b : b.subarray(off, off + n);
            }
            ac.resume('loft_start');
          })
          .catch(() => { asyncCtrl.httpBytes = null; ac.resume('loft_start'); });
        ac.suspend();
        return 0;
      },
      loft_host_http_range_total() {
        return asyncCtrl && asyncCtrl.httpTotal != null ? asyncCtrl.httpTotal : -1;
      },
      // @PLN105 Phase 2 — deliver(tag, value): the JS host receives the value's store base +
      // DbRef (store_base, rec, pos) + its layout descriptor (JSON), and reconstructs the value
      // with no serialization via readLoftValue (embedded reader — main.rs inlines
      // doc/loft-deliver.js AHEAD of this file). SYNCHRONOUS — read within this call, the borrow
      // ends on return (§5). A page observes deliveries by setting globalThis.loftDeliver(tag,
      // value, type_id).
      loft_host_deliver(tag, store_base, rec, pos, type_id, dptr, dlen) {
        const desc = JSON.parse(readStr(dptr, dlen));
        const value = readLoftValue(getMem(), store_base, desc, type_id, rec, pos);
        if (globalThis.loftDeliver) globalThis.loftDeliver(tag, value, type_id);
        else console.log("[loft:deliver]", tag, value);
      },
      // @PLN105 Phase 3 — expose/release: a long-lived deliver. Stash a RE-READER closure by tag
      // (loft pins the value's store so its addresses stay valid across frames); a page calls
      // globalThis.loftExposed.get(String(tag))() each frame for a fresh value (re-derives the view
      // from getMem().buffer — memory.grow-safe). release drops the stash.
      loft_host_expose(tag, store_base, rec, pos, type_id, dptr, dlen) {
        const desc = JSON.parse(readStr(dptr, dlen));
        const reread = () => readLoftValue(getMem(), store_base, desc, type_id, rec, pos);
        (globalThis.loftExposed || (globalThis.loftExposed = new Map())).set(String(tag), reread);
        if (globalThis.loftExpose) globalThis.loftExpose(tag, reread, type_id);
      },
      loft_host_release(tag) {
        if (globalThis.loftExposed) globalThis.loftExposed.delete(String(tag));
        if (globalThis.loftRelease) globalThis.loftRelease(tag);
      }
    }),
    loft_gl: coerceArgs({
      loft_gl_create_window(w, h, tp, tl) {
        dpr = window.devicePixelRatio || 1;
        canvas.width = toDevice(w); canvas.height = toDevice(h);
        // ⚠ THE CSS SIZE IS MANDATORY, not cosmetic.  Without it the canvas lays
        // out at its ATTRIBUTE size, so a 2x backing store draws a window twice
        // as wide on screen and every pointer coordinate is off by the ratio.
        canvas.style.width = w + 'px'; canvas.style.height = h + 'px';
        canvas.style.display = 'block';
        output.style.display = 'none';
        gl.viewport(0, 0, canvas.width, canvas.height);
        gl.enable(gl.DEPTH_TEST);
        shouldClose = false;
        return 1;
      },
      loft_gl_poll_events() { return shouldClose ? 0 : 1; },
      loft_gl_swap_buffers() {
        gl.flush();
        // Trigger asyncify suspension so the browser can render this frame.
        if (asyncCtrl && asyncCtrl.ac) asyncCtrl.ac.suspend();
      },
      loft_gl_clear(color) {
        const a = ((color >>> 24) & 0xff) / 255, r = ((color >>> 16) & 0xff) / 255;
        const g = ((color >>> 8) & 0xff) / 255, b = (color & 0xff) / 255;
        gl.clearColor(r, g, b, a);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      },
      loft_gl_destroy_window() {
        for (const p of programs) if (p) gl.deleteProgram(p);
        for (const v of vaos) if (v) { if (v.vao) gl.deleteVertexArray(v.vao);
                                       if (v.vbo) gl.deleteBuffer(v.vbo); }
        for (const t of textures) if (t) gl.deleteTexture(t);
        for (const f of fbos) if (f) gl.deleteFramebuffer(f);
        programs = []; vaos = []; textures = []; fbos = [];
      },
      loft_gl_create_shader(vp, vl, fp, fl) {
        const vertSrc = translateShader(readStr(vp, vl), false);
        const fragSrc = translateShader(readStr(fp, fl), true);
        const vs = gl.createShader(gl.VERTEX_SHADER);
        gl.shaderSource(vs, vertSrc); gl.compileShader(vs);
        if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
          console.error('Vertex:', gl.getShaderInfoLog(vs)); gl.deleteShader(vs); return 0;
        }
        const fs = gl.createShader(gl.FRAGMENT_SHADER);
        gl.shaderSource(fs, fragSrc); gl.compileShader(fs);
        if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
          console.error('Fragment:', gl.getShaderInfoLog(fs)); gl.deleteShader(vs); gl.deleteShader(fs); return 0;
        }
        const p = gl.createProgram();
        gl.attachShader(p, vs); gl.attachShader(p, fs); gl.linkProgram(p);
        gl.deleteShader(vs); gl.deleteShader(fs);
        if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
          console.error('Link:', gl.getProgramInfoLog(p)); gl.deleteProgram(p); return 0;
        }
        return hold(programs, p);
      },
      loft_gl_use_shader(p) { const o = slot(programs, p); if (o) gl.useProgram(o); },
      loft_gl_upload_vertices(ptr, count, stride) {
        const data = new Float32Array(getMem().buffer, ptr, count);
        const vao = gl.createVertexArray();
        gl.bindVertexArray(vao);
        const vbo = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
        gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
        const bpv = stride * 4;
        gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, bpv, 0);
        if (stride >= 6) { gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 3, gl.FLOAT, false, bpv, 12); }
        if (stride >= 8) { gl.enableVertexAttribArray(2); gl.vertexAttribPointer(2, 2, gl.FLOAT, false, bpv, 24); }
        if (stride >= 10) { gl.enableVertexAttribArray(2); gl.vertexAttribPointer(2, 4, gl.FLOAT, false, bpv, 24); }
        gl.bindVertexArray(null);
        return hold(vaos, { vao, vbo, n: count / stride });
      },
      // ── Instancing (@PLN144 A0 found these four absent) ───────────────────
      //
      // A0 built its batching probe with `--html` and the page reported all four
      // of these unimplemented: it BUILT, each returned its zero value, and the
      // stage drew nothing — the third time this shape has appeared (loft#737's
      // text stubs, E1's audio stubs).  A3's whole design is one instanced call
      // instead of N draws, so without these the browser half of the 2-D stack is
      // dead on arrival.
      //
      // An instance buffer is held in the SAME table as a VAO, as `{vao: null,
      // vbo, n}`.  That is not tidiness: `gl_update_buffer` documents its argument
      // as "a handle returned by gl_upload_vertices OR gl_upload_instance_buffer",
      // and two index spaces would make handle 1 ambiguous between them.
      loft_gl_upload_instance_buffer(ptr, count) {
        const data = new Float32Array(getMem().buffer, ptr, count);
        const vbo = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
        gl.bufferData(gl.ARRAY_BUFFER, data, gl.DYNAMIC_DRAW);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
        return hold(vaos, { vao: null, vbo, n: count });
      },
      loft_gl_instance_attrib(vaoIdx, ivboIdx, location, components, strideFloats, offsetFloats) {
        const v = slot(vaos, vaoIdx), b = slot(vaos, ivboIdx);
        if (!v || !v.vao || !b) return;
        gl.bindVertexArray(v.vao);
        gl.bindBuffer(gl.ARRAY_BUFFER, b.vbo);
        gl.vertexAttribPointer(location, components, gl.FLOAT, false,
                               strideFloats * 4, offsetFloats * 4);
        gl.enableVertexAttribArray(location);
        gl.vertexAttribDivisor(location, 1);   // the line that makes it per-INSTANCE
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
        gl.bindVertexArray(null);
      },
      // Block until every GL command has completed — the call that makes a
      // read-back trustworthy.  WebGL has no glFinish that is meaningfully
      // synchronous, so `finish()` is the honest best effort here and a page that
      // needs a hard guarantee reads through a fence instead.
      loft_gl_finish() { gl.finish(); },
      loft_gl_draw_instanced(vaoIdx, vertexCount, instanceCount) {
        const o = slot(vaos, vaoIdx);
        if (!o || !o.vao) return;
        gl.bindVertexArray(o.vao);
        gl.drawArraysInstanced(gl.TRIANGLES, 0, vertexCount, instanceCount);
        gl.bindVertexArray(null);
      },
      // Orphan + re-upload, matching the native contract's DYNAMIC_DRAW: a
      // per-frame update must not stall on the buffer still being read.
      loft_gl_update_buffer(h, ptr, count) {
        const o = slot(vaos, h);
        if (!o || !o.vbo) return;
        const data = new Float32Array(getMem().buffer, ptr, count);
        gl.bindBuffer(gl.ARRAY_BUFFER, o.vbo);
        gl.bufferData(gl.ARRAY_BUFFER, data, gl.DYNAMIC_DRAW);
        gl.bindBuffer(gl.ARRAY_BUFFER, null);
      },
      loft_gl_draw(vaoIdx, n) {
        const o = slot(vaos, vaoIdx); if (o) { gl.bindVertexArray(o.vao); gl.drawArrays(gl.TRIANGLES, 0, n); gl.bindVertexArray(null); }
      },
      loft_gl_draw_mode(v, n, m) {
        const o = slot(vaos, v); if (o) { gl.bindVertexArray(o.vao); gl.drawArrays(glMode(m), 0, n); gl.bindVertexArray(null); }
      },
      loft_gl_draw_elements(v, n, m) {
        const o = slot(vaos, v); if (o) { gl.bindVertexArray(o.vao); gl.drawElements(glMode(m), n, gl.UNSIGNED_INT, 0); gl.bindVertexArray(null); }
      },
      loft_gl_draw_fullscreen_quad() {
        if (!_fsQuad) {
          const v = gl.createVertexArray(); gl.bindVertexArray(v);
          const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b);
          gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,0,0, 1,-1,1,0, -1,1,0,1, 1,-1,1,0, 1,1,1,1, -1,1,0,1]), gl.STATIC_DRAW);
          gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
          gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);
          gl.bindVertexArray(null); _fsQuad = v;
        }
        gl.bindVertexArray(_fsQuad); gl.drawArrays(gl.TRIANGLES, 0, 6); gl.bindVertexArray(null);
      },
      loft_gl_set_mat4(prog, np, nl, mp, mc) {
        const o = slot(programs, prog);
        if (o) {
          const name = readStr(np, nl);
          const f64 = new Float64Array(getMem().buffer, mp, mc < 16 ? 0 : 16);
          const f32 = new Float32Array(16); for (let i = 0; i < 16; i++) f32[i] = f64[i];
          const loc = gl.getUniformLocation(o, name);
          if (loc) gl.uniformMatrix4fv(loc, false, f32);
        }
      },
      loft_gl_set_uniform_float(p, np, nl, v) {
        const o = slot(programs, p); if (o) { const loc = gl.getUniformLocation(o, readStr(np, nl)); if (loc) gl.uniform1f(loc, v); }
      },
      loft_gl_set_uniform_int(p, np, nl, v) {
        const o = slot(programs, p); if (o) { const loc = gl.getUniformLocation(o, readStr(np, nl)); if (loc) gl.uniform1i(loc, v); }
      },
      loft_gl_set_uniform_vec3(p, np, nl, x, y, z) {
        const o = slot(programs, p); if (o) { const loc = gl.getUniformLocation(o, readStr(np, nl)); if (loc) gl.uniform3f(loc, x, y, z); }
      },
      loft_gl_enable(c) { gl.enable(glCap(c)); },
      loft_gl_disable(c) { gl.disable(glCap(c)); },
      loft_gl_blend_func(s, d) { gl.blendFunc(glBF(s), glBF(d)); },
      loft_gl_cull_face(f) { gl.cullFace(f === 1 ? gl.FRONT : gl.BACK); },
      loft_gl_depth_mask(w) { gl.depthMask(!!w); },
      loft_gl_viewport(x, y, w, h) { gl.viewport(x, y, w, h); },
      // ⚠ Scissor was never bridged AT ALL.  An absent import is a dead page
      // (loft#668), so every `--html` program that clipped anything — every
      // `stage` scene using A6's clip, P6's viewports or L2's composite — failed
      // to instantiate rather than rendering without the clip.
      loft_gl_scissor(x, y, w, h) { gl.scissor(x, y, w, h); },
      loft_gl_line_width(w) { gl.lineWidth(w); },
      loft_gl_point_size(_s) { /* use gl_PointSize in shader */ },
      loft_gl_create_framebuffer() { return hold(fbos, gl.createFramebuffer()); },
      loft_gl_bind_framebuffer(i) { gl.bindFramebuffer(gl.FRAMEBUFFER, slot(fbos, i)); },
      loft_gl_framebuffer_texture(fi, att, ti) {
        const fb = slot(fbos, fi), tex = slot(textures, ti);
        if (fb && tex) {
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.framebufferTexture2D(gl.FRAMEBUFFER, att === 1 ? gl.DEPTH_ATTACHMENT : gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
        }
      },
      loft_gl_create_depth_texture(w, h) {
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT24, w, h, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        return hold(textures, t);
      },
      loft_gl_create_color_texture(w, h) {
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        return hold(textures, t);
      },
      // 0 = failure, the same sentinel every other handle here uses (loft#669) — and
      // never a valid handle, since `hold` is 1-based, so a caller CAN tell the two
      // apart.  loft#738: a BUNDLED asset works here. `--html` embeds every `.png`
      // sibling of the entry file and `decodeLoftAssets` decodes them to raw pixels
      // BEFORE `loft_start`, so the lookup is synchronous and no fetch is involved.
      // Only a path with no bundled asset — a runtime URL — is still unsupported;
      // that one genuinely needs async, and it reports failure rather than pretending.
      loft_gl_load_texture(pp, pl) {
        const name = readStr(pp, pl).split(/[\\/]/).pop();
        const a = (ctrl && ctrl.assets) ? ctrl.assets[name] : null;
        if (!a || !a.bytes || a.width <= 0 || a.height <= 0) return 0;
        // The asset table holds RGBA already, which is what GL wants — the copy
        // that rebuilt it here forced every alpha to 255. C58: no upload-side Y flip.
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, a.width, a.height, 0, gl.RGBA, gl.UNSIGNED_BYTE, a.bytes);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        return hold(textures, t);
      },
      loft_gl_upload_canvas(ptr, count, w, h) {
        // C58: no upload-side Y flip; canvas-top = GL TC.y=0.
        //
        // A `vector<integer>` marshals to this import as `*const i64` — codegen picks the
        // element width from the vector's STORAGE STRIDE (`vector_elem_rust_type`, @P310),
        // and a plain `integer` vector strides 8 bytes with the packed 0xAARRGGBB colour in
        // the LOW half.  Reading it at i32 stride took every other 4-byte word, so half the
        // "pixels" were a neighbour's zero high-half — transparent black.  The native
        // backend documents fixing exactly this at the 2c migration ("moiré on textured
        // surfaces, missing pixels on rasterised text"); this bridge still had the old read.
        // `count` is the ELEMENT count, so the word view is twice as long.
        const data = new Int32Array(getMem().buffer, ptr, count * 2);
        const px = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) {
          for (let x = 0; x < w; x++) { const c = data[(y * w + x) * 2], di = (y * w + x) * 4;
            px[di] = (c>>>16)&0xff; px[di+1] = (c>>>8)&0xff; px[di+2] = c&0xff; px[di+3] = (c>>>24)&0xff;
          }
        }
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, px);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        return hold(textures, t);
      },
      loft_gl_bind_texture(ti, u) { gl.activeTexture(gl.TEXTURE0 + u); const o = slot(textures, ti); if (o) gl.bindTexture(gl.TEXTURE_2D, o); },
      loft_gl_delete_texture(ti) { const o = slot(textures, ti); if (o) { gl.deleteTexture(o); textures[ti - 1] = null; } },
      loft_gl_delete_shader(p) { const o = slot(programs, p); if (o) { gl.deleteProgram(o); programs[p - 1] = null; } },
      loft_gl_delete_vao(v) { const o = slot(vaos, v); if (o) { gl.deleteVertexArray(o.vao); gl.deleteBuffer(o.vbo); vaos[v - 1] = null; } },
      loft_gl_delete_framebuffer(fi) { const o = slot(fbos, fi); if (o) { gl.deleteFramebuffer(o); fbos[fi - 1] = null; } },
      loft_gl_key_pressed(k) { return keys.has(k) ? 1 : 0; },
      loft_gl_mouse_x() { return mouseX; },
      loft_gl_mouse_y() { return mouseY; },
      loft_gl_mouse_button() { return mouseBtn; },
      // The canvas IS the window here, so its size is the window's inner size in
      // physical pixels — exactly what these are documented to report.  They used to be
      // absent, and an absent import is not a missing feature but a dead page: the whole
      // module fails to instantiate (loft#668).
      loft_gl_window_width() { return canvas.width; },
      loft_gl_window_height() { return canvas.height; },
      // Delta since the last call, reset on read.
      loft_gl_mouse_wheel() { const w = Math.trunc(wheelAcc); wheelAcc -= w; return w; },
      // `lib/graphics` already documents this as non-fatal outside a user gesture in the
      // browser, which is exactly what a rejected requestFullscreen() promise is.
      loft_gl_set_fullscreen(on) {
        try {
          if (on) { const p = canvas.requestFullscreen?.(); if (p) p.catch(() => {}); }
          else if (document.fullscreenElement) { const p = document.exitFullscreen?.(); if (p) p.catch(() => {}); }
        } catch (e) { /* no gesture / not permitted — non-fatal by contract */ }
      },
      // loft#737 — the text bridge. A font PATH becomes a CSS family (see `familyFor`);
      // the handle is a 0-BASED index, matching the desktop backend, so 0 is a valid
      // font and the null sentinel is i32::MIN.
      loft_gl_load_font(pp, pl) {
        const path = readStr(pp, pl);
        if (!path) return -2147483648;
        const base = path.split(/[\\/]/).pop().replace(/\.[^.]*$/, '');
        // @PLN146 F5 — a font that did not resolve draws perfectly well, in the
        // wrong face, for ever.  Report it once, here, where the page can still be
        // told which family it asked for; `globalThis.loftFonts` records every
        // resolution so a page (or a gate) can read what it actually got.
        const resolved = familyResolves(base);
        if (!resolved) {
          console.warn('[loft] font "' + base + '" is not available to this page — text '
            + 'draws in ' + genericFor(base) + '.  Declare it with `[[font]]` in loft.toml '
            + '(family must equal "' + base + '"), or name a family the browser has.');
        }
        fonts.push({ family: familyFor(base), base: base, resolved: resolved });
        (globalThis.loftFonts || (globalThis.loftFonts = [])).push(
          { base: base, family: familyFor(base), resolved: resolved });
        return fonts.length - 1;
      },
      loft_gl_measure_text(fi, tp, tl, sz) {
        if (!fonts[fi]) return 0.0;
        return fontMetrics(fi, sz).cx.measureText(readStr(tp, tl)).width;
      },
      // The line height a rasterised bitmap gets. The `sz * 1.2` fallback is what the
      // desktop backend also falls back to when a font exposes no usable metrics.
      loft_text_height(fi, sz) {
        if (!fonts[fi]) return Math.ceil(sz * 1.2);
        return fontMetrics(fi, sz).line;
      },
      // @P340 — baseline → top of glyphs, so callers can baseline-align mixed sizes.
      loft_gl_font_ascent(fi, sz) {
        if (!fonts[fi]) return sz * 0.8;
        return fontMetrics(fi, sz).asc;
      },
      // Writes alpha (0-255) into a loft `vector<integer>` and returns the bitmap
      // WIDTH — the caller sized its buffer from `measure_text` + `text_height` and
      // indexes rows by this width. `integer` is i64 storage, so each element is two
      // 32-bit words (little-endian: low word first, high word 0 for 0-255).
      loft_rasterize_text_into(fi, tp, tl, sz, bp, bc) {
        const r = rasterText(fi, readStr(tp, tl), sz);
        if (!r) return 0;
        const words = new Int32Array(getMem().buffer, bp, bc * 2);
        const n = Math.min(r.w * r.h, bc);
        for (let i = 0; i < n; i++) {
          words[i * 2] = r.px[i * 4 + 3];
          words[i * 2 + 1] = 0;
        }
        return r.w;
      },
      loft_save_png(pp, pl, w, h, dp, dc) { return 0; },
      // @lib_plan-29 W1d — generic asset-table existence check; used
      // by `database::io::get_file` so file().png() (and any future
      // asset-using library) sees auto-discovered PNGs as TextFile
      // instead of NotExists.  Library-specific imaging fns
      // (imaging_query / imaging_copy_rgb / imaging_save) live in
      // their package's own host.js — see lib/imaging/wasm/host.js,
      // concatenated into the HTML preamble by `--html` via the
      // `[wasm.bridge].host_js` manifest key.
      host_asset_exists(pp, pl) {
        const name = readStr(pp, pl).split(/[\\/]/).pop();
        return (ctrl.assets && ctrl.assets[name]) ? 1 : 0;
      },
      // loft#738 — an 8-bit coverage buffer the PROGRAM computed, uploaded as a
      // texture. The data is already in wasm memory, so this needs no fetch and no
      // asset pipeline; it is the route for any CPU-rasterised overlay (a glyph atlas,
      // a mask, a generated ramp) and the one that was missing entirely.
      loft_gl_upload_alpha_texture(dp, w, h) {
        if (w <= 0 || h <= 0) return 0;
        const a = new Uint8Array(getMem().buffer, dp, w * h);
        return alphaTexture(i => a[i], w, h);
      },
      // Rasterise + upload in one step, reporting the size through the two out-params
      // (i32, as the desktop backend writes them).
      loft_gl_text_texture(fi, tp, tl, sz, wp, hp) {
        const r = rasterText(fi, readStr(tp, tl), sz);
        const out = new Int32Array(getMem().buffer);
        if (!r) {
          if (wp) out[wp >> 2] = 0;
          if (hp) out[hp >> 2] = 0;
          return 0;
        }
        if (wp) out[wp >> 2] = r.w;
        if (hp) out[hp >> 2] = r.h;
        return alphaTexture(i => r.px[i * 4 + 3], r.w, r.h);
      },
      // G5: Audio via Web Audio API (@PLN146 E1).
      //
      // ⚠ `audio_load` is SYNCHRONOUS and `decodeAudioData` is not.  The handle is
      // therefore returned NOW and the samples land LATER, which is the same
      // plan-then-use shape the asset store runs on: a play before the decode
      // DROPS rather than queueing, because a queue here would fire a sound at a
      // moment the game has moved past.  A load that fails leaves the slot with no
      // buffer for good, so `audio_play` refuses it — the file's absence is
      // reported by refusing to play it, never by the load call, which cannot know.
      loft_audio_load(pp, pl) {
        const path = readStr(pp, pl);
        if (!path) return -2147483648;               // i32::MIN — the null sentinel
        const slot = { buf: null, dead: false };
        audioClips.push(slot);
        const id = audioClips.length - 1;
        fetch(path)
          .then(r => (r.ok ? r.arrayBuffer() : Promise.reject(new Error('HTTP ' + r.status))))
          .then(ab => audioCtx().decodeAudioData(ab))
          .then(b => { slot.buf = b; })
          .catch(() => { slot.dead = true; });
        return id;
      },
      //
      // `looping`, `pan` and `start` (@PLN146 E2) are the four things a game
      // decides at the moment a sound starts, and the native side takes ITS
      // answers from here: `StereoPannerNode`'s pan law is specified where rodio's
      // mixer has no pan at all, and `start(when, offset)` under `loop` repeats
      // from the top, which is why an offset there is a first-pass-only skip on
      // both targets.  A caller from before this existed passes two arguments and
      // the defaults below make that mean what it meant.
      loft_audio_play(clip, volume, looping, pan, start) {
        const c = audioClips[clip];
        if (!c || !c.buf) return -1;                 // absent, failed, or still decoding
        try {
          const sink = { src: null, gain: null, panner: null, buf: c.buf,
                         loop: !!looping, vol: volume };
          audioSinks.push(sink);
          const id = audioSinks.length - 1;         // ids are never reused: a stopped
          if (!startSink(sink, pan || 0, start || 0)) return -1;   // sink must not name a live one
          return id;
        } catch (e) { return -1; }
      },
      loft_audio_stop(sink) {
        const s = audioSinks[sink];
        if (!s) return;
        stopSource(s);
        audioSinks[sink] = null;
      },
      // Every sound at once — a pause menu, a scene change, a game over.  The
      // slots are cleared, so no id survives this: a stop-all a caller can undo by
      // reusing an old handle is not one.
      loft_audio_stop_all() {
        for (let i = 0; i < audioSinks.length; i++) {
          const s = audioSinks[i];
          if (!s) continue;
          stopSource(s);
          audioSinks[i] = null;
        }
      },
      loft_audio_set_volume(sink, volume) {
        const s = audioSinks[sink];
        if (!s) return;
        s.vol = volume;
        if (s.gain) s.gain.gain.value = volume;
      },
      // Move a playing sound across the stereo field: -1 left, 0 centre, 1 right.
      loft_audio_set_pan(sink, pan) {
        const s = audioSinks[sink];
        if (!s || !s.panner) return;
        s.panner.pan.value = Math.max(-1, Math.min(1, pan));
      },
      // Jump a playing sound to `seconds` from its start.
      //
      // An `AudioBufferSourceNode` cannot be moved once started — it is a
      // one-shot — so this stops the current one and starts another at the offset
      // through the SAME gain and panner.  The sink id therefore survives a seek,
      // which is what makes it a seek rather than a stop plus a play.
      loft_audio_seek(sink, seconds) {
        const s = audioSinks[sink];
        if (!s || !s.buf) return 0;
        const pan = s.panner ? s.panner.pan.value : 0;
        stopSource(s);
        return startSink(s, pan, seconds) ? 1 : 0;
      },
      loft_audio_play_raw(ptr, count, sample_rate, volume) {
        try {
          const ctx = audioCtx();
          const f32 = new Float32Array(getMem().buffer, ptr, count);
          const buf = ctx.createBuffer(1, count, sample_rate);
          buf.getChannelData(0).set(f32);
          const src = ctx.createBufferSource();
          src.buffer = buf;
          const gain = ctx.createGain();
          gain.gain.value = volume;
          src.connect(gain);
          gain.connect(ctx.destination);
          src.start();
          return 0;
        } catch(e) { return -1; }
      },
      ...ggImports(),
    }),   // ← close coerceArgs() for loft_gl
    env: {}
  };
}
