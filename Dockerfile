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
RUN cargo build --release

# Runtime stage: minimal debian + the binary + static + migrations
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates curl && rm -rf /var/lib/apt/lists/*
RUN adduser --disabled-password --no-create-home appuser
WORKDIR /app
COPY --from=builder /app/target/release/mediatracker /app/mediatracker
COPY --from=builder /app/static /app/static
COPY --from=builder /app/migrations /app/migrations
RUN chown -R appuser:appuser /app
USER appuser
EXPOSE 8080
CMD ["./mediatracker"]
