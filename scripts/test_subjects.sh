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
# Every test binary must match at least one pattern — `tests/doc_hygiene.rs::
# every_test_binary_matches_a_subject` asks `unmatched_binaries` below and goes red on
# a name it reports (@PLN159 phase H).  A binary no subject reaches never runs in the
# seconds-long loop (`--subject`, `--changed`) that exists to catch a regression while
# the edit is warm; it still runs in the curated and full sets, so this is a guard on
# the LOOP's reach, never on coverage.  Adding a binary: name it so an existing pattern
# picks it up, or add the pattern here in the same commit.
# `~stem` matches a binary whose name CONTAINS the stem; `=name` matches the whole
# name.  Read each subject's list once after editing a pattern
# (`--list-subjects` prints the counts; `subject_filter <name>` the names): a bare
# stem over-matches as easily as it under-matches — `~par` (for par_*/parallel)
# also took `group_apart_lint`, `~own` took `viewer_markd*own*`, and `~import`
# took the browser test `html_gl_imports`, which made `--subject packages` rebuild
# the wasm rlibs it never links.  The doc_hygiene guard below catches an UNMATCHED
# binary, never a wrongly matched one; only reading the list does.
pattern_matches() {
  local p="$1" b="$2"
  case "$p" in
    =*) [[ "$b" == "${p#=}" ]] ;;
    *)  [[ "$b" == *"${p#\~}"* ]] ;;
  esac
}

SUBJECT_NAMES='parser scopes codegen runtime store wasm packages lsp sql docs host'

# Spelled as a case-function, not `declare -A`: macOS ships bash 3.2 as BOTH /bin/sh and
# /bin/bash, which has no associative arrays — the array form made every `--subject` run on a
# Mac die with `parser: unbound variable` while the same script worked on every Linux box.
# Same data, portable spelling.
subject_patterns() {
  case "$1" in
    (parser)    echo '~pars ~keyless_sorted ~expression ~error_messages ~suggestion ~strings ~spans ~tuple ~qq_null ~dn4 ~nullflow ~steer ~lint ~match ~const_ ~diagnostic ~main_signature ~plan25 ~pln25 ~nullable ~variant_field ~template ~fault_position ~pln14' ;;
    (scopes)    echo '~slot ~leak ~ownership ~use_analysis ~uaf ~frame_vars ~closure ~callarg ~alias ~borrow ~branch_join ~copy_advice ~double_move ~loop_binding ~own_ ~owns_ ~ref_param ~redundant_free ~returned_text ~value_struct ~text_buffer ~text_return ~early_text ~nullable_ret ~generic_discharged ~link_' ;;
    (codegen)   echo '~codegen ~copy_fresh_dest ~inplace_callee_hoist ~fused_append ~scalar_hoist ~mint_hoist ~record_push ~move_append ~retbuf_adopt ~selfread_literal ~literal_hoist ~complete_write ~element_first ~view_header ~wrapper_op ~callee_inputs ~push_hoist ~emission_audit ~native ~n2_ ~n3_ ~g2_ ~ir_ ~introspect ~slots ~entry_signature ~differential ~hoist ~e1_ ~n0_ ~behavior_golden ~compile_scaling ~windows ~append_in_place ~retbuf ~view_elision' ;;
    (runtime)   echo '~wrap ~issues ~thread ~par_ ~parallel ~parity ~coroutine ~runtime ~dispatch ~panic ~exit_codes ~crash ~error_path ~soft_halt ~log ~math ~format_width ~profiling ~sandbox ~script_mode ~self_append ~timeout ~json_corpus ~test ~env_' ;;
    (store)     echo '~store ~database ~data_ ~paged ~lazy ~field_without ~layout ~watermark ~binary_io ~heap_free ~clear_release' ;;
    (wasm)      echo '~wasm ~html ~deliver ~browser ~gl_ ~android' ;;
    (packages)  echo '~registry ~package =imports ~api_ ~compat ~manifest ~extract ~resolution ~cache ~self_update ~install ~lib_ ~library ~module_name ~path_flag ~placement ~stdlib_target ~undeclared ~dep_' ;;
    (lsp)       echo '~lsp ~dap ~debugger ~repl' ;;
    (sql)       echo '~lazy_sql ~sql' ;;
    (docs)      echo '~doc ~features ~index_hygiene ~comment ~viewer ~check_line ~expectation ~function_coverage ~typst' ;;
    (host)      echo '~engine_host ~host_ ~multiplayer ~serve ~rpc ~mock ~audio ~crystal' ;;
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
      pattern_matches "$p" "$b" && { seen="$seen$b "; parts+=("binary($b)"); }
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
      pattern_matches "$p" "$b" && { hit=1; break; }
    done
    [[ -n "$hit" ]] || echo "$b"
  done
}

# @PLN159 phase D — the subjects a DIFF touches, read off the SUBJECT_PATHS map above.
#
# `changed_paths [ref]` lists what differs from `ref` (default HEAD: the uncommitted
# edits, untracked files included).  `changed_filter [ref]` maps them to a nextest
# filterset: a path under a subject's pattern selects that subject's binaries, an edited
# `tests/<name>.rs` selects its own binary, an edited corpus file selects the three corpus
# runners.  It REFUSES (exit 1, reason on stderr) when the diff touches something every
# binary depends on — the shared test harness, Cargo.toml, build.rs, loft-ffi — because
# the honest selection is then the curated run, not a guess.  This is the ITERATION
# loop's selection; `make ci` still runs everything, it only runs these first.
changed_paths() {
  local ref="${1:-HEAD}"
  { git diff --name-only "$ref" -- ; git ls-files --others --exclude-standard; } | sort -u
}

changed_filter() {
  local ref="${1:-HEAD}" paths p n b
  paths=$(changed_paths "$ref")
  [[ -n "$paths" ]] || { echo "changed: nothing differs from $ref" >&2; return 1; }
  local -A seen=() subs=()
  local -a parts=()
  local wide=""
  while IFS= read -r p; do
    [[ -n "$p" ]] || continue
    case "$p" in
      tests/common/*|Cargo.toml|Cargo.lock|build.rs|.config/nextest.toml|loft-ffi/*|loft-ffi-*/*)
        wide="$p" ;;
      tests/scripts/*.loft|tests/docs/*.loft)
        for b in wrap native ir_schema_roundtrip; do
          [[ -z "${seen[$b]:-}" ]] && { seen[$b]=1; parts+=("binary($b)"); }
        done ;;
      tests/*.rs)
        b=$(basename "$p" .rs)
        [[ -f "tests/$b.rs" && -z "${seen[$b]:-}" ]] && { seen[$b]=1; parts+=("binary($b)"); } ;;
    esac
    for n in "${!SUBJECT_PATHS[@]}"; do
      [[ "$p" =~ ${SUBJECT_PATHS[$n]} ]] && subs[$n]=1
    done
  done <<<"$paths"
  if [[ -n "$wide" ]]; then
    echo "changed: the diff touches $wide, which every binary depends on — running the curated set" >&2
    return 1
  fi
  local f
  for n in "${!subs[@]}"; do
    f=$(subject_filter "$n") || continue
    parts+=("$f")
  done
  [[ ${#parts[@]} -gt 0 ]] || { echo "changed: nothing in the diff maps to a subject — running the curated set" >&2; return 1; }
  [[ ${#subs[@]} -gt 0 ]] && echo "changed: subjects ${!subs[*]}" >&2
  local joined
  joined=$(IFS='+'; echo "${parts[*]}")
  echo "${joined//+/ + }"
}
