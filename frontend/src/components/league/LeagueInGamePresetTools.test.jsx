import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueInGamePresetTools from "./LeagueInGamePresetTools";
import {
  cancelLeagueInGameSend,
  fetchLeagueLabStatus,
  fetchLeagueOngoingGame,
  sendLeagueInGameLines,
  sendLeagueInGamePreset,
} from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  cancelLeagueInGameSend: vi.fn(),
  fetchLeagueLabStatus: vi.fn(),
  fetchLeagueOngoingGame: vi.fn(),
  sendLeagueInGameLines: vi.fn(),
  sendLeagueInGamePreset: vi.fn(),
}));

const baseSettings = {
  toolkit_account_actions_enabled: false,
  in_game_send_enabled: false,
  in_game_send_interval_ms: 250,
  in_game_fixed_presets: [{ id: "hello", title: "问候", shortcut: "Ctrl+Alt+H", content: "你好" }],
};

const renderTools = (settings = baseSettings, overrides = {}) => render(
  <LeagueInGamePresetTools
    settings={settings}
    busy={false}
    onSettingsUpdate={vi.fn()}
    onBusyChange={vi.fn()}
    onError={vi.fn()}
    {...overrides}
  />,
);

describe("LeagueInGamePresetTools", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.prompt = vi.fn();
    window.confirm = vi.fn();
    window.localStorage.clear();
  });

  it("keeps the send action disabled while either safety switch is off", () => {
    renderTools();
    fireEvent.click(screen.getByRole("tab", { name: "固定文字" }));
    expect(screen.getByRole("button", { name: "发送 问候" }).disabled).toBe(true);
  });

  it("requires the exact phrase before manually sending the selected preset", async () => {
    sendLeagueInGamePreset.mockResolvedValue({ sent: true });
    window.prompt.mockReturnValueOnce("错误").mockReturnValueOnce("我确认发送");
    const settings = { ...baseSettings, toolkit_account_actions_enabled: true, in_game_send_enabled: true };
    renderTools(settings);
    fireEvent.click(screen.getByRole("tab", { name: "固定文字" }));
    fireEvent.click(screen.getByRole("button", { name: "发送 问候" }));
    expect(sendLeagueInGamePreset).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "发送 问候" }));
    await waitFor(() => expect(sendLeagueInGamePreset).toHaveBeenCalledWith("hello", "manual", "我确认发送"));
  });

  it("persists fixed presets in the user-selected order", async () => {
    const onSettingsUpdate = vi.fn().mockResolvedValue({});
    const settings = {
      ...baseSettings,
      toolkit_account_actions_enabled: true,
      in_game_fixed_presets: [
        { id: "first", title: "第一条", shortcut: null, content: "一" },
        { id: "second", title: "第二条", shortcut: null, content: "二" },
      ],
    };
    renderTools(settings, { onSettingsUpdate });
    fireEvent.click(screen.getByRole("tab", { name: "固定文字" }));
    fireEvent.click(screen.getByRole("button", { name: "上移 第二条" }));
    await waitFor(() => expect(onSettingsUpdate).toHaveBeenCalledWith({ in_game_fixed_presets: [
      settings.in_game_fixed_presets[1],
      settings.in_game_fixed_presets[0],
    ] }));
  });

  it("starts with LeagueAkari rating defaults and exposes four preset tabs", () => {
    renderTools();
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Rating", "打野画像", "组排关系", "固定文字"]);
    expect(screen.getByRole("tab", { name: "Rating" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("checkbox", { name: "胜率" }).checked).toBe(true);
    expect(screen.getByRole("checkbox", { name: "平均 KDA" }).checked).toBe(true);
    expect(screen.getByRole("checkbox", { name: "场均单杀" }).checked).toBe(true);
    expect(screen.getByRole("checkbox", { name: "主力英雄" }).checked).toBe(true);
    expect(screen.getByRole("checkbox", { name: "主位置" }).checked).toBe(true);
    expect(screen.getByRole("checkbox", { name: "场均视野" }).checked).toBe(false);
    expect(screen.getByRole("checkbox", { name: "显示当前英雄" }).checked).toBe(false);
  });

  it("loads players on demand, supports manual target selection, and previews only selected payload data", async () => {
    fetchLeagueOngoingGame.mockResolvedValue({ players: [
      { puuid: "alice-puuid", team: 100, summoner: { gameName: "Alice" }, champion_name: "Ahri", rating_summary: { win_rate: .75, avg_kda: 2.5, avg_solo_kills: 1.2, main_champions: [{ champion_name: "Ahri" }], main_positions: [{ position: "MIDDLE" }] } },
      { puuid: "bob-puuid", team: 200, summoner: { gameName: "Bob" }, champion_name: "Braum", rating_summary: { win_rate: .25, avg_kda: .8, avg_solo_kills: 0, main_champions: [{ champion_name: "Braum" }], main_positions: [{ position: "UTILITY" }] } },
    ] });
    fetchLeagueLabStatus.mockResolvedValue({ current_summoner: { puuid: "alice-puuid" } });
    renderTools();

    expect(fetchLeagueOngoingGame).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "读取当前玩家" }));
    await waitFor(() => expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(1));
    fireEvent.change(screen.getByRole("combobox", { name: "Rating目标范围" }), { target: { value: "selected" } });
    fireEvent.click(screen.getByRole("checkbox", { name: "选择 Ahri" }));
    fireEvent.click(screen.getByRole("button", { name: "生成预览" }));

    await waitFor(() => expect(screen.getByRole("textbox", { name: "Rating生成预览" }).value).toContain("Ahri：胜率 75%"));
    expect(screen.getByRole("textbox", { name: "Rating生成预览" }).value).not.toContain("Bob");
  });

  it("renders the full LeagueAkari rating metric set and omits null solo kills", async () => {
    fetchLeagueOngoingGame.mockResolvedValue({ players: [
      {
        puuid: "metric-puuid",
        team: 100,
        summoner: { gameName: "MetricPlayer" },
        rating_summary: {
          win_rate: .8,
          avg_kda: 3.25,
          avg_solo_kills: null,
          avg_vision_score: 8.4,
          avg_champion_damage_percentage_of_team: .35,
          avg_damage_taken_percentage_of_team: .22,
          avg_gold_percentage_of_team: .18,
          avg_cs_per_minute: 7.5,
          avg_kill_participation: .64,
          avg_damage_gold_efficiency: 1.23,
          main_champions: [{ champion_name: "Ahri" }, { champion_name: "Syndra" }],
          main_positions: [{ position: "MIDDLE" }, { position: "TOP" }],
        },
      },
    ] });
    fetchLeagueLabStatus.mockResolvedValue({ current_summoner: { puuid: "metric-puuid" } });
    renderTools();

    fireEvent.click(screen.getByRole("button", { name: "读取当前玩家" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "生成预览" }).disabled).toBe(false));
    ["场均视野", "团队输出占比", "团队承伤占比", "团队经济占比", "每分钟补刀", "参团率", "伤害经济效率"].forEach((label) => {
      fireEvent.click(screen.getByRole("checkbox", { name: label }));
    });
    fireEvent.click(screen.getByRole("button", { name: "生成预览" }));

    const preview = screen.getByRole("textbox", { name: "Rating生成预览" });
    await waitFor(() => expect(preview.value).toContain("MetricPlayer：胜率 80%，KDA 3.25"));
    expect(preview.value).toContain("场均视野 8.4");
    expect(preview.value).toContain("团队输出 35%");
    expect(preview.value).toContain("团队承伤 22%");
    expect(preview.value).toContain("团队经济 18%");
    expect(preview.value).toContain("补刀 7.5/分");
    expect(preview.value).toContain("参团 64%");
    expect(preview.value).toContain("伤害/经济 1.23");
    expect(preview.value).toContain("主力 Ahri/Syndra");
    expect(preview.value).toContain("主位置 MIDDLE/TOP");
    expect(preview.value).not.toContain("场均单杀");
  });

  it("sends the generated preview only after the exact confirmation and can cancel an active send", async () => {
    fetchLeagueOngoingGame.mockResolvedValue({ players: [
      { puuid: "alice-puuid", team: 100, summoner: { gameName: "Alice" }, rating_summary: { win_rate: 1, avg_kda: 1.5, avg_solo_kills: null, main_champions: [], main_positions: [] } },
    ] });
    fetchLeagueLabStatus.mockResolvedValue({ current_summoner: { puuid: "alice-puuid" } });
    sendLeagueInGameLines.mockResolvedValue({ sent: true });
    cancelLeagueInGameSend.mockResolvedValue({ cancelled: true });
    window.prompt.mockReturnValue("我确认发送");
    const settings = { ...baseSettings, toolkit_account_actions_enabled: true, in_game_send_enabled: true };
    renderTools(settings);
    fireEvent.click(screen.getByRole("button", { name: "读取当前玩家" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "生成预览" }).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "生成预览" }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Rating生成预览" }).value).toContain("Alice"));
    fireEvent.click(screen.getByRole("button", { name: "发送预览" }));
    await waitFor(() => expect(sendLeagueInGameLines).toHaveBeenCalledWith(["Alice：胜率 100%，KDA 1.50"], "我确认发送", "manual", "rating", "all"));
    fireEvent.click(screen.getByRole("button", { name: "取消发送", exact: true }));
    await waitFor(() => expect(cancelLeagueInGameSend).toHaveBeenCalledTimes(1));
  });

  it("keeps an empty premade preview when the payload has no premade grouping", async () => {
    fetchLeagueOngoingGame.mockResolvedValue({ players: [{ puuid: "solo", team: 100, summoner: { gameName: "Solo" } }] });
    fetchLeagueLabStatus.mockResolvedValue({ current_summoner: { puuid: "solo" } });
    renderTools();
    fireEvent.click(screen.getByRole("tab", { name: "组排关系" }));
    fireEvent.click(screen.getByRole("button", { name: "读取当前玩家" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "生成预览" }).disabled).toBe(false));
    fireEvent.click(screen.getByRole("button", { name: "生成预览" }));
    await waitFor(() => expect(screen.getByRole("textbox", { name: "组排关系生成预览" }).value).toBe(""));
  });
});
