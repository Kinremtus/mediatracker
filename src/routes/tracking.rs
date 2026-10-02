use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

use super::home::{SidebarStats, get_sidebar_stats};
use super::progress::ProgressRow;
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::tracking_entry::{TrackingEntryWithMedia, UpdateTracking};
use crate::services::external::dispatch::{Provider, ProviderClients};

#[derive(Template)]
#[template(path = "tracking_list.html")]
#[expect(dead_code)]
struct TrackingListTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    entries: Vec<TrackingEntryWithMedia>,
    status_label: String,
    current_status: String,
    current_media_type: String,
    search_query: String,
    media_types: Vec<MediaTypeItem>,
    statuses: Vec<StatusItem>,
    current_type_label: String,
}

pub(crate) struct MediaTypeItem {
    pub(crate) key: String,
    pub(crate) icon: String,
    pub(crate) label: String,
}

struct StatusItem {
    key: String,
    label: String,
}

/// Canonical media-type registry: `(key, icon, label)`.
/// Single source of truth for the search page, the tracking list, the manual
/// form and manual-add validation.
pub(crate) fn get_all_media_types() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("anime", "▶", "Аниме"),
        ("manga", "📚", "Манга"),
        ("manhwa", "📚", "Манхва"),
        ("manhua", "📚", "Маньхуа"),
        ("novel", "📝", "Новеллы"),
        ("comic", "🦸", "Комиксы"),
        ("other-comics", "📚", "Другие комиксы"),
        ("movie", "🎥", "Фильмы"),
        ("series", "📺", "Сериалы"),
        ("dramas", "🎬", "Дорамы"),
        ("cartoons", "📺", "Мультсериалы"),
        ("animated-movies", "🎞", "Мультфильмы"),
        ("game", "🎮", "Игры"),
        ("book", "📖", "Книги"),
    ]
}

/// Media types accepted by `post_manual_add` — derived from the canonical list.
pub(crate) fn manual_media_types() -> Vec<&'static str> {
    get_all_media_types()
        .into_iter()
        .map(|(k, _, _)| k)
        .collect()
}

/// Canonical list rendered as template items (search page + manual form).
pub(crate) fn all_media_type_items() -> Vec<MediaTypeItem> {
    get_all_media_types()
        .into_iter()
        .map(|(k, i, l)| MediaTypeItem {
            key: k.to_string(),
            icon: i.to_string(),
            label: l.to_string(),
        })
        .collect()
}

fn get_status_label(status: &str) -> String {
    match status {
        "in_progress" => "В процессе",
        "completed" => "Завершено",
        "planned" => "Запланировано",
        "dropped" => "Брошено",
        "paused" => "Приостановлено",
        _ => "Все списки",
    }
    .to_string()
}

#[derive(Deserialize)]
pub struct TrackingQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    media_type: Option<String>,
    q: Option<String>,
}

#[derive(Deserialize)]
pub struct ManualFormQuery {
    flash: Option<String>,
}

#[derive(Template)]
#[template(path = "tracking_manual_form.html")]
#[expect(dead_code)]
struct ManualFormTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    media_types: Vec<MediaTypeItem>,
    statuses: Vec<StatusItem>,
    flash_message: String,
    metric_fields: &'static [ManualMetricField],
    media_type_default: String,
}

#[derive(Deserialize)]
pub struct ManualAddForm {
    pub title: String,
    pub media_type: String,
    #[serde(default)]
    pub tracking_status: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub poster_url: Option<String>,
    #[serde(default)]
    pub year: Option<i16>,
    #[serde(default)]
    pub episodes: Option<i32>,
    #[serde(default)]
    pub chapters: Option<i32>,
    #[serde(default)]
    pub volumes: Option<i32>,
    #[serde(default)]
    pub pages: Option<i32>,
    #[serde(default)]
    pub runtime_minutes: Option<i32>,
    #[serde(default)]
    pub playtime_hours: Option<i32>,
}

