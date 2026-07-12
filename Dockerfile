# Build stage
FROM rust:1.94-bookworm AS builder

WORKDIR /app

# Copy dependency manifests first for layer caching
COPY Cargo.toml Cargo.lock build.rs ./

# Create dummy src to build dependencies
RUN mkdir src && \
    echo 'fn main() {}' > src/main.rs && \
    mkdir -p src/bin && \
    echo 'fn main() {}' > src/bin/obsidian-semanticd.rs

# Build dependencies only (cached unless Cargo.toml/lock change)
RUN cargo build --release --features embeddings-api --bin obsidian-mcp 2>/dev/null || true

# Copy actual source
COPY src/ src/

# Touch main.rs to invalidate the dummy binary but keep deps
RUN touch src/main.rs

# Build the real binary
RUN cargo build --release --features embeddings-api --bin obsidian-mcp

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/obsidian-mcp /usr/local/bin/obsidian-mcp

ENTRYPOINT ["obsidian-mcp"]
