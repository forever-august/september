#!/bin/bash
# Teardown script for September integration test environment
#
# This script stops all Docker services and removes volumes.
# Run from anywhere - it will cd to the correct directory.

set -e

# Change to the directory containing this script
cd "$(dirname "$0")"

echo "Stopping integration test environment..."

# Stop all services and remove volumes
docker compose down -v

echo "Environment stopped and cleaned up."
