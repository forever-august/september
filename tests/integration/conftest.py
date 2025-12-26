"""
Pytest fixtures for September integration tests.

Provides:
- Selenium WebDriver connected to Chrome container
- Helper functions for authentication
- Base URLs for services
- Log capture and failure analysis for debugging
"""

import json
import re
import subprocess
import time
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Generator

import pytest
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import TimeoutException
import os

# Path to the environment directory containing Docker setup
ENVIRONMENT_DIR = Path(__file__).parent / "environment"

# Path to the environment directory containing Docker setup
ENVIRONMENT_DIR = Path(__file__).parent / "environment"


# Service URLs - use Docker service names when running in container,
# localhost when running tests from host
SELENIUM_URL = os.environ.get("SELENIUM_URL", "http://localhost:4445/wd/hub")
SEPTEMBER_URL = os.environ.get("SEPTEMBER_URL", "http://september:3000")
DEX_URL = os.environ.get("DEX_URL", "http://dex:5556")

# Test user credentials (matches dex.yaml staticPasswords)
TEST_USER_EMAIL = "testuser@example.com"
TEST_USER_PASSWORD = "password"  # bcrypt hash in dex.yaml is for "password"
TEST_USER_NAME = "testuser"

# Services to capture logs from
LOG_SERVICES = ["september", "nntp", "dex"]

# WebDriver wait timeouts (in seconds)
# Keep these low - pytest-timeout will catch tests that hang
WAIT_TIMEOUT_DEFAULT = 3  # Default wait for most elements
WAIT_TIMEOUT_OIDC = 5  # OIDC flows may need slightly longer due to redirects
WAIT_TIMEOUT_POLL = 0.2  # Poll frequency for WebDriverWait


# =============================================================================
# Log Capture and Analysis
# =============================================================================


@dataclass
class LogEntry:
    """Parsed log entry from a service."""

    service: str
    timestamp: datetime | None
    level: str
    message: str
    raw: str
    fields: dict = field(default_factory=dict)


@dataclass
class TestLogCapture:
    """Captures logs during a test for failure analysis."""

    test_name: str
    start_time: datetime
    end_time: datetime | None = None
    logs: list[LogEntry] = field(default_factory=list)

    def get_logs_in_window(self) -> list[LogEntry]:
        """Get logs that occurred during the test window."""
        if self.end_time is None:
            self.end_time = datetime.now(timezone.utc)
        return [
            log
            for log in self.logs
            if log.timestamp is None
            or (self.start_time <= log.timestamp <= self.end_time)
        ]

    def get_error_logs(self) -> list[LogEntry]:
        """Get only error and warning level logs."""
        return [
            log
            for log in self.get_logs_in_window()
            if log.level.lower() in ("error", "warn", "warning", "fatal", "panic")
        ]


def parse_json_log(line: str, service: str) -> LogEntry | None:
    """Parse a JSON log line."""
    try:
        data = json.loads(line)
        # Handle different JSON log formats
        timestamp = None
        for ts_field in ("timestamp", "ts", "time", "@timestamp", "t"):
            if ts_field in data:
                try:
                    ts_str = data[ts_field]
                    # Try ISO format
                    if isinstance(ts_str, str):
                        # Remove trailing Z and parse
                        ts_str = ts_str.rstrip("Z")
                        if "." in ts_str:
                            timestamp = datetime.fromisoformat(ts_str).replace(
                                tzinfo=timezone.utc
                            )
                        else:
                            timestamp = datetime.fromisoformat(ts_str).replace(
                                tzinfo=timezone.utc
                            )
                    elif isinstance(ts_str, (int, float)):
                        timestamp = datetime.fromtimestamp(ts_str, tz=timezone.utc)
                    break
                except (ValueError, TypeError):
                    pass

        level = data.get("level", data.get("lvl", data.get("severity", "info")))

        # Handle September's nested format: {"fields": {"message": "..."}}
        message = ""
        if "fields" in data and isinstance(data["fields"], dict):
            message = data["fields"].get("message", "")
            # Include error field if present
            if "error" in data["fields"]:
                message = f"{message} (error: {data['fields']['error']})"
        else:
            message = data.get("message", data.get("msg", ""))

        # Collect other fields for context
        fields = {}
        if "target" in data:
            fields["target"] = data["target"]
        if "span" in data and isinstance(data["span"], dict):
            # Include useful span info
            for key in ("path", "method", "group", "operation", "request_id"):
                if key in data["span"]:
                    fields[key] = data["span"][key]

        return LogEntry(
            service=service,
            timestamp=timestamp,
            level=str(level).upper(),
            message=str(message),
            raw=line,
            fields=fields,
        )
    except json.JSONDecodeError:
        return None


