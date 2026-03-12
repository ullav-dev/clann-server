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
src/config.rs                  Config::from_env()
src/db.rs                      connect() runs schema migration; pub type Db = Surreal<Any>
src/error.rs                   AppError → JSON { "error": "..." }
src/lib.rs                     pub mod declarations for integration tests
src/models/person.rs           Person, CreatePerson, UpdatePerson, Sex — includes nickname, username, email, verified, biography, created_by
src/models/relationship.rs     RelationshipType, FamilyTreeNode, SpouseInfo, etc.
src/handlers/person.rs         CRUD handlers
src/handlers/relationship.rs   relationship + family tree handlers
src/handlers/image.rs          image upload/retrieval handlers
src/openapi.rs                 ApiDoc, swagger_ui(), openapi_json()
src/routes/mod.rs              build_router(db, upload_dir)
migrations/schema.surql        SurrealDB schema (run at startup via include_str!)
tests/api.rs                   integration tests (40 tests)
```

## Environment variables

| Variable       | Default                  | Description                        |
|----------------|--------------------------|------------------------------------|
| `DB_URL`       | `ws://localhost:8000`    | SurrealDB WebSocket URL            |
| `DB_NAMESPACE` | `clann`                  | SurrealDB namespace                |
| `DB_DATABASE`  | `ancestry`               | SurrealDB database                 |
| `DB_USERNAME`  | `root`                   | SurrealDB username                 |
| `DB_PASSWORD`  | `secret`                 | SurrealDB password                 |
| `PORT`         | `3000`                   | HTTP listen port                   |
| `UPLOAD_DIR`   | `./uploads`              | Directory for person image files   |
| `DB_PATH`      | `/opt/ullav/clann/data.db` | SurrealDB data file path (used when starting SurrealDB with `surrealkv:$DB_PATH`) |

## SurrealDB notes

- **Do not use `type::thing()`** in SurrealQL — it fails in the embedded `kv-mem` engine. Bind `RecordId` objects directly instead.
- `RecordId` is `surrealdb::types::RecordId`; construct with `RecordId::new("table", "id")`.
- Types passed to/from the DB need `#[derive(SurrealValue)]` or a manual `SurrealValue` impl.
- `Root` auth takes owned `String` fields in v3: `Root { username: "x".to_string(), password: "y".to_string() }`.
- File-backed storage uses the `surrealkv:` prefix (not `file:`): `surreal start surrealkv:/path/to/data.db`.
- The `fetch_spouses` query in `relationship.rs` builds an explicit person sub-object to avoid field name collisions with the edge's own `id` — **any new person fields must also be added to that query**.
- When adding new fields to the schema after the database already exists, restart the server (which re-runs the migration) then backfill existing records: `UPDATE person SET new_field = <value> WHERE new_field = NONE;`
- Inspect the live schema with `INFO FOR DB;` or `INFO FOR TABLE person;` in the SurrealQL REPL.

## Testing

Integration tests use `any::connect("mem://")` with a unique namespace/database per test (via `AtomicU64` counter) for isolation. Each test calls `setup()` which connects, runs the schema migration, and returns a `Router`. No external database or services required to run tests.

## Relationship types

| JSON `type` | Edge table   | Extra fields                                      |
|-------------|--------------|---------------------------------------------------|
| `Father`    | `has_father` | —                                                 |
| `Mother`    | `has_mother` | —                                                 |
| `Sibling`   | `has_sibling`| `sibling_type`: `"Brother"` or `"Sister"`         |
| `Spouse`    | `has_spouse` | `spouse_from`, `spouse_to` (optional date strings)|

Spouse edges are stored bidirectionally (A→B and B→A). `GET .../relationships` returns `spouse: Vec<SpouseInfo>` which includes `spouse_from` and `spouse_to` from the edge. `PATCH .../spouse-dates/{related_id}` updates those fields on both directions simultaneously.
