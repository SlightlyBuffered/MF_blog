# Security model

MF-Blog is a single-writer personal publishing engine. Its security model is intentionally narrower than that of a multi-tenant CMS.

## Trust boundaries

- Public visitors may read released articles and their assets.
- The authenticated writer may read and mutate all article folders.
- Imported folders and ZIPs are untrusted until validated.
- Markdown and raw HTML are untrusted until sanitized.
- The host filesystem and process environment are trusted administrative surfaces.

## Authentication

- One local username.
- Argon2id password hash in `.mf-blog-auth.json`.
- No default credentials, registration, email reset, or password recovery service.
- Eight-hour signed Flask sessions.
- Session cookie is HTTP-only and SameSite Strict.
- Five failed attempts per remote address trigger a 15-minute in-memory pause.

The rate limiter is process-local. Multiple production workers do not share counters, and restarting clears them. Put an additional rate limiter at the reverse proxy when exposing Writer.

## Network policy

By default Writer requires both:

- A loopback client address.
- A loopback Host such as `127.0.0.1` or `localhost`.

This second check avoids accidentally making Writer reachable merely because a local reverse proxy connects to Flask from loopback.

Remote Writer access requires:

```powershell
$env:WRITER_ALLOW_REMOTE = "1"
```

Do not enable it without HTTPS and a correctly configured proxy.

## TOTP

TOTP is implemented but disabled by default:

```powershell
$env:WRITER_TOTP_ENABLED = "1"
$env:WRITER_TOTP_SECRET = "BASE32SECRET"
```

Generate a secret interactively:

```powershell
.venv\Scripts\python
```

```python
import pyotp
pyotp.random_base32()
```

Enroll the returned Base32 value manually in an authenticator. MF-Blog accepts the current 30-second step plus one adjacent step on either side. A step already accepted for that username cannot be reused during the current server process.

TOTP replay state is in memory and resets with the process. TOTP is a second factor, not a substitute for HTTPS.

## CSRF and browser controls

- Mutating Writer API calls require a session-bound `X-CSRF-Token`.
- Setup and login forms require a session-bound hidden token.
- Writer responses use `Cache-Control: no-store`.
- CSP permits local scripts/styles, HTTPS images, and no plugins/frames.
- `X-Frame-Options: DENY` blocks clickjacking.
- `X-Content-Type-Options: nosniff` reduces MIME confusion.
- Camera, microphone, and geolocation permissions are disabled.

## Content and upload controls

- YAML uses `safe_load`.
- Rendered Markdown passes through Bleach.
- Relative asset paths reject traversal.
- `send_from_directory` confines asset responses.
- ZIP paths, symlinks, expanded size, member count, and per-file size are checked.
- Only a small media extension allowlist is accepted.
- SVG active content and external references are rejected.
- Imports are staged before entering `articles/`.
- Existing slugs and existing asset paths are not overwritten.
- Draft/scheduled assets return 404 to anonymous requests.

## Secrets and local state

| File/value | Purpose | Backup? | Commit? |
| --- | --- | --- | --- |
| `.mf-blog-auth.json` | Username and Argon2id hash | Yes | No |
| `.mf-blog-session-key` | Cookie signing secret | Yes | No |
| `BLOG_SECRET_KEY` | Optional environment replacement for signing key | Yes | No |
| `WRITER_TOTP_SECRET` | TOTP seed | Yes, securely | No |
| `.mf-blog-trash/` | Recoverable deleted content | According to policy | Usually no |

If `.mf-blog-session-key` is exposed, replace it and restart; all sessions become invalid. If the password hash is exposed, choose a new passphrase. If the TOTP seed is exposed, rotate and re-enroll it.

## Production minimum

For remote Writer access:

1. HTTPS at the edge.
2. `BLOG_SECURE_COOKIES=1`.
3. Persistent, high-entropy `BLOG_SECRET_KEY` shared by all workers.
4. `WRITER_ALLOW_REMOTE=1` only after the above.
5. TOTP strongly recommended.
6. Proxy-level request and login rate limiting.
7. Backups for articles and local auth state.
8. Never run Flask debug mode publicly.

MF-Blog has not undergone an independent security audit and should not be represented as a hardened multi-user CMS.
