# Deployment

How Nomad Server reaches users. Two tracks, both FOSS-friendly.

## Track 1 — Umbrel (primary)

Package source of truth: `infra/umbrel/nomad-server/`
(`umbrel-app.yml`, `docker-compose.yml`, `data/`). The contract follows the
official packaging guide (`getumbrel/umbrel-apps` repo, `.claude/skills`):
multi-arch prebuilt images pinned by digest, `${APP_DATA_DIR}` persistence,
`app_proxy` in front of the web UI with Umbrel auth **enabled** (the
pairing QR sits behind the Umbrel login), `electrs` as the only dependency
(`APP_ELECTRS_NODE_IP`/`_PORT` are injected by umbrelOS).

### Phase 1 — community app store (fast iteration)

1. Tag a release: `git tag v0.1.0 && git push --tags`.
   `.github/workflows/release.yml` builds `linux/amd64` + `linux/arm64`
   and pushes `ghcr.io/zndtoshi/nomad-server:0.1.0`.
2. Copy the printed multi-arch index digest into
   `infra/umbrel/nomad-server/docker-compose.yml`
   (`image: ghcr.io/zndtoshi/nomad-server:0.1.0@sha256:<digest>`).
3. Sync `infra/umbrel/nomad-server/` into the community app-store repo
   (`zndtoshi/umbrel-app-store`, one top-level dir per app).
4. On the Umbrel: Settings → App Store → Community App Stores → add the
   repo URL → install Nomad Server like any app.

### Phase 2 — official App Store

1. Same release flow; package already pinned and digest-verified.
2. Fork `getumbrel/umbrel-apps`, add the `nomad-server/` directory from
   `infra/umbrel/`, run `npm run lint:apps -- nomad-server --check-images`
   and `git diff --check`.
3. PR with screenshots + logo in the body (Umbrel team hosts final icon and
   gallery assets). Set `submission:` in `umbrel-app.yml` to the PR URL.

## Track 2 — plain Docker (non-Umbrel)

```sh
cd infra/standalone
ELECTRS_ADDR=my-electrum-host:50001 docker compose up -d
```

The UI listens on `0.0.0.0:3829` — put it behind your own auth/reverse
proxy; on plain Docker there is no Umbrel login protecting the pairing
page, so keep it LAN-only.

## Image supply chain

- GHCR (`ghcr.io/zndtoshi/nomad-server`) — free for public repos, no rate
  limits, permissions via `GITHUB_TOKEN` (see `.github/workflows/release.yml`).
- Versioned tags only, never `latest`; Umbrel packages pin
  `tag@sha256:<multi-arch index digest>`.
- `provenance: false` / `sbom: false` in buildx so the tag digest is a
  plain multi-arch index (what Umbrel's linter expects).