/// Статусы трекинга (migrations/004_status_rename.sql).
const MANUAL_STATUSES: &[&str] = &["in_progress", "completed", "planned", "dropped", "paused"];

/// Поле метрики ручной формы: имя input'а, подпись и типы медиа,
/// для которых метрика имеет смысл.
pub struct ManualMetricField {
    pub name: &'static str,
    pub label: &'static str,
    /// Подпись для media_type == "comic" («Выпуски» вместо «Главы»).
    pub label_comic: Option<&'static str>,
    pub types: &'static [&'static str],
}

impl ManualMetricField {
    /// JS-выражение для Alpine `x-show` (список типов в одинарных кавычках).
    pub fn alpine_show(&self) -> String {
        let list = self
            .types
            .iter()
            .map(|t| format!("'{t}'"))
            .collect::<Vec<_>>()
            .join(",");
        format!("[{list}].includes(mediaType)")
    }
}

/// Метрики ручного добавления и типы медиа, к которым они применимы.
pub const MANUAL_METRIC_FIELDS: &[ManualMetricField] = &[
    ManualMetricField {
        name: "episodes",
        label: "Эпизоды",
        label_comic: None,
        types: &["anime", "series", "cartoons", "animated-movies"],
    },
    ManualMetricField {
        name: "chapters",
        label: "Главы",
        label_comic: Some("Выпуски"),
        types: &[
            "manga",
            "manhwa",
            "manhua",
            "novel",
            "comic",
            "other-comics",
        ],
    },
    ManualMetricField {
        name: "volumes",
        label: "Тома",
        label_comic: None,
        types: &[
            "manga",
            "manhwa",
            "manhua",
            "novel",
            "comic",
            "other-comics",
        ],
    },
    ManualMetricField {
        name: "pages",
        label: "Страницы",
        label_comic: None,
        types: &["book"],
    },
    ManualMetricField {
        name: "runtime_minutes",
        label: "Длительность, мин",
        label_comic: None,
        types: &["movie", "dramas"],
    },
    ManualMetricField {
        name: "playtime_hours",
        label: "Время игры, ч",
        label_comic: None,
        types: &["game"],
    },
];

/// Применима ли метрика `metric` к типу медиа `media_type`.
fn manual_metric_relevant(media_type: &str, metric: &str) -> bool {
    MANUAL_METRIC_FIELDS
        .iter()
        .filter(|f| f.name == metric)
        .any(|f| f.types.contains(&media_type))
}

fn manual_flash(flash: Option<&str>) -> String {
    match flash {
        Some("error") => "Не удалось добавить: проверьте название и тип".to_string(),
        _ => String::new(),
    }
}

pub async fn get_tracking_list(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<TrackingQuery>,
) -> Response {
    let status = params.status.as_deref();
    let media_type = params.media_type.as_deref();
    let search_query = params.q.as_deref();
    let entries = match state
        .tracking
        .get_user_entries(user.id, status, media_type, search_query)
        .await
    {
        Ok(entries) => entries,
        Err(e) => return super::internal_error("tracking: list entries", e),
    };
    let stats = get_sidebar_stats(&state, &user).await;
    let current_status = params.status.unwrap_or_default();
    let current_media_type = params.media_type.unwrap_or_default();
    let search_query = params.q.unwrap_or_default();
    let status_label = get_status_label(&current_status);

    let all_types = get_all_media_types();
    let current_type_label = if current_media_type.is_empty() {
        "Все".to_string()
    } else {
        all_types
            .iter()
            .find(|(k, _, _)| *k == current_media_type)
            .map(|(_, _, l)| l.to_string())
            .unwrap_or_default()
    };
    let media_types = all_media_type_items();
    let statuses: Vec<StatusItem> = vec![
        ("", "Все списки"),
        ("in_progress", "В процессе"),
        ("completed", "Завершено"),
        ("planned", "Запланировано"),
        ("dropped", "Брошено"),
    ]
    .into_iter()
    .map(|(k, l)| StatusItem {
        key: k.to_string(),
        label: l.to_string(),
    })
    .collect();

    TrackingListTemplate {
        username: user.username,
        role: user.role.clone(),
        stats,
        active_page: "tracking".to_string(),
        entries,
        status_label,
        current_status,
        current_media_type,
        search_query,
        media_types,
        statuses,
        current_type_label,
    }
    .render()
    .map(Html)
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        Html(String::from("Internal Server Error"))
    })
    .into_response()
}