def parse_text_log(line: str, service: str) -> LogEntry:
    """Parse a plain text log line."""
    # Strip ANSI escape codes (color codes from tracing-subscriber)
    ansi_escape = re.compile(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])")
    clean_line = ansi_escape.sub("", line)

    # Try to extract level from common patterns
    level = "INFO"
    level_patterns = [
        (r"\b(ERROR|ERR)\b", "ERROR"),
        (r"\b(WARN|WARNING)\b", "WARN"),
        (r"\b(DEBUG|DBG)\b", "DEBUG"),
        (r"\b(TRACE|TRC)\b", "TRACE"),
        (r"\b(INFO|INF)\b", "INFO"),
    ]
    for pattern, lvl in level_patterns:
        if re.search(pattern, clean_line, re.IGNORECASE):
            level = lvl
            break

    # Try to extract timestamp
    timestamp = None
    # ISO format: 2024-01-15T10:30:00
    ts_match = re.search(r"(\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2})", clean_line)
    if ts_match:
        try:
            ts_str = ts_match.group(1).replace(" ", "T")
            timestamp = datetime.fromisoformat(ts_str).replace(tzinfo=timezone.utc)
        except ValueError:
            pass

    return LogEntry(
        service=service,
        timestamp=timestamp,
        level=level,
        message=clean_line.strip(),
        raw=line,
    )


def fetch_service_logs(service: str, since: datetime) -> list[LogEntry]:
    """Fetch logs from a Docker service since a given time."""
    try:
        # Calculate the time delta for --since
        delta = datetime.now(timezone.utc) - since
        since_seconds = max(1, int(delta.total_seconds()) + 5)  # Add buffer

        result = subprocess.run(
            [
                "docker",
                "compose",
                "logs",
                "--no-color",
                "--since",
                f"{since_seconds}s",
                service,
            ],
            capture_output=True,
            text=True,
            timeout=10,
            cwd=ENVIRONMENT_DIR,
        )

        logs = []
        for line in result.stdout.splitlines():
            if not line.strip():
                continue

            # Docker compose prefixes logs with "service-1  | "
            # Remove the prefix
            if "|" in line:
                line = line.split("|", 1)[1].strip()

            # Try JSON first, fall back to text
            entry = parse_json_log(line, service)
            if entry is None:
                entry = parse_text_log(line, service)
            logs.append(entry)

        return logs
    except (subprocess.TimeoutExpired, subprocess.SubprocessError, FileNotFoundError):
        return []


