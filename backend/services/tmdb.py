import os

import httpx

TMDB_BASE = "https://api.themoviedb.org/3"
TMDB_KEY = os.getenv("TMDB_API_KEY")
TMDB_IMG = "/tmdb-image"
TIMEOUT = 20.0
SEARCH_LIMIT = 30
LANGUAGE = "ru-RU"

MOVIE_TYPES = ("movie", "movies", "animated-movies")
TV_TYPES = ("tv", "tv-shows", "dramas", "cartoons")


def _poster_url(poster_path: str | None) -> str | None:
    if not poster_path:
        return None
    return f"{TMDB_IMG}/{poster_path.lstrip('/')}"


def _matches_genre(item: dict, genre_id: int | None) -> bool:
    if genre_id is None:
        return True

    genre_ids = item.get("genre_ids") or []
    return genre_id in genre_ids


def _movie_media_type(genre_id: int | None) -> str:
    if genre_id == 16:
        return "animated-movies"
    return "movies"


def _tv_media_type(genre_id: int | None) -> str:
    if genre_id == 18:
        return "dramas"
    if genre_id == 16:
        return "cartoons"
    return "tv"


def format_tmdb_item(item: dict, media_type: str) -> dict:
    vote_average = item.get("vote_average")

    return {
        "external_id": str(item["id"]),
        "title": item.get("title") or item.get("name") or "",
        "title_english": item.get("original_title")
        or item.get("original_name"),
        "title_native": None,
        "title_russian": item.get("title") or item.get("name"),
        "poster_url": _poster_url(item.get("poster_path")),
        "media_type": media_type,
        "episodes": item.get("number_of_episodes"),
        "seasons": item.get("number_of_seasons"),
        "status": item.get("status"),
        "description": item.get("overview"),
        "score": (
            int(round(vote_average * 10))
            if vote_average is not None
            else None
        ),
    }


async def _search_tmdb(
    endpoint: str,
    query: str,
    include_adult: bool = False,
) -> list[dict]:
    results: list[dict] = []

    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        page = 1

        while len(results) < SEARCH_LIMIT and page <= 2:
            params = {
                "api_key": TMDB_KEY,
                "query": query,
                "language": LANGUAGE,
                "page": page,
            }

            if include_adult:
                params["include_adult"] = True

            response = await client.get(
                f"{TMDB_BASE}{endpoint}",
                params=params,
            )
            response.raise_for_status()

            data = response.json()
            results.extend(data.get("results", []))

            total_pages = data.get("total_pages", 1)
            if page >= total_pages:
                break

            page += 1

    return results[:SEARCH_LIMIT]


async def search_movies(
    query: str,
    genre_id: int | None = None,
) -> list[dict]:
    results = await _search_tmdb(
        "/search/movie",
        query,
        include_adult=True,
    )
    filtered = [item for item in results if _matches_genre(item, genre_id)]
    media_type = _movie_media_type(genre_id)

    return [
        format_tmdb_item(item, media_type) for item in filtered[:SEARCH_LIMIT]
    ]


async def search_tv(
    query: str,
    genre_id: int | None = None,
) -> list[dict]:
    results = await _search_tmdb("/search/tv", query)
    filtered = [item for item in results if _matches_genre(item, genre_id)]
    media_type = _tv_media_type(genre_id)

    return [
        format_tmdb_item(item, media_type) for item in filtered[:SEARCH_LIMIT]
    ]


async def get_by_id(tmdb_id: int, media_type: str) -> dict | None:
    endpoint = "movie" if media_type in MOVIE_TYPES else "tv"

    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(
            f"{TMDB_BASE}/{endpoint}/{tmdb_id}",
            params={"api_key": TMDB_KEY, "language": LANGUAGE},
        )

    if response.status_code == 404:
        return None

    response.raise_for_status()

    return format_tmdb_item(response.json(), media_type)