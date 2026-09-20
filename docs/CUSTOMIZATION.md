# Theming and UI customization

MF-Blog separates content, semantic structure, core layout, and visual overrides. Use the highest customization layer that meets the requirement so upgrades remain straightforward.

## Layer 1: site identity and copy

Edit `site.yaml`:

```yaml
name: MF-Blog
wordmark: MF-Blog
tagline: Markdown publishing, organized by folders.
language: en

home:
  eyebrow: Latest articles
  title: |-
    Writing, published
    from Markdown.
  empty: No articles have been published yet.
```

The file is loaded on each rendered request, so changes do not require restarting the service.

| Field | Used for |
| --- | --- |
| `name` | Browser titles and general site name. |
| `wordmark` | Header brand text. |
| `tagline` | Header secondary text. |
| `language` | HTML `lang` attribute. |
| `home.eyebrow` | Small homepage label. |
| `home.title` | Large homepage heading; line breaks are preserved. |
| `home.empty` | Empty homepage message. |

Unknown fields are ignored. Known fields with an invalid type raise a configuration error.

## Layer 2: design tokens

Edit `static/theme.css`. It is linked after `style.css`, `writer.css`, and `auth.css`, so token overrides affect all surfaces.

```css
:root {
  --color-accent: #3155ff;
  --color-paper: #f7f8fc;
  --font-display: ui-rounded, system-ui, sans-serif;
  --content-width: 78rem;
  --card-image-ratio: 3 / 2;
}
```

### Color tokens

| Token | Default | Main consumers |
| --- | --- | --- |
| `--color-ink` | `#171713` | Text, borders, dark buttons. |
| `--color-paper` | `#f3f0e7` | Page and preview background. |
| `--color-accent` | `#ff4d00` | Brand dot, links, focus, primary actions. |
| `--color-line` | `#c9c4b8` | Dividers, fields, tables. |
| `--color-muted` | `#6b675e` | Metadata and secondary controls. |
| `--color-subtle` | `#565249` | Deck text and blockquotes. |
| `--color-panel` | `#eeeae0` | Editor metadata surfaces. |
| `--color-panel-strong` | `#dcd7ca` | Writer rail and asset dock. |
| `--color-workspace` | `#e9e5da` | Writer canvas. |
| `--color-code-bg` | `#24241f` | Code blocks. |
| `--color-code-ink` | `#f6f0df` | Code-block text. |
| `--color-inline-code` | `#e5e0d4` | Inline code background. |
| `--color-danger` | `#a23518` | Delete and error states. |
| `--color-success` | `#69945e` | Saved/auth-success states. |
| `--color-on-accent` | `#fff` | Text placed on accent surfaces. |
| `--color-on-danger` | `#fff` | Text placed on destructive surfaces. |
| `--color-error-bg` | `#ffe0d4` | Development content-error panel. |
| `--color-overlay` | translucent black | Dialog backdrop. |
| `--color-auth-bg` | `#171713` | Authentication page background. |
| `--color-auth-panel` | `#1d1d18` | Authentication card. |
| `--color-auth-line` | `#56534b` | Authentication borders. |
| `--color-auth-muted` | `#aaa69d` | Authentication secondary text. |

Legacy aliases `--ink`, `--paper`, `--accent`, and `--line` point to their semantic equivalents. New themes should override the `--color-*` tokens.

### Typography and geometry

| Token | Default | Meaning |
| --- | --- | --- |
| `--font-ui` | Inter/system sans stack | Navigation, controls, metadata. |
| `--font-display` | Georgia/serif stack | Large headings. |
| `--font-prose` | Georgia/serif stack | Article body. |
| `--font-mono` | System monospace stack | Code and Markdown editor. |
| `--content-width` | `70rem` | Header and homepage maximum width. |
| `--reader-width` | `51.25rem` | Article shell width. |
| `--prose-width` | `45rem` | Article body measure. |
| `--page-gutter` | `1.5rem` | Horizontal page padding. |
| `--card-image-ratio` | `16 / 10` | Homepage hero crop. |
| `--radius-small` | `.25rem` | Small code-block radius. |

