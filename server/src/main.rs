//! Nomad Server — private wallet↔node bridge over Nostr.
//!
//! Boot order: config → identity → stores → HTTP UI → (transport, watcher
//! — wired in later phases). See docs/PROTOCOL.md and docs/THREAT_MODEL.md.

mod config;
mod http;
mod identity;
mod pairing;
mod store;

use std::sync::Arc;

use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nomad_server=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!(
        "nomad-server starting: network={:?} electrs={} relays={} data_dir={}",
        config.network,
        config.electrs_addr,
        config.relays.len(),
        config.data_dir.display()
    );

    let keys = identity::load_or_create(&config.data_dir)?;
    let server_pubkey = keys.public_key().to_hex();

    let state = Arc::new(http::AppState {
        allowlist: store::Allowlist::load(&config.data_dir)?,
        pairing: std::sync::Mutex::new(pairing::PairingManager::new()),
        server_pubkey,
        config: config.clone(),
    });

    let addr = format!("0.0.0.0:{}", config.http_port);
    tracing::info!("pairing UI on http://{addr} (LAN only)");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, http::router(state)).await?;
    Ok(())
}
