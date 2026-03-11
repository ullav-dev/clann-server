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

# Run (in-memory)
surreal start --user root --pass secret memory

# Or with file-backed storage
surreal start --user root --pass secret file:./data.db
```

### 2. Run the server

```bash
cargo run
```

The server listens on `http://localhost:3000` by default.

### 3. Browse the API

Open **http://localhost:3000/swagger-ui** for the interactive Swagger UI.

## Configuration

All configuration is via environment variables:

| Variable       | Default               | Description                      |
|----------------|-----------------------|----------------------------------|
| `DB_URL`       | `ws://localhost:8000` | SurrealDB WebSocket URL          |
| `DB_NAMESPACE` | `clann`               | SurrealDB namespace              |
| `DB_DATABASE`  | `ancestry`            | SurrealDB database               |
| `DB_USERNAME`  | `root`                | SurrealDB username               |
| `DB_PASSWORD`  | `secret`              | SurrealDB password               |
| `PORT`         | `3000`                | HTTP listen port                 |
| `UPLOAD_DIR`   | `./uploads`           | Directory for person image files |

## API

### Persons

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
| `middle_name`    | no       |                                          |
| `date_of_birth`  | no       | ISO 8601 or free-form string             |
| `place_of_birth` | no       |                                          |
| `date_of_death`  | no       |                                          |
| `place_of_death` | no       |                                          |
| `nickname`       | no       |                                          |
| `username`       | no       |                                          |
| `email`          | no       |                                          |

`PUT` uses MERGE semantics — only supplied fields are updated. Omitted or `null` fields leave the existing value unchanged.

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

## Development

```bash
cargo build       # Build
cargo test        # Run integration tests
cargo clippy      # Lint
cargo fmt         # Format
```

Tests use an in-memory SurrealDB instance with an isolated namespace per test — no external database or services needed.
