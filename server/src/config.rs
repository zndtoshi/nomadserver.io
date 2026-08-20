//! Server configuration from environment variables.
//!
//! Everything has a sane default for the Umbrel deployment; local dev
//! overrides via env. No config file on purpose — Umbrel apps are
//! configured through the manifest env.

use std::path::PathBuf;

/// Default public relays when NOSTR_RELAYS is unset. The wallet adopts the
/// server's relay set at pairing time, so both sides always agree.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.nostr.band",
    "wss://nostr.wine",
    "wss://relay.snort.social",
];

pub const DEFAULT_HTTP_PORT: u16 = 3829;
pub const DEFAULT_ELECTRS_ADDR: &str = "electrs:50001";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Bitcoin,
    Testnet,
    Signet,
    Regtest,
}

impl Network {
    fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "bitcoin" | "mainnet" => Ok(Self::Bitcoin),
            "testnet" => Ok(Self::Testnet),
            "signet" => Ok(Self::Signet),
            "regtest" => Ok(Self::Regtest),
            other => anyhow::bail!("unknown NOMAD_NETWORK: {other}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Persistent data dir (keys, allowlist, watch sets).
    pub data_dir: PathBuf,
    /// Electrum protocol endpoint of the local Electrs, host:port (TCP).
    pub electrs_addr: String,
    /// Nostr relays both sides will use.
    pub relays: Vec<String>,
    /// LAN HTTP UI port.
    pub http_port: u16,
    /// Bitcoin network the node is on; addresses are validated against it.
    pub network: Network,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = std::env::var("UMBREL_APP_DATA_DIR")
            .or_else(|_| std::env::var("NOMAD_DATA_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./data"));

        let electrs_addr = std::env::var("ELECTRS_ADDR")
            .unwrap_or_else(|_| DEFAULT_ELECTRS_ADDR.to_string());

        let relays = match std::env::var("NOSTR_RELAYS") {
            Ok(csv) => csv
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Err(_) => DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect(),
        };

        let http_port = std::env::var("NOMAD_HTTP_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_HTTP_PORT);

        let network = match std::env::var("NOMAD_NETWORK") {
            Ok(s) => Network::parse(&s)?,
            Err(_) => Network::Bitcoin,
        };

        Ok(Self {
            data_dir,
            electrs_addr,
            relays,
            http_port,
            network,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_networks() {
        assert_eq!(Network::parse("mainnet").unwrap(), Network::Bitcoin);
        assert_eq!(Network::parse("REGTEST").unwrap(), Network::Regtest);
        assert!(Network::parse("dogecoin").is_err());
    }
}
