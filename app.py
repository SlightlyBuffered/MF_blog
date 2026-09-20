from __future__ import annotations

import json
import os
import re
import secrets
import shutil
import tempfile
import time as system_time
import xml.etree.ElementTree as ET
import zipfile
from collections import defaultdict
from dataclasses import dataclass
from datetime import date, datetime, time, timedelta
from functools import wraps
from ipaddress import ip_address
from pathlib import Path, PurePosixPath
from urllib.parse import urlsplit
from zoneinfo import ZoneInfo

import bleach
import markdown
import pyotp
import yaml
from argon2 import PasswordHasher
from argon2.exceptions import InvalidHashError, VerificationError, VerifyMismatchError
from flask import (
    Flask,
    abort,
    jsonify,
    redirect,
    render_template,
    request,
    send_from_directory,
    session,
    url_for,
)
from markdown.extensions import Extension
from markdown.treeprocessors import Treeprocessor

ROOT = Path(__file__).resolve().parent
ARTICLE_ROOT = Path(os.getenv("BLOG_ARTICLE_ROOT", ROOT / "articles"))
SITE_TIMEZONE = ZoneInfo(os.getenv("BLOG_TIMEZONE", "Asia/Makassar"))
SLUG_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
ALLOWED_ASSET_EXTENSIONS = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".svg", ".mp4", ".webm"}
MAX_IMPORT_FILES = 100
MAX_FILE_BYTES = 15 * 1024 * 1024
AUTH_FILE = Path(os.getenv("BLOG_AUTH_FILE", ROOT / ".mf-blog-auth.json"))
SESSION_KEY_FILE = Path(os.getenv("BLOG_SESSION_KEY_FILE", ROOT / ".mf-blog-session-key"))
SITE_FILE = Path(os.getenv("BLOG_SITE_FILE", ROOT / "site.yaml"))
PASSWORD_HASHER = PasswordHasher(time_cost=3, memory_cost=65536, parallelism=2)
DUMMY_PASSWORD_HASH = PASSWORD_HASHER.hash(secrets.token_urlsafe(24))
LOGIN_ATTEMPTS: dict[str, list[float]] = defaultdict(list)
LAST_TOTP_STEP: dict[str, int] = {}
DEFAULT_SITE = {
    "name": "MF-Blog",
    "wordmark": "MF-Blog",
    "tagline": "Markdown publishing, organized by folders.",
    "language": "en",
    "home": {
        "eyebrow": "Latest articles",
        "title": "Writing, published\nfrom Markdown.",
        "empty": "No articles have been published yet.",
    },
}


def trash_root() -> Path:
    configured = os.getenv("BLOG_TRASH_ROOT", "").strip()
    return Path(configured) if configured else ARTICLE_ROOT.parent / ".mf-blog-trash"


def write_private_text(path: Path, value: str) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary.write_text(value, encoding="utf-8")
    os.replace(temporary, path)
    try:
        os.chmod(path, 0o600)
    except OSError:
        pass


def load_or_create_session_key() -> str:
    configured = os.getenv("BLOG_SECRET_KEY", "").strip()
    if configured:
        return configured
    try:
        existing = SESSION_KEY_FILE.read_text(encoding="utf-8").strip()
    except FileNotFoundError:
        existing = ""
    if existing:
        return existing
    generated = secrets.token_urlsafe(48)
    write_private_text(SESSION_KEY_FILE, generated + "\n")
    return generated


def load_site_config() -> dict:
    configured = {**DEFAULT_SITE, "home": dict(DEFAULT_SITE["home"])}
    if SITE_FILE.is_file():
        data = yaml.safe_load(SITE_FILE.read_text(encoding="utf-8")) or {}
        if not isinstance(data, dict):
            raise ValueError("site.yaml must contain a mapping")
        for key in ("name", "wordmark", "tagline", "language"):
            if key in data:
                if not isinstance(data[key], str) or not data[key].strip():
                    raise ValueError(f"site.yaml field {key!r} must be a non-empty string")
                configured[key] = data[key].strip()
        if "home" in data:
            if not isinstance(data["home"], dict):
                raise ValueError("site.yaml field 'home' must be a mapping")
            for key in ("eyebrow", "title", "empty"):
                if key in data["home"]:
                    if not isinstance(data["home"][key], str):
                        raise ValueError(f"site.yaml home.{key} must be a string")
                    configured["home"][key] = data["home"][key].strip()
    return configured


