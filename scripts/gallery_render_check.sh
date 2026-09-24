#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# gallery_render_check.sh — does the built gallery actually RENDER?
#
# `make gallery`'s other steps are structural: the files are present, the glue
# and the wasm come from one build, every asset HEAD-answers 200.  Every one of
# them passes over a page that loads cleanly and draws nothing, which is how a
# gallery whose only example fails to COMPILE shipped while the target printed
# `gallery ready` (loft#1545).
#
# So this step opens each example in headless Chrome and asserts the page's own
# status is not a failure state.  That is the signal the other layers cannot
# reach: `gallery-run.html` reports compile and runtime errors into its DOM
# (`#status`, `#output`) and never to `console.error`, so a console-only check
# records a clean run while the page shows the error in red.
#
# The assertion is a BOOLEAN on purpose.  `--assert` compares the JSON of the
# expression against the literal `true`, so `--assert "<status>.textContent"`
# fails on EVERY page — including a working one — and would look like a gate
# while proving nothing.
#
# `--ready` waits for a state the assertion is about.  Without it a page that is
# still compiling reads as a pass, and a page whose autorun never fired reports
# the timeout instead of a silent success.
#
# Exit 0 = every example rendered, or the check could not run (see SKIP).
# Exit 1 = an example reached a failure state; its report is printed.
#
# SKIPs rather than fails when node or a chrome binary is absent: not having a
# browser installed is not a broken gallery, and a step that fails for the
# environment gets deleted rather than fixed.
set -u

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root" || exit 1

harness="$root/tools/html_render_check.mjs"
chrome=""
for n in google-chrome chromium chromium-browser chrome; do
	if command -v "$n" >/dev/null 2>&1; then chrome="$n"; break; fi
done
if ! command -v node >/dev/null 2>&1; then
	echo "    SKIP: render check needs node"; exit 0
fi
if [ -z "$chrome" ]; then
	echo "    SKIP: render check needs a chrome binary (tried google-chrome, chromium, chromium-browser, chrome)"; exit 0
fi
if [ ! -f "$harness" ]; then
	echo "    SKIP: tools/html_render_check.mjs missing"; exit 0
fi

# Every example the gallery lists, so a newly added one is covered without
# editing this script — the failure mode loft#1545 describes is an example
# whose dependencies the browser bundle cannot resolve, and that arrives with
# the next example as easily as it did with this one.
keys=$(grep -oE 'key:[[:space:]]*"[^"]+"' doc/gallery-examples.js 2>/dev/null | sed 's/.*"\(.*\)"/\1/')
if [ -z "$keys" ]; then
	echo "    FAIL: no example keys in doc/gallery-examples.js — nothing would be checked"
	exit 1
fi

port=18766
# The `trap … EXIT` below releases the port on every path this script takes,
# including a failing example.  What it cannot cover is the script being KILLED:
# the server is then reparented to init and keeps the port bound, and the next
# run binds nothing and reports a failure that is the previous run's corpse.
# Measured 2026-09-16: 16 such orphans on this box, all `ppid=1`, the oldest
# 6480 s old, two of them on step 6's own port.  If that bites, the cure is to
# reap by port rather than to change how the pid is captured — `$!` is correct
# here and the subshell form in step 6 is not the cause.
python3 -m http.server "$port" --bind 127.0.0.1 --directory doc \
	>/tmp/loft_gallery_render.log 2>&1 &
server_pid=$!
cleanup() {
	kill "$server_pid" 2>/dev/null || true
	wait "$server_pid" 2>/dev/null || true
	rm -f /tmp/loft_gallery_render.log
}
trap cleanup EXIT
for _ in 1 2 3 4 5 6 7 8 9 10; do
	sleep 0.3
	if curl -s -o /dev/null "http://127.0.0.1:$port/gallery-run.html"; then break; fi
done

# The states `runExample` leaves behind once it has decided: it is running, it
# finished, or it failed.  Waiting for one of these is what makes the assertion
# an observation rather than a race against `Compiling...`.
ready="['Running','Ok','Done','Failed','Error'].includes(document.getElementById('status').textContent)"

# TWO questions, because the status alone answers only the first.
#
#   1. did the page refuse?  `Failed` / `Error` is `runExample` reporting a
#      compile error, an asset that did not arrive, or a runtime fault.
#   2. did the PROGRAM refuse?  A loft program that cannot load what it needs
#      keeps running and draws a fallback — `25-brick-buster` answers a 1x1
#      atlas and says so.  The page stays `Running`, the canvas fills with
#      colour, and every outside check passes over a page missing its sprites.
#      That is not hypothetical: the shipped `--html` build did exactly this,
#      and a screenshot of it carried 767 distinct colours.
#
# So the program's own words are the second half, and they are only visible
# because a running frame now carries them (`resume_frame` → `#output`).  The
# match is deliberately narrow — the words a loft program uses when it gives up
# — rather than a generic /error/i, which any legitimate output could trip.
assert="(function(){
  var s = document.getElementById('status').textContent;
  var o = (document.getElementById('output') || {}).textContent || '';
  var refused = /sprite pack|Asset load failed|could not load|not found/i.test(o);
  return !['Failed','Error'].includes(s) && !refused;
})()"

failed=0
for key in $keys; do
	url="http://127.0.0.1:$port/gallery-run.html?example=$key&autorun=1"
	# THREE questions, and each catches what the others cannot.
	#   --ready/--assert : did the page, or the program, report a refusal?
	#   --canvas         : did anything actually get DRAWN?  A page can reach
	#                      `Running` with a clean console over a canvas holding
	#                      nothing but its clear colour.
	# The canvas count alone is NOT sufficient and this is measured, not assumed:
	# the shipped Brick Buster drew 767 distinct colours while every one of its
	# sprites was missing, because the program falls back to procedural drawing
	# and says so in words rather than in pixels.  Colours answer "did it draw",
	# the assertion answers "did it draw what it meant to".
	out=$(node "$harness" "$url" --wait-ms 15000 --ready "$ready" --assert "$assert" \
		--canvas '#gl-canvas' --canvas-min-colors 20 2>&1)
	rc=$?
	if [ "$rc" = 2 ]; then
		echo "    SKIP: $key — $(printf '%s' "$out" | head -1)"
		continue
	fi
	if [ "$rc" != 0 ]; then
		echo "    FAIL: $key does not render — the page reports a failure state"
		# The whole record: its tail carries the page's own text, which says why.
		printf '%s\n' "$out" | head -80 | sed 's/^/      /'
		failed=$((failed + 1))
	fi
done

if [ "$failed" -gt 0 ]; then
	echo "    $failed example(s) build into a page that does not run."
	echo "    The gallery is what most users meet first; a bundle that loads and"
	echo "    draws nothing is not a successful build (loft#1545)."
	exit 1
fi
echo "    every gallery example reaches a running state in a real browser"
exit 0
