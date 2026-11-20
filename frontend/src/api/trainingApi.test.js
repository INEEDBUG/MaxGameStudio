import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import API from "./api";
import {
  createInputAnalysis,
  createSensitivityRecommendation,
  fetchInputHistory,
  fetchSensitivityHistory,
} from "./trainingApi";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

afterEach(() => {
  delete window.__TAURI_INTERNALS__;
  vi.mocked(invoke).mockReset();
  vi.restoreAllMocks();
});

it("uses the native commands for desktop analysis and the existing history shape", async () => {
  window.__TAURI_INTERNALS__ = {};
  const post = vi.spyOn(API, "post").mockRejectedValue(new Error("Unexpected HTTP call"));
  const get = vi.spyOn(API, "get").mockRejectedValue(new Error("Unexpected HTTP call"));
  const analysis = { id: 5, created_at: "2026-11-20T17:00:00+00:00" };
  vi.mocked(invoke)
    .mockResolvedValueOnce(analysis)
    .mockResolvedValueOnce({ items: [analysis] })
    .mockResolvedValueOnce(analysis)
    .mockResolvedValueOnce({ items: [analysis] });

  const sensitivity = { dpi: 800 };
  const keyboard = { keyboard_name: "键盘" };
  await expect(createSensitivityRecommendation(sensitivity)).resolves.toEqual(analysis);
  await expect(fetchSensitivityHistory(12)).resolves.toEqual({ items: [analysis] });
  await expect(createInputAnalysis(keyboard)).resolves.toEqual(analysis);
  await expect(fetchInputHistory()).resolves.toEqual({ items: [analysis] });
  expect(invoke.mock.calls).toEqual([
    ["save_sensitivity_recommendation", { request: sensitivity }],
    ["list_sensitivity_history", { limit: 12 }],
    ["save_input_analysis", { request: keyboard }],
    ["list_input_history", { limit: 20 }],
  ]);
  expect(post).not.toHaveBeenCalled();
  expect(get).not.toHaveBeenCalled();
});

it("reports a native storage error without submitting the session twice", async () => {
  window.__TAURI_INTERNALS__ = {};
  const post = vi.spyOn(API, "post").mockRejectedValue(new Error("Unexpected HTTP call"));
  const error = { response: { status: 500, data: { detail: "无法保存训练记录。" } } };
  vi.mocked(invoke).mockRejectedValueOnce(error);
  await expect(createInputAnalysis({})).rejects.toEqual(error);
  expect(post).not.toHaveBeenCalled();
});

it("keeps the training HTTP routes in browser builds", async () => {
  const data = { items: [] };
  const post = vi.spyOn(API, "post").mockResolvedValue({ data });
  const get = vi.spyOn(API, "get").mockResolvedValue({ data });
  const request = { mode: "rapid_tap" };

  await expect(createSensitivityRecommendation(request)).resolves.toEqual(data);
  await expect(fetchSensitivityHistory(12)).resolves.toEqual(data);
  await expect(createInputAnalysis(request)).resolves.toEqual(data);
  await expect(fetchInputHistory()).resolves.toEqual(data);
  expect(post.mock.calls).toEqual([
    ["/training/sensitivity/recommend", request],
    ["/training/input/analyze", request],
  ]);
  expect(get.mock.calls).toEqual([
    ["/training/sensitivity/history", { params: { limit: 12 } }],
    ["/training/input/history", { params: { limit: 20 } }],
  ]);
  expect(invoke).not.toHaveBeenCalled();
});
