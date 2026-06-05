# AGENTS.md — MediaTracker

## Environment
- Code edited **locally** (~/mediatracker)
- App runs **only on remote server** via Docker Compose
- **DO NOT** run docker compose locally — no .env, no nginx
- Deploy: `git push` → GitHub Actions → auto-deploy
- Verify: `ssh VPN_Server`

## Critical Rules
- DB: user=`Kin`, db=`tracker`
- After `migrations/` changes: migrations auto-run on startup via `sqlx::migrate!()`
- Deploy: `docker compose up -d --build`
- Logs: `docker compose logs --tail=50 app|db`

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

## Project Structure
```
├── src/
│   ├── main.rs              # Entry point
│   ├── config.rs            # Config from env
│   ├── app_state.rs         # AppState (DB + services)
│   ├── routes/              # auth, home, media, search, tracking, stats, settings, calendar
│   ├── services/            # auth, search, tracking, stats, release_schedule
│   │   ├── external/        # shikimori, mal, mangaupdates, tmdb, rawg, igdb, google_books, openlibrary
│   │   └── notifications/   # telegram
│   ├── models/              # user, session, media_item, tracking_entry, stats, schedule
│   ├── middleware/           # auth
│   └── utils/               # activity_calendar
├── templates/               # Askama HTML (base, app_shell, pages, partials/)
├── static/
│   ├── css/                 # main.css, components.css, animations.css, utilities.css, themes/
│   └── js/                  # alpine.min.js, htmx.min.js, app.js
├── migrations/              # 001-007 SQL
├── docker-compose.yml       # Dev
├── docker-compose.prod.yml  # Prod (restart, limits, healthcheck)
├── nginx.conf               # SSL, gzip, security headers, rate limit
├── .env.example             # Variable template
└── scripts/                 # backup-db.sh, restore-db.sh
```

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

## Commands
```bash
# Local (if needed)
cargo check / cargo build --release

# Server
docker compose up -d --build
docker compose -f docker-compose.prod.yml up -d --build
docker compose exec db psql -U Kin -d tracker

# Backup
./scripts/backup-db.sh
./scripts/restore-db.sh backups/20240101_120000.sql.gz
```

## Deployment
- GitHub Actions on push to `main`
- Healthcheck: `GET /health` → `{"status":"ok"}`
- Nginx: 8443 (Cloudflare), SSL, TMDB image proxy
