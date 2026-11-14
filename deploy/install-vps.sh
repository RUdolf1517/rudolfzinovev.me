#!/usr/bin/env bash
# Idempotent deployment for a clean Debian 12 / Ubuntu 22.04+ VPS.
# Creates a locked-down `rz` user, builds the release binary with the system
# toolchain, installs the systemd unit and enables Caddy for automatic TLS.
#
#   sudo ./deploy/install-vps.sh
#
# Re-running is safe: the binary is rebuilt, the database is never touched.
set -euo pipefail

APP_USER=rz
APP_DIR=/srv/rudolfzinovev
SRC_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

log() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }

if [[ $EUID -ne 0 ]]; then
  echo "run as root (sudo ./deploy/install-vps.sh)" >&2
  exit 1
fi

log "installing build + runtime packages"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y --no-install-recommends \
  build-essential pkg-config ca-certificates curl sqlite3 \
  libsqlite3-dev libssl-dev zlib1g-dev

if ! command -v cargo >/dev/null 2>&1; then
  log "installing Rust toolchain (rustup, minimal profile)"
  RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  ln -sf /usr/local/cargo/bin/cargo /usr/local/bin/cargo
  ln -sf /usr/local/cargo/bin/rustc /usr/local/bin/rustc
fi

log "creating system user ${APP_USER}"
id -u "$APP_USER" >/dev/null 2>&1 || useradd --system --home "$APP_DIR" --shell /usr/sbin/nologin "$APP_USER"
install -d -o "$APP_USER" -g "$APP_USER" -m 0750 "$APP_DIR" "$APP_DIR/data"

log "building release binary"
( cd "$SRC_DIR" && cargo build --release --locked 2>/dev/null || cargo build --release )
install -o root -g root -m 0755 "$SRC_DIR/target/release/rudolfzinovev" "$APP_DIR/rudolfzinovev"

if [[ ! -f "$APP_DIR/.env" ]]; then
  log "writing $APP_DIR/.env (generating owner password)"
  OWNER_PW=$(head -c 18 /dev/urandom | base64 | tr -d '/+=' | head -c 20)
  cat >"$APP_DIR/.env" <<EOF
RZ_BIND=127.0.0.1:8080
RZ_DATA=$APP_DIR/data
RZ_OWNER=owner
RZ_PASSWORD=$OWNER_PW
RZ_WORKERS=2
RZ_GZIP=1
RZ_SEED=1
EOF
  chmod 0600 "$APP_DIR/.env"
  chown root:root "$APP_DIR/.env"
  printf '\n\033[1;32mowner password (shown once):\033[0m %s\n\n' "$OWNER_PW"
else
  log "keeping existing $APP_DIR/.env"
fi

log "installing systemd unit"
install -m 0644 "$SRC_DIR/deploy/rudolfzinovev.service" /etc/systemd/system/rudolfzinovev.service
systemctl daemon-reload
systemctl enable --now rudolfzinovev
systemctl restart rudolfzinovev

log "waiting for the app on 127.0.0.1:8080"
for _ in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:8080/healthz >/dev/null 2>&1; then break; fi
  sleep 0.5
done
curl -fsS http://127.0.0.1:8080/healthz && echo

# --- reverse proxy ---------------------------------------------------------
if command -v caddy >/dev/null 2>&1; then
  log "updating Caddyfile"
  install -m 0644 "$SRC_DIR/deploy/Caddyfile" /etc/caddy/Caddyfile
  systemctl reload caddy || systemctl restart caddy
else
  log "Caddy is not installed — install it, then: cp deploy/Caddyfile /etc/caddy/Caddyfile"
fi

log "nightly SQLite backup (03:20 UTC, 7 day rotation)"
cat >/etc/cron.d/rudolfzinovev-backup <<EOF
20 3 * * * $APP_USER /usr/bin/sqlite3 $APP_DIR/data/site.sqlite ".backup '$APP_DIR/data/backup.sqlite'" && /usr/bin/find $APP_DIR/data -name 'backup.sqlite*' -mtime +7 -delete
EOF
chmod 0644 /etc/cron.d/rudolfzinovev-backup

log "done — https://rudolfzinovev.me (status: systemctl status rudolfzinovev)"
