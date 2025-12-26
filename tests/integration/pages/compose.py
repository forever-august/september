"""Page object for the compose page."""

from selenium.webdriver.remote.webdriver import WebDriver
from selenium.webdriver.remote.webelement import WebElement

from helpers.selectors import Selectors

from .base import BasePage


class ComposePage(BasePage):
    """Page object for compose page (/g/{group}/compose)."""

    def __init__(self, driver: WebDriver, group_name: str):
        super().__init__(driver)
        self.group_name = group_name

    def load(self) -> "ComposePage":
        """Navigate to the compose page."""
        self.driver.get(f"{self.base_url}/g/{self.group_name}/compose")
        return self

    def has_form(self) -> bool:
        """Check if compose form exists."""
        return self.exists(Selectors.Compose.FORM)

    def has_specific_form(self) -> bool:
        """Check if the specific compose form exists."""
        return self.exists(Selectors.Compose.FORM_SPECIFIC)

    def has_subject_field(self) -> bool:
        """Check if subject input exists."""
        return self.exists(Selectors.Compose.SUBJECT_INPUT)

    def has_body_field(self) -> bool:
        """Check if body textarea exists."""
        return self.exists(Selectors.Compose.BODY_INPUT)

    def has_submit_button(self) -> bool:
        """Check if submit button exists."""
        return self.exists(Selectors.Compose.SUBMIT_BUTTON)

    def has_csrf_token(self) -> bool:
        """Check if CSRF token field exists."""
        return self.exists(Selectors.Compose.CSRF_TOKEN)

    def get_csrf_token_value(self) -> str | None:
        """Get the CSRF token value."""
        elem = self.find_optional(Selectors.Compose.CSRF_TOKEN)
        if elem:
            return elem.get_attribute("value")
        return None

    def get_subject_input(self) -> WebElement:
        """Get the subject input element."""
        return self.find(Selectors.Compose.SUBJECT_INPUT)

    def get_body_input(self) -> WebElement:
        """Get the body textarea element."""
        return self.find(Selectors.Compose.BODY_INPUT)

    def get_submit_button(self) -> WebElement:
        """Get the submit button element."""
        return self.find(Selectors.Compose.SUBMIT_SPECIFIC)

    def fill_subject(self, subject: str) -> "ComposePage":
        """Fill in the subject field."""
        elem = self.get_subject_input()
        elem.clear()
        elem.send_keys(subject)
        return self

    def fill_body(self, body: str) -> "ComposePage":
        """Fill in the body field."""
        elem = self.get_body_input()
        elem.clear()
        elem.send_keys(body)
        return self

    def submit(self) -> "GroupPage | ComposePage":
        """Submit the compose form and return the resulting page."""
        from .group import GroupPage

        submit = self.get_submit_button()
        submit.click()

        # Wait for navigation away from compose page or stay for error
        self.wait_for_url_not_contains("/compose")

        # Return GroupPage if we navigated away successfully
        if "/compose" not in self.current_url:
            return GroupPage(self.driver, self.group_name)

        # Still on compose page (likely validation error)
        return self

    def compose_and_submit(self, subject: str, body: str) -> "GroupPage | ComposePage":
        """Fill in and submit a new post."""
        self.fill_subject(subject)
        self.fill_body(body)
        return self.submit()

    def is_on_compose_page(self) -> bool:
        """Check if still on compose page."""
        return "/compose" in self.current_url

    def has_error_message(self) -> bool:
        """Check if page contains error indicators."""
        page_lower = self.page_source.lower()
        return "error" in page_lower or "required" in page_lower

    def requires_auth(self) -> bool:
        """Check if page indicates authentication is required."""
        page_lower = self.page_source.lower()
        url_lower = self.current_url.lower()
        return (
            "login" in url_lower
            or "auth" in url_lower
            or "sign in" in page_lower
            or "log in" in page_lower
            or "authentication" in page_lower
            or "not authorized" in page_lower
            or "must be logged in" in page_lower
        )


# Forward reference import
from .group import GroupPage
