#!/bin/bash
# Browser test runner script
# This script starts the necessary services and runs browser tests

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Configuration
SERVER_PORT=3001
WEBDRIVER_PORT=4444
WAIT_TIMEOUT=30

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

cleanup() {
    log_info "Cleaning up..."

    # Kill the server if we started it
    if [ ! -z "$SERVER_PID" ]; then
        log_info "Stopping application server (PID: $SERVER_PID)"
        kill $SERVER_PID 2>/dev/null || true
    fi

    # Kill chromedriver if we started it
    if [ ! -z "$CHROMEDRIVER_PID" ]; then
        log_info "Stopping chromedriver (PID: $CHROMEDRIVER_PID)"
        kill $CHROMEDRIVER_PID 2>/dev/null || true
    fi
}

trap cleanup EXIT

wait_for_port() {
    local port=$1
    local name=$2
    local timeout=$3
    local elapsed=0

    log_info "Waiting for $name on port $port..."

    while ! nc -z localhost $port 2>/dev/null; do
        if [ $elapsed -ge $timeout ]; then
            log_error "$name did not start within $timeout seconds"
            return 1
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    log_info "$name is ready on port $port"
    return 0
}

# Check for required tools
check_requirements() {
    if ! command -v chromedriver &> /dev/null; then
        log_error "chromedriver is not installed. Please install it:"
        log_error "  - Ubuntu/Debian: sudo apt install chromium-chromedriver"
        log_error "  - macOS: brew install chromedriver"
        log_error "  - Or download from: https://chromedriver.chromium.org/"
        exit 1
    fi

    if ! command -v nc &> /dev/null; then
        log_warn "netcat (nc) not found, will use alternative port check"
    fi
}

# Start chromedriver if not already running
start_chromedriver() {
    if nc -z localhost $WEBDRIVER_PORT 2>/dev/null; then
        log_info "WebDriver already running on port $WEBDRIVER_PORT"
        return 0
    fi

    log_info "Starting chromedriver on port $WEBDRIVER_PORT..."
    chromedriver --port=$WEBDRIVER_PORT &
    CHROMEDRIVER_PID=$!

    if ! wait_for_port $WEBDRIVER_PORT "chromedriver" $WAIT_TIMEOUT; then
        exit 1
    fi
}

# Build and start the application server
start_server() {
    cd "$PROJECT_DIR"

    if nc -z localhost $SERVER_PORT 2>/dev/null; then
        log_info "Server already running on port $SERVER_PORT"
        return 0
    fi

    log_info "Building application..."
    cargo build

    log_info "Starting application server on port $SERVER_PORT..."

    # Use test configuration
    RUST_LOG=september=info ./target/debug/september &
    SERVER_PID=$!

    if ! wait_for_port $SERVER_PORT "application server" $WAIT_TIMEOUT; then
        exit 1
    fi
}

# Run the browser tests
run_tests() {
    cd "$PROJECT_DIR"

    log_info "Running browser tests..."
    cargo test --test browser_tests -- --test-threads=1 "$@"
}

# Main execution
main() {
    log_info "Starting browser test suite"
    log_info "Project directory: $PROJECT_DIR"

    check_requirements
    start_chromedriver
    start_server
    run_tests "$@"

    log_info "Browser tests completed successfully!"
}

main "$@"
