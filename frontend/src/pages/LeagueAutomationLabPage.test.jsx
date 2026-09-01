import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import LeagueAutomationLabPage from "./LeagueAutomationLabPage";
import { fetchLeagueClientInstallations, fetchLeagueClients, fetchLeagueLabStatus, fetchLeagueMatches, fetchLeagueOngoingGame, fetchLeagueReplay, runLeagueLabAction, saveLeagueLabSettings } from "../api/leagueLabApi";
import { useLocaleStore } from "../i18n/localeStore.js";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn().mockResolvedValue(null) }));

vi.mock("../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
  fetchLeagueClients: vi.fn(),
  fetchLeagueClientInstallations: vi.fn(),
  fetchLeagueMatches: vi.fn(),
  fetchLeagueOngoingGame: vi.fn(),
  fetchLeagueReplay: vi.fn(),
  fetchLeagueChampions: vi.fn().mockResolvedValue({ champions: [] }),
  fetchLeagueLoadoutCatalog: vi.fn(),
  fetchLeagueMatchDetails: vi.fn(),
  selectLeagueClient: vi.fn(),
  launchLeagueClient: vi.fn(),
  saveLeagueLabSettings: vi.fn(),
  runLeagueLabAction: vi.fn(),
}));

vi.mock("../components/league/LeagueOngoingGame", () => ({
  default: () => <div>实时对局内容</div>,
}));

const status = {
  connected: true,
  phase: "Lobby",
  summoner_name: "Tester",
  platform_id: "HN1",
  settings: {
    automation_enabled: false,
    auto_accept_enabled: false,
    auto_accept_delay_seconds: 1,
    play_again_enabled: false,
    auto_reconnect_enabled: false,
    invitation_strategy: "ignore",
  },
};

