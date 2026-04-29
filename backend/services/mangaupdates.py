import httpx

MU_BASE = "https://api.mangaupdates.com/v1"
TIMEOUT = 20.0


def format_mu_item(item: dict) -> dict:
    # The API wraps the series data in a 'record' field during search
    record = item.get("record", item)
    
    # Map MU type to our media_type
    mu_type = (record.get("type") or "Manga").lower()
    if "novel" in mu_type:
        internal_type = "novels"
    elif "manhwa" in mu_type:
        internal_type = "manhwa"
    elif "manhua" in mu_type:
        internal_type = "manhua"
    else:
        internal_type = "manga"

    return {
        "external_id": str(record.get("series_id") or record.get("id")),
        "provider": "mangaupdates",
        "title": record.get("title") or "",
        "title_english": record.get("title"),
        "title_native": None,
        "title_russian": None,
        "poster_url": record.get("image", {}).get("url", {}).get("original") if isinstance(record.get("image"), dict) else None,
        "media_type": internal_type,
        "episodes": record.get("latest_chapter") or record.get("chapters"),
        "status": record.get("status") or record.get("series_status", {}).get("status"),
        "description": record.get("description"),
        "score": round(record.get("bayesian_rating")) if record.get("bayesian_rating") is not None else None,
    }


async def search_series(query: str, allowed_types: list[str] = None) -> list[dict]:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(
            f"{MU_BASE}/series/search",
            json={"search": query, "page": 1},
        )
        response.raise_for_status()

    data = response.json()
    results = data.get("results", [])

    filtered = []
    for r in results:
        record = r.get("record", r)
        series_type = (record.get("type") or "").lower()
        
        if allowed_types:
            # Мягкая фильтрация на бэкенде (case-insensitive in)
            match = False
            for t in allowed_types:
                if t.lower() in series_type:
                    match = True
                    break
            if not match:
                continue
        
        filtered.append(r)

    return [format_mu_item(r) for r in filtered]


async def get_series_by_id(series_id: str) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(f"{MU_BASE}/series/{series_id}")

    if response.status_code == 404:
        return None

    response.raise_for_status()
    return format_mu_item(response.json())