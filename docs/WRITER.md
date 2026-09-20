# Writer workstation

Writer is a browser interface over the same article directories that can be edited directly with a text editor. It does not create a separate content model.

## First run

1. Start MF-Blog.
2. Open <http://127.0.0.1:5000/writer/>.
3. Create one username and a passphrase of at least 12 characters.
4. Sign in to Writer.

The generated `.mf-blog-auth.json` file contains the username and an Argon2id password hash, never the plaintext passphrase. It is excluded from Git.

## Workstation layout

- **Article rail:** lists every valid draft, scheduled article, and published article.
- **Metadata panel:** edits the slug, title, subtitle, publication time, release time, hero path, and Published state. It can be collapsed from the toolbar; the preference is stored in the browser.
- **Markdown pane:** edits the article body without exposing YAML front matter.
- **Preview pane:** displays sanitized server-rendered Markdown using the same renderer as the public article page.
- **Asset dock:** lists local assets, inserts Markdown image paths, and accepts new uploads.
- **Import panel:** accepts a complete article directory or ZIP archive.

The slug becomes read-only after creation because changing it would also change the public URL and directory name. Stop the server and rename the article directory manually when that change is intentional.

On viewports narrower than `800px`, the article rail becomes an overlay drawer and the editing panes become **Write** and **Preview** tabs. Short viewports use compact controls while keeping the editor and asset dock accessible.

## Saving and publication

Writer displays three computed states:

- **Draft:** Published is disabled.
- **Scheduled:** Published is enabled and Release is in the future.
- **Published:** Published is enabled and Release has elapsed.

The main toolbar provides an explicit state action:

- **Publish** enables Published and saves when Release is current or in the past.
- **Schedule** enables Published and saves when Release is in the future.
- **Unpublish** disables Published and saves, returning the article to Draft.

The Published checkbox in the metadata panel controls the same state. **Save article** writes all current fields without changing the selected publication state.

Unsaved changes are identified in the toolbar and protected by a browser navigation warning.

## Markdown editing

The Markdown toolbar includes undo, redo, bold, italic, link, inline code, heading, quote, and bulleted-list commands. Undo history is scoped to the currently open article and resets when another article is opened.

Keyboard shortcuts:

```text
Ctrl/Command+S          Save
Ctrl/Command+Z          Undo Markdown edit
Ctrl+Y                  Redo on Windows/Linux
Ctrl/Command+Shift+Z    Redo
Ctrl/Command+B          Bold selection
Ctrl/Command+I          Italicize selection
Ctrl/Command+K          Insert link
Ctrl/Command+Shift+M    Toggle metadata
```

## Create an article

Use the plus button, enter a valid slug and title, and select **Create article**. New articles begin as drafts with a small Markdown placeholder. MF-Blog creates:

```text
articles/<slug>/article.md
articles/<slug>/assets/
```

## Import a folder

Use **Choose folder** or drop a directory onto the import panel. Directory selection depends on the browser `webkitdirectory` and entry APIs; Chromium-based browsers provide the most consistent support.

The selected paths must resolve to exactly one article directory:

```text
my-story/article.md
my-story/assets/hero.webp
```

## Import a ZIP

Choose a `.zip` containing the same structure. MF-Blog validates member paths, file types, sizes, symlink metadata, expanded size, and article structure before moving content into `articles/`.

Imports are additive. An existing slug returns HTTP 409 and is not overwritten.

## Upload assets

Select one or more files from the asset dock. Nested paths from directory imports are retained. Uploads do not replace an existing asset path.

Clicking an asset inserts:

```md
![filename.webp](assets/filename.webp)
```

Replace the generated filename with useful alternative text before publishing.

## Delete and recover

Delete requires confirmation and moves the complete article directory into:

```text
.mf-blog-trash/<slug>-YYYYMMDD-HHMMSS/
```

Restore it manually while the server is stopped:

```powershell
Move-Item -LiteralPath .mf-blog-trash\my-story-20260920-120000 -Destination articles\my-story
```

The destination slug must not already exist. MF-Blog does not automatically purge `.mf-blog-trash/`.

## Session behavior

- Sessions last eight hours.
- **Sign out** clears the current Writer session.
- The signing key persists in `.mf-blog-session-key`, so a normal restart does not invalidate sessions.
- Deleting or replacing the signing key invalidates existing sessions.
- An API response with HTTP 401 redirects Writer to the sign-in page.