pub async fn get_manual_form(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<ManualFormQuery>,
) -> Response {
    let stats = get_sidebar_stats(&state, &user).await;
    let media_types = all_media_type_items();
    let media_type_default = media_types
        .first()
        .map(|t| t.key.clone())
        .unwrap_or_default();
    let statuses = MANUAL_STATUSES
        .iter()
        .map(|k| StatusItem {
            key: (*k).to_string(),
            label: get_status_label(k),
        })
        .collect();

    ManualFormTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "tracking".to_string(),
        media_types,
        statuses,
        metric_fields: MANUAL_METRIC_FIELDS,
        media_type_default,
        flash_message: manual_flash(params.flash.as_deref()),
    }
    .render()
    .map(Html)
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        Html(String::from("Internal Server Error"))
    })
    .into_response()
}

pub async fn post_manual_add(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<ManualAddForm>,
) -> Response {
    let title = form.title.trim().to_string();
    let media_type = form.media_type.trim().to_string();
    let status = if form.tracking_status.trim().is_empty() {
        "planned".to_string()
    } else {
        form.tracking_status.trim().to_string()
    };

    if title.is_empty()
        || !manual_media_types().contains(&media_type.as_str())
        || !MANUAL_STATUSES.contains(&status.as_str())
    {
        tracing::warn!(
            media_type = %media_type,
            status = %status,
            "manual add rejected"
        );
        return Redirect::to("/tracking/manual?flash=error").into_response();
    }

    match state.tracking.find_duplicate(&media_type, &title).await {
        Ok(Some((provider, external_id))) => {
            tracing::info!(provider, external_id, "manual add blocked: duplicate");
            return Redirect::to(&format!("/media/{provider}/{external_id}?flash=duplicate"))
                .into_response();
        }
        Ok(None) => {}
        Err(e) => {
            tracing::error!(error = %e, "manual duplicate lookup failed");
            return Redirect::to("/tracking/manual?flash=error").into_response();
        }
    }

    let external_id = Uuid::new_v4().to_string();

    // Оставляем только метрики, релевантные выбранному типу:
    // остальные обнуляем, чтобы в БД не оседали чужие поля.
    let episodes = form
        .episodes
        .filter(|_| manual_metric_relevant(&media_type, "episodes"));
    let chapters = form
        .chapters
        .filter(|_| manual_metric_relevant(&media_type, "chapters"));
    let volumes = form
        .volumes
        .filter(|_| manual_metric_relevant(&media_type, "volumes"));
    let pages = form
        .pages
        .filter(|_| manual_metric_relevant(&media_type, "pages"));
    let runtime_minutes = form
        .runtime_minutes
        .filter(|_| manual_metric_relevant(&media_type, "runtime_minutes"));
    let playtime_hours = form
        .playtime_hours
        .filter(|_| manual_metric_relevant(&media_type, "playtime_hours"));

    let media = crate::models::media_item::CreateMediaItem {
        provider: "manual".to_string(),
        external_id: external_id.clone(),
        media_type,
        title,
        description: form.description.filter(|s| !s.trim().is_empty()),
        poster_url: form.poster_url.filter(|s| !s.trim().is_empty()),
        year: form.year,
        episodes,
        chapters,
        volumes,
        pages,
        runtime_minutes,
        playtime_hours,
        ..Default::default()
    };

    match state.tracking.add_to_list(user.id, &media, &status).await {
        Ok(_) => Redirect::to(&format!("/media/manual/{external_id}")).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "manual add failed");
            Redirect::to("/tracking/manual?flash=error").into_response()
        }
    }
}

