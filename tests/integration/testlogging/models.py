"""Data models for log capture and analysis."""

from dataclasses import dataclass, field
from datetime import datetime, timezone


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
