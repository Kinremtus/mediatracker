from sqlalchemy import Column, Integer, String, Float, ForeignKey
from sqlalchemy.orm import relationship
from database import Base

class User(Base):
    __tablename__ = "users"

    id = Column(Integer, primary_key=True)
    username = Column(String, unique=True, nullable=False)
    hashed_password = Column(String, nullable=False)

    tracking_entries = relationship("TrackingEntry", back_populates="owner")

class MediaItem(Base):
    __tablename__ = "media_items"

    id = Column(Integer, primary_key=True)
    title = Column(String, nullable=False)
    title_english = Column(String, nullable=True)
    title_native = Column(String, nullable=True)
    title_russian = Column(String, nullable=True)
    media_type = Column(String, nullable=False)
    external_id = Column(String, nullable=True)  # anilist_id как строка
    poster_url = Column(String, nullable=True)
    episodes = Column(Integer, nullable=True)

    tracking_entries = relationship("TrackingEntry", back_populates="media")

class TrackingEntry(Base):
    __tablename__ = "tracking_entries"

    id = Column(Integer, primary_key=True)
    status = Column(String, nullable=False, default="planned")
    rating = Column(Float, nullable=True)        # оценка 1-10
    progress = Column(Integer, default=0)        # серия/страница/часы

    user_id = Column(Integer, ForeignKey("users.id"), nullable=False)
    media_id = Column(Integer, ForeignKey("media_items.id"), nullable=False)

    owner = relationship("User", back_populates="tracking_entries")
    media = relationship("MediaItem", back_populates="tracking_entries")