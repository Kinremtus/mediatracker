use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "media_drawer_content.html")]
struct MediaDrawerTemplate {
    item: CreateMediaItem,
    tracking_id: Option<Uuid>,
    current_status: Option<String>,
    progress: Option<i32>,
    rating: Option<f64>,
    total_count: Option<i32>,
    progress_unit: String,
    has_progress: bool,
    role: String,
    star_classes: Vec<&'static str>,
    progress_display: i32,
    total_display: String,
    rating_display: String,
    can_increment: bool,
    next_progress: i32,
    status_display: String,
}

impl MediaDrawerTemplate {
    fn compute_star_classes(rating: Option<f64>) -> Vec<&'static str> {
        (1..=10).map(|star| {
            match rating {
                Some(r) => {
                    if r >= star as f64 { "active" }
                    else if r >= (star as f64) - 0.5 { "half" }
                    else { "" }
                }
                None => "",
            }
        }).collect()
    }
}

#[derive(Template)]
#[template(path = "media_detail.html")]
struct MediaDetailTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    item: CreateMediaItem,
    current_status: String,
    flash_message: String,
}

#[derive(Deserialize)]
pub struct MediaDetailQuery {
    media_type: Option<String>,
    flash: Option<String>,
}

pub async fn get_media_detail(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(params): Query<MediaDetailQuery>,
) -> impl IntoResponse {
    let item = match provider.as_str() {
        "shikimori" => state.shikimori.get_details(&external_id).await,
        "mal" => state.mal.get_details(&external_id).await,
        "mangaupdates" => state.mangaupdates.get_details(&external_id).await,
        "tmdb" => {
            let media_type = params.media_type.as_deref().unwrap_or("movie");
            state.tmdb.get_details(&external_id, media_type).await
        }
        "rawg" => state.rawg.get_details(&external_id).await,
        "igdb" => state.igdb.get_details(&external_id).await,
        "google_books" => state.google_books.get_details(&external_id).await,
        "openlibrary" => state.openlibrary.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    let stats = get_sidebar_stats(&state, &user).await;

    match item {
        Ok(mut item) => {
            if let Ok(Some(_)) = state.tracking.find_entry_by_media(
                user.id, &item.provider, &item.external_id,
            ).await {
                item.is_tracked = true;
            }
            let flash_message = params.flash.as_deref().map(|f| match f {
                "added" => "✓ Медиа добавлено в список".to_string(),
                "error" => "Ошибка при добавлении".to_string(),
                _ => String::new(),
            }).unwrap_or_default();

            Html(
                MediaDetailTemplate {
                    username: user.username,
                    role: user.role,
                    stats,
                    active_page: "search".to_string(),
                    item,
                    current_status: String::new(),
                    flash_message,
                }
                .render()
                .unwrap(),
            )
            .into_response()
        }
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

pub async fn get_media_drawer_content(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(params): Query<MediaDetailQuery>,
) -> impl IntoResponse {
    let item = match provider.as_str() {
        "shikimori" => state.shikimori.get_details(&external_id).await,
        "mal" => state.mal.get_details(&external_id).await,
        "mangaupdates" => state.mangaupdates.get_details(&external_id).await,
        "tmdb" => {
            let media_type = params.media_type.as_deref().unwrap_or("movie");
            state.tmdb.get_details(&external_id, media_type).await
        }
        "rawg" => state.rawg.get_details(&external_id).await,
        "igdb" => state.igdb.get_details(&external_id).await,
        "google_books" => state.google_books.get_details(&external_id).await,
        "openlibrary" => state.openlibrary.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    match item {
        Ok(item) => {
            let tracking = state.tracking.find_entry_by_media(user.id, &provider, &external_id).await.unwrap_or(None);
            let (tracking_id, current_status, progress, rating) = match tracking {
                Some((id, status, prog, rat)) => (Some(id), Some(status), Some(prog), rat),
                None => (None, None, None, None),
            };
            let total_count = item.total_count();
            let progress_unit = item.progress_unit_ru().to_string();
            let has_progress = matches!(item.media_type.as_str(),
                "anime" | "series" | "cartoons" | "animated-movies"
                | "manga" | "manhwa" | "manhua" | "novel" | "other-comics"
                | "book" | "game"
            );
            let star_classes = MediaDrawerTemplate::compute_star_classes(rating);
            let progress_display = progress.unwrap_or(0);
            let total_display = match total_count {
                Some(tc) => format!(" / {tc}"),
                None => String::new(),
            };
            let rating_display = match rating {
                Some(r) => format!("{:.1}", r),
                None => "—".to_string(),
            };
            let can_increment = has_progress
                && progress_display < total_count.unwrap_or(i32::MAX);
            let next_progress = progress_display + 1;
            let status_display = current_status.clone().unwrap_or_else(|| "in_progress".to_string());
            Html(
                MediaDrawerTemplate {
                    item,
                    tracking_id,
                    current_status,
                    progress,
                    rating,
                    total_count,
                    progress_unit,
                    has_progress,
                    role: user.role,
                    star_classes,
                    progress_display,
                    total_display,
                    rating_display,
                    can_increment,
                    next_progress,
                    status_display,
                }
                .render()
                .unwrap()
            )
            .into_response()
        }
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

async fn get_sidebar_stats(state: &AppState, user: &CurrentUser) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user.id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp, role: user.role.clone() }
}

#[derive(Template)]
#[template(path = "partials/_episode_list.html")]
struct EpisodeListPartial {
    episodes: Vec<crate::services::episodes::StoredEpisode>,
    provider: String,
    external_id: String,
}

#[derive(Template)]
#[template(path = "partials/_episode_item.html")]
struct EpisodeItemPartial {
    episode: crate::services::episodes::StoredEpisode,
    provider: String,
    external_id: String,
}

#[derive(Deserialize)]
pub struct SetWatchedForm {
    #[serde(default)]
    pub watched: bool,
}

/// Lazy-loaded endpoint for the drawer's "Episodes" section.
/// If episodes aren't in the DB yet (e.g. background fetch from
/// post_add_to_tracking hasn't completed), trigger a synchronous
/// fetch+store so the drawer doesn't show "Эпизоды не загружены"
/// on first open.
///
/// Episode source is always Jikan v4. We store them under
/// `provider = "mal"`, `external_id = mal_id.to_string()`. For
/// Shikimori-sourced entries the URL still has the shikimori id
/// in `external_id`, so we look up `mal_id` from `media_items`
/// first and key the episode read/fetch on that.
pub async fn get_episodes(
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // Resolve the MAL id (episode key) for this anime.
    let mal_id: Option<i64> = match provider.as_str() {
        "mal" => external_id.parse::<i64>().ok(),
        "shikimori" => {
            match crate::services::episodes::lookup_mal_id(
                &state.db, &provider, &external_id,
            )
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!(provider, external_id, error = %e, "lookup_mal_id failed");
                    None
                }
            }
        }
        _ => None,
    };

    // Try DB first (episodes are stored under provider="mal" keyed by mal_id).
    let mut existing = Vec::new();
    if let Some(mal_id) = mal_id {
        existing = crate::services::episodes::get_episodes(
            &state.db,
            "mal",
            &mal_id.to_string(),
        )
        .await
        .unwrap_or_default();
    }

    // If empty, fetch on-demand via Jikan.
    if existing.is_empty() {
        if let Some(mal_id) = mal_id {
            if let Err(e) = crate::services::episodes::fetch_and_store_mal(
                state.db.clone(),
                &state.mal,
                mal_id,
            )
            .await
            {
                tracing::warn!(provider, external_id, mal_id, error = %e, "on-demand episode fetch failed");
            }
        } else {
            tracing::debug!(provider, external_id, "no mal_id available; cannot fetch episodes");
        }
    }

    let episodes = match mal_id {
        Some(id) => crate::services::episodes::get_episodes(
            &state.db,
            "mal",
            &id.to_string(),
        )
        .await
        .unwrap_or_default(),
        None => Vec::new(),
    };

    let html = EpisodeListPartial {
        episodes,
        provider: provider.clone(),
        external_id: external_id.clone(),
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "episode list render failed");
        String::new()
    });
    Html(html)
}

