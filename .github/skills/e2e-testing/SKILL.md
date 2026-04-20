---
name: e2e-testing
description: 'Run and write end-to-end tests for gitscale storage (MinIO/S3). USE WHEN: adding e2e tests, debugging metadata push/pull against MinIO, verifying S3 storage integration, running the docker-compose stack. DO NOT USE FOR: unit tests that mock storage.'
---

# End-to-End Testing with MinIO

## Prerequisites

- Docker (with compose plugin)
- Python 3.12+ with the project virtualenv activated
- `mc` (MinIO Client) — only needed for manual bucket setup

## Quick Start

```bash
# 1. Start MinIO
docker compose up -d

# 2. Wait for healthy
docker compose ps   # STATE should be "running (healthy)"

# 3. Run e2e tests
pytest tests/e2e/ -v

# 4. Tear down
docker compose down -v
```

## Architecture

```
docker compose up
  └─ minio (network_mode: host)
       ├─ S3 API  → localhost:9000
       └─ Console → localhost:9001
```

`network_mode: host` is used because the dev environment already runs
inside a container (devcontainer / remote SSH).  Regular port mapping
(`-p 9000:9000`) would map to the outer container, not your browser host.
With host networking the ports bind directly to the machine running Docker.

## MinIO Console (Web UI)

Open **http://localhost:9001** in a browser on the Docker host.

| Field    | Value       |
|----------|-------------|
| Username | `minioadmin`|
| Password | `minioadmin`|

From inside the dev container, if `localhost` doesn't resolve to the
Docker host, use the host's IP address or the special DNS name
`host.docker.internal` (Docker Desktop) / gateway IP.

## Environment Variables for Tests

The e2e test fixtures set these automatically:

| Variable               | Value                                                  |
|------------------------|--------------------------------------------------------|
| `AWS_ACCESS_KEY_ID`    | `minioadmin`                                           |
| `AWS_SECRET_ACCESS_KEY`| `minioadmin`                                           |
| `AWS_DEFAULT_REGION`   | `us-east-1`                                            |
| `MINIO_ENDPOINT`       | `localhost:9000`                                       |

## Writing New E2E Tests

1. Put test files in `tests/e2e/`.
2. All tests are auto-marked with `@pytest.mark.e2e` via `conftest.py`.
3. Use the `minio_storage_url` fixture for a pre-created, unique bucket URL.
4. Use the `s3_env` fixture if you only need the env vars set.

```python
def test_example(minio_storage_url: str) -> None:
    """minio_storage_url gives you a fresh bucket like
    http://localhost:9000/gitscale-<uuid>
    """
    from gitscale.storage import upload_metadata, download_metadata

    upload_metadata(minio_storage_url, "https://github.com/o/r.git", "main", {"k": 1})
    data = download_metadata(minio_storage_url, "https://github.com/o/r.git", "main")
    assert data == {"k": 1}
```

## Running Only E2E Tests

```bash
pytest tests/e2e/ -v                # all e2e
pytest tests/e2e/ -v -k push        # filter by name
pytest -m e2e -v                    # via marker
```

## Skipping E2E When MinIO Is Down

Tests skip automatically with a clear message if MinIO is unreachable.

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `Connection refused :9000` | `docker compose up -d` and wait for healthy |
| `Access Denied` | Check `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` |
| `NoSuchBucket` | The fixture creates buckets automatically; check MinIO logs |
| Console unreachable at `:9001` | Verify `network_mode: host` in compose and no port conflicts |
