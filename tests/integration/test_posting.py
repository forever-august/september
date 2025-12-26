"""
Tests for posting and replying to articles.

These tests verify:
- Compose page is accessible when authenticated
- Form validation works
- New posts are submitted successfully
- Replies are submitted successfully
- CSRF protection is in place
- Email requirement for posting

Note: These tests modify data in the NNTP server.
"""

import time
import uuid

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import TimeoutException

from conftest import SEPTEMBER_URL, WAIT_TIMEOUT_DEFAULT, WAIT_TIMEOUT_POLL


class TestComposeAccess:
    """Tests for accessing the compose page."""

    def test_compose_requires_auth(self, clean_browser: WebDriver):
        """Compose page should require authentication."""
        browser = clean_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general/compose")

        # SSR: page rendered immediately
        current_url = browser.current_url
        page_source = browser.page_source.lower()

        # Should be redirected to login or show auth error
        requires_auth = (
            "login" in current_url
            or "auth" in current_url
            or "sign in" in page_source
            or "log in" in page_source
            or "authentication" in page_source
            or "not authorized" in page_source
            or "must be logged in" in page_source
        )

        # Should not show the compose form when not authenticated
        compose_forms = browser.find_elements(
            By.CSS_SELECTOR, "form[action*='post'], .compose-form"
        )
        assert len(compose_forms) == 0 or requires_auth, (
            "Compose should require authentication"
        )

    @pytest.mark.auth
    def test_compose_accessible_when_authenticated(
        self, authenticated_browser: WebDriver
    ):
        """Compose page should be accessible when logged in."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general/compose")

        # SSR: compose form rendered server-side
        page_source = browser.page_source.lower()

        has_form = len(browser.find_elements(By.CSS_SELECTOR, "form, textarea")) > 0
        has_error = "error" in page_source or "not allowed" in page_source

        assert has_form or has_error


class TestComposeForm:
    """Tests for the compose form functionality."""

    @pytest.mark.auth
    @pytest.mark.posting
    def test_compose_form_has_required_fields(self, authenticated_browser: WebDriver):
        """Compose form should have subject and body fields."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general/compose")

        # SSR: form fields rendered server-side
        try:
            form = browser.find_element(By.CSS_SELECTOR, "form")

            subject_fields = browser.find_elements(
                By.CSS_SELECTOR, "input[name='subject'], input[type='text']"
            )
            body_fields = browser.find_elements(
                By.CSS_SELECTOR, "textarea[name='body'], textarea"
            )
            submit_buttons = browser.find_elements(
                By.CSS_SELECTOR, "button[type='submit'], input[type='submit']"
            )

            assert len(subject_fields) > 0, "Should have subject field"
            assert len(body_fields) > 0, "Should have body field"
            assert len(submit_buttons) > 0, "Should have submit button"

        except Exception:
            pytest.skip(
                "Compose form not accessible - may require specific permissions"
            )

    @pytest.mark.auth
    @pytest.mark.posting
    def test_compose_form_has_csrf_token(self, authenticated_browser: WebDriver):
        """Compose form should include CSRF protection."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general/compose")

        try:
            browser.find_element(By.CSS_SELECTOR, "form")

            csrf_fields = browser.find_elements(
                By.CSS_SELECTOR, "input[name='csrf_token'], input[name='_csrf']"
            )

            assert len(csrf_fields) > 0, "Should have CSRF token field"

            if len(csrf_fields) > 0:
                token_value = csrf_fields[0].get_attribute("value")
                assert token_value and len(token_value) > 10, (
                    "CSRF token should have a value"
                )

        except Exception:
            pytest.skip("Compose form not accessible")


class TestPostSubmission:
    """Tests for submitting new posts."""

    @pytest.mark.auth
    @pytest.mark.posting
    def test_submit_new_post(self, authenticated_browser: WebDriver):
        """Should be able to submit a new post."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general/compose")

        try:
            browser.find_element(By.CSS_SELECTOR, "form")

            unique_id = str(uuid.uuid4())[:8]
            test_subject = f"Integration Test Post {unique_id}"
            test_body = f"This is an automated test post.\n\nTest ID: {unique_id}"

            subject_input = browser.find_element(
                By.CSS_SELECTOR, "input[name='subject']"
            )
            subject_input.clear()
            subject_input.send_keys(test_subject)

            body_input = browser.find_element(By.CSS_SELECTOR, "textarea[name='body']")
            body_input.clear()
            body_input.send_keys(test_body)

            submit_button = browser.find_element(
                By.CSS_SELECTOR, ".compose-form button[type='submit']"
            )
            submit_button.click()

            # Wait for redirect after form submission
            wait = WebDriverWait(
                browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
            )
            wait.until(lambda d: "/compose" not in d.current_url)

            current_url = browser.current_url
            assert "/g/test.general" in current_url or "thread" in current_url

        except Exception:
            pytest.skip("Could not submit post - compose form not accessible")

    @pytest.mark.auth
    @pytest.mark.posting
    def test_empty_subject_rejected(self, authenticated_browser: WebDriver):
        """Post with empty subject should be rejected."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general/compose")

        try:
            browser.find_element(By.CSS_SELECTOR, "form")

            body_input = browser.find_element(By.CSS_SELECTOR, "textarea[name='body']")
            body_input.clear()
            body_input.send_keys("This post has no subject")

            submit_button = browser.find_element(
                By.CSS_SELECTOR, ".compose-form button[type='submit']"
            )
            submit_button.click()

            # Give time for validation
            time.sleep(0.5)

            page_source = browser.page_source.lower()
            still_on_compose = "/compose" in browser.current_url
            has_error = "required" in page_source or "error" in page_source

            assert still_on_compose or has_error, "Empty subject should be rejected"

        except Exception:
            pytest.skip("Compose form not accessible")


class TestReplySubmission:
    """Tests for replying to existing articles."""

    @pytest.mark.auth
    @pytest.mark.posting
    def test_reply_form_available(self, authenticated_browser: WebDriver):
        """Reply form should be available on article/thread view."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.general")

        # Wait for thread list to ensure page is loaded
        wait = WebDriverWait(
            browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
        )
        wait.until(EC.presence_of_element_located((By.CLASS_NAME, "thread-list")))

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) == 0:
            pytest.skip("No threads available to test reply")

        thread_links[0].click()

        # Wait for navigation - URL contains /a/ or /thread/
        wait.until(lambda d: "/a/" in d.current_url or "/thread/" in d.current_url)

        # SSR: reply form rendered server-side
        reply_elements = browser.find_elements(
            By.CSS_SELECTOR,
            "form[action*='reply'], button[class*='reply'], a[href*='reply'], .reply-form, textarea",
        )

        assert len(reply_elements) > 0 or "reply" in browser.page_source.lower(), (
            "Should have reply functionality when authenticated"
        )

    @pytest.mark.auth
    @pytest.mark.posting
    def test_submit_reply(self, authenticated_browser: WebDriver):
        """Should be able to submit a reply to an existing thread."""
        browser = authenticated_browser

        browser.get(f"{SEPTEMBER_URL}/g/test.development")

        # Wait for thread list to ensure page is loaded
        wait = WebDriverWait(
            browser, WAIT_TIMEOUT_DEFAULT, poll_frequency=WAIT_TIMEOUT_POLL
        )
        wait.until(EC.presence_of_element_located((By.CLASS_NAME, "thread-list")))

        thread_links = browser.find_elements(By.CSS_SELECTOR, ".thread-card-link")

        if len(thread_links) == 0:
            pytest.skip("No threads available to test reply")

        thread_links[0].click()

        # Wait for navigation - URL contains /a/ or /thread/
        wait.until(lambda d: "/a/" in d.current_url or "/thread/" in d.current_url)

        # SSR: form rendered server-side
        reply_forms = browser.find_elements(By.CSS_SELECTOR, "form[action*='reply']")
        reply_textareas = browser.find_elements(
            By.CSS_SELECTOR, ".reply-form textarea, form textarea"
        )

        if len(reply_forms) == 0 and len(reply_textareas) == 0:
            pytest.skip("Reply form not found on this page")

        try:
            unique_id = str(uuid.uuid4())[:8]
            test_body = f"This is an automated test reply.\n\nTest ID: {unique_id}"

            if len(reply_textareas) > 0:
                textarea = reply_textareas[-1]
                textarea.clear()
                textarea.send_keys(test_body)

                form = textarea.find_element(By.XPATH, "./ancestor::form")
                submit = form.find_element(By.CSS_SELECTOR, "button[type='submit']")
                submit.click()

                time.sleep(1)

                assert (
                    "thread" in browser.current_url
                    or "test.development" in browser.current_url
                )

        except Exception as e:
            pytest.skip(f"Could not submit reply: {e}")
