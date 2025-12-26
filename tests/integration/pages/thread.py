"""Page object for the thread view page."""

from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement

from helpers.exceptions import NoTestDataError
from helpers.selectors import Selectors

from .base import BasePage


class ThreadPage(BasePage):
    """Page object for thread/article view (/a/{message_id} or /g/{group}/thread/{id})."""

    def __init__(self, driver: WebDriver):
        super().__init__(driver)

    def has_articles(self) -> bool:
        """Check if any articles are displayed."""
        return self.count(Selectors.Article.CONTENT) > 0

    def get_article_count(self) -> int:
        """Get number of articles displayed."""
        return self.count(Selectors.Article.CONTENT)

    def get_articles(self) -> list[WebElement]:
        """Get all article content elements."""
        return self.find_all(Selectors.Article.CONTENT)

    def require_articles(self) -> list[WebElement]:
        """Get articles, raising NoTestDataError if none exist."""
        articles = self.get_articles()
        if not articles:
            raise NoTestDataError("No articles found in thread view")
        return articles

    def get_article_links(self) -> list[WebElement]:
        """Get all links to individual articles."""
        return self.find_all(Selectors.Article.ARTICLE_LINK)

    def click_article_link(self, index: int = 0) -> "ArticlePage":
        """Click an article link and return ArticlePage."""
        from .article import ArticlePage

        links = self.get_article_links()
        if not links:
            raise NoTestDataError("No article links found in thread")
        if index >= len(links):
            raise NoTestDataError(
                f"Article index {index} out of range (have {len(links)} links)"
            )

        links[index].click()
        self.wait_for_url_contains("/a/")

        return ArticlePage(self.driver)

    def has_reply_form(self) -> bool:
        """Check if reply form exists."""
        return self.exists(Selectors.Reply.FORM)

    def has_reply_elements(self) -> bool:
        """Check if any reply-related elements exist."""
        return self.exists(Selectors.Reply.ELEMENTS)

    def has_reply_textarea(self) -> bool:
        """Check if reply textarea exists."""
        return self.exists(Selectors.Reply.TEXTAREA)

    def get_reply_textareas(self) -> list[WebElement]:
        """Get all reply textarea elements."""
        return self.find_all(Selectors.Reply.TEXTAREA)

    def get_reply_forms(self) -> list[WebElement]:
        """Get all reply form elements."""
        return self.find_all(Selectors.Reply.FORM)

    def submit_reply(self, body: str) -> "ThreadPage":
        """Submit a reply to the thread."""
        from selenium.webdriver.common.by import By

        textareas = self.get_reply_textareas()
        if not textareas:
            raise NoTestDataError("No reply textarea found")

        # Use the last textarea (usually the reply form at the bottom)
        textarea = textareas[-1]
        textarea.clear()
        textarea.send_keys(body)

        # Find the form containing this textarea and submit
        form = textarea.find_element(By.XPATH, "./ancestor::form")
        submit = form.find_element(By.CSS_SELECTOR, Selectors.Compose.SUBMIT_BUTTON)
        submit.click()

        return self

    def navigate_to_group(self, group: str) -> "GroupPage":
        """Navigate back to the group page."""
        from .group import GroupPage

        selector = Selectors.Article.group_link(group)
        links = self.find_all(selector)

        if links:
            links[0].click()
            self.wait_for_url_contains(f"/g/{group}")
        else:
            # Fall back to browser back
            self.driver.back()

        return GroupPage(self.driver, group)


# Forward reference imports
from .article import ArticlePage
from .group import GroupPage
