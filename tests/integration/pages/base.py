"""Base page object with common functionality."""

from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

from helpers.data import SEPTEMBER_URL
from helpers.exceptions import ElementNotFoundError
from helpers.selectors import Selectors
from helpers.waits import POLL_FREQUENCY, TIMEOUT_DEFAULT


class BasePage:
    """Base class for all page objects."""

    def __init__(self, driver: WebDriver):
        self.driver = driver
        self.base_url = SEPTEMBER_URL
        self._wait: WebDriverWait | None = None

    @property
    def wait(self) -> WebDriverWait:
        """Lazy-initialized WebDriverWait."""
        if self._wait is None:
            self._wait = WebDriverWait(
                self.driver, TIMEOUT_DEFAULT, poll_frequency=POLL_FREQUENCY
            )
        return self._wait

    @property
    def current_url(self) -> str:
        """Get current browser URL."""
        return self.driver.current_url

    @property
    def title(self) -> str:
        """Get page title."""
        return self.driver.title

    @property
    def page_source(self) -> str:
        """Get page source HTML."""
        return self.driver.page_source

    # Element finding methods
    def find(self, selector: str) -> WebElement:
        """Find single element by CSS selector. Raises if not found."""
        elements = self.driver.find_elements(By.CSS_SELECTOR, selector)
        if not elements:
            raise ElementNotFoundError(f"Element not found: {selector}")
        return elements[0]

    def find_all(self, selector: str) -> list[WebElement]:
        """Find all elements matching CSS selector."""
        return self.driver.find_elements(By.CSS_SELECTOR, selector)

    def find_optional(self, selector: str) -> WebElement | None:
        """Find element, return None if not found."""
        elements = self.driver.find_elements(By.CSS_SELECTOR, selector)
        return elements[0] if elements else None

    def find_by_link_text(self, text: str) -> WebElement | None:
        """Find element by link text, return None if not found."""
        elements = self.driver.find_elements(By.LINK_TEXT, text)
        return elements[0] if elements else None

    def find_by_name(self, name: str) -> WebElement | None:
        """Find element by name attribute, return None if not found."""
        elements = self.driver.find_elements(By.NAME, name)
        return elements[0] if elements else None

    def exists(self, selector: str) -> bool:
        """Check if element exists on page."""
        return len(self.find_all(selector)) > 0

    def count(self, selector: str) -> int:
        """Count elements matching selector."""
        return len(self.find_all(selector))

    # Wait methods
    def wait_for(self, selector: str, timeout: float = TIMEOUT_DEFAULT) -> WebElement:
        """Wait for element to be present."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        return wait.until(EC.presence_of_element_located((By.CSS_SELECTOR, selector)))

    def wait_for_clickable(
        self, selector: str, timeout: float = TIMEOUT_DEFAULT
    ) -> WebElement:
        """Wait for element to be clickable."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        return wait.until(EC.element_to_be_clickable((By.CSS_SELECTOR, selector)))

    def wait_for_url_contains(self, substring: str, timeout: float = TIMEOUT_DEFAULT):
        """Wait for URL to contain substring."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        wait.until(EC.url_contains(substring))

    def wait_for_url_not_contains(
        self, substring: str, timeout: float = TIMEOUT_DEFAULT
    ):
        """Wait for URL to NOT contain substring."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        wait.until(lambda d: substring not in d.current_url)

    def wait_for_navigation_from(
        self, original_url: str, timeout: float = TIMEOUT_DEFAULT
    ):
        """Wait for URL to change from original."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        wait.until(lambda d: d.current_url != original_url)

    # Common checks
    def has_main_content(self) -> bool:
        """Verify page has main content area."""
        return self.exists(Selectors.Layout.MAIN)

    def has_body(self) -> bool:
        """Verify page has body element."""
        return self.exists(Selectors.Layout.BODY)

    def get_header(self) -> WebElement:
        """Get the page header element."""
        return self.find(Selectors.Layout.HEADER)

    def get_nav(self) -> WebElement:
        """Get navigation element."""
        return self.find(Selectors.Layout.NAV)

    def click_home_link(self) -> "HomePage":
        """Click the home link in header and return HomePage."""
        from pages.home import HomePage

        home_link = self.find(Selectors.Layout.HOME_LINK)
        home_link.click()
        self.wait_for_url_contains(self.base_url)
        return HomePage(self.driver)

    def is_on_compose_page(self) -> bool:
        """Check if currently on a compose page."""
        return "/compose" in self.current_url