fn csv_str<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(de).unwrap_or_default();
    Ok(raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect())
}

#[derive(Deserialize)]
pub struct AddToTrackingForm {
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub title_russian: Option<String>,
    pub poster_url: Option<String>,
    pub episodes: Option<i32>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub score: Option<f64>,
    #[serde(default)]
    pub mal_id: Option<i64>,
    #[serde(default)]
    pub shikimori_id: Option<i64>,
    pub tracking_status: String,
    pub redirect_to: Option<String>,

    // === Расширенные метаданные ===
    pub format_type: Option<String>,
    pub chapters: Option<i32>,
    pub volumes: Option<i32>,
    pub pages: Option<i32>,
    pub runtime_minutes: Option<i32>,
    pub playtime_hours: Option<i32>,
    pub year: Option<i16>,
    pub aired_from: Option<chrono::NaiveDate>,
    pub aired_to: Option<chrono::NaiveDate>,
    pub premiered_season: Option<String>,
    pub premiered_year: Option<i16>,
    pub broadcast: Option<String>,
    pub completed: Option<bool>,
    pub licensed: Option<bool>,
    pub source: Option<String>,
    pub duration: Option<String>,
    pub rating: Option<String>,
    pub rating_votes: Option<i32>,
    #[serde(default, deserialize_with = "csv_str")]
    pub authors: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub artists: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub studios: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub producers: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub licensors: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub publishers: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub serialized_in: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub networks: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub platforms: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub genres: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub themes: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub demographics: Vec<String>,
    #[serde(default, deserialize_with = "csv_str")]
    pub categories: Vec<String>,
}

