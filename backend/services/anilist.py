import httpx

# URL куда отправляем все запросы к AniList
ANILIST_URL = "https://graphql.anilist.co"

# GraphQL запрос — описываем какие поля хотим получить
# $search — переменная, подставим реальное значение при запросе
SEARCH_QUERY = """
query ($search: String!) {
  Page(perPage: 10) {
    media(search: $search, type: ANIME) {
      id
      title {
        romaji
        english
        native
      }
      synonyms
      coverImage {
        large
      }
      episodes
      status
      averageScore
    }
  }
}
"""

async def search_anime(query: str) -> list[dict]:
    # async with — открываем HTTP клиент, после блока закрываем автоматически
    async with httpx.AsyncClient() as client:
        response = await client.post(
            ANILIST_URL,
            json={
                "query": SEARCH_QUERY,
                "variables": {"search": query}  # подставляем поисковый запрос
            }
        )
        data = response.json()

    # data["data"]["Page"]["media"] — путь до списка аниме в ответе
    results = data["data"]["Page"]["media"]

    # Преобразуем каждый результат в удобный формат
    # Здесь используем list comprehension — это как цикл for но в одну строку
    return [format_anime(item) for item in results]

def format_anime(item: dict) -> dict:
    title = item["title"]

    # Ищем русское название в synonyms
    # synonyms — список всех альтернативных названий
    russian_title = None
    for synonym in item.get("synonyms", []):
        # Проверяем есть ли кириллица в названии
        if any("а" <= char <= "я" or "А" <= char <= "Я" for char in synonym):
            russian_title = synonym
            break  # нашли первое русское — останавливаемся

    return {
        "anilist_id": item["id"],
        "title_romaji": title.get("romaji"),
        "title_english": title.get("english"),
        "title_native": title.get("native"),
        "title_russian": russian_title,  # None если нет русского
        "poster_url": item["coverImage"]["large"],
        "episodes": item.get("episodes"),
        "status": item.get("status"),
        "score": item.get("averageScore"),
    }
    
ID_QUERY = """
query ($id: Int!) {
  Media(id: $id, type: ANIME) {
    id
    title {
      romaji
      english
      native
    }
    synonyms
    coverImage {
      large
    }
    episodes
    status
    averageScore
  }
}
"""

async def search_anime_by_id(anilist_id: int) -> dict | None:
    async with httpx.AsyncClient() as client:
        response = await client.post(
            ANILIST_URL,
            json={
                "query": ID_QUERY,
                "variables": {"id": anilist_id}
            }
        )
        data = response.json()

    item = data.get("data", {}).get("Media")
    if not item:
        return None
    return format_anime(item)