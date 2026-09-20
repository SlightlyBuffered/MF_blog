# MF-Blog

MF-Blog is a filesystem-first publishing engine built with Flask. Each article is a portable directory containing Markdown, YAML front matter, and optional local assets:

```text
articles/
└── my-article/
    ├── article.md
    └── assets/
        ├── hero.webp
        └── diagram.svg
```

There is no content database. Article discovery, publication state, rendering, and asset delivery are derived directly from the filesystem. The authenticated Writer interface provides article creation, editing, preview, scheduling, publishing, asset upload, bundle import, and recoverable deletion.

## Requirements

- Python 3.11 or newer
- A modern browser
- Chromium-based browser recommended for folder upload support

## Quick start

```powershell
py -m venv .venv
.venv\Scripts\python -m pip install -e ".[dev]"
.venv\Scripts\python app.py
```

Open:

- Public site: <http://127.0.0.1:5000/>
- Writer: <http://127.0.0.1:5000/writer/>

The first Writer visit creates the single local writer account. TOTP support is available but disabled by default.

Run verification:

```powershell
.venv\Scripts\python -m ruff check .
.venv\Scripts\python -m pytest -q
node --check static\writer.js
```

## Quick run with Docker Compose

Docker Compose is the recommended container workflow:

```powershell
docker compose up --build
```

Open:

- Public site: <http://127.0.0.1:8000/>
- Writer: <http://127.0.0.1:8000/writer/>

Stop the service with `docker compose down`. The named `mf-blog-state` volume retains the writer account, session-signing key, and recoverable trash. Article directories remain in the host `articles/` directory. Do not use `docker compose down -v` unless the container-managed state should be deleted.

The included [`compose.yaml`](compose.yaml) is equivalent to:

```yaml
services:
  mf-blog:
    build:
      context: .
      args:
        APP_UID: ${MF_BLOG_UID:-1000}
        APP_GID: ${MF_BLOG_GID:-1000}
    ports:
      - "127.0.0.1:8000:8000"
    environment:
      BLOG_ARTICLE_ROOT: /app/articles
      BLOG_AUTH_FILE: /data/.mf-blog-auth.json
      BLOG_SESSION_KEY_FILE: /data/.mf-blog-session-key
      BLOG_TRASH_ROOT: /data/.mf-blog-trash
      BLOG_SITE_FILE: /app/site.yaml
      WRITER_ALLOW_REMOTE: "1"
    volumes:
      - ./articles:/app/articles
      - ./site.yaml:/app/site.yaml:ro
      - ./static/theme.css:/app/static/theme.css:ro
      - mf-blog-state:/data
    restart: unless-stopped

volumes:
  mf-blog-state:
```

`WRITER_ALLOW_REMOTE=1` is required because requests arrive through the container network. The published port remains restricted to host loopback, so the example is local-only. Review the [deployment guide](docs/DEPLOYMENT.md) before exposing Writer through a reverse proxy.

On Linux systems where the current user is not UID/GID `1000`, build with matching IDs:

```bash
MF_BLOG_UID=$(id -u) MF_BLOG_GID=$(id -g) docker compose up --build
```

## Quick run with Docker

To run without Compose while keeping content and application state in named volumes:

```powershell
docker build -t mf-blog .
docker volume create mf-blog-content
docker volume create mf-blog-state
docker run --rm --name mf-blog `
  -p 127.0.0.1:8000:8000 `
  -e WRITER_ALLOW_REMOTE=1 `
  -e BLOG_AUTH_FILE=/data/.mf-blog-auth.json `
  -e BLOG_SESSION_KEY_FILE=/data/.mf-blog-session-key `
  -e BLOG_TRASH_ROOT=/data/.mf-blog-trash `
  -v mf-blog-content:/app/articles `
  -v mf-blog-state:/data `
  mf-blog
```

The image runs Gunicorn as an unprivileged user and includes a public homepage health check.

## Article format

```md
---
title: My article
subtitle: Optional summary
hero: assets/hero.webp
publication_date: 2026-09-20
release_date: 2026-09-20T09:00:00+08:00
draft: true
---

## Start writing

Article content is written in Markdown.

![Diagram](assets/diagram.webp)
```

Publication state is determined by `draft` and `release_date`:

- `draft: true`: private draft.
- `draft: false` with a future `release_date`: scheduled.
- `draft: false` with an elapsed `release_date`: published.

The Writer toolbar exposes **Publish**, **Schedule**, or **Unpublish** as appropriate. **Save article** preserves the currently selected publication state.

## Writer capabilities

- Create and edit folder-based articles.
- Live, sanitized Markdown preview.
- Collapsible metadata panel with a remembered browser preference.
- Undo and redo history scoped to the open article.
- Markdown formatting toolbar and keyboard shortcuts.
- Draft, scheduled, and published states.
- Explicit Publish, Schedule, and Unpublish actions.
- Upload article assets and insert their Markdown paths.
- Import a complete article folder or ZIP archive.
- Move deleted articles into `.mf-blog-trash/` for manual recovery.
- Responsive desktop and narrow-viewport layouts.

## Customization

MF-Blog separates content, site identity, core layout, and visual overrides:

1. Edit [`site.yaml`](site.yaml) for the site name, wordmark, tagline, language, and homepage text. It reloads on each rendered request.
2. Edit [`static/theme.css`](static/theme.css) for colors, typography, spacing, and component overrides. It loads after the core stylesheets.
3. Review the complete themes in [`docs/examples/themes`](docs/examples/themes).
4. Edit Jinja templates only when the semantic page structure must change.

Every surface exposes a stable `data-template` attribute, and the core styles use documented CSS custom properties.

## Documentation

- [Content and article format](docs/CONTENT.md)
- [Writer workstation](docs/WRITER.md)
- [Theming and UI customization](docs/CUSTOMIZATION.md)
- [Writer HTTP API](docs/API.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Security model](docs/SECURITY.md)
- [Development and deployment](docs/DEPLOYMENT.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)

## Repository map

```text
app.py                    Application engine, security, and routes
articles/                 Article directories and local assets
templates/                Jinja page structure
static/style.css          Shared public styles and design tokens
static/writer.css         Writer layout
static/auth.css           Setup and sign-in layout
static/theme.css          User-owned visual overrides
static/writer.js          Writer behavior
site.yaml                 Site identity and homepage text
tests/                    Isolated regression tests
docs/                     Manuals and examples
Dockerfile                Container image definition
compose.yaml              Local Docker Compose configuration
.dockerignore             Container build-context exclusions
```

## Local state

These generated paths are ignored by Git:

- `.mf-blog-auth.json`: username and Argon2id password hash.
- `.mf-blog-session-key`: persistent Flask session-signing key.
- `.mf-blog-trash/`: recoverable deleted article directories.

Back up the authentication and session-key files with the article directories when the deployment is important. Never commit them.
