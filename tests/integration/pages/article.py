"""Page object for the individual article view page."""

from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement

from helpers.selectors import Selectors

from .base import BasePage


class ArticlePage(BasePage):
    """Page object for individual article view (/a/{message_id})."""

    def __init__(self, driver: WebDriver):
        super().__init__(driver)

    def has_content(self) -> bool:
        """Check if article content is displayed."""
        return self.exists(Selectors.Article.CONTENT)

    def get_content(self) -> WebElement:
        """Get the article content element."""
        return self.find(Selectors.Article.CONTENT)

    def get_article_links(self) -> list[WebElement]:
        """Get all links to other articles."""
        return self.find_all(Selectors.Article.ARTICLE_LINK)

    def navigate_to_group(self, group: str) -> "GroupPage":
        """Navigate back to the group page."""
        from .group import GroupPage

        selector = Selectors.Article.group_link(group)
        links = self.find_all(selector)

        if links:
            links[0].click()
            # Wait for URL to contain group path but not thread/article paths
            self.wait.until(
                lambda d: f"/g/{group}" in d.current_url
                and "/thread/" not in d.current_url
            )
        else:
            # Fall back to browser back
            self.driver.back()

        return GroupPage(self.driver, group)

    def navigate_home(self) -> "HomePage":
        """Navigate to home using header link."""
        from selenium.webdriver.common.by import By

        from .home import HomePage

        header = self.get_nav()
        home_links = header.find_elements(By.CSS_SELECTOR, Selectors.Layout.HOME_LINK)

        if home_links:
            home_links[0].click()
            self.wait_for_url_contains(self.base_url)

        return HomePage(self.driver)

    def click_home_in_header(self) -> "HomePage":
        """Click home link in header and return HomePage."""
        from selenium.webdriver.common.by import By

        from .home import HomePage

        nav = self.get_nav()
        home_link = nav.find_element(By.CSS_SELECTOR, Selectors.Layout.HOME_LINK)
        home_link.click()
        self.wait.until(lambda d: d.current_url == f"{self.base_url}/")
        return HomePage(self.driver)


# Forward reference imports
from .group import GroupPage
from .home import HomePage
