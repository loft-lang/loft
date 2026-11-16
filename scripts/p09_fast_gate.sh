#!/usr/bin/env bash
# Fast regression gate for plan 09 work.
#
# Compares native emission for a 7-file doc corpus against a baseline
# captured at /tmp/p09-baseline/, then runs the P-issue reproducers as
# native binaries.  Together these enforce phase 00's byte-identical
# contract + closed-bug guards in well under 1.5 minutes — replacing the
# 4-minute `cargo test --test issues` cycle for routine step-by-step work.
#
# As phases land, the gate also reports progress markers relevant to the
# current phase (e.g. legacy-fn migration count for phase 01).  Each
# phase doc lists the gate updates expected at that step (capture
# refreshes, new structural assertions) — see the "Gate updates" note
# in each phase's "Detailed steps with validation" section.
#
# Usage:
#   scripts/p09_fast_gate.sh                # diff vs baseline + reproducers + progress
#   scripts/p09_fast_gate.sh --capture       # (re)capture baseline at HEAD
#   scripts/p09_fast_gate.sh --full-suite    # also run full issues suite
#
# Plan reference: doc/claude/plans/09-native-runtime-rewrite/00-scaffold.md

set -euo pipefail

# Cache clean/release rebuilds with sccache when present (no-op otherwise).
source "$(dirname "${BASH_SOURCE[0]}")/sccache_env.sh"

CORPUS=(
    tests/docs/03-integer.loft
    tests/docs/04-boolean.loft
    tests/docs/07-vector.loft
    tests/docs/08-struct.loft
    tests/docs/13-file.loft
    tests/docs/19-threading.loft
    tests/docs/25-generics.loft
    # Phase 03: parallel-for emitter validation.  22-threading
    # exercises the n_parallel_for_native dispatch with scalar +
    # float closure shapes — covers the bulk of the special case
    # phase 03 retires.
    tests/scripts/22-threading.loft
    # Phase 04: OpGetRecord / OpIterate emitter validation.
    # 10-sorted hits both Ops 20 times across the file.
    tests/docs/10-sorted.loft
)

REPROS=(
    tests/scripts/repro_p203.loft
    tests/scripts/repro_p204.loft
    tests/scripts/repro_p205.loft
)

BASELINE_DIR="/tmp/p09-baseline"
CURRENT_DIR="/tmp/p09-current"

mode="diff"
run_full=0
for arg in "$@"; do
    case "$arg" in
        --capture) mode="capture" ;;
        --full-suite) run_full=1 ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

emit() {
    local out_dir="$1"
    mkdir -p "$out_dir"
    for t in "${CORPUS[@]}"; do
        local name
        name=$(basename "$t" .loft)
        cargo run --bin loft --release --quiet -- \
            --native-rust --native-emit "$out_dir/$name.rs" "$t" 2>/dev/null
    done
}

if [[ "$mode" == "capture" ]]; then
    echo "[p09 gate] capturing baseline at $BASELINE_DIR"
    rm -rf "$BASELINE_DIR"
    emit "$BASELINE_DIR"
    echo "[p09 gate] baseline: $(ls "$BASELINE_DIR" | wc -l) files"
    exit 0
fi

if [[ ! -d "$BASELINE_DIR" ]]; then
    echo "[p09 gate] no baseline at $BASELINE_DIR — run with --capture first" >&2
    exit 1
fi

echo "[p09 gate] emitting current corpus"
rm -rf "$CURRENT_DIR"
emit "$CURRENT_DIR"

echo "[p09 gate] diffing vs baseline"
diff_status=0
for t in "${CORPUS[@]}"; do
    name=$(basename "$t" .loft)
    if ! diff -q "$BASELINE_DIR/$name.rs" "$CURRENT_DIR/$name.rs" >/dev/null; then
        echo "  DIFFERS: $name.rs"
        diff_status=1
    fi
done
if [[ $diff_status -eq 0 ]]; then
    echo "  byte-identical: $(ls "$CURRENT_DIR" | wc -l) files"
fi

echo "[p09 gate] running P-issue reproducers"
repro_status=0
# P203: closed by template let-bind-on-repeat fix.  Must still pass.
# P204, P205: still open as of 2026-05-02; reported but not blocking.
declare -A REPRO_REQUIRED=(
    ["tests/scripts/repro_p203.loft"]=1
    ["tests/scripts/repro_p204.loft"]=0
    ["tests/scripts/repro_p205.loft"]=0
)
for r in "${REPROS[@]}"; do
    if cargo run --bin loft --release --quiet -- "$r" 2>/dev/null; then
        echo "  PASS: $r"
    else
        if [[ "${REPRO_REQUIRED[$r]:-0}" == "1" ]]; then
            echo "  FAIL: $r  (REGRESSION — was closed)"
            repro_status=1
        else
            echo "  fail: $r  (still open — informational)"
        fi
    fi
