# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`clann-server` is a Rust REST API server for managing ancestry and family tree data, backed by SurrealDB.

## Commands

```bash
cargo build          # Build the project
cargo run            # Run the server
cargo test           # Run all tests
cargo test <name>    # Run a single test by name
cargo clippy         # Lint
cargo fmt            # Format code
```

## Architecture

- **Web framework**: Axum 0.8 (`{id}` path param syntax, not `:id`)
- **Database**: SurrealDB 3.x via `surrealdb::engine::any` (`Surreal<Any>`) — supports both `ws://` (prod) and `mem://` (tests)
- **OpenAPI**: utoipa v5 — Swagger UI at `/swagger-ui`, spec at `/api-docs/openapi.json`

### Key files

```
src/main.rs                    entry point
src/config.rs                  Config::from_env() — supports DB_USERNAME_FILE/DB_PASSWORD_FILE
src/db.rs                      connect() runs schema migration; pub type Db = Surreal<Any>
src/error.rs                   AppError → JSON { "error": "..." }
src/lib.rs                     pub mod declarations for integration tests
src/models/family_tree.rs      FamilyTree, CreateFamilyTree
src/models/person.rs           Person, CreatePerson, UpdatePerson, Sex — includes tree, nickname, username, email, verified, biography, created_by
src/models/relationship.rs     RelationshipType, FamilyTreeNode (includes sex, date_of_birth, place_of_birth, biography), SpouseInfo, etc.
src/handlers/family_tree.rs    CRUD handlers for family trees
src/handlers/person.rs         CRUD handlers (validates tree exists on create)
src/handlers/relationship.rs   relationship + family tree handlers
src/handlers/image.rs          image upload/retrieval handlers
src/openapi.rs                 ApiDoc, swagger_ui(), openapi_json()
src/routes/mod.rs              build_router(db, upload_dir)
migrations/schema.surql        SurrealDB schema (run at startup via include_str!)
openapi.json                   committed OpenAPI spec (regenerate with: curl localhost:3000/api-docs/openapi.json)
tests/api.rs                   integration tests (52 tests)
Dockerfile                     multi-stage build (rust:1.93-slim → debian:bookworm-slim)
docker-compose.yml             production compose: surrealdb + migrate + server
.env.prod                      non-secret env vars for docker-compose (gitignored)
secrets/                       Docker secret files: db_username.txt, db_password.txt (gitignored)
scripts/migrate.sh             migration runner — applies .surql files with schema_migration tracking
scripts/Dockerfile.migrate     alpine + surreal binary for the migrate service
```

## Environment variables

| Variable              | Default                    | Description                                                           |
|-----------------------|----------------------------|-----------------------------------------------------------------------|
| `DB_URL`              | `ws://localhost:8000`      | SurrealDB WebSocket URL                                               |
| `DB_NAMESPACE`        | `clann`                    | SurrealDB namespace                                                   |
| `DB_DATABASE`         | `ancestry`                 | SurrealDB database                                                    |
| `DB_USERNAME`         | `root`                     | SurrealDB username                                                    |
| `DB_USERNAME_FILE`    | —                          | Path to file containing DB username (Docker secrets — takes priority) |
| `DB_PASSWORD`         | `secret`                   | SurrealDB password                                                    |
| `DB_PASSWORD_FILE`    | —                          | Path to file containing DB password (Docker secrets — takes priority) |
| `PORT`                | `3000`                     | HTTP listen port                                                      |
| `UPLOAD_DIR`          | `./uploads`                | Directory for person image files                                      |
| `DB_PATH`             | `/opt/ullav/clann/data.db` | SurrealDB data file path (used when starting SurrealDB with `surrealkv:$DB_PATH`) |

## SurrealDB notes

- **Do not use `type::thing()`** in SurrealQL — it fails in the embedded `kv-mem` engine. Bind `RecordId` objects directly instead.
- `RecordId` is `surrealdb::types::RecordId`; construct with `RecordId::new("table", "id")`.
- Types passed to/from the DB need `#[derive(SurrealValue)]` or a manual `SurrealValue` impl.
- `Root` auth takes owned `String` fields in v3: `Root { username: "x".to_string(), password: "y".to_string() }`.
- File-backed storage uses the `surrealkv:` prefix (not `file:`): `surreal start surrealkv:/path/to/data.db`.
- The `fetch_spouses` query in `relationship.rs` builds an explicit person sub-object to avoid field name collisions with the edge's own `id` — **any new person fields must also be added to that query**.
- When adding new fields to the schema after the database already exists, restart the server (which re-runs the migration) then backfill existing records: `UPDATE person SET new_field = <value> WHERE new_field = NONE;`
- Inspect the live schema with `INFO FOR DB;` or `INFO FOR TABLE person;` in the SurrealQL REPL.
- Migration state in Docker is tracked in the `schema_migration` table (filename + applied_at). `scripts/migrate.sh` skips already-applied files — safe to run on every deploy.

## Testing

Integration tests use `any::connect("mem://")` with a unique namespace/database per test (via `AtomicU64` counter) for isolation. Each test calls `setup()` which connects, runs the schema migration, and returns a `Router`. No external database or services required to run tests.

## Family trees

- `family_tree` table: `name` (globally unique slug), `display_name`, `owner`, `is_primary` (bool)
- Each `person` record has a required `tree` field (validated against `family_tree.name` on creation)
- `DELETE /api/trees/{name}` cascade-deletes all persons in the tree and their relationship edges
- Setting `is_primary: true` on create clears `is_primary` on all other trees for the same owner
- `GET /api/persons` accepts `?tree=` filter in addition to `?created_by=`
- Tests seed a default tree (`"test-tree"`) in `setup()` and pass `"tree": TEST_TREE` in every person creation

## Relationship types

| JSON `type` | Edge table   | Extra fields                                      |
|-------------|--------------|---------------------------------------------------|
| `Father`    | `has_father` | —                                                 |
| `Mother`    | `has_mother` | —                                                 |
| `Sibling`   | `has_sibling`| `sibling_type`: `"Brother"` or `"Sister"`         |
| `Spouse`    | `has_spouse` | `spouse_from`, `spouse_to` (optional date strings)|

Spouse edges are stored bidirectionally (A→B and B→A). `GET .../relationships` returns `spouse: Vec<SpouseInfo>` which includes `spouse_from` and `spouse_to` from the edge. `PATCH .../spouse-dates/{related_id}` updates those fields on both directions simultaneously.
