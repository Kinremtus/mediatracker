"""update provider from anilist to mal/mangadex

Revision ID: 20230427_update_provider_anilist
Revises: bdedc21ab4e8
Create Date: 2026-04-27 00:00:00.000000
"""

from alembic import op
import sqlalchemy as sa

revision = "20230427a1b2"
down_revision = "bdedc21ab4e8"
branch_labels = None
depends_on = None


def upgrade() -> None:
    # Anime – MyAnimeList
    op.execute(
        """
        UPDATE media_items
        SET provider = 'mal'
        WHERE provider = 'anilist' AND media_type = 'anime'
        """
    )
    # Manga family – MangaDex
    op.execute(
        """
        UPDATE media_items
        SET provider = 'mangadex'
        WHERE provider = 'anilist' AND media_type IN ('manga', 'manhwa', 'manhua', 'novels')
        """
    )


def downgrade() -> None:
    # Re‑vert back to anilist (useful only for local dev)
    op.execute(
        """
        UPDATE media_items
        SET provider = 'anilist'
        WHERE provider IN ('mal', 'mangadex')
        """
    )
