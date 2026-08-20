# Nomad Threat Model

Status: draft · Companion to `docs/ARCHITECTURE.md` and `docs/PROTOCOL.md`.

This document is the security contract for the project. Any change to
authentication, authorization, key handling, backup/recovery, or remote
access must be checked against it, and material security decisions require
architectural review before implementation.

## Assets

| Asset | Where it lives | Impact if compromised |
|-------|----------------|------------------------|
| Wallet seed / signing keys | Phone only, platform secure storage (post–watch-only milestones) | **Total loss of funds** |
| Watch-only xpubs/addresses | Phone; replicated on the server as watch sets | Financial privacy loss |
| Wallet Nostr secret key | Phone, secure storage | Attacker can query the user's server as this wallet (privacy loss); cannot spend |
| Server Nostr secret key | Server, `0600` file | Attacker can impersonate the server, decrypt paired wallets' messages (privacy loss); cannot spend |
| Pairing secret | Server RAM only, ≤10 min TTL | Attacker who sees the QR can pair their own key (privacy loss, server abuse) |
| Server availability | Home node | Wallet loses chain access and notifications (funds never at risk) |

The fundamental safety property: **no protocol compromise can move funds.**
Bitcoin keys and signing never leave the phone, and the server never
receives anything it could sign with. The watch-only milestone goes
further: the phone holds no Bitcoin secrets at all. The worst case across
every server and relay compromise below is privacy loss and denial of
service.

## Adversaries and analysis

### 1. Relay operators and passive network observers

All protocol messages are NIP-59 gift wraps: the outer event is signed by a
random one-time key, p-tagged to the recipient, with a randomized
timestamp, and relays retain it at most until its 7-day expiration tag.

A relay therefore sees: *someone* sent an encrypted blob *to* pubkey X, of
approximate size, at approximately some time. It cannot see the sender, the
content, the message type, or the real timestamp.

**Residual leaks, stated honestly:**

- **Recipient visibility.** An observer watching relays can see that server
  pubkey Y receives messages (it is semi-public anyway) and that wallet
  inbox X receives messages. The wallet inbox is a pseudonym used for
  nothing else.
- **Timing correlation.** A global observer who can watch many relays could
  correlate a wallet→server message with a server→wallet message by timing
  and size. Defeating that requires cover traffic or mix networks — out of
  scope for v1. Randomized `created_at` blurs (not erases) the timeline.
- **IP visibility.** Relays see the connecting IP. Users wanting more can
  run Tor/VPN at the OS level; the protocol neither requires nor prevents
  it.
- **Stored ciphertext.** Gift wraps sit on relays up to 7 days. They are
  un-linkable to sender and indecipherable, but they exist; NIP-40-compliant
  relays then delete them. A future key compromise could decrypt retained
  copies — NIP-44 has no forward secrecy; accepted, documented.

A malicious relay can drop, delay, reorder, or replay messages. Signatures
prevent forgery, NIP-44 prevents reading, the envelope-id cache plus
idempotent message semantics make replays harmless, and multi-relay
redundancy limits availability attacks. A relay that partitions the pair is
indistinguishable from downtime: the wallet shows "server unreachable",
never stale data presented as fresh.

### 2. Active attacker on the LAN (pairing moment)

The server's HTTP UI is unauthenticated by design (Umbrel LAN convention).
On Umbrel it is additionally fronted by the platform's `app_proxy`, which
requires the user's Umbrel login before any page loads — that raises the
bar for LAN attackers, but the design below does not rely on it (plain
Docker deployments have no such gate).

- **Attacker fetches the QR and pairs first.** Possession of the QR *is*
  the authorization (the secret is inside it). Mitigations: QR shown only
  on demand, secret single-use and ≤10-minute TTL, pairing UI lists all
  paired wallets so unexpected pairings are visible and revocable.
  **Residual risk accepted**: LAN pairing is a one-time, user-supervised
  ceremony — same trust model as reading a Wi-Fi password off a router
  label.
- **Attacker substitutes their own server's QR.** The wallet pairs with the
  wrong server; that server learns addresses but cannot steal funds, and
  its messages fail the wallet's server-key check once the wallet is
  repaired. Mitigation: the wallet displays the paired server pubkey for
  out-of-band comparison with the server UI.
- **Attacker spams pair attempts.** Rate-limited; 5 failures burn the
  secret.

