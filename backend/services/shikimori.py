import asyncio
import httpx

SHIKI_BASE = "https://shikimori.one/api"
TIMEOUT = 20.0
HEADERS = {
    "User-Agent": "MediaTracker/1.0 (contact: your@email.com)"
}


def _poster_url(image_meta: dict | None) -> str | None:
    if not image_meta:
        return None
    original = image_meta.get("original")
    if not original:
        return None
    if original.startswith("http"):
        return original
    return f"https://shikimori.one{original}"


def _media_type_from_kind(kind: str | None, is_manga: bool = False) -> str:
    if not kind:
        return "manga" if is_manga else "anime"
    mapping = {
        "tv": "anime",
        "movie": "anime",
        "ova": "anime",
        "ona": "anime",
        "special": "anime",
        "music": "anime",
        "tv_special": "anime",
        "manga": "manga",
        "manhwa": "manhwa",
        "manhua": "manhua",
        "light_novel": "novels",
        "novel": "novels",
        "one_shot": "manga",
        "doujin": "manga",
    }
    return mapping.get(kind, "manga" if is_manga else "anime")


def _score_to_int(score: str | float | None) -> int | None:
    if score is None:
        return None
    try:
        return int(round(float(score) * 10))
    except (ValueError, TypeError):
        return None


def format_shiki_item(item: dict, is_manga: bool = False) -> dict:
    title = item.get("name") or ""
    russian = item.get("russian")
    english = item.get("english")
    japanese = item.get("japanese") or item.get("japanese")

    return {
        "external_id": str(item["id"]),
        "provider": "shikimori",
        "title": russian or english or title,
        "title_english": english or title,
        "title_native": japanese,
        "title_russian": russian,
        "poster_url": _poster_url(item.get("image")),
        "media_type": _media_type_from_kind(item.get("kind"), is_manga),
        "episodes": item.get("episodes") or item.get("chapters"),
        "status": item.get("status"),
        "description": item.get("description"),
        "score": _score_to_int(item.get("score")),
    }


async def search_anime(query: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=TIMEOUT, headers=HEADERS) as client:
        response = await client.get(
            f"{SHIKI_BASE}/animes",
            params={"search": query, "limit": 30},
        )
        response.raise_for_status()
    items = response.json()
    return [format_shiki_item(item, is_manga=False) for item in items]


async def search_manga(query: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=TIMEOUT, headers=HEADERS) as client:
        response = await client.get(
            f"{SHIKI_BASE}/mangas",
            params={"search": query, "limit": 30},
        )
        response.raise_for_status()
    items = response.json()
    return [format_shiki_item(item, is_manga=True) for item in items]


async def get_anime_by_id(shiki_id: int) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT, headers=HEADERS) as client:
        response = await client.get(f"{SHIKI_BASE}/animes/{shiki_id}")
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return format_shiki_item(response.json(), is_manga=False)


async def get_manga_by_id(shiki_id: int) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT, headers=HEADERS) as client:
        response = await client.get(f"{SHIKI_BASE}/mangas/{shiki_id}")
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return format_shiki_item(response.json(), is_manga=True)