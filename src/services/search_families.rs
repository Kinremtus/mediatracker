//! Media-family registry for the quick-search panel.
//!
//! Pure data: maps the 14 raw media types to 7 user-facing families and
//! provides `(icon, label)` metadata for a raw type. Single source of truth
//! shared by the `/api/search/panel` handler and its unit tests.

/// A user-facing family ("tab") of related raw media types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    pub key: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub types: &'static [&'static str],
}

/// Family key of the default "Все" tab (no filter).
pub const ALL_TAB: &str = "";

/// Label of the default "Все" tab.
pub fn all_label() -> &'static str {
    "Все"
}

/// The approved family order. Every raw media type appears in exactly one
/// family.
pub fn families() -> Vec<Family> {
    vec![
        Family {
            key: "anime",
            label: "Аниме",
            icon: "▶",
            types: &["anime"],
        },
        Family {
            key: "manga",
            label: "Манга",
            icon: "📚",
            types: &["manga", "manhwa", "manhua", "novel", "other-comics"],
        },
        Family {
            key: "movies",
            label: "Фильмы",
            icon: "🎥",
            types: &["movie", "animated-movies"],
        },
        Family {
            key: "series",
            label: "Сериалы",
            icon: "📺",
            types: &["series", "dramas", "cartoons"],
        },
        Family {
            key: "games",
            label: "Игры",
            icon: "🎮",
            types: &["game"],
        },
        Family {
            key: "books",
            label: "Книги",
            icon: "📖",
            types: &["book"],
        },
        Family {
            key: "comics",
            label: "Комиксы",
            icon: "🦸",
            types: &["comic"],
        },
    ]
}

/// Look up a family by its key.
pub fn family(key: &str) -> Option<Family> {
    families().into_iter().find(|f| f.key == key)
}

/// `(icon, label)` metadata for a raw media type, sourced from the canonical
/// 14-key registry. Unknown ids fall back to `("", media_type)`.
pub fn type_meta(media_type: &str) -> (String, String) {
    crate::routes::tracking::get_all_media_types()
        .into_iter()
        .find(|(k, _, _)| *k == media_type)
        .map(|(_, icon, label)| (icon.to_string(), label.to_string()))
        .unwrap_or_else(|| (String::new(), media_type.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn families_cover_all_fourteen_types() {
        let mut seen: Vec<&str> = Vec::new();
        for f in families() {
            for t in f.types {
                assert!(!seen.contains(t), "duplicate raw type {t}");
                seen.push(*t);
            }
        }
        let canonical: HashSet<&str> = crate::routes::tracking::get_all_media_types()
            .into_iter()
            .map(|(k, _, _)| k)
            .collect();
        let got: HashSet<&str> = seen.iter().copied().collect();
        assert_eq!(got, canonical);
        assert_eq!(seen.len(), 14);
        assert_eq!(families().len(), 7);
    }

    #[test]
    fn family_lookup_returns_modules() {
        let movies = family("movies").unwrap();
        assert!(movies.types.contains(&"movie"));
        assert!(movies.types.contains(&"animated-movies"));
        let manga = family("manga").unwrap();
        assert!(manga.types.contains(&"other-comics"));
        assert_eq!(family("comics").unwrap().types, &["comic"]);
    }

    #[test]
    fn type_meta_known_and_unknown() {
        assert_eq!(type_meta("anime"), ("▶".to_string(), "Аниме".to_string()));
        assert_eq!(type_meta("nope"), (String::new(), "nope".to_string()));
    }
}
