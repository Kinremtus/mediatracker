use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use uuid::Uuid;

use super::home::{SidebarStats, get_sidebar_stats};
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::{
    CreateMediaItem, derived_status_class, derived_status_label, parse_description,
};
use crate::services::external::dispatch::{Provider, ProviderClients};
use crate::services::external::is_not_found;

use super::progress::ProgressRow;

#[derive(Template)]
#[template(path = "media_drawer_content.html")]
#[expect(dead_code)]
struct MediaDrawerTemplate {
    item: CreateMediaItem,
    tracking_id: Option<Uuid>,
    current_status: Option<String>,
    progress: Option<i32>,
    rating: Option<f64>,
    total_count: Option<i32>,
    progress_unit: String,
    mal_id: Option<i64>,
    has_progress: bool,
    role: String,
    star_classes: Vec<&'static str>,
    progress_display: i32,
    total_display: String,
    rating_display: String,
    can_increment: bool,
    can_decrement: bool,
    status_display: String,
    from_cache: bool,
    fallback_notice: String,
    alt_titles: Vec<String>,
    status_label: Option<&'static str>,
    status_class: &'static str,
    description_clean: Option<String>,
    translations: Vec<String>,
    original_novel: Option<String>,
    original_webtoon: Option<String>,
    mu_url: Option<String>,
}

impl MediaDrawerTemplate {
    fn compute_star_classes(rating: Option<f64>) -> Vec<&'static str> {
        (1..=10)
            .map(|star| match rating {
                Some(r) => {
                    if r >= star as f64 {
                        "active"
                    } else if r >= (star as f64) - 0.5 {
                        "half"
                    } else {
                        ""
                    }
                }
                None => "",
            })
            .collect()
    }
}

#[derive(Template)]
#[template(path = "media_detail.html")]
#[expect(dead_code)]
struct MediaDetailTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    item: CreateMediaItem,
    current_status: String,
    flash_message: String,
    from_cache: bool,
    fallback_notice: String,
    mal_id: Option<i64>,
    alt_titles: Vec<String>,
    status_label: Option<&'static str>,
    status_class: &'static str,
    description_clean: Option<String>,
    translations: Vec<String>,
    original_novel: Option<String>,
    original_webtoon: Option<String>,
    mu_url: Option<String>,
}

#[derive(Deserialize)]
pub struct MediaDetailQuery {
    media_type: Option<String>,
    flash: Option<String>,
}

#[derive(Deserialize)]
pub struct EpisodesQuery {
    /// Клиентский hint MAL id. Принимается только для anime-провайдеров
    /// (mal | shikimori | anilist) — защита от подмены чужого id.
    mal_id: Option<i64>,
    /// Клиентский hint на общее число эпизодов. Используется только как
    /// fallback для синтеза строк, когда каталог провайдера пуст.
    episodes: Option<i32>,
    /// `?all=true` returns the full episode list instead of the window.
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Deserialize)]
pub struct ChaptersQuery {
    /// `?all=true` returns the full list instead of the latest window.
    #[serde(default)]
    all: Option<bool>,
}

/// Why a card is rendering from stored data (or not).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fallback {
    Fresh,
    /// Provider explicitly reported the entity as gone (404).
    ProviderGone,
    /// Network / auth / decode / 5xx failure.
    ProviderUnavailable,
}

impl Fallback {
    pub(crate) fn is_from_cache(&self) -> bool {
        !matches!(self, Fallback::Fresh)
    }

    pub(crate) fn notice(&self) -> &'static str {
        match self {
            Fallback::Fresh => "",
            Fallback::ProviderGone => "Показаны сохранённые данные — источник удалил тайтл",
            Fallback::ProviderUnavailable => "Показаны сохранённые данные — источник недоступен",
        }
    }
}

/// Media types whose progress unit is a chapter (see `MediaItem::total_count`).
fn is_chapter_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "manga" | "manhwa" | "manhua" | "novel" | "other-comics" | "comic"
    )
}

