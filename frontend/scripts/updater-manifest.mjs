import { readFileSync } from "node:fs";

export function githubReleaseAssetName(installerName) {
  return String(installerName).replaceAll(" ", ".");
}

export function createGithubUpdaterManifest({
  version,
  repository,
  installerName,
  signature,
  notes = "",
  pubDate = new Date().toISOString(),
  updateMode = "normal",
}) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`Updater manifest requires a stable semantic version (x.y.z): ${version}`);
  }
  if (!/^[^/]+\/[^/]+$/.test(repository)) {
    throw new Error(`Invalid GitHub repository: ${repository}`);
  }
  if (!String(signature || "").trim()) {
    throw new Error("Updater signature is required");
  }

  const assetName = githubReleaseAssetName(installerName);
  const assetUrl = `https://github.com/${repository}/releases/download/v${version}/${encodeURIComponent(assetName)}`;

  return {
    version,
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
}

export function readUpdaterSignature(signaturePath) {
  return readFileSync(signaturePath, "utf8").trim();
}