/// Toggle `watched` for a single episode and (if successful) recompute
/// `tracking_entries.progress = max(progress, max_watched_ep)`.
///
/// Always keys on MAL id under the hood, regardless of which
/// provider the user originally added the anime with. For
/// Shikimori-sourced items we look up `mal_id` from `media_items`.
///
/// Returns the updated row HTML so HTMX can swap it in place, plus
/// an `HX-Trigger: progressUpdated` event with `{maxWatched: N}` for
/// the drawer progress text to update without a full refresh.
pub async fn set_episode_watched(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id, episode_number)): Path<(String, String, i32)>,
    Form(form): Form<SetWatchedForm>,
) -> impl IntoResponse {
    // Resolve the MAL id (the actual storage key for episodes).
    let mal_id: Option<i64> = match provider.as_str() {
        "mal" => external_id.parse::<i64>().ok(),
        "shikimori" => match crate::services::episodes::lookup_mal_id(
            &state.db, &provider, &external_id,
        )
        .await
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(provider, external_id, error = %e, "lookup_mal_id failed");
                None
            }
        },
        _ => None,
    };

    let mal_id = match mal_id {
        Some(id) => id,
        None => {
            tracing::debug!(provider, external_id, episode_number, "no mal_id available, ignoring toggle");
            return Html(String::new()).into_response();
        }
    };

    if let Err(e) = crate::services::episodes::set_watched(
        &state.db, mal_id, episode_number, form.watched,
    )
    .await
    {
        tracing::warn!(provider, external_id, mal_id, episode_number, error = %e, "set_watched failed");
        return Html(String::new()).into_response();
    }

    // Recompute progress for the tracking entry (if any).
    let max_watched = crate::services::episodes::count_watched(&state.db, mal_id)
        .await
        .unwrap_or(0);

    // Resolve media_id once (for both progress sync and HX-Trigger broadcast).
    let media_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(&provider)
    .bind(&external_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if let Some(media_id) = media_id {
        if let Err(e) = crate::services::episodes::update_progress_from_watched(
            &state.db, user.id, media_id, max_watched,
        )
        .await
        {
            tracing::warn!(provider, external_id, error = %e, "update_progress_from_watched failed");
        }
    }

    // Render the new row HTML and attach a progressUpdated event.
    let html = match crate::services::episodes::get_episode(
        &state.db, mal_id, episode_number,
    )
    .await
    {
        Ok(Some(ep)) => EpisodeItemPartial {
            episode: ep,
            provider: provider.clone(),
            external_id: external_id.clone(),
        }
        .render()
        .unwrap_or_default(),
        _ => String::new(),
    };

    // Pull authoritative state for ALL episodes so the drawer can sync
    // every visible checkbox (bulk-fill on watch flips 1..N rows; the
    // single-row HTMX swap only refreshes the clicked one).
    let states = crate::services::episodes::get_episode_states(&state.db, mal_id)
        .await
        .unwrap_or_default();
    let states_json: Vec<[serde_json::Value; 2]> = states
        .into_iter()
        .map(|(n, w)| [serde_json::Value::from(n), serde_json::Value::from(w)])
        .collect();

    let mut trigger = serde_json::json!({
        "progressUpdated": {
            "maxWatched": max_watched,
        },
        "episodesChanged": {
            "states": states_json,
        }
    });
    if let Some(media_id) = media_id {
        let id_str = serde_json::Value::String(media_id.to_string());
        trigger["progressUpdated"]["mediaId"] = id_str.clone();
        trigger["episodesChanged"]["mediaId"] = id_str;
    }
    let mut resp = Html(html).into_response();
    resp.headers_mut().insert(
        "HX-Trigger",
        trigger.to_string().parse().unwrap(),
    );
    resp
}

