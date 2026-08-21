# Building

## Server (`server/`)

Rust 1.85+ (stable). No system dependencies beyond a C toolchain for
`secp256k1`/`aws-lc-sys` builds.

```sh
cd server
cargo test                 # unit tests
cargo run                  # dev server on 0.0.0.0:3829
```

Config (all env, all optional): `NOMAD_DATA_DIR` (default `./data`),
`ELECTRS_ADDR` (default `electrs:50001`), `NOSTR_RELAYS` (csv),
`NOMAD_HTTP_PORT` (default 3829), `NOMAD_NETWORK` (default bitcoin),
`NOMAD_WATCH_INTERVAL_SECS` (default 60).

Dev harnesses:

```sh
python3 scripts/mock_electrum.py 50001        # canned Electrum backend
cargo run --example pair_client               # pairing handshake
cargo run --example pair_client -- <pairing-url> <address>   # + chain probes
cargo run --example notify_probe -- <pairing-url> <address>  # watcher probe
```

Docker: `docker build -t nomad-server ./server` (multi-stage, slim).
Umbrel packaging: see `infra/README.md`.

## Android wallet (`wallet/android/`)

Requirements: JDK 17, Android SDK (platform 35, build-tools 35),
Gradle 8.13 (wrapper TBD; any 8.13+ works).

```sh
cd wallet/android
gradle :app:assembleDebug    # APK at app/build/outputs/apk/debug/
```

Set `ANDROID_HOME` (or `sdk.dir` in the gitignored `local.properties`).

Pinned dependency coordinates:

- `org.rust-nostr:nostr-sdk:0.44.8` (UniFFI Android bindings; NIP-44/59)
- `org.bitcoindevkit:bdk-android:3.0.0` (bdk_wallet 3.x core)
- AGP 8.13.2, Kotlin 2.3.21, Compose BOM 2025.10.00 (newer BOMs require
  AGP 9 — do not bump blindly)
- CameraX 1.4.2 + `com.google.mlkit:barcode-scanning:17.3.0` (pairing QR
  scan; bundled model, no Play Services dependency; CameraX held at 1.4.x
  because 1.6.x requires compileSdk 36)

## Protocol

Everything on the wire is defined in `docs/PROTOCOL.md` with
machine-readable schemas in `shared/schemas/v1/`. If you change one,
change the other in the same commit.
