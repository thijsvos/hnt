# Stage 1: Builder
FROM rust:1.95-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Cache dependencies by copying manifests first
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release 2>/dev/null || true
RUN rm -rf src

# Copy source and build. The `touch` is load-bearing: Docker COPY may set
# source mtimes older than the cached stub-binary artifact from the layer
# above, in which case cargo's fingerprint check decides "nothing changed"
# and skips the application rebuild, leaving the stub `fn main() {}` binary
# in target/release/hnt. Touching every .rs file forces cargo to invalidate
# the application crate while preserving the warmed dependency cache.
COPY src/ src/
RUN find src -name '*.rs' -exec touch {} + && cargo build --release

# Stage 2: Runtime
FROM debian:trixie-slim AS runtime

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Containers have no host browser; tell hnt to copy URLs to the user's
# clipboard via OSC 52 instead of trying (and silently failing) to spawn
# xdg-open. See `App::open_url`. `/.dockerenv` would also auto-detect
# this; the explicit env var is belt-and-suspenders for runtimes that
# don't drop that marker (e.g. some podman configs).
ENV HNT_NO_BROWSER=1

COPY --from=builder /app/target/release/hnt /usr/local/bin/hnt

ENTRYPOINT ["hnt"]
