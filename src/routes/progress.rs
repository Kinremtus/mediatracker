//! Shared view model for the drawer's "Мой прогресс" row.
//!
//! The media drawer (`media.rs`) and the HTMX tracking endpoints
//! (`tracking.rs`) both render — and therefore both have to *compute* — the
//! same `[−]  N / M unit  [+1]` line. Keeping that arithmetic in one place is
//! what guarantees the `+`/`−` buttons stay in sync with the value after an
//! increment or decrement.
//!
//! Before this module existed the drawer initially rendered the buttons on the
//! server, but `afterProgressChange()` in `app.js` only knew how to *remove*
//! buttons client-side. So decrementing to zero removed `−` permanently and
//! stepping back up never recreated `+1` (and symmetrically for the other
//! button) until the drawer was closed and reopened. Rendering the row from
//! the server on every change removes that asymmetry entirely.

use askama::Template;
use uuid::Uuid;

use crate::models::tracking_entry::TrackingEntryWithMedia;

/// Media types whose progress is counted in discrete steps (episodes,
/// chapters, pages, hours). Mirrors the list previously inlined in
/// `media.rs`; only these get the `−`/`+1` controls at all.
const PROGRESS_MEDIA_TYPES: &[&str] = &[
    "anime",
    "series",
    "cartoons",
    "animated-movies",
    "manga",
    "manhwa",
    "manhua",
    "novel",
    "other-comics",
    "book",
    "game",
];

/// `true` when the media type has a meaningful step-by-step progress.
fn supports_progress(media_type: &str) -> bool {
    PROGRESS_MEDIA_TYPES.contains(&media_type)
}

/// Everything `templates/partials/_progress_row.html` needs to render.
///
/// The same struct doubles as the Askama template context, so the drawer
/// (via `{% include %}`) and the HTMX endpoint render byte-identical rows.
#[derive(Debug, Clone, PartialEq, Eq, Template)]
#[template(path = "partials/_progress_row.html")]
pub struct ProgressRow {
    /// Tracking entry id — baked into the `hx-post` URLs.
    pub tid: Uuid,
    /// Current progress, clamped to `0` when the entry has no value yet.
    pub progress_display: i32,
    /// `" / 12"` or `""` when the total is unknown.
    pub total_display: String,
    /// Localised unit, e.g. `"эп."`, `"гл."`, `"стр."`.
    pub progress_unit: String,
    /// Show `+1` only while below the known total.
    pub can_increment: bool,
    /// Show `−` only above zero.
    pub can_decrement: bool,
    /// Status sent along with the increment so the update never clobbers it.
    pub status_display: String,
    /// Whether this media type has a progress row at all.
    pub has_progress: bool,
}

impl ProgressRow {
    /// Build the row from raw media/progress values.
    pub fn compute(
        tid: Uuid,
        media_type: &str,
        total_count: Option<i32>,
        progress_unit: impl Into<String>,
        progress: Option<i32>,
        status_display: impl Into<String>,
    ) -> Self {
        let progress_display = progress.unwrap_or(0);
        let total_display = match total_count {
            Some(tc) => format!(" / {tc}"),
            None => String::new(),
        };
        let has_progress = supports_progress(media_type);
        let can_increment = has_progress && progress_display < total_count.unwrap_or(i32::MAX);
        Self {
            tid,
            progress_display,
            total_display,
            progress_unit: progress_unit.into(),
            can_increment,
            can_decrement: progress_display > 0,
            status_display: status_display.into(),
            has_progress,
        }
    }

    /// Build the row from a tracking entry joined with its media row.
    pub fn from_entry(entry: &TrackingEntryWithMedia) -> Self {
        Self::compute(
            entry.entry.id,
            &entry.media.media_type,
            entry.media.total_count(),
            entry.media.progress_unit_ru(),
            Some(entry.entry.progress),
            entry.entry.status.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(media_type: &str, total: Option<i32>, progress: i32) -> ProgressRow {
        ProgressRow::compute(
            Uuid::nil(),
            media_type,
            total,
            "эп.",
            Some(progress),
            "in_progress",
        )
    }

    #[test]
    fn progress_at_zero_hides_decrement() {
        let r = row("anime", Some(12), 0);
        assert_eq!(r.progress_display, 0);
        assert!(!r.can_decrement, "− must be hidden at zero");
        assert!(r.can_increment, "+1 must be available below the total");
    }

    #[test]
    fn progress_at_total_disables_increment() {
        let r = row("anime", Some(12), 12);
        assert!(!r.can_increment, "+1 must disappear at the total");
        assert!(r.can_decrement, "− must be available above zero");
    }

    #[test]
    fn progress_below_total_enables_increment() {
        let r = row("anime", Some(12), 5);
        assert!(r.can_increment);
        assert!(r.can_decrement);
    }

    #[test]
    fn unknown_total_allows_increment_without_total_suffix() {
        let r = row("game", None, 3);
        assert!(r.can_increment, "no total means no upper bound");
        assert_eq!(r.total_display, "");
        assert_eq!(r.progress_display, 3);
    }

    #[test]
    fn non_progress_media_disables_increment() {
        let r = row("movie", Some(120), 0);
        assert!(!r.has_progress);
        assert!(!r.can_increment, "movies have no step progress");
    }

    #[test]
    fn progress_none_reads_as_zero() {
        let r = ProgressRow::compute(Uuid::nil(), "manga", Some(40), "гл.", None, "in_progress");
        assert_eq!(r.progress_display, 0);
        assert_eq!(r.total_display, " / 40");
        assert_eq!(r.progress_unit, "гл.");
    }

    #[test]
    fn supports_progress_matches_drawer_media_types() {
        for t in [
            "anime",
            "series",
            "cartoons",
            "animated-movies",
            "manga",
            "manhwa",
            "manhua",
            "novel",
            "other-comics",
            "book",
            "game",
        ] {
            assert!(supports_progress(t), "{t} should support progress");
        }
        for t in ["movie", "dramas", ""] {
            assert!(!supports_progress(t), "{t} should not support progress");
        }
    }
}
