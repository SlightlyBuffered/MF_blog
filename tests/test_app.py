import io
import zipfile
from datetime import datetime

import pyotp

import app as blog
from app import SITE_TIMEZONE, app, asset_url, discover_articles


def test_index_and_article_render(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    article_dir = tmp_path / "hello-folders"
    (article_dir / "assets").mkdir(parents=True)
    (article_dir / "article.md").write_text(
        "---\ntitle: The Blog Is Just a Folder\npublication_date: 2020-01-01\n"
        "release_date: 2020-01-01\n---\n\n![Diagram](assets/folder-diagram.svg)\n",
        encoding="utf-8",
    )
    (article_dir / "assets" / "folder-diagram.svg").write_text("<svg/>", encoding="utf-8")
    client = app.test_client()
    assert client.get("/").status_code == 200
    response = client.get("/articles/hello-folders/")
    assert response.status_code == 200
    assert b"The Blog Is Just a Folder" in response.data
    assert b'/articles/hello-folders/assets/folder-diagram.svg' in response.data


def test_release_date_hides_future_content(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    article_dir = tmp_path / "future"
    article_dir.mkdir()
    (article_dir / "article.md").write_text(
        "---\ntitle: Future\npublication_date: 2030-01-01\nrelease_date: 2030-01-01\n---\n",
        encoding="utf-8",
    )
    articles, errors = discover_articles(datetime(2020, 1, 1, tzinfo=SITE_TIMEZONE))
    assert articles == []
    assert errors == []


def test_asset_urls_cannot_escape_article_folder():
    assert asset_url("hello", "assets/image.png") == "/articles/hello/assets/image.png"
    assert asset_url("hello", "../secret.txt") == "#"


def csrf_client():
    client = app.test_client()
    with client.session_transaction() as session:
        session["writer_authenticated"] = True
        session["writer_username"] = "tester"
        session["csrf_token"] = "test-csrf-token"
    assert client.get("/writer/").status_code == 200
    return client, {"X-CSRF-Token": "test-csrf-token"}


def test_writer_creates_and_saves_draft(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    client, headers = csrf_client()
    payload = {
        "slug": "fresh-draft",
        "title": "Fresh Draft",
        "subtitle": "Still fermenting",
        "hero": "",
        "publication_date": "2026-09-20T10:00:00+08:00",
        "release_date": "2026-09-20T10:00:00+08:00",
        "published": False,
        "body": "## Unreleased\n",
    }
    response = client.post("/writer/api/articles", json=payload, headers=headers)
    assert response.status_code == 201
    assert (tmp_path / "fresh-draft" / "article.md").is_file()
    assert client.get("/articles/fresh-draft/").status_code == 404

    payload["published"] = True
    response = client.put("/writer/api/articles/fresh-draft", json=payload, headers=headers)
    assert response.status_code == 200
    assert client.get("/articles/fresh-draft/").status_code == 404

    payload["release_date"] = "2020-01-01T00:00:00+08:00"
    response = client.put("/writer/api/articles/fresh-draft", json=payload, headers=headers)
    assert response.status_code == 200
    assert client.get("/articles/fresh-draft/").status_code == 200


def test_zip_import_rejects_path_traversal(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    client, headers = csrf_client()
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, "w") as zipped:
        zipped.writestr("../escape.txt", "nope")
        zipped.writestr("article/article.md", "---\ntitle: Nope\npublication_date: 2020-01-01\n---\n")
    archive.seek(0)
    response = client.post(
        "/writer/api/import",
        data={"files": (archive, "article.zip")},
        headers=headers,
        content_type="multipart/form-data",
    )
    assert response.status_code == 400
    assert not (tmp_path.parent / "escape.txt").exists()


def test_writer_mutation_requires_csrf():
    client, _ = csrf_client()
    assert client.post("/writer/api/preview", json={}).status_code == 403


def test_zip_import_accepts_article_bundle(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    client, headers = csrf_client()
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, "w") as zipped:
        zipped.writestr(
            "imported-story/article.md",
            "---\ntitle: Imported Story\npublication_date: 2020-01-01\n"
            "release_date: 2020-01-01\n---\n\nHello.\n",
        )
        zipped.writestr("imported-story/assets/pixel.png", b"not-a-real-png-but-an-asset")
    archive.seek(0)
    response = client.post(
        "/writer/api/import",
        data={"files": (archive, "imported-story.zip")},
        headers=headers,
        content_type="multipart/form-data",
    )
    assert response.status_code == 201
    assert response.get_json()["slug"] == "imported-story"
    assert (tmp_path / "imported-story" / "assets" / "pixel.png").is_file()


def test_first_run_setup_and_login_without_totp(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "AUTH_FILE", tmp_path / "auth.json")
    monkeypatch.setenv("WRITER_TOTP_ENABLED", "0")
    blog.LOGIN_ATTEMPTS.clear()
    client = app.test_client()

    setup = client.get("/writer/setup")
    assert setup.status_code == 200
    with client.session_transaction() as session:
        setup_csrf = session["csrf_token"]
    created = client.post("/writer/setup", data={
        "csrf_token": setup_csrf,
        "username": "dave",
        "password": "long-development-passphrase",
        "password_confirmation": "long-development-passphrase",
    })
    assert created.status_code == 302
    assert (tmp_path / "auth.json").is_file()
    assert "long-development-passphrase" not in (tmp_path / "auth.json").read_text(encoding="utf-8")

    login = client.get("/writer/login")
    assert login.status_code == 200
    assert b"TOTP is disabled" in login.data
    with client.session_transaction() as session:
        login_csrf = session["csrf_token"]
    authenticated = client.post("/writer/login", data={
        "csrf_token": login_csrf,
        "username": "dave",
        "password": "long-development-passphrase",
    })
    assert authenticated.status_code == 302
    assert client.get("/writer/").status_code == 200


def test_totp_can_be_enabled_and_codes_cannot_be_replayed(monkeypatch):
    secret = pyotp.random_base32()
    monkeypatch.setenv("WRITER_TOTP_ENABLED", "1")
    monkeypatch.setenv("WRITER_TOTP_SECRET", secret)
    blog.LAST_TOTP_STEP.clear()
    code = pyotp.TOTP(secret).now()
    assert blog.validate_totp("dave", code)
    assert not blog.validate_totp("dave", code)


def test_delete_moves_complete_article_to_recoverable_trash(tmp_path, monkeypatch):
    article_root = tmp_path / "articles"
    monkeypatch.setattr(blog, "ARTICLE_ROOT", article_root)
    article_dir = article_root / "doomed-draft"
    (article_dir / "assets").mkdir(parents=True)
    (article_dir / "article.md").write_text(
        "---\ntitle: Doomed Draft\npublication_date: 2020-01-01\n"
        "release_date: 2020-01-01\ndraft: true\n---\n",
        encoding="utf-8",
    )
    (article_dir / "assets" / "kept.png").write_bytes(b"still recoverable")
    client, headers = csrf_client()
    response = client.delete("/writer/api/articles/doomed-draft", headers=headers)
    assert response.status_code == 200
    assert not article_dir.exists()
    trash_dir = tmp_path / ".mf-blog-trash" / response.get_json()["trash_name"]
    assert (trash_dir / "article.md").is_file()
    assert (trash_dir / "assets" / "kept.png").is_file()


def test_draft_assets_are_private_but_available_to_writer(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    article_dir = tmp_path / "secret-draft"
    (article_dir / "assets").mkdir(parents=True)
    (article_dir / "article.md").write_text(
        "---\ntitle: Secret\npublication_date: 2020-01-01\ndraft: true\n---\n",
        encoding="utf-8",
    )
    (article_dir / "assets" / "secret.png").write_bytes(b"secret")
    anonymous = app.test_client()
    assert anonymous.get("/articles/secret-draft/assets/secret.png").status_code == 404
    writer, _ = csrf_client()
    assert writer.get("/articles/secret-draft/assets/secret.png").status_code == 200


def test_quoted_draft_boolean_is_rejected(tmp_path):
    article_file = tmp_path / "article.md"
    article_file.write_text(
        "---\ntitle: Ambiguous\npublication_date: 2020-01-01\ndraft: 'false'\n---\n",
        encoding="utf-8",
    )
    try:
        blog.load_article(article_file)
    except ValueError as exc:
        assert "draft must be true or false" in str(exc)
    else:
        raise AssertionError("quoted draft boolean should be rejected")


def test_markdown_rewrites_assets_but_preserves_normal_relative_links():
    article = blog.Article(
        slug="links",
        title="Links",
        subtitle="",
        hero=None,
        published_at=datetime(2020, 1, 1, tzinfo=SITE_TIMEZONE),
        release_at=datetime(2020, 1, 1, tzinfo=SITE_TIMEZONE),
        body="[Notes](notes)\n\n![Diagram](assets/diagram.png)",
    )
    html = blog.render_markdown(article)
    assert 'href="notes"' in html
    assert 'src="/articles/links/assets/diagram.png"' in html


def test_writer_rejects_non_object_json():
    client, headers = csrf_client()
    response = client.post("/writer/api/preview", json=[], headers=headers)
    assert response.status_code == 400
    assert response.get_json()["error"] == "request body must be a JSON object"


def test_site_yaml_customizes_default_templates(tmp_path, monkeypatch):
    site_file = tmp_path / "site.yaml"
    site_file.write_text(
        "name: Test Gazette\nwordmark: GAZETTE\ntagline: Test words\nlanguage: id\n"
        "home:\n  eyebrow: Fresh\n  title: Custom front page\n  empty: Nothing here\n",
        encoding="utf-8",
    )
    monkeypatch.setattr(blog, "SITE_FILE", site_file)
    response = app.test_client().get("/")
    assert response.status_code == 200
    assert b"GAZETTE" in response.data
    assert b"Custom front page" in response.data
    assert b'<html lang="id">' in response.data


def test_writer_rejects_non_loopback_host_by_default(monkeypatch):
    monkeypatch.delenv("WRITER_ALLOW_REMOTE", raising=False)
    response = app.test_client().get("/writer/login", headers={"Host": "blog.example"})
    assert response.status_code == 403


def test_root_level_zip_uses_archive_name_as_slug(tmp_path, monkeypatch):
    monkeypatch.setattr(blog, "ARTICLE_ROOT", tmp_path)
    client, headers = csrf_client()
    archive = io.BytesIO()
    with zipfile.ZipFile(archive, "w") as zipped:
        zipped.writestr(
            "article.md",
            "---\ntitle: Root Story\npublication_date: 2020-01-01\n"
            "release_date: 2020-01-01\ndraft: true\n---\n",
        )
    archive.seek(0)
    response = client.post(
        "/writer/api/import",
        data={"files": (archive, "root-story.zip")},
        headers=headers,
        content_type="multipart/form-data",
    )
    assert response.status_code == 201
    assert response.get_json()["slug"] == "root-story"
    assert (tmp_path / "root-story" / "article.md").is_file()
