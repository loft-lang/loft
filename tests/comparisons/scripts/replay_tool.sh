#!/usr/bin/env bash
# @PLN179 strand 4c — the ORIGINAL's half of a recording: a shim installed under a tool's
# name (`bin/git`, `bin/gh`, …) that answers from the same directory `lib/process`'s `run`
# replays, so a twin's two sides consume the same bytes and never a live tool.
#
#   LOFT_RUN_REPLAY=<dir>  answer from <dir>/<NNN>-<tool>/{argv,stdout,stderr,code}: the first
#                          entry whose `argv` (one word per line, the tool first) equals this
#                          call's that has not been used yet — a repeated call walks its
#                          entries in order and stays on the last.  No entry: exit 127 and say
#                          so on stderr, so the twin goes red rather than running anything.
#   LOFT_RUN_RECORD=<dir>  run the real tool (the next one on PATH) and write the entry.
#
# A wrapper under bin/ is two lines: `exec "$(dirname "$0")/../replay_tool.sh" "$0" "$@"`.
# Stdin is not compared: the scripts this serves do not feed their tools.  Kept in bash with
# the harness: it must run while no loft binary exists.
set -u
# The wrapper under bin/ passes its own path first: the tool's name and the directory to
# skip when looking for the real one.
self="$1"; shift
tool=$(basename "$self")
here=$(cd "$(dirname "$self")" && pwd)
argv=$(printf '%s\n' "$tool" "$@")

if [ -n "${LOFT_RUN_REPLAY:-}" ]; then
  used_dir="${TMPDIR:-/tmp}/loft_replay_used_$$_${PPID}"
  mkdir -p "$used_dir"
  key=$(printf '%s' "$argv" | cksum | cut -d' ' -f1)
  n=0; [ -f "$used_dir/$key" ] && n=$(cat "$used_dir/$key")
  i=0; last=""
  for e in "$LOFT_RUN_REPLAY"/*/; do
    [ -f "$e/argv" ] || continue
    [ "$(cat "$e/argv")" = "$argv" ] || continue
    last="$e"
    if [ "$i" -eq "$n" ]; then break; fi
    i=$((i + 1))
  done
  if [ -z "$last" ]; then
    echo "$tool: no recording in $LOFT_RUN_REPLAY for: $tool $*" >&2; exit 127
  fi
  echo $((n + 1)) > "$used_dir/$key"
  cat "$last/stdout"; cat "$last/stderr" >&2
  exit "$(cat "$last/code")"
fi

# The real tool: the first on PATH that is not this shim's own directory.
real=""
IFS=: read -ra dirs <<< "$PATH"
for d in "${dirs[@]}"; do
  [ "$d" = "$here" ] && continue
  [ -x "$d/$tool" ] && { real="$d/$tool"; break; }
done
[ -n "$real" ] || { echo "$tool: not found on PATH beside the shim" >&2; exit 127; }

if [ -z "${LOFT_RUN_RECORD:-}" ]; then exec "$real" "$@"; fi
mkdir -p "$LOFT_RUN_RECORD"
n=$(find "$LOFT_RUN_RECORD" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')
e=$(printf '%s/%03d-%s' "$LOFT_RUN_RECORD" $((n + 1)) "$tool")
mkdir -p "$e"
printf '%s\n' "$argv" > "$e/argv"
: > "$e/stdin"
"$real" "$@" > "$e/stdout" 2> "$e/stderr"; code=$?
echo "$code" > "$e/code"
cat "$e/stdout"; cat "$e/stderr" >&2
exit "$code"
