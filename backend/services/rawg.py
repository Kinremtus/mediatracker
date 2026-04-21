import os

import httpx

RAWG_BASE = "https://api.rawg.io/api"
RAWG_KEY = os.getenv("RAWG_API_KEY")
TIMEOUT = 20.0
SEARCH_LIMIT = 30


def format_game(item: dict) -> dict:
    rating = item.get("rating")

    return {
        "external_id": str(item["id"]),
        "title": item.get("name", ""),
        "title_english": item.get("name"),
        "title_native": None,
        "title_russian": None,
        "poster_url": item.get("background_image"),
        "media_type": "games",
        "episodes": None,
        "status": "FINISHED" if item.get("released") else "UNKNOWN",
        "description": item.get("description_raw"),
        "score": int(round(rating * 20)) if rating is not None else None,
    }


async def search_games(query: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(
            f"{RAWG_BASE}/games",
            params={
                "key": RAWG_KEY,
                "search": query,
                "page_size": SEARCH_LIMIT,
            },
        )
        response.raise_for_status()

    data = response.json()
    results = data.get("results", [])

    return [format_game(item) for item in results]


async def get_game_by_id(rawg_id: str) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(
            f"{RAWG_BASE}/games/{rawg_id}",
            params={"key": RAWG_KEY},
        )

    if response.status_code == 404:
        return None

    response.raise_for_status()

    return format_game(response.json())