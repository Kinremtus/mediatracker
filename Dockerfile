# Builder stage
FROM rust:1.95-slim-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/mediatracker /app/mediatracker
COPY --from=builder /app/static /app/static
COPY --from=builder /app/migrations /app/migrations
EXPOSE 8080
CMD ["./mediatracker"]
