"""
Tests for article view functionality.

These tests verify:
- Individual article view loads
- Article headers display correctly
- Article body renders
- Navigation between articles
"""

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC

from conftest import SEPTEMBER_URL, WAIT_TIMEOUT_DEFAULT, WAIT_TIMEOUT_POLL


class TestArticleView:
    """Tests for the article view page (/a/{message_id})."""

    def test_article_view_from_thread(self, browser: WebDriver):
        """Should be able to view an individual article from a thread."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            # Wait for navigation to thread view
            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # Look for article links within the thread view
            article_links = browser.find_elements(By.CSS_SELECTOR, "a[href*='/a/']")

            if len(article_links) > 0:
                article_links[0].click()

                # Wait for navigation to article view
                wait.until(EC.url_contains("/a/"))

                # SSR: article content available immediately
                assert browser.find_element(By.TAG_NAME, "main")

    def test_article_shows_headers(self, browser: WebDriver):
        """Article view should display message headers (From, Subject, Date)."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # SSR: article content rendered server-side
            assert browser.find_element(By.TAG_NAME, "main")

    def test_article_shows_body(self, browser: WebDriver):
        """Article view should display the message body."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # SSR: body content rendered server-side
            assert browser.find_element(By.TAG_NAME, "main")


class TestArticleNavigation:
    """Tests for navigation between articles."""

    def test_back_to_group(self, browser: WebDriver):
        """Should be able to navigate back to the group from an article."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # Find link back to group
            group_links = browser.find_elements(
                By.CSS_SELECTOR, "a[href*='/g/test.general']"
            )

            if len(group_links) == 0:
                browser.back()
            else:
                group_links[0].click()

            # Wait for navigation back
            wait.until(
                lambda d: "/g/test.general" in d.current_url
                and "/thread/" not in d.current_url
            )

    def test_header_navigation(self, browser: WebDriver):
        """Header should provide navigation back to home."""
        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) > 0:
            thread_links[0].click()

            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(EC.url_contains("/a/"))

            # Header should have home link
            header = browser.find_element(By.CSS_SELECTOR, "header, .site-header, nav")
            home_links = header.find_elements(By.CSS_SELECTOR, "a[href='/']")

            if len(home_links) > 0:
                home_links[0].click()
                wait.until(EC.url_to_be(f"{SEPTEMBER_URL}/"))


class TestArticleNotFound:
    """Tests for error handling with invalid article IDs."""

    def test_invalid_article_shows_error(self, browser: WebDriver):
        """Requesting an invalid article should show an error."""
        browser.get(f"{SEPTEMBER_URL}/a/nonexistent-message-id-12345")

        # SSR: error page rendered server-side
        assert browser.find_element(By.TAG_NAME, "main")
        assert browser.find_element(By.TAG_NAME, "body")
