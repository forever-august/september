"""
Pytest fixtures for September integration tests.

Provides:
- Selenium WebDriver connected to Chrome container
- Page object factory fixtures
- Authentication helpers
- Log capture and failure analysis for debugging
- Support for parallel test execution with pytest-xdist
"""

import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Generator

import pytest
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait
from selenium.common.exceptions import TimeoutException

# Add the integration test directory to the path for imports
sys.path.insert(0, str(Path(__file__).parent))

import requests

from helpers import (
    LOG_SERVICES,
    LogCapture,
    POLL_FREQUENCY,
    SELENIUM_URL,
    SEPTEMBER_URL,
    TEST_USER_EMAIL,
    TEST_USER_PASSWORD,
    TIMEOUT_DEFAULT,
    TIMEOUT_OIDC,
    Selectors,
)
from testlogging import (
    PerformanceReport,
    RouteTiming,
    TestLogCapture,
    VisibilityReport,
    VisibilityTimer,
    VisibilityTiming,
    clear_performance_entries,
    fetch_service_logs,
    format_failure_report,
    format_performance_report,
    format_visibility_report,
    get_navigation_timing,
    get_resource_timings,
)
from pages import (
    ArticlePage,
    BrowsePage,
    ComposePage,
    DexLoginPage,
    GroupPage,
    HomePage,
    ThreadPage,
)

# Environment variable to control performance reporting
# Set to "always" to show on every test, "failure" for only failures, "none" to disable
PERF_REPORT_MODE = os.environ.get("PERF_REPORT", "none")


def is_xdist_worker() -> bool:
    """Check if we're running as a pytest-xdist worker."""
    return os.environ.get("PYTEST_XDIST_WORKER") is not None


def get_worker_id() -> str:
    """Get the xdist worker ID, or 'master' if not running in parallel."""
    return os.environ.get("PYTEST_XDIST_WORKER", "master")


# =============================================================================
# Pytest Configuration
# =============================================================================


def pytest_addoption(parser):
    """Add custom command-line options."""
    parser.addoption(
        "--repeat",
        action="store",
        default=1,
        type=int,
        help="Number of times to repeat each test for performance statistics",
    )


def pytest_generate_tests(metafunc):
    """Parametrize tests to run multiple times when --repeat is specified."""
    count = metafunc.config.getoption("--repeat")
    if count > 1:
        # Add a fixture that provides the iteration number
        metafunc.fixturenames.append("_repeat_iteration")
        metafunc.parametrize(
            "_repeat_iteration", range(count), ids=[f"run{i}" for i in range(count)]
        )


# =============================================================================
# Pytest Hooks for Log Capture and Performance Tracking
# =============================================================================

# Global performance report for the session (per-worker in parallel mode)
_performance_report: PerformanceReport | None = None
# Global visibility report for the session (per-worker in parallel mode)
_visibility_report: VisibilityReport | None = None
# Current test name for attribution
_current_test_name: str = ""
# Reference to browser for performance capture
_session_browser: WebDriver | None = None
# Temp directory for worker timing files (set by master in parallel mode)
_timing_tmpdir: Path | None = None
# Collected timings from workers (master only)
_worker_timings: list[RouteTiming] = []
# Collected visibility timings from workers (master only)
_worker_visibility_timings: list[VisibilityTiming] = []


def pytest_configure(config):
    """Configure xdist worker communication for performance data."""
    global _timing_tmpdir

    # In parallel mode, set up a shared temp directory for timing data
    if hasattr(config, "workerinput"):
        # Worker: get the timing dir from master
        _timing_tmpdir = Path(config.workerinput.get("timing_tmpdir", ""))
    elif hasattr(config, "pluginmanager") and config.pluginmanager.hasplugin("xdist"):
        # Master with xdist: create timing temp dir
        import tempfile

        _timing_tmpdir = Path(tempfile.mkdtemp(prefix="september_perf_"))


def pytest_configure_node(node):
    """Called on xdist master to configure each worker node."""
    global _timing_tmpdir
    if _timing_tmpdir:
        node.workerinput["timing_tmpdir"] = str(_timing_tmpdir)


