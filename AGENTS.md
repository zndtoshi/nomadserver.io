# Nomad Server Agent Instructions

This repository is a monorepo for the Nomad Server ecosystem.

Workspace-level `/workspace/AGENTS.md` rules apply unless explicitly overridden here.

## Product structure

- `server/` — self-hosted Nomad Server backend and infrastructure.
- `wallet/android/` — Nomad Wallet for Android.
- `wallet/ios/` — Nomad Wallet for iOS.
- `wallet/shared/` — wallet-side concepts or generated artifacts shared across mobile platforms where appropriate.
- `shared/protocol/` — canonical protocol definitions shared by server and clients.
- `shared/schemas/` — canonical machine-readable schemas and message formats.
- `docs/` — architecture, protocol, security, decisions, and project documentation.
- `infra/` — deployment and self-hosting infrastructure.
- `tests/` — repository-level integration and end-to-end tests.

## Agent roles

### Codex — project manager and integrator

Codex owns:
- task definition and `tasks.md`
- repository coordination
- review and integration
- test/build verification
- Git workflow
- keeping server, Android, and iOS implementations synchronized

Codex should avoid implementing large application features directly when the work is appropriate for Claude.

### Claude Fable — architecture

Use Fable for:
- initial architecture
- protocol design
- security architecture
- threat modeling
- important architectural decisions
- major redesigns
- detailed implementation planning

Fable should normally produce plans and architecture documentation rather than application code.

### Claude Opus — implementation

Use Opus for:
- server implementation
- Android implementation
- iOS implementation
- tests
- refactors
- implementation of approved architecture

Opus must follow approved architectural documents and current `tasks.md`.

## Architecture rules

- Treat server and wallets as independently deployable applications.
- Do not create hidden coupling between wallet and server implementations.
- Canonical wire formats and protocol behavior belong under `shared/`.
- Android and iOS must implement the same protocol semantics.
- Do not put platform-specific implementation details into canonical protocol definitions.
- Preserve the ability for third-party wallets/apps to connect to Nomad Server.
- Nomad Server must not assume Nomad Wallet is the only client.
- Nostr-specific behavior must be explicitly documented rather than scattered implicitly through application code.
- Security-sensitive behavior must be documented in `docs/THREAT_MODEL.md`.

## Security

This project handles wallet-related and potentially sensitive data.

Agents must:
- avoid logging secrets, seeds, private keys, tokens, or sensitive payloads
- never invent cryptographic protocols
- prefer established cryptographic libraries and standards
- document trust boundaries
- treat authentication, authorization, key handling, backup, recovery, and remote access as security-critical
- stop and request architectural review for material security decisions

## Cross-component changes

Any change to a protocol, schema, API, or message format must consider:

1. server
2. Android wallet
3. iOS wallet
4. third-party client compatibility
5. migration/backward compatibility
6. tests
7. documentation

Do not modify only one side of a shared contract unless explicitly intended.

## Documentation

Important architecture decisions should be recorded under `docs/`.

Keep these documents current:

- `docs/ARCHITECTURE.md`
- `docs/PROTOCOL.md`
- `docs/THREAT_MODEL.md`
- `docs/ROADMAP.md`

## Development checkpoints

- When a coherent, verified slice of work is complete (build + tests green,
  docs in sync), commit and push it to `origin`
  (`zndtoshi/nomadserver.io`) as a checkpoint without waiting for explicit
  approval. Never push broken or half-finished work.
- Call out testable checkpoints as they arrive: moments where behavior can
  be exercised on real infrastructure (pairing UI reachable on the LAN, QR
  scanning, Electrs answers from the home node, APK install, Umbrel app
  install). Surface them to the user so they can double-check real
  behavior, not just test results.
