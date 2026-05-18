# AGENTS.md

# Mediatracker Project Rules (Rust Edition)

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

- After ANY change to `migrations/` → immediately:
```bash
  docker compose exec db psql -U Kin -d tracker -c "SELECT 1" # Verify connection
  # Migrations run automatically on app startup via sqlx::migrate!()
```
- DB: user=`Kin`, db=`tracker` (NOT `postgres`, NOT `mediatracker`)
- Deploy: `docker compose up -d --build`
- Logs: `docker compose logs --tail=50 app`
- Logs: `docker compose logs --tail=50 db`

## Project Overview

Full-stack media tracking app: **Rust (Axum) backend**, **Askama + HTMX frontend**, PostgreSQL, nginx reverse proxy.

## Project Structure
```text
mediatracker/
├── Cargo.toml                          # Зависимости Rust
├── docker-compose.yml                  # Оркестрация сервисов
├── nginx.conf                          # Настройки веб-сервера
├── .env                                # Переменные окружения
├── migrations/
│   └── 001_init.sql                    # Схема БД (users, sessions, media, tracking)
├── src/
│   ├── main.rs                         # Точка входа, запуск сервера
│   ├── config.rs                       # Загрузка конфигурации
│   ├── app_state.rs                    # Состояние приложения (БД + сервисы)
│   ├── lib.rs                          # Экспорт модулей
│   ├── routes/                         # HTTP эндпоинты (auth, home, media, search, stats, tracking)
│   ├── services/                       # Бизнес-логика (auth, tracking, stats, external providers)
│   ├── models/                         # Структуры данных (user, session, media, tracking)
│   ├── middleware/                     # Middleware (auth, logging)
│   ├── templates/                      # Askama шаблоны
│   ├── static/                         # CSS, JS, images
│   └── utils/                          # Вспомогательные функции
├── docs/                               # Документация
├── infra/                              # Terraform (будущее)
└── k8s/                                # Kubernetes (будущее)
```

## Current Status
- [x] Инициализация Rust-проекта
- [x] Настройка `Cargo.toml`
- [x] Создание структуры папок
- [x] Схема БД (`001_init.sql`)
- [x] Базовый сервер (`main.rs`)
- [x] AppState и сервисы
- [x] Middleware аутентификации (`auth_middleware`, `CurrentUser`)
- [x] Базовые шаблоны (home, search, media, stats, tracking)
- [x] Модель `TrackingEntryWithMedia` для отображения списков
- [ ] Реализация поиска (Shikimori, MangaUpdates)
- [ ] Реализация деталей медиа
- [x] Удаление legacy-кода (`backend/`)
- [ ] Тестирование через Docker Compose

## Next Steps
1. Удалить папку `backend/` (legacy Python код).
2. Реализовать `src/middleware/auth.rs`.
3. Заполнить роуты и шаблоны для Фазы 1 (Auth, Home).
4. Протестировать запуск через Docker Compose.

## Development Commands

```bash
# Backend (Rust)
cargo run                    # Run locally (requires local Postgres)
cargo build --release        # Build release binary
cargo check                  # Check compilation
cargo test                   # Run tests

# Docker
docker compose up -d --build # Run with Docker
docker compose logs -f app   # View app logs
docker compose down          # Stop services

# Database
docker compose exec db psql -U Kin -d tracker    # psql console
```

## Architecture

- **Backend**: Rust, Axum, SQLx, Askama, Tokio
- **Frontend**: Askama templates, HTMX, Alpine.js, Custom CSS
- **Database**: PostgreSQL 17
- **Entry points**:
  - Backend: `src/main.rs`
  - Templates: `templates/`
  - Static: `static/`
- **Routers**: `src/routes/` — auth, media, tracking, search, stats
- **Services**: `src/services/` — shikimori, mangaupdates, tmdb, rawg, auth, tracking, stats

## Key Conventions

- Axum: Router-based, middleware for auth
- CORS: `localhost:5173` и `mediatracker.web-socket-test-bench.site:2053`
- Health: `GET /health` → `{"status":"ok"}`
- Media items: composite unique `(provider, external_id)`
- Auth: Session-based (PostgreSQL), cookie `session_id`
- API: `/api/v1/` reserved for future mobile apps

## Deployment

- GitHub Actions на push в `main`
- Deploy: git pull → `docker compose up --build -d` → healthcheck `:2053/health`

## Stack Versions

- Rust: 1.88+
- Axum: 0.8
- SQLx: 0.8
- Askama: 0.16
- PostgreSQL: 17
- Docker Compose: v2+