pub async fn post_add_to_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AddToTrackingForm>,
) -> Response {
    let needs_tmdb_fetch = form.provider == "tmdb" && form.episodes.is_none();
    let ext_id = form.external_id.clone();
    let media_type = form.media_type.clone();
    let needs_rawg = form.provider == "rawg";
    let needs_openlib = form.provider == "openlibrary";

    let mut media = crate::models::media_item::CreateMediaItem {
        provider: form.provider,
        external_id: form.external_id,
        media_type: form.media_type,
        title: form.title,
        title_english: form.title_english,
        title_native: form.title_native,
        title_russian: form.title_russian,
        poster_url: form.poster_url,
        episodes: form.episodes,
        seasons: None,
        description: form.description,
        status: form.status,
        score: form.score,
        is_tracked: false,
        mal_id: form.mal_id,
        shikimori_id: form.shikimori_id,
        comparison_key: None,
        format_type: form.format_type,
        details: None,
        chapters: form.chapters,
        volumes: form.volumes,
        pages: form.pages,
        runtime_minutes: form.runtime_minutes,
        playtime_hours: form.playtime_hours,
        year: form.year,
        aired_from: form.aired_from,
        aired_to: form.aired_to,
        premiered_season: form.premiered_season,
        premiered_year: form.premiered_year,
        broadcast: form.broadcast,
        completed: form.completed,
        licensed: form.licensed,
        source: form.source,
        duration: form.duration,
        rating: form.rating,
        rating_votes: form.rating_votes,
        authors: form.authors,
        artists: form.artists,
        studios: form.studios,
        producers: form.producers,
        licensors: form.licensors,
        publishers: form.publishers,
        serialized_in: form.serialized_in,
        networks: form.networks,
        platforms: form.platforms,
        genres: form.genres,
        themes: form.themes,
        demographics: form.demographics,
        categories: form.categories,
        associated_titles: Vec::new(),
    };

    // The add form does not carry associated/alternative titles (search results
    // do not include them). Pull them from the provider's detail endpoint so the
    // item is findable by its native/alternative titles in the admin panel.
    if media.associated_titles.is_empty() {
        let clients = ProviderClients::from_state(&state);
        if let Some(p) = Provider::from_name(&clients, &media.provider)
            && let Ok(details) = p.fetch(&media.external_id, &media.media_type).await
        {
            media.associated_titles = details.associated_titles;
            if media.title_native.as_deref().unwrap_or("").is_empty() {
                media.title_native = details.title_native;
            }
        }
    }

    let status = if form.tracking_status.is_empty() {
        "planned"
    } else {
        &form.tracking_status
    };

    let redirect_url = crate::utils::safe_redirect_path(form.redirect_to.as_deref(), "/tracking");

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    match state.tracking.add_to_list(user.id, &media, status).await {
        Ok(_) => {
            // Official-source auto-verification: if a binding already exists for
            // the source matching this media type, enrich the chapter counter
            // right after the add (bounded, non-fatal — never fails the add).
            if let Some(source) =
                crate::services::official_resolve::official_source_for_media_type(&media.media_type)
                && let Ok(Some(_)) = crate::services::source_ids::get_source_id(
                    &state.db,
                    &media.provider,
                    &media.external_id,
                    source,
                )
                .await
            {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    crate::services::chapter_enrich::enrich_chapter_count(
                        &state.db,
                        &media.provider,
                        &media.external_id,
                        &media.media_type,
                    ),
                )
                .await;
            }

            // For anime, fire-and-forget fetch of the episode list via
            // Jikan v4 so the drawer's "Эпизоды" section has data ready
            // by the time the user opens it. The drawer also falls back
            // to on-demand fetch if this hasn't completed. We need a
            // MAL id to hit Jikan — most Shikimori-sourced anime carry
            // one in `CreateMediaItem.mal_id`. Anime without any MAL id
            // (very rare) just won't show episodes for now.
            if media.media_type == "anime" {
                let mal_id = media.mal_id;
                if let Some(mal_id) = mal_id {
                    let pool = state.db.clone();
                    let service = state.mal.clone();
                    let provider = media.provider.clone();
                    let external_id = media.external_id.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            crate::services::episodes::fetch_and_store_mal(pool, &service, mal_id)
                                .await
                        {
                            tracing::warn!(provider, external_id, mal_id, error = %e, "background episode fetch failed");
                        }
                    });
                }
            }

            // Если добавляем TMDB элемент без эпизодов — подтягиваем полные данные
            if needs_tmdb_fetch {
                match state.tmdb.get_details(&ext_id, &media_type).await {
                    Ok(detailed) => {
                        let _ = sqlx::query(
                            "UPDATE media_items SET episodes = $1, runtime_minutes = $2, genres = $3 WHERE provider = 'tmdb' AND external_id = $4"
                        )
                        .bind(detailed.episodes)
                        .bind(detailed.runtime_minutes)
                        .bind(&detailed.genres)
                        .bind(&ext_id)
                        .execute(&state.db)
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to fetch TMDB details for backfill");
                    }
                }
            }

            // RAWG backfill — подтягиваем жанры (search их не возвращает)
            if needs_rawg {
                match state.rawg.get_details(&ext_id).await {
                    Ok(detailed) => {
                        let _ = sqlx::query(
                            "UPDATE media_items SET genres = $1 WHERE provider = 'rawg' AND external_id = $2"
                        )
                        .bind(&detailed.genres)
                        .bind(&ext_id)
                        .execute(&state.db)
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to fetch RAWG details for genre backfill");
                    }
                }
            }

            // OpenLibrary backfill — подтягиваем жанры, темы и категории (из work-эндпоинта)
            if needs_openlib {
                match state.openlibrary.get_details(&ext_id).await {
                    Ok(detailed) => {
                        let _ = sqlx::query(
                            "UPDATE media_items SET genres = $1, themes = $2, categories = $3 WHERE provider = 'openlibrary' AND external_id = $4"
                        )
                        .bind(&detailed.genres)
                        .bind(&detailed.themes)
                        .bind(&detailed.categories)
                        .bind(&ext_id)
                        .execute(&state.db)
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to fetch OpenLibrary details for genre backfill");
                    }
                }
            }

            if is_htmx {
                let mut resp = Html(ADDED_BADGE_HTML.to_string()).into_response();
                crate::utils::set_hx_trigger(&mut resp, "trackingUpdated");
                resp
            } else {
                let url = add_flash_param(&redirect_url, "added");
                Redirect::to(&url).into_response()
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to add to tracking");
            if is_htmx {
                let mut resp = Html(r#"<span class="btn btn-secondary" style="width:100%;height:32px;font-size:12px;display:flex;align-items:center;justify-content:center;cursor:default;opacity:0.6;color:var(--dropped);">✕ Ошибка</span>"#.to_string()).into_response();
                crate::utils::set_hx_trigger(&mut resp, "trackingError");
                resp
            } else {
                let url = add_flash_param(&redirect_url, "error");
                Redirect::to(&url).into_response()
            }
        }
    }
}

