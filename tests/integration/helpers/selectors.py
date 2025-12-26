"""Centralized CSS selectors for integration tests.

Single point of change when template structure is modified.
Selectors are organized by page/component for easy maintenance.
"""


class Selectors:
    """CSS selectors for page elements."""

    class Layout:
        """Common layout elements."""

        MAIN = "main"
        BODY = "body"
        HEADER = "header, .site-header"
        NAV = "header, .site-header, nav"
        HOME_LINK = "a[href='/']"

    class Home:
        """Home page elements."""

        GROUP_CARDS = ".group-cards"
        GROUP_CARD = ".group-card"
        GROUP_CARD_LINK = ".group-card a.group-card-link"
        SEARCH_INPUT = "#group-search"

    class Browse:
        """Browse page elements."""

        PAGE_HEADER = ".page-header"
        BREADCRUMB_HOME = ".page-header a[href='/']"

    class ThreadList:
        """Thread list (group page) elements."""

        CONTAINER = ".thread-list"
        EMPTY_STATE = ".empty-state"
        THREAD_CARD = ".thread-card, .thread-card-link"
        THREAD_LINK = ".thread-card-link"
        THREAD_TITLE = ".thread-title, .thread-card-link"

    class Article:
        """Article/thread view elements."""

        CONTENT = ".comment, .article-content"
        ARTICLE_LINK = "a[href*='/a/']"

        @staticmethod
        def group_link(group: str) -> str:
            """Get selector for link back to specific group."""
            return f"a[href*='/g/{group}']"

    class Compose:
        """Compose page elements."""

        FORM = "form, .compose-form"
        FORM_SPECIFIC = ".compose-form"
        POST_FORM = "form[action*='post'], .compose-form"
        SUBJECT_INPUT = "input[name='subject']"
        SUBJECT_ANY = "input[name='subject'], input[type='text']"
        BODY_INPUT = "textarea[name='body']"
        BODY_ANY = "textarea[name='body'], textarea"
        SUBMIT_BUTTON = "button[type='submit']"
        SUBMIT_SPECIFIC = ".compose-form button[type='submit']"
        SUBMIT_ANY = "button[type='submit'], input[type='submit']"
        CSRF_TOKEN = "input[name='csrf_token'], input[name='_csrf']"

    class Reply:
        """Reply form elements."""

        FORM = "form[action*='reply']"
        TEXTAREA = ".reply-form textarea, form textarea"
        ELEMENTS = (
            "form[action*='reply'], button[class*='reply'], "
            "a[href*='reply'], .reply-form, textarea"
        )

    class Auth:
        """Authentication elements."""

        USER_INFO = ".user-info"
        LOGOUT_FORM = "form[action*='logout']"
        LOGOUT_LINK = "a[href*='logout']"
        LOGGED_IN_INDICATORS = ".user-info, [href*='logout'], form[action*='logout']"

    class Dex:
        """Dex OIDC provider elements."""

        EMAIL_CONNECTOR_TEXT = "Log in with Email"
        LOGIN_INPUT_NAME = "login"
        PASSWORD_INPUT_NAME = "password"
        SUBMIT = "button[type='submit']"

    class Notifications:
        """Notification/flash message elements."""

        FLASH_MESSAGE = ".flash-message, .notification"
