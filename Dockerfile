# Build stage
FROM rust:1.86-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

RUN useradd -ms /bin/bash clann
USER clann
WORKDIR /app

COPY --from=builder /app/target/release/clann-server ./

EXPOSE 3000

CMD ["./clann-server"]
