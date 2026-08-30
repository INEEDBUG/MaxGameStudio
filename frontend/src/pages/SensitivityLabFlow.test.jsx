import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLocaleStore } from "../i18n/localeStore.js";
import {
  createSensitivityRecommendation,
  fetchLocalCs2Settings,
  fetchSensitivityHistory,
} from "../api/trainingApi.js";
import SensitivityLabPage from "./SensitivityLabPage.jsx";

vi.mock("../components/training/SensitivityAimArena.jsx", () => ({
  default: ({ trial, setup, onComplete }) => (
    <button
      type="button"
      onClick={() => onComplete({
        kind: trial.kind,
        multiplier: trial.multiplier,
        duration_ms: 15_000,
        hits: trial.kind === "flick" ? 12 : 0,
        targets: 15,
        average_reaction_ms: 420,
        path_efficiency: 0.8,
        overshoots: 1,
        on_target_ratio: trial.kind === "tracking" ? 0.72 : 0,
      })}
    >
      complete {trial.kind} at {setup.current_sensitivity}
    </button>
  ),
}));

vi.mock("../api/trainingApi.js", () => ({
  createSensitivityRecommendation: vi.fn(),
  fetchLocalCs2Settings: vi.fn(),
  fetchSensitivityHistory: vi.fn(),
}));

describe("SensitivityLabPage full flow", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useLocaleStore.getState().hydrate("zh");
    fetchSensitivityHistory.mockResolvedValue({ items: [] });
    fetchLocalCs2Settings.mockResolvedValue({
      active_account_id: "42",
      accounts: [{
        account_id: "42",
        steam_id64: "76561198000000042",
        persona_name: "Local player",
        settings: {
          current_sensitivity: 0.35,
          m_yaw: 0.025,
          game_width: 1024,
          game_height: 1080,
          display_aspect: "other",
        },
      }],
    });
    createSensitivityRecommendation.mockResolvedValue({
      recommended_sensitivity: 0.32,
      current_sensitivity: 0.35,
      multiplier: 0.9143,
      edpi: 256,
      cm_per_360: 45.72,
      current_cm_per_360: 41.47,
      m_yaw: 0.025,
      confidence: 0.78,
      console_command: 'sensitivity "0.32"',
      diagnosis_label: "当前灵敏度偏快",
      click_tendency: "late",
      click_tendency_label: "点晚（过甩偏多）",
      click_evidence: ["点晚 8 次"],
      adjustment_percent: -8.6,
      suggested_min: 0.3072,
      suggested_max: 0.3328,
      insights: ["过冲偏多"],
      action_plan: ["降低灵敏度后复测"],
      resolution_context: "1024×1080 已记录",
    });
  });

  it("uses the imported m_yaw and reveals actionable results after all target rounds", async () => {
    render(<SensitivityLabPage />);

    expect((await screen.findByLabelText("当前 CS2 灵敏度")).value).toBe("0.35");
    expect(screen.getByLabelText("水平视角系数 m_yaw").value).toBe("0.025");
    fireEvent.click(screen.getByRole("button", { name: "进入小球靶场并开始完整测试" }));

    for (let index = 0; index < 6; index += 1) {
      fireEvent.click(await screen.findByRole("button", { name: /complete (flick|tracking) at 0.35/ }));
    }

    await waitFor(() => expect(createSensitivityRecommendation).toHaveBeenCalledTimes(1));
    expect(createSensitivityRecommendation.mock.calls[0][0]).toMatchObject({
      current_sensitivity: 0.35,
      m_yaw: 0.025,
      dpi: 800,
    });
    expect(await screen.findByText("当前灵敏度偏快")).toBeTruthy();
    expect(screen.getByText("点晚（过甩偏多）")).toBeTruthy();
    expect(screen.getByText("点击证据")).toBeTruthy();
    expect(screen.getByText("降低灵敏度后复测")).toBeTruthy();
    expect(screen.getByRole("button", { name: "调整参数后复测" })).toBeTruthy();
  });
});