@dataclass(frozen=True)
class Article:
    slug: str
    title: str
    subtitle: str
    hero: str | None
    published_at: datetime
    release_at: datetime
    body: str
    draft: bool = False

    @property
    def url(self) -> str:
        return f"/articles/{self.slug}/"

    @property
    def hero_url(self) -> str | None:
        return asset_url(self.slug, self.hero) if self.hero else None


def parse_datetime(value: object, field: str) -> datetime:
    if isinstance(value, datetime):
        parsed = value
    elif isinstance(value, date):
        parsed = datetime.combine(value, time.min)
    elif isinstance(value, str):
        try:
            parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
        except ValueError as exc:
            raise ValueError(f"Invalid {field}: {value!r}") from exc
    else:
        raise ValueError(f"Missing or invalid {field}")

    return parsed.replace(tzinfo=SITE_TIMEZONE) if parsed.tzinfo is None else parsed


def split_front_matter(text: str) -> tuple[dict, str]:
    lines = text.lstrip("\ufeff").splitlines()
    if not lines or lines[0].strip() != "---":
        raise ValueError("Article must begin with YAML front matter")

    try:
        end = next(i for i, line in enumerate(lines[1:], 1) if line.strip() == "---")
    except StopIteration as exc:
        raise ValueError("YAML front matter is not closed") from exc

    metadata = yaml.safe_load("\n".join(lines[1:end])) or {}
    if not isinstance(metadata, dict):
        raise ValueError("YAML front matter must be a mapping")
    return metadata, "\n".join(lines[end + 1 :]).lstrip()


def load_article(path: Path) -> Article:
    metadata, body = split_front_matter(path.read_text(encoding="utf-8"))
    published = metadata.get("publication_date", metadata.get("publishedAt"))
    released = metadata.get("release_date", metadata.get("releaseAt", published))
    title = metadata.get("title")
    if not isinstance(title, str) or not title.strip():
        raise ValueError("title is required")

    subtitle = metadata.get("subtitle", "")
    if subtitle is None:
        subtitle = ""
    if not isinstance(subtitle, str):
        raise ValueError("subtitle must be a string")

    hero = metadata.get("hero")
    if hero is not None and not isinstance(hero, str):
        raise ValueError("hero must be a string")

    draft = metadata.get("draft", False)
    if not isinstance(draft, bool):
        raise ValueError("draft must be true or false, without quotes")

    return Article(
        slug=path.parent.name,
        title=title.strip(),
        subtitle=subtitle.strip(),
        hero=hero,
        published_at=parse_datetime(published, "publication_date"),
        release_at=parse_datetime(released, "release_date"),
        body=body,
        draft=draft,
    )


def discover_all_articles() -> tuple[list[Article], list[str]]:
    articles: list[Article] = []
    errors: list[str] = []
    for path in sorted(ARTICLE_ROOT.glob("*/article.md")):
        try:
            articles.append(load_article(path))
        except (OSError, ValueError, yaml.YAMLError) as exc:
            errors.append(f"{path.relative_to(ROOT)}: {exc}")
    articles.sort(key=lambda article: article.published_at, reverse=True)
    return articles, errors


def discover_articles(now: datetime | None = None) -> tuple[list[Article], list[str]]:
    now = now or datetime.now(SITE_TIMEZONE)
    articles, errors = discover_all_articles()
    return [article for article in articles if not article.draft and article.release_at <= now], errors


def asset_url(slug: str, value: str) -> str:
    parsed = urlsplit(value)
    if parsed.scheme or parsed.netloc or value.startswith(("/", "#")):
        return value
    path = PurePosixPath(value)
    parts = list(path.parts)
    if parts and parts[0] == ".":
        parts.pop(0)
    if parts and parts[0] == "assets":
        parts.pop(0)
    if not parts or any(part in {"", ".", ".."} for part in parts):
        return "#"
    return f"/articles/{slug}/assets/{'/'.join(parts)}"


