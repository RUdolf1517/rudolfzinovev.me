# syntax=docker/dockerfile:1
# Two-stage build: the build stage needs a network for the two crates (axum +
# tokio), the runtime image carries one binary, a SQLite file and nothing else.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev sqlite-dev openssl-dev openssl-libs-static zlib-dev
WORKDIR /src
# No Cargo.lock in the repository: cargo resolves the tree during the build.
COPY Cargo.toml ./
COPY src ./src
COPY assets ./assets
RUN cargo build --release && strip target/release/rudolfzinovev

FROM alpine:3.20
# libssl is linked directly now: the GitHub client speaks TLS without crates.
RUN adduser -D -H -u 10001 rz && apk add --no-cache sqlite-libs zlib libcrypto3 libssl3
COPY --from=build /src/target/release/rudolfzinovev /usr/local/bin/rudolfzinovev
USER 10001:10001
ENV RZ_BIND=0.0.0.0:8080 \
    RZ_DATA=/data \
    RZ_OWNER=owner \
    RZ_GZIP=1 \
    RZ_SEED=1
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://127.0.0.1:8080/healthz || exit 1
ENTRYPOINT ["rudolfzinovev", "serve"]