const ADDED_BADGE_HTML: &str = r#"<span class="btn btn-secondary added-badge">✓ Добавлено</span>"#;

fn add_flash_param(url: &str, flash: &str) -> String {
    if url.contains('?') {
        format!("{}&flash={}", url, flash)
    } else {
        format!("{}?flash={}", url, flash)
    }
}

#[derive(Deserialize)]
pub struct UpdateTrackingForm {
    pub status: Option<String>,
    pub rating: Option<f64>,
    pub progress: Option<i32>,
    #[serde(default)]
    pub increment: Option<i32>,
}

pub async fn post_update_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateTrackingForm>,
) -> Response {
    let update = UpdateTracking {
        status: form.status,
        rating: form.rating,
        progress: form.progress,
    };

    match state.tracking.update_entry(id, user.id, &update).await {
        Ok(_) => Redirect::to("/tracking").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to update tracking");
            Redirect::to("/tracking").into_response()
        }
    }
}

pub async fn post_delete_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.tracking.delete_entry(id, user.id).await {
        Ok(_) => Redirect::to("/tracking").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete tracking");
            Redirect::to("/tracking").into_response()
        }
    }
}

// ========== HTMX Endpoints ==========

#[derive(Template)]
#[template(path = "partials/tracking_card.html")]
struct TrackingCardPartial {
    entry_with_media: TrackingEntryWithMedia,
}

#[derive(Template)]
#[template(path = "partials/tracking_grid.html")]
#[expect(dead_code)]
struct TrackingGridPartial {
    entries: Vec<TrackingEntryWithMedia>,
    current_status: String,
    current_media_type: String,
    search_query: String,
}

#[derive(Template)]
#[template(path = "partials/message.html")]
struct MessagePartial {
    message: Option<String>,
    error: Option<String>,
}

pub async fn htmx_update_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateTrackingForm>,
) -> Response {
    let progress = if let Some(delta) = form.increment {
        let current: i32 = sqlx::query_scalar(
            "SELECT COALESCE(progress, 0) FROM tracking_entries WHERE id = $1 AND user_id = $2",
        )
        .bind(id)
        .bind(user.id)
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
        Some((current + delta).max(0))
    } else {
        form.progress
    };

    let update = UpdateTracking {
        status: form.status,
        rating: form.rating,
        progress,
    };

    match state.tracking.update_entry(id, user.id, &update).await {
        Ok(_entry) => match state.tracking.get_entry_with_media(user.id, id).await {
            Ok(Some(ewm)) => {
                // An increment/decrement comes from the drawer's progress row
                // and must return the refreshed row so the +/− buttons can be
                // added *and* removed symmetrically. Every other caller (the
                // tracking card and the drawer status chips) still wants the
                // full card partial; the drawer chips discard it (hx-swap
                // none), the card swaps it in.
                let html = if form.increment.is_some() {
                    ProgressRow::from_entry(&ewm).render()
                } else {
                    TrackingCardPartial {
                        entry_with_media: ewm,
                    }
                    .render()
                }
                .unwrap_or_else(|e| {
                    tracing::error!(error = %e, "template render failed");
                    String::from("Internal Server Error")
                });
                ([("HX-Trigger", "trackingUpdated")], Html(html)).into_response()
            }
            Ok(None) => Redirect::to("/tracking").into_response(),
            Err(e) => {
                tracing::error!(error = %e, "failed to load updated tracking entry");
                Redirect::to("/tracking").into_response()
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "failed to update tracking");
            Redirect::to("/tracking").into_response()
        }
    }
}

