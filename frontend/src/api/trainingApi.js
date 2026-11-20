import { invoke } from "@tauri-apps/api/core";
import API from "./api";

export async function createSensitivityRecommendation(payload) {
  if (window.__TAURI_INTERNALS__) {
    return invoke("save_sensitivity_recommendation", { request: payload });
  }
  const { data } = await API.post("/training/sensitivity/recommend", payload);
  return data;
}

export async function fetchSensitivityHistory(limit = 20) {
  if (window.__TAURI_INTERNALS__) {
    return invoke("list_sensitivity_history", { limit });
  }
  const { data } = await API.get("/training/sensitivity/history", { params: { limit } });
  return data;
}

export async function fetchLocalCs2Settings() {
  if (window.__TAURI_INTERNALS__) {
    return invoke("get_local_cs2_settings");
  }
  const { data } = await API.get("/training/cs2-settings");
  return data;
}

export async function createInputAnalysis(payload) {
  if (window.__TAURI_INTERNALS__) {
    return invoke("save_input_analysis", { request: payload });
  }
  const { data } = await API.post("/training/input/analyze", payload);
  return data;
}

export async function fetchInputHistory(limit = 20) {
  if (window.__TAURI_INTERNALS__) {
    return invoke("list_input_history", { limit });
  }
  const { data } = await API.get("/training/input/history", { params: { limit } });
  return data;
}
