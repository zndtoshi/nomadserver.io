//! LAN-only HTTP UI: pairing QR, status, paired-wallet management.
//!
//! This is the server's one unauthenticated surface (THREAT_MODEL.md §2):
//! LAN only, and everything sensitive is gated by the one-time pairing
//! secret inside the QR. HTML is server-rendered with inline strings —
//! deliberately minimal, no assets, no JS frameworks.

use std::sync::{Arc, Mutex};

use axum::extract::{Form, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::config::Config;
use crate::pairing::{now_secs, PairingManager, PairingPayload};
use crate::store::Allowlist;

pub struct AppState {
    pub config: Config,
    pub server_pubkey: String,
    pub pairing: Arc<Mutex<PairingManager>>,
    pub allowlist: Arc<Allowlist>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/pairing", get(pairing_json))
        .route("/qr", get(pairing_qr_svg))
        .route("/health", get(health))
        .route("/revoke", post(revoke_wallet))
        .with_state(state)
}

fn current_payload(state: &AppState) -> PairingPayload {
    state
        .pairing
        .lock()
        .unwrap()
        .payload_at(now_secs(), &state.server_pubkey, &state.config.relays)
}

async fn index(State(state): State<Arc<AppState>>) -> Html<String> {
    let wallets = state.allowlist.list();
    let wallet_rows = wallets
        .iter()
        .map(|pk| {
            format!(
                "<tr><td><code>{pk}</code></td>\
                 <td><form method='post' action='/revoke' style='display:inline'>\
                 <input type='hidden' name='pubkey' value='{pk}'>\
                 <button type='submit'>revoke</button></form></td></tr>"
            )
        })
        .collect::<String>();
    let relay_items = state
        .config
        .relays
        .iter()
        .map(|r| format!("<li><code>{r}</code></li>"))
        .collect::<String>();

    Html(format!(
        "<!doctype html><html><head><meta charset='utf-8'>\
         <meta name='viewport' content='width=device-width, initial-scale=1'>\
         <title>Nomad Server</title>\
         <style>body{{font-family:system-ui,sans-serif;max-width:44em;margin:2em auto;padding:0 1em}}\
         code{{word-break:break-all}}img{{max-width:280px}}</style></head><body>\
         <h1>Nomad Server</h1>\
         <p>Pubkey: <code>{}</code><br>Network: {:?} · Electrs: <code>{}</code></p>\
         <h2>Pair a wallet</h2>\
         <p>Scan with the Nomad wallet (valid for one pairing, 10 minutes):</p>\
         <p><img src='/qr' alt='pairing QR'></p>\
         <p><a href='/pairing'>pairing JSON</a> (manual fallback — treat it like a password)</p>\
         <h2>Relays</h2><ul>{relay_items}</ul>\
         <h2>Paired wallets ({})</h2>\
         <table>{wallet_rows}</table>\
         </body></html>",
        state.server_pubkey,
        state.config.network,
        state.config.electrs_addr,
        wallets.len()
    ))
}

async fn pairing_json(State(state): State<Arc<AppState>>) -> Json<PairingPayload> {
    Json(current_payload(&state))
}

async fn pairing_qr_svg(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let payload = current_payload(&state);
    let json = serde_json::to_string(&payload).expect("payload serializes");
    match qrcode::QrCode::new(json.as_bytes()) {
        Ok(code) => {
            let svg = code
                .render::<qrcode::render::svg::Color>()
                .min_dimensions(280, 280)
                .build();
            Ok(([(header::CONTENT_TYPE, "image/svg+xml")], svg))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize)]
struct RevokeForm {
    pubkey: String,
}

async fn revoke_wallet(
    State(state): State<Arc<AppState>>,
    Form(form): Form<RevokeForm>,
) -> Redirect {
    match state.allowlist.remove(&form.pubkey) {
        Ok(true) => tracing::info!(wallet = %form.pubkey, "wallet revoked via LAN UI"),
        Ok(false) => {}
        Err(e) => tracing::error!("failed to persist revocation: {e}"),
    }
    Redirect::to("/")
}
