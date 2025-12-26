"""Page object for the home page."""

from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement

from helpers.selectors import Selectors

from .base import BasePage


class HomePage(BasePage):
    """Page object for the home page (/)."""

    def __init__(self, driver: WebDriver):
        super().__init__(driver)

    def load(self) -> "HomePage":
        """Navigate to the home page."""
        self.driver.get(f"{self.base_url}/")
        return self

    def has_group_cards(self) -> bool:
        """Check if group cards container exists."""
        return self.exists(Selectors.Home.GROUP_CARDS)

    def get_group_cards(self) -> list[WebElement]:
        """Get all group card elements."""
        return self.find_all(Selectors.Home.GROUP_CARD)

    def get_group_card_links(self) -> list[WebElement]:
        """Get all group card link elements."""
        return self.find_all(Selectors.Home.GROUP_CARD_LINK)

    def get_group_count(self) -> int:
        """Get number of group cards displayed."""
        return self.count(Selectors.Home.GROUP_CARD)

    def has_search_input(self) -> bool:
        """Check if search input exists."""
        return self.exists(Selectors.Home.SEARCH_INPUT)

    def get_search_input(self) -> WebElement:
        """Get the search input element."""
        return self.find(Selectors.Home.SEARCH_INPUT)

    def search(self, query: str) -> "HomePage":
        """Type in the search box."""
        search_input = self.get_search_input()
        search_input.send_keys(query)
        return self

    def get_search_value(self) -> str:
        """Get current value of search input."""
        return self.get_search_input().get_attribute("value") or ""

    def click_first_group(self) -> "GroupPage | BrowsePage":
        """Click the first group card and return the resulting page."""
        from .browse import BrowsePage
        from .group import GroupPage

        links = self.get_group_card_links()
        if not links:
            from helpers.exceptions import NoTestDataError

            raise NoTestDataError("No group cards found on home page")

        original_url = self.current_url
        links[0].click()
        self.wait_for_navigation_from(original_url)

        # Determine what page we landed on
        if "/g/" in self.current_url:
            return GroupPage(self.driver, self._extract_group_from_url())
        else:
            return BrowsePage(self.driver, self._extract_prefix_from_url())

    def _extract_group_from_url(self) -> str:
        """Extract group name from current URL."""
        # URL format: .../g/group.name
        if "/g/" in self.current_url:
            return self.current_url.split("/g/")[1].split("/")[0].split("?")[0]
        return ""

    def _extract_prefix_from_url(self) -> str:
        """Extract browse prefix from current URL."""
        # URL format: .../browse/prefix
        if "/browse/" in self.current_url:
            return self.current_url.split("/browse/")[1].split("/")[0].split("?")[0]
        return ""


# Forward reference imports at module level to avoid circular imports
from .browse import BrowsePage
from .group import GroupPage
