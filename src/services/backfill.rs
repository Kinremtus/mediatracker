//! Pure helpers for the `backfill_anime` CLI binary.
//!
//! The binary lives in `src/bin/backfill_anime.rs`; the strategy
//! decision and Shikimori response parsing live here so they're
//! testable without a DB or a network.
//!
//! Three resolution strategies for `mal_id`, in order of preference:
//!
//! 1. **`MalIdFromExternal`** — `provider = "mal"` and the
//!    `external_id` is all digits, so it IS the MAL id. No HTTP.
//! 2. **`ShikimoriLookup`** — `shikimori_id` is known, so we can
//!    GET `https://shikimori.one/api/animes/{id}` and read the
//!    `mal_id` field from the response.
//! 3. **`JikanSearchByTitle`** — neither of the above works, so we
//!    fall back to Jikan search by title and take the first hit.
//!    Fuzzy match can be wrong for ambiguous names, so this is
//!    logged loudly and only used as a last resort.
//!
//! `Unresolvable` is returned when we have nothing to go on (empty
//! title, non-numeric external_id for a MAL row, etc.).

use serde::Deserialize;
use uuid::Uuid;

/// Minimal row we need to decide what to do with one anime entry.
#[derive(Debug, Clone)]
pub struct BackfillCandidate {
    pub id: Uuid,
    pub provider: String,
    pub external_id: String,
    pub shikimori_id: Option<i64>,
    pub title: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Strategy {
    MalIdFromExternal,
    ShikimoriLookup,
    JikanSearchByTitle,
    Unresolvable,
}

pub fn choose_strategy(c: &BackfillCandidate) -> Strategy {
    if c.provider == "mal" && c.external_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Strategy::MalIdFromExternal;
    }
    if c.shikimori_id.is_some() {
        return Strategy::ShikimoriLookup;
    }
    if !c.title.trim().is_empty() {
        return Strategy::JikanSearchByTitle;
    }
    Strategy::Unresolvable
}

/// `Strategy::MalIdFromExternal` implementation: parse the external_id
/// as i64. Only succeeds for the MAL provider.
pub fn try_external_id_as_mal(c: &BackfillCandidate) -> Option<i64> {
    if c.provider != "mal" {
        return None;
    }
    c.external_id.parse().ok()
}

/// Shikimori `/api/animes/{id}` response — we only need the `mal_id`
/// field. Shikimori sometimes returns `mal_id: null` for entries
/// that don't have a MAL counterpart, and old entries may not have
/// the field at all; both must come out as `None`.
#[derive(Debug, Deserialize)]
struct ShikimoriAnimeEnvelope {
    mal_id: Option<i64>,
}

/// Parse a Shikimori `/api/animes/{id}` response body and return
/// the `mal_id` field if present and non-null. Returns `None` for
/// missing field, explicit `null`, or malformed JSON. Pure, no IO.
pub fn mal_id_from_shikimori_anime_response(json: &str) -> Option<i64> {
    serde_json::from_str::<ShikimoriAnimeEnvelope>(json)
        .ok()
        .and_then(|e| e.mal_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(provider: &str, ext: &str, shiki: Option<i64>, title: &str) -> BackfillCandidate {
        BackfillCandidate {
            id: Uuid::nil(),
            provider: provider.to_string(),
            external_id: ext.to_string(),
            shikimori_id: shiki,
            title: title.to_string(),
        }
    }

    #[test]
    fn strategy_mal_numeric_uses_external_id() {
        assert_eq!(
            choose_strategy(&cand("mal", "21", None, "One Piece")),
            Strategy::MalIdFromExternal
        );
    }

    #[test]
    fn strategy_shikimori_with_known_id() {
        assert_eq!(
            choose_strategy(&cand("shikimori", "1", Some(1), "Cowboy Bebop")),
            Strategy::ShikimoriLookup
        );
    }

    #[test]
    fn strategy_shikimori_without_id_falls_back_to_title() {
        assert_eq!(
            choose_strategy(&cand("shikimori", "1", None, "Cowboy Bebop")),
            Strategy::JikanSearchByTitle
        );
    }

    #[test]
    fn strategy_mal_with_nonnumeric_external_id_falls_back_to_title() {
        // Some legacy import might have provider=mal but external_id is
        // a slug. We can't parse it; fall back to title search.
        assert_eq!(
            choose_strategy(&cand("mal", "abc", None, "Berserk")),
            Strategy::JikanSearchByTitle
        );
    }

    #[test]
    fn strategy_empty_title_is_unresolvable() {
        assert_eq!(
            choose_strategy(&cand("shikimori", "1", None, "")),
            Strategy::Unresolvable
        );
        assert_eq!(
            choose_strategy(&cand("mal", "abc", None, "  ")),
            Strategy::Unresolvable
        );
    }

    #[test]
    fn try_external_id_only_for_mal() {
        assert_eq!(try_external_id_as_mal(&cand("mal", "21", None, "")), Some(21));
        assert_eq!(try_external_id_as_mal(&cand("mal", "abc", None, "")), None);
        assert_eq!(try_external_id_as_mal(&cand("shikimori", "21", None, "")), None);
        assert_eq!(try_external_id_as_mal(&cand("mal", "", None, "")), None);
    }

    #[test]
    fn shikimori_response_with_mal_id() {
        let json = r#"{"id":1,"mal_id":21,"name":"One Piece"}"#;
        assert_eq!(mal_id_from_shikimori_anime_response(json), Some(21));
    }

    #[test]
    fn shikimori_response_with_null_mal_id() {
        let json = r#"{"id":1,"mal_id":null,"name":"Custom"}"#;
        assert_eq!(mal_id_from_shikimori_anime_response(json), None);
    }

    #[test]
    fn shikimori_response_without_mal_id_field() {
        let json = r#"{"id":1,"name":"Old Entry"}"#;
        assert_eq!(mal_id_from_shikimori_anime_response(json), None);
    }

    #[test]
    fn shikimori_malformed_json() {
        assert_eq!(mal_id_from_shikimori_anime_response("not json"), None);
        assert_eq!(mal_id_from_shikimori_anime_response(""), None);
    }
}
