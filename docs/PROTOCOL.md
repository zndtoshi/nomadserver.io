# Nomad Protocol

Version 1 · Status: draft

Canonical specification of the wire protocol between a Nomad wallet (or any
compatible client) and a Nomad Server. Everything here is platform-neutral;
implementations live in `server/`, `wallet/android/`, `wallet/ios/`, and the
machine-readable schemas in `shared/schemas/`.

Design goals, in order:

1. **Private** — relay operators and observers learn neither content nor
   *who is talking to whom*, beyond the minimum Nostr requires.
2. **Authenticated** — the server answers only paired wallets; the wallet
   accepts only its own server's messages.
3. **Simple** — a third-party wallet should be able to implement this from
   this document alone.
4. **Asynchronous** — the server can reach the wallet while it is offline
   (notifications); the wallet can reach an always-on server any time.

## 1. Identities

Both sides hold a long-lived Nostr identity: a secp256k1 keypair.

- The **server pubkey** is the server's identity and the pairing target. Its
  secret key lives only on the server (`nostr_secret`, file mode `0600`).
- The **wallet pubkey** identifies one wallet installation to one server.
  It is not the user's public Nostr identity and MUST NOT be reused for
  anything else.

Bitcoin keys are entirely separate and never appear in this protocol. The
only Bitcoin material on the wire is *signed* transactions in
`broadcast_tx`. Watch-only clients supply only addresses/xpub-derived
addresses — never private material.

## 2. Transport

### 2.1 Relays

Both sides connect outbound-only to a configured set of public relays
(NIP-01 `wss://`). Recommended: 3–5. The pairing QR carries the server's
relay set; both sides publish and subscribe on exactly that set.
Implementations SHOULD reconnect and resubscribe aggressively; relays are
assumed flaky.

### 2.2 Every message is a gift wrap

