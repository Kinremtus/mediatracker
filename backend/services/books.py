import httpx

BOOKS_BASE = "https://www.googleapis.com/books/v1"
TIMEOUT = 20.0
SEARCH_LIMIT = 30


def _normalize_thumbnail(url: str | None) -> str | None:
    if not url:
        return None
    return url.replace("http://", "https://")


def format_book(item: dict) -> dict:
    volume = item.get("volumeInfo") or {}
    image_links = volume.get("imageLinks") or {}

    return {
        "external_id": str(item["id"]),
        "title": volume.get("title", ""),
        "title_english": volume.get("title"),
        "title_native": None,
        "title_russian": None,
        "poster_url": _normalize_thumbnail(image_links.get("thumbnail")),
        "media_type": "books",
        "episodes": volume.get("pageCount"),
        "status": None,
        "description": volume.get("description"),
        "score": None,
    }


async def search_books(query: str) -> list[dict]:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(
            f"{BOOKS_BASE}/volumes",
            params={"q": query, "maxResults": SEARCH_LIMIT},
        )
        response.raise_for_status()

    data = response.json()
    items = data.get("items", [])

    return [format_book(item) for item in items]


async def get_book_by_id(book_id: str) -> dict | None:
    async with httpx.AsyncClient(timeout=TIMEOUT) as client:
        response = await client.get(f"{BOOKS_BASE}/volumes/{book_id}")

    if response.status_code == 404:
        return None

    response.raise_for_status()

    return format_book(response.json())