import httpx
import os

TMDB_BASE = "https://api.themoviedb.org/3"
TMDB_KEY = os.getenv("TMDB_API_KEY")
TMDB_IMG = "/tmdb-image"


async def search_movies(query: str, genre_id: int = None) -> list[dict]:
    params = {
        "api_key": TMDB_KEY,
        "query": query,
        "language": "ru-RU",
        "include_adult": True,
    }
    if genre_id:
        params["with_genres"] = genre_id
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{TMDB_BASE}/search/movie", params=params)
    data = response.json()
    return [format_movie(item) for item in data.get("results", [])[:10]]

async def search_tv(query: str, genre_id: int = None) -> list[dict]:
    params = {
        "api_key": TMDB_KEY,
        "query": query,
        "language": "ru-RU",
    }
    if genre_id:
        params["with_genres"] = genre_id
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{TMDB_BASE}/search/tv", params=params)
    data = response.json()
    return [format_tv(item) for item in data.get("results", [])[:10]]


def format_movie(item: dict) -> dict:
    return {
        "tmdb_id": item["id"],
        "title": item.get("title", ""),
        "title_russian": item.get("title"),
        "title_english": item.get("original_title"),
        "title_native": None,
        "poster_url": (
            f"{TMDB_IMG}{item['poster_path']}"
            if item.get("poster_path")
            else None
        ),
        "media_type": "movies",
        "episodes": None,
        "status": "FINISHED" if item.get("release_date") else "UNKNOWN",
        "score": int(item["vote_average"] * 10) if item.get("vote_average") else None,
    }


def format_tv(item: dict) -> dict:
    return {
        "tmdb_id": item["id"],
        "title": item.get("name", ""),
        "title_russian": item.get("name"),
        "title_english": item.get("original_name"),
        "title_native": None,
        "poster_url": (
            f"{TMDB_IMG}{item['poster_path']}"
            if item.get("poster_path")
            else None
        ),
        "media_type": "tv-shows",
        "episodes": item.get("number_of_episodes"),
        "status": "FINISHED" if item.get("last_air_date") else "ONGOING",
        "score": int(item["vote_average"] * 10) if item.get("vote_average") else None,
    }

async def get_by_id(tmdb_id: int, media_type: str) -> dict | None:
    # Нормализуем тип — принимаем и "movies" и "movie"
    endpoint = "movie" if media_type in ("movie", "movies") else "tv"
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{TMDB_BASE}/{endpoint}/{tmdb_id}",
            params={"api_key": TMDB_KEY, "language": "ru-RU"},
        )
    if response.status_code != 200:
        return None
    item = response.json()
    return format_movie(item) if endpoint == "movie" else format_tv(item)