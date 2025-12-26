#!/bin/bash
# Integration test runner for September
#
# Usage: ./test.sh [OPTIONS] [PYTEST_ARGS...]
#
# Options:
#   --skip-teardown    Don't tear down environment after tests (for debugging)
#   --rebuild          Force rebuild Docker containers
#   --verbose          Enable verbose output
#   -h, --help         Show this help
#
# Examples:
#   ./test.sh                      # Run all tests
#   ./test.sh -k test_auth         # Run only auth tests
#   ./test.sh --skip-teardown      # Keep containers running after tests
#   ./test.sh --rebuild            # Rebuild containers before running

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Ensure we're in the correct directory
cd "$SCRIPT_DIR"

# Parse arguments
SKIP_TEARDOWN=0
REBUILD=0
VERBOSE=0
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
PYTEST_CMD_ARGS+=("${PYTEST_ARGS[@]}")

# Run tests
uv run pytest "${PYTEST_CMD_ARGS[@]}"
