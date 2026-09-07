#!/bin/bash
# Start `make ci` detached, or report the verdict of the run that is (or was) going.
#
# Why this exists: reading `result.txt` for a `CI-RESULT` line is not a reliable
# completion signal, and three different ways of being wrong showed up in one day:
#
#   * a run that DIES — an oomd kill, a link error, a Ctrl-C — never writes a verdict, so
#     a waiter blocks forever on a process that is already gone;
#   * a verdict from a PREVIOUS run is still in the file while the next one is compiling,
#     so `grep -q CI-RESULT` answers "finished" about the wrong run;
#   * `.ci-running` is left behind by a killed run, so its presence means nothing.
#
# The fix is to record the run's own identity and check the PROCESS, not the log.
# `.ci-verdict` holds one line: STATE PID EPOCH [detail].  `status` re-reads the pid, so a
# run that vanished reports DIED rather than RUNNING.
# ⚠ EDITING THIS FILE: the whole `start` wrapper below is ONE single-quoted `bash -c`
# string, so an APOSTROPHE anywhere inside it — including in a comment — ends that string.
# The failure does not point at your line: `bash -n` reports a syntax error wherever the
# NEXT quote happens to be, which was 25 lines away in an unrelated comment.  Write "the lint
# doc URL", never "the lint's doc URL".  This warning is up here rather than beside the code
# because the apostrophe gets written before anyone reads the middle of the file.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
V=.ci-verdict

