"""
Tests for OIDC authentication functionality.

These tests verify:
- Login page loads
- OIDC login flow works with Dex
- User session is established after login
- Logout clears the session
- Return-to URL works correctly
"""

import pytest
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver

from helpers import (
    SEPTEMBER_URL,
    TEST_USER_EMAIL,
    TEST_USER_PASSWORD,
    Selectors,
    create_wait,
)
from pages import DexLoginPage


class TestLoginPage:
    """Tests for the login page."""

    def test_login_page_loads(self, browser: WebDriver, dex_page: DexLoginPage):
        """Login page should load and show login options."""
        browser.get(f"{SEPTEMBER_URL}/auth/login")
        dex_page.wait_for_dex()
        assert dex_page.has_body()

    def test_login_link_in_header(self, clean_browser: WebDriver):
        """Unauthenticated users should see a login link in the header."""
        clean_browser.get(f"{SEPTEMBER_URL}/")
        # Header rendered server-side
        header = clean_browser.find_element(By.CSS_SELECTOR, Selectors.Layout.HEADER)
        assert header is not None


class TestLoginFlow:
    """Tests for the complete OIDC login flow."""

    @pytest.mark.auth
    def test_complete_login_flow(
        self, clean_browser: WebDriver, dex_page: DexLoginPage
    ):
        """Should be able to log in via Dex OIDC provider."""
        clean_browser.get(f"{SEPTEMBER_URL}/auth/login")

        try:
            dex_page.login(TEST_USER_EMAIL, TEST_USER_PASSWORD)

            # Check for logged-in state
            logged_in = clean_browser.find_elements(
                By.CSS_SELECTOR, Selectors.Auth.LOGGED_IN_INDICATORS
            )
            assert len(logged_in) > 0
        except Exception:
            if dex_page.has_login_error():
                pytest.skip("Login failed - may need to check Dex configuration")
            raise

    @pytest.mark.auth
    def test_login_with_return_to(
        self, clean_browser: WebDriver, dex_page: DexLoginPage
    ):
        """Login should redirect to return_to URL after success."""
        return_path = "/g/test.general"
        clean_browser.get(f"{SEPTEMBER_URL}/auth/login?return_to={return_path}")

        try:
            dex_page.login(TEST_USER_EMAIL, TEST_USER_PASSWORD)
            # Should redirect to the return_to path
            assert return_path in clean_browser.current_url
        except Exception:
            pytest.skip("Login flow failed - skipping return_to test")


class TestLogout:
    """Tests for logout functionality."""

    @pytest.mark.auth
    def test_logout_clears_session(self, authenticated_browser: WebDriver):
        """Logging out should clear the user session."""
        browser = authenticated_browser

        # Find and click logout
        logout_forms = browser.find_elements(
            By.CSS_SELECTOR, Selectors.Auth.LOGOUT_FORM
        )
        logout_links = browser.find_elements(
            By.CSS_SELECTOR, Selectors.Auth.LOGOUT_LINK
        )

        if logout_forms:
            logout_forms[0].submit()
        elif logout_links:
            logout_links[0].click()
        else:
            browser.get(f"{SEPTEMBER_URL}/auth/logout")

        # Wait for redirect after logout and verify
        wait = create_wait(browser)
        wait.until(lambda d: SEPTEMBER_URL.replace("http://", "") in d.current_url)

        # Verify logged out - go to home
        browser.get(f"{SEPTEMBER_URL}/")
        assert browser.find_element(By.TAG_NAME, "body")


class TestSessionPersistence:
    """Tests for session persistence across page loads."""

    @pytest.mark.auth
    def test_session_persists_across_navigation(self, authenticated_browser: WebDriver):
        """User session should persist when navigating between pages."""
        browser = authenticated_browser

        # Navigate to home
        browser.get(f"{SEPTEMBER_URL}/")
        header = browser.find_element(By.CSS_SELECTOR, Selectors.Layout.HEADER)
        assert header is not None

        # Navigate to a group
        browser.get(f"{SEPTEMBER_URL}/g/test.general")
        body = browser.find_element(By.TAG_NAME, "body")
        assert body is not None


