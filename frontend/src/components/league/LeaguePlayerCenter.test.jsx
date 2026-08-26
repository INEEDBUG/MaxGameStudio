import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeaguePlayerCenter from "./LeaguePlayerCenter";
import {
  fetchCurrentLeaguePlayer,
  fetchLeagueLoadoutCatalog,
  fetchLeagueMatchDetails,
  fetchLeaguePlayer,
  fetchLeaguePlayerCollection,
  fetchLeaguePlayerEncounters,
  fetchLeaguePlayerFriends,
  fetchLeaguePlayerJungleAnalysis,
  fetchLeaguePlayerSearchHistory,
  fetchLeaguePlayerSearchServers,
  fetchLeaguePlayerMastery,
  fetchRecentLeaguePlayers,
  saveLeaguePlayerTag,
  searchLeaguePlayer,
} from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  deleteLeaguePlayerEncounter: vi.fn(),
  deleteLeaguePlayerSearchHistory: vi.fn(),
  downloadLeagueReplay: vi.fn(),
  fetchCurrentLeaguePlayer: vi.fn(),
  fetchLeagueLoadoutCatalog: vi.fn(),
  fetchLeagueMatchDetails: vi.fn(),
  fetchLeaguePlayer: vi.fn(),
  fetchLeaguePlayerCollection: vi.fn(),
  fetchLeaguePlayerEncounters: vi.fn(),
  fetchLeaguePlayerFriends: vi.fn(),
  fetchLeaguePlayerJungleAnalysis: vi.fn(),
  fetchLeaguePlayerMastery: vi.fn(),
  fetchLeaguePlayerSearchHistory: vi.fn(),
  fetchLeaguePlayerSearchServers: vi.fn(),
  fetchLeagueReplay: vi.fn(),
  fetchRecentLeaguePlayers: vi.fn(),
  pinLeaguePlayerSearchHistory: vi.fn(),
  saveLeaguePlayerTag: vi.fn(),
  searchLeaguePlayer: vi.fn(),
  spectateLeagueFriend: vi.fn(),
  watchLeagueReplay: vi.fn(),
}));

const makeMatch = () => ({
  game_id: 1001,
  participant_puuid: "self",
  team_id: 100,
  champion_id: 1,
  champion_name: "安妮",
  kills: 10,
  deaths: 2,
  assists: 8,
  damage: 20000,
  damage_taken: 8000,
  gold: 12000,
  cs: 180,
  vision_score: 25,
  duration_seconds: 1800,
  played_at: 1786600000000,
  game_mode: "CLASSIC",
  queue_id: 420,
  win: true,
  participants: [
    { participant_id: 1, puuid: "self", team_id: 100, game_name: "Self", champion_id: 1, champion_name: "安妮", kills: 10, deaths: 2, assists: 8, damage: 20000, damage_taken: 8000, gold: 12000, cs: 180, vision_score: 25, win: true },
    { participant_id: 2, puuid: "ally", team_id: 100, game_name: "Ally", champion_id: 2, champion_name: "奥拉夫", kills: 5, deaths: 4, assists: 7, damage: 12000, damage_taken: 7000, gold: 9000, cs: 140, vision_score: 18, win: true },
    { participant_id: 6, puuid: "enemy", team_id: 200, game_name: "Enemy", champion_id: 3, champion_name: "加里奥", kills: 4, deaths: 8, assists: 3, damage: 9000, damage_taken: 9000, gold: 8000, cs: 150, vision_score: 12, win: false },
  ],
  challenges: { kda: 9, killParticipation: 0.6, visionScorePerMinute: 1.2, damagePerMinute: 500 },
});

const makeBundle = (overrides = {}) => ({
  summoner: { puuid: "self", game_name: "Tester", tag_line: "CN1", summoner_level: 30 },
  ranked: { queues: [{ queueType: "RANKED_SOLO_5x5", tier: "GOLD", division: "II", leaguePoints: 50 }] },
  matches: [makeMatch()],
  mastery: [{ championId: 1, championName: "安妮", championPoints: 12345, championLevel: 7 }],
  player_challenges: { playerChallenges: [{ id: 505001, currentValue: 20, currentLevel: 3 }] },
  tag: { label: "可靠队友", note: "本地备注", color: "emerald" },
  match_source: "lcu",
  ranked_source: "lcu",
  page: { has_more: false },
  collection_count: 0,
  ...overrides,
});