case "${1:-status}" in
  start)
    if [ -f $V ] && read -r st pid _ < $V && [ "$st" = RUNNING ] && kill -0 "$pid" 2>/dev/null; then
      echo "already RUNNING (pid $pid) — refusing to start a second gate"; exit 1
    fi
    for p in $(pgrep -f "make ci" 2>/dev/null); do
      cwd=$(readlink "/proc/$p/cwd" 2>/dev/null)
      [ -n "$cwd" ] && [ "$cwd" != "$PWD" ] && echo "note: another gate is running in $cwd — expect ~2x wall time"
    done
    # Three ways this ends, and each writes a verdict so the waiter never guesses:
    #   * make exits 0 / non-zero        → PASSED / FAILED
    #   * make is killed by a signal     → rc > 128, so the signal is rc-128 (the shape an
    #                                      oomd kill takes when the CHILD dies and this
    #                                      wrapper survives)
    #   * THIS wrapper is signalled      → the trap writes KILLED before dying.  TERM, INT,
    #                                      HUP and QUIT are all catchable; only SIGKILL and
    #                                      SIGSTOP are not, which is why `status` still
    #                                      re-reads the pid as a last resort.
    #   * WHO sent the signal.  A signal names its sender only to the process that
    #     receives it, and `make` prints "Terminated" and nothing more — which is exactly
    #     the state two gates on this box died in on 2026-09-04, with no OOM record in the
    #     journal and every checkout's tooling killing by recorded pid only.  So when
    #     `strace` is on the PATH, `make` runs under it tracing SIGNALS ONLY (`-e trace=none`
    #     costs nothing measurable on one process, and children are not followed): every
    #     delivered signal lands in `target/gate-signals.log` as
    #     `--- SIGTERM {si_signo=SIGTERM, si_code=SI_USER, si_pid=N, si_uid=U} ---`, and the
    #     wrapper snapshots the process table into `target/gate-killer-snapshot.txt` the
    #     moment `make` dies, while the sender is most likely still alive to be named.
    #     `si_code` separates a user `kill` (SI_USER) from the kernel (SI_KERNEL) and a
    #     timer or the OOM killer.  `CI_NO_TRACE=1` opts out.
    #
    # The `setsid` is load-bearing beside the traps: a `make ci` run as an agent tool's
    # background task is a child of that tool's process tree and dies with whatever stops
    # that tree; started here it is its own session and only an addressed signal reaches it.
    setsid nohup bash -c '
      s=$(date +%s)
      note() { echo "$1 $$ $(date +%s) $(( $(date +%s) - s ))s $2" > .ci-verdict; }
      snapshot() { mkdir -p target; ps -eo pid,ppid,pgid,sid,uid,etimes,comm,args > target/gate-killer-snapshot.txt 2>/dev/null; }
      for sg in TERM INT HUP QUIT; do
        trap "snapshot; note KILLED \"the gate wrapper received SIG$sg\"; exit 1" $sg
      done
      if [ -z "${CI_NO_TRACE:-}" ] && command -v strace >/dev/null 2>&1; then
        mkdir -p target
        strace -o target/gate-signals.log -tt -e trace=none -e signal=all make ci > /dev/null 2>&1
      else
        make ci > /dev/null 2>&1
      fi
      rc=$?
      if   [ $rc -eq 0 ];   then note PASSED ""
      elif [ $rc -gt 128 ]; then
        snapshot
        sender=$(grep -oE "SIG[A-Z]+ \{[^}]*\}" target/gate-signals.log 2>/dev/null | tail -1)
        note KILLED "make ci died on signal $((rc-128))${sender:+ — $sender (target/gate-signals.log, target/gate-killer-snapshot.txt)}"
      else
        # loft#1448 — name the failing TEST, and say how many.  `grep -m1 "^error|FAIL ["`
        # took whichever came FIRST in the file, and a cargo error always precedes the test
        # run: a gate whose only failure was `doc_hygiene::quality_optional_table_matches_the
        # _audit` reported `error[E0425] … generate_register_from_loft_with_bridges`, a
        # REGISTRY package the branch had never touched, 1500 lines above the real failure.
        # Both agents on this box triaged that line and chased the cdylib.
        #
        # The COUNT is the other half, and it splits the kinds of red that need opposite
        # responses.  FAILED with a COUNT is the code, in a test.
        #
        # FAILED with ZERO is three things, not one, and reading it as "the box" is wrong two
        # times out of three: `make ci` is fmt -> clippy -> test, so a FORMAT or a LINT failure
        # never reaches a test and still reports zero.  Both are the CODE and both are fixed in
        # seconds; only what is left over — a stale rlib, a corrupt incremental cache, a full
        # disk — is the box.  Measured the hard way: two agents on this box lost time to the
        # old wording within one hour, one sent to the format phase and one to the target dir,
        # each while holding a clippy or rustfmt error in the same verdict line.
        #
        # The markers are unambiguous, so the verdict names which of the three it is: clippy
        # prints the lint doc URL, rustfmt prints `Diff in <path>`.  (No apostrophes in here:
        # this whole wrapper is one single-quoted `bash -c` string.)
        #
        # nextest prints `FAIL [   1.23s] (12/34) <binary> <test>` once per attempt, so the
        # retries of one test collapse under `sort -u`.
        ft=$(grep -oE "FAIL \[[^]]*\].*$" result.txt 2>/dev/null \
             | awk "{ print \$(NF-1) \"::\" \$NF }" | sort -u)
        n=$(printf "%s" "$ft" | grep -c . || true)
        if [ "${n:-0}" -gt 0 ]; then
          note FAILED "$n test(s) — $(printf "%s" "$ft" | head -3 | tr "\n" " " | head -c 200)"
        elif grep -q "rust-clippy/.*index\.html#" result.txt 2>/dev/null; then
          lint=$(grep -oE "index\.html#[a-z_]+" result.txt 2>/dev/null | head -1 | sed "s/.*#//")
          note FAILED "0 tests — CLIPPY, so THE CODE (lint ${lint:-?}): $(grep -m1 -E "^error: " result.txt 2>/dev/null | head -c 100)"
        elif grep -q "^Diff in " result.txt 2>/dev/null; then
          note FAILED "0 tests — RUSTFMT, so THE CODE: $(grep -c "^Diff in " result.txt 2>/dev/null) file(s) unformatted; run \`cargo fmt\`"
        else
          note FAILED "0 tests and neither fmt nor clippy — toolchain/box/target-dir: $(grep -m1 -E "^error" result.txt 2>/dev/null | head -c 120)"
        fi
      fi' >/dev/null 2>&1 &
    echo "RUNNING $! $(date +%s) started" > $V
    echo "gate started (pid $!)"
    ;;
  status)
    [ -f $V ] || { echo "NOT-STARTED"; exit 0; }
    read -r st pid epoch rest < $V
    if [ "$st" = RUNNING ] && ! kill -0 "$pid" 2>/dev/null; then
      # No verdict AND no process: the wrapper never got to write one, so it was SIGKILLed
      # (systemd-oomd's cgroup kill, or `kill -9`).  Every catchable end writes its own
      # verdict via the traps above, so reaching here really does mean the uncatchable case.
      echo "DIED after $(( $(date +%s) - epoch ))s — SIGKILL/uncatchable (oomd cgroup kill?); no verdict written"
      exit 2
    fi
    [ "$st" = RUNNING ] && echo "RUNNING ${rest} for $(( $(date +%s) - epoch ))s" || echo "$st $rest"
    ;;
  wait|notify)
    # Designed to be launched with the Bash tool's `run_in_background`: the harness
    # re-invokes the agent when a background command EXITS, so this exits exactly once —
    # the moment a verdict exists — and the agent stays free until then.  That is the
    # difference from a foreground wait, which blocks the agent and the user with it.
    #
    # It ends on DIED too, which a `grep CI-RESULT` waiter never does: a run killed by
    # oomd or a link failure writes no verdict, so the naive loop waits forever on a
    # process that is already gone.
    while :; do
      out=$("$0" status); rc=$?
      case "$out" in RUNNING*) sleep 15 ;; *) echo "$out"; exit $rc ;; esac
    done
    ;;
  *) echo "usage: ci-run.sh {start|status|wait}"; exit 1 ;;
esac
