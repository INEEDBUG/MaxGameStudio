import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueAuxShortcutManager from "./LeagueAuxShortcutManager";
import { cancelLeagueInGameSend, fetchLeagueLabStatus } from "../api/leagueLabApi";
import { invoke } from "@tauri-apps/api/core";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";

vi.mock("../api/leagueLabApi", () => ({ fetchLeagueLabStatus: vi.fn(), cancelLeagueInGameSend: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({ register: vi.fn(), unregister: vi.fn() }));

describe("LeagueAuxShortcutManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.__TAURI_INTERNALS__ = {};
    fetchLeagueLabStatus.mockResolvedValue({ settings: {
      in_game_send_enabled: true,
      in_game_cancel_shortcut: "Ctrl+Alt+C",
      ongoing_window_shortcut: "Ctrl+Alt+O",
      cooldown_window_shortcut: "Ctrl+Alt+T",
    }});
    register.mockResolvedValue(undefined);
    unregister.mockResolvedValue(undefined);
    invoke.mockResolvedValue(undefined);
    cancelLeagueInGameSend.mockResolvedValue({ cancel_requested: true });
  });

  it("registers cancel and auxiliary window shortcuts with upstream-equivalent behavior", async () => {
    render(<LeagueAuxShortcutManager/>);
    await act(async () => {});
    const callbacks = Object.fromEntries(register.mock.calls.map(([shortcut, callback]) => [shortcut, callback]));
    expect(Object.keys(callbacks).sort()).toEqual(["Ctrl+Alt+C","Ctrl+Alt+O","Ctrl+Alt+T"].sort());

    await callbacks["Ctrl+Alt+C"]({ state: "Pressed" });
    await callbacks["Ctrl+Alt+O"]({ state: "Pressed" });
    await callbacks["Ctrl+Alt+O"]({ state: "Released" });
    await callbacks["Ctrl+Alt+T"]({ state: "Pressed" });

    expect(cancelLeagueInGameSend).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("toggle_league_aux_window", { kind: "ongoing", visible: true });
    expect(invoke).toHaveBeenCalledWith("toggle_league_aux_window", { kind: "ongoing", visible: false });
    expect(invoke).toHaveBeenCalledWith("toggle_league_aux_window", { kind: "cooldown", visible: null });
    expect(invoke).not.toHaveBeenCalledWith("toggle_league_aux_window", { kind: "opgg", visible: null });
  });
});