def analyze_failure(capture: TestLogCapture, exception: BaseException | None) -> dict:
    """Analyze a test failure to determine if it's test-related or service-related."""
    analysis = {
        "test_name": capture.test_name,
        "duration_seconds": (
            (capture.end_time - capture.start_time).total_seconds()
            if capture.end_time
            else 0
        ),
        "error_type": "unknown",
        "likely_cause": "unknown",
        "service_errors": [],
        "recommendations": [],
    }

    error_logs = capture.get_error_logs()

    # Categorize by service
    service_errors = {}
    for log in error_logs:
        if log.service not in service_errors:
            service_errors[log.service] = []
        service_errors[log.service].append(log)

    analysis["service_errors"] = [
        {
            "service": svc,
            "count": len(errs),
            "messages": [e.message[:200] for e in errs[:3]],
        }
        for svc, errs in service_errors.items()
    ]

    # Determine likely cause
    exception_str = str(exception) if exception else ""

    # Check for timeout errors (likely test/selector issue)
    if "TimeoutException" in exception_str or "timeout" in exception_str.lower():
        if any(
            log.service == "september" and "error" in log.level.lower()
            for log in error_logs
        ):
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "September returned an error during the request"
            analysis["recommendations"].append("Check September error logs for details")
        elif any(
            log.service == "nntp" and "error" in log.level.lower() for log in error_logs
        ):
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "NNTP server (renews) encountered an error"
            analysis["recommendations"].append("Check NNTP error logs for details")
        else:
            analysis["error_type"] = "test_issue"
            analysis["likely_cause"] = (
                "Element not found - likely incorrect CSS selector or page structure changed"
            )
            analysis["recommendations"].append(
                "Verify CSS selectors match actual page HTML"
            )
            analysis["recommendations"].append(
                "Check if page loaded correctly (use VNC at localhost:7900)"
            )

    # Check for assertion errors
    elif "AssertionError" in exception_str:
        if error_logs:
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "Service error caused unexpected page state"
        else:
            analysis["error_type"] = "test_issue"
            analysis["likely_cause"] = (
                "Test assertion failed - expected condition not met"
            )
            analysis["recommendations"].append("Review test logic and expected values")

    # Check for connection errors
    elif "ConnectionError" in exception_str or "connection" in exception_str.lower():
        analysis["error_type"] = "infrastructure"
        analysis["likely_cause"] = "Service connection failed"
        analysis["recommendations"].append("Check if all Docker services are running")
        analysis["recommendations"].append("Run: docker compose ps")

    # Service errors present
    elif error_logs:
        # Prioritize by service
        if "september" in service_errors:
            analysis["error_type"] = "september_error"
            analysis["likely_cause"] = "September application error"
        elif "nntp" in service_errors:
            analysis["error_type"] = "nntp_error"
            analysis["likely_cause"] = "NNTP server (renews) error"
        elif "dex" in service_errors:
            analysis["error_type"] = "dex_error"
            analysis["likely_cause"] = "Dex OIDC provider error"
        else:
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "Service error detected"

    return analysis


def format_failure_report(
    capture: TestLogCapture, exception: BaseException | None
) -> str:
    """Format a detailed failure report."""
    analysis = analyze_failure(capture, exception)

    lines = [
        "",
        "=" * 80,
        f"TEST FAILURE ANALYSIS: {capture.test_name}",
        "=" * 80,
        "",
        f"Error Type: {analysis['error_type']}",
        f"Likely Cause: {analysis['likely_cause']}",
        f"Test Duration: {analysis['duration_seconds']:.2f}s",
        "",
    ]

    if analysis["recommendations"]:
        lines.append("Recommendations:")
        for rec in analysis["recommendations"]:
            lines.append(f"  - {rec}")
        lines.append("")

    if analysis["service_errors"]:
        lines.append("Service Errors Detected:")
        for svc_err in analysis["service_errors"]:
            lines.append(f"  [{svc_err['service']}] {svc_err['count']} error(s)")
            for msg in svc_err["messages"]:
                lines.append(f"    - {msg}")
        lines.append("")

    # Include relevant logs
    error_logs = capture.get_error_logs()
    if error_logs:
        lines.append("Error/Warning Logs During Test:")
        lines.append("-" * 40)
        for log in error_logs[:10]:  # Limit to 10 most relevant
            ts_str = (
                log.timestamp.strftime("%H:%M:%S.%f")[:-3]
                if log.timestamp
                else "??:??:??"
            )
            lines.append(f"[{log.service}] {ts_str} {log.level}: {log.message[:200]}")
        if len(error_logs) > 10:
            lines.append(f"  ... and {len(error_logs) - 10} more error logs")
        lines.append("")

    lines.append("=" * 80)

    return "\n".join(lines)


# =============================================================================
# Performance Analysis
# =============================================================================


@dataclass
class SpanTiming:
    """Timing information for a single span."""

    name: str
    duration_ms: float
    count: int = 1
    path: str | None = None
    method: str | None = None
    status: int | None = None
    cache_hit: bool | None = None
    group: str | None = None
    operation: str | None = None


