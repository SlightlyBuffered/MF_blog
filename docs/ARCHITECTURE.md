# Architecture

MF-Blog keeps the runtime intentionally compact. The Rust engine is split by responsibility across `src/`, while content, semantic templates, core styling, and user overrides remain separate.

## Runtime flow

### Public index

```text
GET /
  → scan articles/*/article.md
  → parse YAML safely
  → reject invalid articles into an error list
  → remove drafts and future releases
  → sort by publication_date descending
  → render templates/index.html
```

### Public article

```text
GET /articles/<slug>/
  → discover currently public articles
  → return 404 unless slug is public
  → convert Markdown to HTML
  → rewrite article asset URLs
  → sanitize HTML
  → render templates/article.html
```

### Writer save

```text
PUT /writer/api/articles/<slug>
  → verify writer session and CSRF header
  → validate JSON, slug, dates, and metadata
  → serialize canonical YAML + Markdown
  → write sibling temporary file
  → atomically replace article.md
```

### Import

```text
multipart upload
  → temporary staging directory
  → safe relative-path checks
  → ZIP/member/file limits
  → extension and SVG checks
  → parse staged article
  → reject existing slug
  → move complete folder into articles/
  → always remove staging directory
```

## Files and responsibilities

| Path | Responsibility | Expected customization |
| --- | --- | --- |
| `src/app.rs` | Application state, routing, security middleware | Engine development |
| `src/content.rs` | Parsing, rendering, content validation | Engine development |
| `src/auth.rs` | Argon2id, sessions, TOTP | Engine development |
| `src/handlers.rs` | Public and Writer request handlers | Engine development |
| `articles/` | User content | Constantly |
| `site.yaml` | Site identity and homepage text | Freely |
| `templates/` | Semantic HTML structure | Advanced themes/features |
| `static/style.css` | Shared tokens and public core styles | Engine-level changes |
| `static/writer.css` | Workstation core layout | Engine-level changes |
| `static/auth.css` | Login/setup core layout | Engine-level changes |
| `static/theme.css` | Last-loaded overrides | Freely |
| `static/writer.js` | Workstation behavior | Feature development |
| Rust `#[cfg(test)]` modules | Regression contract | With every behavior change |

## Important invariants

1. The article folder is the canonical content unit.
2. Public visibility is computed from `draft` and `release_date` on every request.
3. Invalid articles do not crash discovery; Writer reports them.
4. Draft and scheduled assets are unavailable without an authenticated Writer session.
5. Imported paths cannot escape staging or the article asset directory.
6. Public Markdown output is sanitized.
7. User theme overrides load after core styles.
8. Tests use temporary article roots and do not depend on real editorial state.

## Why there is no database

The filesystem already supplies namespaces, atomic file replacement, backup compatibility, Git diffs, editor interoperability, and portable bundles. Adding a database would create a synchronization problem without solving a current requirement.

Runtime caches are intentionally minimal. Article changes and `site.yaml` changes appear on the next request. This favors predictable behavior and immediate content updates over a more complex cache layer.

## Extension points

- Add metadata in `Article`, `load_article`, `build_article_source`, `article_payload`, and the Writer form together.
- Add Markdown behavior in `render_markdown`; update sanitizer allowlists at the same time.
- Add upload types through `ALLOWED_ASSET_EXTENSIONS`; also review serving, validation, CSP, and documentation.
- Add visual themes only through tokens/`theme.css` unless semantic markup must change.
- Add Writer routes through the shared authentication/CSRF guard when they mutate content.

When changing an invariant, add a regression test and keep the relevant trust boundary explicit.
