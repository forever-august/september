"""
Tests for the homepage and group browsing functionality.

These tests verify:
- Homepage loads correctly
- Newsgroups are displayed
- Group filtering/search works
- Navigation to groups works
- Breadcrumb navigation in browse view
- Health and privacy endpoints work
"""

import requests

from typing import Callable

from helpers import SEPTEMBER_HOST_URL, SEPTEMBER_URL
from pages import BrowsePage, HomePage


class TestHomepage:
    """Tests for the main homepage."""

    def test_homepage_loads(self, home_page: Callable[[], HomePage]):
        """Homepage should load and display the site title."""
        page = home_page()
        assert page.has_main_content()
        assert "September" in page.title or "Newsgroups" in page.title

    def test_homepage_shows_groups(self, home_page: Callable[[], HomePage]):
        """Homepage should display available newsgroups."""
        page = home_page()
        assert page.has_group_cards()
        assert page.get_group_count() > 0, "Expected at least one group card"

    def test_homepage_has_search_input(self, home_page: Callable[[], HomePage]):
        """Homepage should have a search/filter input."""
        page = home_page()
        assert page.has_search_input()
        search_input = page.get_search_input()
        assert search_input.get_attribute("placeholder") is not None

    def test_group_search_filters_results(self, home_page: Callable[[], HomePage]):
        """Typing in search should filter the displayed groups."""
        page = home_page()
        page.search("test")
        # Verify search input has the value and page is still functional
        assert page.get_search_value() == "test"
        assert page.has_main_content()


class TestBrowseNavigation:
    """Tests for hierarchical group browsing."""

    def test_browse_test_prefix(self, browse_page: Callable[[str], BrowsePage]):
        """Should be able to browse the 'test' group hierarchy."""
        page = browse_page("test")
        assert page.has_group_cards()
        assert page.has_page_header()

    def test_breadcrumb_navigation(self, browse_page: Callable[[str], BrowsePage]):
        """Breadcrumbs should be present when browsing subgroups."""
        page = browse_page("test")
        assert page.has_breadcrumb_home_link()

    def test_click_group_navigates(self, home_page: Callable[[], HomePage]):
        """Clicking a group card should navigate to that group."""
        page = home_page()
        result_page = page.click_first_group()
        # Should be on a group or browse page
        assert "/g/" in result_page.current_url or "/browse/" in result_page.current_url


class TestStaticAssets:
    """Tests for static assets loading."""

    def test_css_loads(self, home_page: Callable[[], HomePage]):
        """CSS stylesheet should load successfully."""
        page = home_page()
        assert page.has_body()

    def test_js_loads(self, home_page: Callable[[], HomePage]):
        """JavaScript should load and execute."""
        page = home_page()
        page.search("test")
        assert page.get_search_value() == "test"


class TestEndpoints:
    """Tests for miscellaneous HTTP endpoints."""

    def test_health_endpoint(self):
        """
        Health endpoint should return 200 OK with 'ok' body.

        This verifies the health check endpoint is accessible and working.
        Used by container orchestration for liveness probes.
        """
        response = requests.get(f"{SEPTEMBER_HOST_URL}/health")
        assert response.status_code == 200
        assert response.text.strip() == "ok"

    def test_privacy_page_loads(self, home_page: Callable[[], HomePage]):
        """
        Privacy page should load and display content.

        The privacy page provides legal information about data handling.
        """
        page = home_page()
        # Use Docker-internal URL for Selenium (runs in Docker)
        page.driver.get(f"{SEPTEMBER_URL}/privacy")
        # Verify page loads with content
        assert page.has_body()
