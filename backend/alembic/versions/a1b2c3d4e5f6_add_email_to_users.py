"""add email column to users

Revision ID: a1b2c3d4e5f6
Revises: bdedc21ab4e8
Create Date: 2026-04-25 00:00:00.000000

"""
from alembic import op
import sqlalchemy as sa

revision = "a1b2c3d4e5f6"
down_revision = "bdedc21ab4e8"
branch_labels = None
depends_on = None


def upgrade() -> None:
    op.add_column(
        "users",
        sa.Column("email", sa.String(), nullable=True),
    )


def downgrade() -> None:
    op.drop_column("users", "email")