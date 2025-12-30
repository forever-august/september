"""Visibility latency measurement utilities.

Measures the time between posting an article/reply and when it becomes
visible to the user. This is an opt-in performance metric that tests
can use by requesting the `visibility_timer` fixture.
"""

import time
from dataclasses import dataclass, field

from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver

from .models import _percentile

# Polling interval for visibility checks (10ms for high accuracy)
VISIBILITY_POLL_INTERVAL = 0.01
# Default timeout for waiting for content to appear
VISIBILITY_TIMEOUT = 10.0


@dataclass
class VisibilityTiming:
    """Timing information for content visibility after posting."""

    content_type: str  # "post" or "reply"
    latency_ms: float  # Time from submit to visible
    test_name: str  # Which test recorded this timing
    group: str  # The newsgroup posted to
    unique_id: str  # The content identifier searched for

    def to_dict(self) -> dict:
        """Convert to dictionary for JSON serialization."""
        return {
            "content_type": self.content_type,
            "latency_ms": self.latency_ms,
            "test_name": self.test_name,
            "group": self.group,
            "unique_id": self.unique_id,
        }

    @classmethod
    def from_dict(cls, data: dict) -> "VisibilityTiming":
        """Create from dictionary (JSON deserialization)."""
        return cls(
            content_type=data["content_type"],
            latency_ms=data["latency_ms"],
            test_name=data["test_name"],
            group=data["group"],
            unique_id=data["unique_id"],
        )


@dataclass
class VisibilityReport:
    """Aggregated visibility latency metrics."""

    timings: list[VisibilityTiming] = field(default_factory=list)

    @property
    def post_timings(self) -> list[VisibilityTiming]:
        """Get all post visibility timings."""
        return [t for t in self.timings if t.content_type == "post"]

    @property
    def reply_timings(self) -> list[VisibilityTiming]:
        """Get all reply visibility timings."""
        return [t for t in self.timings if t.content_type == "reply"]

    def _stats(self, timings: list[VisibilityTiming]) -> dict[str, float]:
        """Calculate statistics for a list of timings."""
        if not timings:
            return {
                "count": 0,
                "avg": 0,
                "min": 0,
                "max": 0,
                "p50": 0,
                "p90": 0,
                "p99": 0,
            }

        latencies = [t.latency_ms for t in timings]
        return {
            "count": len(latencies),
            "avg": sum(latencies) / len(latencies),
            "min": min(latencies),
            "max": max(latencies),
            "p50": _percentile(latencies, 50),
            "p90": _percentile(latencies, 90),
            "p99": _percentile(latencies, 99),
        }

    @property
    def post_stats(self) -> dict[str, float]:
        """Get statistics for post visibility latency."""
        return self._stats(self.post_timings)

    @property
    def reply_stats(self) -> dict[str, float]:
        """Get statistics for reply visibility latency."""
        return self._stats(self.reply_timings)


