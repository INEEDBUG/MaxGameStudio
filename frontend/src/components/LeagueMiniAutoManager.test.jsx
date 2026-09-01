import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueMiniAutoManager from "./LeagueMiniAutoManager";
import { fetchLeagueLabStatus } from "../api/leagueLabApi";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../api/leagueLabApi", () => ({ fetchLeagueLabStatus: vi.fn() }));

describe("LeagueMiniAutoManager", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.__TAURI_INTERNALS__ = {};
    invoke.mockResolvedValue(undefined);
  });

  it("shows Mini without user interaction during supported League phases", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "Lobby",
      mini_should_show: true,
      cooldown_timer_should_show: false,
      settings: { mini_enabled: true, mini_auto_show: true },
    });
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_mini", {
      shouldShow: true,
      context: "connected:Lobby:playing",
    }));
    expect(invoke).not.toHaveBeenCalledWith("sync_league_opgg", expect.anything());
    view.unmount();
  });

  it("hides Mini after disconnecting or leaving an eligible phase", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: false,
      phase: "None",
      mini_should_show: false,
      cooldown_timer_should_show: false,
      settings: { mini_enabled: true, mini_auto_show: true },
    });
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_mini", {
      shouldShow: false,
      context: "offline:None:playing",
    }));
    view.unmount();
  });

  it.each(["GameStart", "InProgress", "Reconnect"])("shows the independent ongoing window during %s", async (phase) => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase,
      game_mode: "ARAM",
      mini_should_show: false,
      cooldown_timer_should_show: false,
      settings: {
        mini_enabled: true,
        mini_auto_show: true,
        // This setting controls the main-window route only. The dedicated
        // ongoing window follows the game lifecycle independently.
        ongoing_auto_route_when_game_starts: false,
      },
    });
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_ongoing", {
      shouldShow: true,
      context: `connected:${phase}:ARAM`,
    }));
    view.unmount();
  });

  it("hides the independent ongoing window after the game phase ends or the client disconnects", async () => {
    fetchLeagueLabStatus
      .mockResolvedValueOnce({
        connected: true,
        phase: "InProgress",
        game_mode: "ARAM",
        mini_should_show: false,
        cooldown_timer_should_show: false,
        settings: { ongoing_auto_route_when_game_starts: true },
      })
      .mockResolvedValue({
        connected: false,
        phase: "None",
        game_mode: "ARAM",
        mini_should_show: false,
        cooldown_timer_should_show: false,
        settings: { ongoing_auto_route_when_game_starts: true },
      });
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_ongoing", {
      shouldShow: true,
      context: "connected:InProgress:ARAM",
    }));
    await new Promise((resolve) => setTimeout(resolve, 1650));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_ongoing", {
      shouldShow: false,
      context: "offline:None:ARAM",
    }), { timeout: 2500 });
    view.unmount();
  });

  it("never opens an automatic auxiliary window from a stale InProgress response", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "InProgress",
      // Deliberately inconsistent to model a stale/racing backend response.
      mini_should_show: true,
      cooldown_timer_should_show: true,
      settings: { mini_enabled: true, mini_auto_show: true, cooldown_timer_enabled: true },
    });
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_mini", {
      shouldShow: false,
      context: "connected:InProgress:playing",
    }));
    expect(invoke).toHaveBeenCalledWith("sync_league_cd_timer", {
      shouldShow: true,
      context: "connected:InProgress:unknown",
    });
    view.unmount();
  });

  it("retries a lifecycle sync that failed during desktop startup", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "None",
      mini_should_show: false,
      cooldown_timer_should_show: false,
      settings: { mini_enabled: true, mini_auto_show: true },
    });
    invoke.mockImplementationOnce(() => Promise.reject(new Error("desktop not ready")));
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    await waitFor(
      () => expect(invoke).toHaveBeenCalledWith("sync_league_mini", {
        shouldShow: false,
        context: "connected:None:playing",
      }),
      { timeout: 2500 },
    );
    view.unmount();
  });

  it("applies the newest phase after a slow native window sync", async () => {
    let resolveNativeSync;
    fetchLeagueLabStatus
      .mockResolvedValueOnce({
        connected: true,
        phase: "Lobby",
        mini_should_show: true,
        cooldown_timer_should_show: false,
        settings: { mini_enabled: true, mini_auto_show: true, ongoing_auto_route_when_game_starts: true },
      })
      .mockResolvedValue({
        connected: true,
        phase: "InProgress",
        mini_should_show: false,
        cooldown_timer_should_show: false,
        settings: { mini_enabled: true, mini_auto_show: true, ongoing_auto_route_when_game_starts: true },
      });
    invoke.mockImplementationOnce(() => new Promise((resolve) => { resolveNativeSync = resolve; }));
    const view = render(<LeagueMiniAutoManager />);

    await waitFor(() => expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("set_league_content_protection", { enabled: false }));
    await new Promise((resolve) => setTimeout(resolve, 1600));
    expect(fetchLeagueLabStatus.mock.calls.length).toBeGreaterThanOrEqual(2);

    resolveNativeSync();
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_mini", {
      shouldShow: false,
      context: "connected:InProgress:playing",
    }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("sync_league_ongoing", {
      shouldShow: true,
      context: "connected:InProgress:unknown",
    }));
    view.unmount();
  });
});
