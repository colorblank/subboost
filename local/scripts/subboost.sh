#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

DEFAULT_HOME="/opt/subboost"
DEFAULT_STABLE_RELEASE_URL="https://github.com/SubBoost/subboost/releases/latest/download/release.json"
DEFAULT_BACKUP_RETENTION_COUNT="10"
SUBBOOST_HOME="${SUBBOOST_HOME:-$DEFAULT_HOME}"
ENV_FILE="$SUBBOOST_HOME/.env"
COMPOSE_FILE="$SUBBOOST_HOME/docker-compose.yml"
BACKUP_DIR="$SUBBOOST_HOME/backups"
TMP_DIR=""

say() { printf '%s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
is_root() { [ "$(id -u)" = "0" ]; }
same_content() { [ "$(cat -- "$1" 2>/dev/null; printf x)" = "$(cat -- "$2" 2>/dev/null; printf x)" ]; }

sudo_do() {
  if is_root; then "$@"; else sudo "$@"; fi
}

prepare_private_directory() {
  local directory="$1"
  sudo_do mkdir -p "$directory"
  sudo_do chmod 700 "$directory"
  if ! is_root; then sudo_do chown "$(id -u):$(id -g)" "$directory"; fi
}

install_secret_file() {
  local source="$1"
  local destination="$2"
  sudo_do install -m 600 "$source" "$destination"
  if ! is_root; then
    sudo_do chown "$(id -u):$(id -g)" "$destination"
  fi
}

docker_runner() {
  if docker info >/dev/null 2>&1; then
    printf 'docker\n'
    return 0
  fi
  if ! is_root && command -v sudo >/dev/null 2>&1 && sudo docker info >/dev/null 2>&1; then
    printf 'sudo docker\n'
    return 0
  fi
  printf 'docker\n'
}

ensure_docker_runner() {
  if [ -z "${DOCKER_RUNNER:-}" ]; then
    DOCKER_RUNNER="$(docker_runner)"
  fi
}

docker_cmd() {
  ensure_docker_runner
  if [ "$DOCKER_RUNNER" = "sudo docker" ]; then sudo docker "$@"; else docker "$@"; fi
}

compose() { compose_files "$ENV_FILE" "$COMPOSE_FILE" "$@"; }

compose_files() {
  local env_file="$1"
  local compose_file="$2"
  shift 2
  [ -f "$compose_file" ] || die "Missing $compose_file"
  [ -f "$env_file" ] || die "Missing $env_file"
  (cd "$SUBBOOST_HOME" && docker_cmd compose --project-directory "$SUBBOOST_HOME" --env-file "$env_file" -f "$compose_file" "$@")
}

compose_files_with_images() {
  local image="$1" backend_image="$2"
  shift 2
  SUBBOOST_IMAGE="$image" SUBBOOST_CANDIDATE_IMAGE="$image" SUBBOOST_BACKEND_IMAGE="$backend_image" SUBBOOST_CANDIDATE_BACKEND_IMAGE="$backend_image" compose_files "$@"
}

load_env() {
  local TMP_DIR="$TMP_DIR" SUBBOOST_SCRIPT_SOURCE_ONLY="${SUBBOOST_SCRIPT_SOURCE_ONLY:-0}"
  [ -f "$ENV_FILE" ] || die "Missing $ENV_FILE"
  set -a
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  set +a
}

temp_directory_identity() { stat -c '%d:%i' -- "$1" 2>/dev/null || stat -f '%d:%i' "$1" 2>/dev/null; }
prepare_temp_directory() {
  if [ -z "$TMP_DIR" ]; then
    TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/subboost-manager.XXXXXXXXXX")" || die "Cannot create private temporary directory."
    # Freeze the path and identity used by cleanup, even if variables change later.
    local identity quoted_dir quoted_identity; identity="$(temp_directory_identity "$TMP_DIR")" || die "Cannot identify temporary directory: $TMP_DIR"
    if [ "${SUBBOOST_SCRIPT_SOURCE_ONLY:-0}" != "1" ]; then printf -v quoted_dir '%q' "$TMP_DIR"; printf -v quoted_identity '%q' "$identity"; trap "[ -d $quoted_dir ] && [ ! -L $quoted_dir ] && [ \"\$(temp_directory_identity $quoted_dir)\" = $quoted_identity ] && rm -rf -- $quoted_dir" EXIT; fi
  fi
}
validate_compose_binding() {
  local bind="${1#[}" port="$2"; bind="${bind%]}"
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; s=json.load(sys.stdin)["services"]; a=s["app"]; p=a.get("ports",[]); sys.exit(not (all(v.get("network_mode")!="host" and (k=="app" or not v.get("ports")) for k,v in s.items()) and len(p)==1 and p[0].get("host_ip","0.0.0.0" if sys.argv[1]=="0.0.0.0" else "")==sys.argv[1] and str(p[0].get("target"))=="3000" and str(p[0].get("published"))==sys.argv[2] and p[0].get("protocol","tcp")=="tcp"))' "$bind" "$port"
  elif command -v node >/dev/null 2>&1; then
    node -e 'const s=JSON.parse(require("fs").readFileSync(0,"utf8")).services,p=s.app.ports; process.exit(Object.entries(s).every(([k,v])=>v.network_mode!=="host"&&(k==="app"||!v.ports||Array.isArray(v.ports)&&v.ports.length===0))&&Array.isArray(p)&&p.length===1&&(p[0].host_ip===undefined?(process.argv[1]==="0.0.0.0"?"0.0.0.0":""):p[0].host_ip)===process.argv[1]&&String(p[0].target)==="3000"&&String(p[0].published)===process.argv[2]&&(p[0].protocol===undefined?"tcp":p[0].protocol)==="tcp"?0:1)' "$bind" "$port"
  elif command -v jq >/dev/null 2>&1; then
    jq -e --arg bind "$bind" --arg port "$port" 'all(.services | to_entries[]; .value.network_mode!="host" and (.key=="app" or ((.value.ports // [])|length)==0)) and (.services.app.ports as $p | ($p|length)==1 and ($p[0] | if has("host_ip") then .host_ip elif $bind=="0.0.0.0" then "0.0.0.0" else "" end)==$bind and ($p[0].target|tostring)=="3000" and ($p[0].published|tostring)==$port and ($p[0] | if has("protocol") then .protocol else "tcp" end)=="tcp")' >/dev/null
  else
    # shortcut: parser-free validation accepts canonical single-port Compose JSON; install Python, Node or jq for other layouts.
    awk -v bind="$bind" -v port="$port" '
      NR==1 && $0!="{" {bad=1} {last=$0}
      /^  "services": \{$/ {services=1;next} services && /^  \},?$/ {services=0} services && /^    "app": \{$/ {app=1;apps++;next} app && /^    \},?$/ {app=0}
      /"network_mode"[[:space:]]*:/ {bad=1}
      /"ports"[[:space:]]*:/ && !app && $0 !~ /"ports": \[\],?$/ {bad=1}
      app && /^      "ports": \[$/ {ports=1;lists++;next} ports && /^      \],?$/ {ports=0} ports && /^        \{$/ {items++}
      /"host_ip"[[:space:]]*:/ {if(!ports)bad=1; hosts++;v=$0;sub(/^.*:[[:space:]]*"/,"",v);sub(/",?[[:space:]]*$/,"",v);if(v!=bind)bad=1}
      ports && /^          "(target|published|protocol)":/ {v=$0;sub(/^.*:[[:space:]]*"?/,"",v);sub(/"?,?[[:space:]]*$/,"",v);if($0 ~ /"target":/){targets++;if(v!="3000")bad=1}else if($0 ~ /"published":/){published++;if(v!=port)bad=1}else if(v!="tcp")bad=1}
      END {if(last!="}" || apps!=1 || lists!=1 || items!=1 || targets!=1 || published!=1 || (bind=="0.0.0.0" ? hosts>1 : hosts!=(bind!="")) || bad)exit 1}' || { printf '%s\n' 'Install Python 3, Node.js, or jq to verify this Compose layout.' >&2; return 1; }
  fi
}
inspect_compose_binding() {
  local port="$1"
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; a=json.load(sys.stdin)["services"]["app"]; p=a.get("ports",[]); ok=a.get("network_mode")!="host" and len(p)==1 and str(p[0].get("target"))=="3000" and p[0].get("protocol","tcp")=="tcp" and str(p[0].get("published","")).isdigit(); sys.exit(1) if not ok else print(str(p[0].get("host_ip",""))+"|"+str(p[0]["published"]))'
  elif command -v node >/dev/null 2>&1; then
    node -e 'const a=JSON.parse(require("fs").readFileSync(0,"utf8")).services.app,p=a.ports;if(a.network_mode==="host"||!Array.isArray(p)||p.length!==1||String(p[0].target)!=="3000"||(p[0].protocol===undefined?"tcp":p[0].protocol)!=="tcp"||!/^\d+$/.test(String(p[0].published)))process.exit(1);process.stdout.write((p[0].host_ip??"")+"|"+p[0].published)'
  elif command -v jq >/dev/null 2>&1; then
    jq -er '.services.app as $a | $a.ports as $p | if $a.network_mode!="host" and ($p|length)==1 and ($p[0].target|tostring)=="3000" and ($p[0] | if has("protocol") then .protocol else "tcp" end)=="tcp" and ($p[0].published|tostring|test("^[0-9]+$")) then (($p[0].host_ip // "")+"|"+($p[0].published|tostring)) else error("unsupported binding") end'
  else
    validate_compose_binding "" "$port" && printf '|%s' "$port"
  fi
}

require_sqlite_compose() {
  local services
  services="$(compose config --services)" || return 1
  printf '%s\n' "$services" | grep -Fxq backend &&
    printf '%s\n' "$services" | grep -Fxq app &&
    ! printf '%s\n' "$services" | grep -Eq '^(db|cron)$' ||
    die "Legacy PostgreSQL deployment detected. Export it with the legacy manager and migrate into a separate SQLite installation before switching traffic. Existing containers and volumes were not changed."
}

download_to_temp() {
  local url="$1"
  local output="$2"
  case "$url" in
    file://*) cp "${url#file://}" "$output" ;;
    /*) cp "$url" "$output" ;;
    http://*|https://*) curl -fsSL "$url" -o "$output" ;;
    *) cp "$url" "$output" ;;
  esac
}

json_get() {
  local key="$1"
  local file="$2"
  if [ ! -s "$file" ]; then return 0; fi
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$key" "$file" <<'PY'
import json
import sys
key, path = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as handle:
    data = json.load(handle)
value = data.get(key, "")
print("" if value is None else str(value))
PY
    return 0
  fi
  sed -n "s/.*\"$key\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$file" | head -n 1
}

resolve_url() {
  local base="$1"
  local value="$2"
  [ -n "$value" ] || return 0
  case "$value" in
    http://*|https://*|file://*|/*) printf '%s\n' "$value" ;;
    *)
      case "$base" in
        file://*) printf 'file://%s/%s\n' "$(dirname "${base#file://}")" "$value" ;;
        http://*|https://*) printf '%s/%s\n' "${base%/*}" "$value" ;;
        *) printf '%s/%s\n' "$(dirname "$base")" "$value" ;;
      esac
      ;;
  esac
}

read_env_file() { sudo_do cat "$ENV_FILE"; }

write_env_value() {
  prepare_temp_directory
  local key="$1"
  local value="$2"
  local tmp="$TMP_DIR/env"
  read_env_file | awk -F= -v key="$key" '$1 != key { print }' > "$tmp"
  printf '%s=%s\n' "$key" "$value" >> "$tmp"
  install_secret_file "$tmp" "$ENV_FILE"
}

write_runtime_env_value() {
  local key="$1"
  local value="$2"
  write_env_value "$key" "$value"
  export "$key=$value"
}

is_official_fixed_release_url() {
  case "$1" in
    https://github.com/SubBoost/subboost/releases/download/v[0-9]*.[0-9]*.[0-9]*/release.json) return 0 ;;
    *) return 1 ;;
  esac
}

