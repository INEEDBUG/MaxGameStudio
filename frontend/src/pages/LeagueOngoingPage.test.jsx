import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { fetchLeagueLabStatus, saveLeagueLabSettings } from "../api/leagueLabApi";
import LeagueOngoingPage from "./LeagueOngoingPage";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
  saveLeagueLabSettings: vi.fn(),
}));
vi.mock("../components/league/LeagueOngoingGame", () => ({
  default: ({ streamerMode, useAliases }) => <div data-testid="ongoing-game">game:{String(streamerMode)}:{String(useAliases)}</div>,
}));
vi.mock("../components/league/LeagueOngoingSettings", () => ({
  default: ({ settings, onUpdate }) => (
    <div data-testid="ongoing-settings">
      <span>{settings.ongoing_premade_threshold}</span>
      <button type="button" onClick={() => onUpdate({ ongoing_premade_threshold: 9 })}>保存阈值</button>
    </div>
  ),
}));

const status = {
  connected: true,
  settings: {
    ongoing_premade_threshold: 7,
    streamer_mode_enabled: true,
    streamer_mode_use_aliases: true,
  },
};

describe("LeagueOngoingPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeagueLabStatus.mockResolvedValue(status);
    saveLeagueLabSettings.mockResolvedValue(status);
    invoke.mockResolvedValue(null);
  });

  it("hydrates status settings and renders the dedicated ongoing game", async () => {
    render(<LeagueOngoingPage />);

    expect(await screen.findByText("7")).toBeTruthy();
    expect(screen.getByTestId("ongoing-game").textContent).toBe("game:true:true");
    expect(screen.getByRole("heading", { name: "实时对局" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "自动化" })).toBeNull();
    expect(screen.queryByRole("button", { name: "我的战绩" })).toBeNull();
    expect(screen.queryByRole("button", { name: "玩家中心" })).toBeNull();
    expect(screen.queryByRole("button", { name: "客户端工具" })).toBeNull();
  });

  it("refreshes status on demand and opens the standalone window", async () => {
    render(<LeagueOngoingPage />);
    await screen.findByText("7");

    fireEvent.click(screen.getByRole("button", { name: "刷新实时对局状态" }));
    await waitFor(() => expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(2));

    fireEvent.click(screen.getByRole("button", { name: "打开独立实时对局窗口" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("open_league_ongoing"));
  });

  it("restores settings after a failed save and keeps the error visible", async () => {
    saveLeagueLabSettings.mockRejectedValueOnce(new Error("保存失败"));
    render(<LeagueOngoingPage />);
    await screen.findByText("7");

    fireEvent.click(screen.getByRole("button", { name: "保存阈值" }));

    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenCalledWith(expect.objectContaining({ ongoing_premade_threshold: 9 })));
    await waitFor(() => expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(2));
    expect((await screen.findByRole("alert")).textContent).toContain("保存失败");
    expect(screen.getByTestId("ongoing-settings").textContent).toContain("7");
  });
});