@dataclass
class RequestTiming:
    """Timing information for a complete HTTP request."""

    request_id: str
    method: str
    path: str
    status: int
    duration_ms: float
    timestamp: datetime | None = None
    child_spans: list[SpanTiming] = field(default_factory=list)


@dataclass
class PerformanceMetrics:
    """Performance metrics collected during a test."""

    test_name: str
    test_duration_seconds: float
    requests: list[RequestTiming] = field(default_factory=list)
    span_summary: dict[str, SpanTiming] = field(default_factory=dict)

    def total_request_time_ms(self) -> float:
        """Total time spent in HTTP requests."""
        return sum(r.duration_ms for r in self.requests)

    def request_count(self) -> int:
        """Number of HTTP requests made."""
        return len(self.requests)

    def slowest_requests(self, n: int = 5) -> list[RequestTiming]:
        """Get the N slowest requests."""
        return sorted(self.requests, key=lambda r: r.duration_ms, reverse=True)[:n]

    def slowest_spans(self, n: int = 10) -> list[SpanTiming]:
        """Get the N slowest span types by total time."""
        spans = list(self.span_summary.values())
        return sorted(spans, key=lambda s: s.duration_ms, reverse=True)[:n]


def extract_span_timing(log_data: dict) -> SpanTiming | None:
    """Extract span timing from a log entry if it has duration info."""
    span = log_data.get("span", {})
    if not span or "name" not in span:
        return None

    # Only extract spans that have duration_ms
    duration_ms = span.get("duration_ms")
    if duration_ms is None:
        return None

    return SpanTiming(
        name=span.get("name", "unknown"),
        duration_ms=float(duration_ms),
        path=span.get("path"),
        method=span.get("method"),
        status=log_data.get("fields", {}).get("status"),
        cache_hit=span.get("cache_hit"),
        group=span.get("group"),
        operation=span.get("operation"),
    )


def extract_request_timing(log_data: dict) -> RequestTiming | None:
    """Extract request timing from a 'Request completed' log entry."""
    fields = log_data.get("fields", {})
    if fields.get("message") != "Request completed":
        return None

    span = log_data.get("span", {})
    if span.get("name") != "request":
        return None

    # Parse timestamp
    timestamp = None
    ts_str = log_data.get("timestamp", "")
    if ts_str:
        try:
            ts_str = ts_str.rstrip("Z")
            timestamp = datetime.fromisoformat(ts_str).replace(tzinfo=timezone.utc)
        except ValueError:
            pass

    return RequestTiming(
        request_id=span.get("request_id", "unknown"),
        method=span.get("method", "?"),
        path=span.get("path", "?"),
        status=fields.get("status", 0),
        duration_ms=float(span.get("duration_ms", 0)),
        timestamp=timestamp,
    )


def analyze_performance(
    logs: list[LogEntry], test_name: str, test_duration: float
) -> PerformanceMetrics:
    """Analyze September logs to extract performance metrics."""
    metrics = PerformanceMetrics(
        test_name=test_name,
        test_duration_seconds=test_duration,
    )

    # Track spans by request_id for grouping
    request_spans: dict[str, list[SpanTiming]] = {}

    for log in logs:
        if log.service != "september":
            continue

        # Try to parse as JSON
        try:
            data = json.loads(log.raw)
        except json.JSONDecodeError:
            continue

        # Extract request timing (completed requests)
        request = extract_request_timing(data)
        if request:
            # Attach any child spans we've collected
            if request.request_id in request_spans:
                request.child_spans = request_spans[request.request_id]
            metrics.requests.append(request)
            continue

        # Extract span timing for non-request spans
        span = extract_span_timing(data)
        if span and span.name != "request":
            # Try to associate with a request
            spans_list = data.get("spans", [])
            for s in spans_list:
                if s.get("name") == "request" and "request_id" in s:
                    req_id = s["request_id"]
                    if req_id not in request_spans:
                        request_spans[req_id] = []
                    request_spans[req_id].append(span)
                    break

            # Aggregate into span summary
            if span.name in metrics.span_summary:
                existing = metrics.span_summary[span.name]
                existing.duration_ms += span.duration_ms
                existing.count += 1
            else:
                metrics.span_summary[span.name] = SpanTiming(
                    name=span.name,
                    duration_ms=span.duration_ms,
                    count=1,
                    cache_hit=span.cache_hit,
                    group=span.group,
                    operation=span.operation,
                )

    return metrics


