const { spawn } = require("node:child_process");
const path = require("node:path");

const frontendSettings = require("../config/frontend.json");
const root = path.resolve(__dirname, "..");
const production = process.argv.includes("--production");
const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const binary = path.join(root, "target", "release", process.platform === "win32" ? "subboost.exe" : "subboost");
const backend = spawn(production ? binary : "cargo", production ? ["serve"] : ["run", "-p", "subboost", "--", "serve"], { cwd: root, stdio: "inherit" });
const frontend = spawn(npm, ["--workspace", "@subboost/local", "run", production ? "start" : "dev"], {
  cwd: root, stdio: "inherit",
  env: { ...process.env, PORT: process.env.PORT || String(frontendSettings.development.port), HOSTNAME: frontendSettings.development.hostname },
});
const children = [backend, frontend];
let stopping = false;
function stop(code) {
  if (stopping) return;
  stopping = true;
  process.exitCode = code;
  for (const child of children) if (child.exitCode === null && child.signalCode === null) child.kill("SIGINT");
}
for (const child of children) {
  child.on("error", (error) => { console.error(error.message); stop(1); });
  child.on("exit", (code, signal) => stop(signal ? 1 : (code ?? 1)));
}
process.on("SIGINT", () => stop(0));
process.on("SIGTERM", () => stop(0));
