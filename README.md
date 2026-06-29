# clann-server

A REST API server for managing ancestry and family tree data

## Stack

- **[Axum](https://github.com/tokio-rs/axum)** — async web framework
- **[SurrealDB](https://surrealdb.com)** — graph database for storing persons and relationships
- **[utoipa](https://github.com/juhaku/utoipa)** — OpenAPI 3 spec generation with Swagger UI

## Getting started

### 1. Start SurrealDB

```bash
# Install
curl -sSf https://install.surrealdb.com | sh

# Run with persistent file-backed storage (always use this locally)
surreal start --bind 0.0.0.0:8000 --username root --password secret surrealkv:/opt/ullav/clann/data.db
```

> **Note:** SurrealDB v3 uses the `surrealkv:` prefix for file storage, not `file:`. Always use the persistent database path `/opt/ullav/clann/data.db` for local development — never run with `memory` as data will be lost on restart.

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
| `JWT_SECRET`        | —                          | When set, all requests must include a valid `Authorization: Bearer <jwt>`. Omit to disable auth enforcement. |

## Authentication

All API routes require a valid JWT when `JWT_SECRET` is configured. Pass the token issued by `ullav-user-management` in the `Authorization` header:

```
Authorization: Bearer <token>
```

The token must include a `subscriptions.clann` claim to unlock plan tiers. Missing or inactive subscriptions default to the `individual` tier.

### Subscription tiers

| Tier           | Max trees | Max persons |
|----------------|-----------|-------------|
| `individual`   | 2         | 100         |
| `family`       | 10        | 1,000       |
| `professional` | unlimited | unlimited   |
| `enterprise`   | unlimited | unlimited   |

Exceeding a limit returns `403 Forbidden`. When `JWT_SECRET` is not set, auth is skipped (useful for local dev and integration tests).

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

### Relationship pedigree

Parent and sibling relationships carry an optional `pedigree` field that records the nature of the connection.

**Parent edges** (`has_father`, `has_mother`):

| `pedigree` | Meaning |
|------------|---------|
| `birth`    | Biological parent (default) |
| `adopted`  | Adoptive parent |

**Sibling edges** (`has_sibling`):

| `pedigree` | Meaning |
|------------|---------|
| `birth`    | Full sibling — shares both parents |
| `half`     | Half sibling — shares exactly one biological parent |
| `adopted`  | Sibling through an adoptive family |

**Design rationale:** Step-parent and step-sibling relationships are intentionally not stored as edges. A step-parent is already present in the tree as the spouse of a biological parent — recording a separate step-parent edge would be redundant and genealogically misleading. Step-sibling relationships are likewise derivable from spouse edges and are not genealogical facts. Foster relationships carry no genealogical information and are excluded entirely.

Half siblings arise when two people share exactly one biological parent. This is a genuine genealogical fact and is modelled explicitly. When you add a biological parent to a person, Clann checks each of that parent's other children to determine whether the relationship is full (`birth`) or half — a sibling who shares both parents is `birth`; one who shares only this parent is `half`. The `via_parent_id` field on sibling edges records which parent they share, so half-sibling families can be correctly grouped in the tree view.

### Life Events

Life events record significant occurrences in a person's life (Birth, Death, Marriage, Graduation, Military service, etc.).

| Method   | Path                                  | Description                                      |
|----------|---------------------------------------|--------------------------------------------------|
| `POST`   | `/api/persons/{id}/life-events`       | Create a life event for a person                 |
| `GET`    | `/api/persons/{id}/life-events`       | List all life events for a person (ordered by date ASC) |
| `GET`    | `/api/life-events/{event_id}`         | Get a single life event                          |
| `PUT`    | `/api/life-events/{event_id}`         | Replace a life event (MERGE semantics)           |
| `DELETE` | `/api/life-events/{event_id}`         | Delete a life event                              |

**Life event fields:**

| Field          | Required | Description                                                               |
|----------------|----------|---------------------------------------------------------------------------|
| `name`         | yes      | Short title for the event                                                 |
| `event_type`   | yes      | One of `Birth`, `Death`, `Marriage`, `Graduation`, `Military`, `Immigration`, `Emigration`, `Other` |
| `date`         | no       | ISO 8601 or free-form date string                                         |
| `description`  | no       | Brief summary                                                             |
| `story`        | no       | Long-form narrative                                                        |
| `verified`     | no       | Boolean, defaults to `false`                                              |
| `source_link`  | no       | URL to an external source                                                 |
| `source_image` | no       | Path or URL to a supporting image                                         |
| `source_doc`   | no       | Path or URL to a supporting document                                      |
| `created_by`   | no       | Identifier of the creator                                                 |

```bash
# Create a birth event
curl -X POST http://localhost:3000/api/persons/{id}/life-events \
  -H 'Content-Type: application/json' \
  -d '{"name": "Born in Dublin", "event_type": "Birth", "date": "1920-03-15"}'

# List life events for a person
curl http://localhost:3000/api/persons/{id}/life-events

# Update a life event
curl -X PUT http://localhost:3000/api/life-events/{event_id} \
  -H 'Content-Type: application/json' \
  -d '{"name": "Born in Dublin", "event_type": "Birth", "date": "1920-03-15", "description": "Born at the Rotunda Hospital"}'
```

### AI Settings

Stores per-user AI provider configuration for the Research Assistant. The webapp encrypts the API key before sending — the server stores opaque blobs and never sees the key in plain text.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/ai-settings` | Get AI settings for the authenticated user |
| `PUT` | `/api/ai-settings` | Create or replace settings (upsert by username) |
| `DELETE` | `/api/ai-settings` | Remove AI settings |

### Research Folders

User-scoped containers for grouping research notes. Deleting a folder atomically unfiles all its notes before removing the folder record.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/folders?created_by=<user>` | List folders (ordered by name) |
| `POST` | `/api/folders` | Create a folder |
| `PATCH` | `/api/folders/{id}` | Rename a folder |
| `DELETE` | `/api/folders/{id}` | Delete folder and unfile its notes |
| `PATCH` | `/api/notes/{id}/folder` | Move a note to a folder (`folder_id: null` to unfile) |

### Chat Sessions

Persists AI Research Assistant conversations, scoped per user and family tree. Sessions are created automatically by the webapp on the first message and updated after each AI response.

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/chat/sessions?created_by=<user>&tree=<name>` | List sessions (newest first) |
| `POST` | `/api/chat/sessions` | Create a session |
| `DELETE` | `/api/chat/sessions/{id}` | Delete session and all its messages |
| `GET` | `/api/chat/sessions/{id}/messages` | List messages in order |
| `POST` | `/api/chat/sessions/{id}/messages` | Append a message (role: `user` or `assistant`) |

## MCP (Model Context Protocol) endpoint

Clann exposes an MCP endpoint at `/mcp` that allows AI assistants (Claude Desktop, Claude Code) to query and act on genealogy data through natural conversation.

### Authentication

The `/mcp` endpoint is protected by OAuth2 (RS256, audience-bound). On first connection the client discovers the Authorization Server automatically via the `WWW-Authenticate` header (RFC 9728) and opens a browser login page. After authenticating, tokens are cached and refreshed silently.

Required environment variables:

| Variable | Description |
|---|---|
| `CLANN_MCP_CANONICAL_URI` | Public URL of this server (e.g. `https://clann.example.com`) — used as the OAuth2 audience |
| `OAUTH2_ISSUER` | Issuer URL of the `ullav-user-management` Authorization Server |
| `OAUTH2_JWKS_URL` | JWKS endpoint of the Authorization Server |

### Connecting from Claude Code

```bash
claude mcp add --transport http --scope user clann https://clann.example.com/mcp
```

### Available tools

| Tool | Description |
|---|---|
| `list_trees` | List all family trees owned by a given username |
| `search_persons` | Search for persons by name across trees |
| `get_person` | Get full details for a person (birth, death, life events, notes) |
| `get_family` | Get a person's relationships (parents, children, spouses, siblings) |

**Example interactions:**

```
"My Clann username is colin. What are the names of my parents?"
"Search for anyone named Murphy in my trees"
"Tell me everything you know about person proxy:abc123"
"Who are the siblings of John Manning?"
```

For repeated use, add your username and person proxy ID to `~/.claude/CLAUDE.md` so Claude can go straight to `get_family` without a search step each session.

### Privacy principles

The MCP layer enforces the same privacy model as the REST API:

- **No identity leakage before consent.** Tools never expose another tree owner's username, email, or any identifying information. If a potential match is found in another user's tree, only genealogical facts (name, dates) and an opaque person proxy ID are returned — never the owner.
- **Contact requests are one-way until accepted.** A user can initiate a contact request to the owner of a matched person, but no identity is revealed to either party until the request is accepted through the webapp.
- **Write tools require confirmation.** Tools that create or modify data describe what they are about to do before acting, giving you the opportunity to adjust or cancel.

### Extending the MCP — a worked example

This section shows how to add new MCP tools by walking through a real feature: finding potential duplicate persons and initiating contact with other tree owners.

#### Step 1 — identify the existing REST endpoints

The REST API already has everything needed:

| Endpoint | What it does |
|---|---|
| `GET /api/persons/{id}/find-duplicates` | Returns scored candidate matches across all trees |
| `POST /api/contact-requests` | Creates a contact request with a tree owner |
| `GET /api/contact-requests` | Lists the authenticated user's open contact requests |

#### Step 2 — design the MCP tools

Three tools map onto this workflow:

**`find_duplicates(person_id)`**
Calls `GET /api/persons/{id}/find-duplicates`. Returns a list of candidates with name, birth/death dates, similarity score, and an opaque `person_proxy` ID. The other tree owner's identity is never included.

**`list_contact_requests()`**
Calls `GET /api/contact-requests`. Returns the authenticated user's open, accepted, and ignored requests — without exposing the other party's identity until the request is accepted.

**`create_contact_request(person_proxy_ids, message)`**
Calls `POST /api/contact-requests`. Claude composes a suggested message based on the genealogical context (shared name, dates, etc.) and presents it for your review before sending. You can edit the message or cancel entirely.

#### Step 3 — the natural conversation flow

```
You:   "Find any duplicates that might be me"

Claude: calls find_duplicates(my_person_proxy_id)
        → "Found 2 possible matches:
            1. John Manning, b. 1972 Dublin — 84% match
            2. John Manning, b. 1973 Cork   — 61% match"

You:   "Send a contact request to the 84% match"

Claude: "I'll send this message to the owner of match 1:
         'Hello, I noticed our family trees may share a person —
          John Manning born in Dublin around 1972. I'd be happy to
          compare notes. Would you be open to connecting?'
         Shall I send it, or would you like to change the wording?"

You:   "Change 'compare notes' to 'share research'"

Claude: calls create_contact_request with the updated message
        → "Request sent."
```

#### Step 4 — implement in `src/mcp/server.rs`

Add parameter structs, tool implementations, and register them in `tool_router!` following the same pattern as the existing tools. The REST handlers are already tested — the MCP layer is purely a translation from natural language to structured API calls.

Key points to keep in mind when adding tools:

- Return only what Claude needs to answer the question — omit internal IDs and owner information that isn't relevant to the user.
- For write tools, include enough context in the return value for Claude to confirm the action back to the user in plain language.
- The token claims (`username`, `sub`) are validated by the middleware before any tool is called — tools can trust the caller is authenticated.

## Production deployment

> **macOS:** Uses [Colima](https://github.com/abiosoft/colima) instead of Docker Desktop. Run `colima start` before any Docker commands.

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
ENABLE_DOCS=false
DB_URL=ws://surrealdb:8000
DB_NAMESPACE=clann
DB_DATABASE=ancestry
PORT=3001
UPLOAD_DIR=/app/uploads
```

### Start

```bash
docker compose -f docker-compose-prod.yaml up -d
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
