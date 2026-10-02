import { spawnSync } from "node:child_process";
import { rmSync } from "node:fs";

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, { stdio: "inherit", env, shell: process.platform === "win32" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

const release = process.argv.includes("--release");
rmSync(new URL("../out/", import.meta.url), { recursive: true, force: true });
if (process.platform === "darwin") {
  run("npm", ["run", release ? "build:native:release" : "build:native:debug"]);
}
run("npm", ["run", "build:remote"], release ? { ...process.env, NODE_ENV: "production" } : process.env);