def pytest_testnodedown(node, error):
    """Called on xdist master when a worker node finishes. Collect its timings."""
    global _timing_tmpdir, _worker_timings, _worker_visibility_timings

    if _timing_tmpdir and _timing_tmpdir.exists():
        worker_id = node.workerinput.get("workerid", "unknown")

        # Collect route timings
        timing_file = _timing_tmpdir / f"timings_{worker_id}.json"
        if timing_file.exists():
            try:
                data = json.loads(timing_file.read_text())
                for t in data:
                    _worker_timings.append(
                        RouteTiming(
                            url=t["url"],
                            method=t["method"],
                            duration_ms=t["duration_ms"],
                            ttfb_ms=t["ttfb_ms"],
                            test_name=t["test_name"],
                        )
                    )
            except Exception:
                pass

        # Collect visibility timings
        visibility_file = _timing_tmpdir / f"visibility_{worker_id}.json"
        if visibility_file.exists():
            try:
                data = json.loads(visibility_file.read_text())
                for t in data:
                    _worker_visibility_timings.append(VisibilityTiming.from_dict(t))
            except Exception:
                pass


def pytest_sessionstart(session):
    """Initialize performance tracking at the start of the test session."""
    global _performance_report, _visibility_report
    _performance_report = PerformanceReport(session_start=datetime.now(timezone.utc))
    _visibility_report = VisibilityReport()


def pytest_runtest_setup(item):
    """Record current test name for route timing attribution."""
    global _current_test_name
    _current_test_name = item.name


@pytest.hookimpl(tryfirst=True, hookwrapper=True)
def pytest_runtest_makereport(item, call):
    """Store test result on the item and capture route timings after test execution."""
    global _performance_report, _current_test_name, _session_browser

    outcome = yield
    rep = outcome.get_result()
    setattr(item, f"rep_{rep.when}", rep)

    # Capture route timings after the call phase (actual test execution)
    if (
        rep.when == "call"
        and _session_browser is not None
        and _performance_report is not None
    ):
        try:
            # Capture navigation timing for the current page
            nav_timing = get_navigation_timing(_session_browser, _current_test_name)
            if nav_timing:
                _performance_report.route_timings.append(nav_timing)

            # Capture resource timings (XHR, fetch, etc.)
            resource_timings = get_resource_timings(
                _session_browser, _current_test_name
            )
            _performance_report.route_timings.extend(resource_timings)

            # Clear entries to avoid duplicates in next test
            clear_performance_entries(_session_browser)
        except Exception:
            pass  # Don't fail tests due to performance capture issues


def pytest_sessionfinish(session, exitstatus):
    """Print the performance and visibility reports at the end of the test session."""
    global _performance_report, _visibility_report
    global _timing_tmpdir, _worker_timings, _worker_visibility_timings

    if is_xdist_worker():
        # Worker: write timings to files for master to collect
        if _timing_tmpdir:
            worker_id = get_worker_id()

            # Write route timings
            if _performance_report is not None and _performance_report.route_timings:
                timing_file = _timing_tmpdir / f"timings_{worker_id}.json"
                timings_data = [
                    {
                        "url": t.url,
                        "method": t.method,
                        "duration_ms": t.duration_ms,
                        "ttfb_ms": t.ttfb_ms,
                        "test_name": t.test_name,
                    }
                    for t in _performance_report.route_timings
                ]
                timing_file.write_text(json.dumps(timings_data))

            # Write visibility timings
            if _visibility_report is not None and _visibility_report.timings:
                visibility_file = _timing_tmpdir / f"visibility_{worker_id}.json"
                visibility_data = [t.to_dict() for t in _visibility_report.timings]
                visibility_file.write_text(json.dumps(visibility_data))
    else:
        # Master or non-parallel: aggregate and print reports
        if _performance_report is None:
            _performance_report = PerformanceReport(
                session_start=datetime.now(timezone.utc)
            )
        if _visibility_report is None:
            _visibility_report = VisibilityReport()

        _performance_report.session_end = datetime.now(timezone.utc)

        # Combine local timings (non-parallel) with worker timings (parallel)
        all_timings = list(_performance_report.route_timings) + _worker_timings
        all_visibility = list(_visibility_report.timings) + _worker_visibility_timings

        # Clean up temp dir if it exists
        if _timing_tmpdir and _timing_tmpdir.exists():
            import shutil

            shutil.rmtree(_timing_tmpdir, ignore_errors=True)

        # Print route performance report
        if all_timings:
            _performance_report.route_timings = all_timings
            report = format_performance_report(_performance_report)
            print(f"\n{report}")

        # Print visibility latency report
        if all_visibility:
            _visibility_report.timings = all_visibility
            visibility_report = format_visibility_report(_visibility_report)
            print(f"\n{visibility_report}")


