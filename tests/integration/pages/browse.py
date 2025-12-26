"""Page object for the browse page."""

from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement

from helpers.selectors import Selectors

from .base import BasePage


class BrowsePage(BasePage):
    """Page object for the browse page (/browse/{prefix})."""

    def __init__(self, driver: WebDriver, prefix: str = ""):
        super().__init__(driver)
        self.prefix = prefix

    def load(self) -> "BrowsePage":
        """Navigate to the browse page."""
        if self.prefix:
            self.driver.get(f"{self.base_url}/browse/{self.prefix}")
        else:
            self.driver.get(f"{self.base_url}/")
        return self

    def has_group_cards(self) -> bool:
        """Check if group cards container exists."""
        return self.exists(Selectors.Home.GROUP_CARDS)

    def get_group_cards(self) -> list[WebElement]:
        """Get all group card elements."""
        return self.find_all(Selectors.Home.GROUP_CARD)

    def has_page_header(self) -> bool:
        """Check if page header exists."""
        return self.exists(Selectors.Browse.PAGE_HEADER)

    def get_page_header(self) -> WebElement:
        """Get the page header element."""
        return self.find(Selectors.Browse.PAGE_HEADER)

    def has_breadcrumb_home_link(self) -> bool:
        """Check if breadcrumb has home link."""
        return self.exists(Selectors.Browse.BREADCRUMB_HOME)

    def get_breadcrumb_home_link(self) -> WebElement:
        """Get the breadcrumb home link."""
        return self.find(Selectors.Browse.BREADCRUMB_HOME)

    def click_home_breadcrumb(self) -> "HomePage":
        """Click the home link in breadcrumbs."""
        from .home import HomePage

        link = self.get_breadcrumb_home_link()
        link.click()
        self.wait_for_url_contains(self.base_url)
        return HomePage(self.driver)


# Forward reference import
from .home import HomePage
