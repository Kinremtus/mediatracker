use askama::Template;
use axum::{
    Form,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::home::{SidebarStats, get_sidebar_stats};
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::{CreateMediaItem, chapter_source_label};
use crate::services::chapters::enrich_from_mangadex;
use crate::services::external::dispatch::{Provider, ProviderClients};

#[derive(Template)]
#[template(path = "admin.html")]
#[expect(dead_code)]
struct AdminTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    message: String,
    error: String,
    refreshed: Option<usize>,
    total: Option<usize>,
    query: String,
    items: Vec<AdminChapterItem>,
}

/// Renders a page template, degrading to a 500 instead of panicking the
/// request task if a template ever fails to render.
fn render_page<T: Template>(template: &T) -> Response {
    match template.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

fn require_admin(user: &CurrentUser) -> bool {
    user.role == "admin" || user.role == "moderator"
}

#[derive(Debug, Clone)]
pub struct AdminChapterItem {
    pub provider: String,
    pub external_id: String,
    pub title: String,
    pub media_type: String,
    pub chapters: Option<i32>,
    pub chapters_manual: bool,
    pub source_label: Option<String>,
    pub bindings: Vec<crate::services::source_ids::SourceId>,
}

#[derive(Deserialize)]
pub struct AdminQuery {
    #[serde(default)]
    pub q: String,
}

#[derive(Deserialize)]
pub struct AdminChapterForm {
    pub provider: String,
    pub external_id: String,
    #[serde(default)]
    pub chapters: i32,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub source_id: String,
    #[serde(default)]
    pub q: String,
}

fn normalized_q(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

async fn media_type_of(db: &PgPool, provider: &str, external_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT media_type FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

async fn chapters_source_of(db: &PgPool, provider: &str, external_id: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT chapters_source FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

#[allow(clippy::type_complexity)]
async fn search_chapter_items(db: &PgPool, q: Option<&str>) -> Vec<AdminChapterItem> {
    let Some(q) = q else {
        return Vec::new();
    };
    let pattern = format!("%{}%", q);
    let rows: Vec<(String, String, String, String, Option<i32>, bool, Option<String>)> =
        match sqlx::query_as(
            r#"
            SELECT provider, external_id, title, media_type, chapters, chapters_manual, chapters_source
            FROM media_items
            WHERE media_type IN ('manga','manhwa','manhua','novel','other-comics','comic')
              AND (
                title ILIKE $1
                OR title_russian ILIKE $1
                OR title_english ILIKE $1
                OR title_native ILIKE $1
                OR EXISTS (
                  SELECT 1 FROM unnest(associated_titles) AS t(alt)
                  WHERE alt ILIKE $1
                )
              )
            ORDER BY updated_at DESC
            LIMIT 20
            "#,
        )
        .bind(&pattern)
        .fetch_all(db)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "admin chapter search failed");
                return Vec::new();
            }
        };

    let mut items = Vec::with_capacity(rows.len());
    for (provider, external_id, title, media_type, chapters, chapters_manual, source) in rows {
        let bindings = crate::services::source_ids::list_source_ids(db, &provider, &external_id)
            .await
            .unwrap_or_default();
        items.push(AdminChapterItem {
            provider,
            external_id,
            title,
            media_type,
            chapters,
            chapters_manual,
            source_label: chapter_source_label(source.as_deref()).map(str::to_string),
            bindings,
        });
    }
    items
}

async fn render_admin(
    state: &AppState,
    user: &CurrentUser,
    message: String,
    error: String,
    q: Option<String>,
) -> Response {
    let stats = get_sidebar_stats(state, user).await;
    let items = search_chapter_items(&state.db, q.as_deref()).await;
    let template = AdminTemplate {
        username: user.username.clone(),
        role: user.role.clone(),
        stats,
        active_page: "admin".to_string(),
        message,
        error,
        refreshed: None,
        total: None,
        query: q.unwrap_or_default(),
        items,
    };
    render_page(&template)
}

pub async fn get_admin_panel(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<AdminQuery>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
    let q = normalized_q(&params.q);
    let items = search_chapter_items(&state.db, q.as_deref()).await;
    let stats = get_sidebar_stats(&state, &user).await;
    let template = AdminTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "admin".to_string(),
        message: String::new(),
        error: String::new(),
        refreshed: None,
        total: None,
        query: q.unwrap_or_default(),
        items,
    };
    render_page(&template)
}

