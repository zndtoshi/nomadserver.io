# Nomad protocol schemas

Machine-readable JSON Schemas (draft 2020-12) for the Nomad protocol v1.
The prose specification — semantics, rules, and rationale — is
[`../../docs/PROTOCOL.md`](../../docs/PROTOCOL.md); these files define the
exact wire shapes and are the canonical reference for implementations
(server, Android, iOS, third-party clients).

- `pairing-payload.schema.json` — the QR/pairing JSON served by the
  server's LAN UI (PROTOCOL.md §4.1).
- `request-envelope.schema.json` — decrypted rumor content for
  wallet → server requests (§3), with per-`type` payload validation.
- `response-envelope.schema.json` — server → wallet responses (§3), with
  per-type `result` validation.
- `notification-envelope.schema.json` — server → wallet notifications (§5.9).

Conventions:

- Envelopes are the *decrypted* content of NIP-59 gift-wrap rumors; they
  never appear on relays in plaintext.
- Implementations MUST ignore unknown fields (forward compatibility);
  schemas here use `"additionalProperties": true` deliberately on envelope
  objects, and validate the known required shape only.
- Loose address patterns are intentional — strict bech32/base58 + network
  validation happens in code (PROTOCOL.md §7), not in schema.

Change policy: any edit here is a protocol change — consider server,
Android, iOS, third-party clients, and backward compatibility, and update
`docs/PROTOCOL.md` in the same commit.
