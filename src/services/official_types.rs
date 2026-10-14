//! Shared DTO for official-source search hits (kuaikan, kakao, mangaplus,
//! comicwalker, mangaup, alphapolis, comici, naverwebtoon, wuxiaworld, …).
//!
//! A hit is the source's id plus its title; [`HitExtra`] carries the rich
//! fields a source's *search answer* exposes (poster, description, author,
//! status, genres, year, score, chapter count). Every source fills what it
//! has; the resolve walk ignores the extra and matches on the title alone.

/// Rich search-answer fields. All optional: a source that exposes nothing
/// beyond the title leaves them empty, and the catalog stores what exists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HitExtra {
    pub poster_url: Option<String>,
    pub description: Option<String>,
    /// Authors joined with `, ` when the source lists several.
    pub author: Option<String>,
    /// `completed` / `ongoing` / `hiatus` when the source states it.
    pub status: Option<String>,
    pub genres: Vec<String>,
    pub year: Option<i16>,
    pub score: Option<f64>,
    pub chapters: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OfficialHit {
    pub id: String,
    pub title: String,
    pub extra: HitExtra,
}

impl OfficialHit {
    /// A lean hit: id + title only. The resolve walk and the sources whose
    /// search answer exposes nothing else use this.
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            extra: HitExtra::default(),
        }
    }

    /// A hit with rich search-answer fields.
    pub fn with_extra(id: impl Into<String>, title: impl Into<String>, extra: HitExtra) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            extra,
        }
    }
}
