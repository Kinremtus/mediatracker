from sqlalchemy.orm import relationship
from database import Base
from sqlalchemy import (
    Column,
    Integer,
    ForeignKey,
    String,
    Text,              # ← добавь это
    UniqueConstraint,  # ← и это
)
from sqlalchemy.ext.declarative import declarative_base
from datetime import datetime, timezone

Base = declarative_base()

class User(Base):
    __tablename__ = "users"

    id = Column(Integer, primary_key=True)
    username = Column(String, unique=True, nullable=False)
    hashed_password = Column(String, nullable=False)

    tracking_entries = relationship("TrackingEntry", back_populates="owner")

class MediaItem(Base):
    __tablename__ = "media_items"

    id = Column(Integer, primary_key=True)
    external_id = Column(String, nullable=False)
    provider = Column(String, nullable=False, default="anilist")  # ← новое
    media_type = Column(String, nullable=False)
    title = Column(String, nullable=False)
    title_english = Column(String, nullable=True)
    title_native = Column(String, nullable=True)
    title_russian = Column(String, nullable=True)
    poster_url = Column(String, nullable=True)
    episodes = Column(Integer, nullable=True)
    description = Column(Text, nullable=True)
    status = Column(String, nullable=True)
    score = Column(Integer, nullable=True)

    # Уникальность теперь по паре, а не только по external_id
    __table_args__ = (
        UniqueConstraint("provider", "external_id", name="uix_provider_external"),
    )
class TrackingEntry(Base):
    __tablename__ = "tracking_entries"

    id = Column(Integer, primary_key=True)
    status = Column(String, nullable=False, default="planned")
    rating = Column(Float, nullable=True)        # оценка 1-10
    progress = Column(Integer, default=0)        # серия/страница/часы
    created_at = Column(DateTime, default=lambda: datetime.now(timezone.utc))  # ← добавить

    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)
    media_id = Column(Integer, ForeignKey("media_items.id"), nullable=False)

    owner = relationship("User", back_populates="tracking_entries")
    media = relationship("MediaItem", back_populates="tracking_entries")