#!/bin/bash
# Collect Docker logs from integration test services
#
# Usage: ./collect-logs.sh [SERVICE...]
#
# Examples:
#   ./collect-logs.sh              # Collect all service logs
#   ./collect-logs.sh september    # Collect only september logs

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/environment"

# Default services if none specified
if [[ $# -eq 0 ]]; then
    SERVICES=(september nntp dex chrome)
else
    SERVICES=("$@")
fi

for service in "${SERVICES[@]}"; do
    echo "=== $service logs ==="
    docker compose logs "$service" 2>/dev/null || echo "(no logs available)"
    echo ""
done
