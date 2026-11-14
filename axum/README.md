# axum build — how to compile and run

This directory is a **second backend** for the same site. It reuses the whole
zero-crate core (SQLite FFI, OpenSSL FFI, templates, Markdown, i18n, models)
and replaces only the HTTP layer with axum 0.8 + tokio + tower.

It exists because the same site can be served by two transports, and the
decision between them should be made on numbers, not taste.

## Why it is not part of the default build

The build sandbox this was written in has **no access to crates.io**: TLS to
`index.crates.io` and `static.crates.io` is dropped (`SSL_ERROR_SYSCALL`).
GitHub is reachable, and git dependencies work as long as they have no
transitive crates — axum does, so it cannot be fetched here.

The code was written against the pin in `Cargo.toml` and reviewed by hand, but
**it has not been compiled or tested**. Treat the first `cargo build` as the
real review.

## Build and test

```sh
cargo build -p rudolfzinovev-axum          # debug
cargo build -p rudolfzinovev-axum --release
cargo test  -p rudolfzinovev-axum          # unit tests in util.rs and state.rs
```

The default binary (`rudolfzinovev`) keeps building with **no dependencies at
all**, so nothing here can break the existing deployment.

## Run

```sh
RZ_DATA=./data ./target/release/rudolfzinovev-axum \
  --bind 0.0.0.0:8080 --owner owner --password 'your-password' --workers 4
```

Same CLI as the zero-crate binary: `serve`, `seed`, `hash-password`,
`--bind`, `--data`, `--owner`, `--password`, `--workers`, `--no-gzip`,
`--seed`; same `RZ_*` environment variables; same `data/site.sqlite`.
Point it at an existing database — the schema is identical.

## What is identical, on purpose

| Concern | Implementation |
|---|---|
| Language | URL prefix (`/`, `/en`), never a cookie |
| CSRF | `Origin`/`Referer` + `Sec-Fetch-Site` on every POST, one-time token on the login form |
| Sessions | `rz_session` cookie, `HttpOnly; SameSite=Strict`, SHA-256 of the token in SQLite |
| Rate limits | 6 login attempts / 10 min, 240 writes / min |
| CSP | hash-pinned inline script, `frame-ancestors 'none'`, no `unsafe-inline` for scripts |
| Caching | `ETag` over the uncompressed body, `304` revalidation, gzip for text types |
| Timing | `Server-Timing: render;dur=…` header, body stays byte-stable |
| Telemetry | GET of public pages only, referrer reduced to host, visitor id is a day-scoped hash |

## What actually changes

- 616 lines of hand-written HTTP/1.1 (parse, keep-alive, chunked, thread pool)
  are gone; hyper owns them.
- Routing is a table of `route("/notes/{slug}", get(note))` instead of a
  `match` on strings with manual `strip_prefix`.
- `--workers` sets the tokio worker-thread pool.
- The response pipeline (ETag, gzip, security headers, telemetry) is one
  function, `finish`/`Handler::done`, so there is a single place to audit.

## First things to check when it compiles

1. `GET /` returns 200 and the `Content-Security-Policy` header contains
   `sha256-` followed by 44 base64 characters (see `util::script_csp_hash`).
2. `GET /` again with `If-None-Match` from the first response returns 304.
3. `GET /healthz` returns 200 before the full router is hit — it is registered
   in `main.rs`, not in `handlers.rs`.
4. The login flow: `GET /login` sets `rz_csrf`, `POST /login` with that token
   redirects 303 to `/cabinet`, and `/cabinet` renders.
5. `POST /cabinet/articles/new` with `Origin` matching `Host` creates an entry;
   without that header it must answer 403.
