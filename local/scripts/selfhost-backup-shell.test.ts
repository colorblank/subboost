import { spawnSync } from "node:child_process";
import path from "node:path";
import { describe, expect, it } from "vitest";

function scenario(action: "roundtrip" | "invalid-restore" | "retention" | "snapshot", failure = "none") {
  return spawnSync("bash", ["-s"], {
    cwd: path.resolve(__dirname, "../.."), encoding: "utf8", timeout: 30_000,
    env: { ...process.env, LC_ALL: "C.UTF-8" },
    input: `
      set -Eeuo pipefail
      home="$(mktemp -d)"
      trap 'rm -rf "$home"' EXIT
      export SUBBOOST_SCRIPT_SOURCE_ONLY=1 SUBBOOST_HOME="$home"
      source local/scripts/subboost.sh
      data="$home/data"; mkdir "$data"
      sqlite3 "$data/subboost.sqlite3" "CREATE TABLE records(value); INSERT INTO records VALUES ('before');"
      printf '%032d' 1 > "$data/subboost.key"
      cp "$data/subboost.key" "$home/original.key"
      printf 'SUBBOOST_BACKUP_RETENTION_COUNT=2\n' > "$ENV_FILE"
      printf '[application]\n' > "$home/subboost.toml"
      backend_state=exited
      failure='${failure}'
      service_container_id() { printf 'backend-id\n'; }
      container_state() { printf '%s\n' "$backend_state"; }
      sudo_do() {
        if [ "$failure" = write ] && [ "$1" = tee ]; then return 37; fi
        "$@"
      }
      compose() {
        case "$*" in
          'config --services') printf 'app\nbackend\n' ;;
          'stop app backend') backend_state=exited ;;
          'up -d backend app') backend_state=running ;;
          'run '*)
            [ "$failure" != producer ] || return 23
            if [ "$failure" = invalid-tar ]; then printf broken; return 0; fi
            script="\${!#}"
            sh -ec "\${script//\\/data/$data}"
            ;;
          *) return 99 ;;
        esac
      }
      case "$failure" in
        missing-key) rm "$data/subboost.key" ;;
        corrupt) printf corrupt > "$data/subboost.sqlite3" ;;
        running) backend_state=running ;;
      esac
      snapshot="$home/snapshot.tar"
      if [ '${action}' = retention ]; then
        prepare_private_directory "$BACKUP_DIR"
        for stamp in 20000101T000001Z 20000101T000002Z 20000101T000003Z; do
          for extension in tar env toml; do : > "$BACKUP_DIR/subboost-$stamp.$extension"; done
        done
        backend_state=running
        backup_cmd
        [ "$backend_state" = running ]
        [ ! -e "$BACKUP_DIR/subboost-20000101T000001Z.tar" ]
        [ "$(find "$BACKUP_DIR" -name '*.tar' | wc -l | tr -d ' ')" = 2 ]
        for archive in "$BACKUP_DIR"/*.tar; do
          [ -f "\${archive%.tar}.env" ] && [ -f "\${archive%.tar}.toml" ]
        done
        [ -z "$(find "$BACKUP_DIR" -type f ! -perm 600 -print -quit)" ]
        printf 'retention-and-private-config-ok\n'
      elif [ '${action}' = invalid-restore ]; then
        printf broken > "$snapshot"
        if restore_snapshot "$snapshot"; then exit 40; fi
        [ "$(sqlite3 "$data/subboost.sqlite3" 'SELECT value FROM records;')" = before ]
        cmp "$data/subboost.key" "$home/original.key"
        printf 'invalid-restore-preserved-data\n'
      else
        if create_verified_snapshot "$snapshot"; then saved=1; else saved=0; fi
        [ ! -e "$snapshot.partial" ]
        if [ "$failure" != none ]; then
          [ "$saved" = 0 ] && [ ! -e "$snapshot" ]
          printf 'incomplete-snapshot-rejected\n'
        else
          [ "$saved" = 1 ]
          tar -tf "$snapshot" | grep -q subboost.key
          sqlite3 "$data/subboost.sqlite3" "UPDATE records SET value='candidate';"
          printf '%032d' 2 > "$data/subboost.key"
          restore_snapshot "$snapshot"
          [ "$(sqlite3 "$data/subboost.sqlite3" 'SELECT value FROM records;')" = before ]
          cmp "$data/subboost.key" "$home/original.key"
          printf 'database-and-key-restored\n'
        fi
      fi
    `,
  });
}

describe("SQLite backup integrity", () => {
  it("restores the database and its matching encryption key", () => {
    const result = scenario("roundtrip");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("database-and-key-restored");
  });
  it.each(["missing-key", "corrupt", "running", "write", "producer", "invalid-tar"])("rejects %s without publishing a partial archive", (failure) => {
    const result = scenario("snapshot", failure);
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("incomplete-snapshot-rejected");
  });
  it("does not replace live data with an invalid archive", () => {
    const result = scenario("invalid-restore");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("invalid-restore-preserved-data");
  });
  it("restarts writers and prunes database, environment and runtime-policy backups together", () => {
    const result = scenario("retention");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(result.stdout).toContain("retention-and-private-config-ok");
  });
});