random_hex() {
  local bytes="$1"
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex "$bytes"
  else
    dd if=/dev/urandom bs="$bytes" count=1 2>/dev/null | od -An -tx1 | tr -d ' \n'
  fi
}

set_file_env_value() {
  local file="$1"
  local key="$2"
  local value="$3"
  local tmp="$TMP_DIR/env.$key"
  awk -F= -v key="$key" '$1 != key { print }' "$file" > "$tmp"
  printf '%s=%s\n' "$key" "$value" >> "$tmp"
  mv "$tmp" "$file"
}

atomic_install_file() {
  local source="$1"
  local destination="$2"
  local mode="$3"
  stage_install_file "$source" "$destination" "$mode"
  activate_staged_file "$destination"
}

stage_install_file() {
  local source="$1"
  local destination="$2"
  local mode="$3"
  local staged="${destination}.candidate.$$"
  sudo_do install -m "$mode" "$source" "$staged" || return 1
  if [ "$mode" = "600" ] && ! is_root; then
    sudo_do chown "$(id -u):$(id -g)" "$staged" || return 1
  fi
}

activate_staged_file() {
  local destination="$1"
  sudo_do mv -f "${destination}.candidate.$$" "$destination"
}

install_file_from_url() {
  prepare_temp_directory
  local url="$1"
  local destination="$2"
  local mode="$3"
  local tmp="$TMP_DIR/download"
  download_to_temp "$url" "$tmp"
  sudo_do install -m "$mode" "$tmp" "$destination"
}

