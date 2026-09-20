# Troubleshooting

## The homepage is empty

Check the article state:

- `draft: true` keeps it private.
- A future `release_date` keeps it scheduled.
- An invalid YAML document is skipped.

Open Writer; discovery errors appear there. With `MF_BLOG_DEBUG=1`, content errors also appear on the homepage.

## An article route returns 404

This is expected for drafts and scheduled articles. Public routes intentionally do not reveal whether private content exists.

If the article should be public, verify:

```yaml
draft: false
release_date: 2020-01-01T00:00:00+08:00
```

Also verify that the folder is directly under `articles/` and contains `article.md`.

## An image is broken

For local assets, use an article-relative path:

```md
![Useful alt text](assets/image.webp)
```

Confirm the filename casing and extension exactly. Draft assets return 404 outside an authenticated Writer session.

## Import says the slug already exists

Imports never overwrite. Rename the incoming directory/ZIP slug or intentionally rename/remove the existing article through the filesystem.

## Folder drag-and-drop imports nothing

Use **Choose folder** in a Chromium-family browser. Directory drag/drop and `webkitdirectory` are browser-specific. ZIP import is the most portable option.

## Writer redirects to setup unexpectedly

`.mf-blog-auth.json` is missing or the configured `BLOG_AUTH_FILE` points elsewhere. Restore the file from backup or complete first-run setup again.

## Writer rejects the correct password

- Usernames are case-sensitive.
- After five failures from an address, login is paused for 15 minutes.
- Confirm the server is using the expected `BLOG_AUTH_FILE`.
- Do not edit the Argon2 hash manually.

For a local development reset, stop the server and move `.mf-blog-auth.json` to a secure backup location. The next Writer request starts setup. This invalidates access to the old credential record; it does not affect articles.

## TOTP always fails

- `WRITER_TOTP_ENABLED` must be truthy.
- `WRITER_TOTP_SECRET` must contain the same Base32 secret enrolled in the authenticator.
- System clocks must be reasonably synchronized.
- A code already accepted during the current server process cannot be replayed.
- The verifier accepts the current 30-second step and one adjacent step on either side.

## Login expires after a restart

Confirm `.mf-blog-session-key` is writable and persistent. A newly generated key invalidates cookies signed by the previous key. In multi-process deployments, point every process at the same key with `BLOG_SECRET_KEY` or `BLOG_SESSION_KEY_FILE`.

## Theme changes do not appear

1. Edit `static/theme.css`, not an example file.
2. Hard-refresh the browser to bypass cached CSS.
3. Check selector specificity; the theme loads last, but a more specific core selector can still win.
4. Verify CSS syntax in browser developer tools.
5. Ensure CSP is not blocking a remote font or stylesheet. Remote styles are intentionally disallowed.

## Tests fail because an article was edited

They should not. Regression tests use temporary article directories. If a new test depends on `articles/`, fix the test rather than forcing user content back into a presumed state.

## Port 5000 is already in use

```powershell
.venv\Scripts\python -m flask --app app run --port 5001
```

Then open <http://127.0.0.1:5001/>.

## Docker cannot connect to the engine

If `docker compose up --build` reports that it cannot connect to the Docker API, start Docker Desktop or the system Docker daemon and retry. `docker --version` only verifies that the client is installed; it does not confirm that the engine is running.

## Writer returns 403 in Docker

Requests from the host cross the container network and are not loopback addresses from the service's perspective. Keep this setting in the Compose service:

```yaml
environment:
  WRITER_ALLOW_REMOTE: "1"
```

The supplied Compose file publishes the service only on `127.0.0.1`. If the port is exposed beyond the host, follow the HTTPS and secure-cookie requirements in the deployment guide.

## Docker cannot write articles

The image runs as UID/GID `1000` by default. On Linux, rebuild with the host account IDs:

```bash
MF_BLOG_UID=$(id -u) MF_BLOG_GID=$(id -g) docker compose up --build
```

Also confirm that the host `articles/` directory is writable by that account.

## Docker setup unexpectedly starts account onboarding

The writer account is stored in the named `mf-blog-state` volume, not in the source tree. Confirm the volume is mounted at `/data` and that `BLOG_AUTH_FILE` points to `/data/.mf-blog-auth.json`.

`docker compose down` preserves the volume. `docker compose down -v` deletes it and removes the container-managed credentials, signing key, and recoverable trash.
