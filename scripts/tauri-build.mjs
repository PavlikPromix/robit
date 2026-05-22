import { spawn } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const tauriScript = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const targetDir = join(root, "src-tauri", "target-codex");

const child = spawn(process.execPath, [tauriScript, "build", ...process.argv.slice(2)], {
  cwd: root,
  env: {
    ...process.env,
    CARGO_TARGET_DIR: targetDir
  },
  stdio: "inherit"
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});