require_stopped_backend() {
  local id state
  id="$(service_container_id backend)"
  [ -n "$id" ] || { say "Cannot identify backend container."; return 1; }
  state="$(container_state "$id")"
  case "$state" in exited|created) ;; *) say "Backend must be stopped before copying SQLite and its WAL."; return 1 ;; esac
}

create_verified_snapshot() {
  local output="$1" partial="${1}.partial"
  local -a result
  require_stopped_backend || return 1
  prepare_private_directory "$(dirname "$output")"
  sudo_do install -m 600 /dev/null "$partial" || return 1
  # No writers run while copying: keep database, WAL, SHM and encryption key together.
  compose run --rm --no-deps -T --user 0 --entrypoint sh backend -ec \
    'test -s /data/subboost.sqlite3; test -n "$(find /data -maxdepth 1 -name "*.key" -type f -size +0c -print -quit)"; test "$(sqlite3 -readonly /data/subboost.sqlite3 "PRAGMA quick_check;")" = ok; tar -C /data -cf - .' \
    | sudo_do tee "$partial" >/dev/null
  result=("${PIPESTATUS[@]}")
  if (( result[0] != 0 || result[1] != 0 )) || ! tar -tf "$partial" >/dev/null; then
    sudo_do rm -f "$partial"
    say "SQLite snapshot failed; no completed backup was written."
    return 1
  fi
  sudo_do mv "$partial" "$output"
}

