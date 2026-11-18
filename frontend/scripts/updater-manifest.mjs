import { readFileSync } from "node:fs";
import { MAIN_UPDATE_PRODUCT, mainInstallerNames, mainUpdateChannel } from "../src/utils/mainUpdateIdentity.mjs";

export function githubReleaseAssetName(installerName) {
  return String(installerName).replaceAll(" ", ".");
}

export function createGithubUpdaterManifest({
  version,
  repository,
  installerName,
  signature,
  channel = "stable",
  tag = `v${version}`,
  notes = "",
  pubDate = new Date().toISOString(),
  updateMode = "normal",
  userReleaseNotes = null,
}) {
  if (channel === "stable" && mainUpdateChannel(version) !== "stable") {
    throw new Error(`Updater manifest requires a stable semantic version (x.y.z): ${version}`);
  }
  if (!["stable", "beta"].includes(channel) || mainUpdateChannel(version) !== channel) {
    throw new Error(`Invalid main updater version for the ${channel} channel: ${version}`);
  }
  if (tag !== `v${version}`) {
    throw new Error(`Updater tag must match the main release version: ${tag}`);
  }
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) {
    throw new Error(`Invalid GitHub repository: ${repository}`);
  }
  if (!String(signature || "").trim()) {
    throw new Error("Updater signature is required");
  }

  const assetName = githubReleaseAssetName(installerName);
  if (!mainInstallerNames(version).includes(assetName)) {
    throw new Error(`Unexpected main installer for ${tag}: ${assetName}`);
  }
  const assetUrl = `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(assetName)}`;

  const manifest = {
    version,
    product: MAIN_UPDATE_PRODUCT,
    tag_name: tag,
    channel,
    notes,
    pub_date: pubDate,
    update_mode: updateMode === "force" ? "force" : "normal",
    platforms: {
      "windows-x86_64": {
        signature: String(signature).trim(),
        url: assetUrl,
      },
    },
  };
  if (userReleaseNotes) {
    manifest.user_release_notes = normalizeUserReleaseNotes(userReleaseNotes, version);
  }
  return manifest;
}

export function normalizeUserReleaseNotes(value, expectedVersion = "") {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("User release notes must be a JSON object");
  }
  const version = String(value.version || "").replace(/^[vV]/, "");
  if (expectedVersion && version !== expectedVersion) {
    throw new Error(`User release notes version mismatch: expected ${expectedVersion}, got ${version}`);
  }
  const normalized = { version };
  for (const category of ["fixed", "added", "optimized"]) {
    if (!Array.isArray(value[category])) {
      throw new Error(`User release notes category must be an array: ${category}`);
    }
    normalized[category] = value[category]
      .map((item) => String(item || "").trim())
      .filter(Boolean);
  }
  if (![...normalized.fixed, ...normalized.added, ...normalized.optimized].length) {
    throw new Error("User release notes must contain at least one item");
  }
  return normalized;
}

export function readUpdaterSignature(signaturePath) {
  return readFileSync(signaturePath, "utf8").trim();
}
