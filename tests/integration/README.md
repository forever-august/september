# Integration Tests

This directory contains end-to-end integration tests for September using Docker Compose, Selenium, and pytest.

## Architecture

The test environment consists of:

- **Renews** (`nntp`): NNTP server built from [forever-august/renews](https://github.com/forever-august/renews)
- **Dex** (`dex`): OIDC provider from [dexidp/dex](https://github.com/dexidp/dex)
- **September** (`september`): The application under test
- **Chrome** (`chrome`): Selenium-controlled browser for UI testing
- **Seeder** (`seeder`): One-shot container to populate test data

All services run on a Docker internal network, with Chrome accessing September via `http://september:3000`.

## Prerequisites

- Docker and Docker Compose
- [uv](https://github.com/astral-sh/uv) (Python package manager)

## Quick Start

```bash
# From repository root
cd tests/integration

# Start the environment (builds, starts, and seeds)
./environment/setup.sh

# Run tests
uv run pytest -v

# View browser via VNC (for debugging)
# Open http://localhost:7900 in your browser (password: secret)

# Cleanup
./environment/teardown.sh
```

Alternatively, pytest can manage the Docker environment automatically:

```bash
# Run tests with automatic Docker setup/teardown
uv run pytest -v

# Skip Docker setup if environment is already running
SKIP_DOCKER_SETUP=1 uv run pytest -v
```

## Running Specific Tests

```bash
# Run only homepage tests
uv run pytest tests/integration/test_home.py -v

# Run only auth tests
uv run pytest tests/integration/test_auth.py -v

# Run tests matching a pattern
uv run pytest tests/integration -k "test_login" -v

# Run tests with a specific marker
uv run pytest tests/integration -m "auth" -v
uv run pytest tests/integration -m "posting" -v
```

## Test Markers

- `@pytest.mark.auth` - Tests that involve authentication
- `@pytest.mark.posting` - Tests that create NNTP posts
- `@pytest.mark.slow` - Tests that take longer to run

## Test Data

The seeder creates the following test data:

### Newsgroups
- `test.general` - General discussion (5 articles, 2 threads)
- `test.development` - Development topics (8 articles, 4 threads with replies)
- `test.announce` - Announcements (3 single-post threads)

### Test Users (Dex)
- Email: `testuser@example.com`, Password: `password`
- Email: `otheruser@example.com`, Password: `password`

## Configuration Files

All Docker and configuration files are in the `environment/` directory:

| File | Purpose |
|------|---------|
| `environment/docker-compose.yml` | Docker Compose service definitions |
| `environment/Dockerfile.september` | September Docker image |
| `environment/Dockerfile.renews` | Renews NNTP server Docker image |
| `environment/setup.sh` | Start environment script |
| `environment/teardown.sh` | Stop environment script |
| `environment/seed_nntp.py` | Test data seeder |
| `environment/config/renews.toml` | NNTP server configuration |
| `environment/config/dex.yaml` | OIDC provider configuration with static users |
| `environment/config/september.toml` | September configuration for test environment |

## Debugging

### View Browser Session

The Selenium Chrome container exposes a VNC server on port 7900:

1. Open http://localhost:7900 in your browser
2. Enter password: `secret`
3. Watch tests run in real-time

### View Service Logs

```bash
# From the environment directory
cd environment

# All services
docker compose logs -f

# Specific service
docker compose logs -f september
docker compose logs -f nntp
docker compose logs -f dex
```

### Run Single Test with Debug Output

```bash
uv run pytest tests/integration/test_home.py::TestHomepage::test_homepage_loads -v -s
```

### Rebuild Specific Service

```bash
cd environment
docker compose build september
docker compose up -d september
```

## CI/CD

Integration tests run in GitHub Actions on push and pull request. See `.github/workflows/integration.yml`.

The CI workflow:
1. Starts all Docker services
2. Waits for health checks
3. Seeds test data
4. Runs pytest
5. Collects logs on failure
6. Cleans up

## Troubleshooting

### Services won't start

Check if ports are already in use:
```bash
lsof -i :3000  # September
lsof -i :5556  # Dex
lsof -i :1190  # NNTP
lsof -i :4444  # Selenium
```

### Tests can't connect to Chrome

Ensure the Chrome container is healthy:
```bash
docker compose ps chrome
curl http://localhost:4444/status
```

### Authentication tests fail

Check Dex logs for OIDC errors:
```bash
docker compose logs dex
```

Verify Dex is accessible:
```bash
curl http://localhost:5556/dex/.well-known/openid-configuration
```

### NNTP connection issues

Check Renews logs:
```bash
docker compose logs nntp
```

Test NNTP connection:
```bash
nc localhost 1190
# Should see: 200 ... ready
```

### Resetting test environment

```bash
./environment/teardown.sh
./environment/setup.sh
```

## Writing New Tests

1. Create a new test file `test_*.py`
2. Import fixtures from `conftest.py`
3. Use `browser` fixture for basic Selenium access
4. Use `authenticated_browser` fixture for tests requiring login
5. Use `clean_browser` fixture for tests needing fresh state

Example:

```python
from selenium.webdriver.common.by import By
from selenium.webdriver.remote.webdriver import WebDriver

from conftest import SEPTEMBER_URL

def test_my_feature(browser: WebDriver):
    browser.get(f"{SEPTEMBER_URL}/my-page")
    assert browser.find_element(By.ID, "my-element")
```