def format_performance_report(metrics: PerformanceMetrics) -> str:
    """Format a performance report for display."""
    lines = [
        "",
        "-" * 80,
        f"PERFORMANCE: {metrics.test_name}",
        "-" * 80,
        "",
        f"Test Duration: {metrics.test_duration_seconds:.2f}s",
        f"HTTP Requests: {metrics.request_count()}",
        f"Total Request Time: {metrics.total_request_time_ms():.0f}ms",
        "",
    ]

    # Slowest requests
    slowest = metrics.slowest_requests(5)
    if slowest:
        lines.append("Slowest Requests:")
        for req in slowest:
            cache_info = ""
            for span in req.child_spans:
                if span.cache_hit is not None:
                    cache_info = " [cached]" if span.cache_hit else " [miss]"
                    break
            lines.append(
                f"  {req.duration_ms:>6.0f}ms  {req.method:4} {req.path[:50]:<50} [{req.status}]{cache_info}"
            )
        lines.append("")

    # Slowest span types
    slowest_spans = metrics.slowest_spans(8)
    if slowest_spans:
        lines.append("Slowest Operations (by total time):")
        for span in slowest_spans:
            avg_ms = span.duration_ms / span.count if span.count > 0 else 0
            lines.append(
                f"  {span.duration_ms:>6.0f}ms total  {span.count:>3}x  {avg_ms:>6.1f}ms avg  {span.name}"
            )
        lines.append("")

    # Request breakdown by path pattern
    path_times: dict[str, tuple[float, int]] = {}
    for req in metrics.requests:
        # Normalize path (remove specific IDs)
        path = req.path
        # Generalize message-id patterns
        if "%3C" in path or "%3E" in path:
            path = re.sub(r"%3C[^%]+%3E", "<msg-id>", path)
        # Generalize thread paths
        if "/thread/" in path:
            path = re.sub(r"/thread/.*", "/thread/<id>", path)

        if path in path_times:
            total, count = path_times[path]
            path_times[path] = (total + req.duration_ms, count + 1)
        else:
            path_times[path] = (req.duration_ms, 1)

    if path_times:
        lines.append("Time by Endpoint:")
        sorted_paths = sorted(path_times.items(), key=lambda x: x[1][0], reverse=True)
        for path, (total_ms, count) in sorted_paths[:8]:
            avg_ms = total_ms / count if count > 0 else 0
            lines.append(
                f"  {total_ms:>6.0f}ms  {count:>3}x  {avg_ms:>6.1f}ms avg  {path[:55]}"
            )
        lines.append("")

    lines.append("-" * 80)

    return "\n".join(lines)


# =============================================================================
# Pytest Hooks for Log Capture and Performance
# =============================================================================

# Environment variable to control performance reporting
# Set to "always" to show on every test, "failure" for only failures, "none" to disable
PERF_REPORT_MODE = os.environ.get("PERF_REPORT", "always")