restore_snapshot() {
  local snapshot="$1"
  require_stopped_backend || return 1
  # Validate in a staging directory before replacing any data from the failed update.
  sudo_do cat "$snapshot" | compose run --rm --no-deps -T --user 0 --entrypoint sh backend -ec '
    stage=$(mktemp -d /data/.restore.XXXXXXXX)
    trap '\''rm -rf "$stage"'\'' EXIT
    tar -xf - -C "$stage"
    test -s "$stage/subboost.sqlite3"
    test -n "$(find "$stage" -maxdepth 1 -name "*.key" -type f -size +0c -print -quit)"
    test "$(sqlite3 -readonly "$stage/subboost.sqlite3" "PRAGMA quick_check;")" = ok
    find /data -mindepth 1 -maxdepth 1 ! -path "$stage" -exec rm -rf -- {} +
    cp -a "$stage/." /data/
  '
}

port_number() {
  local value="$1"
  case "$value" in
    *:*) value="${value##*:}" ;;
  esac
  value="${value#[}"
  value="${value%]}"
  case "$value" in ''|*[!0-9]*) ;; *) value="${value#"${value%%[!0]*}"}"; [ -n "$value" ] || value=0 ;; esac
  printf '%s\n' "$value"
}

service_container_id() {
  compose ps -q "$1" 2>/dev/null | head -n 1 || true
}