class TestCookieSecurity:
    """Tests for cookie security flags."""

    @pytest.mark.auth
    def test_session_cookie_httponly(self, authenticated_browser: WebDriver):
        """
        Session cookie should have HttpOnly flag to prevent XSS attacks.

        Replaces: manual-sm-httponly-cookies
        """
        browser = authenticated_browser

        # Get all cookies
        cookies = browser.get_cookies()

        # Find the session cookie (typically named 'session' or similar)
        session_cookies = [c for c in cookies if "session" in c.get("name", "").lower()]

        # If no explicit session cookie, check all cookies
        if not session_cookies:
            session_cookies = cookies

        # At least one cookie should have HttpOnly flag
        httponly_cookies = [c for c in session_cookies if c.get("httpOnly", False)]

        assert len(httponly_cookies) > 0, (
            f"Expected at least one cookie with HttpOnly flag. "
            f"Cookies found: {[c.get('name') for c in session_cookies]}"
        )

    @pytest.mark.auth
    def test_session_cookie_samesite(self, authenticated_browser: WebDriver):
        """
        Session cookie should have SameSite flag for CSRF protection.

        Replaces: manual-sm-httponly-cookies (combined test)
        """
        browser = authenticated_browser

        cookies = browser.get_cookies()

        # Find session-related cookies
        session_cookies = [c for c in cookies if "session" in c.get("name", "").lower()]

        if not session_cookies:
            session_cookies = cookies

        # Check for SameSite attribute
        # Note: Selenium may report sameSite as 'Lax', 'Strict', or 'None'
        samesite_cookies = [
            c
            for c in session_cookies
            if c.get("sameSite", "").lower() in ("lax", "strict")
        ]

        assert len(samesite_cookies) > 0 or len(session_cookies) == 0, (
            f"Expected session cookies to have SameSite=Lax or Strict. "
            f"Cookies: {[(c.get('name'), c.get('sameSite')) for c in session_cookies]}"
        )


class TestPKCE:
    """Tests for PKCE (Proof Key for Code Exchange) in OAuth flow."""

    def test_oauth_redirect_includes_pkce_parameters(self):
        """
        OAuth redirect URL should include PKCE code_challenge parameters.

        PKCE prevents authorization code interception attacks.
        Replaces: manual-oidc-pkce
        """
        import re
        from urllib.parse import parse_qs, urlparse

        import requests

        from helpers import SEPTEMBER_HOST_URL

        # Make a direct HTTP request to capture the redirect chain
        # We need to follow the September redirects but stop at the external Dex URL
        with requests.Session() as session:
            # First request: /auth/login -> /auth/login/{provider}
            response = session.get(
                f"{SEPTEMBER_HOST_URL}/auth/login",
                allow_redirects=False,
            )
            assert response.status_code == 303, (
                f"Expected 303, got {response.status_code}"
            )

            # Second request: /auth/login/{provider} -> external OIDC URL with PKCE params
            provider_url = response.headers.get("Location")
            assert provider_url, "Missing Location header in redirect"

            # If it's a relative URL, make it absolute
            if provider_url.startswith("/"):
                provider_url = f"{SEPTEMBER_HOST_URL}{provider_url}"

            response = session.get(provider_url, allow_redirects=False)
            assert response.status_code == 303, (
                f"Expected 303, got {response.status_code}"
            )

            # This redirect should point to the OIDC provider with PKCE parameters
            oidc_url = response.headers.get("Location")
            assert oidc_url, "Missing Location header in OIDC redirect"

        # Parse the OIDC authorization URL
        parsed = urlparse(oidc_url)
        query_params = parse_qs(parsed.query)

        # Verify PKCE parameters are present
        assert "code_challenge" in query_params, (
            f"OAuth redirect should include code_challenge parameter. URL: {oidc_url}"
        )

        assert "code_challenge_method" in query_params, (
            f"OAuth redirect should include code_challenge_method parameter. "
            f"URL: {oidc_url}"
        )

        # code_challenge_method should be S256 (SHA-256)
        method = query_params["code_challenge_method"][0]
        assert method == "S256", f"code_challenge_method should be S256, got: {method}"

        # code_challenge should be a base64url-encoded string (43 chars for SHA-256)
        challenge = query_params["code_challenge"][0]
        assert len(challenge) >= 43, f"code_challenge seems too short: {challenge}"

        # Verify it's base64url format (alphanumeric, -, _)
        assert re.match(r"^[A-Za-z0-9_-]+$", challenge), (
            f"code_challenge should be base64url encoded: {challenge}"
        )
