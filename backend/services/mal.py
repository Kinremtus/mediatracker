import os
import httpx
from typing import Optional

# MyAnimeList API base URL
MAL_BASE = "https://api.myanimelist.net/v2"

# Environment variables – you need to set these in .env on the server.
#   MAL_CLIENT_ID – Application client ID
#   MAL_CLIENT_SECRET – Application client secret
#   MAL_ACCESS_TOKEN – Optional pre‑generated bearer token (fallback to client‑credentials flow)
MAL_CLIENT_ID = os.getenv("MAL_CLIENT_ID")
MAL_CLIENT_SECRET = os.getenv("MAL_CLIENT_SECRET")
MAL_ACCESS_TOKEN = os.getenv("MAL_ACCESS_TOKEN")

TIMEOUT = 20.0
SEARCH_LIMIT = 30

# ---------------------------------------------------------------------------
# Core request wrapper – adds Client-ID header and parses JSON.
# ---------------------------------------------------------------------------
async def _request(path: str, params: Optional[dict] = None) -> dict:
    if not MAL_CLIENT_ID:
        raise RuntimeError("MAL_CLIENT_ID must be set in environment variables for MyAnimeList API access")
    
    headers = {"X-MAL-CLIENT-ID": MAL_CLIENT_ID}
    async with httpx.AsyncClient(timeout=TIMEOUT, headers=headers) as client:
        resp = await client.get(f"{MAL_BASE}{path}", params=params or {})
        resp.raise_for_status()
        return resp.json()

# ---------------------------------------------------------------------------
# Formatting – convert MAL response to the unified SearchResult schema.
# ---------------------------------------------------------------------------
def _format_mal_item(item: dict, media_type: str = "anime") -> dict:
    # ``title`` can be a dict with keys: "romaji", "english", "native", or just a string.
    title_data = item.get("title", "")
    if isinstance(title_data, dict):
        title = title_data.get("romaji") or title_data.get("english") or title_data.get("native") or ""
        title_english = title_data.get("english")
        title_native = title_data.get("native")
    else:
        title = title_data
        title_english = None
        title_native = None

    return {
        "external_id": str(item.get("id")),
        "provider": "mal",
        "title": title,
        "title_english": title_english,
        "title_native": title_native,
        "title_russian": None,
        "poster_url": (item.get("main_picture") or {}).get("large"),
        "media_type": media_type,
        "episodes": item.get("episodes"),
        "status": item.get("status"),
        "description": item.get("synopsis"),
        "score": int(round(item.get("mean", 0) * 10)) if item.get("mean") is not None else None,
    }

# ---------------------------------------------------------------------------
# Public API – search anime and fetch by ID.
# ---------------------------------------------------------------------------
async def search_anime(query: str) -> list[dict]:
    """Search anime by query string.

    Returns a list of dictionaries compatible with ``schemas.SearchResult``.
    """
    data = await _request("/anime", {"q": query, "limit": SEARCH_LIMIT})
    results = data.get("data", [])
    return [_format_mal_item(item, "anime") for item in results]


async def get_anime_by_id(anime_id: int) -> dict | None:
    """Retrieve a single anime by its MyAnimeList numeric ID.

    Returns ``None`` if the anime is not found.
    """
    try:
        # Request specific fields to ensure we get the title dictionary and other data
        fields = "id,title,main_picture,synopsis,mean,status,num_episodes"
        item = await _request(f"/anime/{anime_id}", {"fields": fields})
    except httpx.HTTPStatusError as exc:
        if exc.response.status_code == 404:
            return None
        raise
    return _format_mal_item(item, "anime")
