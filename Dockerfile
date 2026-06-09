# Chef stage: installs cargo-chef (used in planner + builder)
FROM rust:1.95-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

# Planner stage: produces a recipe.json from Cargo.toml/Cargo.lock
# The recipe is invalidated only when deps change, not when source code changes
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo chef prepare --recipe-path recipe.json
RUN rm -rf src

# Builder stage: cooks deps from recipe (cached in Docker layer until deps change),
# then builds the real app on top
FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
# Test gate: unit tests and the JS-syntax/contract test must pass
# before we build the release binary. The DB-gated integration
# tests (tracking_persistence, episode_persistence) are #[ignore]'d
# so they don't run here (no TEST_DATABASE_URL in build).
# If this fails, the image doesn't build and the deploy rolls back.
# (cargo test doesn't accept --lib and --test <name> in the same
# invocation, so we run them as two separate commands.)
RUN cargo test --lib
RUN cargo test --test app_js_syntax
RUN cargo build --release --bin mediatracker --bin backfill_anime --bin backfill_chapters

# Runtime stage: minimal debian + the binary + static + migrations
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates curl && rm -rf /var/lib/apt/lists/*
RUN adduser --disabled-password --no-create-home appuser
WORKDIR /app
COPY --from=builder /app/target/release/mediatracker /app/mediatracker
COPY --from=builder /app/target/release/backfill_anime /app/backfill_anime
COPY --from=builder /app/target/release/backfill_chapters /app/backfill_chapters
COPY --from=builder /app/static /app/static
COPY --from=builder /app/migrations /app/migrations
RUN chown -R appuser:appuser /app
USER appuser
EXPOSE 8080
CMD ["./mediatracker"]
