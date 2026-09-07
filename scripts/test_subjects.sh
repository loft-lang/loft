#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Which tests a run selects, and why.  Sourced by scripts/find_problems.sh.
#
# ── The shape of the curation, and the reason for it ────────────────────────
#
# Curated by EXCLUSION, not by inclusion.  An additive map ("you touched the
# parser, so run these four suites") keeps 4 binaries of 177 and drops 173 — and
# it demonstrably misses real regressions: an over-broad change to the parser's
# `null()` was caught by `binary_io_matrix`, which no additive map would ever have
# selected for a parser edit.  Curation by inclusion has to predict which suite
# will catch a bug, which is exactly the thing nobody can do in advance.
#
# Excluding instead makes the miss set small, named and reviewable.  The cost is
# concentrated enough that this is nearly free: EIGHT binaries of 177 (4.5%) hold
# 57% of the work, because each is slow AND has very few tests.  Dropping those
# eight runs 3733 of 3833 tests in 67s instead of 367s.
#
#   measured 2026-08-07: full 3833 tests / 367s;  curated 3733 tests / 67s (5.5x)
#
# So the only way the default misses something is one of the eight below — and
# CI's `Test (ubuntu-latest)` job runs the suite UNSHARDED and is a required
# check, so nothing reaches main without them.  Re-measure the split with:
#   cargo nextest run --release --status-level pass | <sum per binary>
# when these numbers look stale.

# ── The excluded eight ──────────────────────────────────────────────────────
# Each is slow-and-few: minutes of wall time for a handful of tests.  Kept OUT of
# the default and pulled back IN automatically when a change touches what they
# cover (see SUBJECT_PATHS).  `--full` always runs them.
HEAVY_BINARIES=(
  deliver_wasm         # 1011s /  17 tests — cross-target delivery matrix
  ir_schema_roundtrip  #  360s /   8 tests — IR codec over every tests/scripts file
  exit_codes           #  210s /  26 tests — spawns a process per case
  codegen_emitter      #  187s /  21 tests — rustc per case
  multiplayer_v5       #  126s /   5 tests — networked, serialised
  engine_host_audience #   93s /   1 test  — one long host session
  html_wasm            #  114s /  20 tests — wasm-pack per case
)

# ── Subjects: a name, and the binary-name PATTERNS it selects ───────────────
# Patterns, not lists.  A hand-written list is incomplete the day it is written
# and gets worse: the first draft of this file listed binaries by hand and left
# 91 of 177 unreachable.  `~` is nextest's substring match, so a subject is a
# RULE and a new binary joins the subject whose name it already matches.
#
# Subjects are a convenience for tight loops, NOT the safety mechanism — the
# default run is subtractive, so a gap here costs seconds, never coverage.  That
# is the whole reason it is safe to keep them approximate.
# Spelled as a case-function, not `declare -A`: macOS ships bash 3.2 as BOTH
# /bin/sh and /bin/bash, which has no associative arrays — the array form made
# every `--subject` run on a Mac die with `parser: unbound variable` while the
# same script worked on every Linux box.  Same data, portable spelling.
SUBJECT_NAMES='parser scopes codegen runtime store wasm packages lsp sql docs host'

subject_patterns() {
  case "$1" in
    (parser)   echo '~pars ~expression ~error_messages ~suggestion ~strings ~spans ~tuple ~qq_null ~dn4 ~nullflow ~steer ~lint' ;;
    (scopes)   echo '~slot ~leak ~ownership ~use_analysis ~uaf ~frame_vars ~closure ~callarg ~alias' ;;
    (codegen)  echo '~codegen ~native ~n2_ ~n3_ ~g2_ ~ir_ ~introspect ~slots ~entry_signature' ;;
    (runtime)  echo '~wrap ~issues ~thread ~par ~coroutine ~runtime ~dispatch ~panic ~exit_codes ~crash' ;;
    (store)    echo '~store ~database ~data_ ~paged ~lazy ~field_without ~layout ~watermark ~binary_io' ;;
    (wasm)     echo '~wasm ~html ~deliver ~browser ~gl_ ~android' ;;
    (packages) echo '~registry ~package ~import ~api_ ~compat ~manifest ~extract ~resolution ~cache ~self_update' ;;
    (lsp)      echo '~lsp ~dap ~debugger ~repl' ;;
    (sql)      echo '~lazy_sql ~sql' ;;
    (docs)     echo '~doc ~features ~index_hygiene ~comment ~viewer' ;;
    (host)     echo '~engine_host ~host_ ~multiplayer ~serve ~rpc ~mock' ;;
    (*)        return 1 ;;
  esac
}

