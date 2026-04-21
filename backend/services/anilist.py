import re

import httpx

ANILIST_URL = "https://graphql.anilist.co"
TIMEOUT = 20.0
SEARCH_LIMIT = 30

ANIME_SEARCH_QUERY = """
query ($search: String!) {
  Page(perPage: 30) {
    media(search: $search, type: ANIME) {
      id
      title {
        romaji
        english
        native
      }
      coverImage {
        large
      }
      episodes
      status
      averageScore
      description(asHtml: false)
    }
  }
}
"""

ANIME_BY_ID_QUERY = """
query ($id: Int!) {
  Media(id: $id, type: ANIME) {
    id
    title {
      romaji
      english
      native
    }
    coverImage {
      large
    }
    episodes
    status
    averageScore
    description(asHtml: false)
  }
}
"""

MANGA_SEARCH_QUERY = """
query ($search: String!, $country: CountryCode, $format: MediaFormat) {
  Page(perPage: 30) {
    media(
      search: $search
      type: MANGA
      countryOfOrigin: $country
      format: $format
    ) {
      id
      title {
        romaji
        english
        native
      }
      coverImage {
        large
      }
      chapters
      status
      averageScore
      format
      description(asHtml: false)
    }
  }
}
"""

MANGA_BY_ID_QUERY = """
query ($id: Int!) {
  Media(id: $id, type: MANGA) {
    id
    title {
      romaji
      english
      native
    }
    coverImage {
      large
    }
    chapters
    status
    averageScore
    format
    description(asHtml: false)
  }
}
"""


def strip_html(text: str | None) -> str | None:
    if not text:
        return None
    return re.sub(r"<[^>]+>", "", text).strip()


async def _post_graphql(query: str, variables: dict) -> dict:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.post(
            ANILIST_URL,
            json={"query": query, "variables": variables},
        )
        response.raise_for_status()

    payload = response.json()

    if payload.get("errors"):
        message = "; ".join(
            error.get("message", "AniList error")
            for error in payload["errors"]
        )
        raise RuntimeError(message)

    return payload


def _resolve_manga_media_type(
    country: str | None = None,
    fmt: str | None = None,
    fallback: str = "manga",
) -> str:
    if fmt == "NOVEL":
        return "novels"
    if country == "KR":
        return "manhwa"
    if country == "CN":
        return "manhua"
    return fallback


def format_anilist_item(item: dict, media_type: str) -> dict:
    title = item.get("title") or {}

    return {
        "external_id": str(item["id"]),
        "title": title.get("romaji")
        or title.get("english")
        or title.get("native")
        or "",
        "title_english": title.get("english"),
        "title_native": title.get("native"),
        "title_russian": None,
        "poster_url": (item.get("coverImage") or {}).get("large"),
        "media_type": media_type,
        "episodes": item.get("episodes") or item.get("chapters"),
        "status": item.get("status"),
        "description": strip_html(item.get("description")),
        "score": item.get("averageScore"),
    }


async def search_anime(query: str) -> list[dict]:
    payload = await _post_graphql(
        ANIME_SEARCH_QUERY,
        {"search": query},
    )
    results = payload.get("data", {}).get("Page", {}).get("media", [])

    return [format_anilist_item(item, "anime") for item in results]


async def search_anime_by_id(anilist_id: int) -> dict | None:
    payload = await _post_graphql(
        ANIME_BY_ID_QUERY,
        {"id": anilist_id},
    )
    item = payload.get("data", {}).get("Media")

    if not item:
        return None

    return format_anilist_item(item, "anime")


async def search_manga(
    query: str,
    country: str | None = None,
    fmt: str | None = None,
) -> list[dict]:
    variables = {"search": query}

    if country:
        variables["country"] = country
    if fmt:
        variables["format"] = fmt

    payload = await _post_graphql(
        MANGA_SEARCH_QUERY,
        variables,
    )
    results = payload.get("data", {}).get("Page", {}).get("media", [])
    media_type = _resolve_manga_media_type(country=country, fmt=fmt)

    return [format_anilist_item(item, media_type) for item in results]


async def search_manga_by_id(
    anilist_id: int,
    media_type: str = "manga",
) -> dict | None:
    payload = await _post_graphql(
        MANGA_BY_ID_QUERY,
        {"id": anilist_id},
    )
    item = payload.get("data", {}).get("Media")

    if not item:
        return None

    if media_type == "manga" and item.get("format") == "NOVEL":
        media_type = "novels"

    return format_anilist_item(item, media_type)