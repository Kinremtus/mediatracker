"""merge provider update and episodes column migrations

Revision ID: 20230427m0g0
Revises: aa7411391eb2, 20230427a1b2
Create Date: 2026-04-27 12:30:00.000000
"""

from alembic import op
import sqlalchemy as sa

revision = "20230427m0g0"
# Down revisions: both previous heads – merge them
# Alembic accepts a tuple/list for multiple parents
down_revision = ("aa7411391eb2", "20230427a1b2")
branch_labels = None
depends_on = None


def upgrade() -> None:
    # No schema changes – just a merge point for Alembic history.
    pass


def downgrade() -> None:
    # Can't really downgrade a merge – raise not implemented.
    raise NotImplementedError("Downgrade of merge revision is not supported")
