import { copyFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const source = join(root, "src-tauri", "target-codex", "release", "robit-helper.exe");
const targetDir = join(root, "src-tauri", "binaries");
const target = join(targetDir, "robit-helper-x86_64-pc-windows-msvc.exe");

mkdirSync(targetDir, { recursive: true });

if (existsSync(source)) {
  copyFileSync(source, target);
  console.log(`Prepared helper sidecar: ${target}`);
} else if (!existsSync(target)) {
  writeFileSync(target, "");
  console.log(`Prepared placeholder helper sidecar: ${target}`);
} else {
  console.log(`Helper sidecar already exists: ${target}`);
}