# ── Which subjects a changed PATH belongs to ────────────────────────────────
# Used by the default run to pull an excluded heavyweight back in.  A path that
# matches NOTHING widens to `--full` rather than narrowing — the fail-safe
# direction, since an unknown path is exactly where a guess is least reliable.
subject_paths() {
  case "$1" in
    (parser)   echo '^src/parser/|^src/lexer\.rs|^src/typedef\.rs|^src/variables/' ;;
    (scopes)   echo '^src/scopes\.rs|^src/use_analysis\.rs|^src/ownership_cfg\.rs' ;;
    (codegen)  echo '^src/generation/|^src/compile\.rs|^src/state/codegen\.rs|^src/codegen_runtime\.rs|^src/fill\.rs' ;;
    (runtime)  echo '^src/state/|^src/parallel\.rs|^src/fill\.rs' ;;
    (store)    echo '^src/store\.rs|^src/store_budget\.rs|^src/database/|^src/keys\.rs' ;;
    (wasm)     echo '^src/wasm|^src/html|^src/deliver|^lib/graphics/' ;;
    (packages) echo '^src/manifest\.rs|^src/registry|^src/cache\.rs|^src/api_' ;;
    (lsp)      echo '^src/lsp/' ;;
    (sql)      echo '^src/database/sql_|^src/database/lazy\.rs' ;;
    (docs)     echo '^doc/|^default/.*\.loft$|\.md$' ;;
    (*)        return 1 ;;
  esac
}

# The nextest filterset for the DEFAULT run: everything except the heavy eight.
curated_filter() {
  local parts=() b
  for b in "${HEAVY_BINARIES[@]}"; do parts+=("binary($b)"); done
  local joined
  joined=$(IFS='+'; echo "${parts[*]}")
  echo "not ( ${joined//+/ + } )"
}

# Every test binary this repo actually has.
all_binaries() {
  local f
  for f in "$(dirname "${BASH_SOURCE[0]}")/../tests"/*.rs; do basename "$f" .rs; done
}

# The nextest filterset for one subject.
#
# Patterns are EXPANDED against the real binary list rather than handed to
# nextest as `binary(~pat)`.  Two reasons, and the first is not optional:
# nextest treats a pattern matching nothing as a filterset PARSE ERROR, so one
# stale pattern takes the whole selection down — and `~database` is exactly that,
# a lib module with no binary of its own.  Expanding also makes a selection
# auditable: `--subject store --dry-run` prints the binaries, not a rule.
subject_filter() {
  local name="$1" parts=() p b pats
  pats=$(subject_patterns "$name") || return 1
  local seen=" "
  for p in $pats; do
    for b in $(all_binaries); do
      case "$seen" in (*" $b "*) continue ;; esac
      [[ "$b" == *"${p#\~}"* ]] && { seen="$seen$b "; parts+=("binary($b)"); }
    done
  done
  [[ ${#parts[@]} -gt 0 ]] || return 1
  local joined
  joined=$(IFS='+'; echo "${parts[*]}")
  echo "${joined//+/ + }"
}

subject_names() { printf '%s\n' $SUBJECT_NAMES | sort; }

# Test binaries no subject pattern matches.
#
# Advisory, deliberately: the default run is subtractive, so an unmatched binary
# still runs by default and the only cost is that `--subject` cannot single it
# out.  Reported so the patterns can be widened when the list grows, not gated —
# a gate here would push someone to invent a subject rather than admit the
# binary belongs to none.
unmatched_binaries() {
  local pats=" " n p
  for n in $SUBJECT_NAMES; do pats+="$(subject_patterns "$n") "; done
  local f b hit
  for f in "$(dirname "${BASH_SOURCE[0]}")/../tests"/*.rs; do
    b=$(basename "$f" .rs); hit=""
    for p in $pats; do
      [[ "$b" == *"${p#\~}"* ]] && { hit=1; break; }
    done
    [[ -n "$hit" ]] || echo "$b"
  done
}
