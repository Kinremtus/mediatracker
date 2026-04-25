"""add provider and description

Revision ID: bdedc21ab4e8
Revises:
Create Date: 2026-04-25 00:00:00.000000

"""
from alembic import op
import sqlalchemy as sa

revision = "bdedc21ab4e8"
down_revision = None
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.add_column(
        "media_items",
        sa.Column("provider", sa.String(), nullable=True),
    )
    op.add_column(
        "media_items",
        sa.Column("description", sa.Text(), nullable=True),
    )

    op.execute(
        """
        UPDATE media_items
        SET provider = CASE
            WHEN media_type IN ('anime', 'manga') THEN 'anilist'
            WHEN media_type IN ('movie', 'movies', 'tv', 'series') THEN 'tmdb'
            WHEN media_type IN ('game', 'games') THEN 'rawg'
            WHEN media_type IN ('book', 'books') THEN 'books'
            ELSE 'unknown'
        END
        WHERE provider IS NULL
        """
    )

    op.execute(
        """
        UPDATE media_items
        SET external_id = 'legacy-' || id::text
        WHERE external_id IS NULL OR external_id = ''
        """
    )

    op.alter_column(
        "media_items",
        "provider",
        existing_type=sa.String(),
        nullable=False,
    )
    op.alter_column(
        "media_items",
        "external_id",
        existing_type=sa.VARCHAR(),
        nullable=False,
    )

    op.create_unique_constraint(
        "uix_provider_external",
        "media_items",
        ["provider", "external_id"],
    )