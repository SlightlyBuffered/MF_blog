# Writer HTTP API

The API is an implementation surface for the bundled Writer, not a versioned public service. It uses the authenticated Flask session cookie and requires `X-CSRF-Token` on every mutating request.

The token is emitted into the Writer page:

```html
<meta name="csrf-token" content="…">
```

Common responses:

| Status | Meaning |
| --- | --- |
| `200` | Read/update/delete succeeded. |
| `201` | Article/import created. |
| `400` | Validation or malformed input. |
| `401` | Writer authentication required. |
| `403` | Network policy or CSRF rejection. |
| `404` | Article not found. |
| `409` | Slug already exists. |
| `413` | Request exceeds 50 MiB. |

Errors use:

```json
{"error": "human-readable explanation"}
```

## Article representation

```json
{
  "slug": "folder-cms",
  "title": "The Folder Is the CMS",
  "subtitle": "Portable content.",
  "hero": "assets/hero.webp",
  "publication_date": "2026-09-20T08:00:00+08:00",
  "release_date": "2026-09-23T09:30:00+08:00",
  "draft": false,
  "published": true,
  "body": "## Start writing\n",
  "assets": ["diagram.svg", "hero.webp"],
  "url": "/articles/folder-cms/"
}
```

`published` means the publication switch is enabled (`not draft`). It does not imply the release time has elapsed.

## `GET /writer/api/articles`

Lists valid articles and content errors:

```json
{
  "articles": [],
  "errors": ["articles/broken/article.md: title is required"]
}
```

## `GET /writer/api/articles/<slug>`

Returns one valid article representation. Invalid or absent content returns 404.

## `POST /writer/api/articles`

Creates a new slug. JSON body:

```json
{
  "slug": "new-story",
  "title": "New Story",
  "subtitle": "",
  "hero": "",
  "publication_date": "2026-09-20T08:00:00+08:00",
  "release_date": "2026-09-20T08:00:00+08:00",
  "published": false,
  "body": "# Start writing\n"
}
```

Returns 409 instead of overwriting an existing directory.

## `PUT /writer/api/articles/<slug>`

Replaces the metadata and Markdown body using an atomic `article.md` replacement. The slug cannot be changed by this endpoint. Accepts `published` or the lower-level `draft`; if both are provided, `draft` is authoritative.

## `DELETE /writer/api/articles/<slug>`

Moves the complete article directory into `.mf-blog-trash/` and returns:

```json
{
  "deleted": "old-story",
  "trash_name": "old-story-20260920-120000"
}
```

## `POST /writer/api/preview`

Accepts the same editable fields as save and returns sanitized rendered Markdown:

```json
{"html": "<h1>Preview</h1>"}
```

It does not write files.

## `POST /writer/api/articles/<slug>/assets`

Multipart form with one or more `files` parts. Paths may begin with `assets/`; that prefix is normalized away. Existing asset paths are rejected rather than overwritten.

```json
{"saved": ["hero.webp", "charts/latency.png"]}
```

## `POST /writer/api/import`

Multipart form:

- `files`: one ZIP, or every file from a selected directory.
- `slug`: optional explicit destination slug.

The successful response is the imported article representation.

## `POST /writer/logout`

Clears the Writer session:

```json
{"ok": true}
```

## Public routes

| Route | Behavior |
| --- | --- |
| `GET /` | Currently public articles ordered by publication date. |
| `GET /articles/<slug>/` | Rendered public article or 404. |
| `GET /articles/<slug>/assets/<path>` | Public asset, or private asset for an authenticated Writer session. |
