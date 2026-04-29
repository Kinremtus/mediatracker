from fastapi import APIRouter, Depends, HTTPException
import models
import schemas
from dependencies import get_current_user
from services import mal, tmdb, rawg, books, shikimori, mangaupdates


router = APIRouter(prefix="/search", tags=["search"])

MANGA_TYPES = ("manga", "manhwa", "manhua", "novels")
TMDB_TYPES = (
    "movies",
    "movie",
    "tv-shows",
    "tv",
    "dramas",
    "cartoons",
    "animated-movies",
)


@router.get("/details", response_model=schemas.SearchResult | None)
async def get_media_details(
    media_type: str,
    external_id: str,
    current_user: models.User = Depends(get_current_user),
):
    if not external_id:
        raise HTTPException(status_code=400, detail="external_id is required")

    if media_type == "anime":
        return await mal.get_anime_by_id(int(external_id))
    elif media_type in MANGA_TYPES:
        return await mangaupdates.get_series_by_id(str(external_id))
    elif media_type in TMDB_TYPES:
        return await tmdb.get_by_id(int(external_id), media_type)
    elif media_type == "games":
        return await rawg.get_game_by_id(external_id)
    elif media_type == "books":
        return await books.get_book_by_id(external_id)

    raise HTTPException(status_code=400, detail="Неизвестный тип медиа")


@router.get("/anime", response_model=list[schemas.SearchResult])
async def search_anime(q: str):
    # Пока используем только Shikimori для аниме
    return await shikimori.search_anime(q)


@router.get("/manga", response_model=list[schemas.SearchResult])
async def search_manga(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await mangaupdates.search_series(
        q,
        allowed_types=[
            "Manga",
            "OEL",
            "Doujinshi",
            "Filipino",
            "Indonesian",
            "Thai",
            "Vietnamese",
            "Malaysian",
        ],
    )


@router.get("/manhwa", response_model=list[schemas.SearchResult])
async def search_manhwa(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await mangaupdates.search_series(q, allowed_types=["Manhwa"])


@router.get("/manhua", response_model=list[schemas.SearchResult])
async def search_manhua(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await mangaupdates.search_series(q, allowed_types=["Manhua"])


@router.get("/novels", response_model=list[schemas.SearchResult])
async def search_novels(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await mangaupdates.search_series(q, allowed_types=["novel"])


@router.get("/movies", response_model=list[schemas.SearchResult])
async def search_movies(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_movies(q)


@router.get("/tv", response_model=list[schemas.SearchResult])
async def search_tv(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_tv(q)


@router.get("/dramas", response_model=list[schemas.SearchResult])
async def search_dramas(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_tv(q, genre_id=18)


@router.get("/cartoons", response_model=list[schemas.SearchResult])
async def search_cartoons(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_tv(q, genre_id=16)


@router.get(
    "/animated-movies",
    response_model=list[schemas.SearchResult],
)
async def search_animated_movies(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_movies(q, genre_id=16)


@router.get("/games", response_model=list[schemas.SearchResult])
async def search_games_endpoint(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await rawg.search_games(q)


@router.get("/books", response_model=list[schemas.SearchResult])
async def search_books_endpoint(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await books.search_books(q)