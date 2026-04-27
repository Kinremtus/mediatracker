'''add constraints to email column

Revision ID: cdef1234add_email_constraints
Revises: add_status_column
Create Date: 2026-04-27 14:00:00.000000
'''

from alembic import op
import sqlalchemy as sa

revision = "cdef1234add_email_constraints"
down_revision = "add_status_column"

branch_labels = None
depends_on = None

def upgrade() -> None:
    # Ensure existing users have an email value
    op.execute(
        "UPDATE users SET email = username || '@example.com' WHERE email IS NULL"
    )
    # Make the column NOT NULL
    op.alter_column('users', 'email', nullable=False)
    # Add a UNIQUE constraint on email
    op.create_unique_constraint('uq_users_email', 'users', ['email'])

def downgrade() -> None:
    # Remove the unique constraint and allow nulls again
    op.drop_constraint('uq_users_email', 'users', type_='unique')
    op.alter_column('users', 'email', nullable=True)