/// `GET /tracking/{id}/progress-row` — re-render just the drawer's progress
/// row. Used after an episode/chapter checkbox toggle, where the server
/// pushes `progressUpdated` and the client asks for the authoritative row
/// (so `+1`/`−` reappear or disappear accordingly). Scoped to the owner.
pub async fn get_progress_row(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.tracking.get_entry_with_media(user.id, id).await {
        Ok(Some(ewm)) => Html(ProgressRow::from_entry(&ewm).render().unwrap_or_else(|e| {
            tracing::error!(error = %e, "template render failed");
            String::from("Internal Server Error")
        }))
        .into_response(),
        Ok(None) => (axum::http::StatusCode::NOT_FOUND, "Not found").into_response(),
        Err(e) => super::internal_error("tracking: progress row", e),
    }
}

pub async fn htmx_delete_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.tracking.delete_entry(id, user.id).await {
        Ok(_) => ([("HX-Trigger", "trackingUpdated")], "").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete tracking");
            Redirect::to("/tracking").into_response()
        }
    }
}

pub async fn htmx_tracking_partial(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<TrackingQuery>,
    headers: HeaderMap,
) -> Response {
    if headers.get("hx-request").is_none() {
        let mut url = "/tracking".to_string();
        let mut query_parts = Vec::new();
        if let Some(ref status) = params.status
            && !status.is_empty()
        {
            query_parts.push(format!("status={}", status));
        }
        if let Some(ref media_type) = params.media_type
            && !media_type.is_empty()
        {
            query_parts.push(format!("type={}", media_type));
        }
        if let Some(ref q) = params.q
            && !q.is_empty()
        {
            query_parts.push(format!("q={}", q));
        }
        if !query_parts.is_empty() {
            url.push('?');
            url.push_str(&query_parts.join("&"));
        }
        return Redirect::to(&url).into_response();
    }

    let status = params.status.as_deref();
    let media_type = params.media_type.as_deref();
    let search_query = params.q.as_deref();
    let entries = match state
        .tracking
        .get_user_entries(user.id, status, media_type, search_query)
        .await
    {
        Ok(entries) => entries,
        Err(e) => return super::internal_error("tracking: list entries partial", e),
    };

    Html(
        TrackingGridPartial {
            entries,
            current_status: params.status.unwrap_or_default(),
            current_media_type: params.media_type.unwrap_or_default(),
            search_query: params.q.unwrap_or_default(),
        }
        .render()
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "template render failed");
            String::from("Internal Server Error")
        }),
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_media_types_have_fourteen_entries() {
        assert_eq!(get_all_media_types().len(), 14);
        assert_eq!(manual_media_types().len(), 14);
        assert!(manual_media_types().contains(&"comic"));
        assert!(manual_media_types().contains(&"animated-movies"));
    }

    #[test]
    fn canonical_items_match_manual_types() {
        let keys: Vec<&str> = get_all_media_types()
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        assert_eq!(keys, manual_media_types());
    }

    #[test]
    fn chapters_metric_has_comic_label() {
        let chapters = MANUAL_METRIC_FIELDS
            .iter()
            .find(|f| f.name == "chapters")
            .expect("chapters field");
        assert_eq!(chapters.label, "Главы");
        assert_eq!(chapters.label_comic, Some("Выпуски"));
    }
}
