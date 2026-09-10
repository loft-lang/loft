#!/usr/bin/env bash
# gate_lock.sh — ONE home for "who holds the box gate lock, and are they alive?"
#
# `make ci` serialises on /tmp/loft-gate.lock (fd 9) so one gate runs at full width.
# The waiter, `ci-run.sh status` and the doctor all need the SAME answer to that
# question, so it lives here once.  loft#1504 is what a second decoder costs: every
# reader of `.ci-running` already gates on `kill -0` (seven sites), the LOCK did not,
# and a gate blocked behind an orphan reported "QUEUED behind another gate on this
# box" — naming a gate that did not exist — for 48 and 20 minutes at load 0.05.
#
# The lock CANNOT be acquired from here: fd 9 must be held by the recipe's own shell
# for the gate's duration, and a script exiting would release it.  So acquisition
# stays inline in the Makefile and everything DIAGNOSTIC lives here.
#
# The holder file is inert by design, exactly like `.ci-running`: it is left behind by
# a killed run, so its presence means nothing and every reader gates on `kill -0`.
# What decides "held" is the LOCK, never the file.
set -u

LOCK=${LOFT_GATE_LOCK:-/tmp/loft-gate.lock}
HOLDER=${LOFT_GATE_HOLDER:-$LOCK.holder}

# Is the lock held right now?  Tested on a SEPARATE fd, opened `>>` so the test does
# not truncate the lock file, and released the instant the subshell exits.
lock_held() {
    ( flock -n 8 ) 8>>"$LOCK" 2>/dev/null && return 1
    return 0
}

# `<pid> <cwd>` of whoever last acquired it, or nothing.
holder_raw() { [ -f "$HOLDER" ] && cat "$HOLDER" 2>/dev/null; }

# FREE | HELD_LIVE <pid> <cwd> | HELD_ORPHAN <pid> <cwd>
#
# Derived from the KERNEL's own facts, never from the holder file.  The file is written by
# the gate that acquired the lock, so it is absent for any gate running an older Makefile
# and for one whose write lost a race — and reading its absence as "orphan" produces a
# confident verdict telling the operator to kill a LIVE gate.  Measured while building
# this: loft2's gate held the lock, mid-`cargo-nextest`, and the file-led version called
# it an orphan.  That is worse than the silence it replaces, so the file only ever
# supplies the human LABEL (which checkout), and `ppid` decides.
#
# The discriminator: a process whose gate died is reparented to init, so `ppid == 1`.  A
# holder still inside a live process tree has a real parent.  A holder is also accounted
# for if some checkout's `.ci-running` names it and that pid is alive.
holder_pids() {   # pid<TAB>ppid<TAB>cwd, one per process holding the lock on ANY fd
    local p fd t
    for p in $(ls /proc 2>/dev/null | grep -E '^[0-9]+$'); do
        for fd in /proc/$p/fd/*; do
            t=$(readlink "$fd" 2>/dev/null) || continue
            [ "$t" = "$LOCK" ] || continue
            printf '%s\t%s\t%s\n' "$p" "$(awk '{print $4}' /proc/$p/stat 2>/dev/null)" \
                "$(readlink /proc/$p/cwd 2>/dev/null)"
            break
        done
    done
}

live_claim() {    # is $1 named by a live .ci-running in any checkout?
    local d pid
    for d in . ../*/; do
        [ -f "$d/.ci-running" ] || continue
        pid=$(cat "$d/.ci-running" 2>/dev/null)
        [ "$pid" = "$1" ] && kill -0 "$pid" 2>/dev/null && return 0
    done
    return 1
}

state() {
    if ! lock_held; then echo FREE; return; fi
    local pid ppid cwd best_pid="" best_cwd="" any=""
    while IFS=$'\t' read -r pid ppid cwd; do
        [ -n "${pid:-}" ] || continue
        any=1
        if [ "${ppid:-1}" != 1 ] || live_claim "$pid"; then
            echo "HELD_LIVE $pid ${cwd:-?}"; return
        fi
        [ -z "$best_pid" ] && { best_pid=$pid; best_cwd=$cwd; }
    done <<EOF
$(holder_pids)
EOF
    # Held, but every holder is an orphan — or /proc showed us none at all, which still
    # means no running gate accounts for the lock.
    if [ -n "$any" ]; then echo "HELD_ORPHAN $best_pid ${best_cwd:-?}"
    else echo "HELD_ORPHAN ? ?"; fi
}

