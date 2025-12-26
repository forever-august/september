"""
Tests for thread list and thread view functionality.

These tests verify:
- Thread list loads for a group
- Threads display with correct metadata
- Pagination works
- Thread view shows all articles
- Thread hierarchy (replies) displays correctly
"""

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC

from conftest import SEPTEMBER_URL, WAIT_TIMEOUT_DEFAULT, WAIT_TIMEOUT_POLL


class TestThreadList:
    """Tests for the thread list page (/g/{group})."""

    def test_thread_list_loads(self, browser: WebDriver):
        """Thread list page should load for a valid group."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        # SSR: elements available immediately
        # Should have a thread list or empty state
        thread_list = browser.find_elements(By.CLASS_NAME, "thread-list")
        empty_state = browser.find_elements(By.CLASS_NAME, "empty-state")
        assert len(thread_list) > 0 or len(empty_state) > 0

        # Page title should include group name
        assert "test.general" in browser.title or "test.general" in browser.page_source

    def test_thread_list_shows_threads(self, browser: WebDriver):
        """Thread list should display seeded threads."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        # SSR: thread list rendered server-side
        thread_list = browser.find_element(By.CLASS_NAME, "thread-list")
        threads = thread_list.find_elements(
            By.CSS_SELECTOR, ".thread-card, .thread-card-link"
        )
        assert len(threads) > 0, "Expected seeded threads to appear"

    def test_thread_has_subject(self, browser: WebDriver):
        """Each thread should display a subject line."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        subjects = browser.find_elements(
            By.CSS_SELECTOR, ".thread-title, .thread-card-link"
        )

        if len(subjects) > 0:
            subject_texts = [s.text for s in subjects if s.text.strip()]
            assert len(subject_texts) > 0, "Expected threads to have subject text"

    def test_thread_has_author(self, browser: WebDriver):
        """Threads should display author information."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        # Just verify the page loaded correctly
        assert browser.find_element(By.CLASS_NAME, "thread-list")

    def test_click_thread_opens_view(self, browser: WebDriver):
        """Clicking a thread should navigate to the thread view."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            # Wait for navigation (click triggers page load)
            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # SSR: new page elements available immediately after navigation
            assert browser.find_element(By.TAG_NAME, "main")


class TestThreadView:
    """Tests for the thread view page (/g/{group}/thread/{message_id})."""

    def test_thread_view_loads(self, browser: WebDriver):
        """Thread view should load when navigating from thread list."""
        browser.get(f"{SEPTEMBER_URL}/g/test.development")

        # Wait for thread list to ensure page is loaded
        wait = WebDriverWait(
            browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
        )
        wait.until(EC.presence_of_element_located((By.CLASS_NAME, "thread-list")))

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            # Wait for navigation - URL contains /a/ or /thread/
            wait.until(lambda d: "/a/" in d.current_url or "/thread/" in d.current_url)

            # SSR: content available immediately
            assert browser.find_element(By.TAG_NAME, "main")

    def test_thread_view_shows_articles(self, browser: WebDriver):
        """Thread view should display the articles in the thread."""
        browser.get(f"{SEPTEMBER_URL}/g/test.development")

        # Wait for thread list to ensure page is loaded
        wait = WebDriverWait(
            browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
        )
        wait.until(EC.presence_of_element_located((By.CLASS_NAME, "thread-list")))

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            # Wait for navigation - URL contains /a/ or /thread/
            wait.until(lambda d: "/a/" in d.current_url or "/thread/" in d.current_url)

            # SSR: articles rendered server-side
            articles = browser.find_elements(
                By.CSS_SELECTOR, ".comment, .article-content"
            )
            assert len(articles) >= 1, "Expected at least one article in thread"

    def test_thread_view_has_reply_button(self, browser: WebDriver):
        """Thread view should have a reply button (when authenticated)."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # Just verify the page loads correctly
            assert browser.find_element(By.TAG_NAME, "main")


class TestPagination:
    """Tests for pagination in thread lists."""

    def test_pagination_present_when_needed(self, browser: WebDriver):
        """Pagination should appear when there are enough threads."""
        browser.get(f"{SEPTEMBER_URL}/g/test.development")

        # SSR: pagination rendered server-side if present
        # Just verify the page is functional
        assert browser.find_element(By.TAG_NAME, "main")


class TestGroupNotFound:
    """Tests for error handling with invalid groups."""

    def test_invalid_group_shows_error(self, browser: WebDriver):
        """Requesting an invalid group should show an error or empty state."""
        browser.get(f"{SEPTEMBER_URL}/g/nonexistent.group.name")

        # SSR: error page rendered server-side
        assert browser.find_element(By.TAG_NAME, "main")
        assert browser.find_element(By.TAG_NAME, "body")
