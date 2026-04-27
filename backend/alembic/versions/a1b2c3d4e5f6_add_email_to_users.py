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
    # 1. Add the column as nullable to avoid breaking existing rows
    op.add_column(
        "users",
        sa.Column("email", sa.String(), nullable=True),
    )
    # 2. Fill existing rows with a placeholder email based on username
    op.execute(
        "UPDATE users SET email = username || '@example.com' WHERE email IS NULL"
    )
    # 3. Make the column NOT NULL
    op.alter_column("users", "email", nullable=False)
    # 4. Add a UNIQUE constraint (the model also defines unique=True)
    op.create_unique_constraint("uq_users_email", "users", ["email"])


def downgrade() -> None:
    # Drop the unique constraint first, then the column
    op.drop_constraint("uq_users_email", "users", type_="unique")
    op.drop_column("users", "email")