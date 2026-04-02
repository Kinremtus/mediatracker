from pydantic import BaseModel
from enum import Enum

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

# --- User ---
class UserCreate(BaseModel):
    username: str
    password: str

class UserResponse(BaseModel):
    id: int
    username: str

    model_config = {"from_attributes": True}

# --- Token ---
class Token(BaseModel):
    access_token: str
    token_type: str

# --- MediaItem ---
class MediaItemCreate(BaseModel):
    title: str
    media_type: MediaType
    poster_url: str | None = None

class MediaItemResponse(BaseModel):
    id: int
    title: str
    title_english: str | None
    title_native: str | None
    title_russian: str | None
    media_type: str
    poster_url: str | None
    episodes: int | None

    model_config = {"from_attributes": True}

# --- TrackingEntry ---
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
    media: MediaItemResponse   # вложенный объект — сразу видно что трекаешь

    model_config = {"from_attributes": True}

# --- Search ---
class TrackingFromSearch(BaseModel):
    anilist_id: int
    status: TrackingStatus = TrackingStatus.planned
    rating: float | None = None
    progress: int = 0