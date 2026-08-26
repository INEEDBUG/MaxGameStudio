import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueMiniPanel from "./LeagueMiniPanel";
import { acceptLeagueChampSelectTrade, cancelLeagueAutoAccept, cancelLeagueDodgeLoop, charityRerollLeagueChampion, declineLeagueChampSelectTrade, declineLeagueReadyCheck, fetchLeagueLabStatus, rerollLeagueChampion, saveLeagueLabSettings, selectLeagueChampionFromMini, selectLeagueChampionSkin, startLeagueDodgeLoop, stopLeagueMatchmaking, swapLeagueBenchChampion } from "../api/leagueLabApi";

const windowActions = {
  close: vi.fn(),
  minimize: vi.fn(),
  setAlwaysOnTop: vi.fn(),
};

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowActions,
}));

vi.mock("../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
  acceptLeagueChampSelectTrade: vi.fn(),
  cancelLeagueAutoAccept: vi.fn(),
  cancelLeagueDodgeLoop: vi.fn(),
  charityRerollLeagueChampion: vi.fn(),
  declineLeagueChampSelectTrade: vi.fn(),
  declineLeagueReadyCheck: vi.fn(),
  rerollLeagueChampion: vi.fn(),
  runLeagueLabAction: vi.fn(),
  saveLeagueLabSettings: vi.fn(),
  selectLeagueChampionFromMini: vi.fn(),
  selectLeagueChampionSkin: vi.fn(),
  setLeagueAutoSelectTemporarilyDisabled: vi.fn(),
  startLeagueDodgeLoop: vi.fn(),
  stopLeagueMatchmaking: vi.fn(),
  swapLeagueBenchChampion: vi.fn(),
}));

vi.mock("../api/api", () => ({
  getLeagueChampionIconUrl: (id) => `champion-${id}.png`,
  getLeagueClientAssetUrl: (path) => path ? `/api/league-lab/assets/client?path=${encodeURIComponent(path)}` : "",
}));

