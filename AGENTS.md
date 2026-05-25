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

Full-stack media tracking app: **Rust (Axum) backend**, **Askama + Alpine.js frontend**, PostgreSQL, nginx reverse proxy.

## Project Structure
```text
mediatracker/
├── Cargo.toml                          # Зависимости Rust
├── docker-compose.yml                  # Оркестрация сервисов
├── nginx.conf                          # Настройки веб-сервера
├── .env                                # Переменные окружения
├── migrations/
│   ├── 001_init.sql                    # Схема БД (users, sessions, media, tracking)
│   ├── 002_add_media_types.sql         # Новые media_type для трекинга
│   ├── 003_backfill_activity_log.sql   # Заполнение activity_log
│   └── 004_status_rename.sql           # watching/reading → in_progress
├── src/
│   ├── main.rs                         # Точка входа, запуск сервера
│   ├── config.rs                       # Загрузка конфигурации
│   ├── app_state.rs                    # Состояние приложения (БД + сервисы)
│   ├── lib.rs                          # Экспорт модулей
│   ├── routes/                         # HTTP эндпоинты (auth, home, media, search, stats, tracking, settings)
│   ├── services/                       # Бизнес-логика (auth, tracking, stats, external providers)
│   ├── models/                         # Структуры данных (user, session, media, tracking, stats)
│   ├── middleware/                     # Middleware (auth)
│   ├── templates/                      # Askama шаблоны
│   ├── static/                         # CSS, JS
│   └── utils/                          # Вспомогательные функции
├── tests/                              # Интеграционные тесты
── docs/                               # Документация
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
- [x] Auth (login, register, logout) с Argon2 хешированием
- [x] Сессии в PostgreSQL, cookie `session_id`
- [x] Sidebar с навигацией (collapsible на десктопе, overlay на мобильных)
- [x] Header с поиском, dropdown тем, dropdown пользователя
- [x] 3 темы: light, graphite (default), dark (localStorage)
- [x] Главная страница со статистикой пользователя
- [x] Поиск с 13 типами (аниме, манга, манхва, маньхуа, новеллы, другие комиксы, фильмы, сериалы, дорамы, мультсериалы, мультфильмы, игры, книги)
- [x] 5 внешних провайдеров: Shikimori, MangaUpdates, TMDB, RAWG, Google Books
- [x] Постеры: Shikimori (полный URL), MangaUpdates (полный URL), TMDB (через nginx-прокси `/tmdb-image/`)
- [x] Детали медиа (poster, meta, description, "В список")
- [x] Трекинг CRUD (add, update progress, complete, delete)
- [x] Фильтры трекинга: два Alpine.js dropdown на странице (категория + статус)
- [x] Статус-фильтры через sidebar (Смотрю/Читаю → В процессе, Просмотрено → Завершено)
- [x] Единый статус `in_progress` вместо раздельных `watching`/`reading`
- [x] Страница статистики (status cards, breakdown bars, activity calendar, progress list)
- [x] Страница настроек (профиль, смена пароля, уведомления, удаление аккаунта)
- [x] GitHub Actions workflow с кэшированием Cargo, timeout 60m, healthcheck
- [x] Dockerfile с multi-stage build и cache mounts
- [x] Nginx: SSL, proxy to app:8080, TMDB image proxy, DNS resolver
- [x] Удаление legacy-кода (`backend/`, `frontend/`)

## Next Steps
1. Реализовать MAL OAuth импорт/экспорт
2. Добавить Telegram бота для уведомлений
3. Реализовать HTMX-интерактивность (обновление прогресса без перезагрузки)
4. Добавить пагинацию для поиска и трекинга
5. Реализовать кэширование внешних API запросов (Redis)

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
- **Frontend**: Askama templates, Alpine.js, Custom CSS (3 темы)
- **Database**: PostgreSQL 17
- **Entry points**:
  - Backend: `src/main.rs`
  - Templates: `templates/`
  - Static: `static/`
- **Routers**: `src/routes/` — auth, home, media, search, stats, tracking, settings
- **Services**: `src/services/` — shikimori, mal (Jikan), mangaupdates, tmdb, rawg, google_books, auth, tracking, stats
- **External Providers**:
  - `anime` → Shikimori + MAL (Jikan v4, без ключа)
  - `manga/manhwa/manhua/novels/other-comics` → MangaUpdates (фильтрация по типу)
  - `movies/tv/dramas/cartoons/animated-movies` → TMDB (IMDb — позже)
  - `games` → RAWG
  - `books` → Google Books

## Key Conventions

- Axum: Router-based, middleware for auth
- Health: `GET /health` → `{"status":"ok"}`
- Media items: composite unique `(provider, external_id)`
- Auth: Session-based (PostgreSQL), cookie `session_id` (HttpOnly, Secure, SameSite=Lax)
- Password hashing: Argon2
- Session token: UUID + SHA256 hash stored in DB
- IP column: PostgreSQL `INET` type, Rust `Option<String>` с кастом `ip::text` в SELECT
- TMDB images: проксируются через nginx `/tmdb-image/` (VPN не нужен)
- Shikimori images: относительные URL дополняются `https://shikimori.one`
- Sidebar: collapsible на десктопе (64px иконки / 260px полный), overlay на мобильных
- Themes: light, graphite (default), dark — сохраняются в `localStorage['mediatracker-theme']`
- Status system: `in_progress` (В процессе), `completed` (Завершено), `planned` (Запланировано), `dropped` (Брошено), `paused` (на схеме, не в UI)
- Status CSS: переменная `--in_progress`, классы `.status-in_progress`, `.status-bar.in_progress`, `.stat-card-icon.in_progress`
- Tracking filters: два Alpine.js dropdown (`x-data="{ typeOpen: false, statusOpen: false }"`) на странице `/tracking`
- Sidebar: без статус-фильтров, только ссылка «Отслеживаемое» → `/tracking?status=in_progress`
- Deploy port: 8443 (nginx), Cloudflare custom port

## Deployment

- GitHub Actions на push в `main`
- Workflow: `git pull` → `docker compose down` → `docker compose up --build -d --remove-orphans` → healthcheck
- Healthcheck: `curl -s http://localhost:8080/health` → `{"status":"ok"}`
- Timeout: 60m (Rust compilation)
- Cargo cache: `~/.cargo/registry` и `~/.cargo/git` через BuildKit mounts
- Nginx: порт 8443, SSL через Cloudflare Origin CA, DNS resolver `127.0.0.11`

## Stack Versions

- Rust: 1.95
- Axum: 0.8
- SQLx: 0.8
- Askama: 0.16
- PostgreSQL: 17
- Docker Compose: v2+
- reqwest: 0.13
- sha2: 0.11
- hex: 0.4
- url: 2
