from fastapi import APIRouter, Depends

import models
import schemas
from dependencies import get_current_user
from services import anilist

router = APIRouter(prefix="/search", tags=["search"])


@router.get("/anime")
async def search_anime(
    q: str,
    current_user: models.User = Depends(get_current_user),
):
    return await anilist.search_anime(q)