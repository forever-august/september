"""Page object for Dex OIDC login page."""

from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.support import expected_conditions as EC
from selenium.webdriver.support.ui import WebDriverWait

from helpers.data import SEPTEMBER_URL, TEST_USER_EMAIL, TEST_USER_PASSWORD
from helpers.exceptions import AuthenticationError
from helpers.selectors import Selectors
from helpers.waits import POLL_FREQUENCY, TIMEOUT_OIDC

from .base import BasePage


class DexLoginPage(BasePage):
    """Page object for Dex OIDC provider login page."""

    def __init__(self, driver: WebDriver):
        super().__init__(driver)

    def is_on_dex(self) -> bool:
        """Check if currently on Dex login page."""
        return "dex" in self.current_url.lower()

    def wait_for_dex(self, timeout: float = TIMEOUT_OIDC) -> "DexLoginPage":
        """Wait for redirect to Dex."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        wait.until(
            lambda d: "dex" in d.current_url.lower() or "login" in d.page_source.lower()
        )
        return self

    def click_email_connector(self) -> "DexLoginPage":
        """Click 'Log in with Email' if present (connector selection page)."""
        try:
            quick_wait = WebDriverWait(self.driver, 1, poll_frequency=POLL_FREQUENCY)
            email_link = quick_wait.until(
                EC.element_to_be_clickable(
                    (By.LINK_TEXT, Selectors.Dex.EMAIL_CONNECTOR_TEXT)
                )
            )
            email_link.click()
        except Exception:
            # Not on connector selection page, continue
            pass
        return self

    def fill_credentials(
        self, email: str = TEST_USER_EMAIL, password: str = TEST_USER_PASSWORD
    ) -> "DexLoginPage":
        """Fill in email and password fields."""
        wait = WebDriverWait(self.driver, TIMEOUT_OIDC, poll_frequency=POLL_FREQUENCY)

        email_input = wait.until(
            EC.presence_of_element_located((By.NAME, Selectors.Dex.LOGIN_INPUT_NAME))
        )
        email_input.clear()
        email_input.send_keys(email)

        password_input = self.find_by_name(Selectors.Dex.PASSWORD_INPUT_NAME)
        if password_input:
            password_input.clear()
            password_input.send_keys(password)

        return self

    def submit(self) -> "DexLoginPage":
        """Submit the login form."""
        submit_button = self.find(Selectors.Dex.SUBMIT)
        submit_button.click()
        return self

    def wait_for_redirect_back(self, timeout: float = TIMEOUT_OIDC):
        """Wait for redirect back to September after login."""
        wait = WebDriverWait(self.driver, timeout, poll_frequency=POLL_FREQUENCY)
        # Wait for URL to contain September URL (without protocol)
        wait.until(EC.url_contains(SEPTEMBER_URL.replace("http://", "")))

    def login(
        self, email: str = TEST_USER_EMAIL, password: str = TEST_USER_PASSWORD
    ) -> None:
        """Complete the full login flow."""
        self.wait_for_dex()
        self.click_email_connector()
        self.fill_credentials(email, password)
        self.submit()
        self.wait_for_redirect_back()

    def login_and_verify(
        self, email: str = TEST_USER_EMAIL, password: str = TEST_USER_PASSWORD
    ) -> bool:
        """Login and verify success by checking for logged-in indicators."""
        try:
            self.login(email, password)
            # Check for logged-in indicators
            return self.exists(Selectors.Auth.LOGGED_IN_INDICATORS)
        except Exception:
            return False

    def has_login_error(self) -> bool:
        """Check if login page shows an error."""
        page_lower = self.page_source.lower()
        return "error" in page_lower or "invalid" in page_lower
