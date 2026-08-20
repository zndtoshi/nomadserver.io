# Nomad Architecture

Nomad is self-hosted Bitcoin infrastructure that connects a home node to a
mobile wallet without any direct network path between them. All
wallet ↔ server communication travels over public **Nostr** relays as
end-to-end encrypted, sender-concealed messages. The server never holds
Bitcoin keys; the wallet never opens a socket to the server.

The first milestone is deliberately narrow: a **watch-only** wallet that
shows balances and history for an address or xpub, served privately by the
user's own node, plus a server that can proactively notify the wallet when
something happens. Spending comes later, on the same foundations.

## Why Nostr as the transport

A home node is typically behind NAT, a dynamic IP, and a paranoid firewall.
Making it reachable from a phone on cellular normally means port forwarding,
dynamic DNS, TLS certificates, or a VPN — each a setup burden and an attack
surface. Nostr inverts the problem:

- **No inbound connectivity required.** Both sides make outbound WebSocket
  connections to public relays. NAT and firewalls are irrelevant.
- **Relays are dumb pipes.** Messages are NIP-44-encrypted and NIP-59
  gift-wrapped: relays see an expiring ciphertext addressed to a pubkey and
  cannot tell who sent it or what it says.
- **Store-and-forward for free.** Gift wraps are retained by relays (until
  an expiration tag), so the server can notify a wallet that is offline,
  and requests can be answered while the app is backgrounded.
- **Redundancy for free.** Both sides publish to several relays; any one
  relay can be down or hostile without breaking the link.
- **An open ecosystem.** The protocol is documented Nostr events
  (`docs/PROTOCOL.md`), so third-party wallets can speak to a Nomad Server
  without any Nomad-specific SDK.

The cost: relays hold expiring, un-linkable ciphertext blobs; wallets must
discard spam wraps that fail to decrypt; and timing metadata exists (see
`docs/THREAT_MODEL.md`).

## Components

```
┌─────────────────────┐                     ┌──────────────────────────┐
│   Nomad Wallet      │                     │      Nomad Server        │
│   (Android; iOS     │                     │      (home node)         │
│    later)           │                     │                          │
│  watch-only: xpub/  │                     │  allowlist + gift-wrap   │
│  address → BDK      │  gift-wrapped Nostr │  protocol handler        │
│  descriptors        │  messages (kind     │  address watcher ────────┼─┐
│  nostr-sdk client ──┼──► public relays ◄──┼── nostr-sdk client       │ │
│  (Rust, bridged)    │  1059, NIP-44)      │        │                 │ │
└─────────────────────┘                     │        ▼                 │ │
      ▲                                     │  Electrs (Electrum TCP)  │ │
      └────────── new_tx / health ◄─────────┼── bitcoind (fees/blocks) │─┘
                notifications               │  local HTTP UI (pairing  │
                                            │  QR, paired wallets)     │
                                            └──────────────────────────┘
```

### Nomad Server (`server/`)

A Rust service, deployed as an Umbrel app (Docker), that:

- holds a single Nostr identity keypair (its only secret; it holds **no**
  Bitcoin keys and can move **no** funds),
- maintains an **allowlist** of paired wallet pubkeys (pairing via QR, see
  `docs/PROTOCOL.md` §4),
- answers encrypted protocol requests by proxying the local Electrs
  (balances, UTXOs, history with block anchors, raw transactions, fee
  estimates) and broadcasting signed transactions,
- **watches** addresses registered by paired wallets (`watch_addresses`)
  and pushes gift-wrapped `notify` messages when a transaction first
  appears (mempool) or confirms,
- serves a minimal LAN-only HTTP UI: pairing QR, paired-wallet list with
  revocation, and health status.

Two deliberate shape decisions:

- **The server is a dumb chain proxy.** It does not derive addresses and
  does not understand descriptors. Wallets derive addresses from xpubs
  themselves (BDK does this in one line) and send address lists. This keeps
  the server small, keeps wallet logic in one place, and makes the server
  trivially usable by third-party wallets.
- **The server is proactive, not just a request handler.** Watching and
  notifying is a first-class role; node-health watchdog alerts (folding in
  the standalone btc-watchdog concept: bitcoind status, Electrs health,
  disk) follow in v1.1.

### Nomad Wallet (`wallet/android/`, later `wallet/ios/`)

Milestone 1 is a **watch-only** Android app:

- user enters an address or an xpub/ypub/zpub; the app builds a BDK
  watch-only wallet (descriptors, gap-limited derivation) — no seed, no
  signing keys on the device in this milestone,
- pairs with the user's server by scanning the QR (or pasting the JSON),
- syncs balances/history/UTXOs exclusively through the Nomad protocol
  (BDK's chain source is our Nostr adapter — see below),
- receives and displays server notifications (new transactions).

Later milestones add the full self-custodial wallet: mnemonic, descriptors,
transaction building, PSBT signing — all on-device via BDK, with the same
protocol carrying `broadcast_tx`.

**BDK integration shape.** `bdk_wallet` performs no network I/O itself: it
emits scan/sync requests (script-pubkey iterators) and applies `Update`s.
Our Nostr chain-source adapter is the fulfiller — it walks the request's
script pubkeys, queries the server, assembles transactions plus block
anchors plus tip checkpoint, and calls `apply_update`. The wallet never
knows Nostr exists; BDK never knows networking exists. This is why the
protocol carries block hashes/timestamps (anchors) and a tip checkpoint in
`ping`.

### Shared protocol (`shared/protocol/`, `shared/schemas/`)

Canonical, platform-neutral definitions of everything on the wire: rumor
kinds, envelope schema, message types, error codes, pairing payload. Server
and wallets are independently deployable, so the shared definitions are the
only coupling between them. Any change here must consider server, Android,
iOS, third-party clients, and backward compatibility.

## Trust boundaries

1. **The phone is (eventually) the vault.** In milestone 1 it holds no
   secrets at all beyond its Nostr key and the watch-only xpubs. When
   spending lands, seed and signing keys live only in platform secure
   storage; only *signed* transactions ever leave.
2. **The relays are untrusted transport.** They see expiring ciphertext
   addressed to a pubkey. They can drop or delay messages but cannot forge
   them (seal signatures), cannot read them (NIP-44), and cannot tell who
   sent them (random wrap keys).
3. **The home server is semi-trusted.** It is the user's own machine, so it
   necessarily learns the watched addresses and balances — that is the
   privacy the user *keeps* by self-hosting instead of using someone
   else's Electrum server. It cannot spend (it never sees a key), and it
   answers only allowlisted wallets.
4. **The LAN is a pairing surface only.** The server's HTTP UI is the one
   unauthenticated endpoint; pairing is protected by a short-lived
   one-time secret inside the QR.

## What the server explicitly cannot do

- Spend or co-sign anything (no keys, no PSBT handling).
- Authorize itself — a message from an unpaired key is dropped after
  unwrap, before any backend work.
- Serve as a public oracle — it answers only allowlisted pubkeys.

## Legacy reference implementation

An earlier proof of concept (Rust server + React Native wallet, kept in
`~/reference` during development) validated the end-to-end concept but
shipped plaintext traffic on public relays, no pairing authorization, and
replaceable event kinds that clobbered concurrent requests. This repository
is a from-scratch implementation that keeps the proven shape (typed
request/response over Nostr, QR pairing, Electrs backend) and fixes the
security model. Hard-won operational patterns worth reusing are noted in
code where applied (Electrs single-flight gating, cooldowns, Umbrel
packaging, native nostr-sdk bridging).
