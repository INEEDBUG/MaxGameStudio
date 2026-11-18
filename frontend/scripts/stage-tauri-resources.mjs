import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const frontendRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = join(frontendRoot, "..");
const destination = join(frontendRoot, "src-tauri", "bundle-resources");
const packageVersion = JSON.parse(readFileSync(join(frontendRoot, "package.json"), "utf8")).version;
const appVersion = process.env.CS2_INSIGHT_APP_VERSION?.trim() || packageVersion;

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(appVersion)) {
  throw new Error(`Invalid desktop resource version: ${appVersion}`);
}

function normalizedRelative(root, path) {
  return relative(root, path).replaceAll("\\", "/");
}

function commonSkip(rel) {
  const path = `/${rel.toLowerCase()}/`;
  return path.includes("/__pycache__/") || path.includes("/.pytest_cache/") || rel.toLowerCase().endsWith(".pyc");
}

function copyFiltered(name, filter) {
  const source = join(repoRoot, name);
  if (!existsSync(source)) throw new Error(`Missing bundle resource: ${source}`);
  const target = join(destination, name);
  cpSync(source, target, {
    recursive: true,
    filter(path) {
      const rel = normalizedRelative(source, path);
      return !rel || (!commonSkip(rel) && filter(rel));
    },
  });
}

rmSync(destination, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
mkdirSync(destination, { recursive: true });
writeFileSync(join(destination, ".gitkeep"), "");

copyFiltered("python", () => true);
copyFiltered("backend", (rel) => {
  const path = rel.toLowerCase();
  const first = path.split("/")[0];
  if (["dist", "logs", "scripts", "tests"].includes(first)) return false;
  if (path === "app/release_version.txt" || path === "app/league_lab.py" || path.startsWith("app/league_lab/")) return false;
  if (/\.db(?:-wal|-shm)?$/i.test(path) || path.endsWith(".exe")) return false;
  return !/^debug_.*\.py$/i.test(path);
});
writeFileSync(join(destination, "backend", "app", "release_version.txt"), `${appVersion}\n`);

// Ship deterministic bytecode for the bundled backend. The desktop runtime is
// read-only from Program Files on many machines and explicitly disables cache
// writes, so compiling here avoids reparsing the full FastAPI application on
// every launch.
const stagedPython = join(destination, "python", "python.exe");
const stagedBackend = join(destination, "backend");
const compileResult = spawnSync(
  stagedPython,
  [
    "-I",
    "-m",
    "compileall",
    "-q",
    "-j",
    "1",
    "--invalidation-mode",
    "unchecked-hash",
    stagedBackend,
  ],
  { encoding: "utf8", windowsHide: true },
);
if (compileResult.status !== 0) {
  throw new Error(`Failed to precompile desktop backend: ${compileResult.stderr || compileResult.stdout}`);
}
copyFiltered("pov", () => true);
const bundledDataFiles = new Set([
  "basic.ini",
  "cs2-insight.config.example.json",
  "lite_cut_effect_contract.json",
  "lite_cut_visual_acceptance.json",
]);
copyFiltered("data", (rel) => bundledDataFiles.has(rel.toLowerCase()));

console.log(`[desktop] staged Tauri resources at ${destination}`);
