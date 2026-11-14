# syntax=docker/dockerfile:1
# Two-stage build: the runtime image carries a ~1 MB static binary, a SQLite
# file and nothing else (no libc from a distro, no package manager).
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev sqlite-dev openssl-dev zlib-dev
WORKDIR /src
COPY Cargo.toml ./
COPY src ./src
COPY assets ./assets
RUN cargo build --release && strip target/release/rudolfzinovev

FROM alpine:3.20
RUN adduser -D -H -u 10001 rz && apk add --no-cache sqlite-libs libgcc
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
