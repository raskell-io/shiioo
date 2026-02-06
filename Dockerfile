# Build stage
FROM rust:1.75-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release --package shiioo-server

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -r -s /bin/false shiioo

# Create data directory
RUN mkdir -p /data && chown shiioo:shiioo /data

# Copy binary from builder
COPY --from=builder /app/target/release/shiioo /usr/local/bin/shiioo

# Use non-root user
USER shiioo

# Set environment defaults
ENV SHIIOO_DATA_DIR=/data
ENV SHIIOO_HOST=0.0.0.0
ENV SHIIOO_PORT=8080
ENV RUST_LOG=shiioo=info,tower_http=info

# Expose port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health/live || exit 1

# Default command
ENTRYPOINT ["/usr/local/bin/shiioo"]
CMD ["--host", "0.0.0.0", "--port", "8080", "--data-dir", "/data"]