# =============================================================================
# Browser Fixtures
# =============================================================================


@pytest.fixture(scope="session")
def browser() -> Generator[WebDriver, None, None]:
    """
    Create a Selenium WebDriver connected to the Chrome container.

    This fixture is session-scoped so the browser persists across all tests
    within a single worker, making the test suite faster.

    In parallel mode (pytest-xdist), each worker gets its own browser instance.
    Note: The Selenium container supports up to 4 concurrent sessions
    (SE_NODE_MAX_SESSIONS=4), so use --parallel 4 or fewer workers.

    Note: Docker environment must be started before running tests.
    Use ./test.sh to run tests with automatic environment setup.
    """
    global _session_browser

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

    # Disable browser cache for accurate performance measurements
    driver.execute_cdp_cmd("Network.setCacheDisabled", {"cacheDisabled": True})

    # Store reference for performance capture
    _session_browser = driver

    yield driver

    _session_browser = None
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
def authenticated_browser(browser: WebDriver) -> Generator[WebDriver, None, None]:
    """
    Provide a browser with an authenticated user session.

    Logs in via the Dex OIDC provider using the test user credentials.
    Clears the session after the test completes.
    """
    # Start login flow
    browser.get(f"{SEPTEMBER_URL}/auth/login")

    wait = WebDriverWait(browser, TIMEOUT_OIDC, poll_frequency=POLL_FREQUENCY)
    quick_wait = WebDriverWait(browser, 1, poll_frequency=POLL_FREQUENCY)

    # Dex login page - enter credentials
    # Dex shows a "Log in with Email" link when using password connector
    try:
        # Quick check for connector selection page (use short timeout)
        email_link = quick_wait.until(
            EC.element_to_be_clickable(
                (By.LINK_TEXT, Selectors.Dex.EMAIL_CONNECTOR_TEXT)
            )
        )
        email_link.click()
    except TimeoutException:
        # Already on the email login form - continue
        pass

    # Fill in email and password
    email_input = wait.until(
        EC.presence_of_element_located((By.NAME, Selectors.Dex.LOGIN_INPUT_NAME))
    )
    email_input.clear()
    email_input.send_keys(TEST_USER_EMAIL)

    password_input = browser.find_element(By.NAME, Selectors.Dex.PASSWORD_INPUT_NAME)
    password_input.clear()
    password_input.send_keys(TEST_USER_PASSWORD)

    # Submit the form
    submit_button = browser.find_element(By.CSS_SELECTOR, Selectors.Dex.SUBMIT)
    submit_button.click()

    # Wait for redirect back to September
    wait.until(EC.url_contains(SEPTEMBER_URL))

    yield browser

    # Cleanup: clear cookies (faster and more reliable than navigating to logout)
    try:
        browser.delete_all_cookies()
    except Exception:
        pass  # Browser may already be closed or unresponsive


# =============================================================================
# Page Object Factory Fixtures
# =============================================================================


@pytest.fixture
def home_page(browser: WebDriver) -> Callable[[], HomePage]:
    """Factory fixture for HomePage."""

    def _create() -> HomePage:
        return HomePage(browser).load()

    return _create


