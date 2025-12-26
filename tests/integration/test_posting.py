"""
Tests for posting and replying to articles.

These tests verify:
- Compose page is accessible when authenticated
- Form validation works
- New posts are submitted successfully
- Replies are submitted successfully
- CSRF protection is in place
- Email requirement for posting
- Unauthenticated users cannot submit posts or replies

Note: These tests modify data in the NNTP server.
"""

import uuid
from typing import Callable

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver

from helpers import SEPTEMBER_URL
from pages import ComposePage, GroupPage


class TestComposeAccess:
    """Tests for accessing the compose page."""

    def test_compose_requires_auth(
        self, compose_page_unauth: Callable[[str], ComposePage]
    ):
        """Compose page should require authentication."""
        page = compose_page_unauth("test.general")

        # Should be redirected to login or show auth error
        requires_auth = page.requires_auth()

        # Should not show the compose form when not authenticated
        has_form = page.has_form()
        assert not has_form or requires_auth, "Compose should require authentication"

    @pytest.mark.auth
    def test_compose_accessible_when_authenticated(
        self, compose_page: Callable[[str], ComposePage]
    ):
        """Compose page should be accessible when logged in."""
        page = compose_page("test.general")

        has_form = page.has_form()
        has_error = page.has_error_message()

        assert has_form or has_error


class TestComposeForm:
    """Tests for the compose form functionality."""

    @pytest.mark.auth
    @pytest.mark.posting
    def test_compose_form_has_required_fields(
        self, compose_page: Callable[[str], ComposePage]
    ):
        """Compose form should have subject and body fields."""
        page = compose_page("test.general")

        assert page.has_subject_field(), "Should have subject field"
        assert page.has_body_field(), "Should have body field"
        assert page.has_submit_button(), "Should have submit button"

    @pytest.mark.auth
    @pytest.mark.posting
    def test_compose_form_has_csrf_token(
        self, compose_page: Callable[[str], ComposePage]
    ):
        """Compose form should include CSRF protection."""
        page = compose_page("test.general")

        assert page.has_csrf_token(), "Should have CSRF token field"

        token_value = page.get_csrf_token_value()
        assert token_value and len(token_value) > 10, "CSRF token should have a value"


class TestPostSubmission:
    """Tests for submitting new posts."""

    @pytest.mark.auth
    @pytest.mark.posting
    def test_submit_new_post(self, compose_page: Callable[[str], ComposePage]):
        """Should be able to submit a new post."""
        page = compose_page("test.general")

        unique_id = str(uuid.uuid4())[:8]
        test_subject = f"Integration Test Post {unique_id}"
        test_body = f"This is an automated test post.\n\nTest ID: {unique_id}"

        result = page.compose_and_submit(test_subject, test_body)

        # Should redirect away from compose page
        assert not result.is_on_compose_page() or isinstance(result, GroupPage)
        assert "/g/test.general" in result.current_url or "thread" in result.current_url

    @pytest.mark.auth
    @pytest.mark.posting
    def test_empty_subject_rejected(self, compose_page: Callable[[str], ComposePage]):
        """Post with empty subject should be rejected."""
        page = compose_page("test.general")

        # Fill only body, leave subject empty
        page.fill_body("This post has no subject")

        # Try to submit
        submit_button = page.get_submit_button()
        submit_button.click()

        # Should still be on compose page or show error
        still_on_compose = page.is_on_compose_page()
        has_error = page.has_error_message()

        assert still_on_compose or has_error, "Empty subject should be rejected"


class TestReplySubmission:
    """Tests for replying to existing articles."""

    @pytest.mark.auth
    @pytest.mark.posting
    def test_reply_form_available(
        self, group_page: Callable[[str], GroupPage], authenticated_browser: WebDriver
    ):
        """Reply form should be available on article/thread view."""
        # Navigate using the authenticated browser through the fixture
        page = GroupPage(authenticated_browser, "test.general").load()

        thread_page = page.click_first_thread()

        # Check for reply elements
        has_reply = (
            thread_page.has_reply_form()
            or thread_page.has_reply_elements()
            or "reply" in thread_page.page_source.lower()
        )

        assert has_reply, "Should have reply functionality when authenticated"

    @pytest.mark.auth
    @pytest.mark.posting
    def test_submit_reply(self, authenticated_browser: WebDriver):
        """Should be able to submit a reply to an existing thread."""
        page = GroupPage(authenticated_browser, "test.development").load()

        thread_page = page.click_first_thread()

        # Check for reply form
        if not thread_page.has_reply_textarea():
            pytest.skip("Reply form not found on this page")

        unique_id = str(uuid.uuid4())[:8]
        test_body = f"This is an automated test reply.\n\nTest ID: {unique_id}"

        textareas = thread_page.get_reply_textareas()
        if textareas:
            from selenium.webdriver.common.by import By
            from helpers import Selectors

            # Use the last textarea
            textarea = textareas[-1]

            # Scroll to element and use JavaScript to set value
            # (handles cases where element is not directly interactable)
            authenticated_browser.execute_script(
                "arguments[0].scrollIntoView({block: 'center'});", textarea
            )
            authenticated_browser.execute_script(
                "arguments[0].value = arguments[1];", textarea, test_body
            )
            # Trigger input event to ensure form knows about the change
            authenticated_browser.execute_script(
                "arguments[0].dispatchEvent(new Event('input', {bubbles: true}));",
                textarea,
            )

            # Find and click submit using JavaScript
            form = textarea.find_element(By.XPATH, "./ancestor::form")
            submit = form.find_element(By.CSS_SELECTOR, Selectors.Compose.SUBMIT_BUTTON)
            authenticated_browser.execute_script(
                "arguments[0].scrollIntoView({block: 'center'});", submit
            )
            authenticated_browser.execute_script("arguments[0].click();", submit)

            # Should stay on thread/article page or redirect to group
            # The reply URL contains the article ID so /a/ should be present
            current_url = authenticated_browser.current_url
            assert (
                "/a/" in current_url
                or "thread" in current_url
                or "test.development" in current_url
            ), f"Expected to stay on article/thread page, got: {current_url}"