describe("LeagueMiniPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1 },
      champ_select: { my_team: [{ cell_id: 1, champion_id: 22 }], my_actions: [{ id: 1, type: "pick", champion_id: 22, in_progress: true }] },
    });
  });

  it("renders client state instead of a blank auxiliary window", async () => {
    render(<LeagueMiniPanel />);
    expect(await screen.findByText("选择英雄")).toBeTruthy();
    expect(screen.getByText("MaxGameStudio Mini")).toBeTruthy();
    expect(screen.getByText("选择英雄")).toBeTruthy();
  });

  it("exposes the compact titlebar controls without a refresh action", async () => {
    render(<LeagueMiniPanel />);
    await screen.findByText("选择英雄");

    fireEvent.click(screen.getByRole("button", { name: "取消置顶" }));
    await waitFor(() => expect(windowActions.setAlwaysOnTop).toHaveBeenCalledWith(false));
    fireEvent.click(screen.getByRole("button", { name: "最小化 Mini" }));
    fireEvent.click(screen.getByRole("button", { name: "关闭 Mini" }));
    expect(windowActions.minimize).toHaveBeenCalledOnce();
    expect(windowActions.close).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: "刷新 Mini" })).toBeNull();
    expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["未连接", { connected: false, phase: "" }],
    ["客户端空闲", { connected: true, phase: "None" }],
    ["房间中", { connected: true, phase: "Lobby", matchmaking_status: "countdown" }],
    ["正在匹配", { connected: true, phase: "Matchmaking", matchmaking_status: "searching" }],
    ["对局已找到", { connected: true, phase: "ReadyCheck", action_countdown: { label: "自动接受对局", remaining_seconds: 2.5 } }],
  ])("renders the compact Lounge view for %s", async (label, patch) => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "Lobby",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      ...patch,
    });
    render(<LeagueMiniPanel />);
    if (label === "未连接" || label === "客户端空闲") {
      expect(await screen.findByTestId("mini-placeholder")).toBeTruthy();
    } else {
      expect(await screen.findByTestId("mini-lounge-view")).toBeTruthy();
    }
    expect(screen.queryByTestId("mini-diagnostics")).toBeNull();
  });

  it("renders the LCU queue mode, map name, and map artwork in Lounge", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "Lobby",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      gameflow_session: {
        gameData: { queue: { name: "海克斯大乱斗" } },
        map: { name: "随机地图", assets: { "game-select-icon-hover": "hextech-map.png" } },
      },
    });
    render(<LeagueMiniPanel />);

    expect(await screen.findByText("海克斯大乱斗 · 随机地图")).toBeTruthy();
    expect(screen.getByTestId("mini-map-icon").querySelector("img")?.getAttribute("src") || screen.getByTestId("mini-map-icon").getAttribute("src")).toBe("/api/league-lab/assets/client?path=hextech-map.png");
  });

  it("switches a failed map asset to the local fallback without leaving a broken image", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "Lobby",
      settings: { mini_opacity: 1 },
      gameflow_session: { map: { assets: { "game-select-icon-hover": "missing-map.png" } } },
    });
    render(<LeagueMiniPanel />);

    const image = await screen.findByTestId("mini-map-icon");
    expect(image.tagName).toBe("IMG");
    fireEvent.error(image);

    await waitFor(() => {
      expect(screen.getByTestId("mini-map-icon").querySelector("img")).toBeNull();
      expect(screen.getByTestId("mini-map-icon").querySelector("svg")).toBeTruthy();
    });
  });

  it("shows compact action progress without a diagnostics accordion", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      champ_select: {
        timer_phase: "BAN_PICK",
        timer_deadline_at: Date.now() / 1000 + 20,
        my_actions: [
          { id: 1, type: "pick", champion_id: 22, completed: true, in_progress: false },
          { id: 2, type: "ban", champion_id: 0, completed: false, in_progress: true },
        ],
      },
    });
    render(<LeagueMiniPanel />);
    expect(await screen.findByTestId("mini-champ-select-view")).toBeTruthy();
    expect(screen.getByText("选择英雄")).toBeTruthy();
    expect(screen.getByText("禁用英雄")).toBeTruthy();
    expect(screen.queryByTestId("mini-diagnostics")).toBeNull();
  });

  it("disables every manual client-write control when account writes are off", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      champ_select: {
        bench_enabled: true,
        allow_rerolling: true,
        rerolls_remaining: 1,
        bench_champions: [22],
        skin_selector: { available: true, disabled: false, champion_id: 22, skins: [{ id: 11, name: "Test Skin", preview_path: "skin-11.jpg" }] },
      },
    });
    render(<LeagueMiniPanel />);
    await screen.findByTestId("mini-account-actions-disabled");
    expect(screen.getByRole("button", { name: "立即秒退" }).disabled).toBe(true);
    expect(screen.getByRole("button", { name: /^重随 1$/ }).disabled).toBe(true);
    expect(screen.getByTestId("mini-charity-reroll").disabled).toBe(true);
    expect(screen.getByAltText("22").parentElement.disabled).toBe(true);
    expect(screen.getByRole("combobox", { name: "Mini 皮肤选择" }).disabled).toBe(true);
    expect(selectLeagueChampionSkin).not.toHaveBeenCalled();
  });

  it.each([
    ["the client does not allow rerolling", { allow_rerolling: false, rerolls_remaining: 2 }],
    ["the client reports no remaining rerolls", { allow_rerolling: true, rerolls_remaining: 0 }],
    ["the client does not return explicit reroll support", { rerolls_remaining: 2 }],
  ])("hides reroll controls when %s", async (_label, evidence) => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      champ_select: {
        bench_enabled: true,
        bench_champions: [22],
        current_champion_id: 22,
        ...evidence,
      },
    });
    render(<LeagueMiniPanel />);

    await screen.findByTestId("mini-champ-select-view");
    expect(screen.queryByTestId("mini-reroll-controls")).toBeNull();
    expect(screen.queryByRole("button", { name: /^重随/ })).toBeNull();
    expect(screen.queryByTestId("mini-charity-reroll")).toBeNull();
  });

  it.each(["ARAM", "KIWI"])("does not infer reroll support from the %s queue name", async (gameMode) => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      gameflow_session: { gameData: { queue: { gameMode } } },
      champ_select: { bench_enabled: true, allow_rerolling: false, rerolls_remaining: 1, bench_champions: [22] },
    });
    render(<LeagueMiniPanel />);

    await screen.findByTestId("mini-champ-select-view");
    expect(screen.queryByTestId("mini-reroll-controls")).toBeNull();
  });

  it("renders the compact skin selector and sends only the selected skin id", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      champ_select: {
        skin_selector: {
          available: true,
          disabled: false,
          champion_id: 22,
          selected_skin_id: 11,
          skins: [
            { id: 11, name: "Test Skin", preview_path: "skin-11.jpg" },
            { id: 12, name: "Test Chroma", preview_path: "chroma-12.jpg", is_chroma: true },
            { id: 13, name: "Locked Skin", preview_path: "skin-13.jpg", owned: false },
          ],
        },
      },
    });
    selectLeagueChampionSkin.mockResolvedValue(null);
    render(<LeagueMiniPanel />);

    const select = await screen.findByRole("combobox", { name: "Mini 皮肤选择" });
    expect(select.value).toBe("11");
    expect(select.querySelector('option[value="12"]').textContent).toContain("炫彩");
    expect(select.querySelector('option[value="13"]').disabled).toBe(true);

    fireEvent.change(select, { target: { value: "12" } });
    await waitFor(() => expect(selectLeagueChampionSkin).toHaveBeenCalledWith(12));
    expect(selectLeagueChampionSkin).toHaveBeenCalledTimes(1);
  });

  it("lets the Mini click a server-provided subset champion to pick or bench-swap", async () => {
    const current = {
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      champ_select: {
        bench_enabled: true,
        allow_subset_champion_picks: true,
        timer_phase: "BAN_PICK",
        current_champion_id: 0,
        subset_champion_ids: [22, 34],
        bench_champions: [55],
        current_pickable_champion_ids: [22, 34, 55],
        current_pickable_ids_available: true,
      },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    selectLeagueChampionFromMini.mockResolvedValue(null);
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-bench-champion-22")).toBeTruthy();
    expect(screen.getByTestId("mini-bench-champion-34")).toBeTruthy();
    expect(screen.getByTestId("mini-bench-champion-55")).toBeTruthy();
    fireEvent.click(screen.getByTestId("mini-bench-champion-22"));
    await waitFor(() => expect(selectLeagueChampionFromMini).toHaveBeenCalledWith(22));
  });

  it("runs charity reroll only after the explicit phrase", async () => {
    const current = { connected: true, phase: "ChampSelect", summoner_name: "Tester", settings: { mini_opacity: 1, toolkit_account_actions_enabled: true }, champ_select: { bench_enabled: true, allow_rerolling: true, rerolls_remaining: 1, current_champion_id: 22 } };
    fetchLeagueLabStatus.mockResolvedValue(current);
    charityRerollLeagueChampion.mockResolvedValue({ ...current, charity_reroll: { rerolled: true, grabbed_back: true } });
    const prompt = vi.spyOn(window, "prompt").mockReturnValue("我确认慈善重随");
    render(<LeagueMiniPanel />);
    fireEvent.click(await screen.findByTestId("mini-charity-reroll"));
    await waitFor(() => expect(charityRerollLeagueChampion).toHaveBeenCalledWith("我确认慈善重随"));
    prompt.mockRestore();
  });

  it("starts and cancels only the backend-managed dodge loop", async () => {
    const current = { connected: true, phase: "ChampSelect", summoner_name: "Tester", settings: { mini_opacity: 1, toolkit_account_actions_enabled: true }, champ_select: {}, dodge_loop: { active: false, attempts: 0 } };
    fetchLeagueLabStatus.mockResolvedValue(current);
    startLeagueDodgeLoop.mockResolvedValue({ ...current, dodge_loop: { active: true, attempts: 0, concurrency: 5 } });
    cancelLeagueDodgeLoop.mockResolvedValue({ ...current, dodge_loop: { active: false, attempts: 2, stop_reason: "user-cancelled" } });
    render(<LeagueMiniPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "立即秒退" }));
    expect(startLeagueDodgeLoop).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole("button", { name: "确认秒退" }));
    await waitFor(() => expect(startLeagueDodgeLoop).toHaveBeenCalledWith("我确认秒退"));
    fireEvent.click(await screen.findByRole("button", { name: "取消循环" }));
    await waitFor(() => expect(cancelLeagueDodgeLoop).toHaveBeenCalledOnce());
  });

  it("keeps the previous status when reroll returns an empty response", async () => {
    const current = {
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      champ_select: { bench_enabled: true, allow_rerolling: true, rerolls_remaining: 1, bench_champions: [22] },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    rerollLeagueChampion.mockResolvedValue(null);
    render(<LeagueMiniPanel />);
    fireEvent.click(await screen.findByRole("button", { name: /^重随/ }));
    await waitFor(() => expect(rerollLeagueChampion).toHaveBeenCalledOnce());
    expect(await screen.findByText(/重随请求已发送.*正在刷新状态/)).toBeTruthy();
    expect(screen.getByText("MaxGameStudio Mini")).toBeTruthy();
  });

  it("renders ReadyCheck evidence and keeps decline behind the toolkit gate while allowing local cancellation", async () => {
    const current = {
      connected: true,
      phase: "ReadyCheck",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      ready_check: { state: "InProgress", player_response: "None", can_accept: true, can_decline: true, timer: { remaining_seconds: 4 } },
      action_plan: { accept_due: { label: "自动接受对局", remaining_seconds: 2.5 }, phase_due: null, champion_due: [] },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    cancelLeagueAutoAccept.mockResolvedValue(null);
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-lounge-view")).toBeTruthy();
    expect(screen.getByText("自动接受对局")).toBeTruthy();
    expect(screen.getByRole("button", { name: "拒绝对局" }).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "取消自动接受" }));
    await waitFor(() => expect(cancelLeagueAutoAccept).toHaveBeenCalledOnce());
    expect(await screen.findByText(/取消本次自动接受.*正在刷新状态/)).toBeTruthy();
    expect(declineLeagueReadyCheck).not.toHaveBeenCalled();
  });

  it("sends a gated ReadyCheck decline and never hides the API error", async () => {
    const current = {
      connected: true,
      phase: "ReadyCheck",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      ready_check: { state: "InProgress", player_response: "None", can_accept: false, can_decline: true, timer: { remaining_seconds: 4 } },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    declineLeagueReadyCheck.mockRejectedValue({ response: { data: { detail: "ReadyCheck 已结束" } } });
    render(<LeagueMiniPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "拒绝对局" }));
    await waitFor(() => expect(declineLeagueReadyCheck).toHaveBeenCalledOnce());
    expect(await screen.findByText("ReadyCheck 已结束")).toBeTruthy();
  });

  it("renders the compact matchmaking Lounge view and keeps stop gated", async () => {
    const current = {
      connected: true,
      phase: "Matchmaking",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      matchmaking_search: { is_currently_in_queue: true, search_state: "Searching", time_in_queue: 12, estimated_queue_time: 30, queue_id: 420, errors: [{ code: "WAIT", message: "等待服务器" }] },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    render(<LeagueMiniPanel />);
    expect(await screen.findByTestId("mini-lounge-view")).toBeTruthy();
    expect(screen.getByText("12 秒 / 30 秒")).toBeTruthy();
    expect(screen.getByRole("button", { name: "停止匹配" }).disabled).toBe(true);
    expect(stopLeagueMatchmaking).not.toHaveBeenCalled();
  });

  it("opens the master automation gate when Mini enables a child without enabling siblings", async () => {
    const current = {
      connected: true,
      phase: "Lobby",
      summoner_name: "Tester",
      matchmaking_status: "waiting_for_invitees",
      matchmaking_due_at: Date.now() / 1000 + 4,
      settings: {
        mini_opacity: 1,
        automation_enabled: false,
        auto_matchmaking_enabled: false,
        auto_matchmaking_minimum_members: 1,
        auto_matchmaking_delay_seconds: 5,
        auto_matchmaking_wait_for_invitees: true,
        auto_matchmaking_rematch_strategy: "never",
      },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    saveLeagueLabSettings.mockImplementation(async (settings) => ({ ...current, settings }));
    render(<LeagueMiniPanel />);

    const card = await screen.findByTestId("mini-lounge-operations");
    const feature = screen.getByRole("switch", { name: "自动匹配（简洁）" });
    expect(feature.disabled).toBe(false);
    fireEvent.click(feature);

    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenCalledWith(expect.objectContaining({
      automation_enabled: true,
      auto_matchmaking_enabled: true,
    })));
    expect(saveLeagueLabSettings.mock.calls[0][0]).not.toHaveProperty("auto_accept_enabled");
    expect(card).toBeTruthy();

    saveLeagueLabSettings.mockImplementation(async (settings) => ({ ...current, settings }));
    fireEvent.click(feature);
    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      automation_enabled: true,
      auto_matchmaking_enabled: false,
    })));
  });

  it("opens the master automation gate for Mini auto-accept without enabling auto-matchmaking", async () => {
    const current = {
      connected: true,
      phase: "ReadyCheck",
      settings: {
        mini_opacity: 1,
        automation_enabled: false,
        auto_accept_enabled: false,
        auto_matchmaking_enabled: false,
      },
      ready_check: { can_accept: true, can_decline: true },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    saveLeagueLabSettings.mockImplementation(async (settings) => ({ ...current, settings }));
    render(<LeagueMiniPanel />);

    const feature = await screen.findByRole("switch", { name: "自动接受" });
    fireEvent.click(feature);

    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenCalledWith(expect.objectContaining({
      automation_enabled: true,
      auto_accept_enabled: true,
      auto_matchmaking_enabled: false,
    })));
  });

  it("edits Mini matchmaking thresholds and renders rematch evidence", async () => {
    const current = {
      connected: true,
      phase: "Matchmaking",
      summoner_name: "Tester",
      matchmaking_status: "searching",
      matchmaking_search: { time_in_queue: 30, estimated_queue_time: 90 },
      settings: {
        mini_opacity: 1,
        automation_enabled: true,
        auto_matchmaking_enabled: true,
        auto_matchmaking_minimum_members: 2,
        auto_matchmaking_delay_seconds: 3,
        auto_matchmaking_wait_for_invitees: true,
        auto_matchmaking_rematch_strategy: "fixed-duration",
        auto_matchmaking_rematch_fixed_duration: 120,
      },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    saveLeagueLabSettings.mockImplementation(async (settings) => ({ ...current, settings }));
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-lounge-operations")).toBeTruthy();
    fireEvent.click(screen.getByText(/自动匹配（3\.0 秒）/));
    fireEvent.change(screen.getByLabelText("Mini 简洁匹配最低人数"), { target: { value: "4" } });
    await waitFor(() => expect(saveLeagueLabSettings).toHaveBeenCalledWith(expect.objectContaining({ auto_matchmaking_minimum_members: 4 })));
  });

  it("keeps trade actions out of the strict upstream Mini surface", async () => {
    const current = {
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      champ_select: {
        trades: [
          { id: 17, state: "AVAILABLE", actionable: true, can_accept: true, can_decline: true, initiated_by_local_player: false, other_player: { game_name: "Ally" } },
          { id: 18, state: "INVALID", actionable: false, can_accept: false, can_decline: false, actionability: { reason: "state-not-actionable" } },
        ],
      },
    };
    fetchLeagueLabStatus.mockResolvedValue(current);
    render(<LeagueMiniPanel />);
    expect(await screen.findByTestId("mini-champ-select-view")).toBeTruthy();
    expect(screen.queryByTestId("mini-trades")).toBeNull();
    expect(acceptLeagueChampSelectTrade).not.toHaveBeenCalled();
    expect(declineLeagueChampSelectTrade).not.toHaveBeenCalled();
  });

  it("shows action_plan champion deadlines without manufacturing a countdown", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: true },
      action_plan: {
        accept_due: null,
        phase_due: { label: "自动返回房间", remaining_seconds: null },
        champion_due: [{ action_id: "pick-1", label: "自动选择 / 禁用英雄", remaining_seconds: 3 }],
      },
      champ_select: { my_actions: [] },
    });
    render(<LeagueMiniPanel />);
    expect(await screen.findByTestId("mini-auto-select-plan")).toBeTruthy();
    expect(screen.getByTestId("mini-auto-select-plan").textContent).toContain("自动选择 / 禁用英雄");
  });

  it("keeps Lounge phases mutually exclusive from ChampSelect content", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "Matchmaking",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      matchmaking_search: { is_currently_in_queue: true, search_state: "Searching" },
    });
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-lounge-view")).toBeTruthy();
    expect(screen.queryByTestId("mini-champ-select-view")).toBeNull();
    expect(screen.queryByTestId("mini-in-progress")).toBeNull();
    expect(screen.queryByTestId("mini-diagnostics")).toBeNull();
  });

  it("keeps the ChampSelect view focused with the upstream card order", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "ChampSelect",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      champ_select: { my_team: [{ cell_id: 1, champion_id: 22 }], my_actions: [{ id: 1, type: "pick", champion_id: 22, in_progress: true }] },
    });
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-champ-select-view")).toBeTruthy();
    expect(screen.queryByTestId("mini-lounge-view")).toBeNull();
    expect(screen.queryByTestId("mini-in-progress")).toBeNull();
    expect(screen.queryByTestId("mini-diagnostics")).toBeNull();
    expect(screen.getByText("选择英雄")).toBeTruthy();
  });

  it("renders the empty Placeholder for non-Lounge/non-ChampSelect phases", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "InProgress",
      summoner_name: "Tester",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      respawn_timer: { enabled: true, dead: true, time_left: 8.5 },
    });
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-placeholder")).toBeTruthy();
    expect(screen.queryByText("复活倒计时")).toBeNull();
    expect(screen.queryByTestId("mini-lounge-view")).toBeNull();
    expect(screen.queryByTestId("mini-champ-select-view")).toBeNull();
  });

  it("keeps live-game Placeholder informative when mode/map evidence is available", async () => {
    fetchLeagueLabStatus.mockResolvedValue({
      connected: true,
      phase: "InProgress",
      settings: { mini_opacity: 1, toolkit_account_actions_enabled: false },
      gameflow_session: {
        gameData: { queue: { name: "经典模式" } },
        map: { name: "召唤师峡谷", assets: { "game-select-icon-hover": "summoners-rift.png" } },
      },
    });
    render(<LeagueMiniPanel />);

    expect(await screen.findByTestId("mini-placeholder")).toBeTruthy();
    expect(screen.getByText("经典模式 · 召唤师峡谷")).toBeTruthy();
    expect(screen.getByText("游戏进行中")).toBeTruthy();
  });
});