/// Решить, что рендерить для карточки.
///
/// Возвращает `(item, kind)`:
/// - `manual` / неизвестный провайдер -> строка из БД, `Fallback::Fresh`
///   (апстрима нет, «провайдер недоступен» показывать нечестно); `None`, если
///   строки нет.
/// - известный провайдер, fetch Ok -> свежий item, `Fallback::Fresh`.
/// - известный провайдер, fetch Err + локальная строка -> item из БД,
///   `Fallback::ProviderGone` (404) / `Fallback::ProviderUnavailable`
///   (этап 4: тайтл выживает 404/сетевой сбой); `None`, если строки нет.
async fn resolve_card_data(
    state: &AppState,
    provider: &str,
    external_id: &str,
    media_type: &str,
) -> Option<(CreateMediaItem, Fallback)> {
    let local = state
        .tracking
        .find_media_item(provider, external_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(provider, external_id, error = %e, "media: local lookup failed");
            None
        });

    let clients = ProviderClients::from_state(state);
    match Provider::from_name(&clients, provider) {
        None => local.map(|m| (m.into(), Fallback::Fresh)),
        Some(p) => match p.fetch(external_id, media_type).await {
            Ok(mut item) => {
                // Chapter-based media: the stored count is authoritative. It may
                // carry a manual override or an enriched value from a source the
                // upstream provider does not know about (e.g. Kakao), while the
                // upstream only reports its own chapter list (often stale/lower).
                // Prefer the DB value so the drawer and detail page agree with
                // the chapter list and progress bar.
                if is_chapter_type(media_type)
                    && let Some(db_chapters) = local.as_ref().and_then(|m| m.chapters)
                    && db_chapters > 0
                {
                    item.chapters = Some(db_chapters);
                }
                Some((item, Fallback::Fresh))
            }
            Err(e) => {
                tracing::debug!(
                    provider,
                    external_id,
                    error = %e,
                    "media: provider fetch failed, falling back to stored data"
                );
                let kind = if is_not_found(&e) {
                    Fallback::ProviderGone
                } else {
                    Fallback::ProviderUnavailable
                };
                local.map(|m| (m.into(), kind))
            }
        },
    }
}

const CHAPTER_WINDOW: usize = 5;
/// Number of episodes rendered in the drawer before "Все эпизоды (N)".
const EPISODE_WINDOW: usize = 5;
const MU_WEB_BASE: &str = "https://www.mangaupdates.com";

fn mu_web_url(item: &CreateMediaItem) -> Option<String> {
    (item.provider == "mangaupdates")
        .then(|| format!("{MU_WEB_BASE}/series.html?id={}", item.external_id))
}

pub async fn get_media_detail(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(params): Query<MediaDetailQuery>,
) -> impl IntoResponse {
    let media_type = params.media_type.as_deref().unwrap_or("movie");
    let resolved = resolve_card_data(&state, &provider, &external_id, media_type).await;

    let stats = get_sidebar_stats(&state, &user).await;

    match resolved {
        Some((mut item, fallback)) => {
            let from_cache = fallback.is_from_cache();
            let fallback_notice = fallback.notice().to_string();
            let mal_id = item.mal_id;
            if let Ok(Some(_)) = state
                .tracking
                .find_entry_by_media(user.id, &item.provider, &item.external_id)
                .await
            {
                item.is_tracked = true;
            }
            let flash_message = params
                .flash
                .as_deref()
                .map(|f| match f {
                    "added" => "✓ Медиа добавлено в список".to_string(),
                    "duplicate" => "Этот тайтл уже есть в базе".to_string(),
                    "error" => "Ошибка при добавлении".to_string(),
                    _ => String::new(),
                })
                .unwrap_or_default();

            let parsed = parse_description(item.description.as_deref());
            let status_label = derived_status_label(item.status.as_deref(), item.completed);
            let status_class = status_label.map(derived_status_class).unwrap_or("");
            let alt_titles = item.associated_titles.clone();
            let mu_url = mu_web_url(&item);

            Html(
                MediaDetailTemplate {
                    username: user.username,
                    role: user.role,
                    stats,
                    active_page: "search".to_string(),
                    item,
                    current_status: String::new(),
                    flash_message,
                    from_cache,
                    fallback_notice,
                    mal_id,
                    alt_titles,
                    status_label,
                    status_class,
                    description_clean: parsed.clean,
                    translations: parsed.translations,
                    original_novel: parsed.original_novel,
                    original_webtoon: parsed.original_webtoon,
                    mu_url,
                }
                .render()
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "template render failed");
                    String::from("Internal Server Error")
                }),
            )
            .into_response()
        }
        None => Html("Not found".to_string()).into_response(),
    }
}

