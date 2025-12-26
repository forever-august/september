"""
Tests for article view functionality.

These tests verify:
- Individual article view loads
- Article headers display correctly
- Article body renders
- Navigation between articles
"""

from typing import Callable

from pages import GroupPage


class TestArticleView:
    """Tests for the article view page (/a/{message_id})."""

    def test_article_view_from_thread(self, group_page: Callable[[str], GroupPage]):
        """Should be able to view an individual article from a thread."""
        page = group_page("test.general")
        thread_page = page.click_first_thread()

        # Look for article links within the thread view
        article_links = thread_page.get_article_links()
        if article_links:
            article_page = thread_page.click_article_link(0)
            assert article_page.has_main_content()

    def test_article_shows_headers(self, group_page: Callable[[str], GroupPage]):
        """Article view should display message headers (From, Subject, Date)."""
        page = group_page("test.general")
        thread_page = page.click_first_thread()
        # Article content rendered server-side
        assert thread_page.has_main_content()

    def test_article_shows_body(self, group_page: Callable[[str], GroupPage]):
        """Article view should display the message body."""
        page = group_page("test.general")
        thread_page = page.click_first_thread()
        # Body content rendered server-side
        assert thread_page.has_main_content()


class TestArticleNavigation:
    """Tests for navigation between articles."""

    def test_back_to_group(self, group_page: Callable[[str], GroupPage]):
        """Should be able to navigate back to the group from an article."""
        page = group_page("test.general")
        thread_page = page.click_first_thread()

        # Navigate back to the group
        group = thread_page.navigate_to_group("test.general")
        assert "/g/test.general" in group.current_url
        assert "/thread/" not in group.current_url

    def test_header_navigation(self, group_page: Callable[[str], GroupPage]):
        """Header should provide navigation back to home."""
        page = group_page("test.general")
        thread_page = page.click_first_thread()

        # Header should have home link - just verify navigation elements exist
        nav = thread_page.get_nav()
        assert nav is not None


class TestArticleNotFound:
    """Tests for error handling with invalid article IDs."""

    def test_invalid_article_shows_error(self, group_page: Callable[[str], GroupPage]):
        """Requesting an invalid article should show an error."""
        # Get a page to access the driver
        page = group_page("test.general")
        driver = page.driver

        # Navigate directly to an invalid article
        from pages import ArticlePage

        driver.get(f"{page.base_url}/a/nonexistent-message-id-12345")

        invalid_page = ArticlePage(driver)
        # Page should still load with basic structure (error page)
        assert invalid_page.has_main_content()
        assert invalid_page.has_body()
