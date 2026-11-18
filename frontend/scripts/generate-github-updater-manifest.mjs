import { existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  createGithubUpdaterManifest,
  readUpdaterSignature,
} from "./updater-manifest.mjs";
import { resolveCargoReleaseRoot } from "./cargo-release-root.mjs";
import { mainInstallerNames, mainUpdateChannel } from "../src/utils/mainUpdateIdentity.mjs";

const version = String(process.env.BUILD_VERSION || "").replace(/^[vV]/, "");
const repository = String(process.env.GITHUB_REPOSITORY || "");
const channel = String(process.env.UPDATE_CHANNEL || "stable");
const tag = process.env.GITHUB_REF_TYPE === "tag" ? process.env.GITHUB_REF_NAME : `v${version}`;
if (mainUpdateChannel(version) !== channel || tag !== `v${version}`) {
  throw new Error(`Invalid main updater release ${tag} for channel ${channel}`);
}
const releaseNotes = String(process.env.RELEASE_NOTES || "");
const updateMode = String(process.env.UPDATE_MODE || "normal").toLowerCase();
const userReleaseNotesPath = String(process.env.USER_RELEASE_NOTES_PATH || "");
const scriptsDir = dirname(fileURLToPath(import.meta.url));
const nsisDir = join(resolveCargoReleaseRoot({
  frontendRoot: join(scriptsDir, ".."),
  cargoTargetDir: process.env.CARGO_TARGET_DIR,
}), "bundle", "nsis");

const [installerName] = mainInstallerNames(version);
const installerPath = join(nsisDir, installerName);
if (!existsSync(installerPath) || !statSync(installerPath).isFile()) {
  throw new Error(`Expected main NSIS installer was not found: ${installerPath}`);
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
  channel,
  tag,
  installerName,
  signature: readUpdaterSignature(signaturePath),
  notes: releaseNotes,
  updateMode,
  userReleaseNotes,
});
const outputPath = join(nsisDir, channel === "beta" ? "beta.json" : "latest.json");
writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
console.log(`Generated updater manifest: ${outputPath}`);
console.log(`Updater asset: ${manifest.platforms["windows-x86_64"].url}`);
