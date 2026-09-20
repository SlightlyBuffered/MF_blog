use std::{
    collections::HashMap,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    extract::DefaultBodyLimit,
    http::{
        header::{CACHE_CONTROL, HeaderName, HeaderValue},
        Request,
    },
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use chrono_tz::Tz;
use tera::Tera;
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{auth, handlers};

pub const MAX_REQUEST_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_FILE_BYTES: usize = 15 * 1024 * 1024;
pub const MAX_IMPORT_FILES: usize = 100;

#[derive(Clone)]
pub struct AppState {
    pub root: PathBuf,
    pub article_root: PathBuf,
    pub auth_file: PathBuf,
    pub session_key_file: PathBuf,
    pub site_file: PathBuf,
    pub trash_root: PathBuf,
    pub timezone: Tz,
    pub bind: String,
    pub writer_allow_remote: bool,
    pub secure_cookies: bool,
    pub totp_enabled: bool,
    pub totp_secret: String,
    pub debug: bool,
    pub tera: Arc<Tera>,
    pub session_key: Arc<Vec<u8>>,
    pub dummy_password_hash: Arc<String>,
    pub login_attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    pub last_totp_step: Arc<Mutex<HashMap<String, i64>>>,
}

impl AppState {
    pub fn from_env() -> Result<Self, String> {
        let root = env::var_os("BLOG_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        let article_root = env_path("BLOG_ARTICLE_ROOT", root.join("articles"));
        let auth_file = env_path("BLOG_AUTH_FILE", root.join(".mf-blog-auth.json"));
        let session_key_file = env_path("BLOG_SESSION_KEY_FILE", root.join(".mf-blog-session-key"));
        let site_file = env_path("BLOG_SITE_FILE", root.join("site.yaml"));
        let trash_root = env::var_os("BLOG_TRASH_ROOT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                article_root
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(".mf-blog-trash")
            });

        let timezone_name = env::var("BLOG_TIMEZONE").unwrap_or_else(|_| "Asia/Makassar".to_string());
        let timezone = Tz::from_str(&timezone_name)
            .map_err(|_| format!("invalid BLOG_TIMEZONE: {timezone_name}"))?;

        let templates_glob = root
            .join("templates")
            .join("**")
            .join("*")
            .to_string_lossy()
            .replace('\\', "/");
        let tera = Tera::new(&templates_glob)
            .map_err(|error| format!("failed to load templates: {error}"))?;

        let session_key = auth::load_or_create_session_key(&session_key_file)?;
        let dummy_password_hash = auth::hash_password(&auth::random_token(24))?;

        Ok(Self {
            root,
            article_root,
            auth_file,
            session_key_file,
            site_file,
            trash_root,
            timezone,
            bind: env::var("BLOG_BIND").unwrap_or_else(|_| "127.0.0.1:8000".to_string()),
            writer_allow_remote: env_flag("WRITER_ALLOW_REMOTE", false),
            secure_cookies: env_flag("BLOG_SECURE_COOKIES", false),
            totp_enabled: env_flag("WRITER_TOTP_ENABLED", false),
            totp_secret: env::var("WRITER_TOTP_SECRET").unwrap_or_default().trim().to_string(),
            debug: env_flag("MF_BLOG_DEBUG", false),
            tera: Arc::new(tera),
            session_key: Arc::new(session_key.into_bytes()),
            dummy_password_hash: Arc::new(dummy_password_hash),
            login_attempts: Arc::new(Mutex::new(HashMap::new())),
            last_totp_step: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

fn env_path(name: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(fallback)
}

pub fn env_flag(name: &str, default: bool) -> bool {
    let fallback = if default { "1" } else { "0" };
    matches!(
        env::var(name)
            .unwrap_or_else(|_| fallback.to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn build_router(state: AppState) -> Router {
    let static_dir = state.root.join("static");

    Router::new()
        .route("/", get(handlers::index))
        .route("/articles/:slug/", get(handlers::article_page))
        .route("/articles/:slug/assets/*filename", get(handlers::article_asset))
        .route("/writer", get(handlers::writer_redirect))
        .route("/writer/", get(handlers::writer))
        .route(
            "/writer/setup",
            get(handlers::writer_setup_get).post(handlers::writer_setup_post),
        )
        .route(
            "/writer/login",
            get(handlers::writer_login_get).post(handlers::writer_login_post),
        )
        .route("/writer/logout", post(handlers::writer_logout))
        .route("/writer/api/articles", get(handlers::writer_articles).post(handlers::writer_create_article))
        .route(
            "/writer/api/articles/:slug",
            get(handlers::writer_article)
                .put(handlers::writer_save_article)
                .delete(handlers::writer_delete_article),
        )
        .route("/writer/api/preview", post(handlers::writer_preview))
        .route(
            "/writer/api/articles/:slug/assets",
            post(handlers::writer_upload_assets),
        )
        .route("/writer/api/import", post(handlers::writer_import))
        .nest_service("/static", ServeDir::new(static_dir))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

pub async fn security_headers(request: Request<axum::body::Body>, next: Next) -> Response {
    let is_writer = request.uri().path().starts_with("/writer");
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.entry(HeaderName::from_static("x-content-type-options")).or_insert(HeaderValue::from_static("nosniff"));
    headers
        .entry(HeaderName::from_static("x-frame-options"))
        .or_insert(HeaderValue::from_static("DENY"));
    headers
        .entry(HeaderName::from_static("referrer-policy"))
        .or_insert(HeaderValue::from_static("same-origin"));
    headers
        .entry(HeaderName::from_static("permissions-policy"))
        .or_insert(HeaderValue::from_static("camera=(), microphone=(), geolocation=()"));
    headers.entry(HeaderName::from_static("content-security-policy")).or_insert(HeaderValue::from_static(
        "default-src 'self'; img-src 'self' data: https:; media-src 'self'; \
         style-src 'self'; script-src 'self'; object-src 'none'; base-uri 'none'; \
         frame-ancestors 'none'; form-action 'self'",
    ));

    if is_writer {
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }

    response
}

pub fn remote_key(address: SocketAddr) -> String {
    address.ip().to_string()
}
