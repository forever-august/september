"""Log capture and analysis utilities for integration tests."""

from .analysis import analyze_failure, format_failure_report, format_performance_report
from .capture import fetch_service_logs, parse_json_log, parse_text_log
from .models import (
    LogEntry,
    PerformanceReport,
    RouteStats,
    RouteTiming,
    TestLogCapture,
)
from .performance import (
    clear_performance_entries,
    get_navigation_timing,
    get_resource_timings,
)

__all__ = [
    # Models
    "LogEntry",
    "TestLogCapture",
    "RouteTiming",
    "RouteStats",
    "PerformanceReport",
    # Capture
    "fetch_service_logs",
    "parse_json_log",
    "parse_text_log",
    # Performance
    "get_navigation_timing",
    "get_resource_timings",
    "clear_performance_entries",
    # Analysis
    "analyze_failure",
    "format_failure_report",
    "format_performance_report",
]
