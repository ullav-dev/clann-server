#!/bin/sh
set -e

SURREAL_PASS=$(cat /run/secrets/db_password)

exec surreal start \
    --user root \
    --pass "$SURREAL_PASS" \
    --bind 0.0.0.0:8000 \
    surrealkv:/data/clann.db