Use system font stacks unless fonts are served locally. The content security policy rejects remote stylesheets, which includes most hosted-font CSS endpoints.

## Layer 3: targeted CSS overrides

Every page has a stable hook on `<body>`:

```html
<body data-template="home">
<body data-template="article">
<body data-template="writer">
<body data-template="writer-login">
<body data-template="writer-setup">
```

Examples:

```css
/* Only the public homepage */
[data-template="home"] .intro h1 {
  max-width: 12ch;
  text-transform: uppercase;
}

/* Only public articles */
[data-template="article"] .prose {
  font-size: 1.125rem;
}

/* Change writer proportions without touching JavaScript */
[data-template="writer"] .writer-shell {
  grid-template-columns: 22rem minmax(0, 1fr);
}

/* Simplify the sign-in card */
[data-template="writer-login"] .auth-number {
  display: none;
}
```

Useful component selectors:

| Selector | Component |
| --- | --- |
| `.site-header`, `.brand`, `.tagline` | Shared header |
| `.intro`, `.article-grid`, `.card` | Homepage |
| `.article-header`, `.hero`, `.prose` | Public reader |
| `.writer-sidebar`, `.metadata-grid`, `.metadata-toggle` | Writer navigation/metadata |
| `.markdown-pane`, `.preview-pane` | Writer editing split |
| `.asset-dock`, `.import-box`, `.publish-button` | Writer asset/import and publication controls |
| `.auth-card` | Setup and login card |

Class names are treated as customization hooks. A cleanup should not rename them casually.

The default Writer switches its article rail into an overlay drawer at `800px`, uses Write/Preview tabs, wraps toolbar actions below `700px`, and applies compact height rules below `650px`. Authentication pages compact below `820px` viewport height. Override those media rules in `theme.css` if a custom control density requires different breakpoints.

## Layer 4: template changes

Edit Tera templates when CSS cannot express the structural change.

`templates/base.html` provides these blocks:

- `title`
- `head`
- `body_class`
- `template_name`
- `main_class`
- `content`
- `scripts`

Public pages extend `base.html`. Writer and authentication pages also extend it, then add their surface-specific stylesheets before `theme.css`.

Keep user-provided values escaped. The only deliberately trusted HTML insertion is `content | safe` in `article.html`, and that value has already passed through the Markdown sanitizer.

## Complete examples

Copy an example into the live override file:

```powershell
Copy-Item -LiteralPath docs\examples\themes\midnight.css -Destination static\theme.css
```

Available examples:

- [`midnight.css`](examples/themes/midnight.css): dark blue editorial theme.
- [`terminal.css`](examples/themes/terminal.css): monospace green-screen theme.
- [`quiet-paper.css`](examples/themes/quiet-paper.css): restrained reading-first theme.

Examples are plain CSS rather than build artifacts. They can be copied in full or used as references for targeted overrides.

## Upgrade-friendly workflow

1. Keep site text in `site.yaml`.
2. Keep visual overrides in `static/theme.css`.
3. Avoid editing core styles unless adding a new reusable token or component.
4. When overriding structure, copy only the relevant template and document the divergence.
5. Run visual checks at desktop and narrow widths.
6. Run `cargo check --all-targets --all-features` and `cargo test --all`; themes should not require Rust changes.

## Accessibility checklist

- Preserve visible keyboard focus; do not remove outlines without a replacement.
- Maintain readable contrast for `--color-muted`, borders, and buttons.
- Keep prose near 45–75 characters per line.
- Do not communicate Draft/Scheduled/Published by color alone; text labels already exist.
- Supply useful Markdown image alt text after inserting assets.
- Verify dialog actions and Writer controls remain reachable by keyboard.