pub async fn get_media_drawer_content(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(params): Query<MediaDetailQuery>,
) -> impl IntoResponse {
    let media_type = params.media_type.as_deref().unwrap_or("movie");
    let resolved = resolve_card_data(&state, &provider, &external_id, media_type).await;

    match resolved {
        Some((item, fallback)) => {
            let from_cache = fallback.is_from_cache();
            let fallback_notice = fallback.notice().to_string();
            let mal_id = item.mal_id;
            let tracking = state
                .tracking
                .find_entry_by_media(user.id, &provider, &external_id)
                .await
                .unwrap_or(None);
            let (tracking_id, current_status, progress, rating) = match tracking {
                Some((id, status, prog, rat)) => (Some(id), Some(status), Some(prog), rat),
                None => (None, None, None, None),
            };
            let total_count = item.total_count();
            let progress_unit = item.progress_unit_ru().to_string();
            let star_classes = MediaDrawerTemplate::compute_star_classes(rating);
            let rating_display = match rating {
                Some(r) => format!("{:.1}", r),
                None => "—".to_string(),
            };
            let status_display = current_status
                .clone()
                .unwrap_or_else(|| "in_progress".to_string());
            // Single source of truth for the "− N / M unit +1" row. The HTMX
            // progress endpoints build the same view model, so the buttons
            // can never drift out of sync with the value.
            let progress_row = ProgressRow::compute(
                tracking_id.unwrap_or_else(Uuid::nil),
                &item.media_type,
                total_count,
                item.progress_unit_ru(),
                progress,
                status_display.clone(),
            );
            let has_progress = progress_row.has_progress;
            let progress_display = progress_row.progress_display;
            let total_display = progress_row.total_display.clone();
            let can_increment = progress_row.can_increment;
            let can_decrement = progress_row.can_decrement;
            let parsed = parse_description(item.description.as_deref());
            let status_label = derived_status_label(item.status.as_deref(), item.completed);
            let status_class = status_label.map(derived_status_class).unwrap_or("");
            let alt_titles = item.associated_titles.clone();
            let mu_url = mu_web_url(&item);
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
                    can_decrement,
                    status_display,
                    from_cache,
                    fallback_notice,
                    mal_id,
                    alt_titles,
                    status_label,
                    status_class,
                    description_clean: parsed.clean,
                    translations: parsed.translations,
                    original_novel: parsed.original_novel,
                    original_webtoon: parsed.original_webtoon,
                    mu_url,
                }
                .render()
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "template render failed");
                    String::from("Internal Server Error")
                }),
            )
            .into_response()
        }
        None => Html("Not found".to_string()).into_response(),
    }
}

