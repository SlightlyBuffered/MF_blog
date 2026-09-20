use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read, Write},
    net::{IpAddr, SocketAddr},
    path::{Path as FsPath, PathBuf},
    time::{Duration, Instant},
};

use axum::{
    extract::{connect_info::ConnectInfo, Form, Multipart, Path, State},
    http::{
        header::{CONTENT_TYPE, COOKIE, HOST, SET_COOKIE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use tempfile::Builder as TempBuilder;
use tera::Context;
use zip::ZipArchive;

use crate::{
    app::{remote_key, AppState, MAX_FILE_BYTES, MAX_IMPORT_FILES, MAX_REQUEST_BYTES},
    auth::{self, SessionData},
    content::{
        self, Article, ArticleInput, ALLOWED_ASSET_EXTENSIONS,
    },
};

fn json_error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({"error": message.into()}))).into_response()
}

fn render(state: &AppState, template: &str, context: &Context) -> Result<Html<String>, Response> {
    state
        .tera
        .render(template, context)
        .map(Html)
        .map_err(|error| {
            tracing::error!(%error, template, "template rendering failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Template rendering failed").into_response()
        })
}

fn site_context(state: &AppState) -> Result<Context, Response> {
    let site = content::load_site_config(&state.site_file)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error).into_response())?;
    let mut context = Context::new();
    context.insert("site", &site);
    Ok(context)
}

fn session_from_headers(headers: &HeaderMap, state: &AppState) -> SessionData {
    auth::parse_session_cookie(
        headers.get(COOKIE).and_then(|value| value.to_str().ok()),
        state,
    )
}

fn with_session_cookie(mut response: Response, session: &SessionData, state: &AppState) -> Response {
    match auth::session_cookie(session, state) {
        Ok(cookie) => {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                response.headers_mut().append(SET_COOKIE, value);
            }
        }
        Err(error) => tracing::error!(%error, "failed to encode session cookie"),
    }
    response
}

