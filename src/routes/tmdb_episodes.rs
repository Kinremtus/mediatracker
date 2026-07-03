use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;

#[derive(Debug, Clone)]
pub struct TmdbSeasonRowData {
    pub season_number: i32,
    pub name: String,
    pub episode_count: i32,
    pub watched_count: i32,
    pub all_watched: bool,
}

#[derive(Template)]
#[template(path = "partials/_tmdb_seasons.html")]
struct TmdbSeasonsV2Template {
    rows: Vec<TmdbSeasonRowData>,
    external_id: String,
}

#[derive(Template)]
#[template(path = "partials/_tmdb_season_header.html")]
struct TmdbSeasonHeaderTemplate {
    row: TmdbSeasonRowData,
    external_id: String,
}

#[derive(Template)]
#[template(path = "partials/_tmdb_episode_list.html")]
struct TmdbEpisodeListTemplate {
    episodes: Vec<crate::services::tmdb_episodes::TmdbStoredEpisode>,
    external_id: String,
}

#[derive(Template)]
#[template(path = "partials/_tmdb_episode_item.html")]
struct TmdbEpisodeItemTemplate {
    episode: crate::services::tmdb_episodes::TmdbStoredEpisode,
    external_id: String,
}

#[derive(Deserialize)]
pub struct SetWatchedForm {
    #[serde(default)]
    pub watched: bool,
}

fn season_name(season_number: i32) -> String {
    if season_number == 0 {
        "Спецматериалы".to_string()
    } else {
        format!("Сезон {}", season_number)
    }
}

async fn get_media_id_by_external(
    pool: &sqlx::PgPool,
    external_id: &str,
) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM media_items WHERE provider = 'tmdb' AND external_id = $1",
    )
    .bind(external_id)
    .fetch_optional(pool)
    .await
    .unwrap_or(None)
}

async fn build_season_rows_from_tmdb(
    state: &AppState,
    external_id: &str,
) -> Vec<TmdbSeasonRowData> {
    let seasons = match state.tmdb.fetch_seasons(external_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(external_id, error = %e, "failed to fetch TMDB seasons");
            return vec![];
        }
    };

    let watched_counts: std::collections::HashMap<i32, i32> = crate::services::tmdb_episodes
        ::get_season_watched_counts(&state.db, external_id)
        .await
        .map(|v| v.into_iter().collect())
        .unwrap_or_default();

    let mut rows: Vec<TmdbSeasonRowData> = seasons
        .into_iter()
        .filter(|s| s.episode_count > 0)
        .map(|s| {
            let watched_count = watched_counts.get(&s.season_number).copied().unwrap_or(0);
            TmdbSeasonRowData {
                season_number: s.season_number,
                name: season_name(s.season_number),
                episode_count: s.episode_count,
                watched_count,
                all_watched: watched_count >= s.episode_count,
            }
        })
        .collect();

    // season 0 (specials) always at bottom
    rows.sort_by_key(|r| (r.season_number == 0, r.season_number));
    rows
}

async fn build_season_rows_from_db(
    state: &AppState,
    external_id: &str,
) -> Vec<TmdbSeasonRowData> {
    let groups = crate::services::tmdb_episodes::get_season_group_counts(
        &state.db,
        external_id,
    )
    .await
    .unwrap_or_default();

    let mut rows: Vec<TmdbSeasonRowData> = groups
        .into_iter()
        .filter(|(_, total, _)| *total > 0)
        .map(|(season_number, total, watched)| TmdbSeasonRowData {
            season_number,
            name: season_name(season_number),
            episode_count: total,
            watched_count: watched,
            all_watched: watched >= total,
        })
        .collect();

    rows.sort_by_key(|r| (r.season_number == 0, r.season_number));
    rows
}

pub async fn get_tmdb_seasons(
    State(state): State<AppState>,
    Path(external_id): Path<String>,
) -> impl IntoResponse {
    let mut rows = build_season_rows_from_tmdb(&state, &external_id).await;
    if rows.is_empty() {
        rows = build_season_rows_from_db(&state, &external_id).await;
    }

    let html = TmdbSeasonsV2Template { rows, external_id }
        .render()
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "tmdb seasons v2 render failed");
            String::new()
        });
    Html(html)
}

