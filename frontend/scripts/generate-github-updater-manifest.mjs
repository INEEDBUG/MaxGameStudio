import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  createGithubUpdaterManifest,
  readUpdaterSignature,
} from "./updater-manifest.mjs";

const version = String(process.env.BUILD_VERSION || "").replace(/^[vV]/, "");
const repository = String(process.env.GITHUB_REPOSITORY || "");
const releaseNotes = String(process.env.RELEASE_NOTES || "");
const updateMode = String(process.env.UPDATE_MODE || "normal").toLowerCase();
const userReleaseNotesPath = String(process.env.USER_RELEASE_NOTES_PATH || "");
const scriptsDir = dirname(fileURLToPath(import.meta.url));
const nsisDir = join(scriptsDir, "../src-tauri/target/release/bundle/nsis");

const installerName = existsSync(nsisDir)
  ? readdirSync(nsisDir)
      .filter((name) => name.endsWith("-setup.exe"))
      .sort()
      .pop()
  : null;

if (!installerName) {
  throw new Error(`No NSIS installer found in ${nsisDir}`);
}

const signaturePath = join(nsisDir, `${installerName}.sig`);
if (!existsSync(signaturePath)) {
  throw new Error(`Updater signature not found: ${signaturePath}`);
}
if (!userReleaseNotesPath || !existsSync(userReleaseNotesPath)) {
  throw new Error(`User release notes not found: ${userReleaseNotesPath || "<empty>"}`);
}
const userReleaseNotes = JSON.parse(readFileSync(userReleaseNotesPath, "utf8"));

const manifest = createGithubUpdaterManifest({
  version,
  repository,
  installerName,
  signature: readUpdaterSignature(signaturePath),
  notes: releaseNotes,
  updateMode,
  userReleaseNotes,
});
const outputPath = join(nsisDir, "latest.json");
writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
console.log(`Generated updater manifest: ${outputPath}`);
console.log(`Updater asset: ${manifest.platforms["windows-x86_64"].url}`);
