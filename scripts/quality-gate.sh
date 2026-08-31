#!/usr/bin/env bash
#
# Grist release quality gate runner.
#
# Encodes the serial release gates documented in docs/complete-parser-contract.md
# ("Release gates") and docs/release-readiness.md ("Verification evidence").
# Every gate runs serially with one Cargo worker and locked dependencies, per
# the release contract. Each run captures per-gate logs and a Markdown report
# under artifacts/quality-gate/<UTC timestamp>/.
#
# Usage:
#   scripts/quality-gate.sh                  # run every gate
#   scripts/quality-gate.sh test-all clippy  # run selected gates
#   scripts/quality-gate.sh --list           # list gate names
#   scripts/quality-gate.sh --fail-fast ...  # stop at the first failing gate
#
# Environment overrides:
#   GRIST_GATE_JOBS          Cargo build jobs (default 1, per release contract)
#   GRIST_GATE_TEST_THREADS  Test threads (default 1, per release contract)
#   GRIST_GATE_LOG_DIR       Log/report directory (default artifacts/quality-gate/<ts>)

set -u -o pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

JOBS="${GRIST_GATE_JOBS:-1}"
TEST_THREADS="${GRIST_GATE_TEST_THREADS:-1}"
FAIL_FAST=0
LOG_DIR="${GRIST_GATE_LOG_DIR:-}"

GATE_ORDER=(fmt test-all clippy test-minimal-lib schema-drift cli-smoke fuzz-target git-diff-check)

GATE_DESCRIPTIONS=(
  "fmt|cargo fmt --all -- --check"
  "test-all|cargo test --locked --all-features --no-fail-fast -- --test-threads=$TEST_THREADS"
  "clippy|cargo clippy --locked --all-targets --all-features (advisory baseline, exit-code gate)"
  "test-minimal-lib|cargo test --locked --no-default-features --lib"
  "schema-drift|cargo run --locked --all-features --example schema_codegen -- --check"
  "cli-smoke|cargo run --all-features: capabilities, parse auto, transform/validate/segment round trip"
  "fuzz-target|cargo check --manifest-path fuzz/Cargo.toml --bin universal_bytes"
  "git-diff-check|git diff --check"
)

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit 0
}

list_gates() {
  printf '%s\n' "${GATE_ORDER[@]}"
  exit 0
}

selected=()
while [ $# -gt 0 ]; do
  case "$1" in
    -h|--help) usage ;;
    --list) list_gates ;;
    --fail-fast) FAIL_FAST=1 ;;
    --log-dir) LOG_DIR="$2"; shift ;;
    -*) echo "error: unknown option: $1" >&2; exit 2 ;;
    *) selected+=("$1") ;;
  esac
  shift
done

if [ ${#selected[@]} -eq 0 ]; then
  selected=("${GATE_ORDER[@]}")
fi

for gate in "${selected[@]}"; do
  known=0
  for candidate in "${GATE_ORDER[@]}"; do
    [ "$gate" = "$candidate" ] && known=1
  done
  if [ "$known" -ne 1 ]; then
    echo "error: unknown gate: $gate (see --list)" >&2
    exit 2
  fi
done

if [ -z "$LOG_DIR" ]; then
  LOG_DIR="artifacts/quality-gate/$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$LOG_DIR"

export CARGO_BUILD_JOBS="$JOBS"
export CARGO_INCREMENTAL=0
export CARGO_TERM_COLOR=never

RESULTS_FILE="$LOG_DIR/results.tsv"
REPORT_FILE="$LOG_DIR/report.md"
: > "$RESULTS_FILE"

describe_gate() {
  local name="$1" entry
  for entry in "${GATE_DESCRIPTIONS[@]}"; do
    [ "${entry%%|*}" = "$name" ] && printf '%s' "${entry#*|}" && return 0
  done
}

# Shared locked-dependency flag for the release-contract gates. The flag must
# precede the subcommand: arguments after `--` belong to the test binary.
cargo_serial() {
  cargo --locked "$@"
}

gate_fmt() {
  cargo fmt --all -- --check
}

gate_test_all() {
  cargo_serial test --all-features -- --test-threads="$TEST_THREADS"
}

gate_clippy() {
  cargo_serial clippy --all-targets --all-features
}

gate_test_minimal_lib() {
  cargo_serial test --no-default-features --lib -- --test-threads="$TEST_THREADS"
}

gate_schema_drift() {
  cargo_serial run --all-features --example schema_codegen -- --check
}

gate_cli_smoke() {
  local cli=(cargo run --quiet --all-features --)
  local graph="$LOG_DIR/cli-smoke-graph.json"
  local segment_options="$LOG_DIR/cli-smoke-segment-options.json"

  printf '%s\n' '{"target_size": 512, "maximum_size": 4096}' > "$segment_options"

  "${cli[@]}" capabilities || return 1
  "${cli[@]}" parse auto README.md || return 1
  "${cli[@]}" transform README.md --to graph > "$graph" || return 1
  "${cli[@]}" validate "$graph" --schema graph-transform-envelope || return 1
  "${cli[@]}" segment "$graph" --graph --config "$segment_options" || return 1
}

gate_fuzz_target() {
  cargo check --manifest-path fuzz/Cargo.toml --bin universal_bytes
}

gate_git_diff_check() {
  git diff --check
}

run_gate() {
  local name="$1"
  local fn="gate_${name//-/_}"
  local log="$LOG_DIR/$name.log"
  local started duration status

  printf '[gate] %-18s running\n' "$name"
  started="$(date +%s)"

  "$fn" > "$log" 2>&1
  status=$?

  duration=$(( $(date +%s) - started ))
  if [ "$status" -eq 0 ]; then
    printf '[gate] %-18s PASS (%ds)\n' "$name" "$duration"
  else
    printf '[gate] %-18s FAIL (%ds) — log: %s\n' "$name" "$duration" "$log"
  fi
  printf '%s\t%s\t%s\n' "$name" "$status" "$duration" >> "$RESULTS_FILE"
  return "$status"
}

overall=0
failed_gates=()
for gate in "${selected[@]}"; do
  if ! run_gate "$gate"; then
    overall=1
    failed_gates+=("$gate")
    [ "$FAIL_FAST" -eq 1 ] && break
  fi
done

# Build the Markdown report from the recorded results.
{
  echo "# Grist quality gate report"
  echo
  echo "- Date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Branch: $(git rev-parse --abbrev-ref HEAD)"
  echo "- Commit: $(git rev-parse --short HEAD)"
  echo "- Cargo: $(cargo --version)"
  echo "- Environment: CARGO_BUILD_JOBS=$JOBS, CARGO_INCREMENTAL=0, test-threads=$TEST_THREADS"
  echo
  echo "| Gate | Command | Result | Duration |"
  echo "|---|---|---|---|"
  while IFS=$'\t' read -r name status duration; do
    if [ "$status" -eq 0 ]; then result="pass"; else result="**FAIL**"; fi
    echo "| \`$name\` | $(describe_gate "$name") | $result | ${duration}s |"
  done < "$RESULTS_FILE"
  echo
  if [ "$overall" -eq 0 ]; then
    echo "All gates passed."
  else
    printf 'Failed gates: %s\n' "${failed_gates[*]}"
  fi
} > "$REPORT_FILE"

echo
echo "Report: $REPORT_FILE"
exit "$overall"
