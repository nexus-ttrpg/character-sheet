# Self-Hosting Nexus Character Sheet

Nexus Character Sheet offers two self-hosting paths:

1. **Docker + Postgres** — the full API server with webhooks, rate limiting, API
   keys, and multi-user support. Use this if you want the REST API and adapter
   sync running as a service.
2. **CLI + SQLite** — the offline-first `nexus` binary. No server, no Docker, no
   network. A GM runs the entire campaign against a local SQLite file. Use this
   if you just want to sync characters between VTTs from the command line.

Both paths are fully self-contained — neither contacts any hosted Nexus Character Sheet
infrastructure (§12).

---

## Path 1: Docker + Postgres

### Prerequisites

- Docker Engine 24+ (or Docker Desktop)
- Docker Compose v2

### Quick start

```bash
# 1. Generate the encryption key (32 bytes for AES-256-GCM)
head -c 32 /dev/urandom > encryption.key

# 2. Set the bootstrap admin API key
export NEXUS_ADMIN_API_KEY=$(head -c 16 /dev/urandom | xxd -p)
echo "Admin API key: $NEXUS_ADMIN_API_KEY"

# 3. Start the stack
docker compose up -d
```

The API is now available at `http://localhost:3000`. The admin API key has full
scope (read/write/key management). Create scoped keys for relay modules and
other clients via the API:

```bash
curl -X POST http://localhost:3000/v1/api-keys \
  -H "Authorization: Bearer $NEXUS_ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"scope": "sync", "label": "foundry-relay"}'
```

### Configuration

Environment variables are set in `docker-compose.yml`:

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | yes | — | Postgres connection string |
| `NEXUS_PORT` | no | `3000` | API listen port |
| `NEXUS_ADAPTERS_DIR` | no | `/app/adapters` | Adapter bundles directory |
| `NEXUS_ENCRYPTION_KEY_FILE` | no* | — | 32-byte key file for credential encryption |
| `NEXUS_ADMIN_API_KEY` | no | — | Bootstrap admin key (full scope) |
| `RUST_LOG` | no | `info` | Log level (`info`, `debug`, `trace`) |

*If `NEXUS_ENCRYPTION_KEY_FILE` is unset, a random key is generated on startup.
This is fine for development but **encrypted credentials become unreadable after
restart** — always set this in production.

### Adapter bundles

The Docker image ships with pre-built Foundry and Roll20 adapter bundles. To add
a custom adapter:

1. Build the adapter bundle: `pnpm --filter @nexus-ttrpg/my-adapter bundle`
2. Mount it: add a volume in `docker-compose.yml`:
   ```yaml
   volumes:
     - ./adapters:/app/adapters:ro
   ```
3. Place your `bundle.js` at `./adapters/my-adapter/bundle.js`

### Postgres

The default `docker-compose.yml` uses Postgres 16 Alpine with a named volume
(`pgdata`) for persistence. To back up:

```bash
docker compose exec postgres pg_dump -U nexus nexus > backup.sql
```

To restore:

```bash
docker compose exec -T postgres psql -U nexus nexus < backup.sql
```

### Health checks

The Postgres service has a built-in health check. The API service depends on it
and will wait for Postgres to be healthy before starting.

### Stopping

```bash
docker compose down          # stop containers, keep data
docker compose down -v       # stop containers, DELETE all data
```

---

## Path 2: CLI + SQLite (offline)

### Prerequisites

- Rust toolchain (1.97+) or download a pre-built binary from releases

### Build

```bash
cargo build --release -p nexus-cli
# Binary: target/release/nexus
```

### Usage

```bash
# Import a Foundry actor export into canonical
nexus import --foundry ./actor-export.json

# Import into an existing character
nexus import --foundry --character 01JZ... ./actor-export.json

# Export the current HEAD back to a platform payload
nexus push --foundry --character 01JZ... --out ./output.json

# See what changed between HEAD and a new import
nexus diff --foundry --character 01JZ... ./actor-export.json

# View snapshot history
nexus history 01JZ...
```

The CLI uses a local SQLite database (`nexus.sqlite` by default). No server, no
network, no Docker. All adapter execution happens in-process via the V8 sandbox
(§5.5).

### Adapter bundles for CLI

The CLI looks for adapter bundles in `./adapters/` by default. Build them:

```bash
pnpm install
pnpm --filter @nexus-ttrpg/foundry-adapter bundle
pnpm --filter @nexus-ttrpg/roll20-adapter bundle
```

This produces `adapters/foundry/dist/bundle/bundle.js` and
`adapters/roll20/dist/bundle/bundle.js`. Copy the `adapters/` directory next to
the `nexus` binary, or use `--adapters-dir /path/to/adapters`.

---

## Security notes (§17)

- **API keys** are hashed (Argon2id) and never stored in plaintext. The bootstrap
  admin key from `NEXUS_ADMIN_API_KEY` is also hashed on storage. Use scoped keys
  (read-only / sync) for relay modules and other clients.
- **Connection credentials** (platform tokens) are encrypted at rest with
  AES-256-GCM. The encryption key comes from `NEXUS_ENCRYPTION_KEY_FILE` — keep
  this file secure and backed up.
- **Rate limiting** is in-memory (per-key token bucket). This is fine for
  single-instance self-hosted deployments. Multi-instance would need Redis.
- **Adapter sandbox**: adapters run in a V8 sandbox with no filesystem, no
  environment access, and HTTPS-only network limited to manifest-allowlisted
  hosts (§5.5, §17).
- **Supply chain**: `cargo-deny` and `cargo-audit` run in CI. Lockfiles are
  committed (`Cargo.lock`, `pnpm-lock.yaml`).

### Generating an encryption key

```bash
# 32 bytes = 256 bits for AES-256-GCM
head -c 32 /dev/urandom > encryption.key
chmod 600 encryption.key
```

Store this file securely. If lost, all encrypted connection credentials become
unreadable. If compromised, rotate it and re-create all platform connections.

---

## Architecture reference

See `cross-vtt-sync-architecture.md` for the full design:
- §12 — offline support and self-hosting philosophy
- §13 — REST API design
- §14 — technology stack
- §17 — security posture