container_state() {
  local container_id="$1"
  [ -n "$container_id" ] || return 0
  docker_cmd inspect -f '{{.State.Status}}' "$container_id" 2>/dev/null || true
}

container_health() {
  local container_id="$1"
  [ -n "$container_id" ] || return 0
  docker_cmd inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$container_id" 2>/dev/null || true
}

service_status_text() {
  local service="$1"
  local container_id state health
  container_id="$(service_container_id "$service")"
  if [ -z "$container_id" ]; then
    printf '未创建\n'
    return 0
  fi

  state="$(container_state "$container_id")"
  case "$state" in
    running)
      printf '运行中\n'
      ;;
    exited) printf '已停止\n' ;;
    restarting) printf '正在重启\n' ;;
    dead) printf '异常停止\n' ;;
    *) printf '%s\n' "${state:-未知}" ;;
  esac
}

health_status_text() {
  health_status_label "$(health_status_code)"
}

health_status_code() {
  local port base live_ok
  port="$(port_number "${SUBBOOST_PORT:-3000}")"
  base="http://127.0.0.1:$port"
  if ! command -v curl >/dev/null 2>&1; then
    printf 'curl-missing\n'
  else
    if curl -fsS "$base/api/health/live" >/dev/null 2>&1; then
      live_ok=1
    else
      live_ok=0
    fi

    if [ "$live_ok" = "1" ] && curl -fsS "$base/api/health/ready" >/dev/null 2>&1; then
      printf 'ok\n'
    elif [ "$live_ok" = "1" ]; then
      printf 'not-ready\n'
    else
      printf 'unhealthy\n'
    fi
  fi
}

health_status_label() {
  case "$1" in
    ok) printf '正常\n' ;;
    not-ready) printf '应用已启动，数据库未就绪\n' ;;
    curl-missing) printf '缺少 curl\n' ;;
    *) printf '异常\n' ;;
  esac
}

wait_for_health() {
  # Default max wait is about 30 seconds: 15 attempts with a 2-second interval.
  local attempts="${SUBBOOST_DOCTOR_HEALTH_ATTEMPTS:-15}"
  local interval="${SUBBOOST_DOCTOR_HEALTH_INTERVAL_SECONDS:-2}"
  local index status
  for index in $(seq 1 "$attempts"); do
    status="$(health_status_code)"
    if [ "$status" = "ok" ]; then
      return 0
    fi
    if [ "$index" != "$attempts" ]; then
      sleep "$interval"
    fi
  done
  return 1
}

doctor_health_failure_message() {
  local status="$1"
  case "$status" in
    not-ready) printf 'Health check failed: database is not ready.' ;;
    curl-missing) printf 'Health check failed: curl command is missing.' ;;
    *) printf 'Health check failed: app is not responding.' ;;
  esac
}

status_cmd() {
  load_env
  say "SubBoost 状态"
  say "访问地址: ${APP_URL:-未配置}"
  say "安装目录: $SUBBOOST_HOME"
  say ""
  say "服务状态:"
  say "前端: $(service_status_text app)"
  say "Rust API / SQLite / scheduler: $(service_status_text backend)"
  say ""
  say "健康检查: $(health_status_text)"
  say "备份目录: $BACKUP_DIR"
  say ""
  say "常用命令: subboost logs / subboost backup / subboost update / subboost restart / subboost doctor"
}

