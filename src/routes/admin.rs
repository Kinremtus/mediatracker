use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
    Form,
};
use serde::Deserialize;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
use crate::services::chapters::enrich_from_mangadex;
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    message: String,
    error: String,
    refreshed: Option<usize>,
    total: Option<usize>,
}

async fn get_sidebar_stats(state: &AppState, user: &CurrentUser) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user.id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp, role: user.role.clone() }
}

fn require_admin(user: &CurrentUser) -> bool {
    user.role == "admin" || user.role == "moderator"
}

pub async fn get_admin_panel(
    user: CurrentUser,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !require_admin(&user) {
        return Redirect::to("/").into_response();
    }
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
    };
    Html(template.render().unwrap()).into_response()
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
    match provider {
        "shikimori" => state.shikimori.get_details(external_id).await,
        "mal" => state.mal.get_details(external_id).await,
        "mangaupdates" => state.mangaupdates.get_details(external_id).await,
        "tmdb" => state.tmdb.get_details(external_id, media_type).await,
        "rawg" => state.rawg.get_details(external_id).await,
        "igdb" => state.igdb.get_details(external_id).await,
        "google_books" => state.google_books.get_details(external_id).await,
        "openlibrary" => state.openlibrary.get_details(external_id).await,
        other => Err(anyhow::anyhow!("Unknown provider: {}", other)),
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

    let mut query = String::from(
        "SELECT id, provider, external_id, media_type FROM media_items WHERE 1=1",
    );
    let mut param_idx = 1;
    let mut bind_count = 0;

    if let Some(mt) = &form.media_type {
        if !mt.is_empty() {
            query.push_str(&format!(" AND media_type = ${}", param_idx));
            param_idx += 1;
            bind_count += 1;
        }
    }
    if let Some(p) = &form.provider {
        if !p.is_empty() {
            query.push_str(&format!(" AND provider = ${}", param_idx));
            bind_count += 1;
        }
    }
    query.push_str(&format!(" ORDER BY created_at ASC LIMIT {}", limit));
    let _ = (param_idx, bind_count);

    let mut q = sqlx::query_as::<_, (Uuid, String, String, String)>(&query);
    if let Some(mt) = &form.media_type {
        if !mt.is_empty() {
            q = q.bind(mt);
        }
    }
    if let Some(p) = &form.provider {
        if !p.is_empty() {
            q = q.bind(p);
        }
    }

    let rows: Vec<(Uuid, String, String, String)> = match q.fetch_all(db).await {
        Ok(r) => r,
        Err(e) => {
            return render_with_error(&state, &user, format!("DB error: {}", e)).await;
        }
    };

    let total = rows.len();
    let mut refreshed = 0usize;
    let mut failed = 0usize;

    for (id, provider, external_id, media_type) in rows {
        match fetch_details_for_provider(&state, &provider, &external_id, &media_type).await {
            Ok(item) => {
                let res = sqlx::query(
                    r#"
                    UPDATE media_items SET
                        title_english = $2, title_native = $3, title_russian = $4,
                        description = $5, status = $6, score = $7,
                        format_type = $8, details = $9,
                        chapters = $10, volumes = $11, pages = $12,
                        runtime_minutes = $13, playtime_hours = $14,
                        year = $15, aired_from = $16, aired_to = $17,
                        premiered_season = $18, premiered_year = $19, broadcast = $20,
                        completed = $21, licensed = $22,
                        source = $23, duration = $24, rating = $25, rating_votes = $26,
                        authors = $27, artists = $28, studios = $29, producers = $30,
                        licensors = $31, publishers = $32, serialized_in = $33,
                        networks = $34, platforms = $35,
                        genres = $36, themes = $37, demographics = $38, categories = $39,
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
                .bind(item.details.unwrap_or(serde_json::Value::Object(Default::default())))
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

    let stats = get_sidebar_stats(&state, &user).await;
    let message = format!("Обновлено: {} из {} (ошибок: {})", refreshed, total, failed);
    let template = AdminTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "admin".to_string(),
        message,
        error: String::new(),
        refreshed: Some(refreshed),
        total: Some(total),
    };
    Html(template.render().unwrap()).into_response()
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
    };
    Html(template.render().unwrap()).into_response()
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
    let mut enriched = 0usize;
    let mut failed = 0usize;

    for row in rows {
        let (provider, external_id): (String, String) = (row.get(0), row.get(1));
        match enrich_from_mangadex(db, &provider, &external_id).await {
            Ok(count) => enriched += count,
            Err(e) => {
                tracing::warn!("enrich_from_mangadex failed for {}/{}: {}", provider, external_id, e);
                failed += 1;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let stats = get_sidebar_stats(&state, &user).await;
    let message = format!("Обогащено глав: {} (объектов: {}, ошибок: {})", enriched, total, failed);
    let template = AdminTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "admin".to_string(),
        message,
        error: String::new(),
        refreshed: Some(enriched),
        total: Some(total),
    };
    Html(template.render().unwrap()).into_response()
}
