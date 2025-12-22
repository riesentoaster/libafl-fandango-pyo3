#!/bin/bash
set -euo pipefail

TIMEOUT="${1:-30}"

if [[ -f ".venv/bin/activate" ]]; then
    source .venv/bin/activate
    export PYTHONPATH=$(echo .venv/lib/python*/site-packages)
fi

# These examples run indefinitely, so we timeout them
run_with_timeout() {
local seconds="$1"
shift
echo "Running command (timeout ${seconds}s): $*"
if timeout "${seconds}"s "$@"; then
    echo "Command finished successfully: $*"
else
    exit_code=$?
    if [ "$exit_code" -eq 124 ]; then
    echo "Command timed out after ${seconds}s (treated as non-fatal): $*"
    else
    echo "Command failed with exit code $exit_code: $*"
    exit "$exit_code"
    fi
fi
}

run_with_timeout $TIMEOUT cargo run --release --example baby_fuzzer_generator    -- --quiet
run_with_timeout $TIMEOUT cargo run --release --example baby_fuzzer_mutator      -- --quiet
run_with_timeout $TIMEOUT cargo run --release --example baby_fuzzer_stage        -- --quiet
run_with_timeout $TIMEOUT cargo run --release --example baby_fuzzer_differential -- --quiet