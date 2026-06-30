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
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo chef cook --release --recipe-path recipe.json
COPY . .
# Release build first, then tests reuse release artifacts
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/app/target \
    cargo build --release --bin mediatracker --bin healthcheck --bin backfill_anime --bin backfill_chapters && \
    cp /app/target/release/mediatracker /app/mediatracker && \
    cp /app/target/release/healthcheck /app/healthcheck && \
    cp /app/target/release/backfill_anime /app/backfill_anime && \
    cp /app/target/release/backfill_chapters /app/backfill_chapters && \
    cargo test --release --lib && \
    cargo test --release --test app_js_syntax

# Runtime stage: distroless — no shell, no apt, no curl
FROM gcr.io/distroless/cc:nonroot
WORKDIR /app
COPY --from=builder --chown=65532:65532 /app/mediatracker /app/mediatracker
COPY --from=builder --chown=65532:65532 /app/healthcheck /app/healthcheck
COPY --from=builder --chown=65532:65532 /app/backfill_anime /app/backfill_anime
COPY --from=builder --chown=65532:65532 /app/backfill_chapters /app/backfill_chapters
COPY --from=builder --chown=65532:65532 /app/static /app/static
COPY --from=builder --chown=65532:65532 /app/migrations /app/migrations
EXPOSE 8080
CMD ["./mediatracker"]