@pytest.fixture(autouse=True)
def capture_logs_and_performance(request) -> Generator[TestLogCapture, None, None]:
    """Automatically capture logs and performance metrics during test execution."""
    capture = TestLogCapture(
        test_name=request.node.name,
        start_time=datetime.now(timezone.utc),
    )

    yield capture

    capture.end_time = datetime.now(timezone.utc)
    test_duration = (capture.end_time - capture.start_time).total_seconds()

    # Fetch logs from all services
    for service in LOG_SERVICES:
        service_logs = fetch_service_logs(service, capture.start_time)
        capture.logs.extend(service_logs)

    # Check test result
    test_failed = hasattr(request.node, "rep_call") and request.node.rep_call.failed
    test_passed = hasattr(request.node, "rep_call") and request.node.rep_call.passed

    # Generate failure report if test failed
    if test_failed:
        exc_info = request.node.rep_call.longrepr
        exception = None
        if hasattr(exc_info, "reprcrash"):
            exception = Exception(exc_info.reprcrash.message)

        report = format_failure_report(capture, exception)
        print(report)

    # Generate performance report based on mode
    show_perf = PERF_REPORT_MODE == "always" or (
        PERF_REPORT_MODE == "failure" and test_failed
    )

    if show_perf and (test_passed or test_failed):
        metrics = analyze_performance(capture.logs, capture.test_name, test_duration)
        # Only show if there were actual requests
        if metrics.request_count() > 0:
            perf_report = format_performance_report(metrics)
            print(perf_report)


