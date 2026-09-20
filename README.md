# MF-Blog

MF-Blog is a filesystem-first publishing engine written in **Rust** with **Axum**.

Each article is a portable directory containing Markdown, YAML front matter, and optional local assets:

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

- Rust 1.88 or newer
- A modern browser
- Chromium-based browser recommended for folder upload support

## Quick start

```bash
cargo run --release
```

Open:

- Public site: <http://127.0.0.1:8000/>
- Writer: <http://127.0.0.1:8000/writer/>

The first Writer visit creates the single local writer account. TOTP support is available but disabled by default.

Run verification:

```bash
cargo check --all-targets --all-features
cargo test --all
```

## Docker Compose

```bash
docker compose up --build
```

Open:

- Public site: <http://127.0.0.1:8000/>
- Writer: <http://127.0.0.1:8000/writer/>

The named `mf-blog-state` volume retains the writer account, session-signing key, and recoverable trash. Article directories remain in the host `articles/` directory.

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

## Writer capabilities

- Create and edit folder-based articles.
- Live, sanitized Markdown preview.
- Draft, scheduled, and published states.
- Publish, schedule, and unpublish actions.
- Upload article assets and insert their Markdown paths.
- Import a complete article folder or ZIP archive.
- Move deleted articles into `.mf-blog-trash/` for manual recovery.
- Argon2id password hashing.
- Signed, HttpOnly, SameSite=Strict sessions.
- Optional TOTP with replay protection.
- CSRF protection on Writer mutations.
- Loopback-only Writer access by default.

## Configuration

Environment variables:

| Variable | Default |
| --- | --- |
| `BLOG_BIND` | `127.0.0.1:8000` |
| `BLOG_ARTICLE_ROOT` | `./articles` |
| `BLOG_AUTH_FILE` | `./.mf-blog-auth.json` |
| `BLOG_SESSION_KEY_FILE` | `./.mf-blog-session-key` |
| `BLOG_TRASH_ROOT` | sibling `.mf-blog-trash` |
| `BLOG_SITE_FILE` | `./site.yaml` |
| `BLOG_TIMEZONE` | `Asia/Makassar` |
| `BLOG_SECRET_KEY` | generated persistent key |
| `BLOG_SECURE_COOKIES` | `0` |
| `WRITER_ALLOW_REMOTE` | `0` |
| `WRITER_TOTP_ENABLED` | `0` |
| `WRITER_TOTP_SECRET` | empty |

## Customization

1. Edit `site.yaml` for site identity and homepage text.
2. Edit `static/theme.css` for visual overrides.
3. Edit Tera templates only when semantic page structure must change.

## Repository map

```text
src/main.rs                Process entrypoint and graceful shutdown
src/app.rs                 App state, routing, security headers
src/content.rs             Filesystem content engine + Markdown rendering
src/auth.rs                Argon2id, signed sessions, TOTP
src/handlers.rs            Public and Writer HTTP handlers
articles/                  Article directories and local assets
templates/                 Tera page structure
static/                    CSS + Writer JavaScript
site.yaml                  Site identity and homepage text
docs/                      Manuals and examples
Dockerfile                 Multi-stage Rust container image
compose.yaml               Local Docker Compose configuration
```

## Design goal

MF-Blog deliberately keeps the boring thing boring: an article is a folder, its content is a Markdown file, its state is front matter, and backups are ordinary filesystem backups.
