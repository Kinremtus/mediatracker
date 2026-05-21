# Builder stage
FROM rust:1.95-slim-bookworm AS builder
WORKDIR /app

# Copy only manifest files first for dependency caching
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies only
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN --mount=type=cache,target=/root/.cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/root/.cargo/git,id=cargo-git \
    cargo build --release
RUN rm -f target/release/deps/mediatracker*

# Copy actual source code
COPY . .

# Build the real application (dependencies already cached)
RUN --mount=type=cache,target=/root/.cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/root/.cargo/git,id=cargo-git \
    cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/mediatracker /app/mediatracker
COPY --from=builder /app/static /app/static
COPY --from=builder /app/migrations /app/migrations
EXPOSE 8080
CMD ["./mediatracker"]