class ArticleLinkRewriter(Treeprocessor):
    def run(self, root):
        slug = self.md.article_slug
        for element in root.iter():
            attribute = "src" if element.tag == "img" else "href" if element.tag == "a" else None
            if attribute and element.get(attribute):
                value = element.get(attribute)
                if element.tag == "img" or value.startswith(("assets/", "./assets/")):
                    element.set(attribute, asset_url(slug, value))
        return root


class ArticleLinkExtension(Extension):
    def __init__(self, slug: str):
        super().__init__()
        self.slug = slug

    def extendMarkdown(self, md):
        md.article_slug = self.slug
        md.treeprocessors.register(ArticleLinkRewriter(md), "article_links", 5)


ALLOWED_TAGS = set(bleach.sanitizer.ALLOWED_TAGS) | {
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "pre", "code", "hr", "br",
    "img", "table", "thead", "tbody", "tr", "th", "td", "del", "blockquote",
}
ALLOWED_ATTRIBUTES = {
    **bleach.sanitizer.ALLOWED_ATTRIBUTES,
    "img": ["src", "alt", "title", "loading"],
    "code": ["class"],
    "a": ["href", "title"],
}


def render_markdown(article: Article) -> str:
    raw_html = markdown.markdown(
        article.body,
        extensions=[
            "extra",
            "sane_lists",
            "smarty",
            ArticleLinkExtension(article.slug),
        ],
    )
    return bleach.clean(
        raw_html,
        tags=ALLOWED_TAGS,
        attributes=ALLOWED_ATTRIBUTES,
        protocols={"http", "https", "mailto"},
        strip=True,
    )


def article_payload(article: Article) -> dict:
    assets_dir = ARTICLE_ROOT / article.slug / "assets"
    assets = []
    if assets_dir.is_dir():
        assets = [path.relative_to(assets_dir).as_posix() for path in assets_dir.rglob("*") if path.is_file()]
    return {
        "slug": article.slug,
        "title": article.title,
        "subtitle": article.subtitle,
        "hero": article.hero or "",
        "publication_date": article.published_at.isoformat(),
        "release_date": article.release_at.isoformat(),
        "draft": article.draft,
        "published": not article.draft,
        "body": article.body,
        "assets": sorted(assets),
        "url": article.url,
    }


def validate_slug(value: object) -> str:
    slug = str(value or "").strip().lower()
    if not SLUG_RE.fullmatch(slug):
        raise ValueError("slug must contain lowercase letters, numbers, and single hyphens")
    return slug


def normalize_datetime_input(value: object, field: str) -> str:
    return parse_datetime(value, field).isoformat()


def build_article_source(data: dict) -> str:
    title = str(data.get("title", "")).strip()
    if not title:
        raise ValueError("title is required")
    hero = str(data.get("hero", "")).strip()
    if hero and asset_url("check", hero) == "#":
        raise ValueError("hero path is invalid")
    draft = bool(data.get("draft", False)) if "draft" in data else not bool(data.get("published", True))
    metadata = {
        "title": title,
        "subtitle": str(data.get("subtitle", "")).strip(),
        "hero": hero or None,
        "publication_date": normalize_datetime_input(data.get("publication_date"), "publication_date"),
        "release_date": normalize_datetime_input(data.get("release_date"), "release_date"),
        "draft": draft,
    }
    front_matter = yaml.safe_dump(metadata, sort_keys=False, allow_unicode=True).strip()
    return f"---\n{front_matter}\n---\n\n{str(data.get('body', '')).lstrip()}\n"


def safe_relative_path(value: str) -> PurePosixPath:
    path = PurePosixPath(value.replace("\\", "/"))
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise ValueError(f"unsafe path: {value}")
    return path


