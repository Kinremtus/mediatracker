from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.orm import Session

import models
import schemas
from database import get_db
from dependencies import get_current_user
from services import anilist

router = APIRouter(prefix="/tracking", tags=["tracking"])


@router.post("", response_model=schemas.TrackingEntryResponse)
def add_tracking(
    entry: schemas.TrackingEntryCreate,
    db: Session = Depends(get_db),
    current_user: models.User = Depends(get_current_user),
):
    media = (
        db.query(models.MediaItem)
        .filter(models.MediaItem.id == entry.media_id)
        .first()
    )
    if not media:
        raise HTTPException(status_code=404, detail="Медиа не найдено")
    db_entry = models.TrackingEntry(
        **entry.model_dump(), user_id=current_user.id
    )
    db.add(db_entry)
    db.commit()
    db.refresh(db_entry)
    return db_entry


@router.get("", response_model=list[schemas.TrackingEntryResponse])
def get_tracking(
    status: schemas.TrackingStatus | None = None,
    db: Session = Depends(get_db),
    current_user: models.User = Depends(get_current_user),
):
    query = db.query(models.TrackingEntry).filter(
        models.TrackingEntry.user_id == current_user.id
    )
    if status:
        query = query.filter(models.TrackingEntry.status == status)
    return query.all()


@router.post(
    "/from-search", response_model=schemas.TrackingEntryResponse
)
async def add_tracking_from_search(
    entry: schemas.TrackingFromSearch,
    db: Session = Depends(get_db),
    current_user: models.User = Depends(get_current_user),
):
    media = (
        db.query(models.MediaItem)
        .filter(
            models.MediaItem.external_id == str(entry.anilist_id)
        )
        .first()
    )
    if not media:
        result = await anilist.search_anime_by_id(entry.anilist_id)
        if not result:
            raise HTTPException(
                status_code=404, detail="Аниме не найдено"
            )
        media = models.MediaItem(
            title=result["title_romaji"],
            title_english=result["title_english"],
            title_native=result["title_native"],
            title_russian=result["title_russian"],
            media_type="anime",
            external_id=str(entry.anilist_id),
            poster_url=result["poster_url"],
            episodes=result["episodes"],
        )
        db.add(media)
        db.commit()
        db.refresh(media)

    existing = (
        db.query(models.TrackingEntry)
        .filter(
            models.TrackingEntry.user_id == current_user.id,
            models.TrackingEntry.media_id == media.id,
        )
        .first()
    )
    if existing:
        raise HTTPException(
            status_code=400, detail="Уже в трекинге"
        )

    db_entry = models.TrackingEntry(
        media_id=media.id,
        user_id=current_user.id,
        status=entry.status,
        rating=entry.rating,
        progress=entry.progress,
    )
    db.add(db_entry)
    db.commit()
    db.refresh(db_entry)
    return db_entry

@router.put("/{entry_id}", response_model=schemas.TrackingEntryResponse)
def update_tracking(
    entry_id: int,
    data: schemas.TrackingEntryUpdate,
    db: Session = Depends(get_db),
    current_user: models.User = Depends(get_current_user),
):
    entry = (
        db.query(models.TrackingEntry)
        .filter(
            models.TrackingEntry.id == entry_id,
            models.TrackingEntry.user_id == current_user.id,
        )
        .first()
    )
    if not entry:
        raise HTTPException(status_code=404, detail="Запись не найдена")

    if data.status is not None:
        entry.status = data.status
    if data.rating is not None:
        entry.rating = data.rating
    if data.progress is not None:
        entry.progress = data.progress

    db.commit()
    db.refresh(entry)
    return entry