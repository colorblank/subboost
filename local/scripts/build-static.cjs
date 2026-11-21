const { spawnSync } = require("node:child_process");
const path = require("node:path");

const localDir = path.resolve(__dirname, "..");
const nextBin = path.resolve(localDir, "..", "node_modules", "next", "dist", "bin", "next");

const result = spawnSync(process.execPath, [nextBin, "build", "--webpack"], {
  cwd: localDir,
  stdio: "inherit",
  env: {
    ...process.env,
    SUBBOOST_STATIC_EXPORT: "1",
  },
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
