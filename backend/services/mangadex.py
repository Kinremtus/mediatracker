import httpx
from typing import Optional, List, Dict

# MangaDex API base URL (v5)
MD_BASE = "https://api.mangadex.org"
TIMEOUT = 20.0
SEARCH_LIMIT = 30

# Helper to build cover image URL when "cover_art" relationship is included.
def _cover_url(item: dict) -> Optional[str]:
    manga_id = item.get("id")
    if not manga_id:
        return None
    for rel in item.get("relationships", []):
        if rel.get("type") == "cover_art":
            file_name = rel.get("attributes", {}).get("fileName")
            if file_name:
                return f"https://uploads.mangadex.org/covers/{manga_id}/{file_name}"
    return None


def _format_md_item(item: dict, media_type: str) -> dict:
    # ``attributes.title`` contains language‑specific titles.
    titles = item.get("attributes", {}).get("title", {})
    # Prefer English, then Japanese, then any available.
    title = titles.get("en") or titles.get("en_us") or titles.get("ja") or next(iter(titles.values()), "")
    return {
        "external_id": item.get("id"),
        "provider": "mangadex",
        "title": title,
        "title_english": titles.get("en") or titles.get("en_us"),
        "title_native": titles.get("ja"),
        "title_russian": None,
        "poster_url": _cover_url(item),
        "media_type": media_type,
        "episodes": item.get("attributes", {}).get("chapterCount"),
        "status": item.get("attributes", {}).get("status"),
        "description": (item.get("attributes", {}).get("description") or {}).get("en"),
        "score": None,
    }


async def _request(path: str, params: Optional[dict] = None) -> dict:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        resp = await client.get(f"{MD_BASE}{path}", params=params or {})
        resp.raise_for_status()
        return resp.json()


async def search_manga(query: str, original_language: Optional[str] = None) -> List[dict]:
    """Search manga (default Japanese). ``original_language`` can be ``"ja"``, ``"ko"`` or ``"zh"``.
    Returns a list of dicts compatible with ``schemas.SearchResult``.
    """
    params: Dict[str, any] = {
        "title": query,
        "limit": SEARCH_LIMIT,
        "includes[]": "cover_art",
        "order[relevance]": "desc",
    }
    if original_language:
        params["originalLanguage[]"] = original_language
    data = await _request("/manga", params)
    results = data.get("data", [])
    # Determine media_type based on language if not supplied explicitly.
    media_type = (
        "manhwa" if original_language == "ko"
        else "manhua" if original_language == "zh"
        else "manga"
    )
    return [_format_md_item(item, media_type) for item in results]


async def get_manga_by_id(manga_id: str) -> Optional[dict]:
    """Retrieve a single manga by its MangaDex UUID.
    Returns ``None`` if not found.
    """
    try:
        data = await _request(f"/manga/{manga_id}", {"includes[]": "cover_art"})
    except httpx.HTTPStatusError as exc:
        if exc.response.status_code == 404:
            return None
        raise
    item = data.get("data")
    if not item:
        return None
    # MangaDex does not provide explicit demographic; default to "manga".
    return _format_md_item(item, "manga")
