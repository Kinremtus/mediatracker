import httpx

MU_BASE = "https://api.mangaupdates.com/v1"
TIMEOUT = 20.0


def format_mu_item(item: dict) -> dict:
    return {
        "external_id": str(item.get("series_id") or item.get("id")),
        "provider": "mangaupdates",
        "title": item.get("title") or "",
        "title_english": item.get("title"),
        "title_native": None,
        "title_russian": None,
        "poster_url": item.get("image", {}).get("url") if isinstance(item.get("image"), dict) else None,
        "media_type": "novels" if "novel" in (item.get("type") or "").lower() else "manga",
        "episodes": item.get("latest_chapter") or item.get("chapters"),
        "status": item.get("status") or item.get("series_status", {}).get("status"),
        "description": item.get("description"),
        "score": None,
    }


async def search_series(query: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(
            f"{MU_BASE}/series/search",
            json={"search": query, "page": 1},
        )
        response.raise_for_status()

    data = response.json()
    results = data.get("results", [])

    return [format_mu_item(r) for r in results]


async def get_series_by_id(series_id: str) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(f"{MU_BASE}/series/{series_id}")

    if response.status_code == 404:
        return None

    response.raise_for_status()
    return format_mu_item(response.json())