@pytest.hookimpl(tryfirst=True, hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Store test result on the item for access in fixtures."""
    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)


# =============================================================================
# Browser Fixtures
# =============================================================================


@pytest.fixture(scope="session")
def docker_environment():
    """
    Start Docker environment for integration tests.

    This fixture is session-scoped, so Docker services are started once
    at the beginning of the test session and stopped at the end.

    Set SKIP_DOCKER_SETUP=1 to skip Docker setup (useful for local development
    when the environment is already running).
    """
    if os.environ.get("SKIP_DOCKER_SETUP", "").lower() in ("1", "true", "yes"):
        print("Skipping Docker setup (SKIP_DOCKER_SETUP is set)")
        yield
        return

    setup_script = ENVIRONMENT_DIR / "setup.sh"
    teardown_script = ENVIRONMENT_DIR / "teardown.sh"

    # Start the environment
    print(f"\nStarting Docker environment from {ENVIRONMENT_DIR}...")
    result = subprocess.run(
        [str(setup_script)],
        cwd=ENVIRONMENT_DIR,
        capture_output=True,
        text=True,
        timeout=300,  # 5 minutes timeout for building/starting
    )

    if result.returncode != 0:
        print(f"Setup stdout: {result.stdout}")
        print(f"Setup stderr: {result.stderr}")
        raise RuntimeError(f"Failed to start Docker environment: {result.stderr}")

    print("Docker environment started successfully")

    yield

    # Teardown the environment
    print("\nStopping Docker environment...")
    subprocess.run(
        [str(teardown_script)],
        cwd=ENVIRONMENT_DIR,
        capture_output=True,
        text=True,
        timeout=60,
    )
    print("Docker environment stopped")


@pytest.fixture(scope="session")
def browser(docker_environment) -> WebDriver:
    """
    Create a Selenium WebDriver connected to the Chrome container.

    This fixture is session-scoped so the browser persists across all tests,
    making the test suite faster.
    """
    options = webdriver.ChromeOptions()
    # Recommended options for containerized Chrome
    options.add_argument("--no-sandbox")
    options.add_argument("--disable-dev-shm-usage")
    options.add_argument("--disable-gpu")
    options.add_argument("--window-size=1920,1080")

    driver = webdriver.Remote(
        command_executor=SELENIUM_URL,
        options=options,
    )
    # Use a short implicit wait - explicit waits handle specific timing needs
    driver.implicitly_wait(1)

    yield driver

    driver.quit()


@pytest.fixture
def clean_browser(browser: WebDriver) -> WebDriver:
    """
    Provide a browser with cleared state (cookies, local storage).

    Use this fixture when tests need a fresh browser state.
    """
    browser.delete_all_cookies()
    browser.get(f"{SEPTEMBER_URL}/")

    # Clear any local storage
    try:
        browser.execute_script("window.localStorage.clear();")
        browser.execute_script("window.sessionStorage.clear();")
    except Exception:
        pass  # May fail if no page is loaded

    return browser


@pytest.fixture
def authenticated_browser(browser: WebDriver) -> WebDriver:
    """
    Provide a browser with an authenticated user session.

    Logs in via the Dex OIDC provider using the test user credentials.
    Clears the session after the test completes.
    """
    # Start login flow
    browser.get(f"{SEPTEMBER_URL}/auth/login")

    wait = WebDriverWait(browser, WAIT_TIMEOUT_OIDC, poll_frequency=WAIT_TIMEOUT_POLL)
    quick_wait = WebDriverWait(browser, 1, poll_frequency=WAIT_TIMEOUT_POLL)

    # Dex login page - enter credentials
    # Dex shows a "Log in with Email" link when using password connector
    try:
        # Quick check for connector selection page (use short timeout)
        email_link = quick_wait.until(
            EC.element_to_be_clickable((By.LINK_TEXT, "Log in with Email"))
        )
        email_link.click()
    except TimeoutException:
        # Already on the email login form - continue
        pass

    # Fill in email and password
    email_input = wait.until(EC.presence_of_element_located((By.NAME, "login")))
    email_input.clear()
    email_input.send_keys(TEST_USER_EMAIL)

    password_input = browser.find_element(By.NAME, "password")
    password_input.clear()
    password_input.send_keys(TEST_USER_PASSWORD)

    # Submit the form
    submit_button = browser.find_element(By.CSS_SELECTOR, "button[type='submit']")
    submit_button.click()

    # Wait for redirect back to September
    wait.until(EC.url_contains(SEPTEMBER_URL))

    yield browser

    # Cleanup: clear cookies (faster and more reliable than navigating to logout)
    try:
        browser.delete_all_cookies()
    except Exception:
        pass  # Browser may already be closed or unresponsive


@pytest.fixture
def september_url() -> str:
    """Return the base URL for the September application."""
    return SEPTEMBER_URL


@pytest.fixture
def wait_for_page(browser: WebDriver):
    """
    Factory fixture that returns a wait helper function.

    Usage:
        def test_something(browser, wait_for_page):
            browser.get(url)
            wait_for_page(browser, "expected-element-id")
    """

    def _wait_for_element(
        driver: WebDriver, element_id: str, timeout: int = WAIT_TIMEOUT_DEFAULT
    ):
        wait = WebDriverWait(driver, timeout, poll_frequency=WAIT_TIMEOUT_POLL)
        return wait.until(EC.presence_of_element_located((By.ID, element_id)))

    return _wait_for_element


class PageHelpers:
    """Helper methods for interacting with September pages."""

    def __init__(self, driver: WebDriver, base_url: str):
        self.driver = driver
        self.base_url = base_url
        self.wait = WebDriverWait(
            driver, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
        )

    def goto_home(self):
        """Navigate to the homepage."""
        self.driver.get(f"{self.base_url}/")
        self.wait.until(EC.presence_of_element_located((By.CLASS_NAME, "group-cards")))

    def goto_group(self, group: str):
        """Navigate to a group's thread list."""
        self.driver.get(f"{self.base_url}/g/{group}")
        # Wait for thread list or empty state
        self.wait.until(
            EC.presence_of_element_located(
                (By.CSS_SELECTOR, ".thread-list, .empty-state")
            )
        )

    def goto_browse(self, prefix: str = ""):
        """Navigate to browse page."""
        url = f"{self.base_url}/browse/{prefix}" if prefix else f"{self.base_url}/"
        self.driver.get(url)
        self.wait.until(EC.presence_of_element_located((By.CLASS_NAME, "group-cards")))

    def get_page_title(self) -> str:
        """Get the current page title."""
        return self.driver.title

    def is_logged_in(self) -> bool:
        """Check if the user is currently logged in."""
        try:
            # Look for logout button or user info in header
            self.driver.find_element(By.CSS_SELECTOR, ".user-info, [href*='logout']")
            return True
        except Exception:
            return False

    def get_flash_messages(self) -> list[str]:
        """Get any flash/notification messages on the page."""
        try:
            messages = self.driver.find_elements(
                By.CSS_SELECTOR, ".flash-message, .notification"
            )
            return [msg.text for msg in messages]
        except Exception:
            return []


@pytest.fixture
def page_helpers(browser: WebDriver, september_url: str) -> PageHelpers:
    """Provide page helper methods."""
    return PageHelpers(browser, september_url)