describe("LeagueAutomationLabPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useLocaleStore.getState().hydrate("zh");
    fetchLeagueLabStatus.mockResolvedValue(status);
    fetchLeagueClients.mockResolvedValue({ clients: [], selected_pid: 0 });
    fetchLeagueClientInstallations.mockResolvedValue({ installations: [] });
    fetchLeagueMatches.mockResolvedValue({ matches: [{ game_id: 1001, champion_id: 1, champion_name: "安妮", participant_puuid: "self", team_id: 100, win: true, participants: [] }] });
    fetchLeagueOngoingGame.mockResolvedValue({ available: false, players: [] });
    fetchLeagueReplay.mockResolvedValue({ enabled: false });
    saveLeagueLabSettings.mockResolvedValue({ ...status, settings: { ...status.settings, automation_enabled: true } });
  });

  it("leaves canonical League navigation to the sidebar", async () => {
    render(<LeagueAutomationLabPage routeTab="history" />);
    expect(await screen.findByText("已连接：Tester")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "自动化" })).toBeNull();
  });

  it("shows the detected League client and persists the master switch", async () => {
    render(<LeagueAutomationLabPage />);
    expect(await screen.findByText("已连接：Tester")).toBeTruthy();

    fireEvent.click(screen.getByRole("switch", { name: "启用英雄联盟自动化" }));
    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenCalledWith(expect.objectContaining({ automation_enabled: true })));
    fireEvent.click(screen.getByRole("button", { name: "其他" }));
    fireEvent.click(screen.getByText("辅助窗口与外观"));
    expect(screen.getByLabelText("Mini 不透明度").value).toBe("1");
    expect(screen.queryByLabelText("OP.GG 不透明度")).toBeNull();
    expect(screen.getByRole("switch", { name: "Mini 显示皮肤选择器" })).toBeTruthy();
  });

  it("offers a confirmed administrator restart when an elevated WeGame client is visible", async () => {
    fetchLeagueLabStatus.mockResolvedValueOnce({
      ...status,
      connected: false,
      client_window_detected: true,
      requires_elevation: true,
      summoner_name: "",
    });
    vi.spyOn(window, "confirm").mockReturnValueOnce(true);

    render(<LeagueAutomationLabPage />);
    expect(await screen.findByText("已发现客户端，但权限不足")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "以管理员身份重启并连接" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("restart_as_administrator"));
  });

  it("uses the detailed match card in the current-account history", async () => {
    render(<LeagueAutomationLabPage routeTab="history" />);
    await screen.findByText("已连接：Tester");
    await waitFor(() => expect(fetchLeagueMatches).toHaveBeenCalledWith(20));
    expect(await screen.findByRole("button", { name: "展开战绩详情" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "结算后自动刷新战绩" })).toBeTruthy();
  });

  it("renders the canonical history route with a scroll container", async () => {
    const scrollTo = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollTo", { configurable: true, value: scrollTo });
    render(<LeagueAutomationLabPage routeTab="history" />);
    await screen.findByText("已连接：Tester");
    expect(scrollTo).toBeDefined();
  });

  it("exposes configurable ongoing-game analysis controls", async () => {
    render(<LeagueAutomationLabPage routeTab="ongoing" />);
    await screen.findByText("已连接：Tester");
    expect(screen.getByRole("switch", { name: "在房间阶段分析队友" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "所有玩家都分析打野路线" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "战绩条目强调边框" })).toBeTruthy();
    expect(screen.getByLabelText("实时玩家排序").value).toBe("default");
    expect(screen.getByLabelText("实时英雄数据来源").value).toBe("recent");
    expect(screen.getByLabelText("实时战绩样本范围").value).toBe("current");
    expect(screen.getByLabelText("实时详情时间线数量").value).toBe("20");
    expect(screen.getByRole("switch", { name: "显示打野路线画像" })).toBeTruthy();
    expect(screen.getByLabelText("实时对局战绩读取数").value).toBe("20");
    expect(screen.getByLabelText("打野画像分析场数").value).toBe("4");
    expect(screen.getByRole("switch", { name: "显示连胜 / 连败标签" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "显示表现画像标签" })).toBeTruthy();
    expect(screen.getByLabelText("实时对局并发查询数").value).toBe("10");
    expect(screen.getByLabelText("组排推断阈值").value).toBe("5");
  });

  it("shows the same read-only auto-select move and delayed plans in the main editor", async () => {
    fetchLeagueLabStatus.mockResolvedValueOnce({
      ...status,
      phase: "ChampSelect",
      settings: { ...status.settings, auto_select_enabled: false },
      auto_select: {
        enabled: false,
        move: "show-ban",
        active_group_id: "ranked",
        assigned_position: "middle",
        actionability: { show: false },
        expected_bans: [{ id: 4, status: "bannable" }],
        delayed_ban: { move: "show-ban", remaining_seconds: 1.25 },
        config: { pick_strategy: "show-and-lock-in", ban_strategy: "show-and-lock-in", show_intent: false },
      },
    });

    render(<LeagueAutomationLabPage routeTab="automation" />);
    await screen.findByText("已连接：Tester");
    fireEvent.click(screen.getByRole("button", { name: "自动选择 / 禁用" }));

    expect(await screen.findByTestId("main-auto-select-runtime")).toBeTruthy();
    expect(screen.getAllByText("亮出禁用").length).toBeGreaterThan(0);
    expect(screen.getByText("禁用计划")).toBeTruthy();
    expect(screen.getByText("1.3 秒")).toBeTruthy();
  });

  it("serializes rapid settings writes and merges each update from the latest settings", async () => {
    const writes = [];
    const resolvers = [];
    saveLeagueLabSettings.mockImplementation((next) => {
      writes.push(next);
      return new Promise((resolve) => resolvers.push(resolve));
    });

    render(<LeagueAutomationLabPage />);
    await screen.findByText("已连接：Tester");

    await act(async () => {
      fireEvent.click(screen.getByRole("switch", { name: "启用英雄联盟自动化" }));
      fireEvent.click(screen.getByRole("switch", { name: "自动接受对局" }));
    });

    await waitFor(() => expect(writes).toHaveLength(1));
    expect(writes[0]).toMatchObject({ automation_enabled: true, auto_accept_enabled: false });
    resolvers[0]({ ...status, settings: { ...status.settings, ...writes[0] } });

    await waitFor(() => expect(writes).toHaveLength(2));
    expect(writes[1]).toMatchObject({ automation_enabled: true, auto_accept_enabled: true });
    resolvers[1]({ ...status, settings: { ...status.settings, ...writes[1] } });
    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenCalledTimes(2));
  });

  it("handles manual accept, play-again and reconnect through one refreshable action path", async () => {
    runLeagueLabAction.mockResolvedValue({ ...status, last_action: "ok" });
    render(<LeagueAutomationLabPage />);
    await screen.findByText("已连接：Tester");

    for (const [label, action] of [["立即接受", "accept"], ["返回房间", "play-again"], ["立即重连", "reconnect"]]) {
      const beforeRefreshes = fetchLeagueLabStatus.mock.calls.length;
      const button = screen.getByRole("button", { name: label });
      fireEvent.click(button);
      await waitFor(() => expect(runLeagueLabAction).toHaveBeenCalledWith(action));
      await waitFor(() => expect(fetchLeagueLabStatus.mock.calls.length).toBeGreaterThan(beforeRefreshes));
      await waitFor(() => expect(button.disabled).toBe(false));
    }

    runLeagueLabAction.mockRejectedValueOnce(new Error("模拟操作失败"));
    fireEvent.click(screen.getByRole("button", { name: "立即接受" }));
    expect(await screen.findByText("模拟操作失败")).toBeTruthy();
    await waitFor(() => expect(screen.getByRole("button", { name: "立即接受" }).disabled).toBe(false));
  });

  it("prevents an older match-history response from replacing a newer page", async () => {
    const requests = [];
    fetchLeagueMatches.mockImplementation((limit, offset) => new Promise((resolve, reject) => requests.push({ limit, offset, resolve, reject })));

    render(<LeagueAutomationLabPage routeTab="history" />);
    await screen.findByText("已连接：Tester");
    await waitFor(() => expect(requests).toHaveLength(1));

    fireEvent.change(screen.getByRole("combobox", { name: "战绩每页数量" }), { target: { value: "10" } });
    await waitFor(() => expect(requests).toHaveLength(2));

    await act(async () => {
      requests[1].resolve({ matches: [{ game_id: 2002, champion_id: 1, champion_name: "安妮", participant_puuid: "self", team_id: 100, win: true, participants: [] }] });
      await Promise.resolve();
    });
    expect(await screen.findByText("对局 ID 2002")).toBeTruthy();
    await act(async () => {
      requests[0].resolve({ matches: [{ game_id: 1001, champion_id: 1, champion_name: "安妮", participant_puuid: "self", team_id: 100, win: true, participants: [] }] });
      await Promise.resolve();
    });
    expect(screen.queryByText("对局 ID 1001")).toBeNull();
  });
});
