import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fetchLeagueLabStatus, fetchLeagueMatchDetails, fetchLeagueOngoingGame } from "../../api/leagueLabApi";
import LeagueOngoingGame from "./LeagueOngoingGame";

vi.mock("../../api/leagueLabApi", () => ({
  fetchLeagueLabStatus: vi.fn(),
  fetchLeagueLoadoutCatalog: vi.fn(),
  fetchLeagueMatchDetails: vi.fn(),
  fetchLeagueOngoingGame: vi.fn(),
}));

describe("LeagueOngoingGame", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeagueLabStatus.mockResolvedValue({ settings: {} });
    fetchLeagueOngoingGame.mockResolvedValue({
      available: true,
      query_stage: "lobby",
      show_match_history_item_border: true,
      players: [{
        puuid: "player-1",
        team: "LOBBY",
        champion_id: 0,
        champion_name: "",
        summoner: { gameName: "Tester", profileIconId: 12 },
        recent: { matches: 3, wins: 2, average_kda: 4.2, akari_score: 7.5 },
        champion_usage: { mode: "none" },
        performance_tags: [],
      }],
    });
    fetchLeagueMatchDetails.mockResolvedValue({ source: "lcu", frame_count: 0, event_count: 0, events: [], frames: [] });
  });

  it("renders lobby-stage players with a single profile icon and opens the player", async () => {
    const onOpenPlayer = vi.fn();
    render(<LeagueOngoingGame onOpenPlayer={onOpenPlayer}/>);

    expect(await screen.findByText("当前房间")).toBeTruthy();
    expect(screen.getByText("房间阶段已开始分析当前队伍；进入英雄选择后会自动补全对手、英雄与分路。")).toBeTruthy();
    expect(screen.getAllByAltText("召唤师头像")).toHaveLength(1);
    expect(screen.getByText(/Akari 7.5/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "打开 Tester 玩家中心" }));
    expect(onOpenPlayer).toHaveBeenCalledWith("player-1");
    await waitFor(() => expect(fetchLeagueOngoingGame).toHaveBeenCalledWith({ snapshot: true }));
  });

  it("does not refetch global status when privacy settings are already provided", async () => {
    render(<LeagueOngoingGame streamerMode={false} useAliases={false} />);

    await screen.findByText("当前房间");
    expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(1);
    expect(fetchLeagueLabStatus).not.toHaveBeenCalled();
  });

  it("renders the roster snapshot before the optional privacy status request resolves", async () => {
    let resolveStatus;
    fetchLeagueLabStatus.mockReturnValueOnce(new Promise((resolve) => { resolveStatus = resolve; }));

    render(<LeagueOngoingGame />);

    expect(await screen.findByText("Tester")).toBeTruthy();
    expect(fetchLeagueLabStatus).toHaveBeenCalledTimes(1);
    await act(async () => { resolveStatus({ settings: { streamer_mode_enabled: false, streamer_mode_use_aliases: false } }); });
  });

  it("keeps the compact player metric strip on a dark surface in light mode", async () => {
    document.documentElement.dataset.theme = "light";
    render(<LeagueOngoingGame streamerMode={false} useAliases={false} />);
    const strip = await screen.findByTestId("ongoing-player-metrics");
    for (const cell of strip.children) {
      expect(cell.className).toContain("bg-[#202126]/95");
      expect(cell.className).not.toContain("bg-cs2-bg-elevated");
    }
    delete document.documentElement.dataset.theme;
  });

  it("renders ready players while another card is still loading, then promotes the snapshot to ready", async () => {
    vi.useFakeTimers();
    try {
      fetchLeagueOngoingGame
        .mockResolvedValueOnce({
          available: true,
          partial: true,
          game_id: 42,
          query_stage: "in-game",
          players: [
            { puuid: "ready", team: 100, load_state: "ready", summoner: { gameName: "Ready" }, recent: { matches: 3, wins: 2, average_kda: 3 }, champion_usage: { mode: "none" }, performance_tags: [], data_availability: { history: true } },
            { puuid: "loading", team: 100, load_state: "loading", summoner: { gameName: "Loading" }, recent: { matches: 0, wins: 0 }, champion_usage: { mode: "none" }, performance_tags: [], data_availability: { history: false } },
          ],
        })
        .mockResolvedValueOnce({
          available: true,
          partial: false,
          game_id: 42,
          query_stage: "in-game",
          players: [
            { puuid: "ready", team: 100, load_state: "ready", summoner: { gameName: "Ready" }, recent: { matches: 3, wins: 2, average_kda: 3 }, champion_usage: { mode: "none" }, performance_tags: [], data_availability: { history: true } },
            { puuid: "loading", team: 100, load_state: "ready", summoner: { gameName: "Loading" }, recent: { matches: 4, wins: 3, average_kda: 4 }, champion_usage: { mode: "none" }, performance_tags: [], data_availability: { history: true } },
          ],
        });

      render(<LeagueOngoingGame streamerMode={false} useAliases={false} />);
      await act(async () => { await Promise.resolve(); });

      expect(screen.getByText("Ready")).toBeTruthy();
      expect(screen.getByTestId("ongoing-player-loading-loading")).toBeTruthy();
      expect(screen.getByTestId("ongoing-player-card-ready").textContent).toContain("67%");
      expect(screen.getByTestId("ongoing-player-card-loading").textContent).not.toContain("0%");

      await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
      expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(2);
      expect(screen.queryByTestId("ongoing-player-loading-loading")).toBeNull();
      expect(screen.getByTestId("ongoing-player-card-loading").textContent).toContain("75%");

      await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
      expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(2);
      await act(async () => { await vi.advanceTimersByTimeAsync(4000); });
      expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("cancels the progressive polling timer on unmount", async () => {
    vi.useFakeTimers();
    try {
      fetchLeagueOngoingGame.mockResolvedValue({ available: true, partial: true, game_id: 7, query_stage: "lobby", players: [] });
      const { unmount } = render(<LeagueOngoingGame streamerMode={false} useAliases={false} />);
      await act(async () => { await Promise.resolve(); });
      expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(1);
      await act(async () => { await vi.advanceTimersByTimeAsync(300); });
      const callsBeforeUnmount = fetchLeagueOngoingGame.mock.calls.length;
      unmount();
      await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
      expect(fetchLeagueOngoingGame).toHaveBeenCalledTimes(callsBeforeUnmount);
    } finally {
      vi.useRealTimers();
    }
  });

  it("expands and collapses one player card without changing the player navigation", async () => {
    render(<LeagueOngoingGame />);

    const expand = await screen.findByRole("button", { name: "展开 Tester 详情" });
    expect(screen.queryByTestId("player-details")).toBeNull();
    fireEvent.click(expand);
    expect(await screen.findByTestId("player-details")).toBeTruthy();
    expect(screen.getByTestId("ongoing-player-card-player-1").className.split(/\s+/)).not.toContain("h-[375px]");
    expect(screen.getByText("当前英雄使用")).toBeTruthy();
    expect(screen.getByText("近期对局")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "收起 Tester 详情" }));
    expect(screen.queryByTestId("player-details")).toBeNull();
  });

  it("shows tag explanations as visible text and a native tooltip", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      query_stage: "lobby",
      players: [{
        puuid: "player-1",
        team: "LOBBY",
        summoner: { gameName: "Tester", profileIconId: 12 },
        recent: { matches: 5, wins: 4, average_kda: 4.2, akari_score: 7.5 },
        champion_usage: { mode: "none" },
        performance_tags: [{ id: "hot", label: "近况强势", tone: "positive", title: "最近 5 场赢下 4 场。" }],
      }],
    });
    render(<LeagueOngoingGame />);
    fireEvent.click(await screen.findByRole("button", { name: "展开 Tester 详情" }));
    expect(await screen.findByTestId("player-tag-explanations")).toBeTruthy();
    expect(screen.getByText("最近 5 场赢下 4 场。")).toBeTruthy();
    expect(screen.getAllByTitle("最近 5 场赢下 4 场。").length).toBeGreaterThanOrEqual(2);
  });

  it("renders recent matches from the existing payload and loads detailed data only on demand", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      query_stage: "lobby",
      players: [{
        puuid: "player-1",
        team: "LOBBY",
        summoner: { gameName: "Tester", profileIconId: 12 },
        recent: { matches: 1, wins: 1, average_kda: 3, akari_score: 6 },
        champion_usage: { mode: "recent", matches: 1, wins: 1, average_kda: 3 },
        games: { games: [{
          gameId: 9001,
          gameCreation: 1786600000000,
          gameDuration: 1200,
          gameMode: "CLASSIC",
          queueId: 420,
          participantIdentities: [{ participantId: 1, player: { puuid: "player-1", gameName: "Tester" } }],
          participants: [{ participantId: 1, teamId: 100, championId: 1, spell1Id: 4, spell2Id: 14, stats: { kills: 8, deaths: 2, assists: 4, win: true, totalMinionsKilled: 120, totalDamageDealtToChampions: 18000, item0: 1001 } }],
        }] },
      }],
    });
    render(<LeagueOngoingGame />);
    expect(await screen.findByTestId("ongoing-mini-history")).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: "展开 Tester 详情" }));
    expect(await screen.findByTestId("player-recent-matches")).toBeTruthy();
    expect(fetchLeagueMatchDetails).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    fireEvent.click(screen.getByRole("button", { name: "时间线" }));
    await waitFor(() => expect(fetchLeagueMatchDetails).toHaveBeenCalledWith(9001, "auto"));
  });

  it("falls back to a populated games list when recent_matches is empty", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      query_stage: "lobby",
      players: [{
        puuid: "player-1",
        team: "LOBBY",
        summoner: { gameName: "Tester" },
        recent: { matches: 1, wins: 1, average_kda: 3, akari_score: 6 },
        recent_matches: [],
        games: { games: [{
          gameId: 9003,
          gameCreation: 1786600000000,
          gameMode: "CLASSIC",
          queueId: 420,
          participantIdentities: [{ participantId: 1, player: { puuid: "player-1", gameName: "Tester" } }],
          participants: [{ participantId: 1, teamId: 100, championId: 1, championName: "安妮", stats: { kills: 8, deaths: 2, assists: 4, win: true } }],
        }] },
        champion_usage: { mode: "none" },
        performance_tags: [],
      }],
    });

    render(<LeagueOngoingGame />);

    const history = await screen.findByTestId("ongoing-mini-history");
    expect(history.textContent).toContain("单双排位");
    expect(history.textContent).toContain("8 / 2 / 4");
  });

  it("keeps the loading state visible until the initial live request resolves", async () => {
    let resolveGame;
    fetchLeagueOngoingGame.mockReturnValueOnce(new Promise((resolve) => { resolveGame = resolve; }));
    render(<LeagueOngoingGame />);

    expect(screen.getByTestId("ongoing-loading-state")).toBeTruthy();
    expect(screen.queryByTestId("ongoing-idle-state")).toBeNull();

    await act(async () => {
      resolveGame({ available: true, query_stage: "lobby", players: [] });
      await Promise.resolve();
    });
    expect(screen.queryByTestId("ongoing-loading-state")).toBeNull();
    expect(screen.queryByTestId("ongoing-idle-state")).toBeNull();
  });

  it("does not request a live snapshot for historical preview data", async () => {
    render(<LeagueOngoingGame previewData={{ historical_preview: true, available: true, game_id: 7000, players: [] }} />);
    expect(await screen.findByText("历史对局模拟 · Game 7000")).toBeTruthy();
    expect(fetchLeagueOngoingGame).not.toHaveBeenCalled();
    expect(fetchLeagueLabStatus).not.toHaveBeenCalled();
  });

  it("does not let a live response overwrite a newly opened historical preview", async () => {
    let resolveGame;
    fetchLeagueOngoingGame.mockReturnValueOnce(new Promise((resolve) => { resolveGame = resolve; }));
    const { rerender } = render(<LeagueOngoingGame />);
    const preview = { historical_preview: true, available: true, game_id: 7001, players: [] };

    rerender(<LeagueOngoingGame previewData={preview} />);
    expect(await screen.findByText("历史对局模拟 · Game 7001")).toBeTruthy();

    await act(async () => {
      resolveGame({ available: true, query_stage: "lobby", players: [{ puuid: "live", team: "LOBBY", summoner: { gameName: "Live" } }] });
      await Promise.resolve();
    });
    expect(screen.getByText("历史对局模拟 · Game 7001")).toBeTruthy();
    expect(screen.queryByText("Live")).toBeNull();
  });

  it("renders LeagueAkari-style recent result rows directly in each player card", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      query_stage: "lobby",
      players: [{
        puuid: "player-1",
        team: "LOBBY",
        summoner: { gameName: "Tester" },
        recent: { matches: 2, wins: 1, average_kda: 3, akari_score: 6 },
        recent_matches: [
          { game_id: 9001, played_at: 1786600000000, queue_id: 420, champion_id: 1, champion_name: "安妮", kills: 8, deaths: 2, assists: 4, win: true },
          { game_id: 9002, played_at: 1786500000000, queue_id: 450, champion_id: 2, champion_name: "阿狸", kills: 2, deaths: 7, assists: 3, win: false },
        ],
        champion_usage: { mode: "none" },
        performance_tags: [],
      }],
    });

    render(<LeagueOngoingGame />);

    const history = await screen.findByTestId("ongoing-mini-history");
    expect(history.textContent).toContain("单双排位");
    expect(history.textContent).toContain("极地大乱斗");
    expect(history.textContent).toMatch(/\d{2}-\d{2} \d{2}:\d{2}/);
    expect(history.textContent).toContain("胜利");
    expect(history.textContent).toContain("失败");
    expect(history.textContent).toContain("8 / 2 / 4");
    expect(history.textContent).toContain("2 / 7 / 3");
    expect(history.querySelectorAll("img")).toHaveLength(2);
  });

  it("accepts camelCase gameId summaries without dropping them", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      query_stage: "lobby",
      players: [{
        puuid: "player-1",
        team: "LOBBY",
        summoner: { gameName: "Tester" },
        recent: { matches: 1, wins: 1, average_kda: 3, akari_score: 6 },
        recent_matches: [{ gameId: 9004, queueId: 420, gameMode: "CLASSIC", championId: 1, championName: "安妮", kills: 1, deaths: 0, assists: 2, win: true }],
        champion_usage: { mode: "none" },
        performance_tags: [],
      }],
    });

    render(<LeagueOngoingGame />);

    expect(await screen.findByTestId("ongoing-mini-history")).toBeTruthy();
    expect(screen.getByText("单双排位")).toBeTruthy();
  });

  it("keeps the expanded card useful when the payload has no recent matches", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      query_stage: "lobby",
      players: [{ puuid: "player-1", team: "LOBBY", summoner: { gameName: "Tester" }, recent: {}, champion_usage: { mode: "none" }, performance_tags: [] }],
    });
    render(<LeagueOngoingGame />);
    fireEvent.click(await screen.findByRole("button", { name: "展开 Tester 详情" }));
    expect(await screen.findByText("暂无可展示的近期对局；当前卡片只显示客户端已返回的聚合指标。"  )).toBeTruthy();
    expect(screen.getByText("当前 payload 没有排位明细。")).toBeTruthy();
    expect(screen.getByText("当前没有可解释的标签。")).toBeTruthy();
  });

  it("explains partial LCU data without turning unavailable history into zero evidence", async () => {
    fetchLeagueOngoingGame.mockResolvedValueOnce({
      available: true,
      players: [{
        puuid: "partial",
        team: 100,
        champion_name: "安妮",
        summoner: { gameName: "Partial" },
        ranked: {},
        recent: { matches: 0, wins: 0 },
        champion_usage: { mode: "recent", matches: 0, wins: 0 },
        performance_tags: [],
        data_availability: { summoner: true, ranked: false, history: false, mastery: true, unavailable: ["ranked", "history"] },
      }],
    });

    render(<LeagueOngoingGame />);
    await screen.findByText("Partial");
    fireEvent.click(screen.getByRole("button", { name: "展开 Partial 详情" }));

    expect(screen.getByTestId("player-data-unavailable").textContent).toContain("排位信息、近期战绩");
    expect(screen.getByText("当前客户端未开放该玩家的排位资料。")).toBeTruthy();
    expect(screen.getByText("当前客户端未开放该玩家的近期战绩。")).toBeTruthy();
  });
});