def validate_asset_file(path: Path) -> None:
    if path.suffix.lower() != ".svg":
        return
    try:
        root = ET.parse(path).getroot()
    except ET.ParseError as exc:
        raise ValueError(f"invalid SVG: {path.name}") from exc
    forbidden_tags = {"script", "foreignobject", "iframe", "object", "embed", "style"}
    for element in root.iter():
        tag = element.tag.rsplit("}", 1)[-1].lower()
        if tag in forbidden_tags:
            raise ValueError(f"active SVG content is not allowed: {path.name}")
        for attribute, value in element.attrib.items():
            name = attribute.rsplit("}", 1)[-1].lower()
            if name.startswith("on") or name == "style":
                raise ValueError(f"active SVG attributes are not allowed: {path.name}")
            if name == "href" and (urlsplit(value).scheme or value.startswith("//")):
                raise ValueError(f"external SVG references are not allowed: {path.name}")


def validate_import_tree(root: Path) -> Path:
    article_files = list(root.rglob("article.md"))
    if len(article_files) != 1:
        raise ValueError("import must contain exactly one article.md")
    article_dir = article_files[0].parent
    stray_files = [
        path for path in root.rglob("*") if path.is_file() and not path.is_relative_to(article_dir)
    ]
    if stray_files:
        stray_name = stray_files[0].relative_to(root).as_posix()
        raise ValueError(f"file exists outside the article folder: {stray_name}")
    for path in article_dir.rglob("*"):
        if path.is_symlink():
            raise ValueError("symlinks are not allowed")
        if path.is_dir():
            continue
        relative = path.relative_to(article_dir)
        if relative == Path("article.md"):
            continue
        valid_asset = (
            relative.parts
            and relative.parts[0] == "assets"
            and path.suffix.lower() in ALLOWED_ASSET_EXTENSIONS
        )
        if not valid_asset:
            raise ValueError(f"file is not allowed: {relative.as_posix()}")
        if path.stat().st_size > MAX_FILE_BYTES:
            raise ValueError(f"file is too large: {relative.as_posix()}")
        validate_asset_file(path)
    load_article(article_files[0])
    return article_dir


def import_staged_tree(
    staging: Path,
    requested_slug: str | None = None,
    fallback_slug: str | None = None,
) -> Article:
    article_dir = validate_import_tree(staging)
    inferred_slug = article_dir.name if article_dir != staging else fallback_slug
    slug = validate_slug(requested_slug or inferred_slug)
    destination = ARTICLE_ROOT / slug
    if destination.exists():
        raise FileExistsError(f"article '{slug}' already exists")
    ARTICLE_ROOT.mkdir(parents=True, exist_ok=True)
    shutil.move(str(article_dir), destination)
    return load_article(destination / "article.md")


def is_loopback_request() -> bool:
    try:
        return ip_address(request.remote_addr or "").is_loopback
    except ValueError:
        return False


def is_loopback_host() -> bool:
    hostname = urlsplit(f"//{request.host}").hostname
    if hostname == "localhost":
        return True
    try:
        return ip_address(hostname or "").is_loopback
    except ValueError:
        return False


def env_flag(name: str, default: bool = False) -> bool:
    fallback = "1" if default else "0"
    return os.getenv(name, fallback).strip().lower() in {"1", "true", "yes", "on"}


def load_auth_config() -> dict | None:
    try:
        data = json.loads(AUTH_FILE.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, json.JSONDecodeError) as exc:
        raise RuntimeError(f"Cannot read writer authentication config: {exc}") from exc
    if not isinstance(data, dict) or not data.get("username") or not data.get("password_hash"):
        raise RuntimeError("Writer authentication config is invalid")
    return data


def save_auth_config(username: str, password: str) -> None:
    data = {
        "username": username,
        "password_hash": PASSWORD_HASHER.hash(password),
        "created_at": datetime.now(SITE_TIMEZONE).isoformat(),
    }
    write_private_text(AUTH_FILE, json.dumps(data, indent=2) + "\n")


