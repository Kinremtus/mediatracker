use askama::Template;
use axum::{
    extract::{Form, Path, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::services::external::tmdb::TmdbSeasonInfo;

#[derive(Template)]
#[template(path = "partials/_tmdb_seasons.html")]
struct TmdbSeasonsTemplate {
    seasons: Vec<TmdbSeasonInfo>,
    external_id: String,
    active_season: i32,
    episodes_html: String,
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

pub async fn get_tmdb_seasons(
    State(state): State<AppState>,
    Path(external_id): Path<String>,
) -> impl IntoResponse {
    let seasons = match state.tmdb.fetch_seasons(&external_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(external_id, error = %e, "failed to fetch TMDB seasons");
            return Html(String::new());
        }
    };

    let valid: Vec<&TmdbSeasonInfo> = seasons
        .iter()
        .filter(|s| s.episode_count > 0)
        .collect();

    let first = valid.first().copied();
    let active_season = first.map(|s| s.season_number).unwrap_or(1);

    let episodes_html = match first {
        Some(s) => render_episodes(&state, &external_id, s.season_number).await,
        None => String::new(),
    };

    let html = TmdbSeasonsTemplate {
        seasons,
        external_id,
        active_season,
        episodes_html,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "tmdb seasons render failed");
        String::new()
    });
    Html(html)
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

    let media_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM media_items WHERE provider = 'tmdb' AND external_id = $1",
    )
    .bind(&external_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

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

    let html = {
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
    let mut resp = Html(html).into_response();
    resp.headers_mut().insert(
        "HX-Trigger",
        trigger.to_string().parse().unwrap(),
    );
    resp
}
