# MediaTracker

Self-hosted platform for tracking movies, TV shows, anime, manga, books, and games.
Keep a personal log of everything you watch, read, and play in one place.

## Features

- **Unified tracking** — single interface for movies, TV, anime, manga, manhwa, books, games, and more
- **Multiple statuses** — `in_progress`, `completed`, `planned`, `dropped`, `paused`
- **Rich metadata** — posters, descriptions, ratings from external providers
- **External providers** — TMDB (movies/TV), Shikimori & MAL (anime), MangaUpdates (manga/manhwa), RAWG & IGDB (games), Google Books & OpenLibrary (books)
- **Release schedule** — upcoming episodes/chapters in a calendar view
- **Telegram notifications** — get notified when new episodes are available
- **Search** — unified search across all media types
- **Themes** — light, graphite, dark mode
- **Session-based auth** — Argon2 password hashing, PostgreSQL sessions
- **HTMX-driven UI** — fast, no full page reloads
- **Docker Compose** — one-command deployment
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
| Containers | Docker Compose, multi-stage build |
| IaC | Terraform (Cloudflare, VULTR) |

## Quick Start

```bash
git clone https://github.com/Kinremtus/mediatracker
cd mediatracker
cp .env.example .env
# edit .env with your settings
docker compose up -d
```

The app will be available at `http://localhost:8080`.

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

## Architecture

```
┌─────────┐     ┌──────────┐     ┌──────────┐
│ Browser │────▶│  Nginx   │────▶│  Axum    │
│ (HTMX)  │     │ (static) │     │ (server) │
└─────────┘     └──────────┘     └────┬─────┘
                                      │
                               ┌──────▼──────┐
                               │ PostgreSQL  │
                               └──────┬──────┘
                                      │
                               ┌──────▼──────┐
                               │ External    │
                               │ Providers   │
                               └─────────────┘
```

## Project Structure

```
├── src/
│   ├── routes/         # HTTP handlers (media, auth, tracking, admin, …)
│   ├── services/
│   │   ├── external/   # Provider clients (TMDB, Shikimori, MAL, IGDB, …)
│   │   └── notifications/ # Telegram bot
│   ├── models/         # Database models
│   └── middleware/     # Auth, sessions, etc.
├── terraform/           # Infrastructure as Code (Cloudflare, VULTR)
├── templates/          # Askama HTML templates
├── migrations/         # SQLx migrations
├── static/             # CSS, JS, images (served by nginx)
├── k8s/                # Kubernetes manifests
├── chart/              # Helm chart
└── scripts/            # Backup, restore utilities
```

## Infrastructure as Code

The [`terraform/`](terraform/) directory contains IaC for managing cloud infrastructure:

| Provider | Resource | Purpose |
|----------|----------|---------|
| **Cloudflare** | `cloudflare_record.main` | DNS CNAME for Cloudflare Tunnel → InterServer VPS |
| **Cloudflare** | `cloudflare_record.dev` | A-record for ephemeral VULTR test VPS |
| **VULTR** | `vultr_instance`, `vultr_ssh_key` | Full lifecycle of a test VM (Docker, Ubuntu) |

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
