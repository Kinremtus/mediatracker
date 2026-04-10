from fastapi import APIRouter, Depends
import models
from dependencies import get_current_user
from services import anilist, tmdb

router = APIRouter(prefix="/search", tags=["search"])


@router.get("/anime")
async def search_anime(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await anilist.search_anime(q)


@router.get("/movies")
async def search_movies(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_movies(q)


@router.get("/tv")
async def search_tv(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await tmdb.search_tv(q)