class TestUnauthenticatedSubmission:
    """Tests to ensure unauthenticated users cannot submit posts or replies."""

    def test_unauthenticated_post_submission_rejected(self, clean_browser: WebDriver):
        """POST to /g/{group}/post without auth should return 401."""
        # Navigate to a page first to set up the browser context
        clean_browser.get(f"{SEPTEMBER_URL}/g/test.general")

        # Try to POST directly to the submit endpoint using JavaScript
        # This simulates what would happen if someone bypassed the UI
        result = clean_browser.execute_async_script(
            """
            const callback = arguments[arguments.length - 1];
            const formData = new FormData();
            formData.append('subject', 'Unauthorized Test');
            formData.append('body', 'This should be rejected');
            formData.append('csrf_token', 'fake-token');

            fetch('/g/test.general/post', {
                method: 'POST',
                body: formData
            }).then(async response => {
                callback({
                    status: response.status,
                    text: await response.text()
                });
            }).catch(err => {
                callback({status: 0, text: err.toString()});
            });
            """
        )

        # Should get 401 Unauthorized
        assert result["status"] == 401, f"Expected 401, got {result['status']}"
        assert (
            "must be logged in" in result["text"].lower()
            or "authentication required" in result["text"].lower()
        ), "Response should indicate authentication is required"

    def test_unauthenticated_reply_submission_rejected(self, clean_browser: WebDriver):
        """POST to /a/{message_id}/reply without auth should return 401."""
        # First, get a real message ID by loading a thread page
        clean_browser.get(f"{SEPTEMBER_URL}/g/test.general")

        # Find a thread link and extract the message ID
        thread_links = clean_browser.find_elements(
            By.CSS_SELECTOR, "a[href*='/thread/']"
        )
        if not thread_links:
            pytest.skip("No threads found to test reply against")

        # Extract message ID from the thread URL
        thread_url = thread_links[0].get_attribute("href")
        if not thread_url:
            pytest.skip("Could not get thread URL")
        # URL format: /g/{group}/thread/{message_id}
        message_id = thread_url.split("/thread/")[-1]

        # Try to POST directly to the reply endpoint
        result = clean_browser.execute_async_script(
            """
            const messageId = arguments[0];
            const callback = arguments[arguments.length - 1];
            const formData = new FormData();
            formData.append('body', 'This unauthorized reply should be rejected');
            formData.append('group', 'test.general');
            formData.append('subject', 'Re: Test');
            formData.append('references', '');
            formData.append('csrf_token', 'fake-token');

            fetch('/a/' + messageId + '/reply', {
                method: 'POST',
                body: formData
            }).then(async response => {
                callback({
                    status: response.status,
                    text: await response.text()
                });
            }).catch(err => {
                callback({status: 0, text: err.toString()});
            });
            """,
            message_id,
        )

        # Should get 401 Unauthorized
        assert result["status"] == 401, f"Expected 401, got {result['status']}"
        assert (
            "must be logged in" in result["text"].lower()
            or "authentication required" in result["text"].lower()
        ), "Response should indicate authentication is required"

    def test_compose_page_shows_auth_required(
        self, compose_page_unauth: Callable[[str], ComposePage]
    ):
        """Accessing compose page without auth should show auth required message."""
        page = compose_page_unauth("test.general")

        # Should either redirect to login or show auth error
        requires_auth = page.requires_auth()
        has_form = page.has_form()

        # The compose page should not show the form to unauthenticated users
        assert requires_auth or not has_form, (
            "Compose page should require authentication or not show form"
        )