done

if [[ $run_full -eq 1 ]]; then
    echo "[p09 gate] running full issues suite (this takes ~2.5 min)"
    cargo test --release --test issues 2>&1 | tail -3
fi

echo "[p09 gate] phase progress markers"

# Phase 01 — ABI consolidation: count LegacyStores entries remaining
# in CODEGEN_RUNTIME_FNS.  Decreases as fns migrate to Cell or None.
# When this reaches 0, phase 01 step 1.7 (cleanup) becomes unblocked.
if [[ -f src/codegen_runtime.rs ]]; then
    legacy_count=$(grep -c "abi: Abi::LegacyStores" src/codegen_runtime.rs || true)
    cell_count=$(grep -c "abi: Abi::Cell" src/codegen_runtime.rs || true)
    none_count=$(grep -c "abi: Abi::None" src/codegen_runtime.rs || true)
    total=$((legacy_count + cell_count + none_count))
    # `LegacyStores` enum variant declared?  Detects whether step 1.7
    # cleanup has shipped (variant removed).
    has_legacy_variant=$(grep -c "^\s*LegacyStores," src/codegen_runtime.rs || true)
    echo "  phase 01 ABI: $cell_count Cell + $none_count None + $legacy_count LegacyStores = $total runtime fns"
    if [[ $legacy_count -eq 0 && $has_legacy_variant -eq 0 ]]; then
        echo "  phase 01 step 1.7 DONE (LegacyStores variant retired; ABI selection collapsed)"
    elif [[ $legacy_count -eq 0 ]]; then
        echo "  phase 01 step 1.7 unblocked (no LegacyStores entries; variant + dispatch arm can be removed)"
    fi
fi

# Phase 00 wart budget — dispatch.rs special-case Op match arm count.
# Decreases as later phases migrate Ops into custom emitters.
if [[ -f src/generation/dispatch.rs ]]; then
    arms=$(awk '/fn output_call_inner/,/^    }$/' src/generation/dispatch.rs \
        | grep -cE '^\s*"Op[A-Z][a-zA-Z]*"\s*(\|.*)?\s*=>' || true)
    echo "  dispatch.rs op match arms: $arms (budget 26 — shrink as phases register custom emitters)"
fi

# Custom emitter registration count.  Counts both individual
# `r.insert(...)` lines and entries in any `pub const ..._OP_NAMES`
# list iterated over in build_registry (phase-00 forwarding smoke test
# pattern).
if [[ -f src/generation/ops/mod.rs ]]; then
    individual=$(awk '/fn build_registry/,/^}/' src/generation/ops/mod.rs \
        | grep -cE '^\s*r\.insert\(' || true)
    # Count entries in any *_OP_NAMES const list across ops/ files.
    list_count=0
    for f in src/generation/ops/*.rs; do
        in_list=$(awk '/pub const [A-Z_]+_OP_NAMES.*&\[/,/^];/' "$f" 2>/dev/null \
            | grep -cE '^\s*"Op[A-Z]' || true)
        list_count=$((list_count + in_list))
    done
    total=$((individual + list_count))
    echo "  custom emitters registered: $total ($individual individual + $list_count via lists)"
fi

if [[ $diff_status -ne 0 ]]; then
    # Heuristic: if every diff file's only changes are `(stores,` →
    # `(cell,` or `(stores)` → `(cell)` substitutions in n_* call
    # sites, the diff is an ABI migration shape — suggest --capture.
    abi_migration_shape=true
    for t in "${CORPUS[@]}"; do
        name=$(basename "$t" .loft)
        if ! diff -q "$BASELINE_DIR/$name.rs" "$CURRENT_DIR/$name.rs" >/dev/null 2>&1; then
            # Get the diff and check if all changed lines fit the ABI pattern.
            diffs=$(diff "$BASELINE_DIR/$name.rs" "$CURRENT_DIR/$name.rs" | grep -E '^[<>]' || true)
            non_abi=$(echo "$diffs" | grep -vE 'n_[a-z_]+\((stores|cell)' || true)
            if [[ -n "$non_abi" ]]; then
                abi_migration_shape=false
                break
            fi
        fi
    done
    if [[ "$abi_migration_shape" == "true" ]]; then
        echo "  [hint] diff shape looks like an ABI migration (stores↔cell only)."
        echo "         If intentional, refresh baseline:  scripts/p09_fast_gate.sh --capture"
    fi
fi

if [[ $diff_status -ne 0 || $repro_status -ne 0 ]]; then
    echo "[p09 gate] FAILED"
    exit 1
fi
echo "[p09 gate] PASS"
