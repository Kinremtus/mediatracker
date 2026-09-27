# MediaTracker

Self-hosted platform for tracking movies, TV shows, anime, manga, books, and games.
Keep a personal log of everything you watch, read, and play in one place.

## Features

- **Unified tracking** — single interface for movies, TV, anime, manga, manhwa, books, games, and more
- **Multiple statuses** — `in_progress`, `completed`, `planned`, `dropped`, `paused`
- **Rich metadata** — posters, descriptions, ratings from external providers
- **External providers** — TMDB (movies/TV), Shikimori & MAL (anime), MangaUpdates + MangaDex (manga/manhwa/manhua/novels), Comic Vine (western comics), RAWG & IGDB (games), Google Books & OpenLibrary (books)
- **Release schedule** — upcoming episodes/chapters in a calendar view
- **Telegram notifications** — get notified when new episodes are available
- **Search** — unified search across all media types
- **Themes** — light, graphite, dark mode
- **Session-based auth** — Argon2 password hashing, PostgreSQL sessions
- **HTMX-driven UI** — fast, no full page reloads
- **GitOps deployment** — the image is built by CI and rolled out to k3s with Helm
- **Statistics** — track your consumption over time

## Tech Stack

| Layer | Tech |
|-------|------|
| Language | Rust 1.95 |
| Web framework | Axum 0.8 |
| Database | PostgreSQL 17 via SQLx 0.8 |
| Templates | Askama 0.16 |
| Frontend | HTMX + Alpine.js |
| Auth | Argon2, session-based (PostgreSQL) |
| Container | Docker (multi-stage build) |
| Runtime | Kubernetes (k3s) + Helm |
| Ingress | Traefik, published through Cloudflare Tunnel |
| Registry | GHCR (`ghcr.io/kinremtus/mediatracker`) |
| CI/CD | GitHub Actions (self-hosted runner) |
| IaC | Terraform (Cloudflare, VULTR, monitoring) |

## Quick Start (development)

```bash
git clone https://github.com/Kinremtus/mediatracker
cd mediatracker
cp .env.example .env   # DATABASE_URL, COOKIE_SECRET, provider keys
cargo run              # requires a reachable PostgreSQL 17
```

The app listens on `http://localhost:8080`.

Static assets are served by the application itself — nginx is no longer part of
the stack. Browsers revalidate them on every request through `ETag` /
`Last-Modified`, so no cache-busting query strings are needed.

### Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `COOKIE_SECRET` | Yes | — | Secret for session cookies |
| `TMDB_API_KEY` | No | — | For movie/TV metadata |
| `SHIKIMORI_API_KEY` | No | — | For anime metadata |
| `TELEGRAM_BOT_TOKEN` | No | — | For Telegram notifications |
| `RAWG_API_KEY` | No | — | For game metadata |
| `IGDB_CLIENT_ID` | No | — | For game metadata (IGDB) |
| `IGDB_CLIENT_SECRET` | No | — | For game metadata (IGDB) |
| `GOOGLE_BOOKS_API_KEY` | No | — | For book metadata |
| `MANGAUPDATES_API_KEY` | No | — | For manga metadata |
| `COMIC_VINE_API_KEY` | No | — | For western comic metadata (Comic Vine) |

MangaDex needs no API key — it is used for chapter enrichment only.

## Deployment

Production runs on k3s (single VPS) and is driven entirely by `git push`:

1. **check** — `cargo clippy -D warnings`, `cargo test`, `cargo audit`, plus k8s YAML validation
2. **build** — Docker buildx pushes `ghcr.io/kinremtus/mediatracker:latest` to GHCR
3. **deploy** — the self-hosted runner SSHs to the VPS and runs
   `helm upgrade --install app chart/ -n mediatracker`

PostgreSQL also runs in-cluster: it is a StatefulSet managed by the same Helm
chart, with the image pinned by digest. Monitoring is applied separately from
`terraform/monitoring/` and is intentionally not part of the application deploy
— the deploy only touches `k8s/` directories that actually exist.

## Architecture

```
+---------+   +-------------+   +----------+   +-----------+
| Browser |-->| Cloudflare  |-->| Traefik  |-->|   Axum    |
| (HTMX)  |   |   Tunnel    |   | ingress  |   |  server   |
+---------+   +-------------+   +----------+   +-----+-----+
                                                     |
                                              +------v-------+
                                              |  PostgreSQL  |
                                              | (StatefulSet)|
                                              +------+-------+
                                                     |
                                              +------v-------+
                                              |   External   |
                                              |   Providers  |
                                              +--------------+
```

## Project Structure

```
├── src/
│   ├── routes/         # HTTP handlers (media, auth, tracking, admin, …)
│   ├── services/
│   │   ├── external/   # Provider clients (TMDB, Shikimori, MAL, IGDB, …)
│   │   └── notifications/ # Telegram bot
│   ├── models/         # Database models
│   ├── middleware/     # Auth, sessions, security headers, rate limiting
│   └── bin/            # One-off backfill utilities
├── terraform/          # Infrastructure as Code (Cloudflare, VULTR, monitoring)
├── templates/          # Askama HTML templates
├── migrations/         # SQLx migrations, applied automatically on startup
├── static/             # CSS, JS, images (served by the app, ETag revalidation)
├── k8s/                # Manifests applied by CI (ingress, backup, cloudflared)
├── chart/              # Helm chart (app, postgres, PDB, network policies)
├── scripts/            # Backup, restore, backfill and deploy helpers
└── thoughts/           # Design docs, plans and session ledgers
```

## Backups

`scripts/backup-db.sh` dumps the cluster database (`pg_dump -Fc`) by running
`kubectl exec` against `statefulset/postgres`, encrypts the dump with `age` and
keeps 7 days of history locally. Restore with `scripts/restore-db.sh`
(requires `CONFIRM=yes`), and prove a dump is usable with
`scripts/restore-drill.sh`, which restores the newest dump into an ephemeral
PostgreSQL 17 container and compares row counts.

In the cluster the same job runs as a CronJob from `k8s/backup/`
(CronJob + PVC + encrypted secret). Retention, RPO/RTO targets and the
open offsite-copy decision are documented in
[`k8s/backup/README.md`](k8s/backup/README.md).

## Infrastructure as Code

The [`terraform/`](terraform/) directory contains IaC for managing cloud infrastructure:

| Provider | Resource | Purpose |
|----------|----------|---------|
| **Cloudflare** | `cloudflare_record.main` | DNS CNAME for Cloudflare Tunnel → InterServer VPS |
| **Cloudflare** | `cloudflare_record.dev` | A-record for ephemeral VULTR test VPS |
| **VULTR** | `vultr_instance`, `vultr_ssh_key` | Full lifecycle of a test VM (Docker, Ubuntu) |
| **Terraform** | `monitoring/` | Prometheus/Grafana stack, applied separately from the app deploy |

Key Terraform patterns used:
- **`terraform import`** — adopt existing resources under management
- **`terraform destroy -target`** — selective teardown of specific resources
- **`user_data`** — bootstrap Docker on VPS at first boot
- **`.terraform.lock.hcl`** — pin provider versions (same as `Cargo.lock`)

All Terraform state is local (`terraform.tfstate`), provider binaries are gitignored.

## License

[AGPL-3.0-only](LICENSE)

Copyright (C) 2026 Kinremtus

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as published
by the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

For commercial licensing options, contact the author.
