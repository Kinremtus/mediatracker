//! Shared DTO for official-source search hits (kuaikan, kakao, mangaplus,
//! comicwalker, mangaup, alphapolis, …).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficialHit {
    pub id: String,
    pub title: String,
}
