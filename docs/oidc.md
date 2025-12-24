# OpenID Connect (OIDC) Authentication

September supports optional user authentication via OpenID Connect (OIDC) and OAuth2 providers. Authentication is stateless - session data is stored in signed cookies.

## Configuration

Add an `[oidc]` section to your configuration file to enable authentication:

```toml
[oidc]
# Secret for signing session cookies (required)
# Supports: env:VAR_NAME, file:/path, or literal value
# Recommended: 64+ characters, stored in environment or secret file
cookie_secret = "env:OIDC_COOKIE_SECRET"

# Session lifetime in days (default: 30)
session_lifetime_days = 30

# Optional: Override auto-detected redirect URI base
# If not set, extracted from request Host header
# redirect_uri_base = "https://news.example.com"
```

### Providers

Add one or more `[[oidc.provider]]` sections. September supports two configuration modes:

#### Discovery Mode (Recommended for OIDC providers)

For providers that support OIDC discovery (Google, Keycloak, Authentik, Auth0, etc.):

```toml
[[oidc.provider]]
name = "google"                              # URL-safe identifier (alphanumeric, -, _)
display_name = "Google"                      # Shown on login page
issuer_url = "https://accounts.google.com"   # OIDC issuer URL
client_id = "your-client-id.apps.googleusercontent.com"
client_secret = "env:GOOGLE_CLIENT_SECRET"   # Supports env:, file:, or literal
```

Endpoints are automatically discovered from `{issuer_url}/.well-known/openid-configuration`.

#### Manual Mode (For OAuth2-only providers)

For providers like GitHub that don't support OIDC discovery:

```toml
[[oidc.provider]]
name = "github"
display_name = "GitHub"
auth_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
userinfo_url = "https://api.github.com/user"
userinfo_sub_field = "id"                    # GitHub uses "id" instead of "sub"
client_id = "your-github-client-id"
client_secret = "env:GITHUB_CLIENT_SECRET"
```

## Secret Management

The `cookie_secret` and `client_secret` fields support three formats:

| Format | Example | Description |
|--------|---------|-------------|
| `env:VAR_NAME` | `env:OIDC_COOKIE_SECRET` | Read from environment variable |
| `file:/path` | `file:/run/secrets/cookie_secret` | Read from file (trimmed) |
| literal | `my-secret-value` | Use value directly (not recommended for production) |

**Security recommendations:**
- Use `env:` or `file:` in production
- Cookie secret should be at least 64 characters
- Never commit secrets to version control

## Provider Setup

### Google

1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a new project or select existing
3. Navigate to **APIs & Services** > **Credentials**
4. Click **Create Credentials** > **OAuth client ID**
5. Application type: **Web application**
6. Add authorized redirect URI: `https://your-domain.com/auth/callback/google`
7. Copy the Client ID and Client Secret

```toml
[[oidc.provider]]
name = "google"
display_name = "Google"
issuer_url = "https://accounts.google.com"
client_id = "xxxxx.apps.googleusercontent.com"
client_secret = "env:GOOGLE_CLIENT_SECRET"
```

### GitHub

1. Go to [GitHub Developer Settings](https://github.com/settings/developers)
2. Click **New OAuth App**
3. Set Homepage URL to your site
4. Set Authorization callback URL: `https://your-domain.com/auth/callback/github`
5. Copy the Client ID and generate a Client Secret

```toml
[[oidc.provider]]
name = "github"
display_name = "GitHub"
auth_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
userinfo_url = "https://api.github.com/user"
userinfo_sub_field = "id"
client_id = "your-github-client-id"
client_secret = "env:GITHUB_CLIENT_SECRET"
```

### Keycloak

1. Create a new Client in your realm
2. Set Access Type to **confidential**
3. Add valid redirect URI: `https://your-domain.com/auth/callback/keycloak`
4. Copy the Client ID and Client Secret from the Credentials tab

```toml
[[oidc.provider]]
name = "keycloak"
display_name = "Keycloak"
issuer_url = "https://keycloak.example.com/realms/your-realm"
client_id = "september"
client_secret = "env:KEYCLOAK_CLIENT_SECRET"
```

### Authentik

1. Create a new OAuth2/OIDC Provider
2. Set redirect URI: `https://your-domain.com/auth/callback/authentik`
3. Create an Application linked to the provider
4. Copy the Client ID and Client Secret

```toml
[[oidc.provider]]
name = "authentik"
display_name = "Authentik"
issuer_url = "https://authentik.example.com/application/o/september/"
client_id = "your-client-id"
client_secret = "env:AUTHENTIK_CLIENT_SECRET"
```

## Routes

When OIDC is configured, the following routes are available:

| Route | Method | Description |
|-------|--------|-------------|
| `/auth/login` | GET | Provider selection page (redirects directly if only one provider) |
| `/auth/login/{provider}` | GET | Initiate login with specific provider |
| `/auth/callback/{provider}` | GET | OAuth2 callback handler |
| `/auth/logout` | POST | Clear session and redirect to home |

## UI Integration

When OIDC is configured:
- A **Login** link appears in the header for unauthenticated users
- Authenticated users see their display name and a **Logout** button
- When no OIDC is configured, no authentication UI is shown

## Session Behavior

- Sessions are stored in signed, HTTP-only cookies
- Cookie signing key is derived from `cookie_secret` using HKDF
- Sessions expire after `session_lifetime_days` (default: 30 days)
- Authentication flow uses PKCE for security
- CSRF protection via state parameter

## Multiple Providers

You can configure multiple providers. When more than one provider is configured, users see a selection page at `/auth/login`. With only one provider, users are redirected directly to that provider.

```toml
[[oidc.provider]]
name = "google"
display_name = "Google"
issuer_url = "https://accounts.google.com"
client_id = "..."
client_secret = "env:GOOGLE_CLIENT_SECRET"

[[oidc.provider]]
name = "github"
display_name = "GitHub"
auth_url = "https://github.com/login/oauth/authorize"
token_url = "https://github.com/login/oauth/access_token"
userinfo_url = "https://api.github.com/user"
userinfo_sub_field = "id"
client_id = "..."
client_secret = "env:GITHUB_CLIENT_SECRET"
```

## Troubleshooting

### "Authentication is not configured"
The `[oidc]` section is missing or has no providers. Check your configuration file.

### "Provider not found"
The provider name in the URL doesn't match any configured provider. Provider names are case-sensitive.

### "Authentication flow expired"
The OAuth2 flow took longer than 10 minutes. Try logging in again.

### "Discovery failed"
The issuer URL is unreachable or doesn't provide valid OIDC metadata. Check the URL and network connectivity.

### Cookie issues
- Ensure your site uses HTTPS in production (SameSite=Lax requires secure context for cross-site flows)
- Check that `cookie_secret` is correctly resolved
- Verify the cookie isn't being blocked by browser settings
