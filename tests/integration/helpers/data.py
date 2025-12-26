"""Test data constants and configuration."""

import os

# Service URLs - use Docker service names when running in container,
# localhost when running tests from host
SELENIUM_URL = os.environ.get("SELENIUM_URL", "http://localhost:4445/wd/hub")
SEPTEMBER_URL = os.environ.get("SEPTEMBER_URL", "http://september:3000")
DEX_URL = os.environ.get("DEX_URL", "http://dex:5556")

# Test user credentials (matches dex.yaml staticPasswords)
TEST_USER_EMAIL = "testuser@example.com"
TEST_USER_PASSWORD = "password"  # bcrypt hash in dex.yaml is for "password"
TEST_USER_NAME = "testuser"

# NNTP credentials (matches renews admin add-user in docker-compose.yml)
NNTP_USERNAME = "testposter"
NNTP_PASSWORD = "testpassword"

# Test groups (seeded by seed_nntp.py)
TEST_GROUPS = [
    "test.general",
    "test.development",
    "test.announce",
]

# Services for log capture
LOG_SERVICES = ["september", "nntp", "dex"]
