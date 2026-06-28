# Build stage
FROM rust:1.93-slim AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev git && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN --mount=type=secret,id=git_auth_token \
    git config --global url."https://x-access-token:$(cat /run/secrets/git_auth_token)@github.com/".insteadOf "https://github.com/" && \
    cargo build --release

# Runtime stage
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

RUN useradd -ms /bin/bash clann && \
    mkdir -p /app/uploads && \
    chown -R clann:clann /app

USER clann
WORKDIR /app

COPY --from=builder /app/target/release/clann-server ./

EXPOSE 3000

CMD ["./clann-server"]
