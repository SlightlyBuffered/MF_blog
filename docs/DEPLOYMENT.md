# Development and deployment

## Supported runtime

- Rust 1.88+
- Windows, Linux, or macOS
- A modern browser; Chromium-family browsers provide the strongest directory-upload support

## Development

```powershell
cargo run
```

By default MF-Blog binds to `127.0.0.1:8000`. Override the listener with `BLOG_BIND`:

```powershell
$env:BLOG_BIND = "127.0.0.1:8001"
cargo run
```

Run the verification suite with:

```powershell
cargo check --all-targets --all-features
cargo test --all
```

## Release build

```powershell
cargo build --release
target\release\mf-blog.exe
```

On Unix-like systems the binary is `target/release/mf-blog`.

## Docker Compose

The repository includes a multi-stage Rust `Dockerfile` and local-only `compose.yaml`:

```powershell
docker compose up --build
```

The service is published at <http://127.0.0.1:8000/>. The final image contains the compiled MF-Blog binary rather than a language runtime.

Storage is divided by responsibility:

| Container path | Source | Contents |
| --- | --- | --- |
| `/app/articles` | Host `./articles` bind mount | Canonical article directories and assets. |
| `/app/site.yaml` | Host file, read-only bind mount | Site identity and homepage text. |
| `/app/static/theme.css` | Host file, read-only bind mount | Visual overrides. |
| `/data` | Named `mf-blog-state` volume | Authentication record, signing key, and recoverable trash. |

The state volume survives `docker compose down`. Running `docker compose down -v` deletes that volume and therefore removes the container-managed writer credentials, signing key, and trash.

The container runs as UID/GID `1000` by default. On Linux, build with the current account IDs when the `articles/` bind mount is not writable:

```bash
MF_BLOG_UID=$(id -u) MF_BLOG_GID=$(id -g) docker compose up --build
```

`WRITER_ALLOW_REMOTE=1` is set inside Compose because requests from the host cross the container network and are not loopback from the service's perspective. The supplied port mapping remains restricted to host loopback. Do not expose `8000:8000` publicly without HTTPS, secure cookies, and edge rate limiting.

Useful commands:

```powershell
docker compose logs -f mf-blog
docker compose restart mf-blog
docker compose down
```

Changes to Rust source or the Dockerfile require a rebuild. Changes to `articles/`, `site.yaml`, and `static/theme.css` are visible through their mounts without rebuilding the image.

## Configuration variables

MF-Blog reads environment variables directly.

| Variable | Default | Purpose |
| --- | --- | --- |
| `BLOG_BIND` | `127.0.0.1:8000` | HTTP listen address. |
| `BLOG_ARTICLE_ROOT` | `articles/` | Article directory root. |
| `BLOG_TIMEZONE` | `Asia/Makassar` | Timezone for naive article dates. |
| `BLOG_AUTH_FILE` | `.mf-blog-auth.json` | Authentication record path. |
| `BLOG_SESSION_KEY_FILE` | `.mf-blog-session-key` | Generated persistent signing-key path. |
| `BLOG_TRASH_ROOT` | `.mf-blog-trash/` beside the article root | Recoverable deletion directory. |
| `BLOG_SECRET_KEY` | Generated/persisted locally | Explicit HMAC session-signing secret. |
| `BLOG_SITE_FILE` | `site.yaml` | Site identity configuration path. |
| `BLOG_SECURE_COOKIES` | `0` | Set `1` when all Writer traffic uses HTTPS. |
| `WRITER_ALLOW_REMOTE` | `0` | Explicitly permit non-loopback Writer access. |
| `WRITER_TOTP_ENABLED` | `0` | Require TOTP during login. |
| `WRITER_TOTP_SECRET` | Empty | Base32 seed when TOTP is enabled. |
| `MF_BLOG_DEBUG` | `0` | Show content discovery errors on the public homepage. |

Truthy flag values are `1`, `true`, `yes`, and `on`, case-insensitively.

## Reverse proxy outline

MF-Blog is already an HTTP server; no WSGI/ASGI process manager is involved. Bind it to loopback or a private interface and terminate HTTPS at Caddy, nginx, Apache, IIS, HAProxy, or another trusted proxy.

For public reading with local-only Writer, do not proxy `/writer`. This is the simplest arrangement.

For intentionally remote Writer:

```powershell
$env:WRITER_ALLOW_REMOTE = "1"
$env:BLOG_SECURE_COOKIES = "1"
$env:BLOG_SECRET_KEY = "<persistent-high-entropy-secret>"
$env:WRITER_TOTP_ENABLED = "1"
$env:WRITER_TOTP_SECRET = "<base32-secret>"
cargo run --release
```

Proxy Writer only over HTTPS. Add edge rate limiting and request-size enforcement consistent with the application's 50 MiB limit.

## Backups

Minimum useful backup:

```text
articles/
site.yaml
static/theme.css
.mf-blog-auth.json
.mf-blog-session-key
```

Include `.mf-blog-trash/` when deleted articles must remain recoverable. Templates and core static files belong in source control.

## Updating

1. Back up content and local state.
2. Preserve `site.yaml` and `static/theme.css`.
3. Pull the updated source.
4. Run `cargo check --all-targets --all-features` and `cargo test --all`.
5. Build a release binary or rebuild the container.
6. Start on a non-public port and inspect home, article, login, Writer, upload, import, and save flows.
7. Switch the proxy only after verification.

## Health verification

There is no dedicated health endpoint yet. A lightweight public check is:

```powershell
Invoke-WebRequest http://127.0.0.1:8000/ -UseBasicParsing
```

Expect HTTP 200. Do not use a Writer endpoint for anonymous health checks because authentication redirects are correct behavior.