# Record this shell as the holder.  Called by the Makefile right after `flock 9`.
claim() { printf '%s %s\n' "${1:-$$}" "${2:-$(pwd -P)}" > "$HOLDER" 2>/dev/null || true; }

load1() { cut -d' ' -f1 /proc/loadavg 2>/dev/null || echo '?'; }

# One line for the waiter's loop — says whether the queue is REAL.
why() {
    local st; st=$(state)
    set -- $st
    case "$1" in
      FREE) echo "the lock is free (acquiring)";;
      HELD_LIVE) echo "held by a LIVE gate in $3 (pid $2); load $(load1)";;
      HELD_ORPHAN) echo "!! held by pid $2 ($3), an ORPHAN reparented to init — it inherited"\
                        "fd 9 from a gate that died (loft#1504); load $(load1)."\
                        "Run: scripts/ci-run.sh doctor";;
    esac
}

# Every process holding the lock file on any fd, with the facts that name an orphan.
fd_holders() {
    local p fd t comm ppid cwd et
    for p in $(ls /proc 2>/dev/null | grep -E '^[0-9]+$'); do
        for fd in /proc/$p/fd/*; do
            t=$(readlink "$fd" 2>/dev/null) || continue
            [ "$t" = "$LOCK" ] || continue
            comm=$(tr -d '\0' < /proc/$p/comm 2>/dev/null)
            ppid=$(awk '{print $4}' /proc/$p/stat 2>/dev/null)
            cwd=$(readlink /proc/$p/cwd 2>/dev/null)
            et=$(ps -o etimes= -p "$p" 2>/dev/null | tr -d ' ')
            printf '  pid %-8s fd %-3s %-18s ppid %-8s %5ss  %s%s\n' \
                "$p" "$(basename "$fd")" "$comm" "$ppid" "${et:-?}" "${cwd:-?}" \
                "$([ "$ppid" = 1 ] && echo '   <== ORPHAN (reparented to init)')"
            break
        done
    done
}

doctor() {
    echo "== gate lock doctor — $LOCK =="
    echo "state    : $(state)"
    echo "why      : $(why)"
    echo "load      : $(cat /proc/loadavg 2>/dev/null | cut -d' ' -f1-3)"
    echo "holder file: $(holder_raw || echo '(none)')  [inert — presence means nothing]"
    echo
    echo "processes holding the lock file (any fd; fd 9 is the gate's):"
    local h; h=$(fd_holders)
    [ -n "$h" ] && echo "$h" || echo "  (none)"
    echo
    echo "gate claims per checkout (.ci-running, gated on kill -0):"
    local d pid
    for d in . ../*/; do
        [ -f "$d/.ci-running" ] || continue
        pid=$(cat "$d/.ci-running" 2>/dev/null)
        printf '  %-28s pid %-8s %s\n' "$(cd "$d" && pwd -P)" "$pid" \
            "$(kill -0 "$pid" 2>/dev/null && echo ALIVE || echo DEAD)"
    done
    echo
    case "$(state)" in
      HELD_ORPHAN*)
        echo "VERDICT: the lock is held by a process that is NOT a running gate."
        echo "  An orphan above (ppid 1) inherited fd 9 from a gate that died."
        echo "  Killing that pid releases every queued gate on this box.";;
      HELD_LIVE*)
        echo "VERDICT: a real gate holds the lock — this is an ordinary queue.";;
      FREE)
        echo "VERDICT: the lock is free; a gate that is not running is stalled elsewhere.";;
    esac
}


