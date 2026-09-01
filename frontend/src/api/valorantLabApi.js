import API, { isTransientAxiosNetworkError } from "./api";
import { normalizeCrosshairProfiles, normalizeDisplayStatus } from "../utils/valorantLab";

/**
 * Valorant Lab API contract. The backend may land after this UI. Keep these
 * paths stable so the page can show an explicit unavailable state meanwhile.
 */
export const VALORANT_LAB_API_CONTRACT = Object.freeze({
  displayStatus: { method: "GET", path: "/valorant-lab/display/status" },
  prepareStretch: { method: "POST", path: "/valorant-lab/stretch/prepare" },
  applyStretch: { method: "POST", path: "/valorant-lab/stretch/apply" },
  confirmStretch: { method: "POST", path: "/valorant-lab/stretch/confirm" },
  restoreStretch: { method: "POST", path: "/valorant-lab/stretch/restore" },
  unlockStretchCfg: { method: "POST", path: "/valorant-lab/stretch/cfg/unlock" },
  restoreStretchCfg: { method: "POST", path: "/valorant-lab/stretch/cfg/restore" },
  openDeviceManager: { method: "POST", path: "/valorant-lab/display/open-device-manager" },
  crosshair: { method: "GET", path: "/valorant-lab/crosshair" },
  encodeCrosshair: { method: "POST", path: "/valorant-lab/crosshair/encode" },
  decodeCrosshair: { method: "POST", path: "/valorant-lab/crosshair/decode" },
  saveCrosshair: { method: "PUT", path: "/valorant-lab/crosshair" },
});

export function isValorantLabApiUnavailable(error) {
  return isTransientAxiosNetworkError(error) || [404, 501, 503].includes(error?.response?.status);
}

export async function fetchValorantDisplayStatus() {
  const { data } = await API.get(VALORANT_LAB_API_CONTRACT.displayStatus.path);
  const normalized = normalizeDisplayStatus(data);
  if (data && typeof data === "object" && Object.prototype.hasOwnProperty.call(data, "cfg_status")) {
    return { ...normalized, cfg_status: data.cfg_status };
  }
  return normalized;
}

export async function prepareValorantStretch(payload) {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.prepareStretch.path, payload);
  return data;
}

export async function applyValorantStretch(payload) {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.applyStretch.path, payload);
  return data;
}

export async function confirmValorantStretch() {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.confirmStretch.path);
  return data;
}

export async function restoreValorantStretch() {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.restoreStretch.path);
  return data;
}

export async function unlockValorantStretchCfg() {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.unlockStretchCfg.path);
  return data;
}

export async function restoreValorantStretchCfg() {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.restoreStretchCfg.path);
  return data;
}

export async function openValorantDeviceManager() {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.openDeviceManager.path);
  return data;
}

export async function fetchValorantCrosshair() {
  const { data } = await API.get(VALORANT_LAB_API_CONTRACT.crosshair.path);
  return normalizeCrosshairResponse(data);
}

function normalizeCrosshairResponse(value) {
  const raw = value && typeof value === "object" ? value : {};
  const source = raw.profiles && typeof raw.profiles === "object" ? raw.profiles : raw;
  const code = typeof raw.code === "string" ? raw.code.trim() : "";
  return {
    ...raw,
    profiles: normalizeCrosshairProfiles(source),
    code,
  };
}

export async function encodeValorantCrosshair(profiles) {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.encodeCrosshair.path, {
    profiles: normalizeCrosshairProfiles(profiles),
  });
  return normalizeCrosshairResponse(data);
}

export async function decodeValorantCrosshair(code) {
  const { data } = await API.post(VALORANT_LAB_API_CONTRACT.decodeCrosshair.path, {
    code: String(code || "").trim(),
  });
  return normalizeCrosshairResponse(data);
}

export async function saveValorantCrosshair(profiles) {
  const { data } = await API.put(VALORANT_LAB_API_CONTRACT.saveCrosshair.path, { profiles: normalizeCrosshairProfiles(profiles) });
  return normalizeCrosshairResponse(data);
}
