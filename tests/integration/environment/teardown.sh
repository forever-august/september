#!/bin/bash
# Teardown script for September integration test environment
#
# Usage: ./teardown.sh
#
# This script is idempotent - safe to run even if nothing is running.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Check if any containers exist for this compose project
if ! docker compose ps -q 2>/dev/null | grep -q .; then
    echo "No containers running, nothing to tear down."
    exit 0
fi

echo "Stopping integration test environment..."

# Stop all services and remove volumes
docker compose down -v

echo "Environment stopped and cleaned up."
