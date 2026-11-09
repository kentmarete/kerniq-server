# tasks-api

A small [Kerniq Server](https://kerniq-server.com/) service that stores tasks in an
in-memory map and lists them. Task routes are protected by an `x-api-key` header.

## Run

```sh
API_KEY=secret kerniq dev          # or: API_KEY=secret cargo run
```

`API_KEY` is required. The server listens on `127.0.0.1:3000` by default
(override with `KERNIQ_HOST` / `KERNIQ_PORT`).

## Endpoints

| Method | Path            | Auth        | Description            |
|--------|-----------------|-------------|------------------------|
| GET    | `/health`       | public      | Health check           |
| POST   | `/api/v1/tasks` | `x-api-key` | Create a task (201)    |
| GET    | `/api/v1/tasks` | `x-api-key` | List all tasks         |
| GET    | `/api/v1/tasks/{id}` | `x-api-key` | Get one task (404 `TASK_NOT_FOUND` if missing) |

```sh
curl -X POST localhost:3000/api/v1/tasks \
  -H 'x-api-key: secret' -H 'content-type: application/json' \
  -d '{"title":"Buy milk"}'

curl localhost:3000/api/v1/tasks -H 'x-api-key: secret'

curl localhost:3000/api/v1/tasks/1 -H 'x-api-key: secret'
```

A missing or wrong key returns `401 {"error":{"code":"UNAUTHORIZED",...}}`.
An empty title returns `422` with code `EMPTY_TITLE`.

## Check

```sh
kerniq check && kerniq test
```
