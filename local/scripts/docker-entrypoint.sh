#!/bin/sh
set -eu

PUID="${PUID:-1000}"
PGID="${PGID:-1000}"

exec_as_user() {
  if [ "$(id -u)" = "0" ] && command -v setpriv >/dev/null 2>&1; then
    exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups "$@"
  else
    exec "$@"
  fi
}

prepare_data_dir() {
  mkdir -p /data 2>/dev/null || true
  if [ -d /data/subboost.toml ]; then
    rmdir /data/subboost.toml 2>/dev/null || true
  fi
  if [ ! -f /data/subboost.toml ]; then
    if [ -f /etc/subboost/subboost.toml ]; then
      cp /etc/subboost/subboost.toml /data/subboost.toml 2>/dev/null || true
    elif [ -f /usr/share/subboost/subboost.toml ]; then
      cp /usr/share/subboost/subboost.toml /data/subboost.toml 2>/dev/null || true
    fi
  fi
  if [ "$(id -u)" = "0" ]; then
    chown -R "$PUID:$PGID" /data 2>/dev/null || true
  fi
}

patch_frontend_backend_url() {
  target_url="${SUBBOOST_BACKEND_URL:-}"
  if [ -z "$target_url" ] || [ ! -d /app/local ]; then
    return 0
  fi
  if [ "$(id -u)" = "0" ]; then
    chown -R "$PUID:$PGID" /app/local/.next /app/local/server.js 2>/dev/null || true
  fi
  if command -v node >/dev/null 2>&1; then
    node -e '
      const fs = require("node:fs");
      const target = (process.env.SUBBOOST_BACKEND_URL || "").replace(/\/+$/, "");
      if (!target || target === "http://backend:3002") process.exit(0);
      for (const file of [
        "/app/local/server.js",
        "/app/local/.next/routes-manifest.json",
        "/app/local/.next/required-server-files.json",
      ]) {
        if (!fs.existsSync(file)) continue;
        const raw = fs.readFileSync(file, "utf8");
        if (raw.includes("http://backend:3002")) {
          fs.writeFileSync(file, raw.split("http://backend:3002").join(target), "utf8");
        }
      }
    ' 2>/dev/null || true
  fi
}

resolve_settings_path() {
  if [ -n "${SUBBOOST_CONFIG:-}" ] && [ -f "${SUBBOOST_CONFIG}" ]; then
    printf '%s' "${SUBBOOST_CONFIG}"
    return 0
  fi
  if [ -f /etc/subboost/subboost.toml ] && [ -f /usr/share/subboost/subboost.toml ]; then
    if ! cmp -s /etc/subboost/subboost.toml /usr/share/subboost/subboost.toml; then
      printf '%s' "/etc/subboost/subboost.toml"
      return 0
    fi
  fi
  if [ -f /data/subboost.toml ]; then
    printf '%s' "/data/subboost.toml"
  elif [ -f /etc/subboost/subboost.toml ]; then
    printf '%s' "/etc/subboost/subboost.toml"
  else
    printf '%s' "/usr/share/subboost/subboost.toml"
  fi
}

if [ "${1:-}" = "aio" ] || [ "${SUBBOOST_MODE:-}" = "aio" ]; then
  prepare_data_dir
  SETTINGS_PATH="$(resolve_settings_path)"
  DB_PATH="${SUBBOOST_DATABASE:-/data/subboost.sqlite3}"
  LISTEN_ADDR="${SUBBOOST_LISTEN:-0.0.0.0:${PORT:-3000}}"

  exec_as_user /usr/local/bin/subboost \
    --settings "$SETTINGS_PATH" \
    --database "$DB_PATH" \
    serve \
    --listen "$LISTEN_ADDR"
fi

if [ "${1#-}" != "${1:-}" ]; then
  set -- /usr/local/bin/subboost "$@"
fi

case "${1:-}" in
  subboost|/usr/local/bin/subboost)
    prepare_data_dir
    SETTINGS_PATH="$(resolve_settings_path)"
    export SUBBOOST_CONFIG="${SUBBOOST_CONFIG:-$SETTINGS_PATH}"
    exec_as_user "$@"
    ;;
  node|/usr/local/bin/node)
    patch_frontend_backend_url
    exec "$@"
    ;;
  *)
    exec "$@"
    ;;
esac
