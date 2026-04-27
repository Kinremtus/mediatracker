# AGENTS.md

# Mediatracker Project Rules

## IMPORTANT: Development Environment

Code is edited LOCALLY on laptop (~/mediatracker).
Application runs ONLY on remote server via Docker Compose.

**DO NOT** try to run docker compose locally — no .env, no nginx here.
**DO NOT** try to verify the application locally.

To check results on server: `ssh VPN_Server` (~/mediatracker)
Deployment: git push → GitHub Actions → auto-deploy to server.

Workflow:
1. Edit files locally
2. git push → GitHub Actions deploys to server
3. Verify results on server manually via ssh VPN_Server

## Critical Project Rules

- After ANY change to `models.py` → immediately:
```bash
  docker compose exec backend alembic revision --autogenerate -m "description"
  docker compose exec backend alembic upgrade head
```
- DB: user=`Kin`, db=`tracker` (NOT `postgres`, NOT `mediatracker`)
- Deploy: `docker compose up -d --build`
- Logs: `docker compose logs --tail=50 backend`
- Logs: `docker compose logs --tail=50 db`

## Project Overview

Full-stack media tracking app: React + Vite frontend, FastAPI backend, PostgreSQL, nginx reverse proxy.

## Development Commands

```bash
# Frontend
cd frontend && npm run dev       # dev server :5173
npm run build                    # builds to frontend/dist/

# Backend (все команды через docker)
docker compose up -d --build     # запуск
docker compose logs -f backend   # логи бэкенда
docker compose exec backend alembic upgrade head  # применить миграции

# Database
docker compose exec db psql -U Kin -d tracker    # psql консоль
```

## Architecture

- **Frontend**: React 19, Vite, TailwindCSS 4, shadcn/ui
- **Backend**: FastAPI, SQLAlchemy 2.0, Alembic, PostgreSQL
- **Entry points**:
  - Frontend: `frontend/src/main.tsx`
  - Backend: `backend/main.py`
- **Routers**: `backend/routers/` — auth, media, tracking, search
- **Services**: `backend/services/` — anilist, shikimori, tmdb, rawg, books, mangaupdates

## Key Conventions

- FastAPI: `redirect_slashes=False`
- CORS: `localhost:5173` и `mediatracker.web-socket-test-bench.site:2053`
- Health: `GET /health` → `{"status":"ok"}`
- Media items: composite unique `(provider, external_id)`

## Deployment

- GitHub Actions на push в `main`
- Deploy: git pull → `docker compose up --build -d` → healthcheck `:2053/health`
