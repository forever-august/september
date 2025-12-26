#!/bin/bash
# Setup script for September integration test environment
#
# This script starts all Docker services and seeds test data.
# Run from anywhere - it will cd to the correct directory.

set -e

# Change to the directory containing this script
cd "$(dirname "$0")"

echo "Starting integration test environment..."

# Build and start all services
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
