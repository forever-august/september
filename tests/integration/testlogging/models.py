"""Data models for log capture and analysis."""

import re
from dataclasses import dataclass, field
from datetime import datetime, timezone
from urllib.parse import unquote, urlparse


def _percentile(values: list[float], p: int) -> float:
    """
    Calculate the p-th percentile of a list of values using linear interpolation.

    Args:
        values: List of numeric values
        p: Percentile to calculate (0-100)

    Returns:
        The value at the p-th percentile, or 0.0 if the list is empty
    """
    if not values:
        return 0.0
    sorted_vals = sorted(values)
    k = (len(sorted_vals) - 1) * p / 100
    f = int(k)
    c = f + 1 if f + 1 < len(sorted_vals) else f
    return sorted_vals[f] + (k - f) * (sorted_vals[c] - sorted_vals[f])


def _extract_route_pattern(path: str) -> str:
    """
    Extract a route pattern from a URL path, replacing dynamic segments.

    Patterns:
    - /a/<message-id> -> /a/{message-id}
    - /g/<group>/t/<message-id> -> /g/{group}/t/{message-id}
    - /g/<group>/compose -> /g/{group}/compose
    - /g/<group>/post -> /g/{group}/post
    - /g/<group> -> /g/{group}
    - /browse/<prefix> -> /browse/{prefix}
    - /static/css/<file> -> /static/css/{file}
    - /static/js/<file> -> /static/js/{file}
    """
    # URL-decode the path first
    path = unquote(path)

    # Article view: /a/<message-id>
    if re.match(r"^/a/.+$", path):
        return "/a/{message-id}"

    # Thread view with message-id: /g/<group>/thread/<message-id>
    if re.match(r"^/g/[^/]+/thread/.+$", path):
        return "/g/{group}/thread/{message-id}"

    # Compose page: /g/<group>/compose
    if re.match(r"^/g/[^/]+/compose$", path):
        return "/g/{group}/compose"

    # Post endpoint: /g/<group>/post
    if re.match(r"^/g/[^/]+/post$", path):
        return "/g/{group}/post"

    # Reply endpoint: /a/<message-id>/reply
    if re.match(r"^/a/.+/reply$", path):
        return "/a/{message-id}/reply"

    # Group view: /g/<group>
    if re.match(r"^/g/[^/]+$", path):
        return "/g/{group}"

    # Browse prefix: /browse/<prefix>
    if re.match(r"^/browse/.+$", path):
        return "/browse/{prefix}"

    # Static files
    if re.match(r"^/static/css/.+$", path):
        return "/static/css/{file}"
    if re.match(r"^/static/js/.+$", path):
        return "/static/js/{file}"

    # Return path as-is for other routes (/, /auth/login, etc.)
    return path


@dataclass
class RouteTiming:
    """Timing information for a single route request."""

    url: str
    method: str  # GET, POST, etc.
    duration_ms: float  # Total request duration in milliseconds
    ttfb_ms: float  # Time to first byte in milliseconds
    test_name: str  # Which test triggered this request

    @property
    def route(self) -> str:
        """Extract the URL-decoded route from the URL (without query params)."""
        parsed = urlparse(self.url)
        return unquote(parsed.path)

    @property
    def route_pattern(self) -> str:
        """Extract route pattern with placeholders for dynamic segments."""
        parsed = urlparse(self.url)
        return _extract_route_pattern(parsed.path)

    @property
    def duration_seconds(self) -> float:
        """Duration in seconds."""
        return self.duration_ms / 1000.0


@dataclass
class RouteStats:
    """Aggregated statistics for a single route pattern."""

    pattern: str  # Route pattern like /g/{group}
    method: str
    timings: list[RouteTiming] = field(default_factory=list)

    @property
    def count(self) -> int:
        """Number of requests to this route."""
        return len(self.timings)

    @property
    def total_ms(self) -> float:
        """Total time spent on this route."""
        return sum(t.duration_ms for t in self.timings)

    @property
    def avg_ms(self) -> float:
        """Average request duration."""
        if not self.timings:
            return 0.0
        return self.total_ms / len(self.timings)

    @property
    def min_ms(self) -> float:
        """Minimum request duration."""
        if not self.timings:
            return 0.0
        return min(t.duration_ms for t in self.timings)

    @property
    def max_ms(self) -> float:
        """Maximum request duration."""
        if not self.timings:
            return 0.0
        return max(t.duration_ms for t in self.timings)

    @property
    def avg_ttfb_ms(self) -> float:
        """Average time to first byte."""
        if not self.timings:
            return 0.0
        return sum(t.ttfb_ms for t in self.timings) / len(self.timings)

    @property
    def p50_ms(self) -> float:
        """50th percentile (median) request duration."""
        return _percentile([t.duration_ms for t in self.timings], 50)

    @property
    def p90_ms(self) -> float:
        """90th percentile request duration."""
        return _percentile([t.duration_ms for t in self.timings], 90)

    @property
    def p99_ms(self) -> float:
        """99th percentile request duration."""
        return _percentile([t.duration_ms for t in self.timings], 99)


@dataclass
class PerformanceReport:
    """Aggregated performance metrics for route timings."""

    session_start: datetime
    session_end: datetime | None = None
    route_timings: list[RouteTiming] = field(default_factory=list)

    @property
    def total_duration_seconds(self) -> float:
        """Total session duration in seconds."""
        if self.session_end is None:
            return 0.0
        return (self.session_end - self.session_start).total_seconds()

    @property
    def total_requests(self) -> int:
        """Total number of route requests."""
        return len(self.route_timings)

    @property
    def total_route_time_ms(self) -> float:
        """Total time spent in route requests."""
        return sum(t.duration_ms for t in self.route_timings)

    @property
    def p50_ms(self) -> float:
        """50th percentile (median) request duration across all requests."""
        return _percentile([t.duration_ms for t in self.route_timings], 50)

    @property
    def p90_ms(self) -> float:
        """90th percentile request duration across all requests."""
        return _percentile([t.duration_ms for t in self.route_timings], 90)

    @property
    def p99_ms(self) -> float:
        """99th percentile request duration across all requests."""
        return _percentile([t.duration_ms for t in self.route_timings], 99)

    def get_route_stats(self) -> list[RouteStats]:
        """Get aggregated stats for each unique route pattern."""
        stats_map: dict[tuple[str, str], RouteStats] = {}

        for timing in self.route_timings:
            key = (timing.route_pattern, timing.method)
            if key not in stats_map:
                stats_map[key] = RouteStats(
                    pattern=timing.route_pattern, method=timing.method
                )
            stats_map[key].timings.append(timing)

        # Sort by total time descending
        return sorted(stats_map.values(), key=lambda s: s.total_ms, reverse=True)

    def get_slowest_requests(self, limit: int = 10) -> list[RouteTiming]:
        """Get the N slowest individual requests."""
        return sorted(self.route_timings, key=lambda t: t.duration_ms, reverse=True)[
            :limit
        ]


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
