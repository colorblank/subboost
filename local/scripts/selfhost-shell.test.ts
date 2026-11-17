import path from "node:path";
import { spawnSync } from "node:child_process";
import { describe, expect, it } from "vitest";

const publicRoot = path.resolve(__dirname, "../..");
const BASH_NON_INTERACTIVE_COMMAND = "exec \"$BASH\" -s";

function shellTimeout(defaultMs: number): number {
  return process.platform === "win32" ? 120_000 : defaultMs;
}

function runBash(script: string) {
  return spawnSync("bash", ["-lc", BASH_NON_INTERACTIVE_COMMAND], {
    cwd: publicRoot,
    encoding: "utf8",
    input: script,
    timeout: shellTimeout(30_000),
    detached: true,
    env: {
      ...process.env,
      LC_ALL: "C.UTF-8",
    },
  });
}

describe("self-host shell scripts", () => {
  it("preserves an explicit Docker config when Docker requires sudo", () => {
    const result = runBash(`
      set -Eeuo pipefail
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1
      source local/scripts/install.sh
      export DOCKER_CONFIG=/tmp/subboost-isolated-docker-config
      DOCKER_RUNNER="sudo docker"
      sudo() { printf 'sudo-call=%s\\n' "$*"; }
      docker_cmd info
    `);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain(
      "sudo-call=env DOCKER_CONFIG=/tmp/subboost-isolated-docker-config docker info",
    );
  });

  it("uses prompt defaults without /dev/tty errors in non-interactive mode", () => {
    const result = runBash(`
      set -Eeuo pipefail
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1
      source local/scripts/install.sh
      export SUBBOOST_ASSUME_YES=0
      value="$(prompt 'Question: ' 'default-value')"
      printf 'value=%s\\n' "$value"
    `);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain("value=default-value");
    expect(result.stderr).not.toContain("/dev/tty");
  });

  it("does not report Doctor OK when health checks fail", () => {
    const script = `
      set -Eeuo pipefail
      home="$(mktemp -d)"
      trap 'rm -rf "$home"' EXIT
      cat > "$home/.env" <<'ENV'
SUBBOOST_IMAGE=image
SUBBOOST_BACKEND_IMAGE=backend-image
APP_URL=http://127.0.0.1:31000
SUBBOOST_PORT=31000
ENV
      : > "$home/docker-compose.yml"
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1
      export SUBBOOST_HOME="$home"
      export SUBBOOST_DOCTOR_HEALTH_ATTEMPTS=1
      export SUBBOOST_DOCTOR_HEALTH_INTERVAL_SECONDS=0
      source local/scripts/subboost.sh
      docker() {
        if [ "$1" = "info" ]; then return 0; fi
        if [ "$1" = "compose" ]; then
          case "$*" in
            "compose version"*) return 0 ;;
            *" config --services") printf 'app\\nbackend\\n'; return 0 ;;
            *" config") return 0 ;;
            *" ps -q app") printf 'app-id\\n'; return 0 ;;
            *" ps -q backend") printf 'backend-id\\n'; return 0 ;;
          esac
        fi
        if [ "$1" = "inspect" ]; then
          case "$*" in
            *".State.Status"*) printf 'running\\n'; return 0 ;;
            *".State.Health"*) printf 'healthy\\n'; return 0 ;;
          esac
        fi
        return 0
      }
      curl() { return 1; }
      set +e
      output="$(doctor_cmd 2>&1)"
      status=$?
      set -e
      printf 'status=%s\\n%s\\n' "$status" "$output"
      [ "$status" -ne 0 ]
      case "$output" in *"Doctor: OK"*) exit 44 ;; esac
      case "$output" in *"健康检查: 异常"*) exit 0 ;; *) exit 45 ;; esac
    `;

    const result = runBash(script);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain("status=1");
    expect(result.stdout).toContain("健康检查: 异常");
    expect(result.stdout).not.toContain("Doctor: OK");
    expect(result.stdout).toContain("ERROR: Health check failed: app is not responding.");
  });

  it("reports Doctor OK only after health checks pass", () => {
    const script = `
      set -Eeuo pipefail
      home="$(mktemp -d)"
      trap 'rm -rf "$home"' EXIT
      cat > "$home/.env" <<'ENV'
SUBBOOST_IMAGE=image
SUBBOOST_BACKEND_IMAGE=backend-image
APP_URL=http://127.0.0.1:31000
SUBBOOST_PORT=31000
ENV
      : > "$home/docker-compose.yml"
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1
      export SUBBOOST_HOME="$home"
      export SUBBOOST_DOCTOR_HEALTH_ATTEMPTS=1
      export SUBBOOST_DOCTOR_HEALTH_INTERVAL_SECONDS=0
      source local/scripts/subboost.sh
      docker() {
        if [ "$1" = "info" ]; then return 0; fi
        if [ "$1" = "compose" ]; then
          case "$*" in
            "compose version"*) return 0 ;;
            *" config --services") printf 'app\\nbackend\\n'; return 0 ;;
            *" config") return 0 ;;
            *" ps -q app") printf 'app-id\\n'; return 0 ;;
            *" ps -q backend") printf 'backend-id\\n'; return 0 ;;
          esac
        fi
        if [ "$1" = "inspect" ]; then
          case "$*" in
            *".State.Status"*) printf 'running\\n'; return 0 ;;
            *".State.Health"*) printf 'healthy\\n'; return 0 ;;
          esac
        fi
        return 0
      }
      curl() { return 0; }
      doctor_cmd
    `;

    const result = runBash(script);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain("健康检查: 正常");
    expect(result.stdout).toContain("Doctor: OK");
  });

  it("loads SUBBOOST_PORT from .env before doctor health checks", () => {
    const script = `
      set -Eeuo pipefail
      home="$(mktemp -d)"
      trap 'rm -rf "$home"' EXIT
      cat > "$home/.env" <<'ENV'
SUBBOOST_IMAGE=image
SUBBOOST_BACKEND_IMAGE=backend-image
APP_URL=http://127.0.0.1:31041
SUBBOOST_PORT=31041
ENV
      : > "$home/docker-compose.yml"
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1
      export SUBBOOST_HOME="$home"
      export SUBBOOST_DOCTOR_HEALTH_ATTEMPTS=1
      export SUBBOOST_DOCTOR_HEALTH_INTERVAL_SECONDS=0
      unset SUBBOOST_PORT APP_URL
      source local/scripts/subboost.sh
      docker() {
        if [ "$1" = "info" ]; then return 0; fi
        if [ "$1" = "compose" ]; then
          case "$*" in
            "compose version"*) return 0 ;;
            *" config --services") printf 'app\\nbackend\\n'; return 0 ;;
            *" config") return 0 ;;
            *" ps -q app") printf 'app-id\\n'; return 0 ;;
            *" ps -q backend") printf 'backend-id\\n'; return 0 ;;
          esac
        fi
        if [ "$1" = "inspect" ]; then
          case "$*" in
            *".State.Status"*) printf 'running\\n'; return 0 ;;
            *".State.Health"*) printf 'healthy\\n'; return 0 ;;
          esac
        fi
        return 0
      }
      curl_urls="$home/curl-urls"
      : > "$curl_urls"
      curl() {
        printf '%s\\n' "$*" >> "$curl_urls"
        case "$*" in
          *"http://127.0.0.1:31041/api/health/"*) return 0 ;;
        esac
        return 1
      }
      doctor_cmd
      cat "$curl_urls"
    `;

    const result = runBash(script);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain("Doctor: OK");
    expect(result.stdout).toContain("http://127.0.0.1:31041/api/health/live");
    expect(result.stdout).toContain("http://127.0.0.1:31041/api/health/ready");
    expect(result.stdout).not.toContain("http://127.0.0.1:3000/api/health");
  });

  it("updates exact env keys without removing similarly prefixed names", () => {
    const script = `
      set -Eeuo pipefail
      home="$(mktemp -d)"
      trap 'rm -rf "$home"' EXIT
      export TMPDIR="$home"
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1
      export SUBBOOST_HOME="$home"
      source local/scripts/subboost.sh
      install_secret_file() {
        cp "$1" "$2"
      }
      read_env_file() {
        cat "$ENV_FILE"
      }
      cat > "$ENV_FILE" <<'ENV'
SUBBOOST_PORT_EXTRA=keep
SUBBOOST_PORT=3000
APP_URL=http://old.example
ENV
      write_env_value SUBBOOST_PORT 31000
      cat "$ENV_FILE"
    `;

    const result = runBash(script);

    expect(result.status).toBe(0);
    expect(result.stdout).toContain("SUBBOOST_PORT_EXTRA=keep");
    expect(result.stdout).toContain("SUBBOOST_PORT=31000");
    expect(result.stdout).not.toContain("SUBBOOST_PORT=3000");
  });

});
