"""Wait utilities and custom expected conditions."""

from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

# Default timeouts (seconds)
TIMEOUT_DEFAULT = 3
TIMEOUT_OIDC = 5
POLL_FREQUENCY = 0.2


def create_wait(driver: WebDriver, timeout: float = TIMEOUT_DEFAULT) -> WebDriverWait:
    """Create a WebDriverWait with standard poll frequency."""
    return WebDriverWait(driver, timeout, poll_frequency=POLL_FREQUENCY)


def wait_for_element(
    driver: WebDriver, selector: str, timeout: float = TIMEOUT_DEFAULT
):
    """Wait for element to be present and return it."""
    wait = create_wait(driver, timeout)
    return wait.until(EC.presence_of_element_located((By.CSS_SELECTOR, selector)))


def wait_for_url_contains(
    driver: WebDriver, substring: str, timeout: float = TIMEOUT_DEFAULT
):
    """Wait for URL to contain substring."""
    wait = create_wait(driver, timeout)
    return wait.until(EC.url_contains(substring))


def wait_for_url_not_contains(
    driver: WebDriver, substring: str, timeout: float = TIMEOUT_DEFAULT
):
    """Wait for URL to NOT contain substring."""
    wait = create_wait(driver, timeout)
    return wait.until(lambda d: substring not in d.current_url)


def wait_for_navigation_from(
    driver: WebDriver, original_url: str, timeout: float = TIMEOUT_DEFAULT
):
    """Wait for URL to change from original."""
    wait = create_wait(driver, timeout)
    return wait.until(lambda d: d.current_url != original_url)


class url_matches_any:
    """Expected condition: URL contains any of the given substrings."""

    def __init__(self, *substrings: str):
        self.substrings = substrings

    def __call__(self, driver):
        return any(s in driver.current_url for s in self.substrings)


class element_has_non_empty_text:
    """Expected condition: Element exists and has non-empty text."""

    def __init__(self, locator):
        self.locator = locator

    def __call__(self, driver):
        elements = driver.find_elements(*self.locator)
        return [e for e in elements if e.text.strip()]
