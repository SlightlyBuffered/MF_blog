mod app;
mod auth;
mod content;
mod handlers;

use std::net::SocketAddr;

use tracing_subscriber::EnvFilter;

use crate::app::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("mf_blog=info,tower_http=info")),
        )
        .init();

    let state = match AppState::from_env() {
        Ok(state) => state,
        Err(error) => {
            eprintln!("MF-Blog startup error: {error}");
            std::process::exit(1);
        }
    };

    let bind: SocketAddr = match state.bind.parse() {
        Ok(bind) => bind,
        Err(error) => {
            eprintln!("Invalid BLOG_BIND {:?}: {error}", state.bind);
            std::process::exit(1);
        }
    };

    let router = app::build_router(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .expect("failed to bind HTTP listener");

    tracing::info!("MF-Blog listening on http://{bind}");

    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("HTTP server failed");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