@pytest.fixture
def browse_page(browser: WebDriver) -> Callable[[str], BrowsePage]:
    """Factory fixture for BrowsePage."""

    def _create(prefix: str = "") -> BrowsePage:
        return BrowsePage(browser, prefix).load()

    return _create


@pytest.fixture
def group_page(browser: WebDriver) -> Callable[[str], GroupPage]:
    """Factory fixture for GroupPage."""

    def _create(group_name: str) -> GroupPage:
        return GroupPage(browser, group_name).load()

    return _create


@pytest.fixture
def compose_page(authenticated_browser: WebDriver) -> Callable[[str], ComposePage]:
    """Factory fixture for ComposePage (requires auth)."""

    def _create(group_name: str) -> ComposePage:
        return ComposePage(authenticated_browser, group_name).load()

    return _create


@pytest.fixture
def compose_page_unauth(browser: WebDriver) -> Callable[[str], ComposePage]:
    """Factory fixture for ComposePage without authentication."""

    def _create(group_name: str) -> ComposePage:
        return ComposePage(browser, group_name).load()

    return _create


@pytest.fixture
def dex_page(browser: WebDriver) -> DexLoginPage:
    """Fixture for DexLoginPage."""
    return DexLoginPage(browser)


# =============================================================================
# Performance Measurement Fixtures
# =============================================================================


@pytest.fixture
def visibility_timer(
    authenticated_browser: WebDriver,
) -> Generator[VisibilityTimer, None, None]:
    """
    Fixture for measuring post/reply visibility latency.

    Provides a VisibilityTimer that can be used to measure the time between
    submitting a post/reply and when it becomes visible on the page.

    Usage:
        def test_post_visibility(self, compose_page, visibility_timer):
            page = compose_page("test.general")
            unique_id = str(uuid.uuid4())[:8]

            page.fill_subject(f"Test {unique_id}")
            page.fill_body(f"Content {unique_id}")

            visibility_timer.mark_submit("post", "test.general", unique_id)
            page.submit()

            timing = visibility_timer.wait_for_visible(unique_id, ".thread-title")
            # timing.latency_ms contains the measured latency
    """
    global _visibility_report, _current_test_name

    timer = VisibilityTimer(authenticated_browser, _current_test_name)
    yield timer

    # After test completes, collect any recorded timing
    if timer.timing is not None and _visibility_report is not None:
        _visibility_report.timings.append(timer.timing)


# =============================================================================
# Legacy Fixtures (for backward compatibility during transition)
# =============================================================================


@pytest.fixture
def september_url() -> str:
    """Return the base URL for the September application."""
    return SEPTEMBER_URL


# =============================================================================
# HTTP Client Fixtures
# =============================================================================


@pytest.fixture
def http_client() -> Generator[requests.Session, None, None]:
    """
    Provide a requests.Session for direct HTTP calls to September.

    Use this for testing HTTP headers, status codes, and other low-level
    HTTP behavior that's hard to verify through Selenium.

    The session is configured with no automatic redirect following for
    cases where you need to inspect redirect responses.
    """
    session = requests.Session()
    # Don't follow redirects by default - tests can override per-request
    session.max_redirects = 0
    yield session
    session.close()


@pytest.fixture
def log_timestamp() -> datetime:
    """
    Provide a timestamp for log assertions.

    Use this fixture to mark the start of an operation, then pass
    the timestamp to log assertion helpers to only check logs after
    that point.

    Usage:
        def test_something(log_timestamp, http_client):
            # log_timestamp is captured at fixture creation
            http_client.get(f"{SEPTEMBER_URL}/some/endpoint")
            assert_log_contains("september", "expected message", log_timestamp)
    """
    return datetime.now(timezone.utc)


@pytest.fixture
def log_capture() -> Callable[[str], LogCapture]:
    """
    Factory fixture for LogCapture context managers.

    Usage:
        def test_something(log_capture, http_client):
            with log_capture("september") as capture:
                http_client.get(f"{SEPTEMBER_URL}/some/endpoint")

            assert capture.contains("expected message")
            assert capture.count("pattern") == 1
    """

    def _create(service: str) -> LogCapture:
        return LogCapture(service)

    return _create
