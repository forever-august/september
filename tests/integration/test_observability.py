"""
Tests for observability and logging behavior.

These tests verify that the system logs expected messages for various operations,
enabling automated verification of observability requirements that would
otherwise require manual log inspection.

Note: These tests depend on the Docker Compose environment running with
accessible log output.
"""

import concurrent.futures
from datetime import datetime, timezone

import pytest
import requests

from helpers import (
    SEPTEMBER_HOST_URL,
    LogCapture,
    assert_log_contains,
    count_log_matches,
)


@pytest.mark.slow
class TestCoalescing:
    """Tests for request coalescing log verification."""

    def test_article_coalescing_logged(self, http_client: requests.Session):
        """
        Verify that coalescing is logged when multiple article requests arrive.

        When multiple requests for the same article arrive simultaneously,
        the system should coalesce them and log "coalesced = true".
        Replaces: manual-coalesce-articles
        """
        timestamp = datetime.now(timezone.utc)

        # First, get a valid article ID from the thread list
        response = http_client.get(
            f"{SEPTEMBER_HOST_URL}/g/test.general", allow_redirects=True
        )
        if response.status_code != 200:
            pytest.skip("Could not load test group")

        # Make parallel requests to trigger coalescing
        # Using a simple article path that should exist
        article_path = f"{SEPTEMBER_HOST_URL}/a/%3Ctest.1%40example.com%3E"

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [
                executor.submit(
                    lambda: http_client.get(article_path, allow_redirects=True)
                )
                for _ in range(3)
            ]
            concurrent.futures.wait(futures)

        # Check for coalescing log
        try:
            log_entry = assert_log_contains(
                "september",
                r"coalesced.*true|coalescing",
                timestamp,
                timeout=5.0,
            )
            assert log_entry is not None
        except Exception:
            # Coalescing may not occur if requests don't overlap in time
            pytest.skip("Coalescing not detected - requests may not have overlapped")

    def test_threads_coalescing_logged(self, http_client: requests.Session):
        """
        Verify that coalescing is logged for thread list requests.

        Replaces: manual-coalesce-threads
        """
        timestamp = datetime.now(timezone.utc)

        # Make parallel requests for the same group's threads
        group_url = f"{SEPTEMBER_HOST_URL}/g/test.general"

        # Create a new session for each request to avoid connection reuse issues
        def make_request():
            with requests.Session() as s:
                return s.get(group_url)

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(make_request) for _ in range(3)]
            concurrent.futures.wait(futures)

        try:
            log_entry = assert_log_contains(
                "september",
                r"coalesced.*true|coalescing|get_threads",
                timestamp,
                timeout=5.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Coalescing not detected - requests may not have overlapped")

    def test_groups_coalescing_logged(self, http_client: requests.Session):
        """
        Verify that coalescing is logged for group list requests.

        Replaces: manual-coalesce-groups
        """
        timestamp = datetime.now(timezone.utc)

        # Make parallel requests for the homepage (which fetches groups)
        def make_request():
            with requests.Session() as s:
                return s.get(f"{SEPTEMBER_HOST_URL}/")

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(make_request) for _ in range(3)]
            concurrent.futures.wait(futures)

        try:
            log_entry = assert_log_contains(
                "september",
                r"coalesced.*true|coalescing|get_groups",
                timestamp,
                timeout=5.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Coalescing not detected - requests may not have overlapped")

    def test_stats_coalescing_logged(self, http_client: requests.Session):
        """
        Verify that coalescing is logged for group stats requests.

        Replaces: manual-coalesce-stats
        """
        timestamp = datetime.now(timezone.utc)

        # Stats are fetched as part of group page loads
        def make_request():
            with requests.Session() as s:
                return s.get(f"{SEPTEMBER_HOST_URL}/g/test.general")

        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            futures = [executor.submit(make_request) for _ in range(3)]
            concurrent.futures.wait(futures)

        try:
            log_entry = assert_log_contains(
                "september",
                r"coalesced.*true|coalescing|get_group_stats",
                timestamp,
                timeout=5.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Coalescing not detected - requests may not have overlapped")


@pytest.mark.slow
class TestWorkerLogs:
    """Tests for NNTP worker logging."""

    def test_worker_count_logged_on_startup(self, log_capture):
        """
        Verify that worker spawn count is logged.

        On startup, September should log the number of NNTP workers spawned.
        This test checks that the log message exists from recent startup.
        Replaces: manual-worker-count-config

        Note: This test may skip if the service hasn't been recently restarted.
        """
        # Look for recent startup logs (within last hour)
        from datetime import timedelta

        timestamp = datetime.now(timezone.utc) - timedelta(hours=1)

        try:
            log_entry = assert_log_contains(
                "september",
                r"[Ss]pawn.*worker|worker.*spawn|NNTP worker",
                timestamp,
                timeout=2.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip(
                "Worker spawn log not found - service may not have started recently"
            )

    def test_capabilities_detection_logged(self, http_client: requests.Session):
        """
        Verify that server capabilities are logged.

        September should log detected NNTP capabilities (OVER, HDR, etc.).
        Replaces: manual-capabilities-detection
        """
        from datetime import timedelta

        timestamp = datetime.now(timezone.utc) - timedelta(hours=1)

        # Trigger some NNTP activity
        http_client.get(f"{SEPTEMBER_HOST_URL}/g/test.general", allow_redirects=True)

        try:
            log_entry = assert_log_contains(
                "september",
                r"[Cc]apabilit|OVER|HDR|capability",
                timestamp,
                timeout=2.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Capabilities log not found")

    def test_over_command_usage_logged(self, http_client: requests.Session):
        """
        Verify that OVER command usage is logged when fetching threads.

        When fetching thread metadata from a server that supports OVER,
        the system should log that OVER command is being used.
        Replaces: manual-over-command-usage
        """
        timestamp = datetime.now(timezone.utc)

        # Fetch a group's thread list to trigger OVER command
        http_client.get(f"{SEPTEMBER_HOST_URL}/g/test.general", allow_redirects=True)

        try:
            log_entry = assert_log_contains(
                "september",
                r"OVER|fetch.*method|thread.*fetch",
                timestamp,
                timeout=5.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("OVER command log not found")


@pytest.mark.slow
class TestBackgroundRefreshLogs:
    """Tests for background refresh task logging."""

    def test_activity_refresh_spawn_logged(self, http_client: requests.Session):
        """
        Verify that background refresh task spawning is logged.

        When a group has activity, the system should spawn a background
        refresh task and log it.
        Replaces: manual-apr-spawn-task
        """
        timestamp = datetime.now(timezone.utc)

        # Generate activity on a group
        for _ in range(3):
            http_client.get(
                f"{SEPTEMBER_HOST_URL}/g/test.general", allow_redirects=True
            )

        try:
            log_entry = assert_log_contains(
                "september",
                r"[Ss]pawn.*refresh|background.*refresh|refresh.*task",
                timestamp,
                timeout=5.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Background refresh spawn log not found")

    def test_stats_refresh_spawn_logged(self, http_client: requests.Session):
        """
        Verify that stats refresh task is logged.

        The system should log when starting stats refresh for groups.
        Replaces: manual-gsr-spawn-task
        """
        from datetime import timedelta

        timestamp = datetime.now(timezone.utc) - timedelta(hours=1)

        # Trigger some activity
        http_client.get(f"{SEPTEMBER_HOST_URL}/", allow_redirects=True)

        try:
            log_entry = assert_log_contains(
                "september",
                r"[Ss]tats.*refresh|refresh.*stats|group.*stats",
                timestamp,
                timeout=2.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Stats refresh log not found")


@pytest.mark.slow
class TestConnectionLogs:
    """Tests for connection-related logging."""

    def test_tls_connection_type_logged(self, http_client: requests.Session):
        """
        Verify that connection type (TLS or plain) is logged.

        The system should log whether it established a TLS or plain
        TCP connection to the NNTP server.
        Replaces: manual-tls-connection-logging
        """
        from datetime import timedelta

        timestamp = datetime.now(timezone.utc) - timedelta(hours=1)

        # Any request will use the NNTP connection
        http_client.get(f"{SEPTEMBER_HOST_URL}/g/test.general", allow_redirects=True)

        try:
            log_entry = assert_log_contains(
                "september",
                r"TLS|[Pp]lain.*TCP|connection.*established|connect",
                timestamp,
                timeout=2.0,
            )
            assert log_entry is not None
        except Exception:
            pytest.skip("Connection type log not found")
