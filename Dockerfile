# Multi-stage Dockerfile for datanalyzer

# Build stage
FROM rust:1.75 as builder

# Create app directory
WORKDIR /usr/src/datanalyzer

# Copy manifest files
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY examples ./examples

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && \
    apt-get install -y \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 datanalyzer

# Copy binary from builder
COPY --from=builder /usr/src/datanalyzer/target/release/datanalyzer /usr/local/bin/datanalyzer

# Set ownership
RUN chown datanalyzer:datanalyzer /usr/local/bin/datanalyzer

# Switch to non-root user
USER datanalyzer

# Set working directory
WORKDIR /home/datanalyzer

# Run the binary
ENTRYPOINT ["datanalyzer"]