### 3. Unpaired/stranger Nostr keys (internet-wide)

Anyone can gift-wrap a message to a server. The wrap's sender is a random
key, so pre-decryption filtering is impossible; the server unwraps, checks
the seal pubkey against the allowlist, and drops strangers before any
backend work. Cost imposed on the server is one cheap decryption; the
attacker gets at most a `not_paired` response. Volume abuse is bounded by
relays themselves (they rate-limit publishing) and by server-side
per-sender buckets for paired keys. Stranger-driven CPU DoS via flood of
unwrappable wraps is a known, bounded risk (a few µs of crypto per
message); revisit if it ever becomes practical.

### 4. Compromised paired wallet key (not the seed)

If the wallet's *Nostr* key is extracted: the attacker can query the user's
own server for that wallet's data until the user revokes the pairing in the
server UI. Mitigation: wallet Nostr key in platform secure storage;
unpairing is first-class and documented.

### 5. Malicious or compromised server

The server learns everything it serves: watched addresses, balances,
history, broadcast transactions. **This is inherent** — the same trust a
user places in any Electrum server, except the server is the user's own
machine, which is the point of self-hosting.

A malicious server can lie about balances, fees, history, and can withhold
a broadcast or a notification. It **cannot** fabricate a valid transaction
or steal keys — with gift-wrapping it also cannot be impersonated to the
wallet without its secret key. Wallet mitigations: sanity-check responses
(heights monotonic, fee estimates within plausible bounds, block hashes
consistent across responses), verify received coins against known
outpoints, and never let server data influence *what* gets signed beyond
the user's own intent. SPV-style header verification to detect a lying
server is a listed future extension. A compromised server is recoverable:
re-pair with a rebuilt one.

### 6. Malicious wallet client (third-party)

The protocol is open; anyone can write a client, and a paired phone can be
malware-ridden. The server treats every message as hostile input: strict
schema validation, address validation before backend calls, length/rate
limits, and no deserialization of anything beyond signed transactions
(gated by consensus validation).

### 7. Physical device compromise

Phone rooted/stolen with unlocked secure storage → (in spending milestones)
seed exposure → funds gone. Out of protocol scope; mitigated wallet-side
(Keystore/Keychain backing, `allowBackup=false`, optional passphrase, no
screenshots of seed screens, seed never logged — the legacy prototype
logged the mnemonic to logcat; that class of bug is explicitly banned). In
the watch-only milestone the phone holds no funds-bearing secret.

## Key handling requirements (normative)

- **Wallet**: seed (when present) and Nostr secret in platform secure
  storage (Keystore/Keychain), never in AsyncStorage/plain files, never
  logged, never sent over the protocol. `android:allowBackup="false"`.
  Private key material stays inside signer objects — never passed as
  strings through layers that might log them.
- **Server**: single Nostr secret file, mode `0600`, owned by the service
  user, under the app data dir (no hardcoded `/data`). Never logged; only
  the *public* key appears in HTTP responses.
- **Pairing secret**: RAM only, single-use, ≤10 min TTL, CSPRNG-generated
  (32 bytes), constant-time comparison, never logged.
- **No invented cryptography.** NIP-44 v2 / NIP-59 gift-wrapping via
  audited nostr-sdk implementations. Pairing proof is plain HMAC-SHA-256.
  Nothing else.

## Logging rules (normative)

- Never log: mnemonics, private keys, pairing secrets, decrypted envelope
  content.
- Log at most: pubkeys, envelope ids, message *types*, error codes, backend
  timings. Addresses/txids at DEBUG only, off by default.

## Assumptions and non-goals

- The user controls the machine the server runs on; physical/SSH access to
  the server is out of scope (game over for privacy, never for funds).
- We do not defend against global passive adversaries correlating timing
  across relays (see §1 residual leaks).
- Relay availability is best-effort; the wallet degrades to "server
  unreachable" rather than silently falling back to an untrusted chain-data
  source. Any optional fallback (e.g. public Esplora) must be an explicit
  user choice with the privacy tradeoff shown.

## Legacy prototype findings this design answers

For the record, the earlier proof-of-concept (reference only) violated
nearly every rule above: plaintext JSON on public relays, mnemonic logged
to logcat, secrets in AsyncStorage, no allowlist (open oracle), replaceable
event kinds clobbering requests. This document exists so those specifics
are never reintroduced; code review should treat any regression toward them
as a release blocker.