def validate_credentials(username: str, password: str) -> bool:
    config = load_auth_config()
    expected_username = config["username"] if config else ""
    password_hash = config["password_hash"] if config else DUMMY_PASSWORD_HASH
    try:
        password_valid = PASSWORD_HASHER.verify(password_hash, password)
    except (VerifyMismatchError, VerificationError, InvalidHashError):
        password_valid = False
    username_valid = config is not None and secrets.compare_digest(username, expected_username)
    if username_valid and password_valid and PASSWORD_HASHER.check_needs_rehash(password_hash):
        config["password_hash"] = PASSWORD_HASHER.hash(password)
        write_private_text(AUTH_FILE, json.dumps(config, indent=2) + "\n")
    return username_valid and password_valid


def totp_enabled() -> bool:
    return env_flag("WRITER_TOTP_ENABLED")


def validate_totp(username: str, code: str) -> bool:
    if not totp_enabled():
        return True
    secret = os.getenv("WRITER_TOTP_SECRET", "").strip()
    if not secret or not code.isdigit() or len(code) != 6:
        return False
    totp = pyotp.TOTP(secret)
    now = datetime.now().timestamp()
    matched_step = None
    current_step = int(now // totp.interval)
    for offset in (-1, 0, 1):
        step = current_step + offset
        if secrets.compare_digest(totp.at(step * totp.interval), code):
            matched_step = step
            break
    if matched_step is None or matched_step <= LAST_TOTP_STEP.get(username, -1):
        return False
    LAST_TOTP_STEP[username] = matched_step
    return True


def login_rate_limit(remote: str) -> tuple[bool, int]:
    now = system_time.monotonic()
    recent = [attempt for attempt in LOGIN_ATTEMPTS[remote] if now - attempt < 900]
    LOGIN_ATTEMPTS[remote] = recent
    if len(recent) < 5:
        return True, 0
    retry_after = max(1, int(900 - (now - recent[0])))
    return False, retry_after


def new_csrf_token() -> str:
    token = secrets.token_urlsafe(32)
    session["csrf_token"] = token
    return token


def require_valid_form_csrf() -> None:
    supplied = request.form.get("csrf_token", "")
    expected = session.get("csrf_token", "")
    if not supplied or not expected or not secrets.compare_digest(supplied, expected):
        abort(403)


def request_json_object() -> dict:
    data = request.get_json(silent=True)
    if not isinstance(data, dict):
        raise ValueError("request body must be a JSON object")
    return data


def writer_only(view):
    @wraps(view)
    def wrapped(*args, **kwargs):
        allow_remote = env_flag("WRITER_ALLOW_REMOTE")
        if not allow_remote and (not is_loopback_request() or not is_loopback_host()):
            abort(403)
        if not session.get("writer_authenticated"):
            if request.path.startswith("/writer/api/"):
                return jsonify(error="authentication required"), 401
            return redirect(url_for("writer_login"))
        if request.method not in {"GET", "HEAD", "OPTIONS"}:
            supplied = request.headers.get("X-CSRF-Token", "")
            expected = session.get("csrf_token", "")
            if not supplied or not expected or not secrets.compare_digest(supplied, expected):
                abort(403)
        return view(*args, **kwargs)
    return wrapped


app = Flask(__name__)
app.secret_key = load_or_create_session_key()
app.config["MAX_CONTENT_LENGTH"] = 50 * 1024 * 1024
app.config["SESSION_COOKIE_HTTPONLY"] = True
app.config["SESSION_COOKIE_SAMESITE"] = "Strict"
app.config["SESSION_COOKIE_SECURE"] = env_flag("BLOG_SECURE_COOKIES")
app.config["PERMANENT_SESSION_LIFETIME"] = timedelta(hours=8)


@app.context_processor
def inject_site_config():
    return {"site": load_site_config()}


@app.after_request
def security_headers(response):
    response.headers.setdefault("X-Content-Type-Options", "nosniff")
    response.headers.setdefault("X-Frame-Options", "DENY")
    response.headers.setdefault("Referrer-Policy", "same-origin")
    response.headers.setdefault("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
    response.headers.setdefault(
        "Content-Security-Policy",
        "default-src 'self'; img-src 'self' data: https:; media-src 'self'; "
        "style-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'; "
        "frame-ancestors 'none'; form-action 'self'",
    )
    if request.path.startswith("/writer"):
        response.headers["Cache-Control"] = "no-store"
    return response


@app.errorhandler(413)
def request_too_large(_error):
    if request.path.startswith("/writer/api/"):
        return jsonify(error="upload exceeds the 50 MB request limit"), 413
    return "Upload exceeds the 50 MB request limit.", 413


@app.get("/")
def index():
    articles, errors = discover_articles()
    return render_template("index.html", articles=articles, errors=errors)


@app.get("/articles/<slug>/")
def article_page(slug: str):
    articles, _ = discover_articles()
    article = next((item for item in articles if item.slug == slug), None)
    if article is None:
        abort(404)
    return render_template("article.html", article=article, content=render_markdown(article))


@app.get("/articles/<slug>/assets/<path:filename>")
def article_asset(slug: str, filename: str):
    article_dir = ARTICLE_ROOT / slug
    try:
        article = load_article(article_dir / "article.md")
    except (OSError, ValueError, yaml.YAMLError):
        abort(404)
    is_public = not article.draft and article.release_at <= datetime.now(SITE_TIMEZONE)
    if not is_public and not session.get("writer_authenticated"):
        abort(404)
    return send_from_directory(article_dir / "assets", filename)


@app.get("/writer/")
@writer_only
def writer():
    session.setdefault("csrf_token", secrets.token_urlsafe(32))
    return render_template(
        "writer.html",
        csrf_token=session["csrf_token"],
        allowed_extensions=sorted(ALLOWED_ASSET_EXTENSIONS),
    )


@app.route("/writer/setup", methods=["GET", "POST"])
def writer_setup():
    if not is_loopback_request() or not is_loopback_host():
        abort(403)
    if load_auth_config() is not None:
        return redirect(url_for("writer_login"))
    if request.method == "GET":
        return render_template("writer_setup.html", csrf_token=new_csrf_token(), error=None)
    require_valid_form_csrf()
    username = request.form.get("username", "").strip()
    password = request.form.get("password", "")
    confirmation = request.form.get("password_confirmation", "")
    error = None
    if not re.fullmatch(r"[A-Za-z0-9_.-]{3,64}", username):
        error = "Username must be 3–64 letters, numbers, dots, underscores, or hyphens."
    elif len(password) < 12:
        error = "Use a passphrase of at least 12 characters."
    elif password != confirmation:
        error = "The passphrases do not match."
    if error:
        return render_template("writer_setup.html", csrf_token=new_csrf_token(), error=error), 400
    save_auth_config(username, password)
    session.clear()
    return redirect(url_for("writer_login", created="1"))


@app.route("/writer/login", methods=["GET", "POST"])
def writer_login():
    allow_remote = env_flag("WRITER_ALLOW_REMOTE")
    if not allow_remote and (not is_loopback_request() or not is_loopback_host()):
        abort(403)
    if load_auth_config() is None:
        return redirect(url_for("writer_setup"))
    if session.get("writer_authenticated"):
        return redirect(url_for("writer"))
    error = None
    retry_after = 0
    if request.method == "POST":
        require_valid_form_csrf()
        remote = request.remote_addr or "unknown"
        allowed, retry_after = login_rate_limit(remote)
        username = request.form.get("username", "")
        password = request.form.get("password", "")
        code = request.form.get("totp", "").replace(" ", "")
        credentials_valid = validate_credentials(username, password) if allowed else False
        totp_valid = validate_totp(username, code) if credentials_valid else False
        if allowed and credentials_valid and totp_valid:
            LOGIN_ATTEMPTS.pop(remote, None)
            session.clear()
            session["writer_authenticated"] = True
            session["writer_username"] = username
            session.permanent = True
            new_csrf_token()
            return redirect(url_for("writer"))
        if allowed:
            LOGIN_ATTEMPTS[remote].append(system_time.monotonic())
        error = (
            "The credentials were not accepted."
            if allowed
            else f"Too many attempts. Try again in {retry_after} seconds."
        )
    return render_template(
        "writer_login.html",
        csrf_token=new_csrf_token(),
        error=error,
        totp_enabled=totp_enabled(),
        created=request.args.get("created") == "1",
    ), 401 if error else 200


@app.post("/writer/logout")
@writer_only
def writer_logout():
    session.clear()
    return jsonify(ok=True)


@app.get("/writer/api/articles")
@writer_only
def writer_articles():
    articles, errors = discover_all_articles()
    return jsonify({"articles": [article_payload(article) for article in articles], "errors": errors})


@app.get("/writer/api/articles/<slug>")
@writer_only
def writer_article(slug: str):
    try:
        slug = validate_slug(slug)
        article = load_article(ARTICLE_ROOT / slug / "article.md")
    except (OSError, ValueError, yaml.YAMLError):
        abort(404)
    return jsonify(article_payload(article))


@app.post("/writer/api/articles")
@writer_only
def writer_create_article():
    staging = None
    try:
        data = request_json_object()
        slug = validate_slug(data.get("slug"))
        destination = ARTICLE_ROOT / slug
        if destination.exists():
            return jsonify(error="that slug already exists"), 409
        source = build_article_source(data)
        ARTICLE_ROOT.mkdir(parents=True, exist_ok=True)
        staging = Path(tempfile.mkdtemp(prefix=".mf-blog-create-", dir=ARTICLE_ROOT.parent))
        (staging / "assets").mkdir()
        (staging / "article.md").write_text(source, encoding="utf-8")
        load_article(staging / "article.md")
        try:
            os.rename(staging, destination)
            staging = None
        except FileExistsError:
            return jsonify(error="that slug already exists"), 409
        return jsonify(article_payload(load_article(destination / "article.md"))), 201
    except (OSError, TypeError, ValueError) as exc:
        return jsonify(error=str(exc)), 400
    finally:
        if staging is not None:
            shutil.rmtree(staging, ignore_errors=True)


@app.put("/writer/api/articles/<slug>")
@writer_only
def writer_save_article(slug: str):
    try:
        slug = validate_slug(slug)
        article_file = ARTICLE_ROOT / slug / "article.md"
        if not article_file.is_file():
            abort(404)
        source = build_article_source(request_json_object())
        temporary = article_file.with_suffix(".md.tmp")
        temporary.write_text(source, encoding="utf-8")
        os.replace(temporary, article_file)
        return jsonify(article_payload(load_article(article_file)))
    except (TypeError, ValueError) as exc:
        return jsonify(error=str(exc)), 400


@app.delete("/writer/api/articles/<slug>")
@writer_only
def writer_delete_article(slug: str):
    try:
        slug = validate_slug(slug)
        article_dir = ARTICLE_ROOT / slug
        if not (article_dir / "article.md").is_file():
            abort(404)
        trash_directory = trash_root()
        trash_directory.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now(SITE_TIMEZONE).strftime("%Y%m%d-%H%M%S")
        destination = trash_directory / f"{slug}-{stamp}"
        suffix = 1
        while destination.exists():
            destination = trash_directory / f"{slug}-{stamp}-{suffix}"
            suffix += 1
        shutil.move(str(article_dir), destination)
        return jsonify(deleted=slug, trash_name=destination.name)
    except ValueError as exc:
        return jsonify(error=str(exc)), 400


@app.post("/writer/api/preview")
@writer_only
def writer_preview():
    try:
        data = request_json_object()
        source = build_article_source(data)
        metadata, body = split_front_matter(source)
        preview = Article(
            slug=validate_slug(data.get("slug", "preview")),
            title=metadata["title"],
            subtitle=metadata.get("subtitle", ""),
            hero=metadata.get("hero"),
            published_at=parse_datetime(metadata["publication_date"], "publication_date"),
            release_at=parse_datetime(metadata["release_date"], "release_date"),
            body=body,
            draft=bool(metadata.get("draft")),
        )
        return jsonify(html=render_markdown(preview))
    except (TypeError, ValueError) as exc:
        return jsonify(error=str(exc)), 400


@app.post("/writer/api/articles/<slug>/assets")
@writer_only
def writer_upload_assets(slug: str):
    staging_root = Path(tempfile.mkdtemp(prefix="mf-blog-assets-"))
    try:
        slug = validate_slug(slug)
        article_dir = ARTICLE_ROOT / slug
        if not (article_dir / "article.md").is_file():
            abort(404)
        uploads = request.files.getlist("files")
        if not uploads or len(uploads) > MAX_IMPORT_FILES:
            raise ValueError("choose between 1 and 100 files")
        saved = []
        for upload in uploads:
            relative = safe_relative_path(upload.filename or "")
            if relative.parts[0] == "assets":
                relative = PurePosixPath(*relative.parts[1:])
            if not relative.parts or relative.suffix.lower() not in ALLOWED_ASSET_EXTENSIONS:
                raise ValueError(f"file type is not allowed: {relative.as_posix()}")
            target = staging_root / Path(*relative.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            upload.save(target)
            if target.stat().st_size > MAX_FILE_BYTES:
                target.unlink()
                raise ValueError(f"file is too large: {relative.as_posix()}")
            validate_asset_file(target)
            if (article_dir / "assets" / Path(*relative.parts)).exists():
                raise ValueError(f"asset already exists: {relative.as_posix()}")
            if relative.as_posix() in saved:
                raise ValueError(f"duplicate upload path: {relative.as_posix()}")
            saved.append(relative.as_posix())
        for relative_name in saved:
            relative = PurePosixPath(relative_name)
            source = staging_root / Path(*relative.parts)
            target = article_dir / "assets" / Path(*relative.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(source, target)
        return jsonify(saved=saved)
    except ValueError as exc:
        return jsonify(error=str(exc)), 400
    finally:
        shutil.rmtree(staging_root, ignore_errors=True)


@app.post("/writer/api/import")
@writer_only
def writer_import():
    staging_root = Path(tempfile.mkdtemp(prefix="mf-blog-import-"))
    try:
        uploads = request.files.getlist("files")
        if not uploads or len(uploads) > MAX_IMPORT_FILES:
            raise ValueError("choose a ZIP or a folder containing no more than 100 files")
        fallback_slug = None
        if len(uploads) == 1 and Path(uploads[0].filename or "").suffix.lower() == ".zip":
            fallback_slug = Path(uploads[0].filename or "").stem
            archive_path = staging_root / "upload.zip"
            uploads[0].save(archive_path)
            with zipfile.ZipFile(archive_path) as archive:
                members = archive.infolist()
                if len(members) > MAX_IMPORT_FILES:
                    raise ValueError("ZIP contains too many files")
                if sum(member.file_size for member in members) > app.config["MAX_CONTENT_LENGTH"]:
                    raise ValueError("ZIP expands beyond the 50 MB import limit")
                for member in members:
                    relative = safe_relative_path(member.filename)
                    if member.is_dir():
                        continue
                    if (member.external_attr >> 16) & 0o170000 == 0o120000:
                        raise ValueError("ZIP symlinks are not allowed")
                    if member.file_size > MAX_FILE_BYTES:
                        raise ValueError(f"file is too large: {member.filename}")
                    target = staging_root / "content" / Path(*relative.parts)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    with archive.open(member) as source, target.open("wb") as destination:
                        shutil.copyfileobj(source, destination)
            staging = staging_root / "content"
        else:
            staging = staging_root / "content"
            for upload in uploads:
                relative = safe_relative_path(upload.filename or "")
                target = staging / Path(*relative.parts)
                target.parent.mkdir(parents=True, exist_ok=True)
                upload.save(target)
                if target.stat().st_size > MAX_FILE_BYTES:
                    raise ValueError(f"file is too large: {relative.as_posix()}")
        article = import_staged_tree(
            staging,
            requested_slug=request.form.get("slug") or None,
            fallback_slug=fallback_slug,
        )
        return jsonify(article_payload(article)), 201
    except FileExistsError as exc:
        return jsonify(error=str(exc)), 409
    except (OSError, RuntimeError, ValueError, zipfile.BadZipFile, yaml.YAMLError) as exc:
        return jsonify(error=str(exc)), 400
    finally:
        shutil.rmtree(staging_root, ignore_errors=True)


if __name__ == "__main__":
    app.run(host="127.0.0.1", debug=env_flag("FLASK_DEBUG"))
