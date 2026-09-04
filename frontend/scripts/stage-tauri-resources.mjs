import {
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { dirname, isAbsolute, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const frontendRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = join(frontendRoot, "..");
const destination = join(frontendRoot, "src-tauri", "bundle-resources");
const leagueRuntimeCache = join(frontendRoot, ".runtime-cache", "league-runtime");
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
  if (path === "app/release_version.txt") return false;
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
    "0",
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

// The League runtime is staged separately because rebuilding ordinary Tauri
// resources clears bundle-resources. Release builds must fail loudly when the
// approved runnable runtime has not been staged for this checkout.
const leagueManifestPath = join(leagueRuntimeCache, "maxgamestudio-runtime-manifest.json");
if (!existsSync(leagueManifestPath)) {
  throw new Error(
    `Missing League runtime manifest: ${leagueManifestPath}. Build the pinned MaxGameStudio League runtime first.`,
  );
}
const leagueManifest = JSON.parse(readFileSync(leagueManifestPath, "utf8"));
if (leagueManifest.sourceMode !== "built-pinned-source" || leagueManifest.sourceVersion !== "1.5.1") {
  throw new Error(
    "League runtime cache is not a source-built v1.5.1 MaxGameStudio League runtime; prebuilt audit runtimes cannot be packaged.",
  );
}
if (!existsSync(join(leagueRuntimeCache, "MaxGameStudioLeague.exe"))) {
  throw new Error(
    `Missing branded League runtime cache: ${leagueRuntimeCache}. Run packaging/windows/stage-league-runtime.ps1 -Build first.`,
  );
}
const leagueHashesName = "maxgamestudio-runtime-hashes.json";
const leagueHashesPath = join(leagueRuntimeCache, leagueHashesName);
if (!existsSync(leagueHashesPath)) {
  throw new Error(`Missing League runtime hash manifest: ${leagueHashesPath}`);
}
const leagueHashes = JSON.parse(readFileSync(leagueHashesPath, "utf8"));
if (!leagueHashes || typeof leagueHashes !== "object" || Array.isArray(leagueHashes)) {
  throw new Error("League runtime hash manifest must be a JSON object.");
}
const normalizedLeagueHashes = new Map();
for (const [rawPath, rawHash] of Object.entries(leagueHashes)) {
  const normalizedPath = String(rawPath).replaceAll("\\", "/");
  const parts = normalizedPath.split("/");
  if (
    !normalizedPath ||
    normalizedPath.startsWith("/") ||
    normalizedPath.includes(":") ||
    parts.some((part) => !part || part === "." || part === "..")
  ) {
    throw new Error(`League runtime hash manifest contains an unsafe path: ${rawPath}`);
  }
  const expectedHash = String(rawHash).trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(expectedHash)) {
    throw new Error(`League runtime hash manifest contains an invalid digest: ${rawPath}`);
  }
  if (normalizedLeagueHashes.has(normalizedPath)) {
    throw new Error(`League runtime hash manifest contains a duplicate path: ${normalizedPath}`);
  }
  normalizedLeagueHashes.set(normalizedPath, expectedHash);
}
if (
  !normalizedLeagueHashes.has("MaxGameStudioLeague.exe") ||
  !normalizedLeagueHashes.has("resources/app.asar")
) {
  throw new Error("League runtime hash manifest is missing required payload files.");
}
const leagueAllowedFiles = new Set([
  ...normalizedLeagueHashes.keys(),
  leagueHashesName,
]);
const leagueAllowedDirectories = new Set();
for (const relativePath of leagueAllowedFiles) {
  const parts = relativePath.split("/");
  for (let index = 1; index < parts.length; index += 1) {
    leagueAllowedDirectories.add(parts.slice(0, index).join("/"));
  }
}
const realLeagueRuntimeCache = realpathSync(leagueRuntimeCache);
for (const [relativePath, expectedHash] of normalizedLeagueHashes) {
  const sourcePath = join(leagueRuntimeCache, ...relativePath.split("/"));
  if (!existsSync(sourcePath)) {
    throw new Error(`League runtime payload is missing: ${sourcePath}`);
  }
  let currentPath = leagueRuntimeCache;
  for (const part of relativePath.split("/")) {
    currentPath = join(currentPath, part);
    if (lstatSync(currentPath).isSymbolicLink()) {
      throw new Error(`League runtime payload contains a symbolic link: ${relativePath}`);
    }
  }
  if (!lstatSync(sourcePath).isFile()) {
    throw new Error(`League runtime payload is not a regular file: ${relativePath}`);
  }
  const realSourcePath = realpathSync(sourcePath);
  const realRelative = relative(realLeagueRuntimeCache, realSourcePath);
  if (!realRelative || realRelative.startsWith("..") || isAbsolute(realRelative)) {
    throw new Error(`League runtime payload escaped its cache root: ${relativePath}`);
  }
  const actualHash = createHash("sha256").update(readFileSync(sourcePath)).digest("hex");
  if (actualHash !== expectedHash) {
    throw new Error(`League runtime payload hash mismatch: ${relativePath}`);
  }
}
cpSync(leagueRuntimeCache, join(destination, "league-runtime"), {
  recursive: true,
  filter(path) {
    const rel = normalizedRelative(leagueRuntimeCache, path);
    return !rel || leagueAllowedFiles.has(rel) || leagueAllowedDirectories.has(rel);
  },
});

console.log(`[desktop] staged Tauri resources at ${destination}`);
