export const MAIN_UPDATE_PRODUCT = "maxgamestudio";

const STABLE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const BETA = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-beta\.(0|[1-9]\d*)$/;

export function mainUpdateChannel(version) {
  if (typeof version !== "string" || version.trim() !== version) return null;
  if (STABLE.test(version)) return "stable";
  if (BETA.test(version)) return "beta";
  return null;
}

export function mainInstallerNames(version, { legacy = false } = {}) {
  if (version === "3.1.4") return ["MaxGameStudio_3.1.4_x64-repair-setup.exe"];
  const current = `MaxGameStudio_${version}_x64-setup.exe`;
  if (!legacy) return [current];
  return [
    current,
    `CS2.Ultimate.Insight.Studio_${version}_x64-setup.exe`,
    `CS2 Ultimate Insight Studio_${version}_x64-setup.exe`,
    `MaxGameStudio-${version}-Setup.exe`,
    `CS2.Insight.Agent.Setup.${version}.exe`,
    `CS2 Insight Agent Setup ${version}.exe`,
    `CS2InsightAgent-${version}-Setup.exe`,
  ];
}