fn host_is_loopback(headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let host = host.trim();
    if host.eq_ignore_ascii_case("localhost") || host.starts_with("localhost:") {
        return true;
    }
    let without_port = if host.starts_with('[') {
        host.split(']').next().map(|value| value.trim_start_matches('[')).unwrap_or(host)
    } else {
        host.rsplit_once(':')
            .filter(|(_, port)| port.chars().all(|ch| ch.is_ascii_digit()))
            .map(|(host, _)| host)
            .unwrap_or(host)
    };
    without_port
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn writer_network_allowed(state: &AppState, address: SocketAddr, headers: &HeaderMap) -> bool {
    state.writer_allow_remote || (address.ip().is_loopback() && host_is_loopback(headers))
}

fn setup_network_allowed(address: SocketAddr, headers: &HeaderMap) -> bool {
    address.ip().is_loopback() && host_is_loopback(headers)
}

fn require_writer(
    state: &AppState,
    address: SocketAddr,
    headers: &HeaderMap,
    mutation: bool,
) -> Result<SessionData, Response> {
    if !writer_network_allowed(state, address, headers) {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    let session = session_from_headers(headers, state);
    if !session.writer_authenticated {
        return Err(json_error(StatusCode::UNAUTHORIZED, "authentication required"));
    }
    if mutation {
        let supplied = headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        if supplied.is_empty() || supplied != session.csrf_token {
            return Err(StatusCode::FORBIDDEN.into_response());
        }
    }
    Ok(session)
}

fn require_writer_page(
    state: &AppState,
    address: SocketAddr,
    headers: &HeaderMap,
) -> Result<SessionData, Response> {
    if !writer_network_allowed(state, address, headers) {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    let session = session_from_headers(headers, state);
    if !session.writer_authenticated {
        return Err(Redirect::to("/writer/login").into_response());
    }
    Ok(session)
}

pub async fn index(State(state): State<AppState>) -> Response {
    let mut context = match site_context(&state) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let (articles, errors) = content::discover_articles(&state.article_root, state.timezone, Utc::now());
    context.insert("articles", &articles);
    context.insert("errors", &errors);
    context.insert("debug", &state.debug);
    render(&state, "index.html", &context)
        .map(IntoResponse::into_response)
        .unwrap_or_else(|response| response)
}

pub async fn article_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Response {
    let (articles, _) = content::discover_articles(&state.article_root, state.timezone, Utc::now());
    let Some(article) = articles.into_iter().find(|article| article.slug == slug) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mut context = match site_context(&state) {
        Ok(context) => context,
        Err(response) => return response,
    };
    context.insert("article", &article);
    context.insert("content", &content::render_markdown(&article));
    render(&state, "article.html", &context)
        .map(IntoResponse::into_response)
        .unwrap_or_else(|response| response)
}

pub async fn article_asset(
    State(state): State<AppState>,
    Path((slug, filename)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let slug = match content::validate_slug(&slug) {
        Ok(slug) => slug,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let relative = match content::safe_relative_path(&filename) {
        Ok(relative) => relative,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let article_dir = state.article_root.join(&slug);
    let article = match content::load_article(&article_dir.join("article.md"), state.timezone) {
        Ok(article) => article,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let session = session_from_headers(&headers, &state);
    let public = !article.draft && article.release_at.with_timezone(&Utc) <= Utc::now();
    if !public && !session.writer_authenticated {
        return StatusCode::NOT_FOUND.into_response();
    }

    let path = article_dir.join("assets").join(relative);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    (
        [(CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")))],
        bytes,
    )
        .into_response()
}

pub async fn writer_redirect() -> Redirect {
    Redirect::permanent("/writer/")
}

pub async fn writer(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    let session = match require_writer_page(&state, address, &headers) {
        Ok(session) => session,
        Err(response) => return response,
    };
    let mut context = match site_context(&state) {
        Ok(context) => context,
        Err(response) => return response,
    };
    context.insert("csrf_token", &session.csrf_token);
    let extensions: Vec<String> = ALLOWED_ASSET_EXTENSIONS.iter().map(|value| format!(".{value}")).collect();
    context.insert("allowed_extensions", &extensions);
    render(&state, "writer.html", &context)
        .map(IntoResponse::into_response)
        .unwrap_or_else(|response| response)
}

#[derive(Deserialize)]
pub struct SetupForm {
    csrf_token: String,
    username: String,
    password: String,
    password_confirmation: String,
}

pub async fn writer_setup_get(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !setup_network_allowed(address, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match auth::load_auth_config(&state.auth_file) {
        Ok(Some(_)) => return Redirect::to("/writer/login").into_response(),
        Ok(None) => {}
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }

    let session = SessionData::anonymous();
    let mut context = match site_context(&state) {
        Ok(context) => context,
        Err(response) => return response,
    };
    context.insert("csrf_token", &session.csrf_token);
    context.insert("error", &Option::<String>::None);
    let response = render(&state, "writer_setup.html", &context)
        .map(IntoResponse::into_response)
        .unwrap_or_else(|response| response);
    with_session_cookie(response, &session, &state)
}

pub async fn writer_setup_post(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<SetupForm>,
) -> Response {
    if !setup_network_allowed(address, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match auth::load_auth_config(&state.auth_file) {
        Ok(Some(_)) => return Redirect::to("/writer/login").into_response(),
        Ok(None) => {}
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }

    let session = session_from_headers(&headers, &state);
    if form.csrf_token.is_empty() || form.csrf_token != session.csrf_token {
        return StatusCode::FORBIDDEN.into_response();
    }

    let username_re = regex::Regex::new(r"^[A-Za-z0-9_.-]{3,64}$").unwrap();
    let error = if !username_re.is_match(form.username.trim()) {
        Some("Username must be 3–64 letters, numbers, dots, underscores, or hyphens.".to_string())
    } else if form.password.len() < 12 {
        Some("Use a passphrase of at least 12 characters.".to_string())
    } else if form.password != form.password_confirmation {
        Some("The passphrases do not match.".to_string())
    } else {
        None
    };

    if let Some(error) = error {
        let refreshed = SessionData::anonymous();
        let mut context = match site_context(&state) {
            Ok(context) => context,
            Err(response) => return response,
        };
        context.insert("csrf_token", &refreshed.csrf_token);
        context.insert("error", &Some(error));
        let response = render(&state, "writer_setup.html", &context)
            .map(|html| (StatusCode::BAD_REQUEST, html).into_response())
            .unwrap_or_else(|response| response);
        return with_session_cookie(response, &refreshed, &state);
    }

    if let Err(error) = auth::save_auth_config(&state.auth_file, form.username.trim(), &form.password) {
        return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
    }
    let mut response = Redirect::to("/writer/login?created=1").into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&auth::clear_session_cookie(&state)).unwrap(),
    );
    response
}

#[derive(Deserialize)]
pub struct LoginForm {
    csrf_token: String,
    username: String,
    password: String,
    #[serde(default)]
    totp: String,
}

pub async fn writer_login_get(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    if !writer_network_allowed(&state, address, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match auth::load_auth_config(&state.auth_file) {
        Ok(None) => return Redirect::to("/writer/setup").into_response(),
        Ok(Some(_)) => {}
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
    let existing = session_from_headers(&headers, &state);
    if existing.writer_authenticated {
        return Redirect::to("/writer/").into_response();
    }

    let session = SessionData::anonymous();
    let mut context = match site_context(&state) {
        Ok(context) => context,
        Err(response) => return response,
    };
    context.insert("csrf_token", &session.csrf_token);
    context.insert("error", &Option::<String>::None);
    context.insert("totp_enabled", &state.totp_enabled);
    context.insert("created", &(query.get("created").map(String::as_str) == Some("1")));
    let response = render(&state, "writer_login.html", &context)
        .map(IntoResponse::into_response)
        .unwrap_or_else(|response| response);
    with_session_cookie(response, &session, &state)
}

pub async fn writer_login_post(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    if !writer_network_allowed(&state, address, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    match auth::load_auth_config(&state.auth_file) {
        Ok(None) => return Redirect::to("/writer/setup").into_response(),
        Ok(Some(_)) => {}
        Err(error) => return (StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }

    let current_session = session_from_headers(&headers, &state);
    if form.csrf_token.is_empty() || form.csrf_token != current_session.csrf_token {
        return StatusCode::FORBIDDEN.into_response();
    }

    let remote = remote_key(address);
    let (allowed, retry_after) = {
        let mut attempts = state.login_attempts.lock().expect("login attempt lock poisoned");
        let entries = attempts.entry(remote.clone()).or_default();
        entries.retain(|instant| instant.elapsed() < Duration::from_secs(900));
        if entries.len() < 5 {
            (true, 0)
        } else {
            let elapsed = entries.first().map(Instant::elapsed).unwrap_or_default().as_secs();
            (false, 900_u64.saturating_sub(elapsed).max(1))
        }
    };

    let credentials_valid = allowed
        && auth::validate_credentials(&state, &form.username, &form.password).unwrap_or(false);
    let totp_valid = credentials_valid && auth::validate_totp(&state, &form.username, &form.totp);

    if allowed && credentials_valid && totp_valid {
        state.login_attempts.lock().expect("login attempt lock poisoned").remove(&remote);
        let session = SessionData::authenticated(form.username);
        let response = Redirect::to("/writer/").into_response();
        return with_session_cookie(response, &session, &state);
    }

    if allowed {
        state.login_attempts
            .lock()
            .expect("login attempt lock poisoned")
            .entry(remote)
            .or_default()
            .push(Instant::now());
    }

    let refreshed = SessionData::anonymous();
    let error = if allowed {
        "The credentials were not accepted.".to_string()
    } else {
        format!("Too many attempts. Try again in {retry_after} seconds.")
    };
    let mut context = match site_context(&state) {
        Ok(context) => context,
        Err(response) => return response,
    };
    context.insert("csrf_token", &refreshed.csrf_token);
    context.insert("error", &Some(error));
    context.insert("totp_enabled", &state.totp_enabled);
    context.insert("created", &false);
    let response = render(&state, "writer_login.html", &context)
        .map(|html| (StatusCode::UNAUTHORIZED, html).into_response())
        .unwrap_or_else(|response| response);
    with_session_cookie(response, &refreshed, &state)
}

pub async fn writer_logout(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }
    let mut response = Json(json!({"ok": true})).into_response();
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&auth::clear_session_cookie(&state)).unwrap(),
    );
    response
}

pub async fn writer_articles(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, false) {
        return response;
    }
    let (articles, errors) = content::discover_all_articles(&state.article_root, state.timezone);
    let payloads: Vec<_> = articles
        .iter()
        .map(|article| content::article_payload(article, &state.article_root))
        .collect();
    Json(json!({"articles": payloads, "errors": errors})).into_response()
}

pub async fn writer_article(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, false) {
        return response;
    }
    let slug = match content::validate_slug(&slug) {
        Ok(slug) => slug,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    match content::load_article(&state.article_root.join(&slug).join("article.md"), state.timezone) {
        Ok(article) => Json(content::article_payload(&article, &state.article_root)).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn writer_create_article(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<ArticleInput>,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }

    let slug = match content::validate_slug(&input.slug) {
        Ok(slug) => slug,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let destination = state.article_root.join(&slug);
    if destination.exists() {
        return json_error(StatusCode::CONFLICT, "that slug already exists");
    }
    let source = match content::build_article_source(&input, state.timezone) {
        Ok(source) => source,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };

    if let Err(error) = fs::create_dir_all(&state.article_root) {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    let parent = state.article_root.parent().unwrap_or(FsPath::new("."));
    let staging = match TempBuilder::new().prefix(".mf-blog-create-").tempdir_in(parent) {
        Ok(staging) => staging,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if let Err(error) = fs::create_dir(staging.path().join("assets"))
        .and_then(|_| fs::write(staging.path().join("article.md"), source))
    {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    if let Err(error) = content::load_article(&staging.path().join("article.md"), state.timezone) {
        return json_error(StatusCode::BAD_REQUEST, error);
    }
    match fs::rename(staging.path(), &destination) {
        Ok(()) => {
            std::mem::forget(staging);
            match content::load_article(&destination.join("article.md"), state.timezone) {
                Ok(article) => (StatusCode::CREATED, Json(content::article_payload(&article, &state.article_root))).into_response(),
                Err(error) => json_error(StatusCode::BAD_REQUEST, error),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            json_error(StatusCode::CONFLICT, "that slug already exists")
        }
        Err(error) => json_error(StatusCode::BAD_REQUEST, error.to_string()),
    }
}

pub async fn writer_save_article(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(input): Json<ArticleInput>,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }
    let slug = match content::validate_slug(&slug) {
        Ok(slug) => slug,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let article_file = state.article_root.join(&slug).join("article.md");
    if !article_file.is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let source = match content::build_article_source(&input, state.timezone) {
        Ok(source) => source,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let temporary = article_file.with_extension("md.tmp");
    if let Err(error) = fs::write(&temporary, source).and_then(|_| fs::rename(&temporary, &article_file)) {
        let _ = fs::remove_file(&temporary);
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    match content::load_article(&article_file, state.timezone) {
        Ok(article) => Json(content::article_payload(&article, &state.article_root)).into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

pub async fn writer_delete_article(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }
    let slug = match content::validate_slug(&slug) {
        Ok(slug) => slug,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let article_dir = state.article_root.join(&slug);
    if !article_dir.join("article.md").is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(error) = fs::create_dir_all(&state.trash_root) {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut trash_name = format!("{slug}-{stamp}");
    let mut destination = state.trash_root.join(&trash_name);
    let mut suffix = 1;
    while destination.exists() {
        trash_name = format!("{slug}-{stamp}-{suffix}");
        destination = state.trash_root.join(&trash_name);
        suffix += 1;
    }
    if let Err(error) = fs::rename(&article_dir, &destination) {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    Json(json!({"deleted": slug, "trash_name": trash_name})).into_response()
}

pub async fn writer_preview(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(input): Json<ArticleInput>,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }
    let source = match content::build_article_source(&input, state.timezone) {
        Ok(source) => source,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let slug = match content::validate_slug(if input.slug.trim().is_empty() { "preview" } else { &input.slug }) {
        Ok(slug) => slug,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let temp = match tempfile::tempdir() {
        Ok(temp) => temp,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let article_dir = temp.path().join(&slug);
    if let Err(error) = fs::create_dir_all(&article_dir).and_then(|_| fs::write(article_dir.join("article.md"), source)) {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    match content::load_article(&article_dir.join("article.md"), state.timezone) {
        Ok(article) => Json(json!({"html": content::render_markdown(&article)})).into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}

pub async fn writer_upload_assets(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    mut multipart: Multipart,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }
    let slug = match content::validate_slug(&slug) {
        Ok(slug) => slug,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let article_dir = state.article_root.join(&slug);
    if !article_dir.join("article.md").is_file() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let staging = match tempfile::tempdir() {
        Ok(staging) => staging,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let mut saved = Vec::<String>::new();

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
        };
        if field.name() != Some("files") {
            continue;
        }
        if saved.len() >= MAX_IMPORT_FILES {
            return json_error(StatusCode::BAD_REQUEST, "choose between 1 and 100 files");
        }
        let filename = field.file_name().unwrap_or("").to_string();
        let mut relative = match content::safe_relative_path(&filename) {
            Ok(relative) => relative,
            Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
        };
        if relative.components().next().map(|part| part.as_os_str() == "assets").unwrap_or(false) {
            relative = relative.iter().skip(1).collect::<PathBuf>();
        }
        if relative.as_os_str().is_empty() || !content::asset_extension_allowed(&relative) {
            return json_error(StatusCode::BAD_REQUEST, format!("file type is not allowed: {}", relative.display()));
        }
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if saved.contains(&relative_text) {
            return json_error(StatusCode::BAD_REQUEST, format!("duplicate upload path: {relative_text}"));
        }
        if article_dir.join("assets").join(&relative).exists() {
            return json_error(StatusCode::BAD_REQUEST, format!("asset already exists: {relative_text}"));
        }
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
        };
        if bytes.len() > MAX_FILE_BYTES {
            return json_error(StatusCode::BAD_REQUEST, format!("file is too large: {relative_text}"));
        }
        let target = staging.path().join(&relative);
        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return json_error(StatusCode::BAD_REQUEST, error.to_string());
            }
        }
        if let Err(error) = fs::write(&target, &bytes) {
            return json_error(StatusCode::BAD_REQUEST, error.to_string());
        }
        if let Err(error) = content::validate_asset_file(&target) {
            return json_error(StatusCode::BAD_REQUEST, error);
        }
        saved.push(relative_text);
    }

    if saved.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "choose between 1 and 100 files");
    }

    for relative_name in &saved {
        let relative = PathBuf::from(relative_name);
        let source = staging.path().join(&relative);
        let target = article_dir.join("assets").join(&relative);
        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return json_error(StatusCode::BAD_REQUEST, error.to_string());
            }
        }
        if let Err(error) = fs::rename(&source, &target) {
            return json_error(StatusCode::BAD_REQUEST, error.to_string());
        }
    }

    Json(json!({"saved": saved})).into_response()
}

pub async fn writer_import(
    State(state): State<AppState>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if let Err(response) = require_writer(&state, address, &headers, true) {
        return response;
    }

    if let Err(error) = fs::create_dir_all(&state.article_root) {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    let parent = state.article_root.parent().unwrap_or(FsPath::new("."));
    let staging_root = match TempBuilder::new().prefix(".mf-blog-import-").tempdir_in(parent) {
        Ok(root) => root,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let content_root = staging_root.path().join("content");
    let mut uploads = Vec::<(String, Vec<u8>)>::new();
    let mut requested_slug: Option<String> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
        };
        match field.name() {
            Some("slug") => {
                requested_slug = field.text().await.ok().filter(|value| !value.trim().is_empty());
            }
            Some("files") => {
                if uploads.len() >= MAX_IMPORT_FILES {
                    return json_error(StatusCode::BAD_REQUEST, "choose a ZIP or a folder containing no more than 100 files");
                }
                let filename = field.file_name().unwrap_or("").to_string();
                let bytes = match field.bytes().await {
                    Ok(bytes) => bytes.to_vec(),
                    Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
                };
                if bytes.len() > MAX_REQUEST_BYTES {
                    return json_error(StatusCode::BAD_REQUEST, "upload exceeds the 50 MB request limit");
                }
                uploads.push((filename, bytes));
            }
            _ => {}
        }
    }

    if uploads.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "choose a ZIP or a folder containing no more than 100 files");
    }

    let mut fallback_slug = None;
    if uploads.len() == 1 && uploads[0].0.to_ascii_lowercase().ends_with(".zip") {
        let (filename, bytes) = uploads.pop().unwrap();
        fallback_slug = FsPath::new(&filename)
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::to_string);
        let mut archive = match ZipArchive::new(Cursor::new(bytes)) {
            Ok(archive) => archive,
            Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
        };
        if archive.len() > MAX_IMPORT_FILES {
            return json_error(StatusCode::BAD_REQUEST, "ZIP contains too many files");
        }
        let mut expanded = 0_u64;
        for index in 0..archive.len() {
            let mut member = match archive.by_index(index) {
                Ok(member) => member,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            expanded = expanded.saturating_add(member.size());
            if expanded > MAX_REQUEST_BYTES as u64 {
                return json_error(StatusCode::BAD_REQUEST, "ZIP expands beyond the 50 MB import limit");
            }
            if member.size() > MAX_FILE_BYTES as u64 {
                return json_error(StatusCode::BAD_REQUEST, format!("file is too large: {}", member.name()));
            }
            if member.unix_mode().map(|mode| mode & 0o170000 == 0o120000).unwrap_or(false) {
                return json_error(StatusCode::BAD_REQUEST, "ZIP symlinks are not allowed");
            }
            let relative = match content::safe_relative_path(member.name()) {
                Ok(relative) => relative,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
            };
            if member.is_dir() {
                continue;
            }
            let target = content_root.join(relative);
            if let Some(parent) = target.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    return json_error(StatusCode::BAD_REQUEST, error.to_string());
                }
            }
            let mut output = match fs::File::create(&target) {
                Ok(output) => output,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error.to_string()),
            };
            if let Err(error) = std::io::copy(&mut member, &mut output) {
                return json_error(StatusCode::BAD_REQUEST, error.to_string());
            }
        }
    } else {
        for (filename, bytes) in uploads {
            if bytes.len() > MAX_FILE_BYTES {
                return json_error(StatusCode::BAD_REQUEST, format!("file is too large: {filename}"));
            }
            let relative = match content::safe_relative_path(&filename) {
                Ok(relative) => relative,
                Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
            };
            let target = content_root.join(relative);
            if let Some(parent) = target.parent() {
                if let Err(error) = fs::create_dir_all(parent) {
                    return json_error(StatusCode::BAD_REQUEST, error.to_string());
                }
            }
            if let Err(error) = fs::write(target, bytes) {
                return json_error(StatusCode::BAD_REQUEST, error.to_string());
            }
        }
    }

    let article_dir = match content::validate_import_tree(&content_root, state.timezone) {
        Ok(path) => path,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let inferred_slug = if article_dir != content_root {
        article_dir.file_name().and_then(|value| value.to_str()).map(str::to_string)
    } else {
        fallback_slug
    };
    let slug_source = requested_slug.as_deref().or(inferred_slug.as_deref()).unwrap_or("");
    let slug = match content::validate_slug(slug_source) {
        Ok(slug) => slug,
        Err(error) => return json_error(StatusCode::BAD_REQUEST, error),
    };
    let destination = state.article_root.join(&slug);
    if destination.exists() {
        return json_error(StatusCode::CONFLICT, format!("article '{slug}' already exists"));
    }
    if let Err(error) = fs::rename(&article_dir, &destination) {
        return json_error(StatusCode::BAD_REQUEST, error.to_string());
    }
    match content::load_article(&destination.join("article.md"), state.timezone) {
        Ok(article) => (StatusCode::CREATED, Json(content::article_payload(&article, &state.article_root))).into_response(),
        Err(error) => json_error(StatusCode::BAD_REQUEST, error),
    }
}
