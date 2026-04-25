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
    # Добавляем колонки которых реально нет на VPS
    op.add_column(
        "media_items",
        sa.Column("provider", sa.String(), nullable=True),
    )
    op.add_column(
        "media_items",
        sa.Column("description", sa.Text(), nullable=True),
    )

    # Заполняем provider для существующих записей
    op.execute("UPDATE media_items SET provider = 'unknown' WHERE provider IS NULL")

    # Теперь делаем NOT NULL
    op.alter_column("media_items", "provider", nullable=False)
    op.alter_column(
        "media_items",
        "external_id",
        existing_type=sa.VARCHAR(),
        nullable=False,
    )

    # Уникальный constraint
    op.create_unique_constraint(
        "uix_provider_external",
        "media_items",
        ["provider", "external_id"],
    )
    # created_at — уже есть, не трогаем
    # status/score в media_items — не нужны


def downgrade() -> None:
    op.drop_constraint(
        "uix_provider_external", "media_items", type_="unique"
    )
    op.alter_column(
        "media_items",
        "external_id",
        existing_type=sa.VARCHAR(),
        nullable=True,
    )
    op.drop_column("media_items", "description")
    op.drop_column("media_items", "provider")