update_cmd() {
  load_env
  require_sqlite_compose
  prepare_temp_directory
  local release_url="${SUBBOOST_RELEASE_URL:-$DEFAULT_STABLE_RELEASE_URL}"
  local release_file="$TMP_DIR/release.json"
  local candidate_env="$TMP_DIR/candidate.env" candidate_compose="$TMP_DIR/candidate-compose.yml"
  local candidate_manager="$TMP_DIR/candidate-manager" old_env="$TMP_DIR/old.env"
  local old_compose="$TMP_DIR/old-compose.yml" old_manager="$TMP_DIR/old-manager"
  local snapshot="$BACKUP_DIR/update-rollback-$(date -u +%Y%m%dT%H%M%SZ).tar"
  local image backend_image compose_url manager_url old_app old_backend
  local binding port old_binding old_host old_port
  port="$(port_number "${SUBBOOST_PORT:?SUBBOOST_PORT is required}")"
  if is_official_fixed_release_url "$release_url"; then release_url="$DEFAULT_STABLE_RELEASE_URL"; fi
  if ! download_to_temp "$release_url" "$release_file"; then
    [ "${SUBBOOST_MANAGER_HANDOFF:-0}" != 1 ] || die "Release manifest unavailable; manager handoff aborted."
    die "Release manifest unavailable; the running installation was not changed."
  fi
  image="$(json_get image "$release_file")"
  backend_image="$(json_get backendImage "$release_file")"
  compose_url="$(resolve_url "$release_url" "$(json_get composeUrl "$release_file")")"
  manager_url="$(resolve_url "$release_url" "$(json_get managerUrl "$release_file")")"
  [ -n "$image" ] && [ -n "$backend_image" ] && [ -n "$compose_url" ] && [ -n "$manager_url" ] || die "Release manifest must name both images, Compose and manager."
  download_to_temp "$compose_url" "$candidate_compose"
  download_to_temp "$manager_url" "$candidate_manager"
  bash -n "$candidate_manager" || die "Candidate manager is invalid."
  if [ "${SUBBOOST_MANAGER_HANDOFF:-0}" = 1 ]; then
    same_content "$candidate_manager" "$0" || die "The release manager does not match the downloaded manager; update aborted."
  elif [ "${SUBBOOST_SCRIPT_SOURCE_ONLY:-0}" != 1 ] && ! same_content "$candidate_manager" "$0"; then
    say "Handing this update to the release manager..."
    SUBBOOST_MANAGER_HANDOFF=1 bash "$candidate_manager" update
    return
  fi
  read_env_file > "$candidate_env"
  cp "$candidate_env" "$old_env"
  cp "$COMPOSE_FILE" "$old_compose"
  if [ -f "${SUBBOOST_BIN:-/usr/local/bin/subboost}" ]; then sudo_do cp "${SUBBOOST_BIN:-/usr/local/bin/subboost}" "$old_manager"; fi
  old_app="$(docker_cmd inspect -f '{{.Image}}' "$(service_container_id app)")"
  old_backend="$(docker_cmd inspect -f '{{.Image}}' "$(service_container_id backend)")"
  [ -n "$old_app" ] && [ -n "$old_backend" ] || die "Cannot pin both current images for rollback."
  set_file_env_value "$candidate_env" SUBBOOST_IMAGE "$image"
  set_file_env_value "$candidate_env" SUBBOOST_BACKEND_IMAGE "$backend_image"
  set_file_env_value "$candidate_env" SUBBOOST_CANDIDATE_IMAGE "$image"
  set_file_env_value "$candidate_env" SUBBOOST_CANDIDATE_BACKEND_IMAGE "$backend_image"
  set_file_env_value "$candidate_env" SUBBOOST_RELEASE_URL "$release_url"
  old_binding="$(compose_files "$old_env" "$old_compose" config --format json | inspect_compose_binding "$port")" \
    || die "Cannot safely inspect the existing Compose binding."
  IFS='|' read -r old_host old_port <<< "$old_binding"
  [ "$old_port" -eq "$port" ] 2>/dev/null || die "Existing Compose publishes a different application port."
  binding="${SUBBOOST_BIND_ADDRESS:-$old_host}"; binding="${binding#[}"; binding="${binding%]}"
  if [ -n "${SUBBOOST_BIND_ADDRESS:-}" ] && [ "$old_host" != "$binding" ]; then die "Existing Compose uses a different bind address."; fi
  compose_files_with_images "$image" "$backend_image" "$candidate_env" "$candidate_compose" config --format json \
    | validate_compose_binding "$binding" "$port" || die "Candidate Compose changes the published binding or exposes an internal service."
  local services
  services="$(compose_files_with_images "$image" "$backend_image" "$candidate_env" "$candidate_compose" config --services)"
  printf '%s\n' "$services" | grep -Fxq backend && printf '%s\n' "$services" | grep -Fxq app \
    && ! printf '%s\n' "$services" | grep -Eq '^(db|cron)$' || die "Candidate is not a Rust/SQLite deployment."
  compose_files_with_images "$image" "$backend_image" "$candidate_env" "$candidate_compose" pull
  compose stop app backend || die "Cannot stop writers; update aborted."
  if ! create_verified_snapshot "$snapshot"; then
    compose up -d backend app || true
    die "Snapshot failed; update aborted without changing images or data."
  fi
  local error=""
  compose_files_with_images "$image" "$backend_image" "$candidate_env" "$candidate_compose" up -d backend app || error="candidate startup failed"
  if [ -z "$error" ] && ! wait_for_health; then error="candidate health check failed"; fi
  if [ -z "$error" ]; then
    atomic_install_file "$candidate_env" "$ENV_FILE" 600 || error="environment activation failed"
    atomic_install_file "$candidate_compose" "$COMPOSE_FILE" 644 || error="Compose activation failed"
    atomic_install_file "$candidate_manager" "${SUBBOOST_BIN:-/usr/local/bin/subboost}" 755 || error="manager activation failed"
  fi
  if [ -n "$error" ]; then
    say "Candidate update failed: $error"
    compose_files_with_images "$image" "$backend_image" "$candidate_env" "$candidate_compose" stop app backend || die "Cannot stop candidate writers; snapshot preserved at $snapshot"
    atomic_install_file "$old_env" "$ENV_FILE" 600 && atomic_install_file "$old_compose" "$COMPOSE_FILE" 644 \
      || die "Cannot restore metadata; keep writers stopped. Snapshot: $snapshot"
    if [ -f "$old_manager" ]; then atomic_install_file "$old_manager" "${SUBBOOST_BIN:-/usr/local/bin/subboost}" 755 || die "Cannot restore manager; snapshot: $snapshot"; fi
    restore_snapshot "$snapshot" || die "Rollback data restore failed; keep writers stopped. Snapshot: $snapshot"
    compose_files_with_images "$old_app" "$old_backend" "$old_env" "$old_compose" up -d backend app \
      || die "Old images failed to start; snapshot: $snapshot"
    wait_for_health || die "Old data and images restored, but health failed. Snapshot: $snapshot"
    say "Previous version restored successfully."
    return 1
  fi
  status_cmd
}

