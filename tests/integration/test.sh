#!/bin/bash
# Integration test runner for September
#
# Usage: ./test.sh [OPTIONS] [PYTEST_ARGS...]
#
# Options:
#   --skip-teardown    Don't tear down environment after tests (for debugging)
#   --rebuild          Force rebuild Docker containers
#   --verbose          Enable verbose output
#   --parallel [N]     Run tests in parallel (N workers, default: auto)
#   -h, --help         Show this help
#
# Examples:
#   ./test.sh                      # Run all tests
#   ./test.sh -k test_auth         # Run only auth tests
#   ./test.sh --skip-teardown      # Keep containers running after tests
#   ./test.sh --rebuild            # Rebuild containers before running
#   ./test.sh --parallel           # Run tests in parallel (auto-detect workers)
#   ./test.sh --parallel 4         # Run tests with 4 parallel workers

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Ensure we're in the correct directory
cd "$SCRIPT_DIR"

# Parse arguments
SKIP_TEARDOWN=0
REBUILD=0
VERBOSE=0
PARALLEL=0
PARALLEL_WORKERS=""
PYTEST_ARGS=()

show_help() {
    sed -n '2,/^$/p' "$0" | sed 's/^# //' | sed 's/^#//'
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-teardown)
            SKIP_TEARDOWN=1
            shift
            ;;
        --rebuild)
            REBUILD=1
            shift
            ;;
        --verbose)
            VERBOSE=1
            shift
            ;;
        --parallel)
            PARALLEL=1
            shift
            # Check if next arg is a number (worker count)
            if [[ $# -gt 0 && $1 =~ ^[0-9]+$ ]]; then
                PARALLEL_WORKERS=$1
                shift
            fi
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            PYTEST_ARGS+=("$1")
            shift
            ;;
    esac
done

# Set up cleanup trap (unless --skip-teardown)
if [[ $SKIP_TEARDOWN -eq 0 ]]; then
    trap './environment/teardown.sh' EXIT
fi

# Build setup args
SETUP_ARGS=()
if [[ $REBUILD -eq 1 ]]; then
    SETUP_ARGS+=("--rebuild")
fi

# Start environment
./environment/setup.sh "${SETUP_ARGS[@]}"

# Build pytest args
PYTEST_CMD_ARGS=("--tb=short")
if [[ $VERBOSE -eq 1 ]]; then
    PYTEST_CMD_ARGS+=("-vv")
else
    PYTEST_CMD_ARGS+=("-v")
fi

# Add parallel execution args
if [[ $PARALLEL -eq 1 ]]; then
    if [[ -n $PARALLEL_WORKERS ]]; then
        PYTEST_CMD_ARGS+=("-n" "$PARALLEL_WORKERS")
    else
        PYTEST_CMD_ARGS+=("-n" "auto")
    fi
fi

PYTEST_CMD_ARGS+=("${PYTEST_ARGS[@]}")

# Run tests
uv run pytest "${PYTEST_CMD_ARGS[@]}"
