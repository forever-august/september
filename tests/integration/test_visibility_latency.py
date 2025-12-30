"""
Tests for measuring post and reply visibility latency.

These tests measure the time between submitting a post or reply and when
the content becomes visible to the user. Results are reported in the
visibility latency report at the end of the test session.

Note: These tests modify data in the NNTP server.
"""

import uuid
from typing import Callable

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver

from helpers import Selectors
from pages import ComposePage, GroupPage
from testlogging import VisibilityTimer


class TestPostVisibilityLatency:
    """Measure time between posting a new article and when it becomes visible."""

    @pytest.mark.auth
    @pytest.mark.posting
    @pytest.mark.performance
    def test_new_post_visibility_latency(
        self,
        compose_page: Callable[[str], ComposePage],
        visibility_timer: VisibilityTimer,
    ):
        """
        Measure time from new post submission to visibility in thread view.

        This test:
        1. Creates a new post with a unique identifier
        2. Records the timestamp just before clicking submit
        3. Submits the post
        4. Polls until the post content is visible
        5. Records the latency measurement

        The timing is automatically collected and reported at the end of
        the test session.
        """
        page = compose_page("test.general")

        unique_id = str(uuid.uuid4())[:8]
        test_subject = f"Visibility Test {unique_id}"
        test_body = f"This is a visibility latency test post.\n\nTest ID: {unique_id}"

        # Fill in the form
        page.fill_subject(test_subject)
        page.fill_body(test_body)

        # Mark timestamp just before submit
        visibility_timer.mark_submit("post", "test.general", unique_id)

        # Submit and wait for navigation
        result = page.submit()

        # Wait for the post to become visible
        # After submission, we're redirected to the group page (thread list)
        # The unique_id is in the subject which appears in .thread-title
        timing = visibility_timer.wait_for_visible(
            unique_id,
            # Look for the subject in thread titles on the group page
            ".thread-title, .thread-card-link",
            timeout=10,
        )

        # Timing is automatically collected by the fixture
        # We just verify the post was found (no assertion on timing value)
        assert timing.latency_ms > 0, "Latency should be positive"


class TestReplyVisibilityLatency:
    """Measure time between replying to a thread and when the reply becomes visible."""

    @pytest.mark.auth
    @pytest.mark.posting
    @pytest.mark.performance
    def test_reply_visibility_latency(
        self,
        authenticated_browser: WebDriver,
        visibility_timer: VisibilityTimer,
    ):
        """
        Measure time from reply submission to visibility in thread view.

        This test:
        1. Navigates to an existing thread
        2. Submits a reply with a unique identifier
        3. Records the timestamp just before clicking submit
        4. Polls until the reply content is visible
        5. Records the latency measurement

        The timing is automatically collected and reported at the end of
        the test session.
        """
        # Navigate to a thread in test.general
        page = GroupPage(authenticated_browser, "test.general").load()

        if not page.has_threads():
            pytest.skip("No threads available to reply to")

        thread_page = page.click_first_thread()

        # Check for reply form
        if not thread_page.has_reply_textarea():
            pytest.skip("Reply form not found on this page")

        unique_id = str(uuid.uuid4())[:8]
        test_body = f"This is a visibility latency test reply.\n\nTest ID: {unique_id}"

        # Get the reply textarea
        textareas = thread_page.get_reply_textareas()
        if not textareas:
            pytest.skip("No reply textarea found")

        # Use the last textarea (usually the reply form at the bottom)
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

        # Find the submit button
        form = textarea.find_element(By.XPATH, "./ancestor::form")
        submit = form.find_element(By.CSS_SELECTOR, Selectors.Compose.SUBMIT_BUTTON)
        authenticated_browser.execute_script(
            "arguments[0].scrollIntoView({block: 'center'});", submit
        )

        # Mark timestamp just before submit
        visibility_timer.mark_submit("reply", "test.general", unique_id)

        # Click submit
        authenticated_browser.execute_script("arguments[0].click();", submit)

        # Wait for the reply to become visible in the thread
        # After reply submission, we're redirected back to the thread view
        # The unique_id is in the reply body which appears in .comment-body or .article-text
        # We also check .comment since the whole comment div contains the text
        #
        # Note: Replies may take longer to appear than new posts because:
        # 1. The thread view is cached
        # 2. Background refresh runs every 1-30s based on activity level
        # 3. In low-activity test environments, refresh period can be ~24s
        # We use a 35s timeout to allow for the full refresh cycle.
        timing = visibility_timer.wait_for_visible(
            unique_id,
            ".comment-body, .article-text, .comment, pre",
            timeout=35,
        )

        # Timing is automatically collected by the fixture
        # We just verify the reply was found (no assertion on timing value)
        assert timing.latency_ms > 0, "Latency should be positive"
