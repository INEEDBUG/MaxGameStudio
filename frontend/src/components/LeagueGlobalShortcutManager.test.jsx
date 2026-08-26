import { cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import LeagueGlobalShortcutManager from "./LeagueGlobalShortcutManager";
import { fetchLeagueLabStatus, terminateLeagueGameClient } from "../api/leagueLabApi";
import { register, unregister } from "@tauri-apps/plugin-global-shortcut";

vi.mock("../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
  terminateLeagueGameClient: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-global-shortcut", () => ({
  register: vi.fn(),
  unregister: vi.fn(),
}));

describe("LeagueGlobalShortcutManager", () => {
  beforeEach(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", { configurable: true, value: {} });
    register.mockResolvedValue(undefined);
    unregister.mockResolvedValue(undefined);
    terminateLeagueGameClient.mockResolvedValue({ terminated: true });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    delete window.__TAURI_INTERNALS__;
  });

  it("does not register the destructive shortcut by default", async () => {
    fetchLeagueLabStatus.mockResolvedValue({ settings: { terminate_game_shortcut_enabled: false } });
    render(<LeagueGlobalShortcutManager />);
    await waitFor(() => expect(fetchLeagueLabStatus).toHaveBeenCalled());
    expect(register).not.toHaveBeenCalled();
  });

  it("registers the configured shortcut and only terminates on a pressed event", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      settings: { terminate_game_shortcut_enabled: true, terminate_game_shortcut: "Ctrl+Alt+End" },
    });
    const view = render(<LeagueGlobalShortcutManager />);
    await waitFor(() => expect(register).toHaveBeenCalledWith("Ctrl+Alt+End", expect.any(Function)));
    const handler = register.mock.calls[0][1];
    await handler({ state: "Released" });
    expect(terminateLeagueGameClient).not.toHaveBeenCalled();
    await handler({ state: "Pressed" });
    expect(terminateLeagueGameClient).toHaveBeenCalledTimes(1);
    view.unmount();
    await waitFor(() => expect(unregister).toHaveBeenCalledWith("Ctrl+Alt+End"));
  });
});
