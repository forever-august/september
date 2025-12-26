"""Custom exceptions for integration tests."""


class IntegrationTestError(Exception):
    """Base exception for integration test failures."""

    pass


class PageLoadError(IntegrationTestError):
    """Page failed to load expected content."""

    pass


class ElementNotFoundError(IntegrationTestError):
    """Expected element was not found on the page."""

    pass


class NoTestDataError(IntegrationTestError):
    """Expected test data (threads, articles, etc.) was not found."""

    pass


class AuthenticationError(IntegrationTestError):
    """Authentication flow failed."""

    pass


class NavigationError(IntegrationTestError):
    """Navigation to expected page failed."""

    pass
