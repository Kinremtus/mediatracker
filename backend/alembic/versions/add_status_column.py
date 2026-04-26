"""add status to media_items

Revision ID: add_status_column
Revises: bdedc21ab4e8
Create Date: 2026-04-26 00:00:00.000000

"""
from alembic import op
import sqlalchemy as sa

revision = "add_status_column"
down_revision = "bdedc21ab4e8"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.add_column(
        "media_items",
        sa.Column("status", sa.String(), nullable=True),
    )


def downgrade() -> None:
    op.drop_column("media_items", "status")