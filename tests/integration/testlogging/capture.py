"""Log capture and fetching utilities."""

import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path

from .models import LogEntry

# Path to the environment directory containing Docker setup
ENVIRONMENT_DIR = Path(__file__).parent.parent / "environment"


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