All protocol messages — requests, responses, and notifications — are
[NIP-59](https://github.com/nostr-protocol/nips/blob/master/59.md) gift
wraps in the style of [NIP-17](https://github.com/nostr-protocol/nips/blob/master/17.md):

```
gift wrap (kind 1059, signed by a fresh random key, p-tag = recipient,
           created_at randomized up to 2 days in the past)
  └─ seal (kind 13, signed by the sender's REAL key, no tags)
       └─ rumor (unsigned; content = the JSON envelope, §3)
```

Rumor kinds:

| Rumor kind | Direction        | Meaning      |
|------------|------------------|--------------|
| 25078      | wallet → server  | request      |
| 25079      | server → wallet  | response     |
| 25080      | server → wallet  | notification |

These kinds appear only as rumors inside seals; they are never published
directly to relays.

Rules:

- The gift wrap SHOULD carry an `expiration` tag (NIP-40) of
  `created_at + 9 days` (the wrap's `created_at` is randomized up to 2
  days in the past, so this is ≈7 days from real send time); the seal
  SHOULD carry the same. Relays that honor NIP-40 then retain nothing
  long-term.
- On receipt: verify the wrap signature, decrypt the wrap, verify the seal
  signature, decrypt the seal, and **check the rumor pubkey equals the seal
  pubkey** (otherwise drop — this is the NIP-17 anti-impersonation rule).
  The sender's identity is the seal/rumor pubkey, never the wrap's.
- The envelope `ts` is the real timestamp (the wrap's `created_at` is
  deliberately randomized and MUST NOT be used for anything).

Why gift wraps instead of plain p-tagged events:

- **Sender is hidden.** The wrap is signed by a random one-time key, so an
  observer cannot link a message to the wallet's or the server's key —
  only the *recipient* (`p` tag) is visible. The server pubkey is
  semi-public anyway; the wallet's inbox stays a pseudonym.
- **Offline delivery.** Kind 1059 is a stored kind: the server can notify a
  wallet that is currently offline, and a wallet's request can be answered
  while the app is backgrounded. Receivers pick up missed messages with a
  `since` query on reconnect.
- Expiration tags bound how long relays hold the (un-linkable) ciphertext.

Deviation from NIP-17: we reuse its encryption/envelope construction but
not its social-DM routing — there is no kind-10050 inbox relay list (the
pairing relay set *is* the inbox), and rumor content is our typed envelope
rather than chat text.

### 2.3 Subscriptions

- Wallet subscribes: `{kinds: [1059], "#p": [<wallet pubkey>], "since": <now - 3 days>}`
- Server subscribes: `{kinds: [1059], "#p": [<server pubkey>], "since": <now - 3 days>}`

The `since` window is a backfill, not a watermark: kind 1059 events are
stored, and a wrap's `created_at` is randomized up to 2 days in the past,
so a fixed 3-day window (tweak + margin) both covers the randomization
and re-delivers messages missed while either side was disconnected.
Backfilled delivery MUST NOT be treated as fresh: dedupe happens at the
envelope-id level (§2.4), and response correlation is by envelope `id`
(§3). Live-only (`limit: 0`) subscriptions are NOT sufficient — a message
published while the receiver is reconnecting would be lost. Implementations
SHOULD wait for relay connections to settle before publishing.

A wallet drops any unwrapped message whose seal pubkey is not its paired
server's — except during pairing (§4), when it expects the server's key
from the QR. A server drops messages whose seal pubkey is not allowlisted,
except `pair` requests (§4).

Spam note: wrap keys are random, so relays cannot reputation-filter them;
receivers will occasionally unwrap garbage that fails decryption. Unwrap is
cheap; failures are discarded silently.

### 2.4 Replay and idempotency

Replay defense is the envelope `id`, not timestamps (offline delivery means
legitimate messages can arrive days late):

- Receivers cache seen envelope `id`s for at least **10 days** (gift-wrap
  retention + margin) and silently drop duplicates.
- All message types are idempotent by design: re-running `get_balance`
  returns the same data; re-broadcasting the same tx is the same tx;
  `watch_addresses` uses replace semantics; `pair` fails on a burned
  secret; `unpair` fails when already unpaired. A replayed message past the
  cache window is therefore harmless.

## 3. Envelope

Decrypted rumor content is one JSON object.

Request:

```json
{
  "v": 1,
  "id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
  "ts": 1755613610,
  "type": "get_utxos",
  "payload": { "addresses": ["bc1q..."] }
}
```

Response:

```json
{ "v": 1, "id": "7c9e6679-7425-40de-944b-e07fc1f90ae7", "ok": true,
  "result": { "utxos": [] } }
```

or

```json
{ "v": 1, "id": "7c9e6679-7425-40de-944b-e07fc1f90ae7", "ok": false,
  "error": { "code": "rate_limited", "message": "slow down" } }
```

Notification (server → wallet, no response expected):

```json
{ "v": 1, "id": "...", "ts": 1755613610, "type": "notify",
  "payload": { "kind": "new_tx", "txid": "...", "addresses": ["bc1q..."] } }
```

Rules:

- `v` is the protocol version (`1`). Unknown `v` → `unsupported_version`.
- `id` is a random 128-bit UUID (v4, CSPRNG — never `Math.random()`).
  Responses echo the request `id`; correlation is by `id` alone.
- Unknown `type` → `unknown_type`. Unknown fields MUST be ignored.
- `ts` is informational (real send time); not a replay defense (§2.4).

## 4. Pairing

Pairing registers a wallet pubkey in the server's allowlist. It happens
once, on the user's LAN, via QR code shown by the server's local HTTP UI.

### 4.1 Pairing payload (QR content)

```json
{
  "v": 1,
  "app": "nomad-server",
  "nodePubkey": "<64 hex chars>",
  "relays": ["wss://relay.damus.io", "..."],
  "pairSecret": "<32 bytes, base64url>",
  "exp": 1755613910
}
```

- `pairSecret` is generated fresh each time the pairing page is loaded,
  valid for **one** successful pairing, expires at `exp` (≤ 10 minutes from
  creation), and lives only in server memory. Never logged.
- The UI also offers the JSON as copyable text (manual fallback).
- Possession of the QR is the authorization — treat the pairing page like a
  password prompt: LAN only, shown on demand.

### 4.2 Handshake

1. Wallet scans QR, validates (`app`, `v`, expiry, pubkey format, ≥1
   relay).
2. Wallet connects to the listed relays and sends a gift-wrapped `pair`
   request to `nodePubkey`:

   ```json
   { "v": 1, "id": "...", "ts": ..., "type": "pair",
     "payload": { "proof": "<hex HMAC-SHA256(key = pairSecret, msg = wallet pubkey hex)>",
                  "client": "nomad-wallet-android/0.1.0" } }
   ```

   The wallet's identity is the seal pubkey — never a payload field.
3. Server checks in order: secret exists and unexpired → HMAC valid →
   pubkey not already allowlisted. On success it atomically burns the
   secret, persists the wallet pubkey in the allowlist, and replies
   `ok: true`, `result: { "server": "nomad-server/x.y.z" }`.
4. Any failure → `pairing_failed` with no detail about which check (no
   secret oracle). Attempts are rate-limited; after 5 failures the secret
   is burned and the user reloads the QR.

### 4.3 Unpairing

`type: "unpair"`, `payload: {}` from an allowlisted key removes the sender
and returns `ok: true`. The server UI also lists paired wallets and can
revoke any of them. Unpairing also deletes that wallet's watch set (§5.8).

## 5. Message types (v1)

All requests require an allowlisted sender; `pair` is the only exception.

### 5.1 `ping`

`payload: {}` →
`result: { "server": "nomad-server/x.y.z", "tip": { "height": 812345, "hash": "0000...", "time": 1755610000 } }`

Liveness plus chain-tip check. The tip data doubles as the chain checkpoint
wallets need for sync.

### 5.2 `get_balance`

`payload: { "addresses": ["bc1q...", ...] }` →
`result: { "confirmed": 123456, "unconfirmed": 0 }` (satoshis, summed)

### 5.3 `get_utxos`

`payload: { "addresses": [...] }` →
`result: { "utxos": [{ "txid": "...", "vout": 0, "value": 123456,
"address": "bc1q...", "confirmations": 12 }] }`

### 5.4 `get_history`

`payload: { "addresses": [...], "limit": 50 }` →
`result: { "txs": [{ "txid": "...", "height": 812000, "block_hash": "0000...",
"block_time": 1755600000 }] }`

`height: 0` with absent `block_hash`/`block_time` means unconfirmed. Block
hash and time are included so wallets can build confirmation anchors
without extra round trips.

### 5.5 `get_transactions`

`payload: { "txids": ["...", ...] }` →
`result: { "txs": [{ "txid": "...", "hex": "0200...", "height": 812000,
"block_hash": "0000...", "block_time": 1755600000 }] }`

Full raw transactions for wallet-side balance/history computation (a BDK
`Update` needs full txs plus anchors). Capped per §7.

### 5.6 `get_fee_estimates`

`payload: {}` →
`result: { "fast": 12, "medium": 6, "slow": 2 }` (sat/vB; ~1/~6/~12 blocks)

### 5.7 `broadcast_tx`

`payload: { "txHex": "0200..." }` → `result: { "txid": "..." }`

The server consensus-deserializes before broadcast; malformed →
`invalid_tx`; node rejection → `broadcast_failed` with the node's message.
(Not used by watch-only clients; part of v1 for completeness.)

### 5.8 `watch_addresses`

`payload: { "addresses": ["bc1q...", ...] }` →
`result: { "watching": 1234 }`

**Replace semantics**: the given set replaces the sender's entire previous
watch set. The server persists the set per wallet pubkey and monitors it
for new transactions (implementation: periodic Electrs poll or scripthash
subscriptions). Cap: 5000 addresses per wallet. Wallets re-assert the set
after every sync, which keeps the server stateless-tolerant: if the server
loses state, the wallet's next sync restores it.

### 5.9 `notify` (server → wallet)

No response. `payload.kind` values:

- `"new_tx"`: `{ "txid": "...", "addresses": ["..."], "height": 812000 }` —
  a transaction touching the wallet's watch set was first seen (mempool)
  or newly confirmed. `height: 0`/absent = mempool.
- `"health"`: **reserved** (v1.1 watchdog: node/Electrs/disk alerts).
  Servers MUST NOT send it in v1; wallets MUST ignore unknown kinds.

### 5.10 `unpair`

See §4.3.

## 6. Error codes

| code                  | meaning                                       |
|-----------------------|-----------------------------------------------|
| `unsupported_version` | `v` not supported                             |
| `unknown_type`        | `type` not recognized                         |
| `not_paired`          | sender not in allowlist                       |
| `pairing_failed`      | pair request rejected (any reason)            |
| `invalid_request`     | schema/validation failure                     |
| `invalid_tx`          | transaction failed to deserialize             |
| `broadcast_failed`    | node rejected the broadcast                   |
| `backend_unavailable` | Electrs/bitcoind unreachable                  |
| `rate_limited`        | sender exceeded limits                        |
| `internal`            | anything else                                 |

`message` is human-readable, never sensitive.

## 7. Limits and validation (server-enforced)

- ≤ 200 addresses per `get_balance` / `get_utxos` / `get_history`
- ≤ 25 txids per `get_transactions`; ≤ 100 KB per transaction hex
- ≤ 5000 watched addresses per wallet
- Per-wallet: ≤ 60 requests/minute, burst 10 (token bucket)
- Pairing: secret TTL ≤ 10 min, ≤ 5 failed attempts
- All strings length-capped; addresses validated (bech32/bech32m/base58,
  correct network) before any backend call
- Global single-flight gate to the Electrs backend with per-call timeouts
  and a cooldown circuit-breaker, so a flaky backend fast-fails instead of
  queueing unbounded work
- The sender check happens right after unwrap, before any backend work;
  messages from non-allowlisted keys get at most a `not_paired` response

## 8. Relay requirements

- Relays must store regular kinds (1059), support `#p` filters and `since`
  queries, and ideally honor NIP-40 expiration. Common public relays
  qualify.
- Implementations MUST NOT rely on relay retention beyond the 7-day
  expiration, and MUST tolerate duplicate, delayed, and reordered delivery.
- Neither side requires NIP-42 relay authentication in v1.

## 9. Versioning and future extensions

- New optional fields may appear in any payload/result at `v: 1`;
  implementations MUST ignore unknown fields.
- New message/notification types at `v: 1` must be safe to ignore (older
  servers return `unknown_type`; wallets ignore unknown notify kinds).
  Feature-detect via `ping`.
- Breaking changes bump `v`; the envelope version is per-message.
- Considered, not in v1:
  - `"health"` notifications and watchdog instruction types (v1.1)
  - `scripts[]` (raw scriptPubKey hex) alongside `addresses[]`, for
    script types without address encodings (bare multisig etc.)
  - multi-server pairing per wallet; server redundancy
  - SPV-style header verification so wallets can detect a lying server
  - NIP-42 relay authentication; optional private/self-hosted relay
