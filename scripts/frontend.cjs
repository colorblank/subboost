const { spawn } = require("node:child_process");
const path = require("node:path");
const settings = require("../config/frontend.json");

const mode = process.argv[2];
if (mode !== "dev") throw new Error("Usage: node scripts/frontend.cjs dev");
const child = spawn(process.execPath, [require.resolve("next/dist/bin/next"), "dev", "--turbopack", "--hostname", settings.development.hostname, "--port", String(process.env.PORT || settings.development.port)], {
  cwd: path.resolve(__dirname, "../local"),
  stdio: "inherit",
});
let stopping = false;
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => { stopping = true; child.kill(signal); });
}
child.on("error", (error) => { console.error(error.message); process.exitCode = 1; });
child.on("exit", (code) => { process.exitCode = stopping ? 0 : (code ?? 1); });