// ─── CHAPTERS (manga-like types) ──────────────────────────────

#[derive(Template)]
#[template(path = "partials/_chapter_list.html")]
struct ChapterListPartial {
    chapters: Vec<crate::services::chapters::StoredChapter>,
    provider: String,
    external_id: String,
}

#[derive(Template)]
#[template(path = "partials/_chapter_item.html")]
struct ChapterItemPartial {
    chapter: crate::services::chapters::StoredChapter,
    formatted_chapter: String,
    provider: String,
    external_id: String,
}

#[derive(Deserialize)]
pub struct SetReadForm {
    #[serde(default)]
    pub read: bool,
}

/// Lazy-loaded chapter list for manga-like drawer sections.
/// Chapters are stored under provider="mangaupdates", external_id=series_id.
/// If chapters aren't in the DB yet, trigger a synchronous fetch from
/// MangaUpdates `latest_chapter` to build the skeleton.
pub async fn get_chapters(
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // For mangaupdates the storage key is (provider, external_id) as-is.
    // For other sources we'd need a lookup — for now handle mangaupdates directly.
    let (mu_provider, mu_id) = match provider.as_str() {
        "mangaupdates" => ("mangaupdates", external_id.clone()),
        _ => {
            // Attempt to find the mangaupdates series_id via media_items.
            let lookup: Option<(String, String)> = sqlx::query_as(
                r#"
                SELECT provider, external_id FROM media_items
                WHERE id IN (
                    SELECT mi2.id FROM media_items mi2
                    WHERE mi2.provider = 'mangaupdates'
                      AND mi2.title ILIKE (
                          SELECT title FROM media_items WHERE provider = $1 AND external_id = $2
                      )
                    LIMIT 1
                )
                "#,
            )
            .bind(&provider)
            .bind(&external_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);
            match lookup {
                Some((p, eid)) => (p, eid),
                None => ("mangaupdates", external_id.clone()),
            }
        }
    };

    // Try DB first.
    let existing = crate::services::chapters::get_chapters(
        &state.db, &mu_provider, &mu_id,
    )
    .await
    .unwrap_or_default();

    // If empty, try to fetch the latest_chapter from MangaUpdates and build skeleton.
    if existing.is_empty() {
        if let Ok(series_id_num) = mu_id.parse::<i64>() {
            let details = crate::services::external::mangaupdates::MangaUpdatesService::new()
                .get_details(&mu_id)
                .await;

            if let Ok(details) = details {
                let lc = details.chapters.map(|v| v as i32).unwrap_or(0);
                if lc > 0 {
                    if let Err(e) = crate::services::chapters::store_chapters_mu(
                        &state.db, series_id_num, lc,
                    )
                    .await
                    {
                        tracing::warn!(series_id_num, error = %e, "store_chapters_mu failed");
                    }
                }
            }
        }
    }

    let chapters = crate::services::chapters::get_chapters(
        &state.db, &mu_provider, &mu_id,
    )
    .await
    .unwrap_or_default();

    let html = ChapterListPartial {
        chapters,
        provider: mu_provider.to_string(),
        external_id: mu_id,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "chapter list render failed");
        String::new()
    });
    Html(html)
}

