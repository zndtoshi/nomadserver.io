//! Nomad Server — private wallet↔node bridge over Nostr.
//!
//! Boot order: config → identity → stores → shared state → HTTP UI +
//! gift-wrap transport (docs/PROTOCOL.md, docs/THREAT_MODEL.md).

mod config;
mod electrs;
mod handlers;
mod http;
mod identity;
mod pairing;
mod protocol;
mod ratelimit;
mod replay;
mod store;
mod transport;
mod watcher;

use std::sync::{Arc, Mutex};

use config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Both rustls backends (ring via nostr-sdk, aws-lc-rs via
    // electrum-client) are in the tree; pick ring explicitly or rustls
    // panics on first TLS handshake.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("install rustls ring provider");

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

    // Chain backend (Electrs) + protocol handlers.
    let electrs = Arc::new(electrs::Electrs::new(&config.electrs_addr));
    let handlers = Arc::new(handlers::Handlers::new(
        electrs.clone(),
        config.network,
        watch.clone(),
    ));

    // One relay-pool client shared by transport (subscribe) and watcher
    // (publish notifications).
    let nostr_client = nostr_sdk::prelude::Client::new();

    // Gift-wrap transport: relays, unwrap, authorize, route.
    let transport = transport::Transport::new(
        keys.clone(),
        nostr_client.clone(),
        config.relays.clone(),
        allowlist.clone(),
        watch.clone(),
        pairing.clone(),
        replay::ReplayCache::load(&config.data_dir)?,
        handlers,
    );
    tokio::spawn(async move {
        if let Err(e) = transport.run().await {
            tracing::error!("transport exited: {e}");
        }
    });

    // Watcher: polls watched addresses, pushes notify/new_tx.
    let watcher = watcher::Watcher::new(
        keys,
        nostr_client,
        electrs.clone(),
        config.network,
        watch,
        allowlist.clone(),
        &config.data_dir,
    )?;
    tokio::spawn(async move { watcher.run().await });

    let state = Arc::new(http::AppState {
        allowlist,
        pairing,
        server_pubkey,
        config: config.clone(),
        electrs,
    });

    let addr = format!("0.0.0.0:{}", config.http_port);
    tracing::info!("pairing UI on http://{addr} (LAN only)");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, http::router(state)).await?;
    Ok(())
}
