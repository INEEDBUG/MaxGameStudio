import API, { API_BASE_URL } from "./api.js";

// File-only loadout editing: these routes never start the native recorder.
export function getAppearanceInventory(demo_path, xuid) { return API.post('/cosmetic-studio/appearance/inventory', { demo_path, ...(xuid?{xuid}:{}) }); }
export function saveAppearanceSettings(payload) { return API.put('/cosmetic-studio/appearance/settings', payload); }
export function applyAppearancePlan(payload) { return API.post('/cosmetic-studio/appearance/apply', payload); }
export function getAppearanceJob(id) { return API.get(`/cosmetic-studio/appearance/jobs/${encodeURIComponent(id)}`); }
export function cancelAppearanceJob(id) { return API.post(`/cosmetic-studio/appearance/jobs/${encodeURIComponent(id)}/cancel`, {}); }
export function playAppearanceDemo(id) { return API.post(`/cosmetic-studio/appearance/jobs/${encodeURIComponent(id)}/playback`, { offline_acknowledged: true }); }
export function getSkyOptions() { return API.get('/cosmetic-studio/appearance/sky/options'); }
export function startSkyPreview(payload) { return API.post('/cosmetic-studio/appearance/sky/preview', payload); }
export function getSkyStatus() { return API.get('/cosmetic-studio/appearance/sky/status'); }
export function stopSkyPreview() { return API.post('/cosmetic-studio/appearance/sky/stop', {}); }

export function getCosmeticProfiles() { return API.get('/cosmetic-studio/profiles'); }
export function saveCosmeticProfile(payload) { return API.post('/cosmetic-studio/profiles', payload); }
export function deleteCosmeticProfile(id) { return API.delete(`/cosmetic-studio/profiles/${encodeURIComponent(id)}`); }

export function getCosmeticCapabilities() {
  return API.get("/cosmetic-studio/capabilities");
}

export function inspectCosmeticDemo(demo_path) {
  return API.post("/cosmetic-studio/inspect", { demo_path });
}

export function importCosmeticDirectorPlan(payload) {
  return API.post('/cosmetic-studio/director/import', payload);
}

export function validateCosmeticInterval(payload) {
  return API.post('/cosmetic-studio/interval', payload);
}

export function createCosmeticJob(payload) {
  return API.post("/cosmetic-studio/jobs", payload);
}

export function getCosmeticJob(jobId) {
  return API.get(`/cosmetic-studio/jobs/${encodeURIComponent(String(jobId))}`);
}

export function getActiveCosmeticJob() {
  return API.get("/cosmetic-studio/active");
}

export function confirmCosmeticJob(jobId, preview_id) {
  return API.post(`/cosmetic-studio/jobs/${encodeURIComponent(String(jobId))}/confirm`, {
    preview_id,
    appearance_confirmed: true,
  });
}

export function cancelCosmeticJob(jobId) {
  return API.post(`/cosmetic-studio/jobs/${encodeURIComponent(String(jobId))}/cancel`, {});
}

export function getCosmeticPreviewBlob(previewUrl) {
  const raw = String(previewUrl || "").trim();
  const parsed = new URL(raw, window.location.origin);
  const allowedOrigins = new Set([window.location.origin]);
  if (API_BASE_URL) allowedOrigins.add(new URL(API_BASE_URL).origin);
  if (!allowedOrigins.has(parsed.origin)) {
    return Promise.reject(new Error("预览地址不是本地后端地址"));
  }
  if (parsed.search || parsed.hash || !parsed.pathname.startsWith("/api/cosmetic-studio/")) {
    return Promise.reject(new Error("预览地址不是受支持的本地预览路径"));
  }
  const localPath = parsed.pathname.slice("/api".length);
  return API.get(localPath, { responseType: "blob" });
}
