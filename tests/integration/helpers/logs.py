"""Log assertion helpers for integration tests.

Provides utilities to verify log messages during tests, enabling automated
verification of observability requirements that would otherwise require manual testing.
"""

import re
import time
from datetime import datetime, timezone
from typing import Callable

from testlogging import LogEntry, fetch_service_logs


class LogAssertionError(Exception):
    """Raised when a log assertion fails."""

    pass


def fetch_logs_containing(
    service: str,
    pattern: str,
    since: datetime,
    timeout: float = 5.0,
    poll_interval: float = 0.5,
) -> list[LogEntry]:
    """
    Fetch logs from a service that match a pattern.

    Args:
        service: Docker service name (e.g., "september", "nntp")
        pattern: Regex pattern to match against log messages
        since: Only return logs after this timestamp
        timeout: Maximum time to wait for matching logs
        poll_interval: Time between log fetch attempts

    Returns:
        List of matching LogEntry objects
    """
    regex = re.compile(pattern, re.IGNORECASE)
    deadline = time.monotonic() + timeout
    matches: list[LogEntry] = []

    while time.monotonic() < deadline:
        logs = fetch_service_logs(service, since)
        matches = [
            entry
            for entry in logs
            if regex.search(entry.message) or regex.search(entry.raw)
        ]
        if matches:
            return matches
        time.sleep(poll_interval)

    return matches


def assert_log_contains(
    service: str,
    pattern: str,
    since: datetime,
    timeout: float = 5.0,
    poll_interval: float = 0.5,
) -> LogEntry:
    """
    Assert that a log message matching the pattern exists.

    Args:
        service: Docker service name (e.g., "september", "nntp")
        pattern: Regex pattern to match against log messages
        since: Only consider logs after this timestamp
        timeout: Maximum time to wait for matching log
        poll_interval: Time between log fetch attempts

    Returns:
        The first matching LogEntry

    Raises:
        LogAssertionError: If no matching log is found within timeout
    """
    matches = fetch_logs_containing(service, pattern, since, timeout, poll_interval)
    if not matches:
        # Fetch all logs for debugging
        all_logs = fetch_service_logs(service, since)
        log_sample = "\n".join(
            f"  [{e.level}] {e.message[:100]}" for e in all_logs[:10]
        )
        raise LogAssertionError(
            f"No log matching pattern '{pattern}' found in {service} logs "
            f"within {timeout}s.\n"
            f"Recent logs ({len(all_logs)} total):\n{log_sample}"
        )
    return matches[0]


def assert_log_not_contains(
    service: str,
    pattern: str,
    since: datetime,
    wait_time: float = 2.0,
) -> None:
    """
    Assert that no log message matches the pattern.

    Waits for wait_time to ensure the log doesn't appear.

    Args:
        service: Docker service name
        pattern: Regex pattern that should NOT match
        since: Only consider logs after this timestamp
        wait_time: Time to wait before asserting absence

    Raises:
        LogAssertionError: If a matching log is found
    """
    time.sleep(wait_time)
    matches = fetch_logs_containing(service, pattern, since, timeout=0.1)
    if matches:
        raise LogAssertionError(
            f"Unexpected log matching pattern '{pattern}' found in {service} logs:\n"
            f"  {matches[0].message}"
        )


def wait_for_log_message(
    service: str,
    pattern: str,
    since: datetime,
    timeout: float = 5.0,
    poll_interval: float = 0.5,
) -> LogEntry:
    """
    Wait for a log message matching the pattern to appear.

    This is an alias for assert_log_contains with clearer semantics
    for use cases where you're waiting for an async operation.

    Args:
        service: Docker service name
        pattern: Regex pattern to match
        since: Only consider logs after this timestamp
        timeout: Maximum time to wait
        poll_interval: Time between checks

    Returns:
        The matching LogEntry

    Raises:
        LogAssertionError: If no match found within timeout
    """
    return assert_log_contains(service, pattern, since, timeout, poll_interval)


def assert_log_field_equals(
    service: str,
    message_pattern: str,
    field: str,
    expected_value: str,
    since: datetime,
    timeout: float = 5.0,
) -> LogEntry:
    """
    Assert that a log message has a specific field value.

    Useful for verifying structured log fields like request_id, group, etc.

    Args:
        service: Docker service name
        message_pattern: Regex pattern to find the log message
        field: Field name to check (from LogEntry.fields)
        expected_value: Expected value for the field
        since: Only consider logs after this timestamp
        timeout: Maximum time to wait

    Returns:
        The matching LogEntry

    Raises:
        LogAssertionError: If no matching log with correct field value found
    """
    matches = fetch_logs_containing(service, message_pattern, since, timeout)
    for entry in matches:
        if entry.fields.get(field) == expected_value:
            return entry

    if matches:
        actual_values = [entry.fields.get(field, "<missing>") for entry in matches]
        raise LogAssertionError(
            f"Found {len(matches)} logs matching '{message_pattern}' but none had "
            f"{field}='{expected_value}'.\n"
            f"Actual values: {actual_values}"
        )
    raise LogAssertionError(
        f"No log matching pattern '{message_pattern}' found in {service} logs"
    )


def count_log_matches(
    service: str,
    pattern: str,
    since: datetime,
    timeout: float = 2.0,
) -> int:
    """
    Count the number of log messages matching a pattern.

    Useful for verifying that coalescing occurred (expect fewer NNTP commands
    than HTTP requests).

    Args:
        service: Docker service name
        pattern: Regex pattern to match
        since: Only consider logs after this timestamp
        timeout: Time to wait for logs to settle

    Returns:
        Number of matching log entries
    """
    matches = fetch_logs_containing(service, pattern, since, timeout)
    return len(matches)


class LogCapture:
    """
    Context manager for capturing logs during a specific operation.

    Usage:
        with LogCapture("september") as capture:
            # perform operation
            pass

        assert capture.contains("expected message")
        assert capture.count("pattern") == 1
    """

    def __init__(self, service: str):
        self.service = service
        self.start_time: datetime | None = None
        self.logs: list[LogEntry] = []

    def __enter__(self) -> "LogCapture":
        self.start_time = datetime.now(timezone.utc)
        return self

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        if self.start_time:
            # Small delay to ensure logs are flushed
            time.sleep(0.5)
            self.logs = fetch_service_logs(self.service, self.start_time)

    def contains(self, pattern: str) -> bool:
        """Check if any log matches the pattern."""
        regex = re.compile(pattern, re.IGNORECASE)
        return any(
            regex.search(entry.message) or regex.search(entry.raw)
            for entry in self.logs
        )

    def count(self, pattern: str) -> int:
        """Count logs matching the pattern."""
        regex = re.compile(pattern, re.IGNORECASE)
        return sum(
            1
            for entry in self.logs
            if regex.search(entry.message) or regex.search(entry.raw)
        )

    def find(self, pattern: str) -> list[LogEntry]:
        """Find all logs matching the pattern."""
        regex = re.compile(pattern, re.IGNORECASE)
        return [
            entry
            for entry in self.logs
            if regex.search(entry.message) or regex.search(entry.raw)
        ]

    def assert_contains(self, pattern: str) -> LogEntry:
        """Assert that at least one log matches the pattern."""
        matches = self.find(pattern)
        if not matches:
            log_sample = "\n".join(
                f"  [{e.level}] {e.message[:100]}" for e in self.logs[:10]
            )
            raise LogAssertionError(
                f"No log matching pattern '{pattern}' found.\n"
                f"Captured logs ({len(self.logs)} total):\n{log_sample}"
            )
        return matches[0]