describe("LeaguePlayerCenter", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchCurrentLeaguePlayer.mockResolvedValue(makeBundle());
    fetchLeaguePlayer.mockResolvedValue(makeBundle());
    searchLeaguePlayer.mockResolvedValue(makeBundle());
    fetchLeaguePlayerSearchHistory.mockResolvedValue({ players: [] });
    fetchLeaguePlayerFriends.mockResolvedValue({ friends: [] });
    fetchRecentLeaguePlayers.mockResolvedValue({ players: [] });
    fetchLeaguePlayerSearchServers.mockResolvedValue({ servers: [], current: "" });
    fetchLeaguePlayerJungleAnalysis.mockResolvedValue({ games_analyzed: 0, reason: "无可用时间线" });
    fetchLeaguePlayerEncounters.mockResolvedValue({ total: 0, page_size: 10, games: [] });
    fetchLeaguePlayerMastery.mockResolvedValue({ mastery: [] });
    fetchLeaguePlayerCollection.mockResolvedValue({ count: 0, matches: [] });
    fetchLeagueLoadoutCatalog.mockResolvedValue({ perks: [] });
    fetchLeagueMatchDetails.mockResolvedValue({ source: "lcu", frame_count: 0, event_count: 0, events: [], frames: [] });
    saveLeaguePlayerTag.mockResolvedValue({ saved: true });
  });

  it("renders a profile header, tabs, sticky sidebar, and navigable relationship summaries", async () => {
    render(<LeaguePlayerCenter currentPuuid="self" onError={vi.fn()} />);

    expect(await screen.findByTestId("player-profile-header")).toBeTruthy();
    expect(screen.getByTestId("player-profile-sidebar").className).toContain("lg:sticky");
    expect(screen.getByRole("tab", { name: "概览" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "战绩" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "英雄/熟练度" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "挑战" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "遇到的对局" })).toBeTruthy();
    expect(screen.getByText("最近队友")).toBeTruthy();
    expect(screen.getByText("最近对手")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Ally/ }));
    await waitFor(() => expect(fetchLeaguePlayer).toHaveBeenCalledWith("ally", 20, 0, ""));
  });

  it("renders the profile icon from the player payload", async () => {
    fetchLeaguePlayer.mockResolvedValueOnce(makeBundle({
      summoner: { puuid: "self", game_name: "Tester", tag_line: "CN1", summoner_level: 30, profile_icon_id: 29 },
    }));

    render(<LeaguePlayerCenter currentPuuid="self" onError={vi.fn()} />);

    const image = await screen.findByAltText("玩家头像");
    expect(image.getAttribute("src")).toContain("/api/league-lab/assets/profile-icons/29.jpg");
  });

  it("switches between tabs without dropping existing search history navigation", async () => {
    render(<LeaguePlayerCenter currentPuuid="self" onError={vi.fn()} />);
    await screen.findByTestId("player-profile-header");

    fireEvent.click(screen.getByRole("tab", { name: "英雄/熟练度" }));
    expect(screen.getByTestId("player-champions-panel")).toBeTruthy();
    expect(screen.getByText("英雄熟练度")).toBeTruthy();
    expect(screen.getByText("常用英雄分析")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "战绩" }));
    expect(screen.getByTestId("player-history-panel")).toBeTruthy();
    expect(screen.getByPlaceholderText("筛选英雄或队列 ID")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "概览" }));
    expect(screen.getByTestId("player-overview-panel")).toBeTruthy();
    await waitFor(() => expect(fetchLeaguePlayerSearchHistory).toHaveBeenCalled());
  });

  it("shows explicit empty states instead of inventing mastery, challenge, or encounter data", async () => {
    fetchLeaguePlayer.mockResolvedValueOnce(makeBundle({ matches: [], mastery: [], player_challenges: { playerChallenges: [] } }));
    render(<LeaguePlayerCenter currentPuuid="self" onError={vi.fn()} />);
    await screen.findByTestId("player-profile-header");

    fireEvent.click(screen.getByRole("tab", { name: "英雄/熟练度" }));
    expect(screen.getByTestId("player-mastery-empty")).toBeTruthy();
    expect(screen.getByTestId("player-champion-empty")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "挑战" }));
    expect(screen.getByTestId("player-collection-empty")).toBeTruthy();
    expect(screen.getByTestId("player-challenge-empty")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "遇到的对局" }));
    expect(await screen.findByTestId("encounters-empty")).toBeTruthy();
    expect(screen.getByText(/encounters 数据为空/)).toBeTruthy();
  });

  it("saves the local tag from the sticky sidebar", async () => {
    render(<LeaguePlayerCenter currentPuuid="self" onError={vi.fn()} />);
    await screen.findByTestId("player-profile-header");
    fireEvent.change(screen.getByPlaceholderText("例如：擅长打野 / 可靠队友"), { target: { value: "高光队友" } });
    fireEvent.click(screen.getByRole("button", { name: "保存标签" }));
    await waitFor(() => expect(saveLeaguePlayerTag).toHaveBeenCalledWith("self", expect.objectContaining({ label: "高光队友" })));
  });
});
