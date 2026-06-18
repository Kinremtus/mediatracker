# AGENTS.md — MediaTracker

## Environment
- Code edited **locally** (~/mediatracker)
- App runs **only on remote server** via Docker Compose
- **DO NOT** run docker compose locally — no .env, no nginx
- Deploy: `git push` → GitHub Actions → auto-deploy
- Verify: `ssh VPS1`

## Critical Rules
- DB: user=`Kin`, db=`tracker`
- After `migrations/` changes: migrations auto-run on startup via `sqlx::migrate!()`
- Deploy: `sudo docker compose up -d --build`
- Logs: `sudo docker compose logs --tail=50 app|db`

## Stack
Rust 1.95 · Axum 0.8 · SQLx 0.8 · Askama 0.16 · PostgreSQL 17 · Alpine.js · HTMX

## Caching & Cache-Bust
- **Static files** (JS/CSS/images under `/static/`): nginx serves them and sets
  `Cache-Control: no-cache, must-revalidate` + `ETag` + `Last-Modified`. Browser
  revalidates every request; nginx returns 304 (cheap) for unchanged files,
  200 with new body for changed ones. **No `?v=hash` in URLs, no build.rs.**
- **HTML pages** (server-rendered by axum): set to `no-cache` by the response
  from the app; browser always revalidates. Dynamic content (drawer, htmx
  swaps) is always fresh.
- **TMDB images** (proxied via `/tmdb-image/`): cached 7 days with
  `Cache-Control: public, immutable` (intentional — TMDB image URLs are
  content-addressed, never change).

## External Providers
| Type | Provider |
|------|----------|
| anime | Shikimori + MAL (Jikan v4) |
| manga/manhwa/manhua/novels/comics | MangaUpdates |
| movies/tv/dramas/cartoons | TMDB |
| games | RAWG + IGDB |
| books | Google Books + OpenLibrary |

## Key Conventions
- Auth: Session-based (PostgreSQL), cookie `session_id` (HttpOnly, Secure, SameSite=Lax)
- Passwords: Argon2 hashing
- Media: composite unique `(provider, external_id)`
- Statuses: `in_progress`, `completed`, `planned`, `dropped`, `paused`
- Themes: light, graphite (default), dark → `localStorage['mediatracker-theme']`
- HTMX: `/tracking/partial`, `/tracking/{id}/htmx`, `/settings/*/htmx`
- Telegram: `TELEGRAM_BOT_TOKEN` in `.env`, notifications via `notify_new_episodes()`
- JS: HTMX-first (server-rendered). Alpine.js only when unavoidable. No custom JS/TS.
- Routes: `src/routes/<domain>.rs`, HTML-шаблоны в `templates/`
- External API клиенты: `src/services/external/<provider>.rs`
- Статика: `static/css/`, `static/js/` (nginx, ETag cache-bust)
- Миграции: `migrations/*.sql`, авто-применение при старте
- Скрипты: `scripts/` (backup, restore, backfill)

## CI / Deploy
- Workflow: `.github/workflows/main.yml`
- Docker buildx с локальным кэшем (быстрые пересборки)
- Cloudflare Tunnel → nginx (port 80) → app (8080)
- Healthcheck: `GET /health` → `{"status":"ok"}`

## Commands
```bash
# Local (if needed)
cargo check / cargo build --release

# Server
sudo docker compose up -d --build
sudo docker compose -f docker-compose.prod.yml up -d --build
sudo docker compose exec db psql -U Kin -d tracker

# Backup
./scripts/backup-db.sh
./scripts/restore-db.sh backups/20240101_120000.sql.gz
```
