# clann-server

A REST API server for managing ancestry and family tree data, written in Rust.

## Stack

- **[Axum](https://github.com/tokio-rs/axum)** — async web framework
- **[SurrealDB](https://surrealdb.com)** — graph database for storing persons and relationships
- **[utoipa](https://github.com/juhaku/utoipa)** — OpenAPI 3 spec generation with Swagger UI

## Getting started

### 1. Start SurrealDB

```bash
# Install
curl -sSf https://install.surrealdb.com | sh

# Run (in-memory, data lost on restart)
surreal start --user root --pass secret memory

# Run with file-backed persistence (recommended)
surreal start --user root --pass secret surrealkv:${DB_PATH:-/opt/ullav/clann/data.db}
```

> **Note:** SurrealDB v3 uses the `surrealkv:` prefix for file storage, not `file:`.

Connect to an existing instance with the SQL REPL:

```bash
surreal sql \
  --endpoint ws://localhost:8000 \
  --username root --password secret \
  --namespace clann --database ancestry

-- useful commands inside the REPL:
INFO FOR DB;           -- list all tables, users, analyzers
INFO FOR TABLE person; -- list fields, indexes, events on a table
SELECT * FROM person;
SELECT * FROM family_tree;
```

### 2. Run the server

```bash
cargo run
```

The server listens on `http://localhost:3000` by default. Schema migration runs automatically on startup.

### 3. Browse the API

Open **http://localhost:3000/swagger-ui** for the interactive Swagger UI.

The OpenAPI JSON spec is also committed at [`openapi.json`](./openapi.json).

## Configuration

All configuration is via environment variables:

| Variable            | Default                    | Description                                                           |
|---------------------|----------------------------|-----------------------------------------------------------------------|
| `DB_URL`            | `ws://localhost:8000`      | SurrealDB WebSocket URL                                               |
| `DB_NAMESPACE`      | `clann`                    | SurrealDB namespace                                                   |
| `DB_DATABASE`       | `ancestry`                 | SurrealDB database                                                    |
| `DB_USERNAME`       | `root`                     | SurrealDB username                                                    |
| `DB_USERNAME_FILE`  | —                          | Path to file containing DB username (Docker secrets — takes priority) |
| `DB_PASSWORD`       | `secret`                   | SurrealDB password                                                    |
| `DB_PASSWORD_FILE`  | —                          | Path to file containing DB password (Docker secrets — takes priority) |
| `PORT`              | `3000`                     | HTTP listen port                                                      |
| `UPLOAD_DIR`        | `./uploads`                | Directory for person image files                                      |
| `DB_PATH`           | `/opt/ullav/clann/data.db` | SurrealDB data file path for persistent storage                       |
| `ENABLE_DOCS`       | `true`                     | Set to `false` to disable Swagger UI and OpenAPI spec endpoints       |

## API

### Family Trees

A family tree groups persons under a named container. Each tree has a unique `name` (slug), a `display_name`, an `owner`, and an optional `is_primary` flag. One tree per owner can be marked as primary.

| Method   | Path               | Description                                  |
|----------|--------------------|----------------------------------------------|
| `POST`   | `/api/trees`       | Create a family tree                         |
| `GET`    | `/api/trees`       | List all trees (optional `?owner=` filter)   |
| `GET`    | `/api/trees/{name}`| Get a tree by its unique name                |
| `DELETE` | `/api/trees/{name}`| Delete a tree and all its persons            |

**Tree fields:**

| Field          | Required | Description                                                      |
|----------------|----------|------------------------------------------------------------------|
| `name`         | yes      | Unique slug identifier (e.g. `"smith-family"`)                   |
| `display_name` | yes      | Human-readable label                                             |
| `owner`        | yes      | Username of the owner                                            |
| `is_primary`   | no       | `true` to mark as the owner's primary tree (defaults to `false`) |

Setting `is_primary: true` automatically clears `is_primary` on all other trees for the same owner.

Deleting a tree **cascade-deletes** all persons in that tree and their relationship edges.

```bash
# Create a tree
curl -X POST http://localhost:3000/api/trees \
  -H 'Content-Type: application/json' \
  -d '{"name": "murphy-family", "display_name": "The Murphy Family", "owner": "colin", "is_primary": true}'

# List trees for an owner
curl http://localhost:3000/api/trees?owner=colin

# Delete a tree (and all its persons)
curl -X DELETE http://localhost:3000/api/trees/murphy-family
```

### Persons

Persons must belong to a family tree. The `tree` field is required on creation and must refer to an existing tree name.

| Method   | Path                  | Description          |
|----------|-----------------------|----------------------|
| `POST`   | `/api/persons`        | Create a person      |
| `GET`    | `/api/persons`        | List all persons     |
| `GET`    | `/api/persons/{id}`   | Get a person         |
| `PUT`    | `/api/persons/{id}`   | Update a person      |
| `DELETE` | `/api/persons/{id}`   | Delete a person      |

**Person fields:**

| Field            | Required | Description                              |
|------------------|----------|------------------------------------------|
| `family_name`    | yes      |                                          |
| `first_name`     | yes      |                                          |
| `sex`            | yes      | `"Male"` or `"Female"`                   |
| `trees`          | yes      | Array of family tree names this person belongs to (at least one required) |
| `middle_name`    | no       |                                          |
| `date_of_birth`  | no       | ISO 8601 or free-form string             |
| `place_of_birth` | no       |                                          |
| `date_of_death`  | no       |                                          |
| `place_of_death` | no       |                                          |
| `nickname`       | no       |                                          |
| `username`       | no       |                                          |
| `email`          | no       |                                          |
| `verified`       | no       | Boolean, defaults to `false`             |
| `biography`      | no       | Free text                                |
| `created_by`     | no       | Identifier of the creator                |

`PUT` uses MERGE semantics — only supplied fields are updated. Omitted or `null` fields leave the existing value unchanged.

**Filtering:** `GET /api/persons` accepts `created_by` and `tree` query parameters:

```bash
curl http://localhost:3000/api/persons?tree=murphy-family
curl http://localhost:3000/api/persons?created_by=colin
curl "http://localhost:3000/api/persons?tree=murphy-family&created_by=colin"
```

### Images

| Method | Path                       | Description                              |
|--------|----------------------------|------------------------------------------|
| `POST` | `/api/persons/{id}/image`  | Upload a JPG or PNG image (max 2 MB)     |
| `GET`  | `/api/persons/{id}/image`  | Retrieve the person's image              |

Images are uploaded as `multipart/form-data` with a field named `image`.

```bash
curl -X POST http://localhost:3000/api/persons/{id}/image \
  -F "image=@photo.jpg;type=image/jpeg"
```

### Relationships

| Method    | Path                                                      | Description                            |
|-----------|-----------------------------------------------------------|----------------------------------------|
| `POST`    | `/api/persons/{id}/relationships`                         | Add a relationship                     |
| `GET`     | `/api/persons/{id}/relationships`                         | Get grouped relationships              |
| `DELETE`  | `/api/persons/{id}/relationships/{rel_type}/{related_id}` | Remove a relationship                  |
| `PATCH`   | `/api/persons/{id}/spouse-dates/{related_id}`             | Update spouse date fields              |
| `GET`     | `/api/persons/{id}/family-tree`                           | Family tree (2 generations up + children + spouses) |

Supported relationship types:

| `type`    | Extra fields                                         |
|-----------|------------------------------------------------------|
| `Father`  | —                                                    |
| `Mother`  | —                                                    |
| `Sibling` | `sibling_type`: `"Brother"` or `"Sister"` (required) |
| `Spouse`  | `spouse_from`, `spouse_to` (optional date strings)   |

```bash
# Add a father
curl -X POST http://localhost:3000/api/persons/{id}/relationships \
  -H 'Content-Type: application/json' \
  -d '{"type": "Father", "related_id": "person:{father_id}"}'

# Add a spouse with dates
curl -X POST http://localhost:3000/api/persons/{id}/relationships \
  -H 'Content-Type: application/json' \
  -d '{"type": "Spouse", "related_id": "person:{spouse_id}", "spouse_from": "1995-06-10"}'

# Update spouse dates
curl -X PATCH http://localhost:3000/api/persons/{id}/spouse-dates/person:{spouse_id} \
  -H 'Content-Type: application/json' \
  -d '{"spouse_from": "1995-06-10", "spouse_to": "2010-03-01"}'
```

## Production deployment

The server is distributed as a Docker image at `ghcr.io/ullav-dev/clann-server:latest`.

### Prerequisites

Create a `secrets/` directory with credentials (both files are gitignored):

```bash
mkdir secrets
echo -n "root" > secrets/db_username.txt
echo -n "your-strong-password" > secrets/db_password.txt
```

Copy `.env.prod` and adjust if needed (no secrets go here):

```
DB_URL=ws://surrealdb:8000
DB_NAMESPACE=clann
DB_DATABASE=ancestry
PORT=3000
UPLOAD_DIR=/app/uploads
```

### Start

```bash
docker compose up -d
```

Startup order: SurrealDB starts → `migrate` service applies any new `.surql` files → server starts. Applied migrations are tracked in the `schema_migration` table and skipped on subsequent deploys.

### Secrets

`DB_USERNAME` and `DB_PASSWORD` are read from Docker secrets mounted at `/run/secrets/`. The `_FILE` env var pattern is also supported for non-Docker environments:

```bash
DB_PASSWORD_FILE=/path/to/secret cargo run
```

## Development

```bash
cargo build       # Build
cargo test        # Run integration tests (no external DB required)
cargo clippy      # Lint
cargo fmt         # Format
```

Tests use an in-memory SurrealDB instance with an isolated namespace per test — no external database or services needed.