#[derive(Template)]
#[template(path = "partials/_episode_list.html")]
struct EpisodeListPartial {
    episodes: Vec<crate::services::episodes::StoredEpisode>,
    provider: String,
    external_id: String,
    mal_id: Option<i64>,
    episodes_hint: Option<i32>,
    windowed: bool,
    total: usize,
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

/// Разрешён ли клиентский `mal_id` для этого провайдера.
fn query_mal_id_allowed(provider: &str) -> bool {
    matches!(provider, "mal" | "shikimori" | "anilist")
}

/// Верхняя граница синтетических эпизодов: защищает от мусорного hint
/// и от генерации миллионов строк.
const SYNTHETIC_EPISODE_CAP: i32 = 1000;

/// Решить, можно ли синтезировать эпизоды `1..=n` из известного count.
///
/// Возвращает `Some(n)` только если:
/// - есть MAL id (ключ хранения эпизодов),
/// - провайдер anime-family (та же allowlist, что `query_mal_id_allowed`),
/// - n в диапазоне `1..=SYNTHETIC_EPISODE_CAP`.
fn synthetic_episode_count(
    mal_id: Option<i64>,
    provider: &str,
    episodes: Option<i32>,
) -> Option<i32> {
    let _ = mal_id?;
    if !query_mal_id_allowed(provider) {
        return None;
    }
    episodes.filter(|n| (1..=SYNTHETIC_EPISODE_CAP).contains(n))
}

/// Приоритет: явный query `mal_id` (только для доверенных провайдеров)
/// выигрывает у значения, выведенного из БД/`external_id`.
fn pick_mal_id(query_mal_id: Option<i64>, provider: &str, stored: Option<i64>) -> Option<i64> {
    if query_mal_id_allowed(provider)
        && let Some(id) = query_mal_id
    {
        return Some(id);
    }
    stored
}

/// Определить MAL id для эпизодов аниме.
async fn resolve_mal_id(
    query_mal_id: Option<i64>,
    provider: &str,
    external_id: &str,
    db: &sqlx::PgPool,
) -> Option<i64> {
    let stored = match provider {
        "mal" => external_id.parse::<i64>().ok(),
        "shikimori" | "anilist" => {
            crate::services::episodes::lookup_mal_id(db, provider, external_id)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(provider, external_id, error = %e, "lookup_mal_id failed");
                    None
                })
        }
        _ => None,
    };
    pick_mal_id(query_mal_id, provider, stored)
}

