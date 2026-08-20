# Task: Milestone 1 — Nomad Server (pairing + private watch-only balance service)

Status: IN PROGRESS
Owner: Claude (implementation) · Codex (review/integration)
Protocol reference: `docs/PROTOCOL.md` v1 · Threat model: `docs/THREAT_MODEL.md`

## Objective

Build the Rust Nomad Server for the user's Umbrel home node (Bitcoin Core +
Electrs, fully synced) such that:

1. It serves a LAN-only HTTP UI displaying a pairing QR code (Nostr identity
   + relays + one-time secret) per PROTOCOL.md §4.
2. It runs the gift-wrapped Nostr transport (NIP-44/NIP-59 via nostr-sdk
   0.45): subscribe, unwrap, authorize against the allowlist, route.
3. It answers v1 message types from paired wallets by proxying Electrs:
   `pair`, `unpair`, `ping`, `get_balance`, `get_utxos`, `get_history`,
   `get_transactions`, `get_fee_estimates`, `broadcast_tx`,
   `watch_addresses`, plus `notify/new_tx` emission for watched addresses.
4. It is packaged as an Umbrel app (Dockerfile + manifest) for deployment on
   the user's node.

The Android APK (native Kotlin + UniFFI, decided 2026-08-19) is a separate
later task; this task ends at a server the APK can pair with and query.

## Acceptance criteria

- [ ] `cargo test` green: envelope/codec, pairing HMAC, allowlist authz,
      replay-id cache, rate limiting, schema fixtures vs `shared/schemas/`
- [ ] Pairing QR renders at the HTTP UI; `pair` handshake allowlists a real
      key; revoked/unpaired senders are dropped after unwrap
- [ ] All v1 message types answered correctly against a local Electrs
      (regtest or the user's node), including block anchors in history
- [ ] `watch_addresses` + Electrs polling emits `notify/new_tx` (mempool +
      confirmation) to the paired wallet
- [ ] No secrets/decrypted content in logs; key file `0600`; documented env
      config (`ELECTRS_ADDR`, `NOSTR_RELAYS`, data dir)
- [ ] Dockerfile builds; `infra/` Umbrel manifest present
- [ ] Docs updated if implementation deviates from PROTOCOL.md

## Constraints (from THREAT_MODEL.md — normative)

- NIP-44 v2 / NIP-59 gift wrap via nostr-sdk only; no hand-rolled crypto;
  pairing proof is HMAC-SHA-256; constant-time compare.
- Sender authorization immediately after unwrap, before any backend work;
  strangers get at most `not_paired`.
- Validation and limits per PROTOCOL.md §7 (address validation, caps, token
  bucket, single-flight Electrs gate + cooldown).
- Never log mnemonics/keys/pairing secrets/decrypted payloads.
- Canonical message definitions only in `shared/`; no protocol behavior
  invented inside server code.

## Phases

1. [x] Schemas (`shared/schemas/v1/`) + repo build plumbing (Rust workspace)
2. [x] Server skeleton: config, identity, allowlist/watch-set stores
3. [x] Pairing (secret, HMAC, QR/SVG HTTP UI)
4. [x] Gift-wrap transport + routing + replay/rate limits
5. [x] Electrs adapter + all v1 handlers
6. [ ] Watcher + `notify/new_tx`
7. [~] Dockerfile + Umbrel manifest + integration test pass
8. [ ] Docs sync, report, READY FOR CODEX REVIEW

## Implementation report (Claude-owned)

_Updated as phases complete._

- 2026-08-20: Task created. Docs revised to v1 gift-wrap design this week;
  Android stack decided (native Kotlin + UniFFI). Rust toolchain installed
  for dev user (rustup, stable, minimal profile) — none existed on the box.
  Pins: nostr-sdk 0.45.2, electrum-client 0.25, axum 0.8, bitcoin 0.32.
- 2026-08-20 (phase 4): Gift-wrap transport live-verified end-to-end on
  public relays with `examples/pair_client.rs` (wallet-side handshake).
  Lessons baked into PROTOCOL.md §2.3: stored-kind backfill subscription
  (since now-3d) + envelope-id dedupe instead of live-only `limit(0)`;
  wait for relay connections before publishing; pairing secret is
  memory-only so outstanding QRs die on server restart. Unit tests cover
  envelope parse, replay cache, rate limiter, routing (pair/unpair/authz),
  and an offline gift-wrap roundtrip. `cargo test`: 18 passed.
- 2026-08-20 (phase 5): Electrs adapter (single-flight, 100 ms spacing,
  15 s cooldown on timeout, per-call timeouts, fresh conn per batch) + all
  v1 handlers implemented and validated (§7 limits, network-checked
  addresses). `scripts/mock_electrum.py` added: full pair→ping→balance→
  utxos→history→fees→watch probe passes end-to-end against the mock
  (`pair_client <url> <addr>`). Default relay set updated to relays
  measured reachable + 1059-friendly from a home network (band/snort
  unreachable, wine AUTH-walled). rustls ring provider pinned (aws-lc-rs
  pulled in by electrum-client conflicted). Live-data verification
  against the user's node pending (node currently down).
