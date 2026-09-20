# Development and deployment

## Supported runtime

- Python 3.11+
- Windows, Linux, or macOS
- A modern browser; Chromium-family browsers provide the strongest directory-upload support

## Development

```powershell
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[dev]"
.venv\Scripts\python app.py
```

`python app.py` binds to loopback and runs without debug mode by default. Set `FLASK_DEBUG=1` only for local development.

For a non-reloading local process:

```powershell
.venv\Scripts\python -m flask --app app run --host 127.0.0.1 --port 5000
```

## Docker Compose

The repository includes a production-style `Dockerfile` and a local-only `compose.yaml`:

```powershell
docker compose up --build
```

The service listens at <http://127.0.0.1:8000/>. Gunicorn runs one worker with four threads so the built-in login limiter and TOTP replay state remain consistent within the container.

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

`WRITER_ALLOW_REMOTE=1` is set because host requests cross the container network and are not loopback from Flask's perspective. The example publishes port 8000 only on `127.0.0.1`, so Writer remains local to the host. Do not change the port mapping to `8000:8000` without adding HTTPS, secure cookies, and the reverse-proxy controls described below.

Useful commands:

```powershell
docker compose logs -f mf-blog
docker compose restart mf-blog
docker compose down
```

After changing Python dependencies or the Dockerfile, run `docker compose up --build` again. Changes to `articles/`, `site.yaml`, and `static/theme.css` are visible through their mounts without rebuilding the image.

## Configuration variables

MF-Blog reads environment variables directly; it does not automatically load a `.env` file.

| Variable | Default | Purpose |
| --- | --- | --- |
| `BLOG_ARTICLE_ROOT` | `articles/` | Article directory root. |
| `BLOG_TIMEZONE` | `Asia/Makassar` | Timezone for naive article dates. |
| `BLOG_AUTH_FILE` | `.mf-blog-auth.json` | Authentication record path. |
| `BLOG_SESSION_KEY_FILE` | `.mf-blog-session-key` | Generated persistent signing-key path. |
| `BLOG_TRASH_ROOT` | `.mf-blog-trash/` beside the article root | Recoverable deletion directory. |
| `BLOG_SECRET_KEY` | Generated/persisted locally | Explicit Flask signing key, useful across workers. |
| `BLOG_SITE_FILE` | `site.yaml` | Site identity configuration path. |
| `BLOG_SECURE_COOKIES` | `0` | Set `1` when all Writer traffic uses HTTPS. |
| `WRITER_ALLOW_REMOTE` | `0` | Explicitly permit non-loopback Writer access. |
| `WRITER_TOTP_ENABLED` | `0` | Require TOTP during login. |
| `WRITER_TOTP_SECRET` | Empty | Base32 seed when TOTP is enabled. |
| `FLASK_DEBUG` | `0` | Enable Flask debug mode for local development only. |

Truthy flag values are `1`, `true`, `yes`, and `on`, case-insensitively.

## Production WSGI example

The Flask development server is not a production server. On Windows, Waitress is a straightforward option:

```powershell
.venv\Scripts\python -m pip install waitress
.venv\Scripts\waitress-serve --listen=127.0.0.1:8000 app:app
```

On Unix-like hosts, Gunicorn is common:

```bash
python -m pip install gunicorn
gunicorn --bind 127.0.0.1:8000 --workers 2 app:app
```

With multiple workers, set the same `BLOG_SECRET_KEY` in every process. The built-in login rate limiter and TOTP replay cache are process-local; enforce rate limits at the reverse proxy as well.

## Reverse proxy outline

Terminate HTTPS at Caddy, nginx, Apache, IIS, or another trusted proxy and forward to the WSGI server on loopback.

For public reading with local-only Writer, do not proxy `/writer`. This is the simplest arrangement.

For intentionally remote Writer:

```powershell
$env:WRITER_ALLOW_REMOTE = "1"
$env:BLOG_SECURE_COOKIES = "1"
$env:BLOG_SECRET_KEY = "<persistent-high-entropy-secret>"
$env:WRITER_TOTP_ENABLED = "1"
$env:WRITER_TOTP_SECRET = "<base32-secret>"
```

Then proxy Writer only over HTTPS. Add edge rate limiting and request-size enforcement consistent with the application's 50 MiB limit.

## Backups

Minimum useful backup:

```text
articles/
site.yaml
static/theme.css
.mf-blog-auth.json
.mf-blog-session-key
```

Include `.mf-blog-trash/` if deleted articles must remain recoverable. Templates and core static files belong in source control.

## Updating

1. Back up content and local state.
2. Preserve `site.yaml` and `static/theme.css`.
3. Install updated dependencies.
4. Run Ruff and pytest.
5. Start on a non-public port and inspect home, article, login, Writer, upload, and save flows.
6. Switch the proxy only after verification.

## Health verification

There is no dedicated health endpoint yet. A lightweight public check is:

```powershell
Invoke-WebRequest http://127.0.0.1:5000/ -UseBasicParsing
```

Expect HTTP 200. Do not use a Writer endpoint for anonymous health checks because authentication redirects are correct behavior.
