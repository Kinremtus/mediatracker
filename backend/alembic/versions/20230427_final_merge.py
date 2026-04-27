"""final merge of all dangling heads

Revision ID: 20230427z9y8
Revises: 20230427m0g0, bdedc21ab4e8, a1b2c3d4e5f6, add_status_column
Create Date: 2026-04-27 13:00:00.000000
"""

from alembic import op
import sqlalchemy as sa

revision = "20230427z9y8"
# Merge all existing heads
#   20230427m0g0 – merge of provider & episodes
#   bdedc21ab4e8 – adds provider column (already applied)
#   a1b2c3d4e5f6 – adds email column (already applied)
#   add_status_column – adds status column (already applied)

down_revision = ("20230427m0g0", "bdedc21ab4e8", "a1b2c3d4e5f6", "add_status_column")
branch_labels = None
depends_on = None


def upgrade() -> None:
    """No schema changes – just a merge point for Alembic history."""
    pass


def downgrade() -> None:
    """Downgrade not supported for merge placeholder."""
    raise NotImplementedError("Downgrade of final merge revision is not supported")
