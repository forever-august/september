#!/bin/bash
# Setup script for September integration test environment
#
# Usage: ./setup.sh [OPTIONS]
#
# Options:
#   --rebuild    Force rebuild Docker containers
#   -h, --help   Show this help
#
# This script is idempotent - safe to run multiple times.

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Parse arguments
REBUILD=0

show_help() {
    sed -n '2,/^$/p' "$0" | sed 's/^# //' | sed 's/^#//'
}

while [[ $# -gt 0 ]]; do
    case $1 in
        --rebuild)
            REBUILD=1
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

# Check if services are already running and healthy
services_healthy() {
    local running
    running=$(docker compose ps --format json 2>/dev/null | grep -c '"State":"running"' || echo "0")
    # We expect at least 4 running services (nntp, dex, september, chrome)
    [[ $running -ge 4 ]]
}

# If rebuild requested, tear down first
if [[ $REBUILD -eq 1 ]]; then
    echo "Rebuild requested, tearing down existing environment..."
    ./teardown.sh
fi

# Check if already running
if services_healthy; then
    echo "Environment already running."
    echo "Ensuring test data is seeded..."
    docker compose run --rm seeder
    echo "Environment ready!"
    exit 0
fi

echo "Starting integration test environment..."

# Build and start all services
if [[ $REBUILD -eq 1 ]]; then
    docker compose build --no-cache
fi
docker compose up -d --build --wait

# Seed test data
echo "Seeding test data..."
docker compose run --rm seeder

echo "Environment ready!"
echo ""
echo "Services:"
echo "  - September: http://localhost:3000"
echo "  - Dex OIDC:  http://localhost:5556"
echo "  - NNTP:      localhost:1190"
echo "  - Selenium:  http://localhost:4445"
echo "  - VNC:       http://localhost:7900 (password: secret)"
