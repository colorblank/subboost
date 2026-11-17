import path from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

function runScenario(mode: "success" | "health-failure" | "restore-failure") {
  return spawnSync("bash", ["-s"], {
    cwd: path.resolve(__dirname, "../.."), encoding: "utf8", timeout: 30_000,
    env: { ...process.env, LC_ALL: "C.UTF-8" },
    input: `
      set -Eeuo pipefail
      home="$(mktemp -d)"
      trap 'rm -rf "$home"' EXIT
      export TMPDIR="$home" SUBBOOST_SCRIPT_SOURCE_ONLY=1 SUBBOOST_HOME="$home" SUBBOOST_BIN="$home/bin/subboost"
      export SUBBOOST_DOCTOR_HEALTH_ATTEMPTS=1 SUBBOOST_DOCTOR_HEALTH_INTERVAL_SECONDS=0
      mkdir -p "$home/release" "$home/bin" "$home/data"
      printf '%s' '{"image":"new-app","backendImage":"new-backend","composeUrl":"compose.yml","managerUrl":"manager"}' > "$home/release/release.json"
      printf 'services:\n  app: {}\n  backend: {}\n' > "$home/release/compose.yml"
      printf '#!/usr/bin/env bash\necho candidate-manager\n' > "$home/release/manager"
      printf '#!/usr/bin/env bash\necho old-manager\n' > "$SUBBOOST_BIN"
      cat > "$home/.env" <<ENV
SUBBOOST_RELEASE_URL=file://$home/release/release.json
SUBBOOST_IMAGE=old-app
SUBBOOST_BACKEND_IMAGE=old-backend
APP_URL=http://127.0.0.1:31000
SUBBOOST_PORT=31000
ENV
      cp "$home/release/compose.yml" "$home/docker-compose.yml"
      cp config/subboost.toml "$home/subboost.toml"
      sqlite3 "$home/data/subboost.sqlite3" "CREATE TABLE records(value); INSERT INTO records VALUES ('old');"
      printf '%032d' 1 > "$home/data/subboost.key"
      cp "$home/data/subboost.key" "$home/original.key"
      source local/scripts/subboost.sh
      sudo_do() { "$@"; }
      docker_log="$home/docker.log"
      deployment_state="$home/deployment-state"
      writer_state="$home/writer-state"
      printf 'old\n' > "$deployment_state"
      printf 'running\n' > "$writer_state"
      docker() {
        printf '%s\n' "$*" >> "$docker_log"
        case "$1" in
          info) return 0 ;;
          inspect)
            case "$*" in
              *'.State.Status'*) cat "$writer_state" ;;
              *'.State.Health'*) printf 'healthy\n' ;;
              *'backend-id') printf 'sha256:old-backend\n' ;;
              *) printf 'sha256:old-app\n' ;;
            esac ;;
          compose)
            case "$*" in
              *' config --services') printf 'app\nbackend\n' ;;
              *' config --format json') printf '%s\n' '{"services":{"app":{"ports":[{"target":3000,"published":"31000","protocol":"tcp"}]},"backend":{}}}' ;;
              *' ps -q app') printf 'app-id\n' ;;
              *' ps -q backend') printf 'backend-id\n' ;;
              *' stop app backend') printf 'exited\n' > "$writer_state" ;;
              *'candidate-compose.yml'*' up -d backend app')
                printf 'candidate\n' > "$deployment_state"
                printf 'running\n' > "$writer_state"
                sqlite3 "$home/data/subboost.sqlite3" "UPDATE records SET value='candidate';"
                ;;
              *'old-compose.yml'*' up -d backend app')
                [ "$(sqlite3 "$home/data/subboost.sqlite3" 'SELECT value FROM records;')" = old ] || return 45
                printf 'old\n' > "$deployment_state"
                printf 'running\n' > "$writer_state"
                ;;
              *' run --rm '*)
                script="\${!#}"
                if [ '${mode}' = restore-failure ] && [[ "$script" = *'stage='* ]]; then cat >/dev/null; return 27; fi
                sh -ec "\${script//\\/data/$home/data}"
                ;;
            esac ;;
        esac
      }
      curl() { [ '${mode}' = success ] || [ "$(cat "$deployment_state")" = old ]; }
      if output="$(update_cmd 2>&1)"; then status=0; else status=$?; fi
      printf 'status=%s\n%s\n' "$status" "$output"
      printf 'data=%s\nwriters=%s\n' "$(sqlite3 "$home/data/subboost.sqlite3" 'SELECT value FROM records;')" "$(cat "$writer_state")"
      cmp "$home/data/subboost.key" "$home/original.key"
      [ -n "$(find "$BACKUP_DIR" -name '*.tar' -print -quit)" ]
      if [ '${mode}' = success ]; then
        [ "$status" = 0 ]
        grep -qx 'SUBBOOST_IMAGE=new-app' "$ENV_FILE"
        grep -qx 'SUBBOOST_BACKEND_IMAGE=new-backend' "$ENV_FILE"
      else
        [ "$status" != 0 ]
        grep -qx 'SUBBOOST_IMAGE=old-app' "$ENV_FILE"
        grep -qx 'SUBBOOST_BACKEND_IMAGE=old-backend' "$ENV_FILE"
      fi
      cat "$docker_log"
    `,
  });
}

describe("SQLite deployment update lifecycle", () => {
  it("activates both images and metadata only after a healthy startup", () => {
    const result = runScenario("success");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("data=candidate\nwriters=running");
  });
  it("restores SQLite and both previous images after a candidate health failure", () => {
    const result = runScenario("health-failure");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("data=old\nwriters=running");
    expect(result.stdout).toMatch(/old-compose\.yml.* up -d backend app/);
  });
  it("keeps writers stopped and the snapshot available when restoration fails", () => {
    const result = runScenario("restore-failure");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("data=candidate\nwriters=exited");
    expect(result.stdout).not.toMatch(/old-compose\.yml.* up -d backend app/);
  });
  it("preserves existing volumes when installation fails", () => {
    const result = spawnSync("bash", ["-s"], {
      cwd: path.resolve(__dirname, "../.."), encoding: "utf8", timeout: 30_000,
      input: `
        set -Eeuo pipefail
        home="$(mktemp -d)"; trap 'rm -rf "$home"' EXIT
        export SUBBOOST_SCRIPT_SOURCE_ONLY=1 SUBBOOST_HOME="$home"
        source local/scripts/install.sh
        printf preserve > "$home/existing-volume"
        docker_cmd() { case "$*" in *'volume rm'*|*' down -v'*) rm "$home/existing-volume" ;; esac; }
        cleanup_failed_install
        [ "$(cat "$home/existing-volume")" = preserve ]
      `,
    });
    expect(result.status, result.stdout + result.stderr).toBe(0);
  });
});
