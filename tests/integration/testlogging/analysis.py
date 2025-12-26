"""Failure analysis and reporting utilities."""

from .models import LogEntry, TestLogCapture


def analyze_failure(capture: TestLogCapture, exception: BaseException | None) -> dict:
    """Analyze a test failure to determine if it's test-related or service-related."""
    analysis = {
        "test_name": capture.test_name,
        "duration_seconds": (
            (capture.end_time - capture.start_time).total_seconds()
            if capture.end_time
            else 0
        ),
        "error_type": "unknown",
        "likely_cause": "unknown",
        "service_errors": [],
        "recommendations": [],
    }

    error_logs = capture.get_error_logs()

    # Categorize by service
    service_errors: dict[str, list[LogEntry]] = {}
    for log in error_logs:
        if log.service not in service_errors:
            service_errors[log.service] = []
        service_errors[log.service].append(log)

    analysis["service_errors"] = [
        {
            "service": svc,
            "count": len(errs),
            "messages": [e.message[:200] for e in errs[:3]],
        }
        for svc, errs in service_errors.items()
    ]

    # Determine likely cause
    exception_str = str(exception) if exception else ""

    # Check for timeout errors (likely test/selector issue)
    if "TimeoutException" in exception_str or "timeout" in exception_str.lower():
        if any(
            log.service == "september" and "error" in log.level.lower()
            for log in error_logs
        ):
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "September returned an error during the request"
            analysis["recommendations"].append("Check September error logs for details")
        elif any(
            log.service == "nntp" and "error" in log.level.lower() for log in error_logs
        ):
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "NNTP server (renews) encountered an error"
            analysis["recommendations"].append("Check NNTP error logs for details")
        else:
            analysis["error_type"] = "test_issue"
            analysis["likely_cause"] = (
                "Element not found - likely incorrect CSS selector or page structure changed"
            )
            analysis["recommendations"].append(
                "Verify CSS selectors match actual page HTML"
            )
            analysis["recommendations"].append(
                "Check if page loaded correctly (use VNC at localhost:7900)"
            )

    # Check for assertion errors
    elif "AssertionError" in exception_str:
        if error_logs:
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "Service error caused unexpected page state"
        else:
            analysis["error_type"] = "test_issue"
            analysis["likely_cause"] = (
                "Test assertion failed - expected condition not met"
            )
            analysis["recommendations"].append("Review test logic and expected values")

    # Check for connection errors
    elif "ConnectionError" in exception_str or "connection" in exception_str.lower():
        analysis["error_type"] = "infrastructure"
        analysis["likely_cause"] = "Service connection failed"
        analysis["recommendations"].append("Check if all Docker services are running")
        analysis["recommendations"].append("Run: docker compose ps")

    # Service errors present
    elif error_logs:
        # Prioritize by service
        if "september" in service_errors:
            analysis["error_type"] = "september_error"
            analysis["likely_cause"] = "September application error"
        elif "nntp" in service_errors:
            analysis["error_type"] = "nntp_error"
            analysis["likely_cause"] = "NNTP server (renews) error"
        elif "dex" in service_errors:
            analysis["error_type"] = "dex_error"
            analysis["likely_cause"] = "Dex OIDC provider error"
        else:
            analysis["error_type"] = "service_error"
            analysis["likely_cause"] = "Service error detected"

    return analysis


def format_failure_report(
    capture: TestLogCapture, exception: BaseException | None
) -> str:
    """Format a detailed failure report."""
    analysis = analyze_failure(capture, exception)

    lines = [
        "",
        "=" * 80,
        f"TEST FAILURE ANALYSIS: {capture.test_name}",
        "=" * 80,
        "",
        f"Error Type: {analysis['error_type']}",
        f"Likely Cause: {analysis['likely_cause']}",
        f"Test Duration: {analysis['duration_seconds']:.2f}s",
        "",
    ]

    if analysis["recommendations"]:
        lines.append("Recommendations:")
        for rec in analysis["recommendations"]:
            lines.append(f"  - {rec}")
        lines.append("")

    if analysis["service_errors"]:
        lines.append("Service Errors Detected:")
        for svc_err in analysis["service_errors"]:
            lines.append(f"  [{svc_err['service']}] {svc_err['count']} error(s)")
            for msg in svc_err["messages"]:
                lines.append(f"    - {msg}")
        lines.append("")

    # Include relevant logs
    error_logs = capture.get_error_logs()
    if error_logs:
        lines.append("Error/Warning Logs During Test:")
        lines.append("-" * 40)
        for log in error_logs[:10]:  # Limit to 10 most relevant
            ts_str = (
                log.timestamp.strftime("%H:%M:%S.%f")[:-3]
                if log.timestamp
                else "??:??:??"
            )
            lines.append(f"[{log.service}] {ts_str} {log.level}: {log.message[:200]}")
        if len(error_logs) > 10:
            lines.append(f"  ... and {len(error_logs) - 10} more error logs")
        lines.append("")

    lines.append("=" * 80)

    return "\n".join(lines)
