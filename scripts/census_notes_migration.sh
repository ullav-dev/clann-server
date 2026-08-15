#!/bin/sh
# Read-only census for the tack-server notes migration (see the plan at
# /Users/colin/.claude/plans/linked-roaming-rabbit.md, "Phase 0").
#
# Every statement below is a plain SELECT -- nothing here writes, updates,
# defines, or deletes anything. Safe to run directly against production.
#
# Usage:
#   DB_USERNAME=... DB_PASSWORD=... BASE_URL=https://<prod-surrealdb-host> \
#     ./census_notes_migration.sh > census_output.json
#
# Then feed census_output.json to census_notes_migration.py (same directory)
# to compute the actual counts this migration's Phase 0 gate needs:
#   - notes with an unresolvable tree slug (references a tree name that
#     doesn't exist in family_tree)
#   - notes with trees=[] but is_shared=true
#   - notes whose trees resolve to more than one distinct team_id
#   - notes with is_shared=true where every resolved tree has team_id=NONE
#
# Deliberately two dumb SELECT dumps + local analysis, not one clever
# SurrealQL query -- this project's own MCP server hit real SurrealQL
# quirks combining string functions with `??` in a single expression
# (see clann-server's search_persons tool), so this avoids relying on
# any non-trivial per-array-element join happening inside SurrealDB
# itself. All resolution logic is in the plain, auditable Python script.

set -e

BASE_URL="${BASE_URL:?set BASE_URL, e.g. https://surrealdb.internal.ullav.com}"
DB_USERNAME="${DB_USERNAME:?set DB_USERNAME}"
DB_PASSWORD="${DB_PASSWORD:?set DB_PASSWORD}"
NS="${DB_NAMESPACE:-clann}"
DB="${DB_DATABASE:-ancestry}"

sql() {
    curl -sf -X POST "$BASE_URL/sql" \
        -u "$DB_USERNAME:$DB_PASSWORD" \
        -H "Surreal-NS: $NS" \
        -H "Surreal-DB: $DB" \
        -H "Content-Type: text/plain" \
        --data-raw "$1"
}

# Every family_tree: name (what research_note.trees[] actually stores) and
# its team_id, if any.
trees=$(sql "SELECT name, team_id FROM family_tree;")

# Every top-level research_note (parent_id IS NONE -- replies are excluded,
# they don't carry their own trees/is_shared meaningfully for this census).
notes=$(sql "SELECT id, title, created_by, created_at, trees, is_shared FROM research_note WHERE parent_id IS NONE;")

printf '{"trees": %s, "notes": %s}\n' "$trees" "$notes"
