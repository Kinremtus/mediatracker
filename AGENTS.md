# AGENTS.md — MediaTracker

## Environment
- Code edited **locally** (~/mediatracker)
- App runs **on k3s (Kubernetes)** on VPS1
- **DO NOT** run docker compose locally — no .env, no nginx
- Deploy: `git push` → GitHub Actions (self-hosted runner) → auto-deploy
- Verify: `ssh VPS1`

## Critical Rules
- DB: user=`Kin`, db=`tracker`, runs as StatefulSet in k3s
- After `migrations/` changes: migrations auto-run on startup via `sqlx::migrate!()`
- Deploy: CI runs → `helm upgrade --install app chart/ -n mediatracker`
- Logs: `kubectl logs -n mediatracker deployment/app` / `kubectl logs -n mediatracker statefulset/postgres`

## Stack
Rust 1.95 · Axum 0.8 · SQLx 0.8 · Askama 0.16 · PostgreSQL 17 · Alpine.js · HTMX

## K8s/DevOps Teaching
При объяснении K8s манифестов, конфигураций, инструментов:
- Никогда не используй dot-нотацию (`spec.selector.app:`) — пиши сразу YAML с отступами
- Перед первым появлением дефиса (`-`) объясни что это массив и почему
- Каждое новое поле объясняй до того как покажешь в коде: что делает, зачем нужно
- Всегда показывай схему/связь объектов (Pod → PVC → PV, app → Service → StatefulSet)
- Не добавляй поля без объяснения (targetPort, mountPath, storageClassName)
- После каждой сессии сохраняй заметку в Obsidian в соответствующей поддиректории DevOps/
- Формат: что сделали → как работает → схема → команды

## Rust Teaching
Если пользователь просит объяснить Rust код — объясняй построчно, каждую конструкцию.
Формат: что делает → синтаксис → аналогия из Python/JS/C.
Покрывай: `let`/`mut`, `&`/`&mut`, `Result`/`Option`, трейты, макросы, итераторы.
После объяснения — таблица-резюме всех концепций в коде.

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
- Скрипты: `scripts/` (backup, restore, backfill, build-deploy, validate-k8s-yaml)

## CI / Deploy
- Workflow: `.github/workflows/main.yml` (self-hosted runner, `runs-on: self-hosted`)
- Pipeline:
  1. **check**: clippy + `cargo test` + `cargo audit` + validate K8s YAML
  2. **build**: Docker buildx (with GHA cache) → push to GHCR (`ghcr.io/kinremtus/mediatracker:latest`)
  3. **deploy**: SSH to VPS1 → `git reset --hard origin/main` → `kubectl apply` (ingress, monitoring) → `helm upgrade --install app chart/ -n mediatracker`
- Runner: installed at `/home/Kinremtus/actions-runner/` on laptop (self-hosted)

## Infrastructure
- Kubernetes: k3s on VPS1
- Registry: GHCR (ghcr.io/kinremtus/mediatracker)
- Ingress: Traefik (k8s/traefik-helm-config.yaml + ingress.yaml)
- Cloudflare Tunnel → Traefik → app (port 8080)
- Monitoring: k8s/monitoring/ (applied on every deploy)
- Postgres: StatefulSet in cluster (not external)
- Healthcheck: `GET /health` → `{"status":"ok"}`
- Helm chart: `chart/` (templates, values.yaml)

## Commands
```bash
# Local
cargo check / cargo build --release

# Server — logs
kubectl logs -n mediatracker deployment/app
kubectl logs -n mediatracker statefulset/postgres
kubectl logs -n mediatracker deployment/app --tail=50 -f

# Server — DB
kubectl exec -n mediatracker statefulset/postgres -- psql -U Kin -d tracker

# Server — restart / rollout
kubectl rollout restart deployment/app -n mediatracker
kubectl rollout status deployment/app -n mediatracker

# Server — helm
sudo helm --kubeconfig /etc/rancher/k3s/k3s.yaml upgrade --install app chart/ -n mediatracker

# Manual build+deploy
./scripts/build-deploy.sh [tag] [ssh-host]
```

## Scripts Status
| Script | k3s compatible? | Notes |
|--------|----------------|-------|
| `build-deploy.sh` | ✅ | Uses kubectl + helm |
| `validate-k8s-yaml.py` | ✅ | Lints k8s/ YAML files |
| `backup-db.sh` | ❌ | Uses `docker compose exec` — needs update for k3s |
| `restore-db.sh` | ❌ | Uses `docker compose exec` — needs update for k3s |
| `backfill-details.sh` | ? | Untested with k3s |
