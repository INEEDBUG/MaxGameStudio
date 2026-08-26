import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueDetailedMatchCard from "./LeagueDetailedMatchCard";
import { fetchLeagueLoadoutCatalog, fetchLeagueMatchDetails } from "../../api/leagueLabApi";

vi.mock("../../api/leagueLabApi", () => ({
  deleteLeaguePlayerEncounter: vi.fn(),
  downloadLeagueReplay: vi.fn(),
  fetchLeagueLoadoutCatalog: vi.fn(),
  fetchLeagueMatchDetails: vi.fn(),
  watchLeagueReplay: vi.fn(),
}));

const match = {
  game_id: 1001,
  participant_puuid: "self",
  team_id: 100,
  champion_id: 1,
  champion_name: "安妮",
  spell1_id: 4,
  spell2_id: 14,
  kills: 10,
  deaths: 2,
  assists: 8,
  damage: 20000,
  cs: 180,
  gold: 12000,
  duration_seconds: 1800,
  played_at: 1786600000000,
  game_mode: "CLASSIC",
  win: true,
  items: [1001, 1002],
  participants: [
    { participant_id: 1, puuid: "self", team_id: 100, champion_id: 1, champion_name: "安妮", game_name: "自己", kills: 10, deaths: 2, assists: 8, damage: 20000, cs: 180, gold: 12000, win: true, items: [1001], perks: [8010], raw_stats: { kills: 10, totalDamageDealtToChampions: 20000 } },
    { participant_id: 2, puuid: "ally", team_id: 100, champion_id: 2, champion_name: "奥拉夫", game_name: "队友", kills: 5, deaths: 4, assists: 7, damage: 12000, cs: 140, gold: 9000, win: true, items: [1002], raw_stats: { kills: 5, totalDamageDealtToChampions: 12000 } },
    { participant_id: 6, puuid: "enemy", team_id: 200, champion_id: 3, champion_name: "加里奥", game_name: "对手", kills: 4, deaths: 8, assists: 3, damage: 9000, cs: 150, gold: 8000, win: false, items: [1003] },
  ],
};

