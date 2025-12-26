"""
Pytest fixtures for September integration tests.

Provides:
- Selenium WebDriver connected to Chrome container
- Page object factory fixtures
- Authentication helpers
- Log capture and failure analysis for debugging
"""

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

from helpers import (
    LOG_SERVICES,
    POLL_FREQUENCY,
    SELENIUM_URL,
    SEPTEMBER_URL,
    TEST_USER_EMAIL,
    TEST_USER_PASSWORD,
    TIMEOUT_DEFAULT,
    TIMEOUT_OIDC,
    Selectors,
)
from testlogging import TestLogCapture, fetch_service_logs, format_failure_report
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


# =============================================================================
# Pytest Hooks for Log Capture
# =============================================================================


@pytest.fixture(autouse=True)
def capture_logs_on_failure(request) -> Generator[TestLogCapture, None, None]:
    """Automatically capture logs during test execution for failure analysis."""
    capture = TestLogCapture(
        test_name=request.node.name,
        start_time=datetime.now(timezone.utc),
    )

    yield capture

    capture.end_time = datetime.now(timezone.utc)

    # Check test result
    test_failed = hasattr(request.node, "rep_call") and request.node.rep_call.failed

    # Only fetch logs and generate report on failure
    if test_failed:
        # Fetch logs from all services
        for service in LOG_SERVICES:
            service_logs = fetch_service_logs(service, capture.start_time)
            capture.logs.extend(service_logs)

        exc_info = request.node.rep_call.longrepr
        exception = None
        if hasattr(exc_info, "reprcrash"):
            exception = Exception(exc_info.reprcrash.message)

        report = format_failure_report(capture, exception)
        print(report)


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
def browser() -> Generator[WebDriver, None, None]:
    """
    Create a Selenium WebDriver connected to the Chrome container.

    This fixture is session-scoped so the browser persists across all tests,
    making the test suite faster.

    Note: Docker environment must be started before running tests.
    Use ./test.sh to run tests with automatic environment setup.
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
# Legacy Fixtures (for backward compatibility during transition)
# =============================================================================


@pytest.fixture
def september_url() -> str:
    """Return the base URL for the September application."""
    return SEPTEMBER_URL
