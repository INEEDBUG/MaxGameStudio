import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LeaguePresetShortcutManager from "./LeaguePresetShortcutManager";
import { fetchLeagueLabStatus, fetchLeagueOngoingGame, sendLeagueInGameLines, sendLeagueInGamePreset } from "../api/leagueLabApi";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";

vi.mock("../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
  fetchLeagueOngoingGame: vi.fn(),
  sendLeagueInGameLines: vi.fn(),
  sendLeagueInGamePreset: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({ register: vi.fn(), unregister: vi.fn() }));

describe("LeaguePresetShortcutManager", () => {
  beforeEach(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
    register.mockResolvedValue(undefined);
    unregister.mockResolvedValue(undefined);
    sendLeagueInGamePreset.mockResolvedValue({ sent: true });
    sendLeagueInGameLines.mockResolvedValue({ sent: true });
  });
  afterEach(() => { cleanup(); vi.clearAllMocks(); delete window.__TAURI_INTERNALS__; });

  it("keeps every preset shortcut unregistered until both safety switches are enabled", async () => {
    fetchLeagueLabStatus.mockResolvedValue({ settings: {
      toolkit_account_actions_enabled: false,
      in_game_send_enabled: true,
      in_game_fixed_presets: [{ id: "hello", shortcut: "Ctrl+Alt+H" }],
    } });
    render(<LeaguePresetShortcutManager />);
    await waitFor(() => expect(fetchLeagueLabStatus).toHaveBeenCalled());
    expect(register).not.toHaveBeenCalled();
  });

  it("registers enabled fixed text shortcuts and dispatches only pressed events", async () => {
    fetchLeagueLabStatus.mockResolvedValue({ settings: {
      toolkit_account_actions_enabled: true,
      in_game_send_enabled: true,
      in_game_fixed_presets: [{ id: "hello", shortcut: "Ctrl+Alt+H" }],
    } });
    render(<LeaguePresetShortcutManager />);
    await waitFor(() => expect(register).toHaveBeenCalledWith("Ctrl+Alt+H", expect.any(Function)));
    const handler = register.mock.calls[0][1];
    await handler({ state: "Released" });
    expect(sendLeagueInGamePreset).not.toHaveBeenCalled();
    await handler({ state: "Pressed" });
    expect(sendLeagueInGamePreset).toHaveBeenCalledWith("hello", "shortcut", "");
  });

  it("builds and sends target-scoped analysis presets from a configured shortcut", async () => {
    fetchLeagueLabStatus.mockResolvedValue({ current_summoner:{puuid:"self"}, settings: {
      toolkit_account_actions_enabled: true,
      in_game_send_enabled: true,
      in_game_fixed_presets: [],
      in_game_rating_shortcuts: { enemy: "Ctrl+Alt+E" },
    } });
    fetchLeagueOngoingGame.mockResolvedValue({players:[
      {puuid:"self",team:100,summoner:{gameName:"我"},recent:{matches:2,wins:1,average_kda:2}},
      {puuid:"enemy",team:200,summoner:{gameName:"敌人"},recent:{matches:2,wins:2,average_kda:4}},
    ]});
    render(<LeaguePresetShortcutManager/>);
    await waitFor(()=>expect(register).toHaveBeenCalledWith("Ctrl+Alt+E",expect.any(Function)));
    const handler=register.mock.calls.find(([shortcut])=>shortcut==="Ctrl+Alt+E")[1];
    await handler({state:"Pressed"});
    expect(sendLeagueInGameLines).toHaveBeenCalledWith([expect.stringContaining("敌人")],"","shortcut","rating","enemy");
  });

  it("uses the backend-saved preset options and shared generator for shortcuts", async () => {
    fetchLeagueLabStatus
      .mockResolvedValueOnce({ settings: {
        toolkit_account_actions_enabled: true,
        in_game_send_enabled: true,
        in_game_rating_shortcuts: { enemy: "Ctrl+Alt+R" },
        in_game_rating_preset_options: {
          target_mode: "all",
          name_display_strategy: "preferName",
          display: { win_rate: true, kda: false, main_champions: false, main_positions: false },
        },
      } })
      .mockResolvedValueOnce({ current_summoner: { puuid: "self" }, settings: {
        in_game_rating_preset_options: {
          target_mode: "all",
          name_display_strategy: "preferName",
          display: { win_rate: true, kda: false, main_champions: false, main_positions: false },
        },
      } });
    fetchLeagueOngoingGame.mockResolvedValue({ players: [
      { puuid: "self", team: 100, summoner: { gameName: "我" }, rating_summary: { win_rate: 1, avg_kda: 9 } },
      { puuid: "enemy", team: 200, summoner: { gameName: "敌人" }, rating_summary: { win_rate: 0, avg_kda: 1 } },
    ] });
    render(<LeaguePresetShortcutManager />);
    await waitFor(() => expect(register).toHaveBeenCalledWith("Ctrl+Alt+R", expect.any(Function)));
    await register.mock.calls.find(([shortcut]) => shortcut === "Ctrl+Alt+R")[1]({ state: "Pressed" });
    await waitFor(() => expect(sendLeagueInGameLines).toHaveBeenCalledWith(
      ["敌人：胜率 0%"], "", "shortcut", "rating", "enemy",
    ));
  });
});
