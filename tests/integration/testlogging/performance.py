"""Performance timing capture via browser Performance API."""

from selenium.webdriver.remote.webdriver import WebDriver

from .models import RouteTiming

# JavaScript to extract navigation timing for the current page
JS_GET_NAVIGATION_TIMING = """
const entries = performance.getEntriesByType('navigation');
if (entries.length === 0) return null;
const nav = entries[0];

// For measuring route performance, we want the time from when the browser
// started the request to when it received the response.
// 
// For redirects (like OIDC flows):
// - responseStart/responseEnd for the final URL may be accurate
// - But requestStart may be 0 for cross-origin redirects
//
// Best approach: use responseStart - fetchStart for TTFB (if responseStart > 0)
// and responseEnd - responseStart for response body time.
// Total page time = responseEnd - fetchStart (when fetchStart > 0)

let ttfb = 0;
let duration = nav.duration;

if (nav.responseStart > 0 && nav.fetchStart > 0) {
    // Normal case: we have valid timing data
    ttfb = nav.responseStart - nav.fetchStart;
    duration = nav.responseEnd - nav.fetchStart;
} else if (nav.responseStart > 0 && nav.requestStart > 0) {
    // Fallback: use requestStart
    ttfb = nav.responseStart - nav.requestStart;
    duration = nav.responseEnd - nav.requestStart;
}
// else: use nav.duration (full navigation time including redirects)

return {
    url: nav.name,
    duration: duration,
    ttfb: ttfb,
    type: nav.initiatorType,
    redirectCount: nav.redirectCount || 0
};
"""

# JavaScript to extract all resource timings since last clear
JS_GET_RESOURCE_TIMINGS = """
const entries = performance.getEntriesByType('resource');
return entries.map(r => ({
    url: r.name,
    duration: r.duration,
    ttfb: r.responseStart - r.requestStart,
    type: r.initiatorType
}));
"""

# JavaScript to clear performance entries
JS_CLEAR_TIMINGS = """
performance.clearResourceTimings();
"""


def get_navigation_timing(driver: WebDriver, test_name: str) -> RouteTiming | None:
    """
    Get navigation timing for the current page load.

    Returns timing info for the main document navigation, or None if not available.

    Duration is measured from fetchStart to responseEnd, which excludes time spent
    in redirect chains (like OIDC flows). This gives a more accurate measure of
    actual page load performance.
    """
    try:
        result = driver.execute_script(JS_GET_NAVIGATION_TIMING)
        if result is None:
            return None

        return RouteTiming(
            url=result["url"],
            method="GET",  # Navigation is always GET
            duration_ms=result["duration"],
            ttfb_ms=max(0, result["ttfb"]),
            test_name=test_name,
        )
    except Exception:
        return None


def get_resource_timings(driver: WebDriver, test_name: str) -> list[RouteTiming]:
    """
    Get resource timings for all fetched resources.

    This includes XHR, fetch, images, scripts, stylesheets, etc.
    Only returns timings for September routes (filters out external resources).
    """
    try:
        results = driver.execute_script(JS_GET_RESOURCE_TIMINGS)
        if not results:
            return []

        timings = []
        for r in results:
            # Only include requests to our app (filter out external CDNs etc)
            url = r.get("url", "")
            if "september" not in url and "localhost" not in url:
                continue

            # Determine method based on initiator type
            # fetch/xmlhttprequest could be POST, but we can't know for sure
            method = "GET"
            if r.get("type") in ("fetch", "xmlhttprequest"):
                method = "XHR"  # Mark as XHR since we can't determine exact method

            timings.append(
                RouteTiming(
                    url=url,
                    method=method,
                    duration_ms=r["duration"],
                    ttfb_ms=max(0, r["ttfb"]),
                    test_name=test_name,
                )
            )

        return timings
    except Exception:
        return []


def clear_performance_entries(driver: WebDriver) -> None:
    """Clear resource timing entries to avoid duplicates."""
    try:
        driver.execute_script(JS_CLEAR_TIMINGS)
    except Exception:
        pass
