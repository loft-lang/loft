<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# A ported script beside its original — the twin's fixtures

One directory per port under `tests/comparisons/scripts/<name>/`, holding what
`scripts/script_twin.sh` needs to run the original and the port on the same input and
compare stdout, stderr, the exit status and the written files byte for byte
([@PLN179 § The one invariant](../../../doc/claude/plans/179-scripts-in-loft/README.md)).
The originals themselves still live where their callers call them; they move here when the
plan closes.

| what | where |
|---|---|
| an input fed to both sides (`--stdin`) | `<name>/<file>` — `opl_points/sample.opl` |
| a recording of the tools both sides ask (`--replay`) | `<name>/replay/<case>/<NNN>-<tool>/{argv, stdin, stdout, stderr, code}` |
| the shims that answer the ORIGINAL from a recording | `bin/<tool>` → `replay_tool.sh`, put first on PATH by `--replay` |

The port reads a recording through `lib/process` (`LOFT_RUN_REPLAY`); the original reads the
same directory through the shim under the tool's name.  A recording is made by running the
original through the shim with `LOFT_RUN_RECORD=<dir>` set, in a scratch repository shaped
for the case, and committed; a call with no entry goes red on both sides rather than
reaching the live tool.  `scripts/script_twin.sh --self-test` is the proof the harness can
fail; `make script-twin ORIG=… PORT=… REPLAY=<case dir>` runs one case.
