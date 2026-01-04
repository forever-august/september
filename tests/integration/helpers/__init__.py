"""Helper utilities for integration tests."""

from .data import (
    DEX_URL,
    LOG_SERVICES,
    NNTP_PASSWORD,
    NNTP_USERNAME,
    SELENIUM_URL,
    SEPTEMBER_HOST_URL,
    SEPTEMBER_URL,
    TEST_GROUPS,
    TEST_USER_EMAIL,
    TEST_USER_NAME,
    TEST_USER_PASSWORD,
)
from .exceptions import (
    AuthenticationError,
    ElementNotFoundError,
    IntegrationTestError,
    NavigationError,
    NoTestDataError,
    PageLoadError,
)
from .selectors import Selectors
from .waits import (
    POLL_FREQUENCY,
    TIMEOUT_DEFAULT,
    TIMEOUT_OIDC,
    create_wait,
    element_has_non_empty_text,
    url_matches_any,
    wait_for_element,
    wait_for_navigation_from,
    wait_for_url_contains,
    wait_for_url_not_contains,
)
from .logs import (
    LogAssertionError,
    LogCapture,
    assert_log_contains,
    assert_log_field_equals,
    assert_log_not_contains,
    count_log_matches,
    fetch_logs_containing,
    wait_for_log_message,
)

__all__ = [
    # Data
    "SELENIUM_URL",
    "SEPTEMBER_URL",
    "SEPTEMBER_HOST_URL",
    "DEX_URL",
    "TEST_USER_EMAIL",
    "TEST_USER_PASSWORD",
    "TEST_USER_NAME",
    "TEST_GROUPS",
    "LOG_SERVICES",
    "NNTP_USERNAME",
    "NNTP_PASSWORD",
    # Exceptions
    "IntegrationTestError",
    "PageLoadError",
    "ElementNotFoundError",
    "NoTestDataError",
    "AuthenticationError",
    "NavigationError",
    # Selectors
    "Selectors",
    # Waits
    "TIMEOUT_DEFAULT",
    "TIMEOUT_OIDC",
    "POLL_FREQUENCY",
    "create_wait",
    "wait_for_element",
    "wait_for_url_contains",
    "wait_for_url_not_contains",
    "wait_for_navigation_from",
    "url_matches_any",
    "element_has_non_empty_text",
    # Log assertions
    "LogAssertionError",
    "LogCapture",
    "assert_log_contains",
    "assert_log_field_equals",
    "assert_log_not_contains",
    "count_log_matches",
    "fetch_logs_containing",
    "wait_for_log_message",
]
