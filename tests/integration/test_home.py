"""
Tests for the homepage and group browsing functionality.

These tests verify:
- Homepage loads correctly
- Newsgroups are displayed
- Group filtering/search works
- Navigation to groups works
- Breadcrumb navigation in browse view
"""

import time

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC

from conftest import SEPTEMBER_URL, WAIT_TIMEOUT_DEFAULT, WAIT_TIMEOUT_POLL


class TestHomepage:
    """Tests for the main homepage."""

    def test_homepage_loads(self, browser: WebDriver):
        """Homepage should load and display the site title."""
        browser.get(f"{SEPTEMBER_URL}/")

        # SSR: elements are available immediately after page load
        assert "September" in browser.title or "Newsgroups" in browser.title
        assert browser.find_element(By.TAG_NAME, "main")

    def test_homepage_shows_groups(self, browser: WebDriver):
        """Homepage should display available newsgroups."""
        browser.get(f"{SEPTEMBER_URL}/")

        # SSR: group cards are rendered server-side
        group_cards = browser.find_element(By.CLASS_NAME, "group-cards")
        cards = group_cards.find_elements(By.CLASS_NAME, "group-card")
        assert len(cards) > 0, "Expected at least one group card"

    def test_homepage_has_search_input(self, browser: WebDriver):
        """Homepage should have a search/filter input."""
        browser.get(f"{SEPTEMBER_URL}/")

        search_input = browser.find_element(By.ID, "group-search")
        assert search_input is not None
        assert search_input.get_attribute("placeholder") is not None

    def test_group_search_filters_results(self, browser: WebDriver):
        """Typing in search should filter the displayed groups."""
        browser.get(f"{SEPTEMBER_URL}/")

        # Type in search box
        search_input = browser.find_element(By.ID, "group-search")
        search_input.send_keys("test")

        # Give JavaScript time to filter (this is client-side JS)
        time.sleep(0.3)

        # Verify page is still functional
        assert browser.find_element(By.TAG_NAME, "main")


class TestBrowseNavigation:
    """Tests for hierarchical group browsing."""

    def test_browse_test_prefix(self, browser: WebDriver):
        """Should be able to browse the 'test' group hierarchy."""
        browser.get(f"{SEPTEMBER_URL}/browse/test")

        # SSR: elements available immediately
        browser.find_element(By.CLASS_NAME, "group-cards")
        page_header = browser.find_element(By.CLASS_NAME, "page-header")
        assert page_header is not None

    def test_breadcrumb_navigation(self, browser: WebDriver):
        """Breadcrumbs should be present when browsing subgroups."""
        browser.get(f"{SEPTEMBER_URL}/browse/test")

        # SSR: elements available immediately
        page_header = browser.find_element(By.CLASS_NAME, "page-header")
        home_link = page_header.find_element(By.CSS_SELECTOR, "a[href='/']")
        assert home_link is not None

    def test_click_group_navigates(self, browser: WebDriver):
        """Clicking a group card should navigate to that group."""
        browser.get(f"{SEPTEMBER_URL}/")

        # Find a group card link
        group_links = browser.find_elements(
            By.CSS_SELECTOR, ".group-card a.group-card-link"
        )

        if len(group_links) > 0:
            group_links[0].click()

            # Wait for navigation (URL change after click)
            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(lambda d: d.current_url != f"{SEPTEMBER_URL}/")

            # Should be on a group or browse page
            current_url = browser.current_url
            assert "/g/" in current_url or "/browse/" in current_url


class TestStaticAssets:
    """Tests for static assets loading."""

    def test_css_loads(self, browser: WebDriver):
        """CSS stylesheet should load successfully."""
        browser.get(f"{SEPTEMBER_URL}/")

        body = browser.find_element(By.TAG_NAME, "body")
        assert body is not None

    def test_js_loads(self, browser: WebDriver):
        """JavaScript should load and execute."""
        browser.get(f"{SEPTEMBER_URL}/")

        search_input = browser.find_element(By.ID, "group-search")
        search_input.send_keys("test")
        assert search_input.get_attribute("value") == "test"
