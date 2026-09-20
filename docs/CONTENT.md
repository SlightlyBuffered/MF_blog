# Content and article format

## Directory contract

Every article is one direct child of `articles/`. The directory name is the public slug.

```text
articles/
└── reliable-slug/
    ├── article.md
    └── assets/
        ├── hero.webp
        ├── architecture.svg
        └── clip.webm
```

Slugs must contain lowercase ASCII letters, numbers, and single hyphen-separated segments. Examples:

- Valid: `folder-cms`, `release-2026`, `notes`
- Invalid: `Folder CMS`, `../escape`, `double--hyphen`, `_private`

`article.md` is mandatory. `assets/` is optional until the article references a local asset.

## Front matter

An article starts with YAML bounded by `---` lines.

```yaml
---
title: The Folder Is the CMS
subtitle: Portable writing with a simple content structure.
hero: assets/hero.webp
publication_date: 2026-09-20T08:00:00+08:00
release_date: 2026-09-23T09:30:00+08:00
draft: false
---
```

| Field | Required | Type | Meaning |
| --- | --- | --- | --- |
| `title` | Yes | Non-empty string | Card, article heading, and browser title. |
| `subtitle` | No | String | Card deck and article introduction. Defaults to empty. |
| `hero` | No | String | Article-relative asset path or an absolute HTTPS URL. |
| `publication_date` | Yes | Date or datetime | Editorial date and listing sort key. |
| `release_date` | No | Date or datetime | Visibility gate. Defaults to `publication_date`. |
| `draft` | No | Boolean | `true` makes the article private. Defaults to `false`. |

Legacy aliases `publishedAt` and `releaseAt` are accepted for imported early prototypes. New content should use the snake-case names.

YAML booleans must not be quoted:

```yaml
draft: false     # boolean, correct
draft: "false"   # string, rejected rather than guessed
```

### Dates and timezone

Accepted forms include:

```yaml
publication_date: 2026-09-20
release_date: 2026-09-20T09:00:00+08:00
release_date: 2026-09-20T01:00:00Z
```

A date without a time means midnight. A datetime without a UTC offset uses `BLOG_TIMEZONE`, which defaults to `Asia/Makassar`. Include an explicit offset when an article is meant to move between servers without reinterpretation.

`publication_date` does not control visibility. It controls the displayed date and descending homepage order. `release_date` controls when a published article becomes public.

## Publication lifecycle

| `draft` | Release time | State | Homepage | Article route | Asset routes |
| --- | --- | --- | --- | --- | --- |
| `true` | Any | Draft | Hidden | 404 | 404 unless authenticated Writer |
| `false` | Future | Scheduled | Hidden | 404 | 404 unless authenticated Writer |
| `false` | Elapsed | Published | Visible | 200 | Public |

The Writer UI represents `draft` with a **Published** checkbox and matching toolbar action:

- Unchecked writes `draft: true`.
- Checked writes `draft: false`.
- A checked article remains scheduled until its release time arrives.
- The toolbar shows **Publish**, **Schedule**, or **Unpublish** according to the current state and release time.

## Markdown behavior

Rendering uses Python-Markdown with these extensions:

- `extra`: tables, fenced code, footnotes, definition lists, and common extras.
- `sane_lists`: predictable ordered/unordered lists.
- `smarty`: typographic punctuation.

Rendered HTML is sanitized with Bleach. Scripts, event attributes, iframes, objects, and arbitrary raw HTML are not retained. This is intentional even for locally authored content: imported folders are an input boundary.

Supported prose includes headings, lists, links, images, blockquotes, code, tables, deletion text, and horizontal rules.

## Links and assets

Markdown images with a relative path are scoped to the article:

```md
![Diagram](assets/diagram.webp)
```

becomes:

```text
/articles/reliable-slug/assets/diagram.webp
```

Links explicitly pointing into `assets/` receive the same rewrite:

```md
[Download the clip](assets/clip.webm)
```

Ordinary relative links are preserved:

```md
[Related notes](related-notes)
```

Root-relative URLs, fragments, mail links, and HTTPS URLs remain unchanged. Unsafe paths containing `..` are neutralized.

## Allowed imported assets

Writer imports and uploads accept:

```text
.png .jpg .jpeg .webp .gif .svg .mp4 .webm
```

Limits:

- 100 archive members or uploaded files per operation.
- 15 MiB per file.
- 50 MiB total request size.
- 50 MiB maximum expanded ZIP size.
- One and only one `article.md` per imported bundle.
- No symlinks, nested archives, path traversal, executables, or files outside the article directory.

SVG is parsed before acceptance. Script-capable tags, event handlers, inline styles, and external references are rejected.

These checks establish structural safety, not media authenticity. A `.png` extension is not decoded to prove the bytes form a valid PNG; browsers will simply fail to display malformed media.

## Portable bundle example

See [`docs/examples/article-bundle`](examples/article-bundle). Zip the containing directory—not only its contents—when you want the folder name to become the slug.

```powershell
Compress-Archive -Path docs\examples\article-bundle -DestinationPath article-bundle.zip
```

The Writer importer rejects a slug collision rather than overwriting an existing article.
