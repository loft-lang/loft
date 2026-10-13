#!/usr/bin/env python3
"""@PLN146 F6 — a static file server that answers font files SLOWLY.

The readiness gate asks whether the emitted `document.fonts.load` await genuinely
holds `loft_start`, and on a fast local server it cannot: the font may arrive
before the program looks at it whether the page waited or not, so the control
(the same page with the await removed) would pass and the gate would be measuring
nothing.  Delaying the font bytes makes the control's failure certain instead of
a race.

Everything else — the page, the script, the stylesheet — is served at once, so
the delay is on the one thing under test.

Each connection is answered on its own thread.  Served one at a time, a browser
connection that sends nothing holds every other request until it closes, and two
delayed fonts wait one after the other — both are time the page under test is
charged for and did not spend.

  python3 tests/data/slow_font_server.py <dir> <port> <delay-ms>
"""
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT, PORT, DELAY_MS = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
ROOT_REAL = os.path.realpath(ROOT)


def _under_root(name):
    """The file `name` names inside ROOT, or None if it names anything else.

    `basename` already drops every directory component, so a traversal cannot be
    spelled — but a served path is still built from a request, and the check that
    proves it stayed inside belongs where the file is opened rather than in a
    reader's head one function away.
    """
    full = os.path.realpath(os.path.join(ROOT_REAL, name))
    if full != ROOT_REAL and not full.startswith(ROOT_REAL + os.sep):
        return None
    return full


TYPES = {
    ".html": "text/html; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".ttf": "font/ttf",
    ".woff2": "font/woff2",
}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_GET(self):                                       # noqa: N802 (stdlib name)
        name = os.path.basename(self.path.split("?")[0].lstrip("/"))
        full = _under_root(name)
        if not name or full is None or not os.path.isfile(full):
            self.send_error(404)
            return
        ext = os.path.splitext(name)[1].lower()
        if ext in (".ttf", ".woff2", ".woff", ".otf"):
            time.sleep(DELAY_MS / 1000.0)
        with open(full, "rb") as fh:
            body = fh.read()
        self.send_response(200)
        self.send_header("Content-Type", TYPES.get(ext, "application/octet-stream"))
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)


ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