/// Compute the `[start, end)` slice of the episode list to render.
///
/// Mirrors the chapter window: `all`, a zero window, or a list no longer
/// than the window returns the whole list with `windowed = false`.
/// Otherwise the window starts at the first episode with
/// `episode_number > progress` (unknown progress => `0` => first window),
/// clamped so it never overruns the end of the list; when every episode is
/// watched the last `window` episodes are shown.
fn episode_window_range(
    episodes: &[crate::services::episodes::StoredEpisode],
    progress: i32,
    all: bool,
    window: usize,
) -> (usize, usize, bool) {
    let total = episodes.len();
    if all || window == 0 || total <= window {
        return (0, total, false);
    }
    let first_unwatched = episodes
        .iter()
        .position(|e| e.episode_number > progress)
        .unwrap_or(total);
    let start = first_unwatched.min(total - window);
    (start, start + window, true)
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
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(query): Query<EpisodesQuery>,
) -> impl IntoResponse {
    // Resolve the MAL id (episode key) for this anime.
    let mal_id: Option<i64> =
        resolve_mal_id(query.mal_id, &provider, &external_id, &state.db).await;

    // Try DB first (episodes are stored under provider="mal" keyed by mal_id).
    let mut existing = Vec::new();
    if let Some(mal_id) = mal_id {
        existing = crate::services::episodes::get_episodes(
            &state.db,
            "mal",
            &mal_id.to_string(),
            user.id,
            &provider,
            &external_id,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "media: failed to load existing episodes");
            Vec::new()
        });
    }

    // If empty, fetch on-demand via Jikan.
    if existing.is_empty() {
        if let Some(mal_id) = mal_id {
            if let Err(e) =
                crate::services::episodes::fetch_and_store_mal(state.db.clone(), &state.mal, mal_id)
                    .await
            {
                tracing::warn!(provider, external_id, mal_id, error = %e, "on-demand episode fetch failed");
            }
        } else {
            tracing::debug!(
                provider,
                external_id,
                "no mal_id available; cannot fetch episodes"
            );
        }
    }

    let mut episodes = match mal_id {
        Some(id) => crate::services::episodes::get_episodes(
            &state.db,
            "mal",
            &id.to_string(),
            user.id,
            &provider,
            &external_id,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "media: failed to load episodes");
            Vec::new()
        }),
        None => Vec::new(),
    };

    // Last-resort fallback: the catalog provider returned nothing (Tenrai
    // returns an empty list for some OVAs / short TV) but we still know the
    // episode count from the card metadata. Synthesize untitled rows so the
    // drawer has working tracking checkboxes instead of an empty state.
    if episodes.is_empty()
        && let Some(count) = synthetic_episode_count(mal_id, &provider, query.episodes)
        && let Some(id) = mal_id
    {
        match crate::services::episodes::store_synthetic_episodes(&state.db, id, count).await {
            Err(e) => {
                tracing::warn!(mal_id = id, count, error = %e, "store_synthetic_episodes failed");
            }
            Ok(()) => {
                episodes = crate::services::episodes::get_episodes(
                    &state.db,
                    "mal",
                    &id.to_string(),
                    user.id,
                    &provider,
                    &external_id,
                )
                .await
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "media: failed to reload synthetic episodes");
                    Vec::new()
                });
            }
        }
    }

    // Window the episode list server-side: 5 starting at the first
    // not-yet-watched episode (or the last 5 when everything is watched),
    // unless the client explicitly asks for the full list via `?all=true`.
    let progress = state
        .tracking
        .find_entry_by_media(user.id, &provider, &external_id)
        .await
        .ok()
        .flatten()
        .map(|(_, _, p, _)| p)
        .unwrap_or(0);
    let (start, end, windowed) = episode_window_range(
        &episodes,
        progress,
        query.all.unwrap_or(false),
        EPISODE_WINDOW,
    );
    let total = episodes.len();
    let episodes = episodes[start..end].to_vec();

    let html = EpisodeListPartial {
        episodes,
        provider: provider.clone(),
        external_id: external_id.clone(),
        mal_id,
        episodes_hint: query.episodes,
        windowed,
        total,
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
        "shikimori" => {
            match crate::services::episodes::lookup_mal_id(&state.db, &provider, &external_id).await
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

    let mal_id = match mal_id {
        Some(id) => id,
        None => {
            tracing::debug!(
                provider,
                external_id,
                episode_number,
                "no mal_id available, ignoring toggle"
            );
            return Html(String::new()).into_response();
        }
    };

    if let Err(e) = crate::services::episodes::set_watched(
        &state.db,
        user.id,
        &provider,
        &external_id,
        mal_id,
        episode_number,
        form.watched,
    )
    .await
    {
        tracing::warn!(provider, external_id, mal_id, episode_number, error = %e, "set_watched failed");
        return Html(String::new()).into_response();
    }

    // Recompute progress for the tracking entry (if any).
    let max_watched = crate::services::episodes::count_watched(
        &state.db,
        user.id,
        &provider,
        &external_id,
        mal_id,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "media: failed to count watched episodes");
        0
    });

    // Resolve media_id once (for both progress sync and HX-Trigger broadcast).
    let media_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM media_items WHERE provider = $1 AND external_id = $2")
            .bind(&provider)
            .bind(&external_id)
            .fetch_optional(&state.db)
            .await
            .unwrap_or(None);

    let mut progress_from_db: Option<i32> = None;
    if let Some(media_id) = media_id {
        match crate::services::episodes::update_progress_from_watched(
            &state.db,
            user.id,
            media_id,
            max_watched,
        )
        .await
        {
            Ok(progress) => progress_from_db = progress,
            Err(e) => {
                tracing::warn!(provider, external_id, error = %e, "update_progress_from_watched failed");
            }
        }
    }

    // Render the new row HTML and attach a progressUpdated event.
    let html = match crate::services::episodes::get_episode(
        &state.db,
        mal_id,
        episode_number,
        user.id,
        &provider,
        &external_id,
    )
    .await
    {
        Ok(Some(ep)) => EpisodeItemPartial {
            episode: ep,
            provider: provider.clone(),
            external_id: external_id.clone(),
        }
        .render()
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "media: episode item render failed");
            String::new()
        }),
        _ => String::new(),
    };

    // Pull authoritative state for ALL episodes so the drawer can sync
    // every visible checkbox (bulk-fill on watch flips 1..N rows; the
    // single-row HTMX swap only refreshes the clicked one).
    let states = crate::services::episodes::get_episode_states(
        &state.db,
        user.id,
        &provider,
        &external_id,
        mal_id,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "media: failed to load episode states");
        Vec::new()
    });
    let states_json: Vec<[serde_json::Value; 2]> = states
        .into_iter()
        .map(|(n, w)| [serde_json::Value::from(n), serde_json::Value::from(w)])
        .collect();

    let mut trigger = serde_json::json!({
        "progressUpdated": {
            "maxWatched": max_watched,
            "progress": progress_from_db,
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
    crate::utils::set_hx_trigger(&mut resp, &trigger.to_string());
    resp
}

// ─── CHAPTERS (manga-like types) ──────────────────────────────

#[derive(Template)]
#[template(path = "partials/_chapter_list.html")]
struct ChapterListPartial {
    chapters: Vec<crate::services::chapters::StoredChapter>,
    provider: String,
    external_id: String,
    windowed: bool,
    total: usize,
}

#[derive(Template)]
#[template(path = "partials/_chapter_item.html")]
struct ChapterItemPartial {
    chapter: crate::services::chapters::StoredChapter,
    provider: String,
    external_id: String,
}

#[derive(Template)]
#[template(path = "partials/_game_additions.html")]
// provider/external_id are passed for the drawer's data-* wiring; the current
// template does not read them yet, so silence the dead_code lint here.
#[allow(dead_code)]
struct GameAdditionsPartial {
    additions: Vec<crate::services::external::AdditionRow>,
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
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(query): Query<ChaptersQuery>,
) -> impl IntoResponse {
    // For mangaupdates the storage key is (provider, external_id) as-is.
    // For other sources we'd need a lookup — for now handle mangaupdates directly.
    let (mu_provider, mu_id) = match provider.as_str() {
        "mangaupdates" => ("mangaupdates".to_string(), external_id.clone()),
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
                None => ("mangaupdates".to_string(), external_id.clone()),
            }
        }
    };

    // Try DB first.
    let existing =
        crate::services::chapters::get_chapters(&state.db, &mu_provider, &mu_id, user.id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "media: failed to load chapters");
                Vec::new()
            });

    // If empty, try to fetch the latest_chapter from MangaUpdates and build skeleton.
    if existing.is_empty()
        && let Ok(series_id_num) = mu_id.parse::<i64>()
    {
        let details = crate::services::external::mangaupdates::MangaUpdatesService::new()
            .get_details(&mu_id)
            .await;

        if let Ok(details) = details {
            let lc = details.chapters.unwrap_or(0);
            if lc > 0
                && let Err(e) =
                    crate::services::chapters::store_chapters_mu(&state.db, series_id_num, lc).await
            {
                tracing::warn!(series_id_num, error = %e, "store_chapters_mu failed");
            }
        }
    }

    let chapters =
        crate::services::chapters::get_chapters(&state.db, &mu_provider, &mu_id, user.id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "media: failed to load chapters after enrich");
                Vec::new()
            });

    // Auto-enrich from MangaDex if chapters lack titles (async, non-blocking)
    let needs_enrich = chapters
        .iter()
        .any(|c| c.title_en.is_none() && c.title_ru.is_none());
    if needs_enrich {
        let db = state.db.clone();
        let prov = mu_provider.to_string();
        let ext_id = mu_id.to_string();
        tokio::spawn(async move {
            if let Err(e) =
                crate::services::chapters::enrich_from_mangadex(&db, &prov, &ext_id).await
            {
                tracing::warn!(provider=%prov, external_id=%ext_id, error=%e, "MangaDex enrichment failed");
            }
        });
    }

    let all = query.all.unwrap_or(false);
    let total = chapters.len();
    let (chapters, windowed) = if !all && total > CHAPTER_WINDOW {
        let start = total - CHAPTER_WINDOW;
        (chapters.into_iter().skip(start).collect::<Vec<_>>(), true)
    } else {
        (chapters, false)
    };

    let html = ChapterListPartial {
        chapters,
        provider: mu_provider.to_string(),
        external_id: mu_id.to_string(),
        windowed,
        total,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "chapter list render failed");
        String::new()
    });
    Html(html)
}