describe("LeagueDetailedMatchCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchLeagueMatchDetails.mockResolvedValue({
      source: "sgp", map_id: 11, frame_count: 2, event_count: 5,
      participants: [{ participant_id: 1, team_id: 100, game_name: "自己" }, { participant_id: 6, team_id: 200, game_name: "对手" }],
      frames: [
        { timestamp: 0, participant_frames: { "1": { totalGold: 500 }, "6": { totalGold: 500 } }, events: [] },
        { timestamp: 60000, participant_frames: { "1": { totalGold: 1200, position: { x: 7000, y: 7000 }, championStats: { health: 640, healthMax: 900, attackDamage: 82, abilityHaste: 15 } }, "6": { totalGold: 900 } }, events: [] },
      ],
      events: [
        { type: "CHAMPION_KILL", timestamp: 45000, killerId: 1, victimId: 6, position: { x: 7000, y: 7000 } },
        { type: "ITEM_PURCHASED", timestamp: 30000, participantId: 1, itemId: 1001 },
        { type: "ITEM_SOLD", timestamp: 90000, participantId: 1, itemId: 1001 },
        { type: "ITEM_PURCHASED", timestamp: 120000, participantId: 1, itemId: 6032 },
        { type: "SKILL_LEVEL_UP", timestamp: 60000, participantId: 1, skillSlot: 1, levelUpType: "EVOLVE" },
      ],
    });
    fetchLeagueLoadoutCatalog.mockResolvedValue({ perks: [{ id: 8010, name: "征服者", long_description: "攻击英雄时提供适应之力。" }] });
  });

  it("expands into both team scoreboards and opens a participant", async () => {
    const onOpenPlayer = vi.fn();
    render(<LeagueDetailedMatchCard match={match} onOpenPlayer={onOpenPlayer} />);

    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    expect(await screen.findByText("队伍 100 · 胜利")).toBeTruthy();
    expect(screen.getByText("队伍 200 · 失败")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: /队友/ }).at(-1));
    expect(onOpenPlayer).toHaveBeenCalledWith("ally");
  });

  it("does not invent team shares from a current-player-only Tencent summary", () => {
    render(<LeagueDetailedMatchCard match={{ ...match, participants: [match.participants[0]] }} />);
    expect(screen.getByText("参团").parentElement.textContent).toContain("—");
    expect(screen.getByText("伤害占比").parentElement.textContent).toContain("—");
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    expect(screen.queryByText("180%")).toBeNull();
    expect(screen.queryByText("100%")).toBeNull();
    expect(screen.getAllByText("—").length).toBeGreaterThanOrEqual(4);
  });

  it("uses non-empty item slots when a summary also contains an empty items array", () => {
    const withSlots = {
      ...match,
      items: [],
      item_slots: [1001, 0, 1002, 0, 0, 0, 2052],
      participants: match.participants.map((player, index) => index === 0
        ? { ...player, items: [], item_slots: [1001, 0, 1002, 0, 0, 0, 2052] }
        : player),
    };
    render(<LeagueDetailedMatchCard match={withSlots} />);

    expect(screen.getAllByTitle("装备 1001").length).toBeGreaterThan(0);
    expect(screen.getAllByTitle("装备 1002").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    const teamTable = screen.getAllByTestId("league-team-table")[0];
    expect(teamTable.firstElementChild.className).toContain("min-w-[920px]");
    expect(teamTable.children[1].className).toContain("min-w-[920px]");
  });

  it("renders LeagueAkari-style spells arrays and the local LCU asset URL", () => {
    const upstreamMatch = {
      ...match,
      spell1_id: undefined,
      spell2_id: undefined,
      participants: match.participants.map((player, index) => index === 0
        ? { ...player, spell1_id: undefined, spell2_id: undefined, spells: [4, 14], perks: [] }
        : player),
    };
    render(<LeagueDetailedMatchCard match={upstreamMatch} />);

    expect(screen.getByTitle("召唤师技能 4").getAttribute("src")).toBe("/api/league-lab/assets/summoner-spells/4.png");
    expect(screen.getByTitle("召唤师技能 14").getAttribute("src")).toBe("/api/league-lab/assets/summoner-spells/14.png");
  });

  it("loads timeline details only when that tab is opened", async () => {
    render(<LeagueDetailedMatchCard match={match} />);
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    expect(fetchLeagueMatchDetails).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "时间线" }));
    await waitFor(() => expect(fetchLeagueMatchDetails).toHaveBeenCalledWith(1001, "auto"));
    expect(await screen.findByText("经济差")).toBeTruthy();
  });

  it("shows the searchable ten-player stat matrix and visual rune metadata", async () => {
    render(<LeagueDetailedMatchCard match={match} />);
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    fireEvent.click(screen.getByRole("button", { name: "详细属性" }));
    expect(screen.getAllByText("战斗数据").length).toBeGreaterThan(0);
    expect(screen.getAllByText("伤害与承伤").length).toBeGreaterThan(0);
    expect(screen.getAllByText("对英雄伤害").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByTitle("totalDamageDealtToChampions · 点击比较十名玩家"));
    expect(screen.getAllByText("20,000").length).toBeGreaterThan(1);
    fireEvent.change(screen.getByPlaceholderText("筛选属性名称…"), { target: { value: "击杀" } });
    expect(screen.getAllByText("击杀").length).toBeGreaterThan(0);
    fireEvent.change(screen.getByLabelText("属性分组"), { target: { value: "damage" } });
    expect(screen.queryByText("击杀")).toBeNull();
    fireEvent.change(screen.getByPlaceholderText("筛选属性名称…"), { target: { value: "" } });
    expect(screen.getAllByText("伤害与承伤").length).toBeGreaterThan(0);

    fireEvent.click(screen.getByRole("button", { name: "符文" }));
    await waitFor(() => expect(fetchLeagueLoadoutCatalog).toHaveBeenCalledTimes(1));
    expect(await screen.findByText("征服者")).toBeTruthy();
  });

  it("filters events by champion and exposes map and player timeline views", async () => {
    render(<LeagueDetailedMatchCard match={{ ...match, map_id: 11 }} />);
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    fireEvent.click(screen.getByRole("button", { name: "事件" }));
    expect(await screen.findByText("按英雄筛选")).toBeTruthy();
    expect(screen.getByText("查看地图位置")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "时间线" }));
    expect(await screen.findByText("1:00 · 经济差 +300")).toBeTruthy();
    fireEvent.click(await screen.findByRole("button", { name: "玩家属性" }));
    expect(screen.getByRole("img", { name: "玩家属性时间线" })).toBeTruthy();
    expect(screen.getByText("1:00 · 总金币 1,200")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("时间线帧"), { target: { value: "1" } });
    expect(screen.getByText("640 / 900")).toBeTruthy();
    expect(screen.getByText("技能急速")).toBeTruthy();
  });

  it("shows purchase, sale and evolved-skill build events with a player navigator", async () => {
    render(<LeagueDetailedMatchCard match={match} />);
    fireEvent.click(screen.getByRole("button", { name: "展开战绩详情" }));
    fireEvent.click(screen.getByRole("button", { name: "出装过程" }));
    expect(await screen.findByText(/出售 1:30/)).toBeTruthy();
    expect(screen.getByTitle("1:00 · EVOLVE")).toBeTruthy();
    expect(screen.getByText("铁砧 × 1")).toBeTruthy();
    expect(screen.getByLabelText("购买阶段分隔")).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: /队友/ }).at(-1));
    expect(screen.getAllByText("无数据")).toHaveLength(2);
  });
});
