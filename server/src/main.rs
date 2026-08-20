//! Nomad Server — private wallet↔node bridge over Nostr.
//!
//! Boot order: config → identity → stores → shared state → HTTP UI +
//! gift-wrap transport (docs/PROTOCOL.md, docs/THREAT_MODEL.md).

mod config;
mod http;
mod identity;
mod pairing;
mod protocol;
mod ratelimit;
mod replay;
mod store;
mod transport;

use std::sync::{Arc, Mutex};

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

    let allowlist = Arc::new(store::Allowlist::load(&config.data_dir)?);
    let watch = Arc::new(store::WatchStore::load(&config.data_dir)?);
    let pairing = Arc::new(Mutex::new(pairing::PairingManager::new()));

    // Gift-wrap transport: relays, unwrap, authorize, route.
    let transport = transport::Transport::new(
        keys,
        config.relays.clone(),
        allowlist.clone(),
        watch,
        pairing.clone(),
        replay::ReplayCache::load(&config.data_dir)?,
    );
    tokio::spawn(async move {
        if let Err(e) = transport.run().await {
            tracing::error!("transport exited: {e}");
        }
    });

    let state = Arc::new(http::AppState {
        allowlist,
        pairing,
        server_pubkey,
        config: config.clone(),
    });

    let addr = format!("0.0.0.0:{}", config.http_port);
    tracing::info!("pairing UI on http://{addr} (LAN only)");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, http::router(state)).await?;
    Ok(())
}
