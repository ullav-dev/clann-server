#!/bin/sh
set -e

DB_PASSWORD=$(cat /run/secrets/db_password)
DB_USERNAME=$(cat /run/secrets/db_username)
BASE_URL="http://surrealdb:8000"
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
        curl -sf -X POST "$BASE_URL/sql" \
            -u "$DB_USERNAME:$DB_PASSWORD" \
            -H "Surreal-NS: $NS" \
            -H "Surreal-DB: $DB" \
            -H "Content-Type: text/plain" \
            --data-binary "@$file"
        sql "CREATE schema_migration SET filename = '$filename', applied_at = time::now();"
        echo "Applied $filename"
    fi
done

echo "Migrations complete."