# ---- selftest --------------------------------------------------------------------------
#
# Run by `make ci` beside the other script self-tests.  The load-bearing cells are 3 and 4:
# a lock held with no holder record is the SAME observation for a live gate and for an
# orphan, and they need opposite verdicts.  Getting it backwards tells an operator to kill
# a running gate — measured against loft2's live gate while this was being written, which
# is why cell 3 is permanent.
selftest() {
    local L; L=$(mktemp -d)/gate.lock
    export LOFT_GATE_LOCK=$L LOFT_GATE_HOLDER=$L.holder
    local p=0 f=0
    local LOG=""
    say(){ LOG="$LOG$1
"; }
    chk(){ if [[ "$3" == "$2"* ]]; then say "  ok   $1 -> $3"; p=$((p+1));
           else say "  FAIL $1 -> got '$3' want '$2'*"; f=$((f+1)); fi; }
    clean(){ rm -f "$L" "$L.holder" "$L.ready" "$L.pid"; : >>"$L"; }
    # a normal child holder (has a real parent)
    hold(){ ( flock 8; echo $BASHPID >"$L.pid"; touch "$L.ready"; sleep "${1:-90}" ) 8>>"$L" &
            while [ ! -f "$L.ready" ]; do sleep 0.05; done; HP=$(cat "$L.pid"); }
    # an ORPHAN holder: the intermediate parent exits, so the holder is reparented to init
    orphan(){ ( ( flock 8; echo $BASHPID >"$L.pid"; touch "$L.ready"; sleep "${1:-90}" ) 8>>"$L" & )
              while [ ! -f "$L.ready" ]; do sleep 0.05; done; OP=$(cat "$L.pid"); }

    clean; say "cell 1: lock FREE"
    chk state FREE "$("$0" state)"

    say "cell 2: LIVE holder, holder file names its checkout"
    hold; printf '%s %s\n' "$HP" "/home/jurjens/workspace/loft2" > "$L.holder"
    chk state "HELD_LIVE $HP" "$("$0" state)"
    chk why "held by a LIVE gate" "$("$0" why)"

    say "cell 3 (CONTROL — the false positive): LIVE holder, NO holder file -> still LIVE"
    rm -f "$L.holder"
    chk state "HELD_LIVE $HP" "$("$0" state)"
    case "$("$0" doctor)" in *"ordinary queue"*) say "  ok   doctor calls it an ordinary queue"; p=$((p+1));;
      *) say "  FAIL doctor mislabels a live gate"; f=$((f+1));; esac
    kill "$HP" 2>/dev/null; sleep 0.3

    say "cell 4: ORPHAN holder (ppid 1), no holder file -> ORPHAN  <-- loft#1504"
    clean; orphan
    ppid=$(awk '{print $4}' /proc/$OP/stat 2>/dev/null)
    say "         (holder pid $OP has ppid $ppid)"
    chk state "HELD_ORPHAN $OP" "$("$0" state)"
    case "$("$0" why)" in *"ORPHAN reparented to init"*) say "  ok   why names the orphan"; p=$((p+1));;
      *) say "  FAIL why: $("$0" why)"; f=$((f+1));; esac
    case "$("$0" doctor)" in *"NOT a running gate"*) say "  ok   doctor verdict actionable"; p=$((p+1));;
      *) say "  FAIL doctor"; f=$((f+1));; esac
    kill "$OP" 2>/dev/null; sleep 0.3

    say "cell 5 (CONTROL): stale holder file, lock FREE -> FREE"
    clean; printf '%s %s\n' "99999999" "/x" > "$L.holder"
    chk state FREE "$("$0" state)"

    say "cell 6: the held-test must not truncate the lock file"
    echo sentinel > "$L"; "$0" state >/dev/null; chk "intact" sentinel "$(cat "$L")"
    rm -f "$L" "$L.holder" "$L.ready" "$L.pid"
    if [ "$f" -eq 0 ]; then
        echo "gate_lock selftest: $p cells OK"
    else
        printf '%s' "$LOG" >&2
        echo "gate_lock selftest: $f of $((p+f)) cells FAILED" >&2
    fi
    [ "$f" -eq 0 ]
}

case "${1:-doctor}" in
    state) state;; why) why;; holder) holder_raw;; claim) shift; claim "$@";;
    held) lock_held && echo held || echo free;;
    doctor) doctor;;
    selftest) selftest;;
    *) echo "usage: $0 {state|why|holder|claim|held|doctor|selftest}" >&2; exit 2;;
esac
