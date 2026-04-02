from fastapi import APIRouter, Depends
from sqlalchemy.orm import Session

import models
import schemas
from database import get_db
from dependencies import get_current_user

router = APIRouter(prefix="/media", tags=["media"])


@router.post("", response_model=schemas.MediaItemResponse)
def create_media(
    media: schemas.MediaItemCreate,
    db: Session = Depends(get_db),
    current_user: models.User = Depends(get_current_user),
):
    db_media = models.MediaItem(**media.model_dump())
    db.add(db_media)
    db.commit()
    db.refresh(db_media)
    return db_media


@router.get("", response_model=list[schemas.MediaItemResponse])
def get_media(
    media_type: schemas.MediaType | None = None,
    db: Session = Depends(get_db),
    current_user: models.User = Depends(get_current_user),
):
    query = db.query(models.MediaItem)
    if media_type:
        query = query.filter(
            models.MediaItem.media_type == media_type
        )
    return query.all()