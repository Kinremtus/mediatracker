# AGENTS.md

## Project Overview

Full-stack media tracking app: React + Vite frontend, FastAPI backend, PostgreSQL, nginx reverse proxy.

## Development Commands

```bash
# Frontend (React + Vite)
cd frontend && npm run dev      # dev server on localhost:5173
npm run build                   # builds to frontend/dist/
npm run lint                    # ESLint

# Backend
# Uses Docker - see docker-compose.yml
docker compose up --build       # starts backend + db + nginx
docker compose logs -f backend  # view backend logs
docker compose restart nginx    # restart nginx after frontend build

# Database migrations
cd backend && alembic upgrade head
cd backend && alembic revision --autogenerate -m "description"
```

## Architecture

- **Frontend**: React 19, Vite, TailwindCSS 4, shadcn/ui, Radix primitives
- **Backend**: FastAPI, SQLAlchemy 2.0, Alembic, PostgreSQL
- **Entry points**:
  - Frontend: `frontend/src/main.tsx`
  - Backend: `backend/main.py` (FastAPI app)
- **Routers**: `backend/routers/` - auth, media, tracking, search
- **Services**: `backend/services/` - anilist, shikimori, tmdb, rawg, books, mangaupdates

## Key Conventions

- FastAPI app has `redirect_slashes=False`
- CORS allows `localhost:5173` and `mediatracker.web-socket-test-bench.site:2053`
- Health endpoint at `/health` returns `{"status":"ok"}`
- Database uses SQLAlchemy 2.0 with `declarative_base()`
- Media items use composite unique constraint: `(provider, external_id)`
- Pyright is configured in `pyrightconfig.json` but type checking is OFF

## Deployment

- GitHub Actions deploys on push to `main` branch
- CI workflow: `cd frontend && npm ci && npm run build`
- Deploy script pulls git, runs `docker compose up --build -d`, then healthchecks `https://localhost:2053/health`

## Important Files

- `docker-compose.yml` - service orchestration
- `nginx.conf` - reverse proxy config
- `.env` - contains `SECRET_KEY`, PostgreSQL credentials (not committed)
- `backend/alembic.ini` - migration configц