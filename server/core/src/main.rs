use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use rkserve_core::api::{AppState, router};
use tower_http::services::{ServeDir, ServeFile};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("rkserve=info,tower_http=info")),
        )
        .init();

    let listen = std::env::var("RKSERVE_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let address: SocketAddr = listen.parse().context("parse RKSERVE_LISTEN")?;
    let plugin_root = std::env::var_os("RKSERVE_PLUGIN_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("plugins"));
    let runtime_root = std::env::var_os("RKSERVE_RUNTIME_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("run/plugins"));
    let state_root = std::env::var_os("RKSERVE_STATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_root.parent().unwrap_or(&runtime_root).to_owned());
    let console_root = std::env::var_os("RKSERVE_CONSOLE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../frontend/dist"));
    let state = AppState::load(&plugin_root, runtime_root.clone(), state_root.clone()).await?;

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("bind {address}"))?;
    let console =
        ServeDir::new(&console_root).fallback(ServeFile::new(console_root.join("index.html")));
    let app = router(state.clone()).fallback_service(console);

    info!(
        %address,
        api_keys = state.api_key_count(),
        trusted_forwarded_headers = state.trusts_forwarded_headers(),
        plugin_root = %plugin_root.display(),
        runtime_root = %runtime_root.display(),
        state_root = %state_root.display(),
        console_root = %console_root.display(),
        "rkserve core is ready"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
        .with_graceful_shutdown(shutdown_signal(state))
        .await
        .context("serve API")?;
    Ok(())
}

async fn shutdown_signal(state: AppState) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    state.shutdown().await;
}
