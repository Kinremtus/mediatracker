"""add provider and description (already applied)

Revision ID: bdedc21ab4e8
Revises: 
Create Date: 2026-04-25 00:00:00.000000
"""

from alembic import op
import sqlalchemy as sa

# Alembic identifiers
revision = "bdedc21ab4e8"
down_revision = None
branch_labels = None
depends_on = None


def upgrade() -> None:
    """Column ``provider`` уже существует, поэтому ничего не делаем.
    Оставляем пустое тело, чтобы Alembic не пытался добавить столбец
    повторно.
    """
    pass


def downgrade() -> None:
    """Откат не требуется – столбец уже есть.
    Функция оставлена только для совместимости с Alembic.
    """
    pass
