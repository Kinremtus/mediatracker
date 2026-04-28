from pydantic import AliasChoices, BaseModel, Field
from enum import Enum
from datetime import datetime


class MediaType(str, Enum):
    anime = "anime"
    movie = "movie"
    game = "game"
    book = "book"


class TrackingStatus(str, Enum):
    planned = "planned"
    in_progress = "in_progress"
    completed = "completed"
    dropped = "dropped"


class UserCreate(BaseModel):
    username: str
    email: str
    password: str


class UserResponse(BaseModel):
    id: int
    username: str
    email: str

    model_config = {"from_attributes": True}


class Token(BaseModel):
    access_token: str
    token_type: str


class MediaItemCreate(BaseModel):
    title: str
    media_type: MediaType
    poster_url: str | None = None


class MediaItemResponse(BaseModel):
    id: int
    external_id: str
    title: str
    title_english: str | None = None
    title_native: str | None = None
    title_russian: str | None = None
    media_type: str
    poster_url: str | None = None
    episodes: int | None = None

    model_config = {"from_attributes": True}


class TrackingEntryCreate(BaseModel):
    media_id: int
    status: TrackingStatus = TrackingStatus.planned
    rating: float | None = None
    progress: int = 0


class TrackingEntryResponse(BaseModel):
    id: int
    status: str
    rating: float | None
    progress: int
    created_at: datetime
    media: MediaItemResponse

    model_config = {"from_attributes": True}


class SearchResult(BaseModel):
    external_id: str
    title: str
    title_english: str | None = None
    title_native: str | None = None
    title_russian: str | None = None
    poster_url: str | None = None
    media_type: str
    episodes: int | None = None
    seasons: int | None = None
    status: str | None = None
    score: int | None = None
    description: str | None = None

class TrackingFromSearch(BaseModel):
    # Можно оставить на 1 релиз для совместимости со старым фронтом
    external_id: str = Field(
        validation_alias=AliasChoices("external_id", "id")
    )
    media_type: str = Field(
        default="anime",
        validation_alias=AliasChoices("media_type", "type")
    )
    provider: str | None = Field(
        default=None,
        validation_alias=AliasChoices("provider")
    )
    status: TrackingStatus = TrackingStatus.planned
    rating: float | None = None
    progress: int = 0


class TrackingEntryUpdate(BaseModel):
    status: str | None = None
    rating: float | None = None
    progress: int | None = None