#[derive(Deserialize)]
pub struct RefreshForm {
    #[serde(default)]
    pub media_type: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

async fn fetch_details_for_provider(
    state: &AppState,
    provider: &str,
    external_id: &str,
    media_type: &str,
) -> Result<CreateMediaItem, anyhow::Error> {
    let clients = ProviderClients::from_state(state);
    match Provider::from_name(&clients, provider) {
        Some(provider) => provider.fetch(external_id, media_type).await,
        None => Err(anyhow::anyhow!("Unknown provider: {}", provider)),
    }
}

/// Single-flight guard for the long-running "refresh details" admin job.
static REFRESH_DETAILS_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Single-flight guard for the long-running "enrich chapters" admin job.
static ENRICH_CHAPTERS_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Clears a single-flight flag when the background job finishes (or panics),
/// so a crashed run cannot wedge the button forever.
struct RunGuard(&'static std::sync::atomic::AtomicBool);

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

pub async fn post_refresh_details(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<RefreshForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }

    let db: &PgPool = &state.db;
    let limit = form.limit.unwrap_or(50).clamp(1, 500);

    // Static SQL with optional filters. `$1`/`$2` are NULL when the filter is
    // absent, so there is no dynamic placeholder construction that could drift
    // out of sync with the bind order.
    let media_type = form.media_type.as_deref().filter(|s| !s.is_empty());
    let provider = form.provider.as_deref().filter(|s| !s.is_empty());

    let rows: Vec<(Uuid, String, String, String)> =
        match sqlx::query_as::<_, (Uuid, String, String, String)>(
            r#"
            SELECT id, provider, external_id, media_type
            FROM media_items
            WHERE ($1::text IS NULL OR media_type = $1)
              AND ($2::text IS NULL OR provider = $2)
              AND provider <> 'manual'
            ORDER BY created_at ASC
            LIMIT $3
            "#,
        )
        .bind(media_type)
        .bind(provider)
        .bind(limit)
        .fetch_all(db)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                return render_with_error(&state, &user, format!("DB error: {}", e)).await;
            }
        };

    let total = rows.len();
    if REFRESH_DETAILS_RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return render_with_error(
            &state,
            &user,
            "Обновление деталей уже выполняется".to_string(),
        )
        .await;
    }

    let bg_state = state.clone();
    tokio::spawn(async move {
        let _guard = RunGuard(&REFRESH_DETAILS_RUNNING);
        let db: &PgPool = &bg_state.db;
        let mut refreshed = 0usize;
        let mut failed = 0usize;
        for (id, provider, external_id, media_type) in rows {
            match fetch_details_for_provider(&bg_state, &provider, &external_id, &media_type).await
            {
                Ok(item) => {
                    let res = sqlx::query(
                        r#"
                    UPDATE media_items SET
                        title_english = $2, title_native = $3, title_russian = $4,
                        description = $5, status = $6, score = $7,
                        format_type = $8, details = $9,
                        chapters = CASE
                            WHEN chapters_manual THEN chapters
                            ELSE $10
                        END,
                        chapters_source = CASE
                            WHEN chapters_manual THEN chapters_source
                            WHEN $10 IS NOT NULL THEN 'auto:' || $42
                            ELSE chapters_source
                        END,
                        volumes = $11, pages = $12,
                        runtime_minutes = $13, playtime_hours = $14,
                        year = $15, aired_from = $16, aired_to = $17,
                        premiered_season = $18, premiered_year = $19, broadcast = $20,
                        completed = $21, licensed = $22,
                        source = $23, duration = $24, rating = $25, rating_votes = $26,
                        authors = $27, artists = $28, studios = $29, producers = $30,
                        licensors = $31, publishers = $32, serialized_in = $33,
                        networks = $34, platforms = $35,
                        genres = $36, themes = $37, demographics = $38, categories = $39,
                        episodes = $40,
                        associated_titles = $41,
                        updated_at = NOW()
                    WHERE id = $1
                    "#,
                    )
                    .bind(id)
                    .bind(&item.title_english)
                    .bind(&item.title_native)
                    .bind(&item.title_russian)
                    .bind(&item.description)
                    .bind(&item.status)
                    .bind(item.score)
                    .bind(&item.format_type)
                    .bind(
                        item.details
                            .unwrap_or(serde_json::Value::Object(Default::default())),
                    )
                    .bind(item.chapters)
                    .bind(item.volumes)
                    .bind(item.pages)
                    .bind(item.runtime_minutes)
                    .bind(item.playtime_hours)
                    .bind(item.year)
                    .bind(item.aired_from)
                    .bind(item.aired_to)
                    .bind(&item.premiered_season)
                    .bind(item.premiered_year)
                    .bind(&item.broadcast)
                    .bind(item.completed)
                    .bind(item.licensed)
                    .bind(&item.source)
                    .bind(&item.duration)
                    .bind(&item.rating)
                    .bind(item.rating_votes)
                    .bind(&item.authors)
                    .bind(&item.artists)
                    .bind(&item.studios)
                    .bind(&item.producers)
                    .bind(&item.licensors)
                    .bind(&item.publishers)
                    .bind(&item.serialized_in)
                    .bind(&item.networks)
                    .bind(&item.platforms)
                    .bind(&item.genres)
                    .bind(&item.themes)
                    .bind(&item.demographics)
                    .bind(&item.categories)
                    .bind(item.episodes)
                    .bind(&item.associated_titles)
                    .bind(&provider)
                    .execute(db)
                    .await;
                    match res {
                        Ok(_) => refreshed += 1,
                        Err(e) => {
                            tracing::error!("Failed to update media_items row {}: {}", id, e);
                            failed += 1;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("get_details failed for {}/{}: {}", provider, external_id, e);
                    failed += 1;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
        tracing::info!(refreshed, failed, total, "refresh-details job finished");
    });

    let stats = get_sidebar_stats(&state, &user).await;
    let template = AdminTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "admin".to_string(),
        message: format!("Обновление деталей запущено в фоне ({total} элементов)"),
        error: String::new(),
        refreshed: None,
        total: Some(total),
        query: String::new(),
        items: Vec::new(),
    };
    render_page(&template)
}

async fn render_with_error(
    state: &AppState,
    user: &CurrentUser,
    error: String,
) -> axum::response::Response {
    let stats = get_sidebar_stats(state, user).await;
    let template = AdminTemplate {
        username: user.username.clone(),
        role: user.role.clone(),
        stats,
        active_page: "admin".to_string(),
        message: String::new(),
        error,
        refreshed: None,
        total: None,
        query: String::new(),
        items: Vec::new(),
    };
    render_page(&template)
}

#[derive(Deserialize)]
pub struct EnrichChaptersForm {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub external_id: Option<String>,
}

pub async fn post_enrich_chapters(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<EnrichChaptersForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }

    let db: &PgPool = &state.db;

    let query = if let (Some(provider), Some(external_id)) = (form.provider, form.external_id) {
        // Single manga
        sqlx::query("SELECT provider, external_id FROM media_items WHERE provider = $1 AND external_id = $2 AND media_type IN ('manga','manhwa','manhua','novel','other-comics')")
            .bind(&provider)
            .bind(&external_id)
            .fetch_all(db)
            .await
    } else {
        // All manga-like items without titles
        sqlx::query(
            r#"
            SELECT mi.provider, mi.external_id FROM media_items mi
            WHERE mi.media_type IN ('manga','manhwa','manhua','novel','other-comics')
            AND NOT EXISTS (
                SELECT 1 FROM series_chapters sc
                WHERE sc.provider = mi.provider AND sc.external_id = mi.external_id
                AND (sc.title_en IS NOT NULL OR sc.title_ru IS NOT NULL)
                LIMIT 1
            )
            "#,
        )
        .fetch_all(db)
        .await
    };

    let rows = match query {
        Ok(r) => r,
        Err(e) => {
            return render_with_error(&state, &user, format!("DB error: {}", e)).await;
        }
    };

    let total = rows.len();
    if ENRICH_CHAPTERS_RUNNING.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return render_with_error(&state, &user, "Обогащение глав уже выполняется".to_string())
            .await;
    }

    let bg_state = state.clone();
    tokio::spawn(async move {
        let _guard = RunGuard(&ENRICH_CHAPTERS_RUNNING);
        let db: &PgPool = &bg_state.db;
        let mut enriched = 0usize;
        let mut failed = 0usize;
        for row in rows {
            let (provider, external_id): (String, String) = (row.get(0), row.get(1));
            match enrich_from_mangadex(db, &provider, &external_id).await {
                Ok(count) => enriched += count,
                Err(e) => {
                    tracing::warn!(
                        "enrich_from_mangadex failed for {}/{}: {}",
                        provider,
                        external_id,
                        e
                    );
                    failed += 1;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        tracing::info!(enriched, failed, total, "enrich-chapters job finished");
    });

    let stats = get_sidebar_stats(&state, &user).await;
    let template = AdminTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "admin".to_string(),
        message: format!("Обогащение глав запущено в фоне ({total} объектов)"),
        error: String::new(),
        refreshed: None,
        total: Some(total),
        query: String::new(),
        items: Vec::new(),
    };
    render_page(&template)
}

/// POST /admin/chapters/manual — pin a manual chapter count (ground truth).
pub async fn post_chapter_manual(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<AdminChapterForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
    let q = normalized_q(&form.q);
    match crate::services::chapters::set_manual_chapters(
        &state.db,
        &form.provider,
        &form.external_id,
        form.chapters,
    )
    .await
    {
        Ok(()) => {
            let msg = format!("Установлено {} глав (manual)", form.chapters.max(0));
            render_admin(&state, &user, msg, String::new(), q).await
        }
        Err(e) => render_admin(&state, &user, String::new(), format!("Ошибка БД: {}", e), q).await,
    }
}

/// POST /admin/chapters/reset — drop the manual pin (auto may set it later).
pub async fn post_chapter_reset(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<AdminChapterForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
    let q = normalized_q(&form.q);
    match crate::services::chapters::clear_manual_chapters(
        &state.db,
        &form.provider,
        &form.external_id,
    )
    .await
    {
        Ok(()) => {
            render_admin(
                &state,
                &user,
                "Ручной счёт сброшен (подтянет авто)".to_string(),
                String::new(),
                q,
            )
            .await
        }
        Err(e) => render_admin(&state, &user, String::new(), format!("Ошибка БД: {}", e), q).await,
    }
}

/// POST /admin/chapters/bind — bind a source id, then enrich immediately.
pub async fn post_chapter_bind(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<AdminChapterForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
    let q = normalized_q(&form.q);
    let source = form.source.trim();
    let source_id = form.source_id.trim();
    if !crate::services::source_ids::is_known_source(source) || source_id.is_empty() {
        return render_admin(
            &state,
            &user,
            String::new(),
            "Неизвестный источник или пустой id".to_string(),
            q,
        )
        .await;
    }
    if let Err(e) = crate::services::source_ids::set_source_id(
        &state.db,
        &form.provider,
        &form.external_id,
        source,
        source_id,
    )
    .await
    {
        return render_admin(&state, &user, String::new(), format!("Ошибка БД: {}", e), q).await;
    }

    let before =
        crate::services::chapters::get_chapter_meta(&state.db, &form.provider, &form.external_id)
            .await
            .ok()
            .flatten()
            .and_then(|(ch, _, _)| ch);

    let applied = match media_type_of(&state.db, &form.provider, &form.external_id).await {
        Some(mt) => {
            crate::services::chapter_enrich::enrich_chapter_count(
                &state.db,
                &form.provider,
                &form.external_id,
                &mt,
            )
            .await
        }
        None => None,
    };

    let msg = match applied {
        Some(n) => {
            let src = chapters_source_of(&state.db, &form.provider, &form.external_id).await;
            let label = chapter_source_label(src.as_deref()).unwrap_or("auto");
            format!("{} -> {} ({})", before.unwrap_or(0), n, label)
        }
        None => format!("Привязано: {} (счёт без изменений)", source),
    };
    render_admin(&state, &user, msg, String::new(), q).await
}

/// POST /admin/chapters/unbind — remove a source binding.
pub async fn post_chapter_unbind(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<AdminChapterForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
    let q = normalized_q(&form.q);
    match crate::services::source_ids::delete_source_id(
        &state.db,
        &form.provider,
        &form.external_id,
        form.source.trim(),
    )
    .await
    {
        Ok(true) => {
            render_admin(
                &state,
                &user,
                "Источник отвязан".to_string(),
                String::new(),
                q,
            )
            .await
        }
        Ok(false) => {
            render_admin(
                &state,
                &user,
                "Привязка не найдена".to_string(),
                String::new(),
                q,
            )
            .await
        }
        Err(e) => render_admin(&state, &user, String::new(), format!("Ошибка БД: {}", e), q).await,
    }
}

/// POST /admin/chapters/refresh — run enrichment for one item on demand.
pub async fn post_chapter_refresh(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<AdminChapterForm>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
    let q = normalized_q(&form.q);
    let before =
        crate::services::chapters::get_chapter_meta(&state.db, &form.provider, &form.external_id)
            .await
            .ok()
            .flatten()
            .and_then(|(ch, _, _)| ch);

    let applied = match media_type_of(&state.db, &form.provider, &form.external_id).await {
        Some(mt) => {
            crate::services::chapter_enrich::enrich_chapter_count(
                &state.db,
                &form.provider,
                &form.external_id,
                &mt,
            )
            .await
        }
        None => None,
    };

    let msg = match applied {
        Some(n) => {
            let src = chapters_source_of(&state.db, &form.provider, &form.external_id).await;
            let label = chapter_source_label(src.as_deref()).unwrap_or("auto");
            format!("{} -> {} ({})", before.unwrap_or(0), n, label)
        }
        None => "Счёт без изменений".to_string(),
    };
    render_admin(&state, &user, msg, String::new(), q).await
}