pub async fn post_tmdb_season_watched(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((external_id, season_number)): Path<(String, i32)>,
    Form(form): Form<SetWatchedForm>,
) -> impl IntoResponse {
    if let Err(e) = crate::services::tmdb_episodes::set_season_watched(
        &state.db,
        &external_id,
        season_number,
        form.watched,
    )
    .await
    {
        tracing::warn!(external_id, season_number, error = %e, "set_season_watched failed");
        return Html(String::new()).into_response();
    }

    let max_watched = crate::services::tmdb_episodes::get_total_watched_episodes(
        &state.db,
        &external_id,
    )
    .await
    .unwrap_or(0);

    if let Some(media_id) = get_media_id_by_external(&state.db, &external_id).await
        && let Err(e) = crate::services::tmdb_episodes::set_progress_direct(
            &state.db,
            user.id,
            media_id,
            max_watched,
        )
        .await
    {
        tracing::warn!(external_id, error = %e, "set_progress_direct failed");
    }

    let mut rows = build_season_rows_from_tmdb(&state, &external_id).await;
    if rows.is_empty() {
        rows = build_season_rows_from_db(&state, &external_id).await;
    }
    let html = TmdbSeasonsV2Template {
        rows,
        external_id: external_id.clone(),
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "tmdb seasons v2 render failed");
        String::new()
    });

    let mut trigger = serde_json::json!({
        "progressUpdated": {
            "maxWatched": max_watched,
        }
    });
    if let Some(media_id) = get_media_id_by_external(&state.db, &external_id).await {
        let id_str = serde_json::Value::String(media_id.to_string());
        trigger["progressUpdated"]["mediaId"] = id_str;
    }

    let mut resp = Html(html).into_response();
    resp.headers_mut().insert(
        "HX-Trigger",
        trigger.to_string().parse().unwrap(),
    );
    resp
}

async fn render_episodes(
    state: &AppState,
    external_id: &str,
    season_number: i32,
) -> String {
    let mut episodes = crate::services::tmdb_episodes::get_episodes(
        &state.db,
        external_id,
        season_number,
    )
    .await
    .unwrap_or_default();

    if episodes.is_empty()
        && let Ok(detail) = state
            .tmdb
            .fetch_season_episodes(external_id, season_number)
            .await
    {
        if let Err(e) = crate::services::tmdb_episodes::store_episodes(
            &state.db,
            external_id,
            season_number,
            &detail.episodes,
        )
        .await
        {
            tracing::warn!(external_id, season_number, error = %e, "store episodes failed");
        }

        episodes = crate::services::tmdb_episodes::get_episodes(
            &state.db,
            external_id,
            season_number,
        )
        .await
        .unwrap_or_default();
    }

    TmdbEpisodeListTemplate {
        episodes,
        external_id: external_id.to_string(),
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "episode list render failed");
        String::new()
    })
}

pub async fn get_tmdb_episodes(
    State(state): State<AppState>,
    Path((external_id, season_number)): Path<(String, i32)>,
) -> impl IntoResponse {
    let html = render_episodes(&state, &external_id, season_number).await;
    Html(html)
}

pub async fn set_tmdb_episode_watched(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((external_id, season_number, episode_number)): Path<(String, i32, i32)>,
    Form(form): Form<SetWatchedForm>,
) -> impl IntoResponse {
    if let Err(e) = crate::services::tmdb_episodes::set_watched(
        &state.db,
        &external_id,
        season_number,
        episode_number,
        form.watched,
    )
    .await
    {
        tracing::warn!(external_id, season_number, episode_number, error = %e, "set_watched failed");
        return Html(String::new()).into_response();
    }

    let max_watched = crate::services::tmdb_episodes::count_watched(
        &state.db,
        &external_id,
        season_number,
    )
    .await
    .unwrap_or(0);

    let media_id = get_media_id_by_external(&state.db, &external_id).await;

    if let Some(media_id) = media_id
        && let Err(e) = crate::services::tmdb_episodes::update_progress_from_watched(
            &state.db,
            user.id,
            media_id,
            max_watched,
        )
        .await
    {
        tracing::warn!(external_id, error = %e, "update_progress_from_watched failed");
    }

    let episode_html = {
        let mut episodes = crate::services::tmdb_episodes::get_episodes(
            &state.db,
            &external_id,
            season_number,
        )
        .await
        .unwrap_or_default();
        if let Some(ep) = episodes.iter_mut().find(|e| e.episode_number == episode_number) {
            TmdbEpisodeItemTemplate {
                episode: ep.clone(),
                external_id: external_id.clone(),
            }
            .render()
            .unwrap_or_default()
        } else {
            String::new()
        }
    };

    let (total_count, watched_count) = crate::services::tmdb_episodes::get_season_counts(
        &state.db,
        &external_id,
        season_number,
    )
    .await
    .unwrap_or((0, 0));

    let row = TmdbSeasonRowData {
        season_number,
        name: season_name(season_number),
        episode_count: total_count,
        watched_count,
        all_watched: total_count > 0 && watched_count >= total_count,
    };
    let header_html = TmdbSeasonHeaderTemplate {
        row,
        external_id: external_id.clone(),
    }
    .render()
    .unwrap_or_default();
    let oob_html = format!(
        r#"<div id="season-header-{}-{}" hx-swap-oob="true">{}</div>"#,
        external_id, season_number, header_html
    );

    let states = crate::services::tmdb_episodes::get_episode_states(
        &state.db,
        &external_id,
        season_number,
    )
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
            "seasonNumber": season_number,
        }
    });
    if let Some(media_id) = media_id {
        let id_str = serde_json::Value::String(media_id.to_string());
        trigger["progressUpdated"]["mediaId"] = id_str.clone();
        trigger["episodesChanged"]["mediaId"] = id_str;
    }
    let combined = format!("{}{}", episode_html, oob_html);
    let mut resp = Html(combined).into_response();
    resp.headers_mut().insert(
        "HX-Trigger",
        trigger.to_string().parse().unwrap(),
    );
    resp
}
