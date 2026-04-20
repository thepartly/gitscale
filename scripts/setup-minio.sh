#!/usr/bin/env bash
# Create the default bucket used by e2e tests.
# Requires: mc (MinIO Client) — comes pre-installed in the minio image,
#           or install via `brew install minio/stable/mc` / download binary.
#
# Usage:
#   ./scripts/setup-minio.sh              # defaults: localhost:9000
#   MINIO_ENDPOINT=host:9000 ./scripts/setup-minio.sh

set -euo pipefail

ENDPOINT="${MINIO_ENDPOINT:-localhost:9000}"
ALIAS="local"
BUCKET="gitscale"

echo "→ Configuring mc alias '${ALIAS}' → http://${ENDPOINT}"
mc alias set "${ALIAS}" "http://${ENDPOINT}" minioadmin minioadmin

echo "→ Creating bucket '${BUCKET}' (if not exists)"
mc mb --ignore-existing "${ALIAS}/${BUCKET}"

echo "✓ MinIO ready — bucket s3://${BUCKET}"
