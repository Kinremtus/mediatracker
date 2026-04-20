import httpx

BOOKS_BASE = "https://www.googleapis.com/books/v1"

async def search_books(query: str) -> list[dict]:
    async with httpx.AsyncClient() as client:
        response = await client.get(
            f"{BOOKS_BASE}/volumes",
            params={"q": query, "maxResults": 10},
        )
    data = response.json()
    return [format_book(item) for item in data.get("items", [])]

async def get_book_by_id(book_id: str) -> dict | None:
    async with httpx.AsyncClient() as client:
        response = await client.get(f"{BOOKS_BASE}/volumes/{book_id}")
    if response.status_code != 200:
        return None
    return format_book(response.json())

def format_book(item: dict) -> dict:
    info = item.get("volumeInfo", {})
    images = info.get("imageLinks", {})
    return {
        "google_id": item["id"],
        "external_id": str(item["id"]),
        "title": info.get("title", ""),
        "title_russian": info.get("title"),
        "title_english": info.get("title"),
        "title_native": None,
        "poster_url": images.get("thumbnail", "").replace("http://", "https://"),
        "media_type": "books",
        "episodes": info.get("pageCount"),
        "status": "FINISHED",
        "description": info.get("description"),
        "score": None,
    }