logs_cmd() {
  compose logs -f --tail="${SUBBOOST_LOG_TAIL:-200}" "$@"
}

backup_cmd() {
  load_env
  require_sqlite_compose
  prepare_private_directory "$BACKUP_DIR"
  local stamp snapshot env_out retention="${SUBBOOST_BACKUP_RETENTION_COUNT:-$DEFAULT_BACKUP_RETENTION_COUNT}"
  [[ "$retention" =~ ^[0-9]+$ ]] && (( retention>0 )) || die "Backup retention must be a positive integer."
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  snapshot="$BACKUP_DIR/subboost-$stamp.tar"
  env_out="$BACKUP_DIR/subboost-$stamp.env"
  compose stop app backend || die "Cannot stop writers for a consistent SQLite backup."
  if ! create_verified_snapshot "$snapshot"; then
    compose up -d backend app || true
    die "Backup failed; no completed snapshot was written."
  fi
  if ! sudo_do install -m 600 "$ENV_FILE" "$env_out"; then
    compose up -d backend app || true
    die "Data snapshot exists, but environment backup failed: $snapshot"
  fi
  local config_source="${SUBBOOST_CONFIG_FILE:-$SUBBOOST_HOME/subboost.toml}"
  if ! sudo_do install -m 600 "$config_source" "${snapshot%.tar}.toml"; then
    compose up -d backend app || true
    die "Data snapshot exists, but runtime configuration backup failed: $snapshot"
  fi
  compose up -d backend app || die "Backup saved, but services failed to restart: $snapshot"
  local -a snapshots
  local index
  shopt -s nullglob
  snapshots=("$BACKUP_DIR"/subboost-*.tar)
  shopt -u nullglob
  for ((index=0;index<${#snapshots[@]}-retention;index++)); do
    sudo_do rm -f -- "${snapshots[$index]}" "${snapshots[$index]%.tar}.env" "${snapshots[$index]%.tar}.toml"
  done
  say "Private SQLite snapshot (database, WAL and key): $snapshot"
  say "Private environment backup: $env_out"
}

restart_cmd() {
  load_env
  require_sqlite_compose
  compose up -d backend app
  compose up -d --no-deps --force-recreate backend app
  status_cmd
}

doctor_cmd() {
  command -v docker >/dev/null 2>&1 || die "docker command is missing"
  docker_cmd compose version >/dev/null 2>&1 || die "docker compose plugin is missing"
  [ -d "$SUBBOOST_HOME" ] || die "Missing $SUBBOOST_HOME"
  [ -f "$ENV_FILE" ] || die "Missing $ENV_FILE"
  [ -f "$COMPOSE_FILE" ] || die "Missing $COMPOSE_FILE"
  load_env
  require_sqlite_compose
  for key in SUBBOOST_IMAGE SUBBOOST_BACKEND_IMAGE APP_URL SUBBOOST_PORT; do
    grep -q "^$key=." "$ENV_FILE" || die "Missing $key in $ENV_FILE"
  done
  compose config >/dev/null
  if ! wait_for_health; then
    local health_status
    health_status="$(health_status_code)"
    status_cmd
    die "$(doctor_health_failure_message "$health_status")"
  fi
  status_cmd
  say "Doctor: OK"
}

menu_cmd() {
  say "SubBoost"
  say "1) Status"
  say "2) Update"
  say "3) Logs"
  say "4) Backup"
  say "5) Restart"
  say "6) Doctor"
  say "0) Exit"
  local choice=""
  if [ -t 0 ]; then
    printf 'Choose: '
    IFS= read -r choice || choice=""
  fi
  case "$choice" in
    1) status_cmd ;;
    2) update_cmd ;;
    3) logs_cmd ;;
    4) backup_cmd ;;
    5) restart_cmd ;;
    6) doctor_cmd ;;
    0|"") exit 0 ;;
    *) die "Unknown menu choice: $choice" ;;
  esac
}

main() {
  local command="${1:-menu}"
  if [ "$#" -gt 0 ]; then shift; fi
  case "$command" in
    menu) menu_cmd ;;
    status) status_cmd ;;
    update) update_cmd ;;
    logs) logs_cmd "$@" ;;
    backup) backup_cmd ;;
    restart) restart_cmd ;;
    doctor) doctor_cmd ;;
    *) die "Unknown command: $command" ;;
  esac
}

if [ "${SUBBOOST_SCRIPT_SOURCE_ONLY:-0}" != "1" ]; then
  trap 'exit 129' HUP; trap 'exit 130' INT; trap 'exit 143' TERM
  DOCKER_RUNNER=""
  main "$@"
fi
