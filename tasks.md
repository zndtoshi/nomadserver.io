# Task: Android watch-only wallet (APK)

Status: IN PROGRESS
Owner: Claude (implementation) · Codex (review/integration)
Protocol reference: `docs/PROTOCOL.md` v1 · Schemas: `shared/schemas/v1/`
Supersedes: Milestone 1 server task (phases 1–6 complete; server is
feature-complete and checkpointed through `1efb34b`).

## Objective

A native Android APK (Kotlin + UniFFI bindings) that:

1. Pairs with a Nomad Server by pasting the pairing JSON (QR camera scan
   in a later iteration) — PROTOCOL.md §4 handshake.
2. Accepts a single address or an xpub/ypub/zpub as a watch target;
   derives address lists locally (BDK descriptors; server stays dumb).
3. Shows confirmed/unconfirmed balance and recent history, synced
   exclusively through the encrypted Nostr channel.
4. Receives `notify/new_tx` messages pushed by the server's watcher.
5. Keeps the Nostr identity in Keystore-backed storage; no Bitcoin private
   material anywhere on the device in this milestone.

## Acceptance criteria

- [ ] `:app:assembleDebug` green from a clean checkout (docs/BUILD.md)
- [ ] Pairing works against the dev server (paired wallet visible in the
      server UI; unpair removes it)
- [ ] xpub target derives addresses (BDK), balance + history render from
      live server answers (mock Electrum OK until the new node is up)
- [ ] `notify/new_tx` surfaces in the UI while the app is open
- [ ] Nostr secret only in EncryptedSharedPreferences; no secrets logged
- [ ] APK installs on the user's phone (checkpoint: real pairing + sync
      against the home server over the LAN/Tailscale)

## Carried over from the server task (do after, not now)

- Live-data verification of all handlers against the real synced node
- Docker image build + Umbrel community-store install test
- Regtest integration test suite

## Notes (Claude-owned)

- Stack: native Kotlin + UniFFI (decided 2026-08-19). Pins in
  docs/BUILD.md: nostr-sdk 0.44.8, bdk-android 3.0.0, AGP 8.13.2,
  Kotlin 2.3.21, Compose BOM 2025.10.00 (newer BOM needs AGP 9).
- nostr-sdk 0.44.8 API verified from the AAR: Client.giftWrap /
  UnwrappedGift.fromGiftWrap / handleNotifications / Filter().kind().
  pubkey().since() / EventBuilder(kind, content).build(pubkey) for rumors.
- Relay patterns implemented in NomadClient per protocol §2.3: persistent
  notification stream, backfill since-3d, idempotent retry same-id,
  correlation by envelope id.
