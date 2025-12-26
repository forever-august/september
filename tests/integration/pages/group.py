"""Page object for the group/thread list page."""

from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement

from helpers.exceptions import NoTestDataError, PageLoadError
from helpers.selectors import Selectors

from .base import BasePage


class GroupPage(BasePage):
    """Page object for /g/{group} - thread list view."""

    def __init__(self, driver: WebDriver, group_name: str):
        super().__init__(driver)
        self.group_name = group_name

    def load(self) -> "GroupPage":
        """Navigate to the group page and wait for it to load."""
        self.driver.get(f"{self.base_url}/g/{self.group_name}")

        # Page must have either thread list or empty state
        has_threads = self.exists(Selectors.ThreadList.CONTAINER)
        has_empty = self.exists(Selectors.ThreadList.EMPTY_STATE)

        if not has_threads and not has_empty:
            raise PageLoadError(
                f"Group page for {self.group_name} did not load correctly"
            )

        return self

    def has_thread_list(self) -> bool:
        """Check if thread list container exists."""
        return self.exists(Selectors.ThreadList.CONTAINER)

    def has_empty_state(self) -> bool:
        """Check if empty state is displayed."""
        return self.exists(Selectors.ThreadList.EMPTY_STATE)

    def has_threads(self) -> bool:
        """Check if the group has any threads."""
        return self.count(Selectors.ThreadList.THREAD_LINK) > 0

    def get_thread_count(self) -> int:
        """Get number of threads displayed."""
        return self.count(Selectors.ThreadList.THREAD_LINK)

    def get_thread_links(self) -> list[WebElement]:
        """Get all thread link elements."""
        return self.find_all(Selectors.ThreadList.THREAD_LINK)

    def get_thread_cards(self) -> list[WebElement]:
        """Get all thread card elements."""
        return self.find_all(Selectors.ThreadList.THREAD_CARD)

    def get_thread_titles(self) -> list[str]:
        """Get text of all thread titles."""
        elements = self.find_all(Selectors.ThreadList.THREAD_TITLE)
        return [e.text.strip() for e in elements if e.text.strip()]

    def require_threads(self) -> list[WebElement]:
        """Get threads, raising NoTestDataError if none exist."""
        threads = self.get_thread_links()
        if not threads:
            raise NoTestDataError(f"No threads found in group {self.group_name}")
        return threads

    def click_first_thread(self) -> "ThreadPage":
        """Click the first thread and return ThreadPage."""
        from .thread import ThreadPage

        threads = self.require_threads()
        threads[0].click()

        # Wait for navigation to article/thread view
        self.wait.until(lambda d: "/a/" in d.current_url or "/thread/" in d.current_url)

        return ThreadPage(self.driver)

    def click_thread(self, index: int) -> "ThreadPage":
        """Click thread at given index."""
        from .thread import ThreadPage

        threads = self.require_threads()
        if index >= len(threads):
            raise NoTestDataError(
                f"Thread index {index} out of range (have {len(threads)} threads)"
            )

        threads[index].click()
        self.wait.until(lambda d: "/a/" in d.current_url or "/thread/" in d.current_url)

        return ThreadPage(self.driver)

    def is_group_in_title(self) -> bool:
        """Check if group name appears in page title or content."""
        return self.group_name in self.title or self.group_name in self.page_source

    def navigate_to_compose(self) -> "ComposePage":
        """Navigate to compose page for this group."""
        from .compose import ComposePage

        self.driver.get(f"{self.base_url}/g/{self.group_name}/compose")
        return ComposePage(self.driver, self.group_name)


# Forward reference imports
from .compose import ComposePage
from .thread import ThreadPage