/// Lazy-loaded DLC / expansions list for a game drawer section.
/// DB-first: only calls the provider when the cache is empty.
pub async fn get_game_additions(
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let existing = crate::services::additions::get_additions(&state.db, &provider, &external_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "media: failed to load game additions");
            Vec::new()
        });

    if existing.is_empty()
        && let Err(e) = crate::services::additions::fetch_and_store(
            &state.db,
            &provider,
            &external_id,
            &state.rawg,
            &state.igdb,
        )
        .await
    {
        tracing::warn!(
            provider = %provider,
            external_id = %external_id,
            error = %e,
            "media: game additions fetch failed"
        );
    }

    let additions = crate::services::additions::get_additions(&state.db, &provider, &external_id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "media: failed to reload game additions");
            Vec::new()
        });

    let html = GameAdditionsPartial {
        additions,
        provider,
        external_id,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "media: game additions render failed");
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
    let media_id: Option<Uuid> =
        crate::services::chapters::lookup_media_id(&state.db, &provider, &external_id)
            .await
            .unwrap_or(None);

    if let Err(e) = crate::services::chapters::set_read(
        &state.db,
        user.id,
        &provider,
        &external_id,
        chapter_number,
        form.read,
    )
    .await
    {
        tracing::warn!(provider, external_id, chapter_number, error = %e, "set_read failed");
        return Html(String::new()).into_response();
    }

    // Recompute progress.
    let max_read =
        crate::services::chapters::count_read(&state.db, user.id, &provider, &external_id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "media: failed to count read chapters");
                0
            });

    if let Some(media_id) = media_id
        && let Err(e) = crate::services::chapters::update_progress_from_read(
            &state.db, user.id, media_id, max_read,
        )
        .await
    {
        tracing::warn!(error = %e, "update_progress_from_read failed");
    }

    // Render updated row.
    let chapter = crate::services::chapters::get_chapter(
        &state.db,
        &provider,
        &external_id,
        chapter_number,
        user.id,
    )
    .await
    .unwrap_or(None);

    let html = match chapter {
        Some(ch) => ChapterItemPartial {
            chapter: ch,
            provider: provider.clone(),
            external_id: external_id.clone(),
        }
        .render()
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "media: chapter item render failed");
            String::new()
        }),
        None => String::new(),
    };

    // Build HX-Trigger with full chapter states.
    let states =
        crate::services::chapters::get_chapter_states(&state.db, user.id, &provider, &external_id)
            .await
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "media: failed to load chapter states");
                Vec::new()
            });
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
    crate::utils::set_hx_trigger(&mut resp, &trigger.to_string());
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_mal_id_allowed_only_anime_providers() {
        assert!(query_mal_id_allowed("mal"));
        assert!(query_mal_id_allowed("shikimori"));
        assert!(query_mal_id_allowed("anilist"));
        assert!(!query_mal_id_allowed("tmdb"));
        assert!(!query_mal_id_allowed("manual"));
        assert!(!query_mal_id_allowed("mangaupdates"));
    }

    #[test]
    fn pick_mal_id_query_wins_for_allowed_provider() {
        assert_eq!(pick_mal_id(Some(42), "anilist", None), Some(42));
        assert_eq!(pick_mal_id(Some(42), "shikimori", Some(7)), Some(42));
    }

    #[test]
    fn pick_mal_id_ignores_query_for_untrusted_provider() {
        assert_eq!(pick_mal_id(Some(42), "tmdb", Some(7)), Some(7));
        assert_eq!(pick_mal_id(Some(42), "tmdb", None), None);
    }

    #[test]
    fn pick_mal_id_falls_back_to_stored() {
        assert_eq!(pick_mal_id(None, "anilist", Some(9)), Some(9));
        assert_eq!(pick_mal_id(None, "tmdb", None), None);
    }

    #[test]
    fn fresh_is_not_from_cache_and_has_no_notice() {
        assert!(!Fallback::Fresh.is_from_cache());
        assert_eq!(Fallback::Fresh.notice(), "");
    }

    #[test]
    fn provider_gone_notice_mentions_deleted_source() {
        assert!(Fallback::ProviderGone.is_from_cache());
        assert!(Fallback::ProviderGone.notice().contains("удалил тайтл"));
        assert!(
            Fallback::ProviderGone
                .notice()
                .contains("сохранённые данные")
        );
    }

    #[test]
    fn provider_unavailable_notice_mentions_unreachable_source() {
        assert!(Fallback::ProviderUnavailable.is_from_cache());
        assert!(
            Fallback::ProviderUnavailable
                .notice()
                .contains("недоступен")
        );
        assert!(
            Fallback::ProviderUnavailable
                .notice()
                .contains("сохранённые данные")
        );
    }

    #[test]
    fn synthetic_count_rejects_missing_mal_id() {
        assert_eq!(synthetic_episode_count(None, "mal", Some(12)), None);
    }

    #[test]
    fn synthetic_count_rejects_non_anime_provider() {
        assert_eq!(synthetic_episode_count(Some(1), "tmdb", Some(12)), None);
        assert_eq!(synthetic_episode_count(Some(1), "manual", Some(12)), None);
        assert_eq!(
            synthetic_episode_count(Some(1), "mangaupdates", Some(12)),
            None
        );
    }

    #[test]
    fn synthetic_count_rejects_out_of_range() {
        assert_eq!(synthetic_episode_count(Some(1), "mal", None), None);
        assert_eq!(synthetic_episode_count(Some(1), "mal", Some(0)), None);
        assert_eq!(synthetic_episode_count(Some(1), "mal", Some(-3)), None);
        assert_eq!(
            synthetic_episode_count(Some(1), "mal", Some(SYNTHETIC_EPISODE_CAP + 1)),
            None
        );
    }

    #[test]
    fn synthetic_count_accepts_valid_anime_hint() {
        assert_eq!(
            synthetic_episode_count(Some(30458), "mal", Some(1)),
            Some(1)
        );
        assert_eq!(
            synthetic_episode_count(Some(30458), "shikimori", Some(2)),
            Some(2)
        );
        assert_eq!(
            synthetic_episode_count(Some(30458), "anilist", Some(5)),
            Some(5)
        );
        assert_eq!(
            synthetic_episode_count(Some(30458), "mal", Some(SYNTHETIC_EPISODE_CAP)),
            Some(SYNTHETIC_EPISODE_CAP)
        );
    }

    fn ep(n: i32) -> crate::services::episodes::StoredEpisode {
        crate::services::episodes::StoredEpisode {
            episode_number: n,
            title_en: None,
            title_ru: None,
            title_jp: None,
            air_date: None,
            duration_minutes: None,
            watched: false,
        }
    }

    #[test]
    fn episode_window_empty_list() {
        let eps: Vec<_> = Vec::new();
        assert_eq!(
            episode_window_range(&eps, 0, false, EPISODE_WINDOW),
            (0, 0, false)
        );
    }

    #[test]
    fn episode_window_short_list_not_windowed() {
        let eps: Vec<_> = (1..=3).map(ep).collect();
        assert_eq!(
            episode_window_range(&eps, 0, false, EPISODE_WINDOW),
            (0, 3, false)
        );
    }

    #[test]
    fn episode_window_exact_window_not_windowed() {
        let eps: Vec<_> = (1..=5).map(ep).collect();
        assert_eq!(
            episode_window_range(&eps, 0, false, EPISODE_WINDOW),
            (0, 5, false)
        );
    }

    #[test]
    fn episode_window_without_progress_starts_first() {
        let eps: Vec<_> = (1..=10).map(ep).collect();
        assert_eq!(
            episode_window_range(&eps, 0, false, EPISODE_WINDOW),
            (0, 5, true)
        );
    }

    #[test]
    fn episode_window_with_progress_starts_at_first_unwatched() {
        let eps: Vec<_> = (1..=10).map(ep).collect();
        assert_eq!(
            episode_window_range(&eps, 4, false, EPISODE_WINDOW),
            (4, 9, true)
        );
    }

    #[test]
    fn episode_window_all_watched_shows_last() {
        let eps: Vec<_> = (1..=10).map(ep).collect();
        assert_eq!(
            episode_window_range(&eps, 10, false, EPISODE_WINDOW),
            (5, 10, true)
        );
    }

    #[test]
    fn episode_window_all_flag_returns_full_list() {
        let eps: Vec<_> = (1..=10).map(ep).collect();
        assert_eq!(
            episode_window_range(&eps, 4, true, EPISODE_WINDOW),
            (0, 10, false)
        );
    }
}
