from fastapi import APIRouter, Depends
import models
from dependencies import get_current_user
from services import anilist, tmdb, rawg, books

router = APIRouter(prefix="/search", tags=["search"])

MANGA_TYPES = ("manga", "manhwa", "manhua", "novels")
TMDB_TYPES = ("movies", "movie", "tv-shows", "tv", "dramas", "cartoons", "animated-movies")

@router.get("/details")
async def get_media_details(
    media_type: str,
    external_id: str,
    current_user: models.User = Depends(get_current_user),
):
    if media_type == "anime":
        return await anilist.search_anime_by_id(int(external_id))
    elif media_type in MANGA_TYPES:
        return await anilist.search_manga_by_id(int(external_id))
    elif media_type in TMDB_TYPES:
        return await tmdb.get_by_id(int(external_id), media_type)
    elif media_type == "games":
        return await rawg.get_game_by_id(external_id)
    elif media_type == "books":
        return await books.get_book_by_id(external_id)
    return None

@router.get("/anime")
async def search_anime(q: str, current_user: models.User = Depends(get_current_user)):
    return await anilist.search_anime(q)

@router.get("/manga")
async def search_manga(q: str, current_user: models.User = Depends(get_current_user)):
    return await anilist.search_manga(q, country="JP")

@router.get("/manhwa")
async def search_manhwa(q: str, current_user: models.User = Depends(get_current_user)):
    return await anilist.search_manga(q, country="KR")

@router.get("/manhua")
async def search_manhua(q: str, current_user: models.User = Depends(get_current_user)):
    return await anilist.search_manga(q, country="CN")

@router.get("/novels")
async def search_novels(q: str, current_user: models.User = Depends(get_current_user)):
    return await anilist.search_manga(q, fmt="NOVEL")

@router.get("/movies")
async def search_movies(q: str, current_user: models.User = Depends(get_current_user)):
    return await tmdb.search_movies(q)

@router.get("/tv")
async def search_tv(q: str, current_user: models.User = Depends(get_current_user)):
    return await tmdb.search_tv(q)

@router.get("/dramas")
async def search_dramas(q: str, current_user: models.User = Depends(get_current_user)):
    return await tmdb.search_tv(q, genre_id=18)

@router.get("/cartoons")
async def search_cartoons(q: str, current_user: models.User = Depends(get_current_user)):
    return await tmdb.search_tv(q, genre_id=16)

@router.get("/animated-movies")
async def search_animated_movies(q: str, current_user: models.User = Depends(get_current_user)):
    return await tmdb.search_movies(q, genre_id=16)

@router.get("/games")
async def search_games_endpoint(q: str, current_user: models.User = Depends(get_current_user)):
    return await rawg.search_games(q)

@router.get("/books")
async def search_books_endpoint(q: str, current_user: models.User = Depends(get_current_user)):
    return await books.search_books(q)