class VisibilityTimer:
    """Timer for measuring post/reply visibility latency.

    Usage:
        timer = VisibilityTimer(driver, "test_name")
        timer.mark_submit("post", "test.general", "abc123")
        # ... click submit button ...
        timing = timer.wait_for_visible("abc123", ".thread-title")
    """

    def __init__(self, driver: WebDriver, test_name: str):
        self.driver = driver
        self.test_name = test_name
        self._submit_time: float | None = None
        self._content_type: str | None = None
        self._group: str | None = None
        self._unique_id: str | None = None
        self.timing: VisibilityTiming | None = None

    def mark_submit(
        self, content_type: str, group: str, unique_id: str
    ) -> "VisibilityTimer":
        """Mark the timestamp just before clicking submit.

        Args:
            content_type: Either "post" or "reply"
            group: The newsgroup being posted to
            unique_id: A unique identifier in the content to search for

        Returns:
            self for method chaining
        """
        self._submit_time = time.perf_counter()
        self._content_type = content_type
        self._group = group
        self._unique_id = unique_id
        return self

    def _page_has_error(self) -> bool:
        """Check if the current page shows an error state.

        Detects 500 errors, blank pages, or error messages that indicate
        we should refresh and try again.
        """
        try:
            # Check page title for error indicators
            title = self.driver.title.lower()
            if "error" in title or "500" in title:
                return True

            # Check for common error patterns in page source
            # (faster than finding elements)
            page_source = self.driver.page_source.lower()
            error_patterns = [
                "internal server error",
                "500 internal",
                "something went wrong",
                "protocol error",
            ]
            for pattern in error_patterns:
                if pattern in page_source:
                    return True

            return False
        except Exception:
            # If we can't check, assume no error
            return False

    def wait_for_visible(
        self,
        unique_id: str,
        selector: str,
        timeout: float = VISIBILITY_TIMEOUT,
    ) -> VisibilityTiming:
        """Wait for content containing unique_id to appear and return timing.

        Polls the page for elements matching `selector` that contain the
        `unique_id` text. Returns the timing when found.

        If the page shows an error (e.g., 500), it will automatically refresh
        and continue polling. This handles race conditions where the content
        hasn't been stored yet when the page first loads.

        Args:
            unique_id: The unique string to search for in element text
            selector: CSS selector for elements to check
            timeout: Maximum seconds to wait

        Returns:
            VisibilityTiming with the measured latency

        Raises:
            TimeoutError: If content not found within timeout
            ValueError: If mark_submit() was not called first
        """
        if self._submit_time is None:
            raise ValueError("mark_submit() must be called before wait_for_visible()")

        deadline = time.perf_counter() + timeout
        last_elements_count = 0
        last_url = ""
        refresh_count = 0
        max_refreshes = 15  # Limit refreshes to avoid infinite loops
        # Don't refresh too frequently - wait at least this long between refreshes
        min_refresh_interval = 0.75
        last_refresh_time = 0.0
        # URLs that indicate a form page (not the result page)
        form_url_patterns = ["/reply", "/post", "/compose"]

        # Instrumentation for debugging latency
        navigation_complete_time = None
        first_content_check_time = None

        while time.perf_counter() < deadline:
            try:
                current_url = self.driver.current_url
                if current_url != last_url:
                    last_url = current_url

                # If we're still on a form page (e.g., /reply, /post),
                # the form submission hasn't completed navigation yet - keep waiting
                still_on_form = any(
                    pattern in current_url for pattern in form_url_patterns
                )
                if still_on_form:
                    time.sleep(VISIBILITY_POLL_INTERVAL)
                    continue

                # Record when navigation completed (first time we're not on form page)
                if navigation_complete_time is None:
                    navigation_complete_time = time.perf_counter()

                # Check if page has an error and we should refresh
                current_time = time.perf_counter()
                if (
                    self._page_has_error()
                    and refresh_count < max_refreshes
                    and (current_time - last_refresh_time) >= min_refresh_interval
                ):
                    refresh_count += 1
                    last_refresh_time = current_time
                    self.driver.refresh()
                    time.sleep(VISIBILITY_POLL_INTERVAL)
                    continue

                # Record first content check time
                if first_content_check_time is None:
                    first_content_check_time = time.perf_counter()

                elements = self.driver.find_elements(By.CSS_SELECTOR, selector)
                last_elements_count = len(elements)

                for elem in elements:
                    if unique_id in elem.text:
                        # Found it - calculate latency
                        end_time = time.perf_counter()
                        latency_ms = (end_time - self._submit_time) * 1000

                        # Log timing breakdown for debugging
                        if navigation_complete_time:
                            nav_ms = (
                                navigation_complete_time - self._submit_time
                            ) * 1000
                            check_ms = (
                                (first_content_check_time - navigation_complete_time)
                                * 1000
                                if first_content_check_time
                                else 0
                            )
                            find_ms = (
                                (end_time - first_content_check_time) * 1000
                                if first_content_check_time
                                else 0
                            )
                            print(
                                f"    [DEBUG] Timing breakdown: nav={nav_ms:.0f}ms, check_overhead={check_ms:.0f}ms, find={find_ms:.0f}ms, total={latency_ms:.0f}ms"
                            )

                        self.timing = VisibilityTiming(
                            content_type=self._content_type or "unknown",
                            latency_ms=latency_ms,
                            test_name=self.test_name,
                            group=self._group or "unknown",
                            unique_id=unique_id,
                        )
                        return self.timing

                # Content not found yet. If elements exist but don't have the
                # content (cached page), or no elements at all, try refreshing
                # to trigger cache update.
                if (
                    refresh_count < max_refreshes
                    and (current_time - last_refresh_time) >= min_refresh_interval
                ):
                    refresh_count += 1
                    last_refresh_time = current_time
                    self.driver.refresh()
                    time.sleep(VISIBILITY_POLL_INTERVAL)
                    continue

            except Exception:
                # Element might be stale or page transitioning - continue polling
                pass

            time.sleep(VISIBILITY_POLL_INTERVAL)

        # Collect debug info for error message
        debug_info = (
            f"URL: {last_url}, Elements found: {last_elements_count}, "
            f"Refreshes: {refresh_count}"
        )
        raise TimeoutError(
            f"Content with '{unique_id}' not found in elements matching '{selector}' "
            f"within {timeout}s. {debug_info}"
        )


def format_visibility_report(report: VisibilityReport) -> str:
    """Format a visibility latency report."""
    if not report.timings:
        return ""

    lines = [
        "",
        "=" * 80,
        "VISIBILITY LATENCY REPORT",
        "=" * 80,
        "",
    ]

    # Summary
    post_stats = report.post_stats
    reply_stats = report.reply_stats

    lines.append("Summary")
    lines.append("-" * 40)
    lines.append(f"  New posts measured:  {int(post_stats['count'])}")
    lines.append(f"  Replies measured:    {int(reply_stats['count'])}")
    lines.append("")

    # Post latency stats
    if post_stats["count"] > 0:
        lines.append("New Post Latency (submit to visible)")
        lines.append("-" * 40)
        lines.append(f"  Avg:     {post_stats['avg']:.0f}ms")
        lines.append(f"  Min:     {post_stats['min']:.0f}ms")
        lines.append(f"  Max:     {post_stats['max']:.0f}ms")
        lines.append(f"  P50:     {post_stats['p50']:.0f}ms")
        lines.append(f"  P90:     {post_stats['p90']:.0f}ms")
        lines.append(f"  P99:     {post_stats['p99']:.0f}ms")
        lines.append("")

    # Reply latency stats
    if reply_stats["count"] > 0:
        lines.append("Reply Latency (submit to visible)")
        lines.append("-" * 40)
        lines.append(f"  Avg:     {reply_stats['avg']:.0f}ms")
        lines.append(f"  Min:     {reply_stats['min']:.0f}ms")
        lines.append(f"  Max:     {reply_stats['max']:.0f}ms")
        lines.append(f"  P50:     {reply_stats['p50']:.0f}ms")
        lines.append(f"  P90:     {reply_stats['p90']:.0f}ms")
        lines.append(f"  P99:     {reply_stats['p99']:.0f}ms")
        lines.append("")

    # Individual measurements
    lines.append("Individual Measurements")
    lines.append("-" * 40)
    for i, timing in enumerate(report.timings, 1):
        lines.append(
            f"  {i:2}. {timing.content_type:<6} {timing.latency_ms:>7.0f}ms  "
            f"{timing.test_name} ({timing.unique_id})"
        )
    lines.append("")

    lines.append("=" * 80)

    return "\n".join(lines)
