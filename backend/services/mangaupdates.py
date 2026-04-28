import httpx

MU_BASE = "https://api.mangaupdates.com/v1"
TIMEOUT = 20.0


def format_mu_item(item: dict) -> dict:
    # The API wraps the series data in a 'record' field during search
    record = item.get("record", item)
    
    return {
        "external_id": str(record.get("series_id") or record.get("id")),
        "provider": "mangaupdates",
        "title": record.get("title") or "",
        "title_english": record.get("title"),
        "title_native": None,
        "title_russian": None,
        "poster_url": record.get("image", {}).get("url", {}).get("original") if isinstance(record.get("image"), dict) else None,
        "media_type": "novels",
        "episodes": record.get("latest_chapter") or record.get("chapters"),
        "status": record.get("status") or record.get("series_status", {}).get("status"),
        "description": record.get("description"),
        "score": round(record.get("bayesian_rating")) if record.get("bayesian_rating") is not None else None,
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

    # Мягкая фильтрация: оставляем только то, что явно является новеллой
    filtered = []
    for r in results:
        record = r.get("record", r)
        series_type = (record.get("type") or "").lower()
        if "novel" in series_type:
            filtered.append(r)

    return [format_mu_item(r) for r in filtered]


async def get_series_by_id(series_id: str) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(f"{MU_BASE}/series/{series_id}")

    if response.status_code == 404:
        return None

    response.raise_for_status()
    return format_mu_item(response.json())