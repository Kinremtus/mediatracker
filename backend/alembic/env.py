import os
import sys
from logging.config import fileConfig

from sqlalchemy import engine_from_config, pool

from alembic import context

# Это нужно, чтобы Alembic видел твои модули (models, database)
sys.path.insert(0, os.path.dirname(os.path.dirname(__file__)))

# Импортируем твою базу и модели — важно, чтобы Alembic видел все таблицы
from database import Base, SQLALCHEMY_DATABASE_URL
import models  # noqa: F401 — модели должны загрузиться, чтобы Base.metadata их увидел

# Alembic Config
config = context.config

# Подменяем placeholder из alembic.ini на реальный URL из проекта
config.set_main_option("sqlalchemy.url", SQLALCHEMY_DATABASE_URL)

# Настройка логирования
if config.config_file_name is not None:
    fileConfig(config.config_file_name)

target_metadata = Base.metadata


def run_migrations_offline() -> None:
    url = config.get_main_option("sqlalchemy.url")
    context.configure(
        url=url,
        target_metadata=target_metadata,
        literal_binds=True,
        dialect_opts={"paramstyle": "named"},
    )

    with context.begin_transaction():
        context.run_migrations()


def run_migrations_online() -> None:
    connectable = engine_from_config(
        config.get_section(config.config_ini_section, {}),
        prefix="sqlalchemy.",
        poolclass=pool.NullPool,
    )

    with connectable.connect() as connection:
        context.configure(
            connection=connection,
            target_metadata=target_metadata,
        )

        with context.begin_transaction():
            context.run_migrations()


if context.is_offline_mode():
    run_migrations_offline()
else:
    run_migrations_online()