/// Toggle `read` for a single chapter with bulk-fill semantics:
/// read N → 1..N marked read; unread N → N..max marked unread.
///
/// Emits `progressUpdated` + `chaptersChanged` HX-Trigger events.
pub async fn set_chapter_read(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id, chapter_number)): Path<(String, String, i32)>,
    Form(form): Form<SetReadForm>,
) -> impl IntoResponse {
    // Resolve media_items.id for this chapter.
    let media_id: Option<Uuid> = crate::services::chapters::lookup_media_id(
        &state.db, &provider, &external_id,
    )
    .await
    .unwrap_or(None);

    if let Err(e) = crate::services::chapters::set_read(
        &state.db, &provider, &external_id, chapter_number, form.read,
    )
    .await
    {
        tracing::warn!(provider, external_id, chapter_number, error = %e, "set_read failed");
        return Html(String::new()).into_response();
    }

    // Recompute progress.
    let max_read = crate::services::chapters::count_read(
        &state.db, &provider, &external_id,
    )
    .await
    .unwrap_or(0);

    if let Some(media_id) = media_id {
        if let Err(e) = crate::services::chapters::update_progress_from_read(
            &state.db, user.id, media_id, max_read,
        )
        .await
        {
            tracing::warn!(error = %e, "update_progress_from_read failed");
        }
    }

    // Render updated row.
    let chapter = crate::services::chapters::get_chapter(
        &state.db, &provider, &external_id, chapter_number,
    )
    .await
    .unwrap_or(None);

    let html = match chapter {
        Some(ch) => ChapterItemPartial {
            formatted_chapter: crate::services::chapters::format_chapter(ch.chapter_number),
            chapter: ch,
            provider: provider.clone(),
            external_id: external_id.clone(),
        }
        .render()
        .unwrap_or_default(),
        None => String::new(),
    };

    // Build HX-Trigger with full chapter states.
    let states = crate::services::chapters::get_chapter_states(
        &state.db, &provider, &external_id,
    )
    .await
    .unwrap_or_default();
    let states_json: Vec<[serde_json::Value; 2]> = states
        .into_iter()
        .map(|(n, r)| [serde_json::Value::from(n), serde_json::Value::from(r)])
        .collect();

    let mut trigger = serde_json::json!({
        "progressUpdated": {
            "maxRead": max_read,
        },
        "chaptersChanged": {
            "states": states_json,
        }
    });
    if let Some(media_id) = media_id {
        let id_str = serde_json::Value::String(media_id.to_string());
        trigger["progressUpdated"]["mediaId"] = id_str.clone();
        trigger["chaptersChanged"]["mediaId"] = id_str;
    }
    let mut resp = Html(html).into_response();
    resp.headers_mut().insert(
        "HX-Trigger",
        trigger.to_string().parse().unwrap(),
    );
    resp
}
