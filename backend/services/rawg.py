import httpx
import os

RAWG_BASE = "https://api.rawg.io/api"
RAWG_KEY = os.getenv("RAWG_API_KEY")

async def search_games(query: str) -> list[dict]:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{RAWG_BASE}/games",
            params={"key": RAWG_KEY, "search": query, "page_size": 10},
        )
    data = response.json()
    return [format_game(item) for item in data.get("results", [])]

async def get_game_by_id(rawg_id: str) -> dict | None:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{RAWG_BASE}/games/{rawg_id}",
            params={"key": RAWG_KEY},
        )
    if response.status_code != 200:
        return None
    return format_game(response.json())

def format_game(item: dict) -> dict:
    return {
        "rawg_id": item["id"],
        "external_id": str(item["id"]),
        "title": item.get("name", ""),
        "title_russian": None,
        "title_english": item.get("name"),
        "title_native": None,
        "poster_url": item.get("background_image"),
        "media_type": "games",
        "episodes": None,
        "status": "FINISHED" if item.get("released") else "UNKNOWN",
        "description": item.get("description_raw"),
        "score": int(item["rating"] * 20) if item.get("rating") else None,
    }