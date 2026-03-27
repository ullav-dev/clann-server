#!/bin/sh
set -e

DB_PASSWORD=$(cat /run/secrets/db_password)
DB_USERNAME=$(cat /run/secrets/db_username)
WS_CONN="ws://surrealdb:8000"
HTTP_CONN="http://surrealdb:8000"
NS="${DB_NAMESPACE:-clann}"
DB="${DB_DATABASE:-ancestry}"

sql() {
    echo "$1" | surreal sql \
        --endpoint "$WS_CONN" \
        --user "$DB_USERNAME" \
        --pass "$DB_PASSWORD" \
        --ns "$NS" \
        --db "$DB" \
        2>/dev/null
}

echo "Ensuring migrations tracking table exists..."
sql "
DEFINE TABLE IF NOT EXISTS schema_migration SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS filename   ON TABLE schema_migration TYPE string;
DEFINE FIELD IF NOT EXISTS applied_at ON TABLE schema_migration TYPE datetime VALUE time::now() READONLY;
DEFINE INDEX IF NOT EXISTS idx_migration_filename ON TABLE schema_migration COLUMNS filename UNIQUE;
"

for file in /migrations/*.surql; do
    filename=$(basename "$file")

    result=$(sql "SELECT filename FROM schema_migration WHERE filename = '$filename' LIMIT 1;")

    if echo "$result" | grep -q "\"$filename\""; then
        echo "Skipping $filename (already applied)"
    else
        echo "Applying $filename..."
        surreal import \
            --endpoint "$HTTP_CONN" \
            --user "$DB_USERNAME" \
            --pass "$DB_PASSWORD" \
            --ns "$NS" \
            --db "$DB" \
            "$file"
        sql "CREATE schema_migration SET filename = '$filename', applied_at = time::now();"
        echo "Applied $filename"
    fi
done

echo "Migrations complete."
