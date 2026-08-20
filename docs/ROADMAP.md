# Nomad Roadmap

Status: draft. Phases are sequential; items within a phase are roughly
ordered. Active work lives in `tasks.md`; this file tracks intent.

## Phase 0 — Foundations (current)

- [x] Repository scaffold (`server/`, `wallet/`, `shared/`, `docs/`, `infra/`)
- [x] `docs/ARCHITECTURE.md`, `docs/PROTOCOL.md`, `docs/THREAT_MODEL.md`
- [ ] Canonical protocol definitions in `shared/protocol/` + JSON schemas in
      `shared/schemas/` matching PROTOCOL.md v1
- [ ] Toolchain pins recorded in `docs/` (Rust, nostr-sdk, BDK, Android)

## Phase 1 — Server (Rust, Umbrel)

Milestone: a user's phone can pair by QR and get private balance/history
answers from their own node.

- Identity (`0600` key file) + persistent allowlist + watch sets
- Gift-wrap transport (NIP-44/NIP-59 via nostr-sdk): subscribe, unwrap,
  authorize, route; replay cache; rate limits
- Pairing: one-time-secret QR + LAN HTTP UI (SVG QR, paired-wallet list,
  revoke)
- Electrs adapter: balance/UTXO/history (with block anchors)/raw tx/fee
  estimates/broadcast; single-flight gate + cooldown circuit-breaker
  (patterns proven in the legacy prototype)
- Watcher: monitor registered watch sets, emit `notify/new_tx` on mempool
  appearance and confirmation
- Umbrel packaging: Dockerfile, app manifest, community app-store metadata
- Tests: protocol unit tests + integration against regtest Electrs

## Phase 2 — Android watch-only wallet (APK)

Milestone: scan QR → enter address or xpub → see balances/history, get
notified on new transactions. No seed, no signing.

- Stack: **native Kotlin + official UniFFI bindings** (bdk-android for the
  BDK wallet core, nostr-sdk for Nostr) — decided 2026-08-19. iOS later
  consumes the same Rust cores via Swift bindings.
- Pairing screen (QR scan + manual paste), server management, unpair
- BDK watch-only wallet from address/xpub input (descriptors, gap-limited
  derivation); Nostr chain-source adapter fulfilling BDK scan/sync via the
  Nomad protocol
- Balance/history/UTXO UI; server-reachable/unreachable states
- Notification receive path (pick up missed gift wraps on open; local
  notifications)
- Nostr key in Keystore-backed storage

## Phase 3 — Spending wallet + watchdog

- Mnemonic create/restore, secure storage, no plaintext secrets anywhere
- Send flow: build → sign (BDK) → `broadcast_tx` → confirm; RBF
- Server v1.1: node-health watchdog (bitcoind/Electrs/disk), `notify/health`
- iOS wallet (same protocol semantics)

## Later / under consideration

- `scripts[]` queries (script types without address encodings)
- SPV header verification to detect a lying server
- Multi-server pairing; optional private/self-hosted relay
- Optional public-Esplora fallback mode (explicit user choice)
- Watch-only companion clients via the open protocol
- v2 wallet: **miniscript** for transaction constructions (vault and
  inheritance spending paths with timelocks/emergency clauses) — pairs
  with the server-side presigned-tx custody + trigger engine; BDK's
  miniscript support is the reference implementation
