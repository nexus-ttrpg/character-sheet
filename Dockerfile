# ── Stage 1: Build adapter bundles (Node) ───────────────────────────────────
FROM node:22-bookworm-slim AS nodebuilder

WORKDIR /app

# Install pnpm
RUN corepack enable && corepack prepare pnpm@latest --activate

# Copy workspace config + lockfile for dependency caching
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY packages/ packages/
COPY adapters/ adapters/

# Install deps and build adapter bundles
RUN pnpm install --frozen-lockfile

# Build each adapter's bundle (single-file esbuild ESM output)
RUN pnpm --filter @nexus-ttrpg/foundry-adapter bundle \
 && pnpm --filter @nexus-ttrpg/roll20-adapter bundle

# ── Stage 2: Build the API binary (Rust) ────────────────────────────────────
FROM rust:1.97-bookworm-slim AS rustbuilder

WORKDIR /app

# Install needed build deps (libssl for reqwest, pkg-config)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy Cargo manifests first for dependency caching
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
COPY cli/ cli/
COPY xtask/ xtask/

# Build the API binary (release)
RUN cargo build --release -p api

# ── Stage 3: Distroless runtime ─────────────────────────────────────────────
FROM gcr.io/distroless/cc-debian12:nonroot

WORKDIR /app

# Copy the API binary
COPY --from=rustbuilder /app/target/release/nexus-api /app/nexus-api

# Copy pre-built adapter bundles
COPY --from=nodebuilder /app/adapters/foundry/dist/bundle/bundle.js /app/adapters/foundry/bundle.js
COPY --from=nodebuilder /app/adapters/roll20/dist/bundle/bundle.js /app/adapters/roll20/bundle.js

# Non-root user (distroless :nonroot provides UID 65532)
USER nonroot

EXPOSE 3000

ENTRYPOINT ["/app/nexus-api"]
